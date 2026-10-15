use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

use super::CANCELLED;
use super::terminal::{read_key, with_terminal};
use super::widgets::block;

pub(crate) fn tunnels_workspace(
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
                    } => {
                        detail_scroll = detail_scroll.saturating_sub(detail_step);
                    }
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => {
                        detail_scroll = detail_scroll.saturating_add(detail_step);
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => {
                        selected = selected.saturating_sub(1);
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if selected + 1 < tunnels.len() => {
                        selected += 1;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Char('s'),
                        ..
                    } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 's') => {
                        return Ok((selected, 's'));
                    }
                    KeyEvent {
                        code: KeyCode::Char('r'),
                        ..
                    } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 'r') => {
                        return Ok((selected, 'r'));
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if !tunnels.is_empty() && allowed.is_none_or(|action| action == 'v') => {
                        return Ok((selected, 'v'));
                    }
                    _ => {}
                }
            }
        },
    )
}
