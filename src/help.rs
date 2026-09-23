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
        "--id ID    Select one HostEntry by stable ID.\n--source PATH --line NUMBER    Select the exact filesystem source of truth.\n--action connect|copy-ssh|copy-sshx|copy-password    Skip the action menu and run one action.\n--format json|yaml    Inspection-only output; conflicts with --action.\n--no-input    Reject prompts.\nCopy SSH asks for a config root when multiple roots reach the HostEntry. Copy password requires a non-empty stored password, a TTY, a retention warning, immediate confirmation, and a working native clipboard backend. Pair routes hide Copy SSH and Copy password.\nThe picker searches alias, destination, and project. Escape or Ctrl-C exits 130 without side effects.",
        "",
        "sshx connect prod\nsshx connect --action copy-sshx",
        "Exit 0 after the selected action. Parse errors and unavailable actions exit 2. JSON or YAML with --action is a conflict. Picker cancellation exits 130. Help exits 0.",
        "sshx host list, sshx doctor, sshx tunnel direct start",
    )
}
