use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (PathBuf, PathBuf) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("temporary directory should resolve")
        .join(format!("sshx-ticket-13-{timestamp}-{ordinal}"));
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
fn doctor_reports_config_stages_without_secret_or_repair() {
    let (root, home) = fixture();
    let config = home.join(".ssh/config");
    write(
        &home.join(".ssh/conf.d/pw.conf"),
        "Host included\n  HostName included.example\n",
    );
    let occupied = TcpListener::bind("127.0.0.1:0").expect("fixture listener should bind");
    let occupied_port = occupied.local_addr().expect("listener address").port();
    write(
        &config,
        &format!(
            concat!(
                "Include conf.d/[pw]*.conf\n",
                "Include missing.conf\n",
                "Match exec true\n",
                "##SSHX ID=duplicate-id\n",
                "Host gateway\n",
                "  HostName gateway.example\n",
                "  LocalForward 2200 vm.internal:22\n",
                "  ##PASSWORD secret-value\n",
                "##SSHX ID=duplicate-id\n",
                "Host vm\n",
                "  HostName vm.internal\n",
                "  Port 22\n",
                "  ##SSHX GATEWAY=missing-gateway\n",
                "  ##SSHX TRANSIT=vm.internal:22\n",
                "  ##PORT {occupied_port}\n",
            ),
            occupied_port = occupied_port
        ),
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644))
        .expect("fixture permissions should be set");
    let before = fs::read(&config).expect("config should be readable");

    let json = run(
        &home,
        &[
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "json",
        ],
    );
    assert_eq!(json.status.code(), Some(2), "{json:?}");
    let json_value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON output");
    let findings = json_value["findings"].as_array().expect("findings array");
    for code in [
        "include_missing",
        "unsupported_semantics",
        "duplicate_id",
        "broken_reference",
        "password_file_permissions",
        "local_port_conflict",
    ] {
        assert!(
            findings.iter().any(|finding| finding["code"] == code),
            "missing finding {code}: {json_value}"
        );
    }
    assert_eq!(json_value["validation"]["remote_servers"], "not_run");
    assert_eq!(json_value["known_hosts"][0]["scope"], "personal");
    assert!(String::from_utf8_lossy(&json.stdout).contains("stage"));
    assert!(!String::from_utf8_lossy(&json.stdout).contains("secret-value"));
    assert_eq!(
        fs::read(&config).expect("config should remain readable"),
        before
    );

    let yaml = run(
        &home,
        &[
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "yaml",
        ],
    );
    assert_eq!(yaml.status.code(), Some(2), "{yaml:?}");
    let yaml_value: serde_yaml::Value = serde_yaml::from_slice(&yaml.stdout).expect("YAML output");
    let yaml_findings = yaml_value["findings"].as_sequence().expect("YAML findings");
    for code in [
        "include_missing",
        "unsupported_semantics",
        "duplicate_id",
        "broken_reference",
        "password_file_permissions",
        "local_port_conflict",
    ] {
        assert!(
            yaml_findings.iter().any(|finding| finding["code"] == code),
            "missing YAML finding {code}: {yaml_value:?}"
        );
    }
    assert_eq!(yaml_value["validation"]["remote_servers"], "not_run");
    assert!(!String::from_utf8_lossy(&yaml.stdout).contains("secret-value"));

    let human = run(
        &home,
        &[
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
        ],
    );
    assert_eq!(human.status.code(), Some(2), "{human:?}");
    let human_text = String::from_utf8_lossy(&human.stdout);
    assert!(human_text.contains("Next:"));
    assert!(human_text.contains("[error] config"));
    assert!(human_text.contains("Path:"));
    assert!(human_text.contains("Evidence: local"));
    assert!(human_text.contains("remote server validation: not run"));
    assert!(!human_text.contains("secret-value"));

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn doctor_checks_sshpass_only_when_password_auth_is_needed() {
    let (root, home) = fixture();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("fake bin should be created");
    let ssh = bin.join("ssh");
    write(&ssh, "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755))
        .expect("fake ssh should be executable");
    let config = home.join(".ssh/config");
    write(&config, "Host key-only\n  HostName key.example\n");

    let key_only = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &bin)
        .args([
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "json",
        ])
        .output()
        .expect("sshx binary should run");
    assert!(key_only.status.success(), "{key_only:?}");
    let key_value: serde_json::Value =
        serde_json::from_slice(&key_only.stdout).expect("JSON output");
    assert!(
        !key_value["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| finding["code"] == "sshpass_missing")
    );

    write(
        &config,
        "Host password\n  HostName password.example\n  PreferredAuthentications publickey,password # comment\n",
    );
    let password = Command::new(env!("CARGO_BIN_EXE_sshx"))
        .env("HOME", &home)
        .env("PATH", &bin)
        .args([
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "json",
        ])
        .output()
        .expect("sshx binary should run");
    assert!(password.status.success(), "{password:?}");
    let password_value: serde_json::Value =
        serde_json::from_slice(&password.stdout).expect("JSON output");
    assert!(
        password_value["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| finding["code"] == "sshpass_missing")
    );

    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn doctor_accepts_tokenized_user_known_hosts_file_and_proxycommand() {
    let (root, home) = fixture();
    write(
        &home.join(".ssh/config"),
        concat!(
            "Host direct\n",
            "  HostName direct.example\n",
            "  UserKnownHostsFile /dev/null %d/known_hosts\n",
            "  ProxyCommand cloudflared access ssh --hostname %h\n",
        ),
    );

    let report = report_for(&home);
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == "unsupported_semantics"
                && (finding.message.contains("UserKnownHostsFile")
                    || finding.message.contains("ProxyCommand"))
        }),
        "directives should remain supported: {:?}",
        report.findings
    );
    fs::remove_dir_all(root).expect("fixture directory should be removed");
}

