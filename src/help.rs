use super::TunnelRoute;
use std::fmt::Write as _;

pub const ROOT_USAGE: &str = "Usage: sshx [GLOBAL OPTIONS] [COMMAND]";
pub const SETUP_USAGE: &str = "Usage: sshx setup [--personal PATH] [--work PATH] [--scope SCOPE --config PATH] [--project NAME]";
pub const DOCTOR_USAGE: &str = "Usage: sshx doctor [--fix-permissions] [--format human|json|yaml]";
pub const CONNECT_USAGE: &str = "Usage: sshx connect [SELECTOR] [OPTIONS]";
pub const TUI_USAGE: &str = "Usage: sshx tui [connect [SELECTOR] | host show [SELECTOR] | host create | tunnel status|stop [ID]] [OPTIONS]";
pub const HOST_USAGE: &str = "Usage: sshx [--config PATH] host COMMAND [OPTIONS]";
pub const PAIR_USAGE: &str = "Usage: sshx pair COMMAND [OPTIONS]";
pub const TUNNEL_USAGE: &str = "Usage: sshx tunnel [HOST] [OPTIONS]\n       sshx tunnel start [HOST] [OPTIONS]\n       sshx tunnel direct|paired start [HOST] [OPTIONS]\n       sshx tunnel list [OPTIONS]\n       sshx tunnel status|stop|restart ID [OPTIONS]\n       sshx tunnel direct|paired list [OPTIONS]\n       sshx tunnel direct|paired status|stop|restart ID [OPTIONS]";
const TUNNEL_DIRECT_USAGE: &str = "Usage: sshx tunnel direct start [HOST] [OPTIONS] | sshx tunnel direct list [OPTIONS] | sshx tunnel direct status|stop|restart ID [OPTIONS]";
const TUNNEL_PAIRED_USAGE: &str = "Usage: sshx tunnel paired start [HOST] [OPTIONS] | sshx tunnel paired list [OPTIONS] | sshx tunnel paired status|stop|restart ID [OPTIONS]";
const TUNNEL_AUTO_START_USAGE: &str = "Usage: sshx tunnel start [HOST] [OPTIONS]";
const TUNNEL_DIRECT_START_USAGE: &str =
    "Usage: sshx tunnel direct start [HOST] [OPTIONS]";
const TUNNEL_PAIRED_START_USAGE: &str =
    "Usage: sshx tunnel paired start [HOST] [OPTIONS]";
const TUNNEL_OPTIONS: &str = "--forward REMOTE[=LOCAL] (repeatable)    Forward declared service.\n--bind    Select declared services and local ports.\n-L SPEC --local-forward SPEC    Add direct local forwarding; combine with --forward on direct routes.\n--allow-bind    Permit specific non-loopback listeners; forbidden bind addresses remain rejected.\n-R SPEC --remote-forward SPEC    Open server-side listener and forward connections to destination on your side; server bind availability is checked by OpenSSH.\n-D SPEC --dynamic-forward SPEC    Open local SOCKS proxy; configure applications to use its bind address and port.\n--password-fd FD --gateway-password-fd FD --vm-password-fd FD    Supply passwords through inherited descriptors.\n--config PATH    Select SSH config roots for start/restart. List/status/stop use the runtime registry under sshx home.\n--format human|json|yaml    Select output.\n--no-input    Disable interactive prompts.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HelpPage {
    Root,
    Setup,
    Doctor,
    Connect,
    Tui,
    Host,
    HostList,
    HostShow,
    HostCreate,
    HostUpdate,
    HostRename,
    HostDelete,
    Pair,
    PairSetup,
    PairList,
    PairValidate,
    PairRecover,
    Tunnel,
    TunnelRoute { route: TunnelRoute },
    TunnelStart { route: Option<TunnelRoute> },
    TunnelLifecycle {
        operation: LifecycleOperation,
        route: Option<TunnelRoute>,
    },
}

impl HelpPage {
    fn usage(self) -> &'static str {
        match self {
            Self::Root => ROOT_USAGE,
            Self::Setup => SETUP_USAGE,
            Self::Doctor => DOCTOR_USAGE,
            Self::Connect => CONNECT_USAGE,
            Self::Tui => TUI_USAGE,
            Self::Host
            | Self::HostList
            | Self::HostShow
            | Self::HostCreate
            | Self::HostUpdate
            | Self::HostRename
            | Self::HostDelete => HOST_USAGE,
            Self::Pair
            | Self::PairSetup
            | Self::PairList
            | Self::PairValidate
            | Self::PairRecover => PAIR_USAGE,
            Self::Tunnel => TUNNEL_USAGE,
            Self::TunnelRoute {
                route: TunnelRoute::Direct,
            }
            | Self::TunnelLifecycle {
                route: Some(TunnelRoute::Direct),
                ..
            } => TUNNEL_DIRECT_USAGE,
            Self::TunnelRoute {
                route: TunnelRoute::Paired,
            }
            | Self::TunnelLifecycle {
                route: Some(TunnelRoute::Paired),
                ..
            } => TUNNEL_PAIRED_USAGE,
            Self::TunnelStart {
                route: Some(TunnelRoute::Direct),
            } => TUNNEL_DIRECT_START_USAGE,
            Self::TunnelStart {
                route: Some(TunnelRoute::Paired),
            } => TUNNEL_PAIRED_START_USAGE,
            Self::TunnelStart { route: None } => TUNNEL_AUTO_START_USAGE,
            Self::TunnelLifecycle { route: None, .. } => TUNNEL_USAGE,
        }
    }
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecycleOperation {
    List,
    Status,
    Stop,
    Restart,
}


