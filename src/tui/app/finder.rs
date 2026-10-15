use super::{
    ConnectRequest, Ctx, Effect, Outcome, Screen, ScreenStatus,
    cmdline::CommandLine,
    copy::CopyRequest,
    delete::{self, DeleteConfirm},
    host_form::{EditorRequest, HostForm, open},
    ports::Ports,
    tunnel_progress, tunnels,
};
use crate::tui::matcher::{Row, compare_rows, matching_rows, row_label, row_rank};
use crate::tui::widgets::block;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line as TextLine, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use sshx::discovery::HostEntry;
use sshx::keymap::{
    self, Action, Availability, CopyTarget, FilterTarget, GotoTarget, InputMode, Key,
};
use std::cell::Cell;
use std::collections::BTreeMap;

const RECENT_LIMIT: usize = 5;
const PREVIEW_MIN_WIDTH: u16 = 70;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RootFilter {
    #[default]
    All,
    Personal,
    Work,
    Recent,
}

impl RootFilter {
    fn label(self) -> &'static str {
        match self {
            RootFilter::All => "all",
            RootFilter::Personal => "personal",
            RootFilter::Work => "work",
            RootFilter::Recent => "recent",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stat {
    pub count: u64,
    pub last_used: Option<u64>,
}

pub(super) struct FinderRow<'a> {
    pub row: Row<'a>,
    pub label: String,
    pub scope: Option<&'a str>,
    pub recent: bool,
}

pub(super) enum Line<'a> {
    Header(String),
    Host(FinderRow<'a>),
}

fn scope_of(entry: &HostEntry) -> Option<&str> {
    entry
        .provenance
        .first()
        .map(|provenance| provenance.scope.as_str())
}

fn root_of(entry: &HostEntry) -> &str {
    entry
        .provenance
        .first()
        .and_then(|provenance| provenance.paths.first())
        .map_or("", String::as_str)
}

fn copy_row<'a>(row: &Row<'a>) -> Row<'a> {
    Row {
        entry: row.entry,
        alias: row.alias,
    }
}

/// Empty query: RECENT group, then every alias grouped by config root. Typed query: fuzzy rank,
/// then use frequency, then alias.
pub(super) fn arrange<'a>(
    entries: &[&'a HostEntry],
    query: &str,
    filter: RootFilter,
    stat: &dyn Fn(&HostEntry) -> Stat,
) -> Vec<Line<'a>> {
    let scoped = entries
        .iter()
        .copied()
        .filter(|entry| match filter {
            RootFilter::All => true,
            RootFilter::Personal => scope_of(entry) == Some("personal"),
            RootFilter::Work => scope_of(entry) == Some("work"),
            RootFilter::Recent => stat(entry).last_used.is_some(),
        })
        .collect::<Vec<_>>();
    let mut rows = matching_rows(&scoped, query);
    if !query.is_empty() {
        rows.sort_by(|left, right| {
            row_rank(left, query)
                .cmp(&row_rank(right, query))
                .then_with(|| stat(right.entry).count.cmp(&stat(left.entry).count))
                .then_with(|| compare_rows(left, right, query))
        });
    }
    let labels = rows
        .iter()
        .map(|row| row_label(row, &rows).into_owned())
        .collect::<Vec<_>>();
    let host = |index: usize, recent: bool| {
        Line::Host(FinderRow {
            row: copy_row(&rows[index]),
            label: labels[index].clone(),
            scope: scope_of(rows[index].entry),
            recent,
        })
    };

    if !query.is_empty() {
        return (0..rows.len()).map(|index| host(index, false)).collect();
    }

    let mut recent = scoped
        .iter()
        .copied()
        .filter_map(|entry| Some((stat(entry).last_used?, entry)))
        .collect::<Vec<_>>();
    recent.sort_by(|(left_time, left), (right_time, right)| {
        right_time
            .cmp(left_time)
            .then_with(|| left.aliases.first().cmp(&right.aliases.first()))
    });
    if filter != RootFilter::Recent {
        recent.truncate(RECENT_LIMIT);
    }
    let mut lines = Vec::new();
    if !recent.is_empty() {
        lines.push(Line::Header("RECENT".to_string()));
        for (_, entry) in &recent {
            let alias = entry.aliases.first().map(String::as_str);
            if let Some(index) = rows
                .iter()
                .position(|row| std::ptr::eq(row.entry, *entry) && Some(row.alias) == alias)
            {
                lines.push(host(index, true));
            }
        }
    }
    if filter == RootFilter::Recent {
        return lines;
    }
    let mut groups: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        groups
            .entry((scope_of(row.entry).unwrap_or(""), root_of(row.entry)))
            .or_default()
            .push(index);
    }
    for ((scope, root), indexes) in groups {
        lines.push(Line::Header(format!("{scope} · {root}")));
        lines.extend(indexes.into_iter().map(|index| host(index, false)));
    }
    lines
}

