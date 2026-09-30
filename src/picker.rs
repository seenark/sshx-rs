use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use sshx::discovery::HostEntry;
use sshx::settings::RegisteredRoot;
use sshx::session::{DeclaredService, ServiceForward};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, IsTerminal, Read};
use std::os::fd::{AsRawFd, FromRawFd};

pub const CANCELLED: &str = "PICKER_CANCELLED";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionMode {
    Session,
    Tunnel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceFocus {
    Route,
    Row,
    Status,
}

pub struct ConnectionSelection {
    pub mode: ConnectionMode,
    pub forwards: Vec<ServiceForward>,
    pub rows: Vec<ServiceForward>,
    pub checked: Vec<bool>,
    pub next_custom_id: usize,
}

type AppTerminal = Terminal<CrosstermBackend<io::Stderr>>;

struct TerminalGuard {
    raw_mode: bool,
    alternate_screen: bool,
    cursor_hidden: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        let mut stderr = io::stderr();
        if self.alternate_screen {
            let _ = execute!(stderr, LeaveAlternateScreen);
        }
        if self.cursor_hidden {
            let _ = execute!(stderr, crossterm::cursor::Show);
        }
    }
}

pub struct Selection<'a> {
    pub entry: &'a HostEntry,
    pub alias: &'a str,
}

#[derive(Clone, Copy, Default)]
enum HostDetailPage {
    #[default]
    Summary,
    Identity,
    Source,
}

#[derive(Default)]
pub struct HostsState {
    query: String,
    selected: Option<(String, usize, usize, String)>,
    detail_page: HostDetailPage,
    detail_scroll: u16,
    pub active_tunnels: usize,
    pub edit_action: Option<sshx::mutation::MutationKind>,
    pub editing_enabled: bool,
}

impl HostsState {
    pub fn prefill(&mut self, entry: &HostEntry, alias: &str, query: &str) {
        self.query = query.to_string();
        self.selected = Some((
            entry.source.path.clone(),
            entry.source.byte_start,
            entry.source.byte_end,
            alias.to_string(),
        ));
    }
}

pub struct MenuOption<'a> {
    pub label: &'a str,
    pub description: &'a str,
}

impl<'a> MenuOption<'a> {
    pub const fn new(label: &'a str, description: &'a str) -> Self {
        Self { label, description }
    }
}

struct Row<'a> {
    entry: &'a HostEntry,
    alias: &'a str,
}

pub fn select<'a>(entries: &[&'a HostEntry], label: &str) -> Result<Selection<'a>, String> {
    if entries.is_empty() {
        return Err("HOST_NOT_FOUND: no hosts match current filters".to_string());
    }

    with_terminal(
        label,
        format!(
            "HOST_REQUIRED: {label} requires a HostEntry selector with a usable interactive terminal"
        ),
        |terminal, input| {
            let mut query = String::new();
            let mut selected = 0usize;
            let mut detail_page = HostDetailPage::Summary;
            let mut detail_scroll = 0u16;

            loop {
                let rows = matching_rows(entries, &query);
                selected = selected.min(rows.len().saturating_sub(1));
                let width = terminal.backend().size().map_or(80, |area| area.width);
                let footer = if width < 24 {
                    "↑↓ move\nTab details\nPgUp/Dn scroll\nEnter select\nEsc cancel"
                } else if width < 32 {
                    "↑↓ move · type search\nTab details\nPgUp/Dn scroll\nEnter select\nEsc cancel"
                } else if width < 80 {
                    "↑↓ move · type search\nTab details · PgUp/Dn scroll\nEnter select · Esc cancel"
                } else {
                    "↑↓ move  Type search  Backspace erase  Tab details\nPgUp/Dn scroll  Enter select  Esc/Ctrl-C cancel"
                };
                let scroll_step = terminal.backend().size().map_or(1, |area| {
                    area.height
                        .saturating_sub(footer.lines().count() as u16)
                        .saturating_sub(1)
                        .max(1)
                });
                draw_host_picker(
                    terminal,
                    &rows,
                    &query,
                    selected,
                    label,
                    "No matching HostEntry aliases.",
                    None,
                    None,
                    detail_page,
                    detail_scroll,
                    footer,
                )?;

                match read_key(input, "HOST_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        if let Some(row) = rows.get(selected) {
                            return Ok(Selection {
                                entry: row.entry,
                                alias: row.alias,
                            });
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Err(CANCELLED.to_string()),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => detail_scroll = detail_scroll.saturating_sub(scroll_step),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => detail_scroll = detail_scroll.saturating_add(scroll_step),
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } => {
                        detail_page = match detail_page {
                            HostDetailPage::Summary => HostDetailPage::Identity,
                            HostDetailPage::Identity => HostDetailPage::Source,
                            HostDetailPage::Source => HostDetailPage::Summary,
                        };
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => {
                        selected = selected.saturating_sub(1);
                        detail_page = HostDetailPage::Summary;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if selected + 1 < rows.len() => {
                        selected += 1;
                        detail_page = HostDetailPage::Summary;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        query.pop();
                        selected = 0;
                        detail_page = HostDetailPage::Summary;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Char(character),
                        modifiers,
                        ..
                    } if !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        query.push(character);
                        selected = 0;
                        detail_page = HostDetailPage::Summary;
                        detail_scroll = 0;
                    }
                    _ => {}
                }
            }
        },
    )
}

pub fn browse_hosts<'a>(
    entries: &[&'a HostEntry],
    source_entries: &[HostEntry],
    state: &mut HostsState,
    status: Option<&str>,
) -> Result<Option<Selection<'a>>, String> {
    let mut route_selection = None;
    let mut route_display = String::new();
    with_terminal(
        "Hosts",
        "HOST_REQUIRED: sshx Hosts requires usable stdin and stderr terminals".to_string(),
        |terminal, input| loop {
            let rows = matching_rows(entries, &state.query);
            let selected = state
                .selected
                .as_ref()
                .and_then(|(path, start, end, alias)| {
                    rows.iter().position(|row| {
                        row.entry.source.path.as_str() == path.as_str()
                            && row.entry.source.byte_start == *start
                            && row.entry.source.byte_end == *end
                            && row.alias == alias.as_str()
                    })
                })
                .unwrap_or(0);
            remember_selection(state, &rows, selected);
            let current_route = rows.get(selected).map(|row| {
                (
                    row.entry.source.path.clone(),
                    row.entry.source.byte_start,
                    row.entry.source.byte_end,
                    row.alias.to_string(),
                )
            });
            if current_route != route_selection {
                route_display = rows.get(selected).map_or_else(String::new, |row| {
                    match sshx::pair::paired_route(source_entries, row.entry) {
                        Ok(Some(route)) => format!(
                            "Pair via {}",
                            route.gateway.aliases.first().map_or("-", String::as_str)
                        ),
                        Ok(None) => "Direct".to_string(),
                        Err(error) => format!("Unavailable: {error}"),
                    }
                });
                route_selection = current_route;
            }
            let empty_message = if entries.is_empty() && state.query.is_empty() {
                "No HostEntries found. Ctrl+S opens Setup to register a config root; Ctrl+D opens Doctor."
            } else {
                "No matching HostEntry aliases."
            };
            let width = terminal.backend().size().map_or(80, |area| area.width);
            let mut footer = hosts_footer(width, !rows.is_empty(), state.active_tunnels);
            if state.editing_enabled && !rows.is_empty() {
                footer.push_str(if width < 48 {
                    "\n^U Update · ^R Rename · ^X Delete"
                } else {
                    "\nCtrl+U Update · Ctrl+R Rename · Ctrl+X Delete"
                });
            }
            let scroll_step = terminal.backend().size().map_or(1, |area| {
                area.height
                    .saturating_sub(footer.lines().count() as u16)
                    .saturating_sub(1)
                    .max(1)
            });
            draw_host_picker(
                terminal,
                &rows,
                &state.query,
                selected,
                "Hosts",
                empty_message,
                status,
                Some(&route_display),
                state.detail_page,
                state.detail_scroll,
                &footer,
            )?;

            match read_key(input, "HOST_REQUIRED")? {
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } => {
                    if let Some(row) = rows.get(selected) {
                        state.detail_scroll = 0;
                        state.detail_page = HostDetailPage::Summary;
                        remember_selection(state, &rows, selected);
                        return Ok(Some(Selection {
                            entry: row.entry,
                            alias: row.alias,
                        }));
                    }
                }
                KeyEvent {
                    code: KeyCode::Char(character @ ('u' | 'r' | 'x')),
                    modifiers,
                    ..
                } if state.editing_enabled && modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(row) = rows.get(selected) {
                        state.edit_action = Some(match character {
                            'u' => sshx::mutation::MutationKind::Update,
                            'r' => sshx::mutation::MutationKind::Rename,
                            _ => sshx::mutation::MutationKind::Delete,
                        });
                        return Ok(Some(Selection { entry: row.entry, alias: row.alias }));
                    }
                }
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => return Ok(None),
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
                KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                } => state.detail_scroll = state.detail_scroll.saturating_sub(scroll_step),
                KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                } => state.detail_scroll = state.detail_scroll.saturating_add(scroll_step),
                KeyEvent {
                    code: KeyCode::Tab, ..
                } => {
                    state.detail_page = match state.detail_page {
                        HostDetailPage::Summary => HostDetailPage::Identity,
                        HostDetailPage::Identity => HostDetailPage::Source,
                        HostDetailPage::Source => HostDetailPage::Summary,
                    };
                    state.detail_scroll = 0;
                }
                KeyEvent {
                    code: KeyCode::Up, ..
                } => {
                    state.detail_page = HostDetailPage::Summary;
                    remember_selection(state, &rows, selected.saturating_sub(1));
                    state.detail_scroll = 0;
                }
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                } if selected + 1 < rows.len() => {
                    state.detail_page = HostDetailPage::Summary;
                    remember_selection(state, &rows, selected + 1);
                    state.detail_scroll = 0;
                }
                KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                } => {
                    state.detail_page = HostDetailPage::Summary;
                    state.query.pop();
                    state.detail_scroll = 0;
                    state.selected = None;
                }
                KeyEvent {
                    code: KeyCode::Char('n'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("HOST_CREATE".to_string());
                }
                KeyEvent {
                    code: KeyCode::Char('p'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("PAIRS_TAB".to_string());
                }
                KeyEvent {
                    code: KeyCode::Char('t'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("TUNNELS_TAB".to_string());
                }
                KeyEvent {
                    code: KeyCode::Char('s'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("SETUP_TAB".to_string());
                }
                KeyEvent {
                    code: KeyCode::Char('d'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("DOCTOR_TAB".to_string());
                }
                KeyEvent {
                    code: KeyCode::Char(character),
                    modifiers,
                    ..
                } if !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.detail_page = HostDetailPage::Summary;
                    state.query.push(character);
                }
                _ => {}
            }
        },
    )
}

pub fn tunnels_workspace(
    tunnels: &[sshx::tunnel::TunnelView],
    status: Option<&str>,
    initial_selected: usize,
    allowed: Option<char>,
) -> Result<(usize, char), String> {
    with_terminal(
        "Tunnels",
        "TUNNEL_REQUIRED: Tunnels requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let mut selected = initial_selected.min(tunnels.len().saturating_sub(1));
            let mut detail_scroll = 0u16;
            let mut detail_step = 1u16;
            loop {
                terminal
                    .draw(|frame| {
                        let width = frame.area().width;
                        let height = frame.area().height;
                        let tiny = width < 32 || height < 14;
                        let compact = tiny || width < 80 || height < 20;
                        let list_height = if tiny { 4 } else if compact { 6 } else { 7 };
                        let footer_height = if tiny { 5 } else if compact { 4 } else { 1 };
                        let actions = match allowed {
                            Some('s') => "s stop",
                            Some('r') => "r restart",
                            Some('v') => "Enter inspect",
                            _ => "s stop  r restart  Enter refresh",
                        };
                        let footer = if tiny {
                            format!(
                                "{}\n↑↓ select\n{}\nPgUp/Dn\nEsc cancel",
                                status.unwrap_or(""), actions,
                            )
                        } else if compact {
                            format!(
                                "{}\n↑↓ select  PgUp/Dn detail\n{}\nEsc cancel",
                                status.unwrap_or(""), actions,
                            )
                        } else {
                            format!("{}  ↑↓ select  PgUp/Dn detail  {}  Esc cancel", status.unwrap_or(""), actions)
                        };
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(list_height),
                                Constraint::Min(3),
                                Constraint::Length(footer_height),
                            ])
                            .split(frame.area());
                        let rows = tunnels
                            .iter()
                            .map(|tunnel| {
                                ListItem::new(format!(
                                    "{}  {}  {}  {}",
                                    tunnel.id, tunnel.state, tunnel.kind, tunnel.selected_alias
                                ))
                            })
                            .collect::<Vec<_>>();
                        let mut list_state = ListState::default();
                        if !tunnels.is_empty() {
                            list_state.select(Some(selected));
                        }
                        frame.render_stateful_widget(
                            List::new(rows)
                                .block(block(" Tunnels ", Color::Cyan))
                                .highlight_symbol("> ")
                                .highlight_style(
                                    Style::default()
                                        .bg(Color::DarkGray)
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            chunks[0],
                            &mut list_state,
                        );
                        let detail = tunnels.get(selected).map_or_else(
                            || "No registered tunnels.".to_string(),
                            |tunnel| {
                                let mappings = tunnel
                                    .forwards
                                    .iter()
                                    .map(|forward| format!("-{} {}", forward.kind, forward.effective))
                                    .collect::<Vec<_>>()
                                    .join("; ");
                                let pair_details = if tunnel.kind == "paired" {
                                    let transit_port = tunnel
                                        .transit_port
                                        .map_or_else(|| "unknown".to_string(), |port| port.to_string());
                                    format!(
                                        "\nGateway: {} ({})\nVM: {} ({})\nTransit: {}:{}",
                                        tunnel.gateway_alias.as_deref().unwrap_or("unknown"),
                                        tunnel.gateway_entry_id.as_deref().unwrap_or("unknown"),
                                        tunnel.vm_alias.as_deref().unwrap_or("unknown"),
                                        tunnel.vm_entry_id.as_deref().unwrap_or("unknown"),
                                        tunnel.transit_host.as_deref().unwrap_or("unknown"),
                                        transit_port
                                    )
                                } else {
                                    String::new()
                                };
                                let master_details = match (
                                    tunnel.gateway_master_status.as_deref(),
                                    tunnel.vm_master_status.as_deref(),
                                ) {
                                    (Some(gateway), Some(vm)) => {
                                        format!(" (gateway {gateway}, VM {vm})")
                                    }
                                    _ => String::new(),
                                };
                                format!(
                                    "ID: {}\nRoute: {} ({}){}\nHostEntry: {} at {}:{}\nAliases: {:?}\nMappings: {}\nMaster: {}{}\nListener: {}\nApplication health: not measured{}",
                                    tunnel.id,
                                    tunnel.kind,
                                    tunnel.selected_alias,
                                    pair_details,
                                    tunnel.entry_id,
                                    tunnel.source_path,
                                    tunnel.source_line,
                                    tunnel.aliases,
                                    mappings,
                                    tunnel.master_status,
                                    master_details,
                                    tunnel.listener_status,
                                    tunnel.error.as_ref().map_or_else(String::new, |error| format!("\nEvidence: {error}"))
                                )
                            },
                        );
                        detail_step = chunks[1].height.saturating_sub(2).max(1);
                        frame.render_widget(
                            Paragraph::new(detail)
                                .block(block(" Tunnel details ", Color::Cyan))
                                .wrap(Wrap { trim: false })
                                .scroll((detail_scroll, 0)),
                            chunks[1],
                        );
                        frame.render_widget(Paragraph::new(footer), chunks[2]);
                    })
                    .map_err(|error| format!("TUNNEL_REQUIRED: cannot render tunnel list: {error}"))?;
                match read_key(input, "TUNNEL_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc, .. } => return Err(CANCELLED.to_string()),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent { code: KeyCode::PageUp, .. } => {
                        detail_scroll = detail_scroll.saturating_sub(detail_step);
                    }
                    KeyEvent { code: KeyCode::PageDown, .. } => {
                        detail_scroll = detail_scroll.saturating_add(detail_step);
                    }
                    KeyEvent { code: KeyCode::Up, .. } => {
                        selected = selected.saturating_sub(1);
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Down, .. } if selected + 1 < tunnels.len() => {
                        selected += 1;
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Char('s'), .. } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 's') => {
                        return Ok((selected, 's'))
                    }
                    KeyEvent { code: KeyCode::Char('r'), .. } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 'r') => {
                        return Ok((selected, 'r'))
                    }
                    KeyEvent { code: KeyCode::Enter, .. } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 'v') => {
                        return Ok((selected, 'v'))
                    }
                    _ => {}
                }
            }
        },
    )
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairWorkspaceAction {
    Setup,
    Recover,
    Validate,
    Exit,
}

