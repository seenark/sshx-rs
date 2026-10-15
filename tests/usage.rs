#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Fake OpenSSH: a session-bound master that, when `SSHX_LISTEN_PORT` is set,
/// also listens there so sshx sees the service forward as ready.
fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "sshx-usage-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let home = root.join("home");
    let bin = root.join("bin");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        r#"#!/bin/sh
case " $* " in
  *" -O check "*) [ -f "$SSHX_STARTED" ]; exit $? ;;
  *" -O exit "*) exit 0 ;;
  *" -N "*)
    : > "$SSHX_STARTED"
    child=
    if [ -n "$SSHX_LISTEN_PORT" ]; then
      python3 -c 'import socket,sys,time; s=socket.socket(); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(); time.sleep(60)' "$SSHX_LISTEN_PORT" >/dev/null 2>&1 &
      child=$!
    fi
    trap '[ -n "$child" ] && kill "$child" 2>/dev/null; exit 0' INT HUP TERM
    while :; do sleep 0.1; done
    ;;
  *) printf 'shell\n' ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    Fixture { root, home, bin }
}

fn connect(
    fixture: &Fixture,
    config: &Path,
    extra: &[&str],
    listen: Option<u16>,
) -> std::process::Output {
    let started = fixture.root.join("started");
    let _ = fs::remove_file(&started);
    let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
    command
        .env("HOME", &fixture.home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                fixture.bin.display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("SSHX_STARTED", &started)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "--no-input",
        ])
        .args(extra);
    if let Some(port) = listen {
        command.env("SSHX_LISTEN_PORT", port.to_string());
    }
    command.output().unwrap()
}

fn assert_connected(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn sidecar(config: &Path) -> PathBuf {
    config.with_file_name(".config.sshx-usage.json")
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn connect_records_use_count_in_sidecar_beside_config_root() {
    let fixture = fixture();
    let config = fixture.home.join(".ssh/config");
    fs::write(
        &config,
        "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost direct\n  HostName direct.example\n",
    )
    .unwrap();
    let original = fs::read(&config).unwrap();

    assert_connected(&connect(&fixture, &config, &[], None));
    let read =
        || -> Value { serde_json::from_slice(&fs::read(sidecar(&config)).unwrap()).unwrap() };
    let first = read();
    let host = &first["hosts"]["11111111-1111-4111-8111-111111111111"];
    assert_eq!(first["version"], 1);
    assert_eq!(host["count"], 1);
    assert_eq!(host["forwards"], Value::Array(Vec::new()));
    assert!(host["last_used"].as_u64().unwrap() > 0);
    assert_eq!(
        fs::metadata(sidecar(&config)).unwrap().permissions().mode() & 0o777,
        0o600
    );

    assert_connected(&connect(&fixture, &config, &[], None));
    assert_eq!(
        read()["hosts"]["11111111-1111-4111-8111-111111111111"]["count"],
        2
    );
    assert_eq!(
        fs::read(&config).unwrap(),
        original,
        "connecting must not write SSH config"
    );
}

#[test]
fn connect_records_declared_service_forward_under_alias_and_relative_path() {
    let fixture = fixture();
    let config = fixture.home.join(".ssh/config");
    fs::write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PORT 5432\n",
    )
    .unwrap();
    let local = free_port();
    let selector = format!("5432={local}");

    assert_connected(&connect(
        &fixture,
        &config,
        &["--forward", &selector],
        Some(local),
    ));

    let document: Value = serde_json::from_slice(&fs::read(sidecar(&config)).unwrap()).unwrap();
    let host = &document["hosts"]["direct@config"];
    assert_eq!(host["count"], 1);
    assert_eq!(
        host["forwards"],
        serde_json::json!([{"remote": 5432, "local": local, "declared": true}])
    );
}

#[test]
fn unwritable_root_directory_does_not_block_connect_or_create_sidecar() {
    let fixture = fixture();
    let directory = fixture.root.join("readonly");
    fs::create_dir(&directory).unwrap();
    let config = directory.join("config");
    fs::write(&config, "Host direct\n  HostName direct.example\n").unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();

    let output = connect(&fixture, &config, &[], None);
    let created = sidecar(&config).exists();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();

    assert_connected(&output);
    assert!(!created, "no sidecar may appear in a read-only directory");
}

/// Fake OpenSSH for standalone tunnels: the detached `-N` master listens on
/// `SSHX_LISTEN_PORT` and exits on `-O exit`.
fn tunnel_ssh(fixture: &Fixture) -> PathBuf {
    let bin = fixture.root.join("tunnel-bin");
    fs::create_dir_all(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        r#"#!/bin/sh
marker="$SSHX_STARTED"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*)
    [ -f "$marker.pid" ] && kill "$(cat "$marker.pid")" 2>/dev/null
    rm -f "$marker" "$marker.pid"
    exit 0
    ;;
  *" -N "*)
    (
      : > "$marker"
      python3 -c 'import socket,sys,time; s=socket.socket(); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(); time.sleep(60)' "$SSHX_LISTEN_PORT" &
      trap 'kill $! 2>/dev/null; exit 0' INT HUP TERM
      wait
    ) </dev/null >/dev/null 2>/dev/null &
    printf '%s' "$!" > "$marker.pid"
    exit 0
    ;;
esac
exit 0
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn standalone_tunnel_start_records_service_forward() {
    let fixture = fixture();
    let bin = tunnel_ssh(&fixture);
    let config = fixture.home.join(".ssh/config");
    fs::write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PORT 5432\n",
    )
    .unwrap();
    let local = free_port();
    let marker = fixture.root.join("tunnel.started");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sshx"))
            .env("HOME", &fixture.home)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("SSHX_STARTED", &marker)
            .env("SSHX_LISTEN_PORT", local.to_string())
            .args(["--config", config.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap()
    };

    let selector = format!("5432={local}");
    let started = run(&[
        "tunnel",
        "direct",
        "start",
        "direct",
        "--forward",
        &selector,
        "--no-input",
        "--format",
        "json",
    ]);
    assert_connected(&started);
    let id = serde_json::from_slice::<Value>(&started.stdout).unwrap()["tunnels"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let _ = run(&["tunnel", "stop", &id, "--no-input"]);

    let document: Value = serde_json::from_slice(&fs::read(sidecar(&config)).unwrap()).unwrap();
    let host = &document["hosts"]["direct@config"];
    assert_eq!(host["count"], 1);
    assert_eq!(
        host["forwards"],
        serde_json::json!([{"remote": 5432, "local": local, "declared": true}])
    );
}
