use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};

use super::terminal::{read_key, with_terminal};
use super::widgets::block;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DoctorAction {
    Exit,
    Repair,
}

pub(crate) fn doctor_workspace(
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
            let mut plan_visible = false;
            let mut repair_plan = None;
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
                        let footer_height = (if status.is_some() && !reviewing { footer_rows + 3 } else { footer_rows })
                            .min(area.height.saturating_sub(3));
                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(2),
                                Constraint::Length(footer_height),
                            ])
                            .split(area);
                        plan_visible = chunks[1].width > 2 && chunks[1].height > 2;
                        let footer = if reviewing && !plan_visible {
                            "Resize terminal to review plan · Esc cancel"
                        } else {
                            footer
                        };
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
                            let plan = repair_plan.get_or_insert_with(|| {
                                let mut plan = String::from(
                                    "Only listed eligible paths will be changed. No ownership changes or host-key enrollment.\n\n",
                                );
                                for candidate in repairs {
                                    use std::fmt::Write;
                                    let _ = writeln!(
                                        plan,
                                        "{}\n  {:04o} -> {:04o} ({})\n",
                                        candidate.path.display(),
                                        candidate.current_mode,
                                        candidate.target.private_mode(),
                                        candidate.kind,
                                    );
                                }
                                plan
                            });
                            frame.render_widget(
                                Paragraph::new(plan.as_str())
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
                        if status.is_some() && !reviewing {
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
                    KeyEvent {
                        code: KeyCode::Esc | KeyCode::Char('n' | 'N'),
                        ..
                    } if reviewing => {
                        reviewing = false;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Char('y' | 'Y'),
                        ..
                    } if reviewing && plan_visible => {
                        return Ok(DoctorAction::Repair);
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Ok(DoctorAction::Exit),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(DoctorAction::Exit);
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } if !reviewing => {
                        selected = selected.saturating_sub(1);
                        expanded = true;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if !reviewing && selected + 1 < order.len() => {
                        selected += 1;
                        expanded = true;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if !reviewing && finding.is_some() => {
                        expanded = !expanded;
                        detail_scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } if !reviewing => scroll_status = !scroll_status,
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } if !reviewing && scroll_status => {
                        status_scroll = status_scroll.saturating_sub(status_page_rows);
                    }
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } if !reviewing && scroll_status => {
                        status_scroll = status_scroll.saturating_add(status_page_rows);
                    }
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => {
                        detail_scroll = detail_scroll.saturating_sub(detail_page_rows);
                    }
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => {
                        detail_scroll = detail_scroll.saturating_add(detail_page_rows);
                    }
                    KeyEvent {
                        code: KeyCode::Char('r' | 'R'),
                        ..
                    } if !reviewing && !repairs.is_empty() => {
                        reviewing = true;
                        detail_scroll = 0;
                    }
                    _ => {}
                }
            }
        },
    )
}