struct LifecycleMetadata {
    arguments: &'static str,
    options: &'static str,
    purpose: &'static str,
    examples: &'static str,
    exits: &'static str,
    detail: &'static str,
    is_list: bool,
}

impl LifecycleOperation {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "list" => Some(Self::List),
            "status" => Some(Self::Status),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            _ => None,
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Status => "status",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }

    fn metadata(self) -> LifecycleMetadata {
        match self {
            Self::List => LifecycleMetadata {
                arguments: "None. Lists every registered tunnel, or route-matching tunnels under direct and paired commands.",
                options: "--config PATH    Accepted for common CLI compatibility; this lifecycle command reads the runtime registry under sshx home and does not select discovery roots.\n--format human|json|yaml    Render lifecycle output.",
                purpose: "List every registered standalone tunnel.",
                examples: "sshx tunnel list --format json",
                exits: "Exit 0 after registry rendering. REGISTRY_UNSAFE and parse errors exit 2. Help exits 0.",
                detail: "The locked registry is the source of truth. Human output includes tunnel ID, state, master responsive, and listener ready. JSON and YAML preserve stable fields including master_status and listener_status; no application status is inferred.",
                is_list: true,
            },
            Self::Status => LifecycleMetadata {
                arguments: "ID    Exact persisted tunnel ID returned by start or list. With usable interactive terminals, omitting ID opens the registered Tunnel list.",
                options: "--config PATH    Accepted for common CLI compatibility; this lifecycle command reads the runtime registry under sshx home and does not select discovery roots.\n--format human|json|yaml    Render lifecycle output.",
                purpose: "Show one registered tunnel's master responsiveness and listener readiness.",
                examples: "sshx tunnel status dt-1",
                exits: "Exit 0 after status rendering. TUNNEL_NOT_FOUND, REGISTRY_UNSAFE, and parse errors exit 2. Help exits 0.",
                detail: "The locked registry and control socket prove ownership. Output includes tunnel ID, state, kind, master responsive, listener ready, selected host, source, and forwards. No application status is inferred.",
                is_list: false,
            },
            Self::Stop => LifecycleMetadata {
                arguments: "ID    Exact persisted tunnel ID returned by start or list. With usable interactive terminals, omitting ID opens the registered Tunnel list.",
                options: "--config PATH    Accepted for common CLI compatibility; this lifecycle command reads the runtime registry under sshx home and does not select discovery roots.\n--format human|json|yaml    Render lifecycle output.",
                purpose: "Stop one registered tunnel using its ownership controls.",
                examples: "sshx tunnel stop dt-1",
                exits: "Exit 0 after the owned tunnel stops. TUNNEL_NOT_FOUND, TUNNEL_STOP_FAILED, REGISTRY_UNSAFE, and parse errors exit 2. Help exits 0.",
                detail: "The command validates the locked registry and control socket, then stops the owned runtime. Paired tunnels stop VM before gateway. It never treats an arbitrary PID as ownership.",
                is_list: false,
            },
            Self::Restart => LifecycleMetadata {
                arguments: "ID    Exact persisted tunnel ID returned by start or list; it is not a process ID.",
                options: "--config PATH    Select the discovery root when restart re-resolves the current HostEntry or Pair identity.\n--format human|json|yaml    Render lifecycle output.\n--password-fd FD    Supply a direct or VM password through an inherited descriptor.\n--gateway-password-fd FD    Supply a Pair gateway password through an inherited descriptor.\n--vm-password-fd FD    Supply a Pair VM password through an inherited descriptor.",
                purpose: "Restart one registered tunnel with its persisted forwarding request.",
                examples: "sshx tunnel restart dt-1 --password-fd 3",
                exits: "Exit 0 after the owned tunnel restarts. TUNNEL_NOT_FOUND, CONFIG_CHANGED, PAIR_BROKEN, REGISTRY_UNSAFE, and parse errors exit 2. Help exits 0.",
                detail: "The command validates the locked registry and current HostEntry or Pair identity, then preserves forwarding configuration. CONFIG_CHANGED and PAIR_BROKEN prevent a stale restart.",
                is_list: false,
            },
        }
    }
}

