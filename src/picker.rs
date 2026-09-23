use sshx::discovery::HostEntry;
use std::cmp::Ordering;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;

pub const CANCELLED: &str = "PICKER_CANCELLED";

pub struct Selection<'a> {
    pub entry: &'a HostEntry,
    pub alias: &'a str,
}

struct Row<'a> {
    entry: &'a HostEntry,
    alias: &'a str,
}

struct RawMode {
    fd: libc::c_int,
    original: libc::termios,
}

impl RawMode {
    fn enter(fd: libc::c_int) -> Result<Self, String> {
        let mut original = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(format!(
                "HOST_REQUIRED: cannot inspect interactive terminal: {}",
                io::Error::last_os_error()
            ));
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_iflag &= !(libc::ICRNL | libc::IXON);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(format!(
                "HOST_REQUIRED: cannot configure interactive terminal: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self { fd, original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

pub fn select<'a>(entries: &[&'a HostEntry], label: &str) -> Result<Selection<'a>, String> {
    if entries.is_empty() {
        return Err("HOST_NOT_FOUND: no hosts match current filters".to_string());
    }

    with_terminal(
        format!(
            "HOST_REQUIRED: {label} requires a HostEntry selector with a usable interactive terminal"
        ),
        |input, error, fd| {
            let mut query = String::new();
            let mut selected = 0usize;

            loop {
                let rows = matching_rows(entries, &query);
                if rows.is_empty() {
                    selected = 0;
                } else {
                    selected = selected.min(rows.len() - 1);
                }
                render(error, &rows, &query, selected, label)?;

                match read_event(input, fd, "HOST_REQUIRED", "HOST_REQUIRED")? {
                    PickerEvent::Enter => {
                        if let Some(row) = rows.get(selected) {
                            return Ok(Selection {
                                entry: row.entry,
                                alias: row.alias,
                            });
                        }
                    }
                    PickerEvent::Cancel => return Err(CANCELLED.to_string()),
                    PickerEvent::Up => selected = selected.saturating_sub(1),
                    PickerEvent::Down if selected + 1 < rows.len() => selected += 1,
                    PickerEvent::Backspace => {
                        query.pop();
                    }
                    PickerEvent::Character(byte) => {
                        query.push(byte as char);
                        selected = 0;
                    }
                    PickerEvent::Down | PickerEvent::Ignore => {}
                }
            }
        },
    )
}

pub fn select_menu(options: &[&str], label: &str) -> Result<usize, String> {
    if options.is_empty() {
        return Err("ACTION_UNAVAILABLE: no host actions are available".to_string());
    }

    with_terminal(
        format!("ACTION_REQUIRED: {label} requires a usable interactive terminal"),
        |input, error, fd| {
            let mut selected = 0usize;

            loop {
                render_menu(error, options, selected, label)?;

                match read_event(input, fd, "ACTION_REQUIRED", "HOST_REQUIRED")? {
                    PickerEvent::Enter => return Ok(selected),
                    PickerEvent::Cancel => return Err(CANCELLED.to_string()),
                    PickerEvent::Up => selected = selected.saturating_sub(1),
                    PickerEvent::Down if selected + 1 < options.len() => selected += 1,
                    PickerEvent::Down
                    | PickerEvent::Backspace
                    | PickerEvent::Character(_)
                    | PickerEvent::Ignore => {}
                }
            }
        },
    )
}

fn with_terminal<T>(
    not_terminal_error: String,
    run: impl FnOnce(&mut io::Stdin, &mut io::Stderr, libc::c_int) -> Result<T, String>,
) -> Result<T, String> {
    let input = io::stdin();
    if !input.is_terminal() {
        return Err(not_terminal_error);
    }

    let fd = input.as_raw_fd();
    let _raw_mode = RawMode::enter(fd)?;
    let mut input = input;
    let mut error = io::stderr();
    let result = run(&mut input, &mut error, fd);
    if result.is_ok() || matches!(&result, Err(error) if error == CANCELLED) {
        clear(&mut error)?;
    }
    result
}

enum PickerEvent {
    Enter,
    Cancel,
    Up,
    Down,
    Backspace,
    Character(u8),
    Ignore,
}

fn read_event(
    input: &mut impl Read,
    fd: libc::c_int,
    input_error_prefix: &str,
    escape_error_prefix: &str,
) -> Result<PickerEvent, String> {
    let mut byte = [0u8; 1];
    input
        .read_exact(&mut byte)
        .map_err(|error| format!("{input_error_prefix}: cannot read picker input: {error}"))?;

    match byte[0] {
        b'\r' | b'\n' => Ok(PickerEvent::Enter),
        0x03 => Ok(PickerEvent::Cancel),
        0x1b => {
            if !arrow_key(input, fd, escape_error_prefix)? {
                return Ok(PickerEvent::Cancel);
            }
            match read_arrow(input, escape_error_prefix)? {
                b'A' => Ok(PickerEvent::Up),
                b'B' => Ok(PickerEvent::Down),
                _ => Ok(PickerEvent::Ignore),
            }
        }
        0x08 | 0x7f => Ok(PickerEvent::Backspace),
        byte if byte.is_ascii_graphic() || byte == b' ' => Ok(PickerEvent::Character(byte)),
        _ => Ok(PickerEvent::Ignore),
    }
}

fn arrow_key(input: &mut impl Read, fd: libc::c_int, error_prefix: &str) -> Result<bool, String> {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll, 1, 30) };
    if result < 0 {
        return Err(format!(
            "{error_prefix}: cannot read picker input: {}",
            io::Error::last_os_error()
        ));
    }
    if result == 0 {
        return Ok(false);
    }
    let mut prefix = [0u8; 1];
    input
        .read_exact(&mut prefix)
        .map_err(|error| format!("{error_prefix}: cannot read picker input: {error}"))?;
    Ok(prefix[0] == b'[')
}

