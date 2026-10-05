#![cfg(unix)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PAIR_GATEWAY_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PAIR_VM_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const PAIR_OTHER_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

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
    let interrupted = fs::read_to_string(&fixture.gateway)
        .unwrap()
        .replace("Host gateway\n", "Host interrupted-gateway\n");
    fs::write(&fixture.gateway, &interrupted).unwrap();
    fs::copy(&fixture.gateway, &after).unwrap();
    fs::write(&lock, b"stale lock file from crashed writer").unwrap();
    // The journal wire format uses FNV-1a digests of the before and after bytes.
    let digest = |bytes: &[u8]| {
        bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    };
    let journal_bytes = serde_json::to_vec(&serde_json::json!({
        "writes": [{
            "path": target,
            "before": before,
            "after": after,
            "before_digest": digest(&original[1].0),
            "after_digest": digest(interrupted.as_bytes()),
            "existed": true
        }]
    }))
    .unwrap();
    fs::write(&journal, &journal_bytes).unwrap();
    let pending = fixture.snapshot();

    let output = fixture
        .command()
        .args(["pair", "setup", "gateway", "--yes", "--no-input"])
        .output()
        .unwrap();
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

    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let mut terminal = PairTerminal::open(&fixture, &arguments);
    terminal.expect("Recover pending Pair mutation before setup?");
    terminal.send(b"y\r");
    assert_eq!(terminal.finish(), Some(2));
    assert!(String::from_utf8_lossy(&terminal.output).contains("MUTATION_BUSY"));
    fixture.assert_snapshot(&pending);
    assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    assert_eq!(fs::read(&before).unwrap(), original[1].0);
    assert_eq!(fs::read(&after).unwrap(), interrupted.as_bytes());
    drop(lock_file);

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
fn malformed_pending_pair_journal_refuses_preview_and_setup_without_mutation() {
    let fixture = PairFixture::new("malformed-journal-pair");
    let before = fixture.snapshot();
    let journal = fixture
        .gateway
        .with_file_name(".gateway-source.sshx.lock.journal");
    let journal_bytes = b"{ invalid journal";
    fs::write(&journal, journal_bytes).unwrap();
    for mode in ["--preview", "--yes"] {
        let arguments = [
            "pair",
            "setup",
            "gateway",
            "vm",
            "--vm-source",
            fixture.vm.to_str().unwrap(),
            "--vm-line",
            "1",
            "--transit-host",
            "127.0.0.1",
            "--transit-port",
            "2222",
            mode,
        ];
        let output = fixture
            .command()
            .args(arguments)
            .arg("--no-input")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{mode}: {output:?}");
        fixture.assert_snapshot(&before);
        assert_eq!(fs::read(&journal).unwrap(), journal_bytes);

        let mut terminal = PairTerminal::open(&fixture, &arguments);
        assert_eq!(terminal.finish(), Some(2), "{mode}");
        fixture.assert_snapshot(&before);
        assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    }
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
    assert!(
        !String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"),
        "Pair review exposed a stored password"
    );
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect("Pair saved.");
    assert!(
        !String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"),
        "Pair result exposed a stored password"
    );
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
    assert!(vm.contains(&format!(
        "##SSHX GATEWAY={}",
        pair["gateway_id"].as_str().unwrap()
    )));
    assert!(vm.contains("##SSHX TRANSIT=127.0.0.2:2222"));
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
}

#[test]
fn pairs_tab_reviews_exact_secondary_alias_sources_then_refreshes_applied_record() {
    let fixture = PairFixture::new("tab-exact-pair");
    let first_gateway = fixture.home.join(".ssh/first-gateway");
    let first_gateway_bytes = b"Host gateway gw\n  HostName decoy-gateway.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n";
    fs::write(&first_gateway, first_gateway_bytes).unwrap();
    fs::set_permissions(&first_gateway, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(
        &fixture.config,
        "Include first-gateway gateway-source first-vm second-vm\n",
    )
    .unwrap();
    for (source, original, aliases) in [
        (&fixture.gateway, "Host gateway\n", "Host gateway gw\n"),
        (&fixture.decoy, "Host vm\n", "Host vm machine\n"),
        (&fixture.vm, "Host vm\n", "Host vm machine\n"),
    ] {
        let bytes = fs::read_to_string(source)
            .unwrap()
            .replace(original, aliases);
        fs::write(source, bytes).unwrap();
    }
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(&fixture, &[]);
    terminal.expect("sshx Hosts");
    terminal.send(b"\x10");
    terminal.expect("No valid Pair relationships.");
    terminal.send(b"s");
    terminal.expect("Pair setup");
    terminal.send(b"gw\x1b[B");
    terminal.expect("gateway-source");
    terminal.send(b"\r");
    terminal.expect("Gateway: gw");
    terminal.send(b"machine\x1b[B");
    terminal.expect("second-vm");
    terminal.send(b"\r\x13");
    terminal.expect("TRANSIT_REQUIRED");
    fixture.assert_snapshot(&before);
    terminal.send(b"\x0e\x0e\x13");
    terminal.expect("Review Pair changes");
    terminal.expect("Gateway: gw");
    terminal.expect("VM: machine");
    terminal.expect("Transit: 127.0.0.2:2222");
    fixture.assert_snapshot(&before);
    assert_eq!(fs::read(&first_gateway).unwrap(), first_gateway_bytes);
    terminal.send(b"\x1b");
    terminal.expect_absent("Review Pair changes");
    fixture.assert_snapshot(&before);
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
    terminal.send(b"\x13");
    terminal.expect("Review Pair changes");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect("Pair saved.");
    terminal.send(b"\x1b");
    terminal.expect("Pair setup complete.");
    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    let pair = &pairs["pairs"][0];
    terminal.expect(pair["gateway_id"].as_str().unwrap());
    terminal.expect(pair["vm_id"].as_str().unwrap());
    assert_eq!(pair["transit_host"], "127.0.0.2");
    assert_eq!(pair["transit_port"], 2222);
    let gateway = fs::read_to_string(&fixture.gateway).unwrap();
    let vm = fs::read_to_string(&fixture.vm).unwrap();
    assert!(gateway.contains("Host gateway gw\n"));
    assert!(vm.contains("Host vm machine\n"));
    assert!(gateway.contains(&format!("##SSHX VM={}", pair["vm_id"].as_str().unwrap())));
    assert!(vm.contains(&format!(
        "##SSHX GATEWAY={}",
        pair["gateway_id"].as_str().unwrap()
    )));
    assert!(vm.contains("##SSHX TRANSIT=127.0.0.2:2222"));
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
    assert_eq!(fs::read(&first_gateway).unwrap(), first_gateway_bytes);
    for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
        assert_eq!(
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
            *mode
        );
    }
    terminal.send(b"\x1b");
    terminal.expect("sshx Hosts");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn pair_setup_review_and_interrupt_cancellation_preserve_all_sources() {
    let fixture = PairFixture::new("cancel-pair");
    let before = fixture.snapshot();
    let vm_source = fixture.vm.to_str().unwrap();
    let arguments = [
        "tui",
        "pair",
        "setup",
        "gateway",
        "vm",
        "--vm-source",
        vm_source,
        "--vm-line",
        "1",
        "--transit-host",
        "127.0.0.1",
        "--transit-port",
        "2222",
        "--yes",
    ];
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
    let mut terminal = PairTerminal::open(
        &fixture,
        &[
            "tui",
            "pair",
            "setup",
            "gateway",
            "vm",
            "--vm-source",
            fixture.vm.to_str().unwrap(),
            "--vm-line",
            "1",
            "--transit-host",
            "127.0.0.1",
            "--transit-port",
            "0",
        ],
    );
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
fn pair_setup_keeps_exact_vm_selectable_without_matching_gateway_transit() {
    for (case, forward) in [
        ("missing-forward", ""),
        (
            "wrong-forward-port",
            "  LocalForward 127.0.0.1:12222 127.0.0.1:2200\n",
        ),
    ] {
        let fixture = PairFixture::new(case);
        fs::write(
            &fixture.config,
            "Include gateway-source first-vm second-vm\n",
        )
        .unwrap();
        fs::write(&fixture.gateway, format!(
            "Host pdms-faq-chatbot\n  HostName gateway.example\n  ##PASSWORD pair-stored-secret\n{forward}",
        )).unwrap();
        fs::write(
            &fixture.decoy,
            format!(
                "##SSHX ID={PAIR_OTHER_ID}\nHost pdms-faq-chatbot-vm\n  HostName decoy.example\n  Port 2222\n"
            ),
        )
        .unwrap();
        fs::write(
            &fixture.vm,
            format!(
                "##SSHX ID={PAIR_VM_ID}\nHost pdms-faq-chatbot-vm\n  HostName actual.example\n  Port 2222\n"
            ),
        )
        .unwrap();
        let before = fixture.snapshot();
        let vm = sshx::discovery::discover(&fixture.vm)
            .unwrap()
            .entries
            .remove(0);
        let mut terminal = PairTerminal::open_sized(&fixture, &["tui", "pair", "setup"], 120, 40);
        terminal.expect("Choose: pdms-faq-chatbot");
        terminal.send(b"pdms-faq-chatbot\r");
        terminal.expect("Gateway: pdms-faq-chatbot");
        terminal.send(b"pdms-faq-chatbot-vm");
        terminal.expect("Search: pdms-faq-chatbot-vm");
        terminal.expect("Choose: pdms-faq-chatbot-vm");
        terminal.expect(PAIR_OTHER_ID);
        terminal.send(b"\x1b[B");
        terminal.expect(&vm.id);
        terminal.send(b"\r");
        terminal.expect("VM: pdms-faq-chatbot-vm");
        terminal.expect(&vm.id);
        terminal.send(b"\x13");
        terminal.expect("TRANSIT_REQUIRED");
        fixture.assert_snapshot(&before);
        terminal.send(b"127.0.0.9\t2222\x13");
        terminal.expect("TRANSIT_MISMATCH");
        terminal.expect("VM: pdms-faq-chatbot-vm");
        terminal.expect(&vm.id);
        fixture.assert_snapshot(&before);
        terminal.send(b"\x1b");
        assert_eq!(terminal.finish(), Some(130));
        fixture.assert_snapshot(&before);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
    }
}

#[test]
fn pair_setup_infers_remote_transit_for_exact_loopback_vm_listener() {
    let fixture = PairFixture::new("loopback-listener-pair");
    fs::write(&fixture.config, "Include gateway-source second-vm\n").unwrap();
    fs::write(&fixture.gateway,
        "Host pdms-faq-chatbot\n  HostName gateway.example\n  LocalForward 2222 192.168.100.130:22\n").unwrap();
    fs::write(
        &fixture.vm,
        "Host pdms-faq-chatbot-vm\n  HostName 127.0.0.1\n  Port 2222\n",
    )
    .unwrap();
    let before = fixture.snapshot();
    let vm = sshx::discovery::discover(&fixture.vm)
        .unwrap()
        .entries
        .remove(0);
    let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "setup"]);
    terminal.expect("Pair setup");
    terminal.send(b"pdms-faq-chatbot\r");
    terminal.expect("Gateway: pdms-faq-chatbot");
    terminal.send(b"pdms-faq-chatbot-vm");
    terminal.expect("Choose: pdms-faq-chatbot-vm");
    terminal.expect(&format!("second-vm:{}", vm.source.line_start));
    terminal.send(b"\r\x13");
    terminal.expect("Review Pair changes");
    terminal.expect("VM: pdms-faq-chatbot-vm");
    terminal.expect(&format!("second-vm:{}", vm.source.line_start));
    terminal.expect("Transit: 192.168.100.130:22");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect("Pair saved.");
    terminal.send(b"\x1b");
    assert_eq!(terminal.finish(), Some(0));
    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    let pair = &pairs["pairs"][0];
    assert_eq!(pair["gateway_alias"], "pdms-faq-chatbot");
    assert_eq!(pair["vm_alias"], "pdms-faq-chatbot-vm");
    assert_eq!(pair["transit_host"], "192.168.100.130");
    assert_eq!(pair["transit_port"], 22);
    let entries = [&fixture.gateway, &fixture.vm]
        .into_iter()
        .flat_map(|source| sshx::discovery::discover(source).unwrap().entries)
        .collect::<Vec<_>>();
    let selected_vm = entries
        .iter()
        .find(|entry| entry.id == pair["vm_id"].as_str().unwrap())
        .unwrap();
    let route = sshx::pair::paired_route(&entries, selected_vm)
        .unwrap()
        .unwrap();
    assert_eq!(route.gateway_id, pair["gateway_id"].as_str().unwrap());
    assert_eq!(route.vm_id, pair["vm_id"].as_str().unwrap());
    assert_eq!(route.transit_host, "192.168.100.130");
    assert_eq!(route.transit_port, 22);
    let diagnostics = sshx::pair::diagnostics(&entries);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for (source, field, original) in [
        (&fixture.gateway, "gateway_id", &before[1].0),
        (&fixture.vm, "vm_id", &before[3].0),
    ] {
        let entries = sshx::discovery::discover(source).unwrap().entries;
        assert_eq!(pair[field], entries[0].id);
        let content = fs::read_to_string(source).unwrap();
        let directives = content
            .lines()
            .filter(|line| !line.trim_start().starts_with("##SSHX"))
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        assert_eq!(directives.as_bytes(), original);
    }
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
    for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
        assert_eq!(
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
            *mode
        );
    }
}

