use crate::discovery::HostEntry;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const SYSTEM_CONFIG: &str = "/etc/ssh/ssh_config";
const MASTER_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

static SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn signal_handler(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            signal_handler as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {}

#[cfg(unix)]
fn reset_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::signal(libc::SIGHUP, libc::SIG_DFL);
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_signal_handlers() {}

fn received_signal() -> Option<i32> {
    match SIGNAL.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}

struct Runtime {
    dir: PathBuf,
    config: PathBuf,
    socket: PathBuf,
    alias: String,
    known_hosts: PathBuf,
}

impl Runtime {
    fn create(entry: &HostEntry, home: &Path, selected_alias: &str) -> Result<Self, String> {
        if !entry.aliases.iter().any(|alias| alias == selected_alias) {
            return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
        }
        let content = compile_config(entry, selected_alias)?;
        let dir = temporary_directory()?;
        let config = dir.join("config");
        let socket = dir.join("master.sock");
        let known_hosts = known_hosts_path(home, entry);
        ensure_known_hosts_parent(&known_hosts)?;
        write_private_file(&config, content.as_bytes())?;
        Ok(Self {
            dir,
            config,
            socket,
            alias: selected_alias.to_string(),
            known_hosts,
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn open(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
) -> Result<(), String> {
    SIGNAL.store(0, Ordering::Relaxed);
    install_signal_handlers();
    let result = open_session(entry, home, no_input, selected_alias);
    reset_signal_handlers();
    result
}

fn open_session(
    entry: &HostEntry,
    home: &Path,
    no_input: bool,
    selected_alias: &str,
) -> Result<(), String> {
    let runtime = Runtime::create(entry, home, selected_alias)?;
    let mut master = spawn_master(&runtime, no_input)?;
    let ready = wait_for_master(&runtime, &mut master, no_input);
    if let Err(error) = ready {
        stop_master(&runtime, &mut master);
        if !no_input && error.starts_with("HOST_KEY_TRUST_REQUIRED") {
            enroll_host_key(&runtime)?;
            master = spawn_master(&runtime, no_input)?;
            if let Err(error) = wait_for_master(&runtime, &mut master, no_input) {
                stop_master(&runtime, &mut master);
                return Err(error);
            }
        } else {
            return Err(error);
        }
    }

    let mut shell = match spawn_shell(&runtime, no_input) {
        Ok(shell) => shell,
        Err(error) => {
            stop_master(&runtime, &mut master);
            return Err(error);
        }
    };
    let status = wait_for_shell(&mut shell)?;
    stop_master(&runtime, &mut master);
    if let Some(signal) = received_signal() {
        return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("SESSION_EXIT: {status}"))
    }
}

fn spawn_master(runtime: &Runtime, no_input: bool) -> Result<Child, String> {
    let mut command = ssh_command(runtime, no_input, true);
    command.args(["-M", "-N", "-o", "ControlMaster=yes"]);
    command.arg(&runtime.alias);
    command
        .stdin(if no_input {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
        .spawn()
        .map_err(|error| format!("SSH_AUTH_FAILED: cannot start OpenSSH: {error}"))
}

fn spawn_shell(runtime: &Runtime, no_input: bool) -> Result<Child, String> {
    let mut command = ssh_command(runtime, no_input, true);
    command.args(["-o", "ControlMaster=no"]);
    command.arg(&runtime.alias);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .map_err(|error| format!("SESSION_FAILED: cannot start shell: {error}"))
}

fn enroll_host_key(runtime: &Runtime) -> Result<(), String> {
    let before = fs::read(&runtime.known_hosts).unwrap_or_default();
    let mut command = ssh_command(runtime, false, false);
    command.args([
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "PreferredAuthentications=none",
        "-o",
        "PubkeyAuthentication=no",
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        "NumberOfPasswordPrompts=0",
        "-o",
        "RequestTTY=force",
        "-o",
        "ConnectTimeout=10",
    ]);
    command.arg(&runtime.alias);
    let _ = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("HOST_KEY_TRUST_REQUIRED: cannot enroll host key: {error}"))?;
    let after = fs::read(&runtime.known_hosts).unwrap_or_default();
    if after != before {
        Ok(())
    } else {
        Err("HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string())
    }
}

fn ssh_command(runtime: &Runtime, no_input: bool, strict: bool) -> Command {
    let mut command = Command::new("ssh");
    command.args(["-F", runtime.config.to_string_lossy().as_ref()]);
    command.args(["-S", runtime.socket.to_string_lossy().as_ref()]);
    let known_hosts = format!("UserKnownHostsFile={}", runtime.known_hosts.display());
    command.args([
        "-o",
        "ControlPersist=no",
        "-o",
        if no_input {
            "BatchMode=yes"
        } else {
            "BatchMode=no"
        },
        "-o",
        if strict {
            "StrictHostKeyChecking=yes"
        } else {
            "StrictHostKeyChecking=ask"
        },
        "-o",
        "NoHostAuthenticationForLocalhost=no",
        "-o",
        "CheckHostIP=no",
        "-o",
        "GlobalKnownHostsFile=/dev/null",
        "-o",
        "KnownHostsCommand=none",
        "-o",
        "VerifyHostKeyDNS=no",
        "-o",
        "UpdateHostKeys=no",
        "-o",
        known_hosts.as_str(),
        "-o",
        "PreferredAuthentications=publickey",
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
    ]);
    command
}

fn wait_for_master(runtime: &Runtime, master: &mut Child, no_input: bool) -> Result<(), String> {
    let started = SystemTime::now();
    loop {
        if let Some(signal) = received_signal() {
            return Err(format!("SESSION_INTERRUPTED: signal {signal}"));
        }
        if let Some(status) = master
            .try_wait()
            .map_err(|error| format!("SSH_AUTH_FAILED: cannot inspect OpenSSH: {error}"))?
        {
            let stderr = read_child_stderr(master);
            if !no_input && !stderr.is_empty() {
                eprint!("{stderr}");
            }
            return Err(classify_master_failure(status, &stderr, no_input));
        }
        if control_master_ready(runtime) {
            return Ok(());
        }
        if started.elapsed().unwrap_or_default().ge(&MASTER_TIMEOUT) {
            return Err(
                "SSH_AUTH_TIMEOUT: OpenSSH authentication did not become ready".to_string(),
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_shell(shell: &mut Child) -> Result<ExitStatus, String> {
    loop {
        if received_signal().is_some() {
            let _ = shell.kill();
            let _ = shell.wait();
            return shell
                .try_wait()
                .map_err(|error| format!("SESSION_FAILED: cannot inspect shell: {error}"))?
                .ok_or_else(|| "SESSION_INTERRUPTED: shell did not stop".to_string());
        }
        if let Some(status) = shell
            .try_wait()
            .map_err(|error| format!("SESSION_FAILED: cannot inspect shell: {error}"))?
        {
            return Ok(status);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn control_master_ready(runtime: &Runtime) -> bool {
    let status = Command::new("ssh")
        .args(["-F", runtime.config.to_string_lossy().as_ref()])
        .args(["-S", runtime.socket.to_string_lossy().as_ref()])
        .args(["-O", "check", &runtime.alias])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    status.is_ok_and(|status| status.success())
}

fn stop_master(runtime: &Runtime, master: &mut Child) {
    let _ = Command::new("ssh")
        .args(["-F", runtime.config.to_string_lossy().as_ref()])
        .args(["-S", runtime.socket.to_string_lossy().as_ref()])
        .args(["-O", "exit", &runtime.alias])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if master.try_wait().ok().flatten().is_none() {
        let _ = master.kill();
    }
    let _ = master.wait();
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut output = String::new();
    let _ = stderr.read_to_string(&mut output);
    output
}

fn classify_master_failure(status: ExitStatus, stderr: &str, no_input: bool) -> String {
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || stderr.contains("Offending ")
        || stderr.contains("host key for .* has changed")
    {
        return "HOST_KEY_CHANGED: remote host key changed; refusing replacement".to_string();
    }
    if stderr.contains("Host key verification failed")
        || stderr.contains("The authenticity of host")
        || stderr.contains("Are you sure you want to continue connecting")
    {
        return "HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string();
    }
    if no_input && stderr.to_ascii_lowercase().contains("host key") {
        return "HOST_KEY_TRUST_REQUIRED: host key requires interactive confirmation".to_string();
    }
    let detail = stderr.lines().last().unwrap_or_default().trim();
    if detail.is_empty() {
        format!("SSH_AUTH_FAILED: OpenSSH exited with {status}")
    } else {
        format!("SSH_AUTH_FAILED: {detail}")
    }
}

fn compile_config(entry: &HostEntry, selected_alias: &str) -> Result<String, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "CONFIG_INVALID: SSH config is not valid UTF-8".to_string())?;
    let start = entry.source.byte_start;
    let end = entry.source.byte_end;
    if start >= end || end > text.len() {
        return Err("CONFIG_INVALID: selected Host span is outside source file".to_string());
    }
    validate_source(text, start, entry)?;
    let block = &text[start..end];
    let mut output = String::new();
    let mut host_written = false;
    for raw in block.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (keyword, argument) = split_directive(line)?;
        if keyword.eq_ignore_ascii_case("host") {
            if host_written {
                return Err(
                    "CONFIG_INVALID: selected span contains multiple Host lines".to_string()
                );
            }
            let aliases = tokenize(argument);
            if aliases.is_empty() || aliases.iter().any(|alias| wildcard(alias)) {
                return Err(
                    "UNSUPPORTED_WILDCARD: selected Host block is not an exact direct host"
                        .to_string(),
                );
            }
            if !aliases.iter().any(|alias| alias == selected_alias) {
                return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
            }
            output.push_str("Host ");
            output.push_str(selected_alias);
            output.push('\n');
            host_written = true;
            continue;
        }
        if keyword.eq_ignore_ascii_case("include") {
            return Err("UNSUPPORTED_CONDITIONAL_INCLUDE: Include inside Host block".to_string());
        }
        if keyword.eq_ignore_ascii_case("match") {
            return Err(
                "UNSUPPORTED_MATCH: Match cannot be preserved in exact Host config".to_string(),
            );
        }
        if argument.contains('%') {
            return Err(format!(
                "UNSUPPORTED_TOKEN_SEMANTICS: {keyword} contains token expansion"
            ));
        }
        if !supported_directive(keyword) {
            return Err(format!(
                "UNSUPPORTED_DIRECTIVE: {keyword} cannot be preserved in direct runtime config"
            ));
        }
        output.push_str("  ");
        output.push_str(keyword);
        if !argument.is_empty() {
            output.push(' ');
            output.push_str(argument);
        }
        output.push('\n');
    }
    if !host_written {
        return Err("CONFIG_INVALID: selected span does not start with Host".to_string());
    }
    output.push_str("Include ");
    output.push_str(SYSTEM_CONFIG);
    output.push('\n');
    Ok(output)
}

fn validate_source(text: &str, start: usize, entry: &HostEntry) -> Result<(), String> {
    let mut offset = 0;
    let mut saw_host = false;
    let mut found_selected = false;
    for raw in text.split_inclusive('\n') {
        let line_start = offset;
        offset += raw.len();
        let line = strip_inline_comment(raw.trim_end_matches(['\n', '\r'])).trim();
        if line.is_empty() {
            continue;
        }
        let (keyword, argument) = split_directive(line)?;
        if keyword.eq_ignore_ascii_case("match") {
            return Err(
                "UNSUPPORTED_MATCH: Match cannot be preserved in exact Host config".to_string(),
            );
        }
        if keyword.eq_ignore_ascii_case("host") {
            saw_host = true;
            let aliases = tokenize(argument);
            if aliases.iter().any(|alias| wildcard(alias)) {
                return Err(
                    "UNSUPPORTED_WILDCARD: wildcard Host cannot be preserved exactly".to_string(),
                );
            }
            if aliases.iter().any(|alias| alias.contains('%')) {
                return Err(
                    "UNSUPPORTED_TOKEN_SEMANTICS: Host alias contains token expansion".to_string(),
                );
            }
            if line_start == start {
                found_selected = true;
                if aliases != entry.aliases {
                    return Err("CONFIG_CHANGED: selected Host block changed on disk".to_string());
                }
            }
            continue;
        }
        if keyword.eq_ignore_ascii_case("include") {
            if saw_host {
                return Err(
                    "UNSUPPORTED_CONDITIONAL_INCLUDE: Include inside Host block".to_string()
                );
            }
            continue;
        }
        if !saw_host {
            return Err(format!(
                "UNSUPPORTED_GLOBAL: {keyword} before selected Host block"
            ));
        }
    }
    if !found_selected {
        return Err("CONFIG_CHANGED: selected Host block no longer exists".to_string());
    }
    Ok(())
}

fn supported_directive(keyword: &str) -> bool {
    matches!(
        keyword.to_ascii_lowercase().as_str(),
        "addkeystoagent"
            | "addressfamily"
            | "batchmode"
            | "bindaddress"
            | "bindinterface"
            | "canonicaldomains"
            | "canonicalizemaxdots"
            | "canonicalizehostname"
            | "canonicalizefallbacklocal"
            | "certificatefile"
            | "ciphers"
            | "compression"
            | "connecttimeout"
            | "connectionattempts"
            | "disableforwarding"
            | "enableescapecommandline"
            | "escapechar"
            | "exitonforwardfailure"
            | "fingerprinthash"
            | "forwardagent"
            | "forwardx11"
            | "forwardx11timeout"
            | "forwardx11trusted"
            | "gatewayports"
            | "gssapiauthentication"
            | "gssapicleanupcredentials"
            | "gssapiclientidentity"
            | "gssapidelegatecredentials"
            | "gssapikeyexchange"
            | "gssapikexalgorithms"
            | "gssapirenewalforcesrekey"
            | "gssapitrustdns"
            | "hostkeyalgorithms"
            | "hostkeyalias"
            | "hostname"
            | "identitiesonly"
            | "identityagent"
            | "identityfile"
            | "usekeychain"
            | "ignoreunknown"
            | "ipqos"
            | "kbdinteractiveauthentication"
            | "kexalgorithms"
            | "localforward"
            | "loglevel"
            | "macs"
            | "nohostauthenticationforlocalhost"
            | "numberofpasswordprompts"
            | "passwordauthentication"
            | "pkcs11provider"
            | "port"
            | "preferredauthentications"
            | "proxycommand"
            | "pubkeyacceptedalgorithms"
            | "pubkeyauthentication"
            | "rekeylimit"
            | "remoteforward"
            | "requesttty"
            | "requiredrsasize"
            | "revokedhostkeys"
            | "sendenv"
            | "serveralivecountmax"
            | "serveraliveinterval"
            | "setenv"
            | "streamlocalbindunlink"
            | "stricthostkeychecking"
            | "syslogfacility"
            | "tcpkeepalive"
            | "tunnel"
            | "tunneldevice"
            | "updatehostkeys"
            | "user"
            | "verifyhostkeydns"
            | "versionaddendum"
            | "visualhostkey"
            | "xauthlocation"
    )
}

fn split_directive(line: &str) -> Result<(&str, &str), String> {
    let line = line.trim();
    let split = line
        .find(char::is_whitespace)
        .or_else(|| line.find('='))
        .unwrap_or(line.len());
    let keyword = &line[..split];
    if keyword.is_empty() {
        return Err("CONFIG_INVALID: empty SSH directive".to_string());
    }
    let argument = line[split..]
        .trim_start_matches(char::is_whitespace)
        .trim_start_matches('=');
    Ok((keyword, argument.trim()))
}

fn strip_inline_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character == '#' {
            return &line[..index];
        }
    }
    line
}

fn tokenize(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(character),
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn wildcard(value: &str) -> bool {
    value.starts_with('!')
        || value
            .chars()
            .any(|character| matches!(character, '*' | '?' | '['))
}

fn temporary_directory() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("SESSION_FAILED: system clock error: {error}"))?
        .as_nanos();
    let directory = PathBuf::from("/tmp").join(format!("sshx-{}-{nanos}", std::process::id()));
    fs::create_dir(&directory)
        .map_err(|error| format!("SESSION_FAILED: cannot create runtime directory: {error}"))?;
    set_mode(&directory, 0o700)?;
    Ok(directory)
}

fn known_hosts_path(home: &Path, entry: &HostEntry) -> PathBuf {
    if entry.scopes.iter().any(|scope| scope == "work")
        && !entry.scopes.iter().any(|scope| scope == "personal")
    {
        home.join(".config/sshx/known_hosts/work")
    } else {
        home.join(".ssh/known_hosts")
    }
}

fn ensure_known_hosts_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "SESSION_FAILED: known-hosts path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("SESSION_FAILED: cannot create host-key directory: {error}"))?;
    set_mode(parent, 0o700)?;
    if !path.exists() {
        write_private_file(path, b"")?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| format!("SESSION_FAILED: cannot create {}: {error}", path.display()))?;
    use std::io::Write;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("SESSION_FAILED: cannot write {}: {error}", path.display()))?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
        format!(
            "SESSION_FAILED: cannot set permissions on {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{compile_config, known_hosts_path};
    use crate::discovery::{HostEntry, Provenance, SourceIdentity};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn entry(path: PathBuf, aliases: &[&str]) -> HostEntry {
        HostEntry {
            id: "entry".to_string(),
            aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
            source: SourceIdentity {
                path: path.to_string_lossy().into_owned(),
                byte_start: 0,
                byte_end: 0,
                line_start: 1,
                line_end: 2,
            },
            destination: Some("example.test".to_string()),
            provenance: vec![Provenance {
                paths: vec![path.to_string_lossy().into_owned()],
                scope: "personal".to_string(),
                project: None,
            }],
            scopes: vec!["personal".to_string()],
            projects: Vec::new(),
        }
    }

    #[test]
    fn runtime_config_drops_comments_and_appends_system_defaults() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = "##SSHX ID=secret\nHost exact\n  HostName example.test # comment\n  IdentityFile ~/.ssh/id_ed25519\n";
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["exact"]);
        selected.source.byte_start = text.find("Host exact").expect("Host should exist");
        selected.source.byte_end = text.len();
        let compiled = compile_config(&selected, "exact").expect("config should compile");
        assert!(compiled.contains("Host exact"));
        assert!(compiled.contains("IdentityFile ~/.ssh/id_ed25519"));
        assert!(compiled.contains("Include /etc/ssh/ssh_config"));
        assert!(!compiled.contains("##SSHX"));
        assert!(!compiled.contains("comment"));
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn runtime_config_rejects_wildcards_and_tokens() {
        let directory = tempfile_directory();
        let path = directory.join("config");
        let text = "Host *.example\n  HostName %h\n";
        fs::write(&path, text).expect("fixture should be written");
        let mut selected = entry(path.clone(), &["*.example"]);
        selected.source.byte_end = text.len();
        let error = compile_config(&selected, "*.example").expect_err("unsafe config should fail");
        assert!(error.contains("UNSUPPORTED_WILDCARD"), "{error}");
        fs::remove_dir_all(directory).expect("fixture should be removed");
    }

    #[test]
    fn work_scope_uses_separate_host_key_store() {
        let entry = HostEntry {
            scopes: vec!["work".to_string()],
            ..entry(PathBuf::from("/tmp/config"), &["work"])
        };
        assert!(known_hosts_path(Path::new("/home/test"), &entry).ends_with("known_hosts/work"));
    }

    fn tempfile_directory() -> PathBuf {
        let ordinal = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sshx-connect-test-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture directory should be created");
        path
    }

    use std::path::Path;
}
