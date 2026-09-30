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
    fs::remove_dir_all(root).unwrap();
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