#[test]
fn pair_setup_explicit_loopback_transit_rejects_duplicate_destinations_and_unmapped_route_changes()
{
    let fixture = PairFixture::new("loopback-route-safety");
    fs::write(&fixture.config, "Include gateway-source second-vm\n").unwrap();
    fs::write(
        &fixture.gateway,
        "Host gateway\n  HostName gateway.example\n  LocalForward 2222 192.168.100.130:22\n",
    )
    .unwrap();
    fs::write(&fixture.vm, "Host vm\n  HostName 127.0.0.1\n  Port 2222\n").unwrap();
    let before = fixture.snapshot();
    let discover = || {
        [&fixture.gateway, &fixture.vm]
            .into_iter()
            .flat_map(|source| sshx::discovery::discover(source).unwrap().entries)
            .collect::<Vec<_>>()
    };
    let gateway = fs::read_to_string(&fixture.gateway).unwrap();
    for (listener, vm_host, accepted) in [
        ("2222", "127.0.0.1", true),
        ("127.0.0.1:2222", "127.0.0.1", true),
        ("[::1]:2222", "::1", true),
        ("127.0.0.2:2222", "127.0.0.1", false),
        ("0.0.0.0:2222", "::1", false),
    ] {
        fs::write(
            &fixture.gateway,
            gateway.replace("LocalForward 2222 ", &format!("LocalForward {listener} ")),
        )
        .unwrap();
        fs::write(
            &fixture.vm,
            format!("Host vm\n  HostName {vm_host}\n  Port 2222\n"),
        )
        .unwrap();
        let binding = fixture.snapshot();
        let entries = discover();
        let result = sshx::pair::plan_setup(
            &entries,
            &entries[0],
            &entries[1],
            "gateway",
            "vm",
            None,
            None,
        );
        if accepted {
            let plan = result.unwrap_or_else(|error| panic!("{listener} / {vm_host}: {error}"));
            assert_eq!(plan.transit_host, "192.168.100.130");
            assert_eq!(plan.transit_port, 22);
        } else {
            let error = result.unwrap_err();
            assert!(
                error.starts_with("TRANSIT_REQUIRED:"),
                "{listener} / {vm_host}: {error}"
            );
        }
        fixture.assert_snapshot(&binding);
    }
    fs::write(&fixture.vm, &before[3].0).unwrap();
    fs::write(
        &fixture.gateway,
        format!("{gateway}  LocalForward 2222 192.168.100.130:22\n"),
    )
    .unwrap();
    let duplicate = fixture.snapshot();
    let entries = discover();
    let error = sshx::pair::plan_setup(
        &entries,
        &entries[0],
        &entries[1],
        "gateway",
        "vm",
        Some("192.168.100.130"),
        Some(22),
    )
    .unwrap_err();
    assert!(error.starts_with("TRANSIT_MISMATCH:"), "{error}");
    fixture.assert_snapshot(&duplicate);
    fs::write(&fixture.gateway, &before[1].0).unwrap();
    let entries = discover();
    let plan = sshx::pair::plan_setup(
        &entries,
        &entries[0],
        &entries[1],
        "gateway",
        "vm",
        Some("192.168.100.130"),
        Some(22),
    )
    .unwrap();
    assert_eq!(plan.transit_host, "192.168.100.130");
    assert_eq!(plan.transit_port, 22);
    fixture.assert_snapshot(&before);
    sshx::mutation::apply_pair(&plan).unwrap();
    assert!(fs::read(&fixture.gateway).unwrap().ends_with(&before[1].0));
    assert!(fs::read(&fixture.vm).unwrap().ends_with(&before[3].0));
    let entries = discover();
    let route = sshx::pair::paired_route(&entries, &entries[1])
        .unwrap()
        .unwrap();
    assert_eq!(route.transit_host, "192.168.100.130");
    assert_eq!(route.transit_port, 22);
    let applied = fixture.snapshot();
    for (source, original, old, changed) in [
        (
            &fixture.gateway,
            &applied[1].0,
            "LocalForward 2222 ",
            "LocalForward 2223 ",
        ),
        (&fixture.vm, &applied[3].0, "Port 2222\n", "Port 2223\n"),
    ] {
        let changed_bytes = String::from_utf8(original.clone())
            .unwrap()
            .replace(old, changed);
        fs::write(source, changed_bytes).unwrap();
        let edited = fixture.snapshot();
        let entries = discover();
        let error = sshx::pair::paired_route(&entries, &entries[1]).unwrap_err();
        assert!(error.starts_with("PAIR_ROUTE_CHANGED:"), "{error}");
        assert!(
            sshx::pair::diagnostics(&entries)
                .iter()
                .any(|diagnostic| diagnostic.code == "PAIR_ROUTE_CHANGED")
        );
        fixture.assert_snapshot(&edited);
        fs::write(source, original).unwrap();
    }
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
    for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
        assert_eq!(
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
            *mode
        );
    }
}

