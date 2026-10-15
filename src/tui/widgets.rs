use ratatui::{
    style::{Color, Style},
    text::Line,
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

pub(super) fn toggle_password_clear(
    value: &mut String,
    original: &str,
    cleared: &mut bool,
    edited: &mut bool,
    original_edited: bool,
) {
    *cleared = !*cleared;
    if *cleared {
        value.clear();
    } else {
        value.clear();
        value.push_str(original);
        *edited = original_edited;
    }
}

pub(super) fn block(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
}

#[cfg(test)]
mod tests {
    use super::{password_backspace, password_insert, toggle_password_clear};

    #[test]
    fn password_clear_toggle_restores_form_baseline() {
        let original = "********";
        let mut value = "replacement-secret".to_string();
        let mut cleared = false;
        let mut edited = true;
        let original_edited = false;

        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            original_edited,
        );
        assert!(cleared);
        assert!(value.is_empty());
        toggle_password_clear(
            &mut value,
            original,
            &mut cleared,
            &mut edited,
            original_edited,
        );
        assert!(!cleared);
        assert_eq!(value, original);
        assert!(!edited);

        let original = "edited-secret";
        let mut value = original.to_string();
        let mut cleared = false;
        let mut edited = true;
        toggle_password_clear(&mut value, original, &mut cleared, &mut edited, true);
        toggle_password_clear(&mut value, original, &mut cleared, &mut edited, true);
        assert_eq!(value, original);
        assert!(edited);
    }

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
}
