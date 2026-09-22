use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        .args(args)
        .output()
        .expect("sshx binary should run")
}

#[test]
fn direct_connect_compiles_exact_block_and_uses_owned_master() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "##SSHX ID=direct-id\nHost direct\n  HostName direct.example # remove this\n  User alice\n  IdentityFile ~/.ssh/id_ed25519\n  ##PASSWORD never-copy-this\n",
    );
    let bin = fake_ssh(&root);
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
    for _ in 0..100 {
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
