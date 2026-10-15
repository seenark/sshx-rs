use super::{
    ConnectRequest, Effect, EffectResult, PreferenceChange, Runtime, Screen, copy, delete,
    failure::Failure, finder::Finder, host_form, input_mode, tunnels::Tunnels, undo,
};
use crate::tui::CANCELLED;
use sshx::discovery::scope_for_path;
use sshx::keymap;
use sshx::settings::{self, RegisteredRoot};
use std::path::Path;

pub(super) fn execute(runtime: &mut Runtime, effect: Effect) -> Result<EffectResult, String> {
    match effect {
        Effect::Quit { cancelled } => quit(cancelled),
        Effect::Connect(request) => connect(runtime, &request),
        Effect::Tunnels => tunnels(runtime),
        Effect::SetPreference(change) => set_preference(runtime, change),
        Effect::RootAdd(path) => root_add(runtime, &path),
        Effect::RootRemove(path) => root_remove(runtime, &path),
        Effect::RegisterRoots(roots) => register_roots(runtime, roots),
        Effect::Copy(request) => copy::copy(runtime, &request),
        Effect::SaveHost(request) => host_form::save(runtime, &request),
        Effect::EditInEditor(request) => host_form::edit_in_editor(&request),
        Effect::DeleteHost(request) => delete::delete(runtime, &request),
        Effect::Undo => undo::undo(runtime),
        Effect::Redo => undo::redo(runtime),
        Effect::TunnelStart(request) => super::tunnel_progress::start(runtime, &request),
        Effect::TunnelStop(ids) => super::tunnels::stop(runtime, &ids),
        Effect::TunnelRestart(id) => super::tunnels::restart(runtime, &id),
        Effect::SaveService(edit) => super::ports::save_service(runtime, &edit),
    }
}

fn toast(message: impl Into<String>) -> EffectResult {
    EffectResult {
        toast: Some(message.into()),
        ..EffectResult::default()
    }
}

fn quit(cancelled: bool) -> Result<EffectResult, String> {
    if cancelled {
        return Err(CANCELLED.to_string());
    }
    Ok(EffectResult {
        exit: Some(0),
        ..EffectResult::default()
    })
}

/// Session mode through the existing direct and Pair session functions. A finished session
/// ends sshx; a master that failed to start shows the failure screen, and any other error
/// (interruptions included) returns to the finder as a toast, after rediscovery because the
/// failure may come from sources that changed on disk.
fn connect(runtime: &Runtime, request: &ConnectRequest) -> Result<EffectResult, String> {
    let mut result = start_session(runtime, request)?;
    result.refresh |= result.exit.is_none();
    Ok(result)
}

fn start_session(runtime: &Runtime, request: &ConnectRequest) -> Result<EffectResult, String> {
    let data = &runtime.data;
    if data
        .catalog
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "unsupported_match")
    {
        return Ok(toast(
            "UNSUPPORTED_MATCH: Match prevents exact runtime configuration",
        ));
    }
    let route = match sshx::pair::paired_route(&data.catalog.entries, &request.entry) {
        Ok(route) => route,
        Err(error) => return Ok(toast(error)),
    };
    if !crate::entry_sources_unchanged(&request.entry, &data.sources)
        || route
            .as_ref()
            .is_some_and(|route| !crate::entry_sources_unchanged(&route.gateway, &data.sources))
    {
        return Ok(toast(format!(
            "HOST_SOURCE_CHANGED: {} changed; select its current HostEntry again",
            request.entry.source.path
        )));
    }
    let target = route.as_ref().map_or(&request.entry, |route| &route.vm);
    let specifications = request
        .forwards
        .iter()
        .map(|forward| format!("{}={}", forward.remote, forward.local))
        .collect::<Vec<_>>();
    let forwards = match sshx::session::resolve_forwards(target, &specifications) {
        Ok(forwards) => forwards,
        Err(error) => return Ok(toast(error)),
    };
    let outcome = match &route {
        Some(route) => {
            let Some(gateway_alias) = route.gateway.aliases.first() else {
                return Ok(toast("PAIR_INVALID: gateway has no alias"));
            };
            sshx::connect::open_paired_with_forwards(
                route,
                &data.home,
                false,
                gateway_alias,
                &request.alias,
                &mut sshx::connect::PairedSessionCredentials::new(
                    sshx::connect::PairedCredentials {
                        gateway_password_fd: None,
                        vm_password_fd: None,
                    },
                ),
                &forwards,
                true,
            )
        }
        None => sshx::connect::open_with_password_fd_and_all_forwards(
            &request.entry,
            &data.home,
            false,
            &request.alias,
            None,
            &forwards,
            &[],
            true,
        ),
    };
    match outcome {
        Ok(code) => Ok(EffectResult {
            exit: Some(code),
            ..EffectResult::default()
        }),
        Err(error) => Ok(match sshx::connect::classify_session_error(&error) {
            Some(failure) => EffectResult {
                push: Some(Box::new(Failure::new(
                    request,
                    failure,
                    route.as_ref().and_then(|route| {
                        Some((
                            route.gateway.aliases.first()?.clone(),
                            request.alias.clone(),
                        ))
                    }),
                    error,
                ))),
                ..EffectResult::default()
            },
            None => toast(error),
        }),
    }
}

