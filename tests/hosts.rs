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
    let original = "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost direct secondary\n  HostName direct.example\nHost other\n  HostName other.example\n";
    fs::write(&config, original).unwrap();
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

    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let started = root.join("started");
    let closed = root.join("closed");
    let capture = root.join("runtime-config");
    let mut terminal = HostEditTerminal::open_with_env(
        &home, &config, &[],
        &[
            ("PATH", &path),
            ("SSHX_STARTED", started.to_str().unwrap()),
            ("SSHX_CLOSED", closed.to_str().unwrap()),
            ("SSHX_CAPTURE", capture.to_str().unwrap()),
        ],
    );
    terminal.expect("sshx Hosts");
    terminal.send(b"secondary\r");
    terminal.expect("Connection workspace");
    terminal.expect("secondary");
    assert!(!started.exists(), "selection started OpenSSH before review");
    assert!(!capture.exists(), "selection emitted runtime configuration before review");
    terminal.send(b"\r");
    terminal.expect("Review:");
    assert!(!started.exists(), "review started OpenSSH before confirmation");
    assert!(!capture.exists(), "review emitted runtime configuration before confirmation");
    terminal.send(b"\r");
    terminal.expect("direct-shell");
    terminal.expect("sshx Hosts");
    terminal.expect("> secondary");
    terminal.expect("direct.example");
    assert!(started.exists(), "confirmed Session did not start its master");
    assert!(closed.exists(), "session-bound master was not closed");
    terminal.send(b"\t");
    terminal.expect("11111111-1111-4111-8111-111111111111");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("Host secondary"), "selected secondary alias lost: {runtime}");
    assert!(!runtime.contains("LocalForward"), "Session unexpectedly opened a forward");
    assert!(!runtime.contains("RemoteForward"), "Session unexpectedly opened a remote forward");
    assert!(!runtime.contains("DynamicForward"), "Session unexpectedly opened a SOCKS forward");
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn continuation_exact_prefills_remain_editable_and_cancel_without_side_effect() {
    let original = concat!(
        "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
        "Host duplicate secondary\n  HostName first.example\n",
        "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
        "Host duplicate other\n  HostName second.example\n",
    );
    let (root, home, config) = host_edit_fixture("editable-continuation", original);
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(&ssh, "#!/bin/sh\ntouch \"$SSHX_SENTINEL\"\nexit 97\n").unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let sentinel = root.join("ssh-started");
    for selection in [
        vec!["--id", "11111111-1111-4111-8111-111111111111"],
        vec!["--source", config.to_str().unwrap(), "--line", "2"],
    ] {
        let mut arguments = vec!["tui", "connect", "secondary"];
        arguments.extend(selection);
        let mut terminal = HostEditTerminal::open_with_env(
            &home, &config, &arguments,
            &[("PATH", &path), ("SSHX_SENTINEL", sentinel.to_str().unwrap())],
        );
        terminal.expect("> secondary");
        terminal.expect("first.example");
        terminal.send(b"other\r");
        terminal.expect("Connection workspace");
        terminal.expect("other");
        terminal.expect("config:5");
        terminal.expect("22222222-2222-4222-8222-222222222222");
        terminal.send(b"\x1b");
        terminal.expect("Search:");
        terminal.send(b"\x03");
        assert_eq!(terminal.finish(), Some(130));
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert!(!sentinel.exists(), "cancellation started OpenSSH");
        assert!(!home.join(".config/sshx").exists(), "cancellation persisted state");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn continuation_explicit_selectors_reject_prefixes_ambiguity_and_conflicts() {
    let original = concat!(
        "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
        "Host duplicate secondary\n  HostName first.example\n",
        "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
        "Host duplicate other\n  HostName second.example\n",
    );
    let (root, home, config) = host_edit_fixture("invalid-continuation", original);
    for (arguments, error) in [
        (vec!["tui", "connect", "secondar"], "HOST_NOT_FOUND"),
        (vec!["tui", "connect", "duplicate"], "HOST_AMBIGUOUS"),
        (vec!["tui", "connect", "other", "--id", "11111111-1111-4111-8111-111111111111"], "HOST_NOT_FOUND"),
    ] {
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        assert_eq!(terminal.finish(), Some(2));
        assert!(contains_tui_text(&terminal.output, error),
            "expected {error}: {}", String::from_utf8_lossy(&terminal.output));
        assert!(!contains_tui_text(&terminal.output, "Search:"));
    }
    let piped = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("--config").arg(&config).args(["tui", "connect", "secondary"])
        .env("HOME", &home).output().unwrap();
    assert_eq!(piped.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&piped.stderr).contains("TUI_REQUIRED"));
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!home.join(".config/sshx").exists());
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
fn host_delete_hosts_shortcut_cancels_then_deletes_exact_duplicate_and_stays_open() {
    let before = "Host duplicate\n  HostName first.example\nHost duplicate\n  HostName second.example\nHost other\n  HostName untouched.example\n";
    let (root, home, config) = host_edit_fixture("hosts-delete", before);
    let mut terminal = HostEditTerminal::open(&home, &config, &["tui", "--yes"]);
    terminal.expect("sshx Hosts");
    terminal.send(b"duplicate\x1b[B\x18");
    terminal.expect("Review HostEntry mutation");
    terminal.expect("-  HostName second.example");
    terminal.send(b"\x1b");
    terminal.expect("HostEntry deletion cancelled.");
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    terminal.send(b"\x18");
    terminal.expect("Review HostEntry mutation");
    terminal.send(b"\r");
    terminal.expect("HostEntry deleted.");
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host duplicate\n  HostName first.example\nHost other\n  HostName untouched.example\n");
    assert!(terminal.child.try_wait().unwrap().is_none());
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_explicit_tui_result_refreshes_hosts_and_stays_open() {
    let before = "Host selected secondary\n  HostName old.example\n  ##PASSWORD stored-secret\nHost untouched\n  HostName untouched.example\n";
    let (root, home, config) = host_edit_fixture("explicit-tui-delete", before);
    for preview in [false, true] {
        fs::write(&config, before).unwrap();
        let mut arguments = vec!["tui", "host", "delete", "secondary", "--yes"];
        if preview { arguments.push("--preview"); }
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect("Review HostEntry mutation");
        terminal.expect("Alias: secondary");
        assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
        terminal.send(b"\r");
        terminal.expect(if preview { "HostEntry preview complete." } else { "HostEntry deleted." });
        terminal.expect("sshx Hosts");
        assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
        assert_eq!(fs::read_to_string(&config).unwrap(), if preview {
            before
        } else {
            "Host untouched\n  HostName untouched.example\n"
        });
        assert!(terminal.child.try_wait().unwrap().is_none());
        let size = libc::winsize { ws_row: 39, ws_col: 119, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::ioctl(terminal.master.as_raw_fd(), libc::TIOCSWINSZ, &size) }, 0);
        terminal.send(b"secondary");
        terminal.expect(if preview { "old.example" } else { "No matching HostEntry aliases." });
        assert!(terminal.child.try_wait().unwrap().is_none());
        terminal.send(b"\x1b");
        assert_eq!(terminal.finish(), Some(0));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_explicit_selectors_do_not_filter_refreshed_hosts() {
    let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let before = format!("##SSHX ID={id}\nHost selected\n  HostName old.example\nHost untouched\n  HostName untouched.example\n");
    let (root, home, config) = host_edit_fixture("delete-selectors", &before);
    let source = config.canonicalize().unwrap();
    for by_id in [true, false] {
        fs::write(&config, &before).unwrap();
        let mut arguments = vec!["tui", "host", "delete"];
        if by_id {
            arguments.extend(["--id", id]);
        } else {
            arguments.extend(["selected", "--source", source.to_str().unwrap(), "--line", "2"]);
        }
        arguments.push("--yes");
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect("Review HostEntry mutation");
        terminal.send(b"\r");
        terminal.expect("HostEntry deleted.");
        terminal.expect("untouched.example");
        assert_eq!(fs::read_to_string(&config).unwrap(),
            "Host untouched\n  HostName untouched.example\n");
        assert!(terminal.child.try_wait().unwrap().is_none());
        terminal.send(b"\x1b");
        assert_eq!(terminal.finish(), Some(0));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_preview_and_review_cancellation_preserve_source() {
    let before = "Host prod secondary\n  HostName old.example\n  ##PASSWORD stored-secret\n";
    let (root, home, config) = host_edit_fixture("preview-delete", before);
    for (arguments, exit, key) in [
        (vec!["tui", "host", "delete", "secondary", "--preview", "--yes"], 0, b"\r".as_slice()),
        (vec!["tui", "host", "delete", "secondary", "--yes"], 130, b"\x03".as_slice()),
        (vec!["tui", "--preview"], 0, b"\r".as_slice()),
    ] {
        let hosts = arguments.len() == 2;
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        if hosts {
            terminal.expect("sshx Hosts");
            terminal.send(b"secondary\x18");
        }
        terminal.expect("Review HostEntry mutation");
        terminal.expect("Alias: secondary");
        terminal.expect("<redacted>");
        assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
        if arguments.contains(&"--preview") {
            if !hosts {
                let size = libc::winsize { ws_row: 8, ws_col: 24, ws_xpixel: 0, ws_ypixel: 0 };
                assert_eq!(unsafe { libc::ioctl(terminal.master.as_raw_fd(), libc::TIOCSWINSZ, &size) }, 0);
                terminal.send(b"\x1b[6~");
            }
            terminal.expect("Enter finish preview");
            if !hosts {
                let size = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
                assert_eq!(unsafe { libc::ioctl(terminal.master.as_raw_fd(), libc::TIOCSWINSZ, &size) }, 0);
            }
        }
        terminal.send(key);
        if arguments.contains(&"--preview") {
            terminal.expect("HostEntry preview complete.");
            assert!(!contains_tui_text(&terminal.output, "HostEntry deleted."));
            assert!(terminal.child.try_wait().unwrap().is_none());
            terminal.send(b"\x1b");
        }
        assert_eq!(terminal.finish(), Some(exit));
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_rejects_changed_source_and_new_dependencies_after_review() {
    let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let before = format!("##SSHX ID={id}\nHost selected\n  HostName old.example\n");
    let (root, home, config) = host_edit_fixture("stale-delete", &before);
    for after_review in [false, true] {
        fs::write(&config, &before).unwrap();
        let arguments = if after_review {
            vec!["tui", "host", "delete", "selected"]
        } else { vec!["host", "delete"] };
        let mut terminal = HostEditTerminal::open(&home, &config, &arguments);
        terminal.expect(if after_review { "Review HostEntry mutation" } else { "host delete" });
        let external = format!("{before}# external edit\n");
        fs::write(&config, &external).unwrap();
        terminal.send(b"\r");
        terminal.expect("HOST_SOURCE_CHANGED");
        assert_eq!(terminal.finish(), Some(2));
        assert_eq!(fs::read_to_string(&config).unwrap(), external);
    }
    fs::write(&config, &before).unwrap();
    let replacement = home.join(".ssh/replacement");
    fs::write(&replacement, &before).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "delete", "selected"]);
    terminal.expect("Review HostEntry mutation");
    fs::remove_file(&config).unwrap();
    std::os::unix::fs::symlink(&replacement, &config).unwrap();
    terminal.send(b"\r");
    terminal.expect("CONFIG_ROOT_SYMLINK");
    assert_eq!(terminal.finish(), Some(2));
    assert!(fs::symlink_metadata(&config).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(&replacement).unwrap(), before);
    fs::remove_file(&config).unwrap();
    let dependency = home.join(".ssh/dependency");
    fs::write(&dependency, "Host vm\n  HostName vm.example\n").unwrap();
    fs::set_permissions(&dependency, fs::Permissions::from_mode(0o600)).unwrap();
    let included = format!("Include dependency\n{before}");
    fs::write(&config, &included).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "delete", "selected"]);
    terminal.expect("Review HostEntry mutation");
    fs::write(&dependency, format!("##SSHX GATEWAY={id}\nHost vm\n  HostName vm.example\n")).unwrap();
    terminal.send(b"\r");
    terminal.expect("DELETE_REFERENCED");
    assert_eq!(terminal.finish(), Some(2));
    assert_eq!(fs::read_to_string(&config).unwrap(), included);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_pair_blocker_shows_exact_sources_and_redacts_password() {
    let gateway = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let vm = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let before = format!("##SSHX ID={gateway}\n##SSHX VM={vm}\nHost gateway\n  HostName gateway.example\n  ##PASSWORD stored-secret\n##SSHX ID={vm}\n##SSHX GATEWAY={gateway}\n##SSHX TRANSIT=127.0.0.1:2222\nHost vm\n  HostName vm.example\n");
    let (root, home, config) = host_edit_fixture("pair-delete", &before);
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "delete", "gateway", "--yes"]);
    terminal.expect("HostEntry deletion blocked");
    terminal.expect("-Host gateway");
    terminal.expect("<redacted>");
    terminal.expect("Pair gateway: gateway");
    terminal.expect("Pair VM: vm");
    terminal.expect("Transit: 127.0.0.1:2222");
    terminal.expect("DELETE_REFERENCED");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(2));
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    fs::write(&config, format!("##SSHX ID={gateway}\n##SSHX TRANSIT=broken\nHost gateway\n  HostName gateway.example\n")).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "delete", "gateway"]);
    terminal.expect("HostEntry deletion blocked");
    terminal.expect("DELETE_REFERENCED");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(2));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_blocks_active_registry_uuid_with_different_case() {
    let selected = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let unrelated = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let before = format!("##SSHX ID={selected}\nHost selected\n  HostName selected.example\n");
    let (root, home, config) = host_edit_fixture("registry-case-delete", &before);
    let registry = home.join(".config/sshx/tunnels/registry.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    let active_id = selected.to_ascii_uppercase();
    for role in ["direct", "gateway", "vm"] {
        let record = match role {
            "gateway" => serde_json::json!({"state": "active", "entry_id": unrelated,
                "pair": {"gateway_entry_id": active_id, "vm_entry_id": unrelated}}),
            "vm" => serde_json::json!({"state": "active", "entry_id": unrelated,
                "pair": {"gateway_entry_id": unrelated, "vm_entry_id": active_id}}),
            _ => serde_json::json!({"state": "active", "entry_id": active_id}),
        };
        fs::write(&registry, serde_json::json!({"version": 1, "tunnels": [record]}).to_string()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .arg("--config").arg(&config).args(["host", "delete", "selected", "--yes", "--no-input"])
            .env("HOME", &home).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{role}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("DELETE_ACTIVE"), "{role}: {output:?}");
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_registry_safety_matches_each_record_and_rechecks_after_review() {
    let selected = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let unrelated = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let before = format!("##SSHX ID={selected}\nHost selected\n  HostName selected.example\n");
    let (root, home, config) = host_edit_fixture("registry-delete", &before);
    let registry = home.join(".config/sshx/tunnels/registry.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    let delete = || Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("--config").arg(&config).args(["host", "delete", "selected", "--yes", "--no-input"])
        .env("HOME", &home).output().unwrap();
    for state in ["active", "starting", "stopping"] {
        for role in ["direct", "gateway", "vm"] {
            let record = match role {
                "gateway" => serde_json::json!({"state": state, "entry_id": unrelated,
                    "pair": {"gateway_entry_id": selected, "vm_entry_id": unrelated}}),
                "vm" => serde_json::json!({"state": state, "entry_id": unrelated,
                    "pair": {"gateway_entry_id": unrelated, "vm_entry_id": selected}}),
                _ => serde_json::json!({"state": state, "entry_id": selected}),
            };
            fs::write(&registry, serde_json::json!({"version": 1, "tunnels": [record]}).to_string()).unwrap();
            let output = delete();
            assert_eq!(output.status.code(), Some(2), "{state} {role}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("DELETE_ACTIVE"), "{output:?}");
            assert_eq!(fs::read_to_string(&config).unwrap(), before);
        }
    }
    for invalid in ["{malformed", "{}", r#"{"version":1,"tunnels":[{"entry_id":"unknown"}]}"#] {
        fs::write(&registry, invalid).unwrap();
        let output = delete();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("DELETE_REGISTRY_INVALID"), "{output:?}");
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    fs::write(&registry, serde_json::json!({"version": 1, "tunnels": [
        {"state": "stopped", "entry_id": selected},
        {"state": "active", "entry_id": unrelated}
    ]}).to_string()).unwrap();
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "delete", "selected"]);
    terminal.expect("Review HostEntry mutation");
    fs::write(&registry, serde_json::json!({"version": 1, "tunnels": [
        {"state": "starting", "entry_id": selected}
    ]}).to_string()).unwrap();
    terminal.send(b"\r");
    terminal.expect("DELETE_ACTIVE");
    assert_eq!(terminal.finish(), Some(2));
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    fs::write(&registry, serde_json::json!({"version": 1, "tunnels": [
        {"state": "stopped", "entry_id": selected,
         "pair": {"gateway_entry_id": selected, "vm_entry_id": unrelated,
                  "gateway_id": selected, "vm_id": unrelated}},
        {"state": "active", "entry_id": unrelated}
    ]}).to_string()).unwrap();
    let output = delete();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs::read_to_string(&config).unwrap(), "");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_complete_cli_keeps_consent_and_exact_selector_errors() {
    let before = "Host prod secondary\n  HostName old.example\nHost prod\n  HostName duplicate.example\n";
    let (root, home, config) = host_edit_fixture("cli-delete", before);
    for (arguments, error) in [
        (vec!["host", "delete", "--no-input"], "HOST_REQUIRED"),
        (vec!["host", "delete", "--format", "json"], "HOST_REQUIRED"),
        (vec!["host", "delete", "secondary", "--no-input"], "CONSENT_REQUIRED"),
        (vec!["host", "delete", "secondary", "--format", "yaml"], "CONSENT_REQUIRED"),
        (vec!["tui", "host", "delete", "pro"], "HOST_NOT_FOUND"),
        (vec!["tui", "host", "delete", "prod"], "HOST_AMBIGUOUS"),
        (vec!["tui", "host", "delete", "secondary"], "TUI_REQUIRED"),
        (vec!["host", "delete", "--user", "alice"], "MUTATION_FIELDS"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx")).arg("--config").arg(&config)
            .args(&arguments).env("HOME", &home).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(error), "{output:?}");
        assert!(!output.stderr.contains(&0x1b));
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
    }
    let mut terminal = HostEditTerminal::open(&home, &config, &["host", "delete", "secondary"]);
    terminal.expect("Apply changes? [y/N]");
    terminal.send(b"n\r");
    assert_eq!(terminal.finish(), Some(2));
    assert!(contains_tui_text(&terminal.output, "MUTATION_DECLINED"));
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    let output = Command::new(env!("CARGO_BIN_EXE_sshx")).arg("--config").arg(&config)
        .args(["host", "delete", "secondary", "--preview", "--format", "json", "--no-input"])
        .env("HOME", &home).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["applied"], false);
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["host", "delete", "secondary", "--yes"]);
    assert_eq!(terminal.finish(), Some(0));
    assert!(!contains_tui_text(&terminal.output, "Review HostEntry mutation"));
    assert_eq!(fs::read_to_string(&config).unwrap(), "Host prod\n  HostName duplicate.example\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_delete_selectorless_reviews_exact_duplicate_before_apply() {
    let before = "Host duplicate\n  HostName first.example\n  ##PASSWORD stored-secret\nHost duplicate\n  HostName second.example\nHost other\n  HostName untouched.example\n";
    let (root, home, config) = host_edit_fixture("delete-duplicate", before);
    let mut terminal = HostEditTerminal::open(&home, &config, &["host", "delete", "--yes"]);
    terminal.expect("host delete");
    terminal.send(b"duplicate\x1b[B\r");
    terminal.expect("Review HostEntry mutation");
    terminal.expect("-  HostName second.example");
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(130));
    assert_eq!(fs::read_to_string(&config).unwrap(), before);
    let mut terminal = HostEditTerminal::open(&home, &config, &["host", "delete", "--yes"]);
    terminal.expect("host delete");
    terminal.send(b"duplicate\x1b[B\r");
    terminal.expect("Review HostEntry mutation");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host duplicate\n  HostName first.example\n  ##PASSWORD stored-secret\nHost other\n  HostName untouched.example\n");
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

#[test]
fn hosts_rename_ignores_unrelated_root_cli_fields() {
    let (root, home, config) = host_edit_fixture("rename-root-fields",
        "Host prod secondary\n  HostName old.example\n  User alice\n  Port 2222\n  ##PASSWORD stored-secret\n");
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "--hostname", "new.example", "--user", "bob", "--clear-port", "--clear-password"]);
    terminal.expect("sshx Hosts");
    terminal.send(b"secondary\x12");
    terminal.expect("Rename HostEntry");
    terminal.send(&[0x7f; 9]);
    terminal.send(b"renamed\x13");
    terminal.expect("Review HostEntry changes");
    terminal.send(b"\r");
    terminal.expect("HostEntry renamed.");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod renamed\n  HostName old.example\n  User alice\n  Port 2222\n  ##PASSWORD stored-secret\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_update_prefills_quoted_port_and_preserves_it_when_kept() {
    let (root, home, config) = host_edit_fixture("quoted-port",
        "Host prod secondary\n  HostName old.example\n  User alice\n  Port \"2222\"\n");
    let mut terminal = HostEditTerminal::open(&home, &config,
        &["tui", "host", "update", "secondary", "--user", "bob"]);
    terminal.expect("Edit HostEntry");
    terminal.expect("Port [keep]: 2222");
    terminal.send(b"\x13");
    terminal.expect("Review HostEntry changes");
    terminal.send(b"\r");
    assert_eq!(terminal.finish(), Some(0));
    assert_eq!(fs::read_to_string(&config).unwrap(),
        "Host prod secondary\n  HostName old.example\n  User bob\n  Port \"2222\"\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hosts_preview_returns_to_unchanged_entry_without_claiming_mutation() {
    let before = "Host prod secondary\n  HostName old.example\n  User alice\n";
    let (root, home, config) = host_edit_fixture("hosts-preview", before);
    for (key, title) in [(b"\x15".as_slice(), "Edit HostEntry"), (b"\x12".as_slice(), "Rename HostEntry")] {
        let mut terminal = HostEditTerminal::open(&home, &config,
            &["tui", "--preview", "--hostname", "new.example", "--alias", "renamed"]);
        terminal.expect("sshx Hosts");
        terminal.send(b"secondary");
        terminal.send(key);
        terminal.expect(title);
        terminal.send(b"\x13");
        terminal.expect("Enter finish preview");
        terminal.send(b"\r");
        terminal.expect("HostEntry preview complete.");
        terminal.expect("secondary");
        assert!(!contains_tui_text(&terminal.output, "HostEntry updated."));
        assert!(!contains_tui_text(&terminal.output, "HostEntry renamed."));
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
        terminal.send(b"\x1b");
        assert_eq!(terminal.finish(), Some(0));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hosts_pairs_inspects_exact_entries_and_refreshes_read_only_validation() {
    let (root, home, config) = pair_inspection_fixture("pi", 3333);
    let gateway_source = home.join(".ssh/g");
    let vm_source = home.join(".ssh/v");
    let sentinel = root.join("ssh-called");
    let path = format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    let config_before = fs::read_to_string(&config).unwrap();
    let gateway_before = fs::read_to_string(&gateway_source).unwrap();
    let vm_before = fs::read_to_string(&vm_source).unwrap();
    let assert_source = |source: &Path, expected: &str| {
        assert_eq!(fs::read(source).unwrap(), expected.as_bytes());
        assert_eq!(fs::metadata(source).unwrap().permissions().mode() & 0o777, 0o600);
    };
    let resize = |terminal: &HostEditTerminal, width| {
        let size = libc::winsize { ws_row: 40, ws_col: width, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::ioctl(terminal.master.as_raw_fd(), libc::TIOCSWINSZ, &size) }, 0);
    };
    let mut terminal = HostEditTerminal::open_with_env(&home, &config, &["tui"],
        &[("PATH", &path), ("SSHX_SENTINEL", sentinel.to_str().unwrap())]);
    terminal.expect("sshx Hosts");
    terminal.send(b"\x10");
    terminal.expect("Status: invalid");
    terminal.expect("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    terminal.expect("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    terminal.expect("exact-gateway");
    terminal.expect("exact-vm");
    terminal.expect(gateway_source.to_str().unwrap());
    terminal.expect(vm_source.to_str().unwrap());
    terminal.expect("Host line: 3");
    terminal.expect("Host line: 4");
    terminal.expect("127.0.0.1:2222");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("stored-secret"));
    resize(&terminal, 121);
    terminal.send(b"\x1b[B");
    terminal.expect("PAIR_ROUTE_CHANGED");
    terminal.expect("Evidence:");
    terminal.expect("Guidance:");
    terminal.expect(gateway_source.to_str().unwrap());
    terminal.expect(vm_source.to_str().unwrap());
    terminal.expect("3333");
    terminal.expect("127.0.0.1:2222");
    assert_source(&config, &config_before);
    assert_source(&gateway_source, &gateway_before);
    assert_source(&vm_source, &vm_before);
    assert!(!sentinel.exists(), "Pair inspection invoked OpenSSH");

    let gateway_refreshed = gateway_before.replace("127.0.0.1:2222", "127.0.0.2:2299");
    let vm_refreshed = vm_before.replace("127.0.0.1:2222", "127.0.0.2:2299")
        .replace("exact-vm", "refreshed-vm").replace("Port 3333", "Port 2299");
    fs::write(&gateway_source, &gateway_refreshed).unwrap();
    fs::write(&vm_source, &vm_refreshed).unwrap();
    terminal.send(b"V");
    terminal.expect("Status: valid");
    terminal.expect("refreshed-vm");
    terminal.expect("127.0.0.2:2299");
    assert!(!contains_tui_text(&terminal.output, "PAIR_ROUTE_CHANGED"));
    assert_source(&config, &config_before);
    assert_source(&gateway_source, &gateway_refreshed);
    assert_source(&vm_source, &vm_refreshed);

    let ambiguous = format!("{config_before}##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\nHost duplicate-id\n  HostName ambiguous.example\n");
    fs::write(&config, &ambiguous).unwrap();
    terminal.send(b"v");
    terminal.expect("broken_reference");
    assert!(!contains_tui_text(&terminal.output, "Status: valid"));
    resize(&terminal, 122);
    terminal.send(b"\x1b[B");
    terminal.expect("duplicate_id");
    terminal.expect("Evidence:");
    terminal.expect("Guidance:");
    terminal.expect("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    terminal.expect(gateway_source.to_str().unwrap());
    terminal.expect(config.to_str().unwrap());
    terminal.send(b"\x1b");
    terminal.expect("sshx Hosts");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    assert_source(&config, &ambiguous);
    assert_source(&gateway_source, &gateway_refreshed);
    assert_source(&vm_source, &vm_refreshed);
    assert_eq!(fs::metadata(home.join(".ssh")).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(!sentinel.exists(), "Pair validation invoked OpenSSH");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_inspection_cli_preserves_version_one_machine_contract_and_sources() {
    let (root, home, config) = pair_inspection_fixture("pm", 2222);
    let gateway_source = home.join(".ssh/g");
    let vm_source = home.join(".ssh/v");
    let sentinel = root.join("ssh-called");
    let path = format!("{}:{}", root.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    let config_before = fs::read_to_string(&config).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let expected = serde_json::json!({
        "version": 1,
        "pairs": [{
            "gateway_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "vm_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            "gateway_alias": "gateway",
            "vm_alias": "vm",
            "transit_host": "127.0.0.1",
            "transit_port": 2222
        }],
        "diagnostics": []
    });
    for ambiguous in [false, true] {
        if ambiguous {
            fs::write(&config, format!("{config_before}##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\nHost duplicate-id\n  HostName ambiguous.example\n")).unwrap();
        }
        let sources = [&config, &gateway_source, &vm_source];
        let before = sources.iter().map(|source| (
            fs::read(source).unwrap(),
            fs::metadata(source).unwrap().permissions().mode() & 0o777
        )).collect::<Vec<_>>();
        for action in ["list", "validate"] {
            for format in ["json", "yaml"] {
                let output = Command::new(env!("CARGO_BIN_EXE_sshx")).arg("--config").arg(&config)
                    .args(["pair", action, "--format", format, "--no-input"])
                    .env("HOME", &home).env("PATH", &path).env("SSHX_SENTINEL", &sentinel)
                    .output().unwrap();
                assert!(output.status.success(), "{action} {format}: {output:?}");
                let document: serde_json::Value = if format == "json" {
                    serde_json::from_slice(&output.stdout).unwrap()
                } else {
                    serde_yaml::from_slice(&output.stdout).unwrap()
                };
                if !ambiguous {
                    assert_eq!(document, expected, "{action} {format}");
                } else {
                    assert_eq!(document.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(),
                        ["diagnostics", "pairs", "version"]);
                    assert_eq!(document["version"], 1);
                    assert_eq!(document["pairs"], serde_json::json!([]));
                    let diagnostic = document["diagnostics"].as_array().unwrap().iter()
                        .find(|diagnostic| diagnostic["code"] == "duplicate_id")
                        .expect("duplicate immutable IDs must remain diagnosable");
                    assert_eq!(diagnostic.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(),
                        ["code", "message"]);
                    let evidence = diagnostic["message"].as_str().unwrap();
                    assert!(evidence.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
                    assert!(evidence.contains(gateway_source.to_str().unwrap()));
                    assert!(evidence.contains(config.to_str().unwrap()));
                }
                assert!(!String::from_utf8_lossy(&output.stdout).contains("stored-secret"));
                assert!(!String::from_utf8_lossy(&output.stderr).contains("stored-secret"));
                for (source, (bytes, mode)) in sources.iter().zip(&before) {
                    assert_eq!(fs::read(source).unwrap(), *bytes, "{action} {format}: {}", source.display());
                    assert_eq!(fs::metadata(source).unwrap().permissions().mode() & 0o777, *mode);
                }
                assert!(!sentinel.exists(), "{action} {format} invoked OpenSSH");
                assert_eq!(fs::metadata(home.join(".ssh")).unwrap().permissions().mode() & 0o777, 0o700);
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

fn pair_inspection_fixture(name: &str, vm_port: u16) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (root, home, config) = host_edit_fixture(name,
        "Include g v\nHost gateway\n  HostName decoy-gateway.example\nHost vm\n  HostName decoy-vm.example\n");
    let gateway = home.join(".ssh/g");
    fs::write(&gateway,
        "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n##SSHX VM=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\nHost gateway exact-gateway\n  HostName actual-gateway.example\n  LocalForward 2222 127.0.0.1:2222\n  ##PASSWORD stored-secret\n").unwrap();
    let vm = home.join(".ssh/v");
    fs::write(&vm, format!(
        "##SSHX ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n##SSHX GATEWAY=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n##SSHX TRANSIT=127.0.0.1:2222\nHost vm exact-vm\n  HostName actual-vm.example\n  Port {vm_port}\n")).unwrap();
    for source in [&gateway, &vm] {
        fs::set_permissions(source, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(&ssh, "#!/bin/sh\ntouch \"$SSHX_SENTINEL\"\nexit 97\n").unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    (root, home, config)
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
        Self::open_with_env(home, config, arguments, &[])
    }

    fn open_with_env(home: &Path, config: &Path, arguments: &[&str], environment: &[(&str, &str)]) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
        let slave = unsafe { File::from_raw_fd(slave) };
        let child = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .arg("--config").arg(config).args(arguments).env("HOME", home).envs(environment.iter().copied())
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
