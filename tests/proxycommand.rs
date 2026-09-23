use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture(label: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "sshx-proxy-test-{}-{label}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).expect("fixture home should be created");
    (root, home)
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).expect("fixture file should be written");
}

fn executable(script: &Path) {
    let mut permissions = fs::metadata(script)
        .expect("fixture script should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(script, permissions).expect("fixture script should be executable");
}

/// Fake OpenSSH engine for direct sessions: captures the `-F` runtime config,
/// simulates a session-bound master, and prints one shell line.
fn session_ssh(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("fake SSH directory should be created");
    let script = bin.join("ssh");
    write(
        &script,
        r#"#!/bin/sh
config=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  previous="$argument"
done
if [ -n "$SSHX_CAPTURE" ] && [ -f "$config" ]; then
  cat "$config" > "$SSHX_CAPTURE"
fi
case " $* " in
  *" -O check "*) [ -f "$SSHX_STARTED" ] && exit 0; exit 1 ;;
  *" -O exit "*) [ -n "$SSHX_CLOSED" ] && : > "$SSHX_CLOSED"; exit 0 ;;
  *" -N "*)
    [ -n "$SSHX_STARTED" ] && : > "$SSHX_STARTED"
    trap 'exit 0' INT HUP TERM
    while :; do sleep 1; done
    ;;
  *) printf 'direct-shell\n' ;;
esac
"#,
    );
    executable(&script);
    bin
}

/// Fake OpenSSH engine for standalone tunnels: detached `-N` master gated by a
/// marker file, `-O exit` honored, runtime config captured once at master start.
fn tunnel_ssh(root: &Path) -> PathBuf {
    let bin = root.join("tunnel-bin");
    fs::create_dir_all(&bin).expect("fake tunnel SSH directory should be created");
    let script = bin.join("ssh");
    write(
        &script,
        r#"#!/bin/sh
marker="$SSHX_TUNNEL_MARKER"
pid_file="$marker.pid"
config=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  previous="$argument"
done
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
    if [ -n "$SSHX_TUNNEL_CONFIG" ] && [ -f "$config" ]; then
      cat "$config" > "$SSHX_TUNNEL_CONFIG"
    fi
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
    );
    executable(&script);
    bin
}

