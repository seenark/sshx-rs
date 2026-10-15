use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::Backend;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use sshx::discovery::HostEntry;

use super::CANCELLED;
use super::matcher::{Row, matching_rows, row_label, source_details};
use super::terminal::{AppTerminal, read_key, with_terminal};
use super::widgets::block;

#[derive(Clone, Copy)]
pub(crate) struct Selection<'a> {
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
pub(crate) struct HostsState {
    query: String,
    selected: Option<(String, usize, usize, String)>,
    detail_page: HostDetailPage,
    detail_scroll: u16,
    pub active_tunnels: usize,
    pub edit_action: Option<sshx::mutation::MutationKind>,
    pub copy_password: bool,
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

pub(crate) fn select<'a>(
    entries: &[&'a HostEntry],
    label: &str,
    state: &mut HostsState,
    status: Option<&str>,
) -> Result<Selection<'a>, String> {
    with_terminal(
        label,
        format!(
            "HOST_REQUIRED: {label} requires a HostEntry selector with a usable interactive terminal"
        ),
        |terminal, input| {
            let mut query = String::new();
            let mut selected = state
                .selected
                .as_ref()
                .and_then(|(path, start, end, alias)| {
                    matching_rows(entries, "").iter().position(|row| {
                        row.entry.source.path == *path
                            && row.entry.source.byte_start == *start
                            && row.entry.source.byte_end == *end
                            && row.alias == alias
                    })
                })
                .unwrap_or(0);
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
                    status,
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
                            state.prefill(row.entry, row.alias, "");
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

pub(crate) fn browse_hosts<'a>(
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
                route_display =
                    rows.get(selected).map_or_else(
                        String::new,
                        |row| match sshx::pair::paired_route(source_entries, row.entry) {
                            Ok(Some(route)) => format!(
                                "Pair via {}",
                                route.gateway.aliases.first().map_or("-", String::as_str)
                            ),
                            Ok(None) => "Direct".to_string(),
                            Err(error) => format!("Unavailable: {error}"),
                        },
                    );
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
                    code: KeyCode::Char('y'),
                    modifiers,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(row) = rows.get(selected) {
                        state.detail_scroll = 0;
                        state.detail_page = HostDetailPage::Summary;
                        state.copy_password = true;
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
                        return Ok(Some(Selection {
                            entry: row.entry,
                            alias: row.alias,
                        }));
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

fn remember_selection(state: &mut HostsState, rows: &[Row<'_>], selected: usize) {
    state.selected = rows.get(selected).map(|row| {
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
        return if has_rows {
            "↑↓ Enter\n^Y Copy password\nTab · Pg scroll\n^T Tunnels\n^P Pairs ^S Setup\n^D Doctor · Esc"
        } else {
            "Tab details\nPg scroll\n^T Tunnels\n^P Pairs ^S Setup\n^D Doctor · Esc"
        }.to_string();
    }
    if width < 48 {
        return if has_rows {
            "↑↓ Enter\n^Y Copy password\nTab details · Pg scroll\n^T Tunnels · ^P Pairs\n^S Setup ^D Doctor · Esc"
        } else {
            "Tab details\nPgUp/Dn scroll\n^T Tunnels · ^P Pairs\n^S Setup · ^D Doctor\nEsc exit"
        }.to_string();
    }
    format!(
        "{}\nPgUp/Dn scroll{}\nCtrl+T Tunnels ({active_tunnels} active) · Ctrl+P Pairs\nCtrl+S Setup · Ctrl+D Doctor · Esc exit",
        if has_rows {
            "↑↓ move · Enter connect · Tab details"
        } else {
            "Tab details"
        },
        if has_rows {
            " · Ctrl+Y Copy password"
        } else {
            ""
        }
    )
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
            let detail_height =
                detail_max.min(area.height.saturating_sub(3 + footer_height + 2).max(5));
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
                        let route_line =
                            route.map_or_else(String::new, |route| format!("route: {route}\n"));
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
