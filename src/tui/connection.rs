use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshx::session::{DeclaredService, ServiceForward};

use super::connection_view::draw_connection_workspace;
use super::terminal::{read_key, with_terminal};
use super::{BACK, CANCELLED};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConnectionMode {
    Session,
    Tunnel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceFocus {
    Route,
    Row,
    Status,
}

pub(crate) struct ConnectionSelection {
    pub mode: ConnectionMode,
    pub forwards: Vec<ServiceForward>,
    pub rows: Vec<ServiceForward>,
    pub checked: Vec<bool>,
    pub next_custom_id: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn connection_workspace(
    route: &str,
    is_pair: bool,
    services: &[DeclaredService],
    preselected: &[ServiceForward],
    restored: Option<&ConnectionSelection>,
    initial_mode: ConnectionMode,
    warning: Option<&str>,
    allow_bind: bool,
    has_active_tunnel: impl Fn(&[ServiceForward]) -> bool,
) -> Result<ConnectionSelection, String> {
    let mut services = services.to_vec();
    let mut forwards = services
        .iter()
        .map(|service| ServiceForward {
            id: service.id.clone(),
            remote_port: service.remote_port,
            destination_host: service.destination_host.clone(),
            local_port: service.default_local_port,
        })
        .collect::<Vec<_>>();
    let mut next_custom_id = restored.map_or(1, |selection| selection.next_custom_id);
    next_custom_id = next_custom_id.max(
        services
            .iter()
            .filter_map(|service| service.id.strip_prefix("custom ")?.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );
    let mut checked = vec![false; services.len()];
    if let Some(restored) = restored {
        let declared_count = services
            .iter()
            .take_while(|service| {
                !service.id.starts_with("custom ")
                    && !service.id.starts_with("local:")
                    && !service.id.starts_with("remote:")
                    && !service.id.starts_with("socks:")
            })
            .count();
        if restored.rows.len() < declared_count
            || restored.checked.len() != restored.rows.len()
            || services[..declared_count]
                .iter()
                .zip(&restored.rows)
                .any(|(service, row)| {
                    service.id != row.id
                        || service.remote_port != row.remote_port
                        || service.destination_host != row.destination_host
                })
        {
            return Err("FORWARD_SELECTION_INVALID: declared services changed".to_string());
        }
        services = restored
            .rows
            .iter()
            .map(|row| DeclaredService {
                id: row.id.clone(),
                remote_port: row.remote_port,
                destination_host: row.destination_host.clone(),
                default_local_port: row.local_port,
                name: None,
            })
            .collect();
        forwards = restored.rows.clone();
        checked = restored.checked.clone();
    } else {
        for requested in preselected {
            let index = services
                .iter()
                .position(|service| service.id == requested.id)
                .ok_or_else(|| {
                    "FORWARD_SELECTION_INVALID: requested service is no longer declared".to_string()
                })?;
            if checked[index] {
                return Err(format!(
                    "FORWARD_DUPLICATE: service `{}` was requested more than once",
                    requested.id
                ));
            }
            checked[index] = true;
            forwards[index].local_port = requested.local_port;
        }
    }

    let mut selected = 0usize;
    let mut mode = restored.map_or(initial_mode, |restored| restored.mode);
    let mut editing: Option<(usize, String)> = None;
    let mut new_custom_row = None;
    let mut edit_row_errors: Option<(usize, Vec<String>)> = None;
    let mut review = false;
    let mut status = warning.map(str::to_string);
    let mut row_errors = vec![Vec::new(); services.len()];
    let mut status_scroll = 0u16;
    let mut route_scroll = 0u16;
    let mut row_scroll = 0u16;
    let mut focus = if status.is_some() {
        WorkspaceFocus::Status
    } else if services.is_empty() {
        WorkspaceFocus::Route
    } else {
        WorkspaceFocus::Row
    };

    with_terminal(
        "Connection workspace",
        "CONNECTION_REQUIRED: connection workspace requires a usable interactive terminal"
            .to_string(),
        |terminal, input| loop {
            draw_connection_workspace(
                terminal,
                route,
                &services,
                &forwards,
                &checked,
                selected,
                &editing,
                &row_errors,
                mode,
                status.as_deref(),
                &mut status_scroll,
                &mut route_scroll,
                &mut row_scroll,
                focus,
                review,
            )?;
            let key = read_key(input, "CONNECTION_REQUIRED")?;
            if editing.is_none()
                && !matches!(key.code, KeyCode::PageUp | KeyCode::PageDown | KeyCode::Tab)
            {
                status_scroll = 0;
                route_scroll = 0;
                row_scroll = 0;
            }
            match key {
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } if let Some((index, value)) = editing.as_ref() => {
                    let index = *index;
                    if services[index].id.starts_with("custom ")
                        || services[index].id.starts_with("local:")
                    {
                        match sshx::session::resolve_custom_local_forwards(
                            std::slice::from_ref(value),
                            allow_bind,
                        ) {
                            Ok(mut parsed) => {
                                let mut forward = parsed.remove(0);
                                if services[index].id.starts_with("custom ")
                                    && !forward.id.starts_with("local:")
                                {
                                    forward.id = services[index].id.clone();
                                }
                                services[index].id = forward.id.clone();
                                services[index].destination_host = forward.destination_host.clone();
                                services[index].remote_port = forward.remote_port;
                                services[index].default_local_port = forward.local_port;
                                forwards[index] = forward;
                                checked[index] = true;
                                editing = None;
                                new_custom_row = None;
                                edit_row_errors = None;
                                clear_preflight_errors(&mut row_errors);
                                status = None;
                                review = false;
                                focus = WorkspaceFocus::Row;
                            }
                            Err(error) => {
                                row_errors[index] = vec![error];
                                status = Some(
                                    "Fix custom forward as [bind:]local:host:remote.".to_string(),
                                );
                                focus = WorkspaceFocus::Row;
                            }
                        }
                    } else if services[index].id.starts_with("remote:")
                        || services[index].id.starts_with("socks:")
                    {
                        let is_remote = services[index].id.starts_with("remote:");
                        let parsed = if is_remote {
                            sshx::tunnel::parse_forwards(
                                &[],
                                std::slice::from_ref(value),
                                &[],
                                allow_bind,
                            )
                        } else {
                            sshx::tunnel::parse_forwards(
                                &[],
                                &[],
                                std::slice::from_ref(value),
                                allow_bind,
                            )
                        };
                        match parsed {
                            Ok(mut parsed) => {
                                let parsed = parsed.remove(0);
                                let id = format!(
                                    "{}:{}",
                                    if is_remote { "remote" } else { "socks" },
                                    if is_remote {
                                        parsed.requested.clone()
                                    } else {
                                        parsed.effective.clone()
                                    }
                                );
                                services[index].id = id.clone();
                                services[index].destination_host =
                                    parsed.remote_host.unwrap_or_default();
                                services[index].remote_port =
                                    parsed.remote_port.unwrap_or_default();
                                services[index].default_local_port = if is_remote {
                                    forward_listener_port(&parsed.effective).unwrap_or(1)
                                } else {
                                    parsed.local_port.unwrap_or(1)
                                };
                                forwards[index] = ServiceForward {
                                    id,
                                    remote_port: services[index].remote_port,
                                    destination_host: services[index].destination_host.clone(),
                                    local_port: services[index].default_local_port,
                                };
                                checked[index] = true;
                                editing = None;
                                new_custom_row = None;
                                edit_row_errors = None;
                                clear_preflight_errors(&mut row_errors);
                                status = None;
                                review = false;
                                focus = WorkspaceFocus::Row;
                            }
                            Err(error) => {
                                row_errors[index] = vec![error];
                                status = Some("Fix forward spec; see -R or -D syntax.".to_string());
                                focus = WorkspaceFocus::Row;
                            }
                        }
                    } else {
                        match value.parse::<u16>().ok().filter(|port| *port > 0) {
                            Some(port) => {
                                forwards[index].local_port = port;
                                checked[index] = true;
                                editing = None;
                                edit_row_errors = None;
                                clear_preflight_errors(&mut row_errors);
                                status = None;
                                review = false;
                                focus = WorkspaceFocus::Row;
                            }
                            None => {
                                row_errors[index] =
                                    vec!["Local port must be between 1 and 65535".to_string()];
                                status = Some("Fix the selected local port.".to_string());
                                focus = WorkspaceFocus::Row;
                            }
                        }
                    }
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } if review => {
                    let selected_forwards = checked
                        .iter()
                        .enumerate()
                        .filter(|(_, selected)| **selected)
                        .map(|(index, _)| forwards[index].clone())
                        .collect();
                    return Ok(ConnectionSelection {
                        mode,
                        forwards: selected_forwards,
                        rows: forwards,
                        checked,
                        next_custom_id,
                    });
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } => {
                    let selected_forwards = checked
                        .iter()
                        .enumerate()
                        .filter(|(_, selected)| **selected)
                        .map(|(index, _)| forwards[index].clone())
                        .collect::<Vec<_>>();
                    if mode == ConnectionMode::Tunnel && selected_forwards.is_empty() {
                        status =
                            Some("Tunnel mode requires at least one local forward.".to_string());
                        focus = WorkspaceFocus::Status;
                        continue;
                    }
                    if is_pair
                        && selected_forwards.iter().any(|forward| {
                            forward.id.starts_with("custom ")
                                || forward.id.starts_with("local:")
                                || forward.id.starts_with("remote:")
                                || forward.id.starts_with("socks:")
                        })
                    {
                        status = Some(
                            "Pair routes accept declared VM services only; remove custom local, remote, and SOCKS rows or leave them unchecked."
                                .to_string(),
                        );
                        focus = WorkspaceFocus::Status;
                        continue;
                    }
                    for errors in &mut row_errors {
                        errors.clear();
                    }
                    let issues = if mode == ConnectionMode::Tunnel
                        && has_active_tunnel(&selected_forwards)
                    {
                        Vec::new()
                    } else {
                        workspace_preflight_issues(&selected_forwards, allow_bind)
                    };
                    if issues.is_empty() {
                        review = true;
                        let review_prompt = format!(
                            "Review: {} mode, {} forwarding row(s). Enter confirms; Esc edits.",
                            match mode {
                                ConnectionMode::Session => "Session",
                                ConnectionMode::Tunnel => "Tunnel",
                            },
                            selected_forwards.len()
                        );
                        status = Some(warning.map_or_else(
                            || review_prompt.clone(),
                            |warning| format!("{warning}\n{review_prompt}"),
                        ));
                        focus = WorkspaceFocus::Status;
                    } else {
                        for issue in issues {
                            if let Some(index) = forwards
                                .iter()
                                .position(|forward| forward.id == issue.service_id)
                            {
                                row_errors[index].push(issue.message);
                            }
                        }
                        status = Some(
                            "Fix marked rows before starting; no master was started.".to_string(),
                        );
                        focus = WorkspaceFocus::Status;
                    }
                }
                KeyEvent {
                    code: KeyCode::Esc, ..
                } if let Some((index, _)) = editing.take() => {
                    if new_custom_row.take() == Some(index) {
                        services.remove(index);
                        forwards.remove(index);
                        checked.remove(index);
                        row_errors.remove(index);
                        selected = selected.min(services.len().saturating_sub(1));
                        edit_row_errors = None;
                    } else {
                        restore_edit_row_errors(&mut row_errors, &mut edit_row_errors);
                    }
                    status = None;
                    focus = WorkspaceFocus::Row;
                    if review {
                        review = false;
                    }
                }
                KeyEvent {
                    code: KeyCode::Esc, ..
                } if review => {
                    review = false;
                    status = None;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => return Err(BACK.to_string()),
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(CANCELLED.to_string());
                }
                KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                } if let Some((_, value)) = editing.as_mut() => {
                    value.pop();
                }
                KeyEvent {
                    code: KeyCode::Char(character),
                    modifiers,
                    ..
                } if let Some((index, value)) = editing.as_mut()
                    && (services[*index].id.starts_with("custom ")
                        || services[*index].id.starts_with("local:")
                        || services[*index].id.starts_with("remote:")
                        || services[*index].id.starts_with("socks:")
                        || character.is_ascii_alphanumeric())
                    && !character.is_control()
                    && !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    value.push(character);
                }
                KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                } => match focus {
                    WorkspaceFocus::Route => {
                        route_scroll = route_scroll.saturating_sub(1);
                    }
                    WorkspaceFocus::Row => {
                        row_scroll = row_scroll.saturating_sub(1);
                    }
                    WorkspaceFocus::Status => {
                        status_scroll = status_scroll.saturating_sub(1);
                    }
                },
                KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                } => match focus {
                    WorkspaceFocus::Route => {
                        route_scroll = route_scroll.saturating_add(1);
                    }
                    WorkspaceFocus::Row => {
                        row_scroll = row_scroll.saturating_add(1);
                    }
                    WorkspaceFocus::Status => {
                        status_scroll = status_scroll.saturating_add(1);
                    }
                },
                KeyEvent {
                    code: KeyCode::Tab, ..
                } if editing.is_none() => {
                    focus = match focus {
                        WorkspaceFocus::Route => WorkspaceFocus::Row,
                        WorkspaceFocus::Row if status.is_some() => WorkspaceFocus::Status,
                        WorkspaceFocus::Row | WorkspaceFocus::Status => WorkspaceFocus::Route,
                    };
                }
                KeyEvent {
                    code: KeyCode::Up, ..
                } if editing.is_none() && review => {
                    selected = selected.saturating_sub(1);
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                } if editing.is_none() && review && selected + 1 < services.len() => {
                    selected += 1;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Up, ..
                } if editing.is_none() => {
                    selected = selected.saturating_sub(1);
                    status = None;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                } if editing.is_none() && selected + 1 < services.len() => {
                    selected += 1;
                    status = None;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Char('m'),
                    ..
                } if editing.is_none() => {
                    mode = match mode {
                        ConnectionMode::Session => ConnectionMode::Tunnel,
                        ConnectionMode::Tunnel => ConnectionMode::Session,
                    };
                    review = false;
                    status = None;
                    focus = WorkspaceFocus::Route;
                }
                KeyEvent {
                    code: KeyCode::Char('a'),
                    ..
                } if editing.is_none() && !review => {
                    add_workspace_row(
                        is_pair,
                        'L',
                        &mut services,
                        &mut forwards,
                        &mut checked,
                        &mut row_errors,
                        &mut selected,
                        &mut editing,
                        &mut new_custom_row,
                        &mut edit_row_errors,
                        &mut next_custom_id,
                        &mut status,
                        &mut focus,
                    );
                }
                KeyEvent {
                    code: KeyCode::Char('r'),
                    ..
                } if editing.is_none() && !review => {
                    add_workspace_row(
                        is_pair,
                        'R',
                        &mut services,
                        &mut forwards,
                        &mut checked,
                        &mut row_errors,
                        &mut selected,
                        &mut editing,
                        &mut new_custom_row,
                        &mut edit_row_errors,
                        &mut next_custom_id,
                        &mut status,
                        &mut focus,
                    );
                }
                KeyEvent {
                    code: KeyCode::Char('d'),
                    ..
                } if editing.is_none() && !review => {
                    add_workspace_row(
                        is_pair,
                        'D',
                        &mut services,
                        &mut forwards,
                        &mut checked,
                        &mut row_errors,
                        &mut selected,
                        &mut editing,
                        &mut new_custom_row,
                        &mut edit_row_errors,
                        &mut next_custom_id,
                        &mut status,
                        &mut focus,
                    );
                }
                KeyEvent {
                    code: KeyCode::Char('x'),
                    ..
                } if editing.is_none()
                    && !review
                    && services.get(selected).is_some_and(|service| {
                        service.id.starts_with("custom ")
                            || service.id.starts_with("local:")
                            || service.id.starts_with("remote:")
                            || service.id.starts_with("socks:")
                    }) =>
                {
                    services.remove(selected);
                    forwards.remove(selected);
                    checked.remove(selected);
                    row_errors.remove(selected);
                    selected = selected.min(services.len().saturating_sub(1));
                    review = false;
                    status = None;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Char(' '),
                    ..
                } if editing.is_none() && !services.is_empty() => {
                    checked[selected] = !checked[selected];
                    clear_preflight_errors(&mut row_errors);
                    review = false;
                    status = None;
                    focus = WorkspaceFocus::Row;
                }
                KeyEvent {
                    code: KeyCode::Char('e'),
                    ..
                } if editing.is_none() && !services.is_empty() => {
                    edit_row_errors = Some((selected, row_errors[selected].clone()));
                    let id = &services[selected].id;
                    let value = if id.starts_with("custom ") {
                        let mut value = String::new();
                        sshx::session::write_local_forward_spec(&mut value, &forwards[selected]);
                        value
                    } else if let Some(specification) = id.strip_prefix("local:") {
                        specification.to_string()
                    } else if id.starts_with("remote:") || id.starts_with("socks:") {
                        id.split_once(':')
                            .map_or_else(String::new, |(_, value)| value.to_string())
                    } else {
                        forwards[selected].local_port.to_string()
                    };
                    editing = Some((selected, value));
                    review = false;
                    status = Some(if id.starts_with("remote:") {
                        "Edit -R [bind:]listen:your-side-host:your-side-port; server bind may expose service. OpenSSH reports startup failure.".to_string()
                    } else if id.starts_with("socks:") {
                        "Edit -D [bind:]port; applications must configure this SOCKS proxy."
                            .to_string()
                    } else if id.starts_with("local:") {
                        "Edit local row [bind:]local:host:remote; Enter saves, Esc cancels."
                            .to_string()
                    } else if id.starts_with("custom ") {
                        "Edit custom row [bind:]local:host:remote; Enter saves, Esc cancels."
                            .to_string()
                    } else {
                        "Edit local listener port; Enter saves, Esc cancels.".to_string()
                    });
                    focus = if id.starts_with("custom ")
                        || id.starts_with("local:")
                        || id.starts_with("remote:")
                        || id.starts_with("socks:")
                    {
                        WorkspaceFocus::Status
                    } else {
                        WorkspaceFocus::Row
                    };
                }
                _ => {}
            }
        },
    )
}

