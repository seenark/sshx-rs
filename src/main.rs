use serde::Serialize;
use sshx::discovery::{HostEntry, discover_roots, scope_for_path};
use sshx::mutation::{self, CreateRequest, MutationKind, UpdateRequest};
use sshx::output::{
    OutputFormat, render_create, render_diagnostic, render_edit, render_human, render_machine,
};
use sshx::settings::{self, RegisteredRoot};
use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process;
const USAGE: &str = "Usage: sshx [--version]";
const HOST_USAGE: &str = "Usage: sshx [--config PATH] host list [--format human|json|yaml]\n       sshx [--config PATH] host show SELECTOR [--format human|json|yaml]\n       sshx [--config PATH] host create --scope SCOPE --file PATH --alias ALIAS --hostname HOSTNAME [options]\n       sshx [--config PATH] host update SELECTOR [options]\n       sshx [--config PATH] host rename SELECTOR --alias ALIAS [options]\n       sshx [--config PATH] host delete SELECTOR [options]";
const SETUP_USAGE: &str = "Usage: sshx setup [--personal PATH] [--work PATH] [--project NAME]\n       sshx connect [SELECTOR] [--id ID] [--source PATH --line NUMBER] [--password-fd FD] [--no-input]";

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
    if matches!(&cli.command, Command::CreateHost) {
        return run_host_create(&cli, &roots);
    }
    if matches!(
        &cli.command,
        Command::UpdateHost(_) | Command::RenameHost(_) | Command::DeleteHost(_)
    ) {
        return run_host_edit(&cli, &roots);
    }
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
                sshx::connect::open_with_password_fd(
                    entry,
                    &home_dir()?,
                    cli.no_input,
                    alias,
                    cli.password_fd,
                )
            }
        }
        Command::Setup
        | Command::CreateHost
        | Command::UpdateHost(_)
        | Command::RenameHost(_)
        | Command::DeleteHost(_) => unreachable!(),
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

