use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Wrap},
};

use super::CANCELLED;
use super::terminal::{read_key, with_terminal};

pub(crate) fn setup_workspace(
    roots: &[sshx::settings::RegisteredRoot],
    mut fields: [String; 3],
    status: Option<&str>,
) -> Result<([String; 3], bool), String> {
    const LABELS: [&str; 3] = [
        "Scope (personal/work)",
        "Project (optional)",
        "SSH config file path",
    ];
    with_terminal(
        "Setup",
        "SETUP_REQUIRED: Setup requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let mut selected = if fields[0].is_empty() {
                0
            } else if fields[2].is_empty() {
                2
            } else {
                0
            };
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
                        return Ok((fields, true));
                    }
                    KeyEvent {
                        code: KeyCode::Tab | KeyCode::Down,
                        ..
                    } => {
                        selected = (selected + 1) % fields.len();
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => selected = selected.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => root_scroll = root_scroll.saturating_sub(8),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => root_scroll = root_scroll.saturating_add(8),
                    KeyEvent {
                        code: KeyCode::Char('u'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        fields[selected].clear();
                    }
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
