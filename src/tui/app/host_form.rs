use super::{Ctx, Effect, EffectResult, Outcome, Runtime, Screen, help::Help, undo::Rekey};
use crate::tui::matcher::matching_rows;
use crate::tui::widgets::{block, password_backspace, password_insert};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, Command, InputMode, Key};
use sshx::mutation::{self, CreateRequest, UpdateRequest};
use sshx::usage;
use std::path::{Path, PathBuf};

const MASKED: &str = "••••";
/// What `mutation` users see for a stored password; typing replaces it, backspace clears it.
const STORED_PASSWORD: &str = "********";
const SUGGESTION_ROWS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Alias,
    Host,
    User,
    Port,
    Via,
    Key,
    Pass,
    File,
}

impl Field {
    const ALL: [Field; 8] = [
        Field::Alias,
        Field::Host,
        Field::User,
        Field::Port,
        Field::Via,
        Field::Key,
        Field::Pass,
        Field::File,
    ];

    fn label(self) -> &'static str {
        match self {
            Field::Alias => "alias",
            Field::Host => "host",
            Field::User => "user",
            Field::Port => "port",
            Field::Via => "via",
            Field::Key => "key",
            Field::Pass => "pass",
            Field::File => "file",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Field::Alias | Field::Host => "required",
            Field::User => "ssh default",
            Field::Port => "22",
            Field::Via | Field::Key | Field::Pass => "none",
            Field::File => "",
        }
    }
}

type Values = [String; 8];

/// A config file a new HostEntry can be written into, with the registered root that reaches it.
struct Candidate {
    path: PathBuf,
    root: PathBuf,
}

struct Edit {
    entry: Box<HostEntry>,
    alias: String,
    original: Values,
    had_password: bool,
}

/// What `:w` hands to the executor; the plan is made again there against the current bytes.
pub(crate) enum HostWrite {
    Create(Box<CreateRequest>),
    Update {
        request: Box<UpdateRequest>,
        entry: Box<HostEntry>,
    },
}

pub(crate) struct SaveRequest {
    write: HostWrite,
    alias: String,
    close: bool,
}

pub(crate) struct EditorRequest {
    path: String,
    line: usize,
}

impl EditorRequest {
    pub(crate) fn new(entry: &HostEntry) -> Self {
        Self {
            path: entry.source.path.clone(),
            line: entry.source.line_start,
        }
    }
}

impl HostWrite {
    /// The exact lines the engine will write or change, password masked.
    fn preview(&self) -> Result<Vec<String>, String> {
        let files = match self {
            HostWrite::Create(request) => mutation::plan_create(request)?.files,
            HostWrite::Update { request, .. } => mutation::plan_update(request)?.files,
        };
        Ok(files
            .iter()
            .flat_map(|file| {
                std::iter::once(format!("# {}", file.path)).chain(file.patch.lines().skip(3).map(
                    |line| {
                        if line.starts_with("+##SSHX ID=") {
                            "+##SSHX ID=<assigned on save>".to_string()
                        } else {
                            line.replace("<redacted>", MASKED)
                        }
                    },
                ))
            })
            .collect())
    }
}

pub(super) fn open<S: Screen + 'static>(screen: Result<S, String>) -> Outcome {
    match screen {
        Ok(screen) => Outcome::Push(Box::new(screen)),
        Err(error) => Outcome::Toast(error),
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Registered roots first, then every included file discovery found hosts in.
fn candidates(ctx: &Ctx) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    let mut add = |path: PathBuf, root: PathBuf| {
        if !found
            .iter()
            .any(|candidate| canonical(&candidate.path) == canonical(&path))
        {
            found.push(Candidate { path, root });
        }
    };
    for root in ctx.roots {
        add(root.path.clone(), root.path.clone());
    }
    for entry in ctx.entries {
        let Some(root) = usage::entry_root(entry) else {
            continue;
        };
        let root = ctx
            .roots
            .iter()
            .find(|registered| canonical(&registered.path) == canonical(&root))
            .map_or(root, |registered| registered.path.clone());
        add(PathBuf::from(&entry.source.path), root);
    }
    found
}

