use crate::connect;
use crate::discovery::HostEntry;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VERSION: u8 = 1;
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(20);
static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ForwardSpec {
    pub kind: String,
    pub requested: String,
    pub effective: String,
    pub bind_address: String,
    pub local_port: Option<u16>,
    pub remote_host: Option<String>,
    pub remote_port: Option<u16>,
}

impl ForwardSpec {
    fn flag(&self) -> char {
        self.kind.chars().next().unwrap_or('L')
    }

    fn listener(&self) -> Option<(String, u16)> {
        self.local_port
            .filter(|port| *port != 0)
            .map(|port| (self.bind_address.clone(), port))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryEntry {
    id: String,
    state: String,
    entry_id: String,
    aliases: Vec<String>,
    selected_alias: String,
    source_path: String,
    source_byte_start: usize,
    source_byte_end: usize,
    source_line_start: usize,
    source_line_end: usize,
    block_fingerprint: String,
    request_signature: String,
    control_dir: String,
    control_socket: String,
    runtime_config: String,
    forwards: Vec<ForwardSpec>,
    error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RegistryFile {
    version: u8,
    tunnels: Vec<RegistryEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TunnelView {
    pub id: String,
    pub state: String,
    pub entry_id: String,
    pub aliases: Vec<String>,
    pub selected_alias: String,
    pub source_path: String,
    pub source_line: usize,
    pub forwards: Vec<ForwardSpec>,
    pub master_status: String,
    pub listener_status: String,
    pub application_health: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TunnelResponse {
    pub operation: String,
    pub tunnels: Vec<TunnelView>,
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    entry: &HostEntry,
    home: &Path,
    selected_alias: &str,
    no_input: bool,
    password_fd: Option<i32>,
    local: &[String],
    remote: &[String],
    dynamic: &[String],
    allow_bind: bool,
) -> Result<TunnelResponse, String> {
    if local.is_empty() && remote.is_empty() && dynamic.is_empty() {
        return Err(
            "TUNNEL_FORWARD_REQUIRED: provide at least one -L, -R, or -D forward".to_string(),
        );
    }
    let forwards = parse_forwards(local, remote, dynamic, allow_bind)?;
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let _lock = RegistryLock::acquire(&root)?;
    let mut registry = read_registry(&root)?;
    let fingerprint = entry_fingerprint(entry)?;
    let signature = request_signature(entry, &fingerprint, selected_alias, &forwards);

    for record in &registry.tunnels {
        if record.state == "active"
            && record.request_signature == signature
            && record.source_path == entry.source.path
            && validate_control_reference(&root, record).is_ok()
            && connect::standalone_master_ready_at(
                Path::new(&record.control_socket),
                &record.selected_alias,
            )
        {
            return Ok(TunnelResponse {
                operation: "start".to_string(),
                tunnels: vec![view(record)],
            });
        }
    }
    reserve_forwards(&forwards)?;

    let id = next_id(&registry);
    let control_dir = root.join(&id);
    fs::create_dir(&control_dir)
        .map_err(|error| format!("REGISTRY_FAILED: cannot create control directory: {error}"))?;
    set_mode(&control_dir, 0o700)?;
    let runtime =
        match connect::prepare_standalone_runtime(entry, home, selected_alias, control_dir.clone())
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = fs::remove_dir_all(&control_dir);
                return Err(error);
            }
        };
    let record = RegistryEntry {
        id,
        state: "starting".to_string(),
        entry_id: entry.id.clone(),
        aliases: entry.aliases.clone(),
        selected_alias: selected_alias.to_string(),
        source_path: entry.source.path.clone(),
        source_byte_start: entry.source.byte_start,
        source_byte_end: entry.source.byte_end,
        source_line_start: entry.source.line_start,
        source_line_end: entry.source.line_end,
        block_fingerprint: fingerprint,
        request_signature: signature,
        control_dir: runtime.control_dir().to_string_lossy().into_owned(),
        control_socket: runtime.control_socket().to_string_lossy().into_owned(),
        runtime_config: runtime.runtime_config().to_string_lossy().into_owned(),
        forwards,
        error: None,
    };
    registry.tunnels.push(record);
    write_registry(&root, &registry)?;
    let index = registry.tunnels.len() - 1;
    let record = &registry.tunnels[index];
    let command_forwards = record
        .forwards
        .iter()
        .map(|forward| (forward.flag(), forward.effective.clone()))
        .collect::<Vec<_>>();
    let listeners = record
        .forwards
        .iter()
        .filter_map(ForwardSpec::listener)
        .collect::<Vec<_>>();
    if let Err(error) = connect::launch_standalone(
        &runtime,
        no_input,
        password_fd,
        &command_forwards,
        &listeners,
    ) {
        connect::stop_standalone(&runtime);
        let record = &mut registry.tunnels[index];
        record.state = "failed".to_string();
        record.error = Some(redact_error(&error));
        write_registry(&root, &registry)?;
        drop(runtime);
        return Err(error);
    }
    registry.tunnels[index].state = "active".to_string();
    if let Err(error) = write_registry(&root, &registry) {
        connect::stop_standalone(&runtime);
        registry.tunnels[index].state = "failed".to_string();
        registry.tunnels[index].error = Some(redact_error(&error));
        let _ = write_registry(&root, &registry);
        let _ = fs::remove_dir_all(&control_dir);
        drop(runtime);
        return Err(error);
    }
    let response_record = registry.tunnels[index].clone();
    let response = TunnelResponse {
        operation: "start".to_string(),
        tunnels: vec![view(&response_record)],
    };
    drop(runtime);
    Ok(response)
}

pub fn list(home: &Path) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let registry = read_registry(&root)?;
    Ok(TunnelResponse {
        operation: "list".to_string(),
        tunnels: registry.tunnels.iter().map(view).collect(),
    })
}

pub fn status(home: &Path, id: &str) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let registry = read_registry(&root)?;
    let record = registry
        .tunnels
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))?;
    Ok(TunnelResponse {
        operation: "status".to_string(),
        tunnels: vec![view(record)],
    })
}

