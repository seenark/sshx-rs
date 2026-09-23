use crate::connect;
use crate::discovery::{
    self, Catalog, Diagnostic as DiscoveryDiagnostic, DiscoveryRoot, HostEntry,
};
use crate::pair;
use crate::session;
use crate::settings::RegisteredRoot;
use crate::tunnel;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Selection {
    pub id: Option<String>,
    pub source: Option<PathBuf>,
    pub line: Option<usize>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidationInfo {
    pub local: String,
    pub remote_servers: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RootReport {
    pub scope: String,
    pub path: String,
    pub project: Option<String>,
    pub status: String,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KnownHostReport {
    pub scope: String,
    pub path: String,
    pub status: String,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StateReport {
    pub kind: String,
    pub path: String,
    pub status: String,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeReport {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub master: String,
    pub gateway_master: Option<String>,
    pub vm_master: Option<String>,
    pub listener: String,
    pub application_health: String,
    pub selection: String,
    pub control_dir: String,
    pub control_socket: String,
    pub runtime_config: String,
    pub control_dir_present: bool,
    pub control_socket_present: bool,
    pub runtime_config_present: bool,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: String,
    pub severity: String,
    pub stage: String,
    pub message: String,
    pub guidance: String,
    pub path: Option<String>,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DoctorReport {
    pub version: u8,
    pub validation: ValidationInfo,
    pub roots: Vec<RootReport>,
    pub known_hosts: Vec<KnownHostReport>,
    pub state: Vec<StateReport>,
    pub runtime: Vec<RuntimeReport>,
    pub findings: Vec<Finding>,
    pub repairs: Vec<crate::permissions::RepairCandidate>,
}

#[derive(Default)]
struct ScanState {
    visited: HashSet<PathBuf>,
    active: Vec<PathBuf>,
    password_auth_configured: bool,
    password_files: HashSet<PathBuf>,
}

struct ReportBuilder {
    report: DoctorReport,
    seen: HashSet<String>,
    planned_repairs: HashSet<PathBuf>,
}

impl ReportBuilder {
    fn new() -> Self {
        Self {
            report: DoctorReport {
                version: 1,
                validation: ValidationInfo {
                    local: "local_and_fixture_only".to_string(),
                    remote_servers: "not_run".to_string(),
                },
                roots: Vec::new(),
                known_hosts: Vec::new(),
                state: Vec::new(),
                runtime: Vec::new(),
                findings: Vec::new(),
                repairs: Vec::new(),
            },
            seen: HashSet::new(),
            planned_repairs: HashSet::new(),
        }
    }
    fn finding(
        &mut self,
        code: &str,
        severity: &str,
        stage: &str,
        message: impl Into<String>,
        guidance: impl Into<String>,
        path: Option<&Path>,
    ) {
        let message = message.into();
        let path = path.map(display_path);
        let key = format!(
            "{code}\0{stage}\0{}\0{message}",
            path.as_deref().unwrap_or_default()
        );
        if !self.seen.insert(key) {
            return;
        }
        self.report.findings.push(Finding {
            code: code.to_string(),
            severity: severity.to_string(),
            stage: stage.to_string(),
            message,
            guidance: guidance.into(),
            path,
            evidence: "local".to_string(),
        });
    }

    fn repair(
        &mut self,
        kind: &str,
        path: &Path,
        target: crate::permissions::PermissionTarget,
        current_mode: u32,
    ) {
        if !self.planned_repairs.insert(path.to_path_buf()) {
            return;
        }
        self.report
            .repairs
            .push(crate::permissions::RepairCandidate {
                kind: kind.to_string(),
                path: path.to_path_buf(),
                target,
                current_mode,
                reason: format!(
                    "current mode {:o} requires {:o}",
                    current_mode,
                    target.private_mode()
                ),
            });
    }

    fn finish(mut self) -> DoctorReport {
        self.report.findings.sort_by(|left, right| {
            (&left.stage, &left.code, &left.path, &left.message).cmp(&(
                &right.stage,
                &right.code,
                &right.path,
                &right.message,
            ))
        });
        self.report
            .repairs
            .sort_by(|left, right| (&left.kind, &left.path).cmp(&(&right.kind, &right.path)));
        self.report
    }
}

pub fn run(
    home: &Path,
    roots: &[RegisteredRoot],
    settings_error: Option<&str>,
    password_requested: bool,
    selection: Selection,
) -> DoctorReport {
    let mut builder = ReportBuilder::new();
    if let Some(error) = settings_error {
        builder.finding(
            "app_state_unreadable",
            "error",
            "state",
            error,
            "Repair or remove the unreadable app settings file, then run doctor again.",
            Some(&crate::settings::settings_path(home)),
        );
    }

    let mut valid_roots = Vec::new();
    let mut scanner = ScanState::default();
    for root in roots {
        let path = absolute_path(&root.path);
        let status = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                builder.finding(
                    "config_root_unreadable",
                    "error",
                    "config",
                    format!("{} is a symlink, not a trusted config root", path.display()),
                    "Point setup at a regular SSH config file; doctor never follows an unsafe root.",
                    Some(&path),
                );
                "unsafe".to_string()
            }
            Ok(metadata) if !metadata.is_file() => {
                builder.finding(
                    "config_root_missing",
                    "error",
                    "config",
                    format!("config root is not a regular file: {}", path.display()),
                    "Create the config file or update setup to reference an existing file.",
                    Some(&path),
                );
                "missing".to_string()
            }
            Ok(_) => match fs::read(&path) {
                Ok(_) => {
                    valid_roots.push(DiscoveryRoot::new(
                        path.clone(),
                        root.scope.clone(),
                        root.project.clone(),
                    ));
                    scan_file(&path, home, &mut scanner, &mut builder, true);
                    "ready".to_string()
                }
                Err(error) => {
                    builder.finding(
                        "config_root_unreadable",
                        "error",
                        "config",
                        format!("cannot read config root {}: {error}", path.display()),
                        "Review ownership manually; use --fix-permissions to chmod eligible paths after one confirmation.",
                        Some(&path),
                    );
                    if let Ok(current_mode) = crate::permissions::assess(
                        &path,
                        crate::permissions::PermissionTarget::File,
                    ) && current_mode != crate::permissions::PRIVATE_FILE_MODE
                    {
                        builder.repair(
                            "config_root",
                            &path,
                            crate::permissions::PermissionTarget::File,
                            current_mode,
                        );
                    }
                    "unreadable".to_string()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                builder.finding(
                    "config_root_missing",
                    "error",
                    "config",
                    format!("config root does not exist: {}", path.display()),
                    "Create the config file or update setup to reference an existing file.",
                    Some(&path),
                );
                "missing".to_string()
            }
            Err(error) => {
                builder.finding(
                    "config_root_unreadable",
                    "error",
                    "config",
                    format!("cannot inspect config root {}: {error}", path.display()),
                    "Fix file ownership or permissions, then run doctor again.",
                    Some(&path),
                );
                "unreadable".to_string()
            }
        };
        builder.report.roots.push(RootReport {
            scope: root.scope.clone(),
            path: display_path(&path),
            project: root.project.clone(),
            status,
            evidence: "local".to_string(),
        });
    }

    check_prerequisites(&mut builder, &scanner, password_requested);
    check_state_locations(home, &mut builder);

    let catalog = if valid_roots.is_empty() {
        Catalog {
            entries: Vec::new(),
            diagnostics: Vec::new(),
        }
    } else {
        match discovery::discover_roots(&valid_roots) {
            Ok(catalog) => catalog,
            Err(error) => {
                let message = error.to_string();
                let code = if message.contains("included config") {
                    "include_unreadable"
                } else {
                    "config_unreadable"
                };
                builder.finding(
                    code,
                    "error",
                    "config",
                    message,
                    "Fix the reported config or Include path; no config is repaired automatically.",
                    None,
                );
                Catalog {
                    entries: Vec::new(),
                    diagnostics: Vec::new(),
                }
            }
        }
    };

    for diagnostic in &catalog.diagnostics {
        add_discovery_diagnostic(&mut builder, diagnostic);
    }
    check_entries(home, &catalog.entries, &scanner, &selection, &mut builder);
    for path in &scanner.password_files {
        check_password_permissions(path, &mut builder);
    }
    check_runtime(home, &catalog.entries, &mut builder);
    builder.finish()
}

fn check_prerequisites(builder: &mut ReportBuilder, scan: &ScanState, password_requested: bool) {
    let ssh_ready = command_succeeds("ssh", &["-V"]);
    if ssh_ready {
        builder.finding(
            "openssh_ready",
            "info",
            "prerequisites",
            "OpenSSH client is available on PATH",
            "No action required.",
            None,
        );
    } else {
        builder.finding(
            "openssh_missing",
            "error",
            "prerequisites",
            "OpenSSH client `ssh` is missing or cannot run",
            "Install the OpenSSH client and ensure `ssh` is available on PATH.",
            None,
        );
    }

    let password_needed =
        password_requested || scan.password_auth_configured || !scan.password_files.is_empty();
    if !password_needed {
        builder.finding(
            "sshpass_not_required",
            "info",
            "prerequisites",
            "No configured or requested password authentication was found",
            "Key and agent authentication do not require sshpass.",
            None,
        );
    } else if command_succeeds("sshpass", &["-V"]) {
        builder.finding(
            "sshpass_ready",
            "info",
            "prerequisites",
            "sshpass is available for configured or requested password authentication",
            "No action required.",
            None,
        );
    } else {
        builder.finding(
            "sshpass_missing",
            "error",
            "prerequisites",
            "Password authentication is configured or requested, but sshpass is missing",
            "Install sshpass, or remove password authentication metadata and use keys or an agent.",
            None,
        );
    }
}

fn check_entries(
    home: &Path,
    entries: &[HostEntry],
    scan: &ScanState,
    selection: &Selection,
    builder: &mut ReportBuilder,
) {
    for diagnostic in pair::diagnostics(entries) {
        let (stage, severity) = match diagnostic.code.as_str() {
            "duplicate_id" | "malformed_id" => ("config", "error"),
            "broken_reference" | "pair_conflict" | "malformed_pair" => ("pair", "error"),
            _ => ("config", "warning"),
        };
        builder.finding(
            &diagnostic.code,
            severity,
            stage,
            diagnostic.message,
            pair_guidance(stage),
            None,
        );
    }

    for entry in entries {
        let alias = entry
            .aliases
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        if alias.is_empty() {
            builder.finding(
                "unsupported_semantics",
                "error",
                "config",
                format!("HostEntry {} has no exact alias", entry.source.path),
                "Give the Host block one exact alias before using sshx.",
                Some(Path::new(&entry.source.path)),
            );
            continue;
        }
        if let Err(error) = connect::compile_config(entry, alias) {
            let unsupported = error.starts_with("UNSUPPORTED_");
            builder.finding(
                if unsupported {
                    "unsupported_semantics"
                } else {
                    "config_invalid"
                },
                "error",
                "config",
                format!("{}: {error}", entry.source.path),
                "Use an exact Host block with supported OpenSSH semantics; doctor does not rewrite it.",
                Some(Path::new(&entry.source.path)),
            );
        }

        match session::declared_services(entry) {
            Ok(services) => {
                let mut seen = HashSet::new();
                for service in services {
                    if !seen.insert(service.default_local_port) {
                        builder.finding(
                            "local_port_conflict",
                            "error",
                            "service_bind",
                            format!(
                                "HostEntry {} declares local port {} more than once",
                                entry.source.path, service.default_local_port
                            ),
                            "Choose distinct local service ports before starting a tunnel.",
                            Some(Path::new(&entry.source.path)),
                        );
                    }
                    if TcpListener::bind(("127.0.0.1", service.default_local_port)).is_err() {
                        builder.finding(
                            "local_port_conflict",
                            "warning",
                            "service_bind",
                            format!(
                                "local service port {} is not available for {}",
                                service.default_local_port, entry.source.path
                            ),
                            "Stop the process using this local port or request another local port; doctor never kills it.",
                            Some(Path::new(&entry.source.path)),
                        );
                    }
                }
            }
            Err(error) => {
                builder.finding(
                    "unsupported_semantics",
                    "error",
                    "service_bind",
                    format!("{}: {error}", entry.source.path),
                    "Fix service metadata and declared ports before requesting a service forward.",
                    Some(Path::new(&entry.source.path)),
                );
            }
        }

        if scan.password_files.contains(Path::new(&entry.source.path)) {
            check_password_permissions(Path::new(&entry.source.path), builder);
        }
    }

    check_pair_routes(entries, builder);
    check_selection(home, entries, selection, builder);
}

fn check_pair_routes(entries: &[HostEntry], builder: &mut ReportBuilder) {
    let vm_ids = pair::records(entries)
        .into_iter()
        .map(|record| record.vm_id.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for entry in entries {
        if !vm_ids.contains(&entry.id.to_ascii_lowercase()) {
            continue;
        }
        match pair::paired_route(entries, entry) {
            Ok(Some(route)) => builder.finding(
                "gateway_route_ready",
                "info",
                "gateway_route",
                format!(
                    "gateway route for {} reaches {}:{} through {}",
                    route.vm.source.path,
                    route.transit_host,
                    route.transit_port,
                    route.gateway.source.path
                ),
                "Gateway route configuration is local evidence only; run the tunnel to validate a real server.",
                Some(Path::new(&route.vm.source.path)),
            ),
            Ok(None) => {}
            Err(error) => builder.finding(
                "gateway_route_invalid",
                "error",
                "gateway_route",
                format!("{}: {error}", entry.source.path),
                "Repair Pair metadata or gateway LocalForward and rerun pair validation.",
                Some(Path::new(&entry.source.path)),
            ),
        }
    }
}

fn check_selection(
    home: &Path,
    entries: &[HostEntry],
    selection: &Selection,
    builder: &mut ReportBuilder,
) {
    let has_selection = selection.id.is_some()
        || selection.source.is_some()
        || selection.line.is_some()
        || selection.alias.is_some();
    if !has_selection {
        return;
    }
    let source = selection
        .source
        .as_deref()
        .map(|path| crate::settings::normalize_path(path, home));
    let matches = entries.iter().filter(|entry| {
        selection.id.as_deref().is_none_or(|id| entry.id == id)
            && selection
                .alias
                .as_deref()
                .is_none_or(|alias| entry.aliases.iter().any(|value| value == alias))
            && source
                .as_deref()
                .is_none_or(|path| Path::new(&entry.source.path) == path)
            && selection
                .line
                .is_none_or(|line| entry.source.line_start == line)
    });
    if matches.count() == 0 {
        builder.finding(
            "stale_selection",
            "warning",
            "selection",
            "requested HostEntry selection is not present in the current config",
            "Refresh the source path, Host line, or immutable ID; doctor never selects a replacement.",
            source.as_deref(),
        );
    }
}

fn check_runtime(home: &Path, entries: &[HostEntry], builder: &mut ReportBuilder) {
    let tunnels = match tunnel::doctor_tunnels(home) {
        Ok(tunnels) => tunnels,
        Err(error) => {
            builder.finding(
                "runtime_registry_unreadable",
                "error",
                "managed_master",
                error,
                "Inspect the registry manually; doctor does not rewrite, adopt, or terminate runtime state.",
                Some(&home.join(".config/sshx/tunnels/registry.json")),
            );
            return;
        }
    };
    for tunnel in tunnels {
        let view = &tunnel.view;
        let selection = entries.iter().any(|entry| {
            entry.id == view.entry_id
                && entry.source.path == view.source_path
                && entry.source.line_start == view.source_line
        });
        let selection_status = if selection { "current" } else { "stale" };
        let runtime = RuntimeReport {
            id: view.id.clone(),
            kind: view.kind.clone(),
            state: view.state.clone(),
            master: view.master_status.clone(),
            gateway_master: view.gateway_master_status.clone(),
            vm_master: view.vm_master_status.clone(),
            listener: view.listener_status.clone(),
            application_health: view.application_health.clone(),
            selection: selection_status.to_string(),
            control_dir: tunnel.control_dir.to_string_lossy().into_owned(),
            control_socket: tunnel.control_socket.to_string_lossy().into_owned(),
            runtime_config: tunnel.runtime_config.to_string_lossy().into_owned(),
            control_dir_present: tunnel.control_dir_exists,
            control_socket_present: tunnel.control_socket_exists,
            runtime_config_present: tunnel.runtime_config_exists,
            evidence: "local".to_string(),
        };
        builder.report.runtime.push(runtime);
        if !selection {
            builder.finding(
                "stale_selection",
                "warning",
                "selection",
                format!("managed tunnel `{}` references a HostEntry absent from current config", view.id),
                "Review the registry and config manually; doctor does not adopt or stop the tunnel.",
                Some(Path::new(&view.source_path)),
            );
        }
        builder.finding(
            "managed_master_state",
            if view.master_status == "responsive" {
                "info"
            } else {
                "warning"
            },
            "managed_master",
            format!(
                "managed tunnel `{}` master is {}",
                view.id, view.master_status
            ),
            "Use `tunnel status` for details; doctor never reconnects or terminates a master.",
            Some(Path::new(&view.source_path)),
        );
        builder.finding(
            "listener_state",
            if view.listener_status == "bound" {
                "info"
            } else {
                "warning"
            },
            "listener",
            format!(
                "managed tunnel `{}` listener is {}",
                view.id, view.listener_status
            ),
            "Resolve local listener conflicts without changing unrelated processes.",
            Some(Path::new(&view.source_path)),
        );
        builder.finding(
            "application_health_unproven",
            "info",
            "application",
            format!(
                "managed tunnel `{}` application health is {}",
                view.id, view.application_health
            ),
            "Test the forwarded application separately; transport and listener state do not prove application health.",
            Some(Path::new(&view.source_path)),
        );
        if !tunnel.control_socket_exists {
            builder.finding(
                "stale_runtime_socket",
                "warning",
                "managed_master",
                format!("managed tunnel `{}` has no recorded control socket", view.id),
                "Inspect or stop only after proving ownership; doctor does not adopt or kill processes.",
                Some(&tunnel.control_socket),
            );
        } else if view.master_status != "responsive" && tunnel.recorded_state == "active" {
            builder.finding(
                "stale_runtime_socket",
                "warning",
                "managed_master",
                format!("managed tunnel `{}` registry is active but its master is not responsive", view.id),
                "Use explicit status or stop after verifying ownership; doctor does not reconnect or terminate it.",
                Some(&tunnel.control_socket),
            );
        }
    }
}

fn check_password_permissions(path: &Path, builder: &mut ReportBuilder) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        builder.finding(
            "password_file_permissions",
            "warning",
            "config",
            format!("password-bearing config file {} is a symlink", path.display()),
            "Replace the path manually with a user-owned regular file; doctor never follows or replaces symlinks.",
            Some(path),
        );
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = metadata.permissions().mode() & 0o7777;
        let unsafe_mode = mode & 0o077 != 0 || mode & 0o700 != 0o600;
        let unsafe_owner = metadata.uid() != unsafe { libc::geteuid() } as u32;
        if unsafe_mode || unsafe_owner {
            builder.finding(
                "password_file_permissions",
                "warning",
                "config",
                format!(
                    "password-bearing config file {} has unsafe ownership or permissions",
                    path.display()
                ),
                "Review ownership manually; use --fix-permissions to chmod eligible files after one confirmation.",
                Some(path),
            );
            if !unsafe_owner
                && metadata.is_file()
                && let Ok(current_mode) =
                    crate::permissions::assess(path, crate::permissions::PermissionTarget::File)
            {
                builder.repair(
                    "password_file",
                    path,
                    crate::permissions::PermissionTarget::File,
                    current_mode,
                );
            }
        }
    }
}

fn check_state_locations(home: &Path, builder: &mut ReportBuilder) {
    let config_dir = home.join(".config/sshx");
    let tunnel_dir = config_dir.join("tunnels");
    let locations = [
        (
            "app_config_dir",
            config_dir.clone(),
            crate::permissions::PermissionTarget::Directory,
        ),
        (
            "app_settings",
            crate::settings::settings_path(home),
            crate::permissions::PermissionTarget::File,
        ),
        (
            "tunnel_registry_dir",
            tunnel_dir.clone(),
            crate::permissions::PermissionTarget::Directory,
        ),
        (
            "tunnel_registry",
            tunnel_dir.join("registry.json"),
            crate::permissions::PermissionTarget::File,
        ),
    ];
    for (kind, path, target) in locations {
        let status = inspect_location(kind, &path, target, builder, "app_state");
        builder.report.state.push(StateReport {
            kind: kind.to_string(),
            path: display_path(&path),
            status,
            evidence: "local".to_string(),
        });
    }

    for (scope, path) in [
        ("personal", home.join(".ssh/known_hosts")),
        ("work", home.join(".config/sshx/known_hosts/work")),
    ] {
        let status = inspect_location(
            scope,
            &path,
            crate::permissions::PermissionTarget::File,
            builder,
            "known_hosts",
        );
        builder.report.known_hosts.push(KnownHostReport {
            scope: scope.to_string(),
            path: display_path(&path),
            status,
            evidence: "local".to_string(),
        });
    }
}
fn inspect_location(
    kind: &str,
    path: &Path,
    target: crate::permissions::PermissionTarget,
    builder: &mut ReportBuilder,
    stage: &str,
) -> String {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let code = if stage == "known_hosts" {
                "known_hosts_missing"
            } else {
                "app_state_missing"
            };
            builder.finding(
                code,
                "info",
                stage,
                format!("{} is missing", path.display()),
                if stage == "known_hosts" {
                    "Create this store only after explicit host-key trust; doctor never accepts keys automatically."
                } else {
                    "The location will be created by an explicit management operation; doctor never creates state."
                },
                Some(path),
            );
            return "missing".to_string();
        }
        Err(error) => {
            let code = if stage == "known_hosts" {
                "known_hosts_unreadable"
            } else {
                "app_state_unreadable"
            };
            builder.finding(
                code,
                "error",
                stage,
                format!("cannot inspect {}: {error}", path.display()),
                "Fix ownership or permissions manually, then run doctor again.",
                Some(path),
            );
            return "unreadable".to_string();
        }
    };
    if metadata.file_type().is_symlink()
        || (target.is_file() && !metadata.is_file())
        || (!target.is_file() && !metadata.is_dir())
    {
        let code = if stage == "known_hosts" {
            "known_hosts_insecure"
        } else {
            "app_state_insecure"
        };
        builder.finding(
            code,
            "error",
            stage,
            format!(
                "{} is not a trusted {}",
                path.display(),
                if target.is_file() {
                    "file"
                } else {
                    "directory"
                }
            ),
            "Replace the path manually with a user-owned regular location; doctor never follows symlinks.",
            Some(path),
        );
        return "unsafe".to_string();
    }
    let (unsafe_mode, unsafe_owner) = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            (
                metadata.permissions().mode() & 0o7777 != target.private_mode(),
                metadata.uid() != unsafe { libc::geteuid() } as u32,
            )
        }
        #[cfg(not(unix))]
        {
            (false, false)
        }
    };
    if unsafe_mode || unsafe_owner {
        let code = if stage == "known_hosts" {
            "known_hosts_insecure"
        } else {
            "app_state_insecure"
        };
        builder.finding(
            code,
            "warning",
            stage,
            format!("{} has unsafe ownership or permissions", path.display()),
            "Review ownership manually; use --fix-permissions to chmod eligible paths after one confirmation. Doctor never changes ownership, replaces paths, or modifies arbitrary parents.",
            Some(path),
        );
        if unsafe_mode
            && !unsafe_owner
            && let Ok(current_mode) = crate::permissions::assess(path, target)
        {
            builder.repair(kind, path, target, current_mode);
        }
        "insecure".to_string()
    } else {
        "ready".to_string()
    }
}