#[test]
fn doctor_accepts_session_type_none_for_transport_hosts() {
    let (root, home) = fixture();
    write(
        &home.join(".ssh/config"),
        "Host gateway\n  HostName gateway.example\n  SessionType none\n  LocalForward 2200 vm.internal:22\n",
    );

    let report = report_for(&home);
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == "unsupported_semantics" && finding.message.contains("SessionType")
        }),
        "SessionType should remain supported: {:?}",
        report.findings
    );
    fs::remove_dir_all(root).expect("fixture directory should be removed");
}
#[test]
fn doctor_reports_stale_runtime_without_repairing_registry() {
    let (root, home) = fixture();
    let config = home.join(".ssh/config");
    write(&config, "Host current\n  HostName current.example\n");
    let tunnel_root = home.join(".config/sshx/tunnels");
    fs::create_dir_all(&tunnel_root).expect("tunnel state should be created");
    fs::set_permissions(home.join(".config/sshx"), fs::Permissions::from_mode(0o700))
        .expect("app state should be private");
    fs::set_permissions(&tunnel_root, fs::Permissions::from_mode(0o700))
        .expect("tunnel state should be private");
    let control_dir = tunnel_root.join("stale");
    let control_socket = control_dir.join("master.sock");
    let runtime_config = control_dir.join("config");
    let registry = serde_json::json!({
        "version": 1,
        "tunnels": [{
            "id": "tunnel-stale",
            "state": "active",
            "kind": "direct",
            "entry_id": "missing-entry",
            "aliases": ["stale"],
            "selected_alias": "stale",
            "source_path": config,
            "source_byte_start": 0,
            "source_byte_end": 10,
            "source_line_start": 1,
            "source_line_end": 1,
            "block_fingerprint": "",
            "request_signature": "",
            "control_dir": control_dir,
            "control_socket": control_socket,
            "runtime_config": runtime_config,
            "forwards": [],
            "pair": null,
            "error": null
        }]
    });
    let registry_path = tunnel_root.join("registry.json");
    write(
        &registry_path,
        &serde_json::to_string_pretty(&registry).expect("registry should serialize"),
    );
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600))
        .expect("registry should be private");
    let before = fs::read(&registry_path).expect("registry should be readable");

    let output = run(
        &home,
        &[
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["runtime"][0]["selection"], "stale");
    for code in [
        "stale_selection",
        "managed_master_state",
        "listener_state",
        "application_health_unproven",
        "stale_runtime_socket",
    ] {
        assert!(
            value["findings"]
                .as_array()
                .expect("findings array")
                .iter()
                .any(|finding| finding["code"] == code),
            "missing runtime finding {code}: {value:?}"
        );
    }
    assert_eq!(
        before,
        fs::read(&registry_path).expect("registry should remain readable")
    );
    fs::remove_dir_all(root).expect("fixture should be removed");
}

