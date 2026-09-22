use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct SourceIdentity {
    pub path: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Provenance {
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HostEntry {
    pub id: String,
    pub aliases: Vec<String>,
    pub source: SourceIdentity,
    pub destination: Option<String>,
    pub provenance: Vec<Provenance>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub entries: Vec<HostEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub struct DiscoveryError {
    message: String,
}

impl DiscoveryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for DiscoveryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DiscoveryError {}

#[derive(Clone, Debug)]
struct Line {
    number: usize,
    start: usize,
    end: usize,
    text: String,
}

#[derive(Clone, Debug)]
struct HostBlock {
    line_index: usize,
    aliases: Vec<String>,
    destination: Option<String>,
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    line_end: usize,
}

#[derive(Clone, Debug)]
enum Item {
    Host(usize),
    Include(Vec<String>),
}

#[derive(Clone, Debug)]
struct ParsedFile {
    hosts: Vec<HostBlock>,
    items: Vec<Item>,
}

#[derive(Default)]
struct State {
    cache: HashMap<PathBuf, ParsedFile>,
    active: Vec<PathBuf>,
    active_set: HashSet<PathBuf>,
    entries: Vec<HostEntry>,
    entry_indexes: HashMap<(PathBuf, usize, usize), usize>,
    diagnostics: Vec<Diagnostic>,
    diagnostic_keys: HashSet<String>,
}

pub fn discover(root: &Path) -> Result<Catalog, DiscoveryError> {
    let root = absolute_path(root)?;
    if !root.is_file() {
        return Err(DiscoveryError::new(format!(
            "config root is not a file: {}",
            root.display()
        )));
    }
    let home_ssh = match std::env::var_os("HOME") {
        Some(home) => absolute_path(&PathBuf::from(home).join(".ssh"))?,
        None => root
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf(),
    };

    let mut state = State::default();
    let chain = vec![display_path(&root)];
    visit_file(&root, &home_ssh, chain, &mut state)?;
    Ok(Catalog {
        entries: state.entries,
        diagnostics: state.diagnostics,
    })
}

fn visit_file(
    path: &Path,
    include_base: &Path,
    chain: Vec<String>,
    state: &mut State,
) -> Result<(), DiscoveryError> {
    let identity = fs::canonicalize(path).map_err(|error| {
        DiscoveryError::new(format!(
            "cannot read included config {}: {error}",
            path.display()
        ))
    })?;

    if state.active_set.contains(&identity) {
        add_cycle_diagnostic(&identity, state);
        return Ok(());
    }

    let file = if let Some(file) = state.cache.get(&identity) {
        file.clone()
    } else {
        let file = parse_file(&identity)?;
        state.cache.insert(identity.clone(), file.clone());
        file
    };

    state.active.push(identity.clone());
    state.active_set.insert(identity.clone());

    for item in &file.items {
        match item {
            Item::Host(index) => {
                let block = &file.hosts[*index];
                let key = (identity.clone(), block.byte_start, block.byte_end);
                let provenance = Provenance {
                    paths: chain.clone(),
                };
                if let Some(entry_index) = state.entry_indexes.get(&key).copied() {
                    let entry = &mut state.entries[entry_index];
                    if !entry.provenance.contains(&provenance) {
                        entry.provenance.push(provenance);
                    }
                } else {
                    let source = SourceIdentity {
                        path: display_path(&identity),
                        byte_start: block.byte_start,
                        byte_end: block.byte_end,
                        line_start: block.line_start,
                        line_end: block.line_end,
                    };
                    let entry = HostEntry {
                        id: format!("{}#{}-{}", source.path, source.byte_start, source.byte_end),
                        aliases: block.aliases.clone(),
                        source,
                        destination: block.destination.clone(),
                        provenance: vec![provenance],
                    };
                    state.entry_indexes.insert(key, state.entries.len());
                    state.entries.push(entry);
                }
            }
            Item::Include(patterns) => {
                for pattern in patterns {
                    for child in expand_include(pattern, include_base) {
                        let mut child_chain = chain.clone();
                        child_chain.push(display_path(&child));
                        visit_file(&child, include_base, child_chain, state)?;
                    }
                }
            }
        }
    }

    state.active.pop();
    state.active_set.remove(&identity);
    Ok(())
}

fn add_cycle_diagnostic(identity: &Path, state: &mut State) {
    let start = state
        .active
        .iter()
        .position(|active| active == identity)
        .unwrap_or(0);
    let mut cycle = state.active[start..]
        .iter()
        .map(|path| display_path(path))
        .collect::<Vec<_>>();
    cycle.push(display_path(identity));
    let key = cycle.join("\0");
    if state.diagnostic_keys.insert(key) {
        state.diagnostics.push(Diagnostic {
            code: "include_cycle".to_string(),
            message: format!("include cycle detected: {}", cycle.join(" -> ")),
        });
    }
}

fn parse_file(path: &Path) -> Result<ParsedFile, DiscoveryError> {
    let bytes = fs::read(path).map_err(|error| {
        DiscoveryError::new(format!("cannot read config {}: {error}", path.display()))
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let lines = split_lines(&text);

    let host_line_indexes = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let tokens = tokenize(&line.text);
            (tokens
                .first()
                .is_some_and(|token| token.eq_ignore_ascii_case("host"))
                && tokens.len() > 1)
                .then_some(index)
        })
        .collect::<Vec<_>>();

    let mut hosts = Vec::with_capacity(host_line_indexes.len());
    for (position, line_index) in host_line_indexes.iter().copied().enumerate() {
        let end_line_index = host_line_indexes
            .get(position + 1)
            .copied()
            .unwrap_or(lines.len());
        let host_tokens = tokenize(&lines[line_index].text);
        let destination = (line_index + 1..end_line_index)
            .find_map(|index| {
                let tokens = tokenize(&lines[index].text);
                tokens
                    .first()
                    .is_some_and(|token| token.eq_ignore_ascii_case("hostname"))
                    .then(|| tokens.get(1).cloned())
                    .flatten()
            })
            .or_else(|| {
                host_tokens
                    .iter()
                    .skip(1)
                    .find(|alias| !alias.starts_with('!') && !has_magic(alias))
                    .cloned()
            });
        let byte_start = lines[line_index].start;
        let byte_end = end_line_index
            .checked_sub(1)
            .and_then(|index| lines.get(index))
            .map_or(bytes.len(), |line| line.end);
        hosts.push(HostBlock {
            line_index,
            aliases: host_tokens.into_iter().skip(1).collect(),
            destination,
            byte_start,
            byte_end,
            line_start: lines[line_index].number,
            line_end: end_line_index
                .checked_sub(1)
                .and_then(|index| lines.get(index))
                .map_or(lines[line_index].number, |line| line.number),
        });
    }

    let mut host_by_line = HashMap::new();
    for (index, host) in hosts.iter().enumerate() {
        host_by_line.insert(host.line_index, index);
    }
    let mut items = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let tokens = tokenize(&line.text);
        if let Some(host_index) = host_by_line.get(&index) {
            items.push(Item::Host(*host_index));
        } else if tokens
            .first()
            .is_some_and(|token| token.eq_ignore_ascii_case("include"))
        {
            let patterns = tokens.into_iter().skip(1).collect::<Vec<_>>();
            if !patterns.is_empty() {
                items.push(Item::Include(patterns));
            }
        }
    }

    Ok(ParsedFile { hosts, items })
}

fn split_lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let end = start + raw.len();
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        lines.push(Line {
            number: index + 1,
            start,
            end,
            text: content.to_string(),
        });
        start = end;
    }
    if start < text.len() {
        lines.push(Line {
            number: lines.len() + 1,
            start,
            end: text.len(),
            text: text[start..].to_string(),
        });
    }
    lines
}

fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' => break,
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(character),
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn expand_include(pattern: &str, base: &Path) -> Vec<PathBuf> {
    let pattern = expand_tilde(pattern);
    let pattern = PathBuf::from(pattern);
    let pattern = if pattern.is_absolute() {
        pattern
    } else {
        base.join(pattern)
    };
    let pattern = lexical_normalize(&pattern);
    let components = pattern.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    let start = if pattern.is_absolute() {
        current.push("/");
        1
    } else {
        0
    };
    let mut matches = Vec::new();
    expand_components(&mut current, &components, start, &mut matches);
    sort_paths(&mut matches);
    matches
}
fn sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by_key(|path| display_path(path));
    paths.dedup();
}

fn expand_components(
    current: &mut PathBuf,
    components: &[Component<'_>],
    index: usize,
    matches: &mut Vec<PathBuf>,
) {
    if index == components.len() {
        if current.is_file() {
            matches.push(current.clone());
        }
        return;
    }

    let component = components[index].as_os_str().to_string_lossy();
    if component == "**" {
        expand_components(current, components, index + 1, matches);
        let Some(entries) = read_directory(current) else {
            return;
        };
        for entry in entries {
            if entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
            {
                current.push(entry.file_name());
                expand_components(current, components, index, matches);
                current.pop();
            }
        }
        return;
    }

    if has_magic(&component) {
        let Some(entries) = read_directory(current) else {
            return;
        };
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !component.starts_with('.') {
                continue;
            }
            if !wildcard_match(&component, &name) {
                continue;
            }
            current.push(entry.file_name());
            expand_components(current, components, index + 1, matches);
            current.pop();
        }
        return;
    }

    current.push(component.as_ref());
    if index + 1 == components.len() {
        if current.is_file() {
            matches.push(current.clone());
        }
    } else if current.is_dir() {
        expand_components(current, components, index + 1, matches);
    }
    current.pop();
}