pub(crate) fn resolve(path: &[String]) -> Result<HelpPage, String> {
    let path = path.iter().map(String::as_str).collect::<Vec<_>>();
    resolve_refs(&path).ok_or_else(|| {
        format!(
            "unknown help path `{}`\n{}",
            path.join(" "),
            usage(path.as_slice())
        )
    })
}

fn resolve_refs(path: &[&str]) -> Option<HelpPage> {
    match path {
        [] => Some(HelpPage::Root),
        ["setup", ..] => Some(HelpPage::Setup),
        ["doctor", ..] => Some(HelpPage::Doctor),
        ["connect", ..] => Some(HelpPage::Connect),
        ["tui", ..] => Some(HelpPage::Tui),
        ["host"] => Some(HelpPage::Host),
        ["host", "list", ..] => Some(HelpPage::HostList),
        ["host", "show", ..] => Some(HelpPage::HostShow),
        ["host", "create", ..] => Some(HelpPage::HostCreate),
        ["host", "update", ..] => Some(HelpPage::HostUpdate),
        ["host", "rename", ..] => Some(HelpPage::HostRename),
        ["host", "delete", ..] => Some(HelpPage::HostDelete),
        ["pair"] => Some(HelpPage::Pair),
        ["pair", "setup", ..] => Some(HelpPage::PairSetup),
        ["pair", "list", ..] => Some(HelpPage::PairList),
        ["pair", "validate", ..] => Some(HelpPage::PairValidate),
        ["pair", "recover", ..] => Some(HelpPage::PairRecover),
        ["tunnel"] => Some(HelpPage::Tunnel),
        ["tunnel", "start", ..] => Some(HelpPage::TunnelStart { route: None }),
        ["tunnel", "direct"] => Some(HelpPage::TunnelRoute {
            route: TunnelRoute::Direct,
        }),
        ["tunnel", "paired"] => Some(HelpPage::TunnelRoute {
            route: TunnelRoute::Paired,
        }),
        ["tunnel", "direct", "start", ..] => Some(HelpPage::TunnelStart {
            route: Some(TunnelRoute::Direct),
        }),
        ["tunnel", "paired", "start", ..] => Some(HelpPage::TunnelStart {
            route: Some(TunnelRoute::Paired),
        }),
        ["tunnel", "direct", operation, ..] => LifecycleOperation::parse(operation).map(
            |operation| HelpPage::TunnelLifecycle {
                operation,
                route: Some(TunnelRoute::Direct),
            },
        ),
        ["tunnel", "paired", operation, ..] => LifecycleOperation::parse(operation).map(
            |operation| HelpPage::TunnelLifecycle {
                operation,
                route: Some(TunnelRoute::Paired),
            },
        ),
        ["tunnel", operation, ..] => LifecycleOperation::parse(operation).map(|operation| {
            HelpPage::TunnelLifecycle {
                operation,
                route: None,
            }
        }),
        _ => None,
    }
}

pub fn render(path: &[String]) -> Result<String, String> {
    Ok(render_page(resolve(path)?))
}

pub(crate) fn render_page(page: HelpPage) -> String {
    match page {
        HelpPage::Root => root(),
        HelpPage::Setup => setup(),
        HelpPage::Doctor => doctor(),
        HelpPage::Tui => tui(),
        HelpPage::Connect => connect(),
        HelpPage::Host => host(),
        HelpPage::HostList => host_list(),
        HelpPage::HostShow => host_show(),
        HelpPage::HostCreate => host_create(),
        HelpPage::HostUpdate => host_update(),
        HelpPage::HostRename => host_rename(),
        HelpPage::HostDelete => host_delete(),
        HelpPage::Pair => pair(),
        HelpPage::PairSetup => pair_setup(),
        HelpPage::PairList => pair_list(),
        HelpPage::PairValidate => pair_validate(),
        HelpPage::PairRecover => pair_recover(),
        HelpPage::Tunnel => tunnel(),
        HelpPage::TunnelRoute { route } => tunnel_route(route),
        HelpPage::TunnelStart { route } => tunnel_start(route),
        HelpPage::TunnelLifecycle { operation, route } => tunnel_lifecycle(operation, route),
    }
}

pub fn usage(path: &[&str]) -> &'static str {
    (0..=path.len())
        .rev()
        .find_map(|length| resolve_refs(&path[..length]).map(HelpPage::usage))
        .unwrap_or(ROOT_USAGE)
}

#[allow(clippy::too_many_arguments)]
fn page(
    name: &str,
    purpose: &str,
    usage: &str,
    arguments: &str,
    options: &str,
    subcommands: &str,
    examples: &str,
    exits: &str,
    related: &str,
) -> String {
    let mut output = String::new();
    writeln!(output, "Name: {name}\nPurpose: {purpose}\n").unwrap();
    writeln!(output, "Usage forms:\n{usage}\n").unwrap();
    writeln!(output, "Positional arguments:\n{arguments}\n").unwrap();
    writeln!(output, "Options:\n{options}\n").unwrap();
    if !subcommands.is_empty() {
        writeln!(output, "Subcommands:\n{subcommands}\n").unwrap();
    }
    writeln!(output, "Examples:\n{examples}\n").unwrap();
    writeln!(output, "Exit behavior:\n{exits}\n").unwrap();
    writeln!(output, "Related commands:\n{related}").unwrap();
    output
}

