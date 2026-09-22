use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture() -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "sshx-tunnel-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let home = root.join("home");
    let bin = root.join("bin");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let config = home.join(".ssh/config");
    fs::write(&config, "Host direct\n  HostName direct.example\n").unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        r#"#!/bin/sh
marker="$SSHX_TUNNEL_MARKER"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*) rm -f "$marker"; exit 0 ;;
  *" -N "*) ( : > "$marker"; trap 'rm -f "$marker"; exit 0' INT HUP TERM; while :; do sleep 1; done ) </dev/null >/dev/null 2>/dev/null & exit 0 ;;
esac
exit 0
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    (root, home, bin)
}

fn run(home: &Path, bin: &Path, marker: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("SSHX_TUNNEL_MARKER", marker)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn standalone_direct_tunnel_survives_launcher_and_stops_by_id() {
    let (root, home, bin) = fixture();
    let marker = root.join("master.started");
    let config = home.join(".ssh/config");
    let started = run(
        &home,
        &bin,
        &marker,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
            "start",
            "direct",
            "-R",
            "127.0.0.1:2222:127.0.0.1:22",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(
        started.status.success(),
        "status={:?} stdout={} stderr={}",
        started.status,
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    let document: Value = serde_json::from_slice(&started.stdout).unwrap();
    let tunnel = &document["tunnels"][0];
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["master_status"], "responsive");
    assert_eq!(tunnel["application_health"], "unknown");
    let id = tunnel["id"].as_str().unwrap();
    assert!(marker.exists());
    let duplicate = run(
        &home,
        &bin,
        &marker,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
            "start",
            "direct",
            "-R",
            "127.0.0.1:2222:127.0.0.1:22",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(duplicate.status.success(), "{duplicate:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&duplicate.stdout).unwrap()["tunnels"][0]["id"],
        id
    );

    let status = run(
        &home,
        &bin,
        &marker,
        &["tunnel", "status", id, "--format", "json"],
    );
    assert!(status.status.success(), "{status:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["tunnels"][0]["master_status"],
        "responsive"
    );

    let stopped = run(
        &home,
        &bin,
        &marker,
        &["tunnel", "stop", id, "--format", "json"],
    );
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!marker.exists());
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap()["tunnels"][0]["state"],
        "stopped"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn standalone_tunnel_rejects_non_loopback_bind_without_opt_in() {
    let (root, home, bin) = fixture();
    let config = home.join(".ssh/config");
    let output = run(
        &home,
        &bin,
        &root.join("master.started"),
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
            "start",
            "direct",
            "-L",
            "0.0.0.0:1234:127.0.0.1:22",
            "--no-input",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("FORWARD_BIND_"));
    fs::remove_dir_all(root).unwrap();
}
