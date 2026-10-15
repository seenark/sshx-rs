use crate::discovery::HostEntry;
use crate::settings::{self, Preferences};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Hidden subcommand the detached auto-clear helper is started with.
pub const CLEAR_COMMAND: &str = "__clipboard-clear";

#[derive(Clone, Copy)]
struct Tool {
    program: &'static str,
    args: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct Backend {
    copy: Tool,
    paste: Tool,
}

#[cfg(target_os = "macos")]
const BACKENDS: &[Backend] = &[Backend {
    copy: Tool {
        program: "pbcopy",
        args: &[],
    },
    paste: Tool {
        program: "pbpaste",
        args: &[],
    },
}];
#[cfg(target_os = "linux")]
const BACKENDS: &[Backend] = &[
    Backend {
        copy: Tool {
            program: "wl-copy",
            args: &[],
        },
        paste: Tool {
            program: "wl-paste",
            args: &["-n"],
        },
    },
    Backend {
        copy: Tool {
            program: "xclip",
            args: &["-selection", "clipboard"],
        },
        paste: Tool {
            program: "xclip",
            args: &["-selection", "clipboard", "-o"],
        },
    },
    Backend {
        copy: Tool {
            program: "xsel",
            args: &["--clipboard", "--input"],
        },
        paste: Tool {
            program: "xsel",
            args: &["--clipboard", "--output"],
        },
    },
];
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const BACKENDS: &[Backend] = &[];

fn on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|directory| directory.join(program).is_file())
    })
}

fn backend() -> Option<Backend> {
    BACKENDS
        .iter()
        .copied()
        .find(|backend| on_path(backend.copy.program))
}

pub fn clipboard_available() -> bool {
    backend().is_some()
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn local_forward(bind: &str, remote: u16, local: u16) -> String {
    let bind = if bind.contains(':') {
        format!("[{bind}]")
    } else {
        bind.to_string()
    };
    format!(" -L {bind}:{local}:127.0.0.1:{remote}")
}

/// `forwards` holds `(remote, local)` pairs; `bind` is `Preferences.bind`.
pub fn ssh_command(config: &str, alias: &str, bind: &str, forwards: &[(u16, u16)]) -> String {
    ssh_line("ssh", config, alias, bind, forwards)
}

pub fn ssh_verbose_command(
    config: &str,
    alias: &str,
    bind: &str,
    forwards: &[(u16, u16)],
) -> String {
    ssh_line("ssh -v", config, alias, bind, forwards)
}

fn ssh_line(
    program: &str,
    config: &str,
    alias: &str,
    bind: &str,
    forwards: &[(u16, u16)],
) -> String {
    let mut command = format!(
        "{program} -F {} {}",
        shell_quote(config),
        shell_quote(alias)
    );
    for &(remote, local) in forwards {
        command.push_str(&local_forward(bind, remote, local));
    }
    command
}

/// The Pair-capable form; `forwards` holds `(remote, local)` pairs.
pub fn sshx_command(
    alias: &str,
    id: &str,
    config: Option<&str>,
    source: Option<(&str, usize)>,
    forwards: &[(u16, u16)],
) -> String {
    let mut command = format!(
        "sshx connect {} --id {}",
        shell_quote(alias),
        shell_quote(id)
    );
    if let Some(config) = config {
        command.push_str(&format!(" --config {}", shell_quote(config)));
    }
    if let Some((path, line)) = source {
        command.push_str(&format!(" --source {} --line {line}", shell_quote(path)));
    }
    for &(remote, local) in forwards {
        command.push_str(&format!(" --forward {remote}={local}"));
    }
    command
}

/// Leading space keeps the line out of history in shells that ignore space-prefixed commands.
pub fn sshpass_command(
    password: &str,
    config: &str,
    alias: &str,
    bind: &str,
    forwards: &[(u16, u16)],
) -> String {
    format!(
        " SSHPASS={} sshpass -e {}",
        shell_quote(password),
        ssh_command(config, alias, bind, forwards)
    )
}

/// Direct HostEntries with a stored password only.
pub fn sshpass_command_for_entry(
    entries: &[HostEntry],
    entry: &HostEntry,
    config: &str,
    alias: &str,
    bind: &str,
    forwards: &[(u16, u16)],
) -> Result<String, String> {
    if crate::pair::paired_route(entries, entry)?.is_some() {
        return Err(
            "ACTION_UNAVAILABLE: sshpass command is unavailable for Pair routes".to_string(),
        );
    }
    let password = crate::connect::stored_password(entry, false)?.ok_or_else(|| {
        "ACTION_UNAVAILABLE: selected HostEntry has no stored password".to_string()
    })?;
    Ok(sshpass_command(&password, config, alias, bind, forwards))
}

fn write_clipboard(tool: Tool, content: &str) -> Result<(), String> {
    let mut child = Command::new(tool.program)
        .args(tool.args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("CLIPBOARD_FAILED: cannot start {}: {error}", tool.program))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "CLIPBOARD_FAILED: clipboard stdin is unavailable".to_string())?
        .write_all(content.as_bytes())
        .map_err(|error| format!("CLIPBOARD_FAILED: cannot write clipboard content: {error}"))?;
    let status = child.wait().map_err(|error| {
        format!(
            "CLIPBOARD_FAILED: cannot wait for {}: {error}",
            tool.program
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "CLIPBOARD_FAILED: {} exited with {status}",
            tool.program
        ))
    }
}

