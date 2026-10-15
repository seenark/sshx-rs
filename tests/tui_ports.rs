#![cfg(unix)]

use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicU64 = AtomicU64::new(0);
static PTY: Mutex<()> = Mutex::new(());

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(config: &str) -> Fixture {
    let root = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "sshx-tui-ports-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
    let home = root.join("home");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    let path = home.join(".ssh/config");
    fs::write(&path, config).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    Fixture {
        root,
        home,
        config: path,
    }
}

fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Session-bound master that waits for `SSHX_STARTED` listeners the test binds, then a shell.
fn session_ssh(fixture: &Fixture) -> PathBuf {
    let bin = fixture.root.join("session-bin");
    script(
        &bin.join("ssh"),
        r#"#!/bin/sh
config=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  previous="$argument"
done
if [ -f "$config" ]; then cat "$config" > "$SSHX_CAPTURE"; fi
case " $* " in
  *" -O check "*) if [ -f "$SSHX_STARTED" ]; then exit 0; fi; exit 1 ;;
  *" -O exit "*) : > "$SSHX_CLOSED"; exit 0 ;;
  *" -N "*) : > "$SSHX_STARTED"; trap 'exit 0' INT HUP TERM; while :; do sleep 0.1; done ;;
  *) printf 'direct-shell\n' ;;
esac
"#,
    );
    bin
}

/// Detached `-N` master that listens on every `-L` local port and exits on `-O exit`.
fn tunnel_ssh(fixture: &Fixture) -> PathBuf {
    let bin = fixture.root.join("tunnel-bin");
    script(
        &bin.join("ssh"),
        r#"#!/bin/sh
socket=
previous=
local_ports=
for argument in "$@"; do
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  if [ "$previous" = "-L" ]; then
    local_port=${argument#*:}
    local_port=${local_port%%:*}
    local_ports="$local_ports $local_port"
  fi
  previous="$argument"
done
marker="$socket.started"
pid_file="$socket.pid"
case " $* " in
  *" -O check "*) test -f "$marker"; exit ;;
  *" -O exit "*)
    if [ -f "$pid_file" ]; then
      kill "$(cat "$pid_file")" 2>/dev/null || :
      i=0
      while [ -e "$marker" ] && [ "$i" -lt 100 ]; do sleep 0.01; i=$((i + 1)); done
    fi
    rm -f "$marker" "$pid_file"
    exit 0
    ;;
  *" -N "*)
    python3 - "$marker" "$local_ports" <<'PY' >/dev/null 2>&1 &
import os,sys
os.setsid()
with open(os.devnull) as stdin:
    os.dup2(stdin.fileno(), 0)
os.execl("/bin/sh", "sh", "-c", r'''
marker=$1
local_ports=$2
: > "$marker"
python3 -c 'import socket,sys,time; ss=[socket.socket() for _ in sys.argv[1:]]; [s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1) for s in ss]; [s.bind(("127.0.0.1",int(p))) for s,p in zip(ss,sys.argv[1:])]; [s.listen() for s in ss]; time.sleep(60)' $local_ports >/dev/null 2>&1 &
listener=$!
trap 'kill "$listener" 2>/dev/null || :; wait "$listener" 2>/dev/null || :; rm -f "$marker"; exit 0' INT HUP TERM
while :; do sleep 0.01; done
''', "sshx-fixture-master", sys.argv[1], sys.argv[2])
PY
    printf '%s' "$!" > "$pid_file"
    exit 0
    ;;
  *) printf 'direct-shell\n' ;;