pub fn stop(home: &Path, id: &str) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let _lock = RegistryLock::acquire(&root)?;
    let mut registry = read_registry(&root)?;
    let index = registry
        .tunnels
        .iter()
        .position(|record| record.id == id)
        .ok_or_else(|| format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))?;
    let record = &registry.tunnels[index];
    validate_control_reference(&root, record)?;
    connect::stop_standalone_at(Path::new(&record.control_socket), &record.selected_alias);
    if connect::standalone_master_ready_at(
        Path::new(&record.control_socket),
        &record.selected_alias,
    ) {
        return Err(format!(
            "TUNNEL_STOP_FAILED: tunnel `{id}` master remains responsive"
        ));
    }
    let record = &mut registry.tunnels[index];
    record.state = "stopped".to_string();
    record.error = None;
    write_registry(&root, &registry)?;
    let response_record = registry.tunnels[index].clone();
    let _ = fs::remove_dir_all(&response_record.control_dir);
    Ok(TunnelResponse {
        operation: "stop".to_string(),
        tunnels: vec![view(&response_record)],
    })
}

pub fn restart(
    entries: &[HostEntry],
    home: &Path,
    id: &str,
    no_input: bool,
    password_fd: Option<i32>,
) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let registry = read_registry(&root)?;
    let record = registry
        .tunnels
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))?
        .clone();
    let entry = entries
        .iter()
        .find(|entry| same_entry(entry, &record))
        .ok_or_else(|| {
            format!("CONFIG_CHANGED: tunnel `{id}` selected HostEntry is unavailable")
        })?;
    if entry_fingerprint(entry)? != record.block_fingerprint {
        return Err("CONFIG_CHANGED: selected HostEntry changed on disk".to_string());
    }
    let _ = stop(home, id)?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    let mut dynamic = Vec::new();
    for forward in &record.forwards {
        match forward.kind.as_str() {
            "L" => local.push(forward.effective.clone()),
            "R" => remote.push(forward.effective.clone()),
            "D" => dynamic.push(forward.effective.clone()),
            _ => return Err("TUNNEL_INVALID: registry has unknown forward kind".to_string()),
        }
    }
    start(
        entry,
        home,
        &record.selected_alias,
        no_input,
        password_fd,
        &local,
        &remote,
        &dynamic,
        true,
    )
    .map(|mut response| {
        response.operation = "restart".to_string();
        response
    })
}

