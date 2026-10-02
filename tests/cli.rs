use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);
#[test]
fn help_root_forms_are_complete_and_deterministic() {
    let first = Command::new(env!("CARGO_BIN_EXE_sshx")).output().unwrap();
    let second = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["help"])
        .output()
        .unwrap();
    assert!(first.status.success(), "{first:?}");
    assert!(second.status.success(), "{second:?}");
    assert_eq!(first.stdout, second.stdout);
    let help = String::from_utf8_lossy(&first.stdout);
    assert!(help.contains("Name: sshx"), "{help}");
    assert!(help.contains("Usage forms:"), "{help}");
    assert!(help.contains("Subcommands:"), "{help}");
    assert!(help.contains("setup"), "{help}");
    assert!(help.contains("doctor"), "{help}");
    assert!(help.contains("connect"), "{help}");
    assert!(!help.contains("\x1b["), "{help}");
    assert!(first.stderr.is_empty());
}

#[test]
fn help_flags_route_by_position_and_ignore_format() {
    let cases: &[(&[&str], &str)] = &[
        (&["-h"], "Name: sshx"),
        (&["--help"], "Name: sshx"),
        (&["setup", "-h"], "Name: setup"),
        (&["doctor", "--help"], "Name: doctor"),
        (&["connect", "-h"], "Name: connect"),
        (&["connect", "prod", "--help"], "Name: connect"),
        (&["help", "setup"], "Name: setup"),
        (&["help", "doctor"], "Name: doctor"),
        (&["help", "connect"], "Name: connect"),
    ];
    for (args, marker) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains(marker), "{args:?}: {help}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    }

    let root_before_path = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["--help", "connect"])
        .output()
        .unwrap();
    assert!(root_before_path.status.success());
    assert!(String::from_utf8_lossy(&root_before_path.stdout).contains("Name: sshx"));

    let connect_before_selector = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["connect", "--help", "prod"])
        .output()
        .unwrap();
    assert!(connect_before_selector.status.success());
    assert!(String::from_utf8_lossy(&connect_before_selector.stdout).contains("Name: connect"));

    let plain = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["connect", "--help"])
        .output()
        .unwrap();
    let with_format = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["connect", "--format", "json", "--help"])
        .output()
        .unwrap();
    assert_eq!(plain.stdout, with_format.stdout);
    assert!(with_format.stderr.is_empty());
}

#[test]
fn every_documented_help_path_has_ordered_sections() {
    let paths = ["setup", "doctor", "connect"];
    for path in paths {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(["help", path])
            .output()
            .unwrap();
        assert!(output.status.success(), "{path}: {output:?}");
        let help = String::from_utf8_lossy(&output.stdout);
        for section in [
            "Usage forms:",
            "Positional arguments:",
            "Options:",
            "Exit behavior:",
        ] {
            assert!(help.contains(section), "{path}: missing {section}\n{help}");
        }
        assert!(!help.contains("\x1b["), "{path}: {help}");
    }
    for path in paths {
        for flag in ["-h", "--help"] {
            let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
                .args([path, flag])
                .output()
                .unwrap();
            assert!(output.status.success(), "{path} {flag}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("Name: "),
                "{path} {flag}: {:?}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(output.stderr.is_empty(), "{path} {flag}: {output:?}");
        }
    }
}
#[test]
fn host_and_pair_help_cover_every_path_and_form() {
    let paths = [
        ("host", "Name: host", true),
        ("host list", "Name: host list", false),
        ("host show", "Name: host show", false),
        ("host create", "Name: host create", false),
        ("host update", "Name: host update", false),
        ("host rename", "Name: host rename", false),
        ("host delete", "Name: host delete", false),
        ("pair", "Name: pair", true),
        ("pair setup", "Name: pair setup (alias: pair create)", false),
        (
            "pair create",
            "Name: pair setup (alias: pair create)",
            false,
        ),
        ("pair list", "Name: pair list", false),
        ("pair validate", "Name: pair validate", false),
    ];

    for (path, marker, group) in paths {
        let path_args = path.split_whitespace().collect::<Vec<_>>();
        let mut forms = Vec::new();
        let mut short = path_args.clone();
        short.push("-h");
        forms.push(short);
        let mut long = path_args.clone();
        long.push("--help");
        forms.push(long);
        let mut help = vec!["help"];
        help.extend(path_args.iter().copied());
        forms.push(help);

        for args in forms {
            let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
                .args(&args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{path} {args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{path} {args:?}: {output:?}");
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(text.contains(marker), "{path} {args:?}: {text}");
            for section in [
                "Purpose:",
                "Usage forms:",
                "Positional arguments:",
                "Options:",
                "Examples:",
                "Exit behavior:",
                "Related commands:",
            ] {
                assert!(text.contains(section), "{path}: missing {section}\n{text}");
            }
            if group {
                assert!(text.contains("Subcommands:"), "{path}: {text}");
            }
            assert!(!text.contains("\x1b["), "{path}: {text}");
        }
    }
}

#[test]
fn host_and_pair_help_preserves_position_and_alias_behavior() {
    let cases: &[(&[&str], &str)] = &[
        (&["--help", "host", "list"], "Name: sshx"),
        (&["host", "--help", "list"], "Name: host"),
        (&["host", "list", "--help"], "Name: host list"),
        (&["help", "host", "list"], "Name: host list"),
        (&["host", "show", "prod", "--help"], "Name: host show"),
        (&["--help", "pair", "setup"], "Name: sshx"),
        (&["pair", "--help", "setup"], "Name: pair"),
        (&["pair", "setup", "--help"], "Name: pair setup"),
        (&["help", "pair", "create"], "Name: pair setup"),
        (
            &["pair", "create", "gateway", "vm", "--help"],
            "Name: pair setup",
        ),
    ];
    for (args, marker) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(marker),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    let setup = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["pair", "setup", "--help"])
        .output()
        .unwrap();
    let create = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["pair", "create", "--help"])
        .output()
        .unwrap();
    assert_eq!(setup.stdout, create.stdout);

    let plain = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["host", "update", "--help"])
        .output()
        .unwrap();
    let formatted = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["host", "update", "--format", "json", "--help"])
        .output()
        .unwrap();
    assert_eq!(plain.stdout, formatted.stdout);
}

#[test]
fn host_and_pair_help_documents_selection_prompts_conflicts_and_outcomes() {
    let pages: &[(&[&str], &[&str])] = &[
        (
            &["help", "host", "list"],
            &[
                "HostEntry",
                "--config PATH",
                "--scope SCOPE",
                "--project NAME",
                "--source PATH",
                "--line NUMBER",
                "--id ID",
                "--format human|json|yaml",
                "No prompt supplies missing values",
                "Discovery diagnostics",
            ],
        ),
        (
            &["help", "host", "show"],
            &[
                "Exact alias or stable HostEntry ID",
                "--source PATH --line NUMBER",
                "--id ID",
                "No fuzzy fallback",
                "sshx host show prod",
                "Missing or ambiguous selectors",
            ],
        ),
        (
            &["help", "host", "create"],
            &[
                "--config PATH",
                "--scope SCOPE",
                "--project NAME",
                "--folder PATH",
                "--file PATH",
                "--target-file PATH",
                "--alias ALIAS",
                "--hostname HOSTNAME",
                "--password-stdin",
                "--preview",
                "--yes",
                "--no-input",
                "prompts only with a usable TTY",
                "Conflicts",
            ],
        ),
        (
            &["help", "host", "update"],
            &[
                "Exact alias or stable HostEntry ID",
                "--source PATH --line NUMBER",
                "--id ID",
                "live fuzzy picker",
                "--clear-password",
                "--password-stdin",
                "--preview",
                "--yes",
                "--no-input",
                "Escape or Ctrl-C",
                "Selector errors",
            ],
        ),
        (
            &["help", "host", "rename"],
            &[
                "Exact alias or stable HostEntry ID",
                "--source PATH --line NUMBER",
                "--id ID",
                "live fuzzy picker",
                "--alias ALIAS",
                "--preview",
                "--yes",
                "--no-input",
                "Missing values",
                "rename accepts only --alias",
            ],
        ),
        (
            &["help", "host", "delete"],
            &[
                "Exact alias or stable HostEntry ID",
                "--source PATH --line NUMBER",
                "--id ID",
                "live fuzzy picker",
                "--preview",
                "--yes",
                "--no-input",
                "declined confirmation",
                "Delete does not accept host fields",
            ],
        ),
        (
            &["help", "pair"],
            &["Pair routes", "ProxyCommand", "native Copy SSH"],
        ),
        (
            &["help", "pair", "setup"],
            &[
                "Exact alias or stable HostEntry ID",
                "gateway",
                "VM",
                "interactive picker",
                "--gateway SELECTOR",
                "--gateway-id ID",
                "--vm SELECTOR",
                "--vm-id ID",
                "--gateway-source PATH",
                "--gateway-line NUMBER",
                "--vm-source PATH",
                "--vm-line NUMBER",
                "--source PATH --line NUMBER",
                "--id ID",
                "--transit-host HOST",
                "--transit-port PORT",
                "--no-input",
                "--preview",
                "--yes",
                "Conflicting positional and option selectors",
                "source and line must be paired",
            ],
        ),
        (
            &["help", "pair", "list"],
            &[
                "Pair routes",
                "--config PATH",
                "--format human|json|yaml",
                "Invalid Pair records",
            ],
        ),
        (
            &["help", "pair", "validate"],
            &[
                "report-only",
                "--config PATH",
                "--format human|json|yaml",
                "never repairs permissions or route data",
                "Invalid routes",
            ],
        ),
    ];
    for (args, fragments) in pages {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        for fragment in *fragments {
            assert!(
                text.contains(fragment),
                "{args:?}: missing {fragment}\n{text}"
            );
        }
    }
    let pair = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["help", "pair"])
        .output()
        .unwrap();
    assert!(pair.status.success());
    let pair_text = String::from_utf8_lossy(&pair.stdout);
    assert!(!pair_text.contains("--gateway-password-fd"));
    assert!(!pair_text.contains("--vm-password-fd"));

    let connect = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["help", "connect"])
        .output()
        .unwrap();
    assert!(connect.status.success());
    let connect_text = String::from_utf8_lossy(&connect.stdout);
    for fragment in [
        "--password-fd FD",
        "--gateway-password-fd FD",
        "--vm-password-fd FD",
    ] {
        assert!(
            connect_text.contains(fragment),
            "missing {fragment}\n{connect_text}"
        );
    }

    for (args, usage) in [
        (&["host", "unknown"][..], "Usage: sshx [--config PATH] host"),
        (&["pair", "unknown"][..], "Usage: sshx pair"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(usage),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn tunnel_help_covers_every_path_and_form() {
    let paths = [
        ("tunnel", "Name: tunnel", true),
        ("tunnel direct", "Name: tunnel direct", true),
        ("tunnel paired", "Name: tunnel paired", true),
        ("tunnel direct start", "Name: tunnel direct start", false),
        ("tunnel paired start", "Name: tunnel paired start", false),
        ("tunnel list", "Name: tunnel list", false),
        ("tunnel status", "Name: tunnel status", false),
        ("tunnel stop", "Name: tunnel stop", false),
        ("tunnel restart", "Name: tunnel restart", false),
        ("tunnel direct list", "Name: tunnel direct list", false),
        ("tunnel direct status", "Name: tunnel direct status", false),
        ("tunnel direct stop", "Name: tunnel direct stop", false),
        ("tunnel direct restart", "Name: tunnel direct restart", false),
        ("tunnel paired list", "Name: tunnel paired list", false),
        ("tunnel paired status", "Name: tunnel paired status", false),
        ("tunnel paired stop", "Name: tunnel paired stop", false),
        ("tunnel paired restart", "Name: tunnel paired restart", false),
    ];

    for (path, marker, group) in paths {
        let path_args = path.split_whitespace().collect::<Vec<_>>();
        let mut forms = Vec::new();
        let mut short = path_args.clone();
        short.push("-h");
        forms.push(short);
        let mut long = path_args.clone();
        long.push("--help");
        forms.push(long);
        let mut help = vec!["help"];
        help.extend(path_args.iter().copied());
        forms.push(help);

        for args in forms {
            let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
                .args(&args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{path} {args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{path} {args:?}: {output:?}");
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(text.contains(marker), "{path} {args:?}: {text}");
            for section in [
                "Purpose:",
                "Usage forms:",
                "Positional arguments:",
                "Options:",
                "Examples:",
                "Exit behavior:",
                "Related commands:",
            ] {
                assert!(text.contains(section), "{path}: missing {section}\n{text}");
            }
            if group {
                assert!(text.contains("Subcommands:"), "{path}: {text}");
            }
            assert!(!text.contains("\x1b["), "{path}: {text}");
        }
    }
}
#[test]
fn tunnel_help_preserves_position_and_rejects_unsupported_paths() {
    let cases: &[(&[&str], &str)] = &[
        (&["--help", "tunnel", "list"], "Name: sshx"),
        (&["tunnel", "--help", "list"], "Name: tunnel"),
        (&["tunnel", "list", "--help"], "Name: tunnel list"),
        (&["help", "tunnel", "list"], "Name: tunnel list"),
        (
            &["tunnel", "status", "dt-1", "--help"],
            "Name: tunnel status",
        ),
        (
            &["tunnel", "restart", "pt-1", "--help"],
            "Name: tunnel restart",
        ),
        (&["tunnel", "direct", "--help"], "Name: tunnel direct"),
        (
            &["tunnel", "direct", "start", "--help"],
            "Name: tunnel direct start",
        ),
        (
            &["tunnel", "paired", "status", "pt-1", "--help"],
            "Name: tunnel paired status",
        ),
    ];
    for (args, marker) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(marker),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    let unsupported_paths: &[&[&str]] = &[
        &["help", "tunnel", "direct", "unsupported"],
        &["help", "tunnel", "paired", "unsupported"],
    ];
    for args in unsupported_paths {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown help path"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Name:"));
    }
}

#[test]
fn invalid_tunnel_paths_return_route_specific_usage() {
    let cases: &[(&[&str], &str)] = &[
        (&["tunnel", "direct", "unsupported"], "Usage: sshx tunnel direct"),
        (&["tunnel", "paired", "unsupported"], "Usage: sshx tunnel paired"),
    ];
    for (args, usage) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(usage));
        assert!(output.stdout.is_empty());
    }
}


#[test]
fn help_parse_errors_use_nearest_usage_on_stderr() {
    let connect = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["connect", "--unknown"])
        .output()
        .unwrap();
    assert_eq!(connect.status.code(), Some(2));
    assert!(connect.stdout.is_empty());
    let connect_error = String::from_utf8_lossy(&connect.stderr);
    assert!(
        connect_error.contains("Usage: sshx connect"),
        "{connect_error}"
    );

    let doctor = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .args(["doctor", "unknown"])
        .output()
        .unwrap();
    assert_eq!(doctor.status.code(), Some(2));
    let doctor_error = String::from_utf8_lossy(&doctor.stderr);
    assert!(
        doctor_error.contains("Usage: sshx doctor"),
        "{doctor_error}"
    );
}

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
fn no_arguments_print_complete_root_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .output()
        .expect("sshx binary should run");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Name: sshx"));
    assert!(output.stderr.is_empty());
}

