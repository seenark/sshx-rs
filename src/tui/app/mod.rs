mod cmdline;
mod effects;
mod finder;
mod first_run;
mod help;
mod status;
mod whichkey;

use crate::Cli;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};
use sshx::discovery::{Catalog, HostEntry, discover_roots};
use sshx::keymap::{self, Action, Availability, InputMode, Key, Keymap, MenuEntry, Step};
use sshx::settings::{self, Preferences, RegisteredRoot, StartMode};
use sshx::tunnel::TunnelView;
use sshx::usage::UsageIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::terminal::{AppTerminal, read_key, with_terminal};

pub(crate) struct ForwardRequest {
    pub remote: u16,
    pub local: u16,
}

pub(crate) struct ConnectRequest {
    pub entry: HostEntry,
    pub alias: String,
    pub forwards: Vec<ForwardRequest>,
}

pub(crate) enum PreferenceChange {
    StartMode(StartMode),
    AutoInfo(bool),
    Bind(String),
    ClipboardClear(Option<u64>),
    PasswordConfirm(bool),
}

pub(crate) enum Effect {
    Quit { cancelled: bool },
    Connect(Box<ConnectRequest>),
    Tunnels,
    SetPreference(PreferenceChange),
    RootAdd(String),
    RootRemove(String),
    RegisterRoots(Vec<RegisteredRoot>),
}

#[derive(Default)]
pub(crate) struct EffectResult {
    pub toast: Option<String>,
    pub push: Option<Box<dyn Screen>>,
    pub exit: Option<i32>,
    pub refresh: bool,
}

pub(crate) enum Outcome {
    Stay,
    Toast(String),
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
    Effect(Effect),
}

#[derive(Default)]
pub(crate) struct ScreenStatus {
    pub filter: Option<String>,
    pub position: Option<(usize, usize)>,
}

pub(crate) trait Screen {
    fn kind(&self) -> keymap::Screen;
    fn input_mode(&self) -> InputMode;
    fn availability(&self, ctx: &Ctx) -> Availability;
    fn on_action(&mut self, action: Action, count: usize, ctx: &Ctx) -> Outcome;
    fn on_key(&mut self, key: Key, ctx: &Ctx) -> Outcome;
    fn draw(&self, frame: &mut Frame, area: Rect, ctx: &Ctx);

    fn status(&self, _ctx: &Ctx) -> ScreenStatus {
        ScreenStatus::default()
    }
    /// Overlays are drawn over the screen below them, with the full frame as area.
    fn is_overlay(&self) -> bool {
        false
    }
    /// Insert-mode printable bindings flagged `when_input_empty` act as typed text otherwise.
    fn query_is_empty(&self) -> bool {
        true
    }
    /// Screens that interpret raw keys themselves (not part of the keymap registry).
    fn uses_keymap(&self) -> bool {
        true
    }
    /// Close this screen after it emits a toast or an effect.
    fn dismiss_on_outcome(&self) -> bool {
        false
    }
}

pub(crate) struct Ctx<'a> {
    pub home: &'a Path,
    pub entries: &'a [HostEntry],
    pub visible: Vec<&'a HostEntry>,
    #[allow(dead_code)]
    pub roots: &'a [RegisteredRoot],
    pub usage: &'a UsageIndex,
    pub tunnels: &'a [TunnelView],
    pub prefs: &'a Preferences,
    pub service_counts: &'a HashMap<String, usize>,
    pub now: u64,
    pub toast: Option<&'a str>,
}

impl Ctx<'_> {
    pub(crate) fn running_tunnels(&self) -> usize {
        self.tunnels
            .iter()
            .filter(|tunnel| tunnel.state == "active")
            .count()
    }

    pub(crate) fn service_count(&self, entry: &HostEntry) -> usize {
        self.service_counts.get(&entry.id).copied().unwrap_or(0)
    }

    pub(crate) fn stat(&self, entry: &HostEntry) -> finder::Stat {
        finder::Stat {
            count: self.usage.frequency(entry),
            last_used: self.usage.last_used(entry).filter(|time| *time > 0),
        }
    }
}

