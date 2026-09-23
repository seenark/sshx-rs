use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);
#[test]
fn version_flag_prints_name_and_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("--version")
        .output()
        .expect("sshx binary should run");

    assert!(output.status.success());
    let expected = format!("sshx {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty());
}

#[test]
fn no_arguments_preserve_bootstrap_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .output()
        .expect("sshx binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Usage: sshx [--version]\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn unknown_argument_preserves_bootstrap_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("unknown")
        .output()
        .expect("sshx binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "sshx: unexpected argument `unknown`\nUsage: sshx [--version]\n"
    );
}

fn fixture_root() -> (PathBuf, PathBuf) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let ordinal = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("sshx-ticket-02-{timestamp}-{ordinal}"));
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).expect("fixture home should be created");
    (root, home)
}

fn canonical_fixture_root() -> (PathBuf, PathBuf) {
    let (root, _) = fixture_root();
    let root = fs::canonicalize(root).expect("fixture root should resolve");
    let home = root.join("home");
    (root, home)
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent should be created");
    }
    fs::write(path, contents).expect("fixture should be written");
}

fn run(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .args(args)
        .output()
        .expect("sshx binary should run")
}

fn run_with_stdin(home: &Path, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("sshx binary should run");
    child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(input)
        .expect("password should be written");
    child.wait_with_output().expect("sshx binary should finish")
}