#[test]
fn pair_setup_rejects_source_changes_before_and_after_review_and_keeps_values_editable() {
    for after_review in [false, true] {
        let fixture = PairFixture::new("stale-pair");
        let mut terminal = PairTerminal::open(
            &fixture,
            &[
                "tui",
                "pair",
                "setup",
                "gateway",
                "vm",
                "--vm-source",
                fixture.vm.to_str().unwrap(),
                "--vm-line",
                "1",
                "--transit-host",
                "127.0.0.1",
                "--transit-port",
                "2222",
            ],
        );
        terminal.expect("Pair setup");
        if after_review {
            terminal.send(b"\x13");
            terminal.expect("Review Pair changes");
        }
        let external = format!(
            "{}# external edit\n",
            fs::read_to_string(&fixture.vm).unwrap()
        );
        fs::write(&fixture.vm, external).unwrap();
        let after_external_edit = fixture.snapshot();
        terminal.send(if after_review { b"\r" } else { b"\x13" });
        terminal.expect("HOST_SOURCE_CHANGED");
        terminal.expect("Gateway: gateway");
        terminal.expect("VM: vm");
        terminal.expect("second-vm");
        terminal.expect("Transit host: 127.0.0.1");
        terminal.expect("Transit port: 2222");
        fixture.assert_snapshot(&after_external_edit);
        terminal.send(b"\x0e\x0e");
        terminal.expect("Transit host: 127.0.0.2");
        terminal.send(b"\x13");
        terminal.expect("HOST_SOURCE_CHANGED");
        fixture.assert_snapshot(&after_external_edit);
        terminal.send(b"\x1b");
        assert_eq!(terminal.finish(), Some(130));
        fixture.assert_snapshot(&after_external_edit);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
    }
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
        assert!(
            !terminal.output.contains(&0x1b),
            "explicit selector error opened a workspace: {output}"
        );
        fixture.assert_snapshot(&before);
    }
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_no_input_reports_missing_roles_and_transit_options_without_mutation() {
    let fixture = PairFixture::new("no-input-pair");
    let before = fixture.snapshot();
    for (arguments, code, guidance) in [
        (
            vec!["pair", "setup", "--no-input", "--yes"],
            "GATEWAY_REQUIRED",
            "gateway",
        ),
        (
            vec!["pair", "setup", "gateway", "--no-input", "--yes"],
            "VM_REQUIRED",
            "vm",
        ),
        (
            vec![
                "pair",
                "setup",
                "gateway",
                "vm",
                "--vm-source",
                fixture.vm.to_str().unwrap(),
                "--vm-line",
                "1",
                "--no-input",
                "--yes",
            ],
            "TRANSIT_REQUIRED",
            "--transit-host",
        ),
    ] {
        let output = fixture.command().args(&arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(code), "{arguments:?}: {error}");
        assert!(
            error.to_ascii_lowercase().contains(guidance),
            "{arguments:?}: {error}"
        );
        if code == "TRANSIT_REQUIRED" {
            assert!(error.contains("--transit-port"), "{error}");
        }
        assert!(
            !output.stderr.contains(&0x1b),
            "{arguments:?} opened a workspace"
        );
        fixture.assert_snapshot(&before);
    }
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
}

