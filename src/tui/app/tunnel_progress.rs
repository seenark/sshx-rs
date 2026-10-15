use super::{Ctx, Effect, EffectResult, ForwardRequest, Outcome, Runtime, Screen};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::session::ServiceForward;
use sshx::tunnel::TunnelView;

const NO_FORWARDS: &str = "TUNNEL_FORWARD_REQUIRED: Tunnel mode needs at least one service forward; check rows in the ports screen (⇥) first";

/// A standalone tunnel whose local ports passed preflight.
pub(crate) struct TunnelStartRequest {
    pub entry: HostEntry,
    pub alias: String,
    pub forwards: Vec<ForwardRequest>,
}

fn service_forward(forward: &ForwardRequest) -> ServiceForward {
    ServiceForward {
        id: forward.remote.to_string(),
        remote_port: forward.remote,
        destination_host: "127.0.0.1".to_string(),
        local_port: forward.local,
    }
}

/// Preflight problems as `(index into forwards, message)`.
pub(super) fn issues(forwards: &[ForwardRequest]) -> Vec<(usize, String)> {
    let services = forwards.iter().map(service_forward).collect::<Vec<_>>();
    sshx::session::preflight_issues(&services)
        .into_iter()
        .filter_map(|issue| {
            let index = services
                .iter()
                .position(|service| service.id == issue.service_id)?;
            Some((index, issue.message))
        })
        .collect()
}

fn conflicts(forwards: &[ForwardRequest]) -> Vec<usize> {
    let mut indexes = issues(forwards)
        .into_iter()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indexes.sort_unstable();
    indexes.dedup();
    indexes
}

/// The service forwards selected last time for `entry`, from usage state.
pub(super) fn remembered_forwards(ctx: &Ctx, entry: &HostEntry) -> Vec<ForwardRequest> {
    ctx.usage.get(entry).map_or_else(Vec::new, |usage| {
        usage
            .forwards
            .iter()
            .map(|forward| ForwardRequest {
                remote: forward.remote,
                local: forward.local,
            })
            .collect()
    })
}

/// Preflight the requested local ports. Without conflicts the start effect runs at once;
/// otherwise the progress screen resolves them before anything reaches the registry.
pub(super) fn begin(entry: &HostEntry, alias: &str, forwards: Vec<ForwardRequest>) -> Outcome {
    if forwards.is_empty() {
        return Outcome::Toast(NO_FORWARDS.to_string());
    }
    let mut preflight = Preflight {
        entry: entry.clone(),
        alias: alias.to_string(),
        forwards,
        log: Vec::new(),
        step: Step::Choose(0),
        error: None,
    };
    match preflight.advance() {
        Outcome::Stay => Outcome::Push(Box::new(TunnelProgress {
            alias: alias.to_string(),
            state: State::Preflight(preflight),
        })),
        other => other,
    }
}

enum Step {
    Choose(usize),
    Pick { index: usize, input: String },
}

struct Preflight {
    entry: HostEntry,
    alias: String,
    forwards: Vec<ForwardRequest>,
    log: Vec<String>,
    step: Step,
    error: Option<String>,
}

impl Preflight {
    fn advance(&mut self) -> Outcome {
        match conflicts(&self.forwards).first() {
            Some(&index) => {
                self.step = Step::Choose(index);
                Outcome::Stay
            }
            None if self.forwards.is_empty() => {
                Outcome::Toast("TUNNEL_FORWARD_REQUIRED: every forward was skipped".to_string())
            }
            None => Outcome::Effect(Effect::TunnelStart(Box::new(TunnelStartRequest {
                entry: self.entry.clone(),
                alias: self.alias.clone(),
                forwards: self.forwards.clone(),
            }))),
        }
    }