fn run_host_create(cli: &Cli, roots: &[RegisteredRoot]) -> Result<(), String> {
    let interactive = !cli.no_input && io::stdin().is_terminal();
    if cli.scopes.len() > 1 {
        return Err("SCOPE_AMBIGUOUS: provide one --scope".to_string());
    }
    if cli.projects.len() > 1 {
        return Err("PROJECT_AMBIGUOUS: provide one --project".to_string());
    }
    let scope = if let Some(scope) = cli.scopes.first() {
        scope.clone()
    } else if interactive {
        let mut scopes = roots
            .iter()
            .map(|root| root.scope.as_str())
            .collect::<Vec<_>>();
        scopes.sort_unstable();
        scopes.dedup();
        eprintln!("Scopes:");
        for (index, scope) in scopes.iter().enumerate() {
            eprintln!("  {}. {scope}", index + 1);
        }
        prompt_value("Scope: ", None)?
            .ok_or_else(|| "SCOPE_REQUIRED: scope cannot be empty".to_string())?
    } else {
        return Err("SCOPE_REQUIRED: provide --scope in non-interactive mode".to_string());
    };
    let requested_project = cli.projects.first().cloned();
    let mut candidates = roots
        .iter()
        .filter(|root| root.scope == scope)
        .filter(|root| {
            requested_project
                .as_deref()
                .is_none_or(|project| root.project.as_deref() == Some(project))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(format!(
            "ROOT_NOT_FOUND: no registered root matches scope `{scope}`{}",
            requested_project
                .as_deref()
                .map(|project| format!(" and project `{project}`"))
                .unwrap_or_default()
        ));
    }
    let root = if candidates.len() == 1 {
        candidates.remove(0)
    } else if interactive {
        eprintln!("Config roots:");
        for (index, root) in candidates.iter().enumerate() {
            eprintln!(
                "  {}. {}{}",
                index + 1,
                root.path.display(),
                root.project
                    .as_deref()
                    .map(|project| format!(" ({project})"))
                    .unwrap_or_default()
            );
        }
        let choice = prompt_value("Root number: ", None)?
            .ok_or_else(|| "ROOT_REQUIRED: select one config root".to_string())?
            .parse::<usize>()
            .map_err(|_| "ROOT_REQUIRED: root choice must be a number".to_string())?;
        *candidates
            .get(choice.saturating_sub(1))
            .ok_or_else(|| "ROOT_REQUIRED: root choice is out of range".to_string())?
    } else {
        return Err("ROOT_AMBIGUOUS: provide --project or one registered root".to_string());
    };
    let root_parent = root
        .path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let folder_text = match &cli.folder {
        Some(folder) => folder.to_string_lossy().into_owned(),
        None if interactive => prompt_value(
            &format!("Folder [{}]: ", root_parent.display()),
            Some(root_parent.to_string_lossy().into_owned()),
        )?
        .ok_or_else(|| "FOLDER_REQUIRED: folder cannot be empty".to_string())?,
        None => root_parent.to_string_lossy().into_owned(),
    };
    let folder = resolve_relative_path(&folder_text, root_parent);
    let file_text = match &cli.file {
        Some(file) => file.to_string_lossy().into_owned(),
        None if interactive => prompt_value("File: ", None)?
            .ok_or_else(|| "FILE_REQUIRED: provide a target file".to_string())?,
        None => {
            return Err("FILE_REQUIRED: provide --file in non-interactive mode".to_string());
        }
    };
    let target = resolve_relative_path(&file_text, &folder);
    let alias = required_create_value(cli.alias.as_deref(), "Alias", interactive)?;
    let hostname = required_create_value(cli.hostname.as_deref(), "Hostname", interactive)?;
    let user = optional_create_value(cli.user.as_deref(), "User", interactive)?;
    let port = match cli.port {
        Some(port) => Some(port),
        None if interactive => optional_create_value(None, "Port", true)?
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| "PORT_INVALID: port must be a number".to_string())
            })
            .transpose()?,
        None => None,
    };
    let password = create_password(cli, interactive)?;
    let request = CreateRequest::new(
        root.path.clone(),
        target,
        alias,
        hostname,
        user,
        port,
        password,
    );
    let plan = mutation::plan_create(&request)?;
    if cli.preview {
        print!("{}", render_create(&plan, cli.format, false)?);
        return Ok(());
    }
    if !cli.yes {
        if !interactive {
            return Err("CONSENT_REQUIRED: non-interactive host create requires --yes".to_string());
        }
        eprint!("{}", render_create(&plan, OutputFormat::Human, false)?);
        if !prompt_yes("Apply changes? [y/N]: ")? {
            return Err("MUTATION_DECLINED: host create was not applied".to_string());
        }
    }
    mutation::apply(&plan)?;
    print!("{}", render_create(&plan, cli.format, true)?);
    Ok(())
}

