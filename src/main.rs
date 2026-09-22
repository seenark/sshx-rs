use serde::Serialize;
use sshx::discovery::{HostEntry, discover_roots, scope_for_path};
use sshx::output::{OutputFormat, render_diagnostic, render_human, render_machine};
use sshx::settings::{self, RegisteredRoot};
use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process;

const USAGE: &str = "Usage: sshx [--version]";
const HOST_USAGE: &str = "Usage: sshx [--config PATH] host list [--format human|json|yaml]\n       sshx [--config PATH] host show SELECTOR [--format human|json|yaml]";
const SETUP_USAGE: &str = "Usage: sshx setup [--personal PATH] [--work PATH] [--project NAME]\n       sshx connect [SELECTOR] [--id ID] [--source PATH --line NUMBER] [--no-input]";

fn main() {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("sshx: {error}");
            process::exit(2);
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty() {
        println!("{USAGE}");
        return Ok(());
    }
    if let Some(first) = args.first().and_then(|argument| argument.to_str()) {
        if first == "--version" || first == "-V" {
            println!("{}", sshx::VERSION);
            return Ok(());
        }
        if first == "--help" || first == "-h" {
            println!("{USAGE}");
            return Ok(());
        }
    }

    let cli = Cli::parse(args)?;
    validate_connect_without_catalog(&cli)?;
    if matches!(cli.command, Command::Setup) {
        return run_setup(&cli);
    }

    let roots = registered_roots(&cli)?;
    let configured = settings::discovery_roots(&roots);
    let catalog = discover_roots(&configured).map_err(|error| error.to_string())?;
    for diagnostic in &catalog.diagnostics {
        eprintln!("{}", render_diagnostic(diagnostic));
    }
    let filtered = filter_entries(&catalog.entries, &cli);

    match &cli.command {
        Command::List => render_entries(&filtered, &catalog.diagnostics, cli.format),
        Command::Show(selector) => {
            let entries = select_entries(&filtered, selector)?;
            render_entries(&entries, &catalog.diagnostics, cli.format)
        }
        Command::Connect(selector) => {
            let entry = select_connect_entry(&filtered, selector.as_deref(), &cli)?;
            if cli.format.is_machine() {
                render_entries(&[entry], &catalog.diagnostics, cli.format)
            } else {
                if catalog
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "unsupported_match")
                {
                    return Err(
                        "UNSUPPORTED_MATCH: Match prevents exact runtime configuration".to_string(),
                    );
                }
                let alias = selected_connect_alias(entry, selector.as_deref(), &cli);
                sshx::connect::open(entry, &home_dir()?, cli.no_input, alias)
            }
        }
        Command::Setup => unreachable!(),
    }
}

fn validate_connect_without_catalog(cli: &Cli) -> Result<(), String> {
    let Command::Connect(selector) = &cli.command else {
        return Ok(());
    };
    if selector.is_some() || cli.id.is_some() {
        return Ok(());
    }
    if cli.source.is_some() != cli.line.is_some() {
        return Err(
            "SELECTOR_INCOMPLETE: --source and --line must be provided together".to_string(),
        );
    }
    if cli.source.is_some() {
        return Err("SELECTOR_INCOMPLETE: alias is required with --source and --line".to_string());
    }
    if cli.no_input {
        return Err("HOST_REQUIRED: connect requires a host in --no-input mode".to_string());
    }
    Ok(())
}

fn render_entries(
    entries: &[&HostEntry],
    diagnostics: &[sshx::discovery::Diagnostic],
    format: OutputFormat,
) -> Result<(), String> {
    if format.is_machine() {
        let document = render_machine(entries.to_vec(), diagnostics, format)?;
        print!("{document}");
    } else {
        print!("{}", render_human(entries));
    }
    Ok(())
}

fn run_setup(cli: &Cli) -> Result<(), String> {
    let home = home_dir()?;
    let mut roots = settings::load(&home)?;
    let mut additions = Vec::new();
    for request in &cli.roots {
        let path = settings::normalize_path(&request.path, &home);
        if !path.is_file() {
            return Err(format!(
                "SETUP_ROOT_NOT_FOUND: config root is not a file: {}",
                path.display()
            ));
        }
        additions.push(RegisteredRoot {
            scope: request.scope.clone(),
            path,
            project: request
                .project
                .clone()
                .or_else(|| cli.projects.first().cloned()),
        });
    }
    if additions.is_empty() {
        additions = settings::auto_detect(&home);
        if let Some(project) = cli.projects.first() {
            for root in &mut additions {
                root.project = Some(project.clone());
            }
        }
    }
    if let Some(config) = &cli.config {
        let path = settings::normalize_path(config, &home);
        if !path.is_file() {
            return Err(format!(
                "SETUP_ROOT_NOT_FOUND: config root is not a file: {}",
                path.display()
            ));
        }
        additions.push(RegisteredRoot {
            scope: cli
                .scopes
                .first()
                .cloned()
                .unwrap_or_else(|| scope_for_path(&path)),
            path,
            project: cli.projects.first().cloned(),
        });
    }
    if additions.is_empty() {
        return Err("SETUP_ROOT_REQUIRED: no existing config roots found".to_string());
    }
    settings::merge(&mut roots, additions);
    settings::save(&home, &roots)?;
    render_roots(&roots, cli.format)
}

