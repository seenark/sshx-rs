use super::{
    ConnectRequest, Ctx, Effect, EffectResult, ForwardRequest, Outcome, Runtime, Screen,
    ScreenStatus, cmdline::CommandLine, finder::relative_time, tunnel_progress, tunnels,
    yesno::YesNo,
};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, InputMode, Key};
use sshx::mutation::{self, ServiceDeclaration};
use sshx::session::DeclaredService;
use sshx::usage::{self, UsedForward};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortRow {
    pub remote: u16,
    pub local: u16,
    pub checked: bool,
    /// Last use of the HostEntry, set for rows that only usage state remembers.
    pub used: Option<u64>,
}

/// Declared services first, then forwards remembered from last time that are not declared.
/// A remembered forward is pre-checked and keeps the local port it used; rows are unique by
/// remote port.
pub(super) fn merge_rows(
    declared: &[DeclaredService],
    remembered: &[UsedForward],
    last_used: Option<u64>,
) -> Vec<PortRow> {
    let mut rows: Vec<PortRow> = Vec::new();
    for service in declared {
        if rows.iter().any(|row| row.remote == service.remote_port) {
            continue;
        }
        let used = remembered
            .iter()
            .find(|forward| forward.remote == service.remote_port);
        rows.push(PortRow {
            remote: service.remote_port,
            local: used.map_or(service.default_local_port, |forward| forward.local),
            checked: used.is_some(),
            used: None,
        });
    }
    for forward in remembered {
        if rows.iter().any(|row| row.remote == forward.remote) {
            continue;
        }
        rows.push(PortRow {
            remote: forward.remote,
            local: forward.local,
            checked: true,
            used: last_used,
        });
    }
    rows
}

fn requests(rows: &[PortRow]) -> Vec<ForwardRequest> {
    rows.iter()
        .filter(|row| row.checked)
        .map(|row| ForwardRequest {
            remote: row.remote,
            local: row.local,
        })
        .collect()
}

fn find_entry<'a>(
    entries: &'a [HostEntry],
    id: &str,
    path: &str,
    alias: &str,
) -> Option<&'a HostEntry> {
    let here = |entry: &&HostEntry| {
        entry.source.path == path && entry.aliases.iter().any(|candidate| candidate == alias)
    };
    entries
        .iter()
        .find(|entry| entry.id == id && here(entry))
        .or_else(|| entries.iter().find(here))
}

pub(crate) enum ServiceChange {
    Declare(ServiceDeclaration),
    Undeclare(u16),
}

/// Writes or removes one `##SSHX SERVICE` line, then reopens the ports screen with `rows`.
pub(crate) struct ServiceEdit {
    pub entry: HostEntry,
    pub alias: String,
    pub change: ServiceChange,
    pub rows: Vec<PortRow>,
    pub selected: usize,
}

pub(super) struct Ports {
    id: String,
    path: String,
    alias: String,
    declared: Vec<DeclaredService>,
    rows: Vec<PortRow>,
    selected: usize,
    mode: InputMode,
    draft: String,
    error: Option<String>,
}

impl Ports {
    pub(super) fn open(ctx: &Ctx, entry: &HostEntry, alias: &str) -> Result<Self, String> {
        let declared = sshx::session::declared_services(entry)?;
        let usage = ctx.usage.get(entry);
        let rows = merge_rows(
            &declared,
            usage.map_or(&[][..], |usage| usage.forwards.as_slice()),
            usage.map(|usage| usage.last_used).filter(|time| *time > 0),
        );
        Ok(Self::with_rows(entry, alias, declared, rows, 0))
    }

    fn with_rows(
        entry: &HostEntry,
        alias: &str,
        declared: Vec<DeclaredService>,
        rows: Vec<PortRow>,
        selected: usize,
    ) -> Self {
        Self {
            id: entry.id.clone(),
            path: entry.source.path.clone(),
            alias: alias.to_string(),
            declared,
            selected: selected.min(rows.len().saturating_sub(1)),
            rows,
            mode: InputMode::Normal,
            draft: String::new(),
            error: None,
        }
    }

