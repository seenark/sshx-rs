//! Pure keymap engine: one static registry drives dispatch, which-key menus,
//! help listings and `:` command completion. No terminal I/O lives here.
//!
//! Screens without a text field (ports, tunnels, ...) pass [`InputMode::Normal`]
//! unless they are actively editing a row, in which case they pass
//! [`InputMode::Insert`]. The [`Screen::Help`] and [`Screen::CommandLine`]
//! overlays are fed their own screen value; help *content* is queried with the
//! underlying screen.

use std::fmt;
use std::sync::LazyLock;

/// A terminal-independent key. The binary maps decoded input onto it;
/// `Char(' ')` is the space key (label `space`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    F(u8),
}

impl Key {
    /// Display text, also accepted by [`parse_keys`] (`space`, `^d`, `<esc>` ...).
    pub fn label(&self) -> String {
        match self {
            Key::Char(' ') => "space".into(),
            Key::Char(c) => c.to_string(),
            Key::Ctrl(c) => format!("^{c}"),
            Key::Enter => "enter".into(),
            Key::Esc => "esc".into(),
            Key::Tab => "tab".into(),
            Key::Backspace => "backspace".into(),
            Key::Up => "up".into(),
            Key::Down => "down".into(),
            Key::Left => "left".into(),
            Key::Right => "right".into(),
            Key::PageUp => "pageup".into(),
            Key::PageDown => "pagedown".into(),
            Key::F(n) => format!("F{n}"),
        }
    }

    fn named(name: &str) -> Option<Key> {
        Some(match name {
            "space" => Key::Char(' '),
            "enter" | "cr" => Key::Enter,
            "esc" => Key::Esc,
            "tab" => Key::Tab,
            "backspace" | "bs" => Key::Backspace,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            _ => return Key::function(name),
        })
    }

    fn function(token: &str) -> Option<Key> {
        let digits = token.strip_prefix('F')?;
        let n: u8 = digits.parse().ok()?;
        (1..=24).contains(&n).then_some(Key::F(n))
    }
}

/// Parse binding notation: whitespace-separated tokens. A token is `space`,
/// `^x` (Ctrl), `F1`, `<name>` for any named key, or otherwise a run of plain
/// characters (`gg` is two keys). Example: `"space t t"`, `"<esc>"`.
pub fn parse_keys(notation: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for token in notation.split_whitespace() {
        if let Some(name) = token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            keys.push(Key::named(name).ok_or_else(|| format!("unknown key `<{name}>`"))?);
        } else if token == "space" {
            keys.push(Key::Char(' '));
        } else if let Some(f) = Key::function(token) {
            keys.push(f);
        } else if let Some(c) = token.strip_prefix('^').filter(|c| c.chars().count() == 1) {
            keys.push(Key::Ctrl(c.chars().next().unwrap()));
        } else {
            keys.extend(token.chars().map(Key::Char));
        }
    }
    Ok(keys)
}

fn join_labels(keys: &[Key]) -> String {
    keys.iter().map(Key::label).collect::<Vec<_>>().join(" ")
}

/// TUI keyboard state (glossary: input mode), unrelated to Session/Tunnel mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputMode {
    Insert,
    Normal,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Finder,
    Ports,
    HostForm,
    DeleteConfirm,
    Tunnels,
    TunnelProgress,
    Failure,
    Help,
    CommandLine,
}

/// Help grouping. Declaration order is the display order after the current
/// screen's own section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Connect,
    Copy,
    Hosts,
    Ports,
    Tunnels,
    Form,
    Failure,
    Nav,
    Modes,
    General,
}