struct Data {
    home: PathBuf,
    roots: Vec<RegisteredRoot>,
    catalog: Catalog,
    visible: Vec<usize>,
    sources: HashMap<String, Option<Vec<u8>>>,
    usage: UsageIndex,
    tunnels: Vec<TunnelView>,
    prefs: Preferences,
    service_counts: HashMap<String, usize>,
}

impl Data {
    fn ctx<'a>(&'a self, toast: Option<&'a str>) -> Ctx<'a> {
        Ctx {
            home: &self.home,
            entries: &self.catalog.entries,
            visible: self
                .visible
                .iter()
                .map(|index| &self.catalog.entries[*index])
                .collect(),
            roots: &self.roots,
            usage: &self.usage,
            tunnels: &self.tunnels,
            prefs: &self.prefs,
            service_counts: &self.service_counts,
            now: sshx::usage::now(),
            toast,
        }
    }

    /// Re-reads discovery, usage state and tunnels; returns the discovery error, if any.
    fn refresh(&mut self, cli: &Cli) -> Option<String> {
        let configured = settings::discovery_roots(&self.roots);
        let missing_default = cli.config.is_none()
            && configured.len() == 1
            && configured[0].path == self.home.join(".ssh/config")
            && !configured[0].path.exists();
        let mut error = None;
        self.catalog = if missing_default {
            None
        } else {
            discover_roots(&configured)
                .map_err(|failure| error = Some(failure.to_string()))
                .ok()
        }
        .unwrap_or_else(|| Catalog {
            entries: Vec::new(),
            diagnostics: Vec::new(),
        });
        self.visible = self
            .catalog
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| crate::entry_matches_provenance(entry, cli))
            .map(|(index, _)| index)
            .collect();
        self.sources = crate::catalog_source_snapshots(&self.catalog)
            .into_iter()
            .map(|(path, bytes)| (path.to_string(), bytes))
            .collect();
        self.usage = UsageIndex::load(&configured);
        self.service_counts = self
            .catalog
            .entries
            .iter()
            .filter_map(|entry| {
                let count = sshx::session::declared_services(entry).map_or(0, |list| list.len());
                (count > 0).then(|| (entry.id.clone(), count))
            })
            .collect();
        self.refresh_tunnels();
        error
    }

    fn refresh_tunnels(&mut self) {
        self.tunnels = sshx::tunnel::list(&self.home, None)
            .map(|response| response.tunnels)
            .unwrap_or_default();
    }
}

struct WhichKey {
    title: &'static str,
    entries: Vec<MenuEntry>,
}

pub(crate) struct Runtime<'a> {
    cli: &'a Cli,
    data: Data,
    stack: Vec<Box<dyn Screen>>,
    keymap: Keymap,
    which_key: Option<WhichKey>,
    toast: Option<String>,
}

pub(crate) fn run_app(
    cli: &Cli,
    home: &Path,
    roots: Vec<RegisteredRoot>,
    initial_toast: Option<String>,
) -> Result<(), String> {
    Runtime::new(cli, home, roots, initial_toast)?.run()
}

fn input_mode(start: StartMode) -> InputMode {
    match start {
        StartMode::Insert => InputMode::Insert,
        StartMode::Normal => InputMode::Normal,
    }
}

fn map_key(event: KeyEvent) -> Option<Key> {
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    Some(match event.code {
        KeyCode::Char(character) if control => Key::Ctrl(character),
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F(number) => Key::F(number),
        _ => return None,
    })
}