/// Opens the tunnel list; the run loop re-reads the registry after every effect, so the list
/// always starts from fresh master and listener evidence.
fn tunnels(runtime: &Runtime) -> Result<EffectResult, String> {
    let listing = runtime
        .stack
        .last()
        .is_some_and(|screen| screen.kind() == keymap::Screen::Tunnels);
    Ok(EffectResult {
        push: (!listing).then(|| Box::new(Tunnels::new()) as Box<dyn Screen>),
        ..EffectResult::default()
    })
}

fn set_preference(runtime: &mut Runtime, change: PreferenceChange) -> Result<EffectResult, String> {
    let mut preferences = runtime.data.prefs.clone();
    let described = match change {
        PreferenceChange::StartMode(mode) => {
            preferences.startmode = mode;
            format!("startmode={}", format!("{mode:?}").to_lowercase())
        }
        PreferenceChange::AutoInfo(value) => {
            preferences.autoinfo = value;
            format!("autoinfo={value}")
        }
        PreferenceChange::Bind(value) => {
            let described = format!("bind={value}");
            preferences.bind = value;
            described
        }
        PreferenceChange::ClipboardClear(seconds) => {
            preferences.clipboard_clear = seconds;
            format!(
                "clipboard_clear={}",
                seconds.map_or_else(|| "off".to_string(), |seconds| seconds.to_string())
            )
        }
        PreferenceChange::PasswordConfirm(value) => {
            preferences.password_confirm = value;
            format!("password_confirm={value}")
        }
    };
    if let Err(error) = settings::save_preferences(&runtime.data.home, &preferences) {
        return Ok(toast(error));
    }
    runtime.data.prefs = preferences;
    Ok(toast(format!("✓ {described}")))
}

fn root_add(runtime: &mut Runtime, path: &str) -> Result<EffectResult, String> {
    let path = settings::normalize_path(Path::new(path), &runtime.data.home);
    let root = RegisteredRoot {
        scope: scope_for_path(&path),
        path: path.clone(),
        project: None,
    };
    match crate::register_setup_roots(&runtime.data.home, vec![root]) {
        Ok(roots) => {
            if runtime.cli.config.is_none() {
                runtime.data.roots = roots;
            }
            Ok(EffectResult {
                refresh: true,
                ..toast(format!("✓ root added: {}", path.display()))
            })
        }
        Err(error) => Ok(toast(error)),
    }
}

fn root_remove(runtime: &mut Runtime, path: &str) -> Result<EffectResult, String> {
    let home = &runtime.data.home;
    let path = settings::normalize_path(Path::new(path), home);
    let mut roots = match settings::load(home) {
        Ok(roots) => roots,
        Err(error) => return Ok(toast(error)),
    };
    let before = roots.len();
    roots.retain(|root| root.path != path);
    if roots.len() == before {
        return Ok(toast(format!(
            "ROOT_NOT_FOUND: {} is not a registered config root",
            path.display()
        )));
    }
    if let Err(error) = settings::save(home, &roots) {
        return Ok(toast(error));
    }
    if runtime.cli.config.is_none() {
        runtime.data.roots = roots;
    }
    Ok(EffectResult {
        refresh: true,
        ..toast(format!("✓ root removed: {}", path.display()))
    })
}

fn register_roots(
    runtime: &mut Runtime,
    additions: Vec<RegisteredRoot>,
) -> Result<EffectResult, String> {
    match crate::register_setup_roots(&runtime.data.home, additions) {
        Ok(roots) => {
            runtime.data.roots = roots;
            runtime.stack.clear();
            Ok(EffectResult {
                push: Some(Box::new(Finder::new(input_mode(
                    runtime.data.prefs.startmode,
                )))),
                refresh: true,
                ..toast("✓ config roots registered")
            })
        }
        Err(error) => Ok(toast(error)),
    }
}