fn root() -> String {
    page(
        "sshx",
        "Manage SSH connections, HostEntry records, Pair routes, and tunnels.",
        "sshx [GLOBAL OPTIONS] [COMMAND]\nsshx help [COMMAND PATH]\nsshx --help [COMMAND PATH]\nsshx -h [COMMAND PATH]",
        "COMMAND PATH    Optional command or command group. Bare `sshx` opens the keyboard-accessible Hosts view when stdin and stderr are terminals; otherwise it prints this help.",
        "--config PATH    Use a config root or filesystem source of truth.\n--format human|json|yaml    Select output for commands that support it; help stays plain text.\n--version, -V    Print the version.\n-h, --help    Show help for the current command path.",
        "setup    Register config roots.\ndoctor    Report or repair private permissions.\nconnect    Select a HostEntry and connect or run host action.\ntui    Open Hosts or continue a connection in the TUI.\nhost    List and mutate HostEntry records.\npair    Manage Pair routes.\ntunnel    Run Session or Tunnel workflow; list and manage tunnels.",
        "sshx connect --help",
        "Help exits 0. Parse errors stay on stderr and exit 2. Interactive cancellation exits 130.\nHelp never starts a connection, changes filesystem source of truth, or opens a pager.",
        "sshx setup, sshx doctor, sshx connect, sshx host, sshx pair, sshx tunnel",
    )
}

fn setup() -> String {
    page(
        "setup",
        "Register config roots used as the filesystem source of truth.",
        "sshx setup [OPTIONS]\nsshx setup --help\nsshx help setup",
        "None.",
        "--personal PATH    Register a personal config root.\n--work PATH    Register a work config root.\n--project NAME    Scope the next config root to a project.\nWith a usable TTY, partial scope/project input opens the root workspace and remains prefilled. Setup requires an existing config file and explicit Register action.\nJSON and YAML output do not apply to setup.",
        "",
        "sshx setup --personal ~/.sshx/personal",
        "Exit 0 after roots save. Invalid paths or a declined prompt exit 2. Help exits 0.",
        "sshx doctor, sshx host list",
    )
}

fn doctor() -> String {
    page(
        "doctor",
        "Inspect sshx private permissions without changing files by default.",
        "sshx doctor [OPTIONS]\nsshx doctor --help\nsshx help doctor",
        "None.",
        "--fix-permissions    Show eligible paths and current/target modes, ask once, then set files to 0600 and directories to 0700.\n--format human|json|yaml    Render the diagnostic report.\nDefault behavior is report-only. Wrong-owner paths, symlinks, wrong types, and shared paths remain unchanged. Doctor never enrolls host keys.",
        "",
        "sshx doctor --fix-permissions",
        "Exit 0 when no unsafe finding remains. Remaining findings or failed repairs exit nonzero. Non-interactive execution never applies repairs. Help exits 0.",
        "sshx setup, sshx connect",
    )
}

