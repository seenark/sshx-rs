use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{List, ListItem, ListState, Paragraph},
};
use sshx::discovery::HostEntry;

use super::matcher::{Row, compare_rows, row_rank};
use super::terminal::{read_key, with_terminal};
use super::widgets::wrap_status;

#[derive(Clone, Debug, Default)]
pub(crate) struct PairSetupDraft {
    pub gateway: Option<(usize, usize)>,
    pub vm: Option<(usize, usize)>,
    pub transit_host: String,
    pub transit_port: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PairSetupAction {
    Review,
    Apply,
}

fn pair_setup_entry<'a>(
    entries: &[&'a HostEntry],
    pick: Option<(usize, usize)>,
) -> Option<Row<'a>> {
    let (entry, alias) = pick?;
    let entry = *entries.get(entry)?;
    Some(Row {
        entry,
        alias: entry.aliases.get(alias)?.as_str(),
    })
}

fn pair_setup_options(
    entries: &[&HostEntry],
    eligible: &[usize],
    query: &str,
) -> Vec<(usize, usize)> {
    let mut options = eligible
        .iter()
        .copied()
        .flat_map(|index| (0..entries[index].aliases.len()).map(move |alias| (index, alias)))
        .filter(|pick| row_rank(&pair_setup_entry(entries, Some(*pick)).unwrap(), query).is_some())
        .collect::<Vec<_>>();
    options.sort_by(|left, right| {
        compare_rows(
            &pair_setup_entry(entries, Some(*left)).unwrap(),
            &pair_setup_entry(entries, Some(*right)).unwrap(),
            query,
        )
    });
    options
}

type PairSetupRoute = (usize, Result<Vec<(String, u16)>, String>);

fn pair_setup_routes(
    entries: &[&HostEntry],
    eligible: &[usize],
    gateway: Option<(usize, usize)>,
) -> Vec<PairSetupRoute> {
    let Some(gateway) = pair_setup_entry(entries, gateway) else {
        return Vec::new();
    };
    eligible
        .iter()
        .copied()
        .filter(|index| {
            let source = &entries[*index].source;
            source.path != gateway.entry.source.path
                || source.byte_start != gateway.entry.source.byte_start
                || source.byte_end != gateway.entry.source.byte_end
        })
        .map(|index| {
            (
                index,
                sshx::pair::transit_candidates(gateway.entry, entries[index]),
            )
        })
        .collect()
}

fn pair_setup_vm_eligible(
    eligible: &[usize],
    gateway: Option<(usize, usize)>,
    routes: &[PairSetupRoute],
) -> Vec<usize> {
    if gateway.is_none() {
        return eligible.to_vec();
    }
    routes.iter().map(|(index, _)| *index).collect()
}