pub fn find_entry<'a>(entries: &'a [HostEntry], record: &TunnelView) -> Option<&'a HostEntry> {
    entries.iter().find(|entry| {
        entry.id == record.entry_id
            && entry.source.path == record.source_path
            && entry.source.line_start == record.source_line
    })
}

fn parse_forwards(
    local: &[String],
    remote: &[String],
    dynamic: &[String],
    allow_bind: bool,
) -> Result<Vec<ForwardSpec>, String> {
    let mut forwards = Vec::with_capacity(local.len() + remote.len() + dynamic.len());
    for value in local {
        forwards.push(parse_host_forward('L', value, allow_bind)?);
    }
    for value in remote {
        forwards.push(parse_host_forward('R', value, allow_bind)?);
    }
    for value in dynamic {
        forwards.push(parse_dynamic_forward(value, allow_bind)?);
    }
    Ok(forwards)
}

fn reserve_forwards(forwards: &[ForwardSpec]) -> Result<(), String> {
    let mut reserved = Vec::new();
    for forward in forwards {
        if let Some((bind, port)) = forward.listener() {
            if reserved
                .iter()
                .any(|(old_bind, old_port): &(String, u16)| old_bind == &bind && *old_port == port)
            {
                return Err(format!(
                    "FORWARD_DUPLICATE: local listener {bind}:{port} was requested more than once"
                ));
            }
            let address = listener_address(&bind, port)?;
            let listener = TcpListener::bind(address).map_err(|error| {
                format!("SERVICE_BIND_FAILED: cannot reserve local listener {bind}:{port}: {error}")
            })?;
            reserved.push((bind, port));
            drop(listener);
        }
    }
    Ok(())
}

fn forward_host(value: &str) -> String {
    if value.contains(':') && !value.starts_with('[') {
        format!("[{value}]")
    } else {
        value.to_string()
    }
}

fn parse_host_forward(
    kind: char,
    requested: &str,
    allow_bind: bool,
) -> Result<ForwardSpec, String> {
    let parts = split_colons(requested)?;
    let (bind, port, host, remote_port) = match parts.as_slice() {
        [port, host, remote_port] => ("127.0.0.1".to_string(), *port, *host, *remote_port),
        [bind, port, host, remote_port] => (
            normalize_bind(bind, allow_bind)?,
            *port,
            *host,
            *remote_port,
        ),
        _ => {
            return Err(format!(
                "FORWARD_INVALID: -{kind} requires [bind_address:]port:host:hostport"
            ));
        }
    };
    let bind = normalize_bind(&bind, allow_bind)?;
    let host = normalize_host(host)?;
    let remote_port = parse_port(remote_port, false)?;
    let requested_port = parse_port(port, true)?;
    let local_port = if kind == 'L' {
        Some(if requested_port == 0 {
            allocate_port(&bind)?
        } else {
            requested_port
        })
    } else {
        None
    };
    let effective_port = local_port.unwrap_or(requested_port);
    let effective = format!(
        "{}:{}:{}:{}",
        forward_host(&bind),
        effective_port,
        forward_host(host),
        remote_port
    );
    Ok(ForwardSpec {
        kind: kind.to_string(),
        requested: requested.to_string(),
        effective,
        bind_address: bind,
        local_port,
        remote_host: Some(host.to_string()),
        remote_port: Some(remote_port),
    })
}

