use crate::discovery::{Diagnostic, HostEntry};
use crate::mutation::{self, PairMutationRequest, PairPlan};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;

const DEFAULT_SSH_PORT: u16 = 22;

#[derive(Clone, Debug)]
struct LocalForward {
    host: String,
    port: u16,
}

#[derive(Clone, Debug)]
struct ParsedEntry {
    id: Option<String>,
    id_markers: usize,
    gateway_id: Option<String>,
    gateway_marker: bool,
    paired_vm_id: Option<String>,
    paired_vm_marker: bool,
    transit: Option<(String, u16)>,
    transit_marker: bool,
    port: u16,
    port_set: bool,
    invalid_port: bool,
    forwards: Vec<LocalForward>,
    proxy_command: bool,
    proxy_jump: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairRecord {
    pub gateway_id: String,
    pub vm_id: String,
    pub gateway_alias: String,
    pub vm_alias: String,
    pub transit_host: String,
    pub transit_port: u16,
}

#[derive(Clone, Debug)]
pub struct PairedRoute {
    pub gateway: HostEntry,
    pub vm: HostEntry,
    pub gateway_id: String,
    pub vm_id: String,
    pub transit_host: String,
    pub transit_port: u16,
}

pub fn paired_route(
    entries: &[HostEntry],
    selected_vm: &HostEntry,
) -> Result<Option<PairedRoute>, String> {
    let vm_index = entry_index(entries, selected_vm)?;
    let parsed = entries
        .iter()
        .map(parse_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let vm_data = &parsed[vm_index];
    if !vm_data.gateway_marker && !vm_data.paired_vm_marker && !vm_data.transit_marker {
        return Ok(None);
    }
    if !vm_data.gateway_marker || !vm_data.transit_marker || vm_data.paired_vm_marker {
        return Err("PAIR_BROKEN: VM pair metadata is incomplete or misplaced".to_string());
    }
    if vm_data.id_markers != 1 {
        return Err("PAIR_BROKEN: VM must have exactly one immutable ID".to_string());
    }

    let vm_id = vm_data
        .id
        .as_deref()
        .filter(|id| valid_id(id))
        .ok_or_else(|| "PAIR_INVALID: VM has no valid immutable ID".to_string())?;
    if selected_vm.id != vm_id {
        return Err("PAIR_INVALID: selected VM identity changed on disk".to_string());
    }
    let gateway_id = vm_data
        .gateway_id
        .as_deref()
        .filter(|id| valid_id(id))
        .ok_or_else(|| "PAIR_BROKEN: VM has no valid gateway reference".to_string())?;
    let transit = vm_data
        .transit
        .clone()
        .filter(|(_, port)| *port > 0)
        .ok_or_else(|| "PAIR_BROKEN: VM has no valid approved transit destination".to_string())?;
    if vm_data.transit_marker && vm_data.transit.is_none() {
        return Err("PAIR_BROKEN: VM transit metadata is malformed".to_string());
    }
    if vm_data.invalid_port || vm_data.port != transit.1 {
        return Err("PAIR_ROUTE_CHANGED: VM Port no longer matches approved transit".to_string());
    }

    let mut id_indexes = HashMap::new();
    for (index, data) in parsed.iter().enumerate() {
        let Some(id) = data.id.as_deref().filter(|id| valid_id(id)) else {
            continue;
        };
        if id_indexes.insert(id.to_ascii_lowercase(), index).is_some() {
            return Err(format!("PAIR_INVALID: duplicate immutable ID `{id}`"));
        }
    }
    let gateway_index = id_indexes
        .get(&gateway_id.to_ascii_lowercase())
        .copied()
        .ok_or_else(|| "PAIR_BROKEN: gateway reference no longer exists".to_string())?;
    if gateway_index == vm_index {
        return Err("PAIR_BROKEN: gateway and VM references point to one entry".to_string());
    }
    let gateway = &entries[gateway_index];
    let gateway_data = &parsed[gateway_index];
    if gateway_data.id_markers != 1
        || gateway_data.gateway_marker
        || gateway_data.transit_marker
        || !gateway_data.paired_vm_marker
        || gateway_data
            .paired_vm_id
            .as_deref()
            .is_none_or(|id| !id.eq_ignore_ascii_case(vm_id))
    {
        return Err("PAIR_BROKEN: gateway has no matching VM reference".to_string());
    }
    if gateway_data.proxy_command
        || gateway_data.proxy_jump
        || vm_data.proxy_command
        || vm_data.proxy_jump
    {
        return Err(
            "PAIR_ROUTE_UNSAFE: ProxyCommand and ProxyJump are not allowed for paired routes"
                .to_string(),
        );
    }
    let candidates = gateway_data
        .forwards
        .iter()
        .filter(|candidate| candidate.host == transit.0 && candidate.port == transit.1)
        .count();
    if candidates != 1 {
        return Err(format!(
            "PAIR_ROUTE_CHANGED: approved gateway transit {}:{} has {candidates} current LocalForward candidates",
            transit.0, transit.1
        ));
    }
    Ok(Some(PairedRoute {
        gateway: gateway.clone(),
        vm: selected_vm.clone(),
        gateway_id: gateway_id.to_string(),
        vm_id: vm_id.to_string(),
        transit_host: transit.0,
        transit_port: transit.1,
    }))
}

pub fn diagnostics(entries: &[HostEntry]) -> Vec<Diagnostic> {
    let parsed = entries.iter().map(parse_entry).collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    let mut ids: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, result) in parsed.iter().enumerate() {
        let Ok(entry) = result else {
            push_diagnostic(
                &mut diagnostics,
                "broken_reference",
                format!(
                    "cannot inspect pair metadata for {}",
                    entries[index].source.path
                ),
            );
            continue;
        };
        if entry.id_markers > 1 {
            push_diagnostic(
                &mut diagnostics,
                "duplicate_id",
                format!(
                    "HostEntry {} contains multiple ##SSHX ID markers",
                    entries[index].source.path
                ),
            );
        }
        if let Some(id) = &entry.id {
            ids.entry(id.to_ascii_lowercase()).or_default().push(index);
            if !valid_id(id) {
                push_diagnostic(
                    &mut diagnostics,
                    "malformed_id",
                    format!(
                        "HostEntry {} has malformed ID `{id}`",
                        entries[index].source.path
                    ),
                );
            }
        }
        if entry.invalid_port {
            push_diagnostic(
                &mut diagnostics,
                "malformed_pair",
                format!(
                    "HostEntry {} has an invalid Port directive",
                    entries[index].source.path
                ),
            );
        }
    }
    for (id, indexes) in &ids {
        if indexes.len() > 1 {
            let sources = indexes
                .iter()
                .map(|index| entries[*index].source.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            push_diagnostic(
                &mut diagnostics,
                "duplicate_id",
                format!("ID `{id}` is used by multiple HostEntries: {sources}"),
            );
        }
    }

    let unique = ids
        .iter()
        .filter_map(|(id, indexes)| (indexes.len() == 1).then_some((id, indexes[0])))
        .collect::<HashMap<_, _>>();
    let mut gateway_users: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, result) in parsed.iter().enumerate() {
        let Ok(entry) = result else {
            continue;
        };
        if entry.transit_marker && (!entry.gateway_marker || entry.transit.is_none()) {
            push_diagnostic(
                &mut diagnostics,
                "broken_reference",
                format!(
                    "HostEntry {} has invalid transit metadata",
                    entries[index].source.path
                ),
            );
        }
        if entry.gateway_marker {
            let Some(gateway_id) = entry.gateway_id.as_deref() else {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has an empty gateway reference",
                        entries[index].source.path
                    ),
                );
                continue;
            };
            gateway_users
                .entry(gateway_id.to_ascii_lowercase())
                .or_default()
                .push(index);
            let Some(gateway_index) = unique.get(&gateway_id.to_ascii_lowercase()) else {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} references missing gateway `{gateway_id}`",
                        entries[index].source.path
                    ),
                );
                continue;
            };
            let gateway = parsed[*gateway_index].as_ref().ok();
            let own_id = entry.id.as_deref();
            if own_id.is_none() || !valid_id(own_id.unwrap_or_default()) {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has pair metadata but no valid ID",
                        entries[index].source.path
                    ),
                );
            }
            if entry.transit.is_none() {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has no valid transit destination",
                        entries[index].source.path
                    ),
                );
            }
            if gateway.is_none_or(|gateway| {
                gateway.paired_vm_id.as_deref().map(str::to_ascii_lowercase)
                    != own_id.map(str::to_ascii_lowercase)
            }) {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has no matching gateway pair reference",
                        entries[index].source.path
                    ),
                );
            }
        }
        if entry.paired_vm_marker {
            let Some(vm_id) = entry.paired_vm_id.as_deref() else {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has an empty VM reference",
                        entries[index].source.path
                    ),
                );
                continue;
            };
            let Some(vm_index) = unique.get(&vm_id.to_ascii_lowercase()) else {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} references missing VM `{vm_id}`",
                        entries[index].source.path
                    ),
                );
                continue;
            };
            let vm = parsed[*vm_index].as_ref().ok();
            let own_id = entry.id.as_deref();
            if vm.is_none_or(|vm| {
                vm.gateway_id.as_deref().map(str::to_ascii_lowercase)
                    != own_id.map(str::to_ascii_lowercase)
                    || vm.transit.is_none()
            }) {
                push_diagnostic(
                    &mut diagnostics,
                    "broken_reference",
                    format!(
                        "HostEntry {} has no matching VM pair reference",
                        entries[index].source.path
                    ),
                );
            }
        }
    }
    for (gateway_id, users) in gateway_users {
        if users.len() > 1 {
            push_diagnostic(
                &mut diagnostics,
                "pair_conflict",
                format!("gateway `{gateway_id}` is paired to multiple VM entries"),
            );
        }
    }
    diagnostics
}

