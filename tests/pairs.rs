#![cfg(unix)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn pair_setup_recovers_interrupted_journal_only_after_explicit_consent() {
    let fixture = PairFixture::new("recovery-pair");
    let original = fixture.snapshot();
    let target = fs::canonicalize(&fixture.gateway).unwrap();
    let before = fixture.root.join("gateway-before");
    let after = fixture.root.join("gateway-after");
    let lock = fixture.gateway.with_file_name(".gateway-source.sshx.lock");
    let journal = lock.with_file_name(".gateway-source.sshx.lock.journal");
    fs::copy(&fixture.gateway, &before).unwrap();
    let interrupted = fs::read_to_string(&fixture.gateway).unwrap()
        .replace("Host gateway\n", "Host interrupted-gateway\n");
    fs::write(&fixture.gateway, &interrupted).unwrap();
    fs::copy(&fixture.gateway, &after).unwrap();
    fs::write(&lock, b"stale lock file from crashed writer").unwrap();
    // The journal wire format uses FNV-1a digests of the before and after bytes.
    let digest = |bytes: &[u8]| bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    let journal_bytes = serde_json::to_vec(&serde_json::json!({
        "writes": [{
            "path": target,
            "before": before,
            "after": after,
            "before_digest": digest(&original[1].0),
            "after_digest": digest(interrupted.as_bytes()),
            "existed": true
        }]
    })).unwrap();
    fs::write(&journal, &journal_bytes).unwrap();
    let pending = fixture.snapshot();

    let output = fixture.command()
        .args(["pair", "setup", "gateway", "--yes", "--no-input"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    fixture.assert_snapshot(&pending);
    assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    assert_eq!(fs::read(&before).unwrap(), original[1].0);

    let arguments = ["pair", "setup", "gateway", "--yes"];
    let mut terminal = PairTerminal::open(&fixture, &arguments);
    terminal.expect("Pending Pair mutation affects:");
    terminal.expect("Recover pending Pair mutation before setup?");
    assert!(String::from_utf8_lossy(&terminal.output).contains(target.to_str().unwrap()));
    fixture.assert_snapshot(&pending);
    terminal.send(b"n\r");
    assert_eq!(terminal.finish(), Some(2));
    fixture.assert_snapshot(&pending);
    assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    assert_eq!(fs::read(&before).unwrap(), original[1].0);
    assert_eq!(fs::read(&after).unwrap(), interrupted.as_bytes());

    let mut terminal = PairTerminal::open(&fixture, &arguments);
    terminal.expect("Pending Pair mutation affects:");
    terminal.expect("Recover pending Pair mutation before setup?");
    assert!(String::from_utf8_lossy(&terminal.output).contains(target.to_str().unwrap()));
    fixture.assert_snapshot(&pending);
    terminal.send(b"y\r");
    terminal.expect("Pair setup");
    terminal.expect("Gateway: gateway");
    fixture.assert_snapshot(&original);
    assert!(!journal.exists(), "recovered journal remains pending");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(130));
    fixture.assert_snapshot(&original);
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn partial_pair_setup_reviews_exact_duplicate_vm_and_chosen_transit_before_apply() {
    let fixture = PairFixture::new("exact-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway", "--yes"]);
    terminal.expect("Pair setup");
    terminal.expect("gateway");
    terminal.send(b"vm\x1b[B\r");
    terminal.expect("second-vm");
    terminal.send(b"\x13");
    terminal.expect("TRANSIT_REQUIRED");
    fixture.assert_snapshot(&before);
    terminal.send(b"\x0e\x0e");
    terminal.expect("127.0.0.2");
    terminal.send(b"\x13");
    terminal.expect("Review Pair changes");
    terminal.expect("second-vm");
    terminal.expect("127.0.0.2:2222");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"),
        "Pair review exposed a stored password");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect("Pair saved.");
    assert!(!String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"),
        "Pair result exposed a stored password");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));

    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    let pair = &pairs["pairs"][0];
    assert_eq!(pair["gateway_alias"], "gateway");
    assert_eq!(pair["vm_alias"], "vm");
    assert_eq!(pair["transit_host"], "127.0.0.2");
    assert_eq!(pair["transit_port"], 2222);
    let gateway = fs::read_to_string(&fixture.gateway).unwrap();
    let vm = fs::read_to_string(&fixture.vm).unwrap();
    assert!(gateway.contains(&format!("##SSHX VM={}", pair["vm_id"].as_str().unwrap())));
    assert!(vm.contains(&format!("##SSHX GATEWAY={}", pair["gateway_id"].as_str().unwrap())));
    assert!(vm.contains("##SSHX TRANSIT=127.0.0.2:2222"));
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
}

