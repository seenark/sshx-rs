use super::{Ctx, Effect, Outcome, Screen};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use sshx::keymap::{self, Action, Availability, InputMode, Key};

use crate::tui::widgets::block;

/// A y/N question that runs `effect` on `y` and closes on anything else.
pub(super) struct YesNo {
    title: String,
    detail: Vec<String>,
    effect: Option<Effect>,
}

impl YesNo {
    pub(super) fn new(title: impl Into<String>, detail: Vec<String>, effect: Effect) -> Self {
        Self {
            title: title.into(),
            detail,
            effect: Some(effect),
        }
    }
}

impl Screen for YesNo {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::DeleteConfirm
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Normal
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn dismiss_on_outcome(&self) -> bool {
        true
    }

    fn on_action(&mut self, action: Action, _count: usize, _ctx: &Ctx) -> Outcome {
        match action {
            Action::Confirm => self.effect.take().map_or(Outcome::Pop, Outcome::Effect),
            Action::Back => Outcome::Pop,
            Action::Quit => Outcome::Effect(Effect::Quit { cancelled: false }),
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, _key: Key, _ctx: &Ctx) -> Outcome {
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let width = 64.min(area.width);
        let height = u16::try_from(self.detail.len() + 4)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let [row] = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .areas(row);
        let mut lines = self
            .detail
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect::<Vec<_>>();
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("y", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" confirm · "),
            Span::styled("n", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" cancel"),
        ]));
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block(format!(" {} ", self.title), Color::Yellow)),
            popup,
        );
    }
}
