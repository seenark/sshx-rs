use ratatui::backend::Backend;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
};
use sshx::session::{DeclaredService, ServiceForward};

use super::connection::{ConnectionMode, WorkspaceFocus};
use super::terminal::AppTerminal;
use super::widgets::block;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_connection_workspace(
    terminal: &mut AppTerminal,
    route: &str,
    services: &[DeclaredService],
    forwards: &[ServiceForward],
    checked: &[bool],
    selected: usize,
    editing: &Option<(usize, String)>,
    row_errors: &[Vec<String>],
    mode: ConnectionMode,
    status: Option<&str>,
    status_scroll: &mut u16,
    route_scroll: &mut u16,
    row_scroll: &mut u16,
    focus: WorkspaceFocus,
    review: bool,
) -> Result<(), String> {
    let mode_label = match mode {
        ConnectionMode::Session => "Session",
        ConnectionMode::Tunnel => "Tunnel",
    };
    let ownership = match mode {
        ConnectionMode::Session => "Session-bound master; closes when shell ends",
        ConnectionMode::Tunnel => "Standalone tunnel; remains after TUI exits",
    };
    if terminal
        .backend()
        .size()
        .map_or(true, |area| area.width == 0 || area.height == 0)
    {
        eprintln!(
            "Connection workspace\nMode: {mode_label}\nOwnership: {ownership}\nRoute: {route}"
        );
        if let Some(status) = status {
            eprintln!("{status}");
        }
        for (index, service) in services.iter().enumerate() {
            if service.id.starts_with("remote:") {
                eprintln!(
                    "{} Remote (-R) {}; server bind may expose service; OpenSSH reports startup failures.",
                    if checked[index] { "[x]" } else { "[ ]" },
                    &service.id["remote:".len()..]
                );
            } else if service.id.starts_with("socks:") {
                eprintln!(
                    "{} SOCKS (-D) local proxy {}; applications configure proxy explicitly.",
                    if checked[index] { "[x]" } else { "[ ]" },
                    &service.id["socks:".len()..]
                );
            } else {
                eprintln!(
                    "{} {} | server {}:{} | local 127.0.0.1:{}",
                    if checked[index] { "[x]" } else { "[ ]" },
                    service.id,
                    service.destination_host,
                    service.remote_port,
                    forwards[index].local_port
                );
            }
            for error in &row_errors[index] {
                eprintln!("  ! {error}");
            }
        }
        eprintln!(
            "↑↓ row  Space select  a -L  r -R  d -D  e edit  x remove custom/R/D  m mode  Enter review  Esc back  Ctrl-C cancel"
        );
        return Ok(());
    }
    terminal
        .draw(|frame| {
            let area = frame.area();
            let width = area.width.saturating_sub(2).max(1) as usize;
            let row_width = area.width.saturating_sub(4).max(1) as usize;
            let heading = format!(
                "Mode: {mode_label} [m toggles]\nOwnership: {ownership}\nRoute: {route}"
            );
            let heading_lines = wrap_status(&heading, width);
            let status_lines = status.map_or_else(Vec::new, |value| wrap_status(value, width));
            let row_detail = services.get(selected).map_or_else(
                || "No declared service forwards; Session can start without forwards.".to_string(),
                |service| {
                    let forward = &forwards[selected];
                    let local_port = editing
                        .as_ref()
                        .filter(|(row, _)| *row == selected)
                        .map_or_else(|| forward.local_port.to_string(), |(_, value)| value.clone());
                    let mut detail = if service.id.starts_with("remote:") {
                        format!(
                            "Remote (-R) [{}]\n-R {}\nServer bind/listener; may expose beyond loopback. OpenSSH reports startup failure; local checks cannot verify listener.",
                            if checked[selected] { "checked" } else { "unchecked" },
                            editing.as_ref().filter(|(row, _)| *row == selected)
                                .map_or_else(|| service.id["remote:".len()..].to_string(), |(_, value)| value.clone())
                        )
                    } else if service.id.starts_with("socks:") {
                        format!(
                            "SOCKS (-D) [{}]\n-D {}\nLocal proxy; applications must configure proxy explicitly.",
                            if checked[selected] { "checked" } else { "unchecked" },
                            editing.as_ref().filter(|(row, _)| *row == selected)
                                .map_or_else(|| service.id["socks:".len()..].to_string(), |(_, value)| value.clone())
                        )
                    } else {
                        let selection = if width < 20 {
                            format!(
                                "{} [{}]",
                                service.id,
                                if checked[selected] { "checked" } else { "unchecked" }
                            )
                        } else {
                            format!(
                                "Selected service {} [{}]",
                                service.id,
                                if checked[selected] { "checked" } else { "unchecked" }
                            )
                        };
                        format!(
                            "{selection}\nserver {}:{}\nlocal 127.0.0.1:{}",
                            service.destination_host, service.remote_port, local_port
                        )
                    };
                    for error in &row_errors[selected] {
                        detail.push_str("\nConflict: ");
                        detail.push_str(error);
                    }
                    detail
                },
            );
            let row_lines = wrap_status(&row_detail, width);
            let (detail_lines, requested_detail_scroll) = match focus {
                WorkspaceFocus::Route => (Vec::new(), 0),
                WorkspaceFocus::Row => (row_lines, *row_scroll),
                WorkspaceFocus::Status => (status_lines, *status_scroll),
            };
            let page_tabs = if status.is_some() {
                "Tab route/row/message"
            } else {
                "Tab route/row"
            };
            let scroll_target = match focus {
                WorkspaceFocus::Route => "route",
                WorkspaceFocus::Row => "row",
                WorkspaceFocus::Status => "message",
            };
            let footer = if width < 20 {
                if review {
                    "↑↓ rows\nEnter confirm\nEsc edit\nTab PgUp/Dn".to_string()
                } else if editing.is_some() {
                    "Type row\nEnter save\nEsc cancel\nPgUp/Dn row".to_string()
                } else {
                    "↑↓ Space · Tab\na -L r -R d -D\ne edit x remove\nm mode · Enter\nreview · Esc back".to_string()
                }
            } else if width < 36 {
                if review {
                    "↑↓ inspect\nEnter confirm\nEsc edit\nTab pane PgUp/Dn".to_string()
                } else if editing.is_some() {
                    "Type row\nEnter save\nEsc cancel\nPgUp/Dn row".to_string()
                } else {
                    "↑↓ move Space\na -L · r -R · d -D\ne edit · x remove\nm mode · Enter review · Esc back".to_string()
                }
            } else if review {
                format!(
                    "Review · ↑↓ rows · Enter confirm · Esc edit\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            } else if editing.is_some() {
                format!(
                    "Type row · Enter save · Esc cancel\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            } else {
                format!(
                    "↑↓ row · Space select · a -L · r -R · d -D · e edit\nx remove custom/R/D · m mode · Enter review · Esc back · Ctrl-C cancel\n{page_tabs} · PgUp/PgDn scroll {scroll_target}"
                )
            };
            let footer_lines = wrap_status(&footer, width);
            let footer_height = footer_lines.len() as u16;
            let detail_height = if matches!(focus, WorkspaceFocus::Route) {
                0
            } else {
                (detail_lines.len() as u16)
                    .min(3)
                    .min(area.height.saturating_sub(footer_height + 4))
            };
            let header_height = (heading_lines.len() as u16 + 2)
                .min(area.height.saturating_sub(detail_height + footer_height + 1))
                .max(3);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Length(detail_height),
                    Constraint::Min(1),
                    Constraint::Length(footer_height),
                ])
                .split(area);
            *route_scroll = (*route_scroll).min(
                heading_lines
                    .len()
                    .saturating_sub(chunks[0].height.saturating_sub(2) as usize)
                    .min(u16::MAX as usize) as u16,
            );
            let detail_scroll = requested_detail_scroll.min(
                detail_lines
                    .len()
                    .saturating_sub(chunks[1].height as usize)
                    .min(u16::MAX as usize) as u16,
            );
            match focus {
                WorkspaceFocus::Row => *row_scroll = detail_scroll,
                WorkspaceFocus::Status => *status_scroll = detail_scroll,
                WorkspaceFocus::Route => {}
            }
            let workspace_title = if width < 36 {
                " Workspace "
            } else {
                " Connection workspace "
            };
            frame.render_widget(
                Paragraph::new(heading_lines)
                    .block(block(workspace_title, Color::Cyan))
                    .scroll((*route_scroll, 0)),
                chunks[0],
            );
            if detail_height > 0 {
                frame.render_widget(
                    Paragraph::new(detail_lines).scroll((detail_scroll, 0)),
                    chunks[1],
                );
            }
            let items = if services.is_empty() {
                vec![ListItem::new("No local forwards; Session can start without forwards.")]
            } else {
                services
                    .iter()
                    .enumerate()
                    .map(|(index, service)| {
                        let local_port = editing
                            .as_ref()
                            .filter(|(row, _)| *row == index)
                            .map_or_else(|| forwards[index].local_port.to_string(), |(_, value)| {
                                value.clone()
                            });
                        let label = if service.id.starts_with("remote:") {
                            format!(
                                "{} (-R) server listener; your-side destination {}:{}",
                                service.id, service.destination_host, service.remote_port
                            )
                        } else if service.id.starts_with("socks:") {
                            format!(
                                "{} (-D) local proxy; applications configure explicitly",
                                service.id
                            )
                        } else {
                            format!(
                                "{} {} server {}:{} | local 127.0.0.1:{}",
                                if checked[index] { "[x]" } else { "[ ]" },
                                service.id,
                                service.destination_host,
                                service.remote_port,
                                local_port
                            )
                        };
                        let row = if width < 20 {
                            let label = if service.id.starts_with("remote:") {
                                "Remote -R".to_string()
                            } else if service.id.starts_with("socks:") {
                                "SOCKS -D".to_string()
                            } else if service.id.starts_with("custom ") {
                                format!("{} -L", service.id)
                            } else {
                                service.id.clone()
                            };
                            format!(
                                "{label} [{}]",
                                if checked[index] { "checked" } else { "unchecked" }
                            )
                        } else {
                            format!(
                                "{} {}",
                                if checked[index] { "[x]" } else { "[ ]" },
                                label
                            )
                        };
                        let mut lines = wrap_status(&row, row_width);
                        for error in &row_errors[index] {
                            lines.extend(
                                wrap_status(&format!("! {error}"), row_width)
                                    .into_iter()
                                    .map(|line| line.style(Style::default().fg(Color::Red))),
                            );
                        }
                        ListItem::new(lines)
                    })
                    .collect()
            };
            let list = List::new(items)
                .block(block(" Forwarding rows ", Color::Blue))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");
            let mut list_state = ListState::default();
            list_state.select(services.is_empty().then_some(0).or(Some(selected)));
            frame.render_stateful_widget(list, chunks[2], &mut list_state);
            frame.render_widget(
                Paragraph::new(footer_lines).style(Style::default().fg(Color::DarkGray)),
                chunks[3],
            );
        })
        .map(|_| ())
        .map_err(|error| format!("CONNECTION_REQUIRED: cannot render workspace: {error}"))
}

