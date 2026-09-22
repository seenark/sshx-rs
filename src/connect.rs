use crate::discovery::HostEntry;
use crate::mutation::{self, UpdateRequest};
use crate::pair::PairedRoute;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::net::TcpListener;
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
    entry: HostEntry,
    configured_password: Option<String>,
}

impl Runtime {
    fn create(entry: &HostEntry, home: &Path, selected_alias: &str) -> Result<Self, String> {
        let content = compile_config(entry, selected_alias)?;
        Self::create_with_content(entry, home, selected_alias, content)
    }

    fn create_with_content(
        entry: &HostEntry,
        home: &Path,
        selected_alias: &str,
        content: String,
    ) -> Result<Self, String> {
        if !entry.aliases.iter().any(|alias| alias == selected_alias) {
            return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
        }
        let configured_password = selected_password(entry)?;
        let dir = temporary_directory()?;
        let config = dir.join("config");
        let socket = dir.join("master.sock");
        let known_hosts = known_hosts_path(home, entry);
        ensure_known_hosts_parent(&known_hosts)?;
        write_private_file(&config, content.as_bytes())?;
        Ok(Self {
            dir,
            config,
            socket,
            alias: selected_alias.to_string(),
            known_hosts,
            entry: entry.clone(),
            configured_password,
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
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
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = open_session(entry, home, no_input, selected_alias, password_fd);
    reset_signal_handlers();
    result
}

fn open_session(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
    password_fd: Option<i32>,
) -> Result<(), String> {
    let runtime = Runtime::create(entry, home, selected_alias)?;
    let mut attempt = match password_fd {
        Some(fd) => Some(read_password_fd(fd)?),
        None => runtime
            .configured_password
            .clone()
            .map(PasswordAttempt::configured),
    };
    let mut enrolled = false;
    let master = loop {
        let mut master = spawn_master(&runtime, no_input, attempt.as_ref(), false)?;
        match wait_for_master(&runtime, &mut master, no_input) {
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
    let mut master = master;
    if let Some(attempt) = attempt.as_ref()
        && let Err(error) = save_replacement(&runtime, attempt, no_input)
    {
        stop_master(&runtime, &mut master);
        return Err(error);
    }
    let mut shell = match spawn_shell(&runtime, no_input) {
        Ok(shell) => shell,
        Err(error) => {
            stop_master(&runtime, &mut master);
            return Err(error);
        }
    };
    let status = match wait_for_shell(&mut shell) {
        Ok(status) => status,
        Err(error) => {
            stop_master(&runtime, &mut master);
            return Err(error);
        }
    };
    stop_master(&runtime, &mut master);
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
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = open_paired_session(
        route,
        home,
        no_input,
        gateway_alias,
        vm_alias,
        gateway_password_fd,
        vm_password_fd,
    );
    reset_signal_handlers();
    result
}

fn open_paired_session(
    route: &PairedRoute,
    home: &Path,
    no_input: bool,
    gateway_alias: &str,
    vm_alias: &str,
    gateway_password_fd: Option<i32>,
    vm_password_fd: Option<i32>,
) -> Result<(), String> {
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
    )?;
    let gateway_runtime =
        Runtime::create_with_content(&route.gateway, home, gateway_alias, gateway_config)?;
    let vm_runtime = Runtime::create_with_content(&route.vm, home, vm_alias, vm_config)?;
    let mut gateway_attempt = password_attempt(&gateway_runtime, gateway_password_fd)
        .map_err(|error| paired_stage_error("gateway", error))?;
    let mut vm_attempt = password_attempt(&vm_runtime, vm_password_fd)
        .map_err(|error| paired_stage_error("VM", error))?;

    let mut gateway_master = authenticate_paired_master(
        &gateway_runtime,
        no_input,
        &mut gateway_attempt,
        "gateway",
        true,
    )?;
    let mut vm_master =
        match authenticate_paired_master(&vm_runtime, no_input, &mut vm_attempt, "VM", false) {
            Ok(master) => master,
            Err(error) => {
                stop_master(&gateway_runtime, &mut gateway_master);
                return Err(error);
            }
        };

    if let Some(attempt) = gateway_attempt.as_ref()
        && let Err(error) = save_replacement_for(&gateway_runtime, attempt, no_input, "gateway")
    {
        stop_master(&vm_runtime, &mut vm_master);
        stop_master(&gateway_runtime, &mut gateway_master);
        return Err(error);
    }
    if let Some(attempt) = vm_attempt.as_ref()
        && let Err(error) = save_replacement_for(&vm_runtime, attempt, no_input, "VM")
    {
        stop_master(&vm_runtime, &mut vm_master);
        stop_master(&gateway_runtime, &mut gateway_master);
        return Err(error);
    }

    let mut shell = match spawn_shell(&vm_runtime, no_input) {
        Ok(shell) => shell,
        Err(error) => {
            stop_master(&vm_runtime, &mut vm_master);
            stop_master(&gateway_runtime, &mut gateway_master);
            return Err(format!("VM_SESSION_FAILED: {error}"));
        }
    };
    let status = match wait_for_shell(&mut shell) {
        Ok(status) => status,
        Err(error) => {
            stop_master(&vm_runtime, &mut vm_master);
            stop_master(&gateway_runtime, &mut gateway_master);
            return Err(error);
        }
    };
    stop_master(&vm_runtime, &mut vm_master);
    stop_master(&gateway_runtime, &mut gateway_master);
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
    require_forward: bool,
) -> Result<Child, String> {
    let mut enrolled = false;
    loop {
        let mut master = spawn_master(runtime, no_input, attempt.as_ref(), require_forward)
            .map_err(|error| paired_stage_error(role, error))?;
        match wait_for_master(runtime, &mut master, no_input) {
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
                    && attempt
                        .as_ref()
                        .is_none_or(|value| value.source != PasswordSource::Prompted);
                if can_prompt
                    && let Some(next) = prompt_password_for(role, &runtime.alias, attempt.is_some())
                        .map_err(|error| paired_stage_error(role, error))?
                {
                    *attempt = Some(next);
                    continue;
                }
                return Err(paired_stage_error(role, error));
            }
        }
    }
}

fn paired_stage_error(role: &str, error: String) -> String {
    if error.starts_with("SESSION_INTERRUPTED") {
        return error;
    }
    if role.eq_ignore_ascii_case("gateway") && (error.contains("forward") || error.contains("bind"))
    {
        return format!("TRANSIT_BIND_FAILED: gateway transit forward failed: {error}");
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

fn spawn_shell(runtime: &Runtime, no_input: bool) -> Result<Child, String> {
    let mut command = ssh_command(runtime, no_input, true);
    command.args(["-o", "ControlMaster=no"]);
    command.arg(&runtime.alias);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .map_err(|error| format!("SESSION_FAILED: cannot start shell: {error}"))
}

fn enroll_host_key(runtime: &Runtime) -> Result<(), String> {
    let before = fs::read(&runtime.known_hosts).unwrap_or_default();
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
    let after = fs::read(&runtime.known_hosts).unwrap_or_default();
    if after != before {
        Ok(())
    } else {
        Err("HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string())
    }
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
        "-o",
        if strict {
            "StrictHostKeyChecking=yes"
        } else {
            "StrictHostKeyChecking=ask"
        },
        "-o",
        "NoHostAuthenticationForLocalhost=no",
        "-o",
        "CheckHostIP=no",
        "-o",
        "GlobalKnownHostsFile=/dev/null",
        "-o",
        "KnownHostsCommand=none",
        "-o",
        "VerifyHostKeyDNS=no",
        "-o",
        "UpdateHostKeys=no",
        "-o",
        known_hosts.as_str(),
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

fn wait_for_master(runtime: &Runtime, master: &mut Child, no_input: bool) -> Result<(), String> {
    let started = SystemTime::now();
    loop {
        if let Some(signal) = received_signal() {
            return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
        }
        if let Some(status) = master
            .try_wait()
            .map_err(|error| format!("SSH_AUTH_FAILED: cannot inspect OpenSSH: {error}"))?
        {
            let stderr = read_child_stderr(master);
            if !no_input && !stderr.is_empty() {
                eprint!("{stderr}");
            }
            return Err(classify_master_failure(status, &stderr, no_input));
        }
        if control_master_ready(runtime) {
            return Ok(());
        }
        if started.elapsed().unwrap_or_default().ge(&MASTER_TIMEOUT) {
            return Err(
                "SSH_AUTH_TIMEOUT: OpenSSH authentication did not become ready".to_string(),
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
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

fn classify_master_failure(status: ExitStatus, stderr: &str, no_input: bool) -> String {
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
    if detail.is_empty() {
        format!("SSH_AUTH_FAILED: OpenSSH exited with {status}")
    } else {
        format!("SSH_AUTH_FAILED: {detail}")
    }
}
fn selected_password(entry: &HostEntry) -> Result<Option<String>, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
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
    Gateway {
        transit_host: &'a str,
        transit_port: u16,
        local_port: u16,
    },
    Vm {
        local_port: u16,
        host_key_alias: &'a str,
    },
}

fn compile_config(entry: &HostEntry, selected_alias: &str) -> Result<String, String> {
    compile_config_with_transform(entry, selected_alias, RuntimeTransform::None)
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
) -> Result<String, String> {
    compile_config_with_transform(
        entry,
        selected_alias,
        RuntimeTransform::Vm {
            local_port,
            host_key_alias,
        },
    )
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
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (keyword, argument) = split_directive(line)?;
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
        if argument.contains('%') {
            return Err(format!(
                "UNSUPPORTED_TOKEN_SEMANTICS: {keyword} contains token expansion"
            ));
        }
        if !supported_directive(keyword) {
            return Err(format!(
                "UNSUPPORTED_DIRECTIVE: {keyword} cannot be preserved in direct runtime config"
            ));
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
        RuntimeTransform::Vm {
            local_port,
            host_key_alias,
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
        }
        RuntimeTransform::None | RuntimeTransform::Gateway { .. } => {}
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
    use super::{compile_config, known_hosts_path};
    use crate::discovery::{HostEntry, Provenance, SourceIdentity};
    use std::fs;
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

    use std::path::Path;
}
