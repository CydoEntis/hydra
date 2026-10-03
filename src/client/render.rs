//! Drawing: sidebar, tab bar, panes, status bar and overlays.

use super::copy::Copy;
use super::modal;
use super::{App, Hit, Mode, PickTarget};
use crate::protocol::{Status, TabInfo, TermId, TermInfo};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn draw(app: &mut App, f: &mut Frame) {
    app.hits.clear();
    app.panes.clear();
    app.pane_frames.clear();
    let area = f.area();
    let t = app.theme.clone();
    app.hy_fresh();
    set_palette(t.ansi, t.ansi.map(|_| t.card2));
    // The terminal's own background (its window padding) matches ours while we run.
    if let Color::Rgb(r, g, b) = t.bg
        && app.osc_bg != Some(t.bg)
        && !cfg!(test)
    {
        use std::io::Write;
        let _ = write!(std::io::stdout(), "]11;#{r:02x}{g:02x}{b:02x}");
        app.osc_bg = Some(t.bg);
    }

    if app.splash {
        super::hydra::draw_splash(app, f, area, &t);
        return;
    }
    let panes = match app.cfg.ui.layout.as_str() {
        "sidebar" => draw_sidebar_layout(app, f, area, &t),
        "dock" => draw_dock_layout(app, f, area, &t),
        "tree" => draw_tree_layout(app, f, area, &t),
        "workspaces" => super::design::draw(app, f, area, &t),
        _ => super::hydra::draw(app, f, area, &t),
    };
    draw_overlays(app, f, area, panes, &t);
}

/// The classic layout: sidebar, tab bar, panes, status bar.
fn draw_sidebar_layout(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) -> Rect {
    let t = t.clone();
    let sidebar_w = if app.sidebar { app.cfg.ui.sidebar_width.min(area.width / 2) } else { 0 };
    let right = app.cfg.ui.sidebar_position == "right";
    let [side, main] = if right {
        let [m, s] = Layout::horizontal([Constraint::Min(10), Constraint::Length(sidebar_w)]).areas(area);
        [s, m]
    } else {
        Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(10)]).areas(area)
    };
    let tab_h = app.cfg.ui.tab_bar as u16;
    let status_h = app.cfg.ui.status_bar as u16;
    let [tabs, panes, status] =
        Layout::vertical([Constraint::Length(tab_h), Constraint::Min(1), Constraint::Length(status_h)]).areas(main);

    if sidebar_w > 0 {
        draw_sidebar(app, f, side, &t);
    }
    if tab_h > 0 {
        draw_tabs(app, f, tabs, &t);
    }
    draw_panes(app, f, panes, &t);
    if status_h > 0 {
        draw_status(app, f, status, &t);
    }
    panes
}

/// The dock layout: workspaces and tabs across the top, panes, agent cards along the bottom.
fn draw_dock_layout(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) -> Rect {
    let mut agents: Vec<TermInfo> = app.snap.terms.values().filter(|x| x.agent.is_some()).cloned().collect();
    agents.sort_by_key(|a| (a.status.urgency(), a.id));
    let dock_h = if app.dock && !agents.is_empty() && area.height > 12 { 4 } else { 0 };
    let [top, panes, dock] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(dock_h)]).areas(area);
    draw_topbar(app, f, top, t, true);
    draw_panes(app, f, panes, t);
    if dock_h > 0 {
        draw_dock(app, f, dock, t, &agents);
    }
    panes
}

/// The tree layout: sidebar tree on the left; tabs, panes and the agent dock on the right.
fn draw_tree_layout(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) -> Rect {
    let side_w = if app.sidebar { app.cfg.ui.sidebar_width.max(24).min(area.width / 2) } else { 0 };
    let right = app.cfg.ui.sidebar_position == "right";
    let [side, main] = if right {
        let [m, s] = Layout::horizontal([Constraint::Min(10), Constraint::Length(side_w)]).areas(area);
        [s, m]
    } else {
        Layout::horizontal([Constraint::Length(side_w), Constraint::Min(10)]).areas(area)
    };
    let mut agents: Vec<TermInfo> = app.snap.terms.values().filter(|x| x.agent.is_some()).cloned().collect();
    agents.sort_by_key(|a| (a.status.urgency(), a.id));
    let dock_h = if app.dock && !agents.is_empty() && main.height > 12 { 4 } else { 0 };
    let [top, panes, dock] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(dock_h)]).areas(main);
    if side_w > 0 {
        draw_tree(app, f, side, t);
    }
    draw_topbar(app, f, top, t, side_w == 0);
    draw_panes(app, f, panes, t);
    if dock_h > 0 {
        draw_dock(app, f, dock, t, &agents);
    }
    panes
}

fn draw_tree(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    let base = Style::default().bg(t.sidebar_bg).fg(t.fg);
    f.render_widget(Block::default().style(base), area);
    let rows = app.tree_rows();
    let selected = match app.mode {
        Mode::Tree { sel } => Some(sel),
        _ => None,
    };
    let active = app.snap.active_ws;
    let focused = app.focused();
    let w = area.width.saturating_sub(1) as usize;
    let list_h = area.height.saturating_sub(3) as usize;
    // Keep the selection (or the active workspace) in view.
    let anchor = selected.or_else(|| rows.iter().position(|r| matches!(r, super::TreeRow::Workspace { ws, .. } if Some(*ws) == active))).unwrap_or(0);
    let start = anchor.saturating_sub(list_h.saturating_sub(1));

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" hydra", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(
                match app.snap.workspaces.len() {
                    1 => "  1 workspace".to_string(),
                    n => format!("  {n} workspaces"),
                },
                Style::default().fg(t.muted),
            ),
        ]))
        .style(base),
        Rect { height: 1, ..area },
    );

    for (i, row) in rows.iter().enumerate().skip(start).take(list_h) {
        let y = area.y + 2 + (i - start) as u16;
        let is_sel = selected == Some(i);
        let mut bg = t.sidebar_bg;
        let indent = |d: &u8| "  ".repeat(*d as usize);
        let spans: Vec<Span<'static>> = match row {
            super::TreeRow::Repo { name } => vec![Span::styled(
                format!(" {}{name}", app.cfg.icons.branch),
                Style::default().fg(t.muted).add_modifier(Modifier::BOLD),
            )],
            super::TreeRow::Workspace { ws, depth, label } => {
                let Some(info) = app.snap.workspace(*ws) else { continue };
                let c = app.ws_color(info);
                let is_active = Some(*ws) == active;
                if is_active {
                    bg = blend(t.sidebar_bg, c, 0.22);
                }
                let mut v = vec![
                    Span::raw(format!(" {}", indent(depth))),
                    Span::styled(app.cfg.icons.active_workspace.clone(), Style::default().fg(c)),
                    Span::styled(
                        format!(" {}", truncate(label, w.saturating_sub(*depth as usize * 2 + 10))),
                        Style::default().fg(if is_active { t.fg } else { t.muted }).add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                ];
                if let Some(g) = &info.git {
                    if *depth == 0 && !label.ends_with(&g.branch) {
                        v.push(Span::styled(format!(" {}", truncate(&g.branch, 14)), Style::default().fg(t.muted)));
                    }
                    if g.dirty > 0 {
                        v.push(Span::styled(format!(" ±{}", g.dirty), Style::default().fg(t.working)));
                    }
                }
                v
            }
            super::TreeRow::Agent { term, depth } => {
                let Some(a) = app.snap.terms.get(term) else { continue };
                if Some(*term) == focused {
                    bg = blend(t.sidebar_bg, t.fg, 0.08);
                }
                let icon = status_icon(app, a.status);
                let name = a.display_name().to_string();
                let lead = format!(" {}{icon} {name} ", indent(depth));
                let ws_cwd = app.snap.locate(*term).map(|(w, _)| w.cwd.clone());
                let tail = if a.status == Status::Blocked {
                    "needs you".to_string()
                } else if a.agent.is_some() && !a.summary.is_empty() {
                    a.summary.clone()
                } else if !a.is_shell() && ws_cwd.as_ref() != Some(&a.cwd) {
                    // Where the pane has wandered to.
                    format!("› {}", short_dir(&a.cwd))
                } else {
                    String::new()
                };
                let tail = truncate(&tail, w.saturating_sub(lead.width()));
                vec![
                    Span::raw(format!(" {}", indent(depth))),
                    Span::styled(format!("{icon} "), Style::default().fg(t.status(a.status))),
                    Span::styled(format!("{name} "), Style::default().fg(t.fg)),
                    Span::styled(
                        tail,
                        Style::default().fg(if a.status == Status::Blocked { t.blocked } else { t.muted }),
                    ),
                ]
            }
            super::TreeRow::Closed { entry, depth } => vec![
                Span::raw(format!(" {}", indent(depth))),
                Span::styled(format!("○ {}", truncate(&entry.branch, w.saturating_sub(6))), Style::default().fg(t.muted)),
            ],
            super::TreeRow::NewWorkspace => vec![Span::styled(
                format!(" + new workspace  {} N", app.keymap.prefix),
                Style::default().fg(blend(t.muted, t.idle, 0.4)),
            )],
            super::TreeRow::NewWorktree { depth, .. } => vec![
                Span::raw(format!(" {}", indent(depth))),
                Span::styled("+ new worktree", Style::default().fg(blend(t.muted, t.idle, 0.4))),
            ],
        };
        if is_sel {
            bg = focus_bg(t);
        }
        let r = Rect { y, height: 1, ..area };
        let line = if is_sel { focus_row(Line::from(spans), t) } else { Line::from(spans) };
        f.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), r);
        app.hits.push((r, Hit::TreeRow(i)));
    }

    if area.height > 4 {
        let p = app.keymap.prefix.to_string();
        let hint = if selected.is_some() {
            Line::from(Span::styled(" ↑↓ move · Enter open · Esc back", Style::default().fg(t.muted)))
        } else {
            Line::from(vec![
                Span::styled(format!(" {p} e"), Style::default().fg(t.accent)),
                Span::styled(" browse ", Style::default().fg(t.muted)),
                Span::styled("Space", Style::default().fg(t.accent)),
                Span::styled(" all ", Style::default().fg(t.muted)),
                Span::styled("?", Style::default().fg(t.accent)),
                Span::styled(" help", Style::default().fg(t.muted)),
            ])
        };
        f.render_widget(Paragraph::new(hint).style(base), Rect { y: area.bottom() - 1, height: 1, ..area });
    }
}