pub fn pairs_workspace(
    pairs: &[sshx::pair::PairRecord],
    entries: &[HostEntry],
    findings: &[sshx::discovery::Diagnostic],
    status: Option<&str>,
) -> Result<PairWorkspaceAction, String> {
    let guidance = |code: &str| match code {
        "malformed_id" => "Restore a valid immutable ID marker, then press V to validate.",
        "duplicate_id" | "PAIR_INVALID" => {
            "Give each HostEntry exactly one unique immutable ID; inspect every source in the evidence, then press V."
        }
        "broken_reference" | "PAIR_BROKEN" => {
            "Restore reciprocal gateway and VM ID references and approved transit metadata, then press V."
        }
        "pair_conflict" => {
            "Keep one reciprocal VM relationship per gateway; resolve the conflicting references, then press V."
        }
        "malformed_pair" | "PAIR_ROUTE_CHANGED" => {
            "Restore VM Port and exactly one gateway LocalForward to the approved transit destination, then press V."
        }
        "PAIR_ROUTE_UNSAFE" => {
            "Remove ProxyCommand and ProxyJump from the paired route and its inherited configuration, then press V."
        }
        "PAIR_RECOVERY_PENDING" => {
            "Press R to recover the pending Pair transaction before starting another setup."
        }
        _ => "Inspect the reported source and metadata, correct the finding, then press V to validate.",
    };
    let recovery_available = findings
        .iter()
        .any(|finding| finding.code == "PAIR_RECOVERY_PENDING");
    let mut labels = Vec::new();
    let mut bodies = Vec::new();
    for pair in pairs {
        let gateways = entries
            .iter()
            .filter(|entry| entry.id.eq_ignore_ascii_case(&pair.gateway_id))
            .collect::<Vec<_>>();
        let vms = entries
            .iter()
            .filter(|entry| entry.id.eq_ignore_ascii_case(&pair.vm_id))
            .collect::<Vec<_>>();
        let route_status = if vms.len() == 1 && gateways.len() == 1 {
            match sshx::pair::paired_route(entries, vms[0]) {
                Ok(Some(route))
                    if route.gateway_id.eq_ignore_ascii_case(&pair.gateway_id)
                        && route.vm_id.eq_ignore_ascii_case(&pair.vm_id)
                        && route.transit_host == pair.transit_host
                        && route.transit_port == pair.transit_port =>
                {
                    Ok(())
                }
                Ok(_) => Err("PAIR_BROKEN: reciprocal Pair record no longer matches the current route".to_string()),
                Err(error) => Err(error),
            }
        } else {
            Err("PAIR_INVALID: immutable IDs do not resolve to exactly one gateway and one VM".to_string())
        };
        let mut body = format!(
            "Status: {}\nApproved transit: {}:{}\n\nGateway\nID: {}\nRecorded alias: {}\n",
            if route_status.is_ok() { "valid" } else { "invalid" },
            pair.transit_host,
            pair.transit_port,
            pair.gateway_id,
            pair.gateway_alias,
        );
        for entry in &gateways {
            body.push_str(&format!(
                "Aliases: {}\nSource: {}\nHost line: {}\n",
                entry.aliases.join(", "),
                entry.source.path,
                entry.source.line_start,
            ));
        }
        if gateways.is_empty() {
            body.push_str("Source: unresolved immutable ID\n");
        }
        body.push_str(&format!(
            "\nVM\nID: {}\nRecorded alias: {}\n",
            pair.vm_id, pair.vm_alias,
        ));
        for entry in &vms {
            body.push_str(&format!(
                "Aliases: {}\nSource: {}\nHost line: {}\n",
                entry.aliases.join(", "),
                entry.source.path,
                entry.source.line_start,
            ));
        }
        if vms.is_empty() {
            body.push_str("Source: unresolved immutable ID\n");
        }
        if let Err(error) = &route_status {
            let code = error.split(':').next().unwrap_or_default();
            body.push_str(&format!(
                "\nEvidence: {error}\nGuidance: {}",
                guidance(code),
            ));
        }
        labels.push(format!(
            "{}: {} [{}] / {} [{}]",
            if route_status.is_ok() { "valid" } else { "invalid" },
            pair.gateway_alias,
            pair.gateway_id,
            pair.vm_alias,
            pair.vm_id,
        ));
        bodies.push(body);
    }
    let mut groups = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for finding in findings {
        groups
            .entry(finding.code.as_str())
            .or_default()
            .push(finding.message.as_str());
    }
    for (code, mut messages) in groups {
        messages.sort_unstable();
        let mut body = format!("Validation findings [{code}]\n");
        for message in &messages {
            body.push_str(&format!("\nEvidence: {message}\n"));
        }
        body.push_str(&format!("\nGuidance: {}", guidance(code)));
        labels.push(format!("[{code}] {} finding(s)", messages.len()));
        bodies.push(body);
    }
    if labels.is_empty() {
        labels.push("No exact reciprocal Pair records.".to_string());
        bodies.push(
            "No valid Pair relationships.\nNo validation findings.\nPress S to set up a Pair or V to validate current sources."
                .to_string(),
        );
    }
    let rows = labels
        .iter()
        .map(|label| ListItem::new(label.as_str()))
        .collect::<Vec<_>>();
    let footer = if recovery_available {
        "↑↓ select · PgUp/PgDn detail · V validate · R recover · setup blocked pending recovery · Esc Hosts"
    } else {
        "↑↓ select · PgUp/PgDn detail · V validate · S setup Pair · Esc Hosts"
    };
    let compact_footer = if recovery_available {
        "↑↓ select · PgUp/PgDn\nV validate · R recover\nS blocked · Esc Hosts"
    } else {
        "↑↓ select · PgUp/PgDn\nV validate · S setup\nEsc Hosts"
    };
    let list_title = if pairs.is_empty() {
        " Pairs: no exact records · findings "
    } else {
        " Pairs · validation findings "
    };
    with_terminal(
        "Pairs",
        "PAIR_REQUIRED: Pairs requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let mut selected = 0usize;
            let mut list_state = ListState::default();
            let mut scroll = 0u16;
            let mut detail_step = 1u16;
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let compact = area.width < 100 || area.height < 16;
                        let footer_height = if compact { 3 } else { 1 };
                        let list_height = (area.height / 3).clamp(1, 7);
                        let status_height = u16::from(status.is_some() && area.height >= 8);
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(list_height),
                                Constraint::Min(1),
                                Constraint::Length(status_height),
                                Constraint::Length(footer_height),
                            ])
                            .split(area);
                        list_state.select(Some(selected));
                        let tiny = area.height < 12 || area.width < 32;
                        let list = List::new(rows.iter().cloned())
                            .highlight_symbol("> ")
                            .highlight_style(
                                Style::default()
                                    .bg(Color::DarkGray)
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            );
                        frame.render_stateful_widget(
                            if tiny { list } else { list.block(block(list_title, Color::Cyan)) },
                            chunks[0],
                            &mut list_state,
                        );
                        detail_step = chunks[1].height.saturating_sub(if tiny { 0 } else { 2 }).max(1);
                        let detail = Paragraph::new(bodies[selected].as_str())
                            .wrap(Wrap { trim: false })
                            .scroll((scroll, 0));
                        frame.render_widget(
                            if tiny {
                                detail
                            } else {
                                detail.block(block(" Pair inspection ", Color::Cyan))
                            },
                            chunks[1],
                        );
                        if let Some(status) = status {
                            frame.render_widget(
                                Paragraph::new(status)
                                    .style(Style::default().fg(Color::Green))
                                    .wrap(Wrap { trim: false }),
                                chunks[2],
                            );
                        }
                        frame.render_widget(
                            Paragraph::new(if compact { compact_footer } else { footer }),
                            chunks[3],
                        );
                    })
                    .map_err(|error| format!("PAIR_REQUIRED: cannot render Pairs: {error}"))?;
                match read_key(input, "PAIR_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc, .. } => return Ok(PairWorkspaceAction::Exit),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(PairWorkspaceAction::Exit);
                    }
                    KeyEvent { code: KeyCode::Char('v' | 'V'), .. } => {
                        return Ok(PairWorkspaceAction::Validate);
                    }
                    KeyEvent { code: KeyCode::Char('s' | 'S'), .. } if recovery_available => {}
                    KeyEvent { code: KeyCode::Char('s' | 'S'), .. } => {
                        return Ok(PairWorkspaceAction::Setup);
                    }
                    KeyEvent { code: KeyCode::Char('r' | 'R'), .. } if recovery_available => {
                        return Ok(PairWorkspaceAction::Recover);
                    }
                    KeyEvent { code: KeyCode::Up, .. } => {
                        selected = selected.saturating_sub(1);
                        scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Down, .. } if selected + 1 < bodies.len() => {
                        selected += 1;
                        scroll = 0;
                    }
                    KeyEvent { code: KeyCode::PageUp, .. } => {
                        scroll = scroll.saturating_sub(detail_step);
                    }
                    KeyEvent { code: KeyCode::PageDown, .. } => {
                        scroll = scroll.saturating_add(detail_step);
                    }
                    _ => {}
                }
            }
        },
    )
}

#[derive(Clone, Debug, Default)]
pub struct PairSetupDraft {
    pub gateway: Option<(usize, usize)>,
    pub vm: Option<(usize, usize)>,
    pub transit_host: String,
    pub transit_port: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairSetupAction {
    Review,
    Apply,
}

fn pair_setup_entry<'a>(entries: &[&'a HostEntry], pick: Option<(usize, usize)>) -> Option<Row<'a>> {
    let (entry, alias) = pick?;
    let entry = *entries.get(entry)?;
    Some(Row { entry, alias: entry.aliases.get(alias)?.as_str() })
}

fn pair_setup_options(
    entries: &[&HostEntry],
    eligible: &[usize],
    query: &str,
) -> Vec<(usize, usize)> {
    let mut options = eligible.iter().copied()
        .flat_map(|index| (0..entries[index].aliases.len()).map(move |alias| (index, alias)))
        .filter(|pick| row_rank(&pair_setup_entry(entries, Some(*pick)).unwrap(), query).is_some())
        .collect::<Vec<_>>();
    options.sort_by(|left, right| compare_rows(
        &pair_setup_entry(entries, Some(*left)).unwrap(),
        &pair_setup_entry(entries, Some(*right)).unwrap(),
        query,
    ));
    options
}

fn pair_setup_routes(
    entries: &[&HostEntry],
    eligible: &[usize],
    gateway: Option<(usize, usize)>,
) -> Vec<(usize, Result<Vec<(String, u16)>, String>)> {
    let Some(gateway) = pair_setup_entry(entries, gateway) else { return Vec::new(); };
    eligible.iter().copied().filter(|index| {
        let source = &entries[*index].source;
        source.path != gateway.entry.source.path
            || source.byte_start != gateway.entry.source.byte_start
            || source.byte_end != gateway.entry.source.byte_end
    }).map(|index| (index, sshx::pair::transit_candidates(gateway.entry, entries[index]))).collect()
}

fn pair_setup_vm_eligible(
    eligible: &[usize],
    gateway: Option<(usize, usize)>,
    routes: &[(usize, Result<Vec<(String, u16)>, String>)],
) -> Vec<usize> {
    if gateway.is_none() { return eligible.to_vec(); }
    routes.iter().filter_map(|(index, result)| {
        result.as_ref().is_ok_and(|values| values.iter().any(|candidate| {
            values.iter().filter(|other| *other == candidate).count() == 1
        })).then_some(*index)
    }).collect()
}

