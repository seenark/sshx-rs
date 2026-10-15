mod cmdline;
mod copy;
mod delete;
mod effects;
mod failure;
mod finder;
mod first_run;
mod help;
mod host_form;
mod ports;
mod status;
mod tunnel_progress;
mod tunnels;
mod undo;
mod whichkey;
mod yesno;

pub(crate) use host_form::Prefill;

use crate::Cli;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};
use sshx::connect::{PairedCredentials, PairedSessionCredentials, StandaloneCredentials};
use sshx::discovery::{Catalog, HostEntry, discover_roots};
use sshx::keymap::{
    self, Action, Availability, ForwardArg, InputMode, Key, Keymap, MenuEntry, Step,
};
use sshx::mutation::MutationKind;
use sshx::session::ServiceForward;
use sshx::settings::{self, Preferences, RegisteredRoot, StartMode};
use sshx::tunnel::{TunnelRoute, TunnelView};
use sshx::usage::UsageIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::terminal::{AppTerminal, read_key, with_terminal};

#[derive(Clone, Copy)]
pub(crate) struct ForwardRequest {
    pub remote: u16,
    pub local: u16,
}

/// Service forwards listen on `Preferences.bind`. `:set` refuses non-loopback values; a
/// hand-edited settings file is refused the same way.
pub(crate) fn service_bind(prefs: &Preferences) -> Result<String, String> {
    let bind = cmdline::validate_bind(&prefs.bind)?;
    Ok(if bind == "localhost" {
        sshx::session::LOOPBACK.to_string()
    } else {
        bind
    })
}

/// `REMOTE=LOCAL` requests resolved against the route's last hop, listening on the configured bind.
pub(crate) fn resolve_forwards(
    prefs: &Preferences,
    target: &HostEntry,
    requests: &[ForwardRequest],
) -> Result<Vec<ServiceForward>, String> {
    let bind = service_bind(prefs)?;
    let specifications = requests
        .iter()
        .map(|forward| format!("{}={}", forward.remote, forward.local))
        .collect::<Vec<_>>();
    let mut forwards = sshx::session::resolve_forwards(target, &specifications)?;
    for forward in &mut forwards {
        forward.bind_address.clone_from(&bind);
    }
    Ok(forwards)
}

#[derive(Clone)]
pub(crate) struct ConnectRequest {
    pub entry: HostEntry,
    pub alias: String,
    pub forwards: Vec<ForwardRequest>,
}

/// What Enter does on a finder row. `sshx tui` always uses `Session`; the CLI continuations
/// pick the variant that matches the command they stand in for.
#[derive(Clone, Copy, Default)]
pub(crate) enum Pick {
    #[default]
    Session,
    Tunnel,
    /// `--bind`: Enter opens the ports screen, whose Enter then connects.
    Ports,
    Action(crate::HostAction),
    Show,
    Edit,
    Delete,
}

/// `-L`, `-R` and `-D` from the command line. They are fixed: every direct Session or Tunnel
/// started in this run opens them next to the ports screen's service forwards.
#[derive(Clone, Default)]
pub(crate) struct FixedForwards {
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub dynamic: Vec<String>,
    pub allow_bind: bool,
}

impl FixedForwards {
    pub(crate) fn is_empty(&self) -> bool {
        self.local.is_empty() && self.remote.is_empty() && self.dynamic.is_empty()
    }

    pub(crate) fn specs(&self) -> Result<Vec<sshx::session::ForwardSpec>, String> {
        sshx::tunnel::parse_forwards(&self.local, &self.remote, &self.dynamic, self.allow_bind)
    }

    /// One display line per forward, as the ports screen lists them.
    pub(crate) fn rows(&self) -> Vec<String> {
        self.specs()
            .unwrap_or_default()
            .iter()
            .map(|spec| format!("-{} {} (from command line)", spec.flag(), spec.effective))
            .collect()
    }

