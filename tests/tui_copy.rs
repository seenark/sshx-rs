#![cfg(all(unix, any(target_os = "macos", target_os = "linux")))]

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
const CLIPBOARD_TOOL: &str = "pbcopy";
#[cfg(target_os = "linux")]
const CLIPBOARD_TOOL: &str = "wl-copy";

const DIRECT: &str = "Host direct\n  HostName direct.example\n  ##PASSWORD s3cr3t-pw\n";
const PAIR: &str = concat!(
    "##SSHX ID=11111111-1111-4111-8111-111111111111\n",
    "##SSHX VM=22222222-2222-4222-8222-222222222222\n",
    "Host gateway\n  HostName gateway.example\n  LocalForward 2200 vm.internal:22\n",
    "##SSHX ID=22222222-2222-4222-8222-222222222222\n",
    "##SSHX GATEWAY=11111111-1111-4111-8111-111111111111\n",
    "##SSHX TRANSIT=vm.internal:22\n",
    "Host vm\n  HostName vm.internal\n  Port 22\n",
);
const QUIET: &str = r#"{"password_warning_acknowledged":true,"clipboard_clear":null}"#;

const FAKE_SSH: &str = r#"#!/bin/sh
config=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  previous="$argument"
done
case " $* " in
  *" -O check "*) [ -f "$SSHX_STARTED" ] && exit 0; exit 1 ;;
  *" -O exit "*) rm -f "$SSHX_STARTED"; exit 0 ;;
  *" -N "*)
    printf 'master\n' >> "$SSHX_MASTER_LOG"
    if [ "$SSHX_AUTH_FAIL" = "1" ]; then
      echo "Permission denied (publickey)." >&2
      exit 5
    fi
    : > "$SSHX_STARTED"
    trap 'exit 0' INT HUP TERM
    while :; do sleep 1; done
    ;;
  *) printf 'direct-shell\n' ;;
esac
"#;

const FAKE_PAIRED_SSH: &str = r#"#!/bin/sh
config=
socket=
previous=
for argument in "$@"; do
  if [ "$previous" = "-F" ]; then config="$argument"; fi
  if [ "$previous" = "-S" ]; then socket="$argument"; fi
  previous="$argument"
done
case " $* " in
  *" -O check "*) [ -f "$socket.started" ] && exit 0; exit 1 ;;
  *" -O exit "*) rm -f "$socket.started"; exit 0 ;;
  *" -N "*)
    if grep -q '^Host vm$' "$config"; then role=VM; else role=GATEWAY; fi
    printf '%s\n' "$role" >> "$SSHX_MASTER_LOG"
    if [ "$SSHX_PAIRED_FAIL_ROLE" = "VM" ] && [ "$role" = "VM" ]; then
      echo "Permission denied (publickey)." >&2
      exit 5
    fi
    : > "$socket.started"
    trap 'exit 0' INT HUP TERM
    while :; do sleep 1; done
    ;;
  *) printf 'paired-shell\n' ;;
esac
"#;

