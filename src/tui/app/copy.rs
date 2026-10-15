use super::{EffectResult, ForwardRequest, Runtime};
use sshx::discovery::HostEntry;
use sshx::keymap::CopyTarget;

pub(crate) struct CopyRequest {
    pub target: CopyTarget,
    pub entry: HostEntry,
    pub alias: String,
    pub forwards: Vec<ForwardRequest>,
    pub tunnel_id: Option<String>,
}

/// Runs outside raw mode, so the first-copy warning and the y/N confirmation can prompt on
/// the cooked terminal. Failures become toasts; the TUI never exits over a copy.
pub(super) fn copy(runtime: &mut Runtime, request: &CopyRequest) -> Result<EffectResult, String> {
    finish(copy_target(runtime, request))
}

/// Select input mode: one ssh command per selected row, published in a single clipboard write.
pub(super) fn copy_many(
    runtime: &Runtime,
    requests: &[CopyRequest],
) -> Result<EffectResult, String> {
    let lines = requests
        .iter()
        .map(|request| ssh_form(runtime, request, false))
        .collect::<Result<Vec<_>, _>>();
    finish(lines.and_then(|lines| {
        let plural = if lines.len() == 1 { "" } else { "s" };
        command(
            lines.join("\n"),
            &format!("{} ssh command{plural}", lines.len()),
        )
    }))
}

pub(super) fn copy_tunnel_id(id: &str) -> Result<EffectResult, String> {
    finish(command(id.to_string(), "tunnel ID"))
}

fn finish(result: Result<String, String>) -> Result<EffectResult, String> {
    let toast = match result {
        Ok(copied) => format!("✓ copied {copied}"),
        Err(error) => error,
    };
    Ok(EffectResult {
        toast: Some(toast),
        ..EffectResult::default()
    })
}

fn copy_target(runtime: &mut Runtime, request: &CopyRequest) -> Result<String, String> {
    let entry = &request.entry;
    match request.target {
        CopyTarget::SshCommand => command(ssh_form(runtime, request, false)?, "ssh command"),
        CopyTarget::VerboseSshCommand => {
            command(ssh_form(runtime, request, true)?, "ssh -v command")
        }
        CopyTarget::SshpassCommand => {
            let root = crate::select_config_root(entry, runtime.cli)?;
            let text = sshx::copy::sshpass_command_for_entry(
                &runtime.data.catalog.entries,
                entry,
                &root.path.to_string_lossy(),
                &request.alias,
                &runtime.data.prefs.bind,
                &forward_pairs(request),
            )?;
            secret(runtime, &text, "sshpass command")
        }
        CopyTarget::Password => {
            let password = sshx::connect::stored_password(entry, false)?.ok_or_else(|| {
                "PASSWORD_UNAVAILABLE: selected HostEntry has no non-empty stored password"
                    .to_string()
            })?;
            secret(runtime, &password, "password")
        }
        CopyTarget::HostName => {
            let host_name = entry
                .destination
                .as_deref()
                .ok_or_else(|| "selected HostEntry has no HostName".to_string())?;
            command(host_name.to_string(), "HostName")
        }
        CopyTarget::TunnelId => {
            let id = request
                .tunnel_id
                .clone()
                .ok_or_else(|| "no tunnel selected".to_string())?;
            command(id, "tunnel ID")
        }
    }
}

fn forward_pairs(request: &CopyRequest) -> Vec<(u16, u16)> {
    request
        .forwards
        .iter()
        .map(|forward| (forward.remote, forward.local))
        .collect()
}

fn command(text: String, what: &str) -> Result<String, String> {
    sshx::copy::publish_command(&text)?;
    Ok(what.to_string())
}

/// Direct HostEntries copy the ssh command; Pairs copy the sshx form that reproduces the route.
fn ssh_form(runtime: &Runtime, request: &CopyRequest, verbose: bool) -> Result<String, String> {
    let (entry, alias) = (&request.entry, request.alias.as_str());
    let entries = &runtime.data.catalog.entries;
    let forwards = forward_pairs(request);
    match sshx::pair::paired_route(entries, entry)? {
        Some(route) => {
            let root = crate::select_pair_config_root(&route.vm, &route.gateway, runtime.cli)?;
            let source_needed = root.is_none()
                || crate::source_disambiguation_needed(entry, alias, entries, root.as_ref());
            let config = root
                .as_ref()
                .map(|root| root.path.to_string_lossy().into_owned());
            Ok(sshx::copy::sshx_command(
                alias,
                &entry.id,
                config.as_deref(),
                source_needed.then_some((entry.source.path.as_str(), entry.source.line_start)),
                &forwards,
            ))
        }
        None => {
            let root = crate::select_config_root(entry, runtime.cli)?;
            let config = root.path.to_string_lossy();
            Ok(if verbose {
                sshx::copy::ssh_verbose_command(&config, alias)
            } else {
                sshx::copy::ssh_command(&config, alias, &runtime.data.prefs.bind, &forwards)
            })
        }
    }
}

/// Passwords and sshpass commands: warn once, confirm when `password_confirm` is on, and let
/// `publish_secret` schedule the clipboard clear.
fn secret(runtime: &mut Runtime, text: &str, what: &str) -> Result<String, String> {
    let preferences = &runtime.data.prefs;
    let first_copy = !preferences.password_warning_acknowledged;
    if first_copy {
        eprintln!("Warning: {}", sshx::copy::PASSWORD_WARNING);
    }
    if preferences.password_confirm {
        if !crate::prompt_yes(&format!("Copy {what} now? [y/N]: "))? {
            return Err(format!("{what} not copied"));
        }
    } else if first_copy {
        crate::prompt_value("Press Enter to copy: ", None)?;
    }
    let message = sshx::copy::publish_secret(text, what, preferences)?;
    if first_copy {
        sshx::copy::acknowledge_password_warning(&runtime.data.home, &mut runtime.data.prefs)?;
    }
    Ok(match message.split_once(';') {
        Some((_, outcome)) => format!("{what};{outcome}"),
        None => what.to_string(),
    })
}