pub fn pair_setup_workspace(
    entries: &[&HostEntry],
    mut draft: PairSetupDraft,
    preview: bool,
    mut execute: impl FnMut(&PairSetupDraft, PairSetupAction) -> Result<String, String>,
) -> Result<bool, String> {
    with_terminal(
        "Pair setup",
        "PAIR_REQUIRED: Pair setup requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let eligible = entries.iter().enumerate()
                .filter_map(|(index, entry)| sshx::pair::setup_eligible(entry).then_some(index))
                .collect::<Vec<_>>();
            let mut routes = pair_setup_routes(entries, &eligible, draft.gateway);
            let mut vm_eligible = pair_setup_vm_eligible(&eligible, draft.gateway, &routes);
            let empty_candidates = Ok(Vec::<(String, u16)>::new());
            let initial_candidates = draft.vm.and_then(|(index, _)| {
                routes.iter().find(|(entry, _)| *entry == index).map(|(_, result)| result)
            }).unwrap_or(&empty_candidates);
            let mut focus = if draft.gateway.is_none() {
                0
            } else if draft.vm.is_none() {
                1
            } else if draft.transit_host.is_empty() != draft.transit_port.is_empty()
                || (draft.transit_host.is_empty()
                    && !initial_candidates.as_ref().is_ok_and(|values| values.len() == 1))
            {
                2
            } else {
                0
            };
            let mut queries = [String::new(), String::new()];
            let mut selected = [0usize; 2];
            for index in 0..2 {
                let options = pair_setup_options(entries, if index == 0 { &eligible } else { &vm_eligible }, "");
                let pick = if index == 0 { draft.gateway } else { draft.vm };
                selected[index] = options.iter().position(|option| Some(*option) == pick).unwrap_or(0);
            }
            let mut list_state = ListState::default();
            let mut candidate_index = None::<usize>;
            let mut review = None::<String>;
            let mut status = None::<(String, bool)>;
            let mut completed = false;
            let mut scroll = 0u16;
            let mut scroll_step = 1u16;
            loop {
                let candidates = draft.vm.and_then(|(index, _)| {
                    routes.iter().find(|(entry, _)| *entry == index).map(|(_, result)| result)
                }).unwrap_or(&empty_candidates);
                let options = if focus < 2 && review.is_none() {
                    pair_setup_options(entries, if focus == 0 { &eligible } else { &vm_eligible }, &queries[focus])
                } else {
                    Vec::new()
                };
                if focus < 2 && review.is_none() {
                    selected[focus] = selected[focus].min(options.len().saturating_sub(1));
                }
                let gateway = pair_setup_entry(entries, draft.gateway);
                let vm = pair_setup_entry(entries, draft.vm);
                let mut details = String::new();
                for (label, row, pick) in [
                    ("Gateway", gateway.as_ref(), draft.gateway),
                    ("VM", vm.as_ref(), draft.vm),
                ] {
                    if let Some(row) = row {
                        details.push_str(&format!(
                            "{label}: {}\nSource: {}:{}\nID: {}\n",
                            row.alias, row.entry.source.path, row.entry.source.line_start, row.entry.id,
                        ));
                        if !eligible.iter().any(|index| std::ptr::eq(entries[*index], row.entry)) {
                            details.push_str("Ineligible for Pair setup; select another HostEntry.\n");
                        }
                        if label == "VM" && draft.gateway.is_some()
                            && !vm_eligible.iter().any(|index| std::ptr::eq(entries[*index], row.entry))
                        {
                            details.push_str("VM has no compatible unambiguous gateway transit; select another HostEntry.\n");
                        }
                    } else {
                        details.push_str(&format!("{label}: {}\n", if pick.is_some() {
                            "invalid selection; choose a current HostEntry"
                        } else {
                            "not selected"
                        }));
                    }
                }
                details.push('\n');
                match &candidates {
                    Ok(values) if values.len() == 1 && draft.transit_host.is_empty() && draft.transit_port.is_empty() => {
                        details.push_str(&format!("Automatic transit: {}:{} (unique inference)\n", values[0].0, values[0].1));
                    }
                    Ok(values) if !values.is_empty() => {
                        details.push_str("Transit candidates (Ctrl-N explicitly chooses next):\n");
                        for (index, (host, port)) in values.iter().enumerate() {
                            details.push_str(&format!("{} {host}:{port}\n", if Some(index) == candidate_index { ">" } else { " " }));
                        }
                        if values.len() > 1 && draft.transit_host.is_empty() && draft.transit_port.is_empty() {
                            details.push_str("Ambiguous transit: select a candidate or enter host and port.\n");
                        }
                    }
                    Ok(_) => details.push_str("No transit candidates: enter transit host and port.\n"),
                    Err(error) => details.push_str(&format!("Transit: {error}\n")),
                }
                if let Some(review) = &review {
                    details = format!("Review Pair changes\n{review}\n\n{details}");
                } else if let Some((text, _)) = &status {
                    details = format!("{text}\n\n{details}");
                }
                let success = status.as_ref().is_some_and(|(_, success)| *success);
                let applied = success && !preview;
                let size = terminal.backend().size().ok();
                let compact = size.is_some_and(|size| size.width < 48 || size.height < 14);
                let footer = if review.is_some() && !preview {
                    if compact { "Enter apply · Esc edit\nTab/S-Tab fields · type edits\nPgUp/Dn scroll · ^C cancel" }
                    else { "Enter apply · Esc edit\nTab/Shift-Tab fields · type edits\nPgUp/PgDn details · Ctrl-C cancel" }
                } else if review.is_some() {
                    if compact { "Preview done · Esc edit\nTab/S-Tab fields · type edits\nPgUp/Dn scroll · ^C exit" }
                    else { "Preview complete · Esc edit\nTab/Shift-Tab fields · type edits\nPgUp/PgDn details · Ctrl-C exit" }
                } else if success {
                    if preview {
                        if compact { "Preview done · Esc exit · type edits\nTab/S-Tab fields · ↑↓ options\nPgUp/Dn scroll · ^N transit" }
                        else { "Preview complete · Esc exit · type to edit\nTab/Shift-Tab fields · ↑↓ options\nPgUp/PgDn details · Ctrl-N transit" }
                    } else if compact {
                        "Applied · Esc exit · type edits\nTab/S-Tab fields · ↑↓ options\nPgUp/Dn scroll · ^N transit"
                    } else {
                        "Applied · Esc exit · type to edit\nTab/Shift-Tab fields · ↑↓ options\nPgUp/PgDn details · Ctrl-N transit"
                    }
                } else {
                    if compact { "^S review · Esc cancel\nTab/S-Tab fields · type search/edit\n↑↓/Enter choose · ^N transit\nPgUp/Dn scroll · ^C cancel" }
                    else { "Ctrl-S review · Esc cancel · Ctrl-C cancel\nTab/Shift-Tab fields · type search/edit\n↑↓ options · Enter choose · Ctrl-N transit · PgUp/PgDn details" }
                };
                terminal.draw(|frame| {
                    let area = frame.area();
                    let footer_lines = wrap_status(footer, area.width as usize);
                    let footer_height = (footer_lines.len() as u16).min(area.height.saturating_sub(4));
                    let status_lines = status.as_ref().map(|(text, _)| wrap_status(text, area.width as usize));
                    let status_height = status_lines.as_ref().map_or(0, |lines| {
                        (lines.len() as u16).min(3).min(area.height.saturating_sub(footer_height + 5))
                    });
                    let available = area.height.saturating_sub(footer_height + status_height + 1);
                    let visible_fields = if available >= 6 { 4 } else { available.saturating_sub(1).min(4) };
                    let rows = Layout::default().direction(Direction::Vertical).constraints([
                        Constraint::Length(u16::from(area.height > 2)),
                        Constraint::Length(visible_fields),
                        Constraint::Min(1),
                        Constraint::Length(status_height),
                        Constraint::Length(footer_height),
                    ]).split(area);
                    frame.render_widget(
                        Paragraph::new(if preview { "Pair setup · preview" } else { "Pair setup" })
                            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        rows[0],
                    );
                    let first_field = focus.saturating_sub(visible_fields.saturating_sub(1) as usize);
                    for offset in 0..visible_fields {
                        let index = first_field + offset as usize;
                        let value = match index {
                            0 => gateway.as_ref().map_or("not selected", |row| row.alias),
                            1 => vm.as_ref().map_or("not selected", |row| row.alias),
                            2 => if draft.transit_host.is_empty() { "(infer)" } else { &draft.transit_host },
                            _ => if draft.transit_port.is_empty() { "(infer)" } else { &draft.transit_port },
                        };
                        let label = format!("{}{}: ", if index == focus { "> " } else { "  " },
                            ["Gateway", "VM", "Transit host", "Transit port"][index]);
                        let label_width = (Span::raw(&label).width() as u16).min(rows[1].width.saturating_sub(1));
                        let value_width = rows[1].width.saturating_sub(label_width);
                        let text = if index == focus && index < 2 && !queries[index].is_empty() {
                            format!("{value} · search: {}", queries[index])
                        } else {
                            value.to_string()
                        };
                        let horizontal_scroll = if index == focus {
                            Span::raw(&text).width().saturating_sub(value_width as usize).min(u16::MAX as usize) as u16
                        } else { 0 };
                        let style = if index == focus {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else { Style::default() };
                        frame.render_widget(
                            Paragraph::new(label).style(style),
                            Rect::new(rows[1].x, rows[1].y + offset, label_width, 1),
                        );
                        frame.render_widget(
                            Paragraph::new(text).style(style).scroll((0, horizontal_scroll)),
                            Rect::new(rows[1].x + label_width, rows[1].y + offset, value_width, 1),
                        );
                    }
                    let option_height = if focus < 2 && review.is_none() && rows[2].height >= 4 {
                        (rows[2].height / 2).min(8)
                    } else { 0 };
                    let body = Layout::default().direction(Direction::Vertical).constraints([
                        Constraint::Length(option_height),
                        Constraint::Min(1),
                    ]).split(rows[2]);
                    if option_height > 0 {
                        let items = options.iter().map(|pick| {
                            let row = pair_setup_entry(entries, Some(*pick)).unwrap();
                            ListItem::new(vec![
                                Line::from(row.alias),
                                Line::from(format!("{}:{} · ID {}", row.entry.source.path, row.entry.source.line_start, row.entry.id)),
                            ])
                        }).collect::<Vec<_>>();
                        list_state.select(Some(selected[focus]));
                        frame.render_stateful_widget(
                            List::new(items).highlight_symbol("> ")
                                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                            body[0], &mut list_state,
                        );
                    }
                    if focus < 2 && review.is_none() && status.is_none() {
                        if let Some(pick) = options.get(selected[focus]) {
                            let row = pair_setup_entry(entries, Some(*pick)).unwrap();
                            details.insert_str(0, &format!(
                                "Choose: {}\nSource: {}:{}\nID: {}\n\n",
                                row.alias, row.entry.source.path, row.entry.source.line_start, row.entry.id,
                            ));
                        }
                    }
                    if focus < 2 && review.is_none() && options.is_empty() {
                        details.insert_str(0, "No eligible matching HostEntry aliases.\n\n");
                    }
                    let detail_lines = wrap_status(&details, body[1].width as usize);
                    scroll_step = body[1].height.max(1);
                    scroll = scroll.min(detail_lines.len().saturating_sub(body[1].height as usize).min(u16::MAX as usize) as u16);
                    frame.render_widget(Paragraph::new(detail_lines).scroll((scroll, 0)), body[1]);
                    if let Some(lines) = status_lines {
                        frame.render_widget(
                            Paragraph::new(lines).style(Style::default().fg(if success { Color::Green } else { Color::Red })),
                            rows[3],
                        );
                    }
                    frame.render_widget(Paragraph::new(footer_lines).style(Style::default().fg(Color::Cyan)), rows[4]);
                }).map_err(|error| format!("PAIR_REQUIRED: cannot render Pair setup: {error}"))?;
                let mut edited = false;
                match read_key(input, "PAIR_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc, .. } if review.is_some() => {
                        review = None;
                        scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Esc, .. } => return Ok(completed),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) => return Ok(completed),
                    KeyEvent { code: KeyCode::Char('s'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) && !applied =>
                    {
                        review = None;
                        status = None;
                        scroll = 0;
                        if pair_setup_entry(entries, draft.gateway).is_none()
                            || !draft.gateway.is_some_and(|(index, _)| eligible.contains(&index))
                        {
                            status = Some(("PAIR_SELECTION: select an eligible gateway HostEntry".to_string(), false));
                            focus = 0;
                            continue;
                        }
                        if pair_setup_entry(entries, draft.vm).is_none()
                            || !draft.vm.is_some_and(|(index, _)| vm_eligible.contains(&index))
                        {
                            status = Some(("PAIR_SELECTION: select a different eligible VM with compatible gateway transit".to_string(), false));
                            focus = 1;
                            continue;
                        }
                        match execute(&draft, PairSetupAction::Review) {
                            Ok(text) => {
                                review = Some(text);
                                if preview {
                                    completed = true;
                                    status = Some(("Preview complete — no files changed".to_string(), true));
                                }
                            }
                            Err(error) => status = Some((error, false)),
                        }
                    }
                    KeyEvent { code: KeyCode::Enter, .. } if review.is_some() => {
                        if !preview {
                            review = None;
                            scroll = 0;
                            match execute(&draft, PairSetupAction::Apply) {
                                Ok(text) => {
                                    completed = true;
                                    status = Some((text, true));
                                }
                                Err(error) => status = Some((error, false)),
                            }
                        }
                    }
                    KeyEvent { code: KeyCode::Tab, .. } => {
                        focus = (focus + 1) % 4;
                        if review.is_none() { scroll = 0; }
                    }
                    KeyEvent { code: KeyCode::BackTab, .. } => {
                        focus = (focus + 3) % 4;
                        if review.is_none() { scroll = 0; }
                    }
                    KeyEvent { code: KeyCode::Up, .. } if focus < 2 && review.is_none() => {
                        selected[focus] = selected[focus].saturating_sub(1);
                        scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Down, .. } if focus < 2 && review.is_none() => {
                        selected[focus] = (selected[focus] + 1).min(options.len().saturating_sub(1));
                        scroll = 0;
                    }
                    KeyEvent { code: KeyCode::PageUp | KeyCode::Up, .. } => scroll = scroll.saturating_sub(scroll_step),
                    KeyEvent { code: KeyCode::PageDown | KeyCode::Down, .. } => scroll = scroll.saturating_add(scroll_step),
                    KeyEvent { code: KeyCode::Enter, .. } if focus < 2 => {
                        if let Some(pick) = options.get(selected[focus]) {
                            if focus == 0 {
                                draft.gateway = Some(*pick);
                                routes = pair_setup_routes(entries, &eligible, draft.gateway);
                                vm_eligible = pair_setup_vm_eligible(&eligible, draft.gateway, &routes);
                                let vm_options = pair_setup_options(entries, &vm_eligible, &queries[1]);
                                selected[1] = vm_options.iter().position(|pick| Some(*pick) == draft.vm).unwrap_or(0);
                            } else {
                                draft.vm = Some(*pick);
                            }
                            focus += 1;
                            edited = true;
                            candidate_index = None;
                        }
                    }
                    KeyEvent { code: KeyCode::Char('n'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if let Ok(values) = &candidates {
                            if !values.is_empty() {
                                let index = candidate_index.map_or(0, |index| (index + 1) % values.len());
                                draft.transit_host.clone_from(&values[index].0);
                                draft.transit_port = values[index].1.to_string();
                                candidate_index = Some(index);
                                edited = true;
                            }
                        }
                    }
                    KeyEvent { code: KeyCode::Backspace, .. } => {
                        match focus {
                            0 | 1 => { queries[focus].pop(); selected[focus] = 0; }
                            2 => { draft.transit_host.pop(); candidate_index = None; }
                            _ => { draft.transit_port.pop(); candidate_index = None; }
                        }
                        edited = true;
                    }
                    KeyEvent { code: KeyCode::Char(character), modifiers, .. }
                        if !modifiers.contains(KeyModifiers::CONTROL) && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        match focus {
                            0 | 1 => { queries[focus].push(character); selected[focus] = 0; }
                            2 => { draft.transit_host.push(character); candidate_index = None; }
                            _ => { draft.transit_port.push(character); candidate_index = None; }
                        }
                        edited = true;
                    }
                    _ => {}
                }
                if edited {
                    review = None;
                    status = None;
                    scroll = 0;
                }
            }
        },
    )
}