fn draw_topbar(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, show_workspaces: bool) {
    f.render_widget(Block::default().style(Style::default().bg(t.sidebar_bg)), area);
    let mut x = area.x;
    let put = |f: &mut Frame, x: &mut u16, spans: Vec<Span<'static>>| -> Rect {
        let w: u16 = spans.iter().map(|s| s.content.width() as u16).sum();
        let w = w.min(area.right().saturating_sub(*x));
        let r = Rect { x: *x, width: w, ..area };
        f.render_widget(Paragraph::new(Line::from(spans)), r);
        *x += w;
        r
    };

    let (mode, mode_color) = mode_label(app, t);
    if mode != "NORMAL" {
        put(f, &mut x, vec![Span::styled(format!(" {mode} "), Style::default().bg(mode_color).fg(t.bg).add_modifier(Modifier::BOLD))]);
        put(f, &mut x, vec![Span::raw(" ")]);
    }

    // Workspaces: the active one filled with its colour, the rest in their colour. With the
    // tree sidebar showing, only the active one is named here.
    let active = app.snap.active_ws;
    let workspaces: Vec<_> = if show_workspaces {
        app.snap.workspaces.clone()
    } else {
        app.active_ws().cloned().into_iter().collect()
    };
    for ws in &workspaces {
        let c = app.ws_color(ws);
        let is_active = Some(ws.id) == active;
        let terms: Vec<Status> = ws
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.leaves())
            .filter_map(|id| app.snap.terms.get(&id).map(|ti| ti.status))
            .collect();
        let (bg, fg) = if is_active { (c, t.bg) } else { (t.sidebar_bg, c) };
        let mut spans = vec![Span::styled(
            format!(" {} ", truncate(&ws.name, 22)),
            Style::default().bg(bg).fg(fg).add_modifier(if is_active { Modifier::BOLD } else { Modifier::empty() }),
        )];
        for s in [Status::Blocked, Status::Done, Status::Working] {
            let n = terms.iter().filter(|x| **x == s).count();
            if n > 0 {
                let color = if is_active { t.bg } else { t.status(s) };
                spans.push(Span::styled(format!("{}{n} ", status_icon(app, s)), Style::default().bg(bg).fg(color)));
            }
        }
        if x >= area.right() {
            break;
        }
        let r = put(f, &mut x, spans);
        app.hits.push((r, Hit::Workspace(ws.id)));
        put(f, &mut x, vec![Span::raw(" ")]);
    }

    // Right side: a message if there is one, else the branch.
    let right: Vec<Span<'static>> = if let Some((msg, _, err)) = &app.notice {
        vec![Span::styled(format!("{} ", truncate(msg, 60)), Style::default().fg(if *err { t.blocked } else { t.fg }))]
    } else if let Some(g) = app.active_ws().and_then(|w| w.git.clone()) {
        let mut v = vec![Span::styled(format!("{}{}", app.cfg.icons.branch, g.branch), Style::default().fg(t.muted))];
        if g.dirty > 0 {
            v.push(Span::styled(format!(" ±{}", g.dirty), Style::default().fg(t.working)));
        }
        v.push(Span::raw(" "));
        v
    } else {
        Vec::new()
    };
    let right_w: u16 = right.iter().map(|s| s.content.width() as u16).sum();
    let right_x = area.right().saturating_sub(right_w);

    // Tabs of the active workspace, between the two.
    if let Some(ws) = app.active_ws().cloned() {
        let c = app.ws_color(&ws);
        put(f, &mut x, vec![Span::styled("│ ", Style::default().fg(t.border))]);
        for (i, tab) in ws.tabs.iter().enumerate() {
            let is_active = tab.id == ws.active_tab;
            let status = tab_status(app, tab);
            let icon = if status == Status::None { String::new() } else { format!("{} ", status_icon(app, status)) };
            let label = format!("{} {}", i + 1, truncate(&tab_label(app, tab), 18));
            let style = if is_active {
                Style::default().fg(c).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(t.muted)
            };
            let w = (icon.width() + label.width() + 2) as u16;
            if x + w > right_x {
                break;
            }
            let r = put(f, &mut x, vec![
                Span::styled(icon, Style::default().fg(t.status(status))),
                Span::styled(label, style),
                Span::raw("  "),
            ]);
            app.hits.push((r, Hit::Tab(ws.id, tab.id)));
        }
        if x + 2 <= right_x {
            let r = put(f, &mut x, vec![Span::styled("+ ", Style::default().fg(t.muted))]);
            app.hits.push((r, Hit::NewTab));
        }
    }
    if right_x > x {
        f.render_widget(Paragraph::new(Line::from(right)), Rect { x: right_x, width: right_w, ..area });
    }
}

/// One card per agent, most urgent first. Click a card to jump to its pane.
fn draw_dock(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, agents: &[TermInfo]) {
    f.render_widget(Block::default().style(Style::default().bg(t.bg)), area);
    let card_w = app.cfg.ui.card_width.clamp(18, 60).min(area.width);
    let fit = (area.width / card_w).max(1) as usize;
    let overflow = agents.len() > fit;
    let shown = if overflow { fit - 1 } else { agents.len() };
    let focused = app.focused();
    let bt = border_type(&app.cfg.ui.border_style);
    for (i, a) in agents.iter().take(shown).enumerate() {
        let r = Rect { x: area.x + i as u16 * card_w, width: card_w, ..area };
        let ws = app.snap.locate(a.id).map(|(w, _)| w.clone());
        let wc = ws.as_ref().map(|w| app.ws_color(w)).unwrap_or(t.accent);
        let is_focused = Some(a.id) == focused;
        let border = match a.status {
            Status::Blocked => t.blocked,
            Status::Done => t.done,
            _ if is_focused => wc,
            _ => blend(t.border, wc, 0.45),
        };
        let bg = if is_focused { blend(t.bg, wc, 0.16) } else { t.bg };
        let icon = status_icon(app, a.status);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(bt)
            .border_style(Style::default().fg(border))
            .style(Style::default().bg(bg))
            .padding(ratatui::widgets::Padding::horizontal(1))
            .title(Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(t.status(a.status))),
                Span::styled(format!("{} ", truncate(&a.display_name(), card_w.saturating_sub(8) as usize)), Style::default().fg(t.fg).add_modifier(Modifier::BOLD)),
            ]));
        let inner = block.inner(r);
        f.render_widget(block, r);
        let label = match a.status {
            Status::Blocked => "needs you".to_string(),
            s => s.label().to_string(),
        };
        let ws_name = ws.map(|w| w.name).unwrap_or_default();
        let room = (inner.width as usize).saturating_sub(label.width() + 1);
        let line1 = Line::from(vec![
            Span::styled(truncate(&ws_name, room), Style::default().fg(wc)),
            Span::raw(" ".repeat(room.saturating_sub(truncate(&ws_name, room).width()) + 1)),
            Span::styled(label, Style::default().fg(t.status(a.status)).add_modifier(if a.status == Status::Blocked { Modifier::BOLD } else { Modifier::empty() })),
        ]);
        let doing = if !a.summary.is_empty() {
            a.summary.clone()
        } else if !a.title.is_empty() && a.agent.as_deref() != Some(a.title.as_str()) {
            a.title.clone()
        } else {
            short_dir(&a.cwd)
        };
        let line2 = Line::from(Span::styled(truncate(&doing, inner.width as usize), Style::default().fg(t.muted)));
        f.render_widget(Paragraph::new(vec![line1, line2]), inner);
        app.hits.push((r, Hit::Pane(a.id)));
    }
    if overflow {
        let r = Rect { x: area.x + shown as u16 * card_w, width: card_w, ..area };
        let rest = &agents[shown..];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(bt)
            .border_style(Style::default().fg(t.border))
            .title(Span::styled(format!(" +{} more ", rest.len()), Style::default().fg(t.fg)));
        let inner = block.inner(r);
        f.render_widget(block, r);
        let mut spans = Vec::new();
        for s in [Status::Blocked, Status::Done, Status::Working, Status::Idle] {
            let n = rest.iter().filter(|a| a.status == s).count();
            if n > 0 {
                spans.push(Span::styled(format!("{}{n} ", status_icon(app, s)), Style::default().fg(t.status(s))));
            }
        }
        f.render_widget(Paragraph::new(vec![Line::from(spans), Line::from(Span::styled(format!("{} w to jump", app.keymap.prefix), Style::default().fg(t.muted)))]), inner);
    }
}

