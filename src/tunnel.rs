use crate::connect;
use crate::discovery::HostEntry;
use crate::pair::{self, PairedRoute};
use crate::session::{self, ServiceForward};
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

pub type ForwardSpec = session::ForwardSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunnelRoute {
    Direct,
    Paired,
}

impl TunnelRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Paired => "paired",
        }
    }

    pub fn check(self, kind: &str) -> Result<(), String> {
        if self.as_str() != kind {
            return Err(format!(
                "TUNNEL_ROUTE_MISMATCH: expected {} route, found {kind}",
                self.as_str()
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PairedRegistry {
    gateway_entry_id: String,
    vm_entry_id: String,
    gateway_alias: String,
    vm_alias: String,
    gateway_source_path: String,
    gateway_source_byte_start: usize,
    gateway_source_byte_end: usize,
    gateway_source_line_start: usize,
    gateway_source_line_end: usize,
    vm_source_path: String,
    vm_source_byte_start: usize,
    vm_source_byte_end: usize,
    vm_source_line_start: usize,
    vm_source_line_end: usize,
    gateway_block_fingerprint: String,
    vm_block_fingerprint: String,
    gateway_id: String,
    vm_id: String,
    transit_host: String,
    transit_port: u16,
    transit_local_port: u16,
    gateway_control_dir: String,
    gateway_control_socket: String,
    gateway_runtime_config: String,
    vm_control_dir: String,
    vm_control_socket: String,
    vm_runtime_config: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryEntry {
    id: String,
    state: String,
    #[serde(default = "default_tunnel_kind")]
    kind: String,
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
    #[serde(default)]
    pair: Option<PairedRegistry>,
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
    pub kind: String,
    pub entry_id: String,
    pub aliases: Vec<String>,
    pub selected_alias: String,
    pub source_path: String,
    pub source_line: usize,
    pub forwards: Vec<ForwardSpec>,
    pub master_status: String,
    pub listener_status: String,
    pub application_health: String,
    pub gateway_entry_id: Option<String>,
    pub vm_entry_id: Option<String>,
    pub gateway_alias: Option<String>,
    pub vm_alias: Option<String>,
    pub transit_host: Option<String>,
    pub transit_port: Option<u16>,
    pub gateway_master_status: Option<String>,
    pub vm_master_status: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TunnelResponse {
    pub operation: String,
    pub tunnels: Vec<TunnelView>,
}

#[derive(Clone, Debug)]
pub(crate) struct DoctorTunnel {
    pub view: TunnelView,
    pub recorded_state: String,
    pub control_dir: PathBuf,
    pub control_socket: PathBuf,
    pub runtime_config: PathBuf,
    pub control_dir_exists: bool,
    pub control_socket_exists: bool,
    pub runtime_config_exists: bool,
}
pub(crate) fn doctor_tunnels(home: &Path) -> Result<Vec<DoctorTunnel>, String> {
    let root = registry_root(home);
    let registry = read_registry(&root)?;
    let mut tunnels = Vec::with_capacity(registry.tunnels.len());
    for record in registry.tunnels {
        let view = view_at(&root, &record);
        let path_state = |path: &Path| fs::symlink_metadata(path).is_ok();
        tunnels.push(DoctorTunnel {
            view,
            recorded_state: record.state.clone(),
            control_dir: PathBuf::from(&record.control_dir),
            control_socket: PathBuf::from(&record.control_socket),
            runtime_config: PathBuf::from(&record.runtime_config),
            control_dir_exists: path_state(Path::new(&record.control_dir)),
            control_socket_exists: path_state(Path::new(&record.control_socket)),
            runtime_config_exists: path_state(Path::new(&record.runtime_config)),
        });
    }
    Ok(tunnels)
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

    if let Some(record) = active_direct_record(&root, &registry, entry, &signature) {
        return Ok(TunnelResponse {
            operation: "start".to_string(),
            tunnels: vec![view(record)],
        });
    }
    reserve_forwards(&forwards)?;
    session::warn_remote_exposure(&forwards);

    let id = next_id(&registry);
    let control_dir = root.join(&id);
    fs::create_dir(&control_dir)
        .map_err(|error| format!("REGISTRY_FAILED: cannot create control directory: {error}"))?;
    set_mode(&control_dir, 0o700)?;
    let runtime = match connect::prepare_standalone_runtime(
        entry,
        home,
        selected_alias,
        control_dir.clone(),
        no_input,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = fs::remove_dir_all(&control_dir);
            return Err(error);
        }
    };
    let record = RegistryEntry {
        id,
        state: "starting".to_string(),
        kind: "direct".to_string(),
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
        pair: None,
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
pub fn start_paired(
    route: &PairedRoute,
    home: &Path,
    gateway_alias: &str,
    vm_alias: &str,
    no_input: bool,
    credentials: connect::PairedCredentials,
    forwards: &[ServiceForward],
) -> Result<TunnelResponse, String> {
    if forwards.is_empty() {
        return Err(
            "TUNNEL_FORWARD_REQUIRED: provide at least one --forward or use --bind".to_string(),
        );
    }
    let gateway_fingerprint = entry_fingerprint(&route.gateway)?;
    let vm_fingerprint = entry_fingerprint(&route.vm)?;
    let specs = forwards
        .iter()
        .map(service_forward_spec)
        .collect::<Vec<_>>();
    let signature = paired_request_signature(
        route,
        gateway_alias,
        vm_alias,
        &gateway_fingerprint,
        &vm_fingerprint,
        &specs,
    );
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let _lock = RegistryLock::acquire(&root)?;
    let mut registry = read_registry(&root)?;
    if let Some(record) = matching_paired_record(&root, &registry, &signature) {
        if paired_tunnel_is_responsive(&root, record) {
            return Ok(TunnelResponse {
                operation: "start".to_string(),
                tunnels: vec![view(record)],
            });
        }
        if matches!(record.state.as_str(), "active" | "starting" | "stopping") {
            return Err(format!(
                "TUNNEL_DOWN: paired tunnel `{}` already exists but is not fully responsive; use restart",
                record.id
            ));
        }
    }
    session::preflight(forwards)?;
    let id = next_id(&registry).replace("dt-", "pt-");
    let control_dir = root.join(&id);
    fs::create_dir(&control_dir)
        .map_err(|error| format!("REGISTRY_FAILED: cannot create control directory: {error}"))?;
    set_mode(&control_dir, 0o700)?;
    let runtime = match connect::prepare_paired_standalone_runtime(
        route,
        home,
        gateway_alias,
        vm_alias,
        forwards,
        control_dir.clone(),
        no_input,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = fs::remove_dir_all(&control_dir);
            return Err(error);
        }
    };
    let pair = PairedRegistry {
        gateway_entry_id: route.gateway.id.clone(),
        vm_entry_id: route.vm.id.clone(),
        gateway_alias: gateway_alias.to_string(),
        vm_alias: vm_alias.to_string(),
        gateway_source_path: route.gateway.source.path.clone(),
        gateway_source_byte_start: route.gateway.source.byte_start,
        gateway_source_byte_end: route.gateway.source.byte_end,
        gateway_source_line_start: route.gateway.source.line_start,
        gateway_source_line_end: route.gateway.source.line_end,
        vm_source_path: route.vm.source.path.clone(),
        vm_source_byte_start: route.vm.source.byte_start,
        vm_source_byte_end: route.vm.source.byte_end,
        vm_source_line_start: route.vm.source.line_start,
        vm_source_line_end: route.vm.source.line_end,
        gateway_block_fingerprint: gateway_fingerprint,
        vm_block_fingerprint: vm_fingerprint,
        gateway_id: route.gateway_id.clone(),
        vm_id: route.vm_id.clone(),
        transit_host: route.transit_host.clone(),
        transit_port: route.transit_port,
        transit_local_port: runtime.transit_port(),
        gateway_control_dir: runtime.gateway_control_dir().to_string_lossy().into_owned(),
        gateway_control_socket: runtime
            .gateway_control_socket()
            .to_string_lossy()
            .into_owned(),
        gateway_runtime_config: runtime
            .gateway_runtime_config()
            .to_string_lossy()
            .into_owned(),
        vm_control_dir: runtime.vm_control_dir().to_string_lossy().into_owned(),
        vm_control_socket: runtime.vm_control_socket().to_string_lossy().into_owned(),
        vm_runtime_config: runtime.vm_runtime_config().to_string_lossy().into_owned(),
    };
    registry.tunnels.push(RegistryEntry {
        id,
        state: "starting".to_string(),
        kind: "paired".to_string(),
        entry_id: route.vm.id.clone(),
        aliases: route.vm.aliases.clone(),
        selected_alias: vm_alias.to_string(),
        source_path: route.vm.source.path.clone(),
        source_byte_start: route.vm.source.byte_start,
        source_byte_end: route.vm.source.byte_end,
        source_line_start: route.vm.source.line_start,
        source_line_end: route.vm.source.line_end,
        block_fingerprint: pair.vm_block_fingerprint.clone(),
        request_signature: signature,
        control_dir: control_dir.to_string_lossy().into_owned(),
        control_socket: pair.vm_control_socket.clone(),
        runtime_config: pair.vm_runtime_config.clone(),
        forwards: specs,
        pair: Some(pair),
        error: None,
    });
    write_registry(&root, &registry)?;
    let index = registry.tunnels.len() - 1;
    if let Err(error) = connect::launch_paired_standalone(&runtime, no_input, credentials) {
        registry.tunnels[index].state = "failed".to_string();
        registry.tunnels[index].error = Some(redact_error(&error));
        write_registry(&root, &registry)?;
        drop(runtime);
        return Err(error);
    }
    registry.tunnels[index].state = "active".to_string();
    if let Err(error) = write_registry(&root, &registry) {
        let cleanup = paired_stop(&registry.tunnels[index]);
        registry.tunnels[index].state = "failed".to_string();
        registry.tunnels[index].error = Some(redact_error(&format!(
            "{error}{}",
            cleanup
                .as_deref()
                .map_or(String::new(), |error| format!("; {error}"))
        )));
        let _ = write_registry(&root, &registry);
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

pub fn has_active_direct_request(
    entry: &HostEntry,
    home: &Path,
    selected_alias: &str,
    forwards: &[ForwardSpec],
) -> bool {
    let Some((root, registry)) = existing_registry(home) else {
        return false;
    };
    let Ok(fingerprint) = entry_fingerprint(entry) else {
        return false;
    };
    let signature = request_signature(entry, &fingerprint, selected_alias, forwards);
    active_direct_record(&root, &registry, entry, &signature).is_some()
}

pub fn has_active_paired_request(
    route: &PairedRoute,
    home: &Path,
    gateway_alias: &str,
    vm_alias: &str,
    forwards: &[ServiceForward],
) -> bool {
    let Some((root, registry)) = existing_registry(home) else {
        return false;
    };
    let Ok(gateway_fingerprint) = entry_fingerprint(&route.gateway) else {
        return false;
    };
    let Ok(vm_fingerprint) = entry_fingerprint(&route.vm) else {
        return false;
    };
    let signature = paired_service_request_signature(
        route,
        gateway_alias,
        vm_alias,
        &gateway_fingerprint,
        &vm_fingerprint,
        forwards,
    );
    matching_paired_record(&root, &registry, &signature)
        .is_some_and(|record| paired_tunnel_is_responsive(&root, record))
}

fn existing_registry(home: &Path) -> Option<(PathBuf, RegistryFile)> {
    let root = registry_root(home);
    if !root.exists() || ensure_private_tree(&root).is_err() {
        return None;
    }
    let registry = read_registry(&root).ok()?;
    Some((root, registry))
}

pub fn list(home: &Path, route: Option<TunnelRoute>) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    if matches!(fs::symlink_metadata(&root), Err(error) if error.kind() == std::io::ErrorKind::NotFound) {
        return Ok(TunnelResponse {
            operation: "list".to_string(),
            tunnels: Vec::new(),
        });
    }
    ensure_private_tree(&root)?;
    let registry = read_registry(&root)?;
    Ok(TunnelResponse {
        operation: "list".to_string(),
        tunnels: registry.tunnels.iter()
            .filter(|record| route.is_none_or(|route| record.kind == route.as_str()))
            .map(view).collect(),
    })
}

pub fn status(home: &Path, id: &str, route: Option<TunnelRoute>) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let registry = read_registry(&root)?;
    let record = registry
        .tunnels
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))?;
    if let Some(route) = route {
        route.check(&record.kind)?;
    }
    Ok(TunnelResponse {
        operation: "status".to_string(),
        tunnels: vec![view(record)],
    })
}

pub fn stop(home: &Path, id: &str, route: Option<TunnelRoute>) -> Result<TunnelResponse, String> {
    let root = registry_root(home);
    ensure_private_tree(&root)?;
    let _lock = RegistryLock::acquire(&root)?;
    let mut registry = read_registry(&root)?;
    let index = registry
        .tunnels
        .iter()
        .position(|record| record.id == id)
        .ok_or_else(|| format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))?;
    if let Some(route) = route {
        route.check(&registry.tunnels[index].kind)?;
    }
    if registry.tunnels[index].kind == "paired" {
        validate_pair_control_reference(&root, &registry.tunnels[index])?;
    } else {
        validate_control_reference(&root, &registry.tunnels[index])?;
    }
    registry.tunnels[index].state = "stopping".to_string();
    registry.tunnels[index].error = None;
    write_registry(&root, &registry)?;
    let cleanup = paired_stop(&registry.tunnels[index]);
    if let Some(error) = cleanup {
        registry.tunnels[index].error = Some(redact_error(&error));
        write_registry(&root, &registry)?;
        return Err(error);
    }
    registry.tunnels[index].state = "stopped".to_string();
    registry.tunnels[index].error = None;
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
    expected_route: Option<TunnelRoute>,
    no_input: bool,
    password_fd: Option<i32>,
    gateway_password_fd: Option<i32>,
    vm_password_fd: Option<i32>,
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
    if let Some(route) = expected_route {
        route.check(&record.kind)?;
    }
    if record.kind == "paired" {
        let pair = record
            .pair
            .as_ref()
            .ok_or_else(|| "REGISTRY_INVALID: paired tunnel has no pair metadata".to_string())?;
        let vm = entries
            .iter()
            .find(|entry| {
                entry.id == pair.vm_entry_id
                    && entry.source.path == pair.vm_source_path
                    && entry.source.byte_start == pair.vm_source_byte_start
                    && entry.source.byte_end == pair.vm_source_byte_end
            })
            .ok_or_else(|| {
                format!("CONFIG_CHANGED: tunnel `{id}` selected VM HostEntry is unavailable")
            })?;
        let route = pair::paired_route(entries, vm)?;
        let route = route.ok_or_else(|| {
            "PAIR_BROKEN: selected VM no longer has an approved paired route".to_string()
        })?;
        validate_paired_record(&route, pair)?;
        if record.state != "stopped" {
            let _ = stop(home, id, expected_route)?;
        }
        let forwards = record
            .forwards
            .iter()
            .map(service_forward_from_spec)
            .collect::<Result<Vec<_>, _>>()?;
        return start_paired(
            &route,
            home,
            &pair.gateway_alias,
            &pair.vm_alias,
            no_input,
            connect::PairedCredentials {
                gateway_password_fd: gateway_password_fd.or(password_fd),
                vm_password_fd: vm_password_fd.or(password_fd),
            },
            &forwards,
        )
        .map(|mut response| {
            response.operation = "restart".to_string();
            response
        });
    }
    let entry = entries
        .iter()
        .find(|entry| same_entry(entry, &record))
        .ok_or_else(|| {
            format!("CONFIG_CHANGED: tunnel `{id}` selected HostEntry is unavailable")
        })?;
    if entry_fingerprint(entry)? != record.block_fingerprint {
        return Err("CONFIG_CHANGED: selected HostEntry changed on disk".to_string());
    }
    if record.state != "stopped" {
        let _ = stop(home, id, expected_route)?;
    }
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
        password_fd.or(vm_password_fd),
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
fn default_tunnel_kind() -> String {
    "direct".to_string()
}

fn service_forward_spec(forward: &ServiceForward) -> ForwardSpec {
    ForwardSpec {
        kind: "L".to_string(),
        requested: forward.id.clone(),
        effective: format!(
            "127.0.0.1:{}:{}:{}",
            forward.local_port, forward.destination_host, forward.remote_port
        ),
        bind_address: "127.0.0.1".to_string(),
        local_port: Some(forward.local_port),
        remote_host: Some(forward.destination_host.clone()),
        remote_port: Some(forward.remote_port),
    }
}

fn service_forward_from_spec(forward: &ForwardSpec) -> Result<ServiceForward, String> {
    if forward.kind != "L" || forward.bind_address != "127.0.0.1" {
        return Err(
            "TUNNEL_INVALID: paired registry contains non-loopback service forward".to_string(),
        );
    }
    Ok(ServiceForward {
        id: forward.requested.clone(),
        remote_port: forward
            .remote_port
            .ok_or_else(|| "TUNNEL_INVALID: paired forward has no remote port".to_string())?,
        destination_host: forward
            .remote_host
            .clone()
            .ok_or_else(|| "TUNNEL_INVALID: paired forward has no destination host".to_string())?,
        local_port: forward
            .local_port
            .ok_or_else(|| "TUNNEL_INVALID: paired forward has no local port".to_string())?,
    })
}

fn paired_request_signature(
    route: &PairedRoute,
    gateway_alias: &str,
    vm_alias: &str,
    gateway_fingerprint: &str,
    vm_fingerprint: &str,
    forwards: &[ForwardSpec],
) -> String {
    paired_request_signature_with(
        route,
        gateway_alias,
        vm_alias,
        gateway_fingerprint,
        vm_fingerprint,
        |material| {
            for forward in forwards {
                material.push('\0');
                material.push_str(&forward.kind);
                material.push('=');
                material.push_str(&forward.bind_address);
                material.push('=');
                material.push_str(&forward.effective);
            }
        },
    )
}

fn paired_service_request_signature(
    route: &PairedRoute,
    gateway_alias: &str,
    vm_alias: &str,
    gateway_fingerprint: &str,
    vm_fingerprint: &str,
    forwards: &[ServiceForward],
) -> String {
    paired_request_signature_with(
        route,
        gateway_alias,
        vm_alias,
        gateway_fingerprint,
        vm_fingerprint,
        |material| {
            use std::fmt::Write as _;
            for forward in forwards {
                write!(
                    material,
                    "\0L=127.0.0.1=127.0.0.1:{}:{}:{}",
                    forward.local_port, forward.destination_host, forward.remote_port
                )
                .expect("String writes cannot fail");
            }
        },
    )
}

fn paired_request_signature_with(
    route: &PairedRoute,
    gateway_alias: &str,
    vm_alias: &str,
    gateway_fingerprint: &str,
    vm_fingerprint: &str,
    append_forwards: impl FnOnce(&mut String),
) -> String {
    let mut material = format!(
        "paired\0{}:{}:{}:{}\0{}:{}:{}:{}\0{}\0{}\0{}\0{}\0{}:{}",
        route.gateway.source.path,
        route.gateway.source.byte_start,
        route.gateway.source.byte_end,
        gateway_fingerprint,
        route.vm.source.path,
        route.vm.source.byte_start,
        route.vm.source.byte_end,
        vm_fingerprint,
        route.gateway_id,
        route.vm_id,
        gateway_alias,
        vm_alias,
        route.transit_host,
        route.transit_port
    );
    append_forwards(&mut material);
    hash_hex(material.as_bytes())
}

fn pair_masters_ready(record: &RegistryEntry) -> bool {
    let Some(pair) = record.pair.as_ref() else {
        return false;
    };
    let masters_ready = connect::standalone_master_ready_at(
        Path::new(&pair.gateway_control_socket),
        &pair.gateway_alias,
    ) && connect::standalone_master_ready_at(
        Path::new(&pair.vm_control_socket),
        &pair.vm_alias,
    );
    masters_ready
        && listener_bound("127.0.0.1", pair.transit_local_port)
        && record
            .forwards
            .iter()
            .filter_map(ForwardSpec::listener)
            .all(|(bind, port)| listener_bound(&bind, port))
}

fn validate_pair_control_reference(root: &Path, record: &RegistryEntry) -> Result<(), String> {
    let pair = record
        .pair
        .as_ref()
        .ok_or_else(|| "REGISTRY_INVALID: paired tunnel has no pair metadata".to_string())?;
    let root_dir = Path::new(&record.control_dir);
    if root_dir.parent() != Some(root) {
        return Err("REGISTRY_UNSAFE: paired control path escaped registry directory".to_string());
    }
    ensure_private_dir(root_dir, false)?;
    for (directory, socket, runtime) in [
        (
            &pair.gateway_control_dir,
            &pair.gateway_control_socket,
            &pair.gateway_runtime_config,
        ),
        (
            &pair.vm_control_dir,
            &pair.vm_control_socket,
            &pair.vm_runtime_config,
        ),
    ] {
        let directory = Path::new(directory);
        if directory.parent() != Some(root_dir) {
            return Err(
                "REGISTRY_UNSAFE: paired control path escaped registry directory".to_string(),
            );
        }
        ensure_private_dir(directory, false)?;
        for path in [Path::new(socket), Path::new(runtime)] {
            if let Ok(metadata) = fs::symlink_metadata(path)
                && metadata.file_type().is_symlink()
            {
                return Err(format!("REGISTRY_UNSAFE: {} is a symlink", path.display()));
            }
        }
    }
    Ok(())
}

fn paired_stop(record: &RegistryEntry) -> Option<String> {
    if record.kind != "paired" {
        return connect::stop_standalone_checked_at(
            Path::new(&record.control_socket),
            &record.selected_alias,
        )
        .err()
        .map(|error| format!("TUNNEL_STOP_FAILED: {error}"));
    }
    let Some(pair) = record.pair.as_ref() else {
        return Some("REGISTRY_INVALID: paired tunnel has no pair metadata".to_string());
    };
    let mut errors = Vec::new();
    if let Err(error) =
        connect::stop_standalone_checked_at(Path::new(&pair.vm_control_socket), &pair.vm_alias)
    {
        errors.push(format!("VM: {error}"));
    }
    if let Err(error) = connect::stop_standalone_checked_at(
        Path::new(&pair.gateway_control_socket),
        &pair.gateway_alias,
    ) {
        errors.push(format!("gateway: {error}"));
    }
    (!errors.is_empty()).then(|| format!("TUNNEL_STOP_FAILED: {}", errors.join("; ")))
}

fn validate_paired_record(route: &PairedRoute, pair: &PairedRegistry) -> Result<(), String> {
    if route.gateway.id != pair.gateway_entry_id
        || route.vm.id != pair.vm_entry_id
        || route.gateway_id != pair.gateway_id
        || route.vm_id != pair.vm_id
        || route.transit_host != pair.transit_host
        || route.transit_port != pair.transit_port
    {
        return Err("CONFIG_CHANGED: paired route identity or transit changed on disk".to_string());
    }
    if entry_fingerprint(&route.gateway)? != pair.gateway_block_fingerprint
        || entry_fingerprint(&route.vm)? != pair.vm_block_fingerprint
    {
        return Err("CONFIG_CHANGED: paired HostEntry changed on disk".to_string());
    }
    Ok(())
}

pub fn find_entry<'a>(entries: &'a [HostEntry], record: &TunnelView) -> Option<&'a HostEntry> {
    entries.iter().find(|entry| {
        entry.id == record.entry_id
            && entry.source.path == record.source_path
            && entry.source.line_start == record.source_line
    })
}

pub fn parse_forwards(
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

pub fn reserve_forwards(forwards: &[ForwardSpec]) -> Result<(), String> {
    let mut reserved = Vec::new();
    for forward in forwards {
        if let Some((bind, port)) = forward.listener() {
            let address = listener_address(&bind, port)?;
            if reserved.contains(&address) {
                return Err(format!(
                    "FORWARD_DUPLICATE: local listener {bind}:{port} was requested more than once"
                ));
            }
            let listener = TcpListener::bind(address).map_err(|error| {
                format!("SERVICE_BIND_FAILED: cannot reserve local listener {bind}:{port}: {error}")
            })?;
            reserved.push(address);
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
    let bind = match value.strip_prefix('[') {
        Some(bind) => bind
            .strip_suffix(']')
            .filter(|bind| !bind.contains(['[', ']']))
            .ok_or_else(|| {
                "FORWARD_BIND_INVALID: bind address brackets must be balanced".to_string()
            })?,
        None if value.contains(['[', ']']) => {
            return Err(
                "FORWARD_BIND_INVALID: bind address brackets must be balanced".to_string(),
            );
        }
        None => value,
    };
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
    let address = if bind == "localhost" {
        IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        bind.parse::<IpAddr>().map_err(|_| {
            "FORWARD_BIND_INVALID: bind address must be an IP address or localhost".to_string()
        })?
    };
    if address.is_unspecified() || address.is_multicast() {
        return Err("FORWARD_BIND_INVALID: bind address must be specific".to_string());
    }
    if !loopback && !allow_bind {
        return Err("FORWARD_BIND_UNSAFE: non-loopback bind requires --allow-bind".to_string());
    }
    Ok(bind.to_string())
}

fn normalize_host(value: &str) -> Result<&str, String> {
    let host = session::unbracket_ipv6_host(value)?;
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
    view_at(root, record)
}

fn view_at(root: &Path, record: &RegistryEntry) -> TunnelView {
    let (master_status, gateway_master_status, vm_master_status, control_safe, config_safe) =
        if record.kind == "paired" {
            let control_safe = validate_pair_control_reference(root, record).is_ok();
            let config_safe = record.pair.as_ref().is_some_and(pair_config_current);
            let (gateway, vm) = record.pair.as_ref().map_or(("unknown", "unknown"), |pair| {
                if !control_safe {
                    ("unknown", "unknown")
                } else {
                    (
                        if connect::standalone_master_ready_at(
                            Path::new(&pair.gateway_control_socket),
                            &pair.gateway_alias,
                        ) {
                            "responsive"
                        } else {
                            "down"
                        },
                        if connect::standalone_master_ready_at(
                            Path::new(&pair.vm_control_socket),
                            &pair.vm_alias,
                        ) {
                            "responsive"
                        } else {
                            "down"
                        },
                    )
                }
            });
            (
                if gateway == "responsive" && vm == "responsive" {
                    "responsive"
                } else if control_safe {
                    "down"
                } else {
                    "unknown"
                },
                Some(gateway.to_string()),
                Some(vm.to_string()),
                control_safe,
                config_safe,
            )
        } else {
            let control_safe = validate_control_reference(root, record).is_ok();
            let master = if control_safe
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
            (master, None, None, control_safe, true)
        };
    let listener_status = if master_status == "unknown" {
        "unknown"
    } else if master_status == "down" {
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
    let state = if record.state == "active" && !control_safe {
        "unknown".to_string()
    } else if record.state == "active" && master_status == "down" {
        "down".to_string()
    } else {
        record.state.clone()
    };
    let error = if !config_safe {
        Some("CONFIG_CHANGED: paired HostEntry changed on disk".to_string())
    } else {
        record.error.clone()
    };
    let (gateway_entry_id, vm_entry_id, gateway_alias, vm_alias, transit_host, transit_port) =
        record
            .pair
            .as_ref()
            .map_or((None, None, None, None, None, None), |pair| {
                (
                    Some(pair.gateway_entry_id.clone()),
                    Some(pair.vm_entry_id.clone()),
                    Some(pair.gateway_alias.clone()),
                    Some(pair.vm_alias.clone()),
                    Some(pair.transit_host.clone()),
                    Some(pair.transit_local_port),
                )
            });
    TunnelView {
        id: record.id.clone(),
        state,
        kind: record.kind.clone(),
        entry_id: record.entry_id.clone(),
        aliases: record.aliases.clone(),
        selected_alias: record.selected_alias.clone(),
        source_path: record.source_path.clone(),
        source_line: record.source_line_start,
        forwards: record.forwards.clone(),
        master_status: master_status.to_string(),
        listener_status: listener_status.to_string(),
        application_health: "unknown".to_string(),
        gateway_entry_id,
        vm_entry_id,
        gateway_alias,
        vm_alias,
        transit_host,
        transit_port,
        gateway_master_status,
        vm_master_status,
        error,
    }
}

fn pair_config_current(pair: &PairedRegistry) -> bool {
    entry_span_fingerprint(
        Path::new(&pair.gateway_source_path),
        pair.gateway_source_byte_start,
        pair.gateway_source_byte_end,
    )
    .is_ok_and(|fingerprint| fingerprint == pair.gateway_block_fingerprint)
        && entry_span_fingerprint(
            Path::new(&pair.vm_source_path),
            pair.vm_source_byte_start,
            pair.vm_source_byte_end,
        )
        .is_ok_and(|fingerprint| fingerprint == pair.vm_block_fingerprint)
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

    match fs::symlink_metadata(app_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "REGISTRY_UNSAFE: {} is not a directory",
                app_dir.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(app_dir).map_err(|error| {
                format!(
                    "REGISTRY_FAILED: cannot create {}: {error}",
                    app_dir.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "REGISTRY_FAILED: cannot inspect {}: {error}",
                app_dir.display()
            ));
        }
    }

    match fs::symlink_metadata(config_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "REGISTRY_UNSAFE: {} is not a private directory",
                config_dir.display()
            ));
        }
        Ok(_) => ensure_private_dir(config_dir, false)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(config_dir).map_err(|error| {
                format!(
                    "REGISTRY_FAILED: cannot create {}: {error}",
                    config_dir.display()
                )
            })?;
            set_mode(config_dir, 0o700)?;
        }
        Err(error) => {
            return Err(format!(
                "REGISTRY_FAILED: cannot inspect {}: {error}",
                config_dir.display()
            ));
        }
    }

    ensure_private_dir(root, true)
}
fn ensure_private_dir(path: &Path, create: bool) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                format!("REGISTRY_FAILED: cannot create {}: {error}", path.display())
            })?;
            set_mode(path, 0o700)?;
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
fn entry_span_fingerprint(
    path: &Path,
    byte_start: usize,
    byte_end: usize,
) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if byte_start >= byte_end || byte_end > bytes.len() {
        return Err("source span is outside file".to_string());
    }
    Ok(hash_hex(&bytes[byte_start..byte_end]))
}