static COUNTER: AtomicU64 = AtomicU64::new(0);
static PTY: Mutex<()> = Mutex::new(());

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(config: &str, ssh: &str, preferences: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sshx-tui-copy-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let home = root.join("home");
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let home = root.join("home");
        let config_path = home.join(".ssh/config");
        fs::write(&config_path, config).unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
        executable(&root.join("bin/ssh"), ssh);
        executable(
            &root.join("bin").join(CLIPBOARD_TOOL),
            "#!/bin/sh\ncat > \"$SSHX_CLIPBOARD_CAPTURE\"\n",
        );
        fs::create_dir_all(home.join(".config/sshx")).unwrap();
        fs::write(
            home.join(".config/sshx/config.json"),
            format!("{{\"version\":1,\"roots\":[],\"preferences\":{preferences}}}\n"),
        )
        .unwrap();
        Self {
            root,
            home,
            config: config_path,
        }
    }

    fn clipboard(&self) -> Option<String> {
        fs::read_to_string(self.root.join("clipboard")).ok()
    }

    fn master_starts(&self) -> usize {
        fs::read_to_string(self.root.join("masters")).map_or(0, |log| log.lines().count())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Tui {
    child: Child,
    master: fs::File,
    output: Vec<u8>,
    mark: usize,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Tui {
    fn start(fixture: &Fixture, env: &[(&str, &str)]) -> Self {
        let serial = PTY.lock().unwrap_or_else(|error| error.into_inner());
        let (mut master, mut slave) = (-1, -1);
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
        assert_eq!(opened, 0, "openpty should succeed");
        let slave = unsafe { fs::File::from_raw_fd(slave) };
        let path = format!(
            "{}:{}",
            fixture.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_sshx"));
        command
            .env("HOME", &fixture.home)
            .env("PATH", path)
            .env("SSHX_STARTED", fixture.root.join("started"))
            .env("SSHX_MASTER_LOG", fixture.root.join("masters"))
            .env("SSHX_CLIPBOARD_CAPTURE", fixture.root.join("clipboard"))
            .args(["--config", fixture.config.to_str().unwrap()])
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        for (name, value) in env {
            command.env(name, value);
        }
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
        let child = command.spawn().expect("sshx should start in a pty");
        let master = unsafe { fs::File::from_raw_fd(master) };
        let flags = unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&master), libc::F_GETFL) };
        let nonblocking = unsafe {
            libc::fcntl(
                std::os::fd::AsRawFd::as_raw_fd(&master),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        };
        assert_eq!(nonblocking, 0);
        Self {
            child,
            master,
            output: Vec::new(),
            mark: 0,
            _serial: serial,
        }
    }

    fn pump(&mut self) -> bool {
        let mut buffer = [0u8; 4096];
        match self.master.read(&mut buffer) {
            Ok(size) if size > 0 => {
                self.output.extend_from_slice(&buffer[..size]);
                true
            }
            _ => false,
        }
    }

    /// Waits until `text` is on screen after the last `send`; whitespace is ignored.
    fn expect(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            let needle = squeeze(text.as_bytes());
            if squeeze(&self.output[self.mark..])
                .windows(needle.len())
                .any(|window| window == needle)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "did not see {text:?}: {}",
                String::from_utf8_lossy(&strip_ansi(&self.output))
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "sshx exited before {text:?}: {}",
                String::from_utf8_lossy(&strip_ansi(&self.output))
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&mut self, keys: &str) {
        std::thread::sleep(Duration::from_millis(60));
        while self.pump() {}
        self.mark = self.output.len();
        self.master.write_all(keys.as_bytes()).unwrap();
    }

    fn finish(mut self) -> (ExitStatus, String) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            self.pump();
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "sshx did not exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        while self.pump() {}
        let output = String::from_utf8_lossy(&strip_ansi(&self.output)).into_owned();
        (status, output)
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

fn squeeze(bytes: &[u8]) -> Vec<u8> {
    strip_ansi(bytes)
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect()
}

fn strip_ansi(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] != 0x1b {
            if input[index] != b'\r' {
                output.push(input[index]);
            }
            index += 1;
            continue;
        }
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
            Some(_) => index += 1,
            None => {}
        }
    }
    output
}

/// Starts the finder in normal mode with one selected row.
fn normal_finder(fixture: &Fixture, env: &[(&str, &str)]) -> Tui {
    let mut tui = Tui::start(fixture, env);
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui
}

fn quit(mut tui: Tui) -> String {
    tui.send("q");
    let (status, output) = tui.finish();
    assert!(status.success(), "status={status:?} output={output}");
    output
}

#[test]
fn yy_copies_the_ssh_command_for_the_config_root() {
    let fixture = Fixture::new(DIRECT, FAKE_SSH, QUIET);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yy");
    tui.expect("✓ copied ssh command");
    quit(tui);
    assert_eq!(
        fixture.clipboard().unwrap(),
        format!("ssh -F '{}' 'direct'", fixture.config.display())
    );
}

#[test]
fn yp_copies_the_space_prefixed_sshpass_command_without_rendering_the_password() {
    let fixture = Fixture::new(DIRECT, FAKE_SSH, QUIET);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yp");
    tui.expect("✓ copied sshpass command");
    let output = quit(tui);
    assert_eq!(
        fixture.clipboard().unwrap(),
        format!(
            " SSHPASS='s3cr3t-pw' sshpass -e ssh -F '{}' 'direct'",
            fixture.config.display()
        )
    );
    assert!(!output.contains("s3cr3t-pw"), "{output}");
}

#[test]
fn yp_is_refused_without_a_stored_password() {
    let fixture = Fixture::new("Host direct\n  HostName direct.example\n", FAKE_SSH, QUIET);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yp");
    tui.expect("no stored password");
    quit(tui);
    assert_eq!(fixture.clipboard(), None);
}

#[test]
fn yw_copies_the_password_and_never_renders_it() {
    let fixture = Fixture::new(DIRECT, FAKE_SSH, QUIET);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yw");
    tui.expect("✓ copied password");
    let output = quit(tui);
    assert_eq!(fixture.clipboard().unwrap(), "s3cr3t-pw");
    assert!(!output.contains("s3cr3t-pw"), "{output}");
}

