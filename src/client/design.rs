//! The "workspaces" layout from the design handoff: a sidebar of workspaces (one per repo)
//! → their worktrees → the agent in each; borderless panes with one-row title bars; an
//! answer bar on panes that need you; a crumb in the top bar and counts in the status line.
//! Plus the hydra-native views that replace the pane area (Changes, Files), the
//! talk-to-a-worktree modal and the + Pane menu.

use super::render::{blend, render_screen, status_icon, truncate};
use super::{App, Hit, Mode};
use crate::protocol::{Status, TermId, TermInfo, WorkspaceInfo, WsId};

use crate::theme::Theme;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

// ---- sidebar model -------------------------------------------------------------------------

/// One row of the sidebar: a pane (each is a full screen of its own), a group the user
/// made, or something under a pane (a subagent it's running, or the worktree it's in).
#[derive(Debug, Clone)]
pub(super) enum SideRow {
    Group {
        name: String,
        /// Agents by state: needs, working, done, idle
        counts: [usize; 4],
        open: bool,
    },
    Pane {
        ws: WsId,
        /// Its name if renamed, else where it is.
        name: String,
        /// The agent in it, or empty.
        agent: String,
        status: Status,
        color: Color,
        question: Option<String>,
        /// The pane on screen.
        primary: bool,
        /// Inside a group (indented under it).
        grouped: bool,
        is_new: bool,
    },
    Detail {
        ws: WsId,
        text: String,
        worktree: bool,
        grouped: bool,
    },
}

/// Case- and separator-insensitive form of a path, for comparisons.
pub(super) fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

pub(super) fn inside(child: &Path, parent: &Path) -> bool {
    let (c, p) = (path_key(child), path_key(parent));
    c == p || c.starts_with(&format!("{p}\\"))
}

/// The question an agent is asking, read off the bottom of its screen.
pub(super) fn question(parser: &vt100::Parser) -> Option<String> {
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let lines: Vec<String> = screen.rows(0, cols).collect();
    let tidy = |l: &str| l.trim().trim_matches(|c: char| "│╭╮╰╯─┃ ".contains(c)).trim().to_string();
    let start = lines.len().saturating_sub(24.min(rows as usize));
    lines[start..]
        .iter()
        .rev()
        .map(|l| tidy(l))
        .filter(|l| !l.is_empty())
        .filter(|l| !l.starts_with('❯') && !l.starts_with('›') && !l.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .find(|l| l.ends_with('?') || l.starts_with("Do you want") || l.starts_with("Allow"))
}

impl App {
    /// A group's key in the collapsed set.
    pub(super) fn group_key(name: &str) -> String {
        format!("group:{name}")
    }

    /// Terminals of a pane (all its tabs and splits).
    fn ws_terms<'a>(&'a self, w: &'a WorkspaceInfo) -> impl Iterator<Item = &'a TermInfo> + 'a {
        w.tabs.iter().flat_map(|t| t.layout.leaves()).filter_map(|id| self.snap.terms.get(&id))
    }

    /// A pane's name: what the user called it, else where its focused terminal is.
    pub(super) fn pane_title(&self, w: &WorkspaceInfo) -> String {
        if !w.name.is_empty() {
            return w.name.clone();
        }
        let focus = w.tab().map(|t| t.focus).and_then(|id| self.snap.terms.get(&id));
        tilde(&focus.map(|t| t.cwd.clone()).unwrap_or_else(|| w.cwd.clone()))
    }

    fn pane_rows(&self, w: &WorkspaceInfo, grouped: bool, rows: &mut Vec<SideRow>) {
        let terms: Vec<&TermInfo> = self.ws_terms(w).collect();
        let mut agents: Vec<&TermInfo> = terms.iter().copied().filter(|t| t.agent.is_some()).collect();
        agents.sort_by_key(|t| (t.status.urgency(), t.id));
        let lead = agents.first().copied();
        let status = lead.map(|t| t.status).unwrap_or(Status::None);
        let agent = match agents.len() {
            0 => String::new(),
            1 => lead.and_then(|t| t.agent.clone()).unwrap_or_default(),
            n => format!("{} +{}", lead.and_then(|t| t.agent.clone()).unwrap_or_default(), n - 1),
        };
        let question = lead.filter(|t| t.status == Status::Blocked).and_then(|t| self.parsers.get(&t.id)).and_then(question);
        rows.push(SideRow::Pane {
            ws: w.id,
            name: self.pane_title(w),
            agent,
            status,
            color: self.ws_color(w),
            question,
            primary: Some(w.id) == self.snap.active_ws,
            grouped,
            is_new: w.is_new,
        });
        for t in &terms {
            for sa in &t.subagents {
                rows.push(SideRow::Detail { ws: w.id, text: format!("↳ {sa}"), worktree: false, grouped });
            }
        }
        // The worktree the focused terminal is in.
        let focus = w.tab().map(|t| t.focus).and_then(|id| self.snap.terms.get(&id));
        if let Some(t) = focus.filter(|t| t.linked)
            && let Some(b) = &t.branch
        {
            rows.push(SideRow::Detail { ws: w.id, text: format!("⑂ worktree {b}"), worktree: true, grouped });
        }
    }

    pub(super) fn side_rows(&self) -> Vec<SideRow> {
        let mut rows = Vec::new();
        let mut done_groups: Vec<String> = Vec::new();
        for w in &self.snap.workspaces {
            match &w.group {
                None => self.pane_rows(w, false, &mut rows),
                Some(g) if !done_groups.contains(g) => {
                    done_groups.push(g.clone());
                    let members: Vec<&WorkspaceInfo> =
                        self.snap.workspaces.iter().filter(|m| m.group.as_deref() == Some(g.as_str())).collect();
                    let mut counts = [0usize; 4];
                    for t in members.iter().flat_map(|m| self.ws_terms(m)).filter(|t| t.agent.is_some()) {
                        counts[match t.status {
                            Status::Blocked => 0,
                            Status::Working => 1,
                            Status::Done => 2,
                            _ => 3,
                        }] += 1;
                    }
                    let open = !self.collapsed.contains(&Self::group_key(g));
                    rows.push(SideRow::Group { name: g.clone(), counts, open });
                    if open {
                        for m in members {
                            self.pane_rows(m, true, &mut rows);
                        }
                    }
                }
                Some(_) => {}
            }
        }
        rows
    }

    /// Where a pane is, for its title bar: the worktree's name, else its folder.
    pub(super) fn pane_location(&self, t: &TermInfo) -> String {
        // Shells show where they are; agents show the worktree they work in.
        if t.agent.is_none() {
            return tilde(&t.cwd);
        }
        if let Some((w, _)) = self.snap.locate(t.id)
            && let Some(g) = &w.git
            && let Some(e) = g.worktrees.iter().filter(|e| inside(&t.cwd, &e.path)).max_by_key(|e| path_key(&e.path).len()) {
                return e.branch.clone();
            }
        tilde(&t.cwd)
    }

    /// The crumb: group › pane › what's focused in it (or the open view).
    fn crumb(&self) -> Option<(Color, String, String, String)> {
        let w = self.active_ws()?;
        let focused = self.focused().and_then(|id| self.snap.terms.get(&id));
        let last = match &self.view {
            Some(super::View::Changes(_)) => "changes".to_string(),
            Some(super::View::Files(_)) => "files".to_string(),
            Some(super::View::Settings(_)) => "settings".to_string(),
            Some(super::View::Pr(_)) => "pull request".to_string(),
            Some(super::View::Map(_)) => "map".to_string(),
            None => focused.map(pane_name).unwrap_or_default(),
        };
        let first = w.group.clone().unwrap_or_default();
        Some((self.ws_color(w), first, self.pane_title(w), last))
    }
}

/// `~\code\shop-api` style path.
pub(super) fn tilde(p: &Path) -> String {
    // "C:\dev\" and "C:\dev" are the same place; show it one way (but keep "C:\").
    let text = p.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    let p = if trimmed.len() > 2 && trimmed.len() < text.len() { Path::new(trimmed) } else { p };
    if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
        && let Ok(rel) = p.strip_prefix(&home)
    {
        let sep = std::path::MAIN_SEPARATOR;
        return if rel.as_os_str().is_empty() { "~".into() } else { format!("~{sep}{}", rel.display()) };
    }
    p.display().to_string()
}

/// A pane's name in its title bar: the agent, or what runs in the shell.
pub(super) fn pane_name(t: &TermInfo) -> String {
    match &t.agent {
        Some(a) => a.clone(),
        None if t.is_shell() => "shell".into(),
        None => t.display_name(),
    }
}

// ---- drawing primitives ----------------------------------------------------------------------

/// A styled run of text.
pub(super) type Seg = (String, Style);

pub(super) fn seg(t: impl Into<String>, s: Style) -> Seg {
    (t.into(), s)
}

pub(super) fn segs_width(s: &[Seg]) -> u16 {
    s.iter().map(|(t, _)| t.width() as u16).sum()
}

/// Write segments from (x, y), not past `max_x`; returns the x after the last cell written.
pub(super) fn put(buf: &mut Buffer, x: u16, y: u16, segs: &[Seg], max_x: u16) -> u16 {
    let mut x = x;
    for (text, style) in segs {
        if x >= max_x {
            break;
        }
        let (nx, _) = buf.set_stringn(x, y, text, (max_x - x) as usize, *style);
        x = nx;
    }
    x
}