pub fn records(entries: &[HostEntry]) -> Vec<PairRecord> {
    let parsed = entries.iter().map(parse_entry).collect::<Vec<_>>();
    let by_id = parsed
        .iter()
        .enumerate()
        .filter_map(|(index, result)| {
            let entry = result.as_ref().ok()?;
            let id = entry.id.as_deref()?;
            (valid_id(id)).then_some((id.to_ascii_lowercase(), index))
        })
        .collect::<HashMap<_, _>>();
    let mut records = Vec::new();
    for (index, result) in parsed.iter().enumerate() {
        let Ok(vm) = result else {
            continue;
        };
        let Some(gateway_id) = vm.gateway_id.as_deref() else {
            continue;
        };
        let Some(transit) = vm.transit.as_ref() else {
            continue;
        };
        let Some(gateway_index) = by_id.get(&gateway_id.to_ascii_lowercase()) else {
            continue;
        };
        let gateway = &entries[*gateway_index];
        records.push(PairRecord {
            gateway_id: gateway.id.clone(),
            vm_id: entries[index].id.clone(),
            gateway_alias: gateway.aliases.first().cloned().unwrap_or_default(),
            vm_alias: entries[index].aliases.first().cloned().unwrap_or_default(),
            transit_host: transit.0.clone(),
            transit_port: transit.1,
        });
    }
    records
}