fn remember_selection(state: &mut HostsState, rows: &[Row<'_>], selected: usize) {
    state.selected = rows
        .get(selected)
        .map(|row| {
            (
                row.entry.source.path.clone(),
                row.entry.source.byte_start,
                row.entry.source.byte_end,
                row.alias.to_string(),
            )
        });
}

fn hosts_footer(width: u16, has_rows: bool, active_tunnels: usize) -> String {
    if width < 24 {
        return format!(
            "{}Tab details\nPg scroll\n^T Tunnels\n^P Pairs ^S Setup\n^D Doctor · Esc",
            if has_rows { "↑↓ Enter\n" } else { "" }
        );
    }
    if width < 48 {
        return format!(
            "{}\nPgUp/Dn scroll\n^T Tunnels · ^P Pairs\n^S Setup · ^D Doctor\nEsc exit",
            if has_rows { "↑↓ Enter · Tab details" } else { "Tab details" }
        );
    }
    format!(
        "{}\nPgUp/Dn scroll\nCtrl+T Tunnels ({active_tunnels} active) · Ctrl+P Pairs\nCtrl+S Setup · Ctrl+D Doctor · Esc exit",
        if has_rows { "↑↓ move · Enter connect · Tab details" } else { "Tab details" }
    )
}

pub fn setup_workspace(
    roots: &[sshx::settings::RegisteredRoot],
    mut fields: [String; 3],
    status: Option<&str>,
) -> Result<([String; 3], bool), String> {
    const LABELS: [&str; 3] = ["Scope (personal/work)", "Project (optional)", "SSH config file path"];
    with_terminal(
        "Setup",
        "SETUP_REQUIRED: Setup requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let mut selected = if fields[0].is_empty() { 0 } else if fields[2].is_empty() { 2 } else { 0 };
            let mut root_scroll = 0u16;
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let compact = area.width < 48;
                        let footer_height = if compact { 3 } else { 1 };
                        let status_height = status.map_or(0, |value| value.lines().count().min(1) as u16);
                        let root_lines = roots
                            .iter()
                            .map(|root| {
                                format!(
                                    "{}: {}{}",
                                    root.scope,
                                    root.path.display(),
                                    root.project
                                        .as_deref()
                                        .map_or_else(String::new, |project| format!(" ({project})"))
                                )
                            })
                            .collect::<Vec<_>>();
                        let roots = if root_lines.is_empty() {
                            "(none)".to_string()
                        } else {
                            root_lines.join("\n")
                        };
                        let rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(1),
                                Constraint::Length(1),
                                Constraint::Length(1),
                                Constraint::Length(1),
                                Constraint::Length(status_height),
                                Constraint::Length(footer_height),
                            ])
                            .split(area);
                        frame.render_widget(
                            Paragraph::new("Setup · registered roots (PgUp/PgDn)")
                                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            rows[0],
                        );
                        frame.render_widget(
                            Paragraph::new(roots).wrap(Wrap { trim: false }).scroll((root_scroll, 0)),
                            rows[1],
                        );
                        for index in 0..3 {
                            let style = if selected == index {
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            let label = if compact {
                                ["Scope", "Project", "Path"][index]
                            } else {
                                LABELS[index]
                            };
                            let text = format!("{label}: {}", fields[index]);
                            let scroll = if index == selected {
                                text.chars().count().saturating_sub(area.width as usize) as u16
                            } else {
                                0
                            };
                            frame.render_widget(
                                Paragraph::new(text)
                                    .style(style)
                                    .scroll((0, scroll)),
                                rows[index + 2],
                            );
                        }
                        if status_height > 0 {
                            frame.render_widget(
                                Paragraph::new(status.unwrap_or_default())
                                    .style(Style::default().fg(Color::Red))
                                    .wrap(Wrap { trim: false }),
                                rows[5],
                            );
                        }
                        frame.render_widget(
                            Paragraph::new(if area.width < 24 {
                                "Ctrl+S register\nEsc cancel\nPgUp/Dn roots"
                            } else if compact {
                                "↑↓ fields\nPgUp/Dn roots\nCtrl+S register · Esc"
                            } else {
                                "Tab/↑↓ move fields · PgUp/PgDn roots · Ctrl+S register · Esc cancel"
                            })
                            .wrap(Wrap { trim: false }),
                            rows[6],
                        );
                    })
                    .map_err(|error| format!("SETUP_REQUIRED: cannot render Setup workspace: {error}"))?;
                match read_key(input, "SETUP_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc, .. } => return Err(CANCELLED.to_string()),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent { code: KeyCode::Char('s'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok((fields, true));
                    }
                    KeyEvent { code: KeyCode::Tab | KeyCode::Down, .. } => {
                        selected = (selected + 1) % fields.len();
                    }
                    KeyEvent { code: KeyCode::Up, .. } => selected = selected.saturating_sub(1),
                    KeyEvent { code: KeyCode::PageUp, .. } => root_scroll = root_scroll.saturating_sub(8),
                    KeyEvent { code: KeyCode::PageDown, .. } => root_scroll = root_scroll.saturating_add(8),
                    KeyEvent { code: KeyCode::Char('u'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        fields[selected].clear();
                    }
                    KeyEvent { code: KeyCode::Backspace, .. } => {
                        fields[selected].pop();
                    }
                    KeyEvent { code: KeyCode::Char(character), modifiers, .. }
                        if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        fields[selected].push(character);
                    }
                    _ => {}
                }
            }
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorAction {
    Exit,
    Repair,
}

pub fn doctor_workspace(
    findings: &[sshx::doctor::Finding],
    repairs: &[sshx::permissions::RepairCandidate],
    status: Option<&str>,
) -> Result<DoctorAction, String> {
    let mut order = (0..findings.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        let left = &findings[*left];
        let right = &findings[*right];
        let severity = |finding: &sshx::doctor::Finding| match finding.severity.as_str() {
            "error" => 0,
            "warning" => 1,
            _ => 2,
        };
        (severity(left), left.stage.as_str(), left.message.as_str()).cmp(&(
            severity(right),
            right.stage.as_str(),
            right.message.as_str(),
        ))
    });
    with_terminal(
        "Doctor",
        "DOCTOR_REQUIRED: Doctor requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let mut selected = 0usize;
            let mut expanded = true;
            let mut reviewing = false;
            let mut detail_scroll = 0u16;
            let mut status_scroll = 0u16;
            let mut detail_page_rows = 1u16;
            let mut status_page_rows = 1u16;
            let mut scroll_status = false;
            loop {
                let finding = order.get(selected).map(|index| &findings[*index]);
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let footer = if reviewing {
                            "PgUp/Dn scroll plan · Y apply · N/Esc cancel"
                        } else if area.width < 24 {
                            if repairs.is_empty() {
                                "↑↓ Enter\nTab · PgUp/Dn\nEsc exit"
                            } else {
                                "↑↓ Enter\nTab · PgUp/Dn\nR repair · Esc"
                            }
                        } else if area.width < 48 {
                            if repairs.is_empty() {
                                "↑↓ findings · Enter\nTab status · PgUp/Dn\nEsc exit"
                            } else {
                                "↑↓ findings · Enter\nTab status · PgUp/Dn\nR repair · Esc"
                            }
                        } else if area.width < 96 {
                            if repairs.is_empty() {
                                "↑↓ findings · Enter · Tab status\nPgUp/Dn scroll · Esc"
                            } else {
                                "↑↓ findings · Enter · Tab status\nPgUp/Dn scroll · R repair · Esc"
                            }
                        } else if repairs.is_empty() {
                            "↑↓ findings · Enter evidence · Tab status · PgUp/Dn scroll · Esc"
                        } else {
                            "↑↓ findings · Enter evidence · Tab status · PgUp/Dn scroll · R repair · Esc"
                        };
                        let footer_rows = if area.width < 48 { 3 } else if area.width < 96 { 2 } else { 1 };
                        let footer_height = (if status.is_some() { footer_rows + 3 } else { footer_rows })
                            .min(area.height.saturating_sub(3));
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(2),
                                Constraint::Length(footer_height),
                            ])
                            .split(area);
                        frame.render_widget(
                            Paragraph::new(if reviewing {
                                "Doctor · review eligible permission repairs"
                            } else {
                                "Doctor · report only"
                            })
                                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            chunks[0],
                        );
                        if reviewing {
                            detail_page_rows = chunks[1].height.saturating_sub(2).max(1);
                            let mut plan = String::from(
                                "Only listed eligible paths will be changed. No ownership changes or host-key enrollment.\n\n",
                            );
                            for candidate in repairs {
                                plan.push_str(&format!(
                                    "{}\n  {:04o} -> {:04o} ({})\n\n",
                                    candidate.path.display(),
                                    candidate.current_mode,
                                    candidate.target.private_mode(),
                                    candidate.kind,
                                ));
                            }
                            frame.render_widget(
                                Paragraph::new(plan)
                                    .block(block(" Permission repair plan ", Color::Cyan))
                                    .wrap(Wrap { trim: false })
                                    .scroll((detail_scroll, 0)),
                                chunks[1],
                            );
                        } else {
                            let detail_height = chunks[1].height.max(1) / 2;
                            let panes = Layout::default()
                                .direction(Direction::Vertical)
                                .constraints([Constraint::Length(detail_height), Constraint::Min(1)])
                                .split(chunks[1]);
                            detail_page_rows = panes[1].height.saturating_sub(2).max(1);
                            let mut group = None;
                            let rows = order
                                .iter()
                                .map(|index| {
                                    let finding = &findings[*index];
                                    let next_group = (finding.severity.as_str(), finding.stage.as_str());
                                    let mut lines = Vec::new();
                                    if group != Some(next_group) {
                                        lines.push(Line::from(format!(
                                            "[{}] {}",
                                            finding.severity, finding.stage
                                        )));
                                        group = Some(next_group);
                                    }
                                    lines.push(Line::from(format!("  {}", finding.message)));
                                    ListItem::new(lines)
                                })
                                .collect::<Vec<_>>();
                            let mut list = ListState::default();
                            if !rows.is_empty() {
                                list.select(Some(selected));
                            }
                            frame.render_stateful_widget(
                                List::new(rows)
                                    .block(block(" Findings ", Color::Cyan))
                                    .highlight_symbol("> ")
                                    .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                                panes[0],
                                &mut list,
                            );
                            let detail = if let Some(finding) = finding.filter(|_| expanded) {
                                format!(
                                    "{} [{} / {}]\nPath: {}\nEvidence: {}\nGuidance: {}",
                                    finding.message,
                                    finding.severity,
                                    finding.stage,
                                    finding.path.as_deref().unwrap_or("(not applicable)"),
                                    finding.evidence,
                                    finding.guidance
                                )
                            } else if finding.is_some() {
                                "Finding collapsed. Press Enter to show evidence and guidance.".to_string()
                            } else {
                                "No findings.".to_string()
                            };
                            frame.render_widget(
                                Paragraph::new(detail)
                                    .block(block(" Evidence and guidance ", Color::Cyan))
                                    .wrap(Wrap { trim: false })
                                    .scroll((detail_scroll, 0)),
                                panes[1],
                            );
                        }
                        if status.is_some() {
                            status_page_rows = chunks[2].height.saturating_sub(footer_rows).max(1);
                            frame.render_widget(
                                Paragraph::new(status.unwrap_or(""))
                                    .wrap(Wrap { trim: false })
                                    .scroll((status_scroll, 0)),
                                Rect {
                                    x: chunks[2].x,
                                    y: chunks[2].y,
                                    width: chunks[2].width,
                                    height: chunks[2].height.saturating_sub(footer_rows),
                                },
                            );
                        }
                        frame.render_widget(
                            Paragraph::new(footer).wrap(Wrap { trim: false }),
                            Rect {
                                x: chunks[2].x,
                                y: chunks[2].y + chunks[2].height.saturating_sub(footer_rows),
                                width: chunks[2].width,
                                height: footer_rows,
                            },
                        );
                    })
                    .map_err(|error| format!("DOCTOR_REQUIRED: cannot render Doctor: {error}"))?;
                match read_key(input, "DOCTOR_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc | KeyCode::Char('n' | 'N'), .. }
                        if reviewing =>
                    {
                        reviewing = false;
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Char('y' | 'Y'), .. } if reviewing => {
                        return Ok(DoctorAction::Repair);
                    }
                    KeyEvent { code: KeyCode::Esc, .. } => return Ok(DoctorAction::Exit),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(DoctorAction::Exit);
                    }
                    KeyEvent { code: KeyCode::Up, .. } if !reviewing => {
                        selected = selected.saturating_sub(1);
                        expanded = true;
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Down, .. } if !reviewing && selected + 1 < order.len() => {
                        selected += 1;
                        expanded = true;
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Enter, .. } if !reviewing && finding.is_some() => {
                        expanded = !expanded;
                        detail_scroll = 0;
                    }
                    KeyEvent { code: KeyCode::Tab, .. } if !reviewing => scroll_status = !scroll_status,
                    KeyEvent { code: KeyCode::PageUp, .. } if !reviewing && scroll_status => {
                        status_scroll = status_scroll.saturating_sub(status_page_rows);
                    }
                    KeyEvent { code: KeyCode::PageDown, .. } if !reviewing && scroll_status => {
                        status_scroll = status_scroll.saturating_add(status_page_rows);
                    }
                    KeyEvent { code: KeyCode::PageUp, .. } => {
                        detail_scroll = detail_scroll.saturating_sub(detail_page_rows);
                    }
                    KeyEvent { code: KeyCode::PageDown, .. } => {
                        detail_scroll = detail_scroll.saturating_add(detail_page_rows);
                    }
                    KeyEvent { code: KeyCode::Char('r' | 'R'), .. } if !reviewing && !repairs.is_empty() => {
                        reviewing = true;
                        detail_scroll = 0;
                    }
                    _ => {}
                }
            }
        },
    )
}