fn draw_overlays(app: &mut App, f: &mut Frame, area: Rect, panes: Rect, t: &crate::theme::Theme) {
    let t = t.clone();
    // Copy mode holds the whole history; match by reference and clone only small fields.
    let hydra = !matches!(app.cfg.ui.layout.as_str(), "sidebar" | "dock" | "tree" | "workspaces");
    match &app.mode {
        Mode::Prefix { since } if hydra => {
            if app.cfg.ui.which_key && since.elapsed() >= Duration::from_millis(app.cfg.ui.which_key_delay_ms) {
                super::hydra::draw_keys(app, f, area, &t);
            }
        }
        Mode::Help { .. } if hydra => super::hydra::draw_keys(app, f, area, &t),
        Mode::Talk { term, input } if hydra => {
            let (term, input) = (*term, input.clone());
            super::hydra::draw_talk(app, f, area, &t, term, &input);
        }
        Mode::HyMenu(m) => {
            let m = (**m).clone();
            super::menu::draw_menu(app, f, area, &t, &m);
        }
        Mode::Ideas(v) => {
            let v = (**v).clone();
            super::work::draw_ideas(app, f, area, &t, &v);
        }
        Mode::Tickets(v) => {
            let v = (**v).clone();
            super::work::draw_tickets(app, f, area, &t, &v);
        }
        Mode::RaceNew(v) => {
            let v = (**v).clone();
            super::work::draw_race_new(app, f, area, &t, &v);
        }
        Mode::Race(v) => {
            let v = (**v).clone();
            super::work::draw_race(app, f, area, &t, &v);
        }
        Mode::Ship(ask) => {
            let ask = (**ask).clone();
            super::hydra::draw_ship(app, f, area, &t, &ask);
        }
        Mode::Jump { sel } => {
            let sel = *sel;
            super::hydra::draw_jump(app, f, area, &t, sel);
        }
        Mode::Finder(fd) => {
            let fd = (**fd).clone();
            super::hydra::draw_finder(app, f, area, &t, &fd);
        }
        Mode::HyPane(np) => {
            let np = np.clone();
            super::hydra::draw_new_pane(app, f, area, &t, &np);
        }
        Mode::HySettings(v) => {
            let v = (**v).clone();
            super::hydra::draw_settings(app, f, area, &t, &v);
        }
        Mode::Prefix { since } if app.cfg.ui.which_key => {
            if since.elapsed() >= Duration::from_millis(app.cfg.ui.which_key_delay_ms) {
                if app.cfg.ui.layout == "workspaces" {
                    super::design::draw_keys(app, f, area, &t, true);
                } else {
                    draw_which_key(app, f, panes, &t);
                }
            }
        }
        Mode::Picker { query, sel, commands } => {
            let (query, sel, commands) = (query.clone(), *sel, *commands);
            draw_picker(app, f, area, &t, &query, sel, commands)
        }
        Mode::Menu(m) => {
            let m = m.clone();
            draw_menu(app, f, area, &t, &m)
        }
        Mode::Quick(q) => {
            let q = q.clone();
            draw_quick(app, f, area, &t, &q)
        }
        Mode::Settings(s) => {
            let s = s.clone();
            draw_settings(app, f, area, &t, &s)
        }
        Mode::Files(_) => {
            let v = files_panel(app, &t);
            draw_panel(f, area, &t, v);
        }
        Mode::Tasks { .. } => {
            let v = tasks_panel(app, &t);
            draw_panel(f, area, &t, v);
        }
        Mode::Inbox(_) => {
            let v = inbox_panel(app, &t);
            draw_panel(f, area, &t, v);
        }
        Mode::Toolbox(_) => {
            let v = toolbox_panel(app, &t);
            draw_panel(f, area, &t, v);
        }
        Mode::Prompt { kind, input } => draw_prompt(f, area, &t, kind.label(), input, kind.is_confirm()),
        Mode::Help { .. } if app.cfg.ui.layout == "workspaces" => super::design::draw_keys(app, f, area, &t, false),
        Mode::Help { scroll } => draw_help(app, f, area, &t, *scroll),
        Mode::Talk { term, input } => {
            let (term, input) = (*term, input.clone());
            super::design::draw_talk(app, f, area, &t, term, &input);
        }
        Mode::NewPane(np) => {
            let np = np.clone();
            super::design::draw_new_pane(app, f, area, &t, &np);
        }
        Mode::Worktrees { items, query, sel, .. } => {
            let rows = app.worktree_rows(items.as_deref(), query);
            let (loading, query, sel) = (items.is_none(), query.clone(), *sel);
            draw_worktrees(app, f, area, &t, &rows, &query, sel, loading);
        }
        _ => {}
    }
}