fn run_host_edit(cli: &Cli, roots: &[RegisteredRoot]) -> Result<(), String> {
    let configured = settings::discovery_roots(roots);
    mutation::validate_mutation_roots(&configured)?;
    let catalog = discover_roots(&configured).map_err(|error| error.to_string())?;
    for diagnostic in &catalog.diagnostics {
        eprintln!("{}", render_diagnostic(diagnostic));
    }
    let filtered = filter_entries(&catalog.entries, cli);
    let (operation, positional) = match &cli.command {
        Command::UpdateHost(selector) => (MutationKind::Update, selector.as_deref()),
        Command::RenameHost(selector) => (MutationKind::Rename, selector.as_deref()),
        Command::DeleteHost(selector) => (MutationKind::Delete, selector.as_deref()),
        _ => return Err("MUTATION_COMMAND: unsupported host mutation".to_string()),
    };
    if operation == MutationKind::Delete
        && (cli.alias.is_some()
            || cli.hostname.is_some()
            || cli.user.is_some()
            || cli.port.is_some()
            || cli.password_stdin
            || cli.clear_user
            || cli.clear_port
            || cli.clear_password)
    {
        return Err("MUTATION_FIELDS: delete does not accept host fields".to_string());
    }
    if operation == MutationKind::Rename
        && (cli.hostname.is_some()
            || cli.user.is_some()
            || cli.port.is_some()
            || cli.password_stdin
            || cli.clear_user
            || cli.clear_port
            || cli.clear_password)
    {
        return Err("MUTATION_FIELDS: rename accepts only --alias".to_string());
    }
    let entry = select_mutation_entry(&filtered, positional, cli)?;
    mutation::validate_entry_paths(entry)?;
    let selected_alias = selected_connect_alias(entry, positional, cli).to_string();
    let home = home_dir()?;
    let plan = match operation {
        MutationKind::Delete => {
            ensure_delete_allowed(&home, entry)?;
            mutation::plan_delete(
                PathBuf::from(&entry.source.path).as_path(),
                &entry.id,
                &selected_alias,
                entry.source.byte_start,
                entry.source.byte_end,
            )?
        }
        MutationKind::Update | MutationKind::Rename => {
            let password = if cli.password_stdin {
                create_password(cli, false)?
                    .ok_or_else(|| "PASSWORD_REQUIRED: password input is empty".to_string())?
            } else {
                String::new()
            };
            let mut plan = mutation::plan_update(&UpdateRequest {
                path: PathBuf::from(&entry.source.path),
                expected_id: entry.id.clone(),
                selected_alias,
                byte_start: entry.source.byte_start,
                byte_end: entry.source.byte_end,
                alias: cli.alias.clone(),
                hostname: cli.hostname.clone(),
                user: cli.user.clone(),
                port: cli.port,
                password: cli.password_stdin.then_some(password),
                clear_user: cli.clear_user,
                clear_port: cli.clear_port,
                clear_password: cli.clear_password,
            })?;
            plan.operation = operation;
            plan
        }
    };
    if cli.preview {
        print!("{}", render_edit(&plan, cli.format, false)?);
        return Ok(());
    }
    let interactive = !cli.no_input && io::stdin().is_terminal();
    if !cli.yes {
        if !interactive {
            return Err(
                "CONSENT_REQUIRED: non-interactive host mutation requires --yes".to_string(),
            );
        }
        eprint!("{}", render_edit(&plan, OutputFormat::Human, false)?);
        if !prompt_yes("Apply changes? [y/N]: ")? {
            return Err("MUTATION_DECLINED: host mutation was not applied".to_string());
        }
    }
    mutation::apply_edit(&plan)?;
    print!("{}", render_edit(&plan, cli.format, true)?);
    Ok(())
}

fn select_mutation_entry<'a>(
    entries: &[&'a HostEntry],
    positional: Option<&str>,
    cli: &Cli,
) -> Result<&'a HostEntry, String> {
    if positional.is_none() && cli.id.is_none() {
        return Err("HOST_REQUIRED: host mutation requires a selector".to_string());
    }
    select_connect_entry(entries, positional, cli)
}

fn ensure_delete_allowed(home: &std::path::Path, entry: &HostEntry) -> Result<(), String> {
    let id = &entry.id;
    let mut paths = entry
        .provenance
        .iter()
        .flat_map(|provenance| provenance.paths.iter().cloned())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let metadata_root = home.join(".config/sshx");
    collect_regular_files(&metadata_root, &mut paths);
    paths.sort();
    paths.dedup();
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if has_pair_reference(&text, id) {
            return Err(format!(
                "DELETE_REFERENCED: entry {} is referenced by {}",
                id,
                path.display()
            ));
        }
        if path.starts_with(&metadata_root) && has_active_reference(&text, id) {
            return Err(format!(
                "DELETE_ACTIVE: entry {} has active managed use in {}",
                id,
                path.display()
            ));
        }
    }
    Ok(())
}

fn collect_regular_files(path: &std::path::Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_regular_files(&entry.path(), files);
    }
}

fn has_pair_reference(text: &str, id: &str) -> bool {
    let normalized = text
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let id = id.to_ascii_lowercase();
    normalized.contains(&format!("gateway={id}"))
        || normalized.contains(&format!("gateway_id={id}"))
        || normalized.contains(&format!("gateway_id\":\"{id}\""))
        || normalized.contains(&format!("vm={id}"))
        || normalized.contains(&format!("vm_id={id}"))
        || normalized.contains(&format!("vm_id\":\"{id}\""))
}

fn has_active_reference(text: &str, id: &str) -> bool {
    let lower = text.to_ascii_lowercase().replace(char::is_whitespace, "");
    lower.contains(&format!("\"entry_id\":\"{}\"", id.to_ascii_lowercase()))
        && (lower.contains("\"state\":\"active\"")
            || lower.contains("\"state\":\"starting\"")
            || lower.contains("\"state\":\"stopping\""))
}

