use crate::discovery::HostEntry;
use crate::mutation::{self, UpdateRequest};
use crate::pair::PairedRoute;
use crate::permissions;
use crate::session::{self, ServiceForward};
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::fd::{FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const SYSTEM_CONFIG: &str = "/etc/ssh/ssh_config";
const MASTER_TIMEOUT: Duration = Duration::from_secs(30);
const FORWARD_READY_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

static SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn signal_handler(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            signal_handler as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {}

#[cfg(unix)]
fn reset_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::signal(libc::SIGHUP, libc::SIG_DFL);
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_signal_handlers() {}

fn received_signal() -> Option<i32> {
    match SIGNAL.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PasswordSource {
    Configured,
    Supplied,
    Prompted,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForwardStage {
    None,
    Transit,
    Service,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PairedCredentials {
    pub gateway_password_fd: Option<i32>,
    pub vm_password_fd: Option<i32>,
}

/// Retains standalone-tunnel password attempts across workspace retries.
pub struct StandaloneCredentials {
    input: Option<i32>,
    attempt: Option<PasswordAttempt>,
}

impl StandaloneCredentials {
    /// Stores the password descriptor without reading it until tunnel startup.
    pub fn new(input: Option<i32>) -> Self {
        Self {
            input,
            attempt: None,
        }
    }
}

/// Retains paired-session password attempts across workspace retries.
pub struct PairedSessionCredentials {
    input: PairedCredentials,
    gateway_attempt: Option<PasswordAttempt>,
    vm_attempt: Option<PasswordAttempt>,
}

impl PairedSessionCredentials {
    /// Stores password descriptors without reading them until session startup.
    pub fn new(input: PairedCredentials) -> Self {
        Self {
            input,
            gateway_attempt: None,
            vm_attempt: None,
        }
    }
}

struct PasswordAttempt {
    bytes: Vec<u8>,
    source: PasswordSource,
    replacement: Option<String>,
}

impl PasswordAttempt {
    fn configured(value: String) -> Self {
        Self {
            bytes: value.as_bytes().to_vec(),
            source: PasswordSource::Configured,
            replacement: None,
        }
    }

    fn supplied(bytes: Vec<u8>) -> Self {
        Self {
            replacement: String::from_utf8(bytes.clone()).ok(),
            bytes,
            source: PasswordSource::Supplied,
        }
    }

    fn prompted(value: String) -> Self {
        Self {
            bytes: value.as_bytes().to_vec(),
            source: PasswordSource::Prompted,
            replacement: Some(value),
        }
    }
}
struct Runtime {
    dir: PathBuf,
    config: PathBuf,
    socket: PathBuf,
    alias: String,
    known_hosts: PathBuf,
    user_known_hosts_file_configured: bool,
    strict_host_key_checking_configured: bool,
    entry: HostEntry,
    configured_password: Option<String>,
    forwards: Vec<ServiceForward>,
    extra_listeners: Vec<(String, u16)>,
    preserve_dir: bool,
}
pub(crate) struct StandaloneRuntime {
    runtime: Runtime,
}

impl StandaloneRuntime {
    pub(crate) fn control_dir(&self) -> &Path {
        &self.runtime.dir
    }

    pub(crate) fn control_socket(&self) -> &Path {
        &self.runtime.socket
    }

    pub(crate) fn runtime_config(&self) -> &Path {
        &self.runtime.config
    }

    pub(crate) fn alias(&self) -> &str {
        &self.runtime.alias
    }
}

pub(crate) struct PairedStandaloneRuntime {
    gateway: StandaloneRuntime,
    vm: StandaloneRuntime,
    transit_port: u16,
}

impl PairedStandaloneRuntime {
    pub(crate) fn gateway_control_dir(&self) -> &Path {
        self.gateway.control_dir()
    }

    pub(crate) fn gateway_control_socket(&self) -> &Path {
        self.gateway.control_socket()
    }

    pub(crate) fn gateway_runtime_config(&self) -> &Path {
        self.gateway.runtime_config()
    }

    pub(crate) fn vm_control_dir(&self) -> &Path {
        self.vm.control_dir()
    }

    pub(crate) fn vm_control_socket(&self) -> &Path {
        self.vm.control_socket()
    }

    pub(crate) fn vm_runtime_config(&self) -> &Path {
        self.vm.runtime_config()
    }

    pub(crate) fn transit_port(&self) -> u16 {
        self.transit_port
    }
}

struct OwnedMaster {
    runtime: Runtime,
    child: Child,
}

struct SessionService {
    masters: Vec<OwnedMaster>,
}

impl SessionService {
    fn new() -> Self {
        Self {
            masters: Vec::new(),
        }
    }

    fn add(&mut self, runtime: Runtime, child: Child) -> usize {
        self.masters.push(OwnedMaster { runtime, child });
        self.masters.len() - 1
    }

    fn runtime(&self, index: usize) -> &Runtime {
        &self.masters[index].runtime
    }

    fn stop(&mut self) {
        while let Some(mut master) = self.masters.pop() {
            stop_master(&master.runtime, &mut master.child);
        }
    }
}

impl Drop for SessionService {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Runtime {
    fn create_with_direct_forwards(
        entry: &HostEntry,
        home: &Path,
        selected_alias: &str,
        forwards: &[ServiceForward],
        direct_forwards: &[session::ForwardSpec],
        no_input: bool,
    ) -> Result<Self, String> {
        let mut content = if forwards.is_empty() {
            compile_config(entry, selected_alias)?
        } else {
            compile_config_with_forwards(entry, selected_alias, forwards)?
        };
        for forward in direct_forwards {
            let (directive, argument) = match forward.flag() {
                'L' => ("LocalForward", local_forward_argument(forward)?),
                'R' => ("RemoteForward", remote_forward_argument(forward)?),
                'D' => ("DynamicForward", forward.effective.clone()),
                _ => continue,
            };
            let index = content.rfind("Include ").ok_or_else(|| {
                "CONFIG_INVALID: runtime config has no system Include boundary".to_string()
            })?;
            content.insert_str(index, &format!("  {directive} {argument}\n"));
        }
        let mut runtime =
            Self::create_with_content(entry, home, selected_alias, content, forwards, no_input)?;
        runtime.extra_listeners = direct_forwards
            .iter()
            .filter(|forward| matches!(forward.flag(), 'L' | 'D'))
            .filter_map(session::ForwardSpec::listener)
            .collect();
        Ok(runtime)
    }

    fn create_with_content(
        entry: &HostEntry,
        home: &Path,
        selected_alias: &str,
        content: String,
        forwards: &[ServiceForward],
        no_input: bool,
    ) -> Result<Self, String> {
        let dir = temporary_directory()?;
        Self::create_with_directory(
            entry,
            home,
            selected_alias,
            content,
            forwards,
            dir,
            false,
            no_input,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_with_directory(
        entry: &HostEntry,
        home: &Path,
        selected_alias: &str,
        content: String,
        forwards: &[ServiceForward],
        dir: PathBuf,
        preserve_dir: bool,
        no_input: bool,
    ) -> Result<Self, String> {
        if !entry.aliases.iter().any(|alias| alias == selected_alias) {
            return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
        }
        let configured_password = match selected_password(entry, no_input) {
            Ok(password) => password,
            Err(error) => {
                if !preserve_dir {
                    let _ = fs::remove_dir_all(&dir);
                }
                return Err(error);
            }
        };
        let config = dir.join("config");
        let socket = dir.join("master.sock");
        let known_hosts = known_hosts_path(home, entry);
        let user_known_hosts_file_configured = content.lines().any(|raw| {
            let line = strip_inline_comment(raw.trim()).trim();
            split_directive(line)
                .is_ok_and(|(keyword, _)| keyword.eq_ignore_ascii_case("userknownhostsfile"))
        });
        let strict_host_key_checking_configured = content.lines().any(|raw| {
            let line = strip_inline_comment(raw.trim()).trim();
            split_directive(line)
                .is_ok_and(|(keyword, _)| keyword.eq_ignore_ascii_case("stricthostkeychecking"))
        });
        if !user_known_hosts_file_configured {
            ensure_known_hosts_parent(&known_hosts)?;
        }
        write_private_file(&config, content.as_bytes())?;
        Ok(Self {
            dir,
            config,
            socket,
            alias: selected_alias.to_string(),
            known_hosts,
            strict_host_key_checking_configured,
            user_known_hosts_file_configured,
            entry: entry.clone(),
            configured_password,
            forwards: forwards.to_vec(),
            extra_listeners: Vec::new(),
            preserve_dir,
        })
    }
}

pub(crate) fn prepare_standalone_runtime(
    entry: &HostEntry,
    home: &Path,
    selected_alias: &str,
    control_dir: PathBuf,
    no_input: bool,
) -> Result<StandaloneRuntime, String> {
    let content = compile_standalone_config(entry, selected_alias)?;
    Ok(StandaloneRuntime {
        runtime: Runtime::create_with_directory(
            entry,
            home,
            selected_alias,
            content,
            &[],
            control_dir,
            true,
            no_input,
        )?,
    })
}
pub(crate) fn prepare_paired_standalone_runtime(
    route: &PairedRoute,
    home: &Path,
    gateway_alias: &str,
    vm_alias: &str,
    forwards: &[ServiceForward],
    control_dir: PathBuf,
    no_input: bool,
) -> Result<PairedStandaloneRuntime, String> {
    let transit_port = allocate_transit_port()?;
    let gateway_config = compile_gateway_config(
        &route.gateway,
        gateway_alias,
        &route.transit_host,
        route.transit_port,
        transit_port,
    )?;
    let vm_config = compile_vm_config(
        &route.vm,
        vm_alias,
        transit_port,
        &stable_vm_host_key_alias(&route.vm_id),
        forwards,
    )?;
    let gateway_dir = control_dir.join("gateway");
    let vm_dir = control_dir.join("vm");
    for directory in [&gateway_dir, &vm_dir] {
        fs::create_dir(directory).map_err(|error| {
            format!(
                "REGISTRY_FAILED: cannot create paired control directory {}: {error}",
                directory.display()
            )
        })?;
        set_mode(directory, 0o700)?;
    }
    let gateway = match Runtime::create_with_directory(
        &route.gateway,
        home,
        gateway_alias,
        gateway_config,
        &[],
        gateway_dir,
        true,
        no_input,
    ) {
        Ok(runtime) => StandaloneRuntime { runtime },
        Err(error) => {
            let _ = fs::remove_dir_all(&control_dir);
            return Err(error);
        }
    };
    let vm = match Runtime::create_with_directory(
        &route.vm, home, vm_alias, vm_config, forwards, vm_dir, true, no_input,
    ) {
        Ok(runtime) => StandaloneRuntime { runtime },
        Err(error) => {
            let _ = fs::remove_dir_all(&control_dir);
            return Err(error);
        }
    };
    Ok(PairedStandaloneRuntime {
        gateway,
        vm,
        transit_port,
    })
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if !self.preserve_dir {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

pub fn open(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
) -> Result<(), String> {
    open_with_password_fd(entry, home, no_input, selected_alias, None)
}

pub fn open_with_password_fd(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
    password_fd: Option<i32>,
) -> Result<(), String> {
    open_with_password_fd_and_forwards(entry, home, no_input, selected_alias, password_fd, &[])
}

pub fn open_with_password_fd_and_forwards(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
    password_fd: Option<i32>,
    forwards: &[ServiceForward],
) -> Result<(), String> {
    open_with_password_fd_and_all_forwards(
        entry, home, no_input, selected_alias, password_fd, forwards, &[], false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn open_with_password_fd_and_all_forwards(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
    password_fd: Option<i32>,
    forwards: &[ServiceForward],
    direct_forwards: &[session::ForwardSpec],
    terminal: bool,
) -> Result<(), String> {
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = open_session(
        entry,
        home,
        no_input,
        selected_alias,
        password_fd,
        forwards,
        direct_forwards,
        terminal,
    );
    reset_signal_handlers();
    result
}

fn preflight_direct_listeners(
    forwards: &[ServiceForward],
    direct_forwards: &[session::ForwardSpec],
) -> Result<(), String> {
    let mut addresses = Vec::with_capacity(forwards.len() + direct_forwards.len());
    for forward in forwards {
        addresses.push(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            forward.local_port,
        )));
    }
    let mut listeners = Vec::new();
    for forward in direct_forwards
        .iter()
        .filter(|forward| matches!(forward.flag(), 'L' | 'D'))
    {
        let (bind, port) = forward.listener().ok_or_else(|| {
            format!(
                "FORWARD_INVALID: -{} requires a local listener",
                forward.flag()
            )
        })?;
        let address = if bind == "localhost" {
            "127.0.0.1".parse()
        } else {
            bind.parse()
        }
        .map(|ip| std::net::SocketAddr::new(ip, port))
        .map_err(|_| format!("FORWARD_BIND_INVALID: cannot parse bind address {bind}"))?;
        if addresses.contains(&address) {
            return Err(format!(
                "FORWARD_DUPLICATE: local listener {bind}:{port} was requested more than once"
            ));
        }
        listeners.push(TcpListener::bind(address).map_err(|error| {
            format!("SERVICE_BIND_FAILED: cannot reserve local listener {bind}:{port}: {error}")
        })?);
        addresses.push(address);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn open_session(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
    password_fd: Option<i32>,
    forwards: &[ServiceForward],
    direct_forwards: &[session::ForwardSpec],
    terminal: bool,
) -> Result<(), String> {
    session::preflight(forwards)?;
    preflight_direct_listeners(forwards, direct_forwards)?;
    session::warn_remote_exposure(direct_forwards);
    let runtime = Runtime::create_with_direct_forwards(
        entry,
        home,
        selected_alias,
        forwards,
        direct_forwards,
        no_input,
    )?;
    let mut attempt = match password_fd {
        Some(fd) => Some(read_password_fd(fd)?),
        None => runtime
            .configured_password
            .clone()
            .map(PasswordAttempt::configured),
    };
    let mut enrolled = false;
    let master = loop {
        let mut master = spawn_master(&runtime, no_input, attempt.as_ref(), !forwards.is_empty() || !direct_forwards.is_empty())?;
        match wait_for_master(
            &runtime,
            &mut master,
            no_input,
            if forwards.is_empty() && direct_forwards.is_empty() {
                ForwardStage::None
            } else {
                ForwardStage::Service
            },
        ) {
            Ok(()) => break master,
            Err(error) => {
                stop_master(&runtime, &mut master);
                if !enrolled && !no_input && error.starts_with("HOST_KEY_TRUST_REQUIRED") {
                    enroll_host_key(&runtime)?;
                    enrolled = true;
                    continue;
                }
                let can_prompt = !no_input
                    && io::stdin().is_terminal()
                    && error.starts_with("SSH_AUTH_FAILED")
                    && attempt
                        .as_ref()
                        .is_none_or(|value| value.source != PasswordSource::Prompted);
                if can_prompt
                    && let Some(next) = prompt_password(&runtime.alias, attempt.is_some())?
                {
                    attempt = Some(next);
                    continue;
                }
                return Err(error);
            }
        }
    };
    let mut service = SessionService::new();
    let master_index = service.add(runtime, master);
    if let Some(attempt) = attempt.as_ref()
        && let Err(error) = save_replacement(service.runtime(master_index), attempt, no_input)
    {
        return Err(error);
    }
    let mut shell = spawn_shell(service.runtime(master_index), no_input, terminal)?;
    let status = wait_for_shell(&mut shell)?;
    service.stop();
    if let Some(signal) = received_signal() {
        return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("SESSION_EXIT: {status}"))
    }
}
pub fn open_paired(
    route: &PairedRoute,
    home: &Path,
    no_input: bool,
    gateway_alias: &str,
    vm_alias: &str,
    gateway_password_fd: Option<i32>,
    vm_password_fd: Option<i32>,
) -> Result<(), String> {
    let mut credentials = PairedSessionCredentials::new(PairedCredentials {
        gateway_password_fd,
        vm_password_fd,
    });
    open_paired_with_forwards(
        route,
        home,
        no_input,
        gateway_alias,
        vm_alias,
        &mut credentials,
        &[],
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn open_paired_with_forwards(
    route: &PairedRoute,
    home: &Path,
    no_input: bool,
    gateway_alias: &str,
    vm_alias: &str,
    credentials: &mut PairedSessionCredentials,
    forwards: &[ServiceForward],
    terminal: bool,
) -> Result<(), String> {
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = open_paired_session(
        route,
        home,
        no_input,
        gateway_alias,
        vm_alias,
        credentials,
        forwards,
        terminal,
    );
    reset_signal_handlers();
    result
}

#[allow(clippy::too_many_arguments)]
fn open_paired_session(
    route: &PairedRoute,
    home: &Path,
    no_input: bool,
    gateway_alias: &str,
    vm_alias: &str,
    credentials: &mut PairedSessionCredentials,
    forwards: &[ServiceForward],
    terminal: bool,
) -> Result<(), String> {
    session::preflight(forwards).map_err(|error| format!("VM_{error}"))?;
    let transit_port = allocate_transit_port()?;
    let gateway_config = compile_gateway_config(
        &route.gateway,
        gateway_alias,
        &route.transit_host,
        route.transit_port,
        transit_port,
    )?;
    let vm_config = compile_vm_config(
        &route.vm,
        vm_alias,
        transit_port,
        &stable_vm_host_key_alias(&route.vm_id),
        forwards,
    )?;
    let gateway_runtime = Runtime::create_with_content(
        &route.gateway,
        home,
        gateway_alias,
        gateway_config,
        &[],
        no_input,
    )?;
    let vm_runtime =
        Runtime::create_with_content(&route.vm, home, vm_alias, vm_config, forwards, no_input)?;
    if credentials.gateway_attempt.is_none() {
        credentials.gateway_attempt =
            password_attempt(&gateway_runtime, credentials.input.gateway_password_fd)
                .map_err(|error| paired_stage_error("gateway", error))?;
    }
    if credentials.vm_attempt.is_none() {
        credentials.vm_attempt = password_attempt(&vm_runtime, credentials.input.vm_password_fd)
            .map_err(|error| paired_stage_error("VM", error))?;
    }

    let gateway_master = authenticate_paired_master(
        &gateway_runtime,
        no_input,
        &mut credentials.gateway_attempt,
        "gateway",
        ForwardStage::Transit,
    )?;
    let mut service = SessionService::new();
    let gateway_index = service.add(gateway_runtime, gateway_master);
    let vm_master = authenticate_paired_master(
        &vm_runtime,
        no_input,
        &mut credentials.vm_attempt,
        "VM",
        if forwards.is_empty() {
            ForwardStage::None
        } else {
            ForwardStage::Service
        },
    )?;
    let vm_index = service.add(vm_runtime, vm_master);

    if let Some(attempt) = credentials.gateway_attempt.as_ref() {
        save_replacement_for(service.runtime(gateway_index), attempt, no_input, "gateway")?;
    }
    if let Some(attempt) = credentials.vm_attempt.as_ref() {
        save_replacement_for(service.runtime(vm_index), attempt, no_input, "VM")?;
    }

    let mut shell = spawn_shell(service.runtime(vm_index), no_input, terminal)
        .map_err(|error| format!("VM_SESSION_FAILED: {error}"))?;
    let status = wait_for_shell(&mut shell)?;
    service.stop();
    if let Some(signal) = received_signal() {
        return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("VM_SESSION_EXIT: {status}"))
    }
}

fn password_attempt(
    runtime: &Runtime,
    password_fd: Option<i32>,
) -> Result<Option<PasswordAttempt>, String> {
    match password_fd {
        Some(fd) => read_password_fd(fd).map(Some),
        None => Ok(runtime
            .configured_password
            .clone()
            .map(PasswordAttempt::configured)),
    }
}

fn authenticate_paired_master(
    runtime: &Runtime,
    no_input: bool,
    attempt: &mut Option<PasswordAttempt>,
    role: &str,
    forward_stage: ForwardStage,
) -> Result<Child, String> {
    let mut enrolled = false;
    let mut prompted = false;
    loop {
        let mut master = spawn_master(
            runtime,
            no_input,
            attempt.as_ref(),
            matches!(forward_stage, ForwardStage::Transit | ForwardStage::Service),
        )
        .map_err(|error| paired_stage_error(role, error))?;
        match wait_for_master(runtime, &mut master, no_input, forward_stage) {
            Ok(()) => return Ok(master),
            Err(error) => {
                stop_master(runtime, &mut master);
                if !enrolled && !no_input && error.starts_with("HOST_KEY_TRUST_REQUIRED") {
                    enroll_host_key(runtime).map_err(|error| paired_stage_error(role, error))?;
                    enrolled = true;
                    continue;
                }
                let can_prompt = !no_input
                    && io::stdin().is_terminal()
                    && error.starts_with("SSH_AUTH_FAILED")
                    && !prompted;
                if can_prompt
                    && let Some(next) = prompt_password_for(role, &runtime.alias, attempt.is_some())
                        .map_err(|error| paired_stage_error(role, error))?
                {
                    *attempt = Some(next);
                    prompted = true;
                    continue;
                }
                return Err(paired_stage_error(role, error));
            }
        }
    }
}

fn paired_stage_error(role: &str, error: String) -> String {
    if error.starts_with("SESSION_INTERRUPTED") || error.starts_with("SESSION_START_INTERRUPTED") {
        return error;
    }
    if role.eq_ignore_ascii_case("gateway")
        && (error.starts_with("TRANSIT_BIND_FAILED")
            || error.starts_with("SERVICE_BIND_FAILED")
            || error.contains("forward")
            || error.contains("bind"))
    {
        return format!("TRANSIT_BIND_FAILED: gateway transit forward failed: {error}");
    }
    if error.starts_with("SERVICE_BIND_FAILED") {
        return format!("{}_SERVICE_BIND_FAILED: {error}", role.to_ascii_uppercase());
    }
    let stage = if error.starts_with("HOST_KEY_TRUST_REQUIRED") {
        "TRUST_REQUIRED"
    } else if error.starts_with("HOST_KEY_CHANGED") {
        "TRUST_FAILED"
    } else {
        "AUTH_FAILED"
    };
    format!("{}_{}: {error}", role.to_ascii_uppercase(), stage)
}
fn spawn_master(
    runtime: &Runtime,
    no_input: bool,
    attempt: Option<&PasswordAttempt>,
    require_forward: bool,
) -> Result<Child, String> {
    let (mut command, password_pipe) = auth_command(runtime, no_input, true, attempt)?;
    command.args(["-M", "-N", "-o", "ControlMaster=yes"]);
    if require_forward {
        command.args(["-o", "ExitOnForwardFailure=yes"]);
    }
    command.arg(&runtime.alias);
    command
        .stdin(if no_input {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("SSH_AUTH_FAILED: cannot start OpenSSH: {error}"))?;
    if let Some(password_pipe) = password_pipe
        && let Err(error) = password_pipe.send()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

pub(crate) fn launch_standalone(
    runtime: &StandaloneRuntime,
    no_input: bool,
    credentials: &mut StandaloneCredentials,
    forwards: &[(char, String)],
    local_listeners: &[(String, u16)],
) -> Result<(), String> {
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result =
        launch_standalone_session(runtime, no_input, credentials, forwards, local_listeners);
    reset_signal_handlers();
    result
}

fn launch_standalone_session(
    runtime: &StandaloneRuntime,
    no_input: bool,
    credentials: &mut StandaloneCredentials,
    forwards: &[(char, String)],
    local_listeners: &[(String, u16)],
) -> Result<(), String> {
    if credentials.attempt.is_none() {
        credentials.attempt = password_attempt(&runtime.runtime, credentials.input)?;
    }
    let mut enrolled = false;
    loop {
        match launch_standalone_once(
            runtime,
            no_input,
            credentials.attempt.as_ref(),
            forwards,
            local_listeners,
        ) {
            Ok(()) => break,
            Err(error) => {
                stop_standalone(runtime);
                if !enrolled && !no_input && error.starts_with("HOST_KEY_TRUST_REQUIRED") {
                    enroll_host_key(&runtime.runtime)?;
                    enrolled = true;
                    continue;
                }
                let can_prompt = !no_input
                    && io::stdin().is_terminal()
                    && error.starts_with("SSH_AUTH_FAILED")
                    && credentials.attempt
                        .as_ref()
                        .is_none_or(|value| value.source != PasswordSource::Prompted);
                if can_prompt
                    && let Some(next) = prompt_password(&runtime.runtime.alias, credentials.attempt.is_some())?
                {
                    credentials.attempt = Some(next);
                    continue;
                }
                return Err(error);
            }
        }
    }
    if let Some(attempt) = credentials.attempt.as_ref()
        && attempt.source == PasswordSource::Prompted
    {
        save_replacement(&runtime.runtime, attempt, no_input)?;
    }
    Ok(())
}

pub(crate) fn launch_paired_standalone(
    runtime: &PairedStandaloneRuntime,
    no_input: bool,
    credentials: PairedCredentials,
) -> Result<(), String> {
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = launch_paired_standalone_session(runtime, no_input, credentials);
    reset_signal_handlers();
    result
}

fn launch_paired_standalone_session(
    runtime: &PairedStandaloneRuntime,
    no_input: bool,
    credentials: PairedCredentials,
) -> Result<(), String> {
    let gateway_listeners = vec![("127.0.0.1".to_string(), runtime.transit_port)];
    let vm_listeners = runtime
        .vm
        .runtime
        .forwards
        .iter()
        .map(|forward| ("127.0.0.1".to_string(), forward.local_port))
        .collect::<Vec<_>>();
    let gateway_attempt = match launch_detached_master(
        &runtime.gateway,
        no_input,
        credentials.gateway_password_fd,
        &[],
        &gateway_listeners,
        "gateway",
    ) {
        Ok(attempt) => attempt,
        Err(error) => {
            return Err(with_pair_cleanup(
                error,
                cleanup_paired_masters(runtime, false, true),
            ));
        }
    };
    let vm_result = launch_detached_master(
        &runtime.vm,
        no_input,
        credentials.vm_password_fd,
        &[],
        &vm_listeners,
        "VM",
    );
    let vm_attempt = match vm_result {
        Ok(attempt) => attempt,
        Err(error) => {
            return Err(with_pair_cleanup(
                error,
                cleanup_paired_masters(runtime, true, true),
            ));
        }
    };
    if let Some(attempt) = gateway_attempt.as_ref()
        && attempt.source == PasswordSource::Prompted
        && let Err(error) =
            save_replacement_for(&runtime.gateway.runtime, attempt, no_input, "gateway")
    {
        return Err(with_pair_cleanup(
            error,
            cleanup_paired_masters(runtime, true, true),
        ));
    }
    if let Some(attempt) = vm_attempt.as_ref()
        && attempt.source == PasswordSource::Prompted
        && let Err(error) = save_replacement_for(&runtime.vm.runtime, attempt, no_input, "VM")
    {
        return Err(with_pair_cleanup(
            error,
            cleanup_paired_masters(runtime, true, true),
        ));
    }
    Ok(())
}

fn launch_detached_master(
    runtime: &StandaloneRuntime,
    no_input: bool,
    password_fd: Option<i32>,
    forwards: &[(char, String)],
    local_listeners: &[(String, u16)],
    role: &str,
) -> Result<Option<PasswordAttempt>, String> {
    let mut attempt = match password_fd {
        Some(fd) => Some(read_password_fd(fd).map_err(|error| paired_stage_error(role, error))?),
        None => runtime
            .runtime
            .configured_password
            .clone()
            .map(PasswordAttempt::configured),
    };
    let mut enrolled = false;
    loop {
        match launch_standalone_once(
            runtime,
            no_input,
            attempt.as_ref(),
            forwards,
            local_listeners,
        ) {
            Ok(()) => return Ok(attempt),
            Err(error) => {
                if !enrolled && !no_input && error.starts_with("HOST_KEY_TRUST_REQUIRED") {
                    stop_standalone(runtime);
                    enroll_host_key(&runtime.runtime)
                        .map_err(|error| paired_stage_error(role, error))?;
                    enrolled = true;
                    continue;
                }
                let can_prompt = !no_input
                    && io::stdin().is_terminal()
                    && error.starts_with("SSH_AUTH_FAILED")
                    && attempt
                        .as_ref()
                        .is_none_or(|value| value.source != PasswordSource::Prompted);
                if can_prompt
                    && let Some(next) =
                        prompt_password_for(role, &runtime.runtime.alias, attempt.is_some())
                            .map_err(|error| paired_stage_error(role, error))?
                {
                    stop_standalone(runtime);
                    attempt = Some(next);
                    continue;
                }
                return Err(paired_stage_error(role, error));
            }
        }
    }
}

fn with_pair_cleanup(error: String, cleanup_errors: Vec<String>) -> String {
    if cleanup_errors.is_empty() {
        error
    } else {
        format!("{error}; CLEANUP_FAILED: {}", cleanup_errors.join("; "))
    }
}

fn cleanup_paired_masters(
    runtime: &PairedStandaloneRuntime,
    stop_vm: bool,
    stop_gateway: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if stop_vm
        && let Err(error) =
            stop_standalone_checked_at(runtime.vm.control_socket(), runtime.vm.alias())
    {
        errors.push(format!("VM: {error}"));
    }
    if stop_gateway
        && let Err(error) =
            stop_standalone_checked_at(runtime.gateway.control_socket(), runtime.gateway.alias())
    {
        errors.push(format!("gateway: {error}"));
    }
    errors
}

fn launch_standalone_once(
    runtime: &StandaloneRuntime,
    no_input: bool,
    attempt: Option<&PasswordAttempt>,
    forwards: &[(char, String)],
    local_listeners: &[(String, u16)],
) -> Result<(), String> {
    let (mut command, password_pipe) = auth_command(&runtime.runtime, no_input, true, attempt)?;
    command.args([
        "-M",
        "-N",
        "-f",
        "-o",
        "ControlMaster=yes",
        "-o",
        "ExitOnForwardFailure=yes",
    ]);
    for (kind, specification) in forwards {
        command.arg(format!("-{kind}"));
        command.arg(specification);
    }
    command.arg(&runtime.runtime.alias);
    command
        .stdin(if no_input {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("SSH_AUTH_FAILED: cannot start OpenSSH: {error}"))?;
    if let Some(password_pipe) = password_pipe
        && let Err(error) = password_pipe.send()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("SSH_AUTH_FAILED: cannot inspect OpenSSH: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_master_failure(
            output.status,
            &stderr,
            no_input,
            ForwardStage::Service,
        ));
    }
    wait_for_standalone_ready(runtime, local_listeners)
}

fn wait_for_standalone_ready(
    runtime: &StandaloneRuntime,
    local_listeners: &[(String, u16)],
) -> Result<(), String> {
    let started = SystemTime::now();
    loop {
        if let Some(signal) = received_signal() {
            return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
        }
        if standalone_master_ready(runtime) {
            if local_listeners_ready(local_listeners) {
                return Ok(());
            }
            if started
                .elapsed()
                .unwrap_or_default()
                .ge(&FORWARD_READY_TIMEOUT)
            {
                return Err(
                    "SERVICE_BIND_FAILED: timed out waiting for requested tunnel listener"
                        .to_string(),
                );
            }
        }
        if started.elapsed().unwrap_or_default().ge(&MASTER_TIMEOUT) {
            return Err("SSH_AUTH_TIMEOUT: OpenSSH master did not become ready".to_string());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn local_listeners_ready(listeners: &[(String, u16)]) -> bool {
    listeners.iter().all(|(host, port)| {
        let address = if host == "localhost" {
            Some(std::net::SocketAddr::from((
                std::net::Ipv4Addr::LOCALHOST,
                *port,
            )))
        } else {
            host.parse::<std::net::IpAddr>()
                .map(|address| std::net::SocketAddr::new(address, *port))
                .or_else(|_| format!("{host}:{port}").parse())
                .ok()
        };
        address
            .and_then(|address| {
                TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()
            })
            .is_some()
    })
}
fn remote_forward_argument(forward: &session::ForwardSpec) -> Result<String, String> {
    let fields = split_forward_fields(&forward.effective);
    let [bind, port, _, _] = fields.as_slice() else {
        return Err("FORWARD_INVALID: remote forward has invalid effective value".to_string());
    };
    let host = forward.remote_host.as_deref().ok_or_else(|| {
        "FORWARD_INVALID: remote forward has no destination host".to_string()
    })?;
    let remote_port = forward.remote_port.ok_or_else(|| {
        "FORWARD_INVALID: remote forward has no destination port".to_string()
    })?;
    Ok(format!("{bind}:{port} {}:{remote_port}", forward_host(host)))
}
fn local_forward_argument(forward: &session::ForwardSpec) -> Result<String, String> {
    let (bind, port) = forward.listener().ok_or_else(|| {
        "FORWARD_INVALID: local forward requires a local listener".to_string()
    })?;
    let host = forward
        .remote_host
        .as_deref()
        .ok_or_else(|| "FORWARD_INVALID: local forward has no destination host".to_string())?;
    let remote_port = forward.remote_port.ok_or_else(|| {
        "FORWARD_INVALID: local forward has no destination port".to_string()
    })?;
    Ok(format!(
        "{}:{port} {}:{remote_port}",
        forward_host(&bind),
        forward_host(host)
    ))
}


fn split_forward_fields(value: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut bracketed = false;
    for (index, character) in value.char_indices() {
        match character {
            '[' => bracketed = true,
            ']' => bracketed = false,
            ':' if !bracketed => {
                fields.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(&value[start..]);
    fields
}

fn forward_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub(crate) fn standalone_master_ready(runtime: &StandaloneRuntime) -> bool {
    standalone_master_ready_at(runtime.control_socket(), runtime.alias())
}

pub(crate) fn standalone_master_ready_at(socket: &Path, alias: &str) -> bool {
    Command::new("ssh")
        .args([
            "-F",
            "none",
            "-S",
            socket.to_string_lossy().as_ref(),
            "-O",
            "check",
            alias,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn stop_standalone(runtime: &StandaloneRuntime) {
    stop_standalone_at(runtime.control_socket(), runtime.alias());
}

pub(crate) fn stop_standalone_at(socket: &Path, alias: &str) {
    let _ = Command::new("ssh")
        .args([
            "-F",
            "none",
            "-S",
            socket.to_string_lossy().as_ref(),
            "-O",
            "exit",
            alias,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
pub(crate) fn stop_standalone_checked_at(socket: &Path, alias: &str) -> Result<(), String> {
    let status = Command::new("ssh")
        .args([
            "-F",
            "none",
            "-S",
            socket.to_string_lossy().as_ref(),
            "-O",
            "exit",
            alias,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("cannot stop master: {error}"))?;
    if !status.success() {
        return Err(format!("control exit returned {status}"));
    }
    if standalone_master_ready_at(socket, alias) {
        return Err("master remains responsive after control exit".to_string());
    }
    Ok(())
}

fn spawn_shell(runtime: &Runtime, no_input: bool, terminal: bool) -> Result<Child, String> {
    let mut command = ssh_command(runtime, no_input, true);
    command.args(["-o", "ControlMaster=no"]);
    command.arg(&runtime.alias);
    // The TUI uses stderr as its terminal even when stdout is redirected or closed.
    command
        .stdin(Stdio::inherit())
        .stdout(if terminal {
            Stdio::from(io::stderr())
        } else {
            Stdio::inherit()
        })
        .stderr(Stdio::inherit());
    command
        .spawn()
        .map_err(|error| format!("SESSION_FAILED: cannot start shell: {error}"))
}

fn enroll_host_key(runtime: &Runtime) -> Result<(), String> {
    let known_hosts = effective_known_hosts(runtime)?;
    let before = known_hosts_snapshot(&known_hosts)?;
    let mut command = ssh_command(runtime, false, false);
    command.args([
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "PreferredAuthentications=none",
        "-o",
        "PubkeyAuthentication=no",
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        "NumberOfPasswordPrompts=0",
        "-o",
        "RequestTTY=force",
        "-o",
        "ConnectTimeout=10",
    ]);
    command.arg(&runtime.alias);
    let _ = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("HOST_KEY_TRUST_REQUIRED: cannot enroll host key: {error}"))?;
    verify_host_key_enrollment(&known_hosts, &before)
}

fn verify_host_key_enrollment(paths: &[PathBuf], before: &[Option<Vec<u8>>]) -> Result<(), String> {
    if known_hosts_snapshot(paths)? != before {
        Ok(())
    } else {
        Err("HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string())
    }
}
fn effective_known_hosts(runtime: &Runtime) -> Result<Vec<PathBuf>, String> {
    if !runtime.user_known_hosts_file_configured {
        return Ok(vec![runtime.known_hosts.clone()]);
    }
    let mut command = ssh_command(runtime, false, false);
    command.args(["-G", "-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    command.arg(&runtime.alias);
    let output = command.output().map_err(|error| {
        format!("HOST_KEY_TRUST_REQUIRED: cannot read effective SSH config: {error}")
    })?;
    if !output.status.success() {
        return Err("HOST_KEY_TRUST_REQUIRED: cannot read effective SSH config".to_string());
    }
    parse_effective_known_hosts(&String::from_utf8_lossy(&output.stdout))
}

fn parse_effective_known_hosts(text: &str) -> Result<Vec<PathBuf>, String> {
    let paths = text
        .lines()
        .find_map(|line| line.strip_prefix("userknownhostsfile "))
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter(|path| *path != "none")
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err(
            "HOST_KEY_TRUST_REQUIRED: effective SSH config has no known-hosts file".to_string(),
        );
    }
    Ok(paths)
}
fn known_hosts_snapshot(paths: &[PathBuf]) -> Result<Vec<Option<Vec<u8>>>, String> {
    paths
        .iter()
        .map(|path| match fs::read(path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "HOST_KEY_TRUST_REQUIRED: cannot inspect {}: {error}",
                path.display()
            )),
        })
        .collect()
}
#[cfg(unix)]
struct PasswordPipe {
    read: RawFd,
    writer: Option<File>,
    password: Vec<u8>,
}

#[cfg(unix)]
impl PasswordPipe {
    fn new(password: &[u8]) -> Result<Self, String> {
        let mut descriptors = [0; 2];
        if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
            return Err(format!(
                "SSH_AUTH_FAILED: cannot create password pipe: {}",
                io::Error::last_os_error()
            ));
        }
        let read = descriptors[0];
        let write = descriptors[1];
        if let Err(error) = set_cloexec(read).and_then(|_| set_cloexec(write)) {
            unsafe {
                libc::close(read);
                libc::close(write);
            }
            return Err(format!(
                "SSH_AUTH_FAILED: cannot protect password pipe: {error}"
            ));
        }
        Ok(Self {
            read,
            writer: Some(unsafe { File::from_raw_fd(write) }),
            password: password.to_vec(),
        })
    }

    fn send(mut self) -> Result<(), String> {
        let mut writer = self
            .writer
            .take()
            .expect("password pipe writer should exist");
        writer
            .write_all(&self.password)
            .and_then(|_| writer.write_all(b"\n"))
            .map_err(|error| format!("SSH_AUTH_FAILED: cannot write password pipe: {error}"))
    }
}

#[cfg(unix)]
impl Drop for PasswordPipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read);
        }
    }
}

#[cfg(unix)]
fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn auth_command(
    runtime: &Runtime,
    no_input: bool,
    strict: bool,
    attempt: Option<&PasswordAttempt>,
) -> Result<(Command, Option<PasswordPipe>), String> {
    let Some(attempt) = attempt else {
        return Ok((ssh_command(runtime, no_input, strict), None));
    };
    let pipe = PasswordPipe::new(&attempt.bytes)?;
    let fd = pipe.read;
    let mut command = Command::new("sshpass");
    command.arg("-d").arg(fd.to_string()).arg("ssh");
    append_ssh_args(&mut command, runtime, no_input, strict, true);
    unsafe {
        command.pre_exec(move || clear_cloexec(fd));
    }
    Ok((command, Some(pipe)))
}

#[cfg(not(unix))]
fn auth_command(
    runtime: &Runtime,
    no_input: bool,
    strict: bool,
    attempt: Option<&PasswordAttempt>,
) -> Result<(Command, Option<()>), String> {
    if attempt.is_some() {
        return Err(
            "SSH_AUTH_FAILED: password authentication requires Unix file descriptors".to_string(),
        );
    }
    Ok((ssh_command(runtime, no_input, strict), None))
}

fn ssh_command(runtime: &Runtime, no_input: bool, strict: bool) -> Command {
    let mut command = Command::new("ssh");
    append_ssh_args(&mut command, runtime, no_input, strict, false);
    command
}

fn append_ssh_args(
    command: &mut Command,
    runtime: &Runtime,
    no_input: bool,
    strict: bool,
    password: bool,
) {
    command.args(["-F", runtime.config.to_string_lossy().as_ref()]);
    command.args(["-S", runtime.socket.to_string_lossy().as_ref()]);
    let known_hosts = format!("UserKnownHostsFile={}", runtime.known_hosts.display());
    command.args([
        "-o",
        "ControlPersist=no",
        "-o",
        if password || !no_input {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        },
    ]);
    if !runtime.strict_host_key_checking_configured {
        command.args([
            "-o",
            if strict {
                "StrictHostKeyChecking=yes"
            } else {
                "StrictHostKeyChecking=ask"
            },
        ]);
    }
    if !runtime.user_known_hosts_file_configured {
        command.args(["-o", known_hosts.as_str()]);
    }
    command.args([
        "-o",
        if password {
            "PreferredAuthentications=password"
        } else {
            "PreferredAuthentications=publickey"
        },
        "-o",
        if password {
            "PasswordAuthentication=yes"
        } else {
            "PasswordAuthentication=no"
        },
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        if password {
            "NumberOfPasswordPrompts=1"
        } else {
            "NumberOfPasswordPrompts=0"
        },
    ]);
}

fn wait_for_master(
    runtime: &Runtime,
    master: &mut Child,
    no_input: bool,
    forward_stage: ForwardStage,
) -> Result<(), String> {
    let started = SystemTime::now();
    loop {
        if let Some(signal) = received_signal() {
            return Err(format!("SESSION_START_INTERRUPTED: signal {signal}"));
        }
        if let Some(status) = master
            .try_wait()
            .map_err(|error| format!("SSH_AUTH_FAILED: cannot inspect OpenSSH: {error}"))?
        {
            let stderr = read_child_stderr(master);
            if !no_input && !stderr.is_empty() {
                eprint!("{stderr}");
            }
            return Err(classify_master_failure(
                status,
                &stderr,
                no_input,
                forward_stage,
            ));
        }
        if control_master_ready(runtime) {
            if matches!(forward_stage, ForwardStage::Service) && !service_forwards_ready(runtime) {
                if started
                    .elapsed()
                    .unwrap_or_default()
                    .ge(&FORWARD_READY_TIMEOUT)
                {
                    return Err(
                        "SERVICE_BIND_FAILED: timed out waiting for requested local listener"
                            .to_string(),
                    );
                }
            } else {
                return Ok(());
            }
        }
        if started.elapsed().unwrap_or_default().ge(&MASTER_TIMEOUT) {
            return Err(
                "SSH_AUTH_TIMEOUT: OpenSSH authentication did not become ready".to_string(),
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn service_forwards_ready(runtime: &Runtime) -> bool {
    runtime.forwards.iter().all(|forward| {
        TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], forward.local_port)),
            Duration::from_millis(100),
        )
        .is_ok()
    }) && local_listeners_ready(&runtime.extra_listeners)
}

fn wait_for_shell(shell: &mut Child) -> Result<ExitStatus, String> {
    loop {
        if received_signal().is_some() {
            let _ = shell.kill();
            let _ = shell.wait();
            return shell
                .try_wait()
                .map_err(|error| format!("SESSION_FAILED: cannot inspect shell: {error}"))?
                .ok_or_else(|| "SESSION_INTERRUPTED: shell did not stop".to_string());
        }
        if let Some(status) = shell
            .try_wait()
            .map_err(|error| format!("SESSION_FAILED: cannot inspect shell: {error}"))?
        {
            return Ok(status);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn control_master_ready(runtime: &Runtime) -> bool {
    let status = Command::new("ssh")
        .args(["-F", runtime.config.to_string_lossy().as_ref()])
        .args(["-S", runtime.socket.to_string_lossy().as_ref()])
        .args(["-O", "check", &runtime.alias])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    status.is_ok_and(|status| status.success())
}

fn stop_master(runtime: &Runtime, master: &mut Child) {
    let _ = Command::new("ssh")
        .args(["-F", runtime.config.to_string_lossy().as_ref()])
        .args(["-S", runtime.socket.to_string_lossy().as_ref()])
        .args(["-O", "exit", &runtime.alias])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if master.try_wait().ok().flatten().is_none() {
        let _ = master.kill();
    }
    let _ = master.wait();
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut output = String::new();
    let _ = stderr.read_to_string(&mut output);
    output
}

fn classify_master_failure(
    status: ExitStatus,
    stderr: &str,
    no_input: bool,
    forward_stage: ForwardStage,
) -> String {
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || stderr.contains("Offending ")
        || stderr.contains("host key for .* has changed")
    {
        return "HOST_KEY_CHANGED: remote host key changed; refusing replacement".to_string();
    }
    if stderr.contains("Host key verification failed")
        || stderr.contains("The authenticity of host")
        || stderr.contains("Are you sure you want to continue connecting")
    {
        return "HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string();
    }
    if no_input && stderr.to_ascii_lowercase().contains("host key") {
        return "HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string();
    }
    let detail = stderr.lines().last().unwrap_or_default().trim();
    if matches!(forward_stage, ForwardStage::Transit)
        && (stderr.to_ascii_lowercase().contains("forward")
            || stderr.to_ascii_lowercase().contains("bind"))
    {
        return if detail.is_empty() {
            "TRANSIT_BIND_FAILED: gateway transit forward failed".to_string()
        } else {
            format!("TRANSIT_BIND_FAILED: {detail}")
        };
    }
    if matches!(forward_stage, ForwardStage::Service)
        && (stderr.to_ascii_lowercase().contains("forward")
            || stderr.to_ascii_lowercase().contains("bind"))
    {
        return if detail.is_empty() {
            "SERVICE_BIND_FAILED: requested service forward failed".to_string()
        } else {
            format!("SERVICE_BIND_FAILED: {detail}")
        };
    }
    if detail.is_empty() {
        format!("SSH_AUTH_FAILED: OpenSSH exited with {status}")
    } else {
        format!("SSH_AUTH_FAILED: {detail}")
    }
}
pub fn stored_password(entry: &HostEntry, no_input: bool) -> Result<Option<String>, String> {
    selected_password(entry, no_input)
}

pub fn has_stored_password(entry: &HostEntry) -> Result<bool, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
    Ok(extract_password(&bytes, entry)?.is_some())
}

/// Repair an unreadable registered config root once before discovery retries.
pub fn repair_discovery_permissions(path: &Path, no_input: bool) -> Result<bool, String> {
    let current_mode = permissions::assess(path, permissions::PermissionTarget::File)
        .map_err(|reason| format!("PERMISSION_REPAIR_UNSAFE: {} ({reason})", path.display()))?;
    if current_mode == permissions::PRIVATE_FILE_MODE {
        return Ok(false);
    }
    let candidate = permissions::RepairCandidate {
        kind: "config_root".to_string(),
        path: path.to_path_buf(),
        target: permissions::PermissionTarget::File,
        current_mode,
        reason: format!(
            "current mode {:o} requires {:o}",
            current_mode,
            permissions::PRIVATE_FILE_MODE
        ),
    };
    repair_permission_candidate(&candidate, no_input)?;
    Ok(true)
}

fn repair_permission_candidate(
    candidate: &permissions::RepairCandidate,
    no_input: bool,
) -> Result<(), String> {
    if no_input || !io::stdin().is_terminal() {
        return Err(permission_manual_error(
            "PERMISSION_REPAIR_REQUIRED",
            candidate,
            "interactive confirmation unavailable",
        ));
    }
    eprint!(
        "Permission repair required\n  path: {}\n  current mode: {:o}\n  required mode: {:o}\n  reason: {}\nApply permission repair? [y/N]: ",
        candidate.path.display(),
        candidate.current_mode,
        candidate.target.private_mode(),
        candidate.reason
    );
    io::stderr()
        .flush()
        .map_err(|error| format!("PERMISSION_REPAIR_PROMPT_FAILED: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("PERMISSION_REPAIR_PROMPT_FAILED: {error}"))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(permission_manual_error(
            "PERMISSION_REPAIR_DECLINED",
            candidate,
            "confirmation declined",
        ));
    }
    let result = permissions::apply(candidate);
    if result.outcome == permissions::RepairOutcome::Fixed {
        Ok(())
    } else {
        Err(permission_manual_error(
            "PERMISSION_REPAIR_FAILED",
            candidate,
            &format!("{}: {}", result.outcome, result.detail),
        ))
    }
}

fn selected_password(entry: &HostEntry, no_input: bool) -> Result<Option<String>, String> {
    let mut repaired = false;
    loop {
        let bytes = fs::read(&entry.source.path).map_err(|error| {
            format!(
                "CONFIG_READ_FAILED: cannot read {}: {error}",
                entry.source.path
            )
        })?;
        let Some(password) = extract_password(&bytes, entry)? else {
            return Ok(None);
        };
        let current_mode = match permissions::assess(
            Path::new(&entry.source.path),
            permissions::PermissionTarget::File,
        ) {
            Ok(mode) => mode,
            Err(reason) => {
                return Err(format!(
                    "PERMISSION_REPAIR_UNSAFE: {} ({reason})",
                    entry.source.path
                ));
            }
        };
        if current_mode == permissions::PRIVATE_FILE_MODE {
            return Ok(Some(password));
        }
        if repaired {
            let candidate = permissions::RepairCandidate {
                kind: "password_file".to_string(),
                path: PathBuf::from(&entry.source.path),
                target: permissions::PermissionTarget::File,
                current_mode,
                reason: format!(
                    "current mode {:o} requires {:o}",
                    current_mode,
                    permissions::PRIVATE_FILE_MODE
                ),
            };
            return Err(permission_manual_error(
                "PERMISSION_REPAIR_FAILED",
                &candidate,
                "retry still has unsafe permissions",
            ));
        }
        let candidate = permissions::RepairCandidate {
            kind: "password_file".to_string(),
            path: PathBuf::from(&entry.source.path),
            target: permissions::PermissionTarget::File,
            current_mode,
            reason: format!(
                "current mode {:o} requires {:o}",
                current_mode,
                permissions::PRIVATE_FILE_MODE
            ),
        };
        repair_permission_candidate(&candidate, no_input)?;
        repaired = true;
    }
}

fn extract_password(bytes: &[u8], entry: &HostEntry) -> Result<Option<String>, String> {
    let end = entry.source.byte_end;
    if entry.source.byte_start >= end || end > bytes.len() {
        return Err("CONFIG_INVALID: selected Host span is outside source file".to_string());
    }
    let block = &bytes[entry.source.byte_start..end];
    for raw in block.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let start = raw
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(raw.len());
        let line = &raw[start..];
        if line.len() < 10
            || !line[..10].eq_ignore_ascii_case(b"##PASSWORD")
            || line.get(10).is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            continue;
        }
        let value = line[10..]
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .map_or(&[][..], |offset| &line[10 + offset..]);
        if value.is_empty() {
            return Ok(None);
        }
        let value = std::str::from_utf8(value)
            .map_err(|_| "CONFIG_INVALID: password metadata is not valid UTF-8".to_string())?;
        return Ok(Some(value.to_string()));
    }
    Ok(None)
}
fn permission_manual_error(
    prefix: &str,
    candidate: &permissions::RepairCandidate,
    detail: &str,
) -> String {
    format!(
        "{prefix}: {} ({detail}; {}); manual repair: {}",
        candidate.path.display(),
        candidate.reason,
        permissions::manual_chmod_command(&candidate.path, candidate.target)
    )
}

#[cfg(unix)]
fn read_password_fd(fd: i32) -> Result<PasswordAttempt, String> {
    if fd < 0 {
        return Err("PASSWORD_FD_INVALID: file descriptor must be non-negative".to_string());
    }
    let duplicate = unsafe { libc::dup(fd) };
    if duplicate < 0 {
        return Err(format!(
            "PASSWORD_FD_INVALID: cannot read file descriptor {fd}: {}",
            io::Error::last_os_error()
        ));
    }
    set_cloexec(fd).map_err(|error| {
        unsafe {
            libc::close(duplicate);
        }
        format!("PASSWORD_FD_INVALID: cannot protect file descriptor {fd}: {error}")
    })?;
    let mut file = unsafe { File::from_raw_fd(duplicate) };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| {
        format!("PASSWORD_FD_INVALID: cannot read file descriptor {fd}: {error}")
    })?;
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.iter().any(|byte| matches!(*byte, b'\r' | b'\n' | 0)) {
        return Err("PASSWORD_FD_INVALID: password must contain one non-empty line".to_string());
    }
    Ok(PasswordAttempt::supplied(bytes))
}

#[cfg(not(unix))]
fn read_password_fd(_fd: i32) -> Result<PasswordAttempt, String> {
    Err("PASSWORD_FD_INVALID: explicit password descriptors require Unix".to_string())
}

fn prompt_password(alias: &str, replacement: bool) -> Result<Option<PasswordAttempt>, String> {
    prompt_password_for("direct host", alias, replacement)
}

fn prompt_password_for(
    role: &str,
    alias: &str,
    replacement: bool,
) -> Result<Option<PasswordAttempt>, String> {
    if !io::stdin().is_terminal() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        use std::os::fd::AsRawFd;

        let label = if replacement {
            format!("Password for {role} `{alias}` (replacement): ")
        } else {
            format!("Password for {role} `{alias}`: ")
        };
        eprint!("{label}");
        io::stderr()
            .flush()
            .map_err(|error| format!("PASSWORD_PROMPT_FAILED: cannot flush prompt: {error}"))?;
        let fd = io::stdin().as_raw_fd();
        let mut original = MaybeUninit::uninit();
        let original = unsafe {
            if libc::tcgetattr(fd, original.as_mut_ptr()) != 0 {
                return Err(
                    "PASSWORD_PROMPT_FAILED: interactive input is not a terminal".to_string(),
                );
            }
            original.assume_init()
        };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
            return Err("PASSWORD_PROMPT_FAILED: cannot hide password input".to_string());
        }
        let mut value = String::new();
        let read_result = io::stdin().read_line(&mut value);
        let restore_result = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
        eprintln!();
        if read_result.is_err() || restore_result != 0 {
            return Err("PASSWORD_PROMPT_FAILED: cannot read hidden password".to_string());
        }
        let value = value.trim_end_matches(['\r', '\n']).to_string();
        Ok((!value.is_empty()).then(|| PasswordAttempt::prompted(value)))
    }
    #[cfg(not(unix))]
    {
        let _ = (role, alias, replacement);
        Err("PASSWORD_PROMPT_FAILED: hidden password input is unsupported".to_string())
    }
}

fn save_replacement(
    runtime: &Runtime,
    attempt: &PasswordAttempt,
    no_input: bool,
) -> Result<(), String> {
    save_replacement_for(runtime, attempt, no_input, "direct host")
}

fn save_replacement_for(
    runtime: &Runtime,
    attempt: &PasswordAttempt,
    no_input: bool,
    role: &str,
) -> Result<(), String> {
    let Some(password) = attempt.replacement.as_deref() else {
        return Ok(());
    };
    if no_input || !io::stdin().is_terminal() {
        return Ok(());
    }
    eprint!(
        "Save replacement password for {role} `{}`? [y/N]: ",
        runtime.alias
    );
    io::stderr()
        .flush()
        .map_err(|error| format!("PASSWORD_PROMPT_FAILED: cannot flush prompt: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("PASSWORD_PROMPT_FAILED: cannot read consent: {error}"))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(());
    }
    mutation::validate_entry_paths(&runtime.entry)?;
    let plan = mutation::plan_update(&UpdateRequest {
        path: PathBuf::from(&runtime.entry.source.path),
        expected_id: runtime.entry.id.clone(),
        selected_alias: runtime.alias.clone(),
        byte_start: runtime.entry.source.byte_start,
        byte_end: runtime.entry.source.byte_end,
        alias: None,
        hostname: None,
        user: None,
        port: None,
        password: Some(password.to_string()),
        clear_user: false,
        clear_port: false,
        clear_password: false,
    })?;
    mutation::apply_edit(&plan)
}

#[derive(Clone, Copy)]
enum RuntimeTransform<'a> {
    None,
    Standalone,
    Service {
        forwards: &'a [ServiceForward],
    },
    Gateway {
        transit_host: &'a str,
        transit_port: u16,
        local_port: u16,
    },
    Vm {
        local_port: u16,
        host_key_alias: &'a str,
        forwards: &'a [ServiceForward],
    },
}
pub(crate) fn compile_config(entry: &HostEntry, selected_alias: &str) -> Result<String, String> {
    compile_config_with_transform(entry, selected_alias, RuntimeTransform::None)
}

fn compile_standalone_config(entry: &HostEntry, selected_alias: &str) -> Result<String, String> {
    compile_config_with_transform(entry, selected_alias, RuntimeTransform::Standalone)
}

fn compile_config_with_forwards(
    entry: &HostEntry,
    selected_alias: &str,
    forwards: &[ServiceForward],
) -> Result<String, String> {
    compile_config_with_transform(
        entry,
        selected_alias,
        RuntimeTransform::Service { forwards },
    )
}

fn compile_gateway_config(
    entry: &HostEntry,
    selected_alias: &str,
    transit_host: &str,
    transit_port: u16,
    local_port: u16,
) -> Result<String, String> {
    compile_config_with_transform(
        entry,
        selected_alias,
        RuntimeTransform::Gateway {
            transit_host,
            transit_port,
            local_port,
        },
    )
}

fn compile_vm_config(
    entry: &HostEntry,
    selected_alias: &str,
    local_port: u16,
    host_key_alias: &str,
    forwards: &[ServiceForward],
) -> Result<String, String> {
    compile_config_with_transform(
        entry,
        selected_alias,
        RuntimeTransform::Vm {
            local_port,
            host_key_alias,
            forwards,
        },
    )
}

fn append_service_forwards(output: &mut String, forwards: &[ServiceForward]) {
    for forward in forwards {
        output.push_str("  LocalForward 127.0.0.1:");
        output.push_str(&forward.local_port.to_string());
        output.push(' ');
        if forward.destination_host.contains(':') && !forward.destination_host.starts_with('[') {
            output.push('[');
            output.push_str(&forward.destination_host);
            output.push(']');
        } else {
            output.push_str(&forward.destination_host);
        }
        output.push(':');
        output.push_str(&forward.remote_port.to_string());
        output.push('\n');
    }
}
fn compile_config_with_transform(
    entry: &HostEntry,
    selected_alias: &str,
    transform: RuntimeTransform<'_>,
) -> Result<String, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "CONFIG_INVALID: SSH config is not valid UTF-8".to_string())?;
    let start = entry.source.byte_start;
    let end = entry.source.byte_end;
    if start >= end || end > text.len() {
        return Err("CONFIG_INVALID: selected Host span is outside source file".to_string());
    }
    validate_source(text, start, entry)?;
    let block = &text[start..end];
    let mut output = String::new();
    let mut host_written = false;
    let mut hostname_written = false;
    let mut port_written = false;
    let mut host_key_alias_written = false;
    let mut gateway_forward_matches = 0usize;
    for raw in block.lines() {
        let raw_line = raw.trim();
        let line = strip_inline_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let (keyword, mut argument) = split_directive(line)?;
        if keyword.eq_ignore_ascii_case("proxycommand") {
            argument = split_directive(raw_line)?.1;
        }
        if keyword.eq_ignore_ascii_case("host") {
            if host_written {
                return Err(
                    "CONFIG_INVALID: selected span contains multiple Host lines".to_string()
                );
            }
            let aliases = tokenize(argument);
            if aliases.is_empty() || aliases.iter().any(|alias| wildcard(alias)) {
                return Err(
                    "UNSUPPORTED_WILDCARD: selected Host block is not an exact direct host"
                        .to_string(),
                );
            }
            if !aliases.iter().any(|alias| alias == selected_alias) {
                return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
            }
            output.push_str("Host ");
            output.push_str(selected_alias);
            output.push('\n');
            host_written = true;
            continue;
        }
        if keyword.eq_ignore_ascii_case("include") {
            return Err("UNSUPPORTED_CONDITIONAL_INCLUDE: Include inside Host block".to_string());
        }
        if keyword.eq_ignore_ascii_case("match") {
            return Err(
                "UNSUPPORTED_MATCH: Match cannot be preserved in exact Host config".to_string(),
            );
        }
        let proxy_command = keyword.eq_ignore_ascii_case("proxycommand");
        let user_known_hosts_file = keyword.eq_ignore_ascii_case("userknownhostsfile");
        if proxy_command
            && matches!(
                transform,
                RuntimeTransform::Gateway { .. } | RuntimeTransform::Vm { .. }
            )
        {
            return Err(
                "PAIR_ROUTE_UNSAFE: ProxyCommand and ProxyJump are not allowed for paired routes"
                    .to_string(),
            );
        }
        if argument.contains('%') && !proxy_command && !user_known_hosts_file {
            return Err(format!(
                "UNSUPPORTED_TOKEN_SEMANTICS: {keyword} contains token expansion"
            ));
        }
        if !supported_directive(keyword) {
            return Err(format!(
                "UNSUPPORTED_DIRECTIVE: {keyword} cannot be preserved in direct runtime config"
            ));
        }
        if matches!(transform, RuntimeTransform::Standalone)
            && (keyword.eq_ignore_ascii_case("localforward")
                || keyword.eq_ignore_ascii_case("remoteforward"))
        {
            continue;
        }
        if keyword.eq_ignore_ascii_case("localforward")
            && let RuntimeTransform::Gateway {
                transit_host,
                transit_port,
                local_port,
            } = transform
            && local_forward_destination(argument).is_some_and(|destination| {
                destination.0 == transit_host && destination.1 == transit_port
            })
        {
            gateway_forward_matches += 1;
            output.push_str("  LocalForward 127.0.0.1:");
            output.push_str(&local_port.to_string());
            output.push(' ');
            output.push_str(argument.split_whitespace().last().unwrap_or_default());
            output.push('\n');
            continue;
        }
        if keyword.eq_ignore_ascii_case("hostname")
            && matches!(transform, RuntimeTransform::Vm { .. })
        {
            output.push_str("  HostName 127.0.0.1\n");
            hostname_written = true;
            continue;
        }
        if keyword.eq_ignore_ascii_case("port")
            && let RuntimeTransform::Vm { local_port, .. } = transform
        {
            output.push_str("  Port ");
            output.push_str(&local_port.to_string());
            output.push('\n');
            port_written = true;
            continue;
        }
        if keyword.eq_ignore_ascii_case("hostkeyalias")
            && let RuntimeTransform::Vm { host_key_alias, .. } = transform
        {
            output.push_str("  HostKeyAlias ");
            output.push_str(host_key_alias);
            output.push('\n');
            host_key_alias_written = true;
            continue;
        }
        output.push_str("  ");
        output.push_str(keyword);
        if !argument.is_empty() {
            output.push(' ');
            output.push_str(argument);
        }
        output.push('\n');
    }
    if !host_written {
        return Err("CONFIG_INVALID: selected span does not start with Host".to_string());
    }
    match transform {
        RuntimeTransform::Gateway { .. } if gateway_forward_matches != 1 => {
            return Err(format!(
                "PAIR_ROUTE_CHANGED: approved gateway transit has {gateway_forward_matches} current LocalForward candidates"
            ));
        }
        RuntimeTransform::Service { forwards } => append_service_forwards(&mut output, forwards),
        RuntimeTransform::Vm {
            local_port,
            host_key_alias,
            forwards,
        } => {
            if !hostname_written {
                output.push_str("  HostName 127.0.0.1\n");
            }
            if !port_written {
                output.push_str("  Port ");
                output.push_str(&local_port.to_string());
                output.push('\n');
            }
            if !host_key_alias_written {
                output.push_str("  HostKeyAlias ");
                output.push_str(host_key_alias);
                output.push('\n');
            }
            append_service_forwards(&mut output, forwards);
        }
        RuntimeTransform::None
        | RuntimeTransform::Standalone
        | RuntimeTransform::Gateway { .. } => {}
    }
    output.push_str("Include ");
    output.push_str(SYSTEM_CONFIG);
    output.push('\n');
    Ok(output)
}

fn local_forward_destination(argument: &str) -> Option<(String, u16)> {
    let destination = argument.split_whitespace().last()?;
    if let Some((host, port)) = destination.rsplit_once(':') {
        return Some((
            host.trim_matches(['[', ']']).to_string(),
            port.parse().ok()?,
        ));
    }
    None
}

fn allocate_transit_port() -> Result<u16, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("TRANSIT_BIND_FAILED: cannot allocate transit port: {error}"))?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("TRANSIT_BIND_FAILED: cannot inspect transit port: {error}"))
}

fn stable_vm_host_key_alias(vm_id: &str) -> String {
    format!("sshx-vm-{vm_id}")
}

fn validate_source(text: &str, start: usize, entry: &HostEntry) -> Result<(), String> {
    let mut offset = 0;
    let mut saw_host = false;
    let mut found_selected = false;
    for raw in text.split_inclusive('\n') {
        let line_start = offset;
        offset += raw.len();
        let line = strip_inline_comment(raw.trim_end_matches(['\n', '\r'])).trim();
        if line.is_empty() {
            continue;
        }
        let (keyword, argument) = split_directive(line)?;
        if keyword.eq_ignore_ascii_case("match") {
            return Err(
                "UNSUPPORTED_MATCH: Match cannot be preserved in exact Host config".to_string(),
            );
        }
        if keyword.eq_ignore_ascii_case("host") {
            saw_host = true;
            let aliases = tokenize(argument);
            if aliases.iter().any(|alias| wildcard(alias)) {
                return Err(
                    "UNSUPPORTED_WILDCARD: wildcard Host cannot be preserved exactly".to_string(),
                );
            }
            if aliases.iter().any(|alias| alias.contains('%')) {
                return Err(
                    "UNSUPPORTED_TOKEN_SEMANTICS: Host alias contains token expansion".to_string(),
                );
            }
            if line_start == start {
                found_selected = true;
                if aliases != entry.aliases {
                    return Err("CONFIG_CHANGED: selected Host block changed on disk".to_string());
                }
            }
            continue;
        }
        if keyword.eq_ignore_ascii_case("include") {
            if saw_host {
                return Err(
                    "UNSUPPORTED_CONDITIONAL_INCLUDE: Include inside Host block".to_string()
                );
            }
            continue;
        }
        if !saw_host {
            return Err(format!(
                "UNSUPPORTED_GLOBAL: {keyword} before selected Host block"
            ));
        }
    }
    if !found_selected {
        return Err("CONFIG_CHANGED: selected Host block no longer exists".to_string());
    }
    Ok(())
}

fn supported_directive(keyword: &str) -> bool {
    matches!(
        keyword.to_ascii_lowercase().as_str(),
        "addkeystoagent"
            | "addressfamily"
            | "batchmode"
            | "bindaddress"
            | "bindinterface"
            | "canonicaldomains"
            | "canonicalizemaxdots"
            | "canonicalizehostname"
            | "canonicalizefallbacklocal"
            | "certificatefile"
            | "ciphers"
            | "compression"
            | "connecttimeout"
            | "connectionattempts"
            | "disableforwarding"
            | "enableescapecommandline"
            | "escapechar"
            | "exitonforwardfailure"
            | "fingerprinthash"
            | "forwardagent"
            | "forwardx11"
            | "forwardx11timeout"
            | "forwardx11trusted"
            | "gatewayports"
            | "gssapiauthentication"
            | "gssapicleanupcredentials"
            | "gssapiclientidentity"
            | "gssapidelegatecredentials"
            | "gssapikeyexchange"
            | "gssapikexalgorithms"
            | "gssapirenewalforcesrekey"
            | "gssapitrustdns"
            | "hostkeyalgorithms"
            | "hostkeyalias"
            | "hostname"
            | "identitiesonly"
            | "identityagent"
            | "identityfile"
            | "usekeychain"
            | "userknownhostsfile"
            | "ignoreunknown"
            | "ipqos"
            | "kbdinteractiveauthentication"
            | "kexalgorithms"
            | "localforward"
            | "loglevel"
            | "macs"
            | "nohostauthenticationforlocalhost"
            | "numberofpasswordprompts"
            | "passwordauthentication"
            | "pkcs11provider"
            | "port"
            | "preferredauthentications"
            | "proxycommand"
            | "pubkeyacceptedalgorithms"
            | "pubkeyauthentication"
            | "rekeylimit"
            | "remoteforward"
            | "requesttty"
            | "requiredrsasize"
            | "revokedhostkeys"
            | "sendenv"
            | "serveralivecountmax"
            | "serveraliveinterval"
            | "sessiontype"
            | "setenv"
            | "streamlocalbindunlink"
            | "stricthostkeychecking"
            | "syslogfacility"
            | "tcpkeepalive"
            | "tunnel"
            | "tunneldevice"
            | "updatehostkeys"
            | "user"
            | "verifyhostkeydns"
            | "versionaddendum"
            | "visualhostkey"
            | "xauthlocation"
    )
}

fn split_directive(line: &str) -> Result<(&str, &str), String> {
    let line = line.trim();
    let split = line
        .find(char::is_whitespace)
        .or_else(|| line.find('='))
        .unwrap_or(line.len());
    let keyword = &line[..split];
    if keyword.is_empty() {
        return Err("CONFIG_INVALID: empty SSH directive".to_string());
    }
    let argument = line[split..]
        .trim_start_matches(char::is_whitespace)
        .trim_start_matches('=');
    Ok((keyword, argument.trim()))
}

fn strip_inline_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character == '#' {
            return &line[..index];
        }
    }
    line
}

fn tokenize(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(character),
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn wildcard(value: &str) -> bool {
    value.starts_with('!')
        || value
            .chars()
            .any(|character| matches!(character, '*' | '?' | '['))
}

fn temporary_directory() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("SESSION_FAILED: system clock error: {error}"))?
        .as_nanos();
    let directory = PathBuf::from("/tmp").join(format!("sshx-{}-{nanos}", std::process::id()));
    fs::create_dir(&directory)
        .map_err(|error| format!("SESSION_FAILED: cannot create runtime directory: {error}"))?;
    set_mode(&directory, 0o700)?;
    Ok(directory)
}

fn known_hosts_path(home: &Path, entry: &HostEntry) -> PathBuf {
    if entry.scopes.iter().any(|scope| scope == "work")
        && !entry.scopes.iter().any(|scope| scope == "personal")
    {
        home.join(".config/sshx/known_hosts/work")
    } else {
        home.join(".ssh/known_hosts")
    }
}

fn ensure_known_hosts_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "SESSION_FAILED: known-hosts path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("SESSION_FAILED: cannot create host-key directory: {error}"))?;
    set_mode(parent, 0o700)?;
    if !path.exists() {
        write_private_file(path, b"")?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| format!("SESSION_FAILED: cannot create {}: {error}", path.display()))?;
    use std::io::Write;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("SESSION_FAILED: cannot write {}: {error}", path.display()))?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
        format!(
            "SESSION_FAILED: cannot set permissions on {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Runtime, compile_config, compile_config_with_forwards, effective_known_hosts,
        known_hosts_path, paired_stage_error, service_forwards_ready,
    };
    use crate::discovery::{HostEntry, Provenance, SourceIdentity};
    use crate::session::ServiceForward;
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn entry(path: PathBuf, aliases: &[&str]) -> HostEntry {
        HostEntry {
            id: "entry".to_string(),
            aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
            source: SourceIdentity {
                path: path.to_string_lossy().into_owned(),
                byte_start: 0,
                byte_end: 0,
                line_start: 1,
                line_end: 2,
            },
            destination: Some("example.test".to_string()),
            provenance: vec![Provenance {
                paths: vec![path.to_string_lossy().into_owned()],
                scope: "personal".to_string(),
                project: None,
            }],
            scopes: vec!["personal".to_string()],
            projects: Vec::new(),
        }
    }

    #[test]
    fn runtime_config_drops_comments_and_appends_system_defaults() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = "##SSHX ID=secret\nHost exact\n  HostName example.test # comment\n  IdentityFile ~/.ssh/id_ed25519\n";
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["exact"]);
        selected.source.byte_start = text.find("Host exact").expect("Host should exist");
        selected.source.byte_end = text.len();
        let compiled = compile_config(&selected, "exact").expect("config should compile");
        assert!(compiled.contains("Host exact"));
        assert!(compiled.contains("IdentityFile ~/.ssh/id_ed25519"));
        assert!(compiled.contains("Include /etc/ssh/ssh_config"));
        assert!(!compiled.contains("##SSHX"));
        assert!(!compiled.contains("comment"));
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn explicit_host_key_directives_are_not_overridden() {
        let directory = tempfile_directory();
        let source = directory.join("config");
        let mut selected = entry(source.clone(), &["exact"]);
        let content = concat!(
            "Host exact\n",
            "  HostName example.test\n",
            "  StrictHostKeyChecking no\n",
            "  UserKnownHostsFile /dev/null\n",
        )
        .to_string();
        fs::write(&source, &content).expect("source should be written");
        selected.source.byte_end = content.len();
        let runtime =
            Runtime::create_with_content(&selected, &directory, "exact", content, &[], false)
                .expect("runtime should preserve host-key directives");
        assert!(runtime.strict_host_key_checking_configured);
        assert!(runtime.user_known_hosts_file_configured);
        assert!(!runtime.known_hosts.exists());
        let arguments = super::ssh_command(&runtime, false, true)
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!arguments.iter().any(|argument| {
            argument == "StrictHostKeyChecking=yes" || argument == "StrictHostKeyChecking=ask"
        }));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("UserKnownHostsFile="))
        );
        for option in [
            "NoHostAuthenticationForLocalhost=no",
            "CheckHostIP=no",
            "GlobalKnownHostsFile=/dev/null",
            "KnownHostsCommand=none",
            "VerifyHostKeyDNS=no",
            "UpdateHostKeys=no",
        ] {
            assert!(
                !arguments.iter().any(|argument| argument == option),
                "{option}"
            );
        }
        drop(runtime);
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn runtime_config_adds_loopback_service_forwards_without_metadata_or_secrets() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = concat!(
            "##SSHX ID=secret\n",
            "Host exact\n",
            "  HostName example.test\n",
            "  ##PORT 5432\n",
            "  ##SSHX SERVICE 5432 HOST=db.internal LOCAL=15432\n",
            "  ##PASSWORD never-copy-this\n",
        );
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["exact"]);
        selected.source.byte_start = text.find("Host exact").expect("host should exist");
        selected.source.byte_end = text.len();
        let forwards = [crate::session::ServiceForward {
            id: "5432#1".to_string(),
            remote_port: 5432,
            destination_host: "db.internal".to_string(),
            local_port: 15432,
        }];
        let compiled = compile_config_with_forwards(&selected, "exact", &forwards)
            .expect("service config should compile");
        assert!(compiled.contains("LocalForward 127.0.0.1:15432 db.internal:5432"));
        assert!(!compiled.contains("##SSHX"));
        assert!(!compiled.contains("##PORT"));
        assert!(!compiled.contains("never-copy-this"));
        assert_eq!(
            paired_stage_error(
                "VM",
                "SERVICE_BIND_FAILED: requested service forward failed".to_string()
            ),
            "VM_SERVICE_BIND_FAILED: SERVICE_BIND_FAILED: requested service forward failed"
        );
        assert_eq!(
            paired_stage_error("VM", "SESSION_START_INTERRUPTED: signal 2".to_string()),
            "SESSION_START_INTERRUPTED: signal 2"
        );
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn remote_forward_uses_server_listener_and_user_side_destination() {
        let forward = crate::session::ForwardSpec {
            kind: "R".to_string(),
            requested: "127.0.0.1:15432:db.internal:5432".to_string(),
            effective: "127.0.0.1:15432:db.internal:5432".to_string(),
            bind_address: "127.0.0.1".to_string(),
            local_port: None,
            remote_host: Some("db.internal".to_string()),
            remote_port: Some(5432),
        };
        assert_eq!(
            super::remote_forward_argument(&forward).unwrap(),
            "127.0.0.1:15432 db.internal:5432"
        );
    }

    #[test]
    fn service_readiness_requires_each_local_listener() {
        let directory = tempfile_directory();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener should bind");
        let port = listener.local_addr().expect("listener address").port();
        let runtime = Runtime {
            dir: directory.clone(),
            config: directory.join("config"),
            socket: directory.join("master.sock"),
            alias: "exact".to_string(),
            known_hosts: directory.join("known_hosts"),
            user_known_hosts_file_configured: false,
            strict_host_key_checking_configured: false,
            entry: entry(directory.join("source"), &["exact"]),
            configured_password: None,
            forwards: vec![ServiceForward {
                id: "5432#1".to_string(),
                remote_port: 5432,
                destination_host: "127.0.0.1".to_string(),
                local_port: port,
            }],
            extra_listeners: Vec::new(),
            preserve_dir: false,
        };
        assert!(service_forwards_ready(&runtime));
        drop(listener);
        assert!(!service_forwards_ready(&runtime));
        drop(runtime);
    }

    #[test]
    fn runtime_config_rejects_wildcards_and_tokens() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = "Host *.example\n  HostName %h\n";
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["*.example"]);
        selected.source.byte_end = text.len();
        let error = compile_config(&selected, "*.example").expect_err("unsafe config should fail");
        assert!(error.contains("UNSUPPORTED_WILDCARD"), "{error}");
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn work_scope_uses_separate_host_key_store() {
        let entry = HostEntry {
            scopes: vec!["work".to_string()],
            ..entry(PathBuf::from("/tmp/config"), &["work"])
        };
        assert!(known_hosts_path(Path::new("/home/test"), &entry).ends_with("known_hosts/work"));
    }

    fn tempfile_directory() -> PathBuf {
        let ordinal = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sshx-connect-test-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture directory should be created");
        path
    }

    #[test]
    fn pair_transform_compilation_rejects_proxycommand() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = "Host hop\n  HostName hop.example\n  ProxyCommand nc %h 22\n";
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["hop"]);
        selected.source.byte_start = text.find("Host hop").expect("Host should exist");
        selected.source.byte_end = text.len();
        let gateway = super::compile_gateway_config(&selected, "hop", "vm.internal", 22, 2200)
            .expect_err("gateway compilation should reject ProxyCommand");
        assert!(gateway.contains("PAIR_ROUTE_UNSAFE"), "{gateway}");
        let vm = super::compile_vm_config(&selected, "hop", 2200, "sshx-vm-test", &[])
            .expect_err("VM compilation should reject ProxyCommand");
        assert!(vm.contains("PAIR_ROUTE_UNSAFE"), "{vm}");
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn dynamic_listener_preflight_compares_full_addresses() {
        let probe = TcpListener::bind(("127.0.0.1", 0)).expect("port should be available");
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let dynamic = |bind: &str| crate::session::ForwardSpec {
            kind: "D".to_string(),
            requested: format!("{bind}:{port}"),
            effective: format!("{bind}:{port}"),
            bind_address: bind.to_string(),
            local_port: Some(port),
            remote_host: None,
            remote_port: None,
        };
        let service = ServiceForward {
            id: "service".to_string(),
            remote_port: 5432,
            destination_host: "db.internal".to_string(),
            local_port: port,
        };
        let local = crate::session::ForwardSpec {
            kind: "L".to_string(),
            requested: format!("127.0.0.1:{port}:db.internal:5432"),
            effective: format!("127.0.0.1:{port}:db.internal:5432"),
            bind_address: "127.0.0.1".to_string(),
            local_port: Some(port),
            remote_host: Some("db.internal".to_string()),
            remote_port: Some(5432),
        };
        assert!(super::preflight_direct_listeners(&[], std::slice::from_ref(&local)).is_ok());
        assert!(
            super::preflight_direct_listeners(
                std::slice::from_ref(&service),
                std::slice::from_ref(&local)
            )
            .unwrap_err()
            .starts_with("FORWARD_DUPLICATE:")
        );
        assert!(super::preflight_direct_listeners(
            std::slice::from_ref(&service),
            &[dynamic("::1")]
        )
        .is_ok());
        assert!(
            super::preflight_direct_listeners(&[service], &[dynamic("127.0.0.1")])
                .unwrap_err()
                .starts_with("FORWARD_DUPLICATE:")
        );
        assert!(
            super::preflight_direct_listeners(&[], &[dynamic("127.0.0.1"), dynamic("127.0.0.1")])
                .unwrap_err()
                .starts_with("FORWARD_DUPLICATE:")
        );
    }

    #[test]
    fn local_forward_runtime_preserves_specific_non_loopback_listener() {
        let directory = tempfile_directory();
        let source = directory.join("config");
        let text = "Host exact\n  HostName example.test\n";
        fs::write(&source, text).expect("source should be written");
        let mut selected = entry(source, &["exact"]);
        selected.source.byte_end = text.len();
        let forward = crate::session::ForwardSpec {
            kind: "L".to_string(),
            requested: "192.0.2.10:15432:db.internal:5432".to_string(),
            effective: "192.0.2.10:15432:db.internal:5432".to_string(),
            bind_address: "192.0.2.10".to_string(),
            local_port: Some(15432),
            remote_host: Some("db.internal".to_string()),
            remote_port: Some(5432),
        };
        let runtime = Runtime::create_with_direct_forwards(
            &selected,
            &directory,
            "exact",
            &[],
            &[forward],
            true,
        )
        .expect("specific listener should be preserved");
        let config =
            fs::read_to_string(&runtime.config).expect("runtime config should be readable");
        assert!(config.contains("LocalForward 192.0.2.10:15432 db.internal:5432"));
        drop(runtime);
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn host_enrollment_uses_effective_known_hosts_paths() {
        let paths = super::parse_effective_known_hosts(
            "host example.test\nuserknownhostsfile /tmp/custom_hosts /tmp/other_hosts\n",
        )
        .expect("OpenSSH should report known-hosts paths");
        assert_eq!(
            paths,
            [
                PathBuf::from("/tmp/custom_hosts"),
                PathBuf::from("/tmp/other_hosts")
            ]
        );
        assert!(
            super::parse_effective_known_hosts("userknownhostsfile none\n").is_err(),
            "none must not be treated as a writable host-key file"
        );
    }

    #[test]
    fn host_enrollment_verifies_configured_known_hosts_change() {
        let directory = tempfile_directory();
        let custom = directory.join("custom_known_hosts");
        let default = directory.join("known_hosts");
        fs::write(&custom, b"before\n").expect("custom file should be written");
        fs::write(&default, b"unchanged\n").expect("default file should be written");
        let configured = super::parse_effective_known_hosts(&format!(
            "userknownhostsfile {}\n",
            custom.display()
        ))
        .expect("configured known-hosts path should parse");
        let before = super::known_hosts_snapshot(&configured).expect("snapshot should read");
        assert!(
            super::verify_host_key_enrollment(&configured, &before).is_err(),
            "unchanged configured file must not satisfy consent"
        );
        fs::write(&custom, b"after\n").expect("custom file should change");
        super::verify_host_key_enrollment(&configured, &before)
            .expect("configured file change should satisfy consent");
        assert_eq!(
            fs::read(&default).expect("default runtime file should remain readable"),
            b"unchanged\n"
        );
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }
    #[test]
    fn host_enrollment_uses_runtime_store_without_host_override() {
        let directory = tempfile_directory();
        let known_hosts = directory.join("known_hosts");
        let runtime = Runtime {
            dir: directory.clone(),
            config: directory.join("config"),
            socket: directory.join("master.sock"),
            alias: "exact".to_string(),
            known_hosts: known_hosts.clone(),
            user_known_hosts_file_configured: false,
            strict_host_key_checking_configured: false,
            entry: entry(directory.join("source"), &["exact"]),
            configured_password: None,
            forwards: Vec::new(),
            extra_listeners: Vec::new(),
            preserve_dir: true,
        };
        assert_eq!(effective_known_hosts(&runtime).unwrap(), vec![known_hosts]);
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    use std::path::Path;
}
