use super::{Ctx, Effect, Outcome, Screen};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use sshx::keymap::{self, Action, Availability, HelpRow, InputMode, Key, Section};

pub(super) struct Help {
    title: &'static str,
    sections: Vec<(Section, Vec<HelpRow>)>,
    mode: InputMode,
    filter: String,
    scroll: usize,
}

fn merge(
    screen: keymap::Screen,
    mode: InputMode,
    availability: &Availability,
) -> Vec<(Section, Vec<HelpRow>)> {
    let modes = if mode == InputMode::Normal {
        vec![InputMode::Normal]
    } else {
        vec![mode, InputMode::Normal]
    };
    let mut merged: Vec<(Section, Vec<HelpRow>)> = Vec::new();
    for mode in modes {
        for (section, rows) in keymap::help(screen, mode, availability) {
            let target = match merged.iter().position(|(existing, _)| *existing == section) {
                Some(index) => &mut merged[index].1,
                None => {
                    merged.push((section, Vec::new()));
                    &mut merged.last_mut().expect("section was just pushed").1
                }
            };
            for row in rows {
                if !target
                    .iter()
                    .any(|seen| seen.keys == row.keys && seen.description == row.description)
                {
                    target.push(row);
                }
            }
        }
    }
    if let Some(home) = merged.first().map(|(section, _)| *section) {
        merged.sort_by_key(|(section, _)| (*section != home, *section));
    }
    merged
}

fn screen_title(screen: keymap::Screen) -> &'static str {
    match screen {
        keymap::Screen::Finder => "Finder",
        keymap::Screen::Ports => "Ports",
        keymap::Screen::HostForm => "Host form",
        keymap::Screen::DeleteConfirm => "Delete",
        keymap::Screen::Tunnels => "Tunnels",
        keymap::Screen::TunnelProgress => "Tunnel progress",
        keymap::Screen::Failure => "Failure",
        keymap::Screen::Help => "Help",
        keymap::Screen::CommandLine => "Command line",
    }
}

fn fuzzy_matches(haystack: &str, needle: &str) -> bool {
    let mut remaining = haystack.chars().flat_map(char::to_lowercase);
    needle
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| remaining.any(|candidate| candidate == wanted))
}

impl Help {
    pub(super) fn new(screen: keymap::Screen, mode: InputMode, availability: Availability) -> Self {
        Self {
            title: screen_title(screen),
            sections: merge(screen, mode, &availability),
            mode: InputMode::Normal,
            filter: String::new(),
            scroll: 0,
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (section, rows) in &self.sections {
            let rows = rows
                .iter()
                .filter(|row| {
                    self.filter.is_empty()
                        || fuzzy_matches(
                            &format!("{} {} {}", row.keys, row.description, section.title()),
                            &self.filter,
                        )
                })
                .collect::<Vec<_>>();
            if rows.is_empty() {
                continue;
            }
            lines.push(Line::from(Span::styled(
                section.title(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            for row in rows {
                let style = if row.available {
                    Style::default()
                } else {
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM)
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<14}", row.keys),
                        style.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(row.description, style),
                ]));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from("No matching keys."));
        }
        lines
    }
}

impl Screen for Help {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::Help
    }

    fn input_mode(&self) -> InputMode {
        self.mode
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn query_is_empty(&self) -> bool {
        self.filter.is_empty()
    }

    fn on_action(&mut self, action: Action, count: usize, _ctx: &Ctx) -> Outcome {
        let last = self.lines().len().saturating_sub(1);
        match action {
            Action::Back => return Outcome::Pop,
            Action::Quit => return Outcome::Effect(Effect::Quit { cancelled: false }),
            Action::EnterInsert => self.mode = InputMode::Insert,
            Action::Up => self.scroll = self.scroll.saturating_sub(count),
            Action::Down => self.scroll = (self.scroll + count).min(last),
            Action::HalfPageUp => self.scroll = self.scroll.saturating_sub(count * 10),
            Action::HalfPageDown => self.scroll = (self.scroll + count * 10).min(last),
            Action::Top => self.scroll = 0,
            Action::Bottom => self.scroll = last,
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        match key {
            Key::Char(character) => {
                self.filter.push(character);
                self.scroll = 0;
            }
            Key::Backspace => {
                self.filter.pop();
                self.scroll = 0;
            }
            Key::Enter => self.mode = InputMode::Normal,
            _ => {}
        }
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let width = (area.width * 4 / 5).max(area.width.min(40)).min(area.width);
        let height = (area.height * 4 / 5)
            .max(area.height.min(10))
            .min(area.height);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let outer = Block::default()
            .title(format!(" Help · {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer.inner(popup);
        frame.render_widget(outer, popup);
        let [filter_area, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
        let cursor = if self.mode == InputMode::Insert {
            "▏"
        } else {
            ""
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Filter: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{}{cursor}", self.filter)),
            ])),
            filter_area,
        );
        let lines = self
            .lines()
            .into_iter()
            .skip(self.scroll)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), body);
    }
}