fn read_arrow(input: &mut impl Read, error_prefix: &str) -> Result<u8, String> {
    let mut direction = [0u8; 1];
    input
        .read_exact(&mut direction)
        .map_err(|error| format!("{error_prefix}: cannot read picker input: {error}"))?;
    Ok(direction[0])
}

fn matching_rows<'a>(entries: &[&'a HostEntry], query: &str) -> Vec<Row<'a>> {
    let mut rows = entries
        .iter()
        .flat_map(|entry| {
            entry.aliases.iter().map(move |alias| Row {
                entry,
                alias: alias.as_str(),
            })
        })
        .filter(|row| row_rank(row, query).is_some())
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| compare_rows(left, right, query));
    rows
}

fn compare_rows(left: &Row<'_>, right: &Row<'_>, query: &str) -> Ordering {
    row_rank(left, query)
        .cmp(&row_rank(right, query))
        .then_with(|| {
            left.alias
                .to_ascii_lowercase()
                .cmp(&right.alias.to_ascii_lowercase())
        })
        .then_with(|| left.entry.source.path.cmp(&right.entry.source.path))
        .then_with(|| {
            left.entry
                .source
                .line_start
                .cmp(&right.entry.source.line_start)
        })
        .then_with(|| left.entry.id.cmp(&right.entry.id))
}

fn row_rank(row: &Row<'_>, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    let query = query.to_ascii_lowercase();
    let alias = row.alias.to_ascii_lowercase();
    if alias == query {
        return Some((0, 0));
    }
    if alias.starts_with(&query) {
        return Some((1, 0));
    }

    let score = std::iter::once(fuzzy_score(&alias, &query))
        .chain(
            row.entry
                .destination
                .iter()
                .map(|destination| fuzzy_score(destination, &query)),
        )
        .chain(
            row.entry
                .projects
                .iter()
                .map(|project| fuzzy_score(project, &query)),
        )
        .flatten()
        .min();
    score.map(|score| (2, score))
}

fn fuzzy_score(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut cursor = 0usize;
    let mut previous = None;
    let mut score = 0usize;
    for needle in needle.chars() {
        let position = haystack[cursor..].find(needle)? + cursor;
        if let Some(previous) = previous {
            score += position.saturating_sub(previous + 1);
        }
        score += position;
        previous = Some(position);
        cursor = position + needle.len_utf8();
    }
    Some(score)
}

fn render(
    error: &mut impl Write,
    rows: &[Row<'_>],
    query: &str,
    selected: usize,
    label: &str,
) -> Result<(), String> {
    write!(
        error,
        "\x1b[2J\x1b[Hsshx {label} picker\nSearch: {query}\n\n"
    )
    .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))?;
    if rows.is_empty() {
        writeln!(error, "  No matching HostEntry aliases.")
            .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))?;
    } else {
        for (index, row) in rows.iter().enumerate() {
            let marker = if index == selected { ">" } else { " " };
            let destination = row.entry.destination.as_deref().unwrap_or("-");
            let project = if row.entry.projects.is_empty() {
                "-".to_string()
            } else {
                row.entry.projects.join(",")
            };
            let scope = if row.entry.scopes.is_empty() {
                "-".to_string()
            } else {
                row.entry.scopes.join(",")
            };
            writeln!(
                error,
                "{marker} {}  {}  project={} scope={}  {}:{}",
                row.alias,
                destination,
                project,
                scope,
                row.entry.source.path,
                row.entry.source.line_start
            )
            .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))?;
        }
    }
    writeln!(error, "\nArrow keys move, Enter selects, Esc cancels.")
        .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))?;
    error
        .flush()
        .map_err(|error| format!("HOST_REQUIRED: cannot render picker: {error}"))
}