impl<'a> Runtime<'a> {
    fn new(
        cli: &'a Cli,
        home: &Path,
        roots: Vec<RegisteredRoot>,
        initial_toast: Option<String>,
    ) -> Result<Self, String> {
        let first_run = cli.config.is_none() && settings::load(home)?.is_empty();
        let mut data = Data {
            home: home.to_path_buf(),
            roots,
            catalog: Catalog {
                entries: Vec::new(),
                diagnostics: Vec::new(),
            },
            visible: Vec::new(),
            sources: HashMap::new(),
            usage: UsageIndex::default(),
            tunnels: Vec::new(),
            prefs: settings::load_preferences(home)?,
            service_counts: HashMap::new(),
        };
        let discovery_error = data.refresh(cli);
        let mode = input_mode(data.prefs.startmode);
        let mut finder = finder::Finder::new(mode);
        if let Some((id, alias)) = prefill_selection(&data, cli)? {
            finder.select_entry(&data.ctx(None), &id, &alias);
        }
        let screen: Box<dyn Screen> = if first_run {
            Box::new(first_run::FirstRun::new(home))
        } else {
            Box::new(finder)
        };
        Ok(Self {
            cli,
            data,
            stack: vec![screen],
            keymap: Keymap::new(),
            which_key: None,
            toast: match (initial_toast, discovery_error) {
                (Some(toast), Some(error)) => Some(format!("{toast}\n{error}")),
                (toast, error) => toast.or(error),
            },
        })
    }

    fn run(&mut self) -> Result<(), String> {
        loop {
            let effect = with_terminal(
                "hosts",
                "TUI_REQUIRED: sshx tui requires usable stdin and stderr terminals".to_string(),
                |terminal, input| self.event_loop(terminal, input),
            )?;
            let result = effects::execute(self, effect)?;
            if let Some(code) = result.exit {
                return match code {
                    0 => Ok(()),
                    code => std::process::exit(code),
                };
            }
            let error = if result.refresh {
                self.data.refresh(self.cli)
            } else {
                self.data.refresh_tunnels();
                None
            };
            if error.is_some() {
                self.toast = error;
            }
            if let Some(toast) = result.toast {
                self.toast = Some(toast);
            }
            if let Some(screen) = result.push {
                self.stack.push(screen);
            }
            self.which_key = None;
            self.keymap.reset();
        }
    }