pub fn plan_setup(
    entries: &[HostEntry],
    gateway: &HostEntry,
    vm: &HostEntry,
    transit_host: Option<&str>,
    transit_port: Option<u16>,
) -> Result<PairPlan, String> {
    if same_entry(gateway, vm) {
        return Err("PAIR_SAME_ENTRY: gateway and VM must be different HostEntries".to_string());
    }
    let parsed = entries
        .iter()
        .map(parse_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let all_diagnostics = diagnostics(entries);
    if let Some(diagnostic) = all_diagnostics.iter().find(|diagnostic| {
        matches!(
            diagnostic.code.as_str(),
            "malformed_id"
                | "duplicate_id"
                | "broken_reference"
                | "pair_conflict"
                | "malformed_pair"
        )
    }) {
        return Err(format!("PAIR_INVALID: {}", diagnostic.message));
    }
    let gateway_index = entry_index(entries, gateway)?;
    let vm_index = entry_index(entries, vm)?;
    let gateway_data = &parsed[gateway_index];
    let vm_data = &parsed[vm_index];
    if gateway_data.proxy_command
        || gateway_data.proxy_jump
        || vm_data.proxy_command
        || vm_data.proxy_jump
    {
        return Err(
            "PAIR_ROUTE_UNSAFE: ProxyCommand and ProxyJump are not allowed for paired routes"
                .to_string(),
        );
    }

    let mut ids = HashSet::new();
    for entry in &parsed {
        if let Some(id) = &entry.id {
            if !valid_id(id) {
                return Err(format!("ID_MALFORMED: malformed HostEntry ID `{id}`"));
            }
            ids.insert(id.to_ascii_lowercase());
        }
    }
    let gateway_id = gateway_data
        .id
        .clone()
        .unwrap_or_else(|| mutation::generate_id(&ids));
    ids.insert(gateway_id.to_ascii_lowercase());
    let vm_id = vm_data
        .id
        .clone()
        .unwrap_or_else(|| mutation::generate_id(&ids));
    if gateway_data.id.is_some() && !valid_id(&gateway_id) {
        return Err(format!("ID_MALFORMED: malformed gateway ID `{gateway_id}`"));
    }
    if vm_data.id.is_some() && !valid_id(&vm_id) {
        return Err(format!("ID_MALFORMED: malformed VM ID `{vm_id}`"));
    }
    if gateway_data.paired_vm_marker
        || entries.iter().enumerate().any(|(index, entry)| {
            parsed[index]
                .gateway_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(&gateway_id))
                && !same_entry(entry, vm)
        })
    {
        return Err(format!(
            "PAIR_GATEWAY_IN_USE: gateway `{gateway_id}` is already paired"
        ));
    }
    if vm_data.gateway_marker || vm_data.paired_vm_marker {
        return Err(format!("PAIR_VM_IN_USE: VM `{vm_id}` is already paired"));
    }

    let (transit_host, transit_port) = match (transit_host, transit_port) {
        (Some(host), Some(port)) => {
            validate_transit_host(host)?;
            validate_transit_port(port)?;
            (host.to_string(), port)
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(
                "TRANSIT_INCOMPLETE: provide both --transit-host and --transit-port".to_string(),
            );
        }
        (None, None) => {
            let vm_port = if vm_data.invalid_port {
                return Err("PORT_INVALID: VM Port must be a number".to_string());
            } else {
                vm_data.port
            };
            let candidates = gateway_data
                .forwards
                .iter()
                .filter(|candidate| candidate.port == vm_port)
                .collect::<Vec<_>>();
            let [candidate] = candidates.as_slice() else {
                return Err(format!(
                    "TRANSIT_REQUIRED: VM port {vm_port} matches {} gateway LocalForward candidates; provide --transit-host and --transit-port",
                    candidates.len()
                ));
            };
            (candidate.host.clone(), candidate.port)
        }
    };

    mutation::plan_pair(&PairMutationRequest {
        gateway_path: gateway.source.path.clone().into(),
        gateway_expected_id: gateway.id.clone(),
        gateway_id,
        gateway_alias: gateway.aliases.first().cloned().unwrap_or_default(),
        gateway_byte_start: gateway.source.byte_start,
        gateway_byte_end: gateway.source.byte_end,
        vm_path: vm.source.path.clone().into(),
        vm_expected_id: vm.id.clone(),
        vm_id,
        vm_alias: vm.aliases.first().cloned().unwrap_or_default(),
        vm_byte_start: vm.source.byte_start,
        vm_byte_end: vm.source.byte_end,
        transit_host,
        transit_port,
    })
}

