#![cfg(unix)]

//! Host form, delete confirmation and undo/redo, driven through the binary in a PTY.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ORIGINAL: &str = concat!(
    "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
    "Host alpha\n",
    "  HostName alpha.example\n",
    "  CustomDirective keep-me\n",
    "  # note stays\n",
    "Host beta\n",
    "  HostName beta.example\n",
);

const PAIRED: &str = concat!(
    "##SSHX ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
    "##SSHX VM=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
    "Host gw\n",
    "  HostName gateway.example\n",
    "  LocalForward 127.0.0.1:12224 127.0.0.3:2222\n",
    "##SSHX ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\n",
    "##SSHX GATEWAY=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n",
    "##SSHX TRANSIT=127.0.0.3:2222\n",
    "Host vm\n",
    "  HostName 127.0.0.1\n",
    "  Port 12224\n",
);

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(name: &str, contents: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sshx-tui-hosts-{name}-{}-{}",
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
        fs::write(&config, contents).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        Self { root, home, config }
    }

    fn text(&self) -> String {
        fs::read_to_string(&self.config).unwrap()
    }

    /// The finder in Normal input mode.
    fn open(&self, env: &[(&str, &str)]) -> Terminal {
        let mut terminal = Terminal::open(&self.home, &self.config, env);
        terminal.expect("Search:");
        terminal.send(b"\x1b");
        terminal.expect("NORMAL");
        terminal
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// `##SSHX ID=<uuid>` line that sshx put on a new block.
fn without_new_id(text: &str, prefix: &str) -> String {
    let rest = text
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{prefix:?} should stay the file prefix: {text:?}"));
    let (first, block) = rest.split_once('\n').expect("a new block should follow");
    let id = first
        .strip_prefix("##SSHX ID=")
        .expect("new block has an ID");
    assert_eq!(id.len(), 36, "{first}");
    block.to_string()
}

fn create_gamma(terminal: &mut Terminal) {
    terminal.send(b"o");
    terminal.expect("new host");
    terminal.send(b"igamma\rjigamma.example\r");
    terminal.expect("HostName gamma.example");
    terminal.send(b":wq\r");
    terminal.expect("Search:");
}

#[test]
fn new_host_form_creates_the_block_on_write() {
    let fixture = Fixture::new("create", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"o");
    terminal.expect("new host");
    terminal.send(b"igamma\rjigamma.example\r");
    terminal.expect("+Host gamma");
    terminal.expect("+  HostName gamma.example");
    assert_eq!(fixture.text(), ORIGINAL, "preview must not write");
    terminal.send(b":w\r");
    terminal.expect("saved gamma");
    assert_eq!(
        without_new_id(&fixture.text(), ORIGINAL),
        "Host gamma\n  HostName gamma.example\n"
    );
    terminal.send(b":q\r");
    terminal.expect("Search:");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn only_alias_and_host_are_required_and_errors_show_inline() {
    let fixture = Fixture::new("required", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"o");
    terminal.expect("alias is required");
    terminal.send(b"igamma\r");
    terminal.expect("host is required");
    terminal.send(b":w\r");
    terminal.expect("host is required");
    terminal.send(b"jjjiseventy\r");
    terminal.expect("PORT_INVALID");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"\x03");
    assert_eq!(terminal.finish(), Some(130));
}

#[test]
fn edit_form_rewrites_only_the_host_block() {
    let fixture = Fixture::new("edit", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"cc");
    terminal.expect("edit host");
    terminal.expect("alpha.example");
    terminal.send(b"j");
    terminal.send(b"i");
    terminal.send(&[0x7f; 13]);
    terminal.send(b"new.example\r");
    terminal.expect("+  HostName new.example");
    terminal.send(b":wq\r");
    terminal.expect("Search:");
    assert_eq!(
        fixture.text(),
        ORIGINAL.replace("alpha.example", "new.example")
    );
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn zz_saves_and_closes_and_q_cancels() {
    let fixture = Fixture::new("zz", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"cc");
    terminal.expect("edit host");
    terminal.send(b"jifresh.example\r");
    terminal.expect("fresh.example");
    terminal.send(b"q");
    terminal.expect("Search:");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"cc");
    terminal.expect("edit host");
    terminal.send(b"jifresh.example\r");
    terminal.expect("fresh.example");
    terminal.send(b"ZZ");
    terminal.expect("Search:");
    assert!(
        fixture
            .text()
            .contains("HostName alpha.examplefresh.example")
    );
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn edit_of_a_host_without_id_adds_one_and_keeps_everything_else() {
    let fixture = Fixture::new("edit-no-id", "Host solo\n  HostName solo.example\n# tail\n");
    let mut terminal = fixture.open(&[]);
    terminal.send(b"cc");
    terminal.expect("edit host");
    terminal.send(b"jjji2222\r");
    terminal.expect("+  Port 2222");
    terminal.send(b":wq\r");
    terminal.expect("Search:");
    let text = fixture.text();
    let (first, rest) = text.split_once('\n').unwrap();
    assert!(first.starts_with("##SSHX ID="), "{text}");
    assert!(rest.starts_with("Host solo\n"), "{text}");
    assert!(rest.ends_with("# tail\n"), "{text}");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn via_picks_an_existing_alias_and_writes_proxy_jump() {
    let fixture = Fixture::new("via", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"o");
    terminal.expect("new host");
    terminal.send(b"igamma\rjigamma.example\r");
    terminal.expect("HostName gamma.example");
    terminal.send(b"jjji");
    terminal.send(b"alph");
    terminal.expect("↳ alpha");
    terminal.send(b"\r");
    terminal.expect("+  ProxyJump alpha");
    terminal.send(b":wq\r");
    terminal.expect("Search:");
    assert_eq!(
        without_new_id(&fixture.text(), ORIGINAL),
        "Host gamma\n  HostName gamma.example\n  ProxyJump alpha\n"
    );
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn password_is_stored_through_the_file_mode_check_and_never_rendered() {
    let fixture = Fixture::new("password", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"o");
    terminal.expect("new host");
    terminal.send(b"igamma\rjigamma.example\rjjjjjis3cretpw\r");
    terminal.expect("+  ##PASSWORD ••••");
    terminal.send(b":w\r");
    terminal.expect("saved gamma");
    assert!(
        fixture.text().ends_with("  ##PASSWORD s3cretpw\n"),
        "{}",
        fixture.text()
    );
    terminal.expect("pass ••••");
    terminal.send(b":q\r");
    terminal.expect("Search:");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
    assert!(!terminal.seen_contains("s3cretpw"), "password was rendered");
}

#[test]
fn password_is_refused_inline_when_the_file_is_not_private() {
    let fixture = Fixture::new("insecure", ORIGINAL);
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o644)).unwrap();
    let mut terminal = fixture.open(&[]);
    terminal.send(b"o");
    terminal.expect("new host");
    terminal.send(b"igamma\rjigamma.example\rjjjjjihunter2\r");
    terminal.expect("PASSWORD_FILE_INSECURE");
    terminal.send(b":w\r");
    terminal.expect("PASSWORD_FILE_INSECURE");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"\x03");
    assert_eq!(terminal.finish(), Some(130));
}

#[test]
fn pair_members_show_a_read_only_pair_line_instead_of_via() {
    let fixture = Fixture::new("pair-form", PAIRED);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"cc");
    terminal.expect("pair: gw → vm");
    terminal.send(b"\x03");
    assert_eq!(terminal.finish(), Some(130));
    assert!(
        !terminal.seen_contains("via"),
        "Pair members must not offer a via field"
    );
}