#[test]
fn pair_setup_review_and_interrupt_cancellation_preserve_all_sources() {
    let fixture = PairFixture::new("cancel-pair");
    let before = fixture.snapshot();
    let vm_source = fixture.vm.to_str().unwrap();
    let arguments = ["tui", "pair", "setup", "gateway", "vm", "--vm-source", vm_source,
        "--vm-line", "1", "--transit-host", "127.0.0.1", "--transit-port", "2222", "--yes"];
    for interrupt in [false, true] {
        let mut terminal = PairTerminal::open(&fixture, &arguments);
        terminal.expect("Pair setup");
        terminal.send(b"\x13");
        terminal.expect("Review Pair changes");
        fixture.assert_snapshot(&before);
        if interrupt {
            terminal.send(b"\x03");
        } else {
            terminal.send(b"\x1b");
            terminal.expect_absent("Review Pair changes");
            fixture.assert_snapshot(&before);
            terminal.send(b"\x1b");
        }
        assert_eq!(terminal.finish(), Some(130));
        fixture.assert_snapshot(&before);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
    }
}

#[test]
fn pair_setup_invalid_transit_keeps_workspace_open_without_partial_metadata() {
    let fixture = PairFixture::new("invalid-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "setup", "gateway", "vm",
        "--vm-source", fixture.vm.to_str().unwrap(), "--vm-line", "1",
        "--transit-host", "127.0.0.1", "--transit-port", "0"]);
    terminal.expect("Pair setup");
    terminal.send(b"\x13");
    terminal.expect("TRANSIT_PORT_INVALID");
    fixture.assert_snapshot(&before);
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(130));
    fixture.assert_snapshot(&before);
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_rejects_source_changes_after_review_without_writing_either_side() {
    let fixture = PairFixture::new("stale-pair");
    let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "setup", "gateway", "vm",
        "--vm-source", fixture.vm.to_str().unwrap(), "--vm-line", "1",
        "--transit-host", "127.0.0.1", "--transit-port", "2222"]);
    terminal.expect("Pair setup");
    terminal.send(b"\x13");
    terminal.expect("Review Pair changes");
    let external = format!("{}# external edit after review\n", fs::read_to_string(&fixture.vm).unwrap());
    fs::write(&fixture.vm, external).unwrap();
    let after_external_edit = fixture.snapshot();
    terminal.send(b"\r");
    terminal.expect("HOST_SOURCE_CHANGED");
    fixture.assert_snapshot(&after_external_edit);
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(130));
    fixture.assert_snapshot(&after_external_edit);
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_explicit_ambiguous_or_unknown_selectors_fail_before_opening_workspace() {
    let fixture = PairFixture::new("selector-pair");
    let before = fixture.snapshot();
    for (vm, code) in [("vm", "HOST_AMBIGUOUS"), ("missing-vm", "HOST_NOT_FOUND")] {
        let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "setup", "gateway", vm]);
        assert_eq!(terminal.finish(), Some(2));
        let output = String::from_utf8_lossy(&terminal.output);
        assert!(output.contains(code), "{output}");
        assert!(!terminal.output.contains(&0x1b), "explicit selector error opened a workspace: {output}");
        fixture.assert_snapshot(&before);
    }
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_no_input_reports_missing_roles_and_transit_options_without_mutation() {
    let fixture = PairFixture::new("no-input-pair");
    let before = fixture.snapshot();
    for (arguments, code, guidance) in [
        (vec!["pair", "setup", "--no-input", "--yes"], "GATEWAY_REQUIRED", "gateway"),
        (vec!["pair", "setup", "gateway", "--no-input", "--yes"], "VM_REQUIRED", "vm"),
        (vec!["pair", "setup", "gateway", "vm", "--vm-source", fixture.vm.to_str().unwrap(),
            "--vm-line", "1", "--no-input", "--yes"], "TRANSIT_REQUIRED", "--transit-host"),
    ] {
        let output = fixture.command().args(&arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(code), "{arguments:?}: {error}");
        assert!(error.to_ascii_lowercase().contains(guidance), "{arguments:?}: {error}");
        if code == "TRANSIT_REQUIRED" {
            assert!(error.contains("--transit-port"), "{error}");
        }
        assert!(!output.stderr.contains(&0x1b), "{arguments:?} opened a workspace");
        fixture.assert_snapshot(&before);
    }
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_gateway_filters_same_entry_wrong_port_unsafe_proxy_and_existing_pair() {
    let fixture = PairFixture::new("filtered-pair");
    let gateway = fs::read_to_string(&fixture.gateway).unwrap();
    fs::write(&fixture.gateway, format!("{gateway}  Port 2222\n")).unwrap();
    let mut config = fs::OpenOptions::new().append(true).open(&fixture.config).unwrap();
    config.write_all(b"Host wrong-port\n  HostName wrong.example\n  Port 22\n\
Host unsafe-command\n  HostName command.example\n  Port 2222\n  ProxyCommand nc %h %p\n\
Host unsafe-jump\n  HostName jump.example\n  Port 2222\n  ProxyJump gateway\n\
Host existing-gateway\n  HostName existing.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n\
Host existing-vm\n  HostName paired.example\n  Port 2222\n").unwrap();
    let output = fixture.command().args(["pair", "setup", "existing-gateway", "existing-vm",
        "--transit-host", "127.0.0.3", "--transit-port", "2222", "--yes", "--no-input"])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let before = fixture.snapshot();
    let existing = fixture.pairs();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway"]);
    terminal.expect("Gateway: gateway");
    terminal.expect("first-vm");
    let mut previous = 0;
    for alias in ["gateway", "wrong-port", "unsafe-command", "unsafe-jump", "existing-vm"] {
        terminal.send(&vec![127; previous]);
        terminal.send(alias.as_bytes());
        terminal.expect(&format!("search: {alias}"));
        terminal.expect("No eligible matching HostEntry aliases");
        terminal.send(b"\r");
        terminal.expect("VM: not selected");
        previous = alias.len();
    }
    terminal.send(&vec![127; previous]);
    terminal.send(b"vm");
    terminal.expect("first-vm");
    terminal.send(b"\x1b[B");
    terminal.expect("Choose: vm");
    terminal.expect("second-vm");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(130));
    fixture.assert_snapshot(&before);
    assert_eq!(fixture.pairs(), existing);
}