fn run(home: &Path, bin: &Path, args: &[&str], envs: &[(&str, PathBuf)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
    command.env("HOME", home).env(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    for (key, value) in envs {
        command.env(key, value);
    }
    command.args(args).output().expect("sshx binary should run")
}

#[test]
fn direct_session_copies_exact_block_proxycommand_verbatim_with_tokens() {
    let (root, home) = fixture("direct-session");
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=33333333-3333-4333-8333-333333333333\n",
            "Host direct\n",
            "  HostName direct.example\n",
            "  ProxyCommand cloudflared access ssh --hostname %h\n",
        ),
    );
    let bin = session_ssh(&root);
    let capture = root.join("runtime-config");
    let output = run(
        &home,
        &bin,
        &["connect", "direct", "--no-input"],
        &[
            ("SSHX_CAPTURE", capture.clone()),
            ("SSHX_STARTED", root.join("master-started")),
            ("SSHX_CLOSED", root.join("master-closed")),
        ],
    );
    assert!(
        output.status.success(),
        "status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "direct-shell\n");
    let runtime =
        fs::read_to_string(&capture).expect("runtime config should reach the OpenSSH engine");
    assert!(runtime.contains("Host direct"));
    assert!(
        runtime.contains("ProxyCommand cloudflared access ssh --hostname %h"),
        "ProxyCommand must be copied without expansion or rewriting: {runtime}"
    );
    assert!(runtime.contains("Include /etc/ssh/ssh_config"));
    assert!(!runtime.contains("##SSHX"));
    assert!(root.join("master-started").exists());
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn direct_standalone_tunnel_copies_exact_block_proxycommand_verbatim() {
    let (root, home) = fixture("direct-tunnel");
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=44444444-4444-4444-8444-444444444444\n",
            "Host direct\n",
            "  HostName direct.example\n",
            "  ProxyCommand cloudflared access ssh --hostname %h\n",
        ),
    );
    let bin = tunnel_ssh(&root);
    let marker = root.join("master.started");
    let capture = root.join("runtime-config");
    let started = run(
        &home,
        &bin,
        &[
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
        &[
            ("SSHX_TUNNEL_MARKER", marker.clone()),
            ("SSHX_TUNNEL_CONFIG", capture.clone()),
        ],
    );
    assert!(
        started.status.success(),
        "status={:?} stdout={} stderr={}",
        started.status,
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
    let tunnel = &document["tunnels"][0];
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["master_status"], "responsive");
    let id = tunnel["id"].as_str().unwrap();
    let runtime =
        fs::read_to_string(&capture).expect("runtime config should reach the OpenSSH engine");
    assert!(
        runtime.contains("ProxyCommand cloudflared access ssh --hostname %h"),
        "ProxyCommand must be copied without expansion or rewriting: {runtime}"
    );
    let stopped = run(
        &home,
        &bin,
        &["tunnel", "stop", id, "--format", "json"],
        &[("SSHX_TUNNEL_MARKER", marker.clone())],
    );
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!marker.exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

fn pair_fixture(label: &str, proxy_line: &str, on_gateway: bool) -> (PathBuf, PathBuf) {
    let (root, home) = fixture(label);
    let gateway_proxy = if on_gateway { proxy_line } else { "" };
    let vm_proxy = if on_gateway { "" } else { proxy_line };
    write(
        &home.join(".ssh/config"),
        &format!(
            concat!(
                "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
                "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
                "Host gateway\n",
                "  HostName gateway.example\n",
                "  LocalForward 2200 vm.internal:22\n",
                "{gateway_proxy}",
                "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
                "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
                "##SSHX TRANSIT=vm.internal:22\n",
                "Host vm\n",
                "  HostName vm.internal\n",
                "{vm_proxy}"
            ),
            gateway_proxy = gateway_proxy,
            vm_proxy = vm_proxy
        ),
    );
    (root, home)
}

#[test]
fn paired_route_rejects_proxycommand_on_gateway() {
    let (root, home) = pair_fixture(
        "pair-gateway",
        "  ProxyCommand cloudflared access ssh --hostname %h\n",
        true,
    );
    let bin = session_ssh(&root);
    let output = run(
        &home,
        &bin,
        &["connect", "vm", "--no-input"],
        &[
            ("SSHX_CAPTURE", root.join("runtime-config")),
            ("SSHX_STARTED", root.join("master-started")),
            ("SSHX_CLOSED", root.join("master-closed")),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("PAIR_ROUTE_UNSAFE"), "{text}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn paired_route_rejects_proxycommand_on_vm() {
    let (root, home) = pair_fixture(
        "pair-vm",
        "  ProxyCommand cloudflared access ssh --hostname %h\n",
        false,
    );
    let bin = session_ssh(&root);
    let output = run(
        &home,
        &bin,
        &["connect", "vm", "--no-input"],
        &[
            ("SSHX_CAPTURE", root.join("runtime-config")),
            ("SSHX_STARTED", root.join("master-started")),
            ("SSHX_CLOSED", root.join("master-closed")),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("PAIR_ROUTE_UNSAFE"), "{text}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn proxyjump_remains_unsupported_for_direct_entries() {
    let (root, home) = fixture("proxyjump");
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n  ProxyJump gateway\n",
    );
    let bin = session_ssh(&root);
    let output = run(
        &home,
        &bin,
        &["connect", "direct", "--no-input"],
        &[
            ("SSHX_CAPTURE", root.join("runtime-config")),
            ("SSHX_STARTED", root.join("master-started")),
            ("SSHX_CLOSED", root.join("master-closed")),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("UNSUPPORTED_DIRECTIVE"), "{text}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn token_expansion_in_other_directives_remains_unsupported() {
    let (root, home) = fixture("token-boundary");
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n  IdentityFile %d/id_ed25519\n",
    );
    let bin = session_ssh(&root);
    let output = run(
        &home,
        &bin,
        &["connect", "direct", "--no-input"],
        &[
            ("SSHX_CAPTURE", root.join("runtime-config")),
            ("SSHX_STARTED", root.join("master-started")),
            ("SSHX_CLOSED", root.join("master-closed")),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("UNSUPPORTED_TOKEN_SEMANTICS"), "{text}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}