fn read_directory(path: &Path) -> Option<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    Some(entries)
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let text = text.chars().collect::<Vec<_>>();
    let mut memo = HashMap::new();
    wildcard_match_at(&pattern, &text, 0, 0, &mut memo)
}

fn wildcard_match_at(
    pattern: &[char],
    text: &[char],
    pattern_index: usize,
    text_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(result) = memo.get(&(pattern_index, text_index)) {
        return *result;
    }
    let result = if pattern_index == pattern.len() {
        text_index == text.len()
    } else {
        match pattern[pattern_index] {
            '*' => {
                wildcard_match_at(pattern, text, pattern_index + 1, text_index, memo)
                    || (text_index < text.len()
                        && wildcard_match_at(pattern, text, pattern_index, text_index + 1, memo))
            }
            '?' => {
                text_index < text.len()
                    && wildcard_match_at(pattern, text, pattern_index + 1, text_index + 1, memo)
            }
            '[' => {
                if let Some((next, matches)) =
                    character_class(pattern, pattern_index, text, text_index)
                {
                    matches && wildcard_match_at(pattern, text, next, text_index + 1, memo)
                } else {
                    text_index < text.len()
                        && text[text_index] == '['
                        && wildcard_match_at(pattern, text, pattern_index + 1, text_index + 1, memo)
                }
            }
            character => {
                text_index < text.len()
                    && text[text_index] == character
                    && wildcard_match_at(pattern, text, pattern_index + 1, text_index + 1, memo)
            }
        }
    };
    memo.insert((pattern_index, text_index), result);
    result
}

fn character_class(
    pattern: &[char],
    pattern_index: usize,
    text: &[char],
    text_index: usize,
) -> Option<(usize, bool)> {
    let mut index = pattern_index + 1;
    let negated = pattern
        .get(index)
        .is_some_and(|character| *character == '!' || *character == '^');
    if negated {
        index += 1;
    }
    let start = index;
    let mut matched = false;
    let value = text.get(text_index).copied()?;
    while index < pattern.len() && pattern[index] != ']' {
        let first = pattern[index];
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            matched |= first <= value && value <= pattern[index + 2];
            index += 3;
        } else {
            matched |= first == value;
            index += 1;
        }
    }
    if index == pattern.len() || index == start {
        return None;
    }
    Some((index + 1, if negated { !matched } else { matched }))
}

fn has_magic(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '*' | '?' | '['))
}

fn expand_tilde(pattern: &str) -> String {
    if (pattern == "~" || pattern.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        let suffix = pattern.strip_prefix('~').unwrap_or_default();
        return PathBuf::from(home)
            .join(suffix.trim_start_matches('/'))
            .to_string_lossy()
            .into_owned();
    }
    pattern.to_string()
}

fn absolute_path(path: &Path) -> Result<PathBuf, DiscoveryError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| DiscoveryError::new(format!("cannot resolve config path: {error}")))?
            .join(path)
    };
    Ok(lexical_normalize(&path))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn display_path(path: &Path) -> String {
    lexical_normalize(path).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::wildcard_match;

    #[test]
    fn wildcard_matching_supports_glob_tokens() {
        assert!(wildcard_match("*.conf", "10-main.conf"));
        assert!(wildcard_match("host?.conf", "host1.conf"));
        assert!(wildcard_match("[ab].conf", "a.conf"));
        assert!(wildcard_match("host[12].conf", "host2.conf"));
        assert!(!wildcard_match("[ab].conf", "c.conf"));
    }
}
