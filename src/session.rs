use crate::discovery::HostEntry;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::net::TcpListener;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredService {
    pub id: String,
    pub remote_port: u16,
    pub destination_host: String,
    pub default_local_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceForward {
    pub id: String,
    pub remote_port: u16,
    pub destination_host: String,
    pub local_port: u16,
}

pub fn declared_services(entry: &HostEntry) -> Result<Vec<DeclaredService>, String> {
    let bytes = fs::read(&entry.source.path).map_err(|error| {
        format!(
            "CONFIG_READ_FAILED: cannot read {}: {error}",
            entry.source.path
        )
    })?;
    let end = entry.source.byte_end;
    if entry.source.byte_start >= end || end > bytes.len() {
        return Err("CONFIG_INVALID: selected Host span is outside source file".to_string());
    }
    let text = std::str::from_utf8(&bytes[entry.source.byte_start..end])
        .map_err(|_| "CONFIG_INVALID: selected Host span is not valid UTF-8".to_string())?;

    let mut ports = Vec::new();
    let mut metadata = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(port) = declared_port(line)? {
            ports.push(port);
        }
        if let Some(value) = service_metadata(line)? {
            metadata.push(value);
        }
    }

    let mut services = Vec::with_capacity(ports.len());
    for (index, remote_port) in ports.into_iter().enumerate() {
        services.push(DeclaredService {
            id: format!("{remote_port}#{}", index + 1),
            remote_port,
            destination_host: "127.0.0.1".to_string(),
            default_local_port: remote_port,
        });
    }

    for metadata in metadata {
        let mut matched = false;
        for service in services
            .iter_mut()
            .filter(|service| service.remote_port == metadata.remote_port)
        {
            matched = true;
            if let Some(host) = metadata.destination_host.as_deref() {
                validate_destination_host(host)?;
                service.destination_host = host.to_string();
            }
            if let Some(local_port) = metadata.default_local_port {
                validate_port(local_port, "SERVICE_DEFAULT")?;
                service.default_local_port = local_port;
            }
        }
        if !matched {
            return Err(format!(
                "FORWARD_METADATA_INVALID: service metadata references undeclared remote port {}",
                metadata.remote_port
            ));
        }
    }
    Ok(services)
}

pub fn resolve_forwards(
    entry: &HostEntry,
    specifications: &[String],
) -> Result<Vec<ServiceForward>, String> {
    let services = declared_services(entry)?;
    let mut forwards = Vec::with_capacity(specifications.len());
    for specification in specifications {
        let (selector, local_override) = parse_specification(specification)?;
        let matches = services
            .iter()
            .filter(|service| service.id == selector || service.remote_port.to_string() == selector)
            .collect::<Vec<_>>();
        let service = match matches.as_slice() {
            [service] => *service,
            [] => {
                return Err(format!(
                    "FORWARD_NOT_FOUND: service `{selector}` is not declared for selected host"
                ));
            }
            _ => {
                return Err(format!(
                    "FORWARD_AMBIGUOUS: remote port `{selector}` has multiple declarations; use PORT#INDEX"
                ));
            }
        };
        if forwards
            .iter()
            .any(|forward: &ServiceForward| forward.id == service.id)
        {
            return Err(format!(
                "FORWARD_DUPLICATE: service `{}` was requested more than once",
                service.id
            ));
        }
        let local_port = local_override.unwrap_or(service.default_local_port);
        validate_port(local_port, "SERVICE_BIND")?;
        forwards.push(ServiceForward {
            id: service.id.clone(),
            remote_port: service.remote_port,
            destination_host: service.destination_host.clone(),
            local_port,
        });
    }
    Ok(forwards)
}