#[test]
fn doctor_accepts_private_ready_state_locations() {
    let (root, home) = fixture();
    let config = home.join(".ssh/config");
    write(&config, "Host ready\n  HostName ready.example\n");
    let app_dir = home.join(".config/sshx");
    let tunnel_dir = app_dir.join("tunnels");
    fs::create_dir_all(&tunnel_dir).expect("app state should be created");
    for path in [&app_dir, &tunnel_dir] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .expect("app directory should be private");
    }
    let settings = app_dir.join("config.json");
    write(&settings, "{}");
    let registry = tunnel_dir.join("registry.json");
    write(&registry, r#"{"version":1,"tunnels":[]}"#);
    for path in [&settings, &registry] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("app file should be private");
    }
    let personal_known_hosts = home.join(".ssh/known_hosts");
    let work_known_hosts = app_dir.join("known_hosts/work");
    write(&personal_known_hosts, "");
    write(&work_known_hosts, "");
    for path in [&personal_known_hosts, &work_known_hosts] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("known-host file should be private");
    }

    let output = run(
        &home,
        &[
            "--config",
            config.to_str().expect("UTF-8 fixture path"),
            "doctor",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert!(
        value["state"]
            .as_array()
            .expect("state array")
            .iter()
            .all(|state| state["status"] == "ready"),
        "state locations not ready: {value:?}"
    );
    assert!(
        value["known_hosts"]
            .as_array()
            .expect("known-host array")
            .iter()
            .all(|state| state["status"] == "ready")
    );
    assert!(
        !value["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| {
                finding["code"] == "app_state_insecure" || finding["code"] == "known_hosts_insecure"
            }),
        "private state should not be insecure: {value:?}"
    );
    fs::remove_dir_all(root).expect("fixture should be removed");
}

use sshx::doctor::{self, Selection};
use sshx::permissions;
use sshx::settings::{self, RegisteredRoot};

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .expect("path should exist")
        .permissions()
        .mode()
        & 0o7777
}

fn report_for(home: &Path) -> doctor::DoctorReport {
    let roots = vec![RegisteredRoot {
        scope: "personal".to_string(),
        path: home.join(".ssh/config"),
        project: None,
    }];
    doctor::run(
        home,
        &roots,
        None,
        false,
        Selection {
            id: None,
            source: None,
            line: None,
            alias: None,
        },
    )
}