pub fn select_menu(options: &[MenuOption<'_>], label: &str) -> Result<usize, String> {
    if options.is_empty() {
        return Err("ACTION_UNAVAILABLE: no menu options are available".to_string());
    }

    with_terminal(
        label,
        format!("ACTION_REQUIRED: {label} requires a usable interactive terminal"),
        |terminal, input| {
            let mut selected = 0usize;
            loop {
                draw_menu(terminal, options, selected, label)?;
                match read_key(input, "ACTION_REQUIRED")? {
                    KeyEvent { code: KeyCode::Enter, .. }
                        if terminal.backend().size().is_ok_and(|area| {
                            area.width == 0
                                || area.height == 0
                                || (area.width >= 3 && area.height >= 2)
                        }) =>
                    {
                        return Ok(selected);
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Err(CANCELLED.to_string()),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => selected = selected.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if selected + 1 < options.len() => selected += 1,
                    _ => {}
                }
            }
        },
    )
}


#[allow(clippy::too_many_arguments)]
pub fn connection_workspace(
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
        let declared_count = services.iter().take_while(|service| {
            !service.id.starts_with("custom ")
                && !service.id.starts_with("local:")
                && !service.id.starts_with("remote:")
                && !service.id.starts_with("socks:")
        }).count();
        if restored.rows.len() < declared_count
            || restored.checked.len() != restored.rows.len()
            || services[..declared_count].iter().zip(&restored.rows).any(|(service, row)| {
                service.id != row.id || service.remote_port != row.remote_port
                    || service.destination_host != row.destination_host
            })
        {
            return Err("FORWARD_SELECTION_INVALID: declared services changed".to_string());
        }
        services = restored.rows.iter().map(|row| DeclaredService {
            id: row.id.clone(),
            remote_port: row.remote_port,
            destination_host: row.destination_host.clone(),
            default_local_port: row.local_port,
        }).collect();
        forwards = restored.rows.clone();
        checked = restored.checked.clone();
    } else {
        for requested in preselected {
            let index = services
                .iter()
                .position(|service| service.id == requested.id)
                .ok_or_else(|| {
                    "FORWARD_SELECTION_INVALID: requested service is no longer declared"
                        .to_string()
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
                && !matches!(
                    key.code,
                    KeyCode::PageUp | KeyCode::PageDown | KeyCode::Tab
                )
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
                                status = Some("Fix custom forward as [bind:]local:host:remote.".to_string());
                                focus = WorkspaceFocus::Row;
                            }
                        }
                    } else if services[index].id.starts_with("remote:")
                        || services[index].id.starts_with("socks:")
                    {
                        let is_remote = services[index].id.starts_with("remote:");
                        let parsed = if is_remote {
                            sshx::tunnel::parse_forwards(&[], std::slice::from_ref(value), &[], allow_bind)
                        } else {
                            sshx::tunnel::parse_forwards(&[], &[], std::slice::from_ref(value), allow_bind)
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
                                services[index].destination_host = parsed.remote_host.unwrap_or_default();
                                services[index].remote_port = parsed.remote_port.unwrap_or_default();
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
                        status = Some(
                            "Tunnel mode requires at least one local forward.".to_string(),
                        );
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
                            if let Some(index) =
                                forwards.iter().position(|forward| forward.id == issue.service_id)
                            {
                                row_errors[index].push(issue.message);
                            }
                        }
                        status = Some("Fix marked rows before starting; no master was started.".to_string());
                        focus = WorkspaceFocus::Status;
                    }
                }
                KeyEvent {
                    code: KeyCode::Esc,
                    ..
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
                } => return Err(CANCELLED.to_string()),
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
                    code: KeyCode::PageUp, ..
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
                    code: KeyCode::PageDown, ..
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
                    code: KeyCode::Down, ..
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
                    code: KeyCode::Down, ..
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
                        id.split_once(':').map_or_else(String::new, |(_, value)| value.to_string())
                    } else {
                        forwards[selected].local_port.to_string()
                    };
                    editing = Some((selected, value));
                    review = false;
                    status = Some(if id.starts_with("remote:") {
                        "Edit -R [bind:]listen:your-side-host:your-side-port; server bind may expose service. OpenSSH reports startup failure.".to_string()
                    } else if id.starts_with("socks:") {
                        "Edit -D [bind:]port; applications must configure this SOCKS proxy.".to_string()
                    } else if id.starts_with("local:") {
                        "Edit local row [bind:]local:host:remote; Enter saves, Esc cancels.".to_string()
                    } else if id.starts_with("custom ") {
                        "Edit custom row [bind:]local:host:remote; Enter saves, Esc cancels.".to_string()
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
        .filter(|forward| {
            !forward.id.starts_with("remote:") && !forward.id.starts_with("socks:")
        })
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

#[allow(clippy::too_many_arguments)]
fn draw_connection_workspace(
    terminal: &mut AppTerminal,
    route: &str,
    services: &[DeclaredService],
    forwards: &[ServiceForward],
    checked: &[bool],
    selected: usize,
    editing: &Option<(usize, String)>,
    row_errors: &[Vec<String>],
    mode: ConnectionMode,
    status: Option<&str>,
    status_scroll: &mut u16,
    route_scroll: &mut u16,
    row_scroll: &mut u16,
    focus: WorkspaceFocus,
    review: bool,
) -> Result<(), String> {
    let mode_label = match mode {
        ConnectionMode::Session => "Session",
        ConnectionMode::Tunnel => "Tunnel",
    };
    let ownership = match mode {
        ConnectionMode::Session => "Session-bound master; closes when shell ends",
        ConnectionMode::Tunnel => "Standalone tunnel; remains after TUI exits",
    };
    if terminal
        .backend()
        .size()
        .map_or(true, |area| area.width == 0 || area.height == 0)
    {
        eprintln!(
            "Connection workspace\nMode: {mode_label}\nOwnership: {ownership}\nRoute: {route}"
        );
        if let Some(status) = status {
            eprintln!("{status}");
        }
        for (index, service) in services.iter().enumerate() {
            if service.id.starts_with("remote:") {
                eprintln!(
                    "{} Remote (-R) {}; server bind may expose service; OpenSSH reports startup failures.",
                    if checked[index] { "[x]" } else { "[ ]" },
                    &service.id["remote:".len()..]
                );
            } else if service.id.starts_with("socks:") {
                eprintln!(
                    "{} SOCKS (-D) local proxy {}; applications configure proxy explicitly.",
                    if checked[index] { "[x]" } else { "[ ]" },
                    &service.id["socks:".len()..]
                );
            } else {
                eprintln!(
                    "{} {} | server {}:{} | local 127.0.0.1:{}",
                    if checked[index] { "[x]" } else { "[ ]" },
                    service.id,
                    service.destination_host,
                    service.remote_port,
                    forwards[index].local_port
                );
            }
            for error in &row_errors[index] {
                eprintln!("  ! {error}");
            }
        }
        eprintln!("↑↓ row  Space select  a -L  r -R  d -D  e edit  x remove custom/R/D  m mode  Enter review  Esc cancel");
        return Ok(());
    }
    terminal
        .draw(|frame| {
            let area = frame.area();
            let width = area.width.saturating_sub(2).max(1) as usize;
            let row_width = area.width.saturating_sub(4).max(1) as usize;
            let heading = format!(
                "Mode: {mode_label} [m toggles]\nOwnership: {ownership}\nRoute: {route}"
            );
            let heading_lines = wrap_status(&heading, width);
            let status_lines = status.map_or_else(Vec::new, |value| wrap_status(value, width));
            let row_detail = services.get(selected).map_or_else(
                || "No declared service forwards; Session can start without forwards.".to_string(),
                |service| {
                    let forward = &forwards[selected];
                    let local_port = editing
                        .as_ref()
                        .filter(|(row, _)| *row == selected)
                        .map_or_else(|| forward.local_port.to_string(), |(_, value)| value.clone());
                    let mut detail = if service.id.starts_with("remote:") {
                        format!(
                            "Remote (-R) [{}]\n-R {}\nServer bind/listener; may expose beyond loopback. OpenSSH reports startup failure; local checks cannot verify listener.",
                            if checked[selected] { "checked" } else { "unchecked" },
                            editing.as_ref().filter(|(row, _)| *row == selected)
                                .map_or_else(|| service.id["remote:".len()..].to_string(), |(_, value)| value.clone())
                        )
                    } else if service.id.starts_with("socks:") {
                        format!(
                            "SOCKS (-D) [{}]\n-D {}\nLocal proxy; applications must configure proxy explicitly.",
                            if checked[selected] { "checked" } else { "unchecked" },
                            editing.as_ref().filter(|(row, _)| *row == selected)
                                .map_or_else(|| service.id["socks:".len()..].to_string(), |(_, value)| value.clone())
                        )
                    } else {
                        let selection = if width < 20 {
                            format!(
                                "{} [{}]",
                                service.id,
                                if checked[selected] { "checked" } else { "unchecked" }
                            )
                        } else {
                            format!(
                                "Selected service {} [{}]",
                                service.id,
                                if checked[selected] { "checked" } else { "unchecked" }
                            )
                        };
                        format!(
                            "{selection}\nserver {}:{}\nlocal 127.0.0.1:{}",
                            service.destination_host, service.remote_port, local_port
                        )
                    };
                    for error in &row_errors[selected] {
                        detail.push_str("\nConflict: ");
                        detail.push_str(error);
                    }
                    detail
                },
            );
            let row_lines = wrap_status(&row_detail, width);
            let (detail_lines, requested_detail_scroll) = match focus {
                WorkspaceFocus::Route => (Vec::new(), 0),
                WorkspaceFocus::Row => (row_lines, *row_scroll),
                WorkspaceFocus::Status => (status_lines, *status_scroll),
            };
            let page_tabs = if status.is_some() {
                "Tab route/row/message"
            } else {
                "Tab route/row"
            };
            let scroll_target = match focus {
                WorkspaceFocus::Route => "route",
                WorkspaceFocus::Row => "row",
                WorkspaceFocus::Status => "message",
            };
            let footer = if width < 20 {
                if review {
                    "↑↓ rows\nEnter confirm\nEsc edit\nTab PgUp/Dn".to_string()
                } else if editing.is_some() {
                    "Type row\nEnter save\nEsc cancel\nPgUp/Dn row".to_string()
                } else {
                    "↑↓ Space · Tab\na -L r -R d -D\ne edit x remove\nm mode · Enter\nreview · Esc".to_string()
                }
            } else if width < 36 {
                if review {
                    "↑↓ inspect\nEnter confirm\nEsc edit\nTab pane PgUp/Dn".to_string()
                } else if editing.is_some() {
                    "Type row\nEnter save\nEsc cancel\nPgUp/Dn row".to_string()
                } else {
                    "↑↓ move Space\na -L · r -R · d -D\ne edit · x remove\nm mode · Enter review · Esc cancel".to_string()
                }
            } else if review {
                format!(
                    "Review · ↑↓ rows · Enter confirm · Esc edit\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            } else if editing.is_some() {
                format!(
                    "Type row · Enter save · Esc cancel\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            } else {
                format!(
                    "↑↓ row · Space select · a -L · r -R · d -D · e edit\nx remove custom/R/D · m mode · Enter review · Esc cancel\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            };
            let footer_lines = wrap_status(&footer, width);
            let footer_height = footer_lines.len() as u16;
            let detail_height = if matches!(focus, WorkspaceFocus::Route) {
                0
            } else {
                (detail_lines.len() as u16)
                    .min(3)
                    .min(area.height.saturating_sub(footer_height + 4))
            };
            let header_height = (heading_lines.len() as u16 + 2)
                .min(area.height.saturating_sub(detail_height + footer_height + 1))
                .max(3);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Length(detail_height),
                    Constraint::Min(1),
                    Constraint::Length(footer_height),
                ])
                .split(area);
            *route_scroll = (*route_scroll).min(
                heading_lines
                    .len()
                    .saturating_sub(chunks[0].height.saturating_sub(2) as usize)
                    .min(u16::MAX as usize) as u16,
            );
            let detail_scroll = requested_detail_scroll.min(
                detail_lines
                    .len()
                    .saturating_sub(chunks[1].height as usize)
                    .min(u16::MAX as usize) as u16,
            );
            match focus {
                WorkspaceFocus::Row => *row_scroll = detail_scroll,
                WorkspaceFocus::Status => *status_scroll = detail_scroll,
                WorkspaceFocus::Route => {}
            }
            let workspace_title = if width < 36 {
                " Workspace "
            } else {
                " Connection workspace "
            };
            frame.render_widget(
                Paragraph::new(heading_lines)
                    .block(block(workspace_title, Color::Cyan))
                    .scroll((*route_scroll, 0)),
                chunks[0],
            );
            if detail_height > 0 {
                frame.render_widget(
                    Paragraph::new(detail_lines).scroll((detail_scroll, 0)),
                    chunks[1],
                );
            }
            let items = if services.is_empty() {
                vec![ListItem::new("No local forwards; Session can start without forwards.")]
            } else {
                services
                    .iter()
                    .enumerate()
                    .map(|(index, service)| {
                        let local_port = editing
                            .as_ref()
                            .filter(|(row, _)| *row == index)
                            .map_or_else(|| forwards[index].local_port.to_string(), |(_, value)| {
                                value.clone()
                            });
                        let label = if service.id.starts_with("remote:") {
                            format!(
                                "{} (-R) server listener; your-side destination {}:{}",
                                service.id, service.destination_host, service.remote_port
                            )
                        } else if service.id.starts_with("socks:") {
                            format!(
                                "{} (-D) local proxy; applications configure explicitly",
                                service.id
                            )
                        } else {
                            format!(
                                "{} {} server {}:{} | local 127.0.0.1:{}",
                                if checked[index] { "[x]" } else { "[ ]" },
                                service.id,
                                service.destination_host,
                                service.remote_port,
                                local_port
                            )
                        };
                        let row = if width < 20 {
                            let label = if service.id.starts_with("remote:") {
                                "Remote -R".to_string()
                            } else if service.id.starts_with("socks:") {
                                "SOCKS -D".to_string()
                            } else if service.id.starts_with("custom ") {
                                format!("{} -L", service.id)
                            } else {
                                service.id.clone()
                            };
                            format!(
                                "{label} [{}]",
                                if checked[index] { "checked" } else { "unchecked" }
                            )
                        } else {
                            format!(
                                "{} {}",
                                if checked[index] { "[x]" } else { "[ ]" },
                                label
                            )
                        };
                        let mut lines = wrap_status(&row, row_width);
                        for error in &row_errors[index] {
                            lines.extend(
                                wrap_status(&format!("! {error}"), row_width)
                                    .into_iter()
                                    .map(|line| line.style(Style::default().fg(Color::Red))),
                            );
                        }
                        ListItem::new(lines)
                    })
                    .collect()
            };
            let list = List::new(items)
                .block(block(" Forwarding rows ", Color::Blue))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");
            let mut list_state = ListState::default();
            list_state.select(services.is_empty().then_some(0).or(Some(selected)));
            frame.render_stateful_widget(list, chunks[2], &mut list_state);
            frame.render_widget(
                Paragraph::new(footer_lines).style(Style::default().fg(Color::DarkGray)),
                chunks[3],
            );
        })
        .map(|_| ())
        .map_err(|error| format!("CONNECTION_REQUIRED: cannot render workspace: {error}"))
}

fn wrap_status(value: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in value.split('\n') {
        let mut line = String::new();
        let mut line_width = 0;
        for grapheme in Span::raw(paragraph).styled_graphemes(Style::default()) {
            let grapheme_width = Span::raw(grapheme.symbol).width();
            if line_width > 0 && line_width + grapheme_width > width {
                lines.push(Line::from(std::mem::take(&mut line)));
                line_width = 0;
            }
            line.push_str(grapheme.symbol);
            line_width += grapheme_width;
        }
        lines.push(Line::from(line));
    }
    lines
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostCreateAction {
    Submit,
    Apply,
    Edit,
    PreviewComplete,
}

pub fn host_create_workspace(
    mut fields: [String; 10],
    roots: &[RegisteredRoot],
    initial_focus: Option<usize>,
    status: Option<&str>,
    review: Option<&str>,
    preview: bool,
) -> Result<([String; 10], usize, HostCreateAction), String> {
    const LABELS: [&str; 10] = [
        "Config root",
        "Scope",
        "Project",
        "Folder",
        "Destination file",
        "Alias",
        "Host destination",
        "User",
        "Port",
        "Password",
    ];
    with_terminal(
        "Create HostEntry",
        "HOST_CREATE_REQUIRED: creating a HostEntry requires usable stdin and stderr terminals"
            .to_string(),
        |terminal, input| {
            let first_missing = || {
                [0, 1, 4, 5, 6]
                    .into_iter()
                    .find(|index| fields[*index].trim().is_empty())
                    .unwrap_or(0)
            };
            let mut selected = status
                .and_then(|message| match message.split(':').next()?.trim() {
                    "ROOT_NOT_FOUND" | "CONFIG_ROOT" => Some(0),
                    code if code.starts_with("CONFIG_ROOT_") => Some(0),
                    "SCOPE_ROOT_CONFLICT" => Some(1),
                    "PROJECT_ROOT_CONFLICT" => Some(2),
                    "SCOPE_REQUIRED" => Some(1),
                    "PROJECT_REQUIRED" => Some(2),
                    "FOLDER_REQUIRED" => Some(3),
                    "FILE_REQUIRED" | "TARGET_FILE" | "PASSWORD_FILE_INSECURE" => Some(4),
                    code if code.starts_with("TARGET_FILE_") => Some(4),
                    "ALIAS_REQUIRED" | "ALIAS_INVALID" => Some(5),
                    "HOSTNAME_REQUIRED" | "HOSTNAME_INVALID" => Some(6),
                    "USER_INVALID" => Some(7),
                    "PORT_INVALID" => Some(8),
                    "PASSWORD_INVALID" => Some(9),
                    _ => None,
                })
                .unwrap_or_else(|| initial_focus.unwrap_or_else(first_missing))
                .min(fields.len() - 1);
            if let Some(review) = review {
                let mut scroll = 0u16;
                loop {
                    terminal
                        .draw(|frame| {
                            let rows = Layout::default()
                                .direction(Direction::Vertical)
                                .constraints([
                                    Constraint::Length(1),
                                    Constraint::Min(1),
                                    Constraint::Length(1),
                                ])
                                .split(frame.area());
                            frame.render_widget(
                                Paragraph::new("Review HostEntry changes")
                                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                                rows[0],
                            );
                            frame.render_widget(
                                Paragraph::new(review)
                                    .wrap(Wrap { trim: false })
                                    .scroll((scroll, 0)),
                                rows[1],
                            );
                            frame.render_widget(
                                Paragraph::new(if preview {
                                    "Enter finish preview · E edit · Esc back · Ctrl-C cancel · PgUp/PgDn scroll"
                                } else {
                                    "Enter apply · E edit · Esc back · Ctrl-C cancel · PgUp/PgDn scroll"
                                })
                                .style(Style::default().fg(Color::Cyan)),
                                rows[2],
                            );
                        })
                        .map_err(|error| {
                            format!("HOST_CREATE_REQUIRED: cannot render review: {error}")
                        })?;
                    match read_key(input, "HOST_CREATE_REQUIRED")? {
                        KeyEvent {
                            code: KeyCode::Enter, ..
                        } => {
                            return Ok((
                                fields,
                                selected,
                                if preview {
                                    HostCreateAction::PreviewComplete
                                } else {
                                    HostCreateAction::Apply
                                },
                            ));
                        }
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        }
                        | KeyEvent {
                            code: KeyCode::Char('e' | 'E'), ..
                        } => return Ok((fields, selected, HostCreateAction::Edit)),
                        KeyEvent {
                            code: KeyCode::Char('c'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            return Err(CANCELLED.to_string());
                        }
                        KeyEvent {
                            code: KeyCode::Down, ..
                        } => scroll = scroll.saturating_add(1),
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => scroll = scroll.saturating_sub(1),
                        KeyEvent {
                            code: KeyCode::PageDown, ..
                        } => scroll = scroll.saturating_add(10),
                        KeyEvent {
                            code: KeyCode::PageUp, ..
                        } => scroll = scroll.saturating_sub(10),
                        KeyEvent {
                            code: KeyCode::Home, ..
                        } => scroll = 0,
                        _ => {}
                    }
                }
            }
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let has_status = status.is_some() && area.height >= 3;
                        let fixed_rows = if has_status { 3 } else { 2 };
                        let visible = area
                            .height
                            .saturating_sub(fixed_rows)
                            .min(fields.len() as u16) as usize;
                        let mut constraints = vec![Constraint::Length(1)];
                        if has_status {
                            constraints.push(Constraint::Length(1));
                        }
                        constraints.extend((0..visible).map(|_| Constraint::Length(1)));
                        constraints.extend([Constraint::Min(0), Constraint::Length(1)]);
                        let rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints(constraints)
                            .split(area);
                        frame.render_widget(
                            Paragraph::new("Create HostEntry")
                                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            rows[0],
                        );
                        if has_status {
                            frame.render_widget(
                                Paragraph::new(status.unwrap_or_default())
                                    .style(Style::default().fg(Color::Red)),
                                rows[1],
                            );
                        }
                        let first_visible = selected.saturating_sub(visible.saturating_sub(1));
                        let field_offset = if has_status { 2 } else { 1 };
                        for offset in 0..visible {
                            let index = first_visible + offset;
                            let value = if index == 9 {
                                "•".repeat(fields[index].chars().count())
                            } else {
                                fields[index].clone()
                            };
                            let style = if index == selected {
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            let text = format!("{}: {value}", LABELS[index]);
                            let scroll = text.chars().count().saturating_sub(area.width as usize) as u16;
                            frame.render_widget(
                                Paragraph::new(text)
                                    .style(style)
                                    .scroll((0, if index == selected { scroll } else { 0 })),
                                rows[field_offset + offset],
                            );
                        }
                        let footer = if area.width < 20 {
                            "^S review"
                        } else if area.width < 32 {
                            "^S review · Esc"
                        } else {
                            "Tab/↑↓ move · ←/→ choose root · Ctrl-S review · Esc cancel"
                        };
                        frame.render_widget(
                            Paragraph::new(footer).style(Style::default().fg(Color::Cyan)),
                            rows[rows.len() - 1],
                        );
                    })
                    .map_err(|error| format!("HOST_CREATE_REQUIRED: cannot render form: {error}"))?;
                match read_key(input, "HOST_CREATE_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Err(CANCELLED.to_string()),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent {
                        code: KeyCode::Char('s'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok((fields, selected, HostCreateAction::Submit));
                    }
                    KeyEvent {
                        code: direction @ (KeyCode::Left | KeyCode::Right),
                        ..
                    } if selected == 0 && !roots.is_empty() => {
                        let current = roots.iter().position(|root| root.path.to_string_lossy() == fields[0]);
                        let index = match (direction, current) {
                            (KeyCode::Right, Some(index)) => (index + 1) % roots.len(),
                            (KeyCode::Left, Some(0)) => roots.len() - 1,
                            (KeyCode::Left, Some(index)) => index - 1,
                            _ => 0,
                        };
                        let root = &roots[index];
                        fields[0] = root.path.to_string_lossy().into_owned();
                        fields[1] = root.scope.clone();
                        fields[2] = root.project.clone().unwrap_or_default();
                        fields[3] = root.path.parent().map_or_else(String::new, |path| path.to_string_lossy().into_owned());
                    }
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Down, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Enter, ..
                    } => selected = (selected + 1) % fields.len(),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => selected = selected.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::Backspace, ..
                    } => {
                        fields[selected].pop();
                    }
                    KeyEvent {
                        code: KeyCode::Char(character),
                        modifiers,
                        ..
                    } if !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        fields[selected].push(character);
                    }
                    _ => {}
                }
            }
        },
    )
}
fn password_backspace(value: &mut String, edited: &mut bool) {
    if *edited {
        value.pop();
    } else {
        value.clear();
    }
    *edited = true;
}

fn password_insert(value: &mut String, edited: &mut bool, character: char) {
    if !*edited && value.as_str() == "********" {
        value.clear();
    }
    *edited = true;
    value.push(character);
}

fn toggle_password_clear(
    value: &mut String,
    original: &str,
    cleared: &mut bool,
    edited: &mut bool,
    original_edited: bool,
) {
    *cleared = !*cleared;
    if *cleared {
        value.clear();
    } else {
        value.clear();
        value.push_str(original);
        *edited = original_edited;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEditAction {
    Submit,
    Apply,
    Edit,
    PreviewComplete,
}

pub fn host_edit_workspace(
    mut fields: [String; 5],
    mut clears: [bool; 3],
    mut password_edited: bool,
    original_fields: &[String; 5],
    rename_only: bool,
    focus: Option<usize>,
    status: Option<&str>,
    review: Option<&str>,
    preview: bool,
) -> Result<([String; 5], [bool; 3], bool, usize, HostEditAction), String> {
    const LABELS: [&str; 5] = ["Alias", "Host destination", "User", "Port", "Password"];
    let label = if rename_only {
        "Rename HostEntry"
    } else {
        "Edit HostEntry"
    };
    with_terminal(
        label,
        "HOST_EDIT_REQUIRED: editing a HostEntry requires usable stdin and stderr terminals"
            .to_string(),
        |terminal, input| {
            let field_count = if rename_only { 1 } else { LABELS.len() };
            let mut selected = status
                .and_then(|message| match message.split(':').next()?.trim() {
                    "ALIAS_INVALID" => Some(0),
                    "HOSTNAME_INVALID" | "hostname_INVALID" => Some(1),
                    "USER_INVALID" | "user_INVALID" => Some(2),
                    "PORT_INVALID" => Some(3),
                    "PASSWORD_INVALID" | "password_INVALID" | "PASSWORD_FILE_INSECURE" => Some(4),
                    _ => None,
                })
                .or(focus)
                .unwrap_or(0)
                .min(field_count - 1);
            if let Some(review) = review {
                let mut scroll = 0u16;
                loop {
                    terminal.draw(|frame| {
                        let footer = if frame.area().width < 32 {
                            if preview {
                                "Enter finish preview\nE/Esc edit · ^C cancel\nPgUp/Dn scroll"
                            } else {
                                "Enter apply\nE/Esc edit · ^C cancel\nPgUp/Dn scroll"
                            }
                        } else if frame.area().width < 80 {
                            if preview {
                                "Enter finish preview · E/Esc edit\nCtrl-C cancel · PgUp/Dn scroll"
                            } else {
                                "Enter apply · E/Esc edit\nCtrl-C cancel · PgUp/Dn scroll"
                            }
                        } else if preview {
                            "Enter finish preview · E/Esc edit · Ctrl-C cancel · PgUp/PgDn scroll"
                        } else {
                            "Enter apply · E/Esc edit · Ctrl-C cancel · PgUp/PgDn scroll"
                        };
                        let rows = Layout::default().direction(Direction::Vertical)
                            .constraints([Constraint::Length(1), Constraint::Min(1),
                                Constraint::Length(footer.lines().count() as u16)])
                            .split(frame.area());
                        frame.render_widget(Paragraph::new("Review HostEntry changes")
                            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), rows[0]);
                        frame.render_widget(Paragraph::new(review).wrap(Wrap { trim: false })
                            .scroll((scroll, 0)), rows[1]);
                        frame.render_widget(Paragraph::new(footer)
                            .style(Style::default().fg(Color::Cyan)), rows[2]);
                    }).map_err(|error| format!("HOST_EDIT_REQUIRED: cannot render review: {error}"))?;
                    match read_key(input, "HOST_EDIT_REQUIRED")? {
                        KeyEvent { code: KeyCode::Enter, .. } => return Ok((fields, clears, password_edited,
                            selected, if preview { HostEditAction::PreviewComplete } else { HostEditAction::Apply })),
                        KeyEvent { code: KeyCode::Esc | KeyCode::Char('e' | 'E'), .. } =>
                            return Ok((fields, clears, password_edited, selected, HostEditAction::Edit)),
                        KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                            if modifiers.contains(KeyModifiers::CONTROL) => return Err(CANCELLED.to_string()),
                        KeyEvent { code: KeyCode::Down, .. } => scroll = scroll.saturating_add(1),
                        KeyEvent { code: KeyCode::Up, .. } => scroll = scroll.saturating_sub(1),
                        KeyEvent { code: KeyCode::PageDown, .. } => scroll = scroll.saturating_add(10),
                        KeyEvent { code: KeyCode::PageUp, .. } => scroll = scroll.saturating_sub(10),
                        KeyEvent { code: KeyCode::Home, .. } => scroll = 0,
                        _ => {}
                    }
                }
            }
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let mut constraints = vec![Constraint::Length(1), Constraint::Min(1)];
                        if status.is_some() {
                            constraints.push(Constraint::Length(1));
                        }
                        constraints.push(Constraint::Length(1));
                        let rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints(constraints)
                            .split(area);
                        frame.render_widget(
                            Paragraph::new(label).style(
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                            ),
                            rows[0],
                        );
                        let field_area = rows[1];
                        let visible = field_area.height as usize;
                        let start = selected
                            .saturating_add(1)
                            .saturating_sub(visible)
                            .min(field_count.saturating_sub(visible));
                        for index in start..(start + visible).min(field_count) {
                            let (label, value) = (LABELS[index], &fields[index]);
                            let display = if (2..=4).contains(&index) && clears[index - 2] {
                                "<clear>".to_string()
                            } else if index == 4 && !value.is_empty() {
                                "••••".to_string()
                            } else {
                                value.clone()
                            };
                            let style = if index == selected {
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            let state = if (2..=4).contains(&index) && clears[index - 2] {
                                "clear"
                            } else if index == 4 && password_edited || index != 4 && fields[index] != original_fields[index] {
                                "replace"
                            } else {
                                "keep"
                            };
                            let marker = if index == selected { "> " } else { "  " };
                            let text = format!("{marker}{label} [{state}]: {display}");
                            let width = field_area.width.saturating_sub(2) as usize;
                            let scroll = text.chars().count().saturating_sub(width) as u16;
                            frame.render_widget(
                                Paragraph::new(text)
                                    .style(style)
                                    .scroll((0, if index == selected { scroll } else { 0 })),
                                Rect {
                                    x: field_area.x,
                                    y: field_area.y + (index - start) as u16,
                                    width: field_area.width,
                                    height: 1,
                                },
                            );
                        }
                        let status_row = if status.is_some() { 2 } else { usize::MAX };
                        if let Some(status) = status {
                            frame.render_widget(
                                Paragraph::new(status).style(Style::default().fg(Color::Red)),
                                rows[status_row],
                            );
                        }
                        let width = area.width;
                        let footer = if width < 20 {
                            "Ctrl-S review Esc"
                        } else if width < 32 {
                            if rename_only {
                                "Ctrl-S review · Esc"
                            } else {
                                "Ctrl-S review · Esc · Ctrl-X"
                            }
                        } else if rename_only {
                            "Rename selected alias only · Ctrl-S review · Esc cancel"
                        } else {
                            "Tab/↑↓ move · Ctrl-X clear optional field · Ctrl-S review · Esc cancel"
                        };
                        frame.render_widget(
                            Paragraph::new(footer).style(Style::default().fg(Color::Cyan)),
                            rows[rows.len() - 1],
                        );
                    })
                    .map_err(|error| format!("HOST_EDIT_REQUIRED: cannot render form: {error}"))?;
                match read_key(input, "HOST_EDIT_REQUIRED")? {
                    KeyEvent { code: KeyCode::Esc, .. } => return Err(CANCELLED.to_string()),
                    KeyEvent { code: KeyCode::Char('c'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) => return Err(CANCELLED.to_string()),
                    KeyEvent { code: KeyCode::Char('s'), modifiers, .. }
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok((fields, clears, password_edited, selected, HostEditAction::Submit));
                    }
                    KeyEvent { code: KeyCode::Char('x'), modifiers, .. }
                        if !rename_only
                            && modifiers.contains(KeyModifiers::CONTROL)
                            && (2..=4).contains(&selected) =>
                    {
                        let index = selected - 2;
                        if selected == 4 {
                            toggle_password_clear(
                                &mut fields[selected],
                                &original_fields[selected],
                                &mut clears[index],
                                &mut password_edited,
                                false,
                            );
                        } else {
                            clears[index] = !clears[index];
                            fields[selected] = if clears[index] {
                                String::new()
                            } else {
                                original_fields[selected].clone()
                            };
                        }
                    }
                    KeyEvent { code: KeyCode::Tab, .. }
                    | KeyEvent { code: KeyCode::Down, .. }
                    | KeyEvent { code: KeyCode::Enter, .. } if !rename_only => {
                        selected = (selected + 1) % field_count;
                    }
                    KeyEvent { code: KeyCode::Up, .. } if !rename_only => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyEvent { code: KeyCode::Backspace, .. } => {
                        if selected == 4 {
                            password_backspace(&mut fields[selected], &mut password_edited);
                        } else {
                            fields[selected].pop();
                        }
                    }
                    KeyEvent { code: KeyCode::Char(character), modifiers, .. }
                        if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        if selected == 4 {
                            password_insert(
                                &mut fields[selected],
                                &mut password_edited,
                                character,
                            );
                        }
                        if (2..=4).contains(&selected) {
                            clears[selected - 2] = false;
                        }
                        if selected != 4 {
                            fields[selected].push(character);
                        }
                    }
                    _ => {}
                }
            }
        },
    )
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationReviewAction {
    Apply,
    Edit,
    Acknowledge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationReviewMode {
    EditApply,
    Delete,
    PreviewOnly,
    Blocked,
}

pub fn mutation_review_workspace(
    review: &str,
    mode: MutationReviewMode,
) -> Result<MutationReviewAction, String> {
    let title = if mode == MutationReviewMode::Blocked {
        "HostEntry deletion blocked"
    } else {
        "Review HostEntry mutation"
    };
    with_terminal(
        title,
        "HOST_REVIEW_REQUIRED: reviewing a HostEntry mutation requires usable stdin and stderr terminals"
            .to_string(),
        |terminal, input| {
            let mut scroll = 0u16;
            loop {
                terminal
                    .draw(|frame| {
                        let rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(1),
                                Constraint::Length(1),
                            ])
                            .split(frame.area());
                        frame.render_widget(
                            Paragraph::new(title)
                                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            rows[0],
                        );
                        frame.render_widget(
                            Paragraph::new(review)
                                .wrap(Wrap { trim: false })
                                .scroll((scroll, 0)),
                            rows[1],
                        );
                        let footer = match mode {
                            MutationReviewMode::EditApply => {
                                "Enter apply · E/Esc edit · Ctrl-C cancel · PgUp/PgDn scroll"
                            }
                            MutationReviewMode::Delete => {
                                "Enter delete · Esc/Ctrl-C cancel · PgUp/PgDn scroll"
                            }
                            MutationReviewMode::PreviewOnly => {
                                "Enter finish preview · Esc/Ctrl-C cancel · PgUp/PgDn scroll"
                            }
                            MutationReviewMode::Blocked => {
                                "Enter/Esc return · Ctrl-C cancel · PgUp/PgDn scroll"
                            }
                        };
                        frame.render_widget(
                            Paragraph::new(footer).style(Style::default().fg(Color::Cyan)),
                            rows[2],
                        );
                    })
                    .map_err(|error| {
                        format!("HOST_REVIEW_REQUIRED: cannot render mutation review: {error}")
                    })?;
                match read_key(input, "HOST_REVIEW_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Enter, ..
                    } if mode == MutationReviewMode::Blocked
                        || mode == MutationReviewMode::PreviewOnly =>
                    {
                        return Ok(MutationReviewAction::Acknowledge);
                    }
                    KeyEvent {
                        code: KeyCode::Enter, ..
                    } => return Ok(MutationReviewAction::Apply),
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } if mode == MutationReviewMode::EditApply => {
                        return Ok(MutationReviewAction::Edit);
                    }
                    KeyEvent {
                        code: KeyCode::Char('e' | 'E'), ..
                    } if mode == MutationReviewMode::EditApply => {
                        return Ok(MutationReviewAction::Edit);
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } if mode == MutationReviewMode::Blocked => {
                        return Ok(MutationReviewAction::Acknowledge);
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Err(CANCELLED.to_string()),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(CANCELLED.to_string());
                    }
                    KeyEvent {
                        code: KeyCode::Down, ..
                    } => scroll = scroll.saturating_add(1),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => scroll = scroll.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::PageDown, ..
                    } => scroll = scroll.saturating_add(10),
                    KeyEvent {
                        code: KeyCode::PageUp, ..
                    } => scroll = scroll.saturating_sub(10),
                    KeyEvent {
                        code: KeyCode::Home, ..
                    } => scroll = 0,
                    _ => {}
                }
            }
        },
    )
}