pub(super) fn fill(buf: &mut Buffer, r: Rect, bg: Color) {
    buf.set_style(r, Style::reset().bg(bg));
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            buf[(x, y)].set_symbol(" ");
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum BtnKind {
    Primary,
    Normal,
    Ghost,
}

/// The design's button: " Label " + "key " on a filled ground. Hovered buttons use `hov`.
pub(super) fn button(t: &Theme, label: &str, key: &str, kind: BtnKind, hovered: bool) -> Vec<Seg> {
    let (bg, fg, kfg) = match kind {
        BtnKind::Primary => (t.accent, t.acc_ink, t.acc_ink),
        BtnKind::Normal => (t.btn, t.strong, t.accent),
        BtnKind::Ghost => (t.sidebar_bg, t.text, t.accent),
    };
    let bg = if hovered && kind != BtnKind::Primary { t.hov } else { bg };
    let bold = if kind == BtnKind::Ghost { Modifier::empty() } else { Modifier::BOLD };
    let mut v = vec![seg(format!(" {label} "), Style::default().bg(bg).fg(fg).add_modifier(bold))];
    if !key.is_empty() {
        v.push(seg(format!("{key} "), Style::default().bg(bg).fg(kfg).add_modifier(Modifier::BOLD)));
    }
    v
}

/// Draw a button at (x, y), record its hit area, return the x after it.
#[allow(clippy::too_many_arguments)]
pub(super) fn put_button(app: &mut App, buf: &mut Buffer, x: u16, y: u16, label: &str, key: &str, kind: BtnKind, hit: Hit, max_x: u16) -> u16 {
    let t = app.theme.clone();
    let w = segs_width(&button(&t, label, key, kind, false));
    let r = Rect { x, y, width: w.min(max_x.saturating_sub(x)), height: 1 };
    let hovered = app.hover.is_some_and(|p| r.contains(p));
    let nx = put(buf, x, y, &button(&t, label, key, kind, hovered), max_x);
    app.hits.push((r, hit));
    nx
}

/// A key drawn as a little keycap: the key on a raised ground.
pub(super) fn keycap(t: &Theme, key: &str) -> Seg {
    seg(format!(" {key} "), Style::default().bg(t.btn).fg(t.strong).add_modifier(Modifier::BOLD))
}

/// Keys as keycaps. Keys separated by two spaces are different sets; a set of four or more
/// single keys shares one cap (` h j k l `), otherwise every key gets its own.
pub(super) fn keycaps(t: &Theme, keys: &str, gap_bg: Color) -> Vec<Seg> {
    let mut out = Vec::new();
    for (gi, group) in keys.split("  ").filter(|g| !g.is_empty()).enumerate() {
        let ks: Vec<&str> = group.split(' ').filter(|k| !k.is_empty()).collect();
        let caps: Vec<String> = if ks.len() >= 4 && ks.iter().all(|k| k.chars().count() == 1) {
            vec![ks.join(" ")]
        } else {
            ks.iter().map(|k| k.to_string()).collect()
        };
        for (ci, c) in caps.iter().enumerate() {
            if gi > 0 || ci > 0 {
                out.push(seg(" ", Style::default().bg(gap_bg)));
            }
            out.push(keycap(t, c));
        }
    }
    out
}

/// "Enter open   Tab switch" style hints, with keycaps.
pub(super) fn cap_hints(t: &Theme, bg: Color, pairs: &[(&str, &str)]) -> Vec<Seg> {
    let mut out = Vec::new();
    for (i, (k, what)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(seg("   ", Style::default().bg(bg)));
        }
        out.push(keycap(t, k));
        out.push(seg(format!(" {what}"), Style::default().bg(bg).fg(t.muted)));
    }
    out
}

/// The key bound to an action after the leader, for button labels.
pub(super) fn key_of(app: &App, a: &crate::keys::Action) -> String {
    app.keymap
        .prefixed_order
        .iter()
        .find(|k| app.keymap.prefixed.get(k) == Some(a))
        .map(|k| k.to_string())
        .unwrap_or_default()
}

pub(super) fn glyph(app: &App, s: Status) -> String {
    match s {
        Status::None => app.cfg.icons.idle.clone(),
        s => status_icon(app, s),
    }
}

pub(super) fn state_label(s: Status) -> &'static str {
    match s {
        Status::Blocked => "needs you",
        Status::Working => "working",
        Status::Done => "done",
        _ => "idle",
    }
}

// ---- the layout ------------------------------------------------------------------------------

/// Sidebar width by terminal width: 26 below 140 columns, 34 below 200, else 38.
fn side_width(app: &App, w: u16) -> u16 {
    if !app.sidebar {
        return 0;
    }
    match w {
        0..140 => 26,
        140..200 => 34,
        _ => 38,
    }
}

/// Draw the whole layout; returns the pane area (for overlays).
pub(super) fn draw(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) -> Rect {
    let buf = f.buffer_mut();
    fill(buf, area, t.bg);
    let sw = side_width(app, area.width);
    let top = Rect { height: 1, ..area };
    let status = Rect { y: area.bottom().saturating_sub(1), height: 1, ..area };
    let side = Rect { y: area.y + 1, height: area.height.saturating_sub(2), width: sw, ..area };
    let px = if sw > 0 { area.x + sw + 1 } else { area.x };
    let tabs = Rect { x: px, y: area.y + 1, width: area.right().saturating_sub(px), height: 1 };
    let panes = Rect { x: px, y: area.y + 2, width: area.right().saturating_sub(px), height: area.height.saturating_sub(3) };

    draw_top(app, f.buffer_mut(), top, sw, t);
    if sw > 0 {
        draw_side(app, f.buffer_mut(), side, t);
    }
    draw_tab_strip(app, f.buffer_mut(), tabs, t);
    match app.view.take() {
        Some(super::View::Changes(v)) => {
            draw_changes(app, f.buffer_mut(), panes, t, &v);
            app.view = Some(super::View::Changes(v));
        }
        Some(super::View::Files(v)) => {
            draw_files(app, f.buffer_mut(), panes, t, &v);
            app.view = Some(super::View::Files(v));
        }
        Some(super::View::Settings(v)) => {
            draw_settings_view(app, f.buffer_mut(), panes, t, &v);
            app.view = Some(super::View::Settings(v));
        }
        Some(super::View::Pr(v)) => {
            super::hydra::draw_pr(app, f.buffer_mut(), panes, t, &v);
            app.view = Some(super::View::Pr(v));
        }
        Some(super::View::Map(v)) => {
            super::hydra::draw_map(app, f.buffer_mut(), panes, t, &v);
            app.view = Some(super::View::Map(v));
        }
        None => draw_panes(app, f, panes, t),
    }
    draw_status(app, f.buffer_mut(), status, t);
    panes
}

