use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::Rect,
};
use std::fs::File;
use std::io::{self, IsTerminal, Read};
use std::os::fd::{AsRawFd, FromRawFd};

pub(super) type AppTerminal = Terminal<CrosstermBackend<io::Stderr>>;

struct TerminalGuard {
    raw_mode: bool,
    alternate_screen: bool,
    cursor_hidden: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        let mut stderr = io::stderr();
        if self.alternate_screen {
            let _ = execute!(stderr, LeaveAlternateScreen);
        }
        if self.cursor_hidden {
            let _ = execute!(stderr, crossterm::cursor::Show);
        }
    }
}

pub(super) fn with_terminal<T>(
    label: &str,
    not_terminal_error: String,
    run: impl FnOnce(&mut AppTerminal, &mut File) -> Result<T, String>,
) -> Result<T, String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(not_terminal_error);
    }

    enable_raw_mode()
        .map_err(|error| format!("HOST_REQUIRED: cannot enable terminal input: {error}"))?;
    let mut guard = TerminalGuard {
        raw_mode: true,
        alternate_screen: false,
        cursor_hidden: false,
    };
    let mut terminal = AppTerminal::new(CrosstermBackend::new(io::stderr()))
        .map_err(|error| format!("HOST_REQUIRED: cannot initialize terminal: {error}"))?;
    guard.cursor_hidden = true;
    let fallback = terminal
        .backend()
        .size()
        .is_ok_and(|area| area.width == 0 || area.height == 0);
    if fallback {
        let _ = terminal.resize(Rect::new(0, 0, 80, 24));
    }
    execute!(terminal.backend_mut(), EnterAlternateScreen)
        .map_err(|error| format!("HOST_REQUIRED: cannot enter alternate screen: {error}"))?;
    guard.alternate_screen = true;
    if fallback {
        eprint!("sshx {label} picker");
    }
    let input_fd = unsafe { libc::dup(io::stdin().as_raw_fd()) };
    if input_fd < 0 {
        return Err(format!(
            "HOST_REQUIRED: cannot duplicate interactive terminal: {}",
            io::Error::last_os_error()
        ));
    }
    let mut input = unsafe { File::from_raw_fd(input_fd) };

    let result = run(&mut terminal, &mut input);
    let restore = disable_raw_mode().and_then(|_| {
        guard.raw_mode = false;
        execute!(terminal.backend_mut(), LeaveAlternateScreen).and_then(|_| {
            guard.alternate_screen = false;
            terminal.show_cursor().map(|()| guard.cursor_hidden = false)
        })
    });
    match (result, restore) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("HOST_REQUIRED: cannot restore terminal: {error}")),
        (Err(error), Err(restore_error)) => {
            Err(format!("{error}; cannot restore terminal: {restore_error}"))
        }
    }
}

pub(super) fn read_key(input: &mut File, prefix: &str) -> Result<KeyEvent, String> {
    let mut byte = [0u8; 1];
    input
        .read_exact(&mut byte)
        .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
    let code = match byte[0] {
        b'\r' | b'\n' => KeyCode::Enter,
        b'\t' => KeyCode::Tab,
        0x03 => return Ok(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        0x0e => return Ok(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        0x10 => return Ok(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        0x13 => return Ok(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        0x18 => return Ok(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        0x19 => return Ok(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
        0x14 => return Ok(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)),
        0x04 => return Ok(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        0x15 => return Ok(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        0x12 => return Ok(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        0x08 | 0x7f => KeyCode::Backspace,
        0x1b => {
            let Some(prefix_byte) = read_escape_byte(input, prefix)? else {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            };
            if prefix_byte == b'O' {
                return Ok(match read_escape_byte(input, prefix)? {
                    Some(b'P') => KeyEvent::new(KeyCode::F(1), KeyModifiers::empty()),
                    _ => KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
                });
            }
            if prefix_byte != b'[' {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            }
            let Some(sequence_byte) = read_escape_byte(input, prefix)? else {
                return Ok(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
            };
            match sequence_byte {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'Z' => KeyCode::BackTab,
                b'5' if read_escape_byte(input, prefix)? == Some(b'~') => KeyCode::PageUp,
                b'6' if read_escape_byte(input, prefix)? == Some(b'~') => KeyCode::PageDown,
                b'1' if read_escape_byte(input, prefix)? == Some(b'1')
                    && read_escape_byte(input, prefix)? == Some(b'~') =>
                {
                    KeyCode::F(1)
                }
                _ => KeyCode::Esc,
            }
        }
        byte if byte.is_ascii_graphic() || byte == b' ' => KeyCode::Char(byte as char),
        byte if byte >= 0x80 => {
            let length = match byte {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(format!("{prefix}: invalid UTF-8 terminal input")),
            };
            let mut sequence = [0u8; 4];
            sequence[0] = byte;
            input
                .read_exact(&mut sequence[1..length])
                .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
            let text = std::str::from_utf8(&sequence[..length])
                .map_err(|error| format!("{prefix}: invalid UTF-8 terminal input: {error}"))?;
            let character = text
                .chars()
                .next()
                .ok_or_else(|| format!("{prefix}: invalid UTF-8 terminal input"))?;
            KeyCode::Char(character)
        }
        _ => return read_key(input, prefix),
    };
    Ok(KeyEvent::new(code, KeyModifiers::empty()))
}
fn read_escape_byte(input: &mut File, prefix: &str) -> Result<Option<u8>, String> {
    let mut poll = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut poll, 1, 30) } <= 0 {
        return Ok(None);
    }
    let mut byte = [0u8; 1];
    input
        .read_exact(&mut byte)
        .map_err(|error| format!("{prefix}: cannot read terminal input: {error}"))?;
    Ok(Some(byte[0]))
}

#[cfg(test)]
mod tests {
    use super::read_key;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn decode(bytes: &[u8]) -> KeyEvent {
        let path = std::env::temp_dir().join(format!(
            "sshx-picker-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes).unwrap();
        let event = read_key(&mut std::fs::File::open(&path).unwrap(), "TEST").unwrap();
        std::fs::remove_file(path).unwrap();
        event
    }

    #[test]
    fn read_key_decodes_ctrl_p() {
        assert_eq!(
            decode(&[0x10]),
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn read_key_decodes_both_f1_encodings() {
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::empty());
        assert_eq!(decode(b"\x1bOP"), f1);
        assert_eq!(decode(b"\x1b[11~"), f1);
    }
}