fn connect() -> String {
    page(
        "connect",
        "Resolve one exact HostEntry and connect or copy an applicable host action.",
        "sshx connect [SELECTOR] [OPTIONS]\nsshx connect --help\nsshx help connect",
        "SELECTOR    Exact alias, HostEntry ID, or exact source disambiguator. With no selector, usable stdin and stderr terminals open HostEntry selection followed by the Session workspace; a fully specified selector connects directly. Explicit selectors never become fuzzy searches.\n`sshx tui connect [SELECTOR]` opens the Session workspace with supplied values visible and editable. The workspace shows mode, route, forwarding rows, and review before SSH starts.",
        "--id ID    Select one HostEntry by stable ID.\n--source PATH --line NUMBER    Select the exact filesystem source of truth.\n--action connect|copy-ssh|copy-sshx|copy-password    Choose an action and skip the action menu. Copy SSH is direct-only; Copy sshx supports direct and Pair routes; Copy password requires an eligible direct stored password, warning, immediate TTY confirmation, and clipboard support.\n--forward REMOTE[=LOCAL]    Repeat to request several declared services in one Session or Tunnel.\n-L SPEC --local-forward SPEC    Repeat direct custom local rows; mix with --forward. SPEC is [bind_address:]local_port:destination_host:remote_port.\n-R SPEC --remote-forward SPEC    Open a server-side listener; connections go to your-side destination. OpenSSH reports server bind failures.\n-D SPEC --dynamic-forward SPEC    Open local SOCKS proxy; configure applications explicitly to use it.\nNo selector with an explicit action skips the action menu. `--format json|yaml` remains inspection-only. Non-interactive selectorless commands require a host; explicit selectors retain exact-match behavior.",
        "",
        "sshx connect prod\nsshx connect --action copy-sshx",
        "Exit 0 after the selected action. Parse errors and unavailable actions exit 2. JSON or YAML with --action is a conflict. Picker cancellation exits 130. Help exits 0.",
        "sshx host list, sshx doctor, sshx tunnel",
    )
}
fn tui() -> String {
    page(
        "tui",
        "Open Hosts or a focused CLI continuation in the interactive terminal UI.",
        "sshx tui [connect [SELECTOR] | host show [SELECTOR] | host create | tunnel status|stop [ID]] [OPTIONS]\nsshx tui --help\nsshx help tui",
        "connect    Select a HostEntry when missing, then edit the Session or Tunnel workspace.\nhost show    Select an exact HostEntry and inspect its identity.\nhost create    Edit required fields before the reviewed mutation.\ntunnel status|stop    Browse registered Tunnels when ID is missing.",
        "--config PATH    Select config roots.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--id ID    Select a HostEntry by stable ID.\n--source PATH --line NUMBER    Require an exact source and Host line.\n--forward REMOTE[=LOCAL]    Repeat to preselect declared-service rows.\n-L SPEC --local-forward SPEC    Repeat to preselect custom local rows on direct routes; SPEC is [bind_address:]local_port:destination_host:remote_port.\n-R SPEC --remote-forward SPEC    Open a server-side listener; connections go to your-side destination. OpenSSH reports server bind failures.\n-D SPEC --dynamic-forward SPEC    Open a local SOCKS proxy; configure applications explicitly.\nIn the workspace, use a to add -L, r to add -R, d to add -D, e to edit, x to remove custom/R/D rows, and Space to select.\n--bind    Open the workspace with declared service rows available.\nPair routes reject custom local, remote, and SOCKS rows before startup.\n--no-input and JSON/YAML output are not valid with `tui`.",
        "",
        "sshx tui\nsshx tui connect prod\nsshx tui host show\nsshx tui tunnel stop",
        "Requires usable stdin and stderr terminals. Parse errors exit 2. Cancelling unfinished CLI continuation exits 130. Help exits 0.",
        "sshx connect, sshx host show",
    )
}
fn host() -> String {
    page(
        "host",
        "Inspect and mutate HostEntry records from the filesystem source of truth.",
        "sshx host COMMAND [OPTIONS]\nsshx host --help\nsshx help host",
        "COMMAND    list, show, create, update, rename, or delete.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Filter or disambiguate the exact source and Host line.\n--id ID, --host-id ID    Select a stable HostEntry ID.\n--no-input    Reject picker and confirmation prompts.\nSelectors stay exact. update, rename, and delete use the live fuzzy picker only when a selector is missing and a usable TTY exists.",
        "list    List HostEntry records.\nshow    Show one exact HostEntry.\ncreate    Create a HostEntry.\nupdate    Update a HostEntry.\nrename    Rename an alias.\ndelete    Delete a HostEntry.",
        "sshx host list",
        "Exit 0 on success. Invalid selectors, conflicts, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx connect, sshx pair setup",
    )
}

fn host_list() -> String {
    page(
        "host list",
        "List discovered HostEntry records and diagnostics.",
        "sshx host list [OPTIONS]\nsshx host list --help\nsshx help host list",
        "None.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Filter by exact source path and Host line.\n--id ID, --host-id ID    Filter by stable HostEntry ID.\nNo prompt supplies missing values.",
        "",
        "sshx host list --format json",
        "Exit 0 after rendering. Discovery diagnostics can make the command nonzero. Help exits 0.",
        "sshx host show SELECTOR, sshx connect",
    )
}

fn host_show() -> String {
    page(
        "host show",
        "Show one exact HostEntry.",
        "sshx host show [SELECTOR] [OPTIONS]\nsshx host show --help\nsshx help host show",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH with --line NUMBER to disambiguate duplicate aliases. With no selector, human output and usable stdin and stderr TTYs open HostEntry selection.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output. Missing selector cannot use machine output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\nExplicit selectors stay exact. Missing selectors open a focused picker only with interactive human output. No fuzzy fallback applies to supplied selectors.",
        "",
        "sshx host show prod\nsshx host show",
        "Exit 0 when matching HostEntries render. Missing selectors require human output and a usable TTY. Ambiguous selectors, source mismatches, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx host list, sshx connect",
    )
}