fn with_terminal<T>(
    label: &str,
    not_terminal_error: String,
    run: impl FnOnce(&mut AppTerminal, &mut File) -> Result<T, String>,
) -> Result<T, String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(not_terminal_error);
    }

    enable_raw_mode()
        .map_err(|error| format!("HOST_REQUIRED: cannot enable terminal input: {error}"))?;
    let mut guard = TerminalGuard {
        raw_mode: true,
        alternate_screen: false,
        cursor_hidden: false,
    };
    let mut terminal = AppTerminal::new(CrosstermBackend::new(io::stderr()))
        .map_err(|error| format!("HOST_REQUIRED: cannot initialize terminal: {error}"))?;
    guard.cursor_hidden = true;
    let fallback = terminal
        .backend()
        .size()
        .is_ok_and(|area| area.width == 0 || area.height == 0);
    if fallback {
        let _ = terminal.resize(Rect::new(0, 0, 80, 24));
    }
    execute!(terminal.backend_mut(), EnterAlternateScreen)
        .map_err(|error| format!("HOST_REQUIRED: cannot enter alternate screen: {error}"))?;
    guard.alternate_screen = true;
    if fallback {
        eprint!("sshx {label} picker");
    }
    let input_fd = unsafe { libc::dup(io::stdin().as_raw_fd()) };
    if input_fd < 0 {
        return Err(format!(
            "HOST_REQUIRED: cannot duplicate interactive terminal: {}",
            io::Error::last_os_error()
        ));
    }
    let mut input = unsafe { File::from_raw_fd(input_fd) };

    let result = run(&mut terminal, &mut input);
    let restore = disable_raw_mode().and_then(|_| {
        guard.raw_mode = false;
        execute!(terminal.backend_mut(), LeaveAlternateScreen).and_then(|_| {
            guard.alternate_screen = false;
            terminal.show_cursor().map(|()| guard.cursor_hidden = false)
        })
    });
    match (result, restore) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("HOST_REQUIRED: cannot restore terminal: {error}")),
        (Err(error), Err(restore_error)) => {
            Err(format!("{error}; cannot restore terminal: {restore_error}"))
        }
    }
}

