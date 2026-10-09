//! The main screen: the sidebar card, the tab row, the pane cards and the toast.

use super::*;

// ---- main screen ---------------------------------------------------------------------------

pub(in crate::client) fn side_w(width: u16) -> u16 {
    if width < NARROW { 26 } else { 34 }
}

/// The Nerd Font cog (nf-fa-cog) at the sidebar's foot.
const SETTINGS_COG: &str = "\u{f013}";

/// Below this many columns the layout tightens: a slimmer sidebar without its right-hand
/// meta, and tabs other than the current one shrink to their number and state.
pub(in crate::client) const NARROW: u16 = 140;

/// Where everything goes: the sidebar card, the tab row and the pane area.
pub(in crate::client) struct Grid {
    pub side: Rect,
    pub tabs: Rect,
    pub panes: Rect,
    /// Between the sidebar and the right column (drag it to resize).
    pub edge: Rect,
    /// The sheet docked right of the panes, when one is open (else empty).
    pub sheet: Rect,
}

/// The floating grid: a margin of a column and a row, the sidebar card the full height, a
/// column of gap, then the tab row one row down and the pane cards below a row of air. Tiled
/// packs it all edge to edge.
pub(in crate::client) fn grid(app: &App, area: Rect) -> Grid {
    let look = Look::of(&app.cfg.ui);
    let (mx, my, gx) = if look.tiled { (0, 0, 0) } else { (1, 1, 1) };
    let inner = Rect { x: area.x + mx, y: area.y + my, width: area.width.saturating_sub(2 * mx), height: area.height.saturating_sub(2 * my) };
    let sw = if app.sidebar {
        app.hy.saved.side_w.unwrap_or_else(|| side_w(area.width)).clamp(SIDE_MIN, SIDE_MAX).min(inner.width / 2)
    } else {
        0
    };
    let right_side = app.cfg.ui.sidebar_position == "right";
    let gap = if sw > 0 { gx } else { 0 };
    let col_w = inner.width.saturating_sub(sw + gap);
    let (side, col, edge) = if right_side {
        let side = Rect { x: inner.right().saturating_sub(sw), width: sw, ..inner };
        (side, Rect { width: col_w, ..inner }, Rect { x: side.x.saturating_sub(gap.max(1)), width: gap.max(1), ..inner })
    } else {
        let side = Rect { width: sw, ..inner };
        (side, Rect { x: inner.x + sw + gap, width: col_w, ..inner }, Rect { x: side.right(), width: gap.max(1), ..inner })
    };
    // The tab row floats one row below the top, a row of air under it.
    let (tab_dy, air) = if look.tiled { (0, 0) } else { (1, 1) };
    let tabs = Rect { y: col.y + tab_dy, height: 1, ..col };
    let top = tabs.y + 1 + air;
    let panes = Rect { y: top, height: col.bottom().saturating_sub(top), ..col };
    let (sheet, panes) = if open_sheet(app).is_some() { split_for_sheet(area.width, panes, look.gap.max(1)) } else { (Rect::default(), panes) };
    Grid { side, tabs, panes, edge, sheet }
}

/// Draw the main screen; returns the pane area.
pub(in crate::client) fn draw(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) -> Rect {
    let model = app.hy_model();
    app.hy.wt_keys.clear();
    app.hy.branch_keys.clear();
    fill(f.buffer_mut(), area, t.bg);
    let g = grid(app, area);
    if g.side.width > 0 {
        draw_side(app, f.buffer_mut(), g.side, &model, t);
        // The gap beside the sidebar: drag it. It lights up while you point at it.
        if app.hy.drag == Some(Drag::Side) || hovered(app, g.edge) {
            let ex = g.edge.x + g.edge.width / 2;
            for yy in g.side.top() + 1..g.side.bottom().saturating_sub(1) {
                if let Some(px) = f.buffer_mut().cell_mut((ex, yy)) {
                    px.set_symbol("│").set_style(Style::default().fg(t.accent).bg(t.bg));
                }
            }
        }
        hit(app, g.edge, HyHit::SideEdge);
    }
    let col = Rect { y: g.tabs.y, height: g.panes.bottom().saturating_sub(g.tabs.y), ..g.tabs };
    if g.panes.width > 0 {
        draw_main(app, f, Rect { width: g.panes.width, ..col }, &model, t);
    }
    if let Some(kind) = open_sheet(app) {
        draw_sheet(app, f.buffer_mut(), g.sheet, kind, t);
    }
    app.hy.crumb_x = g.panes.x + 1;
    // A popup (`seshi popup`) floats over everything, the rest dimmed.
    if let Some(term) = app.popup() {
        let whole = f.area();
        dim_all(f.buffer_mut(), whole, t);
        let w = (whole.width * 4 / 5).max(40).min(whole.width);
        let h = (whole.height * 3 / 4).max(12).min(whole.height);
        let r = Rect { x: whole.x + (whole.width - w) / 2, y: whole.y + (whole.height - h) / 2, width: w, height: h };
        draw_session(app, f, r, term, true, &model, t);
    }
    draw_toast(app, f.buffer_mut(), g.panes, t);
    // Files open over everything in one tool-window size; Esc closes. (Changes is a sheet.)
    if let Some(view) = app.view.take_if(|v| matches!(v, crate::client::View::Files(_))) {
        let buf = f.buffer_mut();
        dim_all(buf, area, t);
        let frame = tool_rect(area);
        hit(app, area, HyHit::Noop);
        // A lit card around it; the tool draws inside with a cell of air.
        let mut c = Card::new(t, "").lit(t.accent);
        c.bg = t.bg;
        let inside = card(app, buf, frame, &c, t);
        let inner = Rect { x: inside.x + 1, width: inside.width.saturating_sub(2), ..inside };
        match view {
            crate::client::View::Files(v) => {
                crate::client::design::draw_files(app, buf, inner, t, &v);
                app.view = Some(crate::client::View::Files(v));
            }
            // In the sheet (above).
            v @ crate::client::View::Changes(_) => app.view = Some(v),
        }
    }
    g.panes
}