fn normalized_listener_bind(bind: &str) -> &str {
    if bind == "localhost" {
        "127.0.0.1"
    } else {
        bind
    }
}

fn workspace_preflight_issues(
    forwards: &[ServiceForward],
    allow_bind: bool,
) -> Vec<sshx::session::ForwardIssue> {
    let local = forwards
        .iter()
        .filter(|forward| !forward.id.starts_with("remote:") && !forward.id.starts_with("socks:"))
        .cloned()
        .collect::<Vec<_>>();
    let mut issues = sshx::session::preflight_issues(&local);
    let dynamic = forwards
        .iter()
        .filter_map(|forward| {
            let value = forward.id.strip_prefix("socks:")?;
            let parsed = sshx::tunnel::parse_forwards(&[], &[], &[value.to_string()], allow_bind);
            match parsed {
                Ok(mut parsed) => parsed.pop().map(|parsed| (forward, parsed)),
                Err(message) => {
                    issues.push(sshx::session::ForwardIssue {
                        service_id: forward.id.clone(),
                        message,
                    });
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    for (index, (forward, spec)) in dynamic.iter().enumerate() {
        let (bind, port) = spec.listener().expect("parsed SOCKS forward has listener");
        let bind = normalized_listener_bind(&bind);
        for local_forward in &local {
            if bind == "127.0.0.1" && local_forward.local_port == port {
                for service_id in [&forward.id, &local_forward.id] {
                    issues.push(sshx::session::ForwardIssue {
                        service_id: service_id.clone(),
                        message: format!(
                            "SERVICE_BIND_FAILED: local listener {bind}:{port} is requested by multiple rows"
                        ),
                    });
                }
            }
        }
        for (other, other_spec) in &dynamic[..index] {
            if other_spec
                .listener()
                .is_some_and(|(other_bind, other_port)| {
                    normalized_listener_bind(&other_bind) == bind && other_port == port
                })
            {
                for service_id in [&forward.id, &other.id] {
                    issues.push(sshx::session::ForwardIssue {
                        service_id: service_id.clone(),
                        message: format!(
                            "FORWARD_DUPLICATE: local listener {bind}:{port} was requested more than once"
                        ),
                    });
                }
            }
        }
        if let Err(message) = sshx::tunnel::reserve_forwards(std::slice::from_ref(spec)) {
            issues.push(sshx::session::ForwardIssue {
                service_id: forward.id.clone(),
                message,
            });
        }
    }
    issues
}

#[allow(clippy::too_many_arguments)]
fn add_workspace_row(
    is_pair: bool,
    kind: char,
    services: &mut Vec<DeclaredService>,
    forwards: &mut Vec<ServiceForward>,
    checked: &mut Vec<bool>,
    row_errors: &mut Vec<Vec<String>>,
    selected: &mut usize,
    editing: &mut Option<(usize, String)>,
    new_row: &mut Option<usize>,
    edit_row_errors: &mut Option<(usize, Vec<String>)>,
    next_custom_id: &mut usize,
    status: &mut Option<String>,
    focus: &mut WorkspaceFocus,
) {
    if is_pair {
        *status = Some(
            "Pair routes accept declared VM services only; custom local, remote, and SOCKS rows are not allowed."
                .to_string(),
        );
        *focus = WorkspaceFocus::Status;
        return;
    }
    let index = services.len();
    let (id, value, host, port) = match kind {
        'L' => {
            let id = format!("custom {}", *next_custom_id);
            *next_custom_id = next_custom_id.saturating_add(1);
            (id, String::new(), "127.0.0.1".to_string(), 1)
        }
        'R' => (
            "remote:127.0.0.1:1:127.0.0.1:1".to_string(),
            "127.0.0.1:1:127.0.0.1:1".to_string(),
            "127.0.0.1".to_string(),
            1,
        ),
        _ => (
            "socks:127.0.0.1:1".to_string(),
            "127.0.0.1:1".to_string(),
            String::new(),
            1,
        ),
    };
    services.push(DeclaredService {
        id: id.clone(),
        remote_port: if kind == 'R' { 1 } else { 0 },
        destination_host: host.clone(),
        default_local_port: port,
        name: None,
    });
    forwards.push(ServiceForward {
        id,
        remote_port: if kind == 'R' { 1 } else { 0 },
        destination_host: host,
        local_port: port,
    });
    checked.push(false);
    row_errors.push(Vec::new());
    *selected = index;
    *editing = Some((index, value));
    *new_row = Some(index);
    *edit_row_errors = Some((index, Vec::new()));
    *status = Some(match kind {
        'L' => "Enter custom row as [bind:]local:host:remote.".to_string(),
        'R' => "Enter -R [bind:]listen:your-side-host:your-side-port. Server bind may expose service; OpenSSH reports startup failures.".to_string(),
        _ => "Enter -D [bind:]port. Applications must configure the local SOCKS proxy.".to_string(),
    });
    *focus = WorkspaceFocus::Status;
}

fn forward_listener_port(specification: &str) -> Option<u16> {
    let mut bracketed = false;
    for (index, character) in specification.char_indices() {
        match character {
            '[' => bracketed = true,
            ']' => bracketed = false,
            ':' if !bracketed => {
                let rest = &specification[index + 1..];
                return rest.split(':').next()?.parse().ok();
            }
            _ => {}
        }
    }
    None
}

fn restore_edit_row_errors(
    row_errors: &mut [Vec<String>],
    edit_row_errors: &mut Option<(usize, Vec<String>)>,
) {
    if let Some((index, errors)) = edit_row_errors.take() {
        row_errors[index] = errors;
    }
}

fn clear_preflight_errors(row_errors: &mut [Vec<String>]) {
    for errors in row_errors {
        errors.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_preflight_errors, restore_edit_row_errors, workspace_preflight_issues};

    #[test]
    fn cancelling_port_edit_restores_preflight_errors_and_clears_local_error() {
        let local_error = "Local port must be between 1 and 65535".to_string();
        let preflight_error = "SERVICE_BIND_FAILED: local service port is occupied".to_string();
        let other_error = "another row error".to_string();
        let mut row_errors = vec![vec![local_error], vec![other_error.clone()]];
        let mut saved_errors = Some((0, vec![preflight_error.clone()]));
        restore_edit_row_errors(&mut row_errors, &mut saved_errors);
        assert_eq!(row_errors, vec![vec![preflight_error], vec![other_error]]);
        assert!(saved_errors.is_none());

        let mut row_errors = vec![vec!["Local port must be between 1 and 65535".to_string()]];
        let mut saved_errors = Some((0, Vec::new()));
        restore_edit_row_errors(&mut row_errors, &mut saved_errors);
        assert!(row_errors[0].is_empty());
    }

    #[test]
    fn clearing_preflight_errors_removes_stale_conflicts_from_all_rows() {
        let mut row_errors = vec![
            vec!["SERVICE_BIND_FAILED: duplicate local listener".to_string()],
            vec!["SERVICE_BIND_FAILED: duplicate local listener".to_string()],
        ];
        clear_preflight_errors(&mut row_errors);
        assert_eq!(row_errors, vec![Vec::<String>::new(), Vec::new()]);
    }

    fn socks(bind: &str) -> sshx::session::ServiceForward {
        sshx::session::ServiceForward {
            id: format!("socks:{bind}"),
            remote_port: 0,
            destination_host: String::new(),
            local_port: 1080,
        }
    }

    #[test]
    fn socks_listener_duplicates_normalize_localhost_but_keep_ipv6_distinct() {
        let duplicate =
            workspace_preflight_issues(&[socks("localhost:1080"), socks("127.0.0.1:1080")], false);
        assert!(
            duplicate
                .iter()
                .any(|issue| issue.message.starts_with("FORWARD_DUPLICATE:"))
        );

        let distinct =
            workspace_preflight_issues(&[socks("127.0.0.1:1080"), socks("[::1]:1080")], false);
        assert!(
            distinct
                .iter()
                .all(|issue| !issue.message.starts_with("FORWARD_DUPLICATE:"))
        );
    }
}
