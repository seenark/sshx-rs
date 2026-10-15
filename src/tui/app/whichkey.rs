use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use sshx::keymap::MenuEntry;

pub(super) fn draw(frame: &mut Frame, area: Rect, title: &str, entries: &[MenuEntry]) {
    let key_width = entries
        .iter()
        .map(|entry| entry.key_label.chars().count())
        .max()
        .unwrap_or(1);
    let text_width = entries
        .iter()
        .map(|entry| entry.description.chars().count())
        .max()
        .unwrap_or(0);
    let title = if title.is_empty() { "Keys" } else { title };
    let inner = (key_width + 3 + text_width).max(title.chars().count() + 2);
    let width = u16::try_from(inner + 2).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(entries.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let popup = Rect::new(
        area.right().saturating_sub(width),
        area.bottom().saturating_sub(height),
        width,
        height,
    );
    let lines = entries
        .iter()
        .map(|entry| {
            let style = if entry.available {
                Style::default()
            } else {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM)
            };
            Line::from(vec![
                Span::styled(
                    format!("{:<key_width$}", entry.key_label),
                    style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", entry.description), style),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(" {title} "))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup,
    );
}