fn request_signature(
    entry: &HostEntry,
    fingerprint: &str,
    selected_alias: &str,
    forwards: &[ForwardSpec],
) -> String {
    request_signature_with(entry, fingerprint, selected_alias, |material| {
        for forward in forwards {
            material.push('\0');
            material.push_str(&forward.kind);
            material.push('=');
            material.push_str(&forward.effective);
        }
    })
}


fn request_signature_with(
    entry: &HostEntry,
    fingerprint: &str,
    selected_alias: &str,
    append_forwards: impl FnOnce(&mut String),
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
    append_forwards(&mut material);
    hash_hex(material.as_bytes())
}

fn active_direct_record<'a>(
    root: &Path,
    registry: &'a RegistryFile,
    entry: &HostEntry,
    signature: &str,
) -> Option<&'a RegistryEntry> {
    registry.tunnels.iter().find(|record| {
        active_direct_identity_matches(record, &entry.id, &entry.source.path, signature)
            && validate_control_reference(root, record).is_ok()
            && connect::standalone_master_ready_at(
                Path::new(&record.control_socket),
                &record.selected_alias,
            )
    })
}

fn active_direct_identity_matches(
    record: &RegistryEntry,
    entry_id: &str,
    source_path: &str,
    signature: &str,
) -> bool {
    record.state == "active"
        && record.entry_id == entry_id
        && record.request_signature == signature
        && record.source_path == source_path
}

