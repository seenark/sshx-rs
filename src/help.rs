use std::fmt::Write as _;

pub const ROOT_USAGE: &str = "Usage: sshx [GLOBAL OPTIONS] [COMMAND]";
pub const SETUP_USAGE: &str = "Usage: sshx setup [--personal PATH] [--work PATH] [--project NAME]";
pub const DOCTOR_USAGE: &str = "Usage: sshx doctor [--fix-permissions] [--format human|json|yaml]";
pub const CONNECT_USAGE: &str = "Usage: sshx connect [SELECTOR] [OPTIONS]";
pub const HOST_USAGE: &str = "Usage: sshx [--config PATH] host COMMAND [OPTIONS]";
pub const PAIR_USAGE: &str = "Usage: sshx pair COMMAND [OPTIONS]";
pub const TUNNEL_USAGE: &str = "Usage: sshx tunnel COMMAND [OPTIONS]";
pub const TUNNEL_DIRECT_USAGE: &str = "Usage: sshx tunnel direct COMMAND [OPTIONS]";
pub const TUNNEL_PAIRED_USAGE: &str = "Usage: sshx tunnel paired COMMAND [OPTIONS]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HelpPage {
    Root,
    Setup,
    Doctor,
    Connect,
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
    Tunnel,
    TunnelDirect,
    TunnelPaired,
    TunnelStart,
    TunnelDirectStart,
    TunnelPairedStart,
    TunnelLifecycle {
        group: TunnelLifecycleGroup,
        operation: LifecycleOperation,
    },
}

