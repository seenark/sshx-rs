use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Wrap},
};
use sshx::settings::RegisteredRoot;

use super::CANCELLED;
use super::terminal::{read_key, with_terminal};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostCreateAction {
    Submit,
    Apply,
    Edit,
    PreviewComplete,
}

pub(crate) fn host_create_workspace(
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
                            code: KeyCode::Enter,
                            ..
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
                            code: KeyCode::Char('e' | 'E'),
                            ..
                        } => return Ok((fields, selected, HostCreateAction::Edit)),
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
                        let has_status = status.is_some() && area.height >= 3;
                        let fixed_rows = if has_status { 3 } else { 2 };
                        let visible =
                            area.height
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
                            Paragraph::new("Create HostEntry").style(
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
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
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            let text = format!("{}: {value}", LABELS[index]);
                            let scroll =
                                text.chars().count().saturating_sub(area.width as usize) as u16;
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
                    .map_err(|error| {
                        format!("HOST_CREATE_REQUIRED: cannot render form: {error}")
                    })?;
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
                        let current = roots
                            .iter()
                            .position(|root| root.path.to_string_lossy() == fields[0]);
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
                        fields[3] = root
                            .path
                            .parent()
                            .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
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
                    } => selected = (selected + 1) % fields.len(),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => selected = selected.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
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