#[derive(Serialize)]
struct RootDocument<'a> {
    version: u8,
    roots: &'a [RegisteredRoot],
}

fn render_roots(roots: &[RegisteredRoot], format: OutputFormat) -> Result<(), String> {
    if format.is_machine() {
        let document = RootDocument { version: 1, roots };
        let mut rendered = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&document)
                .map_err(|error| format!("cannot render JSON output: {error}"))?,
            OutputFormat::Yaml => serde_yaml::to_string(&document)
                .map_err(|error| format!("cannot render YAML output: {error}"))?,
            OutputFormat::Human => unreachable!(),
        };
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        print!("{rendered}");
    } else {
        for root in roots {
            println!(
                "{}: {}{}",
                root.scope,
                root.path.display(),
                root.project
                    .as_deref()
                    .map(|project| format!(" ({project})"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn registered_roots(cli: &Cli) -> Result<Vec<RegisteredRoot>, String> {
    if let Some(config) = &cli.config {
        let path = settings::normalize_path(config, &home_dir()?);
        return Ok(vec![RegisteredRoot {
            scope: scope_for_path(&path),
            path,
            project: cli.projects.first().cloned(),
        }]);
    }
    let home = home_dir()?;
    let roots = settings::load(&home)?;
    if !roots.is_empty() {
        return Ok(roots);
    }
    let detected = settings::auto_detect(&home);
    if !detected.is_empty() {
        return Ok(detected);
    }
    Ok(vec![RegisteredRoot {
        scope: "personal".to_string(),
        path: home.join(".ssh/config"),
        project: None,
    }])
}

fn filter_entries<'a>(entries: &'a [HostEntry], cli: &Cli) -> Vec<&'a HostEntry> {
    let source = cli.source.as_ref().map(|path| {
        settings::normalize_path(path, &home_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .to_string_lossy()
            .into_owned()
    });
    entries
        .iter()
        .filter(|entry| {
            let provenance_matches = entry.provenance.iter().any(|provenance| {
                (cli.scopes.is_empty() || cli.scopes.iter().any(|scope| scope == &provenance.scope))
                    && (cli.projects.is_empty()
                        || cli
                            .projects
                            .iter()
                            .any(|project| provenance.project.as_deref() == Some(project.as_str())))
            });
            provenance_matches
                && source
                    .as_deref()
                    .is_none_or(|path| entry.source.path == path)
                && cli.line.is_none_or(|line| entry.source.line_start == line)
                && cli.id.as_deref().is_none_or(|id| entry.id == id)
        })
        .collect()
}

fn select_entries<'a>(
    entries: &[&'a HostEntry],
    selector: &str,
) -> Result<Vec<&'a HostEntry>, String> {
    let selected = entries
        .iter()
        .copied()
        .filter(|entry| entry.id == selector || entry.aliases.iter().any(|alias| alias == selector))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(format!("host selector `{selector}` matched no entries"));
    }
    Ok(selected)
}

fn select_connect_entry<'a>(
    entries: &[&'a HostEntry],
    positional: Option<&str>,
    cli: &Cli,
) -> Result<&'a HostEntry, String> {
    if cli.id.is_some() && positional.is_some() {
        return Err("SELECTOR_CONFLICT: use either --id or a positional selector".to_string());
    }
    if cli.id.is_some() && (cli.source.is_some() || cli.line.is_some()) {
        return Err(
            "SELECTOR_CONFLICT: use either --id or an alias with complete source location"
                .to_string(),
        );
    }
    let selector = cli.id.as_deref().or(positional);
    if cli.source.is_some() != cli.line.is_some() {
        return Err(
            "SELECTOR_INCOMPLETE: --source and --line must be provided together".to_string(),
        );
    }
    if selector.is_none() && cli.source.is_some() {
        return Err("SELECTOR_INCOMPLETE: alias is required with --source and --line".to_string());
    }
    let Some(selector) = selector else {
        if cli.no_input {
            return Err("HOST_REQUIRED: connect requires a host in --no-input mode".to_string());
        }
        if !io::stdin().is_terminal() {
            return Err(
                "HOST_REQUIRED: connect requires a host outside interactive mode".to_string(),
            );
        }
        return interactive_select(entries);
    };

    let by_id = cli.id.is_some()
        || entries.iter().any(|entry| {
            entry.id == selector && !entry.aliases.iter().any(|alias| alias == selector)
        });
    let matches = entries
        .iter()
        .copied()
        .filter(|entry| {
            if by_id {
                entry.id == selector
            } else {
                entry.aliases.iter().any(|alias| alias == selector)
            }
        })
        .filter(|entry| {
            cli.source.as_ref().is_none_or(|source| {
                let home = home_dir().unwrap_or_else(|_| PathBuf::from("."));
                entry.source.path == settings::normalize_path(source, &home).to_string_lossy()
            })
        })
        .filter(|entry| cli.line.is_none_or(|line| entry.source.line_start == line))
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [entry] => Ok(*entry),
        [] if cli.source.is_some() => Err(format!(
            "HOST_MISMATCH: selector `{selector}` does not match source and Host line"
        )),
        [] => Err(format!(
            "HOST_NOT_FOUND: selector `{selector}` matched no entries"
        )),
        many => Err(format_ambiguous(selector, many)),
    }
}
fn selected_connect_alias<'a>(
    entry: &'a HostEntry,
    positional: Option<&str>,
    cli: &Cli,
) -> &'a str {
    if cli.id.is_none()
        && let Some(selector) = positional
        && let Some(alias) = entry.aliases.iter().find(|alias| alias == &selector)
    {
        return alias;
    }
    entry
        .aliases
        .first()
        .map(String::as_str)
        .unwrap_or_default()
}