/// Mix `b` into `a` by `t` (0..1). Non-RGB colours can't be mixed and fall back.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black | Color::Reset => Some((12, 12, 16)),
        Color::White => Some((240, 240, 240)),
        _ => None,
    };
    match (rgb(a), rgb(b)) {
        (Some(x), Some(y)) => {
            let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t.clamp(0.0, 1.0)).round() as u8;
            Color::Rgb(m(x.0, y.0), m(x.1, y.1), m(x.2, y.2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// Background of the row the keyboard is on: clearly stronger than a hover or the
/// active-workspace tint.
pub(super) fn focus_bg(t: &crate::theme::Theme) -> Color {
    blend(t.selection_bg, t.accent, 0.30)
}

/// Mark a list row as the selected one: an accent bar at the left edge, the focus band
/// behind every cell, and bold text. The bar takes the place of the row's leading space so
/// nothing shifts sideways when the selection moves.
pub(super) fn focus_row(line: Line<'static>, t: &crate::theme::Theme) -> Line<'static> {
    let bg = focus_bg(t);
    let mut spans: Vec<Span<'static>> = line.spans;
    if let Some(first) = spans.first_mut()
        && first.content.starts_with(' ')
    {
        first.content = first.content[1..].to_string().into();
    }
    spans.insert(0, Span::styled("▌", Style::default().fg(t.accent)));
    Line::from(
        spans
            .into_iter()
            .map(|sp| {
                let style = sp.style.bg(bg).add_modifier(Modifier::BOLD);
                Span::styled(sp.content, style)
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn status_icon(app: &App, s: Status) -> String {
    let i = &app.cfg.icons;
    match s {
        Status::Working if !app.cfg.ui.spinner.is_empty() => {
            let n = app.cfg.ui.spinner.len() as u64;
            app.cfg.ui.spinner[(app.spinner_frame() % n) as usize].clone()
        }
        Status::Working => i.working.clone(),
        Status::Blocked => i.blocked.clone(),
        Status::Done => i.done.clone(),
        Status::Idle => i.idle.clone(),
        Status::None => i.shell.clone(),
    }
}

/// A folder's name for display; the whole path for a drive root like `C:\`.
pub(super) fn short_dir(p: &std::path::Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => p.display().to_string(),
    }
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + 1 >= max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn draw_sidebar(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    let base = Style::default().bg(t.sidebar_bg).fg(t.fg);
    f.render_widget(Block::default().style(base), area);
    let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    let w = inner.width as usize;
    let mut y = area.y;
    let line = |f: &mut Frame, y: &mut u16, l: Line| {
        if *y < area.bottom() {
            f.render_widget(Paragraph::new(l).style(base), Rect { y: *y, height: 1, ..inner });
        }
        *y += 1;
    };

    line(f, &mut y, Line::from(Span::styled(" hydra", Style::default().fg(t.accent).add_modifier(Modifier::BOLD))));
    y += 1;
    line(f, &mut y, Line::from(Span::styled("WORKSPACES", Style::default().fg(t.muted).add_modifier(Modifier::BOLD))));

    let active = app.snap.active_ws;
    let workspaces = app.snap.workspaces.clone();
    for (i, ws) in workspaces.iter().enumerate() {
        let is_active = Some(ws.id) == active;
        let terms: Vec<&TermInfo> =
            ws.tabs.iter().flat_map(|t| t.layout.leaves()).filter_map(|id| app.snap.terms.get(&id)).collect();
        let mut badges: Vec<Span> = Vec::new();
        for s in [Status::Blocked, Status::Done, Status::Working] {
            let n = terms.iter().filter(|x| x.status == s).count();
            if n > 0 {
                badges.push(Span::styled(format!(" {}{n}", status_icon(app, s)), Style::default().fg(t.status(s))));
            }
        }
        let badge_w: usize = badges.iter().map(|s| s.content.width()).sum();
        let wcolor = app.ws_color(ws);
        let marker = app.cfg.icons.active_workspace.as_str();
        let label = format!("{} {}", i + 1, ws.name);
        // Branch (unless the name already says it) and uncommitted-change count.
        let git = ws.git.as_ref().map(|g| {
            let branch = if ws.name.ends_with(&g.branch) { String::new() } else { format!(" {}{}", app.cfg.icons.branch, g.branch) };
            let dirty = if g.dirty > 0 { format!(" ±{}", g.dirty) } else { String::new() };
            (branch, dirty)
        });
        let (branch, dirty) = git.unwrap_or_default();
        let room = w.saturating_sub(badge_w + 2);
        // Drop the branch before squeezing the name.
        let (branch, dirty) = if label.width() + branch.width() + dirty.width() <= room {
            (branch, dirty)
        } else if label.width() + dirty.width() <= room {
            (String::new(), dirty)
        } else {
            (String::new(), String::new())
        };
        let name = truncate(&label, room.saturating_sub(branch.width() + dirty.width()));
        let pad = w.saturating_sub(name.width() + branch.width() + dirty.width() + badge_w + 2);
        let row_bg = if is_active { blend(t.sidebar_bg, wcolor, 0.22) } else { t.sidebar_bg };
        let style = if is_active {
            Style::default().fg(t.fg).bg(row_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        };
        let mut spans = vec![
            Span::styled(marker.to_string(), Style::default().fg(if is_active { wcolor } else { blend(t.sidebar_bg, wcolor, 0.55) })),
            Span::styled(format!(" {name}"), style),
            Span::styled(branch, Style::default().fg(t.muted).bg(row_bg)),
            Span::styled(dirty, Style::default().fg(t.working).bg(row_bg)),
        ];
        spans.push(Span::styled(" ".repeat(pad), style));
        spans.extend(badges.into_iter().map(|s| s.patch_style(Style::default().bg(row_bg))));
        if y < area.bottom() {
            app.hits.push((Rect { y, height: 1, ..area }, Hit::Workspace(ws.id)));
        }
        line(f, &mut y, Line::from(spans));
    }

    y += 1;
    let scoped = app.cfg.ui.agents_scope != "all";
    let here: Vec<TermId> = app
        .active_ws()
        .map(|w| w.tabs.iter().flat_map(|t| t.layout.leaves()).collect())
        .unwrap_or_default();
    let header = match (scoped, app.active_ws()) {
        (true, Some(w)) => Line::from(vec![
            Span::styled("AGENTS ", Style::default().fg(t.muted).add_modifier(Modifier::BOLD)),
            Span::styled(truncate(&w.name, 20), Style::default().fg(app.ws_color(w))),
        ]),
        _ => Line::from(Span::styled("AGENTS", Style::default().fg(t.muted).add_modifier(Modifier::BOLD))),
    };
    line(f, &mut y, header);
    let all: Vec<TermInfo> = app.snap.terms.values().filter(|x| x.agent.is_some()).cloned().collect();
    let (mut agents, elsewhere): (Vec<TermInfo>, Vec<TermInfo>) =
        all.into_iter().partition(|a| !scoped || here.contains(&a.id));
    agents.sort_by_key(|a| (a.status.urgency(), a.id));
    if agents.is_empty() {
        let msg = if scoped { "  none in this workspace" } else { "  none running" };
        line(f, &mut y, Line::from(Span::styled(msg, Style::default().fg(t.muted))));
    }
    let focused = app.focused();
    for a in agents {
        let loc = app
            .snap
            .locate(a.id)
            .map(|(w, tab)| {
                let idx = w.tabs.iter().position(|x| x.id == tab.id).unwrap_or(0) + 1;
                if scoped { format!("tab {idx}") } else { format!("{}›{idx}", w.name) }
            })
            .unwrap_or_default();
        let icon = status_icon(app, a.status);
        let name = a.display_name().to_string();
        let label = a.status.label();
        let left = format!(" {icon} {name}");
        let right = format!("{label} ");
        let loc = truncate(&loc, w.saturating_sub(left.width() + right.width() + 2));
        let pad = w.saturating_sub(left.width() + loc.width() + right.width() + 1);
        let sel = Some(a.id) == focused;
        let bg = if sel { t.selection_bg } else { t.sidebar_bg };
        let spans = vec![
            Span::styled(format!(" {icon}"), Style::default().fg(t.status(a.status)).bg(bg)),
            Span::styled(format!(" {name} "), Style::default().fg(t.fg).bg(bg).add_modifier(Modifier::BOLD)),
            Span::styled(loc, Style::default().fg(t.muted).bg(bg)),
            Span::styled(" ".repeat(pad), Style::default().bg(bg)),
            Span::styled(right, Style::default().fg(t.status(a.status)).bg(bg)),
        ];
        if y < area.bottom() {
            app.hits.push((Rect { y, height: 1, ..area }, Hit::Pane(a.id)));
        }
        line(f, &mut y, Line::from(spans));
    }

    // Agents in other workspaces: one line saying what they need, click to jump.
    if scoped && !elsewhere.is_empty() {
        let mut spans = vec![Span::styled("  elsewhere", Style::default().fg(t.muted))];
        for s in [Status::Blocked, Status::Done, Status::Working, Status::Idle] {
            let n = elsewhere.iter().filter(|a| a.status == s).count();
            if n > 0 {
                spans.push(Span::styled(format!(" {}{n}", status_icon(app, s)), Style::default().fg(t.status(s))));
            }
        }
        if let Some(target) = elsewhere.iter().min_by_key(|a| (a.status.urgency(), a.id))
            && y < area.bottom()
        {
            app.hits.push((Rect { y, height: 1, ..area }, Hit::Pane(target.id)));
        }
        line(f, &mut y, Line::from(spans));
    }

    if area.height > 2 {
        let prefix = app.keymap.prefix.to_string();
        let hint = Line::from(vec![
            Span::styled(format!(" {prefix} ?"), Style::default().fg(t.accent)),
            Span::styled(" help  ", Style::default().fg(t.muted)),
            Span::styled(format!("{prefix} w"), Style::default().fg(t.accent)),
            Span::styled(" jump", Style::default().fg(t.muted)),
        ]);
        f.render_widget(Paragraph::new(hint).style(base), Rect { y: area.bottom() - 1, height: 1, ..inner });
    }
}

fn tab_status(app: &App, tab: &TabInfo) -> Status {
    tab.layout
        .leaves()
        .iter()
        .filter_map(|id| app.snap.terms.get(id))
        .map(|t| t.status)
        .min_by_key(|s| s.urgency())
        .unwrap_or(Status::None)
}

fn tab_label(app: &App, tab: &TabInfo) -> String {
    if !tab.name.is_empty() {
        return tab.name.clone();
    }
    app.snap.terms.get(&tab.focus).map(|t| t.display_name().to_string()).unwrap_or_else(|| "shell".into())
}

fn draw_tabs(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    f.render_widget(Block::default().style(Style::default().bg(t.bg)), area);
    let Some(ws) = app.active_ws().cloned() else { return };
    let mut x = area.x;
    for (i, tab) in ws.tabs.iter().enumerate() {
        let active = tab.id == ws.active_tab;
        let status = tab_status(app, tab);
        let icon = if status == Status::None { String::new() } else { format!("{} ", status_icon(app, status)) };
        let label = format!(" {} {}{} ", i + 1, icon, truncate(&tab_label(app, tab), 24));
        let width = label.width() as u16;
        if x + width > area.right() {
            break;
        }
        let style = if active {
            Style::default().bg(app.ws_color(&ws)).fg(t.tab_active_fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(t.bg).fg(t.muted)
        };
        let mut spans = vec![Span::styled(label, style)];
        if status != Status::None && !active {
            // Colour just the icon on inactive tabs so attention stands out.
            let s = format!(" {} ", i + 1);
            spans = vec![
                Span::styled(s, style),
                Span::styled(icon.clone(), Style::default().bg(t.bg).fg(t.status(status))),
                Span::styled(format!("{} ", truncate(&tab_label(app, tab), 24)), style),
            ];
        }
        let r = Rect { x, width, ..area };
        f.render_widget(Paragraph::new(Line::from(spans)), r);
        app.hits.push((r, Hit::Tab(ws.id, tab.id)));
        x += width + 1;
    }
    if x + 3 <= area.right() {
        let r = Rect { x, width: 3, ..area };
        f.render_widget(Paragraph::new(Span::styled(" + ", Style::default().fg(t.muted))), r);
        app.hits.push((r, Hit::NewTab));
    }
    let name = format!(" {} ", ws.name);
    let w = name.width() as u16;
    if area.width > w + x.saturating_sub(area.x) + 4 {
        let r = Rect { x: area.right() - w, width: w, ..area };
        f.render_widget(Paragraph::new(Span::styled(name, Style::default().fg(t.accent).add_modifier(Modifier::BOLD))), r);
    }
}

fn border_type(s: &str) -> BorderType {
    match s {
        "plain" => BorderType::Plain,
        "double" => BorderType::Double,
        "thick" => BorderType::Thick,
        _ => BorderType::Rounded,
    }
}

fn draw_panes(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    f.render_widget(Block::default().style(Style::default().bg(t.bg)), area);
    let Some(tab) = app.active_tab().cloned() else {
        let msg = Paragraph::new(Line::from(Span::styled("starting…", Style::default().fg(t.muted))));
        f.render_widget(msg, area);
        return;
    };
    let zoomed = app.is_zoomed();
    let rects: Vec<(TermId, Rect)> =
        if zoomed { vec![(tab.focus, area)] } else { tab.layout.rects(area) };
    let multi = tab.layout.leaves().len() > 1;
    let bt = border_type(&app.cfg.ui.border_style);
    // Every pane in a workspace wears its colour: a faint wash behind the text and a
    // coloured border, so typing into the wrong workspace is obvious at a glance.
    let wcolor = app.active_ws().map(|w| app.ws_color(w)).unwrap_or(t.accent);
    let tint = app.cfg.ui.workspace_tint;
    let pane_bg = if tint > 0.0 { blend(t.bg, wcolor, tint) } else { Color::Reset };

    for (term, rect) in rects {
        let focused = term == tab.focus;
        let info = app.snap.terms.get(&term).cloned();
        let color = if tint <= 0.0 {
            if focused && multi { t.border_active } else { t.border }
        } else if focused {
            wcolor
        } else {
            blend(t.border, wcolor, 0.35)
        };
        let mut title = vec![Span::raw(" ")];
        if let Some(info) = &info {
            if info.status != Status::None && app.cfg.ui.pane_status {
                title.push(Span::styled(format!("{} ", status_icon(app, info.status)), Style::default().fg(t.status(info.status))));
            }
            let name_style = if focused { Style::default().fg(t.fg).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.muted) };
            title.push(Span::styled(truncate(&info.display_name(), rect.width.saturating_sub(16) as usize), name_style));
            // Shells: where the pane is right now.
            // A program running in a shell pane: say where. (At a prompt the name is the folder.)
            if info.agent.is_none() && !info.is_shell() && rect.width > 30 {
                let dir = truncate(&short_dir(&info.cwd), rect.width.saturating_sub(24) as usize);
                title.push(Span::styled(format!(" · {dir}"), Style::default().fg(t.muted)));
            }
            if info.status != Status::None && app.cfg.ui.pane_status {
                title.push(Span::styled(format!(" {}", info.status.label()), Style::default().fg(t.status(info.status))));
            }
        }
        title.push(Span::raw(" "));
        let mut right = Vec::new();
        let copying = matches!(&app.mode, Mode::Copy(c) if c.term == term);
        if let Mode::Copy(c) = &app.mode
            && copying
        {
            let pos = format!(" COPY {}/{} ", c.cur.0 + 1, c.lines.len());
            right.push(Span::styled(pos, Style::default().fg(t.bg).bg(t.accent).add_modifier(Modifier::BOLD)));
        } else if let Some(n) = app.scroll.get(&term) {
            right.push(Span::styled(format!(" ↑{n} "), Style::default().fg(t.accent)));
        }
        if zoomed {
            right.push(Span::styled(" zoom ", Style::default().fg(t.accent)));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(bt)
            .border_style(Style::default().fg(color))
            .title(Line::from(title))
            .title(Line::from(right).right_aligned())
            .style(Style::default().bg(pane_bg));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        app.pane_frames.push((term, rect));
        app.panes.push((term, inner));

        if copying {
            if let Mode::Copy(c) = &mut app.mode {
                c.height = inner.height as usize;
                c.width = inner.width as usize;
                f.render_widget(Clear, inner);
                render_copy(c, inner, f.buffer_mut(), t);
            }
            continue;
        }
        if let Some(p) = app.parsers.get(&term) {
            let screen = p.screen();
            render_screen(screen, inner, f.buffer_mut(), pane_bg);
            if focused && !screen.hide_cursor() && !app.scroll.contains_key(&term) && matches!(app.mode, Mode::Normal | Mode::Prefix { .. }) {
                let (row, col) = screen.cursor_position();
                if row < inner.height && col < inner.width {
                    f.set_cursor_position(Position::new(inner.x + col, inner.y + row));
                }
            }
        }
    }
}

pub(super) fn render_copy(c: &Copy, area: Rect, buf: &mut Buffer, t: &crate::theme::Theme) {
    let select = Style::default().bg(t.accent).fg(t.bg);
    let hit = Style::default().bg(t.working).fg(Color::Black);
    for row in 0..area.height {
        let li = c.top + row as usize;
        let Some(line) = c.lines.get(li) else { break };
        let y = area.y + row;
        let matches = c.matches(li);
        let mut x = area.x;
        let mut n = 0;
        for (ci, ch) in line.chars().enumerate() {
            n = ci + 1;
            let w = ch.width().unwrap_or(0) as u16;
            if w == 0 {
                continue;
            }
            if x + w > area.right() {
                break;
            }
            let mut style = Style::default().fg(t.fg);
            if matches.iter().any(|(s, e)| ci >= *s && ci < *e) {
                style = hit;
            }
            if c.selected(li, ci) {
                style = select;
            }
            if (li, ci) == c.cur {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let mut tmp = [0u8; 4];
            buf[(x, y)].set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
            x += w;
        }
        // Show the cursor (and line selections) past the end of the text.
        if x < area.right() {
            if c.cur.0 == li && c.cur.1 >= n {
                buf[(x, y)].set_symbol(" ").set_style(Style::default().add_modifier(Modifier::REVERSED));
            } else if c.selected(li, n) {
                buf[(x, y)].set_symbol(" ").set_style(select);
            }
        }
    }
    let bar = if let Some(input) = &c.input {
        Some((format!("{}{input}", if c.backward { "?" } else { "/" }), t.accent))
    } else {
        c.message.clone().map(|m| (m, t.blocked))
    };
    if let Some((text, color)) = bar
        && area.height > 0
    {
        let r = Rect { y: area.bottom() - 1, height: 1, ..area };
        Paragraph::new(Span::styled(text, Style::default().fg(t.bg).bg(color)))
            .style(Style::default().bg(color))
            .render(r, buf);
    }
}

/// The theme's terminal colours, if it has its own (set each frame from the theme).
static PALETTE: std::sync::RwLock<Option<[Color; 7]>> = std::sync::RwLock::new(None);

/// What programs' "bright black" backgrounds become: a subtle panel, not a grey slab.
static PANEL: std::sync::RwLock<Option<Color>> = std::sync::RwLock::new(None);

pub(super) fn set_palette(p: Option<[Color; 7]>, panel: Option<Color>) {
    if let Ok(mut w) = PALETTE.write() {
        *w = p;
    }
    if let Ok(mut w) = PANEL.write() {
        *w = panel;
    }
}

/// Map the basic ANSI colours (red … cyan, and gray) to the theme's.
fn themed(c: Color, pal: &Option<[Color; 7]>) -> Color {
    match (c, pal) {
        (Color::Indexed(i @ 1..=6), Some(p)) => p[i as usize - 1],
        (Color::Indexed(i @ 9..=14), Some(p)) => p[i as usize - 9],
        (Color::Indexed(8), Some(p)) => p[6],
        _ => c,
    }
}

pub(super) fn vt_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Draw a terminal screen. Cells with the default background get `default_bg` (the
/// workspace tint); programs' own background colours are left alone.
pub(super) fn render_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer, default_bg: Color) {
    let (rows, cols) = screen.size();
    let pal = PALETTE.read().map(|p| *p).unwrap_or(None);
    let panel = PANEL.read().map(|p| *p).unwrap_or(None);
    for row in 0..area.height.min(rows) {
        for col in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = themed(vt_color(cell.fgcolor()), &pal);
            let mut bg = match vt_color(cell.bgcolor()) {
                Color::Reset => default_bg,
                Color::Indexed(8) if panel.is_some() => panel.unwrap_or(default_bg),
                c => c,
            };
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
                if fg == Color::Reset {
                    fg = Color::Black;
                }
                if bg == Color::Reset {
                    bg = Color::White;
                }
            }
            let mut style = Style::default().fg(fg).bg(bg);
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.dim() {
                style = style.add_modifier(Modifier::DIM);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            let contents = cell.contents();
            let c = &mut buf[(area.x + col, area.y + row)];
            c.set_symbol(if contents.is_empty() { " " } else { contents });
            c.set_style(style);
        }
    }
}

fn draw_status(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    f.render_widget(Block::default().style(Style::default().bg(t.sidebar_bg)), area);
    let (mode, mode_color) = mode_label(app, t);
    let mut left = vec![
        Span::styled(format!(" {mode} "), Style::default().bg(mode_color).fg(t.bg).add_modifier(Modifier::BOLD)),
    ];
    if let Some(w) = app.active_ws() {
        left.push(Span::styled(format!(" {} ", w.name), Style::default().bg(app.ws_color(w)).fg(t.bg).add_modifier(Modifier::BOLD)));
    }
    left.push(Span::raw(" "));
    if let Some((msg, _, err)) = &app.notice {
        left.push(Span::styled(msg.clone(), Style::default().fg(if *err { t.blocked } else { t.fg })));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);

    let mut right: Vec<Span> = Vec::new();
    for s in [Status::Blocked, Status::Done, Status::Working, Status::Idle] {
        let n = app.snap.terms.values().filter(|x| x.status == s).count();
        if n > 0 {
            right.push(Span::styled(format!("{} {n} {}  ", status_icon(app, s), s.label()), Style::default().fg(t.status(s))));
        }
    }
    let w: usize = right.iter().map(|s| s.content.width()).sum();
    if (w as u16) < area.width / 2 {
        let r = Rect { x: area.right() - w as u16, width: w as u16, ..area };
        f.render_widget(Paragraph::new(Line::from(right)), r);
    }
}

pub(super) fn mode_label(app: &App, t: &crate::theme::Theme) -> (&'static str, Color) {
    match app.mode {
        Mode::Normal => ("NORMAL", t.muted),
        Mode::Prefix { .. } => ("PREFIX", t.accent),
        Mode::Picker { commands: true, .. } => ("PALETTE", t.accent),
        Mode::Picker { .. } => ("JUMP", t.accent),
        Mode::Prompt { .. } => ("INPUT", t.accent),
        Mode::Help { .. } => ("HELP", t.accent),
        Mode::Copy(_) => ("COPY", t.working),
        Mode::Worktrees { .. } => ("WORKTREE", t.accent),
        Mode::Menu(_) => ("MENU", t.accent),
        Mode::Quick(_) => ("PROMPT", t.idle),
        Mode::Settings(_) | Mode::HySettings(_) => ("SETTINGS", t.accent),
        Mode::Jump { .. } => ("JUMP", t.accent),
        Mode::Finder(_) => ("OPEN", t.accent),
        Mode::HyPane(_) => ("NEW", t.accent),
        Mode::Side => ("SIDEBAR", t.accent),
        Mode::Ship(_) => ("SHIP", t.accent),
        Mode::Ideas(_) => ("IDEAS", t.accent),
        Mode::HyMenu(_) => ("MENU", t.accent),
        Mode::Tickets(_) => ("TICKETS", t.accent),
        Mode::RaceNew(_) | Mode::Race(_) => ("RACE", t.accent),
        Mode::Tree { .. } => ("BROWSE", t.accent),
        Mode::Talk { .. } => ("TALK", t.accent),
        Mode::NewPane(_) => ("NEW PANE", t.accent),
        Mode::Files(_) => ("FILES", t.accent),
        Mode::Tasks { review: Some(_), .. } => ("REVIEW", t.done),
        Mode::Tasks { .. } => ("TASKS", t.accent),
        Mode::Inbox(_) => ("INBOX", t.accent),
        Mode::Toolbox(_) => ("TOOLBOX", t.accent),
    }
}

pub(super) fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn overlay_block<'a>(t: &crate::theme::Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .style(Style::default().bg(t.sidebar_bg).fg(t.fg))
        .title(Span::styled(format!(" {title} "), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))
}

fn draw_which_key(app: &App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    let entries: Vec<(String, String)> = app
        .keymap
        .prefixed_order
        .iter()
        .filter_map(|k| app.keymap.prefixed.get(k).map(|a| (k.to_string(), a.describe())))
        .collect();
    let col_w = 26u16;
    let cols = ((area.width.saturating_sub(4)) / col_w).max(1);
    let rows = (entries.len() as u16).div_ceil(cols);
    let h = (rows + 2).min(area.height);
    let w = (cols * col_w + 2).min(area.width);
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.bottom().saturating_sub(h), width: w, height: h };
    f.render_widget(Clear, r);
    let block = overlay_block(t, "prefix");
    let inner = block.inner(r);
    f.render_widget(block, r);
    for (i, (k, d)) in entries.iter().enumerate() {
        let c = i as u16 / rows;
        let row = i as u16 % rows;
        if row >= inner.height {
            continue;
        }
        let cell = Rect { x: inner.x + c * col_w, y: inner.y + row, width: col_w.min(inner.right().saturating_sub(inner.x + c * col_w)), height: 1 };
        let line = Line::from(vec![
            Span::styled(format!("{k:>6} "), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
            Span::styled(truncate(d, col_w as usize - 8), Style::default().fg(t.fg)),
        ]);
        f.render_widget(Paragraph::new(line), cell);
    }
}

fn draw_picker(app: &App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, query: &str, sel: usize, commands: bool) {
    let items = app.pick_items(query, commands);
    let r = centered(area, 80, 22);
    f.render_widget(Clear, r);
    let block = overlay_block(t, if commands { "command palette" } else { "jump to" });
    let inner = block.inner(r);
    f.render_widget(block, r);
    let input = Line::from(vec![Span::styled("› ", Style::default().fg(t.accent)), Span::raw(query.to_string())]);
    f.render_widget(Paragraph::new(input), Rect { height: 1, ..inner });
    f.set_cursor_position(Position::new(inner.x + 2 + query.width() as u16, inner.y));
    let list_h = inner.height.saturating_sub(2) as usize;
    let start = sel.saturating_sub(list_h.saturating_sub(1));
    for (i, item) in items.iter().enumerate().skip(start).take(list_h) {
        let y = inner.y + 2 + (i - start) as u16;
        let selected = i == sel;
        let selected_row = selected;
        let bg = if selected_row { focus_bg(t) } else { t.sidebar_bg };
        let (icon, color) = match &item.target {
            PickTarget::Workspace(id) => {
                ("■".to_string(), app.snap.workspace(*id).map(|w| app.ws_color(w)).unwrap_or(t.muted))
            }
            PickTarget::Pane(_) => (status_icon(app, item.status), t.status(item.status)),
            PickTarget::Command(_) => ("›".to_string(), t.accent),
        };
        let line = if let PickTarget::Command(_) = item.target {
            let key_w = item.key.width();
            let label_w = (inner.width as usize).saturating_sub(key_w + 5);
            let label = truncate(&item.label, label_w);
            let pad = label_w.saturating_sub(label.width());
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color).bg(bg)),
                Span::styled(label, Style::default().fg(t.fg).bg(bg)),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(format!("{} ", item.key), Style::default().fg(t.muted).bg(bg)),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color).bg(bg)),
                Span::styled(format!("{:<20} ", truncate(&item.label, 20)), Style::default().fg(t.fg).bg(bg).add_modifier(Modifier::BOLD)),
                Span::styled(truncate(&item.detail, inner.width.saturating_sub(26) as usize), Style::default().fg(t.muted).bg(bg)),
            ])
        };
        f.render_widget(Paragraph::new(if selected_row { focus_row(line, t) } else { line }).style(Style::default().bg(bg)), Rect { y, height: 1, ..inner });
    }
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  no matches", Style::default().fg(t.muted))),
            Rect { y: inner.y + 2, height: 1, ..inner },
        );
    }
}

fn draw_menu(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, m: &modal::Menu) {
    let label_w = m.items.iter().map(|i| i.label.width()).max().unwrap_or(10);
    let key_w = m.items.iter().map(|i| i.key.width()).max().unwrap_or(0);
    let w = (label_w + key_w + 7) as u16;
    let h = m.items.len() as u16 + 2;
    // Menus open in the middle of the screen, like every other popup.
    let _ = m.at;
    super::design::dim(f.buffer_mut(), area, t);
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    let title = app.focused().and_then(|id| app.snap.terms.get(&id)).map(|ti| ti.display_name().to_string()).unwrap_or_default();
    let block = overlay_block(t, &title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    for (i, item) in m.items.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let selected_row = i == m.sel;
        let bg = if selected_row { focus_bg(t) } else { t.sidebar_bg };
        let pad = (inner.width as usize).saturating_sub(item.label.width() + item.key.width() + 3);
        let line = Line::from(vec![
            Span::styled(format!(" {}", item.label), Style::default().fg(t.fg).bg(bg)),
            Span::styled(" ".repeat(pad), Style::default().bg(bg)),
            Span::styled(format!("{} ", item.key), Style::default().fg(t.muted).bg(bg)),
        ]);
        let row = Rect { y, height: 1, ..inner };
        f.render_widget(Paragraph::new(if selected_row { focus_row(line, t) } else { line }).style(Style::default().bg(bg)), row);
        app.hits.push((row, Hit::MenuItem(i)));
    }
}

/// Wrap text into lines of at most `width` columns (by character, keeping explicit newlines).
fn wrap_chars(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut w = 0;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(String::new());
            w = 0;
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if w + cw > width.max(1) {
            lines.push(String::new());
            w = 0;
        }
        lines.last_mut().unwrap().push(ch);
        w += cw;
    }
    lines
}