#[test]
fn delete_shows_the_block_and_y_removes_only_it() {
    let fixture = Fixture::new("delete", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"dd");
    terminal.expect("Delete this HostEntry block");
    terminal.expect("-  CustomDirective keep-me");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"y");
    terminal.expect("deleted alpha");
    assert_eq!(fixture.text(), "Host beta\n  HostName beta.example\n");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn delete_n_cancels_and_redacts_the_password() {
    let text = format!(
        "{ORIGINAL}##SSHX ID=22222222-2222-4222-8222-222222222222\nHost vault\n  HostName vault.example\n  ##PASSWORD hunter2\n"
    );
    let fixture = Fixture::new("delete-n", &text);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"jjdd");
    terminal.expect("Delete this HostEntry block");
    terminal.expect("##PASSWORD ••••");
    terminal.send(b"n");
    terminal.expect("Search:");
    assert_eq!(fixture.text(), text);
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
    assert!(!terminal.seen_contains("hunter2"), "password was rendered");
}

#[test]
fn delete_is_refused_for_pair_members() {
    let fixture = Fixture::new("delete-pair", PAIRED);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"dd");
    terminal.expect("dissolve the Pair first");
    assert_eq!(fixture.text(), PAIRED);
    terminal.send(b"jdd");
    terminal.expect("dissolve the Pair first");
    assert_eq!(fixture.text(), PAIRED);
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn delete_is_refused_while_a_standalone_tunnel_uses_the_host() {
    let fixture = Fixture::new("delete-active", ORIGINAL);
    let tunnels = fixture.home.join(".config/sshx/tunnels");
    fs::create_dir_all(&tunnels).unwrap();
    fs::write(
        tunnels.join("registry.json"),
        r#"{"version":1,"tunnels":[{"state":"active","entry_id":"11111111-1111-4111-8111-111111111111","pair":null}]}"#,
    )
    .unwrap();
    let mut terminal = fixture.open(&[]);
    terminal.send(b"dd");
    terminal.expect("DELETE_ACTIVE");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"\x03");
    assert_eq!(terminal.finish(), Some(130));
}

