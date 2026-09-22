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