fn interactive_select<'a>(entries: &[&'a HostEntry]) -> Result<&'a HostEntry, String> {
    if entries.is_empty() {
        return Err("HOST_NOT_FOUND: no hosts match current filters".to_string());
    }
    eprintln!("Search hosts:");
    for (index, entry) in entries.iter().enumerate() {
        eprintln!(
            "  {}. {} ({})",
            index + 1,
            entry.aliases.join(" "),
            entry.source.path
        );
    }
    eprint!("Search: ");
    io::stderr()
        .flush()
        .map_err(|error| format!("cannot flush selector: {error}"))?;
    let mut query = String::new();
    io::stdin()
        .read_line(&mut query)
        .map_err(|error| format!("cannot read selector: {error}"))?;
    let query = query.trim();
    let matches = entries
        .iter()
        .copied()
        .filter(|entry| searchable(entry, query))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [entry] => Ok(*entry),
        [] => Err(format!(
            "HOST_NOT_FOUND: search `{query}` matched no entries"
        )),
        many => {
            eprintln!("Matches:");
            for (index, entry) in many.iter().enumerate() {
                eprintln!(
                    "  {}. {} ({})",
                    index + 1,
                    entry.aliases.join(" "),
                    entry.id
                );
            }
            eprint!("Select number: ");
            io::stderr()
                .flush()
                .map_err(|error| format!("cannot flush selector: {error}"))?;
            let mut selection = String::new();
            io::stdin()
                .read_line(&mut selection)
                .map_err(|error| format!("cannot read selector: {error}"))?;
            let index = selection
                .trim()
                .parse::<usize>()
                .map_err(|_| "HOST_REQUIRED: selector choice must be a number".to_string())?;
            many.get(index.saturating_sub(1))
                .copied()
                .ok_or_else(|| "HOST_NOT_FOUND: selector choice is out of range".to_string())
        }
    }
}

fn searchable(entry: &HostEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_ascii_lowercase();
    entry.id.to_ascii_lowercase().contains(&query)
        || entry
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().contains(&query))
        || entry.source.path.to_ascii_lowercase().contains(&query)
        || entry
            .projects
            .iter()
            .any(|project| project.to_ascii_lowercase().contains(&query))
        || entry
            .scopes
            .iter()
            .any(|scope| scope.to_ascii_lowercase().contains(&query))
}