#[test]
fn host_list_follows_include_order_and_deduplicates_physical_entries() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(
        &ssh.join("config"),
        "Include parts/*.conf\nInclude alias.conf\nHost root\n  HostName root.example\n",
    );
    write(
        &ssh.join("parts/20.conf"),
        "Include shared.conf\nHost duplicate\n  HostName same.example\n",
    );
    write(
        &ssh.join("parts/10.conf"),
        "Host duplicate\n  HostName same.example\n",
    );
    write(
        &ssh.join("shared.conf"),
        "Host shared\n  HostName shared.example\n",
    );
    symlink("parts/10.conf", ssh.join("alias.conf")).expect("fixture symlink should be created");

    let output = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
            "--format",
            "json",
        ],
    );

    assert!(output.status.success(), "{:?}", output);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    let entries = document["entries"]
        .as_array()
        .expect("entries should be an array");
    let aliases = entries
        .iter()
        .map(|entry| {
            entry["aliases"][0]
                .as_str()
                .expect("alias should be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(aliases, ["duplicate", "shared", "duplicate", "root"]);
    assert_eq!(
        entries[0]["provenance"]
            .as_array()
            .expect("provenance should be an array")
            .len(),
        2
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("password"));

    let human = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
        ],
    );
    assert!(human.status.success(), "{:?}", human);
    let human_text = String::from_utf8_lossy(&human.stdout);
    assert!(human_text.contains("source:"));
    assert!(human_text.contains("destination:"));
    assert!(human_text.contains("provenance:"));
    assert!(!human_text.contains("password"));

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn host_entries_with_same_values_remain_distinct_and_show_is_exact() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(&ssh.join("config"), "Include one.conf two.conf\n");
    write(
        &ssh.join("one.conf"),
        "Host copied\n  HostName same.example\n  User same\n  Password secret-one\n",
    );
    write(
        &ssh.join("two.conf"),
        "Host copied\n  HostName same.example\n  User same\n  Password secret-two\n",
    );

    let list = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
            "--format",
            "yaml",
        ],
    );
    assert!(list.status.success(), "{:?}", list);
    let yaml: serde_yaml::Value =
        serde_yaml::from_slice(&list.stdout).expect("YAML output should parse");
    assert_eq!(
        serde_yaml::Deserializer::from_slice(&list.stdout).count(),
        1,
        "YAML output should contain one document"
    );
    assert_eq!(
        yaml["entries"]
            .as_sequence()
            .expect("entries should be a sequence")
            .len(),
        2
    );
    let rendered = String::from_utf8_lossy(&list.stdout);
    assert!(!rendered.contains("secret-one"));
    assert!(!rendered.contains("secret-two"));

    let show = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "show",
            "copied",
            "--format",
            "json",
        ],
    );
    assert!(show.status.success(), "{:?}", show);
    let json: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("show JSON should parse");
    assert_eq!(
        json["entries"]
            .as_array()
            .expect("entries should be an array")
            .len(),
        2
    );
    assert!(!String::from_utf8_lossy(&show.stdout).contains("secret"));
    assert!(!String::from_utf8_lossy(&show.stderr).contains("secret"));

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn include_arguments_keep_written_order() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(&ssh.join("config"), "Include=z/*.conf a/*.conf\n");
    write(
        &ssh.join("z/10.conf"),
        "Host=z-entry\n  HostName=z.example\n",
    );
    write(
        &ssh.join("a/10.conf"),
        "Host=a-entry\n  HostName=a.example\n",
    );

    let output = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{:?}", output);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    let aliases = document["entries"]
        .as_array()
        .expect("entries should be an array")
        .iter()
        .map(|entry| {
            entry["aliases"][0]
                .as_str()
                .expect("alias should be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(aliases, ["z-entry", "a-entry"]);

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn match_stops_host_entry_span_and_destination() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(
        &ssh.join("config"),
        "Host=first\nMatch all\n  HostName=wrong.example\n",
    );

    let output = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{:?}", output);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    let entry = &document["entries"][0];
    assert_eq!(entry["aliases"], serde_json::json!(["first"]));
    assert_eq!(entry["destination"], "first");
    assert_eq!(entry["source"]["line_end"], 1);

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn include_cycles_report_diagnostic_without_looping() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(&ssh.join("config"), "Include a.conf\n");
    write(
        &ssh.join("a.conf"),
        "Host cycle-a\n  HostName cycle-a.example\nInclude b.conf\n",
    );
    write(
        &ssh.join("b.conf"),
        "Host cycle-b\n  HostName cycle-b.example\nInclude a.conf\n",
    );
    let output = run(
        &home,
        &[
            "--config",
            ssh.join("config").to_str().expect("UTF-8 fixture path"),
            "host",
            "list",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{:?}", output);
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    assert_eq!(
        document["diagnostics"]
            .as_array()
            .expect("diagnostics should be an array")
            .len(),
        1
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("include cycle"));

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn setup_registers_existing_roots_and_filters_scope_and_project() {
    let (root, home) = fixture_root();
    let personal = home.join(".ssh/config");
    let work = home.join(".private-key/private-key/config");
    write(
        &personal,
        "Include projects/alpha/hosts.conf\nHost personal\n  HostName personal.example\n",
    );
    write(
        &home.join(".ssh/projects/alpha/hosts.conf"),
        "Host alpha\n  HostName alpha.example\n",
    );
    write(&work, "Host work\n  HostName work.example\n");

    let setup = run(&home, &["setup", "--format", "json"]);
    assert!(setup.status.success(), "{setup:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&setup.stdout).expect("setup JSON should parse");
    let roots = document["roots"]
        .as_array()
        .expect("roots should be an array");
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().any(|root| root["scope"] == "personal"));
    assert!(roots.iter().any(|root| root["scope"] == "work"));
    assert!(!home.join(".private-key/config").exists());

    let work_list = run(
        &home,
        &["host", "list", "--scope", "work", "--format", "json"],
    );
    assert!(work_list.status.success(), "{work_list:?}");
    let work_document: serde_json::Value =
        serde_json::from_slice(&work_list.stdout).expect("work JSON should parse");
    assert_eq!(work_document["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        work_document["entries"][0]["scopes"],
        serde_json::json!(["work"])
    );

    let project_list = run(
        &home,
        &["host", "list", "--project", "alpha", "--format", "json"],
    );
    assert!(project_list.status.success(), "{project_list:?}");
    let project_document: serde_json::Value =
        serde_json::from_slice(&project_list.stdout).expect("project JSON should parse");
    assert_eq!(project_document["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        project_document["entries"][0]["projects"],
        serde_json::json!(["alpha"])
    );

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn connect_requires_host_without_input_and_selects_exact_entries() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(&ssh.join("config"), "Include one.conf two.conf\n");
    write(
        &ssh.join("one.conf"),
        "Host copied\n  HostName one.example\nHost unique\n  HostName unique.example\n",
    );
    write(
        &ssh.join("two.conf"),
        "Host copied\n  HostName two.example\n",
    );

    let missing = run(&home, &["connect", "--no-input"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("HOST_REQUIRED"));

    let ambiguous = run(&home, &["connect", "copied", "--no-input"]);
    assert_eq!(ambiguous.status.code(), Some(2));
    let ambiguous_error = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(ambiguous_error.contains("HOST_AMBIGUOUS"));
    assert!(ambiguous_error.contains("one.conf"));
    assert!(ambiguous_error.contains("two.conf"));

    let unique = run(
        &home,
        &["connect", "unique", "--no-input", "--format", "json"],
    );
    assert!(unique.status.success(), "{unique:?}");
    let unique_document: serde_json::Value =
        serde_json::from_slice(&unique.stdout).expect("unique JSON should parse");
    assert_eq!(unique_document["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        unique_document["entries"][0]["aliases"],
        serde_json::json!(["unique"])
    );

    let list = run(&home, &["host", "list", "--format", "json"]);
    assert!(list.status.success(), "{list:?}");
    let list_document: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("list JSON should parse");
    let exact = list_document["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["source"]["path"]
                .as_str()
                .unwrap()
                .ends_with("one.conf")
        })
        .expect("one.conf entry should exist");
    let id = exact["id"].as_str().unwrap();
    let source = exact["source"]["path"].as_str().unwrap();
    let line = exact["source"]["line_start"].as_u64().unwrap().to_string();

    let by_id = run(
        &home,
        &["connect", "--id", id, "--no-input", "--format", "json"],
    );
    assert!(by_id.status.success(), "{by_id:?}");
    let by_id_document: serde_json::Value =
        serde_json::from_slice(&by_id.stdout).expect("ID JSON should parse");
    assert_eq!(by_id_document["entries"].as_array().unwrap().len(), 1);
    assert_eq!(by_id_document["entries"][0]["id"], id);

    let by_location = run(
        &home,
        &[
            "connect",
            "copied",
            "--source",
            source,
            "--line",
            &line,
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(by_location.status.success(), "{by_location:?}");

    let partial = run(
        &home,
        &["connect", "copied", "--source", source, "--no-input"],
    );
    assert_eq!(partial.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&partial.stderr).contains("SELECTOR_INCOMPLETE"));

    let mismatch = run(
        &home,
        &[
            "connect",
            "copied",
            "--source",
            ssh.join("two.conf").to_str().unwrap(),
            "--line",
            &line,
            "--no-input",
        ],
    );
    assert_eq!(mismatch.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("HOST_MISMATCH"));

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn shared_entry_keeps_personal_and_work_provenance() {
    let (root, home) = fixture_root();
    let personal = home.join(".ssh/config");
    let work = home.join(".private-key/private-key/config");
    let shared = root.join("shared.conf");
    write(
        &personal,
        &format!("Include {}\n", shared.to_str().unwrap()),
    );
    write(&work, &format!("Include {}\n", shared.to_str().unwrap()));
    write(&shared, "Host shared\n  HostName shared.example\n");

    let setup = run(
        &home,
        &[
            "setup",
            "--personal",
            personal.to_str().unwrap(),
            "--work",
            work.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert!(setup.status.success(), "{setup:?}");

    let list = run(&home, &["host", "list", "--format", "json"]);
    assert!(list.status.success(), "{list:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("list JSON should parse");
    let entries = document["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["scopes"],
        serde_json::json!(["personal", "work"])
    );
    assert_eq!(entries[0]["provenance"].as_array().unwrap().len(), 2);

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn no_input_missing_host_fails_before_config_discovery() {
    let (root, home) = fixture_root();
    let output = run(&home, &["connect", "--no-input"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("HOST_REQUIRED"));
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn persistent_sshx_id_selects_exact_entry() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "##SSHX ID=entry-uuid\nHost stable\n  HostName stable.example\n",
    );

    let output = run(
        &home,
        &[
            "connect",
            "--id",
            "entry-uuid",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("connect JSON should parse");
    assert_eq!(document["entries"][0]["id"], "entry-uuid");

    fs::remove_dir_all(root).expect("fixture should be removed");
}
fn fake_ssh(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("fake SSH directory should be created");
    let script = bin.join("ssh");
    write(
        &script,
        r#"#!/bin/sh
args="$*"
if [ -n "$SSHX_CAPTURE" ] && [ -n "$1" ]; then
  config=
  previous=
  for argument in "$@"; do
    if [ "$previous" = "-F" ]; then config="$argument"; fi
    previous="$argument"
  done
  if [ -f "$config" ]; then cat "$config" > "$SSHX_CAPTURE"; fi
fi
case " $args " in
  *" -O check "*) if [ -f "$SSHX_STARTED" ]; then exit 0; fi; exit 1 ;;
  *" -O exit "*)
    [ -n "$SSHX_CLOSED" ] && : > "$SSHX_CLOSED"
    exit 0
    ;;
  *" -N "*)
    if [ "$SSHX_UNKNOWN" = "1" ]; then
      echo "Host key verification failed." >&2
      exit 255
    fi
    if [ "$SSHX_AUTH_FAIL" = "1" ]; then
      echo "Permission denied, please try again." >&2
      exit 5
    fi
    [ -n "$SSHX_STARTED" ] && : > "$SSHX_STARTED"
    trap 'exit 0' INT HUP TERM
    while :; do sleep 1; done
    ;;
  *)
    if [ "$SSHX_HOLD_SHELL" = "1" ]; then
      trap 'exit 0' INT HUP TERM
      while :; do sleep 1; done
    else
      printf 'direct-shell\n'
    fi
    ;;
esac
"#,
    );
    let mut permissions = fs::metadata(&script)
        .expect("fake SSH should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("fake SSH should be executable");
    bin
}
fn fake_sshpass(root: &Path) {
    let script = root.join("bin/sshpass");
    write(
        &script,
        r#"#!/bin/sh
if [ "$1" != "-d" ] || [ -z "$2" ]; then exit 97; fi
password_fd="$2"
shift 2
[ "$1" = "ssh" ] || exit 98
shift
[ -n "$SSHX_PASSPASS_COUNT" ] && printf '1' >> "$SSHX_PASSPASS_COUNT"
[ -n "$SSHX_PASSWORD_CAPTURE" ] && eval "cat <&$password_fd" > "$SSHX_PASSWORD_CAPTURE"
[ -n "$SSHX_ARG_CAPTURE" ] && printf '%s\n' "$*" > "$SSHX_ARG_CAPTURE"
exec ssh "$@"
"#,
    );
    let mut permissions = fs::metadata(&script)
        .expect("fake sshpass should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("fake sshpass should be executable");
}

fn run_fake_ssh(home: &Path, args: &[&str], bin: &Path, root: &Path) -> std::process::Output {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config").as_os_str())
        .env("SSHX_STARTED", root.join("master-started").as_os_str())
        .env("SSHX_CLOSED", root.join("master-closed").as_os_str())
        .env(
            "SSHX_PASSWORD_CAPTURE",
            root.join("password-capture").as_os_str(),
        )
        .env("SSHX_ARG_CAPTURE", root.join("sshpass-args").as_os_str())
        .env(
            "SSHX_PASSPASS_COUNT",
            root.join("sshpass-count").as_os_str(),
        )
        .args(args)
        .output()
        .expect("sshx binary should run")
}

fn run_fake_ssh_owned(
    home: &Path,
    args: &[String],
    bin: &Path,
    root: &Path,
) -> std::process::Output {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config").as_os_str())
        .env("SSHX_STARTED", root.join("master-started").as_os_str())
        .env("SSHX_CLOSED", root.join("master-closed").as_os_str())
        .args(args)
        .output()
        .expect("sshx binary should run")
}

fn paired_fake_ssh(root: &Path) -> PathBuf {
    let bin = root.join("paired-bin");
    fs::create_dir_all(&bin).expect("paired fake SSH directory should be created");
    let script = bin.join("ssh");
    write(
        &script,
        r#"#!/bin/sh
config=
socket=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  previous="$argument"
done
if [ -z "$socket" ]; then socket="$config.sock"; fi
if [ -n "$SSHX_PAIRED_CAPTURE_DIR" ] && printf '%s' "$*" | grep -q -- '-N'; then
  mkdir -p "$SSHX_PAIRED_CAPTURE_DIR"
  key=$(printf '%s' "$socket" | cksum | cut -d' ' -f1)
  cp "$config" "$SSHX_PAIRED_CAPTURE_DIR/$key.config"
fi
case " $* " in
  *" -O check "*) [ -f "$socket.started" ] && exit 0; exit 1 ;;
  *" -O exit "*)
    if grep -q '^Host vm$' "$config"; then role=VM; else role=GATEWAY; fi
    [ -n "$SSHX_PAIRED_CLOSE_LOG" ] && printf '%s\n' "$role" >> "$SSHX_PAIRED_CLOSE_LOG"
    rm -f "$socket.started"
    exit 0
    ;;
  *" -N "*)
    if [ "$SSHX_PAIRED_FAIL_ROLE" = "gateway" ] && grep -q '^Host gateway$' "$config"; then
      echo "Permission denied, please try again." >&2
      exit 5
    fi
    if [ "$SSHX_PAIRED_FAIL_ROLE" = "VM" ] && grep -q '^Host vm$' "$config"; then
      echo "Permission denied, please try again." >&2
      exit 5
    fi
    : > "$socket.started"
    trap 'exit 0' INT HUP TERM
    while :; do sleep 1; done
    ;;
  *) if [ "$SSHX_PAIRED_HOLD_SHELL" = "1" ]; then trap 'exit 0' INT HUP TERM; while :; do sleep 1; done; else printf 'paired-shell\n'; fi ;;
esac
"#,
    );
    let mut permissions = fs::metadata(&script)
        .expect("paired fake SSH should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("paired fake SSH should be executable");
    let sshpass = bin.join("sshpass");
    write(
        &sshpass,
        r#"#!/bin/sh
[ "$1" = "-d" ] || exit 97
password_fd="$2"
shift 2
[ "$1" = "ssh" ] || exit 98
shift
config=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  previous="$argument"
done
if grep -q '^Host gateway$' "$config"; then
  eval "cat <&$password_fd" > "$SSHX_PAIRED_GATEWAY_PASSWORD"
else
  eval "cat <&$password_fd" > "$SSHX_PAIRED_VM_PASSWORD"
fi
exec ssh "$@"
"#,
    );
    let mut permissions = fs::metadata(&sshpass)
        .expect("paired fake sshpass should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&sshpass, permissions).expect("paired fake sshpass should be executable");
    bin
}

fn run_paired_fake_ssh(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("SSHX_PAIRED_CAPTURE_DIR", root.join("captures"))
        .env("SSHX_PAIRED_CLOSE_LOG", root.join("close-log"))
        .env(
            "SSHX_PAIRED_GATEWAY_PASSWORD",
            root.join("gateway-password"),
        )
        .env("SSHX_PAIRED_VM_PASSWORD", root.join("vm-password"))
        .args(args)
        .output()
        .expect("sshx binary should run")
}

fn run_paired_fake_ssh_with_failure(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    role: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("SSHX_PAIRED_CAPTURE_DIR", root.join("captures"))
        .env("SSHX_PAIRED_CLOSE_LOG", root.join("close-log"))
        .env(
            "SSHX_PAIRED_GATEWAY_PASSWORD",
            root.join("gateway-password"),
        )
        .env("SSHX_PAIRED_VM_PASSWORD", root.join("vm-password"))
        .env("SSHX_PAIRED_FAIL_ROLE", role)
        .args(args)
        .output()
        .expect("sshx binary should run")
}

fn spawn_paired_fake_ssh(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("SSHX_PAIRED_CAPTURE_DIR", root.join("captures"))
        .env("SSHX_PAIRED_CLOSE_LOG", root.join("close-log"))
        .env(
            "SSHX_PAIRED_GATEWAY_PASSWORD",
            root.join("gateway-password"),
        )
        .env("SSHX_PAIRED_VM_PASSWORD", root.join("vm-password"))
        .env("SSHX_PAIRED_HOLD_SHELL", "1")
        .args(args)
        .spawn()
        .expect("sshx binary should start")
}

#[test]
fn paired_connect_uses_isolated_route_and_reverse_cleanup() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = concat!(
        "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
        "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
        "Host gateway\n",
        "  HostName gateway.example\n",
        "  User gateway-user\n",
        "  Port 220\n",
        "  LocalForward 2200 vm.internal:22\n",
        "  ##PASSWORD gateway-secret\n",
        "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
        "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
        "##SSHX TRANSIT=vm.internal:22\n",
        "Host vm\n",
        "  HostName vm.internal\n",
        "  User vm-user\n",
        "  Port 22\n",
        "  ##PASSWORD vm-secret\n",
    );
    write(&config, original);
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = paired_fake_ssh(&root);
    let output = run_paired_fake_ssh(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "vm",
            "--no-input",
        ],
        &bin,
        &root,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "paired-shell\n");
    assert_eq!(
        fs::read_to_string(root.join("gateway-password")).unwrap(),
        "gateway-secret\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("vm-password")).unwrap(),
        "vm-secret\n"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), original);

    let mut gateway_runtime = String::new();
    let mut vm_runtime = String::new();
    for capture in fs::read_dir(root.join("captures")).unwrap() {
        let text = fs::read_to_string(capture.unwrap().path()).unwrap();
        if text.contains("Host gateway\n") {
            gateway_runtime = text;
        } else if text.contains("Host vm\n") {
            vm_runtime = text;
        }
    }
    assert!(gateway_runtime.contains("HostName gateway.example"));
    assert!(gateway_runtime.contains("Port 220"));
    assert!(gateway_runtime.contains("LocalForward 127.0.0.1:"));
    assert!(gateway_runtime.contains(" vm.internal:22"));
    assert!(!gateway_runtime.contains("gateway-secret"));
    assert!(!gateway_runtime.contains("##SSHX"));

    let transit_port = gateway_runtime
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next() == Some("LocalForward"))
                .then(|| fields.next()?.rsplit_once(':')?.1.parse::<u16>().ok())?
        })
        .expect("gateway runtime should select transit port");
    assert!(transit_port > 0);
    assert!(vm_runtime.contains("HostName 127.0.0.1"));
    assert!(vm_runtime.contains(&format!("Port {transit_port}")));
    assert!(vm_runtime.contains("User vm-user"));
    assert!(vm_runtime.contains("HostKeyAlias sshx-vm-22222222-2222-4222-8222-222222222222"));
    assert!(!vm_runtime.contains("vm-secret"));
    assert!(!vm_runtime.contains("##SSHX"));
    assert_eq!(
        fs::read_to_string(root.join("close-log")).unwrap(),
        "VM\nGATEWAY\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn paired_connect_attributes_gateway_auth_failure() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "  ##PASSWORD gateway-secret\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = paired_fake_ssh(&root);
    let output = run_paired_fake_ssh_with_failure(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "vm",
            "--no-input",
        ],
        &bin,
        &root,
        "gateway",
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("GATEWAY_AUTH_FAILED"), "{text}");
    assert!(!text.contains("gateway-secret"), "{text}");
    assert!(!root.join("vm-password").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn paired_connect_attributes_vm_auth_failure_and_cleans_gateway() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "  ##PASSWORD gateway-secret\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
            "  ##PASSWORD vm-secret\n",
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = paired_fake_ssh(&root);
    let output = run_paired_fake_ssh_with_failure(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "vm",
            "--no-input",
        ],
        &bin,
        &root,
        "VM",
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("VM_AUTH_FAILED"), "{text}");
    assert!(!text.contains("gateway-secret"), "{text}");
    assert!(!text.contains("vm-secret"), "{text}");
    assert_eq!(
        fs::read_to_string(root.join("close-log")).unwrap(),
        "VM\nGATEWAY\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn paired_connect_rejects_route_changes_before_open_ssh() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 changed.internal:22\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let bin = paired_fake_ssh(&root);
    let output = run_paired_fake_ssh(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "vm",
            "--no-input",
        ],
        &bin,
        &root,
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("PAIR_ROUTE_CHANGED"));
    assert!(!root.join("captures").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn paired_sessions_use_distinct_ports_and_cleanup_independently() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
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
            "  User vm-user\n",
            "  Port 22\n",
        ),
    );
    let bin = paired_fake_ssh(&root);
    let args = [
        "--config",
        config.to_str().unwrap(),
        "connect",
        "vm",
        "--no-input",
    ];
    let mut first = spawn_paired_fake_ssh(&home, &args, &bin, &root);
    for _ in 0..200 {
        let count = fs::read_dir(root.join("captures"))
            .map(|entries| entries.count())
            .unwrap_or_default();
        if count >= 2 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut second = spawn_paired_fake_ssh(&home, &args, &bin, &root);
    for _ in 0..200 {
        let count = fs::read_dir(root.join("captures"))
            .map(|entries| entries.count())
            .unwrap_or_default();
        if count >= 4 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut ports = Vec::new();
    for capture in fs::read_dir(root.join("captures")).unwrap() {
        let text = fs::read_to_string(capture.unwrap().path()).unwrap();
        if let Some(line) = text
            .lines()
            .find(|line| line.starts_with("  LocalForward "))
        {
            ports.push(
                line.split_whitespace()
                    .nth(1)
                    .unwrap()
                    .rsplit_once(':')
                    .unwrap()
                    .1
                    .parse::<u16>()
                    .unwrap(),
            );
        }
    }
    assert_eq!(ports.len(), 2);
    assert_ne!(ports[0], ports[1]);

    assert_eq!(
        unsafe { libc::kill(first.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert_eq!(first.wait().unwrap().code(), Some(2));
    thread::sleep(Duration::from_millis(100));
    assert!(second.try_wait().unwrap().is_none());
    assert_eq!(
        unsafe { libc::kill(second.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert_eq!(second.wait().unwrap().code(), Some(2));
    let close_log = fs::read_to_string(root.join("close-log")).unwrap();
    let roles = close_log.lines().collect::<Vec<_>>();
    assert_eq!(roles.len(), 4);
    assert_eq!(&roles[..2], ["VM", "GATEWAY"]);
    assert_eq!(&roles[2..], ["VM", "GATEWAY"]);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn direct_connect_compiles_exact_block_and_uses_owned_master() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "##SSHX ID=direct-id\nHost direct\n  HostName direct.example # remove this\n  User alice\n  IdentityFile ~/.ssh/id_ed25519\n  ##PASSWORD never-copy-this\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let output = run_fake_ssh(&home, &["connect", "direct", "--no-input"], &bin, &root);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "direct-shell\n");
    let runtime =
        fs::read_to_string(root.join("runtime-config")).expect("runtime config should be captured");
    assert!(runtime.contains("Host direct"));
    assert!(runtime.contains("HostName direct.example"));
    assert!(runtime.contains("IdentityFile ~/.ssh/id_ed25519"));
    assert!(runtime.contains("Include /etc/ssh/ssh_config"));
    assert!(!runtime.contains("##SSHX"));
    assert!(!runtime.contains("PASSWORD"));
    assert!(!runtime.contains("remove this"));
    assert!(root.join("master-started").exists());
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn direct_service_forward_preflight_preserves_busy_external_listener() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PORT 5432\n",
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("external listener should bind");
    let local_port = listener
        .local_addr()
        .expect("external listener address")
        .port();
    let bin = fake_ssh(&root);
    let args = vec![
        "--config".to_string(),
        config.to_string_lossy().into_owned(),
        "connect".to_string(),
        "direct".to_string(),
        "--no-input".to_string(),
        "--forward".to_string(),
        format!("5432={local_port}"),
    ];
    let output = run_fake_ssh_owned(&home, &args, &bin, &root);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("SERVICE_BIND_FAILED"), "{text}");
    assert!(!root.join("master-started").exists());
    assert!(listener.local_addr().is_ok());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn direct_password_uses_selected_metadata_once_through_anonymous_fd() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host sibling\n  HostName sibling.example\n  ##PASSWORD sibling-secret\nHost selected\n  HostName selected.example\n  ##PASSWORD selected-secret\n",
    );
    fs::set_permissions(home.join(".ssh/config"), fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let output = run_fake_ssh(&home, &["connect", "selected", "--no-input"], &bin, &root);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("password-capture")).unwrap(),
        "selected-secret\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("sshpass-count"))
            .unwrap()
            .len(),
        1
    );
    let args = fs::read_to_string(root.join("sshpass-args")).unwrap();
    assert!(!args.contains("selected-secret"));
    assert!(!args.contains("sibling-secret"));
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(!runtime.contains("selected-secret"));
    assert!(!runtime.contains("sibling-secret"));
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("selected-secret"));
    assert!(!output_text.contains("sibling-secret"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn wrong_configured_password_fails_without_retry_in_no_input_mode() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(home.join(".ssh/config"), fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config").as_os_str())
        .env("SSHX_STARTED", root.join("master-started").as_os_str())
        .env("SSHX_CLOSED", root.join("master-closed").as_os_str())
        .env("SSHX_AUTH_FAIL", "1")
        .env(
            "SSHX_PASSWORD_CAPTURE",
            root.join("password-capture").as_os_str(),
        )
        .env(
            "SSHX_PASSPASS_COUNT",
            root.join("sshpass-count").as_os_str(),
        )
        .args(["connect", "selected", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("SSH_AUTH_FAILED"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Password for direct host"));
    assert_eq!(
        fs::read_to_string(root.join("password-capture")).unwrap(),
        "configured-secret\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("sshpass-count"))
            .unwrap()
            .len(),
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_password_fd_is_consumed_without_password_argument() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host selected\n  HostName selected.example\n",
    );
    let password_file = root.join("caller-password");
    write(&password_file, "fd-secret\n");
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("sh")
        .env("HOME", &home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config").as_os_str())
        .env("SSHX_STARTED", root.join("master-started").as_os_str())
        .env("SSHX_CLOSED", root.join("master-closed").as_os_str())
        .env(
            "SSHX_PASSWORD_CAPTURE",
            root.join("password-capture").as_os_str(),
        )
        .env("SSHX_ARG_CAPTURE", root.join("sshpass-args").as_os_str())
        .arg("-c")
        .arg(r#"exec 3<"$1"; exec "$2" connect selected --password-fd 3 --no-input"#)
        .arg("sshx-fd-test")
        .arg(password_file.as_os_str())
        .arg(env!("CARGO_BIN_EXE_sshx"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("password-capture")).unwrap(),
        "fd-secret\n"
    );
    assert!(
        !fs::read_to_string(root.join("sshpass-args"))
            .unwrap()
            .contains("fd-secret")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn key_only_entry_uses_plain_ssh_without_sshpass() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host selected\n  HostName selected.example\n  IdentityFile ~/.ssh/id_ed25519\n",
    );
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let output = run_fake_ssh(&home, &["connect", "selected", "--no-input"], &bin, &root);
    assert!(output.status.success(), "{output:?}");
    assert!(!root.join("sshpass-count").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsafe_config_fails_before_open_ssh_evaluation() {
    let (root, home) = fixture_root();
    let bin = fake_ssh(&root);
    let cases = [
        (
            "wildcard",
            "exact*",
            "Host exact*\n  HostName example.test\n",
            "UNSUPPORTED_WILDCARD",
        ),
        (
            "match",
            "exact",
            "Host exact\n  HostName example.test\nMatch all\n  User inherited\n",
            "UNSUPPORTED_MATCH",
        ),
        (
            "conditional-include",
            "exact",
            "Host exact\n  Include child.conf\n",
            "UNSUPPORTED_CONDITIONAL_INCLUDE",
        ),
        (
            "global",
            "exact",
            "User inherited\nHost exact\n  HostName example.test\n",
            "UNSUPPORTED_GLOBAL",
        ),
        (
            "token",
            "exact",
            "Host exact\n  HostName %h\n",
            "UNSUPPORTED_TOKEN_SEMANTICS",
        ),
    ];
    for (name, selector, contents, expected) in cases {
        let config = home.join(".ssh/config");
        write(&config, contents);
        let invoked = root.join("master-started");
        let _ = fs::remove_file(&invoked);
        let output = run_fake_ssh(&home, &["connect", selector, "--no-input"], &bin, &root);
        assert_eq!(output.status.code(), Some(2), "{name}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{name}: {output:?}"
        );
        assert!(!invoked.exists(), "{name}: OpenSSH was invoked");
    }
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn unknown_key_in_no_input_mode_returns_stable_trust_error() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n",
    );
    let bin = fake_ssh(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("SSHX_UNKNOWN", "1")
        .args(["connect", "direct", "--no-input"])
        .output()
        .expect("sshx binary should run");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("HOST_KEY_TRUST_REQUIRED"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("direct-shell"));
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn signal_cleanup_closes_only_owned_master() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n",
    );
    let bin = fake_ssh(&root);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", path)
        .env("SSHX_STARTED", root.join("master-started").as_os_str())
        .env("SSHX_CLOSED", root.join("master-closed").as_os_str())
        .env("SSHX_HOLD_SHELL", "1")
        .args(["connect", "direct", "--no-input"])
        .spawn()
        .expect("sshx should start");
    for _ in 0..250 {
        if root.join("master-started").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(root.join("master-started").exists());
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(result, 0);
    let status = child.wait().expect("sshx should stop after SIGTERM");
    assert_eq!(status.code(), Some(2));
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn host_create_preview_redacts_and_applies_unique_ids() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = b"# keep\nHost existing\n  HostName old.example\n";
    write(&config, std::str::from_utf8(original).unwrap());
    let config_text = config.to_str().unwrap();
    let preview = run_with_stdin(
        &home,
        &[
            "--config",
            config_text,
            "host",
            "create",
            "--scope",
            "personal",
            "--folder",
            home.join(".ssh").to_str().unwrap(),
            "--file",
            "created.conf",
            "--alias",
            "created",
            "--hostname",
            "created.example",
            "--user",
            "alice",
            "--port",
            "2222",
            "--password-stdin",
            "--preview",
            "--no-input",
            "--format",
            "json",
        ],
        b"secret-value\n",
    );
    assert!(preview.status.success(), "{preview:?}");
    assert!(!home.join(".ssh/created.conf").exists());
    let preview_text = String::from_utf8_lossy(&preview.stdout);
    assert!(preview_text.contains("created.conf"));
    assert!(preview_text.contains("<redacted>"));
    assert!(!preview_text.contains("secret-value"));
    let preview_document: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("preview should be one JSON document");
    assert_eq!(preview_document["applied"], false);
    assert_eq!(fs::read(&config).unwrap(), original);

    let apply = run_with_stdin(
        &home,
        &[
            "--config",
            config_text,
            "host",
            "create",
            "--scope",
            "personal",
            "--folder",
            home.join(".ssh").to_str().unwrap(),
            "--file",
            "created.conf",
            "--alias",
            "created",
            "--hostname",
            "created.example",
            "--user",
            "alice",
            "--port",
            "2222",
            "--password-stdin",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
        b"secret-value\n",
    );
    assert!(apply.status.success(), "{apply:?}");
    let apply_text = String::from_utf8_lossy(&apply.stdout);
    assert!(!apply_text.contains("secret-value"));
    let created = home.join(".ssh/created.conf");
    let created_text = fs::read_to_string(&created).unwrap();
    assert!(created_text.contains("##PASSWORD secret-value"));
    assert_eq!(&fs::read(&config).unwrap()[..original.len()], original);
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .ends_with("Include created.conf\n")
    );
    assert_eq!(
        fs::metadata(&created).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let second = run(
        &home,
        &[
            "--config",
            config_text,
            "host",
            "create",
            "--scope",
            "personal",
            "--folder",
            home.join(".ssh").to_str().unwrap(),
            "--file",
            "created.conf",
            "--alias",
            "created-two",
            "--hostname",
            "created-two.example",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(second.status.success(), "{second:?}");
    let list = run(
        &home,
        &["--config", config_text, "host", "list", "--format", "json"],
    );
    assert!(list.status.success(), "{list:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("host list should parse");
    let ids = document["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|alias| alias == "created" || alias == "created-two")
        })
        .map(|entry| entry["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(
        fs::read_to_string(&config)
            .unwrap()
            .matches("Include created.conf")
            .count(),
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_create_respects_wildcard_include_and_crlf_modes() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = b"Include hosts/*.conf\r\n# unrelated\r\n";
    fs::create_dir_all(home.join(".ssh/hosts")).unwrap();
    fs::write(&config, original).unwrap();
    let mut permissions = fs::metadata(&config).unwrap().permissions();
    permissions.set_mode(0o640);
    fs::set_permissions(&config, permissions).unwrap();
    let output = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--scope",
            "personal",
            "--folder",
            home.join(".ssh/hosts").to_str().unwrap(),
            "--file",
            "new.conf",
            "--alias",
            "wild",
            "--hostname",
            "wild.example",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("create output should parse");
    assert_eq!(document["files"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(&config).unwrap(), original);
    let target = home.join(".ssh/hosts/new.conf");
    let target_bytes = fs::read(&target).unwrap();
    assert!(target_bytes.windows(2).any(|window| window == b"\r\n"));
    assert!(!target_bytes.windows(2).any(|window| window == b"\n\n"));
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o640
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_create_selects_registered_project_root() {
    let (root, home) = fixture_root();
    let work = home.join("work/config");
    write(&work, "Host work\n  HostName work.example\n");
    let setup = run(
        &home,
        &[
            "setup",
            "--project",
            "alpha",
            "--work",
            work.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert!(setup.status.success(), "{setup:?}");
    let target_folder = home.join("work/hosts");
    let create = run(
        &home,
        &[
            "host",
            "create",
            "--scope",
            "work",
            "--project",
            "alpha",
            "--folder",
            target_folder.to_str().unwrap(),
            "--file",
            "created.conf",
            "--alias",
            "created",
            "--hostname",
            "created.example",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(create.status.success(), "{create:?}");
    assert!(target_folder.join("created.conf").is_file());
    let document: serde_json::Value =
        serde_json::from_slice(&create.stdout).expect("create output should parse");
    assert_eq!(document["files"].as_array().unwrap().len(), 2);
    assert!(
        document["files"][1]["patch"]
            .as_str()
            .unwrap()
            .contains(target_folder.join("created.conf").to_str().unwrap())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_create_requires_consent_without_prompting() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host root\n  HostName root.example\n");
    let output = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--scope",
            "personal",
            "--folder",
            home.join(".ssh").to_str().unwrap(),
            "--file",
            "new.conf",
            "--alias",
            "new",
            "--hostname",
            "new.example",
            "--no-input",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("CONSENT_REQUIRED"));
    assert!(!home.join(".ssh/new.conf").exists());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_update_changes_selected_block_only_and_preserves_id_and_bytes() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = concat!(
        "# before\r\n",
        "##SSHX ID=11111111-1111-4111-8111-111111111111\r\n",
        "Host target\r\n",
        "  HostName old.example # target comment\r\n",
        "  User old\r\n",
        "  Port 22\r\n",
        "  ##PASSWORD old-secret\r\n",
        "# between\r\n",
        "Host unrelated\r\n",
        "  HostName unrelated.example\r\n",
        "# after\r\n",
    )
    .as_bytes();
    fs::write(&config, original).unwrap();

    let output = run_with_stdin(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "update",
            "target",
            "--hostname",
            "new.example",
            "--user",
            "new-user",
            "--port",
            "2200",
            "--password-stdin",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
        b"new-secret\n",
    );
    assert!(output.status.success(), "{output:?}");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(!rendered.contains("old-secret"));
    assert!(!rendered.contains("new-secret"));

    let updated = fs::read(&config).unwrap();
    assert!(updated.starts_with(b"# before\r\n"));
    assert!(updated.ends_with(b"# after\r\n"));
    assert!(updated.windows(2).any(|window| window == b"\r\n"));
    assert!(String::from_utf8_lossy(&updated).contains("Host target\r\n"));
    assert!(
        String::from_utf8_lossy(&updated).contains("HostName new.example # target comment\r\n")
    );
    assert!(String::from_utf8_lossy(&updated).contains("User new-user\r\n"));
    assert!(String::from_utf8_lossy(&updated).contains("Port 2200\r\n"));
    assert!(String::from_utf8_lossy(&updated).contains("##PASSWORD new-secret\r\n"));
    assert!(String::from_utf8_lossy(&updated).contains("Host unrelated\r\n"));
    assert!(String::from_utf8_lossy(&updated).contains("  HostName unrelated.example\r\n"));
    assert_eq!(updated[..20], original[..20]);

    let list = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "list",
            "--format",
            "json",
        ],
    );
    assert!(list.status.success(), "{list:?}");
    let document: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let target = document["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|alias| alias == "target")
        })
        .unwrap();
    assert_eq!(
        target["id"].as_str().unwrap(),
        "11111111-1111-4111-8111-111111111111"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_rename_preserves_id_and_delete_removes_one_block() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "Host old\n",
            "  HostName old.example\n",
            "Host same\n",
            "  HostName same.example\n",
        ),
    );

    let rename = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "rename",
            "old",
            "--alias",
            "renamed",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(rename.status.success(), "{rename:?}");
    let renamed = String::from_utf8_lossy(&fs::read(&config).unwrap()).into_owned();
    assert!(renamed.contains("Host renamed\n"));
    assert!(renamed.contains("Host same\n"));
    assert!(!renamed.contains("Host old\n"));

    let list = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "list",
            "--format",
            "json",
        ],
    );
    let document: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let entry = document["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|alias| alias == "renamed")
        })
        .unwrap();
    assert_eq!(
        entry["id"].as_str().unwrap(),
        "22222222-2222-4222-8222-222222222222"
    );

    let delete = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "delete",
            "renamed",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(delete.status.success(), "{delete:?}");
    let remaining = String::from_utf8_lossy(&fs::read(&config).unwrap()).into_owned();
    assert!(!remaining.contains("Host renamed\n"));
    assert!(!remaining.contains("22222222-2222-4222-8222-222222222222"));
    assert!(remaining.contains("Host same\n"));
    assert!(remaining.contains("  HostName same.example\n"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_mutation_rejects_stale_source_selection_and_pair_reference() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host first\n",
            "  HostName first.example\n",
            "##SSHX ID=33333333-3333-4333-8333-333333333333\n",
            "Host target\n",
            "  HostName target.example\n",
        ),
    );
    let stale = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "rename",
            "target",
            "--source",
            config.to_str().unwrap(),
            "--line",
            "1",
            "--alias",
            "new-target",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(stale.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("HOST_MISMATCH"));

    write(
        &home.join(".config/sshx/pairs.json"),
        "{\"gateway_id\":\"33333333-3333-4333-8333-333333333333\"}\n",
    );
    let blocked = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "delete",
            "target",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(blocked.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("DELETE_REFERENCED"));
    assert!(fs::read_to_string(&config).unwrap().contains("Host target"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_mutation_rejects_symlinked_config_root() {
    let (root, home) = fixture_root();
    let real = home.join(".ssh/real-config");
    let link = home.join(".ssh/config");
    write(&real, "Host target\n  HostName target.example\n");
    symlink(&real, &link).expect("fixture symlink should be created");

    let output = run(
        &home,
        &[
            "--config",
            link.to_str().unwrap(),
            "host",
            "rename",
            "target",
            "--alias",
            "renamed",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("CONFIG_ROOT_SYMLINK"));
    assert_eq!(
        fs::read_to_string(real).unwrap(),
        "Host target\n  HostName target.example\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_mutation_outputs_redact_secret_for_human_json_and_yaml() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=44444444-4444-4444-8444-444444444444\n",
            "Host secret-host\n",
            "  HostName secret.example\n",
            "  ##PASSWORD old-secret\n",
        ),
    );
    for format in ["human", "json", "yaml"] {
        let output = run_with_stdin(
            &home,
            &[
                "--config",
                config.to_str().unwrap(),
                "host",
                "update",
                "secret-host",
                "--hostname",
                "new.example",
                "--password-stdin",
                "--preview",
                "--no-input",
                "--format",
                format,
            ],
            b"new-secret\n",
        );
        assert!(output.status.success(), "{format}: {output:?}");
        let rendered = String::from_utf8_lossy(&output.stdout);
        assert!(!rendered.contains("old-secret"));
        assert!(!rendered.contains("new-secret"));
        assert!(!rendered.contains("Host secret-host"));
        if format == "json" {
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .expect("JSON mutation output should parse");
        } else if format == "yaml" {
            serde_yaml::from_slice::<serde_yaml::Value>(&output.stdout)
                .expect("YAML mutation output should parse");
        }
    }
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        concat!(
            "##SSHX ID=44444444-4444-4444-8444-444444444444\n",
            "Host secret-host\n",
            "  HostName secret.example\n",
            "  ##PASSWORD old-secret\n",
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_assigns_ids_and_infers_unique_transit() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let output = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gateway_id = document["gateway_id"].as_str().unwrap();
    let vm_id = document["vm_id"].as_str().unwrap();
    assert_ne!(gateway_id, vm_id);
    assert_eq!(document["transit_host"], "vm.internal");
    assert_eq!(document["transit_port"], 22);
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains(&format!("##SSHX ID={gateway_id}")));
    assert!(text.contains(&format!("##SSHX ID={vm_id}")));
    assert!(text.contains(&format!("##SSHX GATEWAY={gateway_id}")));
    assert!(text.contains("##SSHX TRANSIT=vm.internal:22"));
    assert!(text.contains(&format!("##SSHX VM={vm_id}")));

    let listed = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "list",
            "--format",
            "json",
        ],
    );
    assert!(listed.status.success(), "{listed:?}");
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["pairs"].as_array().unwrap().len(), 1);
    assert!(listed["diagnostics"].as_array().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_requires_explicit_transit_for_ambiguous_candidates() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 first.internal:22\n",
            "  LocalForward 2201 second.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let missing = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("TRANSIT_REQUIRED"));
    assert!(!fs::read_to_string(&config).unwrap().contains("##SSHX ID="));

    let explicit = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--transit-host",
            "chosen.internal",
            "--transit-port",
            "22",
            "--yes",
            "--no-input",
        ],
    );
    assert!(explicit.status.success(), "{explicit:?}");
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("##SSHX TRANSIT=chosen.internal:22")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_rejects_proxy_routes_and_duplicate_ids() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  ProxyJump bastion\n",
            "  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let duplicate = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate_id"));
    assert!(!fs::read_to_string(&config).unwrap().contains("GATEWAY="));

    write(
        &config,
        concat!(
            "ProxyJump bastion\n",
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  ProxyJump bastion\n",
            "  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let proxy = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(proxy.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&proxy.stderr).contains("PAIR_ROUTE_UNSAFE"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_references_block_delete_and_report_external_breakage() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let setup = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
            "--format",
            "json",
        ],
    );
    assert!(setup.status.success(), "{setup:?}");
    let document: serde_json::Value = serde_json::from_slice(&setup.stdout).unwrap();
    let gateway_id = document["gateway_id"].as_str().unwrap().to_string();
    let vm_id = document["vm_id"].as_str().unwrap().to_string();
    for id in [&gateway_id, &vm_id] {
        let blocked = run(
            &home,
            &[
                "--config",
                config.to_str().unwrap(),
                "host",
                "delete",
                "--id",
                id,
                "--yes",
                "--no-input",
            ],
        );
        assert_eq!(blocked.status.code(), Some(2), "{blocked:?}");
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("DELETE_REFERENCED"));
    }
    let broken = fs::read_to_string(&config)
        .unwrap()
        .replace(&format!("##SSHX GATEWAY={gateway_id}\n"), "");
    fs::write(&config, broken).unwrap();
    let validate = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "validate",
            "--format",
            "json",
        ],
    );
    assert!(validate.status.success(), "{validate:?}");
    let document: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert!(
        document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "broken_reference")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_requires_exact_source_for_duplicate_aliases() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host duplicate\n",
            "  HostName first.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "Host duplicate\n",
            "  HostName second.example\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let ambiguous = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "duplicate",
            "vm",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("HOST_AMBIGUOUS"));
    let exact = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "duplicate",
            "vm",
            "--gateway-source",
            config.to_str().unwrap(),
            "--gateway-line",
            "1",
            "--yes",
            "--no-input",
        ],
    );
    assert!(exact.status.success(), "{exact:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_rejects_malformed_ids_without_regeneration() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = concat!(
        "##SSHX ID=not-a-uuid\n",
        "Host gateway\n",
        "  HostName gateway.example\n",
        "  LocalForward 2200 vm.internal:22\n",
        "Host vm\n",
        "  HostName vm.internal\n",
        "  Port 22\n",
    );
    write(&config, original);
    let output = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("malformed_id"));
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pair_setup_enforces_gateway_cardinality_by_entry_identity() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host gateway-one\n",
            "  HostName shared.gateway.example\n",
            "  LocalForward 2200 vm-one.internal:22\n",
            "Host vm-one\n",
            "  HostName vm-one.internal\n",
            "Host gateway-two\n",
            "  HostName shared.gateway.example\n",
            "  LocalForward 2201 vm-two.internal:22\n",
            "Host vm-two\n",
            "  HostName vm-two.internal\n",
        ),
    );
    let setup_one = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway-one",
            "vm-one",
            "--yes",
            "--no-input",
        ],
    );
    assert!(setup_one.status.success(), "{setup_one:?}");
    let setup_two = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway-two",
            "vm-two",
            "--yes",
            "--no-input",
        ],
    );
    assert!(setup_two.status.success(), "{setup_two:?}");
    let reuse = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway-one",
            "vm-two",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(reuse.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&reuse.stderr).contains("PAIR_GATEWAY_IN_USE"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_fix_permissions_skips_without_tty_and_keeps_modes() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let output = run_with_stdin(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "doctor",
            "--fix-permissions",
            "--no-input",
            "--format",
            "json",
        ],
        b"y\n",
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor JSON should parse");
    assert!(
        document["repair_results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["outcome"] == "skipped"),
        "repair should be skipped: {document}"
    );
    assert_eq!(mode_of(&config), 0o644);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn doctor_fix_permissions_applies_after_one_tty_confirmation() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "doctor",
            "--fix-permissions",
        ],
        &bin,
        &root,
        b"y\n",
        b"Repair plan:",
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(output.matches("Repair plan:").count(), 1, "output={output}");
    assert!(output.contains("fixed:"), "doctor output={output}");
    assert_eq!(mode_of(&config), 0o600);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn doctor_fix_permissions_yes_still_requires_confirmation() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "doctor",
            "--fix-permissions",
            "--yes",
        ],
        &bin,
        &root,
        b"n\n",
        b"Repair plan:",
    );
    assert_eq!(status.code(), Some(2), "status={status:?} output={output}");
    assert!(
        output.contains("Apply permission repairs?"),
        "output={output}"
    );
    assert_eq!(mode_of(&config), 0o644);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn direct_connect_repairs_password_config_before_retry() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &["connect", "selected"],
        &bin,
        &root,
        b"y\n",
        b"Permission repair required",
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("current mode: 644"), "output={output}");
    assert!(output.contains("required mode: 600"), "output={output}");
    assert!(output.contains("reason:"), "output={output}");
    assert_eq!(mode_of(&config), 0o600);
    assert!(output.contains("direct-shell"), "output={output}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn direct_connect_declined_permission_repair_keeps_password_config() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &["connect", "selected"],
        &bin,
        &root,
        b"n\n",
        b"Permission repair required",
    );
    assert_eq!(status.code(), Some(2), "status={status:?} output={output}");
    assert!(
        output.contains("PERMISSION_REPAIR_DECLINED"),
        "output={output}"
    );
    assert!(output.contains("chmod 600 '"), "output={output}");
    assert_eq!(mode_of(&config), 0o644);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_connect_no_input_keeps_password_config_and_shows_manual_repair() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let output = run_fake_ssh(&home, &["connect", "selected", "--no-input"], &bin, &root);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 600 '"));
    assert_eq!(mode_of(&config), 0o644);
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]

fn direct_connect_no_tty_keeps_password_config_and_shows_manual_repair() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let output = run_fake_ssh(&home, &["connect", "selected"], &bin, &root);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("chmod 600 '"));
    assert_eq!(mode_of(&config), 0o644);
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn direct_connect_repair_does_not_retry_real_auth_failure() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    write(&root.join("auth-fail"), "");
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &["connect", "selected"],
        &bin,
        &root,
        b"y\nn\n",
        b"Permission repair required",
    );
    assert_eq!(status.code(), Some(2), "status={status:?} output={output}");
    assert!(output.contains("SSH_AUTH_FAILED"), "output={output}");
    assert_eq!(
        output.matches("Permission repair required").count(),
        1,
        "permission repair must prompt once: {output}"
    );
    assert_eq!(mode_of(&config), 0o600);
    fs::remove_dir_all(root).unwrap();
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
fn run_with_pty(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    input: &[u8],
) -> (std::process::ExitStatus, String) {
    run_with_pty_header(home, args, bin, root, input, b"sshx connect host picker")
}

#[cfg(unix)]
fn run_with_pty_header(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    input: &[u8],
    header: &[u8],
) -> (std::process::ExitStatus, String) {
    let mut master = -1;
    let mut slave = -1;
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "openpty should succeed");
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    let stdin = slave.try_clone().expect("pty stdin should clone");
    let stdout = slave.try_clone().expect("pty stdout should clone");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .env("SSHX_STARTED", root.join("master-started"))
        .env("SSHX_CLOSED", root.join("master-closed"))
        .env(
            "SSHX_AUTH_FAIL",
            if root.join("auth-fail").exists() {
                "1"
            } else {
                ""
            },
        )
        .args(args)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(slave))
        .spawn()
        .expect("sshx should start in pty");
    let mut master = unsafe { std::fs::File::from_raw_fd(master) };
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "pty flags should be readable");
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0,
        "pty should become nonblocking"
    );
    let mut output = Vec::new();
    let startup_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut buffer = [0u8; 4096];
        match master.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => panic!("picker PTY read failed before startup: {error}"),
        }
        if output.windows(header.len()).any(|window| window == header) {
            break;
        }
        assert!(
            Instant::now() < startup_deadline,
            "sshx picker should render before input"
        );
        assert!(
            child
                .try_wait()
                .expect("sshx status should be readable")
                .is_none(),
            "sshx picker exited before rendering"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    master.write_all(input).expect("pty input should write");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        let mut buffer = [0u8; 4096];
        match master.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break child.wait().expect("sshx should exit"),
        }
        if let Some(status) = child.try_wait().expect("sshx status should be readable") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("sshx picker should stop after timeout");
            break child.wait().expect("sshx picker should exit after timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    for _ in 0..10 {
        let mut buffer = [0u8; 4096];
        match master.read(&mut buffer) {
            Ok(size) if size > 0 => output.extend_from_slice(&buffer[..size]),
            _ => break,
        }
    }
    (status, String::from_utf8_lossy(&output).into_owned())
}

#[cfg(unix)]
#[test]
fn selectorless_connect_picker_preserves_secondary_alias() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host first second\n  HostName destination.example\nHost other\n  HostName other.example\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        b"second\n",
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("second"), "picker output={output}");
    assert_eq!(
        fs::read_to_string(root.join("runtime-config")).unwrap(),
        "Host second\n  HostName destination.example\nInclude /etc/ssh/ssh_config\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_connect_picker_escape_cancels_without_side_effect() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host first\n  HostName destination.example\n");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        b"\x1b",
    );
    assert_eq!(
        status.code(),
        Some(130),
        "status={status:?} output={output}"
    );
    assert!(output.contains("Cancelled."), "picker output={output}");
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}