fn host_create() -> String {
    page(
        "host create",
        "Create one HostEntry in the filesystem source of truth.",
        "sshx host create [OPTIONS]\nsshx host create --help\nsshx help host create",
        "None.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Select one config root; repeat values conflict.\n--folder PATH    Base folder for a relative target.\n--file PATH, --target-file PATH    Target SSH source file.\n--alias ALIAS --hostname HOSTNAME    Required HostEntry fields.\n--user USER --port PORT    Optional connection fields.\n--password-stdin    Read an optional password from a pipe; never from a TTY.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject missing-value prompts.\nMissing required values open one editable workspace only with usable TTY and human output. Hosts uses Ctrl+N. Tab/arrow keys move; Ctrl-S reviews plan; password stays masked; Esc cancels. Missing values in non-interactive mode fail with actionable errors.",
        "",
        "sshx host create --scope personal --file ~/.ssh/config --alias prod --hostname prod.example",
        "Exit 0 after writing private files safely. Conflicts, invalid values, declined consent, and missing non-interactive values exit 2. Help exits 0.",
        "sshx host update, sshx host list",
    )
}

fn host_update() -> String {
    page(
        "host update",
        "Update one exact HostEntry while preserving its source identity.",
        "sshx host update [SELECTOR] [OPTIONS]\nsshx host update --help\nsshx help host update",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH with --line NUMBER to disambiguate duplicate aliases. Missing selector invokes the live fuzzy picker on a usable TTY.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--alias ALIAS --hostname HOSTNAME --user USER --port PORT    Replace fields.\n--clear-user --clear-port --clear-password    Remove fields.\n--password-stdin    Read a replacement password from a pipe.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nWith usable terminals, a missing selector or missing change opens one editable workspace with current values. Blank password keeps stored password; Ctrl-X clears optional fields. Review exact source diff before confirmation. Escape cancels without mutation.",
        "",
        "sshx host update prod --hostname prod.example",
        "Exit 0 after mutation. Selector errors, source mismatches, conflicts, declined consent, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx host rename, sshx host delete",
    )
}

fn host_rename() -> String {
    page(
        "host rename",
        "Rename one exact HostEntry alias.",
        "sshx host rename [SELECTOR] --alias ALIAS\nsshx host rename --help\nsshx help host rename",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH with --line NUMBER to disambiguate duplicate aliases. Missing selector invokes the live fuzzy picker on a usable TTY.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--alias ALIAS    New alias; rename accepts only --alias.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nWith usable terminals, a missing selector or alias opens the prefilled edit workspace. Rename changes only selected alias. Review exact source diff before confirmation; Escape cancels without mutation.",
        "",
        "sshx host rename prod --alias production",
        "Exit 0 after mutation. Missing values, selector errors, source mismatches, conflicts, declined consent, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx host update, sshx host delete",
    )
}

fn host_delete() -> String {
    page(
        "host delete",
        "Delete one exact HostEntry after confirmation.",
        "sshx host delete [SELECTOR] [OPTIONS]\nsshx host delete --help\nsshx help host delete",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH with --line NUMBER to disambiguate duplicate aliases. Missing selector invokes the live fuzzy picker on a usable TTY.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--yes    Skip delete confirmation.\n--preview, --dry-run    Render the mutation plan without writing.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nDelete does not accept host fields. Hosts offers Update, Rename, and Delete actions for the selected exact HostEntry. Review shows its source block and Pair references block unsafe deletion. Escape or decline leaves files unchanged.",
        "",
        "sshx host delete obsolete --yes",
        "Exit 0 after mutation. Missing selectors, source mismatches, declined confirmation, conflicts, pair references, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx host list, sshx host update",
    )
}

fn pair() -> String {
    page(
        "pair",
        "Manage gateway-to-VM Pair routes.",
        "sshx pair COMMAND [OPTIONS]\nsshx pair --help\nsshx help pair",
        "COMMAND    setup, list, validate, or recover.",
        "--config PATH    Select config roots used by Pair discovery.\n--format human|json|yaml    Select output where supported.\nPair recovery is explicit and restores saved pre-mutation content. Pair owns its transit route. ProxyCommand and native Copy SSH are unavailable for Pair routes.",
        "setup    Select a gateway and VM, then save a Pair.\nlist    List Pair records.\nvalidate    Validate Pair records.\nrecover    Review and recover pending Pair setup journals.",
        "sshx pair list",
        "Exit 0 on success. Invalid selectors, route conflicts, and parse errors exit 2. Help exits 0.",
        "sshx connect, sshx tunnel",
    )
}