pub(in crate::client) fn find(model: &[Proj], term: TermId) -> Option<(&Proj, &Wt, &Session)> {
    model.iter().find_map(|p| p.wts.iter().find_map(|w| w.sessions.iter().find(|s| s.term == term).map(|s| (p, w, s))))
}


pub(in crate::client) fn hline(buf: &mut Buffer, x: u16, y: u16, w: u16, t: &Theme, bg: Color) {
    for i in 0..w {
        if let Some(px) = buf.cell_mut((x + i, y)) {
            px.set_symbol("─").set_style(Style::default().fg(t.line).bg(bg));
        }
    }
}

/// A line of the sidebar tree.
#[derive(Debug, Clone)]
pub(in crate::client) enum Line {
    /// A section's heading: AGENTS, TERMINALS, SSH (with how many sessions).
    Section(Kind, usize),
    Proj(usize),
    /// A heading: BRANCHES or WORKTREES.
    /// An agent or shell (pi, wi, si).
    Sess(usize, usize, usize),
    /// A dim line under an agent: what it's on, its question, a subagent. Shares the
    /// agent's highlight.
    Note(String, Color, TermId),
    /// Nothing running in a folder project.
    Empty(usize),
    Gap,
}

/// What a sidebar row shows of a session: the agent and its state on top, what it's working
/// on (or asking) underneath.
pub(in crate::client) fn session_lines(s: &Session, t: &Theme, out: &mut Vec<Line>) {
    let notes_c = t.muted;
    if let Some(q) = &s.question {
        out.push(Line::Note(q.clone(), blend(t.blocked, t.sidebar_bg, 0.25), s.term));
    }
    // Several of one kind (a workflow's fleet) are one line with a count, not a wall of rows.
    let mut kinds: Vec<(&String, usize)> = Vec::new();
    for sub in &s.subagents {
        match kinds.iter_mut().find(|(k, _)| *k == sub) {
            Some((_, n)) => *n += 1,
            None => kinds.push((sub, 1)),
        }
    }
    for (sub, n) in kinds {
        let label = if n > 1 { format!("↳ {sub} ×{n}") } else { format!("↳ {sub}") };
        out.push(Line::Note(label, notes_c, s.term));
    }
}

pub(in crate::client) fn side_lines(app: &App, model: &[Proj], t: &Theme) -> Vec<Line> {
    let mut out = Vec::new();
    for (pi, p) in model.iter().enumerate() {
        if pi == 0 || model[pi - 1].kind != p.kind {
            let n = model.iter().filter(|q| q.kind == p.kind).map(|q| q.sessions().count()).sum();
            out.push(Line::Section(p.kind, n));
        }
        out.push(Line::Proj(pi));
        if app.hy.saved.closed.contains(&format!("p:{}", p.key)) {
            out.push(Line::Gap);
            continue;
        }
        // Just the sessions, most urgent first: needs you, done, working, idle (the repo
        // folder's before the worktrees' when equal).
        let mut rows: Vec<(usize, usize)> = (0..p.wts.len()).flat_map(|wi| (0..p.wts[wi].sessions.len()).map(move |si| (wi, si))).collect();
        // Your order (dragged) within the same urgency.
        let order = &app.hy.saved.session_order;
        let at = |t: TermId| order.iter().position(|x| *x == t).unwrap_or(usize::MAX);
        rows.sort_by_key(|&(wi, si)| {
            let s = &p.wts[wi].sessions[si];
            let mine = at(s.term);
            (rank(s.status), if mine == usize::MAX { !p.wts[wi].main } else { false }, mine, s.term)
        });
        let mut any = false;
        for (wi, si) in rows {
            any = true;
            out.push(Line::Sess(pi, wi, si));
            session_lines(&p.wts[wi].sessions[si], t, &mut out);
        }
        if !any {
            out.push(Line::Empty(pi));
        }
        out.push(Line::Gap);
    }
    out
}

/// The session a sidebar line stands for, if any.
pub(in crate::client) fn line_term(model: &[Proj], l: &Line) -> Option<TermId> {
    match l {
        Line::Sess(pi, wi, si) => Some(model[*pi].wts[*wi].sessions[*si].term),
        _ => None,
    }
}

