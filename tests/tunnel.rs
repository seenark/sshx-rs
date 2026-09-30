use serde_json::Value;
use std::fs;
use std::net::TcpListener;
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
    let id = tunnel["id"].as_str().unwrap();
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["kind"], "direct");
    assert_eq!(tunnel["selected_alias"], "direct");
    assert_eq!(tunnel["source_line"], 1);
    assert_eq!(tunnel["master_status"], "responsive");
    assert_eq!(tunnel["listener_status"], "unknown");
    assert_eq!(tunnel["application_health"], "unknown");
    assert_eq!(tunnel["forwards"][0]["kind"], "R");
    assert_eq!(
        tunnel["forwards"][0]["effective"],
        "127.0.0.1:2222:127.0.0.1:22"
    );
    assert!(marker.exists());
    for path in [home.join(".config/sshx"), home.join(".config/sshx/tunnels")] {
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o700,
            "new private directory must be 0700: {}",
            path.display()
        );
    }
    let duplicate = run(
        &home,
        &bin,
        &marker,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
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
    let listed = run(
        &home,
        &bin,
        &marker,
        &["tunnel", "direct", "list", "--format", "json"],
    );
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&listed.stdout).unwrap()["tunnels"][0]["id"],
        id
    );
    fs::remove_file(&marker).unwrap();
    let down = run(&home, &bin, &marker, &["tunnel", "status", &id, "--format", "json"]);
    assert!(down.status.success(), "{down:?}");
    let down_tunnel = &serde_json::from_slice::<Value>(&down.stdout).unwrap()["tunnels"][0];
    assert_eq!(down_tunnel["state"], "down");
    assert_eq!(down_tunnel["master_status"], "down");
    fs::write(&marker, "responsive").unwrap();
    let registry_path = home.join(".config/sshx/tunnels/registry.json");
    let original_registry = fs::read(&registry_path).unwrap();
    let mut unsafe_registry: Value = serde_json::from_slice(&original_registry).unwrap();
    unsafe_registry["tunnels"][0]["control_socket"] = "/tmp/unproved-sshx-control".into();
    fs::write(&registry_path, serde_json::to_vec_pretty(&unsafe_registry).unwrap()).unwrap();
    let unsafe_stop = run(&home, &bin, &marker, &["tunnel", "stop", id]);
    assert_eq!(unsafe_stop.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unsafe_stop.stderr).contains("REGISTRY_UNSAFE"));
    assert!(marker.exists(), "unproved control reference must not stop master");
    fs::write(&registry_path, original_registry).unwrap();

    let wrong_route = run(
        &home,
        &bin,
        &marker,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "paired",
            "start",
            "direct",
            "--no-input",
        ],
    );
    assert_eq!(wrong_route.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_route.stderr).contains("TUNNEL_ROUTE_MISMATCH"));

    for operation in ["status", "stop", "restart"] {
        let wrong_lifecycle_route = run(
            &home,
            &bin,
            &marker,
            &[
                "--config", config.to_str().unwrap(),
                "tunnel", "paired", operation, id, "--format", "json",
            ],
        );
        assert_eq!(wrong_lifecycle_route.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&wrong_lifecycle_route.stderr).contains("TUNNEL_ROUTE_MISMATCH"),
            "{operation}: {wrong_lifecycle_route:?}"
        );
        assert!(marker.exists(), "wrong-route {operation} must not stop the master");
    }
    let paired_list = run(
        &home, &bin, &marker, &["tunnel", "paired", "list", "--format", "json"],
    );
    assert!(paired_list.status.success(), "{paired_list:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&paired_list.stdout).unwrap()["tunnels"],
        serde_json::json!([])
    );

    let status = run(
        &home,
        &bin,
        &marker,
        &["tunnel", "direct", "status", id, "--format", "json"],
    );
    assert!(status.status.success(), "{status:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["tunnels"][0]["master_status"],
        "responsive"
    );
    let restarted = run(
        &home,
        &bin,
        &marker,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
            "restart",
            id,
            "--format",
            "json",
        ],
    );
    assert!(restarted.status.success(), "{restarted:?}");
    let restarted_document: Value = serde_json::from_slice(&restarted.stdout).unwrap();
    let restarted_tunnel = &restarted_document["tunnels"][0];
    assert_eq!(restarted_tunnel["kind"], "direct");
    assert_eq!(restarted_tunnel["state"], "active");
    let restarted_id = restarted_tunnel["id"].as_str().unwrap();

    let stopped = run(
        &home,
        &bin,
        &marker,
        &[
            "tunnel",
            "direct",
            "stop",
            restarted_id,
            "--format",
            "json",
        ],
    );
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!marker.exists());
    let listed_after_stop = run(
        &home,
        &bin,
        &marker,
        &["tunnel", "list", "--format", "json"],
    );
    assert!(listed_after_stop.status.success(), "{listed_after_stop:?}");
    let listed_document: Value = serde_json::from_slice(&listed_after_stop.stdout).unwrap();
    let stopped_record = listed_document["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == restarted_id)
        .expect("restarted Tunnel must remain in the registry");
    assert_eq!(stopped_record["state"], "stopped");
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
    let service_listeners = (0..3)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect::<Vec<_>>();
    let service_ports = service_listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap().port())
        .collect::<Vec<_>>();
    drop(service_listeners);
    let service_forwards = [
        format!("5432={}", service_ports[0]),
        format!("6379={}", service_ports[1]),
        format!("3001={}", service_ports[2]),
    ];
    fs::write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "  ##PORT 2222\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
            "  ##PORT 5432\n",
            "  ##PORT 6379\n",
            "  ##PORT 3001\n",
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
      i=0
      while [ -e "$marker" ] && [ "$i" -lt 100 ]; do
        sleep 0.01
        i=$((i + 1))
      done
    fi
    rm -f "$marker" "$pid_file"
    if [ -n "$SSHX_PAIRED_CLOSE_LOG" ]; then
      case "$socket" in */vm/*) printf 'VM\n' >> "$SSHX_PAIRED_CLOSE_LOG" ;; *) printf 'GATEWAY\n' >> "$SSHX_PAIRED_CLOSE_LOG" ;; esac
    fi
    exit 0
    ;;
  *" -N "*)
    case "$socket" in
      */vm/*)
        if [ -f "$SSHX_PAIRED_FAIL_VM" ]; then
          echo "Permission denied" >&2
          exit 5
        fi
        ;;
    esac
    ports=$(awk '/^[[:space:]]*LocalForward[[:space:]]+127\.0\.0\.1:/{split($2,a,":"); print a[2]}' "$config")
    (
      : > "$marker"
      listeners=
      for port in $ports; do
        python3 -c 'import socket,sys,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(("127.0.0.1",int(sys.argv[1]))); s.listen(); time.sleep(60)' "$port" >/dev/null 2>&1 &
        listeners="$listeners $!"
      done
      trap 'for listener in $listeners; do kill "$listener" 2>/dev/null || :; wait "$listener" 2>/dev/null || :; done; rm -f "$marker"; exit 0' INT HUP TERM
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
            .env("SSHX_PAIRED_FAIL_VM", root.join("fail-vm"))
            .args(args)
            .output()
            .unwrap()
    };
    let wrong_route = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "direct",
        "start",
        "vm",
        "--forward",
        service_forwards[0].as_str(),
        "--no-input",
    ]);
    assert_eq!(wrong_route.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_route.stderr).contains("TUNNEL_ROUTE_MISMATCH"));

    let gateway_only = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "start",
        "vm",
        "--forward",
        "2222",
        "--no-input",
    ]);
    assert_eq!(gateway_only.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&gateway_only.stderr).contains("FORWARD_NOT_FOUND"));
    assert!(!root.join("close.log").exists());

    let started = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "start",
        "vm",
        "--forward",
        service_forwards[0].as_str(),
        "--forward",
        service_forwards[1].as_str(),
        "--forward",
        service_forwards[2].as_str(),
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
    for operation in ["status", "stop", "restart"] {
        let wrong_lifecycle_route = run(&[
            "--config", config.to_str().unwrap(),
            "tunnel", "direct", operation, id, "--format", "json",
        ]);
        assert_eq!(wrong_lifecycle_route.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&wrong_lifecycle_route.stderr).contains("TUNNEL_ROUTE_MISMATCH"),
            "{operation}: {wrong_lifecycle_route:?}"
        );
        assert!(!root.join("close.log").exists(), "wrong-route {operation} must not close either master");
    }
    let direct_list = run(&["tunnel", "direct", "list", "--format", "json"]);
    assert!(direct_list.status.success(), "{direct_list:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&direct_list.stdout).unwrap()["tunnels"],
        serde_json::json!([])
    );
    let forwards = tunnel["forwards"].as_array().unwrap();
    assert_eq!(forwards.len(), 3);
    for (forward, (remote_port, local_port)) in forwards
        .iter()
        .zip([5432, 6379, 3001].into_iter().zip(&service_ports))
    {
        assert_eq!(forward["remote_port"].as_u64(), Some(remote_port));
        assert_eq!(
            forward["local_port"].as_u64(),
            Some(u64::from(*local_port))
        );
    }
    let automatic_duplicate = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "vm",
        "--forward",
        service_forwards[0].as_str(),
        "--forward",
        service_forwards[1].as_str(),
        "--forward",
        service_forwards[2].as_str(),
        "--no-input",
        "--format",
        "json",
    ]);
    assert!(automatic_duplicate.status.success(), "{automatic_duplicate:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&automatic_duplicate.stdout).unwrap()["tunnels"][0]["id"],
        id
    );
    let listed = run(&["tunnel", "paired", "list", "--format", "json"]);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&listed.stdout).unwrap()["tunnels"][0]["id"],
        id
    );
    let direct_list = run(&["tunnel", "direct", "list", "--format", "json"]);
    assert!(direct_list.status.success(), "{direct_list:?}");
    assert!(
        serde_json::from_slice::<Value>(&direct_list.stdout).unwrap()["tunnels"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let wrong_stop = run(&["tunnel", "direct", "stop", id, "--format", "json"]);
    assert_eq!(wrong_stop.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_stop.stderr).contains("TUNNEL_ROUTE_MISMATCH"));

    let status = run(&["tunnel", "paired", "status", id, "--format", "json"]);
    assert!(status.status.success(), "{status:?}");
    let status_tunnel = &serde_json::from_slice::<Value>(&status.stdout).unwrap()["tunnels"][0];
    assert_eq!(status_tunnel["state"], "active");
    assert_eq!(status_tunnel["master_status"], "responsive");
    assert_eq!(status_tunnel["listener_status"], "bound");

    let original_config = fs::read_to_string(&config).unwrap();
    let changed_config = original_config.replace("HostName vm.internal", "HostName vm.hostname");
    assert_ne!(changed_config, original_config);
    fs::write(&config, changed_config).unwrap();
    let stale_status = run(&["tunnel", "paired", "status", id, "--format", "json"]);
    fs::write(&config, original_config).unwrap();
    assert!(stale_status.status.success(), "{stale_status:?}");
    let stale_tunnel =
        &serde_json::from_slice::<Value>(&stale_status.stdout).unwrap()["tunnels"][0];
    assert_eq!(stale_tunnel["state"], "active");
    assert_eq!(stale_tunnel["master_status"], "responsive");
    assert_eq!(stale_tunnel["gateway_master_status"], "responsive");
    assert_eq!(stale_tunnel["vm_master_status"], "responsive");
    assert_eq!(stale_tunnel["listener_status"], "bound");
    assert!(stale_tunnel["error"]
        .as_str()
        .unwrap()
        .contains("CONFIG_CHANGED"));

    let restarted = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "restart",
        id,
        "--format",
        "json",
    ]);
    assert!(restarted.status.success(), "{restarted:?}");
    let restarted_document: Value = serde_json::from_slice(&restarted.stdout).unwrap();
    let restarted_tunnel = &restarted_document["tunnels"][0];
    assert_eq!(restarted_tunnel["kind"], "paired");
    assert_eq!(restarted_tunnel["state"], "active");
    let restarted_id = restarted_tunnel["id"].as_str().unwrap();
    let duplicate_after_restart = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "start",
        "vm",
        "--forward",
        service_forwards[0].as_str(),
        "--forward",
        service_forwards[1].as_str(),
        "--forward",
        service_forwards[2].as_str(),
        "--no-input",
        "--format",
        "json",
    ]);
    assert!(
        duplicate_after_restart.status.success(),
        "{duplicate_after_restart:?}"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&duplicate_after_restart.stdout).unwrap()["tunnels"][0]
            ["id"],
        restarted_id
    );

    let stopped = run(&[
        "tunnel",
        "paired",
        "stop",
        restarted_id,
        "--format",
        "json",
    ]);
    assert!(stopped.status.success(), "{stopped:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap()["tunnels"][0]["state"],
        "stopped"
    );
    let restarted_stopped = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "restart",
        restarted_id,
        "--format",
        "json",
    ]);
    assert!(restarted_stopped.status.success(), "{restarted_stopped:?}");
    let stopped_restart_document: Value =
        serde_json::from_slice(&restarted_stopped.stdout).unwrap();
    let stopped_restart_tunnel = &stopped_restart_document["tunnels"][0];
    assert_eq!(stopped_restart_tunnel["state"], "active");
    let stopped_restart_id = stopped_restart_tunnel["id"].as_str().unwrap();
    assert_ne!(stopped_restart_id, restarted_id);

    let stopped_again = run(&[
        "tunnel",
        "paired",
        "stop",
        stopped_restart_id,
        "--format",
        "json",
    ]);
    assert!(stopped_again.status.success(), "{stopped_again:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped_again.stdout).unwrap()["tunnels"][0]["state"],
        "stopped"
    );
    assert_eq!(
        fs::read_to_string(root.join("close.log")).unwrap(),
        "VM\nGATEWAY\nVM\nGATEWAY\nVM\nGATEWAY\n"
    );
    fs::write(root.join("fail-vm"), "").unwrap();
    let failed_start = run(&[
        "--config",
        config.to_str().unwrap(),
        "tunnel",
        "paired",
        "start",
        "vm",
        "--forward",
        service_forwards[0].as_str(),
        "--forward",
        service_forwards[1].as_str(),
        "--forward",
        service_forwards[2].as_str(),
        "--no-input",
        "--format",
        "json",
    ]);
    assert_eq!(failed_start.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&failed_start.stderr).contains("VM_AUTH_FAILED"));
    assert_eq!(
        fs::read_to_string(root.join("close.log")).unwrap(),
        "VM\nGATEWAY\nVM\nGATEWAY\nVM\nGATEWAY\nVM\nGATEWAY\n"
    );
    fs::remove_dir_all(root).unwrap();
}
