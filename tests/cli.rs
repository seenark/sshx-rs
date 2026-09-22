use std::process::Command;

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
