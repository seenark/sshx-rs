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
pid_file="$marker.pid"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*)
    if [ -f "$pid_file" ]; then
      master_pid=$(cat "$pid_file")
      kill "$master_pid" 2>/dev/null || :
      i=0
      while [ -e "$marker" ] && [ "$i" -lt 100 ]; do
        sleep 0.01
        i=$((i + 1))
      done
    fi
    rm -f "$marker" "$pid_file"
    exit 0
    ;;
  *" -N "*)
    (
      : > "$marker"
      child=
      trap 'kill "$child" 2>/dev/null; wait "$child" 2>/dev/null; rm -f "$marker" "$pid_file"; exit 0' INT HUP TERM
      while :; do
        sleep 1 &
        child=$!
        wait "$child"
      done
    ) </dev/null >/dev/null 2>/dev/null &
    printf '%s' "$!" > "$pid_file"
    exit 0
    ;;
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

#[test]
fn standalone_paired_tunnel_owns_both_masters_and_stops_in_reverse() {
    let root = std::env::temp_dir().join(format!(
        "sshx-paired-tunnel-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let home = root.join("home");
    let bin = root.join("bin");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let config = home.join(".ssh/config");
    fs::write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
            "  ##PORT 15432\n",
        ),
    )
    .unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        r#"#!/bin/sh
config=
socket=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  previous="$argument"
done
marker="$socket.started"
pid_file="$socket.pid"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*)
    if [ -f "$pid_file" ]; then
      pid=$(cat "$pid_file")
      kill "$pid" 2>/dev/null || :
      wait "$pid" 2>/dev/null || :
    fi
    rm -f "$marker" "$pid_file"
    if [ -n "$SSHX_PAIRED_CLOSE_LOG" ]; then
      case "$socket" in */vm/*) printf 'VM\n' >> "$SSHX_PAIRED_CLOSE_LOG" ;; *) printf 'GATEWAY\n' >> "$SSHX_PAIRED_CLOSE_LOG" ;; esac
    fi
    exit 0
    ;;
  *" -N "*)
    port=$(awk '/^[[:space:]]*LocalForward[[:space:]]+127\.0\.0\.1:/{split($2,a,":"); print a[2]; exit}' "$config")
    (
      : > "$marker"
      listener=
      if [ -n "$port" ]; then
        python3 -c 'import socket,sys,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(("127.0.0.1",int(sys.argv[1]))); s.listen(); time.sleep(60)' "$port" >/dev/null 2>&1 &
        listener=$!
      fi
      trap 'kill "$listener" 2>/dev/null || :; rm -f "$marker"; exit 0' INT HUP TERM
      while :; do sleep 1; done
    ) </dev/null >/dev/null 2>&1 &
    printf '%s' "$!" > "$pid_file"
    exit 0
    ;;
esac
exit 0
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sshx"))
            .env("HOME", &home)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("SSHX_PAIRED_CLOSE_LOG", root.join("close.log"))
            .args(args)
            .output()
            .unwrap()
    };
    let started = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "start",
        "vm",
        "--forward",
        "15432=15433",
        "--no-input",
        "--format",
        "json",
    ]);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let document: Value = serde_json::from_slice(&started.stdout).unwrap();
    let tunnel = &document["tunnels"][0];
    assert_eq!(tunnel["kind"], "paired");
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["gateway_master_status"], "responsive");
    assert_eq!(tunnel["vm_master_status"], "responsive");
    let id = tunnel["id"].as_str().unwrap();
    let stopped = run(&["tunnel", "paired", "stop", id, "--format", "json"]);
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap()["tunnels"][0]["state"],
        "stopped"
    );
    assert_eq!(
        fs::read_to_string(root.join("close.log")).unwrap(),
        "VM\nGATEWAY\n"
    );
    fs::remove_dir_all(root).unwrap();
}