fn draw_top(app: &mut App, buf: &mut Buffer, r: Rect, sw: u16, t: &Theme) {
    fill(buf, r, t.bg);
    put(buf, r.x + 1, r.y, &[seg(">_ hydra", Style::default().fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
    let key = key_of(app, &crate::keys::Action::NewPane);
    let bw = segs_width(&button(t, "+ Pane", &key, BtnKind::Primary, false));
    let bx = r.right().saturating_sub(bw);
    if let Some((color, ws, wt, last)) = app.crumb() {
        let mut c = vec![seg("▌", Style::default().fg(color))];
        if !ws.is_empty() {
            c.push(seg(ws, Style::default().fg(t.text)));
            c.push(seg("  ›  ", Style::default().fg(t.muted)));
        }
        c.push(seg(wt, Style::default().fg(t.strong).add_modifier(Modifier::BOLD)));
        if r.width >= 140 && !last.is_empty() {
            c.push(seg("  ›  ", Style::default().fg(t.muted)));
            c.push(seg(last, Style::default().fg(t.text)));
        }
        let cx = if sw > 0 { r.x + sw + 1 } else { r.x + 12 };
        put(buf, cx, r.y, &c, bx.saturating_sub(2));
    }
    put_button(app, buf, bx, r.y, "+ Pane", &key, BtnKind::Primary, Hit::AddPane, r.right());
}

fn draw_side(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme) {
    fill(buf, r, t.sidebar_bg);
    let base = Style::default().bg(t.sidebar_bg);
    let rows = app.side_rows();
    let compact = r.width < 28;
    let cursor = match app.mode {
        Mode::Tree { sel } => Some(sel),
        _ => None,
    };
    let bottom_limit = r.bottom().saturating_sub(4);
    let mut y = r.y + 1;
    for (i, row) in rows.iter().enumerate() {
        if y >= bottom_limit {
            break;
        }
        match row {
            SideRow::Group { name, counts, open } => {
                if i > 0 {
                    y += 1;
                    if y >= bottom_limit {
                        break;
                    }
                }
                let row_r = Rect { x: r.x, y, width: r.width, height: 1 };
                let hovered = app.hover.is_some_and(|p| row_r.contains(p)) || cursor == Some(i);
                let bg = if hovered { t.hov } else { t.sidebar_bg };
                if hovered {
                    fill(buf, row_r, bg);
                }
                let s = Style::default().bg(bg);
                put(
                    buf,
                    r.x + 1,
                    y,
                    &[
                        seg(if *open { "▾ " } else { "▸ " }, s.fg(t.muted)),
                        seg(truncate(name, r.width.saturating_sub(14) as usize), s.fg(t.strong).add_modifier(Modifier::BOLD)),
                    ],
                    r.right(),
                );
                let mut c: Vec<Seg> = Vec::new();
                for (k, st) in [Status::Blocked, Status::Working, Status::Done, Status::Idle].iter().enumerate() {
                    if counts[k] > 0 {
                        let mut style = s.fg(t.status(*st));
                        if k == 0 {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        c.push(seg(format!("{}{} ", glyph(app, *st), counts[k]), style));
                    }
                }
                let cw = segs_width(&c);
                put(buf, r.right().saturating_sub(cw + 1), y, &c, r.right());
                app.hits.push((row_r, Hit::SideRow(i)));
                y += 1;
            }
            SideRow::Pane { name, agent, status, color, question, primary, grouped, is_new, .. } => {
                let ask = *status == Status::Blocked && !compact && question.is_some();
                let h = if ask { 2 } else { 1 };
                let row_r = Rect { x: r.x, y, width: r.width, height: h.min(bottom_limit - y) };
                let hovered = app.hover.is_some_and(|p| row_r.contains(p)) || cursor == Some(i);
                let (bg, ink) = if *primary {
                    (t.accent, Some(t.acc_ink))
                } else if hovered {
                    (t.hov, None)
                } else {
                    (t.sidebar_bg, None)
                };
                if *primary || hovered {
                    fill(buf, row_r, bg);
                }
                let s = Style::default().bg(bg);
                let fg = |c: Color| s.fg(ink.unwrap_or(c));
                let x0 = r.x + if *grouped { 3 } else { 1 };
                let has_agent = !agent.is_empty();
                let icon = if has_agent { glyph(app, *status) } else { app.cfg.icons.shell.clone() };
                let mut st = fg(if has_agent { t.status(*status) } else { t.muted });
                if *status == Status::Blocked {
                    st = st.add_modifier(Modifier::BOLD);
                }
                let mut name_style = fg(t.strong);
                if *primary {
                    name_style = name_style.add_modifier(Modifier::BOLD);
                }
                // Right side: the agent; the T chip to talk to it when hovered.
                let right: Vec<Seg> = if hovered && !*primary && has_agent {
                    vec![seg(" T ", Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)), seg(format!(" {agent}"), s.fg(t.text))]
                } else if has_agent {
                    vec![seg(agent.clone(), fg(t.muted))]
                } else {
                    Vec::new()
                };
                let rw = segs_width(&right);
                let mut left = vec![seg("▌", fg(*color)), seg(format!("{icon} "), st)];
                let room = r.right().saturating_sub(x0 + 4 + rw + 2) as usize;
                left.push(seg(truncate(name, room), name_style));
                if *is_new {
                    left.push(seg(" NEW", fg(t.accent).add_modifier(Modifier::BOLD)));
                }
                put(buf, x0, y, &left, r.right().saturating_sub(rw + 1));
                if !right.is_empty() {
                    let rx = r.right().saturating_sub(rw + 1);
                    put(buf, rx, y, &right, r.right());
                    if hovered && !*primary && has_agent {
                        app.hits.push((Rect { x: rx, y, width: 3, height: 1 }, Hit::SideTalk(i)));
                    }
                }
                if ask && y + 1 < bottom_limit {
                    let q = truncate(question.as_deref().unwrap_or(""), r.width.saturating_sub(x0 - r.x + 5) as usize);
                    put(buf, x0 + 3, y + 1, &[seg(q, fg(t.blocked))], r.right());
                }
                app.hits.push((row_r, Hit::SideRow(i)));
                y += h;
            }
            SideRow::Detail { text, worktree, grouped, .. } => {
                let row_r = Rect { x: r.x, y, width: r.width, height: 1 };
                let x0 = r.x + if *grouped { 6 } else { 4 };
                let color = if *worktree { t.muted } else { t.working };
                put(buf, x0, y, &[seg(truncate(text, r.right().saturating_sub(x0 + 1) as usize), base.fg(color))], r.right());
                app.hits.push((row_r, Hit::SideRow(i)));
                y += 1;
            }
        }
    }
    // + Pane right under the list, so starting something is always one click away.
    if y + 1 < bottom_limit {
        let key = key_of(app, &crate::keys::Action::NewPane);
        put_button(app, buf, r.x + 1, y + 1, "+ Pane", &key, BtnKind::Ghost, Hit::AddPane, r.right());
    }
    let rule_y = r.bottom().saturating_sub(3);
    put(buf, r.x + 1, rule_y, &[seg("─".repeat(r.width.saturating_sub(2) as usize), base.fg(t.line))], r.right());
    let key = key_of(app, &crate::keys::Action::Settings);
    put_button(app, buf, r.x + 2, rule_y + 1, "Settings", &key, BtnKind::Ghost, Hit::Button(super::Btn::Settings), r.right());
}

/// Tabs along the top of the pane: click to switch, + for a new one.
fn draw_tab_strip(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme) {
    fill(buf, r, t.bg);
    let Some(w) = app.active_ws().cloned() else { return };
    let mut x = r.x;
    for (i, tab) in w.tabs.iter().enumerate() {
        let active = tab.id == w.active_tab;
        let label = if !tab.name.is_empty() {
            tab.name.clone()
        } else {
            app.snap.terms.get(&tab.focus).map(pane_name).unwrap_or_else(|| "shell".into())
        };
        let status = tab.layout.leaves().iter().filter_map(|id| app.snap.terms.get(id)).filter(|x| x.agent.is_some()).map(|x| x.status).min_by_key(|s| s.urgency());
        let mut segs = vec![seg(format!(" {} ", i + 1), Style::default())];
        if let Some(st) = status {
            segs.push(seg(format!("{} ", glyph(app, st)), Style::default().fg(t.status(st))));
        }
        segs.push(seg(format!("{} ", truncate(&label, 20)), Style::default()));
        let wdt = segs_width(&segs);
        if x + wdt + 4 > r.right() {
            break;
        }
        let rr = Rect { x, y: r.y, width: wdt, height: 1 };
        let hovered = app.hover.is_some_and(|p| rr.contains(p));
        let (bg, fg) = if active {
            (t.card2, t.strong)
        } else if hovered {
            (t.hov, t.text)
        } else {
            (t.bg, t.muted)
        };
        let segs: Vec<Seg> = segs
            .into_iter()
            .map(|(s, st)| {
                let mut st = st.bg(bg);
                if st.fg.is_none() {
                    st = st.fg(fg);
                }
                if active {
                    st = st.add_modifier(Modifier::BOLD);
                }
                (s, st)
            })
            .collect();
        put(buf, x, r.y, &segs, r.right());
        if active {
            // A lime underline marks the tab you're in.
            for cx in x..x + wdt {
                buf[(cx, r.y)].set_style(Style::default().add_modifier(Modifier::UNDERLINED).underline_color(t.accent));
            }
        }
        app.hits.push((rr, Hit::Tab(w.id, tab.id)));
        x += wdt + 1;
    }
    let rr = Rect { x, y: r.y, width: 3, height: 1 };
    let hovered = app.hover.is_some_and(|p| rr.contains(p));
    put(buf, x, r.y, &[seg(" + ", Style::default().fg(if hovered { t.accent } else { t.muted }).bg(if hovered { t.hov } else { t.bg }))], r.right());
    app.hits.push((rr, Hit::NewTab));
}

/// A title bar: name + location on the left, the given segments on the right.
pub(super) fn title_bar(buf: &mut Buffer, r: Rect, t: &Theme, left: &[Seg], right: &[Seg], focus: bool) {
    let bg = if focus { t.accent } else { t.sidebar_bg };
    fill(buf, Rect { height: 1, ..r }, bg);
    let ink = |segs: &[Seg]| -> Vec<Seg> {
        segs.iter()
            .map(|(s, st)| {
                let mut st = st.bg(bg);
                if focus {
                    st = st.fg(t.acc_ink);
                }
                (s.clone(), st)
            })
            .collect()
    };
    let rw = segs_width(right);
    put(buf, r.x + 1, r.y, &ink(left), r.right().saturating_sub(rw + 2));
    put(buf, r.right().saturating_sub(rw), r.y, &ink(right), r.right());
}

fn draw_panes(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) {
    let Some(tab) = app.active_tab().cloned() else {
        let buf = f.buffer_mut();
        put(buf, area.x + 2, area.y + 1, &[seg("starting…", Style::default().fg(t.muted))], area.right());
        return;
    };
    let zoomed = app.is_zoomed();
    let rects = if zoomed { vec![(tab.focus, area)] } else { tab.layout.rects(area) };
    let multi = tab.layout.leaves().len() > 1;
    for (term, r) in rects {
        let Some(info) = app.snap.terms.get(&term).cloned() else { continue };
        let focused = term == tab.focus;
        // Columns are separated by one │; the left pane gives up its last column for it.
        let w = if r.right() < area.right() { r.width.saturating_sub(1) } else { r.width };
        if r.right() < area.right() {
            let buf = f.buffer_mut();
            for y in r.top()..r.bottom() {
                buf[(r.right() - 1, y)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
            }
        }
        let pr = Rect { width: w, ..r };
        let st = info.status;
        let mut right: Vec<Seg> = Vec::new();
        if info.agent.is_some() {
            let mut s = Style::default().fg(t.status(st));
            if st == Status::Blocked {
                s = s.add_modifier(Modifier::BOLD);
            }
            right.push(seg(format!("{} {}  ", glyph(app, st), state_label(st)), s));
        }
        if let Some(n) = app.scroll.get(&term) {
            right.push(seg(format!("↑{n}  "), Style::default().fg(t.accent)));
        }
        right.push(seg("⤢ ", Style::default().fg(t.muted)));
        right.push(seg("✕ ", Style::default().fg(t.muted)));
        let left = vec![
            seg(format!("{}  ", pane_name(&info)), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
            seg(app.pane_location(&info), Style::default().fg(t.muted)),
        ];
        title_bar(f.buffer_mut(), pr, t, &left, &right, focused && (multi || zoomed || true));
        // Zoom and close are buttons at the right end of the title bar.
        let close_x = pr.right().saturating_sub(2);
        let zoom_x = pr.right().saturating_sub(4);
        app.hits.push((Rect { x: close_x, y: pr.y, width: 2, height: 1 }, Hit::Button(super::Btn::Close(term))));
        app.hits.push((Rect { x: zoom_x, y: pr.y, width: 2, height: 1 }, Hit::Button(super::Btn::Zoom(term))));

        // Content: real terminal output, starting two columns in.
        let inner = Rect {
            x: pr.x + 2,
            y: pr.y + 1,
            width: pr.width.saturating_sub(3),
            height: pr.height.saturating_sub(1),
        };
        app.pane_frames.push((term, r));
        app.panes.push((term, inner));
        let copying = matches!(&app.mode, Mode::Copy(c) if c.term == term);
        if copying {
            if let Mode::Copy(c) = &mut app.mode {
                c.height = inner.height as usize;
                c.width = inner.width as usize;
                super::render::render_copy(c, inner, f.buffer_mut(), t);
            }
        } else if let Some(p) = app.parsers.get(&term) {
            let screen = p.screen();
            render_screen(screen, inner, f.buffer_mut(), t.bg);
            if focused
                && !screen.hide_cursor()
                && !app.scroll.contains_key(&term)
                && matches!(app.mode, Mode::Normal | Mode::Prefix { .. })
                && app.view.is_none()
            {
                let (row, col) = screen.cursor_position();
                if row < inner.height && col < inner.width {
                    f.set_cursor_position(Position::new(inner.x + col, inner.y + row));
                }
            }
        }
        // Answer bar on a pane whose agent needs you: the agent's own numbered choices.
        if st == Status::Blocked && info.agent.is_some() && inner.height > 2 {
            let y = pr.bottom() - 1;
            let buf = f.buffer_mut();
            fill(buf, Rect { x: pr.x, y, width: pr.width, height: 1 }, t.card2);
            let x = put(
                buf,
                pr.x + 2,
                y,
                &[seg(format!("{} answer  ", app.cfg.icons.blocked), Style::default().bg(t.card2).fg(t.blocked).add_modifier(Modifier::BOLD))],
                pr.right(),
            );
            let mut x = put_button(app, f.buffer_mut(), x, y, "Yes", "1", BtnKind::Primary, Hit::Button(super::Btn::Answer(term, '1')), pr.right());
            for (label, key) in [("Always", '2'), ("No", '3')] {
                x += 1;
                x = put_button(app, f.buffer_mut(), x, y, label, &key.to_string(), BtnKind::Normal, Hit::Button(super::Btn::Answer(term, key)), pr.right());
            }
            x += 1;
            put_button(app, f.buffer_mut(), x, y, "Reply", "r", BtnKind::Normal, Hit::Button(super::Btn::Reply(term)), pr.right());
        }
    }
}

fn draw_status(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme) {
    fill(buf, r, t.sidebar_bg);
    let s = Style::default().bg(t.sidebar_bg);
    let prefix = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let right_w = segs_width(&button(t, "Keys", "?", BtnKind::Normal, false)) + 1 + segs_width(&button(t, &prefix, "", BtnKind::Primary, false));
    let max = r.right().saturating_sub(right_w + 2);
    let left: Vec<Seg> = if let Some((msg, _, err)) = &app.notice {
        let mut v = vec![
            seg(if *err { "✕ " } else { "✓ " }, s.fg(if *err { t.err } else { t.done }).add_modifier(Modifier::BOLD)),
            seg(msg.clone(), s.fg(t.strong)),
        ];
        if app.undo_hint {
            v.push(seg("   undo ", s.fg(t.muted)));
            v.push(seg("u", s.fg(t.accent).add_modifier(Modifier::BOLD)));
        }
        v
    } else {
        let count = |st: Status| app.snap.terms.values().filter(|x| x.agent.is_some() && x.status == st).count();
        let (needs, work, done) = (count(Status::Blocked), count(Status::Working), count(Status::Done));
        let groups = app.snap.workspaces.len();
        let mut v = Vec::new();
        if needs > 0 {
            v.push(seg(format!("{} {needs} need you", app.cfg.icons.blocked), s.fg(t.blocked).add_modifier(Modifier::BOLD)));
            v.push(seg("   ", s));
        }
        if work > 0 {
            v.push(seg(format!("{} {work} working", glyph(app, Status::Working)), s.fg(t.text)));
            v.push(seg("   ", s));
        }
        if done > 0 {
            v.push(seg(format!("{} {done} done", app.cfg.icons.done), s.fg(t.done)));
            v.push(seg("   ", s));
        }
        let idle = count(Status::Idle);
        if idle > 0 {
            v.push(seg(format!("{} {idle} idle", app.cfg.icons.idle), s.fg(t.muted)));
            v.push(seg("   ", s));
        }
        if v.is_empty() {
            v.push(seg("no agents running", s.fg(t.muted)));
            v.push(seg("   ", s));
        }
        v.push(seg(format!("{groups} pane{}", if groups == 1 { "" } else { "s" }), s.fg(t.muted)));
        v
    };
    put(buf, r.x + 1, r.y, &left, max);
    let x = r.right().saturating_sub(right_w);
    let x = put_button(app, buf, x, r.y, "Keys", "?", BtnKind::Normal, Hit::Button(super::Btn::Keys), r.right());
    put_button(app, buf, x + 1, r.y, &prefix, "", BtnKind::Primary, Hit::Button(super::Btn::Leader), r.right());
}

/// Fade everything already drawn (behind a modal).
pub(super) fn dim(buf: &mut Buffer, area: Rect, t: &Theme) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let c = &mut buf[(x, y)];
            let bg = if c.bg == Color::Reset { t.bg } else { c.bg };
            let fg = if c.fg == Color::Reset { t.fg } else { c.fg };
            c.fg = blend(blend(fg, bg, 0.6), Color::Rgb(0, 0, 0), 0.45);
            c.bg = blend(bg, Color::Rgb(0, 0, 0), 0.45);
        }
    }
}

// ---- views that replace the pane area ------------------------------------------------------

/// The bottom action row of a view.
fn action_row(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme, buttons: &[(&str, &str, BtnKind, Hit)], tail: &[Seg]) {
    let y = r.bottom().saturating_sub(1);
    fill(buf, Rect { y, height: 1, ..r }, t.card2);
    // Keycap first: in a tool window you just press the key (no Ctrl+Space).
    let mut x = r.x + 2;
    for (label, key, kind, hit) in buttons {
        let cap = format!(" {key} ");
        let text = format!(" {label} ");
        let w = (cap.width() + text.width()) as u16;
        let br = Rect { x, y, width: w.min(r.right().saturating_sub(x)), height: 1 };
        let hov = app.hover.is_some_and(|p| br.contains(p));
        let cap_st = if *kind == BtnKind::Primary {
            Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)
        };
        let text_st = Style::default().bg(if hov { t.hov } else { t.card2 }).fg(t.strong);
        x = put(buf, x, y, &[seg(cap, cap_st), seg(text, text_st)], r.right()) + 2;
        app.hits.push((br, *hit));
    }
    let tail: Vec<Seg> = tail.iter().map(|(s, st)| (s.clone(), st.bg(t.card2))).collect();
    put(buf, x + 2, y, &tail, r.right());
}

pub(super) fn diff_line(t: &Theme, l: &str, old: &mut u32, new: &mut u32) -> Vec<Seg> {
    let (red, green, cyan, gray) = (t.err, t.done, Color::Rgb(0x3d, 0xd6, 0xc0), t.muted);
    if let Some(rest) = l.strip_prefix("@@") {
        // @@ -a,b +c,d @@ context
        let nums: Vec<u32> = rest
            .split_whitespace()
            .take(2)
            .filter_map(|p| p.trim_start_matches(['-', '+']).split(',').next()?.parse().ok())
            .collect();
        if nums.len() == 2 {
            *old = nums[0];
            *new = nums[1];
        }
        return vec![seg(l.to_string(), Style::default().fg(cyan))];
    }
    if let Some(body) = l.strip_prefix('+') {
        *new += 1;
        return vec![seg("      + ", Style::default().fg(green)), seg(body.to_string(), Style::default().fg(green))];
    }
    if let Some(body) = l.strip_prefix('-') {
        let n = *old;
        *old += 1;
        return vec![seg(format!("{n:>4}  - "), Style::default().fg(red)), seg(body.to_string(), Style::default().fg(red))];
    }
    let body = l.strip_prefix(' ').unwrap_or(l);
    let n = *new;
    *old += 1;
    *new += 1;
    vec![seg(format!("{n:>4}    "), Style::default().fg(gray)), seg(body.to_string(), Style::default().fg(t.fg))]
}

pub(super) fn draw_changes(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &super::views::ChangesView) {
    use super::views::ChangesRow;
    fill(buf, area, t.bg);
    let (green, red, yellow) = (t.done, t.err, Color::Rgb(0xe8, 0xc5, 0x65));
    let mut left: Vec<Seg> = vec![seg("changes  ", Style::default().fg(t.strong).add_modifier(Modifier::BOLD))];
    let mut right: Vec<Seg> = Vec::new();
    if let Some(r) = &v.review {
        let added: i64 = r.files.iter().map(|f| f.added).sum();
        let removed: i64 = r.files.iter().map(|f| f.removed).sum();
        let agent = if v.agent.is_empty() { String::new() } else { format!("{} · ", v.agent) };
        left.push(seg(format!("{} · {agent}", r.task.branch), Style::default().fg(t.muted)));
        left.push(seg(format!("+{added}"), Style::default().fg(green)));
        left.push(seg(format!(" −{removed}"), Style::default().fg(red)));
        left.push(seg(format!(" · {} files", r.files.len()), Style::default().fg(t.muted)));
        if !v.reviewed.is_empty() {
            let all = v.reviewed.len() == r.files.len();
            left.push(seg(format!(" · {} of {} reviewed", v.reviewed.len(), r.files.len()), Style::default().fg(if all { t.done } else { t.muted })));
        }
    }
    if let Some(c) = &v.checks {
        right.push(seg(format!("{c}  "), Style::default().fg(if c.starts_with('✓') { t.done } else { t.err })));
    }
    right.push(seg("✕ ", Style::default().fg(t.muted)));
    title_bar(buf, area, t, &left, &right, true);
    app.hits.push((Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, Hit::Button(super::Btn::CloseView)));

    let body = Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area };
    let Some(r) = &v.review else {
        let msg = v.error.clone().unwrap_or_else(|| "reading changes…".into());
        put(buf, body.x + 2, body.y + 1, &[seg(msg, Style::default().fg(t.muted))], body.right());
        return;
    };
    let fw: u16 = 34.min(body.width / 2);
    // File tree
    let rows = v.rows();
    let mut y = body.y + 1;
    for (i, row) in rows.iter().enumerate() {
        if y >= body.bottom().saturating_sub(8) {
            break;
        }
        let rr = Rect { x: body.x + 1, y, width: fw - 1, height: 1 };
        let sel = matches!(row, ChangesRow::File(fi, _) if *fi == r.sel);
        if sel {
            fill(buf, rr, t.hov);
            put(buf, body.x + 1, y, &[seg(">", Style::default().fg(t.accent).bg(t.hov).add_modifier(Modifier::BOLD))], body.right());
        }
        let bg = if sel { t.hov } else { t.bg };
        let s = Style::default().bg(bg);
        match row {
            ChangesRow::Reviewed(n) => {
                put(buf, body.x + 2, y, &[seg(format!("REVIEWED · {n}"), s.fg(t.muted).add_modifier(Modifier::BOLD))], body.x + fw);
            }
            ChangesRow::Dir(name, depth) => {
                put(buf, body.x + 2, y, &[seg(format!("{}▾ {name}", "  ".repeat(*depth)), s.fg(t.text))], body.x + fw);
            }
            ChangesRow::File(fi, depth) => {
                let f = &r.files[*fi];
                let done = v.reviewed.contains(&f.path);
                let name = Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let name = if done { format!("✓ {name}") } else { name };
                let mut ns = s.fg(if sel { t.strong } else if done { t.muted } else { t.fg });
                if sel {
                    ns = ns.add_modifier(Modifier::BOLD);
                }
                put(buf, body.x + 2, y, &[seg(format!("{}  ", "  ".repeat(*depth)), s.fg(t.muted)), seg(name, ns)], body.x + fw - 10);
                let counts = if f.untracked { format!("+{}", f.added) } else { format!("+{} −{}", f.added, f.removed) };
                let (letter, lc) = if f.untracked { ("A", green) } else { ("M", yellow) };
                let mark = vec![seg(counts, s.fg(t.muted)), seg(format!(" {letter}"), s.fg(lc).add_modifier(Modifier::BOLD))];
                let mw = segs_width(&mark);
                put(buf, body.x + fw - mw - 1, y, &mark, body.x + fw);
            }
        }
        app.hits.push((rr, Hit::Button(super::Btn::Row(i))));
        y += 1;
    }
    if r.files.is_empty() {
        put(buf, body.x + 2, body.y + 1, &[seg("No changes yet.", Style::default().fg(t.muted))], body.x + fw);
    }
    // What the agent said about it.
    if !v.said.is_empty() {
        let mut sy = y + 1;
        let who = if v.agent.is_empty() { "AGENT".to_string() } else { v.agent.to_uppercase() };
        put(buf, body.x + 2, sy, &[seg(format!("{who} SAYS"), Style::default().fg(t.muted).add_modifier(Modifier::BOLD))], body.x + fw);
        sy += 1;
        for line in super::views::wrap(&v.said, (fw - 3) as usize) {
            if sy >= body.bottom() {
                break;
            }
            put(buf, body.x + 2, sy, &[seg(line, Style::default().fg(t.text))], body.x + fw);
            sy += 1;
        }
    }
    for yy in body.top()..body.bottom() {
        buf[(body.x + fw, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
    }
    // Diff of the selected file
    let dx = body.x + fw + 3;
    if let Some(f) = r.files.get(r.sel) {
        let header = vec![
            seg(f.path.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
            seg(if f.untracked { format!("   +{} new", f.added) } else { format!("   +{} −{}", f.added, f.removed) }, Style::default().fg(t.muted)),
        ];
        put(buf, dx, body.y + 1, &header, body.right());
    }
    let (mut old, mut new) = (1u32, 1u32);
    let lines: Vec<Vec<Seg>> = r.diff.iter().map(|l| diff_line(t, l, &mut old, &mut new)).collect();
    for (k, l) in lines.iter().skip(r.scroll as usize).enumerate() {
        let yy = body.y + 3 + k as u16;
        if yy >= body.bottom() {
            break;
        }
        put(buf, dx, yy, l, body.right().saturating_sub(1));
    }
    // Actions
    let who = if v.agent.is_empty() { "agent".to_string() } else { v.agent.clone() };
    let reply = format!("Reply to {who}");
    let base = r.task.base.clone();
    let merge = format!("Merge into {base}");
    if let Some((q, _)) = &v.confirm {
        action_row(
            app,
            buf,
            area,
            t,
            &[("Yes", "y", BtnKind::Primary, Hit::Button(super::Btn::ViewKey('y'))), ("No", "n", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('n')))],
            &[seg(q.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD))],
        );
    } else if v.linked {
        action_row(
            app,
            buf,
            area,
            t,
            &[
                ("Commit", "c", BtnKind::Primary, Hit::Button(super::Btn::ViewKey('c'))),
                (&merge, "m", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('m'))),
                ("Open PR", "p", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('p'))),
                ("PR", "v", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('v'))),
                ("Editor", "e", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('e'))),
                ("Reviewed", "x", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('x'))),
                (&reply, "r", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('r'))),
            ],
            &[seg("d", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), seg(" discard", Style::default().fg(t.err))],
        );
        app.hits.push((Rect { x: area.right().saturating_sub(1), y: area.bottom() - 1, width: 1, height: 1 }, Hit::Button(super::Btn::ViewKey('d'))));
    } else {
        action_row(
            app,
            buf,
            area,
            t,
            &[
                ("Commit", "c", BtnKind::Primary, Hit::Button(super::Btn::ViewKey('c'))),
                ("Open PR", "p", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('p'))),
                ("PR", "v", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('v'))),
                ("Editor", "e", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('e'))),
                ("Reviewed", "x", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('x'))),
                (&reply, "r", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('r'))),
            ],
            &[seg("Esc close", Style::default().fg(t.muted))],
        );
    }
}

