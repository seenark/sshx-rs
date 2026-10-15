use super::{
    Ctx, Effect, EffectResult, Outcome, Preview, Runtime, Screen, tunnel_progress::TunnelProgress,
    yesno::YesNo,
};
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
    review: Option<String>,
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
        let mut plan = plan(entry, alias)?;
        let review = match ctx.intent.preview {
            Some(Preview::Edit(operation)) => {
                plan.operation = operation;
                Some(sshx::output::render_edit(
                    &plan,
                    sshx::output::OutputFormat::Human,
                    false,
                )?)
            }
            _ => None,
        };
        let block = plan
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
            review,
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
            Action::Confirm => Outcome::Effect(match &self.review {
                Some(review) => Effect::Print(review.clone()),
                None => Effect::DeleteHost(Box::new(DeleteRequest {
                    entry: self.request.entry.clone(),
                    alias: self.request.alias.clone(),
                })),
            }),
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
            if self.review.is_some() {
                "preview only: y prints the review, nothing is deleted   n cancel"
            } else {
                "y delete   n cancel   u undoes it afterwards"
            },
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// `d` in Select input mode: one combined y/N for every row that may go. Rows refused for the
/// same reasons as a single delete (Pair members, in-use or referenced HostEntries) are listed
/// and skipped.
pub(super) fn begin_many(ctx: &Ctx, rows: &[(&HostEntry, &str)]) -> Outcome {
    let mut requests: Vec<DeleteRequest> = Vec::new();
    let mut refused = Vec::new();
    for (entry, alias) in rows {
        if let Some(error) = refusal(ctx, entry, alias) {
            refused.push(format!("{alias}: {error}"));
            continue;
        }
        let same_block = |other: &DeleteRequest| {
            other.entry.source.path == entry.source.path
                && other.entry.source.byte_start == entry.source.byte_start
        };
        if !requests.iter().any(same_block) {
            requests.push(DeleteRequest {
                entry: (*entry).clone(),
                alias: (*alias).to_string(),
            });
        }
    }
    if requests.is_empty() {
        let lines = refused.into_iter().map(|text| (false, text)).collect();
        return Outcome::Push(Box::new(TunnelProgress::report("delete refused", lines)));
    }
    let aliases = requests
        .iter()
        .map(|request| request.alias.as_str())
        .collect::<Vec<_>>();
    let mut detail = vec![
        format!("Delete {}", aliases.join(", ")),
        "Their declared services are removed with the blocks.".to_string(),
    ];
    if !refused.is_empty() {
        detail.push(String::new());
        detail.push("Refused, not deleted:".to_string());
        detail.extend(refused.into_iter().map(|text| format!("✗ {text}")));
    }
    Outcome::Push(Box::new(YesNo::new(
        format!("Delete {} HostEntries?", requests.len()),
        detail,
        Effect::DeleteHosts(requests),
    )))
}

/// Re-checks the refusals and applies the plan for one HostEntry, recording it for undo.
fn remove(runtime: &mut Runtime, request: &DeleteRequest) -> Result<(), String> {
    let entry = &request.entry;
    if let Some(error) = refusal(&runtime.data.ctx(None), entry, &request.alias) {
        return Err(error);
    }
    let plan = plan(entry, &request.alias)?;
    let change = plan.change();
    mutation::apply_edit(&plan)?;
    runtime
        .undo
        .push(format!("delete {}", request.alias), change, None);
    Ok(())
}

fn ensure_current(runtime: &Runtime, request: &DeleteRequest) -> Result<(), String> {
    if crate::entry_sources_unchanged(&request.entry, &runtime.data.sources) {
        Ok(())
    } else {
        Err(format!(
            "HOST_SOURCE_CHANGED: {} changed; select its current HostEntry again",
            request.entry.source.path
        ))
    }
}

fn done(toast: Result<String, String>) -> Result<EffectResult, String> {
    Ok(match toast {
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

pub(super) fn delete(
    runtime: &mut Runtime,
    request: &DeleteRequest,
) -> Result<EffectResult, String> {
    done(ensure_current(runtime, request).and_then(|()| {
        remove(runtime, request)?;
        Ok(format!("✓ deleted {} (u undoes)", request.alias))
    }))
}

/// Removing a block moves the byte spans of the blocks around it, so each later HostEntry is
/// looked up again after the rediscovery that follows every write.
fn current_entry(runtime: &Runtime, request: &DeleteRequest) -> Option<HostEntry> {
    let wanted = &request.entry;
    let entries = &runtime.data.catalog.entries;
    let same_file = |entry: &&HostEntry| entry.source.path == wanted.source.path;
    entries
        .iter()
        .filter(same_file)
        .find(|entry| entry.id == wanted.id)
        .or_else(|| {
            entries
                .iter()
                .filter(same_file)
                .find(|entry| entry.aliases.contains(&request.alias))
        })
        .cloned()
}

/// Every source must still match discovery before the first write.
pub(super) fn delete_many(
    runtime: &mut Runtime,
    requests: &[DeleteRequest],
) -> Result<EffectResult, String> {
    if let Some(error) = requests
        .iter()
        .find_map(|request| ensure_current(runtime, request).err())
    {
        return done(Err(error));
    }
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for (position, request) in requests.iter().enumerate() {
        let current = if position == 0 {
            Some(request.entry.clone())
        } else {
            current_entry(runtime, request)
        };
        let Some(entry) = current else {
            failed.push(format!(
                "{}: HOST_NOT_FOUND after an earlier delete",
                request.alias
            ));
            continue;
        };
        let request = DeleteRequest {
            entry,
            alias: request.alias.clone(),
        };
        match remove(runtime, &request) {
            Ok(()) => {
                deleted.push(request.alias.clone());
                runtime.data.refresh(runtime.cli);
            }
            Err(error) => failed.push(format!("{}: {error}", request.alias)),
        }
    }
    let mut toast = format!(
        "✓ deleted {} of {} (u undoes one at a time)",
        deleted.len(),
        requests.len()
    );
    if !failed.is_empty() {
        toast = format!("{toast}\n{}", failed.join("\n"));
    }
    done(if deleted.is_empty() {
        Err(failed.join("\n"))
    } else {
        Ok(toast)
    })
}
