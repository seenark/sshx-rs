use super::{Ctx, Effect, Outcome, Screen, cmdline::CommandLine, finder::Finder, input_mode};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::settings::{self, RegisteredRoot};
use std::path::Path;

struct Candidate {
    root: RegisteredRoot,
    ticked: bool,
}

pub(super) struct FirstRun {
    candidates: Vec<Candidate>,
    selected: usize,
}

fn detect(home: &Path) -> Vec<Candidate> {
    let mut candidates = settings::auto_detect(home)
        .into_iter()
        .map(|root| Candidate { root, ticked: true })
        .collect::<Vec<_>>();
    let mut included = std::fs::read_dir(home.join(".ssh/config.d"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    included.sort();
    for path in included {
        if !candidates
            .iter()
            .any(|candidate| candidate.root.path == path)
        {
            candidates.push(Candidate {
                root: RegisteredRoot {
                    scope: "personal".to_string(),
                    path,
                    project: None,
                },
                ticked: false,
            });
        }
    }
    candidates
}

impl FirstRun {
    pub(super) fn new(home: &Path) -> Self {
        Self {
            candidates: detect(home),
            selected: 0,
        }
    }

    fn label(&mut self, scope: &str) {
        if let Some(candidate) = self.candidates.get_mut(self.selected) {
            candidate.root.scope = scope.to_string();
        }
    }

    fn current(&mut self) -> Option<&mut Candidate> {
        self.candidates.get_mut(self.selected)
    }
}

impl Screen for FirstRun {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::FirstRun
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Normal
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn on_action(&mut self, action: Action, _count: usize, ctx: &Ctx) -> Outcome {
        let last = self.candidates.len().saturating_sub(1);
        match action {
            Action::Down => self.selected = (self.selected + 1).min(last),
            Action::Up => self.selected = self.selected.saturating_sub(1),
            Action::ToggleRow => {
                if let Some(candidate) = self.current() {
                    candidate.ticked = !candidate.ticked;
                }
            }
            Action::LabelPersonal => self.label("personal"),
            Action::LabelWork => self.label("work"),
            Action::Back => {
                return Outcome::Replace(Box::new(Finder::new(input_mode(ctx.prefs.startmode))));
            }
            Action::Quit => return Outcome::Effect(Effect::Quit { cancelled: false }),
            Action::CommandLine => {
                return Outcome::Push(Box::new(CommandLine::new(
                    self.kind(),
                    self.input_mode(),
                    self.availability(ctx),
                )));
            }
            Action::Confirm => {
                let roots = self
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.ticked)
                    .map(|candidate| candidate.root.clone())
                    .collect::<Vec<_>>();
                if roots.is_empty() {
                    return Outcome::Toast("tick at least one config file with x".to_string());
                }
                return Outcome::Effect(Effect::RegisterRoots(roots));
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, _key: Key, _ctx: &Ctx) -> Outcome {
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let mut lines = vec![
            Line::from("No config roots are registered yet."),
            Line::from(
                "Tick the SSH config files sshx should read, and label each personal or work.",
            ),
            Line::default(),
        ];
        if self.candidates.is_empty() {
            lines.push(Line::from(
                "No SSH config files detected. Create ~/.ssh/config and start sshx again, or press s to continue without one.",
            ));
        }
        for (index, candidate) in self.candidates.iter().enumerate() {
            let style = if index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "[{}] {:<9} {}",
                    if candidate.ticked { 'x' } else { ' ' },
                    candidate.root.scope,
                    candidate.root.path.display()
                ),
                style,
            )));
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "x tick · p personal · w work · enter save · s skip · q quit · ? help",
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(
            Paragraph::new(lines).block(block(" Welcome to sshx ", Color::Cyan)),
            area,
        );
    }
}