    fn entry<'a>(&self, ctx: &Ctx<'a>) -> Option<&'a HostEntry> {
        find_entry(ctx.entries, &self.id, &self.path, &self.alias)
    }

    fn declaration(&self, remote: u16) -> Option<&DeclaredService> {
        self.declared
            .iter()
            .find(|service| service.remote_port == remote)
    }

    fn issues(&self) -> Vec<(usize, String)> {
        let checked = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.checked)
            .collect::<Vec<_>>();
        let forwards = checked
            .iter()
            .map(|(_, row)| ForwardRequest {
                remote: row.remote,
                local: row.local,
            })
            .collect::<Vec<_>>();
        tunnel_progress::issues(&forwards)
            .into_iter()
            .map(|(position, message)| (checked[position].0, message))
            .collect()
    }

    fn add_draft(&mut self) {
        let forward = match keymap::parse_forward(self.draft.trim()) {
            Ok(forward) => forward,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let default_local = self
            .declaration(forward.remote)
            .map_or(forward.remote, |service| service.default_local_port);
        let local = forward.local.unwrap_or(default_local);
        match self
            .rows
            .iter()
            .position(|row| row.remote == forward.remote)
        {
            Some(index) => {
                let row = &mut self.rows[index];
                row.checked = true;
                if forward.local.is_some() {
                    row.local = local;
                }
                self.selected = index;
            }
            None => {
                self.rows.push(PortRow {
                    remote: forward.remote,
                    local,
                    checked: true,
                    used: None,
                });
                self.selected = self.rows.len() - 1;
            }
        }
        self.draft.clear();
        self.error = None;
        self.mode = InputMode::Normal;
    }

    fn save(&self, ctx: &Ctx) -> Outcome {
        let (Some(row), Some(entry)) = (self.rows.get(self.selected), self.entry(ctx)) else {
            return Outcome::Stay;
        };
        let existing = self.declaration(row.remote);
        if existing.is_some_and(|service| service.default_local_port == row.local) {
            return Outcome::Toast(format!("{} is already declared", row.remote));
        }
        let declaration = ServiceDeclaration {
            remote_port: row.remote,
            local_port: (row.local != row.remote).then_some(row.local),
            host: existing
                .map(|service| service.destination_host.clone())
                .filter(|host| host != "127.0.0.1"),
            name: existing.and_then(|service| service.name.clone()),
        };
        Outcome::Effect(Effect::SaveService(Box::new(
            self.edit(entry, ServiceChange::Declare(declaration)),
        )))
    }

    fn remove(&mut self, ctx: &Ctx) -> Outcome {
        let Some(row) = self.rows.get(self.selected) else {
            return Outcome::Stay;
        };
        let remote = row.remote;
        if self.declaration(remote).is_none() {
            self.rows.remove(self.selected);
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            return Outcome::Stay;
        }
        let Some(entry) = self.entry(ctx) else {
            return Outcome::Toast(self.missing());
        };
        Outcome::Push(Box::new(YesNo::new(
            format!("Remove declared service {remote}?"),
            vec![format!(
                "The service lines for remote port {remote} are removed from {}.",
                entry.source.path
            )],
            Effect::SaveService(Box::new(self.edit(entry, ServiceChange::Undeclare(remote)))),
        )))
    }

    fn edit(&self, entry: &HostEntry, change: ServiceChange) -> ServiceEdit {
        ServiceEdit {
            entry: entry.clone(),
            alias: self.alias.clone(),
            change,
            rows: self.rows.clone(),
            selected: self.selected,
        }
    }

    fn missing(&self) -> String {
        format!(
            "HOST_NOT_FOUND: {} changed on disk; reopen the ports screen",
            self.alias
        )
    }

    fn connect(&self, ctx: &Ctx) -> Outcome {
        let Some(entry) = self.entry(ctx) else {
            return Outcome::Toast(self.missing());
        };
        if let Some((_, message)) = self.issues().into_iter().next() {
            return Outcome::Toast(message);
        }
        Outcome::Effect(Effect::Connect(Box::new(ConnectRequest {
            entry: entry.clone(),
            alias: self.alias.clone(),
            forwards: requests(&self.rows),
        })))
    }
}