pub(super) fn draw_files(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &super::views::FilesTree) {
    fill(buf, area, t.bg);
    let (green, yellow) = (t.done, Color::Rgb(0xe8, 0xc5, 0x65));
    let branch = v.branch.clone().map(|b| format!(" · {b}")).unwrap_or_default();
    let left = vec![
        seg("files  ", Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
        seg(format!("{}{branch}", tilde(&v.root)), Style::default().fg(t.muted)),
    ];
    let filter = if v.filtering || !v.filter.is_empty() { format!("/{}  ", v.filter) } else { "/ filter  ".into() };
    let mode = if v.recent { "Tab project  " } else { "Tab recent  " };
    let right = vec![
        seg(mode, Style::default().fg(t.muted)),
        seg(filter, Style::default().fg(if v.filtering { t.accent } else { t.muted })),
        seg("✕ ", Style::default().fg(t.muted)),
    ];
    title_bar(buf, area, t, &left, &right, true);
    app.hits.push((Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, Hit::Button(super::Btn::CloseView)));

    let body = Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area };
    let fw: u16 = 44.min(body.width / 2);
    let rows = v.visible();
    let list_h = body.height.saturating_sub(4) as usize;
    let start = v.sel.saturating_sub(list_h.saturating_sub(1));
    for (k, (i, e)) in rows.iter().skip(start).take(list_h).enumerate() {
        let y = body.y + 1 + (k as u16);
        let sel = *i == v.sel;
        let rr = Rect { x: body.x + 1, y, width: fw - 1, height: 1 };
        let hovered = app.hover.is_some_and(|p| rr.contains(p));
        let bg = if sel || hovered { t.hov } else { t.bg };
        if sel || hovered {
            fill(buf, rr, bg);
        }
        if sel {
            put(buf, body.x + 1, y, &[seg(">", Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], body.right());
        }
        let s = Style::default().bg(bg);
        let lead = if e.is_dir { if v.expanded.contains(&e.path) { "▾ " } else { "▸ " } } else { "  " };
        let mut ns = s.fg(if e.is_dir { t.text } else if sel { t.strong } else { t.fg });
        if sel {
            ns = ns.add_modifier(Modifier::BOLD);
        }
        put(buf, body.x + 2, y, &[seg(format!("{}{lead}", "  ".repeat(e.depth)), s.fg(t.muted)), seg(e.label.clone(), ns)], body.x + fw - 5);
        let mut marks: Vec<Seg> = Vec::new();
        if v.editing.contains_key(&e.rel) {
            marks.push(seg(app.cfg.icons.blocked.clone(), s.fg(t.blocked)));
        }
        if let Some(m) = v.git.get(&e.rel) {
            marks.push(seg(format!(" {m}"), s.fg(if *m == 'A' { green } else { yellow }).add_modifier(Modifier::BOLD)));
        }
        let mw = segs_width(&marks);
        put(buf, body.x + fw - mw - 1, y, &marks, body.x + fw);
        app.hits.push((rr, Hit::Button(super::Btn::Row(*i))));
    }
    if rows.is_empty() {
        let msg = if v.loading { "reading files…" } else if v.recent { "Nothing new in Downloads, Desktop, Documents or here lately." } else { "No files match." };
        put(buf, body.x + 2, body.y + 1, &[seg(msg, Style::default().fg(t.muted))], body.x + fw);
    }
    if !v.recent {
        let legend = vec![
            seg("M", Style::default().fg(yellow).add_modifier(Modifier::BOLD)),
            seg(" changed   ", Style::default().fg(t.muted)),
            seg("A", Style::default().fg(green).add_modifier(Modifier::BOLD)),
            seg(" new   ", Style::default().fg(t.muted)),
            seg(app.cfg.icons.blocked.clone(), Style::default().fg(t.blocked)),
            seg(" an agent is editing", Style::default().fg(t.muted)),
        ];
        put(buf, body.x + 2, body.bottom().saturating_sub(2), &legend, body.x + fw);
    }
    for yy in body.top()..body.bottom() {
        buf[(body.x + fw, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
    }
    // Preview
    let px = body.x + fw + 3;
    if let Some((path, lines)) = &v.preview {
        let rel = path.strip_prefix(&v.root).map(|p| p.display().to_string()).unwrap_or_else(|_| tilde(path));
        #[allow(unused_assignments)]
        let mut header = vec![seg(rel, Style::default().fg(t.strong).add_modifier(Modifier::BOLD))];
        if let Some(who) = v.editing_by(path) {
            header.push(seg(format!("   {who} is editing this file"), Style::default().fg(t.muted).add_modifier(Modifier::ITALIC)));
        }
        let prect = Rect { x: px, y: body.y + 3, width: body.right().saturating_sub(px + 1), height: body.bottom().saturating_sub(body.y + 3) };
        app.hy.preview_rect = prect;
        match &v.edit {
            Some(ed) => {
                header = vec![
                    seg(ed.path.strip_prefix(&v.root).map(|p| p.display().to_string()).unwrap_or_default(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
                    seg(if ed.dirty { "  ● unsaved" } else { "  editing" }, Style::default().fg(if ed.dirty { t.blocked } else { t.accent })),
                    seg("   Ctrl+S save · Esc done", Style::default().fg(t.muted)),
                ];
                put(buf, px, body.y + 1, &header, body.right());
                for (k, l) in ed.lines.iter().enumerate().skip(v.scroll).take(prect.height as usize) {
                    let yy = prect.y + (k - v.scroll) as u16;
                    let num = Style::default().fg(if k == ed.row { t.accent } else { t.muted });
                    let mut segs = vec![seg(format!("{:>4}  ", k + 1), num)];
                    segs.extend(super::views::highlight(&l.replace('\t', "    "), t));
                    put(buf, px, yy, &segs, prect.right());
                    if k == ed.row {
                        let cx = px + 6 + ed.cursor_x() as u16;
                        if cx < prect.right() {
                            let cell = &mut buf[(cx, yy)];
                            let sym = if cell.symbol().trim().is_empty() { " ".to_string() } else { cell.symbol().to_string() };
                            cell.set_symbol(&sym).set_style(Style::default().bg(t.accent).fg(t.acc_ink));
                        }
                    }
                }
            }
            None => {
                if v.in_preview {
                    header.push(seg("   ↑↓ PgUp PgDn read · i edit · ← back", Style::default().fg(t.accent)));
                }
                put(buf, px, body.y + 1, &header, body.right());
                // The side the arrows are on gets the accent line.
                if v.in_preview {
                    for yy in body.top()..body.bottom() {
                        buf[(body.x + fw, yy)].set_symbol("┃").set_style(Style::default().fg(t.accent).bg(t.bg));
                    }
                }
                for (k, l) in lines.iter().enumerate().skip(v.scroll).take(prect.height as usize) {
                    let yy = prect.y + (k - v.scroll) as u16;
                    let mut segs = vec![seg(format!("{:>4}  ", k + 1), Style::default().fg(t.muted))];
                    segs.extend(super::views::highlight(l, t));
                    put(buf, px, yy, &segs, prect.right());
                }
                if lines.len() > prect.height as usize {
                    let at = format!(" {}–{} of {} ", v.scroll + 1, (v.scroll + prect.height as usize).min(lines.len()), lines.len());
                    put(buf, body.right().saturating_sub(at.width() as u16 + 1), body.y + 1, &[seg(at, Style::default().fg(t.muted))], body.right());
                }
            }
        }
    }
    let who = app.focused().and_then(|id| app.snap.terms.get(&id)).map(pane_name).unwrap_or_else(|| "the pane".into());
    let insert = format!("Insert path into {who}");
    action_row(
        app,
        buf,
        area,
        t,
        &[
            (&insert, "Enter", BtnKind::Primary, Hit::Button(super::Btn::ViewKey('\n'))),
            ("Edit here", "i", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('i'))),
            ("Editor", "e", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('e'))),
            ("Open", "o", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('o'))),
            ("Copy path", "y", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('y'))),
            ("Show diff", "d", BtnKind::Normal, Hit::Button(super::Btn::ViewKey('d'))),
        ],
        &[seg("PgUp PgDn / wheel scroll", Style::default().fg(t.muted))],
    );
}

// ---- modals: talk, + Pane -----------------------------------------------------------------

pub(super) fn draw_talk(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, term: TermId, input: &str) {
    dim(f.buffer_mut(), area, t);
    let bw = 104.min(area.width.saturating_sub(4));
    let bh = 30.min(area.height.saturating_sub(4));
    let bx = area.x + (area.width.saturating_sub(bw)) / 2 + 6.min(area.width.saturating_sub(bw) / 2);
    let by = area.y + 6.min(area.height.saturating_sub(bh) / 2);
    let r = Rect { x: bx, y: by, width: bw, height: bh };
    let buf = f.buffer_mut();
    fill(buf, r, t.card);
    let info = app.snap.terms.get(&term).cloned();
    let (name, where_, st) = match &info {
        Some(i) => {
            let ws = app.snap.locate(term).map(|(w, _)| w.git.as_ref().map(|g| g.repo.clone()).unwrap_or(w.name.clone())).unwrap_or_default();
            (pane_name(i), format!("{ws} › {}", app.pane_location(i)), i.status)
        }
        None => ("pane".into(), String::new(), Status::None),
    };
    let left = vec![
        seg(format!("{name}  "), Style::default().add_modifier(Modifier::BOLD)),
        seg(format!("{where_}  "), Style::default()),
        seg(format!("{} {}", glyph(app, st), state_label(st)), Style::default()),
    ];
    let right = vec![
        seg("o", Style::default().add_modifier(Modifier::BOLD)),
        seg(" open as pane   ", Style::default()),
        seg("Esc", Style::default().add_modifier(Modifier::BOLD)),
        seg(" close ", Style::default()),
    ];
    title_bar(buf, r, t, &left, &right, true);
    // The agent's live screen, bottom-anchored above the input.
    let body = Rect { x: r.x + 3, y: r.y + 1, width: r.width.saturating_sub(5), height: r.height.saturating_sub(5) };
    if let Some(p) = app.parsers.get(&term) {
        let screen = p.screen();
        let (rows, cols) = screen.size();
        let text: Vec<String> = screen.rows(0, cols).collect();
        let last = text.iter().rposition(|l| !l.trim().is_empty()).map(|i| i as u16 + 1).unwrap_or(rows);
        let first = last.saturating_sub(body.height);
        let shown = Rect { height: (last - first).min(body.height), ..body };
        let shown = Rect { y: body.bottom() - shown.height, ..shown };
        render_screen_from(screen, shown, buf, t.card, first);
    }
    let ir = Rect { x: r.x, y: r.bottom().saturating_sub(3), width: r.width, height: 3 };
    fill(buf, ir, t.card2);
    let s = Style::default().bg(t.card2);
    let shown = truncate(input, ir.width.saturating_sub(20) as usize);
    let x = put(
        buf,
        ir.x + 3,
        ir.y + 1,
        &[seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(shown, s.fg(t.strong)), seg("█", s.fg(t.accent))],
        ir.right(),
    );
    let _ = x;
    let hint = vec![seg("Enter", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" send", s.fg(t.muted))];
    let hw = segs_width(&hint);
    put(buf, ir.right().saturating_sub(hw + 2), ir.y + 1, &hint, ir.right());
}

/// Like `render_screen`, but starting at screen row `first`.
fn render_screen_from(screen: &vt100::Screen, area: Rect, buf: &mut Buffer, bg: Color, first: u16) {
    let (rows, cols) = screen.size();
    for row in 0..area.height {
        let sr = first + row;
        if sr >= rows {
            break;
        }
        for col in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(sr, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = super::render::vt_color(cell.fgcolor());
            let mut cbg = match super::render::vt_color(cell.bgcolor()) {
                Color::Reset => bg,
                c => c,
            };
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut cbg);
            }
            let mut style = Style::default().fg(fg).bg(cbg);
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            let c = cell.contents();
            buf[(area.x + col, area.y + row)].set_symbol(if c.is_empty() { " " } else { c }).set_style(style);
        }
    }
}

