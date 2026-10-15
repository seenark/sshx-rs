use super::{
    ConnectRequest, Ctx, Effect, ForwardRequest, Outcome, PreferenceChange, Screen, help::Help,
    tunnel_progress,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{self, Action, Availability, Command, ForwardArg, InputMode, Key};
use sshx::settings::StartMode;

const CANDIDATE_ROWS: usize = 8;

pub(super) struct CommandLine {
    under: keymap::Screen,
    under_mode: InputMode,
    under_availability: Availability,
    target: Option<(HostEntry, String)>,
    text: String,
}

fn parse_bool(option: &str, value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("true" | "on" | "yes" | "1") => Ok(true),
        Some("false" | "off" | "no" | "0") => Ok(false),
        Some(other) => Err(format!("{option} expects true or false, not `{other}`")),
    }
}

pub(super) fn validate_bind(value: &str) -> Result<String, String> {
    let address = if value.contains(':') && !value.starts_with('[') {
        format!("[{value}]")
    } else {
        value.to_string()
    };
    sshx::session::resolve_custom_local_forwards(&[format!("{address}:1:127.0.0.1:1")], false)?;
    Ok(value.to_string())
}

pub(super) fn preference_change(
    option: &str,
    value: Option<&str>,
) -> Result<PreferenceChange, String> {
    let required = || format!("{option} needs a value: `:set {option}=<value>`");
    match option {
        "startmode" => match value.ok_or_else(required)? {
            "insert" => Ok(PreferenceChange::StartMode(StartMode::Insert)),
            "normal" => Ok(PreferenceChange::StartMode(StartMode::Normal)),
            other => Err(format!("startmode expects insert or normal, not `{other}`")),
        },
        "autoinfo" => Ok(PreferenceChange::AutoInfo(parse_bool(option, value)?)),
        "password_confirm" => Ok(PreferenceChange::PasswordConfirm(parse_bool(
            option, value,
        )?)),
        "bind" => Ok(PreferenceChange::Bind(validate_bind(
            value.ok_or_else(required)?,
        )?)),
        "clipboard_clear" => match value.ok_or_else(required)? {
            "off" => Ok(PreferenceChange::ClipboardClear(None)),
            seconds => seconds
                .parse::<u64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .map(|seconds| PreferenceChange::ClipboardClear(Some(seconds)))
                .ok_or_else(|| format!("clipboard_clear expects off or seconds, not `{seconds}`")),
        },
        other => Err(format!("unknown option `{other}`")),
    }
}

fn forward_requests(
    entry: &HostEntry,
    arguments: &[ForwardArg],
) -> Result<Vec<ForwardRequest>, String> {
    let services = sshx::session::declared_services(entry)?;
    Ok(arguments
        .iter()
        .map(|argument| ForwardRequest {
            remote: argument.remote,
            local: argument.local.unwrap_or_else(|| {
                services
                    .iter()
                    .find(|service| service.remote_port == argument.remote)
                    .map_or(argument.remote, |service| service.default_local_port)
            }),
        })
        .collect())
}

fn resolve_host<'a>(ctx: &Ctx<'a>, host: &str) -> Result<(&'a HostEntry, String), String> {
    let by_alias = ctx
        .visible
        .iter()
        .copied()
        .filter(|entry| entry.aliases.iter().any(|alias| alias == host))
        .collect::<Vec<_>>();
    let matches = if by_alias.is_empty() {
        ctx.visible
            .iter()
            .copied()
            .filter(|entry| entry.id == host)
            .collect()
    } else {
        by_alias
    };
    match matches.as_slice() {
        [entry] => {
            let alias = entry
                .aliases
                .iter()
                .find(|alias| *alias == host)
                .or_else(|| entry.aliases.first())
                .cloned()
                .unwrap_or_default();
            Ok((entry, alias))
        }
        [] => Err(format!("HOST_NOT_FOUND: no HostEntry matches `{host}`")),
        many => Err(crate::format_ambiguous(host, many)),
    }
}