fn pair_setup() -> String {
    page(
        "pair setup",
        "Create one Pair route between a gateway and a VM.",
        "sshx pair setup [GATEWAY] [VM] [OPTIONS]\nsshx pair setup --help\nsshx help pair setup",
        "GATEWAY    Exact alias or stable HostEntry ID; missing value invokes the interactive picker.\nVM    Exact alias or stable HostEntry ID; missing value invokes the interactive picker. Duplicate aliases require source and Host line disambiguation.",
        "--config PATH    Select config roots used by discovery.\n--format human|json|yaml    Select output.\n--gateway SELECTOR, --gateway-id ID, --gateway-selector SELECTOR    Select an exact gateway alias or stable HostEntry ID.\n--vm SELECTOR, --vm-id ID, --vm-selector SELECTOR    Select an exact VM alias or stable HostEntry ID.\n--gateway-source PATH --gateway-line NUMBER    Disambiguate the gateway by exact source and Host line.\n--vm-source PATH --vm-line NUMBER    Disambiguate the VM by exact source and Host line.\n--source PATH --line NUMBER    Gateway source and Host line fallback; provide both together.\n--id ID, --host-id ID    Gateway stable ID fallback.\n--transit-host HOST --transit-port PORT    Configure the Pair transit endpoint.\n--preview, --dry-run    Render the Pair plan without writing.\n--yes    Apply without consent prompt; required without a TTY unless previewing.\n--no-input, --non-interactive    Reject missing-selector and consent prompts.\nConflicting positional and option selectors exit 2; source and line must be paired.\nEscape or Ctrl-C cancels picker selection. Password descriptors are for later Pair-routed connect or tunnel operations, not Pair setup.",
        "",
        "sshx pair setup gateway vm",
        "Exit 0 after saving a Pair. Missing selectors, source mismatches, route conflicts, declined consent, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx pair list, sshx tunnel",
    )
}

fn pair_list() -> String {
    page(
        "pair list",
        "List Pair routes.",
        "sshx pair list [OPTIONS]\nsshx pair list --help\nsshx help pair list",
        "None.",
        "--config PATH    Select config roots used by Pair discovery.\n--format human|json|yaml    Select output.",
        "",
        "sshx pair list --format json",
        "Exit 0 after rendering. Invalid Pair records can make the command nonzero. Help exits 0.",
        "sshx pair validate, sshx tunnel list",
    )
}

fn pair_validate() -> String {
    page(
        "pair validate",
        "Validate Pair routes without changing the filesystem source of truth.",
        "sshx pair validate [OPTIONS]\nsshx pair validate --help\nsshx help pair validate",
        "None.",
        "--config PATH    Select config roots used by Pair discovery.\n--format human|json|yaml    Select output.\nValidation is report-only and never repairs permissions or route data.",
        "",
        "sshx pair validate",
        "Exit 0 when all Pair routes validate. Invalid routes exit nonzero. Help exits 0.",
        "sshx pair list, sshx doctor",
    )
}

fn pair_recover() -> String {
    page(
        "pair recover",
        "Recover pending Pair setup journals after explicit review.",
        "sshx pair recover [OPTIONS]\nsshx pair recover --help\nsshx help pair recover",
        "None.",
        "--config PATH    Select config roots used by Pair discovery.\n--yes    Confirm recovery without a prompt.\n--no-input    Disable prompts; requires --yes when journals are pending.\nRecovery restores saved pre-mutation config content. Pair records are rediscovered after recovery; run Pair setup separately.",
        "",
        "sshx pair recover --yes --no-input",
        "Exit 0 after recovery or when no journals are pending. Unconfirmed or declined recovery changes nothing. Help exits 0.",
        "sshx pair setup, sshx pair validate",
    )
}
fn tunnel() -> String {
    page(
        "tunnel",
        "Run automatic-route Tunnel workflows, select a direct or paired route explicitly, or manage registered tunnels. Interactive starts without explicit forwarding options open the connection workspace and return to Hosts with the Tunnel ID; leaving the TUI does not stop a standalone Tunnel.",
        "sshx tunnel [HOST] [OPTIONS]\nsshx tunnel start [HOST] [OPTIONS]\nsshx tunnel direct|paired start [HOST] [OPTIONS]\nsshx tunnel list\nsshx tunnel status|stop|restart ID\nsshx tunnel direct|paired list\nsshx tunnel direct|paired status|stop|restart ID\nsshx tunnel --help\nsshx help tunnel",
        "HOST    Exact alias or stable HostEntry ID. `tunnel HOST` and `tunnel start HOST` select direct or Pair route automatically. Host aliases named `direct` or `paired` remain valid in the automatic form. Missing HOST opens the HostEntry picker when input is available.",
        TUNNEL_OPTIONS,
        "start    Start a tunnel with its HostEntry route selected automatically.\ndirect    Start only when HOST resolves to a direct route; list and manage direct tunnel IDs.\npaired    Start only when HOST resolves to a Pair route; list and manage paired tunnel IDs.\nlist    List every registered tunnel.\nstatus ID    Show one registered tunnel by persisted ID.\nstop ID    Stop one registered tunnel by persisted ID.\nrestart ID    Restart one registered tunnel by persisted ID.",
        "sshx tunnel db-prod\nsshx tunnel direct start db-prod --forward 5432=5432 --forward 6379=6378 --forward 3001=3001 --no-input\nsshx tunnel paired start vm-alias --forward 5432=15432 --no-input",
        "Exit 0 after workflow or lifecycle success. Missing values, route errors, listener errors, and parse errors exit 2. Interactive cancellation exits 130.",
        "sshx connect, sshx pair list, sshx host list",
    )
}