#[test]
fn partial_pair_setup_keeps_supplied_gateway_and_vm_visible_and_editable() {
    let fixture = PairFixture::new("editable-pair");
    let mut config = fs::OpenOptions::new().append(true).open(&fixture.config).unwrap();
    config.write_all(b"Host alternate-gateway\n  HostName alternate.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n").unwrap();
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway", "vm",
        "--vm-source", fixture.vm.to_str().unwrap(), "--vm-line", "1", "--yes"]);
    terminal.expect("Gateway: gateway");
    terminal.expect("VM: vm");
    terminal.expect("second-vm");
    terminal.send(b"\t\talternate-gateway\r");
    terminal.expect("Gateway: alternate-gateway");
    terminal.expect("VM: vm");
    terminal.expect("second-vm");
    terminal.send(b"vm\r");
    terminal.expect("first-vm");
    terminal.send(b"\x13");
    terminal.expect("Review Pair changes");
    terminal.expect("Gateway: alternate-gateway");
    terminal.expect("first-vm");
    terminal.expect("Transit: 127.0.0.3:2222");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect("Pair saved.");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    assert_eq!(pairs["pairs"][0]["gateway_alias"], "alternate-gateway");
    assert_eq!(pairs["pairs"][0]["vm_alias"], "vm");
    assert!(fs::read_to_string(&fixture.decoy).unwrap().contains("##SSHX TRANSIT=127.0.0.3:2222"));
    assert_eq!(fs::read(&fixture.gateway).unwrap(), before[1].0);
    assert_eq!(fs::read(&fixture.vm).unwrap(), before[3].0);
}