fn completion(text: &str) -> Option<String> {
    let typed = text.trim_start();
    let candidates = keymap::complete_command(typed);
    let (first, rest) = candidates.split_first()?;
    if rest.is_empty() {
        return Some(if first.ends_with('=') {
            (*first).to_string()
        } else {
            format!("{first} ")
        });
    }
    let common = rest.iter().fold(first.to_string(), |prefix, candidate| {
        prefix
            .chars()
            .zip(candidate.chars())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character)
            .collect()
    });
    (common.len() > typed.len()).then_some(common)
}

impl CommandLine {
    pub(super) fn new(
        under: keymap::Screen,
        under_mode: InputMode,
        under_availability: Availability,
    ) -> Self {
        Self {
            under,
            under_mode,
            under_availability,
            target: None,
            text: String::new(),
        }
    }

    pub(super) fn with_target(mut self, entry: &HostEntry, alias: &str) -> Self {
        self.target = Some((entry.clone(), alias.to_string()));
        self
    }
    fn run(&self, ctx: &Ctx) -> Outcome {
        let command = match keymap::parse_command(&self.text) {
            Ok(command) => command,
            Err(error) => return Outcome::Toast(error),
        };
        match command {
            Command::Quit => Outcome::Effect(Effect::Quit { cancelled: false }),
            Command::Write | Command::WriteQuit => {
                Outcome::Toast(":w and :wq apply to the host form".to_string())
            }
            Command::Help => Outcome::Replace(Box::new(Help::new(
                self.under,
                self.under_mode,
                self.under_availability,
            ))),
            Command::Ssh { host, forwards } => {
                let request = resolve_host(ctx, &host).and_then(|(entry, alias)| {
                    Ok(ConnectRequest {
                        forwards: forward_requests(entry, &forwards)?,
                        entry: entry.clone(),
                        alias,
                    })
                });
                match request {
                    Ok(request) => Outcome::Effect(Effect::Connect(Box::new(request))),
                    Err(error) => Outcome::Toast(error),
                }
            }
            Command::TunnelList => Outcome::Effect(Effect::Tunnels),
            Command::TunnelStart { host, forwards } => {
                let started = match host {
                    Some(host) => {
                        resolve_host(ctx, &host).map(|(entry, alias)| (entry.clone(), alias))
                    }
                    None => self
                        .target
                        .clone()
                        .ok_or_else(|| "nothing selected: name a host".to_string()),
                }
                .and_then(|(entry, alias)| {
                    let forwards = if forwards.is_empty() {
                        tunnel_progress::remembered_forwards(ctx, &entry)
                    } else {
                        forward_requests(&entry, &forwards)?
                    };
                    Ok(tunnel_progress::begin(ctx, &entry, &alias, forwards))
                });
                match started {
                    Ok(Outcome::Push(screen)) => Outcome::Replace(screen),
                    Ok(outcome) => outcome,
                    Err(error) => Outcome::Toast(error),
                }
            }
            Command::TunnelStop { id } => {
                if ctx.tunnels.iter().any(|tunnel| tunnel.id == id) {
                    Outcome::Effect(Effect::TunnelStop(vec![id]))
                } else {
                    Outcome::Toast(format!("TUNNEL_NOT_FOUND: tunnel `{id}` does not exist"))
                }
            }
            Command::Set { option, value } => match preference_change(&option, value.as_deref()) {
                Ok(change) => Outcome::Effect(Effect::SetPreference(change)),
                Err(error) => Outcome::Toast(error),
            },
            Command::RootAdd { path } => Outcome::Effect(Effect::RootAdd(path)),
            Command::RootRemove { path } => Outcome::Effect(Effect::RootRemove(path)),
        }
    }
}

