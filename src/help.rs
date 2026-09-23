use std::fmt::Write as _;

pub const ROOT_USAGE: &str = "Usage: sshx [GLOBAL OPTIONS] [COMMAND]";
pub const SETUP_USAGE: &str = "Usage: sshx setup [--personal PATH] [--work PATH] [--project NAME]";
pub const DOCTOR_USAGE: &str = "Usage: sshx doctor [--fix-permissions] [--format human|json|yaml]";
pub const CONNECT_USAGE: &str = "Usage: sshx connect [SELECTOR] [OPTIONS]";
pub const HOST_USAGE: &str = "Usage: sshx [--config PATH] host COMMAND [OPTIONS]";
pub const PAIR_USAGE: &str = "Usage: sshx pair COMMAND [OPTIONS]";
pub const TUNNEL_USAGE: &str = "Usage: sshx tunnel COMMAND [OPTIONS]";

pub fn render(path: &[String]) -> Result<String, String> {
    let path = path.iter().map(String::as_str).collect::<Vec<_>>();
    match path.as_slice() {
        [] => Ok(root()),
        ["setup"] => Ok(setup()),
        ["doctor"] => Ok(doctor()),
        ["connect"] => Ok(connect()),
        ["host"] => Ok(host()),
        ["host", "list"] => Ok(host_list()),
        ["host", "show"] => Ok(host_show()),
        ["host", "create"] => Ok(host_create()),
        ["host", "update"] => Ok(host_update()),
        ["host", "rename"] => Ok(host_rename()),
        ["host", "delete"] => Ok(host_delete()),
        ["pair"] => Ok(pair()),
        ["pair", "setup"] | ["pair", "create"] => Ok(pair_setup()),
        ["pair", "list"] => Ok(pair_list()),
        ["pair", "validate"] => Ok(pair_validate()),
        _ => Err(format!(
            "unknown help path `{}`\n{}",
            path.join(" "),
            usage(path.as_slice())
        )),
    }
}

pub fn usage(path: &[&str]) -> &'static str {
    match path {
        [] => ROOT_USAGE,
        ["setup", ..] => SETUP_USAGE,
        ["doctor", ..] => DOCTOR_USAGE,
        ["connect", ..] => CONNECT_USAGE,
        ["host", ..] => HOST_USAGE,
        ["pair", ..] => PAIR_USAGE,
        ["tunnel", ..] => TUNNEL_USAGE,
        _ => ROOT_USAGE,
    }
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