fn add_discovery_diagnostic(builder: &mut ReportBuilder, diagnostic: &DiscoveryDiagnostic) {
    let unsupported = diagnostic.code.starts_with("unsupported_");
    builder.finding(
        if unsupported {
            "unsupported_semantics"
        } else {
            &diagnostic.code
        },
        "error",
        "config",
        diagnostic.message.clone(),
        if diagnostic.code == "include_cycle" {
            "Break the Include cycle while preserving intended file order."
        } else {
            "Use supported exact Host semantics; doctor does not rewrite config files."
        },
        None,
    );
}

fn pair_guidance(stage: &str) -> &'static str {
    if stage == "pair" {
        "Repair both sides of the Pair, then run `pair validate`; doctor never regenerates IDs or references."
    } else {
        "Keep immutable IDs unique and valid; doctor never rewrites metadata."
    }
}

fn check_file_include(
    path: &Path,
    home: &Path,
    scanner: &mut ScanState,
    builder: &mut ReportBuilder,
    is_root: bool,
) {
    let identity = fs::canonicalize(path).unwrap_or_else(|_| absolute_path(path));
    if scanner.active.contains(&identity) {
        builder.finding(
            "include_cycle",
            "error",
            "config",
            format!("Include cycle reaches {}", path.display()),
            "Break the Include cycle while preserving intended file order.",
            Some(path),
        );
        return;
    }
    if !scanner.visited.insert(identity.clone()) {
        return;
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            if !is_root {
                builder.finding(
                    "include_unreadable",
                    "error",
                    "config",
                    format!("cannot read included config {}: {error}", path.display()),
                    "Fix the Include target ownership or permissions; doctor never skips it silently.",
                    Some(path),
                );
            }
            return;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            builder.finding(
                "unsupported_semantics",
                "error",
                "config",
                format!("config file {} is not valid UTF-8", path.display()),
                "Rewrite the file as valid UTF-8 without changing its intended OpenSSH directives.",
                Some(path),
            );
            return;
        }
    };
    scanner.active.push(identity);
    let mut in_host = false;
    for (line_number, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let Some(value) = trimmed.strip_prefix("##PASSWORD")
                && !value.trim().is_empty()
            {
                scanner.password_files.insert(path.to_path_buf());
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("##PASSWORD") {
            if !value.trim().is_empty() {
                scanner.password_files.insert(path.to_path_buf());
            }
            continue;
        }
        let Some((keyword, argument)) = split_directive(trimmed) else {
            continue;
        };
        let keyword_lower = keyword.to_ascii_lowercase();
        let directive_arguments = discovery::tokenize(argument);
        if keyword_lower == "passwordauthentication"
            && directive_arguments
                .first()
                .is_some_and(|value| value.eq_ignore_ascii_case("yes"))
        {
            scanner.password_auth_configured = true;
        }
        if keyword_lower == "preferredauthentications"
            && directive_arguments.iter().any(|value| {
                value
                    .split(',')
                    .any(|method| method.eq_ignore_ascii_case("password"))
            })
        {
            scanner.password_auth_configured = true;
        }
        match keyword_lower.as_str() {
            "host" => {
                let aliases = tokenize(argument);
                in_host = true;
                if aliases.is_empty()
                    || aliases
                        .iter()
                        .any(|alias| wildcard(alias) || alias.contains('%'))
                {
                    builder.finding(
                        "unsupported_semantics",
                        "error",
                        "config",
                        format!(
                            "{}:{} has wildcard or token Host semantics",
                            path.display(),
                            line_number + 1
                        ),
                        "Select an exact Host alias before using sshx runtime operations.",
                        Some(path),
                    );
                }
            }
            "match" => {
                in_host = false;
                builder.finding(
                    "unsupported_semantics",
                    "error",
                    "config",
                    format!("{}:{} uses Match semantics", path.display(), line_number + 1),
                    "Move the exact HostEntry out of Match or use a config shape sshx can preserve exactly.",
                    Some(path),
                );
            }
            "include" => {
                let patterns = discovery::tokenize(argument);
                if in_host {
                    builder.finding(
                        "unsupported_semantics",
                        "error",
                        "config",
                        format!("{}:{} includes a file inside a Host block", path.display(), line_number + 1),
                        "Move Include before the exact Host block; conditional Host Include cannot be preserved safely.",
                        Some(path),
                    );
                }
                for pattern in patterns {
                    if pattern.contains('%') {
                        builder.finding(
                            "unsupported_semantics",
                            "error",
                            "config",
                            format!(
                                "{}:{} uses tokenized Include `{pattern}`",
                                path.display(),
                                line_number + 1
                            ),
                            "Replace tokenized Include with a stable path before using sshx.",
                            Some(path),
                        );
                        continue;
                    }
                    let matches = discovery::expand_include(&pattern, &home.join(".ssh"));
                    if matches.is_empty() {
                        builder.finding(
                            "include_missing",
                            "error",
                            "config",
                            format!(
                                "{}:{} Include target `{pattern}` does not exist",
                                path.display(),
                                line_number + 1
                            ),
                            "Create the target file or remove the stale Include directive.",
                            Some(path),
                        );
                    }
                    for child in matches {
                        check_file_include(&child, home, scanner, builder, false);
                    }
                }
            }
            _ if !in_host => {
                builder.finding(
                    "unsupported_semantics",
                    "error",
                    "config",
                    format!("{}:{} has global directive `{keyword}`", path.display(), line_number + 1),
                    "Keep global directives in system SSH config or use an exact Host block that sshx can preserve.",
                    Some(path),
                );
            }
            _ => {
                if argument.contains('%') || !supported_directive(keyword) {
                    builder.finding(
                        "unsupported_semantics",
                        "error",
                        "config",
                        format!("{}:{} cannot be preserved exactly: `{keyword}`", path.display(), line_number + 1),
                        "Replace token-sensitive or unsupported directives with supported exact Host semantics.",
                        Some(path),
                    );
                }
            }
        }
    }
    scanner.active.pop();
}