impl Screen for CommandLine {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::CommandLine
    }

    fn input_mode(&self) -> InputMode {
        InputMode::Insert
    }

    fn availability(&self, _ctx: &Ctx) -> Availability {
        Availability::default()
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn dismiss_on_outcome(&self) -> bool {
        true
    }

    fn on_action(&mut self, action: Action, _count: usize, ctx: &Ctx) -> Outcome {
        match action {
            Action::Back => Outcome::Pop,
            Action::Submit => self.run(ctx),
            Action::Complete => {
                if let Some(completed) = completion(&self.text) {
                    self.text = completed;
                }
                Outcome::Stay
            }
            Action::Quit => Outcome::Effect(Effect::Quit { cancelled: false }),
            _ => Outcome::Stay,
        }
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        match key {
            Key::Char(character) => self.text.push(character),
            Key::Backspace if self.text.is_empty() => return Outcome::Pop,
            Key::Backspace => {
                self.text.pop();
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn draw(&self, frame: &mut Frame, area: Rect, _ctx: &Ctx) {
        let prompt = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let candidates = keymap::complete_command(&self.text)
            .into_iter()
            .take(CANDIDATE_ROWS)
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            let width = candidates
                .iter()
                .map(|candidate| candidate.chars().count())
                .max()
                .unwrap_or(0)
                + 4;
            let height = u16::try_from(candidates.len() + 2).unwrap_or(u16::MAX);
            let popup = Rect::new(
                area.x,
                prompt.y.saturating_sub(height),
                u16::try_from(width).unwrap_or(u16::MAX).min(area.width),
                height.min(prompt.y.saturating_sub(area.y)),
            );
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(
                    candidates
                        .iter()
                        .map(|candidate| Line::from(format!(":{candidate}")))
                        .collect::<Vec<_>>(),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray)),
                ),
                popup,
            );
        }
        frame.render_widget(Clear, prompt);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(":", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}▏", self.text)),
            ])),
            prompt,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{completion, preference_change};
    use crate::tui::app::PreferenceChange;
    use sshx::settings::StartMode;

    #[test]
    fn set_validates_each_option() {
        assert!(matches!(
            preference_change("autoinfo", Some("false")),
            Ok(PreferenceChange::AutoInfo(false))
        ));
        assert!(matches!(
            preference_change("startmode", Some("normal")),
            Ok(PreferenceChange::StartMode(StartMode::Normal))
        ));
        assert!(matches!(
            preference_change("clipboard_clear", Some("off")),
            Ok(PreferenceChange::ClipboardClear(None))
        ));
        assert!(matches!(
            preference_change("clipboard_clear", Some("45")),
            Ok(PreferenceChange::ClipboardClear(Some(45)))
        ));
        assert!(preference_change("clipboard_clear", Some("0")).is_err());
        assert!(preference_change("startmode", Some("visual")).is_err());
        assert!(preference_change("startmode", None).is_err());
    }

    #[test]
    fn bind_refuses_non_loopback_like_forwards_do() {
        assert!(matches!(
            preference_change("bind", Some("127.0.0.2")),
            Ok(PreferenceChange::Bind(_))
        ));
        assert!(matches!(
            preference_change("bind", Some("::1")),
            Ok(PreferenceChange::Bind(_))
        ));
        let error = preference_change("bind", Some("0.0.0.0"))
            .err()
            .expect("wildcard bind must be refused");
        assert!(error.starts_with("FORWARD_BIND_INVALID"), "{error}");
        let error = preference_change("bind", Some("192.168.1.5"))
            .err()
            .expect("non-loopback bind must be refused");
        assert!(error.starts_with("FORWARD_BIND_UNSAFE"), "{error}");
    }

    #[test]
    fn tab_completes_unique_commands_and_common_prefixes() {
        assert_eq!(completion("he").as_deref(), Some("help "));
        assert_eq!(
            completion("set clip").as_deref(),
            Some("set clipboard_clear=")
        );
        assert_eq!(completion("set "), None);
        assert_eq!(completion("zz"), None);
    }
}