impl Section {
    pub fn title(&self) -> &'static str {
        match self {
            Section::Connect => "CONNECT",
            Section::Copy => "COPY",
            Section::Hosts => "HOSTS",
            Section::Ports => "PORTS",
            Section::Tunnels => "TUNNELS",
            Section::Form => "FORM",
            Section::Failure => "FAILURE",
            Section::Nav => "NAV",
            Section::Modes => "MODES",
            Section::General => "GENERAL",
        }
    }

    fn home(screen: Screen) -> Section {
        match screen {
            Screen::Finder => Section::Connect,
            Screen::Ports => Section::Ports,
            Screen::HostForm => Section::Form,
            Screen::DeleteConfirm => Section::Hosts,
            Screen::Tunnels | Screen::TunnelProgress => Section::Tunnels,
            Screen::Failure => Section::Failure,
            Screen::Help | Screen::CommandLine => Section::General,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyTarget {
    SshCommand,
    SshpassCommand,
    Password,
    HostName,
    TunnelId,
    VerboseSshCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoTarget {
    Recent,
    Personal,
    Work,
    Tunnels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterTarget {
    Personal,
    Work,
    Recent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
    EnterInsert,
    EnterNormal,
    EnterSelect,
    Connect,
    OpenPorts,
    Copy(CopyTarget),
    NewHost,
    EditHostForm,
    EditHostInEditor,
    DeleteHost,
    Undo,
    Redo,
    CommandLine,
    Help,
    Quit,
    Goto(GotoTarget),
    Filter(FilterTarget),
    TunnelStart,
    TunnelList,
    TunnelStop,
    TunnelStopAll,
    TunnelRestart,
    ToggleRow,
    AddRow,
    DeleteRow,
    SaveRow,
    Write,
    WriteQuit,
    NextField,
    PrevField,
    /// Leave the current screen or overlay.
    Back,
    Confirm,
    /// Commit the text being typed (row, field, command line).
    Submit,
    /// Tab-complete the command line.
    Complete,
    Retry,
    ShowLog,
}

impl Action {
    /// Only movement honours a numeric count prefix.
    pub fn is_movement(&self) -> bool {
        matches!(
            self,
            Action::Up
                | Action::Down
                | Action::Top
                | Action::Bottom
                | Action::HalfPageDown
                | Action::HalfPageUp
        )
    }
}

/// What the highlighted row offers; drives dimming and refusal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Availability {
    /// A stored password exists for the highlighted HostEntry.
    pub has_password: bool,
    /// The highlighted HostEntry is a Pair member.
    pub is_pair: bool,
    /// A standalone tunnel is selected (or runs for the highlighted HostEntry).
    pub tunnel_selected: bool,
    /// The current list has a highlighted row.
    pub row_selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    Always,
    Row,
    /// A stored password, any route.
    Password,
    /// A stored password on a direct (non-Pair) HostEntry.
    DirectPassword,
    Tunnel,
}

impl Requirement {
    /// `None` when satisfied, otherwise why the action is unavailable.
    pub fn unmet(&self, avail: &Availability) -> Option<&'static str> {
        match self {
            Requirement::Always => None,
            Requirement::Row => (!avail.row_selected).then_some("nothing selected"),
            Requirement::Password => (!avail.has_password).then_some("no stored password"),
            Requirement::DirectPassword => {
                if avail.is_pair {
                    Some("not available for Pairs")
                } else {
                    (!avail.has_password).then_some("no stored password")
                }
            }
            Requirement::Tunnel => (!avail.tunnel_selected).then_some("no tunnel selected"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Key notation (see [`parse_keys`]). Empty means `:` command only.
    pub keys: &'static str,
    pub modes: &'static [InputMode],
    pub screens: &'static [Screen],
    pub action: Action,
    pub section: Section,
    pub description: &'static str,
    /// `:` command that triggers the same action.
    pub command: Option<&'static str>,
    pub requires: Requirement,
    /// Insert-mode printable binding that the screen must only honour while
    /// its query is empty (`?`). Printable keys without it are typed text.
    pub when_input_empty: bool,
}

const fn bind(
    keys: &'static str,
    modes: &'static [InputMode],
    screens: &'static [Screen],
    action: Action,
    section: Section,
    description: &'static str,
) -> Binding {
    Binding {
        keys,
        modes,
        screens,
        action,
        section,
        description,
        command: None,
        requires: Requirement::Always,
        when_input_empty: false,
    }
}

impl Binding {
    const fn cmd(mut self, command: &'static str) -> Self {
        self.command = Some(command);
        self
    }
    const fn req(mut self, requires: Requirement) -> Self {
        self.requires = requires;
        self
    }
    const fn if_empty(mut self) -> Self {
        self.when_input_empty = true;
        self
    }
}

const INSERT: &[InputMode] = &[InputMode::Insert];
const NORMAL: &[InputMode] = &[InputMode::Normal];
const SELECT: &[InputMode] = &[InputMode::Select];
const INSERT_NORMAL: &[InputMode] = &[InputMode::Insert, InputMode::Normal];
const NORMAL_SELECT: &[InputMode] = &[InputMode::Normal, InputMode::Select];
const ALL_MODES: &[InputMode] = &[InputMode::Insert, InputMode::Normal, InputMode::Select];

const FINDER: &[Screen] = &[Screen::Finder];
const PORTS: &[Screen] = &[Screen::Ports];
const FORM: &[Screen] = &[Screen::HostForm];
const CONFIRM: &[Screen] = &[Screen::DeleteConfirm];
const TUNNELS: &[Screen] = &[Screen::Tunnels];
const FAILURE: &[Screen] = &[Screen::Failure];
const HELP: &[Screen] = &[Screen::Help];
const COMMAND_LINE: &[Screen] = &[Screen::CommandLine];
const ALL_SCREENS: &[Screen] = &[
    Screen::Finder,
    Screen::Ports,
    Screen::HostForm,
    Screen::DeleteConfirm,
    Screen::Tunnels,
    Screen::TunnelProgress,
    Screen::Failure,
    Screen::Help,
    Screen::CommandLine,
];
const LISTS: &[Screen] = &[Screen::Finder, Screen::Ports, Screen::Tunnels, Screen::Help];
const PORTS_FORM: &[Screen] = &[Screen::Ports, Screen::HostForm];
const HELP_COMMAND_LINE: &[Screen] = &[Screen::Help, Screen::CommandLine];
const FINDER_PORTS: &[Screen] = &[Screen::Finder, Screen::Ports];
const FINDER_HELP: &[Screen] = &[Screen::Finder, Screen::Help];
const FINDER_TUNNELS: &[Screen] = &[Screen::Finder, Screen::Tunnels];
const TUNNEL_ID: &[Screen] = &[Screen::Finder, Screen::Tunnels, Screen::TunnelProgress];
const NOT_OVERLAY: &[Screen] = &[
    Screen::Finder,
    Screen::Ports,
    Screen::HostForm,
    Screen::DeleteConfirm,
    Screen::Tunnels,
    Screen::TunnelProgress,
    Screen::Failure,
];
const QUITTABLE: &[Screen] = &[
    Screen::Finder,
    Screen::Ports,
    Screen::Tunnels,
    Screen::TunnelProgress,
    Screen::Failure,
];
const COMMANDABLE: &[Screen] = &[
    Screen::Finder,
    Screen::Ports,
    Screen::HostForm,
    Screen::Tunnels,
];
const TUNNEL_LISTING: &[Screen] = &[Screen::Finder, Screen::Ports, Screen::Tunnels];
const BACKABLE: &[Screen] = &[
    Screen::Ports,
    Screen::DeleteConfirm,
    Screen::Tunnels,
    Screen::TunnelProgress,
    Screen::Failure,
    Screen::Help,
];

use Action as A;
use Requirement::{DirectPassword, Password, Row, Tunnel};
use Section as S;

/// Menu titles of prefix nodes, keyed by label sequence.
static PREFIX_TITLES: &[(&str, &str)] = &[
    ("space", "Space"),
    ("space f", "Filter"),
    ("space t", "Tunnel"),
    ("g", "Goto"),
    ("y", "Yank"),
    ("d", "Delete"),
    ("c", "Change"),
    ("Z", "Close form"),
];

#[rustfmt::skip]
static BINDINGS: &[Binding] = &[
    // Global
    bind("F1", ALL_MODES, ALL_SCREENS, A::Help, S::General, "Show help").cmd("help"),
    bind("^c", ALL_MODES, ALL_SCREENS, A::Quit, S::General, "Quit"),
    bind("?", NORMAL_SELECT, NOT_OVERLAY, A::Help, S::General, "Show help"),
    bind("?", INSERT, FINDER, A::Help, S::General, "Show help (empty query)").if_empty(),
    bind("?", NORMAL, HELP, A::Back, S::General, "Close help"),
    bind("q", NORMAL, QUITTABLE, A::Quit, S::General, "Quit").cmd("q"),
    bind("q", NORMAL, FORM, A::Quit, S::Form, "Cancel and close form").cmd("q"),
    bind("q", NORMAL, HELP, A::Back, S::General, "Close help"),
    bind(":", NORMAL, COMMANDABLE, A::CommandLine, S::General, "Command line"),

    // Navigation
    bind("j", NORMAL_SELECT, LISTS, A::Down, S::Nav, "Move down"),
    bind("k", NORMAL_SELECT, LISTS, A::Up, S::Nav, "Move up"),
    bind("<down>", ALL_MODES, LISTS, A::Down, S::Nav, "Move down"),
    bind("<up>", ALL_MODES, LISTS, A::Up, S::Nav, "Move up"),
    bind("<pagedown>", ALL_MODES, LISTS, A::HalfPageDown, S::Nav, "Half page down"),
    bind("<pageup>", ALL_MODES, LISTS, A::HalfPageUp, S::Nav, "Half page up"),
    bind("^d", NORMAL_SELECT, LISTS, A::HalfPageDown, S::Nav, "Half page down"),
    bind("^u", NORMAL_SELECT, LISTS, A::HalfPageUp, S::Nav, "Half page up"),
    bind("gg", NORMAL_SELECT, LISTS, A::Top, S::Nav, "Go to top"),
    bind("ge", NORMAL_SELECT, LISTS, A::Bottom, S::Nav, "Go to bottom"),
    bind("gr", NORMAL, FINDER_TUNNELS, A::Goto(GotoTarget::Recent), S::Nav, "Go to recent"),
    bind("gp", NORMAL, FINDER_TUNNELS, A::Goto(GotoTarget::Personal), S::Nav, "Go to personal"),
    bind("gw", NORMAL, FINDER_TUNNELS, A::Goto(GotoTarget::Work), S::Nav, "Go to work"),
    bind("gt", NORMAL, FINDER_TUNNELS, A::Goto(GotoTarget::Tunnels), S::Nav, "Go to tunnels"),
    bind("space f p", NORMAL, FINDER, A::Filter(FilterTarget::Personal), S::Nav, "Filter: personal root"),
    bind("space f w", NORMAL, FINDER, A::Filter(FilterTarget::Work), S::Nav, "Filter: work root"),
    bind("space f r", NORMAL, FINDER, A::Filter(FilterTarget::Recent), S::Nav, "Filter: recent"),

    // Input modes
    bind("<esc>", INSERT, FINDER, A::EnterNormal, S::Modes, "Normal mode"),
    bind("<esc>", INSERT, PORTS_FORM, A::EnterNormal, S::Modes, "Stop typing"),
    bind("i", NORMAL, FINDER_HELP, A::EnterInsert, S::Modes, "Insert mode (type)"),
    bind("a", NORMAL, FINDER_HELP, A::EnterInsert, S::Modes, "Insert mode (type)"),
    bind("/", NORMAL, FINDER_HELP, A::EnterInsert, S::Modes, "Insert mode (type)"),
    bind("V", NORMAL, FINDER, A::EnterSelect, S::Modes, "Select mode (multi-select)"),
    bind("<esc>", SELECT, FINDER, A::EnterNormal, S::Modes, "Leave select mode"),
    bind("V", SELECT, FINDER, A::EnterNormal, S::Modes, "Leave select mode"),

    // Finder: connect, copy, hosts
    bind("<enter>", INSERT_NORMAL, FINDER, A::Connect, S::Connect, "Connect (Session mode)").req(Row),
    bind("<tab>", INSERT_NORMAL, FINDER, A::OpenPorts, S::Connect, "Open ports screen").req(Row),
    bind("y y", NORMAL, FINDER_PORTS, A::Copy(CopyTarget::SshCommand), S::Copy, "Copy ssh command").req(Row),
    bind("y p", NORMAL, FINDER_PORTS, A::Copy(CopyTarget::SshpassCommand), S::Copy, "Copy sshpass command").req(DirectPassword),
    bind("y w", NORMAL, FINDER_PORTS, A::Copy(CopyTarget::Password), S::Copy, "Copy password").req(Password),
    bind("y h", NORMAL, FINDER_PORTS, A::Copy(CopyTarget::HostName), S::Copy, "Copy HostName").req(Row),
    bind("y i", NORMAL, TUNNEL_ID, A::Copy(CopyTarget::TunnelId), S::Copy, "Copy tunnel ID").req(Tunnel),
    bind("y", SELECT, FINDER, A::Copy(CopyTarget::SshCommand), S::Copy, "Copy ssh command per selected row").req(Row),
    bind("o", NORMAL, FINDER, A::NewHost, S::Hosts, "New host (form)"),
    bind("c c", NORMAL, FINDER, A::EditHostForm, S::Hosts, "Edit host (form)").req(Row),
    bind("c e", NORMAL, FINDER, A::EditHostInEditor, S::Hosts, "Edit host in $EDITOR").req(Row),
    bind("d d", NORMAL, FINDER, A::DeleteHost, S::Hosts, "Delete host").req(Row),
    bind("d", SELECT, FINDER, A::DeleteHost, S::Hosts, "Delete selected hosts").req(Row),
    bind("u", NORMAL, FINDER_PORTS, A::Undo, S::Hosts, "Undo"),
    bind("^r", NORMAL, FINDER_PORTS, A::Redo, S::Hosts, "Redo"),

    // Tunnels (standalone)
    bind("space t t", NORMAL_SELECT, FINDER_PORTS, A::TunnelStart, S::Tunnels, "Start standalone tunnel (per selected row in Select)").req(Row).cmd("tunnel start"),
    bind("space t l", NORMAL, TUNNEL_LISTING, A::TunnelList, S::Tunnels, "List standalone tunnels").cmd("tunnel list"),
    bind("space t x", NORMAL, TUNNEL_LISTING, A::TunnelStopAll, S::Tunnels, "Stop all standalone tunnels"),
    bind("<enter>", NORMAL, TUNNELS, A::Connect, S::Tunnels, "Open SSH through tunnel master").req(Row),
    bind("d d", NORMAL, TUNNELS, A::TunnelStop, S::Tunnels, "Stop tunnel").req(Row).cmd("tunnel stop"),
    bind("r", NORMAL, TUNNELS, A::TunnelRestart, S::Tunnels, "Restart tunnel").req(Row),

    // Ports screen
    bind("<enter>", NORMAL, PORTS, A::Connect, S::Ports, "Connect with checked forwards").req(Row),
    bind("x", NORMAL, PORTS, A::ToggleRow, S::Ports, "Toggle row").req(Row),
    bind("o", NORMAL, PORTS, A::AddRow, S::Ports, "Add row (REMOTE[=LOCAL])"),
    bind("d d", NORMAL, PORTS, A::DeleteRow, S::Ports, "Remove row").req(Row),
    bind("s", NORMAL, PORTS, A::SaveRow, S::Ports, "Save row as declared service").req(Row),
    bind("<enter>", INSERT, PORTS, A::Submit, S::Ports, "Add typed row"),

    // Host form
    bind("j", NORMAL, FORM, A::NextField, S::Form, "Next field"),
    bind("<down>", NORMAL, FORM, A::NextField, S::Form, "Next field"),
    bind("<tab>", INSERT_NORMAL, FORM, A::NextField, S::Form, "Next field"),
    bind("k", NORMAL, FORM, A::PrevField, S::Form, "Previous field"),
    bind("<up>", NORMAL, FORM, A::PrevField, S::Form, "Previous field"),
    bind("i", NORMAL, FORM, A::EnterInsert, S::Form, "Edit field"),
    bind("a", NORMAL, FORM, A::EnterInsert, S::Form, "Edit field"),
    bind("<enter>", NORMAL, FORM, A::EnterInsert, S::Form, "Edit field"),
    bind("<enter>", INSERT, FORM, A::Submit, S::Form, "Finish field"),
    bind("", NORMAL, FORM, A::Write, S::Form, "Save host").cmd("w"),
    bind("Z Z", NORMAL, FORM, A::WriteQuit, S::Form, "Save and close form").cmd("wq"),

    // Delete confirmation
    bind("y", NORMAL, CONFIRM, A::Confirm, S::Hosts, "Confirm delete"),
    bind("n", NORMAL, CONFIRM, A::Back, S::Hosts, "Cancel"),

    // Failure
    bind("r", NORMAL, FAILURE, A::Retry, S::Failure, "Retry"),
    bind("e", NORMAL, FAILURE, A::EditHostForm, S::Failure, "Edit host"),
    bind("y", NORMAL, FAILURE, A::Copy(CopyTarget::VerboseSshCommand), S::Failure, "Copy ssh -v command"),
    bind("l", NORMAL, FAILURE, A::ShowLog, S::Failure, "Show full log"),

    // Back / dismiss
    bind("<esc>", NORMAL, BACKABLE, A::Back, S::General, "Back"),
    bind("<esc>", INSERT, HELP_COMMAND_LINE, A::Back, S::General, "Close"),

    // Command line
    bind("<enter>", INSERT, COMMAND_LINE, A::Submit, S::General, "Run command"),
    bind("<tab>", INSERT, COMMAND_LINE, A::Complete, S::General, "Complete command"),
];

/// One parsed registry row.
struct Parsed {
    binding: &'static Binding,
    keys: Vec<Key>,
}

fn registry() -> &'static [Parsed] {
    static PARSED: LazyLock<Vec<Parsed>> = LazyLock::new(|| {
        BINDINGS
            .iter()
            .map(|binding| Parsed {
                binding,
                keys: parse_keys(binding.keys)
                    .unwrap_or_else(|e| panic!("bad binding `{}`: {e}", binding.keys)),
            })
            .collect()
    });
    &PARSED
}

fn applies(p: &Parsed, mode: InputMode, screen: Screen) -> bool {
    p.binding.modes.contains(&mode) && p.binding.screens.contains(&screen)
}

fn prefix_title(keys: &[Key]) -> Option<&'static str> {
    let label = join_labels(keys);
    PREFIX_TITLES
        .iter()
        .find(|(k, _)| *k == label)
        .map(|(_, t)| *t)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// The next key only, e.g. `t` under `space`.
    pub key_label: String,
    /// Submenu descriptions end with `…`.
    pub description: String,
    pub is_submenu: bool,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    /// Full key sequence label, or `:command` for command-only rows.
    pub keys: String,
    pub description: &'static str,
    pub command: Option<&'static str>,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `count` is 1 unless a numeric prefix applied to a movement action.
    /// `when_input_empty` is set for Insert-mode printable bindings (`?`): the
    /// screen must treat the key as typed text unless its query is empty.
    Action {
        action: Action,
        count: u32,
        when_input_empty: bool,
    },
    /// A prefix (or count) is in progress. `title` is empty and `entries` is
    /// empty while only a count has been typed.
    Pending {
        title: &'static str,
        entries: Vec<MenuEntry>,
    },
    /// A complete binding whose requirement is not met; `reason` is toast text.
    Unavailable {
        action: Action,
        reason: &'static str,
    },
    /// No binding. Insert-mode printable keys land here: the screen types them.
    Unmapped,
    /// `esc`/backspace closed the root of a pending sequence.
    Cancelled,
}

/// Key-sequence state machine over the static registry.
#[derive(Debug, Default)]
pub struct Keymap {
    keys: Vec<Key>,
    count: Option<u32>,
}

const MAX_COUNT: u32 = 99_999;

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.keys.clear();
        self.count = None;
    }

    pub fn is_pending(&self) -> bool {
        !self.keys.is_empty() || self.count.is_some()
    }

    /// Pending prefix keys (without the count), for [`entries`].
    pub fn pending(&self) -> &[Key] {
        &self.keys
    }

    /// Status-line text such as `5 g` or `space t`.
    pub fn pending_keys(&self) -> String {
        let mut parts = Vec::new();
        if let Some(n) = self.count {
            parts.push(n.to_string());
        }
        parts.extend(self.keys.iter().map(Key::label));
        parts.join(" ")
    }

    pub fn feed(
        &mut self,
        key: Key,
        mode: InputMode,
        screen: Screen,
        avail: &Availability,
    ) -> Step {
        if mode == InputMode::Insert {
            self.reset();
            return Self::feed_insert(key, screen, avail);
        }

        if self.is_pending() && matches!(key, Key::Esc | Key::Backspace) {
            self.keys.pop();
            if self.keys.is_empty() {
                self.reset();
                return Step::Cancelled;
            }
            return self.pending_step(mode, screen, avail);
        }

        if self.keys.is_empty()
            && let Key::Char(c) = key
            && let Some(d) = c.to_digit(10)
            && (d != 0 || self.count.is_some())
        {
            self.count = Some((self.count.unwrap_or(0) * 10 + d).min(MAX_COUNT));
            return Step::Pending {
                title: "",
                entries: Vec::new(),
            };
        }

        self.keys.push(key);
        let mut exact = None;
        let mut longer = false;
        for p in registry().iter().filter(|p| applies(p, mode, screen)) {
            if p.keys == self.keys {
                exact = Some(p.binding);
            } else if p.keys.starts_with(&self.keys) {
                longer = true;
            }
        }

        if let Some(b) = exact {
            let count = self.count.filter(|_| b.action.is_movement()).unwrap_or(1);
            self.reset();
            return Self::resolve(b, count, avail);
        }
        if longer {
            return self.pending_step(mode, screen, avail);
        }
        self.reset();
        Step::Unmapped
    }

    fn pending_step(&self, mode: InputMode, screen: Screen, avail: &Availability) -> Step {
        Step::Pending {
            title: prefix_title(&self.keys).unwrap_or(""),
            entries: entries(&self.keys, mode, screen, avail),
        }
    }

    fn feed_insert(key: Key, screen: Screen, avail: &Availability) -> Step {
        let printable = matches!(key, Key::Char(_));
        registry()
            .iter()
            .filter(|p| applies(p, InputMode::Insert, screen))
            .find(|p| p.keys == [key] && (!printable || p.binding.when_input_empty))
            .map_or(Step::Unmapped, |p| Self::resolve(p.binding, 1, avail))
    }

    fn resolve(b: &Binding, count: u32, avail: &Availability) -> Step {
        match b.requires.unmet(avail) {
            Some(reason) => Step::Unavailable {
                action: b.action,
                reason,
            },
            None => Step::Action {
                action: b.action,
                count,
                when_input_empty: b.when_input_empty,
            },
        }
    }
}

/// Which-key entries below `prefix` for the given input mode and screen.
pub fn entries(
    prefix: &[Key],
    mode: InputMode,
    screen: Screen,
    avail: &Availability,
) -> Vec<MenuEntry> {
    let mut groups: Vec<(Key, Vec<&Parsed>)> = Vec::new();
    for p in registry().iter().filter(|p| {
        applies(p, mode, screen) && p.keys.len() > prefix.len() && p.keys.starts_with(prefix)
    }) {
        let next = p.keys[prefix.len()];
        match groups.iter_mut().find(|(k, _)| *k == next) {
            Some((_, g)) => g.push(p),
            None => groups.push((next, vec![p])),
        }
    }
    groups
        .into_iter()
        .map(|(next, group)| {
            let leaf = group.iter().find(|p| p.keys.len() == prefix.len() + 1);
            match leaf {
                Some(p) => MenuEntry {
                    key_label: next.label(),
                    description: p.binding.description.to_string(),
                    is_submenu: false,
                    available: p.binding.requires.unmet(avail).is_none(),
                },
                None => {
                    let mut path = prefix.to_vec();
                    path.push(next);
                    MenuEntry {
                        key_label: next.label(),
                        description: format!("{}…", prefix_title(&path).unwrap_or("More")),
                        is_submenu: true,
                        available: group
                            .iter()
                            .any(|p| p.binding.requires.unmet(avail).is_none()),
                    }
                }
            }
        })
        .collect()
}

/// Help rows grouped by section; the screen's own section comes first.
pub fn help(screen: Screen, mode: InputMode, avail: &Availability) -> Vec<(Section, Vec<HelpRow>)> {
    let mut sections: Vec<(Section, Vec<HelpRow>)> = Vec::new();
    for p in registry().iter().filter(|p| applies(p, mode, screen)) {
        let b = p.binding;
        let row = HelpRow {
            keys: if b.keys.is_empty() {
                format!(":{}", b.command.unwrap_or_default())
            } else {
                join_labels(&p.keys)
            },
            description: b.description,
            command: b.command,
            available: b.requires.unmet(avail).is_none(),
        };
        match sections.iter_mut().find(|(s, _)| *s == b.section) {
            Some((_, rows)) => rows.push(row),
            None => sections.push((b.section, vec![row])),
        }
    }
    let home = Section::home(screen);
    sections.sort_by_key(|(s, _)| (*s != home, *s));
    sections
}

/// Forward argument in the `REMOTE[=LOCAL]` grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardArg {
    pub remote: u16,
    pub local: Option<u16>,
}