esac
"#,
    );
    bin
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn command(fixture: &Fixture, bin: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
    command
        .env("HOME", &fixture.home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("SSHX_CAPTURE", fixture.root.join("runtime-config"))
        .env("SSHX_STARTED", fixture.root.join("master-started"))
        .env("SSHX_CLOSED", fixture.root.join("master-closed"))
        .args(["--config", fixture.config.to_str().unwrap()]);
    command
}

fn cli_json(fixture: &Fixture, bin: &Path, args: &[&str]) -> Value {
    let output = command(fixture, bin).args(args).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

struct Tui {
    master: File,
    child: Child,
    output: Vec<u8>,
    mark: usize,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Tui {
    fn drop(&mut self) {
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

fn strip(input: &[u8]) -> String {
    let mut output = Vec::new();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            0x1b => {
                index += 1;
                if input.get(index) == Some(&b'[') {
                    index += 1;
                    while index < input.len() && !(0x40..=0x7e).contains(&input[index]) {
                        index += 1;
                    }
                }
                index += 1;
            }
            b'\r' => index += 1,
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Tui {
    fn start(fixture: &Fixture, bin: &Path) -> Self {
        let guard = PTY.lock().unwrap_or_else(|error| error.into_inner());
        let mut master = -1;
        let mut slave = -1;
        let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
        size.ws_col = 100;
        size.ws_row = 30;
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        };
        assert_eq!(opened, 0);
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut command = command(fixture, bin);
        command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let master = unsafe { File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
        Self {
            master,
            child,
            output: Vec::new(),
            mark: 0,
            _guard: guard,
        }
    }

    fn pump(&mut self) {
        let mut buffer = [0u8; 4096];
        match self.master.read(&mut buffer) {
            Ok(size) => self.output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }

    /// Waits for `text` (ignoring whitespace) to be drawn since the previous expectation.
    fn expect(&mut self, text: &str) {
        let wanted = squeeze(text);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if squeeze(&strip(&self.output[self.mark..])).contains(&wanted) {
                self.mark = self.output.len();
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for `{text}`; screen output:\n{}",
                strip(&self.output[self.mark..])
            );
        }
    }

    fn saw(&self, text: &str) -> bool {
        squeeze(&strip(&self.output)).contains(&squeeze(text))
    }

    fn send(&mut self, keys: &str) {
        self.master.write_all(keys.as_bytes()).unwrap();
    }

    fn finish(mut self) -> (ExitStatus, String) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(status) = self.child.try_wait().unwrap() {
                self.pump();
                return (status, strip(&self.output));
            }
            assert!(
                Instant::now() < deadline,
                "sshx did not exit; screen output:\n{}",
                squeeze(&strip(&self.output))
            );
        }
    }
}

/// Binds `ports` once the fake master has started, standing in for OpenSSH's listeners.
struct Listeners {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Listeners {
    fn after_master(fixture: &Fixture, ports: &[u16]) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let started = fixture.root.join("master-started");
        let ports = ports.to_vec();
        let thread = thread::spawn(move || {
            while !started.exists() && !flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            let _bound = ports
                .iter()
                .filter_map(|port| TcpListener::bind(("127.0.0.1", *port)).ok())
                .collect::<Vec<_>>();
            while !flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn write_usage(fixture: &Fixture, key: &str, forwards: Value) {
    fs::write(
        fixture.config.with_file_name(".config.sshx-usage.json"),
        json!({
            "version": 1,
            "hosts": { key: { "last_used": 1, "count": 1, "forwards": forwards } }
        })
        .to_string(),
    )
    .unwrap();
}

fn runtime_forwards(fixture: &Fixture) -> Vec<String> {
    fs::read_to_string(fixture.root.join("runtime-config"))
        .unwrap()
        .lines()
        .filter(|line| line.trim_start().starts_with("LocalForward"))
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn tab_enter_reconnects_with_last_selected_forwards_and_no_typing() {
    let (first, second) = (free_port(), free_port());
    let fixture = fixture(&format!(
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432 LOCAL={first}\n"
    ));
    write_usage(
        &fixture,
        "db-id",
        json!([
            {"remote": 5432, "local": first, "declared": true},
            {"remote": 6379, "local": second, "declared": false}
        ]),
    );
    let original = fs::read(&fixture.config).unwrap();
    let bin = session_ssh(&fixture);
    let _listeners = Listeners::after_master(&fixture, &[first, second]);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db");
    tui.expect("db");
    tui.send("\t");
    tui.expect(&format!("localhost:{second} → 6379"));
    tui.send("\r");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert_eq!(
        runtime_forwards(&fixture),
        [
            format!("LocalForward 127.0.0.1:{first} 127.0.0.1:5432"),
            format!("LocalForward 127.0.0.1:{second} 127.0.0.1:6379"),
        ]
    );
    assert_eq!(fs::read(&fixture.config).unwrap(), original);
}

#[test]
fn o_adds_a_row_and_enter_connects_with_it() {
    let local = free_port();
    let fixture = fixture("Host db\n  HostName db.example\n");
    let original = fs::read(&fixture.config).unwrap();
    let bin = session_ssh(&fixture);
    let _listeners = Listeners::after_master(&fixture, &[local]);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect("No service forwards");
    tui.send("o");
    tui.expect("Add REMOTE");
    tui.send(&format!("80={local}\r"));
    tui.expect(&format!("localhost:{local} → 80"));
    tui.send("\r");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert_eq!(
        runtime_forwards(&fixture),
        [format!("LocalForward 127.0.0.1:{local} 127.0.0.1:80")]
    );
    assert_eq!(fs::read(&fixture.config).unwrap(), original);
}

#[test]
fn s_declares_the_row_in_the_config_and_moves_usage_to_the_new_id() {
    let local = free_port();
    let fixture = fixture("Host db\n  HostName db.example\n");
    write_usage(
        &fixture,
        "db@config",
        json!([{"remote": 80, "local": local, "declared": false}]),
    );
    let bin = session_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("localhost:{local} → 80"));
    tui.send("s");
    tui.expect("declared service 80");
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let written = fs::read_to_string(&fixture.config).unwrap();
    let (first, rest) = written.split_once('\n').unwrap();
    let id = first.strip_prefix("##SSHX ID=").expect("ID line added");
    assert_eq!(
        rest,
        format!("Host db\n  HostName db.example\n  ##SSHX SERVICE 80 LOCAL={local}\n")
    );
    let usage: Value = serde_json::from_slice(
        &fs::read(fixture.config.with_file_name(".config.sshx-usage.json")).unwrap(),
    )
    .unwrap();
    assert!(usage["hosts"].get("db@config").is_none(), "{usage}");
    assert_eq!(usage["hosts"][id]["forwards"][0]["remote"], 80);
}

#[test]
fn dd_then_y_removes_a_declared_service_and_keeps_the_others() {
    let fixture = fixture(
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432\n  ##SSHX SERVICE 6379 NAME=cache\n",
    );
    let bin = session_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect("localhost:5432 → 5432");
    tui.send("dd");
    tui.expect("Remove declared service 5432");
    tui.send("y");
    tui.expect("removed service 5432");
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert_eq!(
        fs::read_to_string(&fixture.config).unwrap(),
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 6379 NAME=cache\n"
    );
}

#[test]
fn busy_tunnel_port_offers_the_next_free_port_and_persists_the_choice() {
    let busy_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let busy = busy_listener.local_addr().unwrap().port();
    let fixture = fixture(&format!(
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432 LOCAL={busy}\n"
    ));
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": busy, "declared": true}]),
    );
    let bin = tunnel_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("localhost:{busy} → 5432"));
    tui.send(" tt");
    tui.expect("pick another port");
    assert!(tui.saw(&format!("✗ :{busy} port in use")));
    let screen = squeeze(&strip(&tui.output));
    let offered = screen
        .rsplit("use:")
        .next()
        .unwrap()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u16>()
        .unwrap();
    assert_ne!(offered, busy);
    tui.send("n");
    tui.expect(&format!(":{offered} listener ready"));
    assert!(tui.saw("db master responsive"));
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let tunnel = &listed["tunnels"][0];
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["forwards"][0]["local_port"], offered);
    assert_eq!(tunnel["listener_status"], "bound");
    let id = tunnel["id"].as_str().unwrap().to_string();
    assert!(tui.saw(&format!("id {id}")));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    cli_json(
        &fixture,
        &bin,
        &["tunnel", "stop", &id, "--no-input", "--format", "json"],
    );
}

#[test]
fn tunnel_list_shows_the_tunnel_and_dd_then_y_stops_it() {
    let local = free_port();
    let fixture = fixture("Host db\n  HostName db.example\n  ##SSHX SERVICE 5432\n");
    let bin = tunnel_ssh(&fixture);
    let selector = format!("5432={local}");
    let started = cli_json(
        &fixture,
        &bin,
        &[
            "tunnel",
            "direct",
            "start",
            "db",
            "--forward",
            &selector,
            "--no-input",
            "--format",
            "json",
        ],
    );
    let id = started["tunnels"][0]["id"].as_str().unwrap().to_string();
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send("gt");
    tui.expect(&format!("localhost:{local} → 5432"));
    assert!(tui.saw(&id));
    tui.send("dd");
    tui.expect(&format!("Stop tunnel {id}"));
    tui.send("y");
    tui.expect(&format!("stopped {id}"));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    assert_eq!(listed["tunnels"][0]["id"], id.as_str());
    assert_eq!(listed["tunnels"][0]["state"], "stopped");
    assert!(!fixture.root.join("master-started").exists());
}

#[test]
fn tunnel_command_with_a_host_and_forward_starts_it() {
    let local = free_port();
    let fixture = fixture("Host db\n  HostName db.example\n");
    let bin = tunnel_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send(":");
    tui.send(&format!("tunnel start db 5432={local}\r"));
    tui.expect(&format!(":{local} listener ready"));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let id = listed["tunnels"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(listed["tunnels"][0]["forwards"][0]["local_port"], local);
    cli_json(
        &fixture,
        &bin,
        &["tunnel", "stop", &id, "--no-input", "--format", "json"],
    );
}