#[test]
fn unknown_argument_reports_root_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .arg("unknown")
        .output()
        .expect("sshx binary should run");

    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unexpected argument `unknown`"));
    assert!(error.contains("Usage: sshx [GLOBAL OPTIONS] [COMMAND]"));
    assert!(output.stdout.is_empty());
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
    if [ -f "$SSHX_FAIL_ONCE" ]; then
      rm -f "$SSHX_FAIL_ONCE"
      echo "Intentional one-time service forward failure" >&2
      exit 255
    fi
    if [ "$SSHX_HOST_KEY_CHANGED" = "1" ]; then
      echo "REMOTE HOST IDENTIFICATION HAS CHANGED!" >&2
      exit 255
    fi
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
fn pair_list_does_not_render_relationship_with_duplicate_referenced_id() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=33333333-3333-4333-8333-333333333333\n",
            "Host gateway\n",
            "  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "Host duplicate\n",
            "##SSHX ID=33333333-3333-4333-8333-333333333333\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  Port 22\n",
        ),
    );
    let output = run(
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
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(document["pairs"].as_array().unwrap().is_empty());
    let duplicate = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "duplicate_id")
        .unwrap();
    let message = duplicate["message"].as_str().unwrap();
    assert!(message.contains(&format!("{}:3", config.display())), "{message}");
    assert!(message.contains(&format!("{}:6", config.display())), "{message}");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn pair_list_and_validate_keep_stable_read_only_json_output() {
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
            "  Port 22\n",
        ),
    );
    let original = fs::read_to_string(&config).unwrap();

    for command in ["list", "validate"] {
        let args = [
            "--config",
            config.to_str().unwrap(),
            "pair",
            command,
            "--format",
            "json",
        ];
        let first = run(&home, &args);
        let second = run(&home, &args);
        assert!(first.status.success(), "{first:?}");
        assert!(second.status.success(), "{second:?}");
        assert_eq!(first.stdout, second.stdout);
        let document: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
        assert!(document["diagnostics"].as_array().unwrap().is_empty());
        let pairs = document["pairs"].as_array().unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0]["gateway_alias"], "gateway");
        assert_eq!(pairs[0]["vm_alias"], "vm");
        assert_eq!(pairs[0]["transit_host"], "vm.internal");
        assert_eq!(pairs[0]["transit_port"], 22);
    }
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
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
    let original = fs::read(&config).unwrap();
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
    assert_eq!(fs::read(&config).unwrap(), original);

    let unmatched = run(
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
    assert_eq!(unmatched.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unmatched.stderr).contains("TRANSIT_MISMATCH"));
    assert_eq!(fs::read(&config).unwrap(), original);
    let wrong_vm_port = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--transit-host",
            "first.internal",
            "--transit-port",
            "23",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(wrong_vm_port.status.code(), Some(2));
    assert_eq!(fs::read(&config).unwrap(), original);
    write(
        &config,
        concat!(
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 first.internal:22\n",
            "  LocalForward 2201 second.internal:22\n",
            "  LocalForward 2202 first.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    let ambiguous_source = fs::read(&config).unwrap();
    let ambiguous = run(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "pair",
            "setup",
            "gateway",
            "vm",
            "--transit-host",
            "first.internal",
            "--transit-port",
            "22",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(ambiguous.status.code(), Some(2));
    assert_eq!(fs::read(&config).unwrap(), ambiguous_source);
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
            "first.internal",
            "--transit-port",
            "22",
            "--yes",
            "--no-input",
        ],
    );
    assert!(explicit.status.success(), "{explicit:?}");
    assert!(fs::read_to_string(&config)
        .unwrap()
        .contains("##SSHX TRANSIT=first.internal:22"));
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
fn doctor_fix_permissions_repairs_mode_zero_password_config_once() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o000)).unwrap();
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
    assert_eq!(
        output.matches("Apply permission repairs?").count(),
        1,
        "output={output}"
    );
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
fn doctor_workspace_shows_repair_plan_before_confirmation() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &["--config", config.to_str().unwrap(), "tui"],
        &bin,
        &root,
        &[
            (b"Search:", b"\x04"),
            (b"report only", b"yr"),
            (b"0644 -> 0600", b"\rn\x03"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((100, 36)),
        |_| assert_eq!(mode_of(&config), 0o644),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(mode_of(&config), 0o644);
    assert!(!output.contains("stored-secret"), "output={output}");

    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &["--config", config.to_str().unwrap(), "tui", "doctor"],
        &bin,
        &root,
        &[
            (b"report only", b"r"),
            (b"0644 -> 0600", b"y"),
            (b"fixed:", b"\x1b"),
        ],
        None,
        Some((100, 36)),
        |stage| assert_eq!(mode_of(&config), if stage < 2 { 0o644 } else { 0o600 }),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(mode_of(&config), 0o600);
    assert!(!output.contains("stored-secret"), "output={output}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn doctor_workspace_refuses_confirmation_when_plan_cannot_be_seen() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host password\n  HostName password.example\n  ##PASSWORD stored-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &["--config", config.to_str().unwrap(), "tui", "doctor"],
        &bin,
        &root,
        &[
            (b"report only", b"r"),
            (b"Resize", b"yn\x03"),
        ],
        None,
        Some((40, 6)),
        |_| assert_eq!(mode_of(&config), 0o644),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(mode_of(&config), 0o644);
    assert!(!output.contains("stored-secret"), "output={output}");
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
fn direct_connect_repairs_mode_zero_password_config_before_discovery_retry() {
    let (root, home) = canonical_fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host selected\n  HostName selected.example\n  ##PASSWORD configured-secret\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o000)).unwrap();
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
    assert_eq!(
        output.matches("Permission repair required").count(),
        1,
        "output={output}"
    );
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

#[cfg(unix)]
#[test]
fn host_update_short_terminal_keeps_password_and_controls_visible() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host target\n  HostName target.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "host", "update", "target"],
        &bin,
        &root,
        &[(b"Edit HostEntry", b"\t\t\t\t"), (b"> Password [keep]", b"\x1b")],
        None,
        Some((48, 8)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert_eq!(fs::read_to_string(&config).unwrap(), "Host target\n  HostName target.example\n");
    assert!(contains_tui_text(output.as_bytes(), b"Ctrl-S review"), "{output}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_update_workspace_accepts_mask_placeholder_as_password() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=33333333-3333-4333-8333-333333333333\n",
            "Host target\n",
            "  HostName target.example\n",
        ),
    );
    let mut permissions = fs::metadata(&config).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&config, permissions).unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "host", "update", "target"],
        &bin,
        &root,
        &[
            (b"Edit HostEntry", b"\t\t\t\t********\x13"),
            (b"Review HostEntry changes", b"\n"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!output.contains("********"), "password leaked: {output}");
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("##PASSWORD ********"),
        "literal mask password was not applied"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_rename_picker_preserves_selected_alias_and_identity() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
            "Host first common\n",
            "  HostName first.example\n",
            "##SSHX ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
            "Host second common\n",
            "  HostName second.example\n",
        ),
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "rename",
            "--alias",
            "renamed",
        ],
        &bin,
        &root,
        &[
            (b"host rename", b"common\x1b[B\n"),
            (b"Rename HostEntry", b"\x13"),
            (b"Review HostEntry changes", b"\n"),
        ],
        None,
        Some((120, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(contains_tui_text(output.as_bytes(), b"Alias: common"), "{output}");
    assert!(contains_tui_text(output.as_bytes(), b"ID: bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"), "{output}");
    let source = format!("Source: {}:5", fs::canonicalize(&config).unwrap().display());
    assert!(contains_tui_text(output.as_bytes(), source.as_bytes()), "{output}");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        concat!(
            "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
            "Host first common\n",
            "  HostName first.example\n",
            "##SSHX ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
            "Host second renamed\n",
            "  HostName second.example\n",
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_keeps_mutations_available_for_pair_gateway() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = concat!(
        "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
        "##SSHX VM=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
        "Host gateway\n",
        "  HostName gateway.example\n",
        "  LocalForward 2200 vm.internal:22\n",
        "##SSHX ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
        "##SSHX GATEWAY=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
        "##SSHX TRANSIT=vm.internal:22\n",
        "Host vm\n",
        "  HostName vm.internal\n",
        "  Port 22\n",
    );
    write(&config, original);
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "tui"],
        &bin,
        &root,
        &[
            (b"Search:", b"gateway\x15"),
            (b"Alias [keep]: gateway", b"\x1b"),
            (b"HostEntry edit cancelled.", b"\x1b"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}




#[test]
fn copy_ssh_without_clipboard_prints_quoted_command() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    write(
        &config,
        "Host weird-alias\n  HostName selected.example\n  ##PASSWORD never-copy-this\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "weird-alias",
            "--action",
            "copy-ssh",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = format!("ssh -F '{}' 'weird-alias'\n", config.display());
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(String::from_utf8_lossy(&output.stderr).contains("CLIPBOARD_UNAVAILABLE"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("never-copy-this"));
    fs::remove_dir_all(root).unwrap();
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn copy_ssh_sends_command_to_native_clipboard_stdin() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let bin = root.join("clipboard-bin");
    fs::create_dir(&bin).unwrap();
    write(
        &config,
        "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\nHost direct\n  HostName direct.example\n",
    );
    #[cfg(target_os = "macos")]
    let backend = "pbcopy";
    #[cfg(target_os = "linux")]
    let backend = "wl-copy";
    let script = bin.join(backend);
    write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$*\" > \"$SSHX_CLIPBOARD_ARGS\"\ncat > \"$SSHX_CLIPBOARD_CAPTURE\"\n",
    );
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", path)
        .env("SSHX_CLIPBOARD_ARGS", root.join("clipboard-args"))
        .env("SSHX_CLIPBOARD_CAPTURE", root.join("clipboard-content"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "--action",
            "copy-ssh",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Copied command"));
    assert_eq!(
        fs::read_to_string(root.join("clipboard-content")).unwrap(),
        format!("ssh -F '{}' 'direct'", config.display())
    );
    assert!(
        fs::read_to_string(root.join("clipboard-args"))
            .unwrap()
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn action_conflicts_with_machine_output_before_action() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "--action",
            "copy-ssh",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("ACTION_FORMAT_CONFLICT"));
    assert!(output.stdout.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_sshx_preserves_selected_secondary_alias() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    write(
        &config,
        "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\nHost primary secondary\n  HostName direct.example\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "secondary",
            "--action",
            "copy-sshx",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let expected = format!(
        "sshx connect 'secondary' --id 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' --config '{}'\n",
        config.display()
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_sshx_multiple_roots_uses_source_without_prompt() {
    let (root, home) = fixture_root();
    let shared = home.join(".ssh/shared.conf");
    let root_a = home.join(".ssh/root-a.conf");
    let root_b = home.join(".ssh/root-b.conf");
    write(
        &shared,
        "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\nHost shared\n  HostName shared.example\n",
    );
    write(&root_a, &format!("Include {}\n", shared.display()));
    write(&root_b, &format!("Include {}\n", shared.display()));
    write(
        &home.join(".config/sshx/config.json"),
        &format!(
            "{{\"version\":1,\"roots\":[{{\"scope\":\"personal\",\"path\":\"{}\",\"project\":null}},{{\"scope\":\"work\",\"path\":\"{}\",\"project\":null}}]}}\n",
            root_a.display(),
            root_b.display()
        ),
    );
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args(["connect", "shared", "--action", "copy-sshx", "--no-input"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let command = String::from_utf8_lossy(&output.stdout);
    assert!(command.contains("'shared' --id 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'"));
    let source = fs::canonicalize(&shared).unwrap();
    assert!(command.contains(&format!("--source '{}' --line 2", source.display())));
    assert!(!command.contains("--config"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_sshx_disambiguates_duplicate_identity_by_source() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    write(
        &config,
        concat!(
            "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
            "Host same\n",
            "  HostName first.example\n",
            "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
            "Host same\n",
            "  HostName second.example\n",
        ),
    );
    let source = fs::canonicalize(&config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "same",
            "--id",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "--source",
            source.to_str().unwrap(),
            "--line",
            "5",
            "--action",
            "copy-sshx",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let command = String::from_utf8_lossy(&output.stdout);
    assert!(command.contains("'same' --id 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'"));
    assert!(command.contains(&format!("--source '{}' --line 5", source.display())));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_sshx_cross_root_pair_uses_source_disambiguation() {
    let (root, home) = fixture_root();
    let gateway = home.join(".ssh/gateway.conf");
    let vm = home.join(".ssh/vm.conf");
    write(
        &gateway,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
        ),
    );
    write(
        &vm,
        concat!(
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm secondary\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
        ),
    );
    write(
        &home.join(".config/sshx/config.json"),
        &format!(
            "{{\"version\":1,\"roots\":[{{\"scope\":\"personal\",\"path\":\"{}\",\"project\":null}},{{\"scope\":\"work\",\"path\":\"{}\",\"project\":null}}]}}\n",
            gateway.display(),
            vm.display()
        ),
    );
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    let unavailable = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args(["connect", "secondary", "--action", "copy-ssh"])
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(2), "{unavailable:?}");
    assert!(String::from_utf8_lossy(&unavailable.stderr).contains("ACTION_UNAVAILABLE"));
    assert!(unavailable.stdout.is_empty());

    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args(["connect", "secondary", "--action", "copy-sshx"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let command = String::from_utf8_lossy(&output.stdout);
    assert!(command.contains("'secondary' --id '22222222-2222-4222-8222-222222222222'"));
    let source = fs::canonicalize(&vm).unwrap();
    assert!(command.contains(&format!("--source '{}' --line 4", source.display())));
    assert!(!command.contains("--config"));
    fs::remove_dir_all(root).unwrap();
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn fake_clipboard(bin: &Path, root: &Path) -> &'static str {
    #[cfg(target_os = "macos")]
    let backend = "pbcopy";
    #[cfg(target_os = "linux")]
    let backend = "wl-copy";
    let script = bin.join(backend);
    write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$*\" > \"$SSHX_CLIPBOARD_ARGS\"\ncat > \"$SSHX_CLIPBOARD_CAPTURE\"\nenv > \"$SSHX_CLIPBOARD_ENV\"\n",
    );
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    let _ = root;
    backend
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
    run_with_pty_header_path(home, args, bin, root, input, header, None)
}

#[cfg(unix)]
fn run_with_pty_header_path(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    input: &[u8],
    header: &[u8],
    inherited_path: Option<&str>,
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
    let inherited_path = inherited_path
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    let path = if inherited_path.is_empty() {
        bin.display().to_string()
    } else {
        format!("{}:{inherited_path}", bin.display())
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .env("SSHX_STARTED", root.join("master-started"))
        .env("SSHX_CLIPBOARD_CAPTURE", root.join("clipboard-content"))
        .env("SSHX_CLIPBOARD_ARGS", root.join("clipboard-args"))
        .env("SSHX_CLIPBOARD_ENV", root.join("clipboard-env"))
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
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        &[
            (b"Search:", b"second\n"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.lines().any(|line| line == "Host second"), "{runtime}");
    assert!(runtime.lines().any(|line| line.trim() == "HostName destination.example"), "{runtime}");
    assert!(!runtime.lines().any(|line| matches!(line, "Host first" | "Host first second" | "Host other")), "{runtime}");
    assert!(!runtime.lines().any(|line| line.trim() == "HostName other.example"), "{runtime}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_connect_picker_arrow_moves_selection() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host first\n",
            "  HostName first.example\n",
            "Host second\n",
            "  HostName second.example\n",
        ),
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        &[
            (b"Search:", b"\x1b[B\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.lines().any(|line| line == "Host second"), "{runtime}");
    assert!(runtime.lines().any(|line| line.trim() == "HostName second.example"), "{runtime}");
    assert!(!runtime.lines().any(|line| line == "Host first"), "{runtime}");
    assert!(!runtime.lines().any(|line| line.trim() == "HostName first.example"), "{runtime}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_connect_picker_escape_cancels_without_side_effect() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host first\n  HostName destination.example\n");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_header(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        b"\x1b",
        b"Search:",
    );
    assert_eq!(
        status.code(),
        Some(130),
        "status={status:?} output={output}"
    );
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn copy_password_without_clipboard_never_falls_back_to_stdout() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PASSWORD secret-value\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "--action",
            "copy-password",
        ],
        &empty_path,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Copy stored password now?", b"y\n"),
            (b"CLIPBOARD_UNAVAILABLE", b"\x03"),
        ],
        Some(""),
        Some((80, 24)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("CLIPBOARD_UNAVAILABLE"), "output={output}");
    assert!(!output.contains("secret-value"), "output={output}");
    assert!(!root.join("clipboard-content").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_password_without_tty_rejects_without_secret_output() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PASSWORD secret-value\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .args([
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "--action",
            "copy-password",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("PASSWORD_COPY_TTY_REQUIRED"),
        "{combined}"
    );
    assert!(!combined.contains("secret-value"), "{combined}");
    assert!(output.stdout.is_empty());
    fs::remove_dir_all(root).unwrap();
}



fn run_with_pty_interactions(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    interactions: &[(&[u8], &[u8])],
    inherited_path: Option<&str>,
    size: Option<(u16, u16)>,
) -> (std::process::ExitStatus, String) {
    run_with_pty_interactions_with_hook(
        home,
        args,
        bin,
        root,
        interactions,
        inherited_path,
        size,
        |_| {},
    )
}


fn contains_tui_text(output: &[u8], expected: &[u8]) -> bool {
    let output = strip_ansi(output)
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let expected = expected
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    !expected.is_empty()
        && output
            .windows(expected.len())
            .any(|window| window == expected)
}


fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == 0x1b {
            index += 1;
            match input.get(index) {
                Some(b'[') => {
                    index += 1;
                    while index < input.len() {
                        let byte = input[index];
                        index += 1;
                        if (0x40..=0x7e).contains(&byte) {
                            break;
                        }
                    }
                }
                Some(b']' | b'P' | b'_' | b'^') => {
                    index += 1;
                    while index < input.len() {
                        if input[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                }
                Some(_) => index += 1,
                None => {}
            }
        } else {
            if input[index] != b'\r' {
                output.push(input[index]);
            }
            index += 1;
        }
    }
    output
}


#[cfg(unix)]
#[test]
fn connect_picker_scrolls_full_source_identity_on_compact_terminal() {
    let (root, home) = fixture_root();
    let common = format!("{}/{}", "common".repeat(8), "prefix".repeat(8));
    let first = format!("{common}/alpha/hosts.conf");
    let second = format!("{common}/beta/hosts.conf");
    write(
        &home.join(".ssh/config"),
        &format!("Include {first}\nInclude {second}\n"),
    );
    write(
        &home.join(".ssh").join(&first),
        "Host duplicate\n  HostName same.example\n",
    );
    write(
        &home.join(".ssh").join(&second),
        "Host duplicate\n  HostName same.example\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_terminal_hook(
        &home,
        &["connect"],
        &bin,
        &root,
        &[
            (b"Search:", b"\x1b[B\t"),
            (b"ID:", b"\t"),
            (b"PgUp/Dn", b"\x1b[6~"),
            (b"PgUp/Dn", b"\x1b[6~"),
            (b"PgUp/Dn", b"\x1b[6~"),
            (b"PgUp/Dn", b"\x1b[6~"),
            (b"PgUp/Dn", b"\x1b"),
        ],
        None,
        Some((18, 12)),
        |index, terminal| {
            if index > 0 {
                // Resize forces a complete frame instead of diff-only PTY text.
                let dimensions = libc::winsize {
                    ws_col: if index % 2 == 0 { 18 } else { 19 },
                    ws_row: 12,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                assert_eq!(unsafe { libc::ioctl(terminal, libc::TIOCSWINSZ, &dimensions) }, 0);
            }
        },
    );
    assert_eq!(status.code(), Some(130), "output={output}");
    assert!(output.contains("beta"), "output={output}");
    assert!(
        contains_tui_text(output.as_bytes(), b"beta/hosts.conf"),
        "output={output}"
    );
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn missing_tunnel_status_id_continues_in_tunnels_workspace() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let local_port = listener.local_addr().unwrap().port();
    drop(listener);
    let bin = fake_tunnel_ssh(&root);
    let local_forward = format!("127.0.0.1:{local_port}:127.0.0.1:22");
    let started = run_fake_ssh(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
            "start",
            "direct",
            "-L",
            local_forward.as_str(),
            "--no-input",
            "--format",
            "json",
        ],
        &bin,
        &root,
    );
    assert!(started.status.success(), "{started:?}");
    let document: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
    let id = document["tunnels"][0]["id"].as_str().unwrap().to_string();

    let (status, output) = run_with_pty_interactions(
        &home,
        &["tunnel", "status"],
        &bin,
        &root,
        &[(b"ID:", b"\r"), (b"ID:", b"\x1b")],
        None,
        Some((100, 30)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(contains_tui_text(output.as_bytes(), id.as_bytes()), "{output}");
    let inspected = run_fake_ssh(
        &home,
        &["tunnel", "status", &id, "--no-input", "--format", "json"],
        &bin,
        &root,
    );
    assert!(inspected.status.success(), "{inspected:?}");
    let inspection: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(inspection["tunnels"][0]["id"], id);
    assert_eq!(inspection["tunnels"][0]["master_status"], "responsive");

    let stopped = run_fake_ssh(
        &home,
        &["tunnel", "stop", &id, "--no-input"],
        &bin,
        &root,
    );
    assert!(stopped.status.success(), "{stopped:?}");
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn pre_start_interrupt_remains_cancellable_from_tui() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let bin = fake_ssh(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let needle = "  *\" -N \"*)\n";
    let interrupt = concat!(
        "  *\" -N \"*)\n",
        "    kill -INT \"$PPID\"\n",
        "    exit 5\n",
    );
    assert!(script.contains(needle));
    fs::write(&ssh, script.replacen(needle, interrupt, 1)).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "direct",
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"SESSION_START_INTERRUPTED", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("SESSION_START_INTERRUPTED"), "{output}");
    assert!(!root.join("master-started").exists());
    assert_eq!(fs::read_to_string(&config).unwrap(), "Host direct\n  HostName direct.example\n");
    assert!(!home.join(".config/sshx/tunnels.json").exists());
    assert!(!home.join(".config/sshx/config.json").exists());
    assert!(!root.join("clipboard-content").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn run_with_pty_interactions_with_hook(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    interactions: &[(&[u8], &[u8])],
    inherited_path: Option<&str>,
    size: Option<(u16, u16)>,
    mut after_render: impl FnMut(usize),
) -> (std::process::ExitStatus, String) {
    run_with_pty_interactions_with_terminal_hook(
        home, args, bin, root, interactions, inherited_path, size,
        |index, _| after_render(index),
    )
}


#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn run_with_pty_interactions_with_terminal_hook(
    home: &Path,
    args: &[&str],
    bin: &Path,
    root: &Path,
    interactions: &[(&[u8], &[u8])],
    inherited_path: Option<&str>,
    size: Option<(u16, u16)>,
    mut after_render: impl FnMut(usize, libc::c_int),
) -> (std::process::ExitStatus, String) {
    let mut master = -1;
    let mut slave = -1;
    let mut dimensions = unsafe { std::mem::zeroed::<libc::winsize>() };
    if let Some((width, height)) = size {
        dimensions.ws_col = width;
        dimensions.ws_row = height;
    }
    let window: *mut libc::winsize = if size.is_some() {
        &mut dimensions
    } else {
        std::ptr::null_mut()
    };
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            window,
        )
    };
    assert_eq!(result, 0, "openpty should succeed");
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    let stdin = slave.try_clone().expect("pty stdin should clone");
    let stdout = slave.try_clone().expect("pty stdout should clone");
    let inherited_path = inherited_path
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    let path = if inherited_path.is_empty() {
        bin.display().to_string()
    } else {
        format!("{}:{inherited_path}", bin.display())
    };
    let close_stdout = root.join("close-stdout").exists();
    let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
    command
        .env("HOME", home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .env("SSHX_STARTED", root.join("master-started"))
        .env("SSHX_CLIPBOARD_CAPTURE", root.join("clipboard-content"))
        .env("SSHX_CLIPBOARD_ARGS", root.join("clipboard-args"))
        .env("SSHX_CLIPBOARD_ENV", root.join("clipboard-env"))
        .env("SSHX_CLOSED", root.join("master-closed"))
        .env("SSHX_FAIL_ONCE", root.join("fail-once"))
        .env(
            "SSHX_AUTH_FAIL",
            if root.join("auth-fail").exists() {
                "1"
            } else {
                ""
            },
        )
        .env(
            "SSHX_HOST_KEY_CHANGED",
            if root.join("host-key-changed").exists() {
                "1"
            } else {
                ""
            },
        )
        .args(args)
        .stdin(Stdio::from(stdin))
        .stdout(if close_stdout || root.join("redirect-stdout").exists() {
            Stdio::null()
        } else {
            Stdio::from(stdout)
        })
        .stderr(Stdio::from(slave));
    if close_stdout {
        unsafe {
            command.pre_exec(|| {
                if libc::close(libc::STDOUT_FILENO) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().expect("sshx should start in pty");
    let mut master = unsafe { std::fs::File::from_raw_fd(master) };
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "pty flags should be readable");
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0,
        "pty should become nonblocking"
    );
    let mut output = Vec::new();
    for (index, (header, input)) in interactions.iter().enumerate() {
        let stage_start = output.len();
        let startup_deadline = Instant::now() + Duration::from_secs(5);
        let connection_workspace = contains_tui_text(header, b"Connection workspace");
        let action_menu_interaction = !connection_workspace
            && [
            b"Choose host action".as_slice(),
            b"Connect".as_slice(),
            b"Copy SSH".as_slice(),
            b"Copy sshx".as_slice(),
            b"Copy password".as_slice(),
            b"Update".as_slice(),
            b"Rename".as_slice(),
            b"Delete".as_slice(),
        ]
        .iter()
        .any(|action| contains_tui_text(header, action));
        let mut selected_connect = false;
        loop {
            let mut buffer = [0u8; 4096];
            let bytes_before_read = output.len();
            match master.read(&mut buffer) {
                Ok(size) => output.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("picker PTY read failed before startup: {error}"),
            }
            let stage = &output[stage_start..];
            let expected = if connection_workspace {
                b"Mode:".as_slice()
            } else {
                header
            };
            if output.len() > bytes_before_read && contains_tui_text(stage, expected) {
                break;
            }
            if output.len() > bytes_before_read
                && !action_menu_interaction
                && !selected_connect
                && (contains_tui_text(stage, b"Choose host action")
                    || contains_tui_text(stage, b"> Connect"))
            {
                master.write_all(b"\n").expect("Connect selection should write");
                selected_connect = true;
            }
            assert!(
                Instant::now() < startup_deadline,
                "sshx picker did not render interaction {index} {header:?}: {}",
                String::from_utf8_lossy(&strip_ansi(&output))
            );
            assert!(
                child
                    .try_wait()
                    .expect("sshx status should be readable")
                    .is_none(),
                "sshx picker exited before interaction {index} {header:?}: {}",
                String::from_utf8_lossy(&strip_ansi(&output))
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        after_render(index, master.as_raw_fd());
        master.write_all(input).expect("pty input should write");
    }

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
    (status, String::from_utf8_lossy(&strip_ansi(&output)).into_owned())
}


#[cfg(unix)]
#[test]
fn empty_hosts_setup_registers_selected_config() {
    let (root, home) = fixture_root();
    write(&home.join(".ssh/config"), "# no HostEntries\n");
    let config = home.join("configs/ssh_config");
    write(&config, "Host registered\n  HostName registered.example\n");
    let bin = fake_ssh(&root);
    let original = fs::read(&config).unwrap();
    let mut setup_input = b"\x15".to_vec();
    setup_input.extend_from_slice(config.to_string_lossy().as_bytes());
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"No HostEntries", b"\x13"),
            (b"SSH config file path:", &setup_input),
            (b"_config", b"\x13"),
            (b"Config root registered.", b"\x1b"),
        ],
        None,
        Some((100, 36)),
        |stage| {
            if stage == 1 || stage == 2 {
                assert!(!home.join(".config/sshx/config.json").exists());
                assert_eq!(fs::read(&config).unwrap(), original);
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(fs::read(&config).unwrap(), original);
    let hosts = run(&home, &["host", "list", "--format", "json"]);
    assert!(hosts.status.success(), "{hosts:?}");
    let hosts: serde_json::Value = serde_json::from_slice(&hosts.stdout).unwrap();
    assert_eq!(hosts["entries"][0]["aliases"][0], "registered");
    let settings = fs::read_to_string(home.join(".config/sshx/config.json")).unwrap();
    assert!(
        settings.contains(config.to_str().unwrap()),
        "settings={settings}"
    );
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_under_explicit_config_preserves_other_registered_roots() {
    let (root, home) = fixture_root();
    let root_a = home.join("configs/a");
    let root_b = home.join("configs/b");
    let root_c = home.join("configs/c");
    write(&root_a, "# no HostEntries\n");
    write(&root_b, "# registered root B\n");
    write(&root_c, "# saved root C\n");
    fs::create_dir_all(home.join(".config/sshx")).unwrap();
    write(
        &home.join(".config/sshx/config.json"),
        &format!(
            "{{\"version\":1,\"roots\":[{{\"scope\":\"personal\",\"path\":\"{}\",\"project\":null}}]}}",
            root_c.display()
        ),
    );
    let bin = fake_ssh(&root);
    let mut input = b"\x1b[B\x1b[B\x15".to_vec();
    input.extend_from_slice(root_b.to_string_lossy().as_bytes());
    input.extend_from_slice(b"\x13");
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", root_a.to_str().unwrap()],
        &bin,
        &root,
        &[
            (b"No HostEntries", b"\x13"),
            (b"SSH config file path:", &input),
            (b"Config root registered.", b"\x1b"),
        ],
        None,
        Some((100, 36)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    let settings = fs::read_to_string(home.join(".config/sshx/config.json")).unwrap();
    assert!(settings.contains(root_c.to_str().unwrap()), "{settings}");
    assert!(settings.contains(root_b.to_str().unwrap()), "{settings}");
    assert!(!settings.contains(root_a.to_str().unwrap()), "{settings}");
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_workspace_keeps_invalid_path_unregistered_until_explicit_register() {
    let (root, home) = fixture_root();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["setup", "--scope", "work", "--project", "payments"],
        &bin,
        &root,
        &[
            (b"SSH config file path:", b"\x15missing\x13"),
            (b"SETUP_ROOT_NOT_FOUND", b"\x1b"),
        ],
        None,
        Some((100, 36)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("SETUP_ROOT_NOT_FOUND"), "output={output}");
    assert!(!home.join(".config/sshx/config.json").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn partial_setup_prefills_scope_and_project_before_registering() {
    let (root, home) = fixture_root();
    write(&home.join(".ssh/config"), "Host personal\n  HostName personal.example\n");
    let config = home.join("configs/ssh_config");
    write(&config, "Host work-host\n  HostName work.example\n");
    let bin = fake_ssh(&root);
    let mut setup_input = b"\x15".to_vec();
    setup_input.extend_from_slice(config.to_string_lossy().as_bytes());
    setup_input.extend_from_slice(b"\x13");
    let (status, output) = run_with_pty_interactions(
        &home,
        &["setup", "--scope", "work", "--project", "payments"],
        &bin,
        &root,
        &[
            (b"SSH config file path:", &setup_input),
            (b"Config root registered.", b"\x1b"),
        ],
        None,
        Some((100, 36)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    let settings = fs::read_to_string(home.join(".config/sshx/config.json")).unwrap();
    let document: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(document["roots"][0]["scope"], "work");
    assert_eq!(document["roots"][0]["project"], "payments");
    assert_eq!(document["roots"][0]["path"], config.to_str().unwrap());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn explicit_tui_setup_prefills_config_path() {
    let (root, home) = fixture_root();
    let config = home.join("configs/ssh_config");
    write(&config, "Host work-host\n  HostName work.example\n");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "setup", "--config", config.to_str().unwrap(), "--scope", "work", "--project", "demo"],
        &bin,
        &root,
        &[
            (b"SSH config file path:", b"\x13"),
            (b"work-host", b"\x1b"),
        ],
        None,
        Some((100, 36)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    let settings = fs::read_to_string(home.join(".config/sshx/config.json")).unwrap();
    let document: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(document["roots"][0]["scope"], "work");
    assert_eq!(document["roots"][0]["project"], "demo");
    assert_eq!(document["roots"][0]["path"], config.to_str().unwrap());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn empty_hosts_doctor_report_stays_visible_until_acknowledged() {
    let (root, home) = fixture_root();
    write(&root.join("redirect-stdout"), "");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"No HostEntries", b"\x04"),
            (b"Evidence and guidance", b"\x1b"),
            (b"No HostEntries", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!home.join(".config/sshx/config.json").exists());
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_cancel_preserves_config_and_registered_roots() {
    let (root, home) = fixture_root();
    let config = home.join("configs/pending");
    let original = "Host pending\n  HostName pending.example\n";
    write(&config, original);
    let registered = home.join("configs/registered");
    write(&registered, "# registered root\n");
    let saved = format!(
        "{{\"version\":1,\"roots\":[{{\"scope\":\"personal\",\"path\":\"{}\",\"project\":null}}]}}\n",
        registered.display()
    );
    let settings = home.join(".config/sshx/config.json");
    write(&settings, &saved);
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "setup", "--work", config.to_str().unwrap(), "--project", "payments"],
        &bin,
        &root,
        &[(b"SSH config file path:", b"\x1b")],
        None,
        Some((100, 36)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert_eq!(fs::read_to_string(&settings).unwrap(), saved);
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_rejects_missing_config_without_registering_it() {
    let (root, home) = fixture_root();
    let missing = home.join("configs/missing");
    let output = run(
        &home,
        &["setup", "--personal", missing.to_str().unwrap(), "--format", "json"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("SETUP_ROOT_NOT_FOUND"));

    assert!(!home.join(".config/sshx/config.json").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_rejects_symlinked_root_without_registering_it() {
    let (root, home) = fixture_root();
    let target = home.join("configs/ssh_config");
    let link = home.join("configs/ssh_link");
    write(&target, "Host valid\n  HostName valid.example\n");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let output = run(
        &home,
        &["setup", "--personal", link.to_str().unwrap(), "--format", "json"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("CONFIG_ROOT_SYMLINK"));
    assert!(!home.join(".config/sshx/config.json").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn complete_setup_cli_registers_root_without_interactive_continuation() {
    let (root, home) = fixture_root();
    let config = home.join("configs/ssh_config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let output = run(
        &home,
        &[
            "setup",
            "--project",
            "payments",
            "--personal",
            config.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["roots"][0]["scope"], "personal");
    assert_eq!(document["roots"][0]["project"], "payments");
    assert_eq!(document["roots"][0]["path"], config.to_str().unwrap());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn empty_hosts_setup_cancel_keeps_explicit_config_available_for_create() {
    let (root, home) = fixture_root();
    let config = root.join("unregistered.conf");
    write(&config, "");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "tui"],
        &bin,
        &root,
        &[
            (b"No HostEntries", b"\x13"),
            (b"Setup", b"\x1b"),
            (b"No HostEntries", b"\x0e"),
            (
                b"Destination file:",
                b"unregistered.conf\tsaved\tendpoint.example\x13",
            ),
            (b"Review HostEntry changes", b"\n"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("Host saved\n  HostName endpoint.example\n"),
        "{output}"
    );
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn setup_registration_stays_open_when_saved_root_is_missing() {
    let (root, home) = fixture_root();
    let missing = home.join("missing.conf");
    let config = home.join("valid.conf");
    let original = "Host valid\n  HostName valid.example\n";
    write(&config, original);
    write(
        &home.join(".config/sshx/config.json"),
        &format!(
            "{{\"version\":1,\"roots\":[{{\"scope\":\"personal\",\"path\":\"{}\",\"project\":null}}]}}\n",
            missing.display()
        ),
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "setup", "--personal", config.to_str().unwrap()],
        &bin,
        &root,
        &[
            (b"SSH config file path:", b"\x13"),
            (b"Config root registered.", b"\x1b"),
        ],
        None,
        Some((100, 36)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".config/sshx/config.json")).unwrap()
    ).unwrap();
    assert!(settings["roots"].as_array().unwrap().iter()
        .any(|root| root["path"] == config.to_str().unwrap()));
    assert!(!missing.exists());
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_continuation_prefills_editable_fields_masks_password_and_applies() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = "Host existing\n  HostName existing.example\n";
    write(&config, original);
    let mut permissions = fs::metadata(&config).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&config, permissions).unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let config_arg = config.to_str().unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config_arg, "host", "create", "--alias", "prod"],
        &bin,
        &root,
        &[
            (b"Destination file:", b"config\t-new\tprod.example\t\t\tsecret\x13"),
            (b"Review HostEntry changes", b"\n"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("••••••"), "password was not masked: {output}");
    assert!(output.contains("<redacted>"), "review did not redact password: {output}");
    assert!(output.contains("Review HostEntry changes"), "{output}");
    assert!(!output.contains("secret"), "password leaked to terminal: {output}");
    let created = fs::read_to_string(&config).unwrap();
    assert!(created.starts_with(original), "{created}");
    assert!(created.contains("Host prod-new\n  HostName prod.example\n"), "{created}");
    assert!(created.contains("  ##PASSWORD secret\n"), "{created}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_compact_form_shows_destination_value_and_review_control() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--alias",
            "compact",
        ],
        &bin,
        &root,
        &[
            (b"Create HostEntry", b"compact.conf\t-new\tcompact.example\t\t\t\x13"),
            (b"Review HostEntry", b"\n"),
        ],
        None,
        Some((18, 30)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(contains_tui_text(output.as_bytes(), b"compact.conf"), "{output}");
    assert!(contains_tui_text(output.as_bytes(), b"compact.example"), "{output}");
    assert!(contains_tui_text(output.as_bytes(), b"^S review"), "{output}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_retains_values_after_validation_error() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let config_arg = config.to_str().unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config_arg, "host", "create"],
        &bin,
        &root,
        &[
            (b"Destination file:", b"config\tprod\tprod.example\t\tx\x13"),
            (b"PORT_INVALID", b"\x7f22\x13"),
            (b"Review HostEntry changes", b"\n"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("PORT_INVALID"), "{output}");
    assert!(output.contains("Review HostEntry changes"), "{output}");
    let created = fs::read_to_string(&config).unwrap();
    assert!(created.contains("Host prod\n  HostName prod.example\n"), "{created}");
    assert!(created.contains("  Port 22\n"), "{created}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_rejects_explicit_scope_conflict_before_opening_workspace() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_header(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "--scope",
            "work",
            "host",
            "create",
        ],
        &bin,
        &root,
        b"\x1b",
        b"SCOPE_ROOT_CONFLICT",
    );
    assert_eq!(status.code(), Some(2), "status={status:?} output={output}");
    assert!(output.contains("SCOPE_ROOT_CONFLICT"), "{output}");
    assert!(!output.contains("Create HostEntry"), "{output}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_file_required_error_focuses_destination() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--alias",
            "prod",
        ],
        &bin,
        &root,
        &[
            (b"Destination file:", b"\x1b[B\x13"),
            (b"FILE_REQUIRED", b"config\x13"),
            (b"HOSTNAME_REQUIRED", b"prod.example\x13"),
            (b"Review HostEntry changes", b"\n"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("FILE_REQUIRED"), "{output}");
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("Host prod\n  HostName prod.example\n"),
        "{output}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_scrolls_small_form_to_selected_password_field() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--alias",
            "prod",
        ],
        &bin,
        &root,
        &[(b"Create HostEntry", b"\t\t\t\t\t"), (b"assword:", b"\x1b")],
        None,
        Some((100, 8)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("assword:"), "{output}");
    assert!(output.contains("Ctrl-S review"), "{output}");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "Host existing\n  HostName existing.example\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn complete_host_create_does_not_prompt_for_optional_fields() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host existing\n  HostName existing.example\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let config_arg = config.to_str().unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config_arg,
            "--scope",
            "personal",
            "--file",
            "config",
            "--alias",
            "prod",
            "--hostname",
            "prod.example",
            "--yes",
            "host",
            "create",
        ],
        &bin,
        &root,
        &[],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("Applied host entry"), "{output}");
    assert!(!output.contains("Create HostEntry"), "{output}");
    assert!(!output.contains("[optional]"), "{output}");
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("Host prod\n  HostName prod.example\n"),
        "{output}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_noninteractive_errors_and_invalid_target_leave_files_unchanged() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = "Host existing\n  HostName existing.example\n";
    let unrelated = home.join(".ssh/unrelated.conf");
    write(&config, original);
    write(&unrelated, "unrelated\n");
    let config_arg = config.to_str().unwrap();

    let missing = run(
        &home,
        &[
            "--config",
            config_arg,
            "--scope",
            "personal",
            "host",
            "create",
            "--file",
            "config",
            "--alias",
            "prod",
            "--no-input",
        ],
    );
    assert_eq!(missing.status.code(), Some(2), "{missing:?}");
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("HOSTNAME_REQUIRED"),
        "{missing:?}"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), original);

    let invalid_target = run(
        &home,
        &[
            "--config",
            config_arg,
            "--scope",
            "personal",
            "--file",
            ".",
            "--alias",
            "prod",
            "--hostname",
            "prod.example",
            "--yes",
            "host",
            "create",
        ],
    );
    assert_eq!(invalid_target.status.code(), Some(2), "{invalid_target:?}");
    assert!(
        String::from_utf8_lossy(&invalid_target.stderr).contains("TARGET_FILE_INVALID"),
        "{invalid_target:?}"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated\n");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_create_tui_cancellation_leaves_files_unchanged() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = "Host existing\n  HostName existing.example\n";
    let unrelated = home.join(".ssh/unrelated.conf");
    write(&config, original);
    write(&unrelated, "unrelated\n");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "host",
            "create",
            "--alias",
            "prod",
        ],
        &bin,
        &root,
        &[(b"Config root", b"\x1b")],
        None,
        Some((100, 40)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated\n");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_back_and_cancel_leave_pending_operation_untouched() {
    for operation in ["connect", "tunnel", "show"] {
        let (root, home) = fixture_root();
        let config = home.join(".ssh/config");
        let original = "Host prod\n  HostName prod.example\n  ##PORT 5432\n  ##PASSWORD keep-private\n";
        write(&config, original);
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
        let bin = fake_ssh(&root);
        let args: &[&str] = match operation {
            "connect" => &["tui", "connect", "prod", "--forward", "5432=15432"],
            "tunnel" => &["tunnel", "prod"],
            _ => &["host", "show"],
        };
        let interactions: &[(&[u8], &[u8])] = if operation == "show" {
            &[(b"Search:", b"\x1b")]
        } else {
            &[
                (b"Search:", b"\r"),
                (b"Connection workspace", b"\x1b"),
                (b"Search:", b"\x1b"),
            ]
        };
        let (status, output) = run_with_pty_interactions(&home, args, &bin, &root, interactions, None, Some((100, 30)));
        assert_eq!(status.code(), Some(130), "{operation}: {output}");
        if operation != "show" {
            let (status, output) = run_with_pty_interactions(
                &home, args, &bin, &root,
                &[(b"Search:", b"\r"), (b"Connection workspace", b"\x03")],
                None, Some((100, 30)),
            );
            assert_eq!(status.code(), Some(130), "{operation}: {output}");
        }
        assert!(!output.contains("keep-private"), "{output}");
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert_eq!(fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o644);
        for path in ["master-started", "master-closed", "runtime-config", "clipboard-content"] {
            assert!(!root.join(path).exists(), "{path}: {output}");
        }
        assert!(!home.join(".config/sshx/config.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn continuation_bare_tui_selectors_prefill_without_locking_browse_scope() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let first = "11111111-1111-4111-8111-111111111111";
    let original = format!("##SSHX ID={first}\nHost selected\n  HostName selected.example\nHost other\n  HostName other.example\n");
    write(&config, &original);
    let listed = run(&home, &["host", "list", "--format", "json"]);
    let document: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let entry = document["entries"].as_array().unwrap().iter().find(|entry| entry["id"] == first).unwrap();
    let source = entry["source"]["path"].as_str().unwrap();
    let line = entry["source"]["line_start"].as_u64().unwrap().to_string();
    let bin = fake_ssh(&root);
    for selectors in [&["--id", first][..], &["--source", config.to_str().unwrap(), "--line", &line][..]] {
        let mut args = vec!["tui"];
        args.extend_from_slice(selectors);
        let (status, output) = run_with_pty_interactions(
            &home, &args, &bin, &root,
            &[(b"Search:", b"other"), (b"other.example", b"\x1b")],
            None, Some((100, 30)),
        );
        assert!(status.success(), "{selectors:?}: {output}");
        assert!(contains_tui_text(output.as_bytes(), b"> selected"), "{output}");
        assert!(contains_tui_text(output.as_bytes(), b"> other"), "{output}");
    }
    for selectors in [&["--id", "missing"][..], &["--source", source][..], &["--line", &line][..]] {
        let mut args = vec!["tui"];
        args.extend_from_slice(selectors);
        let (status, output) = run_with_pty_interactions(&home, &args, &bin, &root, &[], None, Some((80, 24)));
        assert_eq!(status.code(), Some(2), "{selectors:?}: {output}");
    }
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!root.join("runtime-config").exists());
    assert!(!root.join("clipboard-content").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_changed_host_can_replace_declared_forward_and_start_tunnel() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = "Host first\n  HostName first.example\n  ##PORT 5432\nHost second\n  HostName second.example\n  ##PORT 6379\n";
    write(&config, original);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let local_port = listener.local_addr().unwrap().port();
    drop(listener);
    let forward = format!("5432={local_port}");
    let edit = format!("e{}{}\r\r", "\x7f".repeat(4), local_port);
    let bin = fake_tunnel_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home, &["tui", "tunnel", "first", "--forward", &forward, "--password-fd", "99"], &bin, &root,
        &[
            (b"Search:", b"second\r"),
            (b"Connection workspace", edit.as_bytes()),
            (b"Enter confirm", b"\r"),
            (b"Tunnel started:", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None, Some((110, 35)),
    );
    assert!(status.success(), "{output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("Host second\n"), "{runtime}");
    let arguments = fs::read_to_string(root.join("runtime-config.args")).unwrap();
    assert!(arguments.contains(&format!("-L 127.0.0.1:{local_port}:127.0.0.1:6379")), "{arguments}");
    assert!(!arguments.contains(":5432"), "{arguments}");
    let listed = run_fake_ssh(&home, &["tunnel", "list", "--format", "json"], &bin, &root);
    assert!(listed.status.success(), "{listed:?}");
    let document: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let id = document["tunnels"][0]["id"].as_str().unwrap();
    assert!(contains_tui_text(output.as_bytes(), id.as_bytes()), "{output}");
    let stopped = run_fake_ssh(&home, &["tunnel", "stop", id, "--format", "json"], &bin, &root);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!root.join("master-started").exists());
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_edits_exact_host_without_replaying_credentials() {
    for operation in ["connect", "tunnel"] {
        let (root, home) = fixture_root();
        let config = home.join(".ssh/config");
        let original = concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "Host first secondary\n  HostName first.example\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "Host second\n  HostName second.example\n",
        );
        write(&config, original);
        let bin = fake_ssh(&root);
        let listed = run(&home, &["--config", config.to_str().unwrap(), "host", "list", "--format", "json"]);
        let entries: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
        let line = entries["entries"][0]["source"]["line_start"].as_u64().unwrap().to_string();
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut writer = unsafe { fs::File::from_raw_fd(fds[1]) };
        writer.write_all(b"original-host-only\n").unwrap();
        drop(writer);
        let mut password = unsafe { fs::File::from_raw_fd(fds[0]) };
        let password_fd = fds[0].to_string();
        let mut args = vec![
            "--config", config.to_str().unwrap(), operation, "secondary",
            "--id", "11111111-1111-4111-8111-111111111111",
            "--source", config.to_str().unwrap(), "--line", &line,
            "--password-fd", &password_fd,
        ];
        if operation == "connect" {
            args.insert(2, "tui");
        }
        let begin: &[u8] = if operation == "tunnel" { b"m\r" } else { b"\r" };
        let (status, output) = run_with_pty_interactions(
            &home, &args, &bin, &root,
            &[
                (b"Search:", b"second\r"),
                (b"Connection workspace", begin),
                (b"Enter confirm", b"\r"),
                (b"Session ended.", b"\x1b"),
                (b"Search:", b"\x1b"),
            ],
            None, Some((100, 30)),
        );
        assert!(status.success(), "{operation}: status={status:?} output={output}");
        assert!(contains_tui_text(output.as_bytes(), b"> secondary"), "{output}");
        let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
        assert!(runtime.lines().any(|line| line == "Host second"), "{runtime}");
        assert!(runtime.lines().any(|line| line.trim() == "HostName second.example"), "{runtime}");
        assert!(!runtime.lines().any(|line| matches!(line, "Host first" | "Host secondary" | "Host first secondary")), "{runtime}");
        assert!(!runtime.lines().any(|line| line.trim() == "HostName first.example"), "{runtime}");
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
        assert!(!root.join("clipboard-content").exists());
        assert!(!home.join(".config/sshx/tunnels.json").exists());
        let mut unconsumed = String::new();
        password.read_to_string(&mut unconsumed).unwrap();
        assert_eq!(unconsumed, "original-host-only\n");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn continuation_edits_supplied_forward_and_keeps_failed_start_cancellable() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host prod\n  HostName prod.example\n");
    let bin = fake_ssh(&root);
    let initial = "127.0.0.1:15432:db.internal:5432";
    let final_spec = "127.0.0.1:15433:cache.internal:6379";
    let edit = format!("e{}{}\r\r", "\x7f".repeat(initial.len()), final_spec);
    let (status, output) = run_with_pty_interactions(
        &home, &["tui", "connect", "prod", "-R", initial], &bin, &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", edit.as_bytes()),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None, Some((110, 35)),
    );
    assert!(status.success(), "{output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("RemoteForward 127.0.0.1:15433 cache.internal:6379"), "{runtime}");
    assert!(!runtime.contains("15432"), "{runtime}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();

    let (root, home) = fixture_root();
    write(&home.join(".ssh/config"), "Host prod\n  HostName prod.example\n");
    write(&root.join("host-key-changed"), "");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home, &["connect"], &bin, &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"HOST_KEY_CHANGED", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None, Some((100, 30)),
    );
    assert_eq!(status.code(), Some(130), "{output}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_nonzero_shell_exit_stays_interactive_and_completed() {
    let (root, home) = fixture_root();
    write(&home.join(".ssh/config"), "Host prod\n  HostName prod.example\n");
    let bin = fake_ssh(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    write(&ssh, &script.replace("printf 'direct-shell\\n'", "exit 7"));
    let (status, output) = run_with_pty_interactions(
        &home, &["connect"], &bin, &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"SESSION_EXIT", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None, Some((100, 30)),
    );
    assert!(status.success(), "{output}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_rejects_changed_source_then_cancels_empty_refresh() {
    for (change_at, remove_source) in [(0, false), (1, false), (0, true), (1, true)] {
        let (root, home) = fixture_root();
        let config = home.join(".ssh/config");
        write(&config, "Host prod\n  HostName prod.example\n");
        let bin = fake_ssh(&root);
        let interactions: &[(&[u8], &[u8])] = if change_at == 0 {
            &[(b"Search:", b"\r"), (b"HOST_SOURCE_CHANGED", b"\x1b")]
        } else {
            &[
                (b"Search:", b"\r"),
                (b"Connection workspace", b"\r\r"),
                (b"HOST_SOURCE_CHANGED", b"\x1b"),
            ]
        };
        let (status, output) = run_with_pty_interactions_with_hook(
            &home, &["tui", "connect", "prod"], &bin, &root, interactions, None, Some((100, 30)),
            |index| {
                if index == change_at {
                    if remove_source {
                        fs::remove_file(&config).unwrap();
                    } else {
                        write(&config, "");
                    }
                }
            },
        );
        assert_eq!(status.code(), Some(130), "{output}");
        if remove_source {
            assert!(!config.exists());
        } else {
            assert_eq!(fs::read_to_string(&config).unwrap(), "");
        }
        assert!(!root.join("master-started").exists());
        assert!(!root.join("runtime-config").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn continuation_show_exact_id_stays_interactive_after_inspection() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let first = "11111111-1111-4111-8111-111111111111";
    let second = "22222222-2222-4222-8222-222222222222";
    let original = format!(
        "##SSHX ID={first}\nHost duplicate\n  HostName one.example\n##SSHX ID={second}\nHost duplicate\n  HostName two.example\n"
    );
    write(&config, &original);
    let bin = fake_ssh(&root);
    let output = run(&home, &["host", "show", "--id", second, "--format", "json"]);
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["entries"][0]["id"], second);
    for selector in [&["--id", second][..], &[second][..]] {
        let mut args = vec!["tui", "host", "show"];
        args.extend_from_slice(selector);
        let (status, output) = run_with_pty_interactions(
            &home, &args, &bin, &root,
            &[(b"Search:", b"\r"), (b"Host show:", b"\x1b")],
            None, Some((120, 35)),
        );
        assert!(status.success(), "{selector:?}: {output}");
        assert!(contains_tui_text(output.as_bytes(), b"two.example"), "{output}");
        assert!(contains_tui_text(output.as_bytes(), second.as_bytes()), "{output}");
    }
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!root.join("runtime-config").exists());
    assert!(!root.join("clipboard-content").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn continuation_validates_exact_selectors_and_flags_before_ui() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = concat!(
        "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
        "Host duplicate\n  HostName one.example\n  ##PORT 5432\n",
        "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
        "Host duplicate\n  HostName two.example\n",
        "Host other\n  HostName other.example\n",
    );
    write(&config, original);
    let bin = fake_ssh(&root);
    let cases: &[&[&str]] = &[
        &["tui", "connect", "duplicat"],
        &["tui", "connect", "duplicate"],
        &["tui", "connect", "--id", "missing"],
        &["tui", "host", "show", "--id", "missing"],
        &["host", "show", "other", "--id", "11111111-1111-4111-8111-111111111111", "--format", "json"],
        &["tui", "connect", "other", "--id", "11111111-1111-4111-8111-111111111111"],
        &["tui", "connect", "other", "--source", config.to_str().unwrap(), "--line", "1"],
        &["tui", "connect", "other", "--source", config.to_str().unwrap()],
        &["tui", "unknown"],
        &["tui", "connect", "--format"],
        &["tui", "--format", "--no-input"],
        &["tui", "connect", "--bind", "--forward", "5432"],
        &["connect", "--bind", "--forward", "5432"],
        &["tunnel", "--bind", "--forward", "5432"],
        &["tui", "connect", "--forward", "5432=0"],
        &["tui", "connect", "--forward", "5432#bad=5432"],
        &["tui", "connect", "-L", "bad"],
        &["tui", "connect", "-R", "65536:localhost:22"],
        &["tui", "connect", "-D", "0.0.0.0:1080"],
        &["tui", "connect", "--id", "11111111-1111-4111-8111-111111111111", "--forward", "9999"],
        &["tui", "connect", "--password-fd", "3", "--vm-password-fd", "4"],
        &["tui", "--forward", "5432"],
        &["tui", "--bind"],
    ];
    for args in cases {
        let (status, output) = run_with_pty_interactions(&home, args, &bin, &root, &[], None, Some((80, 24)));
        assert_eq!(status.code(), Some(2), "{args:?}: {output}");
        assert!(!root.join("master-started").exists());
        assert!(!root.join("runtime-config").exists());
        assert!(!root.join("clipboard-content").exists());
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
    }
    let missing = home.join("missing-config");
    for flags in [&["--no-input"][..], &["--format", "json"][..], &["--password-stdin"][..]] {
        let mut args = vec!["--config", missing.to_str().unwrap(), "tui", "connect", "other"];
        args.extend_from_slice(flags);
        let (status, output) = run_with_pty_interactions(&home, &args, &bin, &root, &[], None, Some((80, 24)));
        assert_eq!(status.code(), Some(2), "{args:?}: {output}");
        assert!(output.contains("TUI_REQUIRED"), "{args:?}: {output}");
        assert!(!output.contains("cannot read config"), "{output}");
    }
    for args in [
        &["--config", missing.to_str().unwrap(), "tui", "connect", "other"][..],
        &["connect"][..],
        &["host", "show"][..],
        &["tunnel", "other"][..],
    ] {
        let output = run(&home, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("cannot read config"), "{output:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn paired_copy_actions_reject_direct_ssh_and_password() {
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
            "  Port 22\n",
        ),
    );

    let empty_path = root.join("empty-bin");
    fs::create_dir(&empty_path).unwrap();
    for action in ["copy-ssh", "copy-password"] {
        let unavailable = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .env("HOME", &home)
            .env("PATH", &empty_path)
            .args([
                "--config",
                config.to_str().unwrap(),
                "connect",
                "vm",
                "--action",
                action,
            ])
            .output()
            .unwrap();
        assert_eq!(unavailable.status.code(), Some(2), "{unavailable:?}");
        assert!(String::from_utf8_lossy(&unavailable.stderr).contains("ACTION_UNAVAILABLE"));
        assert!(unavailable.stdout.is_empty());
        assert!(!root.join("runtime-config").exists());
        assert!(!root.join("clipboard-content").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_connect_rejects_unsupported_match_before_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\nMatch host other\n  User other\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"UNSUPPORTED_MATCH", b"\x1b"),
        ],
        None,
        None,
    );
    assert_eq!(
        status.code(),
        Some(130),
        "status={status:?} output={output}"
    );
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[test]
fn selectorless_copy_failure_returns_to_selector_with_error() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let bin = fake_ssh(&root);
    let backend = fake_clipboard(&bin, &root);
    let clipboard = bin.join(backend);
    write(&clipboard, "#!/bin/sh\ncat >/dev/null\nexit 1\n");
    let mut permissions = fs::metadata(&clipboard).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&clipboard, permissions).unwrap();

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-ssh"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"CLIPBOARD_FAILED", b"\x03"),
        ],
        None,
        None,
    );
    assert_eq!(
        status.code(),
        Some(130),
        "status={status:?} output={output}"
    );
    assert!(output.contains("CLIPBOARD_FAILED"), "output={output}");
    assert!(!output.contains("Action complete."), "output={output}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[test]
fn selectorless_explicit_action_copies_after_exact_host_selection() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let bin = fake_ssh(&root);
    fake_clipboard(&bin, &root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "--action",
            "copy-ssh",
        ],
        &bin,
        &root,
        &[(b"Search:", b"\n"), (b"Action complete.", b"\x03")],
        None,
        Some((80, 24)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(
        fs::read_to_string(root.join("clipboard-content")).unwrap(),
        format!("ssh -F '{}' 'direct'", config.display())
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_password_action_without_stored_password_stays_cancellable() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "--action",
            "copy-password",
        ],
        &bin,
        &root,
        &[(b"Search:", b"\n"), (b"PASSWORD_UNAVAILABLE", b"\x03")],
        None,
        Some((80, 24)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("PASSWORD_UNAVAILABLE"), "output={output}");
    assert!(!root.join("clipboard-content").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_operations_and_tui_respect_noninteractive_precedence() {
    let (root, home) = fixture_root();
    let cases: &[(&[&str], &str)] = &[
        (&["connect"], "HOST_REQUIRED"),
        (&["connect", "--format", "json"], "HOST_REQUIRED"),
        (&["host", "show"], "HOST_REQUIRED"),
        (&["host", "show", "--format", "yaml"], "HOST_REQUIRED"),
        (&["tunnel", "status"], "TUNNEL_ID_REQUIRED"),
        (&["tunnel", "stop"], "TUNNEL_ID_REQUIRED"),
        (&["tunnel", "direct", "status", "--format", "json"], "TUNNEL_ID_REQUIRED"),
        (&["tunnel", "paired", "stop", "--no-input"], "TUNNEL_ID_REQUIRED"),
        (&["tui", "connect"], "TUI_REQUIRED"),
        (&["tui", "--no-input"], "TUI_REQUIRED"),
        (
            &[
                "tui",
                "connect",
                "prod",
                "--action",
                "connect",
                "--action",
                "connect",
            ],
            "ACTION_CONFLICT",
        ),
    ];
    for (args, expected) in cases {
        let output = run(&home, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!home.join(".config/sshx/config.json").exists());
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[cfg(unix)]
#[test]
fn interactive_tunnel_startup_failure_cancellation_exits_130_without_listener() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let local_port = listener.local_addr().unwrap().port();
    drop(listener);
    write(
        &config,
        &format!(
            "Host direct\n  HostName direct.example\n  ##PORT 5432\n  ##SSHX SERVICE 5432 LOCAL={local_port}\n"
        ),
    );
    let bin = fake_ssh(&root);
    write(&root.join("auth-fail"), "");
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tunnel",
            "direct",
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b" \r"),
            (b"Enter confirm", b"\r"),
            (b"Password for direct host", b"\n"),
            (b"Connection workspace", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((48, 18)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(output.contains("SSH_AUTH_FAILED"), "{output}");
    assert!(!root.join("master-started").exists());
    let listener = TcpListener::bind(("127.0.0.1", local_port))
        .expect("failed tunnel startup should leave local listener port available");
    drop(listener);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[test]
fn selectorless_copy_action_remains_available_with_unsupported_match() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\nMatch host other\n  User other\n",
    );
    let bin = fake_ssh(&root);
    fake_clipboard(&bin, &root);

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-ssh"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Action complete.", b"\x03"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!output.contains("UNSUPPORTED_MATCH"), "output={output}");
    assert_eq!(
        fs::read_to_string(root.join("clipboard-content")).unwrap(),
        format!("ssh -F '{}' 'direct'", config.display())
    );
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[test]
fn selectorless_copy_actions_preserve_secondary_alias() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "##SSHX ID=33333333-3333-4333-8333-333333333333\nHost primary secondary\n  HostName direct.example\n",
    );
    let bin = fake_ssh(&root);
    fake_clipboard(&bin, &root);

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-ssh"],
        &bin,
        &root,
        &[
            (b"Search:", b"\x1b[B\n"),
            (b"Action complete.", b"\x03"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(
        fs::read_to_string(root.join("clipboard-content")).unwrap(),
        format!("ssh -F '{}' 'secondary'", config.display())
    );
    assert!(!root.join("master-started").exists());

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-sshx"],
        &bin,
        &root,
        &[
            (b"Search:", b"\x1b[B\n"),
            (b"Action complete.", b"\x03"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    let command = fs::read_to_string(root.join("clipboard-content")).unwrap();
    assert!(
        command.starts_with("sshx connect 'secondary' --id '33333333-3333-4333-8333-333333333333'")
    );
    assert!(command.contains(&format!("--config '{}'", config.display())));
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn selectorless_password_action_cancel_does_not_repair_permissions() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PASSWORD secret-value\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let bin = fake_ssh(&root);

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-password"],
        &bin,
        &root,
        &[(b"Search:", b"\x1b")],
        None,
        None,
    );
    assert_eq!(
        status.code(),
        Some(130),
        "status={status:?} output={output}"
    );
    assert!(!output.contains("secret-value"), "output={output}");
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[test]
fn selectorless_password_action_confirms_without_exposing_secret() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host primary secondary\n  HostName direct.example\n  ##PASSWORD secret-value\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    fake_clipboard(&bin, &root);

    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "connect", "--action", "copy-password"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Copy stored password now?", b"y\n"),
            (b"Action complete.", b"\x03"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!output.contains("secret-value"), "output={output}");
    assert_eq!(
        fs::read_to_string(root.join("clipboard-content")).unwrap(),
        "secret-value"
    );
    assert!(
        fs::read_to_string(root.join("clipboard-args"))
            .unwrap()
            .is_empty()
    );
    assert!(
        !fs::read_to_string(root.join("clipboard-env"))
            .unwrap()
            .contains("secret-value")
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn fake_tunnel_ssh(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("fake SSH directory should be created");
    let script = bin.join("ssh");
    write(
        &script,
        r#"#!/bin/sh
config=
socket=
previous=
local_ports=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  if [ "$previous" = "-L" ] || [ "$previous" = "-D" ]; then
    local_port=${argument#*:}
    local_port=${local_port%%:*}
    local_ports="$local_ports $local_port"
  fi
  previous="$argument"
done
if [ -n "$SSHX_CAPTURE" ] && [ -f "$config" ]; then cat "$config" > "$SSHX_CAPTURE"; fi
marker="$socket.started"
pid_file="$socket.pid"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*)
    if [ -f "$pid_file" ]; then
      pid=$(cat "$pid_file")
      kill "$pid" 2>/dev/null || :
      i=0
      while [ -e "$marker" ] && [ "$i" -lt 100 ]; do
        sleep 0.01
        i=$((i + 1))
      done
    fi
    rm -f "$marker" "$pid_file"
    [ -n "$SSHX_STARTED" ] && rm -f "$SSHX_STARTED"
    [ -n "$SSHX_CLOSED" ] && : > "$SSHX_CLOSED"
    exit 0
    ;;
  *" -N "*)
    printf '%s\n' "$*" > "$SSHX_CAPTURE.args"
    (
      : > "$marker"
      [ -n "$SSHX_STARTED" ] && : > "$SSHX_STARTED"
      python3 -c 'import socket,sys,time; ss=[socket.socket() for _ in sys.argv[1:]]; [s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1) for s in ss]; [s.bind(("127.0.0.1",int(p))) for s,p in zip(ss,sys.argv[1:])]; [s.listen() for s in ss]; time.sleep(60)' $local_ports >/dev/null 2>&1 &
      listener=$!
      trap 'kill "$listener" 2>/dev/null || :; wait "$listener" 2>/dev/null || :; rm -f "$marker" "$SSHX_STARTED"; exit 0' INT HUP TERM
      while :; do sleep 0.01; done
    ) </dev/null >/dev/null 2>&1 &
    printf '%s' "$!" > "$pid_file"
    exit 0
    ;;
  *)
    printf 'direct-shell\n'
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

#[cfg(unix)]
#[test]
fn cancelling_edit_keeps_unchecked_service_unselected() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host prod\n  HostName prod.example\n  ##PORT 5432\n",
    );
    let bin = fake_ssh(&root);
    let edit = format!("{}15432\x1b", "\x7f".repeat(4));
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "connect", "prod"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Connection workspace", b"e"),
            (b"Enter save", edit.as_bytes()),
            (b"Ctrl-C cancel", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x03"),
        ],
        None,
        Some((100, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(!runtime.contains("LocalForward"), "{runtime}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn explicit_tui_edits_prefilled_nonloopback_local_forward() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host prod\n  HostName prod.example\n  ##PORT 5432\n",
    );
    let bin = fake_ssh(&root);
    let listeners = (0..3)
        .map(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap())
        .collect::<Vec<_>>();
    let ports = listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap().port())
        .collect::<Vec<_>>();
    drop(listeners);
    let declared = format!("5432={}", ports[0]);
    let initial = format!("192.0.2.1:{}:db.internal:5432", ports[1]);
    let edited = format!("192.0.2.1:{}:cache.internal:6379", ports[2]);
    let rejected = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .args([
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "prod",
            "--forward",
            &declared,
            "-L",
            &initial,
        ])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("FORWARD_BIND_UNSAFE"),
        "{rejected:?}"
    );
    let edited_id = format!("local:{edited}");
    let edit = format!("{}{edited}\r", "\x7f".repeat(initial.len()));
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "prod",
            "--allow-bind",
            "--forward",
            &declared,
            "-L",
            &initial,
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Connection workspace", b"\x1b[Be"),
            (b"Enter save", edit.as_bytes()),
            (edited_id.as_bytes(), b"\r"),
            (b"2 forwarding row(s).", b"\x03"),
        ],
        None,
        Some((100, 40)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(
        contains_tui_text(output.as_bytes(), edited_id.as_bytes()),
        "{output}"
    );
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    assert!(!home.join(".config/sshx/tunnels/registry.json").exists());
    for port in ports {
        drop(TcpListener::bind(("127.0.0.1", port)).unwrap());
    }
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn tui_connection_workspace_handles_three_declared_service_forwards_at_18_by_12() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host prod\n",
            "  HostName prod.example\n",
            "  ##PORT 5432\n",
            "  ##PORT 6379\n",
            "  ##PORT 3001\n",
        ),
    );
    let mut local_ports = Vec::new();
    while local_ports.len() < 3 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        if !local_ports.contains(&port) {
            local_ports.push(port);
        }
    }
    let bin = fake_ssh(&root);
    let listener_stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&listener_stop);
    let master_started = root.join("master-started");
    let thread_ports = local_ports.clone();
    let listener_thread = thread::spawn(move || {
        while !master_started.exists() && !thread_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
        if !thread_stop.load(Ordering::Relaxed) {
            let _listeners = thread_ports
                .into_iter()
                .map(|port| TcpListener::bind(("127.0.0.1", port)).unwrap())
                .collect::<Vec<_>>();
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let edits = local_ports
        .iter()
        .map(|port| format!("{}{port}\r", "\x7f".repeat(4)))
        .collect::<Vec<_>>();
    let interactions: &[(&[u8], &[u8])] = &[
        (b"Search:", b"\n"),
        (b"5432#1", b"e"),
        (b"Enter save", edits[0].as_bytes()),
        (b"Forwarding rows", b"\x1b[Be"),
        (b"Enter save", edits[1].as_bytes()),
        (b"Forwarding rows", b"\x1b[Be"),
        (b"Enter save", edits[2].as_bytes()),
        (b"Forwarding rows", b"\r"),
        (b"Enter confirm", b"\r"),
        (b"Session ended.", b"\x03"),
    ];
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "tui", "connect", "prod"],
        &bin,
        &root,
        interactions,
        None,
        Some((18, 12)),
    );
    listener_stop.store(true, Ordering::Relaxed);
    listener_thread.join().unwrap();
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    for (remote_port, local_port) in [5432, 6379, 3001].into_iter().zip(local_ports) {
        assert!(
            runtime.contains(&format!(
                "LocalForward 127.0.0.1:{local_port} 127.0.0.1:{remote_port}"
            )),
            "{runtime}"
        );
    }
    assert!(
        root.join("master-closed").exists(),
        "Session-owned master should close after shell exits"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn tui_workspace_marks_each_conflicting_service_row_before_start() {
    let (root, home) = fixture_root();
    let config = home.join("route-paging-0123456789/route-paging-abcdefghij/ROUTE_PAGE_TAIL/config");
    write(
        &config,
        concat!(
            "Host direct\n",
            "  HostName direct.example\n",
            "  ##PORT 5432\n",
            "  ##PORT 6379\n",
            "  ##PORT 3001\n",
        ),
    );
    let duplicate_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let duplicate_port = duplicate_listener.local_addr().unwrap().port();
    let busy_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let busy_port = busy_listener.local_addr().unwrap().port();
    let bin = fake_ssh(&root);
    let first = format!("5432={duplicate_port}");
    let second = format!("6379={duplicate_port}");
    let third = format!("3001={busy_port}");
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "direct",
            "--forward",
            first.as_str(),
            "--forward",
            second.as_str(),
            "--forward",
            third.as_str(),
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"5432#1", b"\r"),
            (b"Fix marked", b"\x1b[B"),
            (b"6379#2", b"\x1b[B"),
            (b"cannot reserve", b"\x03"),
        ],
        None,
        Some((80, 24)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    for service_id in ["5432#1", "6379#2", "3001#3"] {
        assert!(
            contains_tui_text(output.as_bytes(), service_id.as_bytes()),
            "{output}"
        );
    }
    assert!(contains_tui_text(output.as_bytes(), b"SERVICE_BIND_FAILED:"), "{output}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn tui_tunnel_returns_id_and_leaves_hosts_available() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let listeners = (0..3)
        .map(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap())
        .collect::<Vec<_>>();
    let ports = listeners.iter()
        .map(|listener| listener.local_addr().unwrap().port())
        .collect::<Vec<_>>();
    drop(listeners);
    write(
        &config,
        &format!(
            "Host direct\n  HostName direct.example\n  ##PORT 5432\n  ##SSHX SERVICE 5432 LOCAL={}\n  ##PORT 6379\n  ##SSHX SERVICE 6379 LOCAL={}\n  ##PORT 3001\n  ##SSHX SERVICE 3001 LOCAL={}\n",
            ports[0], ports[1], ports[2]
        ),
    );
    let bin = fake_tunnel_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"5432#1", b"m \x1b[B \x1b[B \r"),
            (b"Enter confirm", b"\r"),
            (b"dt-", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((48, 18)),
    );
    assert!(status.success(), "status={status:?} output={output}");

    let listed = run_fake_ssh(&home, &["tunnel", "list", "--format", "json"], &bin, &root);
    assert!(listed.status.success(), "{listed:?}");
    let tunnels: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(tunnels["tunnels"].as_array().unwrap().len(), 1);
    assert_eq!(tunnels["tunnels"][0]["state"], "active");
    let id = tunnels["tunnels"][0]["id"].as_str().unwrap();
    assert!(contains_tui_text(output.as_bytes(), id.as_bytes()), "{output}");
    let ssh_args = fs::read_to_string(root.join("runtime-config.args")).unwrap();
    for (remote_port, local_port) in [5432, 6379, 3001].into_iter().zip(&ports) {
        assert!(
            ssh_args.contains(&format!(
                "-L 127.0.0.1:{local_port}:127.0.0.1:{remote_port}"
            )),
            "{ssh_args}"
        );
        assert!(
            TcpListener::bind(("127.0.0.1", *local_port)).is_err(),
            "active tunnel does not own listener {local_port}"
        );
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let stopped = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", path)
        .env("SSHX_CAPTURE", root.join("runtime-config"))
        .env("SSHX_STARTED", root.join("master-started"))
        .env("SSHX_CLOSED", root.join("master-closed"))
        .args(["tunnel", "direct", "stop", id, "--format", "json"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!root.join("master-started").exists());
    let listed = run_fake_ssh(&home, &["tunnel", "list", "--format", "json"], &bin, &root);
    assert!(listed.status.success(), "{listed:?}");
    let tunnels: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(tunnels["tunnels"][0]["id"], id);
    assert_eq!(tunnels["tunnels"][0]["state"], "stopped");
    for port in ports {
        drop(TcpListener::bind(("127.0.0.1", port)).unwrap());
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn tui_pair_workspace_uses_vm_declared_services_only() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "Host gateway\n",
            "  HostName gateway.example\n",
            "  LocalForward 2200 vm.internal:22\n",
            "  ##PORT 2222\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  Port 22\n",
            "  ##PORT 5432\n",
            "  ##PORT 6379\n",
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
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "vm",
            "--forward",
            "5432=15432",
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Mode: Session", b"a"),
            (b"Pair routes", b"\r"),
            (b"Enter confirm", b"\x03"),
        ],
        None,
        Some((140, 36)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(
        !contains_tui_text(output.as_bytes(), b"custom 1"),
        "{output}"
    );
    assert!(
        contains_tui_text(
            output.as_bytes(),
            b"server 127.0.0.1:5432 | local 127.0.0.1:15432"
        ),
        "{output}"
    );
    assert!(
        contains_tui_text(
            output.as_bytes(),
            b"server 127.0.0.1:6379 | local 127.0.0.1:6379"
        ),
        "{output}"
    );
    assert!(!output.contains("2222#1"), "{output}");
    assert!(
        !contains_tui_text(output.as_bytes(), b"server 127.0.0.1:2222"),
        "{output}"
    );
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn workspace_restores_edited_rows_after_startup_failure() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let listeners = (0..2)
        .map(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap())
        .collect::<Vec<_>>();
    let ports = listeners.iter()
        .map(|listener| listener.local_addr().unwrap().port())
        .collect::<Vec<_>>();
    drop(listeners);
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PORT 5432\n  ##PORT 6379\n  ##PORT 3001\n",
    );
    let bin = fake_ssh(&root);
    let error_tail = "PORT_6379_UNAVAILABLE_TICKET03";
    let long_detail = format!(
        "{}{error_tail}",
        "bind failed while opening requested service listener; ".repeat(8)
    );
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let script = script.replace(
        "echo \"Permission denied, please try again.\" >&2",
        &format!("echo \"{long_detail}\" >&2"),
    );
    fs::write(&ssh, script).unwrap();
    fs::write(root.join("auth-fail"), "").unwrap();
    let first_edit = format!("{}{}\r", "\x7f".repeat(4), ports[0]);
    let second_edit = format!("{}{}\r", "\x7f".repeat(4), ports[1]);
    let mut first_runtime = None;
    let page_down = "\x1b[6~".repeat(9);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"5432#1", b"e"),
            (b"Enter save", first_edit.as_bytes()),
            (b"Forwarding rows", b"\x1b[Be"),
            (b"Enter save", second_edit.as_bytes()),
            (b"Forwarding rows", b"\r"),
            (b"Enter confirm", b"\r"),
            ("SERVICE_BIND_FAILED:".as_bytes(), page_down.as_bytes()),
            (error_tail.as_bytes(), b"\r"),
            (b"Review", b"\r"),
            (b"SERVICE_BIND_FAILED:", b"\x03"),
        ],
        None,
        Some((48, 18)),
        |index| {
            if index == 6 {
                assert!(!root.join("master-started").exists());
                assert!(!root.join("runtime-config").exists());
            }
            if index == 7 {
                first_runtime = Some(fs::read_to_string(root.join("runtime-config")).unwrap());
                assert!(!root.join("master-started").exists());
                for port in &ports {
                    drop(TcpListener::bind(("127.0.0.1", *port)).unwrap());
                }
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("SERVICE_BIND_FAILED:"), "{output}");
    let error_position = output
        .find("SERVICE_BIND_FAILED:")
        .expect("startup error should be shown");
    let recovery = &output[error_position..];
    assert!(
        contains_tui_text(recovery.as_bytes(), error_tail.as_bytes()),
        "PageDown should reveal full startup error: {output}"
    );
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    let first_runtime = first_runtime.unwrap();
    assert_eq!(
        runtime.lines().map(str::trim_start).filter(|line| line.starts_with("LocalForward ")).collect::<Vec<_>>(),
        first_runtime.lines().map(str::trim_start).filter(|line| line.starts_with("LocalForward ")).collect::<Vec<_>>(),
        "restored selection changed"
    );
    for (remote_port, local_port) in [5432, 6379].into_iter().zip(ports) {
        assert!(
            runtime.contains(&format!(
                "LocalForward 127.0.0.1:{local_port} 127.0.0.1:{remote_port}"
            )),
            "edited checked row should survive startup failure: {runtime}"
        );
        drop(TcpListener::bind(("127.0.0.1", local_port)).unwrap());
    }
    assert!(!runtime.contains("127.0.0.1:3001"), "unchecked row started: {runtime}");
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_keeps_running_when_config_root_disappears() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"No HostEntries", b"\x13"),
            (b"SSH config file path:", b"\x1b"),
            (b"No HostEntries", b"\x04"),
            (b"Evidence and guidance", b"\x1b"),
            (b"No HostEntries", b"\x1b"),
        ],
        None,
        Some((100, 36)),
        |index| {
            if index == 0 {
                fs::remove_file(&config).unwrap();
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    assert!(!config.exists());
    assert!(!home.join(".config/sshx/config.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn workspace_edit_scroll_reveals_local_listener_on_eighteen_by_twelve_terminal() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host prod\n  HostName prod.example\n  ##PORT 5432\n",
    );
    let bin = fake_ssh(&root);
    let edit = format!("{}18437\r", "\x7f".repeat(4));
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "connect", "prod"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"5432#1", b"e"),
            (b"Enter save", edit.as_bytes()),
            (b"Forwarding rows", b"\x1b[6~\x1b[6~\x1b[6~\x1b[6~\x1b[6~\x1b[6~"),
            (b"18437", b"\x03"),
        ],
        None,
        Some((18, 12)),
    );
    assert_eq!(status.code(), Some(130), "status={status:?} output={output}");
    assert!(
        contains_tui_text(output.as_bytes(), b"18437"),
        "edited local listener should remain visible after scrolling: {output}"
    );
    assert!(!root.join("master-started").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_connect_compiles_exact_block_for_multiple_forwards_with_owned_master() {
    let (root, home) = fixture_root();
    let local_listeners = (0..3)
        .map(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap())
        .collect::<Vec<_>>();
    let local_ports = local_listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap().port())
        .collect::<Vec<_>>();
    drop(local_listeners);
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=direct-id\n",
            "Host direct\n",
            "  HostName direct.example # remove this\n",
            "  User alice\n",
            "  IdentityFile ~/.ssh/id_ed25519\n",
            "  ##PASSWORD never-copy-this\n",
            "  ##PORT 5432\n",
            "  ##PORT 6379\n",
            "  ##PORT 3001\n",
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let mut args = vec!["connect".to_string(), "direct".to_string()];
    for (remote_port, local_port) in [5432, 6379, 3001].into_iter().zip(&local_ports) {
        args.push("--forward".to_string());
        args.push(format!("{remote_port}={local_port}"));
    }
    args.push("--no-input".to_string());
    let listener_stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&listener_stop);
    let master_started = root.join("master-started");
    let thread_ports = local_ports.clone();
    let listener_thread = thread::spawn(move || {
        while !master_started.exists() && !thread_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
        if !thread_stop.load(Ordering::Relaxed) {
            let _listeners = thread_ports
                .into_iter()
                .map(|port| TcpListener::bind(("127.0.0.1", port)).unwrap())
                .collect::<Vec<_>>();
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let output = run_fake_ssh_owned(&home, &args, &bin, &root);
    listener_stop.store(true, Ordering::Relaxed);
    listener_thread.join().unwrap();
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
    for (remote_port, local_port) in [5432, 6379, 3001].into_iter().zip(local_ports) {
        assert!(
            runtime.contains(&format!(
                "LocalForward 127.0.0.1:{local_port} 127.0.0.1:{remote_port}"
            )),
            "{runtime}"
        );
    }
    assert!(root.join("master-started").exists());
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[cfg(unix)]
#[test]
fn hosts_tui_config_refresh_clears_cached_password() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original =
        "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost direct\n  HostName direct.example\n";
    let changed =
        "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost direct\n  HostName remote.example\n";
    write(&config, original);
    write(&root.join("auth-fail"), "");
    let bin = fake_ssh(&root);
    fake_sshpass(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let auth_check = r#"if [ "$SSHX_AUTH_FAIL" = "1" ]; then"#;
    let fail_once = r#"if [ "$SSHX_AUTH_FAIL" = "1" ] && [ ! -f "$SSHX_CAPTURE.failed-once" ]; then
      : > "$SSHX_CAPTURE.failed-once""#;
    assert!(script.contains(auth_check));
    fs::write(&ssh, script.replace(auth_check, fail_once)).unwrap();
    let sshpass = bin.join("sshpass");
    let script = fs::read_to_string(&sshpass).unwrap();
    let capture =
        r#"[ -n "$SSHX_PASSWORD_CAPTURE" ] && eval "cat <&$password_fd" > "$SSHX_PASSWORD_CAPTURE""#;
    assert!(script.contains(capture));
    fs::write(
        &sshpass,
        script.replace(
            capture,
            r#"eval "cat <&$password_fd" >> "$SSHX_CAPTURE.passwords""#,
        ),
    )
    .unwrap();

    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let mut writer = unsafe { fs::File::from_raw_fd(fds[1]) };
    writer.write_all(b"stale-secret\n").unwrap();
    drop(writer);
    let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(fds[0], libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    let password_fd = fds[0].to_string();
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "direct",
            "--password-fd",
            &password_fd,
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (
                b"Password for direct host `direct` (replacement):",
                b"\n",
            ),
            (b"Connection workspace", b"\x1b"),
            (b"Search:", b"\r"),
            (b"HOST_SOURCE_CHANGED", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
        |index| {
            if index == 5 {
                write(&config, changed);
            }
        },
    );
    unsafe {
        libc::close(fds[0]);
    }
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!output.contains("stale-secret"), "{output}");
    assert!(
        fs::read_to_string(root.join("runtime-config"))
            .unwrap()
            .contains("HostName remote.example"),
        "{output}"
    );
    assert_eq!(
        fs::read_to_string(root.join("runtime-config.passwords")).unwrap(),
        "stale-secret\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn malformed_service_comments_do_not_block_zero_forward_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\n  ##PORT invalid\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "connect", "direct"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (
                b"FORWARD_METADATA_INVALID: ##PORT remote port must be a number",
                b"\r",
            ),
            (b"Enter confirm", b"\r"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(!runtime.contains("LocalForward"), "{runtime}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_opens_exact_duplicate_and_restores_selection_after_session() {
    let (root, home) = fixture_root();
    let ssh = home.join(".ssh");
    write(&ssh.join("config"), "Include one.conf two.conf\n");
    write(
        &ssh.join("one.conf"),
        "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost duplicate secondary\n  HostName one.example\n",
    );
    write(
        &ssh.join("two.conf"),
        "##SSHX ID=22222222-2222-4222-8222-222222222222\nHost duplicate secondary\n  HostName two.example\n",
    );
    let config_before = fs::read(ssh.join("config")).unwrap();
    let one_before = fs::read(ssh.join("one.conf")).unwrap();
    let two_before = fs::read(ssh.join("two.conf")).unwrap();
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"sshx Hosts", b"secondary\t\t\x1b[6~\x1b[B\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Search:", b"\t\t\x1b[6~"),
            (b"22222222-2222-4222-8222-222222222222", b"\x1b"),
        ],
        None,
        Some((48, 18)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("one.conf"), "output={output}");
    assert!(output.contains("two.conf"), "output={output}");
    assert!(output.contains("ID:"), "output={output}");
    let session_status = output
        .rfind("Session ended.")
        .or_else(|| output.rfind("Sessionended."))
        .expect("Hosts should show Session completion status");
    assert!(
        output[session_status..].contains("two.conf"),
        "Hosts should preserve the selected source after Session: {output}"
    );
    assert!(contains_tui_text(
        output[session_status..].as_bytes(),
        b"22222222-2222-4222-8222-222222222222",
    ), "Hosts should restore the exact selected ID: {output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.lines().any(|line| line == "Host secondary"), "{runtime}");
    assert!(runtime.contains("HostName two.example"));
    assert!(!runtime.contains("one.example"));
    assert!(!runtime.contains("LocalForward"));
    assert!(!runtime.contains("RemoteForward"));
    assert!(!runtime.contains("DynamicForward"));
    assert!(root.join("master-started").exists());
    assert!(root.join("master-closed").exists());
    assert_eq!(fs::read(ssh.join("config")).unwrap(), config_before);
    assert_eq!(fs::read(ssh.join("one.conf")).unwrap(), one_before);
    assert_eq!(fs::read(ssh.join("two.conf")).unwrap(), two_before);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_session_output_uses_terminal_when_stdout_is_redirected() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n",
    );
    write(&root.join("redirect-stdout"), "");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("direct-shell"), "output={output}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_session_output_uses_terminal_when_stdout_is_closed() {
    let (root, home) = fixture_root();
    write(
        &home.join(".ssh/config"),
        "Host direct\n  HostName direct.example\n",
    );
    write(&root.join("close-stdout"), "");
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("direct-shell"), "output={output}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_refreshes_changed_source_before_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let original = "##SSHX ID=11111111-1111-4111-8111-111111111111\nHost direct\n  HostName direct.example\n";
    let changed = "##SSHX ID=22222222-2222-4222-8222-222222222222\nHost direct\n  HostName direct.example\n";
    write(&config, original);
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"sshx Hosts", b"\r"),
            (b"HOST_SOURCE_CHANGED", b"\x1b"),
        ],
        None,
        None,
        |index| {
            if index == 0 {
                write(&config, changed);
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("22222222-2222-4222-8222-222222222222"), "{output}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    assert_eq!(fs::read_to_string(&config).unwrap(), changed);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_refreshes_changed_include_before_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let first = home.join(".ssh/one.conf");
    let second = home.join(".ssh/two.conf");
    write(&config, "Include one.conf\n");
    write(&first, "Host selected\n  HostName first.example\n");
    write(&second, "Host replacement\n  HostName second.example\n");
    let bin = fake_ssh(&root);
    let changed = "Include two.conf\n";
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"HOST_SOURCE_CHANGED", b"\x1b"),
        ],
        None,
        None,
        |index| {
            if index == 0 {
                write(&config, changed);
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("replacement"), "output={output}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    assert_eq!(fs::read_to_string(&config).unwrap(), changed);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_refreshes_changed_pair_gateway_before_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    let gateway = home.join(".ssh/gateway.conf");
    write(&config, "Include gateway.conf vm.conf\n");
    write(
        &gateway,
        "Host gateway\n  HostName gateway.example\n  LocalForward 2200 vm.internal:22\n",
    );
    write(
        &home.join(".ssh/vm.conf"),
        "Host vm\n  HostName vm.internal\n  Port 22\n",
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
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions_with_hook(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"\x1b[B\r"),
            (b"HOST_SOURCE_CHANGED", b"\x1b"),
        ],
        None,
        None,
        |index| {
            if index == 0 {
                let mut contents = fs::read_to_string(&gateway).unwrap();
                contents.push_str("# changed while Hosts is open\n");
                write(&gateway, &contents);
            }
        },
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(fs::read_to_string(&gateway).unwrap().contains("# changed while Hosts is open"));
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_rejects_unsupported_match_before_session() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host direct\n  HostName direct.example\nMatch host other\n  User other\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"Search:", b"direct\r"),
            (b"UNSUPPORTED_MATCH", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("UNSUPPORTED_MATCH"), "output={output}");
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hosts_tui_searches_unicode_aliases() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host cafe\n  HostName cafe.example\nHost CAFÉ\n  HostName accent.example\n",
    );
    let bin = fake_ssh(&root);
    let (status, output) = run_with_pty_interactions(
        &home,
        &[],
        &bin,
        &root,
        &[
            (b"sshx Hosts", b"caf\xc3\xa9\r"),
            (b"Connection workspace", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Search:", b"\x1b"),
        ],
        None,
        None,
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("CAFÉ"), "output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("Host CAFÉ"), "runtime={runtime}");
    assert!(runtime.contains("HostName accent.example"), "runtime={runtime}");
    assert!(root.join("master-closed").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn paired_nonzero_session_exit_returns_to_hosts() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n  HostName gateway.example\n  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n  HostName vm.internal\n  Port 22\n",
        ),
    );
    let config_before = fs::read(&config).unwrap();
    let bin = fake_ssh(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let failing_shell = script.replace(r#"printf 'direct-shell\n'"#, "exit 7");
    assert_ne!(script, failing_shell);
    fs::write(ssh, failing_shell).unwrap();

    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"vm\r"),
            (b"VM: vm", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Search:", b"\t\t\x1b[6~"),
            (b"22222222-2222-4222-8222-222222222222", b"\x1b"),
        ],
        None,
        Some((80, 24)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(contains_tui_text(output.as_bytes(), b"VM_SESSION_EXIT"), "{output}");
    let returned_hosts = output.rfind("Search:")
        .expect("Session exit should restore Hosts");
    assert!(contains_tui_text(
        output[returned_hosts..].as_bytes(),
        b"22222222-2222-4222-8222-222222222222",
    ), "Hosts should preserve the exact VM identity: {output}");
    assert!(contains_tui_text(output[returned_hosts..].as_bytes(), b"vm.internal"), "{output}");
    assert!(root.join("master-closed").exists());
    assert_eq!(fs::read(&config).unwrap(), config_before);
    assert!(!home.join(".config/sshx/tunnels/registry.json").exists());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn paired_retry_reuses_gateway_and_vm_password_fds() {
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
            "  Port 22\n",
        ),
    );
    let bin = paired_fake_ssh(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let fail_once = script.replace(
        r#"if [ "$SSHX_PAIRED_FAIL_ROLE" = "VM" ] && grep -q '^Host vm$' "$config"; then"#,
        r#"if grep -q '^Host vm$' "$config" && [ ! -f "$SSHX_CAPTURE.vm-failed-once" ]; then
      : > "$SSHX_CAPTURE.vm-failed-once""#,
    );
    assert_ne!(script, fail_once);
    fs::write(ssh, fail_once).unwrap();
    let sshpass = bin.join("sshpass");
    let script = fs::read_to_string(&sshpass).unwrap();
    let gateway_capture = r#"eval "cat <&$password_fd" > "$SSHX_PAIRED_GATEWAY_PASSWORD""#;
    let vm_capture = r#"eval "cat <&$password_fd" > "$SSHX_PAIRED_VM_PASSWORD""#;
    assert!(script.contains(gateway_capture));
    assert!(script.contains(vm_capture));
    let script = script
        .replace(
            gateway_capture,
            r#"eval "cat <&$password_fd" >> "$SSHX_CAPTURE.gateway-passwords""#,
        )
        .replace(
            vm_capture,
            r#"eval "cat <&$password_fd" >> "$SSHX_CAPTURE.vm-passwords""#,
        );
    fs::write(sshpass, script).unwrap();
    let make_password_fd = |password: &[u8]| {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut writer = unsafe { fs::File::from_raw_fd(fds[1]) };
        writer.write_all(password).unwrap();
        drop(writer);
        let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFD) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fds[0], libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
            0
        );
        fds[0]
    };
    let gateway_fd = make_password_fd(b"gateway-secret\n");
    let vm_fd = make_password_fd(b"vm-secret\n");
    let gateway_fd_arg = gateway_fd.to_string();
    let vm_fd_arg = vm_fd.to_string();
    let (status, output) = run_with_pty_interactions(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "tui",
            "connect",
            "vm",
            "--gateway-password-fd",
            &gateway_fd_arg,
            "--vm-password-fd",
            &vm_fd_arg,
        ],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"VM: vm", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Password for VM `vm` (replacement):", b"\n"),
            (b"VM_AUTH_FAILED", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Save replacement password for gateway `gateway`?", b"n\n"),
            (b"Save replacement password for VM `vm`?", b"n\n"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((80, 24)),
    );
    unsafe {
        libc::close(gateway_fd);
        libc::close(vm_fd);
    }
    assert!(status.success(), "status={status:?} output={output}");
    assert_eq!(
        fs::read_to_string(root.join("runtime-config.gateway-passwords"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["gateway-secret", "gateway-secret"]
    );
    assert_eq!(
        fs::read_to_string(root.join("runtime-config.vm-passwords"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["vm-secret", "vm-secret"]
    );
    assert!(!output.contains("gateway-secret"), "{output}");
    assert!(!output.contains("vm-secret"), "{output}");
    fs::remove_dir_all(root).unwrap();
}


#[test]
fn paired_session_rejects_custom_local_remote_and_socks_before_start() {
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
            "  SessionType none\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n",
            "  HostName vm.internal\n",
            "  ##PORT 5432\n",
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let config_before = fs::read(&config).unwrap();
    let bin = fake_ssh(&root);
    for (flag, specification) in [
        ("-L", "127.0.0.1:15432:db.internal:5432"),
        ("-R", "127.0.0.1:15432:db.internal:5432"),
        ("-D", "127.0.0.1:1080"),
    ] {
        let output = run_fake_ssh(
            &home,
            &[
                "--config",
                config.to_str().unwrap(),
                "connect",
                "vm",
                flag,
                specification,
                "--no-input",
            ],
            &bin,
            &root,
        );
        assert_eq!(output.status.code(), Some(2), "{flag}: {output:?}");
        assert!(!root.join("master-started").exists());
        assert!(!root.join("runtime-config").exists());
        assert!(!home.join(".config/sshx/tunnels/registry.json").exists());
        assert_eq!(fs::read(&config).unwrap(), config_before);
    }
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn paired_session_retry_can_correct_rejected_replacement_password() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(
        &config,
        concat!(
            "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
            "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
            "Host gateway\n  HostName gateway.example\n  LocalForward 2200 vm.internal:22\n",
            "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
            "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
            "##SSHX TRANSIT=vm.internal:22\n",
            "Host vm\n  HostName vm.internal\n  ##PASSWORD initial-secret\n",
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let config_before = fs::read(&config).unwrap();
    let bin = paired_fake_ssh(&root);
    let ssh = bin.join("ssh");
    let script = fs::read_to_string(&ssh).unwrap();
    let reject_password = script.replace(
        r#"if [ "$SSHX_PAIRED_FAIL_ROLE" = "VM" ] && grep -q '^Host vm$' "$config"; then"#,
        r#"if grep -q '^Host vm$' "$config" && [ "$(cat "$SSHX_CAPTURE.vm-password")" != "correct-secret" ]; then"#,
    );
    assert_ne!(script, reject_password);
    fs::write(ssh, reject_password).unwrap();
    let sshpass = bin.join("sshpass");
    let script = fs::read_to_string(&sshpass).unwrap();
    let capture_password = script.replace(
        r#"eval "cat <&$password_fd" > "$SSHX_PAIRED_VM_PASSWORD""#,
        r#"eval "cat <&$password_fd" > "$SSHX_CAPTURE.vm-password""#,
    );
    assert_ne!(script, capture_password);
    fs::write(sshpass, capture_password).unwrap();
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "tui", "connect", "vm"],
        &bin,
        &root,
        &[
            (b"Search:", b"\r"),
            (b"VM: vm", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Password for VM `vm` (replacement):", b"wrong-secret\n"),
            (b"VM_AUTH_FAILED", b"\r"),
            (b"Enter confirm", b"\r"),
            (b"Password for VM `vm` (replacement):", b"correct-secret\n"),
            (b"Save replacement password for VM `vm`?", b"n\n"),
            (b"Session ended.", b"\x1b"),
            (b"Search:", b"\x1b"),
        ],
        None,
        Some((80, 24)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(output.contains("paired-shell"), "{output}");
    assert!(!output.contains("initial-secret"), "{output}");
    assert!(!output.contains("wrong-secret"), "{output}");
    assert!(!output.contains("correct-secret"), "{output}");
    assert_eq!(fs::read(&config).unwrap(), config_before);
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn tui_adds_edits_and_starts_remote_and_socks_rows() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host prod\n  HostName prod.example\n");
    let bin = fake_ssh(&root);
    let ssh = bin.join("ssh");
    let remote_initial = "127.0.0.1:1:127.0.0.1:1";
    let remote_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let remote_listener_port = remote_listener.local_addr().unwrap().port();
    let remote_final = format!("127.0.0.1:{remote_listener_port}:cache.internal:6379");
    let socks_initial = "127.0.0.1:1";
    let socks_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let socks_edited_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let socks_port = socks_listener.local_addr().unwrap().port();
    let socks_edited_port = socks_edited_listener.local_addr().unwrap().port();
    assert_ne!(socks_port, socks_edited_port);
    drop((socks_listener, socks_edited_listener));
    let socks_final = format!("127.0.0.1:{socks_port}");
    let socks_edited = format!("127.0.0.1:{socks_edited_port}");
    let script = fs::read_to_string(&ssh).unwrap();
    let started_block = format!(
        r#"    [ -n "$SSHX_STARTED" ] && : > "$SSHX_STARTED"
    python3 -c 'import socket,sys,time; s=socket.socket(); s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(); time.sleep(60)' {socks_edited_port} >/dev/null 2>&1 &
    listener=$!
    printf '%s' "$$" > "$SSHX_CAPTURE.master"
    trap 'kill "$listener" 2>/dev/null || :; wait "$listener" 2>/dev/null || :; rm -f "$SSHX_CAPTURE.master"; exit 0' INT HUP TERM"#
    );
    write(
        &ssh,
        &script.replace(
            r#"    [ -n "$SSHX_STARTED" ] && : > "$SSHX_STARTED"
    trap 'exit 0' INT HUP TERM"#,
            &started_block,
        ).replace(
            r#"  *" -O exit "*)
    [ -n "$SSHX_CLOSED" ]"#,
            r#"  *" -O exit "*)
    kill -TERM "$(cat "$SSHX_CAPTURE.master")" 2>/dev/null || :
    while [ -f "$SSHX_CAPTURE.master" ]; do sleep 0.01; done
    [ -n "$SSHX_CLOSED" ]"#,
        ),
    );
    let initial_remote_input = format!(
        "{}{remote_initial}\n",
        "\x7f".repeat(remote_initial.len())
    );
    let replace_remote = format!(
        "{}{remote_final}\n",
        "\x7f".repeat(remote_initial.len())
    );
    let replace_socks = format!(
        "{}{socks_final}\n",
        "\x7f".repeat(socks_initial.len())
    );
    let edit_socks = format!(
        "{}{socks_edited}\n",
        "\x7f".repeat(socks_final.len())
    );
    let interactions: &[(&[u8], &[u8])] = &[
        (b"Search:", b"\n"),
        (b"Connection workspace", b"r"),
        (b"Enter save", initial_remote_input.as_bytes()),
        (b"Remote (-R)", b"d"),
        (b"Enter -D", replace_socks.as_bytes()),
        (b"SOCKS (-D) [checked]", b"\x1b[A"),
        (b"Remote (-R) [checked]", b"e"),
        (b"Edit -R", replace_remote.as_bytes()),
        (b"Remote (-R) [checked]", b"\x1b[B"),
        (b"SOCKS (-D) [checked]", b"e"),
        (b"Edit -D", edit_socks.as_bytes()),
        (socks_edited.as_bytes(), b"\r"),
        (b"Review", b"\r"),
        (b"Session ended.", b"\x03"),
    ];
    let (status, output) = run_with_pty_interactions(
        &home,
        &["--config", config.to_str().unwrap(), "tui", "connect", "prod"],
        &bin,
        &root,
        interactions,
        None,
        Some((110, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(
        runtime.contains(&format!(
            "RemoteForward 127.0.0.1:{remote_listener_port} cache.internal:6379"
        )),
        "{runtime}"
    );
    assert!(
        runtime.contains(&format!("DynamicForward {socks_edited}")),
        "{runtime}"
    );
    assert!(root.join("master-closed").exists());
    drop(TcpListener::bind(("127.0.0.1", socks_edited_port)).unwrap());
    assert!(remote_listener.local_addr().is_ok());
    fs::remove_dir_all(root).unwrap();
}


#[cfg(unix)]
#[test]
fn tui_preserves_editable_remote_rows_after_startup_failure() {
    let (root, home) = fixture_root();
    write(&home.join(".ssh/config"), "Host prod\n  HostName prod.example\n");
    fs::write(root.join("fail-once"), "").unwrap();
    let bin = fake_ssh(&root);
    let first = "127.0.0.1:2222:127.0.0.1:22";
    let second = "127.0.0.1:2223:127.0.0.1:22";
    let edited_second = "127.0.0.1:2224:127.0.0.1:22";
    let placeholder = "127.0.0.1:1:127.0.0.1:1";
    let add_first = format!("{}{first}\n", "\x7f".repeat(placeholder.len()));
    let add_second = format!("{}{second}\n", "\x7f".repeat(placeholder.len()));
    let edit_second = format!("{}{edited_second}\n", "\x7f".repeat(second.len()));
    let (status, output) = run_with_pty_interactions(
        &home,
        &["tui", "connect", "prod"],
        &bin,
        &root,
        &[
            (b"Search:", b"\n"),
            (b"Connection workspace", b"r"),
            (b"Enter -R", add_first.as_bytes()),
            (b"Remote (-R)", b"r"),
            (b"Enter -R", add_second.as_bytes()),
            (b"Remote (-R)", b"\r"),
            (b"Review", b"\r"),
            (b"SERVICE_BIND_FAILED", b"\x1b[Be"),
            (b"Edit -R", edit_second.as_bytes()),
            (b"Remote (-R)", b"\r"),
            (b"Review", b"\r"),
            (b"Session ended.", b"\x03"),
        ],
        None,
        Some((110, 40)),
    );
    assert!(status.success(), "status={status:?} output={output}");
    assert!(!root.join("fail-once").exists());
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("RemoteForward 127.0.0.1:2222 127.0.0.1:22"), "{runtime}");
    assert!(runtime.contains("RemoteForward 127.0.0.1:2224 127.0.0.1:22"), "{runtime}");
    assert!(!runtime.contains("RemoteForward 127.0.0.1:2223 "), "{runtime}");
    assert!(root.join("master-closed").exists());
    assert!(!home.join(".config/sshx/tunnels/registry.json").exists());
    fs::remove_dir_all(root).unwrap();
}


#[test]
fn direct_session_remote_nonloopback_requires_opt_in_before_start() {
    let (root, home) = fixture_root();
    let config = home.join(".ssh/config");
    write(&config, "Host direct\n  HostName direct.example\n");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let bin = fake_ssh(&root);
    let denied = run_fake_ssh(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "-R",
            "192.0.2.1:15432:db.internal:5432",
            "--no-input",
        ],
        &bin,
        &root,
    );
    assert_eq!(denied.status.code(), Some(2), "{denied:?}");
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("FORWARD_BIND_UNSAFE"),
        "{denied:?}"
    );
    assert!(!root.join("master-started").exists());
    assert!(!root.join("runtime-config").exists());

    let remote = run_fake_ssh(
        &home,
        &[
            "--config",
            config.to_str().unwrap(),
            "connect",
            "direct",
            "--allow-bind",
            "-R",
            "192.0.2.1:15432:db.internal:5432",
            "--no-input",
        ],
        &bin,
        &root,
    );
    assert!(remote.status.success(), "{remote:?}");
    let runtime = fs::read_to_string(root.join("runtime-config")).unwrap();
    assert!(runtime.contains("RemoteForward 192.0.2.1:15432 db.internal:5432"));

    fs::remove_dir_all(root).unwrap();
}