fn draw_quick(app: &App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, q: &modal::Quick) {
    let r = centered(area, 76, 12);
    f.render_widget(Clear, r);
    let block = overlay_block(t, "quick prompt");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let agent = if q.place == modal::Place::Here {
        app.focused().and_then(|id| app.snap.terms.get(&id)).map(|ti| ti.display_name().to_string()).unwrap_or_default()
    } else {
        app.cfg.quick.agents.get(q.agent).map(|a| a.name.clone()).unwrap_or_else(|| "?".into())
    };
    let ws = app.active_ws();
    let ws_name = ws.map(|w| w.name.clone()).unwrap_or_default();
    let ws_color = ws.map(|w| app.ws_color(w)).unwrap_or(t.accent);
    let chip = |s: String, c| Span::styled(format!(" {s} "), Style::default().fg(t.bg).bg(c).add_modifier(Modifier::BOLD));
    let header = Line::from(vec![
        chip(agent, t.idle),
        Span::raw(" in "),
        chip(q.place.label().into(), t.accent),
        Span::raw(" of "),
        chip(ws_name, ws_color),
    ]);
    f.render_widget(Paragraph::new(header), Rect { height: 1, ..inner });

    let text_area = Rect { y: inner.y + 2, height: inner.height.saturating_sub(4), ..inner };
    let lines = wrap_chars(&q.text, text_area.width.saturating_sub(2) as usize);
    let visible = text_area.height as usize;
    let skip = lines.len().saturating_sub(visible);
    for (i, l) in lines.iter().skip(skip).enumerate() {
        let prefix = if i == 0 && skip == 0 { "› " } else { "  " };
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(prefix, Style::default().fg(t.accent)), Span::raw(l.clone())])),
            Rect { y: text_area.y + i as u16, height: 1, ..text_area },
        );
    }
    if q.text.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  describe the task…", Style::default().fg(t.muted))),
            Rect { height: 1, ..text_area },
        );
    }
    let last = lines.last().map(|l| l.width()).unwrap_or(0) as u16;
    let row = (lines.len() - skip).saturating_sub(1) as u16;
    f.set_cursor_position(Position::new(text_area.x + 2 + last, text_area.y + row));

    let hint = "Enter start · Tab agent · Shift+Tab where · Alt+Enter newline · Esc cancel";
    f.render_widget(
        Paragraph::new(Span::styled(truncate(hint, inner.width as usize), Style::default().fg(t.muted))),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

fn draw_settings(app: &App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, s: &modal::Settings) {
    let h = modal::SETTINGS.len() as u16 + 5;
    let r = centered(area, 70, h);
    f.render_widget(Clear, r);
    let block = overlay_block(t, "settings");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let label_w = modal::SETTINGS.iter().map(|x| x.label.width()).max().unwrap_or(10) + 2;
    for (i, setting) in modal::SETTINGS.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom().saturating_sub(2) {
            break;
        }
        let selected = i == s.sel;
        let selected_row = selected;
        let bg = if selected_row { focus_bg(t) } else { t.sidebar_bg };
        let value = if selected && s.capturing {
            "press the new leader key…".to_string()
        } else if selected && s.editing.is_some() {
            format!("{}▏", s.editing.as_deref().unwrap_or(""))
        } else {
            modal::display(&app.cfg, setting)
        };
        let value = match setting.kind {
            modal::Kind::Choice(_) | modal::Kind::Number { .. } | modal::Kind::Int { .. } if selected && !s.capturing => {
                format!("‹ {value} ›")
            }
            _ => value,
        };
        let value_color = if setting.path == "theme" || setting.path.ends_with("tint") { t.accent } else { t.fg };
        let line = Line::from(vec![
            Span::styled(format!(" {:<label_w$}", setting.label), Style::default().fg(if selected { t.fg } else { t.muted }).bg(bg)),
            Span::styled(value, Style::default().fg(value_color).bg(bg).add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() })),
        ]);
        f.render_widget(Paragraph::new(if selected_row { focus_row(line, t) } else { line }).style(Style::default().bg(bg)), Rect { y, height: 1, ..inner });
    }
    let hint = "↑↓ choose · ←→ change · Enter edit · Esc close";
    let path = format!("saved to {}", crate::config::config_path().display());
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(t.muted))),
        Rect { y: inner.bottom().saturating_sub(2), height: 1, ..inner },
    );
    f.render_widget(
        Paragraph::new(Span::styled(truncate(&path, inner.width as usize), Style::default().fg(t.muted))),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_worktrees(
    app: &App,
    f: &mut Frame,
    area: Rect,
    t: &crate::theme::Theme,
    rows: &[super::WtRow],
    query: &str,
    sel: usize,
    loading: bool,
) {
    let r = centered(area, 80, 18);
    f.render_widget(Clear, r);
    let block = overlay_block(t, "worktrees: Enter opens, type a new branch to create");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let input = Line::from(vec![Span::styled("› ", Style::default().fg(t.accent)), Span::raw(query.to_string())]);
    f.render_widget(Paragraph::new(input), Rect { height: 1, ..inner });
    f.set_cursor_position(Position::new(inner.x + 2 + query.width() as u16, inner.y));
    if loading {
        f.render_widget(
            Paragraph::new(Span::styled("  reading worktrees…", Style::default().fg(t.muted))),
            Rect { y: inner.y + 2, height: 1, ..inner },
        );
        return;
    }
    let list_h = inner.height.saturating_sub(2) as usize;
    let start = sel.saturating_sub(list_h.saturating_sub(1));
    for (i, row) in rows.iter().enumerate().skip(start).take(list_h) {
        let y = inner.y + 2 + (i - start) as u16;
        let selected_row = i == sel;
        let bg = if selected_row { focus_bg(t) } else { t.sidebar_bg };
        let line = match row {
            super::WtRow::Existing(w) => {
                let open = app.snap.workspaces.iter().find(|x| super::same_dir(&x.cwd, &w.path));
                let (dot, dot_color) = match open {
                    Some(ws) => ("● ", app.ws_color(ws)),
                    None => ("○ ", t.muted),
                };
                let tag = if w.main { " main" } else { "" };
                Line::from(vec![
                    Span::styled(format!(" {dot}"), Style::default().fg(dot_color).bg(bg)),
                    Span::styled(format!("{:<24}", truncate(&w.branch, 24)), Style::default().fg(t.fg).bg(bg).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{tag:<6}"), Style::default().fg(t.accent).bg(bg)),
                    Span::styled(truncate(&w.path.display().to_string(), inner.width.saturating_sub(34) as usize), Style::default().fg(t.muted).bg(bg)),
                ])
            }
            super::WtRow::Create(text) => Line::from(vec![
                Span::styled(" + ", Style::default().fg(t.idle).bg(bg)),
                Span::styled("create worktree ", Style::default().fg(t.fg).bg(bg)),
                Span::styled(text.clone(), Style::default().fg(t.idle).bg(bg).add_modifier(Modifier::BOLD)),
            ]),
        };
        f.render_widget(Paragraph::new(if selected_row { focus_row(line, t) } else { line }).style(Style::default().bg(bg)), Rect { y, height: 1, ..inner });
    }
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  type a branch name to create a worktree", Style::default().fg(t.muted))),
            Rect { y: inner.y + 2, height: 1, ..inner },
        );
    }
}