#[test]
fn first_password_copy_warns_once_and_persists_the_acknowledgment() {
    let fixture = Fixture::new(DIRECT, FAKE_SSH, r#"{"clipboard_clear":null}"#);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yw");
    tui.expect("Press Enter to copy");
    tui.send("\n");
    tui.expect("✓ copied password");
    tui.send("yw");
    tui.expect("✓ copied password");
    let output = quit(tui);
    assert_eq!(
        output.matches("Clipboard history may retain").count(),
        1,
        "{output}"
    );
    assert_eq!(output.matches("Press Enter to copy").count(), 1, "{output}");
    let settings = fs::read_to_string(fixture.home.join(".config/sshx/config.json")).unwrap();
    assert!(
        settings.contains("\"password_warning_acknowledged\": true")
            || settings.contains("\"password_warning_acknowledged\":true"),
        "{settings}"
    );
}

#[test]
fn password_confirm_gates_the_copy_on_y() {
    let fixture = Fixture::new(
        DIRECT,
        FAKE_SSH,
        r#"{"password_confirm":true,"password_warning_acknowledged":true,"clipboard_clear":null}"#,
    );
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yw");
    tui.expect("[y/N]");
    tui.send("n\n");
    tui.expect("password not copied");
    assert_eq!(fixture.clipboard(), None);
    tui.send("yw");
    tui.expect("[y/N]");
    tui.send("y\n");
    tui.expect("✓ copied password");
    quit(tui);
    assert_eq!(fixture.clipboard().unwrap(), "s3cr3t-pw");
}

#[test]
fn yh_copies_the_host_name() {
    let fixture = Fixture::new(DIRECT, FAKE_SSH, QUIET);
    let mut tui = normal_finder(&fixture, &[]);
    tui.send("yh");
    tui.expect("✓ copied HostName");
    quit(tui);
    assert_eq!(fixture.clipboard().unwrap(), "direct.example");
}

#[test]
fn yy_on_a_pair_copies_the_sshx_form() {
    let fixture = Fixture::new(PAIR, FAKE_PAIRED_SSH, QUIET);
    let mut tui = Tui::start(&fixture, &[]);
    tui.expect("Search:");
    tui.send("vm");
    tui.send("\x1b");
    tui.expect("NORMAL");
    tui.send("yy");
    tui.expect("✓ copied ssh command");
    quit(tui);
    assert_eq!(
        fixture.clipboard().unwrap(),
        format!(
            "sshx connect 'vm' --id '22222222-2222-4222-8222-222222222222' --config '{}'",
            fixture.config.display()
        )
    );
}

#[test]
fn auth_failure_shows_the_failure_screen_and_retry_reruns_ssh() {
    let fixture = Fixture::new("Host direct\n  HostName direct.example\n", FAKE_SSH, QUIET);
    let mut tui = Tui::start(&fixture, &[("SSHX_AUTH_FAIL", "1")]);
    tui.expect("Search:");
    tui.send("\r");
    tui.expect("Password for direct host");
    tui.send("\n");
    tui.expect("ssh direct failed");
    tui.expect("Permission denied (publickey)");
    assert_eq!(fixture.master_starts(), 1);
    tui.send("l");
    tui.expect("log:");
    tui.send("r");
    tui.expect("Password for direct host");
    tui.send("\n");
    tui.expect("ssh direct failed");
    assert_eq!(fixture.master_starts(), 2);
    tui.send("\x1b");
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    quit(tui);
}

#[test]
fn failure_screen_copies_the_verbose_ssh_command() {
    let fixture = Fixture::new("Host direct\n  HostName direct.example\n", FAKE_SSH, QUIET);
    let mut tui = Tui::start(&fixture, &[("SSHX_AUTH_FAIL", "1")]);
    tui.expect("Search:");
    tui.send("\r");
    tui.expect("Password for direct host");
    tui.send("\n");
    tui.expect("ssh direct failed");
    tui.send("y");
    tui.expect("✓ copied ssh -v command");
    assert_eq!(
        fixture.clipboard().unwrap(),
        format!("ssh -v -F '{}' 'direct'", fixture.config.display())
    );
    tui.send("\x1b");
    tui.expect("NORMAL");
    quit(tui);
}

#[test]
fn failure_screen_edit_returns_to_the_finder_with_a_hint() {
    let fixture = Fixture::new("Host direct\n  HostName direct.example\n", FAKE_SSH, QUIET);
    let mut tui = Tui::start(&fixture, &[("SSHX_AUTH_FAIL", "1")]);
    tui.expect("Search:");
    tui.send("\r");
    tui.expect("Password for direct host");
    tui.send("\n");
    tui.expect("ssh direct failed");
    tui.send("e");
    tui.expect("press cc to edit");
    tui.send("\x1b");
    tui.expect("NORMAL");
    quit(tui);
}

#[test]
fn pair_vm_failure_names_the_vm_hop() {
    let fixture = Fixture::new(PAIR, FAKE_PAIRED_SSH, QUIET);
    let mut tui = Tui::start(&fixture, &[("SSHX_PAIRED_FAIL_ROLE", "VM")]);
    tui.expect("Search:");
    tui.send("vm");
    tui.send("\r");
    tui.expect("Password for VM");
    tui.send("\n");
    tui.expect("ssh vm failed");
    tui.expect("gateway gateway → VM vm ✗");
    tui.expect("Permission denied (publickey)");
    tui.send("\x1b");
    tui.expect("Search:");
    tui.send("\x1b");
    tui.expect("NORMAL");
    quit(tui);
}