pub fn deletion_reference(entry: &HostEntry, entries: &[HostEntry]) -> Option<String> {
    let selected = parse_entry(entry).ok()?;
    let selected_id = selected.id.clone().unwrap_or_else(|| entry.id.clone());
    if selected.gateway_marker || selected.paired_vm_marker || selected.transit_marker {
        return Some(format!(
            "DELETE_REFERENCED: entry {} is referenced by its pair metadata",
            selected_id
        ));
    }
    let selected_id = selected.id.as_deref()?;
    for other in entries {
        if same_entry(entry, other) {
            continue;
        }
        let Ok(metadata) = parse_entry(other) else {
            continue;
        };
        if metadata
            .gateway_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(selected_id))
            || metadata
                .paired_vm_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(selected_id))
        {
            return Some(format!(
                "DELETE_REFERENCED: entry {} is referenced by {}",
                selected_id, other.source.path
            ));
        }
    }
    None
}

pub fn valid_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    for (index, byte) in value.as_bytes().iter().copied().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    matches!(value.as_bytes()[14], b'1'..=b'5')
        && matches!(
            value.as_bytes()[19],
            b'8' | b'9' | b'a' | b'b' | b'A' | b'B'
        )
}

fn parse_entry(entry: &HostEntry) -> Result<ParsedEntry, String> {
    let bytes = fs::read(&entry.source.path)
        .map_err(|error| format!("cannot read {}: {error}", entry.source.path))?;
    if entry.source.byte_end > bytes.len() || entry.source.byte_start >= entry.source.byte_end {
        return Err(format!("invalid source span for {}", entry.source.path));
    }
    let lines = byte_lines(&bytes);
    let host_line = lines
        .iter()
        .position(|line| line.0 == entry.source.byte_start)
        .ok_or_else(|| format!("Host line moved in {}", entry.source.path))?;
    let block_start_line = (0..host_line)
        .rev()
        .find(|index| is_boundary(&bytes, lines[*index]))
        .map_or(0, |index| index + 1);
    let next_boundary =
        ((host_line + 1)..lines.len()).find(|index| is_boundary(&bytes, lines[*index]));
    let mut parse_end_line = next_boundary.unwrap_or(lines.len());
    while parse_end_line > host_line + 1 {
        let line = lines[parse_end_line - 1];
        let text = String::from_utf8_lossy(&bytes[line.0..line.1]);
        if text.trim_start().starts_with("##SSHX") {
            parse_end_line -= 1;
        } else {
            break;
        }
    }
    let first_host_line = lines.iter().position(|line| {
        let tokens = directive_tokens(&String::from_utf8_lossy(&bytes[line.0..line.1]));
        tokens
            .first()
            .is_some_and(|keyword| keyword.eq_ignore_ascii_case("host") && tokens.len() > 1)
    });
    let global_proxy = first_host_line.is_some_and(|first_host_line| {
        lines[..first_host_line].iter().any(|line| {
            let tokens = directive_tokens(&String::from_utf8_lossy(&bytes[line.0..line.1]));
            tokens.first().is_some_and(|keyword| {
                keyword.eq_ignore_ascii_case("proxycommand")
                    || keyword.eq_ignore_ascii_case("proxyjump")
            })
        })
    });
    let mut result = ParsedEntry {
        id: None,
        id_markers: 0,
        gateway_id: None,
        gateway_marker: false,
        paired_vm_id: None,
        paired_vm_marker: false,
        transit: None,
        transit_marker: false,
        port: DEFAULT_SSH_PORT,
        port_set: false,
        invalid_port: false,
        forwards: Vec::new(),
        proxy_command: global_proxy,
        proxy_jump: global_proxy,
    };
    for (index, line) in lines
        .iter()
        .copied()
        .enumerate()
        .take(parse_end_line)
        .skip(block_start_line)
    {
        let text = String::from_utf8_lossy(&bytes[line.0..line.1]);
        let trimmed = text.trim();
        if let Some(rest) = trimmed.strip_prefix("##SSHX") {
            let (key, value) = metadata_key_value(rest.trim());
            match key.as_deref() {
                Some("ID") => {
                    result.id_markers += 1;
                    if result.id.is_none() {
                        result.id = value.filter(|value| !value.is_empty());
                    }
                }
                Some("GATEWAY") => {
                    result.gateway_marker = true;
                    result.gateway_id = value.filter(|value| !value.is_empty());
                }
                Some("VM") => {
                    result.paired_vm_marker = true;
                    result.paired_vm_id = value.filter(|value| !value.is_empty());
                }
                Some("TRANSIT") => {
                    result.transit_marker = true;
                    result.transit = value.as_deref().and_then(parse_transit);
                }
                _ => {}
            }
            continue;
        }
        if index <= host_line {
            continue;
        }
        let tokens = directive_tokens(&text);
        let Some(keyword) = tokens.first() else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("port") {
            if result.port_set {
                continue;
            }
            result.port_set = true;
            match tokens.get(1).and_then(|value| value.parse::<u16>().ok()) {
                Some(port) if port > 0 => result.port = port,
                _ => result.invalid_port = true,
            }
        } else if keyword.eq_ignore_ascii_case("localforward") {
            if let Some(candidate) = tokens.get(2).and_then(|value| parse_endpoint(value)) {
                result.forwards.push(LocalForward {
                    host: candidate.0,
                    port: candidate.1,
                });
            }
        } else if keyword.eq_ignore_ascii_case("proxycommand") {
            result.proxy_command = true;
        } else if keyword.eq_ignore_ascii_case("proxyjump") {
            result.proxy_jump = true;
        }
    }
    Ok(result)
}

