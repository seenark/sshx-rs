use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
};

pub(super) fn password_backspace(value: &mut String, edited: &mut bool) {
    if *edited {
        value.pop();
    } else {
        value.clear();
    }
    *edited = true;
}

pub(super) fn password_insert(value: &mut String, edited: &mut bool, character: char) {
    if !*edited && value.as_str() == "********" {
        value.clear();
    }
    *edited = true;
    value.push(character);
}
pub(super) fn block(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
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
    use super::{password_backspace, password_insert, wrap_status};

    #[test]
    fn password_marker_backspace_and_literal_stars_are_distinct() {
        let mut value = "********".to_string();
        let mut edited = false;
        password_backspace(&mut value, &mut edited);
        assert!(value.is_empty());
        assert!(edited);

        let mut value = "********".to_string();
        let mut edited = false;
        for _ in 0..8 {
            password_insert(&mut value, &mut edited, '*');
        }
        password_insert(&mut value, &mut edited, 'x');
        assert_eq!(value, "********x");
    }

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