    fn next_free(&self, index: usize) -> Option<u16> {
        let reserved = self
            .forwards
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, forward)| forward.local)
            .collect::<Vec<_>>();
        sshx::tunnel::next_free_local_port(self.forwards[index].local, &reserved)
    }

    fn substitute(&mut self, index: usize, port: u16) -> Outcome {
        let forward = &mut self.forwards[index];
        self.log.push(format!(
            "✓ :{} → :{port} (remote {})",
            forward.local, forward.remote
        ));
        forward.local = port;
        self.error = None;
        self.advance()
    }

    fn choose(&mut self, index: usize, key: char) -> Outcome {
        match key {
            'n' => match self.next_free(index) {
                Some(port) => self.substitute(index, port),
                None => {
                    self.error = Some("no free local port above this one".to_string());
                    Outcome::Stay
                }
            },
            'p' => {
                self.step = Step::Pick {
                    index,
                    input: String::new(),
                };
                self.error = None;
                Outcome::Stay
            }
            's' => {
                let forward = self.forwards.remove(index);
                self.log.push(format!(
                    "– :{} skipped (remote {})",
                    forward.local, forward.remote
                ));
                self.error = None;
                self.advance()
            }
            _ => Outcome::Stay,
        }
    }

    fn pick(&mut self, index: usize, key: Key) -> Outcome {
        let Step::Pick { input, .. } = &mut self.step else {
            return Outcome::Stay;
        };
        match key {
            Key::Char(character) if character.is_ascii_digit() => input.push(character),
            Key::Backspace => {
                input.pop();
            }
            Key::Esc => {
                self.step = Step::Choose(index);
                self.error = None;
            }
            Key::Enter => match input.parse::<u16>() {
                Ok(port) if port != 0 => return self.substitute(index, port),
                _ => self.error = Some("enter a port between 1 and 65535".to_string()),
            },
            _ => {}
        }
        Outcome::Stay
    }
}

enum State {
    Preflight(Preflight),
    Finished(Result<TunnelView, String>),
}

pub(super) struct TunnelProgress {
    alias: String,
    state: State,
}

impl TunnelProgress {
    /// Outcome of a start or restart, shown as evidence from the registry view.
    pub(super) fn finished(alias: &str, result: Result<TunnelView, String>) -> Self {
        Self {
            alias: alias.to_string(),
            state: State::Finished(result),
        }
    }
}

struct Evidence {
    ok: bool,
    text: String,
}

/// One line per master and per listener, each worded from what the registry view observed.
fn evidence(view: &TunnelView) -> Vec<Evidence> {
    let masters = match (&view.gateway_master_status, &view.vm_master_status) {
        (Some(gateway), Some(vm)) => vec![
            (view.gateway_alias.as_deref().unwrap_or("gateway"), gateway),
            (view.vm_alias.as_deref().unwrap_or("vm"), vm),
        ],
        _ => vec![(view.selected_alias.as_str(), &view.master_status)],
    };
    let mut lines = masters
        .into_iter()
        .map(|(alias, status)| Evidence {
            ok: status == "responsive",
            text: format!("{alias} master {status}"),
        })
        .collect::<Vec<_>>();
    lines.extend(
        view.forwards
            .iter()
            .filter(|forward| forward.flag() != 'R')
            .filter_map(|forward| forward.listener())
            .map(|(_, port)| {
                let ready = view.listener_status == "bound";
                Evidence {
                    ok: ready,
                    text: format!(
                        ":{port} listener {}",
                        if ready {
                            "ready"
                        } else {
                            &view.listener_status
                        }
                    ),
                }
            }),
    );
    if let Some(error) = &view.error {
        lines.push(Evidence {
            ok: false,
            text: error.clone(),
        });
    }
    lines
}

fn mark(ok: bool) -> Span<'static> {
    if ok {
        Span::styled("✓ ", Style::default().fg(Color::Green))
    } else {
        Span::styled("✗ ", Style::default().fg(Color::Red))
    }
}

