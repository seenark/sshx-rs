use super::{ConnectRequest, Ctx, Effect, Outcome, Screen, copy::CopyRequest, host_form::HostForm};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use sshx::connect::{Hop, SessionFailure};
use sshx::keymap::{self, Action, Availability, CopyTarget, InputMode, Key};

/// Shown when a session-bound master fails to start; `pair` holds the gateway and VM aliases.
pub(super) struct Failure {
    request: ConnectRequest,
    failure: SessionFailure,
    pair: Option<(String, String)>,
    log: String,
    expanded: bool,
}

impl Failure {
    pub(super) fn new(
        request: &ConnectRequest,
        failure: SessionFailure,
        pair: Option<(String, String)>,
        log: String,
    ) -> Self {
        Self {
            request: request.clone(),
            failure,
            pair,
            log,
            expanded: false,
        }
    }

    fn step(&self) -> Vec<Span<'static>> {
        let mark = |hop: Hop, name: String| {
            if self.failure.hop == hop {
                Span::styled(
                    format!("{name} ✗"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(name)
            }
        };
        match &self.pair {
            Some((gateway, vm)) => vec![
                mark(Hop::Gateway, format!("gateway {gateway}")),
                Span::raw(" → "),
                mark(Hop::Vm, format!("VM {vm}")),
            ],
            None => vec![Span::raw(self.request.alias.clone())],
        }
    }

    /// The HostEntry that failed: the gateway for a gateway hop, otherwise the selected one.
    /// The form replaces this screen, since a saved edit makes the request stale.
    fn edit(&self, ctx: &Ctx) -> Outcome {
        let selected = &self.request.entry;
        let Some(entry) = ctx
            .entries
            .iter()
            .find(|entry| entry.id == selected.id && entry.source.path == selected.source.path)
        else {
            return Outcome::Toast(format!(
                "HOST_NOT_FOUND: {} changed on disk",
                self.request.alias
            ));
        };
        let form = match (
            self.failure.hop,
            sshx::pair::paired_route(ctx.entries, entry),
        ) {
            (Hop::Gateway, Ok(Some(route))) => {
                let alias = route.gateway.aliases.first().cloned().unwrap_or_default();
                HostForm::edit(ctx, &route.gateway, &alias)
            }
            _ => HostForm::edit(ctx, entry, &self.request.alias),
        };
        match form {
            Ok(form) => Outcome::Replace(Box::new(form)),
            Err(error) => Outcome::Toast(error),
        }
    }
}

impl Screen for Failure {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::Failure
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Normal
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    /// Retry, copy and edit leave the screen; the finder underneath keeps the selected row.
    fn dismiss_on_outcome(&self) -> bool {
        true
    }