fn render_menu(
    error: &mut impl Write,
    options: &[&str],
    selected: usize,
    label: &str,
) -> Result<(), String> {
    write!(error, "\x1b[2J\x1b[Hsshx {label}\n\n")
        .map_err(|error| format!("ACTION_REQUIRED: cannot render menu: {error}"))?;
    for (index, option) in options.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        writeln!(error, "{marker} {option}")
            .map_err(|error| format!("ACTION_REQUIRED: cannot render menu: {error}"))?;
    }
    writeln!(error, "\nArrow keys move, Enter selects, Esc cancels.")
        .map_err(|error| format!("ACTION_REQUIRED: cannot render menu: {error}"))?;
    error
        .flush()
        .map_err(|error| format!("ACTION_REQUIRED: cannot render menu: {error}"))
}

fn clear(error: &mut impl Write) -> Result<(), String> {
    write!(error, "\x1b[2J\x1b[H")
        .and_then(|_| error.flush())
        .map_err(|error| format!("HOST_REQUIRED: cannot clear picker: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{PickerEvent, fuzzy_score, matching_rows, read_event};
    use sshx::discovery::{HostEntry, SourceIdentity};
    use std::io::Cursor;
    #[test]
    fn event_decoder_preserves_enter_and_ctrl_c() {
        let mut enter = Cursor::new(vec![b'\n']);
        assert!(matches!(
            read_event(&mut enter, -1, "HOST_REQUIRED", "HOST_REQUIRED"),
            Ok(PickerEvent::Enter)
        ));

        let mut cancel = Cursor::new(vec![0x03]);
        assert!(matches!(
            read_event(&mut cancel, -1, "ACTION_REQUIRED", "HOST_REQUIRED"),
            Ok(PickerEvent::Cancel)
        ));
    }

    fn host(
        id: &str,
        alias: &str,
        destination: Option<&str>,
        project: Option<&str>,
        path: &str,
        line: usize,
    ) -> HostEntry {
        HostEntry {
            id: id.to_string(),
            aliases: vec![alias.to_string()],
            source: SourceIdentity {
                path: path.to_string(),
                byte_start: 0,
                byte_end: 1,
                line_start: line,
                line_end: line,
            },
            destination: destination.map(str::to_string),
            provenance: Vec::new(),
            scopes: Vec::new(),
            projects: project.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn fuzzy_score_requires_ordered_subsequence() {
        assert!(fuzzy_score("production", "pdt").is_some());
        assert!(fuzzy_score("production", "z").is_none());
    }

    #[test]
    fn destination_and_project_queries_match_without_alias_match() {
        let destination = host(
            "destination",
            "alias",
            Some("prod.example"),
            None,
            "/config",
            1,
        );
        let project = host("project", "alias", None, Some("work"), "/config", 2);
        assert_eq!(
            matching_rows(&[&destination], "prod")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
        assert_eq!(
            matching_rows(&[&project], "work")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
    }

    #[test]
    fn exact_prefix_fuzzy_and_empty_query_order_is_deterministic() {
        let exact = host("exact", "alpha", Some("same"), None, "/b", 2);
        let prefix = host("prefix", "alphabet", Some("same"), None, "/a", 1);
        let fuzzy = host("fuzzy", "zzz", Some("alpha-target"), None, "/c", 1);
        let rows = matching_rows(&[&prefix, &fuzzy, &exact], "alpha");
        assert_eq!(
            rows.iter().map(|row| row.alias).collect::<Vec<_>>(),
            ["alpha", "alphabet", "zzz"]
        );

        let first = host("first", "zeta", None, None, "/b", 2);
        let second = host("second", "Zeta", None, None, "/a", 1);
        let rows = matching_rows(&[&first, &second], "");
        assert_eq!(
            rows.iter()
                .map(|row| row.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
    }
}