fn required_create_value(
    value: Option<&str>,
    label: &str,
    interactive: bool,
) -> Result<String, String> {
    if let Some(value) = value {
        return Ok(value.to_string());
    }
    if !interactive {
        return Err(format!(
            "{}_REQUIRED: provide --{} in non-interactive mode",
            label.to_ascii_uppercase(),
            label.to_ascii_lowercase()
        ));
    }
    prompt_value(&format!("{label}: "), None)?.ok_or_else(|| {
        format!(
            "{}_REQUIRED: value cannot be empty",
            label.to_ascii_uppercase()
        )
    })
}

fn optional_create_value(
    value: Option<&str>,
    label: &str,
    interactive: bool,
) -> Result<Option<String>, String> {
    if let Some(value) = value {
        return Ok(Some(value.to_string()));
    }
    if !interactive {
        return Ok(None);
    }
    prompt_value(&format!("{label} [optional]: "), Some(String::new()))
}

fn create_password(cli: &Cli, interactive: bool) -> Result<Option<String>, String> {
    if cli.password_stdin {
        if io::stdin().is_terminal() {
            return Err("PASSWORD_STDIN: password input must come from a pipe".to_string());
        }
        let mut password = String::new();
        io::stdin()
            .read_to_string(&mut password)
            .map_err(|error| format!("PASSWORD_STDIN: cannot read password: {error}"))?;
        let password = password.trim_end_matches(['\r', '\n']).to_string();
        return Ok((!password.is_empty()).then_some(password));
    }
    if interactive {
        prompt_password()
    } else {
        Ok(None)
    }
}

#[cfg(unix)]
fn prompt_password() -> Result<Option<String>, String> {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    eprint!("Password [optional]: ");
    io::stderr()
        .flush()
        .map_err(|error| format!("cannot flush prompt: {error}"))?;
    let fd = io::stdin().as_raw_fd();
    let mut original = MaybeUninit::uninit();
    let hidden = unsafe {
        if libc::tcgetattr(fd, original.as_mut_ptr()) != 0 {
            return prompt_value("", Some(String::new()));
        }
        let original = original.assume_init();
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if libc::tcsetattr(fd, libc::TCSANOW, &hidden) != 0 {
            return prompt_value("", Some(String::new()));
        }
        (original, hidden)
    };
    let mut value = String::new();
    let read_result = io::stdin().read_line(&mut value);
    unsafe {
        let _ = libc::tcsetattr(fd, libc::TCSANOW, &hidden.0);
    }
    eprintln!();
    read_result.map_err(|error| format!("cannot read password: {error}"))?;
    let value = value.trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}

#[cfg(not(unix))]
fn prompt_password() -> Result<Option<String>, String> {
    prompt_value("Password [optional]: ", Some(String::new()))
}

fn prompt_value(prompt: &str, default: Option<String>) -> Result<Option<String>, String> {
    eprint!("{prompt}");
    io::stderr()
        .flush()
        .map_err(|error| format!("cannot flush prompt: {error}"))?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| format!("cannot read prompt: {error}"))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        Ok(default.filter(|default| !default.is_empty()))
    } else {
        Ok(Some(value))
    }
}

fn prompt_yes(prompt: &str) -> Result<bool, String> {
    Ok(prompt_value(prompt, None)?
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "y" | "yes")))
}