    fn on_action(&mut self, action: Action, _count: usize, ctx: &Ctx) -> Outcome {
        match action {
            Action::Back => Outcome::Pop,
            Action::Retry => Outcome::Effect(Effect::Connect(Box::new(self.request.clone()))),
            Action::EditHostForm => self.edit(ctx),
            Action::Copy(CopyTarget::VerboseSshCommand) => {
                Outcome::Effect(Effect::Copy(Box::new(CopyRequest {
                    target: CopyTarget::VerboseSshCommand,
                    entry: self.request.entry.clone(),
                    alias: self.request.alias.clone(),
                    forwards: Vec::new(),
                    tunnel_id: None,
                })))
            }
            Action::ShowLog => {
                self.expanded = !self.expanded;
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, _key: Key, _ctx: &Ctx) -> Outcome {
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let outer = block(format!(" ssh {} failed ", self.request.alias), Color::Red);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let label =
            |text: &str| Span::styled(format!("{text:<8}"), Style::default().fg(Color::DarkGray));
        let mut step = vec![label("step:")];
        step.extend(self.step());
        let mut lines = vec![
            Line::from(step),
            Line::from(vec![label("error:"), Span::raw(self.failure.code.clone())]),
            Line::from(vec![label(""), Span::raw(self.failure.detail.clone())]),
        ];
        if self.expanded {
            lines.push(Line::default());
            lines.push(Line::from(label("log:")));
            lines.extend(self.log.lines().map(|text| Line::from(text.to_string())));
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "r retry   e edit host   y copy ssh -v command   l show full log   esc back",
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::ForwardRequest;
    use ratatui::{Terminal, backend::TestBackend};
    use sshx::discovery::{HostEntry, SourceIdentity};
    use sshx::settings::Preferences;
    use sshx::usage::UsageIndex;
    use std::collections::HashMap;
    use std::path::Path;

    fn request() -> ConnectRequest {
        ConnectRequest {
            entry: HostEntry {
                id: "id".to_string(),
                aliases: vec!["vm".to_string()],
                source: SourceIdentity {
                    path: "/c".to_string(),
                    byte_start: 0,
                    byte_end: 1,
                    line_start: 1,
                    line_end: 1,
                },
                destination: None,
                provenance: Vec::new(),
                scopes: Vec::new(),
                projects: Vec::new(),
            },
            alias: "vm".to_string(),
            forwards: vec![ForwardRequest {
                remote: 5432,
                local: 15432,
            }],
        }
    }

    fn failure(hop: Hop, pair: Option<(&str, &str)>) -> Failure {
        Failure::new(
            &request(),
            SessionFailure {
                hop,
                code: "SSH_AUTH_FAILED".to_string(),
                detail: "Permission denied (publickey)".to_string(),
            },
            pair.map(|(gateway, vm)| (gateway.to_string(), vm.to_string())),
            "VM_AUTH_FAILED: SSH_AUTH_FAILED: Permission denied (publickey)".to_string(),
        )
    }

    fn with_ctx<R>(run: impl FnOnce(&Ctx) -> R) -> R {
        let usage = UsageIndex::default();
        let prefs = Preferences::default();
        let counts = HashMap::new();
        run(&Ctx {
            home: Path::new("/h"),
            entries: &[],
            visible: Vec::new(),
            roots: &[],
            usage: &usage,
            tunnels: &[],
            prefs: &prefs,
            service_counts: &counts,
            now: 0,
            toast: None,
        })
    }

    fn render(screen: &Failure) -> String {
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        with_ctx(|ctx| {
            terminal
                .draw(|frame| screen.draw(frame, frame.area(), ctx))
                .unwrap()
        });
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(90)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn marks_only_the_failing_pair_hop() {
        let screen = render(&failure(Hop::Vm, Some(("gw", "vm"))));
        assert!(screen.contains("gateway gw → VM vm ✗"), "{screen}");
        assert!(screen.contains("Permission denied (publickey)"), "{screen}");
        let screen = render(&failure(Hop::Gateway, Some(("gw", "vm"))));
        assert!(screen.contains("gateway gw ✗ → VM vm"), "{screen}");
    }

    #[test]
    fn direct_failure_names_the_alias_and_show_log_toggles() {
        let mut screen = failure(Hop::Direct, None);
        assert!(render(&screen).contains("step:   vm"));
        assert!(!render(&screen).contains("VM_AUTH_FAILED"));
        with_ctx(|ctx| screen.on_action(Action::ShowLog, 1, ctx));
        assert!(render(&screen).contains("VM_AUTH_FAILED"));
        with_ctx(|ctx| screen.on_action(Action::ShowLog, 1, ctx));
        assert!(!render(&screen).contains("VM_AUTH_FAILED"));
    }

    #[test]
    fn retry_reruns_the_same_connect_request() {
        let mut screen = failure(Hop::Direct, None);
        let outcome = with_ctx(|ctx| screen.on_action(Action::Retry, 1, ctx));
        let Outcome::Effect(Effect::Connect(request)) = outcome else {
            panic!("retry should emit Connect");
        };
        assert_eq!(request.alias, "vm");
        assert_eq!(request.forwards.len(), 1);
        assert_eq!(request.forwards[0].local, 15432);
    }
}
