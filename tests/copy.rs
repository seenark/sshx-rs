#![cfg(unix)]

use sshx::copy::sshpass_command_for_entry;
use sshx::discovery::discover;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn config_file(contents: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "sshx-copy-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config");
    fs::write(&path, contents).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn command_for(config: &Path, alias: &str) -> Result<String, String> {
    let catalog = discover(config).unwrap();
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.aliases.iter().any(|candidate| candidate == alias))
        .unwrap();
    sshpass_command_for_entry(
        &catalog.entries,
        entry,
        &config.to_string_lossy(),
        alias,
        "127.0.0.1",
        &[(5432, 15432)],
    )
}

#[test]
fn sshpass_command_uses_stored_password_with_forwards() {
    let config = config_file("Host db\n  HostName db.example\n  ##PASSWORD it's secret\n");
    let command = command_for(&config, "db").unwrap();
    assert_eq!(
        command,
        format!(
            " SSHPASS='it'\\''s secret' sshpass -e ssh -F '{}' 'db' -L 127.0.0.1:15432:127.0.0.1:5432",
            config.display()
        )
    );
    fs::remove_dir_all(config.parent().unwrap()).unwrap();
}

#[test]
fn sshpass_command_is_unavailable_without_stored_password() {
    let config = config_file("Host db\n  HostName db.example\n");
    let error = command_for(&config, "db").unwrap_err();
    assert!(error.starts_with("ACTION_UNAVAILABLE"), "{error}");
    fs::remove_dir_all(config.parent().unwrap()).unwrap();
}

#[test]
fn sshpass_command_is_unavailable_for_pairs() {
    let config = config_file(concat!(
        "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
        "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
        "Host gateway\n  HostName gateway.example\n  LocalForward 2200 vm.internal:22\n",
        "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
        "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
        "##SSHX TRANSIT=vm.internal:22\n",
        "Host vm\n  HostName vm.internal\n  Port 22\n  ##PASSWORD vm-secret\n",
    ));
    let error = command_for(&config, "vm").unwrap_err();
    assert!(error.starts_with("ACTION_UNAVAILABLE"), "{error}");
    assert!(!error.contains("vm-secret"));
    fs::remove_dir_all(config.parent().unwrap()).unwrap();
}