fn scan_file(
    path: &Path,
    home: &Path,
    scanner: &mut ScanState,
    builder: &mut ReportBuilder,
    is_root: bool,
) {
    check_file_include(path, home, scanner, builder, is_root);
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let split = line
        .find(char::is_whitespace)
        .or_else(|| line.find('='))
        .unwrap_or(line.len());
    let keyword = &line[..split];
    if keyword.is_empty() {
        return None;
    }
    let argument = line[split..]
        .trim_start_matches(char::is_whitespace)
        .trim_start_matches('=')
        .trim();
    Some((keyword, argument))
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|token| token.trim_matches(['\'', '"']).to_string())
        .collect()
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

fn wildcard(value: &str) -> bool {
    value.starts_with('!')
        || value
            .chars()
            .any(|character| matches!(character, '*' | '?' | '['))
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        lexical_normalize(path)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn display_path(path: &Path) -> String {
    lexical_normalize(path).to_string_lossy().into_owned()
}

/// Permission findings that remain unsafe until repaired. Callers combine
/// this with repair results to decide the `--fix-permissions` exit status.
pub fn unsafe_permission_findings(report: &DoctorReport) -> Vec<&Finding> {
    report
        .findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.code.as_str(),
                "app_state_insecure" | "known_hosts_insecure" | "password_file_permissions"
            )
        })
        .collect()
}