    fn event_loop(
        &mut self,
        terminal: &mut AppTerminal,
        input: &mut std::fs::File,
    ) -> Result<Effect, String> {
        loop {
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|error| format!("HOST_REQUIRED: cannot draw terminal: {error}"))?;
            let event = read_key(input, "HOST_REQUIRED")?;
            self.toast = None;
            let Some(key) = map_key(event) else {
                continue;
            };
            if let Some(effect) = self.handle_key(key) {
                return Ok(effect);
            }
        }
    }

    fn handle_key(&mut self, key: Key) -> Option<Effect> {
        if key == Key::Ctrl('c') {
            return Some(Effect::Quit { cancelled: true });
        }
        let ctx = self.data.ctx(None);
        let top = self.stack.last_mut()?;
        let dismiss = top.dismiss_on_outcome();
        let outcome = if top.uses_keymap() {
            let availability = top.availability(&ctx);
            match self
                .keymap
                .feed(key, top.input_mode(), top.kind(), &availability)
            {
                Step::Action {
                    action,
                    count,
                    when_input_empty,
                } => {
                    self.which_key = None;
                    if when_input_empty && !top.query_is_empty() {
                        top.on_key(key, &ctx)
                    } else if action == Action::Help {
                        match top.kind() {
                            keymap::Screen::Help => Outcome::Pop,
                            keymap::Screen::CommandLine => Outcome::Stay,
                            kind => Outcome::Push(Box::new(help::Help::new(
                                kind,
                                top.input_mode(),
                                availability,
                            ))),
                        }
                    } else {
                        top.on_action(action, count as usize, &ctx)
                    }
                }
                Step::Pending { title, entries } => {
                    self.which_key = (!entries.is_empty()).then_some(WhichKey { title, entries });
                    Outcome::Stay
                }
                Step::Unavailable { reason, .. } => {
                    self.which_key = None;
                    Outcome::Toast(reason.to_string())
                }
                Step::Unmapped => {
                    self.which_key = None;
                    top.on_key(key, &ctx)
                }
                Step::Cancelled => {
                    self.which_key = None;
                    Outcome::Stay
                }
            }
        } else {
            top.on_key(key, &ctx)
        };
        match outcome {
            Outcome::Stay => None,
            Outcome::Toast(toast) => {
                self.toast = Some(toast);
                if dismiss {
                    self.stack.pop();
                }
                None
            }
            Outcome::Push(screen) => {
                self.stack.push(screen);
                None
            }
            Outcome::Pop => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
                None
            }
            Outcome::Replace(screen) => {
                if let Some(top) = self.stack.last_mut() {
                    *top = screen;
                }
                None
            }
            Outcome::Effect(effect) => {
                if dismiss {
                    self.stack.pop();
                }
                Some(effect)
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let full = frame.area();
        let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(full);
        let (body, status_row) = (rows[0], rows[1]);
        let ctx = self.data.ctx(self.toast.as_deref());
        let base = self
            .stack
            .iter()
            .rposition(|screen| !screen.is_overlay())
            .unwrap_or(0);
        self.stack[base].draw(frame, body, &ctx);
        if let Some(top) = self.stack.last() {
            status::draw(
                frame,
                status_row,
                &status::StatusLine {
                    mode: top.input_mode(),
                    info: top.status(&ctx),
                    running_tunnels: ctx.running_tunnels(),
                    toast: ctx.toast,
                    pending: &self.keymap.pending_keys(),
                },
            );
        }
        for screen in &self.stack[base + 1..] {
            screen.draw(frame, full, &ctx);
        }
        if self.data.prefs.autoinfo
            && let Some(which_key) = &self.which_key
        {
            whichkey::draw(frame, body, which_key.title, &which_key.entries);
        }
    }
}

/// `--id` or `--source`/`--line` preselect a HostEntry in the finder.
fn prefill_selection(data: &Data, cli: &Cli) -> Result<Option<(String, String)>, String> {
    if cli.id.is_none() && cli.location.source.is_none() && cli.location.line.is_none() {
        return Ok(None);
    }
    let ctx = data.ctx(None);
    let selected = if cli.id.is_some() {
        let selection = crate::select_connect_entry(&ctx.visible, None, cli, "Hosts")?;
        (selection.entry.id.clone(), selection.alias.to_string())
    } else {
        let path = crate::source_selector_path(
            cli.location.source.as_ref().ok_or_else(|| {
                "SELECTOR_INCOMPLETE: --source and --line must be provided together".to_string()
            })?,
            &data.home,
        );
        let matches = ctx
            .visible
            .iter()
            .copied()
            .filter(|entry| {
                entry.source.path == path.to_string_lossy()
                    && Some(entry.source.line_start) == cli.location.line
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => (
                entry.id.clone(),
                entry.aliases.first().cloned().unwrap_or_default(),
            ),
            [] => return Err("HOST_NOT_FOUND: source and Host line matched no entries".to_string()),
            many => return Err(crate::format_ambiguous("source and Host line", many)),
        }
    };
    Ok(Some(selected))
}

#[cfg(test)]
mod tests {
    use super::map_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use sshx::keymap::Key;

    #[test]
    fn terminal_keys_map_onto_keymap_keys() {
        let key = |code, modifiers| map_key(KeyEvent::new(code, modifiers));
        assert_eq!(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Key::Ctrl('c'))
        );
        assert_eq!(
            key(KeyCode::Char(' '), KeyModifiers::empty()),
            Some(Key::Char(' '))
        );
        assert_eq!(key(KeyCode::F(1), KeyModifiers::empty()), Some(Key::F(1)));
        assert_eq!(key(KeyCode::BackTab, KeyModifiers::empty()), None);
    }
}