#[test]
fn repair_plan_covers_only_eligible_private_paths() {
    let (root, home) = fixture();
    let config = home.join(".ssh/config");
    write(
        &config,
        "Host gateway\n  HostName gateway.example\n  ##PASSWORD stored-secret\nHost plain\n  HostName plain.example\n",
    );
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let app_dir = home.join(".config/sshx");
    let tunnel_dir = app_dir.join("tunnels");
    fs::create_dir_all(&tunnel_dir).unwrap();
    fs::set_permissions(&tunnel_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&app_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let settings_path = app_dir.join("config.json");
    write(&settings_path, "{}");
    fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o644)).unwrap();
    let work_hosts_parent = app_dir.join("known_hosts");
    fs::create_dir_all(&work_hosts_parent).unwrap();
    let work_hosts = work_hosts_parent.join("work");
    write(&work_hosts, "");
    fs::set_permissions(&work_hosts, fs::Permissions::from_mode(0o644)).unwrap();

    let report = report_for(&home);
    let planned = report
        .repairs
        .iter()
        .map(|candidate| (candidate.path.clone(), candidate.target))
        .collect::<Vec<_>>();
    assert!(
        planned.contains(&(settings_path.clone(), permissions::PermissionTarget::File)),
        "settings should be planned: {planned:?}"
    );
    assert!(
        planned.contains(&(app_dir.clone(), permissions::PermissionTarget::Directory)),
        "app dir should be planned: {planned:?}"
    );
    assert!(
        planned.contains(&(work_hosts.clone(), permissions::PermissionTarget::File)),
        "work known-hosts should be planned: {planned:?}"
    );
    assert!(
        planned.contains(&(config.clone(), permissions::PermissionTarget::File)),
        "password-bearing config should be planned: {planned:?}"
    );
    assert!(
        !planned.iter().any(|(path, _)| path.ends_with("tunnels")),
        "tunnel dir mode is 0700 already: {planned:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_readable_config_without_password_never_becomes_repair() {
    let (root, home) = fixture();
    let config = home.join(".ssh/config");
    write(&config, "Host plain\n  HostName plain.example\n");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    let report = report_for(&home);
    assert!(
        report
            .repairs
            .iter()
            .all(|candidate| !candidate.path.ends_with(".ssh/config")),
        "non-password config must not be planned: {:?}",
        report.repairs
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symlinked_state_is_diagnosed_but_never_planned_or_repaired() {
    let (root, home) = fixture();
    write(
        &home.join(".ssh/config"),
        "Host plain\n  HostName plain.example\n",
    );
    let app_dir = home.join(".config/sshx");
    fs::create_dir_all(&app_dir).unwrap();
    let outside = root.join("outside.json");
    write(&outside, "{}");
    let settings_path = app_dir.join("config.json");
    std::os::unix::fs::symlink(&outside, &settings_path).unwrap();

    let report = report_for(&home);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "app_state_insecure"),
        "symlink must remain a diagnostic: {:?}",
        report.findings
    );
    assert!(
        report
            .repairs
            .iter()
            .all(|candidate| candidate.path != settings_path),
        "symlink must not be planned: {:?}",
        report.repairs
    );
    let results = permissions::apply_all(&report.repairs);
    assert!(
        results
            .iter()
            .all(|result| result.candidate.path != settings_path),
        "symlinked path must never be repaired: {results:?}"
    );
    assert!(
        settings_path
            .symlink_metadata()
            .expect("symlink should still exist")
            .file_type()
            .is_symlink(),
        "symlink must never be replaced"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn apply_all_repairs_exact_modes_and_continues_after_failure() {
    let (root, home) = fixture();
    write(
        &home.join(".ssh/config"),
        "Host plain\n  HostName plain.example\n",
    );
    let app_dir = home.join(".config/sshx");
    fs::create_dir_all(&app_dir).unwrap();
    fs::set_permissions(&app_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let settings_path = app_dir.join("config.json");
    write(&settings_path, "{}");
    fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o644)).unwrap();
    let known_hosts = home.join(".ssh/known_hosts");
    write(&known_hosts, "");
    fs::set_permissions(&known_hosts, fs::Permissions::from_mode(0o644)).unwrap();

    let report = report_for(&home);
    assert!(report.repairs.len() >= 3, "{:?}", report.repairs);
    fs::remove_file(&known_hosts).unwrap();
    let results = permissions::apply_all(&report.repairs);
    assert_eq!(results.len(), report.repairs.len());
    let missing = results
        .iter()
        .find(|result| result.candidate.path == known_hosts)
        .expect("removed path should still be reported");
    assert_eq!(missing.outcome, permissions::RepairOutcome::Skipped);
    for result in &results {
        if result.outcome == permissions::RepairOutcome::Fixed {
            assert_eq!(
                mode_of(&result.candidate.path),
                result.candidate.target.private_mode()
            );
        }
    }
    assert_eq!(mode_of(&settings_path), permissions::PRIVATE_FILE_MODE);
    assert_eq!(mode_of(&app_dir), permissions::PRIVATE_DIR_MODE);
    let rendered = sshx::output::render_repairs(&results, sshx::output::OutputFormat::Human)
        .expect("repair results should render");
    assert!(rendered.contains("fixed:"), "{rendered}");
    assert!(rendered.contains("skipped:"), "{rendered}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn apply_permissions_does_not_follow_replacement_symlink() {
    let (root, home) = fixture();
    let path = home.join(".ssh/password");
    let outside = root.join("outside");
    write(&path, "private");
    write(&outside, "outside secret");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
    let candidate = permissions::RepairCandidate {
        kind: "password_file".to_string(),
        path: path.clone(),
        target: permissions::PermissionTarget::File,
        current_mode: permissions::PRIVATE_FILE_MODE,
        reason: "test repair".to_string(),
    };

    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let result = permissions::apply(&candidate);

    assert_eq!(result.outcome, permissions::RepairOutcome::Skipped);
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert_eq!(mode_of(&outside), 0o644);
    assert_eq!(fs::read_to_string(&outside).unwrap(), "outside secret");

    let outside_dir = root.join("outside-dir");
    fs::create_dir(&outside_dir).unwrap();
    let outside_child = outside_dir.join("password");
    write(&outside_child, "outside directory secret");
    fs::set_permissions(&outside_child, fs::Permissions::from_mode(0o644)).unwrap();
    fs::remove_file(&path).unwrap();
    fs::remove_dir_all(home.join(".ssh")).unwrap();
    std::os::unix::fs::symlink(&outside_dir, home.join(".ssh")).unwrap();
    let result = permissions::apply(&candidate);

    assert_eq!(result.outcome, permissions::RepairOutcome::Skipped);
    assert!(fs::symlink_metadata(home.join(".ssh")).unwrap().file_type().is_symlink());
    assert_eq!(mode_of(&outside_child), 0o644);
    assert_eq!(
        fs::read_to_string(&outside_child).unwrap(),
        "outside directory secret"
    );
}

#[test]
fn settings_save_creates_private_modes_regardless_of_umask() {
    let (root, home) = fixture();
    let roots = vec![RegisteredRoot {
        scope: "personal".to_string(),
        path: home.join(".ssh/config"),
        project: None,
    }];
    settings::save(&home, &roots).expect("settings should save");
    let app_dir = home.join(".config/sshx");
    assert_eq!(mode_of(&app_dir), permissions::PRIVATE_DIR_MODE);
    assert_eq!(
        mode_of(&settings::settings_path(&home)),
        permissions::PRIVATE_FILE_MODE
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_chmod_hint_is_shell_quoted() {
    let hint = permissions::manual_chmod_command(
        &PathBuf::from("/tmp/my config's/id_rsa"),
        permissions::PermissionTarget::File,
    );
    assert_eq!(hint, "chmod 600 '/tmp/my config'\\''s/id_rsa'");
}
#[test]
fn permission_contract_rejects_shared_files_and_symlink_components() {
    let (root, home) = fixture();
    let shared = home.join("shared");
    write(&shared, "secret");
    let hard_link = home.join("shared-copy");
    fs::hard_link(&shared, &hard_link).expect("hard link should be created");
    assert!(
        permissions::assess(&shared, permissions::PermissionTarget::File).is_err(),
        "shared file must not be eligible"
    );

    let real_dir = home.join("real");
    fs::create_dir_all(&real_dir).expect("real directory should be created");
    let target = real_dir.join("target");
    write(&target, "secret");
    let symlink_dir = home.join("link");
    std::os::unix::fs::symlink(&real_dir, &symlink_dir).expect("symlink should be created");
    let through_symlink = symlink_dir.join("target");
    assert!(
        permissions::assess(&through_symlink, permissions::PermissionTarget::File).is_err(),
        "path through symlink must not be eligible"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_never_plans_shared_files_or_paths_through_symlink_parents() {
    let (root, home) = fixture();
    write(
        &home.join(".ssh/config"),
        "Host plain\n  HostName plain.example\n",
    );

    let app_dir = home.join(".config/sshx");
    fs::create_dir_all(&app_dir).unwrap();
    let settings_path = app_dir.join("config.json");
    write(&settings_path, "{}");
    fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::hard_link(&settings_path, app_dir.join("settings-copy")).unwrap();

    let outside = root.join("outside-known-hosts");
    fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("work");
    write(&outside_file, "");
    fs::set_permissions(&outside_file, fs::Permissions::from_mode(0o644)).unwrap();
    let known_hosts_parent = app_dir.join("known_hosts");
    std::os::unix::fs::symlink(&outside, &known_hosts_parent).unwrap();

    let report = report_for(&home);
    assert!(
        report
            .repairs
            .iter()
            .all(|candidate| candidate.path != settings_path && candidate.path != outside_file),
        "unsafe paths must remain diagnostics: {:?}",
        report.repairs
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn human_doctor_groups_severity_and_stage_with_actionable_evidence() {
    let report = sshx::doctor::DoctorReport {
        version: 1,
        validation: sshx::doctor::ValidationInfo {
            local: "fixture".to_string(),
            remote_servers: "not_run".to_string(),
        },
        roots: vec![sshx::doctor::RootReport {
            scope: "user".to_string(),
            path: "/home/user/.ssh/config".to_string(),
            project: None,
            status: "readable".to_string(),
            evidence: "fixture root".to_string(),
        }],
        known_hosts: vec![sshx::doctor::KnownHostReport {
            scope: "user".to_string(),
            path: "/home/user/.ssh/known_hosts".to_string(),
            status: "readable".to_string(),
            evidence: "fixture inventory".to_string(),
        }],
        state: Vec::new(),
        runtime: Vec::new(),
        findings: vec![
            sshx::doctor::Finding {
                code: "include_missing".to_string(),
                severity: "error".to_string(),
                stage: "config".to_string(),
                message: "included file is missing".to_string(),
                guidance: "restore the referenced file".to_string(),
                path: Some("/home/user/.ssh/config".to_string()),
                evidence: "Include target does not exist".to_string(),
            },
            sshx::doctor::Finding {
                code: "root_unreadable".to_string(),
                severity: "error".to_string(),
                stage: "config".to_string(),
                message: "config root cannot be read".to_string(),
                guidance: "check file access".to_string(),
                path: Some("/home/user/.ssh/other.conf".to_string()),
                evidence: "read returned permission denied".to_string(),
            },
            sshx::doctor::Finding {
                code: "private_mode".to_string(),
                severity: "error".to_string(),
                stage: "permissions".to_string(),
                message: "private file has broad permissions".to_string(),
                guidance: "review chmod repair".to_string(),
                path: Some("/home/user/.ssh/id_key".to_string()),
                evidence: "mode is 0644".to_string(),
            },
            sshx::doctor::Finding {
                code: "stale_runtime".to_string(),
                severity: "warning".to_string(),
                stage: "runtime".to_string(),
                message: "runtime record is stale".to_string(),
                guidance: "inspect runtime state".to_string(),
                path: Some("/home/user/.config/sshx/run".to_string()),
                evidence: "control socket is absent".to_string(),
            },
            sshx::doctor::Finding {
                code: "remote_unchecked".to_string(),
                severity: "info".to_string(),
                stage: "config".to_string(),
                message: "remote server check was not run".to_string(),
                guidance: "validate in an authorized environment".to_string(),
                path: None,
                evidence: "remote access is outside this report".to_string(),
            },
        ],
        repairs: Vec::new(),
    };
    let rendered =
        sshx::output::render_doctor(&report, sshx::output::OutputFormat::Human).unwrap();

    let config = rendered.find("[error] config\n").unwrap();
    let first = rendered.find("include_missing:").unwrap();
    let second = rendered.find("root_unreadable:").unwrap();
    let permissions = rendered.find("[error] permissions\n").unwrap();
    let warning = rendered.find("[warning] runtime\n").unwrap();
    let info = rendered.find("[info] config\n").unwrap();
    let root_inventory = rendered.find("root user: /home/user/.ssh/config").unwrap();
    let known_hosts = rendered
        .find("known-hosts user: /home/user/.ssh/known_hosts")
        .unwrap();
    assert!(config < permissions && permissions < warning);
    assert!(warning < root_inventory && root_inventory < known_hosts && known_hosts < info);
    assert!(config < first && first < second && second < permissions);
    assert!(permissions < warning && warning < info);
    assert_eq!(rendered.matches("[error] config\n").count(), 1);
    assert!(rendered.contains("Path: /home/user/.ssh/config"));
    assert!(rendered.contains("Evidence: Include target does not exist"));
    assert!(rendered.contains("Next: restore the referenced file"));
}