pub fn preflight(forwards: &[ServiceForward]) -> Result<(), String> {
    let mut listeners = Vec::with_capacity(forwards.len());
    for forward in forwards {
        validate_port(forward.local_port, "SERVICE_BIND")?;
        if listeners.iter().any(|listener: &TcpListener| {
            listener
                .local_addr()
                .is_ok_and(|address| address.port() == forward.local_port)
        }) {
            return Err(format!(
                "SERVICE_BIND_FAILED: local service port {} was requested more than once",
                forward.local_port
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", forward.local_port)).map_err(|error| {
            format!(
                "SERVICE_BIND_FAILED: cannot reserve local service port {}: {error}",
                forward.local_port
            )
        })?;
        listeners.push(listener);
    }
    Ok(())
}

pub fn interactive_forwards(entry: &HostEntry) -> Result<Vec<ServiceForward>, String> {
    if !io::stdin().is_terminal() {
        return Err(
            "FORWARD_INTERACTIVE_REQUIRED: --bind requires an interactive terminal".to_string(),
        );
    }
    let services = declared_services(entry)?;
    if services.is_empty() {
        return Err("FORWARD_NONE: selected host declares no service ports".to_string());
    }

    eprintln!("Available service forwards:");
    for (index, service) in services.iter().enumerate() {
        eprintln!(
            "  {}. {} -> {}:{} (local {})",
            index + 1,
            service.id,
            service.destination_host,
            service.remote_port,
            service.default_local_port
        );
    }
    let selected = prompt_line("Select services (comma-separated numbers): ")?;
    let indexes = parse_selection(&selected, services.len())?;
    if indexes.is_empty() {
        return Err("FORWARD_SELECTION_EMPTY: select at least one service".to_string());
    }

    let mut forwards = indexes
        .into_iter()
        .map(|index| ServiceForward {
            id: services[index].id.clone(),
            remote_port: services[index].remote_port,
            destination_host: services[index].destination_host.clone(),
            local_port: services[index].default_local_port,
        })
        .collect::<Vec<_>>();
    loop {
        for forward in &mut forwards {
            let prompt = format!("Local port for {} [{}]: ", forward.id, forward.local_port);
            let value = prompt_line(&prompt)?;
            if !value.trim().is_empty() {
                forward.local_port = value
                    .trim()
                    .parse()
                    .map_err(|_| "FORWARD_INVALID: local port must be a number".to_string())?;
            }
            validate_port(forward.local_port, "SERVICE_BIND")?;
        }
        match preflight(&forwards) {
            Ok(()) => return Ok(forwards),
            Err(error) => {
                eprintln!("{error}");
                eprintln!("Enter replacement local ports, or press Enter to retry:");
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ServiceMetadata {
    remote_port: u16,
    destination_host: Option<String>,
    default_local_port: Option<u16>,
}

fn declared_port(line: &str) -> Result<Option<u16>, String> {
    let Some(rest) = marker_value(line, "##PORT") else {
        return Ok(None);
    };
    let token = rest.split_whitespace().next().unwrap_or_default();
    if token.is_empty() {
        return Err("FORWARD_METADATA_INVALID: ##PORT requires a remote port".to_string());
    }
    let port = token
        .parse()
        .map_err(|_| "FORWARD_METADATA_INVALID: ##PORT remote port must be a number".to_string())?;
    validate_port(port, "SERVICE_REMOTE")?;
    Ok(Some(port))
}

fn service_metadata(line: &str) -> Result<Option<ServiceMetadata>, String> {
    let Some(mut rest) = marker_value(line, "##SSHX") else {
        return Ok(None);
    };
    rest = rest.trim_start();
    let Some((keyword, remainder)) =
        rest.split_once(|character: char| character.is_whitespace() || character == '=')
    else {
        return Ok(None);
    };
    if !keyword.eq_ignore_ascii_case("SERVICE") {
        return Ok(None);
    }
    rest = remainder.trim_start_matches([' ', '=']);
    let mut remote_port = None;
    let mut destination_host = None;
    let mut default_local_port = None;
    let mut positional_host = false;
    for token in rest.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            match key.to_ascii_lowercase().as_str() {
                "port" | "remote" | "remote_port" | "service" => {
                    remote_port = Some(parse_metadata_port(value)?)
                }
                "host" | "destination" | "destination_host" | "remote_host" | "to" => {
                    destination_host = Some(value.to_string())
                }
                "local" | "local_port" | "bind" | "bind_port" => {
                    default_local_port = Some(parse_metadata_port(value)?)
                }
                _ => {
                    return Err(format!(
                        "FORWARD_METADATA_INVALID: unknown SERVICE field `{key}`"
                    ));
                }
            }
            continue;
        }
        if remote_port.is_none()
            && let Ok(port) = token.parse()
        {
            remote_port = Some(port);
            continue;
        }
        if destination_host.is_none() && !positional_host && token.parse::<u16>().is_err() {
            destination_host = Some(token.to_string());
            positional_host = true;
            continue;
        }
        if default_local_port.is_none()
            && let Ok(port) = token.parse()
        {
            default_local_port = Some(port);
            continue;
        }
        return Err(format!(
            "FORWARD_METADATA_INVALID: unexpected SERVICE value `{token}`"
        ));
    }
    let remote_port = remote_port.ok_or_else(|| {
        "FORWARD_METADATA_INVALID: ##SSHX SERVICE requires a remote port".to_string()
    })?;
    validate_port(remote_port, "SERVICE_REMOTE")?;
    if let Some(host) = destination_host.as_deref() {
        validate_destination_host(host)?;
    }
    if let Some(port) = default_local_port {
        validate_port(port, "SERVICE_DEFAULT")?;
    }
    Ok(Some(ServiceMetadata {
        remote_port,
        destination_host,
        default_local_port,
    }))
}

fn marker_value<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let prefix = line.get(..marker.len())?;
    if !prefix.eq_ignore_ascii_case(marker) {
        return None;
    }
    let rest = &line[marker.len()..];
    if rest
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace() && character != '=')
    {
        return None;
    }
    Some(rest.trim_start_matches([' ', '=']))
}

fn parse_metadata_port(value: &str) -> Result<u16, String> {
    value
        .parse()
        .map_err(|_| "FORWARD_METADATA_INVALID: service port must be a number".to_string())
}

fn validate_port(port: u16, code: &str) -> Result<(), String> {
    if port == 0 {
        return Err(format!("{code}_INVALID: port must be between 1 and 65535"));
    }
    Ok(())
}

fn validate_destination_host(host: &str) -> Result<(), String> {
    if host.is_empty()
        || host.starts_with('-')
        || host.contains(|character: char| {
            character.is_whitespace() || matches!(character, '\0' | '\r' | '\n') || character == '%'
        })
        || matches!(host, "0.0.0.0" | "::" | "*")
    {
        return Err("FORWARD_UNSAFE: service destination host is invalid".to_string());
    }
    Ok(())
}

fn parse_specification(value: &str) -> Result<(String, Option<u16>), String> {
    let (selector, local) = value
        .split_once('=')
        .map_or((value, None), |(left, right)| (left, Some(right)));
    if selector.is_empty() || selector.contains(':') || selector.contains('/') {
        return Err(
            "FORWARD_UNSAFE: forward selector must contain only a declared remote port".to_string(),
        );
    }
    let local =
        match local {
            Some(value) => Some(value.parse().map_err(|_| {
                "FORWARD_INVALID: local port override must be a number".to_string()
            })?),
            None => None,
        };
    if let Some(port) = local {
        validate_port(port, "SERVICE_BIND")?;
    }
    if selector.parse::<u16>().is_err() && !selector.contains('#') {
        return Err("FORWARD_INVALID: forward selector must be REMOTE or REMOTE#INDEX".to_string());
    }
    Ok((selector.to_string(), local))
}

fn parse_selection(value: &str, count: usize) -> Result<Vec<usize>, String> {
    let mut selected = Vec::new();
    for item in value.split([',', ' ', '\t']) {
        if item.is_empty() {
            continue;
        }
        if item.eq_ignore_ascii_case("all") {
            selected.extend(0..count);
            continue;
        }
        let index = item
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .filter(|index| *index < count)
            .ok_or_else(|| {
                "FORWARD_SELECTION_INVALID: select service numbers from the list".to_string()
            })?;
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    Ok(selected)
}
fn prompt_line(prompt: &str) -> Result<String, String> {
    eprint!("{prompt}");
    io::stderr()
        .flush()
        .map_err(|error| format!("FORWARD_PROMPT_FAILED: cannot flush prompt: {error}"))?;
    let mut value = String::new();
    let read = io::stdin()
        .read_line(&mut value)
        .map_err(|error| format!("FORWARD_PROMPT_FAILED: cannot read selection: {error}"))?;
    if read == 0 {
        return Err("FORWARD_INPUT_CLOSED: interactive input ended".to_string());
    }
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(test)]
mod tests {
    use super::{ServiceForward, declared_services, preflight, resolve_forwards};
    use crate::discovery::{HostEntry, Provenance, SourceIdentity};
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn entry(text: &str) -> (PathBuf, HostEntry) {
        let path = std::env::temp_dir().join(format!(
            "sshx-session-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, text).expect("fixture should write");
        let host_start = text.find("Host ").expect("host should exist");
        let entry = HostEntry {
            id: "entry".to_string(),
            aliases: vec!["selected".to_string()],
            source: SourceIdentity {
                path: path.to_string_lossy().into_owned(),
                byte_start: host_start,
                byte_end: text.len(),
                line_start: 1,
                line_end: text.lines().count(),
            },
            destination: Some("example.test".to_string()),
            provenance: vec![Provenance {
                paths: vec![path.to_string_lossy().into_owned()],
                scope: "personal".to_string(),
                project: None,
            }],
            scopes: vec!["personal".to_string()],
            projects: Vec::new(),
        };
        (path, entry)
    }

    #[test]
    fn repeated_ports_get_unique_loopback_services_and_metadata_defaults() {
        let (path, entry) = entry(concat!(
            "Host selected\n",
            "  HostName example.test\n",
            "  ##PORT 5432\n",
            "  ##PORT 5432\n",
            "  ##SSHX SERVICE 5432 HOST=db.internal LOCAL=15432\n",
        ));
        let services = declared_services(&entry).expect("services should parse");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].id, "5432#1");
        assert_eq!(services[1].id, "5432#2");
        assert_eq!(services[0].destination_host, "db.internal");
        assert_eq!(services[0].default_local_port, 15432);
        assert_eq!(services[1].default_local_port, 15432);
        fs::remove_file(path).expect("fixture should remove");
    }

    #[test]
    fn overrides_do_not_change_declared_defaults_and_preflight_rejects_duplicate_ports() {
        let (path, entry) = entry("Host selected\n  ##PORT 5432\n");
        let forwards =
            resolve_forwards(&entry, &["5432=15432".to_string()]).expect("forward should resolve");
        assert_eq!(forwards[0].local_port, 15432);
        assert_eq!(
            declared_services(&entry).unwrap()[0].default_local_port,
            5432
        );
        let duplicate = vec![
            ServiceForward {
                id: "one".to_string(),
                remote_port: 5432,
                destination_host: "127.0.0.1".to_string(),
                local_port: 15432,
            },
            ServiceForward {
                id: "two".to_string(),
                remote_port: 5433,
                destination_host: "127.0.0.1".to_string(),
                local_port: 15432,
            },
        ];
        assert!(preflight(&duplicate).is_err());
        fs::remove_file(path).expect("fixture should remove");
    }
    #[test]
    fn preflight_reports_busy_port_without_touching_existing_listener() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener should bind");
        let local_port = listener.local_addr().expect("listener address").port();
        let error = preflight(&[ServiceForward {
            id: "5432#1".to_string(),
            remote_port: 5432,
            destination_host: "127.0.0.1".to_string(),
            local_port,
        }])
        .expect_err("busy local port should fail");
        assert!(error.starts_with("SERVICE_BIND_FAILED"), "{error}");
        assert!(listener.local_addr().is_ok());
    }
    #[test]
    fn metadata_scan_handles_unicode_and_rejects_ssh_tokens() {
        let (path, entry) = entry(concat!(
            "Host selected\n",
            "  # 日本語\n",
            "  ##PORT 5432\n",
            "  ##SSHX SERVICE 5432 HOST=%h\n",
        ));
        let error = declared_services(&entry).expect_err("SSH tokens must be rejected");
        assert!(error.starts_with("FORWARD_UNSAFE"), "{error}");
        fs::remove_file(path).expect("fixture should remove");
    }
}