#[test]
fn pair_setup_gateway_keeps_wrong_port_selectable_and_filters_same_entry_proxy_and_existing_pair() {
    let fixture = PairFixture::new("filtered-pair");
    let gateway = fs::read_to_string(&fixture.gateway).unwrap();
    fs::write(&fixture.gateway, format!("{gateway}  Port 2222\n")).unwrap();
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(&fixture.config)
        .unwrap();
    config
        .write_all(
            b"Host wrong-port\n  HostName wrong.example\n  Port 22\n\
Host unsafe-command\n  HostName command.example\n  Port 2222\n  ProxyCommand nc %h %p\n\
Host unsafe-jump\n  HostName jump.example\n  Port 2222\n  ProxyJump gateway\n\
Host existing-gateway\n  HostName existing.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n\
Host existing-vm\n  HostName paired.example\n  Port 2222\n",
        )
        .unwrap();
    let output = fixture
        .command()
        .args([
            "pair",
            "setup",
            "existing-gateway",
            "existing-vm",
            "--transit-host",
            "127.0.0.3",
            "--transit-port",
            "2222",
            "--yes",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let before = fixture.snapshot();
    let existing = fixture.pairs();
    let mut terminal = PairTerminal::open(&fixture, &["pair", "setup", "gateway"]);
    terminal.expect("Gateway: gateway");
    terminal.expect("first-vm");
    let mut previous = 0;
    for alias in ["gateway", "unsafe-command", "unsafe-jump", "existing-vm"] {
        terminal.send(&vec![127; previous]);
        terminal.send(alias.as_bytes());
        terminal.expect(&format!("Search: {alias}"));
        terminal.expect("No eligible matching HostEntry aliases");
        terminal.send(b"\r");
        terminal.expect("VM: not selected");
        previous = alias.len();
    }
    terminal.send(&vec![127; previous]);
    terminal.send(b"wrong-port");
    terminal.expect("Choose: wrong-port");
    terminal.expect("config:2");
    terminal.send(&vec![127; "wrong-port".len()]);
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
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(&fixture.config)
        .unwrap();
    config.write_all(b"Host alternate-gateway\n  HostName alternate.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n").unwrap();
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(
        &fixture,
        &[
            "pair",
            "setup",
            "gateway",
            "vm",
            "--vm-source",
            fixture.vm.to_str().unwrap(),
            "--vm-line",
            "1",
            "--yes",
        ],
    );
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
    assert!(
        fs::read_to_string(&fixture.decoy)
            .unwrap()
            .contains("##SSHX TRANSIT=127.0.0.3:2222")
    );
    assert_eq!(fs::read(&fixture.gateway).unwrap(), before[1].0);
    assert_eq!(fs::read(&fixture.vm).unwrap(), before[3].0);
}

#[test]
fn partial_pair_setup_edits_incomplete_manual_transit_and_preview_never_applies() {
    let fixture = PairFixture::new("manual-preview-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(
        &fixture,
        &[
            "pair",
            "setup",
            "gateway",
            "vm",
            "--vm-source",
            fixture.vm.to_str().unwrap(),
            "--vm-line",
            "1",
            "--transit-host",
            "127.0.0.1",
            "--preview",
            "--yes",
        ],
    );
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
    let mut terminal = PairTerminal::open(
        &fixture,
        &[
            "pair",
            "setup",
            "gateway",
            "vm",
            "--vm-source",
            fixture.vm.to_str().unwrap(),
            "--vm-line",
            "1",
            "--transit-host",
            "127.0.0.2",
            "--transit-port",
            "2222",
            "--yes",
            "--no-input",
        ],
    );
    assert_eq!(terminal.finish(), Some(0));
    let output = String::from_utf8_lossy(&terminal.output);
    assert!(output.contains("127.0.0.2:2222"), "{output}");
    assert!(
        !terminal.output.contains(&0x1b),
        "complete CLI opened a workspace: {output}"
    );
    assert!(
        !output.contains("pair-stored-secret"),
        "Pair result exposed a stored password"
    );
    let pairs = fixture.pairs();
    assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
    let pair = &pairs["pairs"][0];
    assert_eq!(pair["gateway_alias"], "gateway");
    assert_eq!(pair["vm_alias"], "vm");
    assert_eq!(pair["transit_host"], "127.0.0.2");
    assert_eq!(pair["transit_port"], 2222);
    assert!(
        fs::read_to_string(&fixture.gateway)
            .unwrap()
            .contains(&format!("##SSHX VM={}", pair["vm_id"].as_str().unwrap()))
    );
    let vm = fs::read_to_string(&fixture.vm).unwrap();
    assert!(vm.contains(&format!(
        "##SSHX GATEWAY={}",
        pair["gateway_id"].as_str().unwrap()
    )));
    assert!(vm.contains("##SSHX TRANSIT=127.0.0.2:2222"));
    assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
    for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
        assert_eq!(
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
            *mode
        );
    }
}

#[test]
fn pair_setup_sectioned_form_preserves_choices_across_sizes() {
    for (width, height) in [(120, 40), (80, 20), (60, 16), (32, 10)] {
        let fixture = PairFixture::new("sectioned-pair");
        let first_gateway = fixture.home.join(".ssh/first-gateway");
        let first_gateway_bytes = b"Host gateway gw\n  HostName decoy-gateway.example\n  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n";
        fs::write(&first_gateway, first_gateway_bytes).unwrap();
        fs::set_permissions(&first_gateway, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(
            &fixture.config,
            "Include first-gateway gateway-source first-vm second-vm\n",
        )
        .unwrap();
        for (source, original, aliases, id) in [
            (
                &fixture.gateway,
                "Host gateway\n",
                "Host gateway gw\n",
                PAIR_GATEWAY_ID,
            ),
            (
                &fixture.decoy,
                "Host vm\n",
                "Host vm machine\n",
                PAIR_OTHER_ID,
            ),
            (&fixture.vm, "Host vm\n", "Host vm machine\n", PAIR_VM_ID),
        ] {
            let body = fs::read_to_string(source)
                .unwrap()
                .replace(original, aliases);
            fs::write(source, format!("##SSHX ID={id}\n{body}")).unwrap();
        }
        let before = fixture.snapshot();
        // Read direct sources: discovery of relative Includes must not use the process HOME.
        let gateway_id = sshx::discovery::discover(&fixture.gateway).unwrap().entries[0]
            .id
            .clone();
        let vm_id = sshx::discovery::discover(&fixture.vm).unwrap().entries[0]
            .id
            .clone();
        for preview in [true, false] {
            let mut arguments = vec!["tui", "pair", "setup"];
            if preview {
                arguments.push("--preview");
            }
            let mut terminal = PairTerminal::open_sized(&fixture, &arguments, width, height);
            terminal.expect("Pair setup");
            terminal.send(b"gw");
            terminal.expect("Search: gw");
            terminal.send(b"\x1b[B");
            // Repeated ID suffixes fit even a one-row context viewport.
            terminal.expect_scrolled("aaaaaaaa");
            terminal.send(b"\r");
            terminal.expect("Gateway: gw");
            terminal.send(b"\x1b[Z-missing");
            terminal.expect("Search: gw-missing");
            terminal.expect_scrolled("No eligible matching");
            terminal.expect("Gateway: gw");
            terminal.send(b"\r\tmachine");
            terminal.expect("Search: machine");
            terminal.send(b"\x1b[B");
            terminal.expect_scrolled("bbbbbbbb");
            terminal.send(b"\r\x1b[Z");
            terminal.expect("VM: machine");
            terminal.send(b"-missing");
            terminal.expect("Search: machine-missing");
            terminal.expect_scrolled("No eligible matching");
            terminal.expect("VM: machine");
            terminal.send(b"\r\t\x13");
            terminal.expect("TRANSIT_REQUIRED");
            fixture.assert_snapshot(&before);
            terminal.send(b"\x0e");
            terminal.expect("Transit host: 127.0.0.1");
            terminal.send(b"\x0e");
            terminal.expect("Transit host: 127.0.0.2");
            terminal.send(b"\x7f3");
            terminal.expect("Transit host: 127.0.0.3");
            terminal.send(b"\x7f2\t");
            terminal.expect("Transit port: 2222");
            terminal.send(b"\x7f");
            terminal.expect_absent("Transit port: 2222");
            terminal.expect("Transit port: 222");
            terminal.send(b"2");
            terminal.expect("Transit port: 2222");
            terminal.send(b"\x13");
            terminal.expect("Review Pair changes");
            terminal.expect_scrolled("Gateway: gw");
            terminal.expect_scrolled("VM: machine");
            terminal.expect_scrolled("Transit: 127.0.0.2:2222");
            fixture.assert_snapshot(&before);
            terminal.send(b"\x1b");
            terminal.expect_absent("Review Pair changes");
            terminal.expect("> Transit port: 2222");
            terminal.send(b"\x1b[Z");
            terminal.expect("> Transit host: 127.0.0.2");
            terminal.send(b"\t\x13");
            terminal.expect("Review Pair changes");
            terminal.expect_scrolled("Transit: 127.0.0.2:2222");
            fixture.assert_snapshot(&before);
            terminal.send(b"\r");
            if preview {
                terminal.send(b"\x1b");
                terminal.expect("> Transit port: 2222");
                fixture.assert_snapshot(&before);
                assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
            } else {
                terminal.expect("Pair saved.");
            }
            terminal.send(b"\x1b");
            assert_eq!(terminal.finish(), Some(0));
            assert_eq!(fs::read(&first_gateway).unwrap(), first_gateway_bytes);
            assert_eq!(
                fs::metadata(&first_gateway).unwrap().permissions().mode() & 0o777,
                0o600
            );
            if preview {
                fixture.assert_snapshot(&before);
            }
        }
        let pairs = fixture.pairs();
        assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
        let pair = &pairs["pairs"][0];
        assert_eq!(pair["gateway_id"], gateway_id);
        assert_eq!(pair["vm_id"], vm_id);
        assert_eq!(pair["transit_host"], "127.0.0.2");
        assert_eq!(pair["transit_port"], 2222);
        assert_eq!(fs::read(&fixture.config).unwrap(), before[0].0);
        assert_eq!(fs::read(&fixture.decoy).unwrap(), before[2].0);
        for (source, (_, mode)) in fixture.sources().iter().zip(&before) {
            assert_eq!(
                fs::metadata(source).unwrap().permissions().mode() & 0o777,
                *mode
            );
        }
    }
    let fixture = PairFixture::new("below-floor-pair");
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open_sized(&fixture, &["tui", "pair", "setup"], 20, 6);
    terminal.expect("Pair setup");
    terminal.send(b"\x03");
    assert_eq!(terminal.finish(), Some(130));
    fixture.assert_snapshot(&before);
}

#[test]
fn pairs_tab_delete_cancels_then_unlinks_exact_relationship() {
    for (width, height) in [(120, 40), (32, 10)] {
        let fixture = PairFixture::new("tab-delete-pair");
        let pair = fixture.setup_exact_pair();
        let paired = fixture.snapshot();
        let gateway_id = pair["gateway_id"].as_str().unwrap();
        let vm_id = pair["vm_id"].as_str().unwrap();
        let mut expected = paired.clone();
        expected[1].0 = String::from_utf8(paired[1].0.clone())
            .unwrap()
            .replace(&format!("##SSHX VM={vm_id}\n"), "")
            .into_bytes();
        expected[3].0 = String::from_utf8(paired[3].0.clone())
            .unwrap()
            .replace(&format!("##SSHX GATEWAY={gateway_id}\n"), "")
            .replace("##SSHX TRANSIT=127.0.0.2:2222\n", "")
            .into_bytes();
        let mut terminal =
            PairTerminal::open_sized(&fixture, &["tui", "pair", "list"], width, height);
        terminal.expect("D delete");
        terminal.send(b"D");
        terminal.expect("Review Pair deletion");
        terminal.expect("Enter delete");
        terminal.expect("cancel");
        if width >= 80 {
            terminal.expect(gateway_id);
            terminal.expect(vm_id);
            terminal.expect(fixture.gateway.to_str().unwrap());
            terminal.expect(fixture.vm.to_str().unwrap());
            terminal.expect("127.0.0.2:2222");
        }
        assert!(!String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"));
        fixture.assert_snapshot(&paired);
        terminal.send(b"\x1b");
        terminal.expect_absent("Review Pair deletion");
        terminal.expect("Pair deletion cancelled.");
        fixture.assert_snapshot(&paired);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([pair.clone()]));

        terminal.send(b"d");
        terminal.expect("Review Pair deletion");
        terminal.expect("Enter delete");
        terminal.send(b"\r");
        terminal.expect_absent("Review Pair deletion");
        terminal.expect("Pair deleted.");
        terminal.expect("No exact reciprocal Pair");
        fixture.assert_snapshot(&expected);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
        assert!(!String::from_utf8_lossy(&terminal.output).contains("pair-stored-secret"));
        terminal.leave_pairs();

        for (source, id) in [(&fixture.gateway, gateway_id), (&fixture.vm, vm_id)] {
            let entries = sshx::discovery::discover(source).unwrap().entries;
            assert!(entries.iter().any(|entry| entry.id == id));
        }
        assert_eq!(fixture.setup_exact_pair(), pair);
        assert_eq!(fs::read(&fixture.config).unwrap(), paired[0].0);
        assert_eq!(fs::read(&fixture.decoy).unwrap(), paired[2].0);
        for (source, (_, mode)) in fixture.sources().iter().zip(&paired) {
            assert_eq!(
                fs::metadata(source).unwrap().permissions().mode() & 0o777,
                *mode
            );
        }
    }
}

#[test]
fn pairs_tab_delete_rechecks_safety_after_review() {
    let fixture = PairFixture::new("registry-tab-delete-pair");
    let pair = fixture.setup_exact_pair();
    let paired = fixture.snapshot();
    let registry = fixture.home.join(".config/sshx/tunnels/registry.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    for state in ["active", "starting", "stopping"] {
        for endpoint in ["gateway_id", "vm_id"] {
            for role in ["direct", "gateway", "vm"] {
                for upper_case in [false, true] {
                    let id = pair[endpoint].as_str().unwrap();
                    let id = if upper_case {
                        id.to_ascii_uppercase()
                    } else {
                        id.to_owned()
                    };
                    let record = match role {
                        "gateway" => serde_json::json!({"state": state, "entry_id": PAIR_OTHER_ID,
                            "pair": {"gateway_entry_id": id, "vm_entry_id": PAIR_OTHER_ID}}),
                        "vm" => serde_json::json!({"state": state, "entry_id": PAIR_OTHER_ID,
                            "pair": {"gateway_entry_id": PAIR_OTHER_ID, "vm_entry_id": id}}),
                        _ => serde_json::json!({"state": state, "entry_id": id}),
                    };
                    let bytes = serde_json::json!({"version": 1, "tunnels": [record]}).to_string();
                    fs::write(&registry, &bytes).unwrap();
                    assert_pair_delete_blocked(&fixture, "DELETE_ACTIVE");
                    fixture.assert_snapshot(&paired);
                    assert_eq!(
                        fs::read_to_string(&registry).unwrap(),
                        bytes,
                        "{state} {endpoint} {role} uppercase={upper_case}"
                    );
                }
            }
        }
    }
    for invalid in [
        "{malformed",
        "{}",
        r#"{"version":2,"tunnels":[]}"#,
        r#"{"version":1,"tunnels":[{"entry_id":"unknown"}]}"#,
    ] {
        fs::write(&registry, invalid).unwrap();
        assert_pair_delete_blocked(&fixture, "DELETE_REGISTRY_INVALID");
        fixture.assert_snapshot(&paired);
        assert_eq!(fs::read_to_string(&registry).unwrap(), invalid);
    }
    fs::remove_file(&registry).unwrap();
    let registry_target = fixture.root.join("registry-target");
    let target_bytes = br#"{"version":1,"tunnels":[]}"#;
    fs::write(&registry_target, target_bytes).unwrap();
    std::os::unix::fs::symlink(&registry_target, &registry).unwrap();
    assert_pair_delete_blocked(&fixture, "DELETE_REGISTRY_INVALID");
    fixture.assert_snapshot(&paired);
    assert_eq!(fs::read_link(&registry).unwrap(), registry_target);
    assert_eq!(fs::read(&registry_target).unwrap(), target_bytes);
    fs::remove_file(&registry).unwrap();

    // Stopped endpoint references and active unrelated records do not block deletion.
    let allowed = serde_json::json!({"version": 1, "tunnels": [
        {"state": "stopped", "entry_id": pair["gateway_id"],
         "pair": {"gateway_entry_id": pair["gateway_id"], "vm_entry_id": pair["vm_id"]}},
        {"state": "stopped", "entry_id": pair["vm_id"]},
        {"state": "active", "entry_id": PAIR_OTHER_ID,
         "pair": {"gateway_entry_id": PAIR_OTHER_ID, "vm_entry_id": PAIR_OTHER_ID}}
    ]})
    .to_string();
    fs::write(&registry, &allowed).unwrap();
    let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
    terminal.expect("D delete");
    terminal.send(b"d");
    terminal.expect("Review Pair deletion");
    terminal.send(b"\r");
    terminal.expect("Pair deleted.");
    assert_eq!(fixture.pairs()["pairs"], serde_json::json!([]));
    assert_eq!(fs::read_to_string(&registry).unwrap(), allowed);
    terminal.leave_pairs();
    assert_eq!(fixture.setup_exact_pair(), pair);
    assert_eq!(fs::read(&fixture.config).unwrap(), paired[0].0);
    assert_eq!(fs::read(&fixture.decoy).unwrap(), paired[2].0);
    for (source, (_, mode)) in fixture.sources().iter().zip(&paired) {
        assert_eq!(
            fs::metadata(source).unwrap().permissions().mode() & 0o777,
            *mode
        );
    }
    let paired = fixture.snapshot();

    for endpoint in ["gateway_id", "vm_id"] {
        fs::write(&registry, &allowed).unwrap();
        let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
        terminal.expect("D delete");
        terminal.send(b"d");
        terminal.expect("Review Pair deletion");
        let active = serde_json::json!({"version": 1, "tunnels": [
            {"state": "starting", "entry_id": pair[endpoint].as_str().unwrap().to_ascii_uppercase()}
        ]})
        .to_string();
        fs::write(&registry, &active).unwrap();
        terminal.send(b"\r");
        terminal.expect_absent("Review Pair deletion");
        terminal.expect("DELETE_ACTIVE");
        fixture.assert_snapshot(&paired);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([pair.clone()]));
        assert_eq!(fs::read_to_string(&registry).unwrap(), active);
        terminal.leave_pairs();
    }

    for source_role in [1, 3] {
        let fixture = PairFixture::new("source-change-tab-delete-pair");
        let pair = fixture.setup_exact_pair();
        let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
        terminal.expect("D delete");
        terminal.send(b"d");
        terminal.expect("Review Pair deletion");
        let source = fixture.sources()[source_role];
        let changed = format!(
            "{}# external source edit\n",
            fs::read_to_string(source).unwrap()
        );
        fs::write(source, changed).unwrap();
        let edited = fixture.snapshot();
        terminal.send(b"\r");
        terminal.expect_absent("Review Pair deletion");
        terminal.expect("CONCURRENT_EDIT");
        fixture.assert_snapshot(&edited);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([pair]));
        terminal.leave_pairs();
    }

    {
        let fixture = PairFixture::new("include-cutover-tab-delete-pair");
        let pair = fixture.setup_exact_pair();
        let paired = fixture.snapshot();
        let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
        terminal.expect("D delete");
        terminal.send(b"d");
        terminal.expect("Review Pair deletion");
        let new_gateway = fixture.home.join(".ssh/current-gateway");
        let new_vm = fixture.home.join(".ssh/current-vm");
        for (path, bytes) in [(&new_gateway, &paired[1].0), (&new_vm, &paired[3].0)] {
            fs::write(path, bytes).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(
            &fixture.config,
            format!(
                "Include \"{}\" \"{}\" \"{}\"\n",
                new_gateway.display(),
                fixture.decoy.display(),
                new_vm.display()
            ),
        )
        .unwrap();
        let cutover = fixture.snapshot();
        terminal.send(b"\r");
        terminal.expect_absent("Review Pair deletion");
        fixture.assert_snapshot(&cutover);
        for (path, bytes) in [(&new_gateway, &paired[1].0), (&new_vm, &paired[3].0)] {
            assert_eq!(fs::read(path).unwrap(), *bytes);
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        terminal.expect("PAIR_SELECTION");
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([pair]));
        terminal.leave_pairs();
        fixture.assert_snapshot(&cutover);
    }

    for after_review in [false, true] {
        let fixture = PairFixture::new("pending-tab-delete-pair");
        let pair = fixture.setup_exact_pair();
        let paired = fixture.snapshot();
        let mut terminal = if after_review {
            let mut terminal = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
            terminal.expect("D delete");
            terminal.send(b"d");
            terminal.expect("Review Pair deletion");
            Some(terminal)
        } else {
            None
        };
        let journal = fixture.pending_pair_journal();
        let journal_bytes = fs::read(&journal).unwrap();
        if let Some(terminal) = terminal.as_mut() {
            terminal.send(b"\r");
            terminal.expect_absent("Review Pair deletion");
            terminal.expect("PAIR_RECOVERY_PENDING");
        } else {
            let mut opened = PairTerminal::open(&fixture, &["tui", "pair", "list"]);
            opened.expect("PAIR_RECOVERY_PENDING");
            opened.send(b"dDsSv");
            opened.expect("Validation refreshed");
            opened.expect_absent("Review Pair deletion");
            opened.expect_absent("Pair setup");
            terminal = Some(opened);
        }
        fixture.assert_snapshot(&paired);
        assert_eq!(fixture.pairs()["pairs"], serde_json::json!([pair]));
        assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
        assert_eq!(
            fs::read(fixture.root.join("pending-before")).unwrap(),
            paired[1].0
        );
        assert_eq!(
            fs::read(fixture.root.join("pending-after")).unwrap(),
            paired[1].0
        );
        terminal.as_mut().unwrap().leave_pairs();
        fixture.assert_snapshot(&paired);
        assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    }

    let fixture = PairFixture::new("conflict-tab-delete-pair");
    let pair = fixture.setup_exact_pair();
    let decoy = fs::read_to_string(&fixture.decoy).unwrap();
    fs::write(
        &fixture.decoy,
        decoy.replace(
            "Host vm\n",
            &format!(
                "Host vm\n##SSHX GATEWAY={}\n##SSHX GATEWAY={PAIR_OTHER_ID}\n",
                pair["gateway_id"].as_str().unwrap()
            ),
        ),
    )
    .unwrap();
    let conflicted = fixture.snapshot();
    assert_pair_delete_blocked(&fixture, "PAIR_INVALID");
    fixture.assert_snapshot(&conflicted);
}

fn assert_pair_delete_blocked(fixture: &PairFixture, error: &str) {
    let before = fixture.snapshot();
    let mut terminal = PairTerminal::open(fixture, &["tui", "pair", "list"]);
    terminal.expect("D delete");
    terminal.send(b"d");
    terminal.expect("Pair deletion blocked");
    terminal.expect(error);
    terminal.expect("Enter/Esc return");
    fixture.assert_snapshot(&before);
    terminal.send(b"\r");
    terminal.expect_absent("Pair deletion blocked");
    terminal.expect(error);
    fixture.assert_snapshot(&before);
    terminal.leave_pairs();
    fixture.assert_snapshot(&before);
}

#[test]
fn pair_delete_preserves_adjacent_entries_and_legacy_metadata() {
    for newline in ["\n", "\r\n"] {
        for (gateway_header, vm_header, third_header) in [
            ("Host gateway gw", "Host vm machine", "Host unrelated"),
            ("Host=gateway", "Host=vm", "Host=unrelated"),
            ("Host=gateway gw", "Host=vm machine", "Host=unrelated third"),
        ] {
            for (in_stanza, match_boundary) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let fixture = PairFixture::new("adjacent-delete-pair");
                fs::write(
                    &fixture.config,
                    format!("Include \"{}\"\n", fixture.gateway.display()),
                )
                .unwrap();
                let gateway_marker = format!("##SSHX vM: {PAIR_VM_ID}\n");
                let vm_markers =
                    format!("##SSHX gateway {PAIR_GATEWAY_ID}\n##SSHX TRANSIT 127.0.0.2 2222\n");
                let gateway_body = "  HostName gateway.example\n  ##PASSWORD pair-stored-secret\n  LocalForward 127.0.0.1:12223 127.0.0.2:2222\n";
                let vm_body = "  HostName actual.example\n  Port 2222\n";
                let gateway = if in_stanza {
                    format!(
                        "##SSHX ID={PAIR_GATEWAY_ID}\n{gateway_header}\n{gateway_marker}{gateway_body}"
                    )
                } else {
                    format!(
                        "##SSHX ID={PAIR_GATEWAY_ID}\n{gateway_marker}{gateway_header}\n{gateway_body}"
                    )
                };
                let vm = if in_stanza {
                    format!("##SSHX ID={PAIR_VM_ID}\n{vm_header}\n{vm_markers}{vm_body}")
                } else {
                    format!("##SSHX ID={PAIR_VM_ID}\n{vm_markers}{vm_header}\n{vm_body}")
                };
                let boundary = if match_boundary {
                    format!(
                        "##SSHX VM={PAIR_VM_ID}\n##SSHX CUSTOM=match-preamble\nMatch all\n  ServerAliveInterval 60\n"
                    )
                } else {
                    String::new()
                };
                // Match/Host preambles and the EOF marker are not owned by either endpoint.
                let original = format!(
                    "# retained ordinary comment\n{gateway}\n{vm}\n{boundary}\
##SSHX ID={PAIR_OTHER_ID}\n##SSHX CUSTOM=third-preamble\n\
{third_header}\n  HostName unrelated.example\n  Port 22\n\
##SSHX GATEWAY={PAIR_GATEWAY_ID}\n"
                )
                .replace('\n', newline);
                fs::write(&fixture.gateway, &original).unwrap();
                fs::set_permissions(&fixture.gateway, fs::Permissions::from_mode(0o640)).unwrap();
                let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
                assert_eq!(entries.len(), 3);
                let records = sshx::pair::records(&entries);
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].gateway_id, PAIR_GATEWAY_ID);
                assert_eq!(records[0].vm_id, PAIR_VM_ID);
                assert_eq!(records[0].transit_host, "127.0.0.2");
                assert_eq!(records[0].transit_port, 2222);
                let mut expected = fixture.snapshot();
                expected[1].0 = original
                    .replace(&gateway_marker.replace('\n', newline), "")
                    .replace(&vm_markers.replace('\n', newline), "")
                    .into_bytes();
                let plan = sshx::pair::plan_delete(&entries, &records[0]).unwrap();
                sshx::mutation::apply_pair(&plan).unwrap();
                fixture.assert_snapshot(&expected);
                let remaining = sshx::discovery::discover(&fixture.config).unwrap().entries;
                assert_eq!(
                    remaining
                        .iter()
                        .map(|entry| entry.id.as_str())
                        .collect::<Vec<_>>(),
                    [PAIR_GATEWAY_ID, PAIR_VM_ID, PAIR_OTHER_ID]
                );
                assert!(sshx::pair::records(&remaining).is_empty());
            }
        }
    }

    // Legacy in-stanza IDs are supported in separate sources without borrowing adjacent IDs.
    for newline in ["\n", "\r\n"] {
        for (gateway_header, vm_header) in [
            ("Host gateway gw", "Host vm machine"),
            ("Host=gateway", "Host=vm"),
            ("Host=gateway gw", "Host=vm machine"),
        ] {
            for id_key in ["ID", "ID:"] {
                let fixture = PairFixture::new("legacy-identity-delete-pair");
                fs::write(
                    &fixture.config,
                    format!(
                        "Include \"{}\" \"{}\" \"{}\"\n",
                        fixture.gateway.display(),
                        fixture.decoy.display(),
                        fixture.vm.display()
                    ),
                )
                .unwrap();
                let gateway_marker = format!("  ##SSHX vm={}\n", PAIR_VM_ID.to_ascii_uppercase());
                let vm_markers = format!(
                    "  ##SSHX GATEWAY: {}\n  ##SSHX transit: [::1]:2222\n",
                    PAIR_GATEWAY_ID.to_ascii_uppercase()
                );
                let gateway = format!(
                    "# gateway comment\n{gateway_header}\n  ##SSHX {id_key} {PAIR_GATEWAY_ID}\n\
{gateway_marker}  ##SSHX UNKNOWN=keep\n  HostName gateway.example\n\
  ##PASSWORD pair-stored-secret\n  LocalForward 127.0.0.1:12222 [::1]:2222\n"
                )
                .replace('\n', newline);
                let vm = format!(
                    "# vm comment\n{vm_header}\n  ##SSHX {id_key} {PAIR_VM_ID}\n\
{vm_markers}  HostName actual.example\n  Port 2222\n"
                )
                .replace('\n', newline);
                fs::write(&fixture.gateway, &gateway).unwrap();
                fs::write(&fixture.vm, &vm).unwrap();
                fs::set_permissions(&fixture.vm, fs::Permissions::from_mode(0o640)).unwrap();
                let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
                let records = sshx::pair::records(&entries);
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].gateway_id, PAIR_GATEWAY_ID);
                assert_eq!(records[0].vm_id, PAIR_VM_ID);
                assert_eq!(records[0].transit_host, "::1");
                let mut expected = fixture.snapshot();
                expected[1].0 = gateway
                    .replace(&gateway_marker.replace('\n', newline), "")
                    .into_bytes();
                expected[3].0 = vm
                    .replace(&vm_markers.replace('\n', newline), "")
                    .into_bytes();
                let plan = sshx::pair::plan_delete(&entries, &records[0]).unwrap();
                sshx::mutation::apply_pair(&plan).unwrap();
                fixture.assert_snapshot(&expected);
                let remaining = sshx::discovery::discover(&fixture.config).unwrap().entries;
                assert!(remaining.iter().any(|entry| entry.id == PAIR_GATEWAY_ID));
                assert!(remaining.iter().any(|entry| entry.id == PAIR_VM_ID));
                assert!(sshx::pair::records(&remaining).is_empty());
            }
        }
    }
}