    /// Pair routes accept only service forwards.
    pub(crate) fn check_direct_route(&self) -> Result<(), String> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(
                "PAIR_FORWARD_INVALID: Pair routes accept only REMOTE[=LOCAL] service forwards"
                    .to_string(),
            )
        }
    }
}

/// `--preview` on a host form or delete confirmation: the review is printed, nothing is written.
#[derive(Clone, Copy)]
pub(crate) enum Preview {
    Create,
    Edit(MutationKind),
}

/// Finder behavior chosen by the CLI command, `--forward` values every ports screen starts
/// with, and the forwards fixed on the command line.
#[derive(Clone, Default)]
pub(crate) struct Intent {
    pub pick: Pick,
    pub preselect: Vec<ForwardArg>,
    pub prefill: Prefill,
    pub fixed: FixedForwards,
    pub preview: Option<Preview>,
}

/// Password attempts retained across retries of one HostEntry, so a descriptor that was read
/// once is reused instead of read again.
#[derive(Default)]
pub(crate) struct Credentials {
    session: Option<(String, PairedSessionCredentials)>,
    tunnel: Option<(String, StandaloneCredentials)>,
}

impl Credentials {
    pub(crate) fn paired_session(
        &mut self,
        id: &str,
        input: impl FnOnce() -> PairedCredentials,
    ) -> &mut PairedSessionCredentials {
        if self.session.as_ref().is_some_and(|(held, _)| held != id) {
            self.session = None;
        }
        &mut self
            .session
            .get_or_insert_with(|| (id.to_string(), PairedSessionCredentials::new(input())))
            .1
    }

    pub(crate) fn standalone(
        &mut self,
        id: &str,
        password_fd: Option<i32>,
    ) -> &mut StandaloneCredentials {
        if self.tunnel.as_ref().is_some_and(|(held, _)| held != id) {
            self.tunnel = None;
        }
        &mut self
            .tunnel
            .get_or_insert_with(|| (id.to_string(), StandaloneCredentials::new(password_fd)))
            .1
    }
}

/// A HostEntry located again in the app's own discovery.
pub(crate) struct HostRef {
    pub id: String,
    pub path: String,
    pub alias: String,
}

impl HostRef {
    pub(crate) fn new(entry: &HostEntry, alias: &str) -> Self {
        Self {
            id: entry.id.clone(),
            path: entry.source.path.clone(),
            alias: alias.to_string(),
        }
    }
}

pub(crate) enum StartScreen {
    Finder,
    Ports(HostRef),
    Tunnels {
        selected: Option<String>,
        route: Option<TunnelRoute>,
    },
    NewHost,
    EditHost(HostRef),
    DeleteHost(HostRef),
}

pub(crate) struct AppStart {
    pub intent: Intent,
    pub screen: StartScreen,
}

impl Default for AppStart {
    fn default() -> Self {
        Self {
            intent: Intent::default(),
            screen: StartScreen::Finder,
        }
    }
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
    Copy(Box<copy::CopyRequest>),
    SaveHost(Box<host_form::SaveRequest>),
    EditInEditor(Box<host_form::EditorRequest>),
    DeleteHost(Box<delete::DeleteRequest>),
    Undo,
    Redo,
    TunnelStart(Box<tunnel_progress::TunnelStartRequest>),
    TunnelStop(Vec<String>),
    TunnelRestart(String),
    TunnelOpen(String),
    SaveService(Box<ports::ServiceEdit>),
    CopyMany(Vec<copy::CopyRequest>),
    CopyTunnelId(String),
    DeleteHosts(Vec<delete::DeleteRequest>),
    TunnelStartMany(Box<tunnel_progress::TunnelBatch>),
    HostAction(Box<effects::HostActionRequest>),
    Print(String),
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
    /// Called on every screen of the stack after discovery re-read the config files.
    fn on_refresh(&mut self, _ctx: &Ctx) {}
}