/// A centered modal: dimmed screen behind, a card with a title bar. Returns the inner area.
pub(super) fn modal_frame(f: &mut Frame, area: Rect, t: &Theme, w: u16, h: u16, title: &str, hint: &str) -> Rect {
    dim(f.buffer_mut(), area, t);
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    let buf = f.buffer_mut();
    fill(buf, r, t.card);
    title_bar(buf, r, t, &[seg(title.to_string(), Style::default().add_modifier(Modifier::BOLD))], &[seg(format!("{hint} "), Style::default())], true);
    Rect { x: r.x + 2, y: r.y + 2, width: r.width.saturating_sub(4), height: r.height.saturating_sub(3) }
}

pub(super) fn draw_new_pane(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, np: &super::NewPane) {
    let runs = app.new_pane_runs();
    let places = app.new_pane_places(np.run);
    let h = (runs.len() + places.len() + 12) as u16;
    let inner = modal_frame(f, area, t, 64, h, "New pane", "Esc close");
    let s = Style::default().bg(t.card);
    let section = |buf: &mut Buffer, y: u16, label: &str, active: bool| {
        put(buf, inner.x, y, &[seg(label, s.fg(if active { t.accent } else { t.muted }).add_modifier(Modifier::BOLD))], inner.right());
    };
    let mut y = inner.y;
    section(f.buffer_mut(), y, "RUN", np.section == 0);
    y += 1;
    for (i, name) in runs.iter().enumerate() {
        row(app, f.buffer_mut(), inner, y, np.run == i, name, "", Hit::Button(super::Btn::NewPaneRow(0, i)), t);
        y += 1;
    }
    y += 1;
    section(f.buffer_mut(), y, "WHERE", np.section == 1);
    y += 1;
    for (i, (a, b)) in places.iter().enumerate() {
        row(app, f.buffer_mut(), inner, y, np.place == i, a, b, Hit::Button(super::Btn::NewPaneRow(1, i)), t);
        y += 1;
    }
    y += 1;
    let buf = f.buffer_mut();
    if let Some((label, text)) = &np.input {
        put(
            buf,
            inner.x + 1,
            y,
            &[seg(format!("{label} › "), s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(text.clone(), s.fg(t.strong)), seg("█", s.fg(t.accent))],
            inner.right(),
        );
    } else {
        let note = match places.get(np.place).map(|p| p.0.as_str()) {
            Some("new worktree") => "Its own branch and folder, so agents never step on each other.",
            Some("same checkout") => "Works in the same files as this pane.",
            _ => "Opens beside the focused pane, in this group.",
        };
        put(buf, inner.x + 1, y, &[seg(note, s.fg(t.muted).add_modifier(Modifier::ITALIC))], inner.right());
    }
    let fy = inner.bottom().saturating_sub(1);
    put(buf, inner.x + 1, fy, &cap_hints(t, t.card, &[("Enter", "open"), ("Tab", "run / where"), ("↑↓", "choose")]), inner.right());
}

/// The shortcut rows by category: (category, [(what it does, the actions it covers)]).
#[allow(clippy::type_complexity)]
pub(super) fn key_rows() -> Vec<(&'static str, Vec<(&'static str, Vec<crate::keys::Action>)>)> {
    use crate::keys::Action as A;
    use crate::layout::Dir::*;
    vec![
        (
            "GET AROUND",
            vec![
                ("Go to a project or session", vec![A::GoTo]),
                ("Command palette", vec![A::Palette]),
                ("Focus the sidebar", vec![A::BrowseTree]),
                ("Focus the pane left", vec![A::Focus(Left)]),
                ("Focus the pane below", vec![A::Focus(Down)]),
                ("Focus the pane above", vec![A::Focus(Up)]),
                ("Focus the pane right", vec![A::Focus(Right)]),
                ("Jump to what needs you", vec![A::Jump]),
            ],
        ),
        (
            "PANES",
            vec![
                ("Split right", vec![A::SplitRight]),
                ("Split down", vec![A::SplitDown]),
                ("Zoom pane", vec![A::Zoom]),
                ("Close pane", vec![A::ClosePane]),
                ("Show / hide the sidebar", vec![A::ToggleSidebar]),
                ("Resize left", vec![A::Resize(Left)]),
                ("Resize down", vec![A::Resize(Down)]),
                ("Resize up", vec![A::Resize(Up)]),
                ("Resize right", vec![A::Resize(Right)]),
                ("Select text with the keyboard", vec![A::CopyMode]),
            ],
        ),
        (
            "TABS",
            vec![
                ("New tab", vec![A::NewTab]),
                ("Previous tab", vec![A::PrevTab]),
                ("Next tab", vec![A::NextTab]),
                ("Close tab", vec![A::CloseTab]),
            ],
        ),
        (
            "START & TALK",
            vec![
                ("New session (a shell here)", vec![A::ShellHere]),
                ("New agent", vec![A::NewPane]),
                ("Open a project", vec![A::OpenProject]),
                ("Message an agent", vec![A::Talk]),
                ("Reply to the focused agent", vec![A::Reply]),
                ("Rename session", vec![A::RenameWorkspace]),
                ("Run a preset", vec![A::Presets]),
            ],
        ),
        (
            "CODE",
            vec![
                ("Files", vec![A::Files]),
                ("Find a file", vec![A::Find(0)]),
                ("Search the code", vec![A::Find(1)]),
                ("Changes", vec![A::Changes]),
                ("Switch branch", vec![A::Branches]),
                ("Pull request", vec![A::PullRequest]),
                ("Ship", vec![A::Ship]),
                ("Tickets", vec![A::Inbox]),
                ("Ideas", vec![A::Ideas]),
                ("Agent tools", vec![A::Toolbox]),
            ],
        ),
        (
            "APP",
            vec![
                ("Settings", vec![A::Settings]),
                ("All keys", vec![A::Help]),
                ("History", vec![A::History]),
                ("Memory", vec![A::Memory]),
                ("Detach", vec![A::Detach]),
                ("Reload config", vec![A::ReloadConfig]),
            ],
        ),
    ]
}

