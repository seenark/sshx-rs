use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Wrap},
};

use super::CANCELLED;
use super::terminal::{read_key, with_terminal};
use super::widgets::{password_backspace, password_insert, toggle_password_clear};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostEditAction {
    Submit,
    Apply,
    Edit,
    PreviewComplete,
}

pub(crate) struct HostEditState {
    pub fields: [String; 5],
    pub clears: [bool; 3],
    pub password_edited: bool,
}

pub(crate) fn host_edit_workspace(
    state: HostEditState,
    original_fields: &[String; 5],
    rename_only: bool,
    focus: Option<usize>,
    status: Option<&str>,
    review: Option<&str>,
    preview: bool,
) -> Result<(HostEditState, usize, HostEditAction), String> {
    let HostEditState {
        mut fields,
        mut clears,
        mut password_edited,
    } = state;
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
                        KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            return Ok((
                                HostEditState {
                                    fields,
                                    clears,
                                    password_edited,
                                },
                                selected,
                                if preview {
                                    HostEditAction::PreviewComplete
                                } else {
                                    HostEditAction::Apply
                                },
                            ));
                        }
                        KeyEvent {
                            code: KeyCode::Esc | KeyCode::Char('e' | 'E'),
                            ..
                        } => {
                            return Ok((
                                HostEditState {
                                    fields,
                                    clears,
                                    password_edited,
                                },
                                selected,
                                HostEditAction::Edit,
                            ));
                        }
                        KeyEvent {
                            code: KeyCode::Char('c'),
                            modifiers,
                            ..
                        } if modifiers.contains(KeyModifiers::CONTROL) => {
                            return Err(CANCELLED.to_string());
                        }
                        KeyEvent {
                            code: KeyCode::Down,
                            ..
                        } => scroll = scroll.saturating_add(1),
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => scroll = scroll.saturating_sub(1),
                        KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        } => scroll = scroll.saturating_add(10),
                        KeyEvent {
                            code: KeyCode::PageUp,
                            ..
                        } => scroll = scroll.saturating_sub(10),
                        KeyEvent {
                            code: KeyCode::Home,
                            ..
                        } => scroll = 0,
                        _ => {}
                    }
                }
            }
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let footer = if area.width < 20 {
                            if rename_only {
                                "Ctrl-S review Esc"
                            } else {
                                "Ctrl-S review Esc\nTab move · ^X clear"
                            }
                        } else if area.width < 32 {
                            if rename_only {
                                "Ctrl-S review · Esc"
                            } else {
                                "Ctrl-S review · Esc\nTab move · Ctrl-X clear"
                            }
                        } else if area.width < 80 {
                            if rename_only {
                                "Ctrl-S review · Esc cancel\nRename selected alias only"
                            } else {
                                "Ctrl-S review · Esc cancel\nTab/↑↓ move · Ctrl-X clear optional field"
                            }
                        } else if rename_only {
                            "Rename selected alias only · Ctrl-S review · Esc cancel"
                        } else {
                            "Tab/↑↓ move · Ctrl-X clear optional field · Ctrl-S review · Esc cancel"
                        };
                        let mut constraints = vec![Constraint::Length(1), Constraint::Min(1)];
                        if status.is_some() {
                            constraints.push(Constraint::Length(1));
                        }
                        constraints.push(Constraint::Length(footer.lines().count() as u16));
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
                        frame.render_widget(
                            Paragraph::new(footer).style(Style::default().fg(Color::Cyan)),
                            rows[rows.len() - 1],
                        );
                    })
                    .map_err(|error| format!("HOST_EDIT_REQUIRED: cannot render form: {error}"))?;
                match read_key(input, "HOST_EDIT_REQUIRED")? {
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
                        return Ok((
                            HostEditState {
                                fields,
                                clears,
                                password_edited,
                            },
                            selected,
                            HostEditAction::Submit,
                        ));
                    }
                    KeyEvent {
                        code: KeyCode::Char('x'),
                        modifiers,
                        ..
                    } if !rename_only
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
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    }
                    | KeyEvent {
                        code: KeyCode::Down,
                        ..
                    }
                    | KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if !rename_only => {
                        selected = (selected + 1) % field_count;
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } if !rename_only => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        if selected == 4 {
                            password_backspace(&mut fields[selected], &mut password_edited);
                        } else {
                            fields[selected].pop();
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char(character),
                        modifiers,
                        ..
                    } if !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        if selected == 4 {
                            password_insert(&mut fields[selected], &mut password_edited, character);
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