impl Screen for Ports {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::Ports
    }

    fn input_mode(&self) -> InputMode {
        self.mode
    }

    fn availability(&self, ctx: &Ctx) -> Availability {
        let entry = self.entry(ctx);
        Availability {
            has_password: entry
                .is_some_and(|entry| sshx::connect::has_stored_password(entry).unwrap_or(false)),
            is_pair: entry.is_some_and(|entry| {
                matches!(sshx::pair::paired_route(ctx.entries, entry), Ok(Some(_)))
            }),
            tunnel_selected: false,
            row_selected: !self.rows.is_empty(),
        }
    }

    fn on_action(&mut self, action: Action, count: usize, ctx: &Ctx) -> Outcome {
        let last = self.rows.len().saturating_sub(1);
        match action {
            Action::Up => self.selected = self.selected.saturating_sub(count),
            Action::Down => self.selected = (self.selected + count).min(last),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            Action::HalfPageDown => self.selected = (self.selected + count * 5).min(last),
            Action::HalfPageUp => self.selected = self.selected.saturating_sub(count * 5),
            Action::ToggleRow => {
                if let Some(row) = self.rows.get_mut(self.selected) {
                    row.checked = !row.checked;
                }
            }
            Action::AddRow => {
                self.mode = InputMode::Insert;
                self.draft.clear();
                self.error = None;
            }
            Action::Submit => self.add_draft(),
            Action::EnterNormal => {
                self.mode = InputMode::Normal;
                self.draft.clear();
                self.error = None;
            }
            Action::DeleteRow => return self.remove(ctx),
            Action::SaveRow => return self.save(ctx),
            Action::Connect => return self.connect(ctx),
            Action::TunnelStart => {
                let Some(entry) = self.entry(ctx) else {
                    return Outcome::Toast(self.missing());
                };
                return tunnel_progress::begin(entry, &self.alias, requests(&self.rows));
            }
            Action::TunnelList => return Outcome::Effect(Effect::Tunnels),
            Action::TunnelStopAll => return tunnels::stop_all(ctx),
            Action::CommandLine => {
                let mut line = CommandLine::new(self.kind(), self.mode, self.availability(ctx));
                if let Some(entry) = self.entry(ctx) {
                    line = line.with_target(entry, &self.alias);
                }
                return Outcome::Push(Box::new(line));
            }
            Action::Back => return Outcome::Pop,
            Action::Quit => return Outcome::Effect(Effect::Quit { cancelled: false }),
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        if self.mode == InputMode::Insert {
            match key {
                Key::Char(character) if character.is_ascii_digit() || character == '=' => {
                    self.draft.push(character);
                }
                Key::Backspace => {
                    self.draft.pop();
                }
                _ => {}
            }
        }
        Outcome::Stay
    }

    fn status(&self, _ctx: &Ctx) -> ScreenStatus {
        let total = self.rows.len();
        ScreenStatus {
            filter: Some(format!("ports · {}", self.alias)),
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
        let outer = block(format!(" ports · {} ", self.alias), Color::Cyan);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let [rows_area, input_area, hint_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        let dim = Style::default().fg(Color::DarkGray);
        if self.rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No service forwards yet. Press o to add one (REMOTE[=LOCAL]).")
                    .style(dim),
                rows_area,
            );
        } else {
            let issues = self.issues();
            let items = self
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let declared = self.declaration(row.remote);
                    let target = match declared.map(|service| service.destination_host.as_str()) {
                        Some(host) if host != "127.0.0.1" => format!("{host}:{}", row.remote),
                        _ => row.remote.to_string(),
                    };
                    let mut spans = vec![
                        Span::raw(if row.checked { "[x] " } else { "[ ] " }),
                        Span::raw(format!("localhost:{} → {target}", row.local)),
                    ];
                    if let Some(name) = declared.and_then(|service| service.name.as_deref()) {
                        spans.push(Span::styled(
                            format!("  {name}"),
                            Style::default().fg(Color::Cyan),
                        ));
                    }
                    if let Some(time) = row.used {
                        spans.push(Span::styled(
                            format!("  (used {})", relative_time(ctx.now, time)),
                            dim,
                        ));
                    }
                    let mut lines = vec![Line::from(spans)];
                    lines.extend(
                        issues
                            .iter()
                            .filter(|(position, _)| *position == index)
                            .map(|(_, message)| {
                                Line::from(Span::styled(
                                    format!("    ✗ {message}"),
                                    Style::default().fg(Color::Red),
                                ))
                            }),
                    );
                    ListItem::new(lines)
                })
                .collect::<Vec<_>>();
            let mut state =
                ListState::default().with_selected(Some(self.selected.min(self.rows.len() - 1)));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                rows_area,
                &mut state,
            );
        }
        if self.mode == InputMode::Insert {
            let mut spans = vec![
                Span::styled("Add REMOTE[=LOCAL]: ", dim),
                Span::raw(format!("{}▏", self.draft)),
            ];
            if let Some(error) = &self.error {
                spans.push(Span::styled(
                    format!("  {error}"),
                    Style::default().fg(Color::Yellow),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), input_area);
        }
        frame.render_widget(
            Paragraph::new("x toggle · o add · s save · dd remove · ⏎ connect · space t t tunnel")
                .style(dim),
            hint_area,
        );
    }
}