fn pair_line(ctx: &Ctx, entry: &HostEntry) -> Option<String> {
    sshx::pair::deletion_reference(entry, ctx.entries)?;
    let records = sshx::pair::records(ctx.entries);
    let pair = records
        .iter()
        .find(|pair| pair.gateway_id == entry.id || pair.vm_id == entry.id);
    Some(pair.map_or_else(
        || "pair: member of an incomplete Pair".to_string(),
        |pair| format!("pair: {} → {}", pair.gateway_alias, pair.vm_alias),
    ))
}

pub(super) struct HostForm {
    mode: InputMode,
    command: Option<String>,
    edit: Option<Edit>,
    fields: Vec<Field>,
    values: Values,
    password_edited: bool,
    files: Vec<Candidate>,
    pair: Option<String>,
    focus: usize,
    pick: usize,
    preview: Result<Vec<String>, String>,
}

impl HostForm {
    pub(super) fn create(ctx: &Ctx) -> Result<Self, String> {
        let files = candidates(ctx);
        if files.is_empty() {
            return Err(
                "CONFIG_ROOT_REQUIRED: no config root is registered; add one with :root add <path>"
                    .to_string(),
            );
        }
        let mut form = Self {
            mode: InputMode::Normal,
            command: None,
            edit: None,
            fields: Field::ALL.to_vec(),
            values: Values::default(),
            password_edited: false,
            files,
            pair: None,
            focus: 0,
            pick: 0,
            preview: Ok(Vec::new()),
        };
        form.refresh();
        Ok(form)
    }

    pub(crate) fn edit(ctx: &Ctx, entry: &HostEntry, alias: &str) -> Result<Self, String> {
        mutation::validate_entry_paths(entry)?;
        let current = mutation::current_values(&UpdateRequest {
            path: PathBuf::from(&entry.source.path),
            expected_id: entry.id.clone(),
            selected_alias: alias.to_string(),
            byte_start: entry.source.byte_start,
            byte_end: entry.source.byte_end,
            ..UpdateRequest::default()
        })?;
        let pair = pair_line(ctx, entry);
        let mut values = Values::default();
        values[Field::Alias as usize] = alias.to_string();
        values[Field::Host as usize] = current.hostname.unwrap_or_default();
        values[Field::User as usize] = current.user.unwrap_or_default();
        values[Field::Port as usize] = current
            .port
            .map(|port| port.to_string())
            .unwrap_or_default();
        values[Field::Via as usize] = current.proxy_jump.unwrap_or_default();
        values[Field::Key as usize] = current.identity_file.unwrap_or_default();
        values[Field::File as usize] = entry.source.path.clone();
        if current.has_password {
            values[Field::Pass as usize] = STORED_PASSWORD.to_string();
        }
        let mut form = Self {
            mode: InputMode::Normal,
            command: None,
            edit: Some(Edit {
                entry: Box::new(entry.clone()),
                alias: alias.to_string(),
                original: values.clone(),
                had_password: current.has_password,
            }),
            fields: Field::ALL
                .into_iter()
                .filter(|field| !(pair.is_some() && *field == Field::Via))
                .collect(),
            values,
            password_edited: false,
            files: Vec::new(),
            pair,
            focus: 0,
            pick: 0,
            preview: Ok(Vec::new()),
        };
        form.refresh();
        Ok(form)
    }

    fn field(&self) -> Field {
        self.fields[self.focus]
    }

    fn text(&self, field: Field) -> &str {
        self.values[field as usize].trim()
    }

    fn read_only(&self, field: Field) -> bool {
        self.edit.is_some() && field == Field::File
    }

    fn resolve_file(&self) -> Result<&Candidate, String> {
        let wanted = self.text(Field::File);
        let found = if wanted.is_empty() {
            self.files.first()
        } else {
            self.files.iter().find(|candidate| {
                candidate.path == Path::new(wanted)
                    || canonical(&candidate.path) == canonical(Path::new(wanted))
            })
        };
        found.ok_or_else(|| {
            "FILE_UNKNOWN: pick a registered config root or an included file".to_string()
        })
    }

    /// Optional field: new text against the baseline, as (value to set, clear the directive).
    fn change(&self, field: Field, original: &str) -> (Option<String>, bool) {
        let now = self.text(field);
        if now == original.trim() {
            (None, false)
        } else if now.is_empty() {
            (None, true)
        } else {
            (Some(now.to_string()), false)
        }
    }