fn entry_index(entries: &[HostEntry], selected: &HostEntry) -> Result<usize, String> {
    entries
        .iter()
        .position(|entry| same_entry(entry, selected))
        .ok_or_else(|| "PAIR_SELECTION: selected HostEntry is not current".to_string())
}

fn same_entry(left: &HostEntry, right: &HostEntry) -> bool {
    left.source.path == right.source.path
        && left.source.byte_start == right.source.byte_start
        && left.source.byte_end == right.source.byte_end
}

fn push_diagnostic(diagnostics: &mut Vec<Diagnostic>, code: &str, message: String) {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code && diagnostic.message == message)
    {
        return;
    }
    diagnostics.push(Diagnostic {
        code: code.to_string(),
        message,
    });
}

fn metadata_key_value(value: &str) -> (Option<String>, Option<String>) {
    let mut parts = value.splitn(2, |character: char| character.is_whitespace());
    let first = parts.next().unwrap_or_default().trim();
    let trailing = parts.next().unwrap_or_default().trim();
    if let Some((key, value)) = first.split_once('=') {
        return (
            Some(key.trim_end_matches(':').to_ascii_uppercase()),
            Some(value.to_string()),
        );
    }
    (
        (!first.is_empty()).then(|| first.trim_end_matches(':').to_ascii_uppercase()),
        (!trailing.is_empty()).then(|| trailing.trim_end_matches(':').to_string()),
    )
}