/// Applies the edit through the reviewed mutation engine, re-keys usage state when the edit
/// gave the HostEntry an ID, rediscovers, and swaps in a fresh ports screen.
pub(super) fn save_service(
    runtime: &mut Runtime,
    edit: &ServiceEdit,
) -> Result<EffectResult, String> {
    let toast = |message: String| {
        Ok(EffectResult {
            toast: Some(message),
            ..EffectResult::default()
        })
    };
    let entry = &edit.entry;
    if !crate::entry_sources_unchanged(entry, &runtime.data.sources) {
        return toast(format!(
            "HOST_SOURCE_CHANGED: {} changed; reopen the ports screen",
            entry.source.path
        ));
    }
    let path = PathBuf::from(&entry.source.path);
    let planned = match &edit.change {
        ServiceChange::Declare(declaration) => mutation::plan_declare_service(
            &path,
            &entry.id,
            &edit.alias,
            entry.source.byte_start,
            entry.source.byte_end,
            declaration,
        ),
        ServiceChange::Undeclare(remote) => mutation::plan_undeclare_service(
            &path,
            &entry.id,
            &edit.alias,
            entry.source.byte_start,
            entry.source.byte_end,
            *remote,
        ),
    };
    let plan = match planned {
        Ok(plan) => plan,
        Err(error) => return toast(error),
    };
    let root = usage::entry_root(entry);
    let old_key = root.as_ref().map(|root| usage::host_key(root, entry));
    if let Err(error) = mutation::apply_edit(&plan) {
        return toast(error);
    }
    if let (Some(root), Some(old_key), Some(new_key)) = (&root, &old_key, &plan.added_id) {
        let _ = usage::rekey(root, old_key, new_key);
    }
    let discovery_error = runtime.data.refresh(runtime.cli);
    if runtime
        .stack
        .last()
        .is_some_and(|screen| screen.kind() == keymap::Screen::Ports)
    {
        runtime.stack.pop();
    }
    let mut rows = edit.rows.clone();
    let described = match &edit.change {
        ServiceChange::Declare(declaration) => {
            format!("✓ declared service {}", declaration.remote_port)
        }
        ServiceChange::Undeclare(remote) => {
            rows.retain(|row| row.remote != *remote);
            format!("✓ removed service {remote}")
        }
    };
    let ctx = runtime.data.ctx(None);
    let reopened = find_entry(ctx.entries, &entry.id, &entry.source.path, &edit.alias)
        .or_else(|| {
            let added = plan.added_id.as_deref()?;
            find_entry(ctx.entries, added, &entry.source.path, &edit.alias)
        })
        .and_then(|entry| {
            let declared = sshx::session::declared_services(entry).ok()?;
            Some(Ports::with_rows(
                entry,
                &edit.alias,
                declared,
                rows,
                edit.selected,
            ))
        });
    Ok(EffectResult {
        toast: Some(discovery_error.unwrap_or(described)),
        push: reopened.map(|screen| Box::new(screen) as Box<dyn Screen>),
        ..EffectResult::default()
    })
}

#[cfg(test)]
mod tests {
    use super::{PortRow, merge_rows};
    use sshx::session::DeclaredService;
    use sshx::usage::UsedForward;

    fn service(remote: u16, local: u16, name: Option<&str>) -> DeclaredService {
        DeclaredService {
            id: format!("{remote}#1"),
            remote_port: remote,
            destination_host: "127.0.0.1".into(),
            default_local_port: local,
            name: name.map(str::to_string),
        }
    }

    fn used(remote: u16, local: u16) -> UsedForward {
        UsedForward {
            remote,
            local,
            declared: false,
        }
    }

    fn row(remote: u16, local: u16, checked: bool, used: Option<u64>) -> PortRow {
        PortRow {
            remote,
            local,
            checked,
            used,
        }
    }

    #[test]
    fn nothing_is_checked_without_usage() {
        assert_eq!(
            merge_rows(
                &[service(5432, 5432, Some("db")), service(80, 3000, None)],
                &[],
                None
            ),
            [row(5432, 5432, false, None), row(80, 3000, false, None)]
        );
    }

    #[test]
    fn remembered_forwards_check_declared_rows_and_keep_their_local_port() {
        assert_eq!(
            merge_rows(
                &[service(5432, 5432, None), service(80, 3000, None)],
                &[used(80, 3010)],
                Some(7)
            ),
            [row(5432, 5432, false, None), row(80, 3010, true, None)]
        );
    }

    #[test]
    fn undeclared_remembered_forwards_follow_as_checked_ad_hoc_rows_with_last_use() {
        assert_eq!(
            merge_rows(
                &[service(5432, 5432, None)],
                &[used(6379, 6379), used(5432, 5432), used(6379, 7000)],
                Some(99)
            ),
            [row(5432, 5432, true, None), row(6379, 6379, true, Some(99))]
        );
    }

    #[test]
    fn duplicate_declarations_of_one_remote_port_make_one_row() {
        assert_eq!(
            merge_rows(
                &[service(80, 8080, None), service(80, 9090, None)],
                &[],
                None
            ),
            [row(80, 8080, false, None)]
        );
    }
}