fn format_ambiguous(selector: &str, entries: &[&HostEntry]) -> String {
    let candidates = entries
        .iter()
        .map(|entry| {
            format!(
                "{}: {} [{}]",
                entry.aliases.join(" "),
                entry.id,
                entry.source.path
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("HOST_AMBIGUOUS: selector `{selector}` matched candidates: {candidates}")
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --config PATH".to_string())
}

#[derive(Debug)]
enum Command {
    List,
    Show(String),
    Connect(Option<String>),
    Setup,
}

#[derive(Debug)]
struct RootRequest {
    scope: String,
    path: PathBuf,
    project: Option<String>,
}

#[derive(Debug)]
struct Cli {
    config: Option<PathBuf>,
    format: OutputFormat,
    command: Command,
    scopes: Vec<String>,
    projects: Vec<String>,
    source: Option<PathBuf>,
    line: Option<usize>,
    id: Option<String>,
    no_input: bool,
    roots: Vec<RootRequest>,
}

impl Cli {
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut config = None;
        let mut format = OutputFormat::Human;
        let mut scopes = Vec::new();
        let mut projects = Vec::new();
        let mut source = None;
        let mut line = None;
        let mut id = None;
        let mut host = None;
        let mut no_input = false;
        let mut roots = Vec::new();
        let mut positional = Vec::new();
        let mut index = 0;

        while index < args.len() {
            let argument = &args[index];
            let text = argument
                .to_str()
                .ok_or_else(|| "arguments must be valid UTF-8".to_string())?;
            let mut next = |name: &str| -> Result<String, String> {
                index += 1;
                args.get(index)
                    .and_then(|argument| argument.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{name} requires a value"))
            };
            if text == "--config" {
                config = Some(PathBuf::from(next(text)?));
            } else if let Some(value) = text.strip_prefix("--config=") {
                config = Some(PathBuf::from(value));
            } else if text == "--format" {
                format = OutputFormat::parse(&next(text)?)?;
            } else if let Some(value) = text.strip_prefix("--format=") {
                format = OutputFormat::parse(value)?;
            } else if text == "--scope" {
                scopes.push(next(text)?);
            } else if text == "--project" {
                projects.push(next(text)?);
            } else if text == "--source" || text == "--source-file" {
                source = Some(PathBuf::from(next(text)?));
            } else if text == "--line" || text == "--host-line" {
                line = Some(
                    next(text)?
                        .parse()
                        .map_err(|_| "--line requires a number".to_string())?,
                );
            } else if text == "--host" || text == "--alias" || text == "--selector" {
                host = Some(next(text)?);
            } else if text == "--id" || text == "--host-id" {
                id = Some(next(text)?);
            } else if text == "--no-input" || text == "--non-interactive" {
                no_input = true;
            } else if matches!(
                text,
                "--personal"
                    | "--personal-root"
                    | "--personal-config"
                    | "--combined"
                    | "--combined-root"
                    | "--combined-config"
            ) {
                roots.push(RootRequest {
                    scope: "personal".to_string(),
                    path: PathBuf::from(next(text)?),
                    project: projects.last().cloned(),
                });
            } else if matches!(text, "--work" | "--work-root" | "--work-config") {
                roots.push(RootRequest {
                    scope: "work".to_string(),
                    path: PathBuf::from(next(text)?),
                    project: projects.last().cloned(),
                });
            } else if text == "--root" {
                let value = next(text)?;
                let (scope, path) = value.split_once('=').map_or_else(
                    || {
                        (
                            scopes.last().map(String::as_str).unwrap_or("personal"),
                            value.as_str(),
                        )
                    },
                    |(scope, path)| (scope, path),
                );
                if scope.is_empty() || path.is_empty() {
                    return Err("--root requires SCOPE=PATH".to_string());
                }
                roots.push(RootRequest {
                    scope: scope.to_string(),
                    path: PathBuf::from(path),
                    project: projects.last().cloned(),
                });
            } else if text.starts_with('-') {
                return Err(format!("unexpected argument `{text}`\n{USAGE}"));
            } else {
                positional.push(text.to_string());
            }
            index += 1;
        }

        let command = match positional.as_slice() {
            [host, list] if host == "host" && list == "list" => Command::List,
            [host, show, selector] if host == "host" && show == "show" => {
                Command::Show(selector.clone())
            }
            [host, show] if host == "host" && show == "show" => {
                return Err("host show requires a selector".to_string());
            }
            [connect] if connect == "connect" => Command::Connect(None),
            [connect, selector] if connect == "connect" => Command::Connect(Some(selector.clone())),
            [setup] if setup == "setup" => Command::Setup,
            _ if positional.len() == 1 => {
                return Err(format!("unexpected argument `{}`\n{USAGE}", positional[0]));
            }
            _ => {
                return Err(format!(
                    "expected `host list`, `host show SELECTOR`, `connect`, or `setup`\n{HOST_USAGE}\n{SETUP_USAGE}"
                ));
            }
        };

        let command = match command {
            Command::Connect(Some(_selector)) if host.is_some() => {
                return Err("SELECTOR_CONFLICT: provide one host selector".to_string());
            }
            Command::Connect(selector) => Command::Connect(selector.or(host)),
            command => {
                if host.is_some() {
                    return Err("--host is only valid with connect".to_string());
                }
                command
            }
        };

        Ok(Self {
            config,
            format,
            command,
            scopes,
            projects,
            source,
            line,
            id,
            no_input,
            roots,
        })
    }
}

#[allow(dead_code)]
fn default_config() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".ssh/config"))
}