    fn port(&self) -> Result<Option<u16>, String> {
        match self.text(Field::Port) {
            "" => Ok(None),
            port => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .map(Some)
                .ok_or_else(|| "PORT_INVALID: enter a port from 1 to 65535".to_string()),
        }
    }

    /// The write the form describes, `None` when an edit changes nothing.
    fn build(&self) -> Result<Option<HostWrite>, String> {
        let port = self.port()?;
        if self.text(Field::Alias).is_empty() {
            return Err("alias is required".to_string());
        }
        let optional = |field| Some(self.text(field)).filter(|text| !text.is_empty());
        let new_password = (self.password_edited && !self.values[Field::Pass as usize].is_empty())
            .then(|| self.values[Field::Pass as usize].clone());
        let Some(edit) = &self.edit else {
            if self.text(Field::Host).is_empty() {
                return Err("host is required".to_string());
            }
            let candidate = self.resolve_file()?;
            let mut request = CreateRequest::new(
                candidate.root.clone(),
                candidate.path.clone(),
                self.text(Field::Alias).to_string(),
                self.text(Field::Host).to_string(),
                optional(Field::User).map(str::to_string),
                port,
                new_password,
            );
            request.proxy_jump = optional(Field::Via).map(str::to_string);
            request.identity_file = optional(Field::Key).map(str::to_string);
            return Ok(Some(HostWrite::Create(Box::new(request))));
        };
        let original = |field: Field| edit.original[field as usize].as_str();
        let host_changed = self.text(Field::Host) != original(Field::Host).trim();
        if host_changed && self.text(Field::Host).is_empty() {
            return Err("host cannot be emptied".to_string());
        }
        let source = &edit.entry.source;
        let mut request = UpdateRequest {
            path: PathBuf::from(&source.path),
            expected_id: edit.entry.id.clone(),
            selected_alias: edit.alias.clone(),
            byte_start: source.byte_start,
            byte_end: source.byte_end,
            alias: (self.text(Field::Alias) != edit.alias)
                .then(|| self.text(Field::Alias).to_string()),
            hostname: host_changed.then(|| self.text(Field::Host).to_string()),
            ..UpdateRequest::default()
        };
        (request.user, request.clear_user) = self.change(Field::User, original(Field::User));
        (request.proxy_jump, request.clear_proxy_jump) =
            self.change(Field::Via, original(Field::Via));
        (request.identity_file, request.clear_identity_file) =
            self.change(Field::Key, original(Field::Key));
        if self.text(Field::Port) != original(Field::Port).trim() {
            request.port = port;
            request.clear_port = port.is_none();
        }
        if self.password_edited {
            request.password = new_password;
            request.clear_password = request.password.is_none() && edit.had_password;
        }
        let unchanged = request.alias.is_none()
            && request.hostname.is_none()
            && request.user.is_none()
            && request.port.is_none()
            && request.password.is_none()
            && request.proxy_jump.is_none()
            && request.identity_file.is_none()
            && !(request.clear_user
                || request.clear_port
                || request.clear_password
                || request.clear_proxy_jump
                || request.clear_identity_file);
        Ok((!unchanged).then(|| HostWrite::Update {
            request: Box::new(request),
            entry: edit.entry.clone(),
        }))
    }

    fn refresh(&mut self) {
        self.preview = match self.build() {
            Ok(Some(write)) => write.preview(),
            Ok(None) => Ok(vec!["no changes".to_string()]),
            Err(error) => Err(error),
        };
    }

    fn save(&self, close: bool) -> Outcome {
        match self.build() {
            Err(error) => Outcome::Toast(error),
            Ok(None) => Outcome::Toast("no changes to write".to_string()),
            Ok(Some(write)) => Outcome::Effect(Effect::SaveHost(Box::new(SaveRequest {
                write,
                alias: self.text(Field::Alias).to_string(),
                close,
            }))),
        }
    }