pub(crate) fn pair_setup_workspace(
    entries: &[&HostEntry],
    mut draft: PairSetupDraft,
    preview: bool,
    mut execute: impl FnMut(&PairSetupDraft, PairSetupAction) -> Result<String, String>,
) -> Result<bool, String> {
    with_terminal(
        "Pair setup",
        "PAIR_REQUIRED: Pair setup requires usable stdin and stderr terminals".to_string(),
        |terminal, input| {
            let eligible = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| sshx::pair::setup_eligible(entry).then_some(index))
                .collect::<Vec<_>>();
            let mut routes = pair_setup_routes(entries, &eligible, draft.gateway);
            let mut vm_eligible = pair_setup_vm_eligible(&eligible, draft.gateway, &routes);
            let empty_candidates = Ok(Vec::<(String, u16)>::new());
            let initial_candidates = draft
                .vm
                .and_then(|(index, _)| {
                    routes
                        .iter()
                        .find(|(entry, _)| *entry == index)
                        .map(|(_, result)| result)
                })
                .unwrap_or(&empty_candidates);
            let mut focus = if draft.gateway.is_none() {
                0
            } else if draft.vm.is_none() {
                1
            } else if draft.transit_host.is_empty() != draft.transit_port.is_empty()
                || (draft.transit_host.is_empty()
                    && !initial_candidates
                        .as_ref()
                        .is_ok_and(|values| values.len() == 1))
            {
                2
            } else {
                0
            };
            let mut queries = [String::new(), String::new()];
            let mut selected = [0usize; 2];
            for (index, selected) in selected.iter_mut().enumerate() {
                let options = pair_setup_options(
                    entries,
                    if index == 0 { &eligible } else { &vm_eligible },
                    "",
                );
                let pick = if index == 0 { draft.gateway } else { draft.vm };
                *selected = options
                    .iter()
                    .position(|option| Some(*option) == pick)
                    .unwrap_or(0);
            }
            let mut list_state = ListState::default();
            let mut candidate_index = None::<usize>;
            let mut review = None::<String>;
            let mut status = None::<(String, bool)>;
            let mut completed = false;
            let mut scroll = 0u16;
            let mut scroll_step = 1u16;
            loop {
                let candidates = draft
                    .vm
                    .and_then(|(index, _)| {
                        routes
                            .iter()
                            .find(|(entry, _)| *entry == index)
                            .map(|(_, result)| result)
                    })
                    .unwrap_or(&empty_candidates);
                let options = if focus < 2 && review.is_none() {
                    pair_setup_options(
                        entries,
                        if focus == 0 { &eligible } else { &vm_eligible },
                        &queries[focus],
                    )
                } else {
                    Vec::new()
                };
                if focus < 2 && review.is_none() {
                    selected[focus] = selected[focus].min(options.len().saturating_sub(1));
                }
                let gateway = pair_setup_entry(entries, draft.gateway);
                let vm = pair_setup_entry(entries, draft.vm);
                let vm_ineligible = draft.gateway.is_some()
                    && draft
                        .vm
                        .is_some_and(|(index, _)| !vm_eligible.contains(&index));
                let vm_warning = "Selected VM is not a distinct eligible HostEntry; value kept. Select another HostEntry.";
                let transit_status = match candidates {
                    Ok(values) if values.len() == 1 => {
                        format!("Automatic transit: {}:{}", values[0].0, values[0].1)
                    }
                    Ok(values) if values.len() > 1 => {
                        "Ambiguous transit: Ctrl-N chooses a candidate; or enter host and port.".to_string()
                    }
                    Ok(_) => "No transit candidates: check gateway LocalForward and VM Port; enter matching transit host and port.".to_string(),
                    Err(error) => format!("Transit: {error}"),
                };
                let mut details = String::new();
                if focus < 2 {
                    if let Some(pick) = options.get(selected[focus]) {
                        let row = pair_setup_entry(entries, Some(*pick)).unwrap();
                        details.push_str(&format!(
                            "Source: {}:{}\nID: {}\nChoose: {}\n",
                            row.entry.source.path,
                            row.entry.source.line_start,
                            row.entry.id,
                            row.alias,
                        ));
                    } else {
                        details.push_str("No eligible matching HostEntry aliases.\n");
                    }
                } else {
                    for (label, row) in [("Gateway", gateway.as_ref()), ("VM", vm.as_ref())] {
                        if let Some(row) = row {
                            details.push_str(&format!(
                                "{label}: {}\nSource: {}:{}\nID: {}\n",
                                row.alias,
                                row.entry.source.path,
                                row.entry.source.line_start,
                                row.entry.id,
                            ));
                        }
                    }
                }
                if vm_ineligible {
                    details.push_str(vm_warning);
                    details.push('\n');
                }
                details.push_str(&transit_status);
                details.push('\n');
                if let Ok(values) = candidates {
                    for (host, port) in values {
                        details.push_str(&format!("Transit choice: {host}:{port}\n"));
                    }
                }
                let success = status.as_ref().is_some_and(|(_, success)| *success);
                let applied = success && !preview;
                terminal.draw(|frame| {
                    let area = frame.area();
                    let wide = area.width >= 80 && area.height >= 20;
                    let tiny = area.width < 48 || area.height < 14;
                    let header = if preview { "Pair setup · preview" } else { "Pair setup" };
                    let footer = if review.is_some() {
                        if preview {
                            if tiny { "Preview done · Esc edit\nTab/S-Tab · type edits\nPgUp/Dn details · ^C exit" }
                            else { "Preview complete · Esc edit · Ctrl-C exit\nTab/Shift-Tab fields · type edits\nPgUp/PgDn review" }
                        } else if tiny {
                            "Enter apply · Esc edit\nTab/S-Tab · type edits\nPgUp/Dn details · ^C cancel"
                        } else {
                            "Enter apply · Esc edit · Ctrl-C cancel\nTab/Shift-Tab fields · type edits\nPgUp/PgDn review"
                        }
                    } else if success {
                        if preview {
                            if tiny { "Preview done · Esc exit\nTab/S-Tab · type edits\n^S review · PgUp/Dn details" }
                            else { "Preview complete · Esc exit · Ctrl-C exit\nTab/Shift-Tab fields · type to edit\nCtrl-S review · PgUp/PgDn details" }
                        } else if tiny {
                            "Applied · Esc exit\nTab/S-Tab · type edits\nPgUp/Dn details · ^N transit"
                        } else {
                            "Applied · Esc exit · Ctrl-C exit\nTab/Shift-Tab fields · type to edit\nPgUp/PgDn details · Ctrl-N transit"
                        }
                    } else if tiny {
                        "^S review · Esc cancel\nTab/S-Tab · ↑↓/Enter choose\n^N transit · PgUp/Dn details"
                    } else {
                        "Ctrl-S review · Esc cancel · Ctrl-C cancel\nTab/Shift-Tab fields · type search/edit\n↑↓ options · Enter choose · Ctrl-N transit · PgUp/PgDn details"
                    };
                    let header_lines = wrap_status(header, area.width as usize);
                    let footer_lines = wrap_status(footer, area.width as usize);
                    let header_height = (header_lines.len() as u16).min(area.height.saturating_sub(1));
                    let footer_height = (footer_lines.len() as u16)
                        .min(area.height.saturating_sub(header_height + 1));
                    let status_text = status.as_ref().map(|(text, _)| text.as_str())
                        .or_else(|| vm_ineligible.then_some(vm_warning));
                    let status_lines = status_text.map(|text| wrap_status(text, area.width as usize));
                    let status_height = status_lines.as_ref().map_or(0, |lines| {
                        (lines.len() as u16).min(3)
                            .min(area.height.saturating_sub(header_height.saturating_add(footer_height).saturating_add(5)))
                    });
                    let rows = Layout::default().direction(Direction::Vertical).constraints([
                        Constraint::Length(header_height),
                        Constraint::Min(0),
                        Constraint::Length(status_height),
                        Constraint::Length(footer_height),
                    ]).split(area);
                    let title_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
                    let focus_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
                    frame.render_widget(Paragraph::new(header_lines).style(title_style), rows[0]);
                    if let Some(review) = &review {
                        let review_lines = wrap_status(
                            &format!("Review Pair changes\n{review}"), rows[1].width as usize,
                        );
                        scroll_step = rows[1].height.max(1);
                        scroll = scroll.min(review_lines.len().saturating_sub(rows[1].height as usize)
                            .min(u16::MAX as usize) as u16);
                        frame.render_widget(Paragraph::new(review_lines).scroll((scroll, 0)), rows[1]);
                    } else {
                        let form_height = if wide { rows[1].height } else {
                            // Leave Search/edit, a candidate, and scrollable provenance visible.
                            rows[1].height.saturating_sub(4).min(5)
                                .max(u16::from(rows[1].height >= 3))
                        };
                        let panes = if wide {
                            Layout::default().direction(Direction::Horizontal).constraints([
                                Constraint::Length(((u32::from(area.width) * 42 / 100) as u16).clamp(32, 44)),
                                Constraint::Length(1),
                                Constraint::Min(0),
                            ]).split(rows[1])
                        } else {
                            Layout::default().direction(Direction::Vertical).constraints([
                                Constraint::Length(form_height),
                                Constraint::Length(0),
                                Constraint::Min(0),
                            ]).split(rows[1])
                        };
                        let form = panes[0];
                        let editor = panes[2];
                        if wide {
                            for (label, y, active) in [
                                ("Gateway", 0, focus == 0),
                                ("VM", 3, focus == 1),
                                ("Transit", 6, focus >= 2),
                            ] {
                                frame.render_widget(
                                    Paragraph::new(label).style(if active { focus_style } else { title_style }),
                                    Rect::new(form.x, form.y + y, form.width, 1),
                                );
                            }
                        }
                        let field_count = if wide { 4 } else { form.height.min(4) };
                        let first_field = if wide { 0 } else if focus >= 2 && field_count >= 2 {
                            4usize.saturating_sub(field_count as usize)
                        } else {
                            focus.saturating_sub(field_count.saturating_sub(1) as usize)
                                .min(4usize.saturating_sub(field_count as usize))
                        };
                        for offset in 0..field_count {
                            let index = first_field + offset as usize;
                            let y = if wide { [1, 4, 7, 8][index] } else { offset };
                            let value = match index {
                                0 => gateway.as_ref().map_or("not selected", |row| row.alias),
                                1 => vm.as_ref().map_or("not selected", |row| row.alias),
                                2 => if draft.transit_host.is_empty() { "(infer)" } else { &draft.transit_host },
                                _ => if draft.transit_port.is_empty() { "(infer)" } else { &draft.transit_port },
                            };
                            let label = format!("{}{}: ", if index == focus { "> " } else { "  " },
                                ["Gateway", "VM", "Transit host", "Transit port"][index]);
                            let label_width = (Span::raw(&label).width() as u16).min(form.width);
                            let value_width = form.width.saturating_sub(label_width);
                            let style = if index == focus { focus_style } else { Style::default() };
                            let horizontal_scroll = if index == focus && index >= 2 {
                                Span::raw(value).width().saturating_sub(value_width as usize)
                                    .min(u16::MAX as usize) as u16
                            } else { 0 };
                            frame.render_widget(Paragraph::new(label).style(style),
                                Rect::new(form.x, form.y + y, label_width, 1));
                            frame.render_widget(Paragraph::new(value).style(style).scroll((0, horizontal_scroll)),
                                Rect::new(form.x + label_width, form.y + y, value_width, 1));
                        }
                        if wide || form.height > 4 {
                            frame.render_widget(Paragraph::new(transit_status.as_str()),
                                Rect::new(form.x, form.y + if wide { 9 } else { 4 }, form.width, 1));
                        }
                        let heading_height = u16::from(editor.height >= 6);
                        let input_height = u16::from(editor.height > heading_height);
                        let remaining = editor.height.saturating_sub(heading_height + input_height);
                        let has_choices = if focus < 2 { !options.is_empty() } else {
                            candidates.as_ref().is_ok_and(|values| !values.is_empty())
                        };
                        let option_height = if has_choices {
                            (remaining / 2).min(8)
                        } else { 0 };
                        let editor_rows = Layout::default().direction(Direction::Vertical).constraints([
                            Constraint::Length(heading_height),
                            Constraint::Length(input_height),
                            Constraint::Length(option_height),
                            Constraint::Min(0),
                        ]).split(editor);
                        let heading = ["Gateway aliases", "VM aliases", "Transit choices", "Transit choices"][focus];
                        frame.render_widget(Paragraph::new(heading).style(title_style), editor_rows[0]);
                        let value = match focus {
                            0 | 1 => queries[focus].as_str(),
                            2 => draft.transit_host.as_str(),
                            _ => draft.transit_port.as_str(),
                        };
                        let input_width = editor_rows[1].width as usize;
                        let input_label = if focus < 2 { "Search: " } else if focus == 2 { "Transit host: " } else { "Transit port: " };
                        let label_width = (input_label.len() as u16).min(editor_rows[1].width);
                        let value_scroll = Span::raw(value).width()
                            .saturating_sub(input_width.saturating_sub(label_width as usize))
                            .min(u16::MAX as usize) as u16;
                        frame.render_widget(Paragraph::new(input_label).style(focus_style),
                            Rect::new(editor_rows[1].x, editor_rows[1].y, label_width, editor_rows[1].height));
                        frame.render_widget(Paragraph::new(value).style(focus_style).scroll((0, value_scroll)),
                            Rect::new(editor_rows[1].x + label_width, editor_rows[1].y,
                                editor_rows[1].width.saturating_sub(label_width), editor_rows[1].height));
                        if option_height > 0 {
                            let items = if focus < 2 {
                                options.iter().map(|pick| {
                                    let row = pair_setup_entry(entries, Some(*pick)).unwrap();
                                    if wide {
                                        ListItem::new(format!("{} · {}:{} · ID {}", row.alias,
                                            row.entry.source.path, row.entry.source.line_start, row.entry.id))
                                    } else { ListItem::new(row.alias) }
                                }).collect::<Vec<_>>()
                            } else {
                                candidates.as_ref().unwrap().iter()
                                    .map(|(host, port)| ListItem::new(format!("{host}:{port}")))
                                    .collect::<Vec<_>>()
                            };
                            list_state.select(if focus < 2 { Some(selected[focus]) } else { candidate_index });
                            frame.render_stateful_widget(
                                List::new(items).highlight_symbol("> ")
                                    .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                                editor_rows[2], &mut list_state,
                            );
                        }
                        let detail_lines = wrap_status(&details, editor_rows[3].width as usize);
                        scroll_step = editor_rows[3].height.max(1);
                        scroll = scroll.min(detail_lines.len().saturating_sub(editor_rows[3].height as usize)
                            .min(u16::MAX as usize) as u16);
                        frame.render_widget(Paragraph::new(detail_lines).scroll((scroll, 0)), editor_rows[3]);
                    }
                    if let Some(lines) = status_lines {
                        frame.render_widget(
                            Paragraph::new(lines).style(Style::default().fg(if success { Color::Green } else { Color::Red })),
                            rows[2],
                        );
                    }
                    frame.render_widget(Paragraph::new(footer_lines).style(Style::default().fg(Color::Cyan)), rows[3]);
                }).map_err(|error| format!("PAIR_REQUIRED: cannot render Pair setup: {error}"))?;
                let mut edited = false;
                match read_key(input, "PAIR_REQUIRED")? {
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } if review.is_some() => {
                        review = None;
                        scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => return Ok(completed),
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => return Ok(completed),
                    KeyEvent {
                        code: KeyCode::Char('s'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) && !applied => {
                        review = None;
                        status = None;
                        scroll = 0;
                        if pair_setup_entry(entries, draft.gateway).is_none()
                            || !draft
                                .gateway
                                .is_some_and(|(index, _)| eligible.contains(&index))
                        {
                            status = Some((
                                "PAIR_SELECTION: select an eligible gateway HostEntry".to_string(),
                                false,
                            ));
                            focus = 0;
                            continue;
                        }
                        if pair_setup_entry(entries, draft.vm).is_none()
                            || !draft
                                .vm
                                .is_some_and(|(index, _)| vm_eligible.contains(&index))
                        {
                            status = Some((
                                "PAIR_SELECTION: select a different eligible VM HostEntry"
                                    .to_string(),
                                false,
                            ));
                            focus = 1;
                            continue;
                        }
                        match execute(&draft, PairSetupAction::Review) {
                            Ok(text) => {
                                review = Some(text);
                                if preview {
                                    completed = true;
                                    status = Some((
                                        "Preview complete — no files changed".to_string(),
                                        true,
                                    ));
                                }
                            }
                            Err(error) => status = Some((error, false)),
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if review.is_some() && !preview => {
                        review = None;
                        scroll = 0;
                        match execute(&draft, PairSetupAction::Apply) {
                            Ok(text) => {
                                completed = true;
                                status = Some((text, true));
                            }
                            Err(error) => status = Some((error, false)),
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if review.is_some() => {}
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } => {
                        focus = (focus + 1) % 4;
                        if review.is_none() {
                            scroll = 0;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::BackTab,
                        ..
                    } => {
                        focus = (focus + 3) % 4;
                        if review.is_none() {
                            scroll = 0;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } if focus < 2 && review.is_none() => {
                        selected[focus] = selected[focus].saturating_sub(1);
                        scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } if focus < 2 && review.is_none() => {
                        selected[focus] =
                            (selected[focus] + 1).min(options.len().saturating_sub(1));
                        scroll = 0;
                    }
                    KeyEvent {
                        code: KeyCode::PageUp | KeyCode::Up,
                        ..
                    } => scroll = scroll.saturating_sub(scroll_step),
                    KeyEvent {
                        code: KeyCode::PageDown | KeyCode::Down,
                        ..
                    } => scroll = scroll.saturating_add(scroll_step),
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } if focus < 2 => {
                        if let Some(pick) = options.get(selected[focus]) {
                            if focus == 0 {
                                draft.gateway = Some(*pick);
                                routes = pair_setup_routes(entries, &eligible, draft.gateway);
                                vm_eligible =
                                    pair_setup_vm_eligible(&eligible, draft.gateway, &routes);
                                let vm_options =
                                    pair_setup_options(entries, &vm_eligible, &queries[1]);
                                selected[1] = vm_options
                                    .iter()
                                    .position(|pick| Some(*pick) == draft.vm)
                                    .unwrap_or(0);
                            } else {
                                draft.vm = Some(*pick);
                            }
                            focus += 1;
                            edited = true;
                            candidate_index = None;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char('n'),
                        modifiers,
                        ..
                    } if modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Ok(values) = &candidates
                            && !values.is_empty()
                        {
                            let index =
                                candidate_index.map_or(0, |index| (index + 1) % values.len());
                            draft.transit_host.clone_from(&values[index].0);
                            draft.transit_port = values[index].1.to_string();
                            candidate_index = Some(index);
                            edited = true;
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        match focus {
                            0 | 1 => {
                                queries[focus].pop();
                                selected[focus] = 0;
                            }
                            2 => {
                                draft.transit_host.pop();
                                candidate_index = None;
                            }
                            _ => {
                                draft.transit_port.pop();
                                candidate_index = None;
                            }
                        }
                        edited = true;
                    }
                    KeyEvent {
                        code: KeyCode::Char(character),
                        modifiers,
                        ..
                    } if !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        match focus {
                            0 | 1 => {
                                queries[focus].push(character);
                                selected[focus] = 0;
                            }
                            2 => {
                                draft.transit_host.push(character);
                                candidate_index = None;
                            }
                            _ => {
                                draft.transit_port.push(character);
                                candidate_index = None;
                            }
                        }
                        edited = true;
                    }
                    _ => {}
                }
                if edited {
                    review = None;
                    status = None;
                    scroll = 0;
                }
            }
        },
    )
}