fn parse_dynamic_forward(requested: &str, allow_bind: bool) -> Result<ForwardSpec, String> {
    let parts = split_colons(requested)?;
    let (bind, port) = match parts.as_slice() {
        [port] => ("127.0.0.1".to_string(), *port),
        [bind, port] => (normalize_bind(bind, allow_bind)?, *port),
        _ => return Err("FORWARD_INVALID: -D requires [bind_address:]port".to_string()),
    };
    let bind = normalize_bind(&bind, allow_bind)?;
    let local_port = parse_port(port, true)?;
    let local_port = if local_port == 0 {
        allocate_port(&bind)?
    } else {
        local_port
    };
    Ok(ForwardSpec {
        kind: "D".to_string(),
        requested: requested.to_string(),
        effective: format!("{}:{}", forward_host(&bind), local_port),
        bind_address: bind,
        local_port: Some(local_port),
        remote_host: None,
        remote_port: None,
    })
}

fn split_colons(value: &str) -> Result<Vec<&str>, String> {
    if value.is_empty()
        || value.contains(['\0', '\r', '\n'])
        || value.chars().any(char::is_whitespace)
    {
        return Err(
            "FORWARD_UNSAFE: forwarding specification contains unsafe characters".to_string(),
        );
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut bracketed = false;
    for (index, character) in value.char_indices() {
        match character {
            '[' => bracketed = true,
            ']' => bracketed = false,
            ':' if !bracketed => {
                parts.push(&value[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    if parts.iter().any(|part| part.is_empty()) {
        return Err("FORWARD_INVALID: forwarding specification has an empty field".to_string());
    }
    Ok(parts)
}

fn normalize_bind(value: &str, allow_bind: bool) -> Result<String, String> {
    let bind = value.trim_matches(['[', ']']);
    if bind.is_empty()
        || bind.starts_with('-')
        || bind.contains(['%', '*', '?', '\0', '\r', '\n'])
        || bind == "0.0.0.0"
        || bind == "::"
    {
        return Err("FORWARD_BIND_INVALID: bind address is unsafe".to_string());
    }
    let loopback = bind == "localhost"
        || bind
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let address = bind.parse::<IpAddr>().map_err(|_| {
        "FORWARD_BIND_INVALID: bind address must be an IP address or localhost".to_string()
    })?;
    if address.is_unspecified() || address.is_multicast() {
        return Err("FORWARD_BIND_INVALID: bind address must be specific".to_string());
    }
    if !loopback && !allow_bind {
        return Err("FORWARD_BIND_UNSAFE: non-loopback bind requires --allow-bind".to_string());
    }
    Ok(bind.to_string())
}

fn normalize_host(value: &str) -> Result<&str, String> {
    let host = value.trim_matches(['[', ']']);
    if host.is_empty() || host.starts_with('-') || host.contains(['%', '*', '?', '\0', '\r', '\n'])
    {
        return Err("FORWARD_UNSAFE: destination host is invalid".to_string());
    }
    Ok(host)
}

fn parse_port(value: &str, allow_zero: bool) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| "FORWARD_INVALID: port must be between 0 and 65535".to_string())?;
    if port == 0 && !allow_zero {
        return Err("FORWARD_INVALID: remote port must be between 1 and 65535".to_string());
    }
    Ok(port)
}

fn allocate_port(bind: &str) -> Result<u16, String> {
    let listener = TcpListener::bind(listener_address(bind, 0)?)
        .map_err(|error| format!("SERVICE_BIND_FAILED: cannot allocate local port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("SERVICE_BIND_FAILED: cannot inspect local port: {error}"))
}

fn listener_address(bind: &str, port: u16) -> Result<SocketAddr, String> {
    let host = if bind == "localhost" {
        "127.0.0.1"
    } else {
        bind
    };
    host.parse::<IpAddr>()
        .map(|address| SocketAddr::new(address, port))
        .map_err(|_| format!("FORWARD_BIND_INVALID: cannot parse bind address {bind}"))
}

fn view(record: &RegistryEntry) -> TunnelView {
    let root = Path::new(&record.control_dir)
        .parent()
        .unwrap_or_else(|| Path::new("/"));
    let control_safe = validate_control_reference(root, record).is_ok();
    let master_status = if control_safe
        && connect::standalone_master_ready_at(
            Path::new(&record.control_socket),
            &record.selected_alias,
        ) {
        "responsive"
    } else if control_safe {
        "down"
    } else {
        "unknown"
    };
    let listener_status = if master_status != "responsive" {
        "down"
    } else if record.forwards.iter().any(|forward| forward.kind == "R") {
        "unknown"
    } else if record
        .forwards
        .iter()
        .filter_map(ForwardSpec::listener)
        .all(|(bind, port)| listener_bound(&bind, port))
    {
        "bound"
    } else {
        "down"
    };
    let state = if record.state == "active" && master_status == "down" {
        "down".to_string()
    } else {
        record.state.clone()
    };
    TunnelView {
        id: record.id.clone(),
        state,
        entry_id: record.entry_id.clone(),
        aliases: record.aliases.clone(),
        selected_alias: record.selected_alias.clone(),
        source_path: record.source_path.clone(),
        source_line: record.source_line_start,
        forwards: record.forwards.clone(),
        master_status: master_status.to_string(),
        listener_status: listener_status.to_string(),
        application_health: "unknown".to_string(),
        error: record.error.clone(),
    }
}

fn listener_bound(bind: &str, port: u16) -> bool {
    listener_address(bind, port)
        .ok()
        .and_then(|address| TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok())
        .is_some()
}

fn registry_root(home: &Path) -> PathBuf {
    home.join(".config/sshx/tunnels")
}

fn ensure_private_tree(root: &Path) -> Result<(), String> {
    let config_dir = root
        .parent()
        .ok_or_else(|| "REGISTRY_FAILED: tunnel registry has no parent".to_string())?;
    let app_dir = config_dir
        .parent()
        .ok_or_else(|| "REGISTRY_FAILED: tunnel registry has no app parent".to_string())?;
    for directory in [app_dir, config_dir] {
        if directory.exists() {
            if fs::symlink_metadata(directory)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(true)
            {
                return Err(format!(
                    "REGISTRY_UNSAFE: {} is a symlink",
                    directory.display()
                ));
            }
        } else {
            fs::create_dir(directory).map_err(|error| {
                format!(
                    "REGISTRY_FAILED: cannot create {}: {error}",
                    directory.display()
                )
            })?;
        }
    }
    if root.exists() {
        ensure_private_dir(root, false)
    } else {
        fs::create_dir(root).map_err(|error| {
            format!("REGISTRY_FAILED: cannot create {}: {error}", root.display())
        })?;
        set_mode(root, 0o700)
    }
}
fn ensure_private_dir(path: &Path, create: bool) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                format!("REGISTRY_FAILED: cannot create {}: {error}", path.display())
            })?;
            fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "REGISTRY_FAILED: cannot inspect {}: {error}",
                    path.display()
                )
            })?
        }
        Err(error) => {
            return Err(format!(
                "REGISTRY_FAILED: cannot inspect {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "REGISTRY_UNSAFE: {} is not a private directory",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(format!(
                "REGISTRY_UNSAFE: {} has unsafe ownership or permissions",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_control_reference(root: &Path, record: &RegistryEntry) -> Result<(), String> {
    let control_dir = Path::new(&record.control_dir);
    let socket = Path::new(&record.control_socket);
    if control_dir.parent() != Some(root) || socket.parent() != Some(control_dir) {
        return Err("REGISTRY_UNSAFE: tunnel control path escaped registry directory".to_string());
    }
    ensure_private_dir(control_dir, false)?;
    for path in [socket, Path::new(&record.runtime_config)] {
        if let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.file_type().is_symlink()
        {
            return Err(format!("REGISTRY_UNSAFE: {} is a symlink", path.display()));
        }
    }
    Ok(())
}

fn read_registry(root: &Path) -> Result<RegistryFile, String> {
    let path = root.join("registry.json");
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(RegistryFile {
            version: VERSION,
            tunnels: Vec::new(),
        });
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("REGISTRY_UNSAFE: registry file is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(
                "REGISTRY_UNSAFE: registry file has unsafe ownership or permissions".to_string(),
            );
        }
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("REGISTRY_FAILED: cannot read registry: {error}"))?;
    let registry: RegistryFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("REGISTRY_FAILED: cannot parse registry: {error}"))?;
    if registry.version != VERSION {
        return Err("REGISTRY_VERSION: unsupported tunnel registry version".to_string());
    }
    Ok(registry)
}