fn hosts<'a, 'b>(lines: &'b [Line<'a>]) -> Vec<&'b FinderRow<'a>> {
    lines
        .iter()
        .filter_map(|line| match line {
            Line::Host(row) => Some(row),
            Line::Header(_) => None,
        })
        .collect()
}

pub(super) fn relative_time(now: u64, then: u64) -> String {
    let elapsed = now.saturating_sub(then);
    match elapsed {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", elapsed / 60),
        3600..86400 => format!("{}h ago", elapsed / 3600),
        _ => format!("{}d ago", elapsed / 86400),
    }
}

pub(super) struct Finder {
    mode: InputMode,
    query: String,
    selected: usize,
    filter: RootFilter,
    viewport: Cell<usize>,
    /// Where Select input mode started; the marked rows run from here to `selected`.
    anchor: Option<usize>,
}

impl Finder {
    pub(super) fn new(mode: InputMode) -> Self {
        Self {
            mode,
            query: String::new(),
            selected: 0,
            filter: RootFilter::All,
            viewport: Cell::new(10),
            anchor: None,
        }
    }

    fn leave_select(&mut self, mode: InputMode) {
        self.mode = mode;
        self.anchor = None;
    }

    /// Rows from the Select anchor to the highlighted row, one per HostEntry alias.
    fn marked<'r, 'a>(&self, rows: &[&'r FinderRow<'a>]) -> Vec<&'r FinderRow<'a>> {
        let Some(anchor) = self.anchor.filter(|_| self.mode == InputMode::Select) else {
            return Vec::new();
        };
        let last = rows.len().saturating_sub(1);
        let (from, to) = (anchor.min(self.selected), anchor.max(self.selected));
        let (from, to) = (from.min(last), to.min(last));
        let mut chosen: Vec<&FinderRow> = Vec::new();
        for row in rows.iter().skip(from).take(to + 1 - from) {
            let known = |other: &&FinderRow| {
                other.row.entry.id == row.row.entry.id && other.row.alias == row.row.alias
            };
            if !chosen.iter().any(known) {
                chosen.push(row);
            }
        }
        chosen
    }

