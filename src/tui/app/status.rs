use super::ScreenStatus;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use sshx::keymap::InputMode;

pub(super) struct StatusLine<'a> {
    pub mode: InputMode,
    pub info: ScreenStatus,
    pub running_tunnels: usize,
    pub toast: Option<&'a str>,
    pub pending: &'a str,
}

fn mode_label(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Insert => "INSERT",
        InputMode::Normal => "NORMAL",
        InputMode::Select => "SELECT",
    }
}

pub(super) fn draw(frame: &mut Frame, area: Rect, status: &StatusLine) {
    let separator = || Span::styled(" │ ", Style::default().fg(Color::DarkGray));
    let position = status
        .info
        .position
        .map_or_else(String::new, |(index, total)| format!("{index}/{total}"));
    let mut left = vec![
        Span::raw("│ "),
        Span::styled(
            mode_label(status.mode),
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        ),
        separator(),
        Span::raw(status.info.filter.clone().unwrap_or_else(|| "all".into())),
        separator(),
        Span::raw(position),
        separator(),
        Span::raw(format!("⇄ {}", status.running_tunnels)),
    ];
    if let Some(toast) = status.toast {
        left.push(separator());
        left.push(Span::styled(
            toast.replace('\n', " · "),
            Style::default().fg(Color::Yellow),
        ));
    }
    let right = Line::from(Span::styled(
        status.pending.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let width = u16::try_from(right.width()).unwrap_or(u16::MAX);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(width + 1)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(left)), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}