fn write_registry(root: &Path, registry: &RegistryFile) -> Result<(), String> {
    let path = root.join("registry.json");
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("REGISTRY_FAILED: cannot encode registry: {error}"))?;
    let temp = root.join(format!("registry.json.tmp-{}", next_number()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| format!("REGISTRY_FAILED: cannot create registry temp file: {error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("REGISTRY_FAILED: cannot write registry: {error}"))?;
    drop(file);
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("REGISTRY_FAILED: cannot replace registry: {error}")
    })
}

struct RegistryLock {
    path: PathBuf,
}

impl RegistryLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        let path = root.join("registry.lock");
        let started = SystemTime::now();
        loop {
            match fs::create_dir(&path) {
                Ok(()) => {
                    set_mode(&path, 0o700)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if started.elapsed().unwrap_or_default() >= LOCK_TIMEOUT {
                        return Err("REGISTRY_BUSY: tunnel registry lock is held".to_string());
                    }
                    thread::sleep(LOCK_POLL);
                }
                Err(error) => {
                    return Err(format!("REGISTRY_FAILED: cannot lock registry: {error}"));
                }
            }
        }
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("REGISTRY_FAILED: cannot set permissions: {error}"))?;
    }
    Ok(())
}

fn next_number() -> u64 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
        ^ SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
}