pub(super) fn wrap_status(value: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in value.split('\n') {
        let mut line = String::new();
        let mut line_width = 0;
        for grapheme in Span::raw(paragraph).styled_graphemes(Style::default()) {
            let grapheme_width = Span::raw(grapheme.symbol).width();
            if line_width > 0 && line_width + grapheme_width > width {
                lines.push(Line::from(std::mem::take(&mut line)));
                line_width = 0;
            }
            line.push_str(grapheme.symbol);
            line_width += grapheme_width;
        }
        lines.push(Line::from(line));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::wrap_status;

    #[test]
    fn wraps_wide_unicode_route_by_display_cell_width() {
        let lines = wrap_status("Route: /项目/服务", 8);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content.as_ref(), "Route: /");
        assert_eq!(lines[1].spans[0].content.as_ref(), "项目/服");
        assert_eq!(lines[2].spans[0].content.as_ref(), "务");
        assert_eq!(
            lines.iter().map(|line| line.width()).collect::<Vec<_>>(),
            [8, 7, 2]
        );
    }

    #[test]
    fn preserves_repeated_spaces_when_wrapping() {
        let lines = wrap_status("/a  b", 3);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.spans[0].content.as_ref())
                .collect::<Vec<&str>>(),
            ["/a ", " b"]
        );
        assert_eq!(
            lines.iter().map(|line| line.width()).collect::<Vec<_>>(),
            [3, 2]
        );
    }
}
