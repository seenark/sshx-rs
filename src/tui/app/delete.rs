use super::{Ctx, Effect, EffectResult, Outcome, Runtime, Screen};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::mutation;
use std::path::Path;

pub(crate) struct DeleteRequest {
    entry: HostEntry,
    alias: String,
}

pub(super) struct DeleteConfirm {
    request: DeleteRequest,
    block: Vec<String>,
    services: usize,
}

/// Why the HostEntry may not be deleted right now: Pair membership first, then the existing
/// active-use and reference checks shared with `sshx host delete`.
fn refusal(ctx: &Ctx, entry: &HostEntry, alias: &str) -> Option<String> {
    if sshx::pair::deletion_reference(entry, ctx.entries).is_some() {
        return Some(format!(
            "DELETE_REFERENCED: `{alias}` is a gateway or VM of a Pair; dissolve the Pair first (`sshx pair list`, then delete the Pair), then delete the HostEntry"
        ));
    }
    crate::ensure_delete_allowed(ctx.home, entry).err()
}

fn plan(entry: &HostEntry, alias: &str) -> Result<mutation::EditPlan, String> {
    mutation::validate_entry_paths(entry)?;
    mutation::plan_delete(
        Path::new(&entry.source.path),
        &entry.id,
        alias,
        entry.source.byte_start,
        entry.source.byte_end,
    )
}

impl DeleteConfirm {
    /// The block that will disappear (password redacted) comes from the real delete plan.
    pub(super) fn new(ctx: &Ctx, entry: &HostEntry, alias: &str) -> Result<Self, String> {
        if let Some(error) = refusal(ctx, entry, alias) {
            return Err(error);
        }
        let block = plan(entry, alias)?
            .files
            .iter()
            .flat_map(|file| {
                file.patch
                    .lines()
                    .skip(3)
                    .map(|line| line.replace("<redacted>", "••••"))
            })
            .collect();
        Ok(Self {
            request: DeleteRequest {
                entry: entry.clone(),
                alias: alias.to_string(),
            },
            block,
            services: ctx.service_count(entry),
        })
    }
}

impl Screen for DeleteConfirm {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::DeleteConfirm
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Normal
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn dismiss_on_outcome(&self) -> bool {
        true
    }

    fn on_action(&mut self, action: Action, _count: usize, _ctx: &Ctx) -> Outcome {
        match action {
            Action::Confirm => Outcome::Effect(Effect::DeleteHost(Box::new(DeleteRequest {
                entry: self.request.entry.clone(),
                alias: self.request.alias.clone(),
            }))),
            Action::Back => Outcome::Pop,
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, _key: Key, _ctx: &Ctx) -> Outcome {
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let outer = block(format!(" delete {} ", self.request.alias), Color::Red);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let mut lines = vec![
            Line::from(format!(
                "Delete this HostEntry block from {}:{}?",
                self.request.entry.source.path, self.request.entry.source.line_start
            )),
            Line::default(),
        ];
        lines.extend(
            self.block
                .iter()
                .map(|line| Line::styled(line.clone(), Style::default().fg(Color::Red))),
        );
        lines.push(Line::default());
        if self.services > 0 {
            lines.push(Line::from(format!(
                "Its {} declared service(s) are removed with the block.",
                self.services
            )));
        }
        lines.push(Line::styled(
            "y delete   n cancel   u undoes it afterwards",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

pub(super) fn delete(
    runtime: &mut Runtime,
    request: &DeleteRequest,
) -> Result<EffectResult, String> {
    let message = (|| {
        let data = &runtime.data;
        let entry = &request.entry;
        if !crate::entry_sources_unchanged(entry, &data.sources) {
            return Err(format!(
                "HOST_SOURCE_CHANGED: {} changed; select its current HostEntry again",
                entry.source.path
            ));
        }
        if let Some(error) = refusal(&data.ctx(None), entry, &request.alias) {
            return Err(error);
        }
        let plan = plan(entry, &request.alias)?;
        let change = plan.change();
        mutation::apply_edit(&plan)?;
        runtime
            .undo
            .push(format!("delete {}", request.alias), change);
        Ok(format!("✓ deleted {} (u undoes)", request.alias))
    })();
    Ok(match message {
        Ok(toast) => EffectResult {
            toast: Some(toast),
            refresh: true,
            ..EffectResult::default()
        },
        Err(error) => EffectResult {
            toast: Some(error),
            ..EffectResult::default()
        },
    })
}