/// The sidebar card: projects (● in their colour, bold), their sessions under them most urgent
/// first, a question under a session that needs you; actions and settings at the foot.
pub(in crate::client) fn draw_side(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let look = Look::of(&app.cfg.ui);
    let pb = pane_bg(t);
    let focused_side = app.mode == Mode::Side;
    let host = crate::ipc::remote().map(|h| format!("⇄ {h}")).unwrap_or_default();
    let mut c = Card::new(t, &host);
    if focused_side {
        c = c.lit(t.accent);
    }
    card(app, buf, r, &c, t);
    app.hy.side_rect = r;
    hit(app, r, HyHit::SideFocus);
    let small = buf.area.width < NARROW;
    let lines = side_lines(app, model, t);
    let shown: Vec<TermId> = app.hy.tabs.get(app.hy.tab).map(|t| t.layout.leaves()).unwrap_or_default();
    // In a split, the split's row (its first pane's) is the open one, whichever side you're on.
    let focus = app.focused().map(|f| if shown.len() > 1 && shown.contains(&f) { shown[0] } else { f });
    // Rows: from 1 below the top border, inset 3; the foot takes the last 4 rows.
    let (x0, w) = (r.x, r.width);
    let top = r.y + 2;
    let list_h = r.height.saturating_sub(6) as usize;
    let right = r.right().saturating_sub(3);
    // Keep the focused (or cursor) row in view when it changes; otherwise the wheel rules.
    let mut scroll = app.hy.side_scroll as usize;
    if app.hy.follow {
        let want = app.hy.cursor.or(focus).and_then(|term| lines.iter().position(|l| line_term(model, l) == Some(term)));
        if let Some(i) = want {
            if i < scroll {
                scroll = i.saturating_sub(1);
            } else if i + 2 >= scroll + list_h {
                scroll = i + 3 - list_h.min(i + 3);
            }
        }
        app.hy.follow = false;
    }
    scroll = scroll.min(lines.len().saturating_sub(list_h));
    app.hy.side_scroll = scroll as u16;

    app.hy.visible = lines.iter().filter_map(|l| line_term(model, l)).collect();
    app.hy.side_items = lines
        .iter()
        .filter_map(|l| match l {
            Line::Proj(pi) => Some(SideItem::Proj(model[*pi].key.clone())),
            Line::Sess(..) => line_term(model, l).map(SideItem::Sess),
            _ => None,
        })
        .collect();
    app.hy.row_y.clear();
    app.hy.proj_keys = model.iter().map(|p| p.key.clone()).collect();

    // One row lit at a time: the one under the mouse; with the mouse elsewhere, the keyboard's
    // row or the session you're in (its bold name still says which while you point around).
    let pointing = app.hover.filter(|p| Rect { x: x0 + 2, y: top, width: w.saturating_sub(4), height: list_h as u16 }.contains(*p)).is_some_and(|p| {
        // Only rows that light up count (not a heading or a question line).
        lines.get(scroll + (p.y - top) as usize).is_some_and(|l| matches!(l, Line::Proj(_) | Line::Sess(..)))
    });
    let shade = |app: &App, term: TermId, row: Rect| -> Option<Color> {
        let mine = if focused_side { app.hy.cursor == Some(term) } else { Some(term) == focus };
        (hovered(app, row) || (!pointing && mine)).then_some(t.hov)
    };
    for (i, line) in lines.iter().enumerate().skip(scroll).take(list_h) {
        let y = top + (i - scroll) as u16;
        let row = Rect { x: x0 + 2, y, width: w.saturating_sub(4), height: 1 };
        match line {
            Line::Proj(pi) => {
                let p = &model[*pi];
                let open = !app.hy.saved.closed.contains(&format!("p:{}", p.key));
                let on = focused_side && app.hy.cursor_proj.as_ref() == Some(&p.key);
                let hov = hovered(app, row) || (on && !pointing);
                let bg = if hov { t.hov } else { pb };
                if bg != pb {
                    row_pill(look, buf, row.x, y, row.width, bg, pb);
                }
                let s = Style::default().bg(bg);
                // Its fold arrow, in the project's colour.
                let mut left = vec![seg(if open { "▾ " } else { "▸ " }, s.fg(p.color).add_modifier(Modifier::BOLD)), seg(p.name.clone(), s.fg(t.strong).add_modifier(Modifier::BOLD))];
                if p.fresh {
                    left.push(seg(" ", s));
                    left.push(seg(" NEW ", Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)));
                }
                // Right: ● n needing you, or "no git"; folded, what's inside.
                let needs = p.sessions().filter(|x| x.status == Status::Blocked).count();
                let mut c: Vec<Seg> = Vec::new();
                if needs > 0 {
                    c.push(seg(format!("● {needs}"), s.fg(t.blocked).add_modifier(Modifier::BOLD)));
                } else if !p.git && p.kind != Kind::Ssh {
                    c.push(seg("no git", s.fg(t.muted)));
                }
                if !open {
                    let n = p.sessions().count();
                    let what = if n == 0 { "empty".to_string() } else { format!("▸ {n}") };
                    c.push(seg(format!("{}{what}", if c.is_empty() { "" } else { "  " }), s.fg(t.muted)));
                }
                let cw = segs_width(&c);
                put(buf, x0 + 3, y, &left, right.saturating_sub(cw + 1));
                hit(app, row, HyHit::ToggleProj(*pi));
                if hov && !small {
                    row_menu_button(app, buf, Rect { x: right.saturating_sub(1), y, width: 1, height: 1 }, bg, t, HyHit::RowMenuProj(*pi));
                    row_glyph(app, buf, Rect { x: right.saturating_sub(3), y, width: 1, height: 1 }, "+", bg, t.accent, t, HyHit::ShellIn(*pi));
                } else {
                    put(buf, right.saturating_sub(cw), y, &c, right);
                }
            }
            Line::Sess(pi, wi, si) => {
                let s = &model[*pi].wts[*wi].sessions[*si];
                app.hy.row_y.insert(s.term, y);
                let lit = shade(app, s.term, row);
                let bg = lit.unwrap_or(pb);
                if lit.is_some() {
                    row_pill(look, buf, row.x, y, row.width, bg, pb);
                }
                let sel = focused_side && app.hy.cursor == Some(s.term);
                let st = Style::default().bg(bg);
                let (gl, gc) = if s.asleep {
                    ("☾".to_string(), t.muted)
                } else if s.is_agent || s.status != Status::None {
                    (glyph(app, s.status), state_color(t, s.status))
                } else {
                    ("›".to_string(), t.muted)
                };
                // A bell from something with no status of its own; an agent's state says more
                // (codex rings it when it finishes).
                let (gl, gc) = if s.bell && !matches!(s.status, Status::Blocked | Status::Done) { ("♪".to_string(), t.blocked) } else { (gl, gc) };
                let mut gs = st.fg(gc);
                if s.status == Status::Blocked {
                    gs = gs.add_modifier(Modifier::BOLD);
                }
                let wt = &model[*pi].wts[*wi];
                let open_row = Some(s.term) == focus;
                // Finished and not looked at yet: green, like its dot.
                // An agent's name wears its state, as its glyph does: amber shimmering while it
                // works, red when it needs you, green until you look at what it finished, dim
                // asleep. The lit row keeps bright text, readable on its pill.
                let unseen = s.is_agent && s.status == Status::Done && !s.asleep;
                let needs = s.is_agent && s.status == Status::Blocked && !s.asleep;
                let working = s.is_agent && s.status == Status::Working && !s.asleep;
                let name_c = if sel {
                    t.strong
                } else if s.asleep {
                    t.muted
                } else if needs {
                    t.blocked
                } else if unseen {
                    t.done
                } else if open_row {
                    t.strong
                } else {
                    t.text
                };
                let mut ns = st.fg(name_c);
                if open_row || sel || unseen || needs {
                    ns = ns.add_modifier(Modifier::BOLD);
                }
                let mut left = vec![];
                if working {
                    let (base, bright) = if sel { (t.strong, t.strong) } else { (t.working, blend(t.working, t.strong, 0.6)) };
                    left.extend(shimmer(&s.name, app.spinner_frame(), base, bright, ns));
                } else {
                    left.push(seg(s.name.clone(), ns));
                }
                // Right: branch · age (amber when it needs you); a limit, asleep.
                let tail: Vec<Seg> = if small {
                    vec![]
                } else if s.asleep {
                    vec![seg("asleep", st.fg(t.muted))]
                } else if let Some(at) = s.resume_at {
                    vec![seg(format!("⏸ resumes in {}", until(at)), st.fg(t.working))]
                } else if s.is_agent {
                    let col = if s.status == Status::Blocked { t.blocked } else { t.muted };
                    let first = if model[*pi].git && !wt.branch.is_empty() && wt.branch != s.name { format!("{} · ", truncate(&wt.branch, 16)) } else { String::new() };
                    let mut tail = context_tag(s, st, t);
                    tail.push(seg(format!("{first}{}", age(s.since)), st.fg(col)));
                    tail
                } else {
                    vec![]
                };
                // The name comes first: when it doesn't fit, the branch gives way (the age stays).
                let name_x = x0 + 7;
                let room = right.saturating_sub(name_x) as usize;
                let tail = if s.is_agent && !sel && !s.asleep && s.resume_at.is_none() && !small && segs_width(&left) as usize + segs_width(&tail) as usize + 2 > room {
                    let col = if s.status == Status::Blocked { t.blocked } else { t.muted };
                    vec![seg(age(s.since), st.fg(col))]
                } else {
                    tail
                };
                let tw = segs_width(&tail);
                // Pointed at: its ⋯ takes the row's last column, the right side steps in.
                let menu = hovered(app, row) && !sel;
                let end = if menu { right.saturating_sub(2) } else { right };
                put(buf, x0 + 5, y, &[seg(gl, gs)], name_x);
                put(buf, name_x, y, &left, end.saturating_sub(tw + 1));
                put(buf, end.saturating_sub(tw), y, &tail, end);
                hit(app, row, HyHit::Session(s.term));
                if menu {
                    row_menu_button(app, buf, Rect { x: right.saturating_sub(1), y, width: 1, height: 1 }, bg, t, HyHit::RowMenuSess(s.term));
                }
            }
            Line::Note(text, c, term) => {
                if small {
                    continue;
                }
                put(buf, x0 + 7, y, &[seg(truncate(text, w.saturating_sub(11) as usize), Style::default().bg(pb).fg(*c).add_modifier(Modifier::ITALIC))], right);
                hit(app, row, HyHit::Session(*term));
            }
            // Nothing running: one click starts a shell there.
            Line::Empty(pi) => {
                let bg = if hovered(app, row) { t.hov } else { pb };
                if bg != pb {
                    row_pill(look, buf, row.x, y, row.width, bg, pb);
                }
                let st = Style::default().bg(bg);
                let nk = k(app, &Action::ShellHere);
                put(
                    buf,
                    x0 + 5,
                    y,
                    &[seg("empty  ", st.fg(t.muted).add_modifier(Modifier::ITALIC)), seg(nk, st.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" new pane", st.fg(t.muted))],
                    right,
                );
                hit(app, row, HyHit::ShellIn(*pi));
            }
            // A quiet heading: "AGENTS 2 ────".
            Line::Section(kind, n) => {
                let st = Style::default().bg(pb);
                let label = format!("{} {n} ", kind.heading().to_uppercase());
                let rest = right.saturating_sub(x0 + 3 + label.width() as u16);
                put(buf, x0 + 3, y, &[seg(label, st.fg(t.muted).add_modifier(Modifier::BOLD)), seg("─".repeat(rest as usize), st.fg(t.line))], right);
            }
            Line::Gap => {}
        }
    }
    let plain = Style::default().bg(pb);
    if scroll > 0 {
        put(buf, r.right() - 2, top, &[seg("▲", plain.fg(t.muted))], r.right() - 1);
    }
    if scroll + list_h < lines.len() {
        put(buf, r.right() - 2, top + list_h as u16 - 1, &[seg("▼", plain.fg(t.muted))], r.right() - 1);
    }
    // The foot: a rule, then actions on the left and settings on the right (a dot when an
    // update is ready).
    if r.height < 8 {
        return;
    }
    let ry = r.bottom() - 4;
    hline(buf, x0 + 3, ry, w.saturating_sub(6), t, pb);
    let fy = ry + 1;
    let hot = plain.fg(t.accent).add_modifier(Modifier::BOLD);
    if focused_side {
        let items = app.cursor_items();
        let keys = crate::client::menu::menu_keys(&items);
        let back = [seg("Esc", hot), seg(" back", plain.fg(t.text))];
        let bx = right.saturating_sub(segs_width(&back));
        put(buf, bx, fy, &back, right);
        let mut segs: Vec<Seg> = Vec::new();
        let mut pairs: Vec<(String, String)> = items
            .iter()
            .zip(keys)
            .filter_map(|((label, _), k)| k.map(|k| (k.to_string(), label.trim_start_matches('▶').trim().trim_end_matches('…').split_whitespace().next().unwrap_or("").to_lowercase())))
            .collect();
        // Closing first: when they don't all fit, it's the one to keep.
        pairs.sort_by_key(|(k, _)| k != "x");
        for (key, word) in pairs {
            let pair = [seg(key, hot), seg(format!(" {word}  "), plain.fg(t.text))];
            if x0 + 3 + segs_width(&segs) + segs_width(&pair) <= bx {
                segs.extend(pair);
            }
        }
        put(buf, x0 + 3, fy, &segs, bx);
        return;
    }
    // A key that's its word's first letter is lit in the word ("actions"), else in front.
    let label = |key: String, word: &str| -> Vec<Seg> {
        match word.strip_prefix(key.as_str()) {
            Some(rest) => vec![seg(key.clone(), hot), seg(rest.to_string(), plain.fg(t.text))],
            None => vec![seg(key, hot), seg(format!(" {word}"), plain.fg(t.text))],
        }
    };
    // Pointed at: a pill a cell wider than the words on each side.
    let show = |app: &mut App, buf: &mut Buffer, x: u16, segs: Vec<Seg>, h: HyHit| {
        let w = segs_width(&segs);
        let pad = Rect { x: x - 1, y: fy, width: w + 2, height: 1 };
        if hovered(app, pad) {
            put(buf, pad.x, fy, &pill(look, segs, t.hov, pb), pad.right());
        } else {
            put(buf, x, fy, &segs, x + w);
        }
        hit(app, pad, h);
    };
    let actions = label(k(app, &Action::Actions), "actions");
    let aw = segs_width(&actions);
    show(app, buf, x0 + 3, actions, HyHit::Actions);
    // Settings: a cog (the Nerd Font one with the round pill ends that need that font too),
    // with a dot when an update is ready.
    let cog = if look.caps { SETTINGS_COG } else { "⚙" };
    let mut set = vec![seg(cog, plain.fg(t.text).add_modifier(Modifier::BOLD))];
    if app.update_available.is_some() {
        set.push(seg(" ●", hot));
    }
    let sw = segs_width(&set);
    let sx = right.saturating_sub(sw);
    if sx > x0 + 3 + aw + 2 {
        show(app, buf, sx, set, HyHit::Settings);
    }
}

