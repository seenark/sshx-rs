use super::{
    Ctx, Effect, EffectResult, Outcome, Runtime, Screen, ScreenStatus, cmdline::CommandLine,
    finder::relative_time, ports::endpoint, tunnel_progress::TunnelProgress, yesno::YesNo,
};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
};
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::session::ForwardSpec;
use sshx::tunnel::TunnelView;
use std::cell::Cell;

fn running(tunnel: &TunnelView) -> bool {
    tunnel.state == "active"
}

/// y/N for stopping every tunnel that is not already stopped.
pub(super) fn stop_all(ctx: &Ctx) -> Outcome {
    let ids = ctx
        .tunnels
        .iter()
        .filter(|tunnel| tunnel.state != "stopped")
        .map(|tunnel| tunnel.id.clone())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Outcome::Toast("no standalone tunnels to stop".to_string());
    }
    Outcome::Push(Box::new(YesNo::new(
        "Stop all standalone tunnels?",
        vec![format!("{} tunnel(s) stop: {}", ids.len(), ids.join(", "))],
        Effect::TunnelStop(ids),
    )))
}

pub(super) struct Tunnels {
    selected: usize,
    viewport: Cell<usize>,
}

impl Tunnels {
    pub(super) fn new() -> Self {
        Self {
            selected: 0,
            viewport: Cell::new(10),
        }
    }

    pub(super) fn selecting(mut self, id: &str, ctx: &Ctx) -> Self {
        if let Some(index) = ctx.tunnels.iter().position(|tunnel| tunnel.id == id) {
            self.selected = index;
        }
        self
    }

    fn current<'a>(&self, ctx: &Ctx<'a>) -> Option<&'a TunnelView> {
        ctx.tunnels
            .get(self.selected.min(ctx.tunnels.len().saturating_sub(1)))
    }
}

fn forward_line(forward: &ForwardSpec) -> String {
    match (forward.flag(), forward.local_port, forward.remote_port) {
        ('L', Some(local), Some(remote)) => {
            let listener = endpoint(&forward.bind_address, local);
            match forward.remote_host.as_deref() {
                Some("127.0.0.1" | "localhost" | "::1") | None => format!("{listener} → {remote}"),
                Some(host) => format!("{listener} → {host}:{remote}"),
            }
        }
        ('D', Some(local), _) => format!("socks :{local}"),
        ('R', _, _) => format!("remote {}", forward.effective),
        _ => forward.effective.clone(),
    }
}

fn dot(status: &str, good: &str) -> Span<'static> {
    let color = match status {
        status if status == good => Color::Green,
        "down" => Color::Red,
        _ => Color::DarkGray,
    };
    Span::styled("●", Style::default().fg(color))
}