fn resolve_relative_path(value: &str, base: &std::path::Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
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
    CreateHost,
    UpdateHost(Option<String>),
    RenameHost(Option<String>),
    DeleteHost(Option<String>),
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
    alias: Option<String>,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    password_stdin: bool,
    password_fd: Option<i32>,
    clear_user: bool,
    clear_port: bool,
    clear_password: bool,
    folder: Option<PathBuf>,
    file: Option<PathBuf>,
    yes: bool,
    preview: bool,
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
        let mut alias = None;
        let mut hostname = None;
        let mut user = None;
        let mut port = None;
        let mut clear_user = false;
        let mut clear_port = false;
        let mut clear_password = false;
        let mut password_stdin = false;
        let mut password_fd = None;
        let mut folder = None;
        let mut file = None;
        let mut yes = false;
        let mut preview = false;
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
            } else if text == "--host" || text == "--selector" {
                host = Some(next(text)?);
            } else if text == "--alias" {
                alias = Some(next(text)?);
            } else if text == "--hostname" {
                hostname = Some(next(text)?);
            } else if text == "--user" {
                user = Some(next(text)?);
            } else if text == "--port" {
                port = Some(
                    next(text)?
                        .parse()
                        .map_err(|_| "--port requires a number".to_string())?,
                );
            } else if text == "--password-stdin" {
                password_stdin = true;
            } else if text == "--password-fd" {
                password_fd = Some(
                    next(text)?
                        .parse()
                        .map_err(|_| "--password-fd requires a number".to_string())?,
                );
            } else if let Some(value) = text.strip_prefix("--password-fd=") {
                password_fd = Some(
                    value
                        .parse()
                        .map_err(|_| "--password-fd requires a number".to_string())?,
                );
            } else if text == "--clear-user" {
                clear_user = true;
            } else if text == "--clear-port" {
                clear_port = true;
            } else if text == "--clear-password" {
                clear_password = true;
            } else if text == "--folder" {
                folder = Some(PathBuf::from(next(text)?));
            } else if text == "--file" || text == "--target-file" {
                file = Some(PathBuf::from(next(text)?));
            } else if text == "--yes" {
                yes = true;
            } else if text == "--preview" || text == "--dry-run" {
                preview = true;
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
            [host, create] if host == "host" && create == "create" => Command::CreateHost,
            [host, update, selector] if host == "host" && update == "update" => {
                Command::UpdateHost(Some(selector.clone()))
            }
            [host, update] if host == "host" && update == "update" => Command::UpdateHost(None),
            [host, rename, selector] if host == "host" && rename == "rename" => {
                Command::RenameHost(Some(selector.clone()))
            }
            [host, rename] if host == "host" && rename == "rename" => Command::RenameHost(None),
            [host, delete, selector] if host == "host" && delete == "delete" => {
                Command::DeleteHost(Some(selector.clone()))
            }
            [host, delete] if host == "host" && delete == "delete" => Command::DeleteHost(None),
            [connect] if connect == "connect" => Command::Connect(None),
            [connect, selector] if connect == "connect" => Command::Connect(Some(selector.clone())),
            [setup] if setup == "setup" => Command::Setup,
            _ if positional.len() == 1 => {
                return Err(format!("unexpected argument `{}`\n{USAGE}", positional[0]));
            }
            _ => {
                return Err(format!(
                    "expected `host list`, `host show SELECTOR`, `host create`, `connect`, or `setup`\n{HOST_USAGE}\n{SETUP_USAGE}"
                ));
            }
        };
        let command = match command {
            Command::Connect(Some(_selector)) if host.is_some() || alias.is_some() => {
                return Err("SELECTOR_CONFLICT: provide one host selector".to_string());
            }
            Command::Connect(selector) => {
                Command::Connect(selector.or(host).or_else(|| alias.clone()))
            }
            Command::CreateHost => {
                if host.is_some() || id.is_some() || source.is_some() || line.is_some() {
                    return Err(
                        "SELECTOR_CONFLICT: host create does not accept host selectors".to_string(),
                    );
                }
                Command::CreateHost
            }
            Command::UpdateHost(selector) => {
                if host.is_some() && selector.is_some() {
                    return Err("SELECTOR_CONFLICT: provide one host selector".to_string());
                }
                Command::UpdateHost(selector.or(host))
            }
            Command::RenameHost(selector) => {
                if host.is_some() && selector.is_some() {
                    return Err("SELECTOR_CONFLICT: provide one host selector".to_string());
                }
                Command::RenameHost(selector.or(host))
            }
            Command::DeleteHost(selector) => {
                if host.is_some() && selector.is_some() {
                    return Err("SELECTOR_CONFLICT: provide one host selector".to_string());
                }
                Command::DeleteHost(selector.or(host))
            }
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
            alias,
            hostname,
            user,
            port,
            password_stdin,
            password_fd,
            clear_user,
            clear_port,
            clear_password,
            folder,
            file,
            yes,
            preview,
            no_input,
            roots,
        })
    }
}

#[allow(dead_code)]
fn default_config() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".ssh/config"))
}