fn read_clipboard(tool: Tool) -> Option<Vec<u8>> {
    let output = Command::new(tool.program)
        .args(tool.args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn publish_command(command: &str) -> Result<String, String> {
    let backend = backend().ok_or_else(|| {
        "CLIPBOARD_UNAVAILABLE: no supported clipboard backend; command not copied".to_string()
    })?;
    write_clipboard(backend.copy, command)?;
    Ok(format!("Copied command to {}.", backend.copy.program))
}

/// Copies a password or sshpass command, then schedules the clear from `preferences`.
pub fn publish_secret(
    secret: &str,
    label: &str,
    preferences: &Preferences,
) -> Result<String, String> {
    let backend = backend().ok_or_else(|| {
        format!("CLIPBOARD_UNAVAILABLE: no supported clipboard backend; {label} not copied")
    })?;
    write_clipboard(backend.copy, secret)?;
    let program = backend.copy.program;
    let Some(seconds) = preferences.clipboard_clear else {
        return Ok(format!("Copied {label} to {program}."));
    };
    if on_path(backend.paste.program)
        && spawn_clear_helper(seconds, content_hash(secret.as_bytes())).is_ok()
    {
        Ok(format!(
            "Copied {label} to {program}; clears in {seconds}s."
        ))
    } else {
        Ok(format!(
            "Copied {label} to {program}; cannot auto-clear without a clipboard readback tool."
        ))
    }
}

fn spawn_clear_helper(seconds: u64, hash: u64) -> io::Result<()> {
    let mut command = Command::new(env::current_exe()?);
    command
        .arg(CLEAR_COMMAND)
        .arg(seconds.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and runs between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{hash:x}")?;
    }
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Detached helper body: the hash arrives on stdin so it never appears in argv.
pub fn run_clear_helper(seconds: &str) -> Result<(), String> {
    let seconds: u64 = seconds
        .parse()
        .map_err(|_| format!("{CLEAR_COMMAND}: invalid seconds `{seconds}`"))?;
    let mut line = String::new();
    io::stdin()
        .read_to_string(&mut line)
        .map_err(|error| format!("{CLEAR_COMMAND}: cannot read hash: {error}"))?;
    let hash = u64::from_str_radix(line.trim(), 16)
        .map_err(|_| format!("{CLEAR_COMMAND}: invalid hash"))?;
    thread::sleep(Duration::from_secs(seconds));
    let Some(backend) = backend().filter(|backend| on_path(backend.paste.program)) else {
        return Ok(());
    };
    if read_clipboard(backend.paste).is_some_and(|content| content_hash(&content) == hash) {
        write_clipboard(backend.copy, "")?;
    }
    Ok(())
}

pub const PASSWORD_WARNING: &str = "Clipboard history may retain this password; sshx clears the clipboard itself only while the setting `clipboard_clear` is on.";

/// Persists the first-copy acknowledgment so the warning is not shown again.
pub fn acknowledge_password_warning(
    home: &Path,
    preferences: &mut Preferences,
) -> Result<(), String> {
    preferences.password_warning_acknowledged = true;
    settings::save_preferences(home, preferences)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_command_appends_local_forwards_with_bind() {
        assert_eq!(
            ssh_command("/c/config", "web", "127.0.0.1", &[]),
            "ssh -F '/c/config' 'web'"
        );
        assert_eq!(
            ssh_command(
                "/c/config",
                "web",
                "127.0.0.1",
                &[(5432, 15432), (80, 8080)]
            ),
            "ssh -F '/c/config' 'web' -L 127.0.0.1:15432:127.0.0.1:5432 -L 127.0.0.1:8080:127.0.0.1:80"
        );
        assert_eq!(
            ssh_command("/c", "web", "::1", &[(80, 8080)]),
            "ssh -F '/c' 'web' -L [::1]:8080:127.0.0.1:80"
        );
    }

    #[test]
    fn verbose_command_adds_v_before_config_and_keeps_forwards() {
        assert_eq!(
            ssh_verbose_command("/c/it's", "web", "127.0.0.1", &[]),
            "ssh -v -F '/c/it'\\''s' 'web'"
        );
        assert_eq!(
            ssh_verbose_command("/c", "web", "127.0.0.1", &[(5432, 15432)]),
            "ssh -v -F '/c' 'web' -L 127.0.0.1:15432:127.0.0.1:5432"
        );
    }

    #[test]
    fn sshx_command_carries_identity_source_and_forwards() {
        assert_eq!(
            sshx_command("vm", "id-1", None, None, &[]),
            "sshx connect 'vm' --id 'id-1'"
        );
        assert_eq!(
            sshx_command(
                "vm",
                "id-1",
                Some("/c"),
                Some(("/c/config", 7)),
                &[(5432, 15432)]
            ),
            "sshx connect 'vm' --id 'id-1' --config '/c' --source '/c/config' --line 7 --forward 5432=15432"
        );
    }

    #[test]
    fn sshpass_command_starts_with_space_and_quotes_password() {
        assert_eq!(
            sshpass_command("pa'ss w", "/c", "web", "127.0.0.1", &[(80, 8080)]),
            " SSHPASS='pa'\\''ss w' sshpass -e ssh -F '/c' 'web' -L 127.0.0.1:8080:127.0.0.1:80"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backends_target_clipboard_selection() {
        let xclip = BACKENDS
            .iter()
            .find(|backend| backend.copy.program == "xclip")
            .expect("xclip backend should exist");
        assert_eq!(xclip.copy.args, &["-selection", "clipboard"]);
        assert_eq!(xclip.paste.args, &["-selection", "clipboard", "-o"]);
        let xsel = BACKENDS
            .iter()
            .find(|backend| backend.copy.program == "xsel")
            .expect("xsel backend should exist");
        assert_eq!(xsel.copy.args, &["--clipboard", "--input"]);
        assert_eq!(xsel.paste.args, &["--clipboard", "--output"]);
    }
}