fn next_id(registry: &RegistryFile) -> String {
    loop {
        let id = format!("dt-{:016x}", next_number());
        if !registry.tunnels.iter().any(|record| record.id == id) {
            return id;
        }
    }
}

fn entry_fingerprint(entry: &HostEntry) -> Result<String, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
    if entry.source.byte_end > bytes.len() || entry.source.byte_start >= entry.source.byte_end {
        return Err("CONFIG_INVALID: selected Host span is outside source file".to_string());
    }
    Ok(hash_hex(
        &bytes[entry.source.byte_start..entry.source.byte_end],
    ))
}

fn request_signature(
    entry: &HostEntry,
    fingerprint: &str,
    selected_alias: &str,
    forwards: &[ForwardSpec],
) -> String {
    let mut material = format!(
        "{}\0{}:{}\0{}\0{}\0{}",
        entry.source.path,
        entry.source.byte_start,
        entry.source.byte_end,
        entry.id,
        selected_alias,
        fingerprint
    );
    for forward in forwards {
        material.push('\0');
        material.push_str(&forward.kind);
        material.push('=');
        material.push_str(&forward.effective);
    }
    hash_hex(material.as_bytes())
}

fn same_entry(entry: &HostEntry, record: &RegistryEntry) -> bool {
    entry.id == record.entry_id
        && entry.source.path == record.source_path
        && entry.source.byte_start == record.source_byte_start
        && entry.source.byte_end == record.source_byte_end
}

fn hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn redact_error(error: &str) -> String {
    error.lines().next().unwrap_or(error).to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_forwards, split_colons};

    #[test]
    fn forwarding_specs_default_to_loopback_and_report_ephemeral_ports() {
        let forwards = parse_forwards(
            &["0:db.internal:5432".to_string()],
            &[],
            &["0".to_string()],
            false,
        )
        .expect("forwards should parse");
        assert_eq!(forwards[0].bind_address, "127.0.0.1");
        assert!(forwards[0].local_port.unwrap() > 0);
        assert!(forwards[1].effective.starts_with("127.0.0.1:"));
    }

    #[test]
    fn non_loopback_bind_requires_opt_in() {
        let error = parse_forwards(&["0.0.0.0:1234:db:5432".to_string()], &[], &[], false)
            .expect_err("unsafe bind should fail");
        assert!(error.starts_with("FORWARD_BIND_"));
    }

    #[test]
    fn bracketed_ipv6_is_not_split_as_fields() {
        assert_eq!(split_colons("[::1]:1234"), Ok(vec!["[::1]", "1234"]));
    }
}