pub(in crate::client) fn draw_main(app: &mut App, f: &mut Frame, area: Rect, model: &[Proj], t: &Theme) {
    let Some(focus) = app.focused() else {
        let nk = k(app, &Action::ShellHere);
        put(f.buffer_mut(), area.x + 4, area.y + 3, &[seg(format!("Nothing open. Press {} {nk} for a new session.", app.keymap.prefix.to_string().replace("C-", "Ctrl+")), Style::default().fg(t.muted))], area.right());
        return;
    };
    // Which tab was on screen last, per session.
    app.hy.tick += 1;
    let (tick, cur) = (app.hy.tick, app.hy.tab);
    if let Some(tab) = app.hy.tabs.get_mut(cur) {
        tab.used = tick;
    }
    let look = Look::of(&app.cfg.ui);
    draw_tab_bar(app, f.buffer_mut(), Rect { height: 1, ..area }, t);
    let skip = if look.tiled { 1 } else { 2 };
    let area = Rect { y: area.y + skip, height: area.height.saturating_sub(skip), ..area };
    // Between cards: `gap` rows stacked, as many columns side by side.
    let (gap_x, gap_y) = (look.gap, look.gap);
    let inset = |r: Rect| -> Rect {
        let mut r = r;
        if r.right() < area.right() {
            r.width = r.width.saturating_sub(gap_x);
        }
        if r.bottom() < area.bottom() {
            r.height = r.height.saturating_sub(gap_y);
        }
        r
    };
    app.hy.dividers.clear();
    let layout = app
        .hy
        .tabs
        .get(app.hy.tab)
        .map(|tab| tab.layout.clone())
        .filter(|l| l.contains(focus) && l.leaves().len() > 1 && app.hy.zoom != Some(focus));
    let Some(layout) = layout else {
        app.hy.leaf_rects = vec![(focus, area)];
        draw_session(app, f, area, focus, true, model, t);
        return;
    };
    let leaves = layout.leaves();
    // Two: where you drag the gap (below). More: tiled.
    if leaves.len() >= 3 {
        let rects = arranged(&leaves, area);
        app.hy.leaf_rects = rects.clone();
        for (id, r) in rects {
            draw_session(app, f, inset(r), id, id == focus, model, t);
        }
        return;
    }
    let rects = layout.rects(area);
    app.hy.leaf_rects = rects.clone();
    for (id, r) in rects {
        draw_session(app, f, inset(r), id, id == focus, model, t);
    }
    for (i, (sa, horizontal, path)) in layout.splits(area).into_iter().enumerate() {
        let ratio = layout.ratio_at(&path).unwrap_or(0.5);
        let div = crate::layout::Node::divider(sa, horizontal, ratio);
        // The gap between the two grabs it, and a cell either side of its middle line (one
        // column is hard to hit); it lights up while you point at it or drag.
        let grab = if horizontal {
            let g = gap_x.max(1);
            Rect { x: div.x.saturating_sub(g), width: g + 2, ..div }
        } else {
            let h = gap_y.max(1);
            Rect { y: div.y.saturating_sub(h), height: h, ..div }
        };
        if app.hy.drag == Some(Drag::Divider(i)) || hovered(app, grab) {
            let buf = f.buffer_mut();
            let (lx, ly) = (grab.x + 1, grab.y + grab.height / 2);
            for yy in grab.top()..grab.bottom() {
                for xx in grab.left()..grab.right() {
                    let on = if horizontal { xx == lx } else { yy == ly };
                    if on && let Some(px) = buf.cell_mut((xx, yy)) {
                        px.set_symbol(if horizontal { "│" } else { "─" }).set_style(Style::default().fg(t.accent).bg(t.bg));
                    }
                }
            }
        }
        hit(app, grab, HyHit::Divider(i));
        app.hy.dividers.push((sa, horizontal, path));
    }
}