fn read_key(input: &mut File, prefix: &str) -> Result<KeyEvent, String> {
    let mut byte = [0u8; 1];
    input
        .read_exact(&mut byte)
        .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
    let code = match byte[0] {
        b'\r' | b'\n' => KeyCode::Enter,
        b'\t' => KeyCode::Tab,
        0x03 => return Ok(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        0x0e => return Ok(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        0x10 => return Ok(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        0x13 => return Ok(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        0x18 => return Ok(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        0x14 => return Ok(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)),
        0x04 => return Ok(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        0x15 => return Ok(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        0x12 => return Ok(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        0x08 | 0x7f => KeyCode::Backspace,
        0x1b => {
            let Some(prefix_byte) = read_escape_byte(input, prefix)? else {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            };
            if prefix_byte != b'[' {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            }
            let Some(sequence_byte) = read_escape_byte(input, prefix)? else {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            };
            match sequence_byte {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'Z' => KeyCode::BackTab,
                b'5' if read_escape_byte(input, prefix)? == Some(b'~') => KeyCode::PageUp,
                b'6' if read_escape_byte(input, prefix)? == Some(b'~') => KeyCode::PageDown,
                _ => KeyCode::Esc,
            }
        }
        byte if byte.is_ascii_graphic() || byte == b' ' => KeyCode::Char(byte as char),
        byte if byte >= 0x80 => {
            let length = match byte {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(format!("{prefix}: invalid UTF-8 terminal input")),
            };
            let mut sequence = [0u8; 4];
            sequence[0] = byte;
            input
                .read_exact(&mut sequence[1..length])
                .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
            let text = std::str::from_utf8(&sequence[..length])
                .map_err(|error| format!("{prefix}: invalid UTF-8 terminal input: {error}"))?;
            let character = text
                .chars()
                .next()
                .ok_or_else(|| format!("{prefix}: invalid UTF-8 terminal input"))?;
            KeyCode::Char(character)
        }
        _ => return read_key(input, prefix),
    };
    Ok(KeyEvent::new(code, KeyModifiers::empty()))
}
fn read_escape_byte(input: &mut File, prefix: &str) -> Result<Option<u8>, String> {
    let mut poll = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut poll, 1, 30) } <= 0 {
        return Ok(None);
    }
    let mut byte = [0u8; 1];
    input
        .read_exact(&mut byte)
        .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
    Ok(Some(byte[0]))
}

fn matching_rows<'a>(entries: &[&'a HostEntry], query: &str) -> Vec<Row<'a>> {
    let mut rows = entries
        .iter()
        .flat_map(|entry| {
            entry.aliases.iter().map(move |alias| Row {
                entry,
                alias: alias.as_str(),
            })
        })
        .filter(|row| row_rank(row, query).is_some())
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| compare_rows(left, right, query));
    rows
}

fn compare_rows(left: &Row<'_>, right: &Row<'_>, query: &str) -> Ordering {
    row_rank(left, query)
        .cmp(&row_rank(right, query))
        .then_with(|| {
            left.alias
                .to_lowercase()
                .cmp(&right.alias.to_lowercase())
        })
        .then_with(|| left.entry.source.path.cmp(&right.entry.source.path))
        .then_with(|| {
            left.entry
                .source
                .line_start
                .cmp(&right.entry.source.line_start)
        })
        .then_with(|| left.entry.id.cmp(&right.entry.id))
}

fn row_label<'a>(row: &Row<'a>, rows: &[Row<'a>]) -> std::borrow::Cow<'a, str> {
    let duplicate = rows.iter().any(|other| {
        other.alias == row.alias
            && (other.entry.id != row.entry.id
                || other.entry.source.path != row.entry.source.path
                || other.entry.source.line_start != row.entry.source.line_start)
    });
    if duplicate {
        std::borrow::Cow::Owned(format!(
            "{} · {}:{} · {}",
            row.alias, row.entry.source.path, row.entry.source.line_start, row.entry.id
        ))
    } else {
        std::borrow::Cow::Borrowed(row.alias)
    }
}

fn source_details(entry: &HostEntry) -> String {
    let mut details = format!(
        "HostEntry ID: {}\nSource: {}\nHost line: {}–{}\nSource bytes: {}–{}\nProvenance:",
        entry.id,
        entry.source.path,
        entry.source.line_start,
        entry.source.line_end,
        entry.source.byte_start,
        entry.source.byte_end
    );
    for provenance in &entry.provenance {
        details.push_str(&format!(
            "\n- scope: {}; project: {}; paths: {}",
            provenance.scope,
            provenance.project.as_deref().unwrap_or("-"),
            provenance.paths.join(", ")
        ));
    }
    details
}

fn row_rank(row: &Row<'_>, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    let query = query.to_lowercase();
    let alias = row.alias.to_lowercase();
    if alias == query {
        return Some((0, 0));
    }
    if alias.starts_with(&query) {
        return Some((1, 0));
    }

    let score = std::iter::once(fuzzy_score(&alias, &query))
        .chain(
            row.entry
                .destination
                .iter()
                .map(|destination| fuzzy_score(destination, &query)),
        )
        .chain(
            row.entry
                .projects
                .iter()
                .map(|project| fuzzy_score(project, &query)),
        )
        .flatten()
        .min();
    score.map(|score| (2, score))
}

fn fuzzy_score(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    let mut cursor = 0usize;
    let mut previous = None;
    let mut score = 0usize;
    for needle in needle.chars() {
        let position = haystack[cursor..].find(needle)? + cursor;
        if let Some(previous) = previous {
            score += position.saturating_sub(previous + 1);
        }
        score += position;
        previous = Some(position);
        cursor = position + needle.len_utf8();
    }
    Some(score)
}

#[allow(clippy::too_many_arguments)]
fn draw_host_picker(
    terminal: &mut AppTerminal,
    rows: &[Row<'_>],
    query: &str,
    selected: usize,
    label: &str,
    empty_message: &str,
    status: Option<&str>,
    route: Option<&str>,
    detail_page: HostDetailPage,
    detail_scroll: u16,
    footer: &str,
) -> Result<(), String> {
    if terminal
        .backend()
        .size()
        .is_ok_and(|area| area.width == 0 || area.height == 0)
    {
        eprintln!("\n{label}\nSearch: {query}");
        if let Some(status) = status {
            eprintln!("{status}");
        }
        if let Some(row) = rows.get(selected) {
            let route_line = route.map_or_else(String::new, |route| format!("route: {route}\n"));
            eprintln!(
                "> {}  {}\n{}source: {}:{}\nHostEntry ID: {}",
                row.alias,
                row.entry.destination.as_deref().unwrap_or("-"),
                route_line,
                row.entry.source.path,
                row.entry.source.line_start,
                row.entry.id
            );
        } else {
            eprintln!("{empty_message}");
        }
        eprintln!("{footer}");
        return Ok(());
    }
    terminal
        .draw(|frame| {
            let area = frame.area();
            if area.width < 40 || area.height < 14 {
                let (heading, detail) = match detail_page {
                    HostDetailPage::Summary => {
                        let detail = rows.get(selected).map_or_else(
                            || empty_message.to_string(),
                            |row| {
                                let route_line = route
                                    .map_or_else(String::new, |route| format!("Route: {route}\n"));
                                let scope = row.entry.scopes.join(", ");
                                let project = row.entry.projects.join(", ");
                                format!(
                                    "> {} {}\n{}Scope: {} · Project: {}\nSource: {}:{}",
                                    row.alias,
                                    row.entry.destination.as_deref().unwrap_or("-"),
                                    route_line,
                                    if scope.is_empty() { "-" } else { &scope },
                                    if project.is_empty() { "-" } else { &project },
                                    row.entry.source.path,
                                    row.entry.source.line_start
                                )
                            },
                        );
                        (format!("sshx {label} · Search: {query}"), detail)
                    }
                    HostDetailPage::Identity => (
                        String::new(),
                        rows.get(selected).map_or_else(
                            || empty_message.to_string(),
                            |row| {
                                format!(
                                    "HostEntry ID: {}\nAliases: {}\nScopes: {}\nProjects: {}",
                                    row.entry.id,
                                    row.entry.aliases.join(", "),
                                    if row.entry.scopes.is_empty() {
                                        "-".to_string()
                                    } else {
                                        row.entry.scopes.join(", ")
                                    },
                                    if row.entry.projects.is_empty() {
                                        "-".to_string()
                                    } else {
                                        row.entry.projects.join(", ")
                                    }
                                )
                            },
                        ),
                    ),
                    HostDetailPage::Source => (
                        String::new(),
                        rows.get(selected).map_or_else(
                            || empty_message.to_string(),
                            |row| source_details(row.entry),
                        ),
                    ),
                };
                let status = status.map_or_else(String::new, |status| format!("{status}\n"));
                let content = if matches!(detail_page, HostDetailPage::Summary) {
                    format!("{heading}\n{status}{detail}")
                } else {
                    format!("{status}{detail}")
                };
                let footer_height = footer.lines().count() as u16;
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(footer_height)])
                    .split(area);
                let paragraph = Paragraph::new(content).wrap(Wrap { trim: false });
                frame.render_widget(paragraph.scroll((detail_scroll, 0)), chunks[0]);
                frame.render_widget(Paragraph::new(footer), chunks[1]);
                return;
            }
            let footer_height = footer.lines().count() as u16;
            let detail_max = if area.width < 80 { 10 } else { 8 };
            let detail_height = detail_max.min(
                area.height
                    .saturating_sub(3 + footer_height + 2)
                    .max(5),
            );
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(detail_height),
                    Constraint::Length(footer_height),
                ])
                .split(area);
            let title = if label == "Hosts" {
                format!(" sshx {label} ")
            } else {
                format!(" sshx {label} picker ")
            };
            let search = Paragraph::new(format!("Search: {query}"))
                .block(block(title, Color::Cyan))
                .style(Style::default().fg(Color::White));
            frame.render_widget(search, chunks[0]);

            let items = rows
                .iter()
                .map(|row| {
                    ListItem::new(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            row_label(row, rows),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            row.entry.destination.as_deref().unwrap_or("-"),
                            Style::default().fg(Color::Green),
                        ),
                    ]))
                })
                .collect::<Vec<_>>();
            let list = List::new(items)
                .block(block(" HostEntry ", Color::Blue))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");
            let mut state = ListState::default();
            state.select(rows.is_empty().then_some(0).or(Some(selected)));
            frame.render_stateful_widget(list, chunks[1], &mut state);

            let detail = rows.get(selected).map_or_else(
                || empty_message.to_string(),
                |row| match detail_page {
                    HostDetailPage::Summary => {
                        let route_line = route.map_or_else(String::new, |route| {
                            format!("route: {route}\n")
                        });
                        format!(
                            "{route_line}destination: {}\nscope: {}\nproject: {}\nsource: {}:{}",
                            row.entry.destination.as_deref().unwrap_or("-"),
                            if row.entry.scopes.is_empty() {
                                "-".to_string()
                            } else {
                                row.entry.scopes.join(", ")
                            },
                            if row.entry.projects.is_empty() {
                                "-".to_string()
                            } else {
                                row.entry.projects.join(", ")
                            },
                            row.entry.source.path,
                            row.entry.source.line_start
                        )
                    }
                    HostDetailPage::Identity => format!(
                        "HostEntry ID: {}\nAliases: {}\nScopes: {}\nProjects: {}",
                        row.entry.id,
                        row.entry.aliases.join(", "),
                        if row.entry.scopes.is_empty() {
                            "-".to_string()
                        } else {
                            row.entry.scopes.join(", ")
                        },
                        if row.entry.projects.is_empty() {
                            "-".to_string()
                        } else {
                            row.entry.projects.join(", ")
                        }
                    ),
                    HostDetailPage::Source => source_details(row.entry),
                },
            );
            let detail = match status {
                Some(status) => format!("{status}\n\n{detail}"),
                None => detail,
            };
            let paragraph = Paragraph::new(detail)
                .block(block(" Selected HostEntry ", Color::Magenta))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph.scroll((detail_scroll, 0)), chunks[2]);
            frame.render_widget(
                Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
                chunks[3],
            );
        })
        .map(|_| ())
        .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))
}

