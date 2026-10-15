use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use sshx::discovery::HostEntry;

use super::connection_view::wrap_status;
use super::terminal::{read_key, with_terminal};
use super::widgets::block;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PairWorkspaceAction {
    Setup,
    Delete(usize),
    Recover,
    Validate,
    Exit,
}

pub(crate) fn pairs_workspace(
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
            "Press R to recover the pending Pair transaction before setup or deletion."
        }
        _ => {
            "Inspect the reported source and metadata, correct the finding, then press V to validate."
        }
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
                Ok(_) => Err(
                    "PAIR_BROKEN: reciprocal Pair record no longer matches the current route"
                        .to_string(),
                ),
                Err(error) => Err(error),
            }
        } else {
            Err(
                "PAIR_INVALID: immutable IDs do not resolve to exactly one gateway and one VM"
                    .to_string(),
            )
        };
        let mut body = format!(
            "Status: {}\nApproved transit: {}:{}\n\nGateway\nID: {}\nRecorded alias: {}\n",
            if route_status.is_ok() {
                "valid"
            } else {
                "invalid"
            },
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
            if route_status.is_ok() {
                "valid"
            } else {
                "invalid"
            },
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
                        let footer = match (recovery_available, compact, selected < pairs.len()) {
                            (true, true, _) => "V validate · R recover\nSetup and delete blocked\nEsc Hosts · PgUp/PgDn",
                            (true, false, _) => "↑↓ select · PgUp/PgDn detail · V validate · R recover · setup and delete blocked pending recovery · Esc Hosts",
                            (false, true, true) => "↑↓ select · PgUp/PgDn\nV validate · S setup · D delete\nEsc Hosts",
                            (false, true, false) => "↑↓ select · PgUp/PgDn\nV validate · S setup\nEsc Hosts",
                            (false, false, true) => "↑↓ select · PgUp/PgDn detail · V validate · S setup Pair · D delete · Esc Hosts",
                            (false, false, false) => "↑↓ select · PgUp/PgDn detail · V validate · S setup Pair · Esc Hosts",
                        };
                        let footer_lines = wrap_status(footer, area.width as usize);
                        let footer_height = (footer_lines.len() as u16).min(area.height.saturating_sub(2));
                        let list_height = (area.height / 3).clamp(1, 7)
                            .min(area.height.saturating_sub(footer_height + 1));
                        let status_lines = status.map(|text| wrap_status(text, area.width as usize));
                        let status_height = status_lines.as_ref().map_or(0, |lines| {
                            (lines.len().min(3) as u16)
                                .min(area.height.saturating_sub(footer_height + list_height + 1))
                        });
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
                        if let Some(lines) = status_lines {
                            frame.render_widget(
                                Paragraph::new(lines)
                                    .style(Style::default().fg(Color::Green))
                                    .wrap(Wrap { trim: false }),
                                chunks[2],
                            );
                        }
                        frame.render_widget(
                            Paragraph::new(footer_lines),
                            chunks[3],
                        );
                    })
                    .map_err(|error| format!("PAIR_REQUIRED: cannot render Pairs: {error}"))?;
                match read_key(input, "PAIR_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Ok(PairWorkspaceAction::Exit),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(PairWorkspaceAction::Exit);
                    }
                    KeyEvent {
                        code: KeyCode::Char('v' | 'V'),
                        ..
                    } => {
                        return Ok(PairWorkspaceAction::Validate);
                    }
                    KeyEvent {
                        code: KeyCode::Char('s' | 'S'),
                        ..
                    } if recovery_available => {}
                    KeyEvent {
                        code: KeyCode::Char('s' | 'S'),
                        ..
                    } => {
                        return Ok(PairWorkspaceAction::Setup);
                    }
                    KeyEvent {
                        code: KeyCode::Char('d' | 'D'),
                        ..
                    } if !recovery_available && selected < pairs.len() => {
                        return Ok(PairWorkspaceAction::Delete(selected));
                    }
                    KeyEvent {
                        code: KeyCode::Char('r' | 'R'),
                        ..
                    } if recovery_available => {
                        return Ok(PairWorkspaceAction::Recover);
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => {
                        selected = selected.saturating_sub(1);
                        scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if selected + 1 < bodies.len() => {
                        selected += 1;
                        scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => {
                        scroll = scroll.saturating_sub(detail_step);
                    }
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => {
                        scroll = scroll.saturating_add(detail_step);
                    }
                    _ => {}
                }
            }
        },
    )
}
