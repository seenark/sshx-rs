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

mod support;

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

/// Binds every `-L` listener spec (`[bind:]port:host:port`) and holds them for a minute.
const LISTEN: &str = r#"import socket, sys, time
sockets = []
for spec in sys.argv[1:]:
    if spec.startswith("["):
        bind, rest = spec[1:].split("]:", 1)
    else:
        parts = spec.split(":")
        bind, rest = (parts[0], ":".join(parts[1:])) if len(parts) == 4 else ("127.0.0.1", spec)
    s = socket.socket(socket.AF_INET6 if ":" in bind else socket.AF_INET)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((bind, int(rest.split(":")[0])))
    s.listen()
    sockets.append(s)
time.sleep(60)
"#;

/// Detached `-N` master that listens on every `-L` spec (bind address included) and exits on
/// `-O exit`. Every invocation is logged to `SSHX_ARGS_LOG`.
fn tunnel_ssh(fixture: &Fixture) -> PathBuf {
    let bin = fixture.root.join("tunnel-bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("listen.py"), LISTEN).unwrap();
    script(
        &bin.join("ssh"),
        r#"#!/bin/sh
[ -n "$SSHX_ARGS_LOG" ] && printf '%s\n' "$*" >> "$SSHX_ARGS_LOG"
socket=
previous=
local_specs=
for argument in "$@"; do
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  if [ "$previous" = "-L" ]; then local_specs="$local_specs $argument"; fi
  previous="$argument"
done
listen="$(dirname "$0")/listen.py"
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
    python3 - "$marker" "$local_specs" "$listen" <<'PY' >/dev/null 2>&1 &
import os,sys
os.setsid()
with open(os.devnull) as stdin:
    os.dup2(stdin.fileno(), 0)
os.execl("/bin/sh", "sh", "-c", r'''
marker=$1
local_specs=$2
listen=$3
: > "$marker"
python3 "$listen" $local_specs >/dev/null 2>&1 &
listener=$!
trap 'kill "$listener" 2>/dev/null || :; wait "$listener" 2>/dev/null || :; rm -f "$marker"; exit 0' INT HUP TERM
while :; do sleep 0.01; done
''', "sshx-fixture-master", sys.argv[1], sys.argv[2], sys.argv[3])
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
        .env("SSHX_ARGS_LOG", fixture.root.join("ssh-args"))
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
    seen: Vec<String>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Tui {
    fn drop(&mut self) {
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

const ROWS: u16 = 30;
const COLUMNS: u16 = 100;

fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Tui {
    fn start(fixture: &Fixture, bin: &Path) -> Self {
        let guard = PTY.lock().unwrap_or_else(|error| error.into_inner());
        let mut master = -1;
        let mut slave = -1;
        let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
        size.ws_col = COLUMNS;
        size.ws_row = ROWS;
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
            seen: Vec::new(),
            _guard: guard,
        }
    }

    /// Returns whether any bytes were read.
    fn pump(&mut self) -> bool {
        let mut buffer = [0u8; 4096];
        match self.master.read(&mut buffer) {
            Ok(size) if size > 0 => {
                self.output.extend_from_slice(&buffer[..size]);
                true
            }
            _ => {
                thread::sleep(Duration::from_millis(5));
                false
            }
        }
    }

    /// The screen as the user sees it now, whitespace removed so layout never matters.
    fn screen(&self) -> String {
        support::render(&self.output, usize::from(ROWS), usize::from(COLUMNS))
    }

    /// Waits for `text` (ignoring whitespace) to be on the screen.
    fn expect(&mut self, text: &str) {
        let wanted = squeeze(text);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            let screen = self.screen();
            let found = screen.contains(&wanted);
            if self.seen.last() != Some(&screen) {
                self.seen.push(screen.clone());
            }
            if found {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for `{text}`; screen:\n{screen}"
            );
        }
    }

    /// Whether any screen this terminal showed contained `text`.
    fn saw(&self, text: &str) -> bool {
        let wanted = squeeze(text);
        self.seen.iter().any(|screen| screen.contains(&wanted))
    }

    fn send(&mut self, keys: &str) {
        self.master.write_all(keys.as_bytes()).unwrap();
    }

    fn finish(mut self) -> (ExitStatus, String) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            if let Some(status) = self.child.try_wait().unwrap() {
                // The child can exit before its last output is read; collect what is still buffered.
                while self.pump() {}
                return (status, self.screen());
            }
            assert!(
                Instant::now() < deadline,
                "sshx did not exit; screen:\n{}",
                self.screen()
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
    fn after_master(fixture: &Fixture, bind: &str, ports: &[u16]) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let started = fixture.root.join("master-started");
        let ports = ports.to_vec();
        let bind = bind.to_string();
        let thread = thread::spawn(move || {
            while !started.exists() && !flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            let _bound = ports
                .iter()
                .filter_map(|port| TcpListener::bind((bind.as_str(), *port)).ok())
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
    write_usages(fixture, &[(key, forwards)]);
}

fn write_usages(fixture: &Fixture, hosts: &[(&str, Value)]) {
    let hosts = hosts
        .iter()
        .map(|(key, forwards)| {
            (
                key.to_string(),
                json!({ "last_used": 1, "count": 1, "forwards": forwards }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    fs::write(
        fixture.config.with_file_name(".config.sshx-usage.json"),
        json!({ "version": 1, "hosts": hosts }).to_string(),
    )
    .unwrap();
}

#[cfg(target_os = "macos")]
const CLIPBOARD_TOOL: &str = "pbcopy";
#[cfg(not(target_os = "macos"))]
const CLIPBOARD_TOOL: &str = "wl-copy";

/// A fake clipboard backend next to the fake `ssh`.
fn with_clipboard(bin: PathBuf, fixture: &Fixture) -> PathBuf {
    script(
        &bin.join(CLIPBOARD_TOOL),
        &format!(
            "#!/bin/sh\ncat > '{}'\n",
            fixture.root.join("clipboard").display()
        ),
    );
    bin
}

fn clipboard(fixture: &Fixture) -> Option<String> {
    fs::read_to_string(fixture.root.join("clipboard")).ok()
}

fn set_bind(fixture: &Fixture, bind: &str) {
    let settings = fixture.home.join(".config/sshx/config.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::set_permissions(
        settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(
        settings,
        json!({ "version": 1, "roots": [], "preferences": { "bind": bind } }).to_string(),
    )
    .unwrap();
}

/// A loopback address besides 127.0.0.1 that this host can bind (macOS only has an alias for
/// 127.0.0.2 when the user adds one, but always has ::1).
fn other_loopback() -> Option<&'static str> {
    ["127.0.0.2", "::1"]
        .into_iter()
        .find(|address| TcpListener::bind((*address, 0)).is_ok())
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
    let _listeners = Listeners::after_master(&fixture, "127.0.0.1", &[first, second]);
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
    let _listeners = Listeners::after_master(&fixture, "127.0.0.1", &[local]);
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
    let screen = tui.screen();
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
    tui.expect("db master responsive");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let tunnel = &listed["tunnels"][0];
    assert_eq!(tunnel["state"], "active");
    assert_eq!(tunnel["forwards"][0]["local_port"], offered);
    assert_eq!(tunnel["listener_status"], "bound");
    let id = tunnel["id"].as_str().unwrap().to_string();
    tui.expect(&format!("id {id}"));
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

fn stop_tunnels(fixture: &Fixture, bin: &Path) {
    let listed = cli_json(fixture, bin, &["tunnel", "list", "--format", "json"]);
    for tunnel in listed["tunnels"].as_array().unwrap() {
        let id = tunnel["id"].as_str().unwrap();
        cli_json(
            fixture,
            bin,
            &["tunnel", "stop", id, "--no-input", "--format", "json"],
        );
    }
}

const DB_WITH_SERVICE: &str =
    "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432\n";

#[test]
fn yy_on_the_ports_screen_copies_the_ssh_command_with_the_checked_forwards() {
    let (first, second) = (free_port(), free_port());
    let fixture = fixture(&format!(
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432 LOCAL={first}\n  ##SSHX SERVICE 6379 LOCAL={second}\n"
    ));
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": first, "declared": true}]),
    );
    let bin = with_clipboard(session_ssh(&fixture), &fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("localhost:{second} → 6379"));
    tui.send("yy");
    tui.expect("copied ssh command");
    assert_eq!(
        clipboard(&fixture).unwrap(),
        format!(
            "ssh -F '{}' 'db' -L 127.0.0.1:{first}:127.0.0.1:5432",
            fixture.config.display()
        )
    );
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
}

#[test]
fn yi_copies_the_tunnel_id_on_the_progress_screen_and_the_list() {
    let local = free_port();
    let fixture = fixture(DB_WITH_SERVICE);
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": local, "declared": true}]),
    );
    let bin = with_clipboard(tunnel_ssh(&fixture), &fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("localhost:{local} → 5432"));
    tui.send(" tt");
    tui.expect(&format!(":{local} listener ready"));
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let id = listed["tunnels"][0]["id"].as_str().unwrap().to_string();
    tui.send("yi");
    tui.expect("copied tunnel ID");
    assert_eq!(clipboard(&fixture).as_deref(), Some(id.as_str()));
    fs::remove_file(fixture.root.join("clipboard")).unwrap();
    tui.send("\x1b");
    tui.expect("x toggle");
    tui.send(" tl");
    tui.expect("standalone tunnels");
    tui.send("yi");
    tui.expect("copied tunnel ID");
    assert_eq!(clipboard(&fixture).as_deref(), Some(id.as_str()));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    stop_tunnels(&fixture, &bin);
}

#[test]
fn starting_a_tunnel_whose_exact_request_is_active_shows_the_existing_tunnel() {
    let local = free_port();
    let fixture = fixture(DB_WITH_SERVICE);
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": local, "declared": true}]),
    );
    let bin = tunnel_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("localhost:{local} → 5432"));
    tui.send(" tt");
    tui.expect(&format!(":{local} listener ready"));
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let id = listed["tunnels"][0]["id"].as_str().unwrap().to_string();
    tui.send("\x1b");
    tui.expect("x toggle");
    tui.expect(&format!("already active (tunnel {id})"));
    assert!(!tui.saw("port in use"));
    tui.send(" tt");
    tui.expect(&format!("already active as tunnel {id}"));
    tui.expect(&format!(":{local} listener ready"));
    assert!(!tui.saw("pick another port"));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    assert_eq!(listed["tunnels"].as_array().unwrap().len(), 1, "{listed}");
    stop_tunnels(&fixture, &bin);
}

#[test]
fn bind_preference_sets_the_listen_address_of_service_forwards() {
    let Some(bind) = other_loopback() else {
        eprintln!("skipped: this host binds neither 127.0.0.2 nor ::1");
        return;
    };
    let local = free_port();
    let fixture = fixture("##SSHX ID=db-id\nHost db\n  HostName db.example\n");
    set_bind(&fixture, bind);
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": local, "declared": false}]),
    );
    let bin = session_ssh(&fixture);
    let _listeners = Listeners::after_master(&fixture, bind, &[local]);
    let shown = if bind.contains(':') {
        format!("[{bind}]")
    } else {
        bind.to_string()
    };
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("{shown}:{local} → 5432"));
    tui.send("\r");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert_eq!(
        runtime_forwards(&fixture),
        [format!("LocalForward {shown}:{local} 127.0.0.1:5432")]
    );
}

#[test]
fn select_mode_tt_starts_each_row_with_its_remembered_forwards_and_reports_the_rest() {
    let (first, second) = (free_port(), free_port());
    let fixture = fixture(concat!(
        "##SSHX ID=db-id\nHost db\n  HostName db.example\n  ##SSHX SERVICE 5432\n",
        "##SSHX ID=cache-id\nHost cache\n  HostName cache.example\n  ##SSHX SERVICE 6379\n",
        "##SSHX ID=bare-id\nHost bare\n  HostName bare.example\n",
    ));
    write_usages(
        &fixture,
        &[
            (
                "db-id",
                json!([{"remote": 5432, "local": first, "declared": true}]),
            ),
            (
                "cache-id",
                json!([{"remote": 6379, "local": second, "declared": true}]),
            ),
        ],
    );
    let bin = tunnel_ssh(&fixture);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send("Vge");
    tui.expect("3 selected");
    tui.send(" tt");
    tui.expect("bare: no remembered service forwards");
    tui.expect(&format!(":{first} listener ready"));
    tui.expect(&format!(":{second} listener ready"));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let tunnels = listed["tunnels"].as_array().unwrap();
    assert_eq!(tunnels.len(), 2, "{listed}");
    let mut ports = tunnels
        .iter()
        .map(|tunnel| tunnel["forwards"][0]["local_port"].as_u64().unwrap())
        .collect::<Vec<_>>();
    ports.sort_unstable();
    let mut wanted = vec![u64::from(first), u64::from(second)];
    wanted.sort_unstable();
    assert_eq!(ports, wanted);
    stop_tunnels(&fixture, &bin);
}

#[test]
fn u_after_s_moves_usage_back_to_the_old_key() {
    let local = free_port();
    let original = "Host db\n  HostName db.example\n";
    let fixture = fixture(original);
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
    tui.send("u");
    tui.expect("undid declare service 80");
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert_eq!(fs::read_to_string(&fixture.config).unwrap(), original);
    let usage: Value = serde_json::from_slice(
        &fs::read(fixture.config.with_file_name(".config.sshx-usage.json")).unwrap(),
    )
    .unwrap();
    let hosts = usage["hosts"].as_object().unwrap();
    assert_eq!(hosts.keys().collect::<Vec<_>>(), ["db@config"], "{usage}");
}

fn ssh_calls(fixture: &Fixture) -> Vec<String> {
    fs::read_to_string(fixture.root.join("ssh-args"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn start_db_tunnel(fixture: &Fixture, bin: &Path, local: u16) -> (String, String) {
    let selector = format!("5432={local}");
    let started = cli_json(
        fixture,
        bin,
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
    let registry: Value = serde_json::from_slice(
        &fs::read(fixture.home.join(".config/sshx/tunnels/registry.json")).unwrap(),
    )
    .unwrap();
    let socket = registry["tunnels"][0]["control_socket"]
        .as_str()
        .unwrap()
        .to_string();
    (id, socket)
}

#[test]
fn enter_on_a_tunnel_opens_ssh_through_its_control_socket_without_a_new_master() {
    let local = free_port();
    let fixture = fixture(DB_WITH_SERVICE);
    let bin = tunnel_ssh(&fixture);
    let (id, socket) = start_db_tunnel(&fixture, &bin, local);
    let masters_before = ssh_calls(&fixture)
        .iter()
        .filter(|call| call.contains(" -N "))
        .count();
    assert_eq!(masters_before, 1);
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send("gt");
    tui.expect(&id);
    tui.send("\r");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    assert!(output.contains("direct-shell"), "{output}");
    let calls = ssh_calls(&fixture);
    let session = calls
        .iter()
        .filter(|call| !call.contains(" -O ") && !call.contains(" -N "))
        .collect::<Vec<_>>();
    assert_eq!(session.len(), 1, "{calls:?}");
    assert!(session[0].contains(&format!("-S {socket}")), "{session:?}");
    assert!(session[0].contains("ControlMaster=no"), "{session:?}");
    assert!(session[0].ends_with(" db"), "{session:?}");
    assert_eq!(
        calls.iter().filter(|call| call.contains(" -N ")).count(),
        masters_before,
        "{calls:?}"
    );
    stop_tunnels(&fixture, &bin);
}

#[test]
fn enter_on_a_tunnel_whose_master_does_not_answer_refuses_and_runs_no_ssh() {
    let local = free_port();
    let fixture = fixture(DB_WITH_SERVICE);
    let bin = tunnel_ssh(&fixture);
    let (id, socket) = start_db_tunnel(&fixture, &bin, local);
    fs::remove_file(format!("{socket}.started")).unwrap();
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send("gt");
    tui.expect(&id);
    let before = ssh_calls(&fixture).len();
    tui.send("\r");
    tui.expect("TUNNEL_DOWN");
    assert!(!tui.saw("direct-shell"));
    let calls = ssh_calls(&fixture);
    assert!(
        calls[before..]
            .iter()
            .all(|call| call.contains(" -O check ")),
        "{calls:?}"
    );
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    stop_tunnels(&fixture, &bin);
}

#[test]
fn tunnel_mode_listens_on_the_configured_loopback_bind() {
    let Some(bind) = other_loopback() else {
        eprintln!("skipped: this host binds neither 127.0.0.2 nor ::1");
        return;
    };
    let local = free_port();
    let fixture = fixture(DB_WITH_SERVICE);
    set_bind(&fixture, bind);
    write_usage(
        &fixture,
        "db-id",
        json!([{"remote": 5432, "local": local, "declared": true}]),
    );
    let bin = tunnel_ssh(&fixture);
    let shown = if bind.contains(':') {
        format!("[{bind}]")
    } else {
        bind.to_string()
    };
    let mut tui = Tui::start(&fixture, &bin);
    tui.expect("Search:");
    tui.send("db\t");
    tui.expect(&format!("{shown}:{local} → 5432"));
    tui.send(" tt");
    tui.expect(&format!(":{local} listener ready"));
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "{status:?} {output}");
    let listed = cli_json(&fixture, &bin, &["tunnel", "list", "--format", "json"]);
    let tunnel = &listed["tunnels"][0];
    assert_eq!(tunnel["forwards"][0]["bind_address"], bind, "{listed}");
    assert_eq!(tunnel["listener_status"], "bound", "{listed}");
    stop_tunnels(&fixture, &bin);
}