fn parse_transit(value: &str) -> Option<(String, u16)> {
    let mut parts = value.split_whitespace();
    let first = parts.next()?;
    if let Some((host, port)) = split_host_port(first) {
        return Some((host, port));
    }
    let port = parts.next()?.parse().ok()?;
    Some((first.to_string(), port))
}

fn parse_endpoint(value: &str) -> Option<(String, u16)> {
    if let Some((host, port)) = split_host_port(value) {
        return Some((host, port));
    }
    let port = value.parse().ok()?;
    Some(("127.0.0.1".to_string(), port))
}

fn split_host_port(value: &str) -> Option<(String, u16)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        return Some((host.to_string(), port.parse().ok()?));
    }
    let (host, port) = value.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port.parse().ok()?))
}

fn directive_tokens(line: &str) -> Vec<String> {
    let mut tokens = line
        .split('#')
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let Some(first) = tokens.next() else {
        return Vec::new();
    };
    let mut result = vec![first.to_string()];
    result.extend(tokens.map(str::to_string));
    if let Some((keyword, value)) = result[0].split_once('=') {
        let keyword = keyword.to_string();
        let value = value.to_string();
        result[0] = keyword;
        if !value.is_empty() {
            result.insert(1, value);
        }
    }
    result
}

fn validate_transit_host(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains(['\0', '\r', '\n'])
    {
        return Err(
            "TRANSIT_HOST_INVALID: transit host cannot be empty or contain whitespace/control lines"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_transit_port(value: u16) -> Result<(), String> {
    (value > 0)
        .then_some(())
        .ok_or_else(|| "TRANSIT_PORT_INVALID: transit port must be non-zero".to_string())
}

fn byte_lines(bytes: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let content_end = if index > start && bytes[index - 1] == b'\r' {
            index - 1
        } else {
            index
        };
        lines.push((start, content_end, index + 1));
        start = index + 1;
    }
    if start < bytes.len() {
        lines.push((start, bytes.len(), bytes.len()));
    }
    lines
}

fn is_boundary(bytes: &[u8], line: (usize, usize, usize)) -> bool {
    let tokens = directive_tokens(&String::from_utf8_lossy(&bytes[line.0..line.1]));
    match tokens.first().map(String::as_str) {
        Some(keyword) if keyword.eq_ignore_ascii_case("match") => true,
        Some(keyword) if keyword.eq_ignore_ascii_case("host") => tokens.len() > 1,
        _ => false,
    }
}