#[test]
fn u_restores_the_file_and_ctrl_r_redoes() {
    let fixture = Fixture::new("undo", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    create_gamma(&mut terminal);
    let created = fixture.text();
    assert_ne!(created, ORIGINAL);
    terminal.send(b"u");
    terminal.expect("undid create gamma");
    assert_eq!(fixture.text(), ORIGINAL);
    terminal.send(b"\x12");
    terminal.expect("redid create gamma");
    assert_eq!(fixture.text(), created);
    terminal.send(b"dd");
    terminal.expect("Delete this HostEntry block");
    terminal.send(b"y");
    terminal.expect("deleted");
    terminal.send(b"u");
    terminal.expect("undid delete");
    assert_eq!(fixture.text(), created);
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn u_is_refused_after_an_external_edit_and_leaves_the_file() {
    let fixture = Fixture::new("undo-external", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    create_gamma(&mut terminal);
    let external = format!("{}# edited elsewhere\n", fixture.text());
    fs::write(&fixture.config, &external).unwrap();
    terminal.send(b"u");
    terminal.expect("UNDO_REFUSED");
    assert_eq!(fixture.text(), external);
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn undo_with_nothing_done_says_so() {
    let fixture = Fixture::new("undo-empty", ORIGINAL);
    let mut terminal = fixture.open(&[]);
    terminal.send(b"u");
    terminal.expect("nothing to undo");
    terminal.send(b"\x12");
    terminal.expect("nothing to redo");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn editing_a_host_without_id_moves_its_usage_record_to_the_new_id() {
    let fixture = Fixture::new("rekey", "Host solo\n  HostName solo.example\n");
    let usage = fixture.home.join(".ssh/.config.sshx-usage.json");
    fs::write(
        &usage,
        r#"{"version":1,"hosts":{"solo@config":{"count":7,"last_used":1000,"forwards":[]}}}"#,
    )
    .unwrap();
    let mut terminal = fixture.open(&[]);
    terminal.send(b"cc");
    terminal.expect("edit host");
    terminal.send(b"jisolo2.example\r:wq\r");
    terminal.expect("Search:");
    let text = fixture.text();
    let id = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("##SSHX ID="))
        .unwrap_or_else(|| panic!("ID should be added: {text}"))
        .to_string();
    let sidecar = fs::read_to_string(&usage).unwrap();
    assert!(sidecar.contains(&id), "{sidecar}");
    assert!(!sidecar.contains("solo@config"), "{sidecar}");
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

#[test]
fn ce_opens_the_editor_at_the_host_line_and_rediscovers() {
    let fixture = Fixture::new("editor", ORIGINAL);
    let editor = fixture.root.join("editor.sh");
    let arguments = fixture.root.join("editor-args");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf 'Host edited\\n  HostName edited.example\\n' >> \"$2\"\n",
            arguments.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal = fixture.open(&[("EDITOR", editor.to_str().unwrap())]);
    terminal.send(b"ce");
    terminal.expect("edited");
    assert_eq!(
        fs::read_to_string(&arguments).unwrap(),
        format!(
            "+2\n{}\n",
            fs::canonicalize(&fixture.config).unwrap().display()
        )
    );
    assert!(
        fixture
            .text()
            .ends_with("Host edited\n  HostName edited.example\n")
    );
    terminal.send(b"q");
    assert_eq!(terminal.finish(), Some(0));
}

struct Terminal {
    child: std::process::Child,
    master: File,
    raw: Vec<u8>,
    snapshots: Vec<String>,
}

impl Terminal {
    fn open(home: &Path, config: &Path, environment: &[(&str, &str)]) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize {
            ws_row: ROWS,
            ws_col: COLUMNS,
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
        let child = Command::new(env!("CARGO_BIN_EXE_sshx"))
            .arg("--config")
            .arg(config)
            .env("HOME", home)
            .envs(environment.iter().copied())
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
            raw: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    fn drain(&mut self) {
        let mut buffer = [0u8; 8192];
        match self.master.read(&mut buffer) {
            Ok(size) => self.raw.extend_from_slice(&buffer[..size]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EIO) => {}
            Err(error) => panic!("PTY read failed: {error}"),
        }
    }

    /// The screen as it looks now, whitespace removed so layout never matters.
    fn snapshot(&mut self) -> String {
        self.drain();
        let text = render(&self.raw);
        self.snapshots.push(text.clone());
        text
    }

    fn expect(&mut self, marker: &str) {
        let wanted = marker.split_whitespace().collect::<String>();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let screen = self.snapshot();
            if screen.contains(&wanted) {
                return;
            }
            if Instant::now() >= deadline || self.child.try_wait().unwrap().is_some() {
                panic!("PTY did not show {marker}: {screen}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&mut self, input: &[u8]) {
        self.snapshot();
        self.master.write_all(input).unwrap();
    }

    fn finish(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.drain();
            if let Some(status) = self.child.try_wait().unwrap() {
                self.snapshot();
                return status.code();
            }
            assert!(
                Instant::now() < deadline,
                "PTY did not exit: {}",
                render(&self.raw)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether any screen this terminal showed contained `text`.
    fn seen_contains(&self, text: &str) -> bool {
        self.snapshots.iter().any(|screen| screen.contains(text))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const ROWS: u16 = 40;
const COLUMNS: u16 = 120;

/// Replays the escape stream onto a character grid and returns its text without whitespace.
/// Handles the cursor moves, erases and alternate-screen switch ratatui emits.
fn render(raw: &[u8]) -> String {
    let (rows, columns) = (usize::from(ROWS), usize::from(COLUMNS));
    let blank = || vec![vec![' '; columns]; rows];
    let mut grid = blank();
    let (mut row, mut column) = (0usize, 0usize);
    let text = String::from_utf8_lossy(raw);
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            '\x1b' => match characters.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    let mut last = ' ';
                    for next in characters.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            last = next;
                            break;
                        }
                        parameters.push(next);
                    }
                    let numbers = parameters
                        .split(';')
                        .map(|part| part.trim_start_matches('?').parse::<usize>().unwrap_or(0))
                        .collect::<Vec<_>>();
                    let first = numbers.first().copied().unwrap_or(0).max(1);
                    match last {
                        'H' | 'f' => {
                            row = first - 1;
                            column = numbers.get(1).copied().unwrap_or(0).max(1) - 1;
                        }
                        'A' => row = row.saturating_sub(first),
                        'B' => row += first,
                        'C' => column += first,
                        'D' => column = column.saturating_sub(first),
                        'J' => grid = blank(),
                        'K' => {
                            if let Some(line) = grid.get_mut(row) {
                                line.iter_mut().skip(column).for_each(|cell| *cell = ' ');
                            }
                        }
                        'h' if parameters == "?1049" => grid = blank(),
                        _ => {}
                    }
                }
                Some(']') => {
                    for next in characters.by_ref() {
                        if next == '\x07' {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => column = 0,
            '\n' => row += 1,
            control if control.is_control() => {}
            printable => {
                if let Some(cell) = grid.get_mut(row).and_then(|line| line.get_mut(column)) {
                    *cell = printable;
                }
                column += 1;
            }
        }
    }
    grid.into_iter()
        .flatten()
        .filter(|character| !character.is_whitespace())
        .collect()
}
