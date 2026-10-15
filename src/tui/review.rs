use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::Paragraph,
};

use super::CANCELLED;
use super::terminal::{read_key, with_terminal};
use super::widgets::wrap_status;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MutationReviewAction {
    Apply,
    Edit,
    Acknowledge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MutationReviewMode {
    EditApply,
    Delete,
    PreviewOnly,
    Blocked,
}

pub(crate) fn mutation_review_workspace(
    title: &str,
    review: &str,
    mode: MutationReviewMode,
) -> Result<MutationReviewAction, String> {
    with_terminal(
        title,
        "MUTATION_REVIEW_REQUIRED: reviewing a mutation requires usable stdin and stderr terminals"
            .to_string(),
        |terminal, input| {
            let mut scroll = 0u16;
            let mut scroll_step = 1u16;
            let footer = match mode {
                MutationReviewMode::EditApply => {
                    "Enter apply · E/Esc edit · Ctrl-C cancel · PgUp/PgDn scroll"
                }
                MutationReviewMode::Delete => "Enter delete · Esc/Ctrl-C cancel · PgUp/PgDn scroll",
                MutationReviewMode::PreviewOnly => {
                    "Enter finish preview · Esc/Ctrl-C cancel · PgUp/PgDn scroll"
                }
                MutationReviewMode::Blocked => {
                    "Enter/Esc return · Ctrl-C cancel · PgUp/PgDn scroll"
                }
            };
            loop {
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        let footer_lines = wrap_status(footer, area.width as usize);
                        let footer_height =
                            (footer_lines.len() as u16).min(area.height.saturating_sub(2));
                        let rows = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Min(1),
                                Constraint::Length(footer_height),
                            ])
                            .split(area);
                        frame.render_widget(
                            Paragraph::new(title).style(
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            rows[0],
                        );
                        let review_lines = wrap_status(review, rows[1].width as usize);
                        scroll_step = rows[1].height.max(1);
                        scroll = scroll.min(
                            review_lines
                                .len()
                                .saturating_sub(rows[1].height as usize)
                                .min(u16::MAX as usize) as u16,
                        );
                        frame.render_widget(
                            Paragraph::new(review_lines).scroll((scroll, 0)),
                            rows[1],
                        );
                        frame.render_widget(
                            Paragraph::new(footer_lines).style(Style::default().fg(Color::Cyan)),
                            rows[2],
                        );
                    })
                    .map_err(|error| {
                        format!("MUTATION_REVIEW_REQUIRED: cannot render mutation review: {error}")
                    })?;
                match read_key(input, "MUTATION_REVIEW_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if mode == MutationReviewMode::Blocked
                        || mode == MutationReviewMode::PreviewOnly =>
                    {
                        return Ok(MutationReviewAction::Acknowledge);
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => return Ok(MutationReviewAction::Apply),
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } if mode == MutationReviewMode::EditApply => {
                        return Ok(MutationReviewAction::Edit);
                    }
                    KeyEvent {
                        code: KeyCode::Char('e' | 'E'),
                        ..
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
                        code: KeyCode::Down,
                        ..
                    } => scroll = scroll.saturating_add(1),
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => scroll = scroll.saturating_sub(1),
                    KeyEvent {
                        code: KeyCode::PageDown,
                        ..
                    } => scroll = scroll.saturating_add(scroll_step),
                    KeyEvent {
                        code: KeyCode::PageUp,
                        ..
                    } => scroll = scroll.saturating_sub(scroll_step),
                    KeyEvent {
                        code: KeyCode::Home,
                        ..
                    } => scroll = 0,
                    _ => {}
                }
            }
        },
    )
}