#[test]
fn pair_delete_allows_changed_route_but_refuses_conflicting_metadata() {
    let seed = |fixture: &PairFixture| {
        fs::write(
            &fixture.config,
            format!(
                "Include \"{}\" \"{}\" \"{}\"\n",
                fixture.gateway.display(),
                fixture.decoy.display(),
                fixture.vm.display()
            ),
        )
        .unwrap();
        for (source, markers) in [
            (
                &fixture.gateway,
                format!("##SSHX ID={PAIR_GATEWAY_ID}\n##SSHX VM={PAIR_VM_ID}\n"),
            ),
            (
                &fixture.vm,
                format!(
                    "##SSHX ID={PAIR_VM_ID}\n##SSHX GATEWAY={PAIR_GATEWAY_ID}\n##SSHX TRANSIT=127.0.0.2:2222\n"
                ),
            ),
        ] {
            let original = fs::read_to_string(source).unwrap();
            fs::write(source, format!("{markers}{original}")).unwrap();
        }
        let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
        let records = sshx::pair::records(&entries);
        assert_eq!(records.len(), 1);
        records.into_iter().next().unwrap()
    };

    for case in [
        "forward",
        "port",
        "proxy-command",
        "proxy-jump",
        "unrelated-invalid-ids",
    ] {
        let fixture = PairFixture::new("route-delete-pair");
        let selected = seed(&fixture);
        let (source, replacement) = match case {
            "forward" => (
                &fixture.gateway,
                fs::read_to_string(&fixture.gateway)
                    .unwrap()
                    .replace("127.0.0.2:2222", "127.0.0.9:2200"),
            ),
            "port" => (
                &fixture.vm,
                fs::read_to_string(&fixture.vm)
                    .unwrap()
                    .replace("  Port 2222\n", "  Port 2200\n"),
            ),
            "proxy-command" => (
                &fixture.gateway,
                format!(
                    "{}  ProxyCommand nc %h %p\n",
                    fs::read_to_string(&fixture.gateway).unwrap()
                ),
            ),
            "proxy-jump" => (
                &fixture.vm,
                format!(
                    "{}  ProxyJump gateway\n",
                    fs::read_to_string(&fixture.vm).unwrap()
                ),
            ),
            "unrelated-invalid-ids" => (
                &fixture.decoy,
                format!(
                    "##SSHX ID=not-a-uuid\n##SSHX ID: also-invalid\n##SSHX GATEWAY={PAIR_OTHER_ID}\n\
##SSHX TRANSIT=invalid\n{}",
                    fs::read_to_string(&fixture.decoy).unwrap()
                ),
            ),
            _ => unreachable!(),
        };
        fs::write(source, replacement).unwrap();
        let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
        if case != "unrelated-invalid-ids" {
            let vm = entries.iter().find(|entry| entry.id == PAIR_VM_ID).unwrap();
            assert!(sshx::pair::paired_route(&entries, vm).is_err(), "{case}");
        }
        let before = fixture.snapshot();
        let mut expected = before.clone();
        expected[1].0 = String::from_utf8(before[1].0.clone())
            .unwrap()
            .replace(&format!("##SSHX VM={PAIR_VM_ID}\n"), "")
            .into_bytes();
        expected[3].0 = String::from_utf8(before[3].0.clone())
            .unwrap()
            .replace(&format!("##SSHX GATEWAY={PAIR_GATEWAY_ID}\n"), "")
            .replace("##SSHX TRANSIT=127.0.0.2:2222\n", "")
            .into_bytes();
        let plan = sshx::pair::plan_delete(&entries, &selected)
            .unwrap_or_else(|error| panic!("{case}: {error}"));
        fixture.assert_snapshot(&before);
        sshx::mutation::apply_pair(&plan).unwrap();
        fixture.assert_snapshot(&expected);
        assert!(
            sshx::pair::records(&sshx::discovery::discover(&fixture.config).unwrap().entries)
                .is_empty(),
            "{case}"
        );
    }

    for (case, role, markers) in [
        (
            "duplicate-gateway-id",
            2,
            format!("##SSHX ID={}\n", PAIR_GATEWAY_ID.to_ascii_uppercase()),
        ),
        (
            "duplicate-vm-id",
            2,
            format!("##SSHX ID={}\n", PAIR_VM_ID.to_ascii_uppercase()),
        ),
        (
            "additional-third-gateway-id",
            2,
            format!("##SSHX ID={PAIR_OTHER_ID}\n##SSHX ID: {PAIR_GATEWAY_ID}\n"),
        ),
        (
            "additional-third-vm-id",
            2,
            format!("##SSHX ID={PAIR_OTHER_ID}\n##SSHX ID {PAIR_VM_ID}\n"),
        ),
        (
            "additional-gateway-id",
            1,
            format!("##SSHX ID={PAIR_OTHER_ID}\n"),
        ),
        (
            "additional-vm-id",
            3,
            format!("##SSHX ID={PAIR_OTHER_ID}\n"),
        ),
        (
            "repeated-gateway-id",
            1,
            format!("##SSHX ID={PAIR_GATEWAY_ID}\n"),
        ),
        ("repeated-vm-id", 3, format!("##SSHX ID={PAIR_VM_ID}\n")),
        (
            "duplicate-gateway-vm",
            1,
            format!("##SSHX VM={PAIR_VM_ID}\n"),
        ),
        (
            "duplicate-vm-gateway",
            3,
            format!("##SSHX GATEWAY={PAIR_GATEWAY_ID}\n"),
        ),
        (
            "duplicate-vm-transit",
            3,
            "##SSHX TRANSIT=127.0.0.2:2222\n".to_string(),
        ),
        ("malformed-gateway-vm", 1, "##SSHX VM=\n".to_string()),
        ("malformed-vm-gateway", 3, "##SSHX GATEWAY=\n".to_string()),
        (
            "malformed-vm-transit",
            3,
            "##SSHX TRANSIT=invalid\n".to_string(),
        ),
        (
            "cross-role-gateway-gateway",
            1,
            format!("##SSHX GATEWAY={PAIR_OTHER_ID}\n"),
        ),
        (
            "cross-role-gateway-transit",
            1,
            "##SSHX TRANSIT=127.0.0.2:2222\n".to_string(),
        ),
        (
            "cross-role-vm-vm",
            3,
            format!("##SSHX VM={PAIR_OTHER_ID}\n"),
        ),
        (
            "third-gateway-reference",
            2,
            format!("##SSHX GATEWAY={PAIR_GATEWAY_ID}\n"),
        ),
        ("third-vm-reference", 2, format!("##SSHX VM={PAIR_VM_ID}\n")),
        (
            "third-gateway-to-vm",
            2,
            format!("##SSHX GATEWAY={PAIR_VM_ID}\n"),
        ),
        (
            "third-vm-to-gateway",
            2,
            format!("##SSHX VM={PAIR_GATEWAY_ID}\n"),
        ),
        (
            "overwritten-third-gateway-reference",
            2,
            format!(
                "##SSHX GATEWAY={}\n##SSHX GATEWAY={PAIR_OTHER_ID}\n",
                PAIR_GATEWAY_ID.to_ascii_uppercase()
            ),
        ),
        (
            "overwritten-third-vm-reference",
            2,
            format!(
                "##SSHX VM={}\n##SSHX VM={PAIR_OTHER_ID}\n",
                PAIR_VM_ID.to_ascii_uppercase()
            ),
        ),
    ] {
        let fixture = PairFixture::new(case);
        let selected = seed(&fixture);
        let source = fixture.sources()[role];
        // Insert inside the stanza so the existing immutable ID remains discovery's first ID.
        let original = fs::read_to_string(source).unwrap();
        let host = if role == 1 {
            "Host gateway\n"
        } else {
            "Host vm\n"
        };
        fs::write(source, original.replace(host, &format!("{host}{markers}"))).unwrap();
        let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
        let before = fixture.snapshot();
        let error = sshx::pair::plan_delete(&entries, &selected).unwrap_err();
        assert_eq!(
            error, "PAIR_INVALID: selected Pair has conflicting metadata",
            "{case}"
        );
        fixture.assert_snapshot(&before);
    }

    for (source_role, original_marker, changed_marker) in [
        (
            1,
            format!("##SSHX VM={PAIR_VM_ID}\n"),
            format!("##SSHX VM={PAIR_OTHER_ID}\n"),
        ),
        (
            3,
            format!("##SSHX GATEWAY={PAIR_GATEWAY_ID}\n"),
            format!("##SSHX GATEWAY={PAIR_OTHER_ID}\n"),
        ),
    ] {
        let fixture = PairFixture::new("nonreciprocal-delete-pair");
        let selected = seed(&fixture);
        let source = fixture.sources()[source_role];
        let original = fs::read_to_string(source).unwrap();
        fs::write(source, original.replace(&original_marker, &changed_marker)).unwrap();
        let entries = sshx::discovery::discover(&fixture.config).unwrap().entries;
        let before = fixture.snapshot();
        let error = sshx::pair::plan_delete(&entries, &selected).unwrap_err();
        assert_eq!(
            error,
            "PAIR_INVALID: selected Pair has conflicting metadata"
        );
        fixture.assert_snapshot(&before);
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
        let root = std::env::temp_dir().join(format!(
            "sshx-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::set_permissions(home.join(".ssh"), fs::Permissions::from_mode(0o700)).unwrap();
        let config = home.join(".ssh/config");
        let gateway = home.join(".ssh/gateway-source");
        let decoy = home.join(".ssh/first-vm");
        let vm = home.join(".ssh/second-vm");
        for (source, content) in [
            (&config, "Include gateway-source first-vm second-vm\n"),
            (
                &gateway,
                "Host gateway\n  HostName gateway.example\n  ##PASSWORD pair-stored-secret\n  LocalForward 127.0.0.1:12222 127.0.0.1:2222\n  LocalForward 127.0.0.1:12223 127.0.0.2:2222\n",
            ),
            (&decoy, "Host vm\n  HostName decoy.example\n  Port 2222\n"),
            (&vm, "Host vm\n  HostName actual.example\n  Port 2222\n"),
        ] {
            fs::write(source, content).unwrap();
            fs::set_permissions(source, fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self {
            root,
            home,
            config,
            gateway,
            decoy,
            vm,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
        command
            .arg("--config")
            .arg(&self.config)
            .env("HOME", &self.home);
        command
    }

    fn sources(&self) -> [&Path; 4] {
        [&self.config, &self.gateway, &self.decoy, &self.vm]
    }

    fn snapshot(&self) -> Vec<(Vec<u8>, u32)> {
        self.sources()
            .iter()
            .map(|source| {
                (
                    fs::read(source).unwrap(),
                    fs::metadata(source).unwrap().permissions().mode() & 0o777,
                )
            })
            .collect()
    }

    fn assert_snapshot(&self, before: &[(Vec<u8>, u32)]) {
        for (source, (bytes, mode)) in self.sources().iter().zip(before) {
            assert_eq!(
                fs::read(source).unwrap(),
                *bytes,
                "{} changed",
                source.display()
            );
            assert_eq!(
                fs::metadata(source).unwrap().permissions().mode() & 0o777,
                *mode
            );
        }
    }

    fn pairs(&self) -> serde_json::Value {
        let output = self
            .command()
            .args(["pair", "list", "--format", "json", "--no-input"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn setup_exact_pair(&self) -> serde_json::Value {
        let entries = sshx::discovery::discover(&self.vm).unwrap().entries;
        let line = entries[0].source.line_start.to_string();
        let output = self
            .command()
            .args([
                "pair",
                "setup",
                "gateway",
                "vm",
                "--vm-source",
                self.vm.to_str().unwrap(),
                "--vm-line",
                &line,
                "--transit-host",
                "127.0.0.2",
                "--transit-port",
                "2222",
                "--yes",
                "--no-input",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let pairs = self.pairs();
        assert_eq!(pairs["pairs"].as_array().unwrap().len(), 1);
        let pair = pairs["pairs"][0].clone();
        for (source, field) in [(&self.gateway, "gateway_id"), (&self.vm, "vm_id")] {
            let entries = sshx::discovery::discover(source).unwrap().entries;
            assert_eq!(pair[field], entries[0].id);
        }
        assert_eq!(pair["transit_host"], "127.0.0.2");
        assert_eq!(pair["transit_port"], 2222);
        pair
    }

    fn pending_pair_journal(&self) -> PathBuf {
        let before = self.root.join("pending-before");
        let after = self.root.join("pending-after");
        let bytes = fs::read(&self.gateway).unwrap();
        fs::write(&before, &bytes).unwrap();
        fs::write(&after, &bytes).unwrap();
        let digest = bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        let journal = self
            .gateway
            .with_file_name(".gateway-source.sshx.lock.journal");
        fs::write(
            &journal,
            serde_json::json!({"writes": [{
                "path": fs::canonicalize(&self.gateway).unwrap(),
                "before": before, "after": after,
                "before_digest": digest, "after_digest": digest, "existed": true
            }]})
            .to_string(),
        )
        .unwrap();
        journal
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
        Self::open_sized(fixture, arguments, 120, 40)
    }

    fn open_sized(fixture: &PairFixture, arguments: &[&str], width: u16, height: u16) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize {
            ws_row: height,
            ws_col: width,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut size,
                )
            },
            0
        );
        let slave = unsafe { File::from_raw_fd(slave) };
        let child = fixture
            .command()
            .args(arguments)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap();
        let master = unsafe { File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        Self {
            child,
            master,
            output: Vec::new(),
            screen: TerminalScreen::new(usize::from(width), usize::from(height)),
        }
    }

    fn expect(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if self.screen.contains(marker) {
                break;
            }
            if Instant::now() >= deadline || self.child.try_wait().unwrap().is_some() {
                panic!(
                    "PTY did not show {marker}: {}",
                    String::from_utf8_lossy(&self.output)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn expect_scrolled(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if self.drain_matching(marker) || self.screen.contains(marker) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "PTY context did not show {marker}: {}",
                String::from_utf8_lossy(&self.output)
            );
            let before = self.output.len();
            self.send(b"\x1b[6~");
            loop {
                if self.drain_matching(marker) {
                    return;
                }
                if self.output.len() > before {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "PTY context did not advance toward {marker}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn drain_matching(&mut self, marker: &str) -> bool {
        let mut buffer = [0u8; 8192];
        let mut found = false;
        match self.master.read(&mut buffer) {
            Ok(size) => {
                self.output.extend_from_slice(&buffer[..size]);
                // One read may contain both candidate selection and the following scroll redraw.
                for byte in &buffer[..size] {
                    self.screen.feed(std::slice::from_ref(byte));
                    if Some(byte) == marker.as_bytes().last() && self.screen.contains(marker) {
                        found = true;
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EIO) => {}
            Err(error) => panic!("PTY read failed: {error}"),
        }
        found
    }

    fn expect_absent(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if !self.screen.contains(marker) {
                break;
            }
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
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EIO) => {}
            Err(error) => panic!("PTY read failed: {error}"),
        }
    }

    fn send(&mut self, input: &[u8]) {
        self.master.write_all(input).unwrap();
    }

    fn leave_pairs(&mut self) {
        self.send(b"\x1b");
        self.expect("sshx Hosts");
        self.send(b"\x1b");
        assert_eq!(self.finish(), Some(0));
    }

    fn finish(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            assert!(
                Instant::now() < deadline,
                "PTY did not exit: {}",
                String::from_utf8_lossy(&self.output)
            );
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
    width: usize,
    height: usize,
    row: usize,
    column: usize,
    pending: Vec<u8>,
}

impl TerminalScreen {
    fn new(width: usize, height: usize) -> Self {
        assert!(
            width > 0 && height > 0,
            "terminal dimensions must be non-zero"
        );
        Self {
            cells: vec![vec![' '; width]; height],
            width,
            height,
            row: 0,
            column: 0,
            pending: Vec::new(),
        }
    }

    fn contains(&self, expected: &str) -> bool {
        let text: String = self
            .cells
            .iter()
            .flatten()
            .filter(|character| !character.is_whitespace())
            .collect();
        let expected: String = expected
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        text.contains(&expected)
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index] == 0x1b {
                let Some(&next) = self.pending.get(index + 1) else {
                    break;
                };
                if next == b'[' {
                    let Some(end) = (index + 2..self.pending.len())
                        .find(|&position| (0x40..=0x7e).contains(&self.pending[position]))
                    else {
                        break;
                    };
                    let parameters =
                        String::from_utf8_lossy(&self.pending[index + 2..end]).into_owned();
                    self.control(&parameters, self.pending[end]);
                    index = end + 1;
                } else if next == b']' {
                    let Some(end) = (index + 2..self.pending.len()).find(|&position| {
                        self.pending[position] == 7
                            || (self.pending[position] == 0x1b
                                && self.pending.get(position + 1) == Some(&b'\\'))
                    }) else {
                        break;
                    };
                    index = end + if self.pending[end] == 7 { 1 } else { 2 };
                } else {
                    index += 2;
                }
                continue;
            }
            let byte = self.pending[index];
            match byte {
                b'\r' => self.column = 0,
                b'\n' => self.row = (self.row + 1).min(self.height - 1),
                8 => self.column = self.column.saturating_sub(1),
                b'\t' => self.column = ((self.column / 8 + 1) * 8).min(self.width - 1),
                0..=31 | 127 => {}
                _ => {
                    let length = match byte {
                        0..=127 => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    if index + length > self.pending.len() {
                        break;
                    }
                    let character = std::str::from_utf8(&self.pending[index..index + length])
                        .expect("PTY emitted invalid UTF-8")
                        .chars()
                        .next()
                        .unwrap();
                    self.cells[self.row][self.column] = character;
                    self.column = (self.column + 1).min(self.width - 1);
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
                for row in &mut self.cells {
                    row.fill(' ');
                }
                self.row = 0;
                self.column = 0;
            }
            return;
        }
        let values: Vec<usize> = parameters
            .split(';')
            .map(|value| value.parse().unwrap_or(0))
            .collect();
        let value = values.first().copied().unwrap_or(0);
        let amount = value.max(1);
        match command {
            b'H' | b'f' => {
                self.row = value.max(1).saturating_sub(1).min(self.height - 1);
                self.column = values
                    .get(1)
                    .copied()
                    .unwrap_or(1)
                    .max(1)
                    .saturating_sub(1)
                    .min(self.width - 1);
            }
            b'A' => self.row = self.row.saturating_sub(amount),
            b'B' => self.row = (self.row + amount).min(self.height - 1),
            b'C' => self.column = (self.column + amount).min(self.width - 1),
            b'D' => self.column = self.column.saturating_sub(amount),
            b'G' => self.column = amount.saturating_sub(1).min(self.width - 1),
            b'd' => self.row = amount.saturating_sub(1).min(self.height - 1),
            b'J' => {
                for row in 0..self.height {
                    for column in 0..self.width {
                        let position = (row, column);
                        if value == 2
                            || value == 3
                            || (value == 0 && position >= (self.row, self.column))
                            || (value == 1 && position <= (self.row, self.column))
                        {
                            self.cells[row][column] = ' ';
                        }
                    }
                }
            }
            b'K' => {
                let range = match value {
                    1 => 0..self.column + 1,
                    2 => 0..self.width,
                    _ => self.column..self.width,
                };
                self.cells[self.row][range].fill(' ');
            }
            _ => {}
        }
    }
}