impl HelpPage {
    fn usage(self) -> &'static str {
        match self {
            Self::Root => ROOT_USAGE,
            Self::Setup => SETUP_USAGE,
            Self::Doctor => DOCTOR_USAGE,
            Self::Connect => CONNECT_USAGE,
            Self::Host
            | Self::HostList
            | Self::HostShow
            | Self::HostCreate
            | Self::HostUpdate
            | Self::HostRename
            | Self::HostDelete => HOST_USAGE,
            Self::Pair | Self::PairSetup | Self::PairList | Self::PairValidate => PAIR_USAGE,
            Self::Tunnel | Self::TunnelStart => TUNNEL_USAGE,
            Self::TunnelDirect | Self::TunnelDirectStart => TUNNEL_DIRECT_USAGE,
            Self::TunnelPaired | Self::TunnelPairedStart => TUNNEL_PAIRED_USAGE,
            Self::TunnelLifecycle { group, .. } => match group {
                TunnelLifecycleGroup::Root => TUNNEL_USAGE,
                TunnelLifecycleGroup::Direct => TUNNEL_DIRECT_USAGE,
                TunnelLifecycleGroup::Paired => TUNNEL_PAIRED_USAGE,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TunnelLifecycleGroup {
    Root,
    Direct,
    Paired,
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
                arguments: "None. The command lists every registered tunnel.",
                options: "--config PATH    Accepted for common CLI compatibility; this lifecycle command reads the runtime registry under sshx home and does not select discovery roots.\n--format human|json|yaml    Render lifecycle output.",
                purpose: "List every registered standalone tunnel.",
                examples: "sshx tunnel list --format json\nsshx tunnel direct list",
                exits: "Exit 0 after registry rendering. REGISTRY_UNSAFE and parse errors exit 2. Help exits 0.",
                detail: "The locked registry is the source of truth. Human output includes tunnel ID, state, master responsive, and listener ready. JSON and YAML preserve stable fields including master_status and listener_status; no application status is inferred.",
                is_list: true,
            },
            Self::Status => LifecycleMetadata {
                arguments: "ID    Exact persisted tunnel ID returned by start or list; it is not a process ID.",
                options: "--config PATH    Accepted for common CLI compatibility; this lifecycle command reads the runtime registry under sshx home and does not select discovery roots.\n--format human|json|yaml    Render lifecycle output.",
                purpose: "Show one registered tunnel's master responsiveness and listener readiness.",
                examples: "sshx tunnel status dt-1\nsshx tunnel paired status pt-1",
                exits: "Exit 0 after status rendering. TUNNEL_NOT_FOUND, REGISTRY_UNSAFE, and parse errors exit 2. Help exits 0.",
                detail: "The locked registry and control socket prove ownership. Output includes tunnel ID, state, kind, master responsive, listener ready, selected host, source, and forwards. No application status is inferred.",
                is_list: false,
            },
            Self::Stop => LifecycleMetadata {
                arguments: "ID    Exact persisted tunnel ID returned by start or list; it is not a process ID.",
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
        ["host"] => Some(HelpPage::Host),
        ["host", "list", ..] => Some(HelpPage::HostList),
        ["host", "show", ..] => Some(HelpPage::HostShow),
        ["host", "create", ..] => Some(HelpPage::HostCreate),
        ["host", "update", ..] => Some(HelpPage::HostUpdate),
        ["host", "rename", ..] => Some(HelpPage::HostRename),
        ["host", "delete", ..] => Some(HelpPage::HostDelete),
        ["pair"] => Some(HelpPage::Pair),
        ["pair", "setup", ..] | ["pair", "create", ..] => Some(HelpPage::PairSetup),
        ["pair", "list", ..] => Some(HelpPage::PairList),
        ["pair", "validate", ..] => Some(HelpPage::PairValidate),
        ["tunnel"] => Some(HelpPage::Tunnel),
        ["tunnel", "direct"] => Some(HelpPage::TunnelDirect),
        ["tunnel", "direct", "start", ..] => Some(HelpPage::TunnelDirectStart),
        ["tunnel", "direct", operation, ..] => {
            LifecycleOperation::parse(operation).map(|operation| HelpPage::TunnelLifecycle {
                group: TunnelLifecycleGroup::Direct,
                operation,
            })
        }
        ["tunnel", "paired"] => Some(HelpPage::TunnelPaired),
        ["tunnel", "paired", "start", ..] => Some(HelpPage::TunnelPairedStart),
        ["tunnel", "paired", operation, ..] => {
            LifecycleOperation::parse(operation).map(|operation| HelpPage::TunnelLifecycle {
                group: TunnelLifecycleGroup::Paired,
                operation,
            })
        }
        ["tunnel", "start", ..] => Some(HelpPage::TunnelStart),
        ["tunnel", operation, ..] => {
            LifecycleOperation::parse(operation).map(|operation| HelpPage::TunnelLifecycle {
                group: TunnelLifecycleGroup::Root,
                operation,
            })
        }
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
        HelpPage::Tunnel => tunnel(),
        HelpPage::TunnelDirect => tunnel_direct(),
        HelpPage::TunnelPaired => tunnel_paired(),
        HelpPage::TunnelStart => tunnel_start(),
        HelpPage::TunnelDirectStart => tunnel_direct_start(),
        HelpPage::TunnelPairedStart => tunnel_paired_start(),
        HelpPage::TunnelLifecycle { group, operation } => tunnel_lifecycle(group, operation),
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
        "COMMAND PATH    Optional command or command group.",
        "--config PATH    Use a config root or filesystem source of truth.\n--format human|json|yaml    Select output for commands that support it; help stays plain text.\n--version, -V    Print the version.\n-h, --help    Show help for the current command path.",
        "setup    Register config roots.\ndoctor    Report or repair eligible private permissions.\nconnect    Select a HostEntry and connect or run a host action.\nhost    List and mutate HostEntry records.\npair    Manage Pair routes.\ntunnel    Manage direct and paired tunnels.",
        "sshx connect --help",
        "Help exits 0. Parse errors stay on stderr and exit 2. Interactive cancellation exits 130.\nHelp never starts a connection, changes the filesystem source of truth, or opens a pager.",
        "sshx setup, sshx doctor, sshx connect, sshx host, sshx pair, sshx tunnel",
    )
}

fn setup() -> String {
    page(
        "setup",
        "Register config roots used as the filesystem source of truth.",
        "sshx setup [OPTIONS]\nsshx setup --help\nsshx help setup",
        "None.",
        "--personal PATH    Register a personal config root.\n--work PATH    Register a work config root.\n--project NAME    Scope the next config root to a project.\nA missing path can be supplied by the setup prompt when a usable TTY exists.\nJSON and YAML output do not apply to setup.",
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
        "--fix-permissions    Plan eligible repairs, ask once, then set files to 0600 and directories to 0700.\n--format human|json|yaml    Render the diagnostic report.\nDefault behavior is report-only. A prompt can supply one confirmation only for --fix-permissions. Wrong-owner paths, symlinks, wrong types, and shared paths remain diagnostics.",
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
        "SELECTOR    Exact alias, HostEntry ID, or exact source disambiguator. With no selector and a usable TTY, the live fuzzy picker selects one HostEntry.",
        "--id ID    Select one HostEntry by stable ID.\n--source PATH --line NUMBER    Select the exact filesystem source of truth.\n--action connect|copy-ssh|copy-sshx|copy-password    Skip the action menu and run one action.\n--password-fd FD    Supply a direct Host password through an inherited descriptor.\n--gateway-password-fd FD --vm-password-fd FD    Supply gateway and VM passwords through inherited descriptors for Pair-routed connections.\n--format json|yaml    Inspection-only output; conflicts with --action.\n--no-input    Reject prompts.\nCopy SSH asks for a config root when multiple roots reach the HostEntry. Copy password requires a non-empty stored password, a TTY, a retention warning, immediate confirmation, and a working native clipboard backend. Pair routes hide Copy SSH and Copy password.\nThe picker searches alias, destination, and project. Escape or Ctrl-C exits 130 without side effects.",
        "",
        "sshx connect prod\nsshx connect --action copy-sshx",
        "Exit 0 after the selected action. Parse errors and unavailable actions exit 2. JSON or YAML with --action is a conflict. Picker cancellation exits 130. Help exits 0.",
        "sshx host list, sshx doctor, sshx tunnel direct start",
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
        "sshx host show SELECTOR [OPTIONS]\nsshx host show --help\nsshx help host show",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH with --line NUMBER to disambiguate duplicate aliases.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\nNo fuzzy fallback or prompt supplies a missing selector.",
        "",
        "sshx host show prod",
        "Exit 0 when exactly one HostEntry matches. Missing or ambiguous selectors, source mismatches, and parse errors exit 2. Help exits 0.",
        "sshx host list, sshx connect",
    )
}

fn host_create() -> String {
    page(
        "host create",
        "Create one HostEntry in the filesystem source of truth.",
        "sshx host create [OPTIONS]\nsshx host create --help\nsshx help host create",
        "None.",
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Select one config root; repeat values conflict.\n--folder PATH    Base folder for a relative target.\n--file PATH, --target-file PATH    Target SSH source file; required without an interactive prompt.\n--alias ALIAS --hostname HOSTNAME    Required HostEntry fields.\n--user USER --port PORT    Optional connection fields.\n--password-stdin    Read an optional password from a pipe; never from a TTY.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject missing-value prompts.\nMissing scope, folder, file, alias, hostname, optional fields, password, and apply consent can use prompts only with a usable TTY.",
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
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--alias ALIAS --hostname HOSTNAME --user USER --port PORT    Replace fields.\n--clear-user --clear-port --clear-password    Remove fields.\n--password-stdin    Read a replacement password from a pipe.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nEscape or Ctrl-C cancels before mutation. --source and --line must be provided together for exact source disambiguation.",
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
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--alias ALIAS    New alias; rename accepts only --alias.\n--preview, --dry-run    Render the mutation plan without writing.\n--yes    Apply without consent prompt.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nEscape or Ctrl-C cancels before mutation. --source and --line must be provided together for exact source disambiguation.",
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
        "--config PATH    Select a config root.\n--format human|json|yaml    Select output.\n--scope SCOPE --project NAME    Filter HostEntry provenance.\n--source PATH --line NUMBER    Require the selector to match one exact source and Host line.\n--id ID, --host-id ID    Select by stable HostEntry ID.\n--yes    Skip delete confirmation.\n--preview, --dry-run    Render the mutation plan without writing.\n--no-input, --non-interactive    Reject picker and confirmation prompts.\nDelete does not accept host fields. Escape or Ctrl-C cancels before mutation. --source and --line must be provided together for exact source disambiguation.",
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
        "COMMAND    setup/create, list, or validate.",
        "--config PATH    Select config roots used by Pair discovery.\n--format human|json|yaml    Select output where supported.\nPair owns its transit route. ProxyCommand and native Copy SSH are unavailable for Pair routes.",
        "setup, create    Select a gateway and VM, then save a Pair.\nlist    List Pair records.\nvalidate    Validate Pair records.",
        "sshx pair list",
        "Exit 0 on success. Invalid selectors, route conflicts, and parse errors exit 2. Help exits 0.",
        "sshx connect, sshx tunnel paired start",
    )
}

fn pair_setup() -> String {
    page(
        "pair setup (alias: pair create)",
        "Create one Pair route between a gateway and a VM.",
        "sshx pair setup [GATEWAY] [VM] [OPTIONS]\nsshx pair create [GATEWAY] [VM] [OPTIONS]\nsshx pair setup --help\nsshx help pair setup\nsshx help pair create",
        "GATEWAY    Exact alias or stable HostEntry ID; missing value invokes the interactive picker.\nVM    Exact alias or stable HostEntry ID; missing value invokes the interactive picker. Duplicate aliases require source and Host line disambiguation.",
        "--config PATH    Select config roots used by discovery.\n--format human|json|yaml    Select output.\n--gateway SELECTOR, --gateway-id ID, --gateway-selector SELECTOR    Select an exact gateway alias or stable HostEntry ID.\n--vm SELECTOR, --vm-id ID, --vm-selector SELECTOR    Select an exact VM alias or stable HostEntry ID.\n--gateway-source PATH --gateway-line NUMBER    Disambiguate the gateway by exact source and Host line.\n--vm-source PATH --vm-line NUMBER    Disambiguate the VM by exact source and Host line.\n--source PATH --line NUMBER    Gateway source and Host line fallback; provide both together.\n--id ID, --host-id ID    Gateway stable ID fallback.\n--transit-host HOST --transit-port PORT    Configure the Pair transit endpoint.\n--preview, --dry-run    Render the Pair plan without writing.\n--yes    Apply without consent prompt; required without a TTY unless previewing.\n--no-input, --non-interactive    Reject missing-selector and consent prompts.\nConflicting positional and option selectors exit 2; source and line must be paired.\nEscape or Ctrl-C cancels picker selection. Password descriptors are for later Pair-routed connect or tunnel operations, not Pair setup.",
        "",
        "sshx pair setup gateway vm",
        "Exit 0 after saving a Pair. Missing selectors, source mismatches, route conflicts, declined consent, and parse errors exit 2. Picker cancellation exits 130. Help exits 0.",
        "sshx pair list, sshx tunnel paired start",
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
        "sshx pair validate, sshx tunnel paired list",
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
fn tunnel() -> String {
    page(
        "tunnel",
        "Manage detached standalone direct and Pair-routed tunnels.",
        "sshx tunnel COMMAND [OPTIONS]\nsshx tunnel --help\nsshx help tunnel",
        "COMMAND    start, direct, paired, list, status, stop, or restart.",
        "--config PATH    Select the config root.\n--format human|json|yaml    Select human or machine tunnel output.\nA standalone tunnel is owned by its locked registry record and control socket. Tunnel IDs, not process IDs, identify tunnel state.",
        "start (compatibility alias; canonical: tunnel paired start)    Run the canonical paired start command.\ndirect    Start or manage direct tunnels.\npaired    Start or manage Pair-routed tunnels.\nlist    List all registered tunnels.\nstatus    Show one tunnel by ID.\nstop    Stop one tunnel by ID.\nrestart    Restart one tunnel by ID.",
        "sshx tunnel direct start prod -L 8080:localhost:80\nsshx tunnel paired start vm --forward 8080=18080",
        "Exit 0 after rendering or lifecycle success. Missing IDs, route conflicts, registry ownership failures, and parse errors exit 2. Help exits 0. Interactive cancellation exits 130.",
        "sshx tunnel direct, sshx tunnel paired, sshx pair list",
    )
}

fn tunnel_direct() -> String {
    page(
        "tunnel direct",
        "Start and manage one-host standalone tunnels with explicit OpenSSH forwarding.",
        "sshx tunnel direct COMMAND [OPTIONS]\nsshx tunnel direct --help\nsshx help tunnel direct",
        "COMMAND    start, list, status, stop, or restart.",
        "-L SPEC, --local-forward SPEC    Add local forwarding.\n-R SPEC, --remote-forward SPEC    Add remote forwarding.\n-D SPEC, --dynamic-forward SPEC    Add dynamic forwarding.\n--allow-bind, --allow-non-loopback    Allow non-loopback listener binds.\n--password-fd FD    Supply the selected Host password through an inherited descriptor.\n--config PATH --format human|json|yaml    Select config root and output.\nDirect start requires at least one -L, -R, or -D. Pair-routed HostEntry records are rejected; exact HostEntry ProxyCommand stays a direct-host concern.",
        "start    Start one direct tunnel.\nlist, status, stop, restart    Operate on registered tunnel IDs; each is an alias of the root tunnel lifecycle command.",
        "sshx tunnel direct start prod -L 8080:localhost:80\nsshx tunnel direct status dt-1",
        "Exit 0 on success. TUNNEL_FORWARD_REQUIRED, TUNNEL_DIRECT_PAIR, selector errors, lifecycle errors, and parse errors exit 2. Help exits 0. Interactive cancellation exits 130.",
        "sshx tunnel direct start, sshx tunnel paired, sshx host show",
    )
}

fn tunnel_paired() -> String {
    page(
        "tunnel paired",
        "Start and manage standalone tunnels through a saved Pair route.",
        "sshx tunnel paired COMMAND [OPTIONS]\nsshx tunnel paired --help\nsshx help tunnel paired",
        "COMMAND    start, list, status, stop, or restart.",
        "--forward REMOTE[=LOCAL]    Forward a declared VM service by remote port or PORT#INDEX; optional LOCAL overrides its local port.\n--bind    Interactively select declared VM service forwards and local ports.\n--password-fd FD    Supply the VM password through an inherited descriptor.\n--gateway-password-fd FD    Supply the gateway password through an inherited descriptor.\n--vm-password-fd FD    Supply the VM password through an inherited descriptor; overrides --password-fd.\n--config PATH --format human|json|yaml    Select config root and output.\nPaired start requires --forward or --bind. Pair owns the transit route; ProxyCommand is unsupported. -L, -R, and -D are direct-mode options.",
        "start    Start one Pair-routed tunnel.\nlist, status, stop, restart    Operate on registered tunnel IDs; each is an alias of the root tunnel lifecycle command.",
        "sshx tunnel paired start vm --forward 8080=18080\nsshx tunnel paired start vm --bind",
        "Exit 0 on success. TUNNEL_FORWARD_REQUIRED, TUNNEL_PAIRED_REQUIRED, TUNNEL_FORWARD_MODE, route errors, lifecycle errors, and parse errors exit 2. Help exits 0. Interactive cancellation exits 130.",
        "sshx tunnel paired start, sshx pair list, sshx tunnel direct",
    )
}

fn tunnel_start() -> String {
    page(
        "tunnel start (compatibility alias; canonical: tunnel paired start)",
        "Start one Pair-routed standalone tunnel; keep this root form for compatibility.",
        "sshx tunnel start [SELECTOR] [OPTIONS]\nsshx tunnel start --help\nsshx help tunnel start\nCompatibility alias for sshx tunnel paired start [SELECTOR] [OPTIONS].",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH --line NUMBER for exact source disambiguation. Without a selector on a usable TTY, a live fuzzy picker selects one HostEntry.",
        "--forward REMOTE[=LOCAL]    Forward a declared VM service by remote port or PORT#INDEX; optional LOCAL overrides its local port.\n--bind    Interactively select declared VM service forwards and local ports.\n--password-fd FD    Supply the VM password through an inherited descriptor.\n--gateway-password-fd FD    Supply the gateway password through an inherited descriptor.\n--vm-password-fd FD    Supply the VM password through an inherited descriptor; overrides --password-fd.\n--no-input    Reject the fuzzy picker, --bind prompts, host-key enrollment, and password prompts.\n--config PATH --format human|json|yaml    Select config root and output.\nDo not pass -L, -R, or -D. Pair owns the transit route; ProxyCommand is unsupported.",
        "",
        "sshx tunnel start vm --forward 8080=18080\nsshx tunnel start --source ~/.ssh/config --line 12 --bind",
        "Exit 0 after both gateway and VM masters and listeners become ready. TUNNEL_PAIRED_REQUIRED, TUNNEL_FORWARD_MODE, selector errors, route errors, and parse errors exit 2. Escape or Ctrl-C cancels before or during setup and exits 130. The response includes a tunnel ID and state.",
        "sshx tunnel paired start, sshx pair list, sshx tunnel list",
    )
}

fn tunnel_direct_start() -> String {
    page(
        "tunnel direct start",
        "Start one detached standalone tunnel for one exact HostEntry using direct forwarding.",
        "sshx tunnel direct start [SELECTOR] [OPTIONS]\nsshx tunnel direct start --help\nsshx help tunnel direct start",
        "SELECTOR    Exact alias or stable HostEntry ID. Use --source PATH --line NUMBER for exact source disambiguation. Without a selector on a usable TTY, a live fuzzy picker selects one HostEntry.",
        "-L SPEC, --local-forward SPEC    Add local forwarding.\n-R SPEC, --remote-forward SPEC    Add remote forwarding.\n-D SPEC, --dynamic-forward SPEC    Add dynamic forwarding.\n--allow-bind, --allow-non-loopback    Allow non-loopback listener binds.\n--password-fd FD    Supply the selected Host password through an inherited descriptor.\n--no-input    Reject the fuzzy picker, host-key enrollment, and password prompts.\n--config PATH --format human|json|yaml    Select config root and output.\nAt least one -L, -R, or -D is required. --forward and --bind belong to paired mode. Exact HostEntry ProxyCommand is supported; Pair-routed entries are rejected.",
        "",
        "sshx tunnel direct start prod -L 8080:localhost:80\nsshx tunnel direct start --id host-123 -D 1080",
        "Exit 0 after the detached master and listeners become ready. TUNNEL_FORWARD_REQUIRED, TUNNEL_DIRECT_PAIR, selector errors, listener errors, and parse errors exit 2. Escape or Ctrl-C cancels before or during setup and exits 130. The response includes a tunnel ID and state.",
        "sshx tunnel direct list, sshx tunnel paired start, sshx host show",
    )
}

fn tunnel_paired_start() -> String {
    page(
        "tunnel paired start",
        "Start one detached standalone tunnel for one Pair route with gateway and VM masters.",
        "sshx tunnel paired start [SELECTOR] [OPTIONS]\nsshx tunnel paired start --help\nsshx help tunnel paired start",
        "SELECTOR    Exact alias or stable HostEntry ID for the VM. Use --source PATH --line NUMBER for exact source disambiguation. Without a selector on a usable TTY, a live fuzzy picker selects one HostEntry.",
        "--forward REMOTE[=LOCAL]    Forward a declared VM service by remote port or PORT#INDEX; optional LOCAL overrides its local port.\n--bind    Interactively select declared VM service forwards and local ports.\n--password-fd FD    Supply the VM password through an inherited descriptor.\n--gateway-password-fd FD    Supply the gateway password through an inherited descriptor.\n--vm-password-fd FD    Supply the VM password through an inherited descriptor; overrides --password-fd.\n--no-input    Reject the fuzzy picker, --bind prompts, host-key enrollment, and password prompts.\n--config PATH --format human|json|yaml    Select config root and output.\nDo not pass -L, -R, or -D. Pair owns the transit route; ProxyCommand is unsupported.",
        "",
        "sshx tunnel paired start vm --forward 8080=18080\nsshx tunnel paired start --id vm-123 --bind",
        "Exit 0 after gateway and VM masters and listeners become ready. TUNNEL_FORWARD_REQUIRED, TUNNEL_PAIRED_REQUIRED, TUNNEL_FORWARD_MODE, selector errors, route errors, and parse errors exit 2. Escape or Ctrl-C cancels before or during setup and exits 130. The response includes one tunnel ID and state for both gateway and VM.",
        "sshx tunnel paired list, sshx pair list, sshx tunnel direct start",
    )
}

impl TunnelLifecycleGroup {
    fn command_path(self, operation: LifecycleOperation) -> String {
        let operation = operation.command();
        match self {
            Self::Root => format!("tunnel {operation}"),
            Self::Direct => format!("tunnel direct {operation}"),
            Self::Paired => format!("tunnel paired {operation}"),
        }
    }

    fn name(self, operation: LifecycleOperation) -> String {
        let command_path = self.command_path(operation);
        match self {
            Self::Root => command_path,
            Self::Direct | Self::Paired => {
                format!("{command_path} (alias: tunnel {})", operation.command())
            }
        }
    }
}

fn tunnel_lifecycle(group: TunnelLifecycleGroup, operation: LifecycleOperation) -> String {
    let metadata = operation.metadata();
    let command_path = group.command_path(operation);
    let name = group.name(operation);
    let usage = if metadata.is_list {
        format!(
            "sshx {command_path} [OPTIONS]\nsshx {command_path} --help\nsshx help {command_path}"
        )
    } else {
        format!(
            "sshx {command_path} ID [OPTIONS]\nsshx {command_path} --help\nsshx help {command_path}"
        )
    };
    page(
        &name,
        metadata.purpose,
        &usage,
        metadata.arguments,
        &format!("{}\n{}", metadata.options, metadata.detail),
        "",
        metadata.examples,
        metadata.exits,
        "sshx tunnel, sshx tunnel direct, sshx tunnel paired",
    )
}