/// The ⋯ that opens a sidebar row's menu (for terminals that keep right-clicks).
pub(in crate::client) fn row_menu_button(app: &mut App, buf: &mut Buffer, r: Rect, bg: Color, t: &Theme, h: HyHit) {
    row_glyph(app, buf, r, "⋯", bg, t.muted, t, h);
}

/// A small glyph button on a sidebar row (⋯, +): just the glyph on the row's own ground,
/// brightening when it's the one under the mouse.
#[allow(clippy::too_many_arguments)]
fn row_glyph(app: &mut App, buf: &mut Buffer, r: Rect, glyph: &str, bg: Color, fg: Color, t: &Theme, h: HyHit) {
    let mut st = Style::default().bg(bg).fg(if hovered(app, r) { t.strong } else { fg });
    if hovered(app, r) {
        st = st.add_modifier(Modifier::BOLD);
    }
    put(buf, r.x, r.y, &[seg(glyph, st)], r.right());
    hit(app, r, h);
}

/// The most urgent state among the panes of a tab, for its pill.
fn tab_state(app: &App, tab: &HyTab) -> Option<Status> {
    tab.layout
        .leaves()
        .iter()
        .filter_map(|id| app.snap.terms.get(id))
        .filter(|i| i.agent.is_some())
        .map(|i| i.status)
        .min_by_key(|s| rank(*s))
}