impl fmt::Display for ForwardArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.local {
            Some(local) => write!(f, "{}={}", self.remote, local),
            None => write!(f, "{}", self.remote),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    Write,
    WriteQuit,
    Help,
    Ssh {
        host: String,
        forwards: Vec<ForwardArg>,
    },
    /// Without a host, starts a tunnel for the highlighted row.
    TunnelStart {
        host: Option<String>,
        forwards: Vec<ForwardArg>,
    },
    TunnelList,
    TunnelStop {
        id: String,
    },
    Set {
        option: String,
        value: Option<String>,
    },
    RootAdd {
        path: String,
    },
    RootRemove {
        path: String,
    },
}

/// `:` commands with no key binding.
const EXTRA_COMMANDS: &[&str] = &[
    "ssh",
    "root add",
    "root remove",
    "set startmode=",
    "set autoinfo=",
    "set bind=",
    "set clipboard_clear=",
    "set password_confirm=",
];

fn is_set_option(option: &str) -> bool {
    EXTRA_COMMANDS
        .iter()
        .filter_map(|c| c.strip_prefix("set ")?.strip_suffix('='))
        .any(|o| o == option)
}

/// Command names (and `set` options) starting with `prefix`, sorted.
pub fn complete_command(prefix: &str) -> Vec<&'static str> {
    let prefix = prefix.trim_start_matches([':', ' ']);
    let mut out: Vec<&'static str> = BINDINGS
        .iter()
        .filter_map(|b| b.command)
        .chain(EXTRA_COMMANDS.iter().copied())
        .filter(|c| c.starts_with(prefix))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Parses one `REMOTE[=LOCAL]` argument.