fn draw_prompt(f: &mut Frame, area: Rect, t: &crate::theme::Theme, label: &str, input: &str, confirm: bool) {
    let r = centered(area, 64, 3);
    f.render_widget(Clear, r);
    let block = overlay_block(t, label);
    let inner = block.inner(r);
    f.render_widget(block, r);
    if !confirm {
        f.render_widget(Paragraph::new(Line::from(vec![Span::styled("› ", Style::default().fg(t.accent)), Span::raw(input.to_string())])), inner);
        f.set_cursor_position(Position::new(inner.x + 2 + input.width() as u16, inner.y));
    } else {
        f.render_widget(Paragraph::new(Span::styled("y to confirm, any other key cancels", Style::default().fg(t.muted))), inner);
    }
}

fn draw_help(app: &App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, scroll: u16) {
    let r = centered(area, 76, area.height.saturating_sub(4));
    f.render_widget(Clear, r);
    let block = overlay_block(t, "help — Esc to close");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let key = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let head = Style::default().fg(t.muted).add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::from(vec![Span::styled("prefix ", head), Span::styled(app.keymap.prefix.to_string(), key)]),
        Line::from(Span::styled(format!("config  {}", crate::config::config_path().display()), Style::default().fg(t.muted))),
        Line::raw(""),
        Line::from(Span::styled("AFTER PREFIX", head)),
    ];
    for k in &app.keymap.prefixed_order {
        if let Some(a) = app.keymap.prefixed.get(k) {
            lines.push(Line::from(vec![Span::styled(format!("  {:<12}", k.to_string()), key), Span::raw(a.describe())]));
        }
    }
    if !app.keymap.global.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled("GLOBAL", head)));
        let mut g: Vec<_> = app.keymap.global.iter().collect();
        g.sort_by_key(|(k, _)| k.to_string());
        for (k, a) in g {
            lines.push(Line::from(vec![Span::styled(format!("  {:<12}", k.to_string()), key), Span::raw(a.describe())]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("MOUSE", head)));
    lines.push(Line::raw("  click a pane, tab, workspace or agent to focus it; wheel scrolls history"));
    lines.push(Line::raw("  hold Shift to select text with your terminal"));
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

// ---- panels: files, tasks, inbox, toolbox -------------------------------------------------

/// What a two-column panel shows: a filterable list on the left, details on the right.
struct PanelView {
    title: String,
    tabs: Vec<String>,
    tab: usize,
    /// None hides the search line (lists that don't filter).
    query: Option<String>,
    rows: Vec<Line<'static>>,
    sel: Option<usize>,
    detail: Vec<Line<'static>>,
    detail_scroll: u16,
    footer: Line<'static>,
    /// Shown instead of rows when there are none (also used for "loading…").
    empty: String,
    left_pct: u16,
}

fn draw_panel(f: &mut Frame, area: Rect, t: &crate::theme::Theme, v: PanelView) {
    let w = (area.width.saturating_mul(92) / 100).max(60).min(area.width);
    let h = (area.height.saturating_mul(88) / 100).max(16).min(area.height);
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, r);
    let block = overlay_block(t, &v.title);
    let inner = block.inner(r);
    f.render_widget(block, r);

    let mut y = inner.y;
    if !v.tabs.is_empty() {
        let mut spans = Vec::new();
        for (i, name) in v.tabs.iter().enumerate() {
            let style = if i == v.tab {
                Style::default().fg(t.bg).bg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.muted)
            };
            spans.push(Span::styled(format!(" {name} "), style));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled("Tab switches", Style::default().fg(t.muted)));
        f.render_widget(Paragraph::new(Line::from(spans)), Rect { y, height: 1, ..inner });
        y += 1;
    }
    if let Some(q) = &v.query {
        let line = Line::from(vec![
            Span::styled("› ", Style::default().fg(t.accent)),
            Span::raw(q.clone()),
            Span::styled(if q.is_empty() { "type to filter" } else { "" }, Style::default().fg(t.muted)),
        ]);
        f.render_widget(Paragraph::new(line), Rect { y, height: 1, ..inner });
        f.set_cursor_position(Position::new(inner.x + 2 + q.width() as u16, y));
        y += 1;
    }
    y += 1;
    let body = Rect { y, height: inner.bottom().saturating_sub(y + 1), ..inner };
    let left_w = body.width * v.left_pct / 100;
    let left = Rect { width: left_w, ..body };
    let right = Rect { x: body.x + left_w + 1, width: body.width.saturating_sub(left_w + 1), ..body };
    for yy in body.y..body.bottom() {
        f.render_widget(
            Paragraph::new(Span::styled("│", Style::default().fg(t.border))),
            Rect { x: body.x + left_w, y: yy, width: 1, height: 1 },
        );
    }

    if v.rows.is_empty() {
        f.render_widget(
            Paragraph::new(v.empty.clone()).style(Style::default().fg(t.muted)).wrap(ratatui::widgets::Wrap { trim: false }),
            Rect { height: body.height.min(6), ..left },
        );
    }
    let h = left.height as usize;
    let sel = v.sel.unwrap_or(0);
    let start = sel.saturating_sub(h.saturating_sub(1));
    for (i, row) in v.rows.into_iter().enumerate().skip(start).take(h) {
        let ry = left.y + (i - start) as u16;
        let selected_row = Some(i) == v.sel;
        let style = if selected_row { Style::default().bg(focus_bg(t)) } else { Style::default() };
        let row = if selected_row { focus_row(row, t) } else { row };
        f.render_widget(Paragraph::new(row).style(style), Rect { y: ry, height: 1, ..left });
    }
    f.render_widget(
        Paragraph::new(v.detail).scroll((v.detail_scroll, 0)).wrap(ratatui::widgets::Wrap { trim: false }),
        right,
    );
    f.render_widget(Paragraph::new(v.footer), Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner });
}