    /// Existing aliases for `via`, existing files for a new HostEntry's `file`.
    fn suggestions(&self, ctx: &Ctx) -> Vec<String> {
        let query = self.text(self.field());
        let mut found = Vec::new();
        match self.field() {
            Field::Via => {
                let entries = ctx
                    .entries
                    .iter()
                    .filter(|entry| {
                        self.edit
                            .as_ref()
                            .is_none_or(|edit| entry.id != edit.entry.id)
                    })
                    .collect::<Vec<_>>();
                for row in matching_rows(&entries, query) {
                    if !found.iter().any(|alias| alias == row.alias) {
                        found.push(row.alias.to_string());
                    }
                }
            }
            Field::File if self.edit.is_none() => {
                let query = query.to_lowercase();
                found.extend(
                    self.files
                        .iter()
                        .map(|candidate| candidate.path.to_string_lossy().into_owned())
                        .filter(|path| path.to_lowercase().contains(&query)),
                );
            }
            _ => {}
        }
        found.truncate(SUGGESTION_ROWS);
        found
    }

    fn complete(&mut self, ctx: &Ctx) {
        let suggestions = self.suggestions(ctx);
        if self.text(self.field()).is_empty() || suggestions.is_empty() {
            return;
        }
        let choice = suggestions[self.pick.min(suggestions.len() - 1)].clone();
        self.values[self.field() as usize] = choice;
    }

    fn type_char(&mut self, character: char) {
        let field = self.field();
        if field == Field::Pass {
            password_insert(
                &mut self.values[field as usize],
                &mut self.password_edited,
                character,
            );
        } else if !self.read_only(field) {
            self.values[field as usize].push(character);
        }
    }

    fn backspace(&mut self) {
        let field = self.field();
        if field == Field::Pass {
            password_backspace(&mut self.values[field as usize], &mut self.password_edited);
        } else if !self.read_only(field) {
            self.values[field as usize].pop();
        }
    }

    fn run_command(&mut self, ctx: &Ctx) -> Outcome {
        let text = self.command.take().unwrap_or_default();
        self.mode = InputMode::Normal;
        match keymap::parse_command(&text) {
            Ok(Command::Write) => self.save(false),
            Ok(Command::WriteQuit) => self.save(true),
            Ok(Command::Quit) => Outcome::Pop,
            Ok(Command::Help) => Outcome::Push(Box::new(Help::new(
                keymap::Screen::HostForm,
                self.mode,
                self.availability(ctx),
            ))),
            Ok(_) => Outcome::Toast("that command does not apply to the host form".to_string()),
            Err(error) => Outcome::Toast(error),
        }
    }

    fn field_line(&self, index: usize, field: Field) -> Line<'static> {
        let focused = index == self.focus;
        let editing = focused && self.mode == InputMode::Insert && self.command.is_none();
        let raw = &self.values[field as usize];
        let (text, real) = match field {
            Field::Pass if !raw.is_empty() => (MASKED.to_string(), true),
            Field::File if raw.is_empty() => (
                self.files
                    .first()
                    .map(|candidate| candidate.path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                false,
            ),
            _ => (raw.clone(), !raw.is_empty()),
        };
        let text = if text.is_empty() && !editing {
            field.hint().to_string()
        } else {
            text
        };
        let value_style = if real || editing {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let marker = if focused { "▸ " } else { "  " };
        let cursor = if editing { "▏" } else { "" };
        Line::from(vec![
            Span::styled(format!("{marker}{:<6}", field.label()), label_style),
            Span::styled(format!("{text}{cursor}"), value_style),
        ])
    }
}

impl Screen for HostForm {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::HostForm
    }