pub fn parse_forward(arg: &str) -> Result<ForwardArg, String> {
    let bad = || format!("invalid forward `{arg}`; expected REMOTE[=LOCAL]");
    let port = |s: &str| s.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(bad);
    match arg.split_once('=') {
        Some((remote, local)) => Ok(ForwardArg {
            remote: port(remote)?,
            local: Some(port(local)?),
        }),
        None => Ok(ForwardArg {
            remote: port(arg)?,
            local: None,
        }),
    }
}

fn parse_forwards(args: &[&str]) -> Result<Vec<ForwardArg>, String> {
    args.iter().map(|a| parse_forward(a)).collect()
}

pub fn parse_command(line: &str) -> Result<Command, String> {
    let line = line.trim().trim_start_matches(':').trim();
    let mut words = line.split_whitespace();
    let name = words.next().ok_or("empty command")?;
    let args: Vec<&str> = words.collect();
    let no_args = |c: Command| {
        if args.is_empty() {
            Ok(c)
        } else {
            Err(format!("`{name}` takes no arguments"))
        }
    };
    match name {
        "q" => no_args(Command::Quit),
        "w" => no_args(Command::Write),
        "wq" => no_args(Command::WriteQuit),
        "help" => no_args(Command::Help),
        "ssh" => match args.split_first() {
            Some((host, forwards)) => Ok(Command::Ssh {
                host: (*host).to_string(),
                forwards: parse_forwards(forwards)?,
            }),
            None => Err("usage: ssh <host> [REMOTE[=LOCAL]…]".into()),
        },
        "tunnel" => match args.split_first() {
            Some((&"start", rest)) => Ok(match rest.split_first() {
                Some((host, forwards)) => Command::TunnelStart {
                    host: Some((*host).to_string()),
                    forwards: parse_forwards(forwards)?,
                },
                None => Command::TunnelStart {
                    host: None,
                    forwards: Vec::new(),
                },
            }),
            Some((&"list", [])) => Ok(Command::TunnelList),
            Some((&"stop", [id])) => Ok(Command::TunnelStop {
                id: (*id).to_string(),
            }),
            _ => Err("usage: tunnel start [host [REMOTE[=LOCAL]…]] | list | stop <id>".into()),
        },
        "set" => {
            let [arg] = args[..] else {
                return Err("usage: set <option>[=value]".into());
            };
            let (option, value) = match arg.split_once('=') {
                Some((o, v)) => (o, Some(v.to_string())),
                None => (arg, None),
            };
            if !is_set_option(option) {
                return Err(format!("unknown option `{option}`"));
            }
            Ok(Command::Set {
                option: option.to_string(),
                value,
            })
        }
        "root" => {
            // Paths may contain spaces, so take the remainder verbatim.
            let rest = line.strip_prefix("root").unwrap_or_default().trim_start();
            let usage = || "usage: root add|remove <path>".to_string();
            let (sub, path) = rest.split_once(char::is_whitespace).ok_or_else(usage)?;
            let path = path.trim();
            if path.is_empty() {
                return Err(usage());
            }
            match sub {
                "add" => Ok(Command::RootAdd { path: path.into() }),
                "remove" => Ok(Command::RootRemove { path: path.into() }),
                _ => Err(usage()),
            }
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Availability = Availability {
        has_password: false,
        is_pair: false,
        tunnel_selected: false,
        row_selected: false,
    };
    const ROW: Availability = Availability {
        row_selected: true,
        ..NONE
    };

    fn feed_seq(
        km: &mut Keymap,
        notation: &str,
        mode: InputMode,
        screen: Screen,
        avail: &Availability,
    ) -> Step {
        let mut last = Step::Unmapped;
        for key in parse_keys(notation).unwrap() {
            last = km.feed(key, mode, screen, avail);
        }
        last
    }

    fn run(notation: &str, mode: InputMode, screen: Screen) -> Step {
        feed_seq(&mut Keymap::new(), notation, mode, screen, &ROW)
    }

    fn act(action: Action) -> Step {
        Step::Action {
            action,
            count: 1,
            when_input_empty: false,
        }
    }

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries.iter().map(|e| e.key_label.clone()).collect()
    }

    #[test]
    fn parses_notation() {
        assert_eq!(
            parse_keys("space t t").unwrap(),
            vec![Key::Char(' '), Key::Char('t'), Key::Char('t')]
        );
        assert_eq!(parse_keys("gg").unwrap(), vec![Key::Char('g'); 2]);
        assert_eq!(parse_keys("^d").unwrap(), vec![Key::Ctrl('d')]);
        assert_eq!(parse_keys("F1").unwrap(), vec![Key::F(1)]);
        assert_eq!(parse_keys("<esc>").unwrap(), vec![Key::Esc]);
        assert_eq!(parse_keys("?").unwrap(), vec![Key::Char('?')]);
        assert!(parse_keys("<nope>").is_err());
        assert_eq!(parse_keys("").unwrap(), vec![]);
    }

    #[test]
    fn labels_round_trip() {
        for key in [
            Key::Char(' '),
            Key::Char('x'),
            Key::Ctrl('r'),
            Key::F(1),
            Key::Enter,
            Key::Esc,
            Key::Tab,
            Key::Backspace,
            Key::PageDown,
        ] {
            let label = key.label();
            let token = if label.len() > 1 && !label.starts_with(['^', 'F']) {
                format!("<{label}>")
            } else {
                label
            };
            assert_eq!(parse_keys(&token).unwrap(), vec![key]);
        }
    }

    #[test]
    fn normal_finder_sequences() {
        use Action::*;
        let n = InputMode::Normal;
        let f = Screen::Finder;
        assert_eq!(run("j", n, f), act(Down));
        assert_eq!(run("gg", n, f), act(Top));
        assert_eq!(run("ge", n, f), act(Bottom));
        assert_eq!(run("^d", n, f), act(HalfPageDown));
        assert_eq!(run("gt", n, f), act(Goto(GotoTarget::Tunnels)));
        assert_eq!(run("space f w", n, f), act(Filter(FilterTarget::Work)));
        assert_eq!(run("space t t", n, f), act(TunnelStart));
        assert_eq!(run("space t x", n, f), act(TunnelStopAll));
        assert_eq!(run("yy", n, f), act(Copy(CopyTarget::SshCommand)));
        assert_eq!(run("cc", n, f), act(EditHostForm));
        assert_eq!(run("ce", n, f), act(EditHostInEditor));
        assert_eq!(run("dd", n, f), act(DeleteHost));
        assert_eq!(run("o", n, f), act(NewHost));
        assert_eq!(run("u", n, f), act(Undo));
        assert_eq!(run("^r", n, f), act(Redo));
        assert_eq!(run(":", n, f), act(CommandLine));
        assert_eq!(run("V", n, f), act(EnterSelect));
        assert_eq!(run("q", n, f), act(Quit));
        assert_eq!(run("?", n, f), act(Help));
        assert_eq!(run("<tab>", n, f), act(OpenPorts));
        assert_eq!(run("<enter>", n, f), act(Connect));
        for k in ["i", "a", "/"] {
            assert_eq!(run(k, n, f), act(EnterInsert));
        }
    }

    #[test]
    fn esc_in_normal_never_quits() {
        assert_eq!(
            run("<esc>", InputMode::Normal, Screen::Finder),
            Step::Unmapped
        );
    }

    #[test]
    fn insert_mode_types_printables_and_maps_special_keys() {
        use Action::*;
        let i = InputMode::Insert;
        let f = Screen::Finder;
        assert_eq!(run("j", i, f), Step::Unmapped);
        assert_eq!(run("space", i, f), Step::Unmapped);
        assert_eq!(run("<esc>", i, f), act(EnterNormal));
        assert_eq!(run("<enter>", i, f), act(Connect));
        assert_eq!(run("<tab>", i, f), act(OpenPorts));
        assert_eq!(run("<down>", i, f), act(Down));
        assert_eq!(run("F1", i, f), act(Help));
        assert_eq!(run("^c", i, f), act(Quit));
        assert_eq!(
            run("?", i, f),
            Step::Action {
                action: Help,
                count: 1,
                when_input_empty: true
            }
        );
    }

    #[test]
    fn ctrl_c_and_f1_work_everywhere() {
        for &screen in ALL_SCREENS {
            for mode in [InputMode::Insert, InputMode::Normal, InputMode::Select] {
                assert_eq!(run("^c", mode, screen), act(Action::Quit), "{screen:?}");
                assert_eq!(run("F1", mode, screen), act(Action::Help), "{screen:?}");
            }
        }
    }

    #[test]
    fn select_mode_bindings() {
        use Action::*;
        let s = InputMode::Select;
        let f = Screen::Finder;
        assert_eq!(run("y", s, f), act(Copy(CopyTarget::SshCommand)));
        assert_eq!(run("d", s, f), act(DeleteHost));
        assert_eq!(run("space t t", s, f), act(TunnelStart));
        assert_eq!(run("<esc>", s, f), act(EnterNormal));
        assert_eq!(run("j", s, f), act(Down));
    }

    #[test]
    fn screen_specific_bindings() {
        use Action::*;
        let n = InputMode::Normal;
        assert_eq!(run("x", n, Screen::Ports), act(ToggleRow));
        assert_eq!(run("o", n, Screen::Ports), act(AddRow));
        assert_eq!(run("dd", n, Screen::Ports), act(DeleteRow));
        assert_eq!(run("s", n, Screen::Ports), act(SaveRow));
        assert_eq!(run("space t t", n, Screen::Ports), act(TunnelStart));
        assert_eq!(run("x", n, Screen::Finder), Step::Unmapped);
        assert_eq!(run("dd", n, Screen::Tunnels), act(TunnelStop));
        assert_eq!(run("r", n, Screen::Tunnels), act(TunnelRestart));
        assert_eq!(run("space t l", n, Screen::Tunnels), act(TunnelList));
        assert_eq!(run("ZZ", n, Screen::HostForm), act(WriteQuit));
        assert_eq!(run("j", n, Screen::HostForm), act(NextField));
        assert_eq!(run("k", n, Screen::HostForm), act(PrevField));
        assert_eq!(run("y", n, Screen::DeleteConfirm), act(Confirm));
        assert_eq!(run("n", n, Screen::DeleteConfirm), act(Back));
        assert_eq!(run("r", n, Screen::Failure), act(Retry));
        assert_eq!(run("q", n, Screen::Help), act(Back));
        assert_eq!(
            run("<enter>", InputMode::Insert, Screen::CommandLine),
            act(Submit)
        );
        assert_eq!(
            run("<tab>", InputMode::Insert, Screen::CommandLine),
            act(Complete)
        );
    }

    #[test]
    fn tunnel_id_copy_is_bound_on_the_finder_list_and_progress_screens() {
        let n = InputMode::Normal;
        let tunnel = Availability {
            tunnel_selected: true,
            ..NONE
        };
        for screen in [Screen::Finder, Screen::Tunnels, Screen::TunnelProgress] {
            let mut km = Keymap::new();
            assert_eq!(
                feed_seq(&mut km, "yi", n, screen, &tunnel),
                act(Action::Copy(CopyTarget::TunnelId)),
                "{screen:?}"
            );
            assert!(matches!(
                run("yi", n, screen),
                Step::Unavailable {
                    reason: "no tunnel selected",
                    ..
                }
            ));
        }
        assert_eq!(run("yi", n, Screen::Ports), Step::Unmapped);
    }

    #[test]
    fn prefixes_show_which_key_menus() {
        let n = InputMode::Normal;
        let f = Screen::Finder;
        let mut km = Keymap::new();

        let Step::Pending { title, entries } = km.feed(Key::Char(' '), n, f, &ROW) else {
            panic!("space should be pending");
        };
        assert_eq!(title, "Space");
        assert_eq!(labels(&entries), ["f", "t"]);
        assert!(entries.iter().all(|e| e.is_submenu));
        assert_eq!(entries[1].description, "Tunnel…");
        assert_eq!(km.pending_keys(), "space");

        let Step::Pending { title, entries } = km.feed(Key::Char('t'), n, f, &ROW) else {
            panic!()
        };
        assert_eq!(title, "Tunnel");
        assert_eq!(labels(&entries), ["t", "l", "x"]);
        assert!(entries.iter().all(|e| !e.is_submenu));
        assert_eq!(km.pending_keys(), "space t");

        for (prefix, title, keys) in [
            ("g", "Goto", vec!["g", "e", "r", "p", "w", "t"]),
            ("y", "Yank", vec!["y", "p", "w", "h", "i"]),
            ("d", "Delete", vec!["d"]),
            ("c", "Change", vec!["c", "e"]),
        ] {
            let mut km = Keymap::new();
            let Step::Pending { title: t, entries } = feed_seq(&mut km, prefix, n, f, &ROW) else {
                panic!("{prefix} should be pending");
            };
            assert_eq!(t, title);
            let mut got = labels(&entries);
            let mut want: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
            got.sort();
            want.sort();
            assert_eq!(got, want, "{prefix}");
        }
    }

    #[test]
    fn counts_apply_to_movement_only() {
        let n = InputMode::Normal;
        let f = Screen::Finder;
        let mut km = Keymap::new();
        assert!(matches!(
            km.feed(Key::Char('5'), n, f, &ROW),
            Step::Pending { title: "", .. }
        ));
        assert_eq!(km.pending_keys(), "5");
        assert_eq!(
            km.feed(Key::Char('j'), n, f, &ROW),
            Step::Action {
                action: Action::Down,
                count: 5,
                when_input_empty: false
            }
        );
        assert!(!km.is_pending());

        assert_eq!(
            feed_seq(&mut km, "12k", n, f, &ROW),
            Step::Action {
                action: Action::Up,
                count: 12,
                when_input_empty: false
            }
        );
        assert_eq!(
            feed_seq(&mut km, "3gg", n, f, &ROW),
            Step::Action {
                action: Action::Top,
                count: 3,
                when_input_empty: false
            }
        );
        // Destructive and other actions ignore the count.
        assert_eq!(
            feed_seq(&mut km, "3dd", n, f, &ROW),
            act(Action::DeleteHost)
        );
        assert_eq!(feed_seq(&mut km, "5q", n, f, &ROW), act(Action::Quit));
        // A leading 0 is not a count.
        assert_eq!(feed_seq(&mut km, "0", n, f, &ROW), Step::Unmapped);
    }

    #[test]
    fn insert_mode_has_no_counts() {
        assert_eq!(run("5", InputMode::Insert, Screen::Finder), Step::Unmapped);
    }

    #[test]
    fn esc_and_backspace_go_up_one_level() {
        let n = InputMode::Normal;
        let f = Screen::Finder;
        for key in [Key::Esc, Key::Backspace] {
            let mut km = Keymap::new();
            feed_seq(&mut km, "space t", n, f, &ROW);
            let Step::Pending { title, .. } = km.feed(key, n, f, &ROW) else {
                panic!("expected parent menu");
            };
            assert_eq!(title, "Space");
            assert_eq!(km.pending_keys(), "space");
            assert_eq!(km.feed(key, n, f, &ROW), Step::Cancelled);
            assert!(!km.is_pending());
        }
    }

    #[test]
    fn esc_cancels_pending_count() {
        let mut km = Keymap::new();
        feed_seq(&mut km, "5", InputMode::Normal, Screen::Finder, &ROW);
        assert_eq!(
            km.feed(Key::Esc, InputMode::Normal, Screen::Finder, &ROW),
            Step::Cancelled
        );
        assert_eq!(km.pending_keys(), "");
    }

    #[test]
    fn unmapped_key_clears_pending() {
        let mut km = Keymap::new();
        let n = InputMode::Normal;
        let f = Screen::Finder;
        assert_eq!(feed_seq(&mut km, "gz", n, f, &ROW), Step::Unmapped);
        assert!(!km.is_pending());
        assert_eq!(feed_seq(&mut km, "j", n, f, &ROW), act(Action::Down));
    }

    #[test]
    fn availability_dims_entries_and_refuses_actions() {
        let n = InputMode::Normal;
        let f = Screen::Finder;
        let y = [Key::Char('y')];
        let find = |avail: &Availability, key: &str| {
            entries(&y, n, f, avail)
                .into_iter()
                .find(|e| e.key_label == key)
                .unwrap()
        };

        let password = Availability {
            has_password: true,
            row_selected: true,
            ..NONE
        };
        assert!(find(&password, "p").available);
        assert!(find(&password, "w").available);

        let pair = Availability {
            is_pair: true,
            ..password
        };
        assert!(!find(&pair, "p").available);
        assert!(find(&pair, "w").available);

        assert!(!find(&ROW, "p").available);
        assert!(!find(&ROW, "w").available);
        assert!(find(&ROW, "y").available);

        assert!(!find(&ROW, "i").available);
        let tunnel = Availability {
            tunnel_selected: true,
            ..ROW
        };
        assert!(find(&tunnel, "i").available);

        assert!(!find(&NONE, "y").available);

        let mut km = Keymap::new();
        assert_eq!(
            feed_seq(&mut km, "yp", n, f, &pair),
            Step::Unavailable {
                action: Action::Copy(CopyTarget::SshpassCommand),
                reason: "not available for Pairs"
            }
        );
        assert_eq!(
            feed_seq(&mut km, "yp", n, f, &ROW),
            Step::Unavailable {
                action: Action::Copy(CopyTarget::SshpassCommand),
                reason: "no stored password"
            }
        );
        assert_eq!(
            feed_seq(&mut km, "yi", n, f, &ROW),
            Step::Unavailable {
                action: Action::Copy(CopyTarget::TunnelId),
                reason: "no tunnel selected"
            }
        );
        assert_eq!(
            feed_seq(&mut km, "yp", n, f, &password),
            act(Action::Copy(CopyTarget::SshpassCommand))
        );
        assert!(matches!(
            feed_seq(&mut km, "<enter>", n, f, &NONE),
            Step::Unavailable { .. }
        ));
    }

    #[test]
    fn submenu_availability_follows_children() {
        let space = [Key::Char(' ')];
        let menu = entries(&space, InputMode::Normal, Screen::Finder, &NONE);
        let tunnel = menu.iter().find(|e| e.key_label == "t").unwrap();
        assert!(tunnel.is_submenu && tunnel.available); // `l` needs no row
        let y_menu = entries(&[Key::Char('y')], InputMode::Normal, Screen::Finder, &NONE);
        assert!(y_menu.iter().all(|e| !e.available));
    }

    #[test]
    fn help_lists_current_screen_section_first() {
        let sections = help(Screen::Ports, InputMode::Normal, &ROW);
        assert_eq!(sections[0].0, Section::Ports);
        let sections = help(Screen::Tunnels, InputMode::Normal, &ROW);
        assert_eq!(sections[0].0, Section::Tunnels);
        let sections = help(Screen::Finder, InputMode::Normal, &ROW);
        assert_eq!(sections[0].0, Section::Connect);
        let all: Vec<_> = sections.iter().flat_map(|(_, rows)| rows).collect();
        assert!(all.iter().any(|r| r.keys == "space t t"));
        assert!(all.iter().any(|r| r.keys == "y p" && !r.available));
    }

    #[test]
    fn help_shows_command_only_rows() {
        let rows: Vec<_> = help(Screen::HostForm, InputMode::Normal, &ROW)
            .into_iter()
            .flat_map(|(_, r)| r)
            .collect();
        let w = rows.iter().find(|r| r.command == Some("w")).unwrap();
        assert_eq!(w.keys, ":w");
        let wq = rows.iter().find(|r| r.command == Some("wq")).unwrap();
        assert_eq!(wq.keys, "Z Z");
    }

    #[test]
    fn registry_has_no_conflicting_sequences() {
        let reg = registry();
        for (i, a) in reg.iter().enumerate() {
            for b in &reg[i + 1..] {
                if a.keys.is_empty() || b.keys.is_empty() {
                    continue;
                }
                let overlap = a.binding.modes.iter().any(|m| b.binding.modes.contains(m))
                    && a.binding
                        .screens
                        .iter()
                        .any(|s| b.binding.screens.contains(s));
                if overlap {
                    assert!(
                        !a.keys.starts_with(&b.keys) && !b.keys.starts_with(&a.keys),
                        "conflict: `{}` vs `{}`",
                        a.binding.keys,
                        b.binding.keys
                    );
                }
            }
        }
    }

    #[test]
    fn registry_is_well_formed() {
        for p in registry() {
            let b = p.binding;
            assert!(!b.modes.is_empty() && !b.screens.is_empty(), "{}", b.keys);
            if b.keys.is_empty() {
                assert!(b.command.is_some(), "keyless binding needs a command");
            }
            if b.modes.contains(&InputMode::Insert) {
                assert!(
                    p.keys.len() == 1,
                    "insert binding `{}` must be one key",
                    b.keys
                );
            }
            if b.when_input_empty {
                assert!(b.modes == INSERT && matches!(p.keys[0], Key::Char(_)));
            }
            if b.modes == INSERT && matches!(p.keys.first(), Some(Key::Char(_))) {
                assert!(
                    b.when_input_empty,
                    "printable `{}` would eat typing",
                    b.keys
                );
            }
            for (i, _) in p.keys.iter().enumerate().skip(1) {
                assert!(
                    prefix_title(&p.keys[..i]).is_some(),
                    "prefix of `{}` has no title",
                    b.keys
                );
            }
        }
    }

    #[test]
    fn every_binding_is_discoverable() {
        for p in registry() {
            let b = p.binding;
            for &mode in b.modes {
                for &screen in b.screens {
                    let rows: Vec<_> = help(screen, mode, &NONE)
                        .into_iter()
                        .flat_map(|(_, r)| r)
                        .collect();
                    assert!(
                        rows.iter()
                            .any(|r| r.description == b.description && r.command == b.command),
                        "`{}` missing from help({screen:?}, {mode:?})",
                        b.keys
                    );
                    // Multi-key sequences must also be walkable via which-key.
                    for i in 0..p.keys.len() {
                        let menu = entries(&p.keys[..i], mode, screen, &NONE);
                        assert!(
                            menu.iter().any(|e| e.key_label == p.keys[i].label()),
                            "`{}` unreachable at depth {i} on {screen:?}/{mode:?}",
                            b.keys
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_action_variant_with_a_key_exists_in_registry() {
        use Action::*;
        let must_have = [
            Up,
            Down,
            Top,
            Bottom,
            HalfPageDown,
            HalfPageUp,
            EnterInsert,
            EnterNormal,
            EnterSelect,
            Connect,
            OpenPorts,
            NewHost,
            EditHostForm,
            EditHostInEditor,
            DeleteHost,
            Undo,
            Redo,
            CommandLine,
            Help,
            Quit,
            TunnelStart,
            TunnelList,
            TunnelStop,
            TunnelStopAll,
            TunnelRestart,
            ToggleRow,
            AddRow,
            DeleteRow,
            SaveRow,
            Write,
            WriteQuit,
            NextField,
            PrevField,
            Back,
            Confirm,
            Submit,
            Complete,
            Retry,
            ShowLog,
            Copy(CopyTarget::SshCommand),
            Copy(CopyTarget::SshpassCommand),
            Copy(CopyTarget::Password),
            Copy(CopyTarget::HostName),
            Copy(CopyTarget::TunnelId),
            Copy(CopyTarget::VerboseSshCommand),
            Goto(GotoTarget::Recent),
            Goto(GotoTarget::Personal),
            Goto(GotoTarget::Work),
            Goto(GotoTarget::Tunnels),
            Filter(FilterTarget::Personal),
            Filter(FilterTarget::Work),
            Filter(FilterTarget::Recent),
        ];
        for action in must_have {
            assert!(
                BINDINGS.iter().any(|b| b.action == action),
                "{action:?} has no binding"
            );
        }
    }

    #[test]
    fn completes_commands() {
        assert_eq!(complete_command(""), {
            let mut all = complete_command("");
            all.sort_unstable();
            all
        });
        assert!(complete_command("").len() >= 10);
        assert_eq!(complete_command("q"), ["q"]);
        assert_eq!(complete_command("w"), ["w", "wq"]);
        assert_eq!(complete_command(":w"), ["w", "wq"]);
        assert_eq!(complete_command("ss"), ["ssh"]);
        assert_eq!(
            complete_command("tunnel "),
            ["tunnel list", "tunnel start", "tunnel stop"]
        );
        assert_eq!(
            complete_command("tunnel s"),
            ["tunnel start", "tunnel stop"]
        );
        assert_eq!(complete_command("root "), ["root add", "root remove"]);
        assert_eq!(complete_command("set a"), ["set autoinfo="]);
        assert_eq!(complete_command("he"), ["help"]);
        assert!(complete_command("zzz").is_empty());
    }

    #[test]
    fn registry_commands_parse() {
        assert_eq!(parse_command("q"), Ok(Command::Quit));
        assert_eq!(parse_command("w"), Ok(Command::Write));
        assert_eq!(parse_command("wq"), Ok(Command::WriteQuit));
        assert_eq!(parse_command("help"), Ok(Command::Help));
        assert_eq!(parse_command("tunnel list"), Ok(Command::TunnelList));
        assert_eq!(
            parse_command("tunnel start"),
            Ok(Command::TunnelStart {
                host: None,
                forwards: vec![]
            })
        );
        for c in complete_command("") {
            let name = c.split_whitespace().next().unwrap();
            assert!(
                !matches!(parse_command(name), Err(e) if e.starts_with("unknown command")),
                "`{c}` is completable but not parseable"
            );
        }
    }

    #[test]
    fn parses_ssh_and_tunnel_commands() {
        assert_eq!(
            parse_command(":ssh db 5432 80=3000"),
            Ok(Command::Ssh {
                host: "db".into(),
                forwards: vec![
                    ForwardArg {
                        remote: 5432,
                        local: None
                    },
                    ForwardArg {
                        remote: 80,
                        local: Some(3000)
                    },
                ],
            })
        );
        assert!(parse_command("ssh").is_err());
        assert!(parse_command("ssh db 0").is_err());
        assert!(parse_command("ssh db abc").is_err());
        assert!(parse_command("ssh db 80=").is_err());
        assert_eq!(
            parse_command("tunnel start db 80=3000"),
            Ok(Command::TunnelStart {
                host: Some("db".into()),
                forwards: vec![ForwardArg {
                    remote: 80,
                    local: Some(3000)
                }],
            })
        );
        assert_eq!(
            parse_command("tunnel stop abc123"),
            Ok(Command::TunnelStop {
                id: "abc123".into()
            })
        );
        assert!(parse_command("tunnel stop").is_err());
        assert!(parse_command("tunnel").is_err());
        assert_eq!(
            ForwardArg {
                remote: 80,
                local: Some(3000)
            }
            .to_string(),
            "80=3000"
        );
    }

    #[test]
    fn parses_set_and_root_commands() {
        assert_eq!(
            parse_command("set autoinfo=false"),
            Ok(Command::Set {
                option: "autoinfo".into(),
                value: Some("false".into())
            })
        );
        assert_eq!(
            parse_command("set clipboard_clear=off"),
            Ok(Command::Set {
                option: "clipboard_clear".into(),
                value: Some("off".into())
            })
        );
        assert_eq!(
            parse_command("set bind"),
            Ok(Command::Set {
                option: "bind".into(),
                value: None
            })
        );
        assert!(parse_command("set theme=dark").is_err());
        assert!(parse_command("set").is_err());
        assert_eq!(
            parse_command("root add ~/my configs/ssh"),
            Ok(Command::RootAdd {
                path: "~/my configs/ssh".into()
            })
        );
        assert_eq!(
            parse_command("root remove /tmp/x"),
            Ok(Command::RootRemove {
                path: "/tmp/x".into()
            })
        );
        assert!(parse_command("root add").is_err());
        assert!(parse_command("root nuke /x").is_err());
    }

    #[test]
    fn rejects_bad_commands() {
        assert!(parse_command("").is_err());
        assert!(parse_command(":").is_err());
        assert!(parse_command("frobnicate").is_err());
        assert!(parse_command("q now").is_err());
    }
}