fn matching_paired_record<'a>(
    root: &Path,
    registry: &'a RegistryFile,
    signature: &str,
) -> Option<&'a RegistryEntry> {
    registry
        .tunnels
        .iter()
        .find(|record| {
            record.kind == "paired"
                && record.request_signature == signature
                && paired_tunnel_is_responsive(root, record)
        })
        .or_else(|| {
            registry.tunnels.iter().find(|record| {
                record.kind == "paired"
                    && record.request_signature == signature
                    && matches!(record.state.as_str(), "active" | "starting" | "stopping")
            })
        })
        .or_else(|| {
            registry
                .tunnels
                .iter()
                .find(|record| record.kind == "paired" && record.request_signature == signature)
        })
}

fn paired_tunnel_is_responsive(root: &Path, record: &RegistryEntry) -> bool {
    record.state == "active"
        && validate_pair_control_reference(root, record).is_ok()
        && pair_masters_ready(record)
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
    use super::{
        RegistryEntry, active_direct_identity_matches, parse_forwards, reserve_forwards,
        split_colons,
    };
    #[test]
    fn active_direct_reuse_requires_entry_id_and_active_state_on_same_record() {
        let record = |entry_id: &str, state: &str| RegistryEntry {
            id: "tunnel".to_string(),
            state: state.to_string(),
            kind: "direct".to_string(),
            entry_id: entry_id.to_string(),
            aliases: vec!["app".to_string()],
            selected_alias: "app".to_string(),
            source_path: "/ssh/config".to_string(),
            source_byte_start: 0,
            source_byte_end: 10,
            source_line_start: 1,
            source_line_end: 2,
            block_fingerprint: String::new(),
            request_signature: "signature".to_string(),
            control_dir: String::new(),
            control_socket: String::new(),
            runtime_config: String::new(),
            forwards: Vec::new(),
            pair: None,
            error: None,
        };
        let wrong_entry_active = record("other-id", "active");
        let correct_entry_stopped = record("entry-id", "stopped");
        let matching_active = record("entry-id", "active");

        assert!(!active_direct_identity_matches(
            &wrong_entry_active,
            "entry-id",
            "/ssh/config",
            "signature"
        ));
        assert!(!active_direct_identity_matches(
            &correct_entry_stopped,
            "entry-id",
            "/ssh/config",
            "signature"
        ));
        assert!(active_direct_identity_matches(
            &matching_active,
            "entry-id",
            "/ssh/config",
            "signature"
        ));
    }

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
    fn localhost_and_ipv4_loopback_are_duplicate_listeners() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("temporary port should bind");
        let port = listener.local_addr().expect("address should exist").port();
        drop(listener);
        let forwards = parse_forwards(
            &[
                format!("127.0.0.1:{port}:db.internal:5432"),
                format!("localhost:{port}:cache.internal:5433"),
            ],
            &[],
            &[],
            false,
        )
        .expect("forwards should parse");
        let error = reserve_forwards(&forwards).expect_err("loopback aliases conflict");
        assert!(error.starts_with("FORWARD_DUPLICATE"), "{error}");
    }

    #[test]
    fn non_loopback_bind_requires_opt_in() {
        let error = parse_forwards(&["0.0.0.0:1234:db:5432".to_string()], &[], &[], false)
            .expect_err("unsafe bind should fail");
        assert!(error.starts_with("FORWARD_BIND_"));
    }

    #[test]
    fn wildcard_bind_remains_forbidden_with_opt_in() {
        let error = parse_forwards(&["0.0.0.0:1234:db:5432".to_string()], &[], &[], true)
            .expect_err("wildcard bind remains forbidden");
        assert!(error.starts_with("FORWARD_BIND_"));
    }

    #[test]
    fn non_loopback_bind_accepts_explicit_opt_in() {
        let forwards = parse_forwards(
            &["192.0.2.1:1234:db.internal:5432".to_string()],
            &[],
            &[],
            true,
        )
        .expect("explicit non-loopback bind should parse");
        assert_eq!(forwards[0].bind_address, "192.0.2.1");
        assert_eq!(forwards[0].local_port, Some(1234));
    }

    #[test]
    fn bracketed_ipv6_is_not_split_as_fields() {
        assert_eq!(split_colons("[::1]:1234"), Ok(vec!["[::1]", "1234"]));
    }
}
