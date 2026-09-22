use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
    write(&ssh.join("config"), "Include z/*.conf a/*.conf\n");
    write(
        &ssh.join("z/10.conf"),
        "Host z-entry\n  HostName z.example\n",
    );
    write(
        &ssh.join("a/10.conf"),
        "Host a-entry\n  HostName a.example\n",
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