/// The name you gave a tab, if any. "tab 2" (what an earlier version named new tabs) isn't
/// one: the pill shows the number already.
pub(in crate::client) fn tab_name(tab: &HyTab) -> Option<String> {
    let n = tab.name.trim();
    let generic = n.strip_prefix("tab ").is_some_and(|d| d.chars().all(|c| c.is_ascii_digit()));
    (!n.is_empty() && !generic).then(|| n.to_string())
}

/// Leader mode is on: waiting for the key, or showing the key map.
pub(in crate::client) fn armed(app: &App) -> bool {
    matches!(app.mode, Mode::Prefix { .. } | Mode::KeyMap(_))
}

/// The tab row: a pill per tab (number, name, its most urgent state; the current one in the
/// accent), a + pill, and the leader pill at the far right while leader mode is on.
pub(in crate::client) fn draw_tab_bar(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme) {
    let look = Look::of(&app.cfg.ui);
    let pb = pane_bg(t);
    fill(buf, r, t.bg);
    let small = buf.area.width < NARROW;
    // The right end first, so the tabs know where to stop.
    let right: Vec<Seg> = if armed(app) {
        let ink = Style::default().fg(t.bg).add_modifier(Modifier::BOLD);
        let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+").to_uppercase();
        let mut segs = vec![seg(format!(" ⌨ {lead} "), ink)];
        if !small {
            segs.push(seg(" press a key · ? shortcuts · Esc ", Style::default().fg(t.bg)));
        }
        pill(look, segs, t.sky(), t.bg)
    } else {
        Vec::new()
    };
    let rw = segs_width(&right);
    let rx = r.right().saturating_sub(rw);
    if rw > 0 {
        put(buf, rx, r.y, &right, r.right());
        hit(app, Rect { x: rx, y: r.y, width: rw, height: 1 }, HyHit::Leader);
    }
    let mut x = r.x;
    let tabs = app.session_tabs();
    for (n, i) in tabs.iter().copied().enumerate() {
        let tab = &app.hy.tabs[i];
        let on = i == app.hy.tab;
        let state = tab_state(app, tab);
        let editing = matches!(&app.mode, Mode::RenameTab(nt) if nt.tab == i);
        let name = if editing {
            match &app.mode {
                Mode::RenameTab(nt) => nt.name.clone(),
                _ => String::new(),
            }
        } else if small && !on {
            String::new()
        } else {
            tab_name(tab).unwrap_or_default()
        };
        // Its name if you gave it one (or are typing one), else its number.
        let fg = if on { t.acc_ink } else { t.text };
        let label = if name.is_empty() && !editing { format!("{}", n + 1) } else { truncate(&name, 24) };
        let mut segs = vec![seg(" ", Style::default()), seg(label, Style::default().fg(fg).add_modifier(Modifier::BOLD))];
        if editing {
            segs.push(seg("█", Style::default().fg(fg)));
        }
        if let Some(st) = state {
            let c = if st == Status::Blocked { t.blocked } else if on { t.acc_ink } else { t.muted };
            let mut gs = Style::default().fg(c);
            if st == Status::Blocked {
                gs = gs.add_modifier(Modifier::BOLD);
            }
            segs.push(seg(format!(" {}", glyph(app, st)), gs));
        }
        let closable = on && tabs.len() > 1;
        if closable {
            segs.push(seg("  ✕", Style::default().fg(t.acc_ink)));
        }
        segs.push(seg(" ", Style::default()));
        let bg = if on { t.accent } else { pb };
        let segs = pill(look, segs, bg, t.bg);
        let w = segs_width(&segs);
        if x + w + 6 > rx {
            break;
        }
        let cr = Rect { x, y: r.y, width: w, height: 1 };
        let segs = if !on && hovered(app, cr) { segs.into_iter().map(|(s, st)| (s, if st.bg == Some(pb) { st.bg(t.hov) } else if st.fg == Some(pb) { st.fg(t.hov) } else { st })).collect() } else { segs };
        put(buf, x, r.y, &segs, rx);
        hit(app, cr, HyHit::TabPick(i));
        if closable {
            // The ✕ sits before the pill's pad and end cap.
            hit(app, Rect { x: x + w - 3, y: r.y, width: 1, height: 1 }, HyHit::TabClose(i));
        }
        x += w + 1;
    }
    let plus = pill(look, vec![seg(" + ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD))], if hovered(app, Rect { x, y: r.y, width: 5, height: 1 }) { t.hov } else { pb }, t.bg);
    let pw = segs_width(&plus);
    if x + pw <= rx {
        put(buf, x, r.y, &plus, rx);
        hit(app, Rect { x, y: r.y, width: pw, height: 1 }, HyHit::TabNew);
    }
}

/// A pane as a card: its name in the top border with project · branch after it, state and ✕
/// on the right, the terminal inside (3 columns and a row in), folder and branch in the
/// footer, model and context on the right of it. Unfocused cards fade; one that needs you
/// keeps its amber.
pub(in crate::client) fn draw_session(app: &mut App, f: &mut Frame, r: Rect, term: TermId, focused: bool, model: &[Proj], t: &Theme) {
    let Some(info) = app.snap.terms.get(&term).cloned() else { return };
    let look = Look::of(&app.cfg.ui);
    let found = find(model, term);
    let st = info.status;
    // While the sidebar has the keys, no pane shows as focused.
    let focused = focused && app.mode != Mode::Side;
    let (name, sub) = found
        .map(|(p, w, s)| {
            let br = if p.git && !w.branch.is_empty() { format!(" · {}", w.branch) } else { String::new() };
            let on = if s.is_agent && s.title != WAITING && !s.title.is_empty() && s.title != s.name { format!("{} · ", s.title) } else { String::new() };
            (s.name.clone(), format!("{on}{}{br}", p.name))
        })
        .unwrap_or_else(|| {
            // A pane beside another in a split has no sidebar row of its own: say what it is
            // from the pane itself.
            let agent = match &info.agent {
                Some(a) => a.clone(),
                None if info.is_shell() => "shell".into(),
                None => info.display_name(),
            };
            (agent, folder_name(&info.cwd))
        });
    let scrolled = app.parsers.get(&term).map(|p| p.screen().scrollback()).filter(|n| *n > 0);
    // Footer: where it runs on the left; model, context and spend on the right.
    let dim = Style::default().fg(t.muted);
    let green = t.ansi.map(|a| a[1]).unwrap_or(t.done);
    let mut foot = vec![seg(tilde(&info.cwd), dim)];
    if let Some(b) = found.map(|(p, w, _)| (p.git && !w.branch.is_empty()).then(|| w.branch.clone())).unwrap_or_else(|| info.branch.clone()) {
        foot.push(seg(" · ", dim));
        foot.push(seg(format!("⎇ {b}"), Style::default().fg(green)));
    }
    let mut foot_r: Vec<Seg> = Vec::new();
    let mut meta: Vec<String> = Vec::new();
    if let Some(m) = found.map(|(_, _, s)| s.model.clone()).filter(|m| !m.is_empty()) {
        meta.push(m);
    }
    if info.agent.is_some() {
        if let Some(c) = info.usage.context {
            meta.push(format!("ctx {c:.0}%"));
        }
        if let Some(c) = info.usage.cost.filter(|c| *c >= 0.01) {
            meta.push(format!("${c:.2}"));
        }
    }
    if let Some(at) = info.resume_at {
        foot_r.push(seg(format!("⏸ limit · continues in {}   ", until(at)), Style::default().fg(t.working)));
    }
    if let Some(n) = scrolled {
        foot_r.push(seg(format!("↑{n}  "), Style::default().fg(t.accent)));
    }
    if !meta.is_empty() {
        foot_r.push(seg(meta.join(" · "), dim));
    }
    let border = if !focused {
        t.line
    } else if armed(app) {
        t.sky()
    } else {
        match app.cfg.ui.focus_border.as_str() {
            "bright" => t.strong,
            "none" => t.line,
            _ => t.accent,
        }
    };
    let mut c = Card::new(t, &name);
    c.sub = &sub;
    c.state = info.agent.is_some().then_some(st).filter(|s| *s != Status::None);
    c.close = Some(HyHit::CloseSplit(term));
    c.foot = foot;
    c.foot_r = foot_r;
    if focused {
        c = c.lit(border);
        c.title_fg = t.accent;
    }
    let buf = f.buffer_mut();
    let inside = card(app, buf, r, &c, t);
    app.pane_frames.push((term, r));

    // The terminal: 3 columns in, a row below the border, above a blank row and the footer.
    // Answer buttons only for a question asked through seshi (ask-human), which has nowhere
    // else to be answered: an agent's own question is answered in its own prompt.
    let ask = st == Status::Blocked && info.agent.is_some() && app.pending_question(term).is_some();
    let below = 3 + if ask { 2 } else { 0 };
    let inner = Rect { x: r.x + 3, y: r.y + 2, width: r.width.saturating_sub(6), height: r.height.saturating_sub(2 + below) };
    app.panes.push((term, inner));
    app.hits.push((inner, Hit::Pane(term)));
    let copying = matches!(&app.mode, Mode::Copy(c) if c.term == term);
    if copying {
        if let Mode::Copy(c) = &mut app.mode {
            c.height = inner.height as usize;
            c.width = inner.width as usize;
            crate::client::render::render_copy(c, inner, f.buffer_mut(), t);
        }
    } else if let Some(p) = app.parsers.get(&term) {
        let screen = p.screen();
        render_screen(screen, inner, f.buffer_mut(), c.bg);
        if focused
            && !screen.hide_cursor()
            && scrolled.is_none()
            && matches!(app.mode, Mode::Normal | Mode::Prefix { .. })
            && app.view.is_none()
        {
            let (row, col) = screen.cursor_position();
            if row < inner.height && col < inner.width {
                f.set_cursor_position(Position::new(inner.x + col, inner.y + row));
            }
        }
    }

    // A scrollbar in the margin when there's history: where you are, click or drag it.
    let (cur, total) = app.history(term);
    if total > 0 && inner.height > 2 {
        let track = Rect { x: inner.right() + 1, y: inner.y, width: 1, height: inner.height };
        let h = track.height as usize;
        let thumb = (h * h / (h + total)).clamp(1, h);
        let top = track.y + ((h - thumb) * (total - cur) / total) as u16;
        let hot = app.hy.drag == Some(Drag::Scroll(term)) || hovered(app, track);
        for yy in track.top()..track.bottom() {
            let on = yy >= top && yy < top + thumb as u16;
            let (sym, col) = if on { ("┃", if hot { t.accent } else { t.muted }) } else { ("│", t.line) };
            if let Some(px) = f.buffer_mut().cell_mut((track.x, yy)) {
                px.set_symbol(sym).set_style(Style::default().fg(col).bg(c.bg));
            }
        }
        if app.hy.drag.is_none() || app.hy.drag == Some(Drag::Scroll(term)) {
            app.hy.bar = Some((term, track, total));
        }
        hit(app, track, HyHit::ScrollBar(term));
    }

    // Scrolled up: say so, and how to get back.
    if let Some(n) = scrolled {
        let note = pill(
            look,
            vec![seg(format!(" ↑ {n} lines up "), Style::default().fg(t.acc_ink).add_modifier(Modifier::BOLD))],
            t.accent,
            c.bg,
        );
        let w = segs_width(&note);
        put(f.buffer_mut(), inner.right().saturating_sub(w), inner.y, &note, inner.right());
    }

    // Asleep: the last screen stays, dimmed, with a note on how to wake it.
    if info.asleep {
        dim_all(f.buffer_mut(), inner, t);
        let note = vec![
            seg(" ☾ asleep to save memory · ", Style::default().bg(t.card2).fg(t.text)),
            seg("click or press any key", Style::default().bg(t.card2).fg(t.accent).add_modifier(Modifier::BOLD)),
            seg(" to wake it where it left off ", Style::default().bg(t.card2).fg(t.text)),
        ];
        let w = segs_width(&note);
        let x = inner.x + inner.width.saturating_sub(w) / 2;
        put(f.buffer_mut(), x, inner.y + inner.height / 2, &note, inner.right());
    }

    // Not the one you're in: its inside fades (the border and its amber tag stay as they are).
    if !focused {
        dim_inside(f.buffer_mut(), inside, look.dim, t);
    }

    // Answer bar: the choices of the question asked through seshi, a full-width pill that
    // keeps its amber even when the card is faded.
    if ask && r.height >= 8 {
        let ay = r.bottom() - 5;
        let (lx, rx) = (r.x + 3, r.right().saturating_sub(3));
        row_pill(look, f.buffer_mut(), lx - 1, ay, rx - lx + 2, t.card2, c.bg);
        let opts = app.answer_options(term);
        let full: u16 = opts.iter().take(4).map(|o| o.width() as u16 + 6).sum();
        // In a narrow pane the label shrinks to its dot, then the buttons to their keys.
        let roomy = rx.saturating_sub(lx) >= full + 11;
        let label = if roomy { "● answer   " } else { "● " };
        let mut x = put(f.buffer_mut(), lx, ay, &[seg(label, Style::default().fg(t.blocked).bg(t.card2).add_modifier(Modifier::BOLD))], rx);
        let keys_only = rx.saturating_sub(x) < full;
        for (i, o) in opts.iter().enumerate().take(4) {
            let key = char::from_digit(i as u32 + 1, 10).unwrap_or('1').to_string();
            let label = if keys_only { String::new() } else { o.clone() };
            let w = segs_width(&button_pill(look, t, &label, &key, i == 0, false, t.card2));
            let br = Rect { x, y: ay, width: w, height: 1 };
            let segs = button_pill(look, t, &label, &key, i == 0, hovered(app, br), t.card2);
            x = put(f.buffer_mut(), x, ay, &segs, rx) + 1;
            app.hits.push((br, Hit::Button(crate::client::Btn::Answer(term, key.chars().next().unwrap_or('1')))));
        }
    }
}

/// A note (copied, saved, couldn't …) as a small pop-up just above the bottom bar, centred
/// over the panes; it goes after a few seconds (errors stay a little longer).
pub(in crate::client) fn draw_toast(app: &mut App, buf: &mut Buffer, panes: Rect, t: &Theme) {
    let Some((msg, at, err)) = app.notice.clone() else { return };
    // About a session: it stays a little longer, and a click goes there.
    let about = app.notice_term.filter(|t| app.snap.terms.contains_key(t));
    if at.elapsed().as_millis() > if err { 4500 } else if about.is_some() { 6000 } else { 2500 } {
        return;
    }
    let hint = if about.is_some() { "   click to open" } else { "" };
    let text = truncate(&msg, (panes.width.saturating_sub(14) as usize).saturating_sub(hint.width()));
    // A pill inside the top right of the panes, a row below the card's border: away from
    // where you type (an agent's prompt is at the bottom).
    let segs = pill(
        Look::of(&app.cfg.ui),
        vec![
            seg(" ", Style::default()),
            seg(if err { "✕ " } else { "✓ " }, Style::default().fg(if err { t.err } else { t.done }).add_modifier(Modifier::BOLD)),
            seg(text, Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
            seg(hint, Style::default().fg(t.muted)),
            seg(" ", Style::default()),
        ],
        t.card2,
        pane_bg(t),
    );
    let w = segs_width(&segs);
    if panes.height < 4 || panes.width < w + 6 {
        return;
    }
    let r = Rect { x: panes.right().saturating_sub(w + 3), y: panes.y + 1, width: w, height: 1 };
    put(buf, r.x, r.y, &segs, r.right());
    if let Some(term) = about {
        hit(app, r, HyHit::Session(term));
    }
}

/// "72% " in front of an agent's row once its context is getting full.
fn context_tag(s: &Session, st: Style, t: &Theme) -> Vec<Seg> {
    match s.context.filter(|c| *c >= CONTEXT_SHOWN_FROM) {
        Some(c) => vec![seg(format!("{c:.0}% "), st.fg(fullness(t, c)))],
        None => Vec::new(),
    }
}