#[test]
fn partial_pair_setup_edits_incomplete_manual_transit_and_preview_never_applies() {
    let fixture = PairFixture::new("manual-preview-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway", "vm",
        "--vm-source", fixture.vm.to_str().unwrap(), "--vm-line", "1",
        "--transit-host", "127.0.0.1", "--preview", "--yes"]);
    terminal.expect("Pair setup");
    terminal.expect("Transit host: 127.0.0.1");
    terminal.expect("Transit port: (infer)");
    terminal.send(b"\x13");
    terminal.expect("TRANSIT_INCOMPLETE");
    fixture.assert_snapshot(&before);
    terminal.send(b"\x7f2\t2222");
    terminal.expect("Transit host: 127.0.0.2");
    terminal.expect("Transit port: 2222");
    terminal.send(b"\x13");
    terminal.expect("Review Pair changes");
    terminal.expect("Transit: 127.0.0.2:2222");
    terminal.expect("Preview complete");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r\x1b");
    terminal.expect_absent("Review Pair changes");
    fixture.assert_snapshot(&before);
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    fixture.assert_snapshot(&before);
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn complete_pair_setup_with_yes_and_no_input_applies_directly_without_workspace() {
    let fixture = PairFixture::new("complete-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway", "vm",
        "--vm-source", fixture.vm.to_str().unwrap(), "--vm-line", "1",
        "--transit-host", "127.0.0.2", "--transit-port", "2222", "--yes", "--no-input"]);
    assert_eq!(terminal.finish(), Some(0));
    let output = String::from_utf8_lossy(&terminal.output);
    assert!(output.contains("127.0.0.2:2222"), "{output}");
    assert!(!terminal.output.contains(&0x1b), "complete CLI opened a workspace: {output}");
    assert!(!output.contains("pair-stored-secret"), "Pair result exposed a stored password");
    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    let pair = &pairs["pairs"][0];
    assert_eq!(pair["gateway_alias"], "gateway");
    assert_eq!(pair["vm_alias"], "vm");
    assert_eq!(pair["transit_host"], "127.0.0.2");
    assert_eq!(pair["transit_port"], 2222);
    assert!(fs::read_to_string(&fixture.gateway).unwrap()
        .contains(&format!("##SSHX VM={}", pair["vm_id"].as_str().unwrap())));
    let vm = fs::read_to_string(&fixture.vm).unwrap();
    assert!(vm.contains(&format!("##SSHX GATEWAY={}", pair["gateway_id"].as_str().unwrap())));
    assert!(vm.contains("##SSHX TRANSIT=127.0.0.2:2222"));
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
    for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
        assert_eq!(fs::metadata(source).unwrap().permissions().mode() & 0o777, *mode);
    }
}

struct PairFixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    gateway: PathBuf,
    decoy: PathBuf,
    vm: PathBuf,
}