    fn marked_pairs<'r>(&self, rows: &[&'r FinderRow<'_>]) -> Vec<(&'r HostEntry, &'r str)> {
        self.marked(rows)
            .into_iter()
            .map(|row| (row.row.entry, row.row.alias))
            .collect()
    }

    pub(super) fn select_entry(&mut self, ctx: &Ctx, id: &str, alias: &str) {
        let lines = self.lines(ctx);
        if let Some(position) = hosts(&lines)
            .iter()
            .position(|row| row.row.entry.id == id && row.row.alias == alias)
        {
            self.selected = position;
        }
    }

    fn lines<'a>(&self, ctx: &Ctx<'a>) -> Vec<Line<'a>> {
        arrange(&ctx.visible, &self.query, self.filter, &|entry| {
            ctx.stat(entry)
        })
    }

    fn jump(&mut self, lines: &[Line], wanted: impl Fn(&FinderRow) -> bool, what: &str) -> Outcome {
        match hosts(lines).iter().position(|row| wanted(row)) {
            Some(position) => {
                self.selected = position;
                Outcome::Stay
            }
            None => Outcome::Toast(format!("no {what} HostEntries")),
        }
    }

    fn preview<'a>(&self, row: &FinderRow, ctx: &Ctx) -> Vec<TextLine<'a>> {
        let entry = row.row.entry;
        let label =
            |text: &str| Span::styled(format!("{text:<9}"), Style::default().fg(Color::DarkGray));
        let route = match sshx::pair::paired_route(ctx.entries, entry) {
            Ok(Some(route)) => format!(
                "{} → {}",
                route.gateway.aliases.first().map_or("?", String::as_str),
                row.row.alias
            ),
            Ok(None) => "direct".to_string(),
            Err(error) => error,
        };
        let services = match sshx::session::declared_services(entry) {
            Ok(list) if list.is_empty() => "none declared".to_string(),
            Ok(list) => list
                .iter()
                .map(|service| match &service.name {
                    Some(name) => {
                        format!(
                            "{name} {}→{}",
                            service.default_local_port, service.remote_port
                        )
                    }
                    None => format!("{}→{}", service.default_local_port, service.remote_port),
                })
                .collect::<Vec<_>>()
                .join(", "),
            Err(error) => error,
        };
        let last_use = ctx
            .stat(entry)
            .last_used
            .map_or_else(|| "never".to_string(), |time| relative_time(ctx.now, time));
        vec![
            TextLine::from(Span::styled(
                row.row.alias.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            TextLine::default(),
            TextLine::from(vec![label("Route"), Span::raw(route)]),
            TextLine::from(vec![
                label("HostName"),
                Span::raw(entry.destination.clone().unwrap_or_else(|| "-".into())),
            ]),
            TextLine::from(vec![label("Services"), Span::raw(services)]),
            TextLine::from(vec![label("Last use"), Span::raw(last_use)]),
            TextLine::default(),
            TextLine::from(vec![
                label("Source"),
                Span::raw(format!("{}:{}", entry.source.path, entry.source.line_start)),
            ]),
            TextLine::from(vec![label("ID"), Span::raw(entry.id.clone())]),
        ]
    }
}

impl Screen for Finder {
    fn kind(&self) -> keymap::Screen {
        keymap::Screen::Finder
    }

    fn input_mode(&self) -> InputMode {
        self.mode
    }

    fn query_is_empty(&self) -> bool {
        self.query.is_empty()
    }

    fn availability(&self, ctx: &Ctx) -> Availability {
        let lines = self.lines(ctx);
        let rows = hosts(&lines);
        let Some(row) = rows.get(self.selected.min(rows.len().saturating_sub(1))) else {
            return Availability::default();
        };
        let entry = row.row.entry;
        Availability {
            has_password: sshx::connect::has_stored_password(entry).unwrap_or(false),
            is_pair: matches!(sshx::pair::paired_route(ctx.entries, entry), Ok(Some(_))),
            tunnel_selected: ctx.tunnels.iter().any(|tunnel| {
                tunnel.state == "active"
                    && (tunnel.entry_id == entry.id
                        || tunnel.vm_entry_id.as_deref() == Some(entry.id.as_str()))
            }),
            row_selected: true,
        }
    }

    fn on_action(&mut self, action: Action, count: usize, ctx: &Ctx) -> Outcome {
        let lines = self.lines(ctx);
        let rows = hosts(&lines);
        let total = rows.len();
        let last = total.saturating_sub(1);
        let step = (self.viewport.get() / 2).max(1);
        match action {
            Action::Up => self.selected = self.selected.saturating_sub(count),
            Action::Down => self.selected = (self.selected + count).min(last),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            Action::HalfPageDown => self.selected = (self.selected + count * step).min(last),
            Action::HalfPageUp => self.selected = self.selected.saturating_sub(count * step),
            Action::EnterInsert => {
                self.mode = InputMode::Insert;
                self.anchor = None;
            }
            Action::EnterNormal => self.leave_select(InputMode::Normal),
            Action::EnterSelect => {
                self.mode = InputMode::Select;
                self.anchor = Some(self.selected.min(last));
            }
            Action::Copy(_) if self.mode == InputMode::Select => {
                let requests = self
                    .marked(&rows)
                    .iter()
                    .map(|row| CopyRequest {
                        target: CopyTarget::SshCommand,
                        entry: row.row.entry.clone(),
                        alias: row.row.alias.to_string(),
                        forwards: Vec::new(),
                        tunnel_id: None,
                    })
                    .collect();
                self.leave_select(InputMode::Normal);
                return Outcome::Effect(Effect::CopyMany(requests));
            }
            Action::DeleteHost if self.mode == InputMode::Select => {
                let outcome = delete::begin_many(ctx, &self.marked_pairs(&rows));
                self.leave_select(InputMode::Normal);
                return outcome;
            }
            Action::TunnelStart if self.mode == InputMode::Select => {
                let outcome = tunnel_progress::begin_many(ctx, &self.marked_pairs(&rows));
                self.leave_select(InputMode::Normal);
                return outcome;
            }
            Action::Connect => {
                let Some(row) = rows.get(self.selected.min(last)) else {
                    return Outcome::Stay;
                };
                return Outcome::Effect(Effect::Connect(Box::new(ConnectRequest {
                    entry: row.row.entry.clone(),
                    alias: row.row.alias.to_string(),
                    forwards: Vec::new(),
                })));
            }
            Action::OpenPorts => {
                let Some(row) = rows.get(self.selected.min(last)) else {
                    return Outcome::Stay;
                };
                return match Ports::open(ctx, row.row.entry, row.row.alias) {
                    Ok(ports) => Outcome::Push(Box::new(ports)),
                    Err(error) => Outcome::Toast(error),
                };
            }
            Action::Copy(target) => {
                let Some(row) = rows.get(self.selected.min(last)) else {
                    return Outcome::Stay;
                };
                let entry = row.row.entry;
                let tunnel_id = ctx
                    .tunnels
                    .iter()
                    .find(|tunnel| {
                        tunnel.state == "active"
                            && (tunnel.entry_id == entry.id
                                || tunnel.vm_entry_id.as_deref() == Some(entry.id.as_str()))
                    })
                    .map(|tunnel| tunnel.id.clone());
                return Outcome::Effect(Effect::Copy(Box::new(CopyRequest {
                    target,
                    entry: entry.clone(),
                    alias: row.row.alias.to_string(),
                    forwards: Vec::new(),
                    tunnel_id,
                })));
            }
            Action::NewHost => return open(HostForm::create(ctx)),
            Action::EditHostForm | Action::EditHostInEditor | Action::DeleteHost => {
                let Some(row) = rows.get(self.selected.min(last)) else {
                    return Outcome::Stay;
                };
                let (entry, alias) = (row.row.entry, row.row.alias);
                return match action {
                    Action::EditHostForm => open(HostForm::edit(ctx, entry, alias)),
                    Action::EditHostInEditor => {
                        Outcome::Effect(Effect::EditInEditor(Box::new(EditorRequest::new(entry))))
                    }
                    _ => open(DeleteConfirm::new(ctx, entry, alias)),
                };
            }
            Action::Undo => return Outcome::Effect(Effect::Undo),
            Action::Redo => return Outcome::Effect(Effect::Redo),
            Action::TunnelStart => {
                let Some(row) = rows.get(self.selected.min(last)) else {
                    return Outcome::Stay;
                };
                let entry = row.row.entry;
                return tunnel_progress::begin(
                    ctx,
                    entry,
                    row.row.alias,
                    tunnel_progress::remembered_forwards(ctx, entry),
                );
            }
            Action::TunnelStopAll => return tunnels::stop_all(ctx),
            Action::CommandLine => {
                let mut line = CommandLine::new(self.kind(), self.mode, self.availability(ctx));
                if let Some(row) = rows.get(self.selected.min(last)) {
                    line = line.with_target(row.row.entry, row.row.alias);
                }
                return Outcome::Push(Box::new(line));
            }
            Action::Quit => return Outcome::Effect(Effect::Quit { cancelled: false }),
            Action::Goto(GotoTarget::Recent) => {
                return self.jump(&lines, |row| row.recent, "recent");
            }
            Action::Goto(GotoTarget::Personal) => {
                return self.jump(
                    &lines,
                    |row| !row.recent && row.scope == Some("personal"),
                    "personal",
                );
            }
            Action::Goto(GotoTarget::Work) => {
                return self.jump(
                    &lines,
                    |row| !row.recent && row.scope == Some("work"),
                    "work",
                );
            }
            Action::Goto(GotoTarget::Tunnels) | Action::TunnelList => {
                return Outcome::Effect(Effect::Tunnels);
            }
            Action::Filter(target) => {
                let wanted = match target {
                    FilterTarget::Personal => RootFilter::Personal,
                    FilterTarget::Work => RootFilter::Work,
                    FilterTarget::Recent => RootFilter::Recent,
                };
                self.filter = if self.filter == wanted {
                    RootFilter::All
                } else {
                    wanted
                };
                self.selected = 0;
                return Outcome::Toast(format!("filter: {}", self.filter.label()));
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn on_key(&mut self, key: Key, _ctx: &Ctx) -> Outcome {
        if self.mode == InputMode::Insert {
            match key {
                Key::Char(character) => {
                    self.query.push(character);
                    self.selected = 0;
                }
                Key::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                }
                _ => {}
            }
        }
        Outcome::Stay
    }

    fn status(&self, ctx: &Ctx) -> ScreenStatus {
        let lines = self.lines(ctx);
        let rows = hosts(&lines);
        let total = rows.len();
        let mut filter = self.filter.label().to_string();
        if self.mode == InputMode::Select {
            filter = format!("{filter} · {} selected", self.marked(&rows).len());
        }
        ScreenStatus {
            filter: Some(filter),
            position: Some((
                if total == 0 {
                    0
                } else {
                    self.selected.min(total - 1) + 1
                },
                total,
            )),
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let lines = self.lines(ctx);
        let rows = hosts(&lines);
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let outer = block(" sshx ", Color::Cyan);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let (list_area, preview_area) = if inner.width >= PREVIEW_MIN_WIDTH {
            let [list, preview] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(inner);
            (list, Some(preview))
        } else {
            (inner, None)
        };
        let [search_area, rows_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(list_area);
        self.viewport.set(usize::from(rows_area.height));
        let cursor = if self.mode == InputMode::Insert {
            "▏"
        } else {
            ""
        };
        frame.render_widget(
            Paragraph::new(TextLine::from(vec![
                Span::styled("Search: ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{}{cursor}", self.query)),
            ])),
            search_area,
        );

        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching HostEntry aliases.")
                    .style(Style::default().fg(Color::DarkGray)),
                rows_area,
            );
        } else {
            let width = usize::from(rows_area.width).saturating_sub(2);
            let marked = self
                .anchor
                .filter(|_| self.mode == InputMode::Select)
                .map(|anchor| (anchor.min(selected), anchor.max(selected)));
            let mut host_index = 0;
            let mut highlighted = 0;
            let items = lines
                .iter()
                .enumerate()
                .map(|(position, line)| match line {
                    Line::Header(title) => ListItem::new(TextLine::from(Span::styled(
                        match ctx.home.to_str() {
                            Some(home) if !home.is_empty() => title.replacen(home, "~", 1),
                            _ => title.clone(),
                        },
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ))),
                    Line::Host(row) => {
                        if host_index == selected {
                            highlighted = position;
                        }
                        let marker = match marked {
                            Some((from, to)) if (from..=to).contains(&host_index) => "● ",
                            Some(_) => "  ",
                            None => "",
                        };
                        host_index += 1;
                        let count = ctx.service_count(row.row.entry);
                        let badge = if count > 0 {
                            format!("⇄{count}")
                        } else {
                            String::new()
                        };
                        let label = row.label.clone();
                        let used =
                            marker.chars().count() + label.chars().count() + badge.chars().count();
                        let pad = width.saturating_sub(used).max(1);
                        ListItem::new(TextLine::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Yellow)),
                            Span::raw(label),
                            Span::raw(" ".repeat(pad)),
                            Span::styled(badge, Style::default().fg(Color::Cyan)),
                        ]))
                    }
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(highlighted));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                rows_area,
                &mut state,
            );
        }

        if let Some(preview_area) = preview_area {
            let content = rows
                .get(selected)
                .map(|row| self.preview(row, ctx))
                .unwrap_or_default();
            frame.render_widget(
                Paragraph::new(content)
                    .wrap(Wrap { trim: false })
                    .block(block(" Preview ", Color::DarkGray)),
                preview_area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, RootFilter, Stat, arrange, relative_time};
    use sshx::discovery::{HostEntry, Provenance, SourceIdentity};
    use std::collections::HashMap;

    fn host(id: &str, alias: &str, scope: &str, root: &str) -> HostEntry {
        HostEntry {
            id: id.to_string(),
            aliases: vec![alias.to_string()],
            source: SourceIdentity {
                path: root.to_string(),
                byte_start: 0,
                byte_end: 1,
                line_start: 1,
                line_end: 1,
            },
            destination: None,
            provenance: vec![Provenance {
                paths: vec![root.to_string()],
                scope: scope.to_string(),
                project: None,
            }],
            scopes: Vec::new(),
            projects: Vec::new(),
        }
    }

    fn stats(values: &[(&str, u64, Option<u64>)]) -> HashMap<String, Stat> {
        values
            .iter()
            .map(|(id, count, last_used)| {
                (
                    (*id).to_string(),
                    Stat {
                        count: *count,
                        last_used: *last_used,
                    },
                )
            })
            .collect()
    }

    fn render(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                Line::Header(title) => format!("# {title}"),
                Line::Host(row) => row.row.alias.to_string(),
            })
            .collect()
    }

    #[test]
    fn empty_query_shows_recent_first_then_groups_by_root() {
        let entries = [
            host("a", "alpha", "personal", "/home/.ssh/config"),
            host("b", "bravo", "personal", "/home/.ssh/config"),
            host("c", "charlie", "work", "/work/config"),
        ];
        let refs = entries.iter().collect::<Vec<_>>();
        let usage = stats(&[("c", 1, Some(200)), ("b", 3, Some(100))]);
        let lines = arrange(&refs, "", RootFilter::All, &|entry| {
            usage.get(&entry.id).copied().unwrap_or_default()
        });
        assert_eq!(
            render(&lines),
            [
                "# RECENT",
                "charlie",
                "bravo",
                "# personal · /home/.ssh/config",
                "alpha",
                "bravo",
                "# work · /work/config",
                "charlie",
            ]
        );
    }

    #[test]
    fn recent_group_is_capped_but_recent_filter_lists_every_used_host() {
        let entries = (0..7)
            .map(|index| {
                host(
                    &format!("id{index}"),
                    &format!("h{index}"),
                    "personal",
                    "/c",
                )
            })
            .collect::<Vec<_>>();
        let refs = entries.iter().collect::<Vec<_>>();
        let usage = (0..7)
            .map(|index| {
                (
                    format!("id{index}"),
                    Stat {
                        count: 1,
                        last_used: Some(100 + index as u64),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let stat = |entry: &HostEntry| usage.get(&entry.id).copied().unwrap_or_default();
        let capped = arrange(&refs, "", RootFilter::All, &stat);
        let recent_count = capped
            .iter()
            .take_while(|line| !matches!(line, Line::Header(title) if title.contains('·')))
            .filter(|line| matches!(line, Line::Host(_)))
            .count();
        assert_eq!(recent_count, 5);
        let filtered = arrange(&refs, "", RootFilter::Recent, &stat);
        assert_eq!(filtered.len(), 8, "header plus seven used hosts");
    }

    #[test]
    fn equal_fuzzy_rank_is_broken_by_use_frequency_then_alias() {
        let entries = [
            host("a", "db-a", "personal", "/c"),
            host("b", "db-b", "personal", "/c"),
            host("c", "db-c", "personal", "/c"),
        ];
        let refs = entries.iter().collect::<Vec<_>>();
        let usage = stats(&[("b", 9, None), ("c", 2, None)]);
        let lines = arrange(&refs, "db", RootFilter::All, &|entry| {
            usage.get(&entry.id).copied().unwrap_or_default()
        });
        assert_eq!(render(&lines), ["db-b", "db-c", "db-a"]);
    }

    #[test]
    fn exact_alias_still_beats_a_more_used_prefix_match() {
        let entries = [
            host("a", "db", "personal", "/c"),
            host("b", "db-prod", "personal", "/c"),
        ];
        let refs = entries.iter().collect::<Vec<_>>();
        let usage = stats(&[("b", 50, None)]);
        let lines = arrange(&refs, "db", RootFilter::All, &|entry| {
            usage.get(&entry.id).copied().unwrap_or_default()
        });
        assert_eq!(render(&lines), ["db", "db-prod"]);
    }

    #[test]
    fn scope_filter_keeps_only_that_root_scope() {
        let entries = [
            host("a", "alpha", "personal", "/p"),
            host("c", "charlie", "work", "/w"),
        ];
        let refs = entries.iter().collect::<Vec<_>>();
        let lines = arrange(&refs, "", RootFilter::Work, &|_| Stat::default());
        assert_eq!(render(&lines), ["# work · /w", "charlie"]);
    }

    #[test]
    fn duplicate_aliases_keep_source_and_identity_labels() {
        let entries = [
            host("1111", "shared", "personal", "/one/config"),
            host("2222", "shared", "personal", "/one/config"),
        ];
        let refs = entries.iter().collect::<Vec<_>>();
        let lines = arrange(&refs, "", RootFilter::All, &|_| Stat::default());
        let labels = lines
            .iter()
            .filter_map(|line| match line {
                Line::Host(row) => Some(row.label.as_str()),
                Line::Header(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "shared · /one/config:1 · 1111",
                "shared · /one/config:1 · 2222"
            ]
        );
    }

    #[test]
    fn relative_time_uses_the_largest_whole_unit() {
        assert_eq!(relative_time(100, 100), "just now");
        assert_eq!(relative_time(400, 100), "5m ago");
        assert_eq!(relative_time(100 + 3 * 3600, 100), "3h ago");
        assert_eq!(relative_time(100 + 2 * 86400, 100), "2d ago");
    }
}