pub(super) fn hint(t: &crate::theme::Theme, pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (k, what)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        }
        spans.push(Span::styled(k.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {what}"), Style::default().fg(t.muted)));
    }
    Line::from(spans)
}

fn files_panel(app: &App, t: &crate::theme::Theme) -> PanelView {
    let Mode::Files(v) = &app.mode else { unreachable!() };
    let list = v.visible();
    let loading = if v.tab == 0 { v.recent.is_none() } else { v.project.is_none() };
    let rows: Vec<Line<'static>> = list
        .iter()
        .map(|e| {
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if v.tab == 0 {
                Line::from(vec![
                    Span::styled(format!(" {name}"), Style::default().fg(t.fg)),
                    Span::styled(format!("  {} · {}", e.place, super::files::ago(e.modified)), Style::default().fg(t.muted)),
                ])
            } else {
                let label = v.label(e);
                let dir = label.strip_suffix(&name).unwrap_or("").to_string();
                Line::from(vec![Span::styled(format!(" {dir}"), Style::default().fg(t.muted)), Span::styled(name, Style::default().fg(t.fg))])
            }
        })
        .collect();
    let mut detail = Vec::new();
    if let Some(e) = list.get(v.sel) {
        detail.push(Line::from(Span::styled(v.label(e), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))));
        detail.push(Line::from(Span::styled(
            format!("{} · {} · {}", e.place, super::files::human_size(e.size), super::files::ago(e.modified)),
            Style::default().fg(t.muted),
        )));
        detail.push(Line::raw(""));
        if let Some((_, lines)) = &v.preview {
            detail.extend(lines.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(t.fg)))));
        }
    }
    let root = v.root.display().to_string();
    PanelView {
        title: format!("files · {}", truncate(&root, 50)),
        tabs: vec!["Recent".into(), "Project".into()],
        tab: v.tab,
        query: Some(v.query.clone()),
        sel: (!rows.is_empty()).then_some(v.sel),
        rows,
        detail,
        detail_scroll: 0,
        footer: hint(t, &[("Enter", "put path in prompt"), ("^O", "open"), ("^F", "show in folder"), ("^Y", "copy path"), ("Esc", "close")]),
        empty: if loading {
            "  looking…".into()
        } else if v.tab == 0 {
            "  Nothing new in Downloads, Desktop, Documents or this project in the last few days.".into()
        } else {
            "  No files match.".into()
        },
        left_pct: 45,
    }
}

fn stage_style(t: &crate::theme::Theme, s: super::tasks::Stage) -> (String, Color) {
    use super::tasks::Stage::*;
    match s {
        NeedsYou => ("▲".into(), t.blocked),
        Ready => ("◆".into(), t.done),
        Working => ("●".into(), t.working),
        Idle => ("○".into(), t.muted),
    }
}