/// A category of shortcuts: its name and (keys, what they do) rows.
type KeyCategory = (&'static str, Vec<(String, String)>);

/// The shortcut list, by category: what each key does after the leader.
fn key_categories(app: &App) -> Vec<KeyCategory> {
    use crate::keys::Action as A;
    let rows = key_rows();
    let pretty = |k: &crate::keys::KeySpec| -> String {
        match k.to_string().as_str() {
            "Left" => "←".into(),
            "Right" => "→".into(),
            "Up" => "↑".into(),
            "Down" => "↓".into(),
            "PageUp" => "PgUp".into(),
            "PageDown" => "PgDn".into(),
            other => other.to_string(),
        }
    };
    // Keys for a row: each action's first key, then each action's second key, … so
    // directions read "h j k l ← ↓ ↑ →".
    let keys_for = |acts: &[A]| -> String {
        let per: Vec<Vec<String>> = acts
            .iter()
            .map(|a| app.keymap.prefixed_order.iter().filter(|k| app.keymap.prefixed.get(k) == Some(a)).map(pretty).collect())
            .collect();
        let mut seen: Vec<String> = Vec::new();
        let mut sets: Vec<String> = Vec::new();
        for rank in 0..per.iter().map(Vec::len).max().unwrap_or(0) {
            let mut set = Vec::new();
            for list in &per {
                if let Some(k) = list.get(rank)
                    && !seen.contains(k)
                {
                    seen.push(k.clone());
                    set.push(k.clone());
                }
            }
            if !set.is_empty() {
                sets.push(set.join(" "));
            }
        }
        sets.join("  ")
    };
    let mut covered: Vec<A> = Vec::new();
    let mut out: Vec<KeyCategory> = rows
        .into_iter()
        .map(|(cat, list)| {
            let items = list
                .into_iter()
                .filter_map(|(label, acts)| {
                    covered.extend(acts.iter().cloned());
                    let keys = keys_for(&acts);
                    (!keys.is_empty()).then(|| (keys, label.to_string()))
                })
                .collect();
            (cat, items)
        })
        .collect();
    // Anything else bound (your own shortcuts, spawn commands).
    let extra: Vec<(String, String)> = app
        .keymap
        .prefixed_order
        .iter()
        .filter_map(|k| app.keymap.prefixed.get(k).map(|a| (k, a)))
        .filter(|(_, a)| !covered.contains(a) && !matches!(a, A::SendPrefix | A::SelectTab(_) | A::SelectWorkspace(_)))
        .map(|(k, a)| (pretty(k), a.describe()))
        .collect();
    if !extra.is_empty() {
        out.push(("Yours", extra));
    }
    out.retain(|(_, items)| !items.is_empty());
    out
}

/// Every shortcut in one centered window, by category. `live`: shown after the leader key
/// (the next key runs a command); otherwise it's the Keys list (any key closes it).
pub(super) fn draw_keys(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, live: bool) {
    let cats = key_categories(app);
    let cols: u16 = if area.width >= 150 { 4 } else if area.width >= 110 { 3 } else { 2 };
    // Balance categories into columns by height.
    let mut columns: Vec<Vec<&KeyCategory>> = vec![Vec::new(); cols as usize];
    let mut heights = vec![0usize; cols as usize];
    for c in &cats {
        let i = (0..cols as usize).min_by_key(|i| heights[*i]).unwrap_or(0);
        heights[i] += c.1.len() + 2;
        columns[i].push(c);
    }
    let tallest = heights.iter().copied().max().unwrap_or(0) as u16;
    let leader = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let title = if live { format!("{leader} …then a key") } else { "Keys".to_string() };
    let hint = if live { "Esc cancel" } else { "any key closes" };
    let w = (cols * 40 + 4).min(area.width.saturating_sub(2));
    let inner = modal_frame(f, area, t, w, tallest + 5, &title, hint);
    let col_w = inner.width / cols;
    let buf = f.buffer_mut();
    let s = Style::default().bg(t.card);
    for (ci, col) in columns.iter().enumerate() {
        let x = inner.x + ci as u16 * col_w;
        let mut y = inner.y;
        // The key column fits this column's longest keys.
        let kw = col
            .iter()
            .flat_map(|(_, items)| items.iter().map(|(k, _)| segs_width(&keycaps(t, k, t.card)) as usize))
            .max()
            .unwrap_or(4)
            .min(22);
        for (cat, items) in col {
            if y >= inner.bottom() {
                break;
            }
            put(buf, x, y, &[seg(cat.to_uppercase(), s.fg(t.muted).add_modifier(Modifier::BOLD))], x + col_w);
            y += 1;
            for (keys, label) in items {
                if y >= inner.bottom() {
                    break;
                }
                let caps = keycaps(t, keys, t.card);
                let pad = " ".repeat(kw.saturating_sub(segs_width(&caps) as usize));
                let mut row = vec![seg(pad, s)];
                row.extend(caps);
                row.push(seg(format!(" {}", truncate(label, (col_w as usize).saturating_sub(kw + 2))), s.fg(t.fg)));
                put(buf, x, y, &row, x + col_w - 1);
                y += 1;
            }
            y += 1;
        }
    }
    let lead_caps: Vec<Seg> = leader.split('+').map(|k| keycap(t, k)).flat_map(|c| [c, seg("+", s.fg(t.muted))]).collect();
    let lead_caps = &lead_caps[..lead_caps.len().saturating_sub(1)];
    let mut foot = vec![seg("Press ", s.fg(t.muted))];
    foot.extend(lead_caps.iter().cloned());
    foot.push(seg(", let go, then the key.   In popups and views, letters work on their own.", s.fg(t.muted)));
    put(buf, inner.x, inner.bottom().saturating_sub(1), &foot, inner.right());
}

#[allow(clippy::too_many_arguments)]
fn row(app: &mut App, buf: &mut Buffer, r: Rect, y: u16, sel: bool, a: &str, b: &str, hit: Hit, t: &Theme) {
    let rr = Rect { x: r.x, y, width: r.width, height: 1 };
    let hovered = app.hover.is_some_and(|p| rr.contains(p));
    let bg = if sel || hovered { t.hov } else { t.card };
    if sel || hovered {
        fill(buf, rr, bg);
    }
    let s = Style::default().bg(bg);
    if sel {
        put(buf, r.x, y, &[seg(">", s.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
    }
    let mut ns = s.fg(t.strong);
    if sel {
        ns = ns.add_modifier(Modifier::BOLD);
    }
    put(buf, r.x + 3, y, &[seg(truncate(a, 17), ns)], r.x + 20);
    if !b.is_empty() {
        put(buf, r.x + 21, y, &[seg(b.to_string(), s.fg(t.muted))], r.right());
    }
    app.hits.push((rr, hit));
}

// ---- settings view -------------------------------------------------------------------------

/// A row of a settings page: a setting, or (on the Keys page) a shortcut.
#[derive(Debug, Clone)]
pub(super) enum SRow {
    Setting(&'static super::modal::Setting),
    Bind { label: &'static str, acts: Vec<crate::keys::Action> },
    /// A project the user opened (Settings → Projects).
    Project(std::path::PathBuf),
    /// One theme (Settings → Appearance): index into theme::BUILTIN.
    Theme(usize),
}

/// The small caps heading a settings row sits under.
pub(super) fn settings_group(row: &SRow) -> &'static str {
    match row {
        SRow::Theme(_) => "THEME",
        SRow::Project(_) => "PROJECTS",
        SRow::Bind { label, .. } => key_rows().into_iter().find(|(_, items)| items.iter().any(|(l, _)| l == label)).map(|(g, _)| g).unwrap_or("KEYS"),
        SRow::Setting(s) => match s.path {
            "prefix" | "ui.mouse" | "ui.which_key" => "INPUT",
            "ui.sidebar_position" | "ui.splash" | "ui.layout" => "LAYOUT",
            "shell" | "editor" | "shell_integration" => "SHELL",
            "ui.attention_sort" => "SIDEBAR",
            p if p.starts_with("notify.") => "ALERTS",
            "worktree.delete_with_last" | "worktree.per_agent" | "worktree.command" => "WORKTREES",
            p if p.starts_with("restore.") || p == "sleep_after" => "RESTARTS",
            "scrollback" => "HISTORY",
            "quick.place" => "QUICK PROMPT",
            p if p.starts_with("mcp.") => "AGENTS TALKING TO AGENTS",
            p if p.starts_with("detection.") => "STATUS DETECTION",
            _ => "OTHER",
        },
    }
}

/// A theme's name as people say it.
pub(super) fn theme_label(name: &str) -> String {
    if let Some((_, l)) = crate::theme::DESIGN.iter().find(|(n, _)| *n == name) {
        return l.to_string();
    }
    name.split('-').map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

/// The rows of one settings page.
pub(super) fn settings_rows(app: &App, cat: super::modal::Cat) -> Vec<SRow> {
    let mut rows: Vec<SRow> = super::modal::in_cat(cat)
        .into_iter()
        .flat_map(|s| {
            // Appearance: one row per theme.
            if s.path == "theme" {
                crate::theme::BUILTIN.iter().enumerate().filter(|(_, n)| **n != "mono").map(|(i, _)| SRow::Theme(i)).collect::<Vec<_>>()
            } else {
                vec![SRow::Setting(s)]
            }
        })
        .collect();
    // Grouped under their headings, in the order the headings first come.
    let order: Vec<&str> = rows.iter().map(settings_group).fold(Vec::new(), |mut v, g| {
        if !v.contains(&g) {
            v.push(g);
        }
        v
    });
    let order: Vec<&str> = ["INPUT", "LAYOUT", "SHELL"].into_iter().filter(|g| order.contains(g)).chain(order.iter().copied().filter(|g| !["INPUT", "LAYOUT", "SHELL"].contains(g))).collect();
    rows.sort_by_key(|r| order.iter().position(|g| *g == settings_group(r)).unwrap_or(99));
    if cat == super::modal::Cat::Projects {
        rows.extend(app.hy.saved.known.iter().cloned().map(SRow::Project));
    }
    if cat == super::modal::Cat::Keys {
        for (_, items) in key_rows() {
            for (label, acts) in items {
                rows.push(SRow::Bind { label, acts });
            }
        }
    }
    rows
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SettingsView {
    pub cat: usize,
    pub sel: usize,
    pub editing: Option<String>,
    /// Waiting for a key: the leader, or a new key for this row's action.
    pub capturing: bool,
    pub scroll: u16,
}

pub(super) fn control(app: &App, t: &Theme, row: &SRow, v: &SettingsView, selected: bool) -> Vec<Seg> {
    use super::modal::Kind;
    let cfg = &app.cfg;
    if selected && v.capturing {
        return vec![seg(" press the new key… ", Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD))];
    }
    match row {
        SRow::Theme(_) => vec![],
        SRow::Project(_) => vec![seg(" forget ", Style::default().bg(t.btn).fg(t.text))],
        SRow::Bind { acts, .. } => {
            let keys = key_text(app, acts);
            if keys.is_empty() {
                vec![seg("not set", Style::default().fg(t.muted))]
            } else {
                keycaps(t, &keys, t.bg)
            }
        }
        SRow::Setting(s) => {
            let cur = super::modal::current(cfg, s.path);
            match s.kind {
                Kind::Bool => {
                    if cur.and_then(|v| v.as_bool()).unwrap_or(false) {
                        vec![seg(" ● on  ", Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD))]
                    } else {
                        vec![seg(" ○ off ", Style::default().bg(t.btn).fg(t.muted).add_modifier(Modifier::BOLD))]
                    }
                }
                Kind::Number { min, max, .. } => {
                    let val = cur.and_then(|v| v.as_float()).unwrap_or(min);
                    let filled = (((val - min) / (max - min)) * 10.0).round() as usize;
                    vec![
                        seg("▰".repeat(filled.min(10)), Style::default().fg(t.accent)),
                        seg("▱".repeat(10 - filled.min(10)), Style::default().fg(t.line)),
                        seg(format!(" {val:.2}"), Style::default().fg(t.strong)),
                    ]
                }
                Kind::Key => {
                    let k = super::modal::display(cfg, s).replace("C-", "Ctrl+");
                    let caps: Vec<Seg> = k.split('+').map(|p| keycap(t, p)).flat_map(|c| [c, seg("+", Style::default().fg(t.muted))]).collect();
                    caps[..caps.len().saturating_sub(1)].to_vec()
                }
                Kind::Text => {
                    if selected && let Some(e) = &v.editing {
                        vec![seg(format!(" {e}"), Style::default().bg(t.card2).fg(t.strong)), seg("█ ", Style::default().bg(t.card2).fg(t.accent))]
                    } else {
                        let shown = super::modal::display(cfg, s);
                        let shown = if shown.is_empty() || shown == "(shell)" { "(default)".to_string() } else { shown };
                        vec![seg(format!(" {} ", truncate(&shown, 28)), Style::default().bg(t.btn).fg(t.strong))]
                    }
                }
                Kind::Choice(_) | Kind::Int { .. } => {
                    let a = Style::default().fg(if selected { t.accent } else { t.muted }).add_modifier(Modifier::BOLD);
                    vec![seg("‹ ", a), seg(super::modal::display(cfg, s), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)), seg(" ›", a)]
                }
            }
        }
    }
}

/// The current keys for a shortcut row (sets separated by two spaces, as keycaps expects).
pub(super) fn key_text(app: &App, acts: &[crate::keys::Action]) -> String {
    let per: Vec<Vec<String>> = acts
        .iter()
        .map(|a| app.keymap.prefixed_order.iter().filter(|k| app.keymap.prefixed.get(k) == Some(a)).map(pretty_key).collect())
        .collect();
    let mut sets = Vec::new();
    for rank in 0..per.iter().map(Vec::len).max().unwrap_or(0) {
        let set: Vec<String> = per.iter().filter_map(|l| l.get(rank).cloned()).collect();
        if !set.is_empty() {
            sets.push(set.join(" "));
        }
    }
    sets.join("  ")
}

pub(super) fn pretty_key(k: &crate::keys::KeySpec) -> String {
    match k.to_string().as_str() {
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "PageUp" => "PgUp".into(),
        "PageDown" => "PgDn".into(),
        other => other.to_string(),
    }
}

pub(super) fn draw_settings_view(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &SettingsView) {
    use super::modal::Cat;
    fill(buf, area, t.bg);
    let path = tilde(&crate::config::config_path());
    title_bar(
        buf,
        area,
        t,
        &[seg("settings  ", Style::default().add_modifier(Modifier::BOLD)), seg(path, Style::default())],
        &[seg("o", Style::default().add_modifier(Modifier::BOLD)), seg(" open file   ", Style::default()), seg("✕ ", Style::default())],
        true,
    );
    app.hits.push((Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, Hit::Button(super::Btn::CloseView)));
    let body = Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area };

    // Pages
    let nav_w = 22u16.min(body.width / 3);
    for (i, cat) in Cat::ALL.iter().enumerate() {
        let y = body.y + 1 + i as u16 * 2;
        if y >= body.bottom() {
            break;
        }
        let rr = Rect { x: body.x, y, width: nav_w, height: 1 };
        let active = i == v.cat;
        let hovered = app.hover.is_some_and(|p| rr.contains(p));
        if active || hovered {
            fill(buf, rr, t.hov);
        }
        let bg = if active || hovered { t.hov } else { t.bg };
        let s = Style::default().bg(bg);
        put(
            buf,
            rr.x,
            y,
            &[
                seg(if active { "▌" } else { " " }, s.fg(t.accent)),
                seg(format!(" {} ", cat.icon()), s.fg(if active { t.accent } else { t.muted })),
                seg(cat.label(), if active { s.fg(t.strong).add_modifier(Modifier::BOLD) } else { s.fg(t.text) }),
            ],
            rr.right(),
        );
        app.hits.push((rr, Hit::Button(super::Btn::SettingsCat(i))));
    }
    for yy in body.top()..body.bottom() {
        buf[(body.x + nav_w, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
    }

    // Settings on this page
    let cx = body.x + nav_w + 3;
    let cr = body.right().saturating_sub(2);
    let cat = Cat::ALL[v.cat.min(Cat::ALL.len() - 1)];
    put(buf, cx, body.y + 1, &[seg(cat.label().to_uppercase(), Style::default().fg(t.muted).add_modifier(Modifier::BOLD))], cr);
    let rows = settings_rows(app, cat);
    let row_h = 3u16;
    let visible = (body.height.saturating_sub(4) / row_h).max(1) as usize;
    let start = v.sel.saturating_sub(visible.saturating_sub(1));
    for (k, (i, row)) in rows.iter().enumerate().skip(start).take(visible).enumerate() {
        let y = body.y + 3 + k as u16 * row_h;
        let selected = i == v.sel;
        let rr = Rect { x: cx - 2, y, width: cr - cx + 3, height: 2 };
        let hovered = app.hover.is_some_and(|p| rr.contains(p));
        let bg = if selected { t.card2 } else if hovered { t.hov } else { t.bg };
        if selected || hovered {
            fill(buf, rr, bg);
        }
        let s = Style::default().bg(bg);
        if selected {
            put(buf, cx - 2, y, &[seg("▌", s.fg(t.accent))], cr);
        }
        let (label, help) = match row {
            SRow::Setting(st) => (st.label.to_string(), st.help.to_string()),
            SRow::Bind { label, acts } => {
                let help = if acts.len() > 1 { "Several keys; change them in the config file (o).".to_string() } else { "Enter, then press the new key.".to_string() };
                (label.to_string(), help)
            }
            SRow::Project(p) => (tilde(p), "Enter forgets it; its sessions keep running.".to_string()),
            SRow::Theme(i) => (theme_label(crate::theme::BUILTIN[*i]), String::new()),
        };
        let mut ls = s.fg(t.strong);
        if selected {
            ls = ls.add_modifier(Modifier::BOLD);
        }
        let ctrl: Vec<Seg> = control(app, t, row, v, selected).into_iter().map(|(x, st)| (x, if st.bg.is_none() { st.bg(bg) } else { st })).collect();
        let cw = segs_width(&ctrl);
        put(buf, cx, y, &[seg(label, ls)], cr.saturating_sub(cw + 2));
        let ctrl_x = cr.saturating_sub(cw);
        put(buf, ctrl_x, y, &ctrl, cr + 1);
        put(buf, cx, y + 1, &[seg(truncate(&help, (cr - cx) as usize), s.fg(t.muted))], cr);
        app.hits.push((rr, Hit::Button(super::Btn::SettingsRow(i))));
        app.hits.push((Rect { x: ctrl_x, y, width: cw, height: 1 }, Hit::Button(super::Btn::SettingsAct(i))));
    }
    let y = area.bottom().saturating_sub(1);
    fill(buf, Rect { y, height: 1, ..area }, t.card2);
    put(
        buf,
        area.x + 2,
        y,
        &cap_hints(t, t.card2, &[("↑↓", "choose"), ("←→", "change"), ("Enter", "edit"), ("Tab", "next page"), ("Esc", "close")]),
        area.right(),
    );
}