impl PairFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("sshx-{name}-{}-{}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let home = root.join("home");
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o700)).unwrap();
        let config = home.join(".ssh/config");
        let gateway = home.join(".ssh/gateway-source");
        let decoy = home.join(".ssh/first-vm");
        let vm = home.join(".ssh/second-vm");
        for (source, content) in [
            (&config, "Include gateway-source first-vm second-vm\n"),
            (&gateway, "Host gateway\n  HostName gateway.example\n  ##PASSWORD pair-stored-secret\n  LocalForward 127.0.0.1:12222 127.0.0.1:2222\n  LocalForward 127.0.0.1:12223 127.0.0.2:2222\n"),
            (&decoy, "Host vm\n  HostName decoy.example\n  Port 2222\n"),
            (&vm, "Host vm\n  HostName actual.example\n  Port 2222\n"),
        ] {
            fs::write(source, content).unwrap();
            fs::set_permissions(source, fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self { root, home, config, gateway, decoy, vm }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
        command.arg("--config").arg(&self.config).env("HOME", &self.home);
        command
    }

    fn sources(&self) -> [&Path; 4] {
        [&self.config, &self.gateway, &self.decoy, &self.vm]
    }

    fn snapshot(&self) -> Vec<(Vec<u8>, u32)> {
        self.sources().iter().map(|source| (
            fs::read(source).unwrap(),
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
        )).collect()
    }

    fn assert_snapshot(&self, before: &[(Vec<u8>, u32)]) {
        for (source, (bytes, mode)) in self.sources().iter().zip(before) {
            assert_eq!(fs::read(source).unwrap(), *bytes, "{} changed", source.display());
            assert_eq!(fs::metadata(source).unwrap().permissions().mode() & 0o777, *mode);
        }
    }

    fn pairs(&self) -> serde_json::Value {
        let output = self.command().args(["pair", "list", "--format", "json", "--no-input"]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for PairFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct PairTerminal {
    child: std::process::Child,
    master: File,
    output: Vec<u8>,
    screen: TerminalScreen,
}

impl PairTerminal {
    fn open(fixture: &PairFixture, arguments: &[&str]) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) }, 0);
        let slave = unsafe { File::from_raw_fd(slave) };
        let child = fixture.command().args(arguments)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave)).spawn().unwrap();
        let master = unsafe { File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
        Self { child, master, output: Vec::new(), screen: TerminalScreen::new() }
    }

    fn expect(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if self.screen.contains(marker) { break; }
            if Instant::now() >= deadline || self.child.try_wait().unwrap().is_some() {
                panic!("PTY did not show {marker}: {}", String::from_utf8_lossy(&self.output));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn expect_absent(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if !self.screen.contains(marker) { break; }
            assert!(Instant::now() < deadline, "PTY still shows {marker}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain(&mut self) {
        let mut buffer = [0u8; 8192];
        match self.master.read(&mut buffer) {
            Ok(size) => {
                self.output.extend_from_slice(&buffer[..size]);
                self.screen.feed(&buffer[..size]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock || error.raw_os_error() == Some(libc::EIO) => {}
            Err(error) => panic!("PTY read failed: {error}"),
        }
    }

    fn send(&mut self, input: &[u8]) {
        self.master.write_all(input).unwrap();
    }

    fn finish(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() { return status.code(); }
            assert!(Instant::now() < deadline, "PTY did not exit: {}", String::from_utf8_lossy(&self.output));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for PairTerminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Ratatui redraws only changed cells, so markers belong to the rendered screen, not each write.
struct TerminalScreen {
    cells: Vec<Vec<char>>,
    row: usize,
    column: usize,
    pending: Vec<u8>,
}

impl TerminalScreen {
    fn new() -> Self {
        Self { cells: vec![vec![' '; 120]; 40], row: 0, column: 0, pending: Vec::new() }
    }

    fn contains(&self, expected: &str) -> bool {
        let text: String = self.cells.iter().flatten().filter(|character| !character.is_whitespace()).collect();
        let expected: String = expected.chars().filter(|character| !character.is_whitespace()).collect();
        text.contains(&expected)
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index] == 0x1b {
                let Some(&next) = self.pending.get(index + 1) else { break };
                if next == b'[' {
                    let Some(end) = (index + 2..self.pending.len())
                        .find(|&position| (0x40..=0x7e).contains(&self.pending[position])) else { break };
                    let parameters = String::from_utf8_lossy(&self.pending[index + 2..end]).into_owned();
                    self.control(&parameters, self.pending[end]);
                    index = end + 1;
                } else if next == b']' {
                    let Some(end) = (index + 2..self.pending.len()).find(|&position| {
                        self.pending[position] == 7
                            || (self.pending[position] == 0x1b && self.pending.get(position + 1) == Some(&b'\\'))
                    }) else { break };
                    index = end + if self.pending[end] == 7 { 1 } else { 2 };
                } else {
                    index += 2;
                }
                continue;
            }
            let byte = self.pending[index];
            match byte {
                b'\r' => self.column = 0,
                b'\n' => self.row = (self.row + 1).min(39),
                8 => self.column = self.column.saturating_sub(1),
                b'\t' => self.column = ((self.column / 8 + 1) * 8).min(119),
                0..=31 | 127 => {}
                _ => {
                    let length = match byte {
                        0..=127 => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    if index + length > self.pending.len() { break; }
                    let character = std::str::from_utf8(&self.pending[index..index + length])
                        .expect("PTY emitted invalid UTF-8").chars().next().unwrap();
                    self.cells[self.row][self.column] = character;
                    self.column = (self.column + 1).min(119);
                    index += length;
                    continue;
                }
            }
            index += 1;
        }
        self.pending.drain(..index);
    }

    fn control(&mut self, parameters: &str, command: u8) {
        if parameters.starts_with('?') {
            if parameters == "?1049" && command == b'h' {
                for row in &mut self.cells { row.fill(' '); }
                self.row = 0;
                self.column = 0;
            }
            return;
        }
        let values: Vec<usize> = parameters.split(';')
            .map(|value| value.parse().unwrap_or(0)).collect();
        let value = values.first().copied().unwrap_or(0);
        let amount = value.max(1);
        match command {
            b'H' | b'f' => {
                self.row = value.max(1).saturating_sub(1).min(39);
                self.column = values.get(1).copied().unwrap_or(1).max(1).saturating_sub(1).min(119);
            }
            b'A' => self.row = self.row.saturating_sub(amount),
            b'B' => self.row = (self.row + amount).min(39),
            b'C' => self.column = (self.column + amount).min(119),
            b'D' => self.column = self.column.saturating_sub(amount),
            b'G' => self.column = amount.saturating_sub(1).min(119),
            b'd' => self.row = amount.saturating_sub(1).min(39),
            b'J' => {
                for row in 0..40 {
                    for column in 0..120 {
                        let position = (row, column);
                        if value == 2 || value == 3
                            || (value == 0 && position >= (self.row, self.column))
                            || (value == 1 && position <= (self.row, self.column)) {
                            self.cells[row][column] = ' ';
                        }
                    }
                }
            }
            b'K' => {
                let range = match value {
                    1 => 0..self.column + 1,
                    2 => 0..120,
                    _ => self.column..120,
                };
                self.cells[self.row][range].fill(' ');
            }
            _ => {}
        }
    }
}
