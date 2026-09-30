#![cfg(unix)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn hosts_opens_direct_session_and_restores_selected_alias() {
    let root = std::env::temp_dir().join(format!(
        "sshx-hosts-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let home = root.join("home");
    let ssh_dir = home.join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let config = ssh_dir.join("config");
    fs::write(&config, "Host direct secondary\n  HostName direct.example\nHost other\n  HostName other.example\n").unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(&ssh, r#"#!/bin/sh
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then cp "$argument" "$SSHX_CAPTURE"; fi
  previous="$argument"
done
case " $* " in
  *" -O check "*) test -f "$SSHX_STARTED"; exit $? ;;
  *" -O exit "*) touch "$SSHX_CLOSED"; exit 0 ;;
  *" -N "*) touch "$SSHX_STARTED"; trap 'exit 0' INT HUP TERM; while :; do sleep 0.1; done ;;
  *) printf 'direct-shell\n' ;;
esac
"#).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();

    let binary = env!("CARGO_BIN_EXE_sshx");
    let help = Command::new(binary).env("HOME", &home).output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Name: sshx"));
    let explicit = Command::new(binary).arg("tui").env("HOME", &home).output().unwrap();
    assert_eq!(explicit.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&explicit.stderr).contains("TUI_REQUIRED"));

    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize { ws_row: 40, ws_col: 100, ws_xpixel: 0, ws_ypixel: 0 };
    assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
    let slave = unsafe { File::from_raw_fd(slave) };
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let mut child = Command::new(binary)
        .arg("--config").arg(&config).arg("tui")
        .env("HOME", &home).env("PATH", path)
        .env("SSHX_STARTED", root.join("started"))
        .env("SSHX_CLOSED", root.join("closed"))
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn().unwrap();
    let mut master = unsafe { File::from_raw_fd(master) };
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
    let mut output = Vec::new();
    for (marker, input) in [("sshx Hosts", b"secondary\r".as_slice()), ("Session ended.", b"\x1b".as_slice())] {
        let stage_start = output.len();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let mut buffer = [0u8; 4096];
            match master.read(&mut buffer) {
                Ok(size) => output.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) => panic!("PTY read failed: {error}"),
            }
            if contains_tui_text(&output, marker) {
                break;
            }
            if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                panic!("Hosts did not show {marker}: {}", String::from_utf8_lossy(&output));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if marker == "Session ended." {
            assert!(
                contains_tui_text(&output[stage_start..], "secondary"),
                "Hosts did not restore the selected alias"
            );
        }
        master.write_all(input).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        panic!("Hosts did not exit after Esc");
    }
    assert!(child.wait().unwrap().success());
    assert!(String::from_utf8_lossy(&output).contains("direct-shell"));
    assert!(root.join("closed").exists(), "session-bound master was not closed");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("Host secondary"), "selected secondary alias lost: {runtime}");
    assert!(!runtime.contains("LocalForward"), "Session unexpectedly opened a forward");
    assert!(Path::new(&config).exists());
    let mut master = -1;
    let mut slave = -1;
    assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
    let slave = unsafe { File::from_raw_fd(slave) };
    let mut child = Command::new(binary)
        .arg("--config").arg(&config).args(["tui", "connect", "secondary"])
        .env("HOME", &home)
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
        .env("SSHX_STARTED", root.join("started"))
        .env("SSHX_CLOSED", root.join("closed"))
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn().unwrap();
    let mut master = unsafe { File::from_raw_fd(master) };
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
    let mut output = Vec::new();
    let mut remote = vec![0x7f; 32];
    remote.extend_from_slice(b"2222:127.0.0.1:2222\r\r");
    for (marker, input) in [
        ("secondary at", b"r".as_slice()),
        ("Enter -R", remote.as_slice()),
        ("Enter confirms", b"\r".as_slice()),
        ("Session ended.", b"\x1b".as_slice()),
    ] {
        let start = output.len();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !contains_tui_text(&output[start..], marker) && Instant::now() < deadline {
            let mut buffer = [0u8; 4096];
            match master.read(&mut buffer) {
                Ok(size) => output.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) => panic!("PTY read failed: {error}"),
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(contains_tui_text(&output[start..], marker), "focused Session did not show {marker}");
        master.write_all(input).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        let mut buffer = [0u8; 4096];
        if let Ok(size) = master.read(&mut buffer) { output.extend_from_slice(&buffer[..size]); }
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        panic!("TUI Session did not exit after Esc");
    }
    assert!(child.wait().unwrap().success());
    assert!(String::from_utf8_lossy(&output).contains("direct-shell"));
    let complete = Command::new(binary)
        .arg("--config").arg(&config).args(["connect", "secondary", "-R", "2222:127.0.0.1:2222", "--no-input"])
        .env("HOME", &home)
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
        .env("SSHX_STARTED", root.join("started"))
        .env("SSHX_CLOSED", root.join("closed"))
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .output().unwrap();
    assert!(complete.status.success(), "complete CLI command failed: {}", String::from_utf8_lossy(&complete.stderr));
    assert!(String::from_utf8_lossy(&complete.stdout).contains("direct-shell"));
    assert!(!String::from_utf8_lossy(&complete.stderr).contains("sshx Hosts"));
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("RemoteForward"), "complete CLI command dropped remote listener: {runtime}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tui_connect_preserves_exact_selection_and_cancels_before_ssh() {
    let root = std::env::temp_dir().join(format!(
        "sshx-continuation-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let home = root.join("home");
    let ssh_dir = home.join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let config = ssh_dir.join("config");
    fs::write(&config, "Host prod secondary\n  HostName direct.example\nHost production\n  HostName other.example\n").unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let binary = env!("CARGO_BIN_EXE_sshx");

    let unknown = Command::new(binary).args(["--config", config.to_str().unwrap(), "tui", "connect", "pro"])
        .env("HOME", &home).output().unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("HOST_NOT_FOUND"));
    let pipe = Command::new(binary).args(["--config", config.to_str().unwrap(), "tui", "connect", "secondary"])
        .env("HOME", &home).output().unwrap();
    assert_eq!(pipe.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&pipe.stderr).contains("TUI_REQUIRED"));

    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize { ws_row: 40, ws_col: 100, ws_xpixel: 0, ws_ypixel: 0 };
    assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
    let slave = unsafe { File::from_raw_fd(slave) };
    let mut child = Command::new(binary)
        .args(["--config", config.to_str().unwrap(), "tui", "connect", "secondary"])
        .env("HOME", &home)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn().unwrap();
    let mut master = unsafe { File::from_raw_fd(master) };
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !contains_tui_text(&output, "secondary at") && Instant::now() < deadline {
        let mut buffer = [0u8; 4096];
        match master.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
            Err(error) => panic!("PTY read failed: {error}"),
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(contains_tui_text(&output, "secondary at"), "focused Session did not show exact secondary alias: {}", String::from_utf8_lossy(&output));
    assert!(contains_tui_text(&output, "Session"), "Session workspace did not open");
    master.write_all(b"\x1b").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        let mut buffer = [0u8; 4096];
        if let Ok(size) = master.read(&mut buffer) {
            output.extend_from_slice(&buffer[..size]);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        panic!("CLI continuation did not exit on Esc: {}", String::from_utf8_lossy(&output));
    }
    assert_eq!(child.wait().unwrap().code(), Some(130));
    assert!(!home.join(".config/sshx/tunnels").exists(), "cancel created tunnel registry");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_show_selector_and_tunnel_id_cancel_without_effect() {
    let root = std::env::temp_dir().join(format!(
        "sshx-missing-selection-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let home = root.join("home");
    let ssh_dir = home.join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let config = ssh_dir.join("config");
    let contents = "Host exact\n  HostName exact.example\n";
    fs::write(&config, contents).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let binary = env!("CARGO_BIN_EXE_sshx");
    for (command, marker) in [
        (["host", "show"], "sshx Hosts"),
        (["tunnel", "stop"], "Tunnels"),
    ] {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize { ws_row: 40, ws_col: 100, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut child = Command::new(binary)
            .arg("--config").arg(&config).args(command)
            .env("HOME", &home)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn().unwrap();
        let mut master = unsafe { File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !contains_tui_text(&output, marker) && Instant::now() < deadline {
            let mut buffer = [0u8; 4096];
            match master.read(&mut buffer) {
                Ok(size) => output.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) => panic!("PTY read failed: {error}"),
            }
            if child.try_wait().unwrap().is_some() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(contains_tui_text(&output, marker), "missing selector did not enter {marker}: {}", String::from_utf8_lossy(&output));
        master.write_all(b"\x1b").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            let mut buffer = [0u8; 4096];
            if let Ok(size) = master.read(&mut buffer) { output.extend_from_slice(&buffer[..size]); }
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            panic!("{marker} did not cancel after Esc");
        }
        assert_eq!(child.wait().unwrap().code(), Some(130));
    }
    assert_eq!(fs::read_to_string(&config).unwrap(), contents);
    assert!(!home.join(".config/sshx/tunnels").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_host_update_reviews_prefilled_values_before_apply() {
    let root = std::env::temp_dir().join(format!("sshx-edit-{}-{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o700)).unwrap();
    let config = home.join(".ssh/config");
    let before = "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n";
    fs::write(&config, before).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config, &["host", "update", "secondary"]);
    terminal.expect("Edit HostEntry");
    terminal.expect("old.example");
    terminal.send(b"\t");
    terminal.send(&[0x7f; 11]);
    terminal.send(b"new.example\x13");
    terminal.expect("Review HostEntry changes");
    terminal.expect("Alias: secondary");
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName new.example\n  User alice\n  Port 2222\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tui_rename_changes_only_selected_secondary_alias_and_keeps_review_edits() {
    let (root, home, config) = host_edit_fixture("rename",
        "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n  ##PASSWORD stored-secret\nHost other\n  HostName untouched.example\n");
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "rename", "secondary", "--alias", "renamed", "--yes"]);
    terminal.expect("Rename HostEntry");
    terminal.expect("renamed");
    terminal.send(b"\x13");
    terminal.expect("Review HostEntry changes");
    terminal.expect("Host prod renamed");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
    terminal.send(b"e");
    terminal.expect("Rename HostEntry");
    terminal.expect("renamed");
    terminal.send(b"2\x13");
    terminal.expect("Host prod renamed2");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod renamed2\n  HostName old.example\n  User alice\n  Port 2222\n  ##PASSWORD stored-secret\nHost other\n  HostName untouched.example\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tui_update_keeps_replaces_and_clears_optional_values_without_exposing_passwords() {
    let before = "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n  ##PASSWORD stored-secret\n";
    let (root, home, config) = host_edit_fixture("fields", before);
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "update", "secondary", "--hostname", "new.example"]);
    terminal.expect("Edit HostEntry");
    terminal.expect("Password [keep]");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
    terminal.send(b"\t\t\x18\t\x18\tnew-secret\x13");
    terminal.expect("Review HostEntry changes");
    terminal.expect("<redacted>");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("new-secret"));
    assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
    terminal.send(b"e");
    terminal.expect("Password [replace]");
    terminal.send(b"\x18\x13");
    terminal.expect("Review HostEntry changes");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName new.example\n");
    fs::write(&config, before).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "update", "secondary", "--user", "bob"]);
    terminal.expect("Edit HostEntry");
    terminal.send(b"\t\t\t\tnew-secret\x13");
    terminal.expect("Review HostEntry changes");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName old.example\n  User bob\n  Port 2222\n  ##PASSWORD new-secret\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_edit_rejects_source_changes_before_selection_after_selection_and_after_review() {
    let before = "Host prod secondary\n  HostName old.example\n  User alice\n";
    let (root, home, config) = host_edit_fixture("stale-edit", before);
    for stage in 0..3 {
        fs::write(&config, before).unwrap();
        let arguments = if stage == 0 {
            vec!["host", "update", "--hostname", "new.example"]
        } else {
            vec!["tui", "host", "update", "secondary", "--hostname", "new.example"]
        };
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect(if stage == 0 { "host update" } else { "Edit HostEntry" });
        if stage == 2 {
            terminal.send(b"\x13");
            terminal.expect("Review HostEntry changes");
        }
        let external = format!("{before}# edited externally\n");
        fs::write(&config, &external).unwrap();
        terminal.send(if stage == 1 { b"\x13" } else { b"\r" });
        terminal.expect("HOST_SOURCE_CHANGED");
        if stage != 0 { terminal.send(b"\x1b"); }
        assert_eq!(terminal.finish(), Some(if stage == 0 { 2 } else { 130 }));
        assert_eq!(fs::read_to_string(&config).unwrap(), external);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_edit_preview_and_cancel_never_apply_pending_changes() {
    let before = "Host prod secondary\n  HostName old.example\n  User alice\n";
    let (root, home, config) = host_edit_fixture("preview-edit", before);
    for arguments in [
        vec!["tui", "host", "rename", "secondary", "--alias", "renamed", "--preview", "--yes"],
        vec!["host", "update", "secondary"],
        vec!["tui", "host", "update", "secondary", "--hostname", "new.example", "--yes"],
    ] {
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect(if arguments.contains(&"rename") { "Rename HostEntry" } else { "Edit HostEntry" });
        if arguments.contains(&"--preview") {
            let size = libc::winsize { ws_row: 8, ws_col: 24, ws_xpixel: 0, ws_ypixel: 0 };
            assert_eq!(unsafe { libc::ioctl(terminal.master.as_raw_fd(), libc::TIOCSWINSZ, &size) }, 0);
            terminal.send(b"\x13");
            terminal.expect("Enter finish preview");
            terminal.send(b"\r");
            assert_eq!(terminal.finish(), Some(0));
        } else if arguments.contains(&"--hostname") {
            terminal.send(b"\x13");
            terminal.expect("Review HostEntry changes");
            terminal.send(b"\x03");
            assert_eq!(terminal.finish(), Some(130));
        } else {
            terminal.send(b"\x1b");
            assert_eq!(terminal.finish(), Some(130));
        }
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_edit_exact_selectors_and_cli_modes_do_not_open_a_workspace() {
    let before = "Host prod secondary\n  HostName old.example\nHost production\n  HostName other.example\nHost prod\n  HostName duplicate.example\n";
    let (root, home, config) = host_edit_fixture("exact-edit", before);
    for (arguments, error) in [
        (vec!["tui", "host", "update", "pro"], "HOST_NOT_FOUND"),
        (vec!["tui", "host", "rename", "prod"], "HOST_AMBIGUOUS"),
        (vec!["tui", "host", "rename", "secondary", "--alias", "renamed"], "TUI_REQUIRED"),
        (vec!["host", "update", "secondary", "--no-input"], "MUTATION_EMPTY"),
        (vec!["host", "update", "--no-input"], "HOST_REQUIRED"),
        (vec!["host", "rename", "secondary", "--format", "json"], "MUTATION_EMPTY"),
        (vec!["host", "update", "--format", "yaml"], "HOST_REQUIRED"),
        (vec!["host", "rename", "secondary", "--user", "bob"], "MUTATION_FIELDS"),
        (vec!["host", "update", "secondary", "--user", "bob", "--clear-user"], "MUTATION_CONFLICT"),
        (vec!["host", "update", "secondary", "--port", "2222", "--clear-port"], "MUTATION_CONFLICT"),
        (vec!["host", "update", "secondary", "--password-stdin", "--clear-password"], "MUTATION_CONFLICT"),
        (vec!["tui", "host", "update", "secondary", "--no-input"], "TUI_REQUIRED"),
        (vec!["tui", "host", "update", "secondary", "--format", "json"], "TUI_REQUIRED"),
        (vec!["tui", "host", "update", "secondary", "--password-stdin"], "TUI_REQUIRED"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .arg("--config").arg(&config).args(&arguments).env("HOME", &home).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(error),
            "{arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(!output.stderr.contains(&0x1b), "{arguments:?} entered a workspace");
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    let source = fs::canonicalize(&config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("--config").arg(&config).args(["host", "update", "prod", "--source", source.to_str().unwrap(),
            "--line", "1", "--hostname", "new.example", "--preview", "--format", "json", "--no-input"])
        .env("HOME", &home).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["applied"], false);
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["host", "update", "secondary", "--hostname", "new.example", "--yes"]);
    assert_eq!(terminal.finish(), Some(0));
    assert!(!contains_tui_text(&terminal.output, "Edit HostEntry"));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName new.example\nHost production\n  HostName other.example\nHost prod\n  HostName duplicate.example\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_partial_host_edit_fails_before_interactive_selection() {
    let (root, home, config) = host_edit_fixture("invalid-edit",
        "Host prod\n  HostName old.example\n");
    for (arguments, error) in [
        (vec!["host", "update", "--alias", "bad alias"], "ALIAS_INVALID"),
        (vec!["host", "update", "--hostname", ""], "hostname_INVALID"),
        (vec!["host", "update", "--user", "bob", "--clear-user"], "MUTATION_CONFLICT"),
        (vec!["host", "rename", "--user", "bob"], "MUTATION_FIELDS"),
    ] {
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect(error);
        assert_eq!(terminal.finish(), Some(2));
        assert!(!terminal.output.contains(&0x1b));
        assert_eq!(fs::read_to_string(&config).unwrap(), "Host prod\n  HostName old.example\n");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hosts_update_and_rename_return_to_persistent_selected_host() {
    let (root, home, config) = host_edit_fixture("hosts-edit",
        "Host prod secondary\n  HostName old.example\n  User alice\nHost other\n  HostName other.example\n");
    let mut terminal = HostEditTerminal::open(&home, &config, &["tui"]);
    terminal.expect("sshx Hosts");
    terminal.send(b"secondary\x15");
    terminal.expect("Edit HostEntry");
    terminal.expect("Alias [keep]: secondary");
    terminal.send(b"\t");
    terminal.send(&[0x7f; 11]);
    terminal.send(b"new.example\x13");
    terminal.expect("Review HostEntry changes");
    terminal.send(b"\r");
    terminal.expect("HostEntry updated.");
    terminal.expect("secondary");
    terminal.send(b"\x12");
    terminal.expect("Rename HostEntry");
    terminal.send(&[0x7f; 9]);
    terminal.send(b"renamed\x13");
    terminal.expect("Host prod renamed");
    terminal.send(b"\r");
    terminal.expect("HostEntry renamed.");
    terminal.expect("renamed");
    terminal.send(b"\x15");
    terminal.expect("Edit HostEntry");
    terminal.expect("Alias [keep]: renamed");
    terminal.send(b"\x1b");
    terminal.expect("HostEntry edit cancelled.");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod renamed\n  HostName new.example\n  User alice\nHost other\n  HostName other.example\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_edit_validation_keeps_values_and_focuses_invalid_port() {
    let (root, home, config) = host_edit_fixture("validation-edit",
        "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n");
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "update", "secondary", "--hostname", "new.example"]);
    terminal.expect("Edit HostEntry");
    terminal.send(b"\t\t\t");
    terminal.send(&[0x7f; 4]);
    terminal.send(b"0\x13");
    terminal.expect("PORT_INVALID");
    terminal.expect("new.example");
    terminal.expect("> Port [replace]: 0");
    terminal.send(b"\x7f22\x13");
    terminal.expect("Review HostEntry changes");
    terminal.expect("+  Port 22");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName new.example\n  User alice\n  Port 22\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_host_selector_prefills_clear_flag_and_can_replace_that_field() {
    let (root, home, config) = host_edit_fixture("partial-clear",
        "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n");
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["host", "update", "--clear-user"]);
    terminal.expect("host update");
    terminal.send(b"secondary\r");
    terminal.expect("Edit HostEntry");
    terminal.expect("User [clear]");
    terminal.send(b"\t\tbob\x13");
    terminal.expect("Review HostEntry changes");
    terminal.expect("+  User bob");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName old.example\n  User bob\n  Port 2222\n");
    fs::remove_dir_all(root).unwrap();
}

fn host_edit_fixture(name: &str, contents: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("sshx-{name}-{}-{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o700)).unwrap();
    let config = home.join(".ssh/config");
    fs::write(&config, contents).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    (root, home, config)
}

struct HostEditTerminal {
    child: std::process::Child,
    master: File,
    output: Vec<u8>,
}

impl HostEditTerminal {
    fn open(home: &Path, config: &Path, arguments: &[&str]) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
        let slave = unsafe { File::from_raw_fd(slave) };
        let child = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .arg("--config").arg(config).args(arguments).env("HOME", home)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave)).spawn().unwrap();
        let master = unsafe { File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
        Self { child, master, output: Vec::new() }
    }

    fn expect(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !contains_tui_text(&self.output, marker) {
            self.drain();
            if contains_tui_text(&self.output, marker) { break; }
            if Instant::now() >= deadline || self.child.try_wait().unwrap().is_some() {
                panic!("PTY did not show {marker}: {}", String::from_utf8_lossy(&self.output));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain(&mut self) {
        let mut buffer = [0u8; 8192];
        match self.master.read(&mut buffer) {
            Ok(size) => self.output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock || error.raw_os_error() == Some(libc::EIO) => {}
            Err(error) => panic!("PTY read failed: {error}"),
        }
    }

    fn send(&mut self, input: &[u8]) {
        self.output.clear();
        self.master.write_all(input).unwrap();
    }

    fn finish(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() { return status.code(); }
            assert!(Instant::now() < deadline, "PTY did not exit: {}", String::from_utf8_lossy(&self.output));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for HostEditTerminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn contains_tui_text(output: &[u8], expected: &str) -> bool {
    let mut text = Vec::with_capacity(output.len());
    let mut index = 0;
    while index < output.len() {
        if output[index] == b'\x1b' && output.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < output.len() && !(0x40..=0x7e).contains(&output[index]) {
                index += 1;
            }
        } else if !output[index].is_ascii_whitespace() {
            text.push(output[index]);
        }
        index += 1;
    }
    let expected = expected
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    text.windows(expected.len()).any(|window| window == expected)
}