fn draw_menu(
    terminal: &mut AppTerminal,
    options: &[MenuOption<'_>],
    selected: usize,
    label: &str,
) -> Result<(), String> {
    if terminal
        .backend()
        .size()
        .is_ok_and(|area| area.width == 0 || area.height == 0)
    {
        for (index, option) in options.iter().enumerate() {
            eprint!(
                "\n{} {} — {}",
                if index == selected { ">" } else { " " },
                option.label,
                option.description
            );
        }
        eprint!("\n↑↓ move  Enter select  Esc cancel");
    }
    terminal
        .draw(|frame| render_menu(frame, options, selected, label))
        .map(|_| ())
        .map_err(|error| format!("ACTION_REQUIRED: cannot render menu: {error}"))
}

fn render_menu(
    frame: &mut ratatui::Frame<'_>,
    options: &[MenuOption<'_>],
    selected: usize,
    label: &str,
) {
    let label_width = options
        .iter()
        .map(|option| option.label.chars().count())
        .max()
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|option| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<width$}", option.label, width = label_width),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(option.description, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect::<Vec<_>>();
    let menu_block = if frame.area().height <= 3 {
        Block::default()
    } else {
        block(format!(" sshx · {label} "), Color::Cyan)
    };
    let list = List::new(items)
        .block(menu_block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(Some(selected));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(1)])
        .split(frame.area());
    frame.render_stateful_widget(list, chunks[0], &mut state);
    frame.render_widget(
        Paragraph::new("↑↓ move  Enter select  Esc cancel").style(Style::default().fg(Color::Gray)),
        chunks[1],
    );
}

fn block(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
}

#[cfg(test)]
mod tests {
    use super::{
        MenuOption, clear_preflight_errors, fuzzy_score, hosts_footer, matching_rows,
        password_backspace, password_insert, read_key, render_menu, restore_edit_row_errors,
        row_label, source_details, toggle_password_clear, workspace_preflight_issues, wrap_status,
    };
    use ratatui::{Terminal, backend::TestBackend};
    use sshx::discovery::{HostEntry, SourceIdentity};


    #[test]
    fn hosts_footer_keeps_detail_and_scroll_controls_at_compact_widths() {
        for width in [18, 24, 32, 47, 48, 80] {
            let footer = hosts_footer(width, true, 2);
            assert!(footer.contains("Tab details"), "width={width}: {footer}");
            assert!(footer.contains("scroll"), "width={width}: {footer}");
        }
    }
    #[test]
    fn password_clear_toggle_restores_form_baseline() {
        let original = "********";
        let mut value = "replacement-secret".to_string();
        let mut cleared = false;
        let mut edited = true;
        let original_edited = false;

        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            original_edited,
        );
        assert!(cleared);
        assert!(value.is_empty());
        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            original_edited,
        );
        assert!(!cleared);
        assert_eq!(value, original);
        assert!(!edited);

        let original = "edited-secret";
        let mut value = original.to_string();
        let mut cleared = false;
        let mut edited = true;
        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            true,
        );
        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            true,
        );
        assert_eq!(value, original);
        assert!(edited);
    }

    #[test]
    fn menu_marks_selected_option() {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal
            .draw(|frame| {
                render_menu(
                    frame,
                    &[
                        MenuOption::new("First", "Open the first thing"),
                        MenuOption::new("Second", "Open the second thing"),
                    ],
                    1,
                    "main menu",
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((1, 2)).unwrap().symbol(), ">");
        assert_eq!(
            buffer.cell((3, 2)).unwrap().fg,
            ratatui::style::Color::White
        );
        assert_eq!(
            buffer.cell((3, 2)).unwrap().bg,
            ratatui::style::Color::DarkGray
        );
        assert!(
            buffer
                .cell((3, 2))
                .unwrap()
                .modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn wraps_wide_unicode_route_by_display_cell_width() {
        let lines = wrap_status("Route: /项目/服务", 8);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content.as_ref(), "Route: /");
        assert_eq!(lines[1].spans[0].content.as_ref(), "项目/服");
        assert_eq!(lines[2].spans[0].content.as_ref(), "务");
        assert_eq!(
            lines.iter().map(|line| line.width()).collect::<Vec<_>>(),
            [8, 7, 2]
        );
    }

    #[test]
    fn preserves_repeated_spaces_when_wrapping() {
        let lines = wrap_status("/a  b", 3);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.spans[0].content.as_ref())
                .collect::<Vec<&str>>(),
            ["/a ", " b"]
        );
        assert_eq!(
            lines.iter().map(|line| line.width()).collect::<Vec<_>>(),
            [3, 2]
        );
    }

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
        assert_eq!(
            row_errors,
            vec![Vec::<String>::new(), Vec::new()]
        );
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
        let duplicate = workspace_preflight_issues(
            &[socks("localhost:1080"), socks("127.0.0.1:1080")],
            false,
        );
        assert!(
            duplicate
                .iter()
                .any(|issue| issue.message.starts_with("FORWARD_DUPLICATE:"))
        );

        let distinct = workspace_preflight_issues(
            &[socks("127.0.0.1:1080"), socks("[::1]:1080")],
            false,
        );
        assert!(
            distinct
                .iter()
                .all(|issue| !issue.message.starts_with("FORWARD_DUPLICATE:"))
        );
    }


    fn host(
        id: &str,
        alias: &str,
        destination: Option<&str>,
        project: Option<&str>,
        path: &str,
        line: usize,
    ) -> HostEntry {
        HostEntry {
            id: id.to_string(),
            aliases: vec![alias.to_string()],
            source: SourceIdentity {
                path: path.to_string(),
                byte_start: 0,
                byte_end: 1,
                line_start: line,
                line_end: line,
            },
            destination: destination.map(str::to_string),
            provenance: Vec::new(),
            scopes: Vec::new(),
            projects: project.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn source_details_include_ranges_and_provenance_paths() {
        let mut entry = host("host-id", "alias", None, None, "/config/main", 7);
        entry.source.byte_start = 12;
        entry.source.byte_end = 42;
        entry.source.line_end = 9;
        entry.provenance.push(sshx::discovery::Provenance {
            paths: vec!["/config/main".to_string(), "/config/included".to_string()],
            scope: "work".to_string(),
            project: Some("app".to_string()),
        });

        let details = source_details(&entry);

        assert!(details.contains("HostEntry ID: host-id"));
        assert!(details.contains("Source: /config/main"));
        assert!(details.contains("Host line: 7–9"));
        assert!(details.contains("Source bytes: 12–42"));
        assert!(
            details.contains("- scope: work; project: app; paths: /config/main, /config/included")
        );
    }

    #[test]
    fn duplicate_alias_labels_show_full_source_and_identity() {
        let first = host(
            "33333333-1111-4111-8111-111111111111",
            "shared",
            None,
            None,
            "/one/.ssh/config",
            7,
        );
        let second = host(
            "33333333-2222-4222-8222-222222222222",
            "shared",
            None,
            None,
            "/two/.ssh/config",
            7,
        );
        let rows = matching_rows(&[&first, &second], "");
        let labels = rows
            .iter()
            .map(|row| row_label(row, &rows).into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            [
                "shared · /one/.ssh/config:7 · 33333333-1111-4111-8111-111111111111",
                "shared · /two/.ssh/config:7 · 33333333-2222-4222-8222-222222222222",
            ]
        );
    }

    #[test]
    fn fuzzy_score_requires_ordered_subsequence() {
        assert!(fuzzy_score("production", "pdt").is_some());
        assert!(fuzzy_score("production", "z").is_none());
    }

    #[test]
    fn destination_and_project_queries_match_without_alias_match() {
        let destination = host(
            "destination",
            "alias",
            Some("prod.example"),
            None,
            "/config",
            1,
        );
        let project = host("project", "alias", None, Some("work"), "/config", 2);
        assert_eq!(
            matching_rows(&[&destination], "prod")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
        assert_eq!(
            matching_rows(&[&project], "work")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
    }

    #[test]
    fn exact_prefix_fuzzy_and_empty_query_order_is_deterministic() {
        let exact = host("exact", "alpha", Some("same"), None, "/b", 2);
        let prefix = host("prefix", "alphabet", Some("same"), None, "/a", 1);
        let fuzzy = host("fuzzy", "zzz", Some("alpha-target"), None, "/c", 1);
        let rows = matching_rows(&[&prefix, &fuzzy, &exact], "alpha");
        assert_eq!(
            rows.iter().map(|row| row.alias).collect::<Vec<_>>(),
            ["alpha", "alphabet", "zzz"]
        );

        let first = host("first", "zeta", None, None, "/b", 2);
        let second = host("second", "Zeta", None, None, "/a", 1);
        let rows = matching_rows(&[&first, &second], "");
        assert_eq!(
            rows.iter()
                .map(|row| row.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
    }
    #[test]
    fn read_key_decodes_ctrl_p() {
        let path = std::env::temp_dir().join(format!(
            "sshx-picker-ctrl-p-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, [0x10]).unwrap();
        let event = read_key(&mut std::fs::File::open(&path).unwrap(), "TEST").unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            event,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('p'),
                crossterm::event::KeyModifiers::CONTROL,
            )
        );
    }
    #[test]
    fn password_marker_backspace_and_literal_stars_are_distinct() {
        let mut value = "********".to_string();
        let mut edited = false;
        password_backspace(&mut value, &mut edited);
        assert!(value.is_empty());
        assert!(edited);

        let mut value = "********".to_string();
        let mut edited = false;
        for _ in 0..8 {
            password_insert(&mut value, &mut edited, '*');
        }
        password_insert(&mut value, &mut edited, 'x');
        assert_eq!(value, "********x");
    }


}