    fn input_mode(&self) -> InputMode {
        self.mode
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn on_action(&mut self, action: Action, _count: usize, ctx: &Ctx) -> Outcome {
        match action {
            Action::NextField => {
                self.focus = (self.focus + 1).min(self.fields.len() - 1);
                self.pick = 0;
            }
            Action::PrevField => {
                self.focus = self.focus.saturating_sub(1);
                self.pick = 0;
            }
            Action::EnterInsert => {
                if self.read_only(self.field()) {
                    return Outcome::Toast(
                        "file cannot change while editing; the HostEntry stays in its file"
                            .to_string(),
                    );
                }
                self.mode = InputMode::Insert;
            }
            Action::EnterNormal => {
                self.command = None;
                self.mode = InputMode::Normal;
            }
            Action::Submit if self.command.is_some() => return self.run_command(ctx),
            Action::Submit => {
                self.complete(ctx);
                self.mode = InputMode::Normal;
                self.refresh();
            }
            Action::CommandLine => {
                self.command = Some(String::new());
                self.mode = InputMode::Insert;
            }
            Action::Write => return self.save(false),
            Action::WriteQuit => return self.save(true),
            Action::Quit | Action::Back => return Outcome::Pop,
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        if self.mode != InputMode::Insert {
            return Outcome::Stay;
        }
        if let Some(command) = &mut self.command {
            match key {
                Key::Char(character) => command.push(character),
                Key::Backspace if command.pop().is_none() => {
                    self.command = None;
                    self.mode = InputMode::Normal;
                }
                _ => {}
            }
            return Outcome::Stay;
        }
        match key {
            Key::Char(character) => {
                self.type_char(character);
                self.pick = 0;
            }
            Key::Backspace => {
                self.backspace();
                self.pick = 0;
            }
            Key::Up => self.pick = self.pick.saturating_sub(1),
            Key::Down => self.pick += 1,
            _ => {}
        }
        self.refresh();
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let title = match &self.edit {
            Some(edit) => format!(" edit host · {} ", edit.alias),
            None => " new host ".to_string(),
        };
        let outer = block(title, Color::Cyan);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let suggestions = if self.mode == InputMode::Insert && self.command.is_none() {
            self.suggestions(ctx)
        } else {
            Vec::new()
        };
        let field_rows = self.fields.len() + usize::from(self.pair.is_some());
        let height = |rows: usize| u16::try_from(rows).unwrap_or(u16::MAX);
        let [form_area, suggestion_area, preview_area, prompt_area] = Layout::vertical([
            Constraint::Length(height(field_rows)),
            Constraint::Length(height(suggestions.len())),
            Constraint::Min(3),
            Constraint::Length(u16::from(self.command.is_some())),
        ])
        .areas(inner);

        let mut lines = self
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| self.field_line(index, *field))
            .collect::<Vec<_>>();
        if let Some(pair) = &self.pair {
            lines.push(Line::styled(
                format!("  {pair} (read-only; manage in the Pair flow)"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        frame.render_widget(Paragraph::new(lines), form_area);

        let picked = self.pick.min(suggestions.len().saturating_sub(1));
        frame.render_widget(
            Paragraph::new(
                suggestions
                    .iter()
                    .enumerate()
                    .map(|(index, suggestion)| {
                        let style = if index == picked {
                            Style::default().add_modifier(Modifier::REVERSED)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        Line::styled(format!("      ↳ {suggestion}"), style)
                    })
                    .collect::<Vec<_>>(),
            ),
            suggestion_area,
        );

        let preview_block = Block::default()
            .title(" preview ")
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray));
        let preview_inner = preview_block.inner(preview_area);
        frame.render_widget(preview_block, preview_area);
        let preview_lines = match &self.preview {
            Ok(lines) => lines
                .iter()
                .map(|line| {
                    let color = match line.chars().next() {
                        Some('+') => Color::Green,
                        Some('-') => Color::Red,
                        _ => Color::DarkGray,
                    };
                    Line::styled(line.clone(), Style::default().fg(color))
                })
                .collect::<Vec<_>>(),
            Err(error) => vec![Line::styled(
                error.clone(),
                Style::default().fg(Color::Yellow),
            )],
        };
        frame.render_widget(Paragraph::new(preview_lines), preview_inner);

        if let Some(command) = &self.command {
            frame.render_widget(Clear, prompt_area);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(":", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{command}▏")),
                ])),
                prompt_area,
            );
        }
    }
}

pub(super) fn save(runtime: &mut Runtime, request: &SaveRequest) -> Result<EffectResult, String> {
    let id = match apply(runtime, &request.write, &request.alias) {
        Ok(id) => id,
        Err(error) => {
            return Ok(EffectResult {
                toast: Some(error),
                ..EffectResult::default()
            });
        }
    };
    let mut result = EffectResult {
        toast: Some(format!("✓ saved {}", request.alias)),
        refresh: true,
        ..EffectResult::default()
    };
    if runtime.stack.len() > 1 {
        runtime.stack.pop();
    }
    if !request.close {
        runtime.data.refresh(runtime.cli);
        let ctx = runtime.data.ctx(None);
        result.push = ctx
            .entries
            .iter()
            .find(|entry| entry.id == id && entry.aliases.contains(&request.alias))
            .and_then(|entry| HostForm::edit(&ctx, entry, &request.alias).ok())
            .map(|form| Box::new(form) as Box<dyn Screen>);
    }
    Ok(result)
}

/// Applies the write, records it for undo and keeps usage state attached to a HostEntry that
/// just received its `##SSHX ID`. Returns the ID of the written HostEntry.
fn apply(runtime: &mut Runtime, write: &HostWrite, alias: &str) -> Result<String, String> {
    let mut rekey = None;
    let (change, id, label) = match write {
        HostWrite::Create(request) => {
            let plan = mutation::plan_create(request)?;
            let change = plan.change();
            mutation::apply(&plan)?;
            (change, plan.id.clone(), format!("create {alias}"))
        }
        HostWrite::Update { request, entry } => {
            if !crate::entry_sources_unchanged(entry, &runtime.data.sources) {
                return Err(format!(
                    "HOST_SOURCE_CHANGED: {} changed; reopen the form on its current HostEntry",
                    entry.source.path
                ));
            }
            mutation::validate_entry_paths(entry)?;
            let plan = mutation::plan_update(request)?;
            let change = plan.change();
            mutation::apply_edit(&plan)?;
            if let (Some(added), Some(root)) = (&plan.added_id, usage::entry_root(entry)) {
                let old = usage::host_key(&root, entry);
                let _ = usage::rekey(&root, &old, added);
                rekey = Some(Rekey {
                    root,
                    old,
                    new: added.clone(),
                });
            }
            let id = plan.added_id.clone().unwrap_or_else(|| entry.id.clone());
            (change, id, format!("edit {alias}"))
        }
    };
    runtime.undo.push(label, change, rekey);
    Ok(id)
}

/// `$EDITOR +<line> <file>` (or `vi`) in the cooked terminal, then rediscovery.
pub(super) fn edit_in_editor(request: &EditorRequest) -> Result<EffectResult, String> {
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|editor| !editor.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg("sh")
        .arg(format!("+{}", request.line))
        .arg(&request.path)
        .status();
    let toast = match status {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!("EDITOR_FAILED: {editor} exited with {status}")),
        Err(error) => Some(format!("EDITOR_FAILED: cannot start {editor}: {error}")),
    };
    Ok(EffectResult {
        toast,
        refresh: true,
        ..EffectResult::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> HostForm {
        let mut form = HostForm {
            mode: InputMode::Normal,
            command: None,
            edit: None,
            fields: Field::ALL.to_vec(),
            values: Values::default(),
            password_edited: false,
            files: vec![Candidate {
                path: PathBuf::from("/tmp/sshx-form-unit/config"),
                root: PathBuf::from("/tmp/sshx-form-unit/config"),
            }],
            pair: None,
            focus: 0,
            pick: 0,
            preview: Ok(Vec::new()),
        };
        form.refresh();
        form
    }

    fn set(form: &mut HostForm, field: Field, value: &str) {
        form.values[field as usize] = value.to_string();
    }

    #[test]
    fn only_alias_and_host_are_required() {
        let mut form = form();
        assert_eq!(form.build().err().as_deref(), Some("alias is required"));
        set(&mut form, Field::Alias, "web");
        assert_eq!(form.build().err().as_deref(), Some("host is required"));
    }

    #[test]
    fn port_must_be_a_real_port() {
        let mut form = form();
        set(&mut form, Field::Alias, "web");
        set(&mut form, Field::Host, "web.example");
        for bad in ["0", "70000", "ssh"] {
            set(&mut form, Field::Port, bad);
            let error = form.build().err().unwrap();
            assert!(error.starts_with("PORT_INVALID"), "{error}");
        }
        set(&mut form, Field::Port, "2222");
        assert!(matches!(form.build(), Ok(Some(HostWrite::Create(_)))));
    }

    #[test]
    fn typed_password_is_never_part_of_displayed_text() {
        let mut form = form();
        form.focus = form
            .fields
            .iter()
            .position(|field| *field == Field::Pass)
            .unwrap();
        form.mode = InputMode::Insert;
        for character in "s3cret".chars() {
            form.type_char(character);
        }
        let line = form.field_line(form.focus, Field::Pass);
        let shown = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(shown.contains(MASKED), "{shown}");
        assert!(!shown.contains("s3cret"), "{shown}");
    }
}