fn tasks_panel(app: &App, t: &crate::theme::Theme) -> PanelView {
    let Mode::Tasks { sel, review } = &app.mode else { unreachable!() };
    if let Some(r) = review {
        return review_panel(t, r);
    }
    let rows_data = super::tasks::rows(&app.snap);
    let rows: Vec<Line<'static>> = rows_data
        .iter()
        .map(|r| {
            let (icon, c) = stage_style(t, r.stage);
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(c)),
                Span::styled(format!("{:<22}", truncate(&r.name, 22)), Style::default().fg(t.fg).add_modifier(Modifier::BOLD)),
                Span::styled(r.stage.label().to_string(), Style::default().fg(c)),
            ])
        })
        .collect();
    let mut detail = Vec::new();
    if let Some(r) = rows_data.get(*sel) {
        let (icon, c) = stage_style(t, r.stage);
        detail.push(Line::from(Span::styled(r.name.clone(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))));
        detail.push(Line::from(vec![Span::styled(format!("{icon} {}", r.stage.label()), Style::default().fg(c))]));
        detail.push(Line::raw(""));
        if !r.summary.is_empty() {
            detail.push(Line::from(Span::styled(format!("asked: {}", r.summary), Style::default().fg(t.fg))));
            detail.push(Line::raw(""));
        }
        detail.push(Line::from(Span::styled(format!("branch   {} → {}", r.branch, r.base), Style::default().fg(t.muted))));
        detail.push(Line::from(Span::styled(
            format!("changes  {} uncommitted file(s), {} commit(s) ahead", r.dirty, r.ahead),
            Style::default().fg(t.muted),
        )));
        detail.push(Line::from(Span::styled(format!("folder   {}", r.dir.display()), Style::default().fg(t.muted))));
    }
    PanelView {
        title: "tasks".into(),
        tabs: Vec::new(),
        tab: 0,
        query: None,
        sel: (!rows.is_empty()).then_some(*sel),
        rows,
        detail,
        detail_scroll: 0,
        footer: hint(t, &[("Enter", "review"), ("o", "open"), ("n", "new task"), ("Esc", "close")]),
        empty: format!(
            "  No tasks yet.\n\n  A task is an agent working on its own copy of the repo.\n  Press n (or {} q, then Shift+Tab to \"new worktree\") and describe the job.",
            app.keymap.prefix
        ),
        left_pct: 45,
    }
}

fn review_panel(t: &crate::theme::Theme, r: &super::tasks::Review) -> PanelView {
    let rows: Vec<Line<'static>> = r
        .files
        .iter()
        .map(|c| {
            let mut spans = vec![Span::styled(format!(" {}", c.path), Style::default().fg(t.fg))];
            if c.untracked {
                spans.push(Span::styled("  new", Style::default().fg(t.idle)));
            } else {
                spans.push(Span::styled(format!("  +{}", c.added), Style::default().fg(t.idle)));
                spans.push(Span::styled(format!(" -{}", c.removed), Style::default().fg(t.blocked)));
            }
            Line::from(spans)
        })
        .collect();
    let detail: Vec<Line<'static>> = r
        .diff
        .iter()
        .map(|l| {
            let color = if l.starts_with('+') {
                t.idle
            } else if l.starts_with('-') {
                t.blocked
            } else if l.starts_with("@@") {
                t.accent
            } else {
                t.muted
            };
            Line::from(Span::styled(l.clone(), Style::default().fg(color)))
        })
        .collect();
    let footer = match &r.confirm {
        Some((q, _)) => Line::from(vec![
            Span::styled(format!(" {q} "), Style::default().fg(t.bg).bg(t.working).add_modifier(Modifier::BOLD)),
            Span::styled("  y yes · any other key no", Style::default().fg(t.muted)),
        ]),
        None => hint(
            t,
            &[("c", "commit"), ("m", "merge"), ("p", "pull request"), ("r", "reply to agent"), ("x", "discard"), ("o", "open"), ("Esc", "back")],
        ),
    };
    let added: i64 = r.files.iter().map(|f| f.added).sum();
    let removed: i64 = r.files.iter().map(|f| f.removed).sum();
    PanelView {
        title: format!("review · {} ({} → {}) · {} files +{added} -{removed}", r.task.name, r.task.branch, r.task.base, r.files.len()),
        tabs: Vec::new(),
        tab: 0,
        query: None,
        sel: (!rows.is_empty()).then_some(r.sel),
        rows,
        detail,
        detail_scroll: r.scroll,
        footer,
        empty: "  The agent hasn't changed anything yet.".into(),
        left_pct: 32,
    }
}

fn inbox_panel(app: &App, t: &crate::theme::Theme) -> PanelView {
    use super::inbox::Tone;
    let Mode::Inbox(v) = &app.mode else { unreachable!() };
    let tone = |x: Tone| match x {
        Tone::Good => t.idle,
        Tone::Bad => t.blocked,
        Tone::Waiting => t.working,
        Tone::Plain => t.muted,
    };
    let list = v.visible();
    let rows: Vec<Line<'static>> = list
        .iter()
        .map(|i| {
            Line::from(vec![
                Span::styled(format!(" {:<8}", i.key), Style::default().fg(t.muted)),
                Span::styled(format!("{} ", truncate(&i.title, 40)), Style::default().fg(t.fg)),
                Span::styled(i.state.clone(), Style::default().fg(tone(i.tone))),
            ])
        })
        .collect();
    let mut detail = Vec::new();
    if let Some(i) = list.get(v.sel) {
        detail.push(Line::from(Span::styled(format!("{} {}", i.key, i.title), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))));
        detail.push(Line::from(Span::styled(i.state.clone(), Style::default().fg(tone(i.tone)))));
        detail.push(Line::from(Span::styled(i.meta.clone(), Style::default().fg(t.muted))));
        detail.push(Line::from(Span::styled(i.url.clone(), Style::default().fg(t.accent))));
        detail.push(Line::raw(""));
        detail.extend(i.body.lines().take(200).map(|l| Line::from(Span::styled(l.replace('\r', ""), Style::default().fg(t.fg)))));
    }
    let empty = match &v.lists[v.tab] {
        None => "  loading…".to_string(),
        Some(Err(e)) => format!("  {e}"),
        Some(Ok(_)) if !v.query.is_empty() => "  Nothing matches.".into(),
        Some(Ok(_)) => "  Nothing open. Nice.".into(),
    };
    PanelView {
        title: format!("inbox · {}", v.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        tabs: super::inbox::TABS.iter().map(|s| s.to_string()).collect(),
        tab: v.tab,
        query: Some(v.query.clone()),
        sel: (!rows.is_empty()).then_some(v.sel),
        rows,
        detail,
        detail_scroll: v.scroll,
        footer: hint(t, &[("Enter", "open in browser"), ("^T", "make it a task"), ("^R", "refresh"), ("Esc", "close")]),
        empty,
        left_pct: 55,
    }
}

fn toolbox_panel(app: &App, t: &crate::theme::Theme) -> PanelView {
    use super::toolbox::Row;
    let Mode::Toolbox(v) = &app.mode else { unreachable!() };
    let mut rows = Vec::new();
    let mut sel_row = None;
    let mut item_i = 0;
    if let Some(sections) = &v.sections {
        for row in v.rows() {
            match row {
                Row::Header(si) => {
                    let s = &sections[si];
                    rows.push(Line::from(vec![
                        Span::styled(format!(" {} ", s.tool), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{} ({})", s.title, s.items.len()), Style::default().fg(t.muted).add_modifier(Modifier::BOLD)),
                    ]));
                }
                Row::Item(si, ii) => {
                    let it = &sections[si].items[ii];
                    if item_i == v.sel {
                        sel_row = Some(rows.len());
                    }
                    item_i += 1;
                    let (dot, c) = if it.enabled { ("●", t.idle) } else { ("○", t.muted) };
                    rows.push(Line::from(vec![
                        Span::styled(format!("   {dot} "), Style::default().fg(c)),
                        Span::styled(format!("{} ", truncate(&it.name, 34)), Style::default().fg(if it.enabled { t.fg } else { t.muted })),
                        Span::styled(it.scope.clone(), Style::default().fg(t.muted)),
                    ]));
                }
            }
        }
    }
    let mut detail = Vec::new();
    if let Some(it) = v.selected() {
        detail.push(Line::from(Span::styled(it.name.clone(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))));
        detail.push(Line::from(vec![
            Span::styled(if it.enabled { "on" } else { "off" }, Style::default().fg(if it.enabled { t.idle } else { t.muted })),
            Span::styled(format!(" · {}", it.scope), Style::default().fg(t.muted)),
        ]));
        detail.push(Line::raw(""));
        detail.extend(it.detail.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(t.fg)))));
        detail.push(Line::raw(""));
        detail.push(Line::from(Span::styled(format!("defined in {}", it.source.display()), Style::default().fg(t.muted))));
    }
    PanelView {
        title: format!("toolbox · {}", truncate(&v.project.display().to_string(), 50)),
        tabs: Vec::new(),
        tab: 0,
        query: Some(v.query.clone()),
        sel: sel_row,
        rows,
        detail,
        detail_scroll: v.scroll,
        footer: hint(t, &[("Enter", "open its config file"), ("^R", "rescan"), ("Esc", "close")]),
        empty: if v.sections.is_none() { "  reading configs…".into() } else { "  Nothing set up here (or nothing matches).".into() },
        left_pct: 50,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_row_marks_without_shifting() {
        let t = crate::theme::Theme::named("catppuccin-mocha");
        let row = Line::from(vec![Span::raw(" fix-totals"), Span::raw("  ready")]);
        let width = row.width();
        let marked = focus_row(row, &t);
        assert_eq!(marked.width(), width, "selection must not move the text");
        assert_eq!(marked.spans[0].content, "▌");
        assert!(marked.spans.iter().all(|s| s.style.bg == Some(focus_bg(&t))));
    }
}
