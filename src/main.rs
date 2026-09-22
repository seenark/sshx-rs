use sshx::discovery::{HostEntry, discover};
use sshx::output::{OutputFormat, render_diagnostic, render_human, render_machine};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process;

const USAGE: &str = "Usage: sshx [--version]";
const HOST_USAGE: &str = "Usage: sshx [--config PATH] host list [--format human|json|yaml]\n       sshx [--config PATH] host show SELECTOR [--format human|json|yaml]";

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
    let catalog = discover(&cli.config).map_err(|error| error.to_string())?;
    for diagnostic in &catalog.diagnostics {
        eprintln!("{}", render_diagnostic(diagnostic));
    }

    let entries = match cli.command {
        Command::List => catalog.entries.iter().collect::<Vec<_>>(),
        Command::Show(selector) => select_entries(&catalog.entries, &selector)?,
    };

    if cli.format.is_machine() {
        let document = render_machine(entries, &catalog.diagnostics, cli.format)?;
        print!("{document}");
    } else {
        print!("{}", render_human(&entries));
    }
    Ok(())
}

fn select_entries<'a>(
    entries: &'a [HostEntry],
    selector: &str,
) -> Result<Vec<&'a HostEntry>, String> {
    let selected = entries
        .iter()
        .filter(|entry| entry.id == selector || entry.aliases.iter().any(|alias| alias == selector))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(format!("host selector `{selector}` matched no entries"));
    }
    Ok(selected)
}

#[derive(Debug)]
enum Command {
    List,
    Show(String),
}

#[derive(Debug)]
struct Cli {
    config: PathBuf,
    format: OutputFormat,
    command: Command,
}

impl Cli {
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut config = None;
        let mut format = OutputFormat::Human;
        let mut positional = Vec::new();
        let mut index = 0;

        while index < args.len() {
            let argument = &args[index];
            let text = argument
                .to_str()
                .ok_or_else(|| "arguments must be valid UTF-8".to_string())?;
            if text == "--config" {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{text} requires a path"))?;
                config = Some(PathBuf::from(value));
            } else if let Some(value) = text.strip_prefix("--config=") {
                if value.is_empty() {
                    return Err("--config requires a path".to_string());
                }
                config = Some(PathBuf::from(value));
            } else if text == "--format" {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|argument| argument.to_str())
                    .ok_or_else(|| format!("{text} requires human, json, or yaml"))?;
                format = OutputFormat::parse(value)?;
            } else if let Some(value) = text.strip_prefix("--format=") {
                format = OutputFormat::parse(value)?;
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
            _ if positional.len() == 1 => {
                return Err(format!("unexpected argument `{}`\n{USAGE}", positional[0]));
            }
            _ => {
                return Err(format!(
                    "expected `host list` or `host show SELECTOR`\n{HOST_USAGE}"
                ));
            }
        };

        let config = match config {
            Some(path) => path,
            None => default_config()?,
        };
        Ok(Self {
            config,
            format,
            command,
        })
    }
}

fn default_config() -> Result<PathBuf, String> {
    let home =
        env::var_os("HOME").ok_or_else(|| "HOME is not set; pass --config PATH".to_string())?;
    Ok(PathBuf::from(home).join(".ssh/config"))
}