fn tunnel_route(route: TunnelRoute) -> String {
    let route_name = route.as_str();
    let name = format!("tunnel {route_name}");
    let usage = format!(
        "sshx {name} start [HOST] [OPTIONS]\nsshx {name} list\nsshx {name} status ID\nsshx {name} stop ID\nsshx {name} restart ID\nsshx help {name}"
    );
    let arguments = format!(
        "HOST    Exact alias or stable HostEntry ID. `start` requires a {route_name} route.\nID    Exact persisted {route_name} Tunnel ID returned by start or list."
    );
    let subcommands = format!(
        "start    Start a {route_name} tunnel.\nlist    List registered {route_name} tunnels.\nstatus ID    Show a {route_name} tunnel.\nstop ID    Stop an owned {route_name} tunnel.\nrestart ID    Restart a {route_name} tunnel."
    );
    let examples = match route {
        TunnelRoute::Direct => {
            "sshx tunnel direct start db-prod --forward 5432=5432 --forward 6379=6378 --forward 3001=3001 --no-input\nsshx tunnel direct list"
        }
        TunnelRoute::Paired => {
            "sshx tunnel paired start vm-alias --forward 5432=5432 --forward 6379=6378 --forward 3001=3001 --no-input\nsshx tunnel paired list"
        }
    };
    page(
        &name,
        &format!("Select {route_name}-only Tunnel start and lifecycle commands."),
        &usage,
        &arguments,
        TUNNEL_OPTIONS,
        &subcommands,
        examples,
        "Exit 0 after success. Missing values, route errors, ownership errors, and parse errors exit 2. Interactive cancellation exits 130.",
        "sshx tunnel, sshx pair list",
    )
}

fn tunnel_start(route: Option<TunnelRoute>) -> String {
    let command_path = route.map_or_else(
        || "tunnel start".to_string(),
        |route| format!("tunnel {} start", route.as_str()),
    );
    let usage = format!(
        "sshx {command_path} [HOST] [OPTIONS]\nsshx {command_path} --help\nsshx help {command_path}"
    );
    let arguments = route.map_or_else(
        || "HOST    Exact alias or stable HostEntry ID. Select its direct or Pair route automatically. Missing HOST opens the picker when input is available.".to_string(),
        |route| format!("HOST    Exact alias or stable HostEntry ID. It must resolve to a {} route. Missing HOST opens the picker when input is available.", route.as_str()),
    );
    let example = match route {
        Some(TunnelRoute::Direct) => {
            "sshx tunnel direct start db-prod -L 127.0.0.1:15432:db.internal:5432 --no-input"
        }
        Some(TunnelRoute::Paired) => {
            "sshx tunnel paired start vm-alias --forward 5432=15432 --no-input"
        }
        None => "sshx tunnel start db-prod --forward 5432=15432 --no-input",
    };
    let purpose = match route {
        Some(TunnelRoute::Direct) => "Start a standalone direct tunnel and return its persisted ID.",
        Some(TunnelRoute::Paired) => "Start a standalone paired tunnel and return its persisted ID.",
        None => "Start a standalone tunnel with its HostEntry route and return its persisted ID.",
    };
    page(
        &command_path,
        purpose,
        &usage,
        &arguments,
        TUNNEL_OPTIONS,
        "",
        example,
        "Exit 0 after start. Missing values, route errors, listener errors, and parse errors exit 2. Interactive cancellation exits 130.",
        "sshx tunnel, sshx tunnel list, sshx pair list",
    )
}

fn tunnel_lifecycle(
    operation: LifecycleOperation,
    route: Option<TunnelRoute>,
) -> String {
    let metadata = operation.metadata();
    let command_group = route.map_or_else(
        || "tunnel".to_string(),
        |route| format!("tunnel {}", route.as_str()),
    );
    let command_path = format!("{command_group} {}", operation.command());
    let usage = if metadata.is_list {
        format!(
            "sshx {command_path} [OPTIONS]\nsshx {command_path} --help\nsshx help {command_path}"
        )
    } else {
        let id = if matches!(operation, LifecycleOperation::Status | LifecycleOperation::Stop) {
            "[ID]"
        } else {
            "ID"
        };
        format!(
            "sshx {command_path} {id} [OPTIONS]\nsshx {command_path} --help\nsshx help {command_path}"
        )
    };
    let examples = route.map_or_else(
        || metadata.examples.to_string(),
        |route| {
            metadata
                .examples
                .replace("sshx tunnel", &format!("sshx tunnel {}", route.as_str()))
        },
    );
    let purpose = route.map_or_else(
        || metadata.purpose.to_string(),
        |route| format!("{} Route filter: {}.", metadata.purpose, route.as_str()),
    );
    page(
        &command_path,
        &purpose,
        &usage,
        metadata.arguments,
        &format!("{}\n{}", metadata.options, metadata.detail),
        "",
        &examples,
        metadata.exits,
        "sshx tunnel, sshx tunnel direct, sshx tunnel paired, sshx pair list",
    )
}