impl Screen for TunnelProgress {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::TunnelProgress
    }

    fn input_mode(&self) -> InputMode {
        match &self.state {
            State::Preflight(Preflight {
                step: Step::Pick { .. },
                ..
            }) => InputMode::Insert,
            _ => InputMode::Normal,
        }
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn dismiss_on_outcome(&self) -> bool {
        true
    }

    fn on_action(&mut self, action: Action, _count: usize, _ctx: &Ctx) -> Outcome {
        match action {
            Action::Back => Outcome::Pop,
            Action::Quit => Outcome::Effect(Effect::Quit { cancelled: false }),
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        let State::Preflight(preflight) = &mut self.state else {
            return Outcome::Stay;
        };
        match (&preflight.step, key) {
            (Step::Choose(index), Key::Char(character)) => {
                let index = *index;
                preflight.choose(index, character)
            }
            (Step::Pick { index, .. }, key) => {
                let index = *index;
                preflight.pick(index, key)
            }
            _ => Outcome::Stay,
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let outer = block(format!(" tunnel · {} ", self.alias), Color::Cyan);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let dim = Style::default().fg(Color::DarkGray);
        let mut lines: Vec<Line> = Vec::new();
        match &self.state {
            State::Preflight(preflight) => {
                lines.push(Line::from(Span::styled("Preflight: local ports", dim)));
                lines.extend(preflight.log.iter().map(|entry| Line::from(entry.as_str())));
                let index = match &preflight.step {
                    Step::Choose(index) | Step::Pick { index, .. } => *index,
                };
                let forward = &preflight.forwards[index];
                lines.push(Line::from(vec![
                    mark(false),
                    Span::raw(format!(":{} port in use", forward.local)),
                    Span::styled(format!("  (remote {})", forward.remote), dim),
                ]));
                match &preflight.step {
                    Step::Choose(_) => {
                        let key = |text: &'static str| {
                            Span::styled(
                                format!("    {text}  "),
                                Style::default().add_modifier(Modifier::BOLD),
                            )
                        };
                        if let Some(port) = preflight.next_free(index) {
                            lines.push(Line::from(vec![
                                key("n"),
                                Span::raw(format!("use :{port} instead")),
                            ]));
                        }
                        lines.push(Line::from(vec![key("p"), Span::raw("pick another port")]));
                        lines.push(Line::from(vec![key("s"), Span::raw("skip")]));
                    }
                    Step::Pick { input, .. } => lines.push(Line::from(vec![
                        Span::styled("    local port: ", dim),
                        Span::raw(format!("{input}▏")),
                        Span::styled("   ⏎ confirm · esc back", dim),
                    ])),
                }
                if let Some(error) = &preflight.error {
                    lines.push(Line::from(Span::styled(
                        format!("    {error}"),
                        Style::default().fg(Color::Yellow),
                    )));
                }
            }
            State::Finished(Ok(view)) => {
                lines.extend(
                    evidence(view)
                        .into_iter()
                        .map(|line| Line::from(vec![mark(line.ok), Span::raw(line.text)])),
                );
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    format!("id {} · yi copy id", view.id),
                    dim,
                )));
            }
            State::Finished(Err(error)) => {
                lines.push(Line::from(vec![mark(false), Span::raw(error.clone())]));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

fn interrupted(error: &str) -> bool {
    error.starts_with("SESSION_INTERRUPTED") || error.starts_with("SESSION_START_INTERRUPTED")
}

/// Standalone tunnel start in cooked mode: the registry request is written only here, with the
/// ports that survived preflight. Failures stay on the evidence screen.
pub(super) fn start(
    runtime: &Runtime,
    request: &TunnelStartRequest,
) -> Result<EffectResult, String> {
    match open(runtime, request) {
        Err(error) if interrupted(&error) => Err(error),
        result => Ok(EffectResult {
            push: Some(Box::new(TunnelProgress::finished(&request.alias, result))),
            refresh: true,
            ..EffectResult::default()
        }),
    }
}

fn open(runtime: &Runtime, request: &TunnelStartRequest) -> Result<TunnelView, String> {
    let data = &runtime.data;
    if data
        .catalog
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "unsupported_match")
    {
        return Err("UNSUPPORTED_MATCH: Match prevents exact runtime configuration".to_string());
    }
    let route = sshx::pair::paired_route(&data.catalog.entries, &request.entry)?;
    if !crate::entry_sources_unchanged(&request.entry, &data.sources)
        || route
            .as_ref()
            .is_some_and(|route| !crate::entry_sources_unchanged(&route.gateway, &data.sources))
    {
        return Err(format!(
            "HOST_SOURCE_CHANGED: {} changed; select its current HostEntry again",
            request.entry.source.path
        ));
    }
    let target = route.as_ref().map_or(&request.entry, |route| &route.vm);
    let specifications = request
        .forwards
        .iter()
        .map(|forward| format!("{}={}", forward.remote, forward.local))
        .collect::<Vec<_>>();
    let forwards = sshx::session::resolve_forwards(target, &specifications)?;
    let response = match &route {
        Some(route) => {
            let gateway_alias = route
                .gateway
                .aliases
                .first()
                .ok_or_else(|| "PAIR_INVALID: gateway has no alias".to_string())?;
            sshx::tunnel::start_paired(
                route,
                &data.home,
                gateway_alias,
                &request.alias,
                false,
                sshx::connect::PairedCredentials {
                    gateway_password_fd: None,
                    vm_password_fd: None,
                },
                &forwards,
            )?
        }
        None => {
            let local = forwards
                .iter()
                .map(|forward| {
                    let mut specification = String::new();
                    sshx::session::write_local_forward_spec(&mut specification, forward);
                    specification
                })
                .collect::<Vec<_>>();
            sshx::tunnel::start(
                &request.entry,
                &data.home,
                &request.alias,
                false,
                &mut sshx::connect::StandaloneCredentials::new(None),
                &local,
                &[],
                &[],
                false,
            )?
        }
    };
    response
        .tunnels
        .into_iter()
        .next()
        .ok_or_else(|| "REGISTRY_INVALID: tunnel start returned no tunnel".to_string())
}

#[cfg(test)]
mod tests {
    use super::{ForwardRequest, begin, conflicts, evidence};
    use crate::tui::app::{Effect, Outcome};
    use sshx::discovery::{HostEntry, SourceIdentity};
    use sshx::session::ForwardSpec;
    use sshx::tunnel::TunnelView;
    use std::net::TcpListener;

    fn forward(remote: u16, local: u16) -> ForwardRequest {
        ForwardRequest { remote, local }
    }

    fn free_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn entry() -> HostEntry {
        HostEntry {
            id: "id".into(),
            aliases: vec!["db".into()],
            source: SourceIdentity {
                path: "/c".into(),
                byte_start: 0,
                byte_end: 1,
                line_start: 1,
                line_end: 1,
            },
            destination: None,
            provenance: Vec::new(),
            scopes: Vec::new(),
            projects: Vec::new(),
        }
    }

    fn view(master: &str, listener: &str, ports: &[u16]) -> TunnelView {
        TunnelView {
            id: "dt-1".into(),
            state: "active".into(),
            kind: "direct".into(),
            entry_id: "id".into(),
            aliases: vec!["db".into()],
            selected_alias: "db".into(),
            source_path: "/c".into(),
            source_line: 1,
            forwards: ports
                .iter()
                .map(|port| ForwardSpec {
                    kind: "L".into(),
                    requested: format!("{port}"),
                    effective: format!("127.0.0.1:{port}:127.0.0.1:5432"),
                    bind_address: "127.0.0.1".into(),
                    local_port: Some(*port),
                    remote_host: Some("127.0.0.1".into()),
                    remote_port: Some(5432),
                })
                .collect(),
            master_status: master.into(),
            listener_status: listener.into(),
            application_health: "unknown".into(),
            gateway_entry_id: None,
            vm_entry_id: None,
            gateway_alias: None,
            vm_alias: None,
            transit_host: None,
            transit_port: None,
            gateway_master_status: None,
            vm_master_status: None,
            error: None,
        }
    }

    #[test]
    fn busy_and_duplicate_local_ports_are_conflicts() {
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let (first, second) = (free_port(), free_port());
        assert_eq!(conflicts(&[forward(1, first), forward(2, busy_port)]), [1]);
        assert!(conflicts(&[forward(1, first), forward(2, second)]).is_empty());
        assert_eq!(conflicts(&[forward(1, first), forward(2, first)]), [0, 1]);
    }

    #[test]
    fn clear_ports_start_immediately_and_busy_ports_open_the_screen() {
        let free = free_port();
        assert!(matches!(
            begin(&entry(), "db", vec![forward(5432, free)]),
            Outcome::Effect(Effect::TunnelStart(_))
        ));
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        assert!(matches!(
            begin(&entry(), "db", vec![forward(5432, busy_port)]),
            Outcome::Push(_)
        ));
        assert!(matches!(
            begin(&entry(), "db", Vec::new()),
            Outcome::Toast(_)
        ));
    }

    #[test]
    fn evidence_words_only_what_the_view_observed() {
        let lines = evidence(&view("responsive", "bound", &[5432]))
            .into_iter()
            .map(|line| (line.ok, line.text))
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                (true, "db master responsive".to_string()),
                (true, ":5432 listener ready".to_string())
            ]
        );
        let lines = evidence(&view("down", "down", &[5432]))
            .into_iter()
            .map(|line| (line.ok, line.text))
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                (false, "db master down".to_string()),
                (false, ":5432 listener down".to_string())
            ]
        );
    }
}