pub(crate) struct Ctx<'a> {
    pub home: &'a Path,
    pub entries: &'a [HostEntry],
    pub visible: Vec<&'a HostEntry>,
    pub roots: &'a [RegisteredRoot],
    pub usage: &'a UsageIndex,
    pub tunnels: &'a [TunnelView],
    pub prefs: &'a Preferences,
    pub service_counts: &'a HashMap<String, usize>,
    pub intent: &'a Intent,
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
    intent: Intent,
    tunnel_route: Option<TunnelRoute>,
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
            intent: &self.intent,
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
        self.tunnels = sshx::tunnel::list(&self.home, self.tunnel_route)
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
    undo: undo::UndoStack,
    credentials: Credentials,
}

pub(crate) fn run_app(
    cli: &Cli,
    home: &Path,
    roots: Vec<RegisteredRoot>,
    initial_toast: Option<String>,
) -> Result<(), String> {
    run_app_with(cli, home, roots, initial_toast, AppStart::default())
}

pub(crate) fn run_app_with(
    cli: &Cli,
    home: &Path,
    roots: Vec<RegisteredRoot>,
    initial_toast: Option<String>,
    start: AppStart,
) -> Result<(), String> {
    Runtime::new(cli, home, roots, initial_toast, start)?.run()
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
        start: AppStart,
    ) -> Result<Self, String> {
        let first_run = cli.config.is_none()
            && matches!(start.screen, StartScreen::Finder)
            && settings::load(home)?.is_empty();
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
            intent: start.intent,
            tunnel_route: match &start.screen {
                StartScreen::Tunnels { route, .. } => *route,
                _ => None,
            },
        };
        let discovery_error = data.refresh(cli);
        let mode = input_mode(data.prefs.startmode);
        let mut finder = finder::Finder::new(mode);
        if let Some((id, alias)) = prefill_selection(&data, cli)? {
            finder.select_entry(&data.ctx(None), &id, &alias);
        }
        let stack = if first_run {
            vec![Box::new(first_run::FirstRun::new(home)) as Box<dyn Screen>]
        } else {
            start_stack(&data, finder, start.screen)?
        };
        Ok(Self {
            cli,
            data,
            stack,
            keymap: Keymap::new(),
            which_key: None,
            undo: undo::UndoStack::default(),
            credentials: Credentials::default(),
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
            if result.refresh {
                let ctx = self.data.ctx(None);
                for screen in &mut self.stack {
                    screen.on_refresh(&ctx);
                }
            }
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

/// The finder, with the screen the CLI command asked for opened on top of it. The tunnel list
/// stands alone: it is the whole operation for `tunnel status|stop|restart`.
fn start_stack(
    data: &Data,
    finder: finder::Finder,
    screen: StartScreen,
) -> Result<Vec<Box<dyn Screen>>, String> {
    let ctx = data.ctx(None);
    let locate = |host: &HostRef| {
        ports::find_entry(ctx.entries, &host.id, &host.path, &host.alias).ok_or_else(|| {
            format!(
                "HOST_NOT_FOUND: {} is no longer in {}",
                host.alias, host.path
            )
        })
    };
    let top: Box<dyn Screen> = match screen {
        StartScreen::Finder => return Ok(vec![Box::new(finder)]),
        StartScreen::Tunnels { selected, .. } => {
            let list = tunnels::Tunnels::new();
            return Ok(vec![Box::new(match selected {
                Some(id) => list.selecting(&id, &ctx),
                None => list,
            })]);
        }
        StartScreen::Ports(host) => {
            Box::new(ports::Ports::open(&ctx, locate(&host)?, &host.alias)?)
        }
        StartScreen::NewHost => {
            Box::new(host_form::HostForm::create(&ctx)?.prefilled(ctx.intent.prefill.clone()))
        }
        StartScreen::EditHost(host) => Box::new(
            host_form::HostForm::edit(&ctx, locate(&host)?, &host.alias)?
                .prefilled(ctx.intent.prefill.clone()),
        ),
        StartScreen::DeleteHost(host) => Box::new(delete::DeleteConfirm::new(
            &ctx,
            locate(&host)?,
            &host.alias,
        )?),
    };
    Ok(vec![Box::new(finder), top])
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
