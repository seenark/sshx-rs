use super::{ConnectRequest, Ctx, Effect, EffectResult, Outcome, Runtime, Screen, effects};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::mutation::{self, PairPlan};
use sshx::usage;

pub(crate) struct PairRequest {
    gateway: HostEntry,
    gateway_alias: String,
    vm: HostEntry,
    vm_alias: String,
    connect: bool,
}

/// Why the confirmation opened: Enter on a VM that looks unpaired also offers to connect
/// directly, as Enter did before the suggestion existed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Opened {
    Connect,
    Pair,
}

/// Review of one Pair setup: the transit sshx inferred and the metadata lines it adds.
pub(super) struct PairConfirm {
    request: PairRequest,
    opened: Opened,
    transit: String,
    added: Vec<String>,
}

fn plan(
    entries: &[HostEntry],
    gateway: &HostEntry,
    gateway_alias: &str,
    vm: &HostEntry,
    vm_alias: &str,
) -> Result<PairPlan, String> {
    mutation::validate_entry_paths(gateway)?;
    mutation::validate_entry_paths(vm)?;
    sshx::pair::plan_setup(entries, gateway, vm, gateway_alias, vm_alias, None, None)
}

/// The one HostEntry an alias typed into `via` names.
pub(super) fn gateway_by_alias<'a>(ctx: &Ctx<'a>, alias: &str) -> Result<&'a HostEntry, String> {
    let mut found = ctx
        .entries
        .iter()
        .filter(|entry| entry.aliases.iter().any(|known| known == alias));
    match (found.next(), found.next()) {
        (Some(entry), None) => Ok(entry),
        (None, _) => Err(format!("PAIR_GATEWAY: no HostEntry is named `{alias}`")),
        (Some(_), Some(_)) => Err(format!(
            "PAIR_GATEWAY: `{alias}` names several HostEntries; rename one first"
        )),
    }
}

impl PairConfirm {
    pub(super) fn new(
        ctx: &Ctx,
        gateway: &HostEntry,
        vm: &HostEntry,
        vm_alias: &str,
        opened: Opened,
    ) -> Result<Self, String> {
        let gateway_alias = gateway
            .aliases
            .first()
            .ok_or("PAIR_GATEWAY: gateway has no alias")?;
        let plan = plan(ctx.entries, gateway, gateway_alias, vm, vm_alias)?;
        let added = plan
            .files
            .iter()
            .flat_map(|file| {
                std::iter::once(format!("# {}", file.path)).chain(
                    file.patch
                        .lines()
                        .filter(|line| {
                            line.strip_prefix('+')
                                .is_some_and(|text| text.trim_start().starts_with("##SSHX"))
                        })
                        .map(str::to_string),
                )
            })
            .collect();
        Ok(Self {
            request: PairRequest {
                gateway: gateway.clone(),
                gateway_alias: gateway_alias.clone(),
                vm: vm.clone(),
                vm_alias: vm_alias.to_string(),
                connect: false,
            },
            opened,
            transit: format!("{}:{}", plan.transit_host, plan.transit_port),
            added,
        })
    }

    fn effect(&self, connect: bool) -> Outcome {
        let request = &self.request;
        Outcome::Effect(Effect::Pair(Box::new(PairRequest {
            gateway: request.gateway.clone(),
            gateway_alias: request.gateway_alias.clone(),
            vm: request.vm.clone(),
            vm_alias: request.vm_alias.clone(),
            connect,
        })))
    }
}

impl Screen for PairConfirm {
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
            Action::Confirm => self.effect(self.opened == Opened::Connect),
            Action::Back => Outcome::Pop,
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        match key {
            Key::Char('d') if self.opened == Opened::Connect => {
                Outcome::Effect(Effect::Connect(Box::new(ConnectRequest {
                    entry: self.request.vm.clone(),
                    alias: self.request.vm_alias.clone(),
                    forwards: Vec::new(),
                })))
            }
            _ => Outcome::Stay,
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let request = &self.request;
        let mut lines = vec![
            Line::from(match self.opened {
                Opened::Connect => format!(
                    "{} looks like a VM behind {}.",
                    request.vm_alias, request.gateway_alias
                ),
                Opened::Pair => format!(
                    "Connect to {} through {}.",
                    request.vm_alias, request.gateway_alias
                ),
            }),
            Line::from(format!(
                "Pair them so every connection goes {} → {} (transit {}).",
                request.gateway_alias, request.vm_alias, self.transit
            )),
            Line::default(),
        ];
        lines.extend(self.added.iter().map(|line| {
            let color = if line.starts_with('+') {
                Color::Green
            } else {
                Color::DarkGray
            };
            Line::styled(line.clone(), Style::default().fg(color))
        }));
        lines.push(Line::default());
        let key =
            |text: &'static str| Span::styled(text, Style::default().add_modifier(Modifier::BOLD));
        lines.push(Line::from(match self.opened {
            Opened::Connect => vec![
                key("y"),
                Span::raw(" pair and connect · "),
                key("d"),
                Span::raw(" connect directly · "),
                key("n"),
                Span::raw(" cancel"),
            ],
            Opened::Pair => vec![
                key("y"),
                Span::raw(" pair · "),
                key("n"),
                Span::raw(" cancel"),
            ],
        }));

        let width = 76.min(area.width);
        let inner = usize::from(width.saturating_sub(2)).max(1);
        let wrapped: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(inner).max(1))
            .sum();
        let height = u16::try_from(wrapped + 2)
            .unwrap_or(u16::MAX)
            .min(area.height);
        let [row] = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .areas(row);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block(format!(" pair {} ", request.vm_alias), Color::Yellow)),
            popup,
        );
    }
}

fn toast(message: String) -> Result<EffectResult, String> {
    Ok(EffectResult {
        toast: Some(message),
        refresh: true,
        ..EffectResult::default()
    })
}

/// Plans again against the current files, applies the journaled Pair mutation, keeps usage
/// state on HostEntries that just received a `##SSHX ID`, then optionally connects to the VM.
pub(super) fn apply(runtime: &mut Runtime, request: &PairRequest) -> Result<EffectResult, String> {
    let data = &runtime.data;
    for entry in [&request.gateway, &request.vm] {
        if !crate::entry_sources_unchanged(entry, &data.sources) {
            return toast(format!(
                "HOST_SOURCE_CHANGED: {} changed; select its current HostEntry again",
                entry.source.path
            ));
        }
    }
    let plan = match plan(
        &data.catalog.entries,
        &request.gateway,
        &request.gateway_alias,
        &request.vm,
        &request.vm_alias,
    ) {
        Ok(plan) => plan,
        Err(error) => return toast(error),
    };
    if let Err(error) = mutation::apply_pair(&plan) {
        return toast(error);
    }
    for (entry, id) in [
        (&request.gateway, &plan.gateway_id),
        (&request.vm, &plan.vm_id),
    ] {
        if entry.id != *id
            && let Some(root) = usage::entry_root(entry)
        {
            let _ = usage::rekey(&root, &usage::host_key(&root, entry), id);
        }
    }
    let paired = format!("✓ paired {} → {}", request.gateway_alias, request.vm_alias);
    if !request.connect {
        return toast(paired);
    }
    runtime.data.refresh(runtime.cli);
    let Some(vm) = runtime
        .data
        .catalog
        .entries
        .iter()
        .find(|entry| entry.id == plan.vm_id)
        .cloned()
    else {
        return toast(paired);
    };
    effects::connect(
        runtime,
        &ConnectRequest {
            entry: vm,
            alias: request.vm_alias.clone(),
            forwards: Vec::new(),
        },
    )
}