impl Screen for Tunnels {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::Tunnels
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Normal
    }

    fn availability(&self, ctx: &Ctx) -> Availability {
        let current = self.current(ctx);
        Availability {
            tunnel_selected: current.is_some_and(running),
            row_selected: current.is_some(),
            ..Availability::default()
        }
    }

    fn on_action(&mut self, action: Action, count: usize, ctx: &Ctx) -> Outcome {
        let last = ctx.tunnels.len().saturating_sub(1);
        let step = (self.viewport.get() / 2).max(1);
        match action {
            Action::Up => self.selected = self.selected.min(last).saturating_sub(count),
            Action::Down => self.selected = (self.selected + count).min(last),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            Action::HalfPageDown => self.selected = (self.selected + count * step).min(last),
            Action::HalfPageUp => {
                self.selected = self.selected.min(last).saturating_sub(count * step)
            }
            Action::Back => return Outcome::Pop,
            Action::Quit => return Outcome::Effect(Effect::Quit { cancelled: false }),
            Action::CommandLine => {
                return Outcome::Push(Box::new(CommandLine::new(
                    self.kind(),
                    self.input_mode(),
                    self.availability(ctx),
                )));
            }
            Action::TunnelList => return Outcome::Effect(Effect::Tunnels),
            Action::TunnelStopAll => return stop_all(ctx),
            Action::TunnelStop => {
                let Some(tunnel) = self.current(ctx) else {
                    return Outcome::Stay;
                };
                if tunnel.state == "stopped" {
                    return Outcome::Toast(format!("tunnel {} is already stopped", tunnel.id));
                }
                return Outcome::Push(Box::new(YesNo::new(
                    format!("Stop tunnel {}?", tunnel.id),
                    vec![format!(
                        "{} · {}",
                        tunnel.selected_alias,
                        tunnel
                            .forwards
                            .iter()
                            .map(forward_line)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )],
                    Effect::TunnelStop(vec![tunnel.id.clone()]),
                )));
            }
            Action::TunnelRestart => {
                let Some(tunnel) = self.current(ctx) else {
                    return Outcome::Stay;
                };
                return Outcome::Effect(Effect::TunnelRestart(tunnel.id.clone()));
            }
            Action::Connect => {
                let Some(tunnel) = self.current(ctx) else {
                    return Outcome::Stay;
                };
                return Outcome::Effect(Effect::TunnelOpen(tunnel.id.clone()));
            }
            Action::Copy(keymap::CopyTarget::TunnelId) => {
                let Some(tunnel) = self.current(ctx) else {
                    return Outcome::Stay;
                };
                return Outcome::Effect(Effect::CopyTunnelId(tunnel.id.clone()));
            }
            Action::Goto(keymap::GotoTarget::Tunnels) => {}
            Action::Goto(_) => return Outcome::Pop,
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, _key: Key, _ctx: &Ctx) -> Outcome {
        Outcome::Stay
    }

    fn status(&self, ctx: &Ctx) -> ScreenStatus {
        let total = ctx.tunnels.len();
        ScreenStatus {
            filter: Some("tunnels".to_string()),
            position: Some((
                if total == 0 {
                    0
                } else {
                    self.selected.min(total - 1) + 1
                },
                total,
            )),
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let outer = block(" standalone tunnels ", Color::Cyan);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let [rows_area, legend_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
        self.viewport.set(usize::from(rows_area.height / 2));
        let dim = Style::default().fg(Color::DarkGray);
        if ctx.tunnels.is_empty() {
            frame.render_widget(
                Paragraph::new(
                    "No standalone tunnels. Start one from the ports screen: ⇥, x, space t t.",
                )
                .style(dim),
                rows_area,
            );
        } else {
            let items = ctx
                .tunnels
                .iter()
                .map(|tunnel| {
                    let age = if running(tunnel) {
                        sshx::tunnel::started_at(ctx.home, &tunnel.id)
                            .map_or_else(|| "-".to_string(), |at| relative_time(ctx.now, at))
                    } else {
                        tunnel.state.clone()
                    };
                    let alias = match (&tunnel.gateway_alias, &tunnel.vm_alias) {
                        (Some(gateway), Some(vm)) => format!("{gateway} → {vm}"),
                        _ => tunnel.selected_alias.clone(),
                    };
                    let evidence = match (&tunnel.gateway_master_status, &tunnel.vm_master_status) {
                        (Some(gateway), Some(vm)) => vec![
                            dot(gateway, "responsive"),
                            dot(vm, "responsive"),
                            dot(&tunnel.listener_status, "bound"),
                        ],
                        _ => vec![
                            dot(&tunnel.master_status, "responsive"),
                            dot(&tunnel.listener_status, "bound"),
                        ],
                    };
                    let mut head = vec![
                        Span::styled(
                            format!("{:<9}", tunnel.id),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("{alias:<20} {age:<10} ")),
                    ];
                    head.extend(evidence);
                    let forwards = tunnel
                        .forwards
                        .iter()
                        .map(forward_line)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut lines = vec![
                        Line::from(head),
                        Line::from(Span::styled(format!("          {forwards}"), dim)),
                    ];
                    if let Some(error) = &tunnel.error {
                        lines.push(Line::from(Span::styled(
                            format!("          {error}"),
                            Style::default().fg(Color::Red),
                        )));
                    }
                    ListItem::new(lines)
                })
                .collect::<Vec<_>>();
            let mut state =
                ListState::default().with_selected(Some(self.selected.min(ctx.tunnels.len() - 1)));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                rows_area,
                &mut state,
            );
        }
        frame.render_widget(
            Paragraph::new(
                "●● master · listener   green responsive/ready · red down · grey unknown",
            )
            .style(dim),
            legend_area,
        );
    }
}

/// Stops each tunnel through the registry (locked, control-socket checked); the toast
/// reports every failure, and the list refreshes afterwards.
pub(super) fn stop(runtime: &Runtime, ids: &[String]) -> Result<EffectResult, String> {
    let (stopped, failures): (Vec<_>, Vec<_>) = ids
        .iter()
        .map(|id| sshx::tunnel::stop(&runtime.data.home, id, None).map(|_| id))
        .partition(Result::is_ok);
    let failures = failures
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    let message = if failures.is_empty() {
        match ids {
            [id] => format!("✓ stopped {id}"),
            ids => format!("✓ stopped {} tunnels", ids.len()),
        }
    } else {
        let stopped = stopped.len();
        format!(
            "{} (stopped {stopped} of {})",
            failures.join("; "),
            ids.len()
        )
    };
    Ok(EffectResult {
        toast: Some(message),
        ..EffectResult::default()
    })
}

/// Restarts from the persisted request in cooked mode and shows the evidence screen.
pub(super) fn restart(runtime: &Runtime, id: &str) -> Result<EffectResult, String> {
    let data = &runtime.data;
    let result = sshx::tunnel::restart(
        &data.catalog.entries,
        &data.home,
        id,
        None,
        false,
        None,
        sshx::connect::PairedCredentials {
            gateway_password_fd: None,
            vm_password_fd: None,
        },
    )
    .and_then(|response| {
        response
            .tunnels
            .into_iter()
            .next()
            .ok_or_else(|| "REGISTRY_INVALID: tunnel restart returned no tunnel".to_string())
    });
    let alias = data
        .tunnels
        .iter()
        .find(|tunnel| tunnel.id == id)
        .map_or(id, |tunnel| tunnel.selected_alias.as_str());
    match result {
        Err(error)
            if error.starts_with("SESSION_INTERRUPTED")
                || error.starts_with("SESSION_START_INTERRUPTED") =>
        {
            Err(error)
        }
        result => Ok(EffectResult {
            push: Some(Box::new(TunnelProgress::finished(alias, result))),
            refresh: true,
            ..EffectResult::default()
        }),
    }
}

/// Enter on a tunnel: ssh through its master after the registry proves it is live. A finished
/// session ends sshx like Session mode; a refusal is a toast and the list stays.
pub(super) fn open(runtime: &Runtime, id: &str) -> Result<EffectResult, String> {
    Ok(match sshx::tunnel::open_shell(&runtime.data.home, id) {
        Ok(code) => EffectResult {
            exit: Some(code),
            ..EffectResult::default()
        },
        Err(error) => EffectResult {
            toast: Some(error),
            ..EffectResult::default()
        },
    })
}
