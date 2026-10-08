//! The main screen: sidebar, panes, tab bar, toast and bottom bar.

use super::*;

// ---- main screen ---------------------------------------------------------------------------

pub(in crate::client) fn side_w(width: u16) -> u16 {
    match width {
        0..140 => 30,
        140..200 => 38,
        _ => 44,
    }
}

/// Draw the main screen; returns the pane area.
pub(in crate::client) fn draw(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) -> Rect {
    let model = app.hy_model();
    app.hy.wt_keys.clear();
    app.hy.branch_keys.clear();
    app.hy.pr_keys.clear();
    fill(f.buffer_mut(), area, t.bg);
    let sw = if app.sidebar {
        app.hy.saved.side_w.unwrap_or_else(|| side_w(area.width)).clamp(SIDE_MIN, SIDE_MAX).min(area.width / 2)
    } else {
        0
    };
    let right_side = app.cfg.ui.sidebar_position == "right";
    // The panes get the full height; the sidebar sits under the logo.
    // No top bar: the sidebar and panes start at the top; a bottom bar for what needs you.
    let mid = Rect { x: area.x, y: area.y, width: area.width, height: area.height.saturating_sub(1) };
    let (side, panes) = if sw == 0 {
        (Rect { width: 0, ..mid }, mid)
    } else if right_side {
        (
            Rect { x: mid.right() - sw, width: sw, ..mid },
            Rect { width: mid.width - sw - 1, ..mid },
        )
    } else {
        (Rect { width: sw, ..mid }, Rect { x: mid.x + sw + 1, width: mid.width.saturating_sub(sw + 1), ..mid })
    };

    if sw > 0 {
        draw_side(app, f.buffer_mut(), side, &model, t);
        if app.mode == Mode::Side {
            outline_side(f.buffer_mut(), side, right_side, t);
        }
        // The edge between sidebar and panes: drag it.
        let ex = if right_side { side.x.saturating_sub(1) } else { side.right() };
        let edge = Rect { x: ex, y: side.y, width: 1, height: side.height };
        let c = if app.hy.drag == Some(Drag::Side) || hovered(app, edge) || app.mode == Mode::Side { t.accent } else { t.line };
        for yy in edge.top()..edge.bottom() {
            if let Some(px) = f.buffer_mut().cell_mut((ex, yy)) {
                px.set_symbol("│").set_style(Style::default().fg(c).bg(t.bg));
            }
        }
        hit(app, edge, HyHit::SideEdge);
    }
    // A cell of air on each side; every pane starts with its own title bar.
    let panes = Rect { x: panes.x + 1, y: panes.y, width: panes.width.saturating_sub(2), height: panes.height };
    draw_main(app, f, panes, &model, t);
    app.hy.crumb_x = panes.x + 1;
    draw_status(app, f.buffer_mut(), Rect { y: area.bottom().saturating_sub(1), height: 1, ..area }, &model, t);
    // A popup (`hydra popup`) floats over everything, the rest dimmed.
    if let Some(term) = app.popup() {
        let whole = f.area();
        dim_all(f.buffer_mut(), whole, t);
        let w = (whole.width * 4 / 5).max(40).min(whole.width);
        let h = (whole.height * 3 / 4).max(12).min(whole.height);
        let r = Rect { x: whole.x + (whole.width - w) / 2, y: whole.y + (whole.height - h) / 2, width: w, height: h };
        fill(f.buffer_mut(), r, t.bg);
        draw_session(app, f, r, term, true, false, &model, t);
    }
    draw_toast(app, f.buffer_mut(), panes, t);
    // Files, diffs and pull requests open over everything in one tool-window size; Esc
    // closes.
    if matches!(app.view, Some(crate::client::View::Changes(_) | crate::client::View::Pr(_) | crate::client::View::Files(_)))
        && let Some(view) = app.view.take()
    {
        let buf = f.buffer_mut();
        dim_all(buf, area, t);
        let inner = tool_rect(area);
        fill(buf, inner, t.bg);
        hit(app, area, HyHit::Noop);
        match view {
            crate::client::View::Files(v) => {
                crate::client::design::draw_files(app, buf, inner, t, &v);
                app.view = Some(crate::client::View::Files(v));
            }
            crate::client::View::Changes(v) => {
                crate::client::design::draw_changes(app, buf, inner, t, &v);
                app.view = Some(crate::client::View::Changes(v));
            }
            crate::client::View::Pr(v) => {
                draw_pr(app, buf, inner, t, &v);
                app.view = Some(crate::client::View::Pr(v));
            }
            other => app.view = Some(other),
        }
    }
    panes
}

pub(in crate::client) fn find(model: &[Proj], term: TermId) -> Option<(&Proj, &Wt, &Session)> {
    model.iter().find_map(|p| p.wts.iter().find_map(|w| w.sessions.iter().find(|s| s.term == term).map(|s| (p, w, s))))
}

/// `●1 ✓1 ⠹2`: counts of a list of sessions by state.
pub(in crate::client) fn counts<'a>(app: &App, t: &Theme, list: impl Iterator<Item = &'a Session>, ink: Option<Color>) -> Vec<Seg> {
    let mut c = [0usize; 4];
    for s in list {
        if s.is_agent || s.status != Status::None {
            c[rank(s.status) as usize] += 1;
        }
    }
    let states = [Status::Blocked, Status::Done, Status::Working, Status::Idle];
    states
        .iter()
        .zip(c)
        .filter(|(_, n)| *n > 0)
        .map(|(st, n)| {
            let mut s = Style::default().fg(ink.unwrap_or(t.status(*st)));
            if *st == Status::Blocked {
                s = s.add_modifier(Modifier::BOLD);
            }
            seg(format!("{}{n} ", glyph(app, *st)), s)
        })
        .collect()
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
    /// A race in this project (race id).
    Race(u64),
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
    for sub in &s.subagents {
        out.push(Line::Note(format!("↳ {sub}"), notes_c, s.term));
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
        for r in app.hy.saved.races.iter().filter(|r| path_key(&r.project) == p.key) {
            out.push(Line::Race(r.id));
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

/// The sidebar has the keys: an accent line all the way round it (the edge to the panes is
/// drawn by the caller). Rows' own markers win over the line.
pub(in crate::client) fn outline_side(buf: &mut Buffer, r: Rect, right_side: bool, t: &Theme) {
    if r.height < 2 {
        return;
    }
    let bottom = r.bottom() - 1;
    let outer = if right_side { r.right() - 1 } else { r.x };
    let blank = |buf: &Buffer, x: u16, y: u16| buf[(x, y)].symbol() == " ";
    for x in r.left()..r.right() {
        if blank(buf, x, bottom) {
            let bg = buf[(x, bottom)].bg;
            if let Some(px) = buf.cell_mut((x, bottom)) {
                px.set_symbol("▁").set_style(Style::default().fg(t.accent).bg(bg));
            }
        }
    }
    for y in r.top()..r.bottom() {
        if blank(buf, outer, y) {
            let bg = buf[(outer, y)].bg;
            if let Some(px) = buf.cell_mut((outer, y)) {
                px.set_symbol(if right_side { "▕" } else { "▏" }).set_style(Style::default().fg(t.accent).bg(bg));
            }
        }
    }
}

pub(in crate::client) fn draw_side(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let surf = t.sidebar_bg;
    fill(buf, r, surf);
    app.hy.side_rect = r;
    let lines = side_lines(app, model, t);
    let shown: Vec<TermId> = app.hy.tabs.get(app.hy.tab).map(|t| t.layout.leaves()).unwrap_or_default();
    // In a split, the split's row (its first pane's) is the open one, whichever side you're on.
    let focus = app.focused().map(|f| if shown.len() > 1 && shown.contains(&f) { shown[0] } else { f });
    let split = shown.iter().copied().find(|t| Some(*t) != focus && shown.len() > 1);
    let focused_side = app.mode == Mode::Side;
    if focused_side {
        // The keys are here: an accent line along the sidebar's top too.
        for xx in r.x..r.right() {
            if let Some(px) = buf.cell_mut((xx, r.y)) {
                px.set_symbol("▔").set_style(Style::default().fg(t.accent).bg(surf));
            }
        }
    }
    // The sections name themselves: the list starts at the top.
    let r = Rect { y: r.y + 1, height: r.height.saturating_sub(1), ..r };
    let list_h = r.height.saturating_sub(3) as usize;
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
    let tk = k(app, &Action::Talk);
    let (x0, w) = (r.x, r.width);
    let right = r.right().saturating_sub(2);

    // Two shades: the one that's open (a touch of accent) and the one under the mouse or
    // cursor (a touch lighter), so you can tell them apart and the status colours still read.
    let active = t.hov;
    let hover = crate::client::render::blend(surf, t.text, 0.10);
    // A session's highlight: focused (filled), in the split, under the cursor or mouse.
    let look = |app: &App, term: TermId, row: Rect| -> (Color, Option<Color>, bool) {
        let prim = Some(term) == focus;
        let sel = !prim && (app.hy.cursor == Some(term) || hovered(app, row));
        let bg = if prim {
            active
        } else if sel {
            hover
        } else if Some(term) == split || (shown.len() > 1 && shown.contains(&term) && Some(term) != focus) {
            t.card2
        } else {
            surf
        };
        (bg, None, sel)
    };
    // The open one is marked by a bar on its left, not a fill.
    let bar = |buf: &mut Buffer, term: TermId, y: u16, bg: Color| {
        if Some(term) == focus
            && let Some(px) = buf.cell_mut((x0, y)) {
                px.set_symbol("▌").set_style(Style::default().fg(t.accent).bg(bg));
            }
    };

    for (i, line) in lines.iter().enumerate().skip(scroll).take(list_h) {
        let y = r.y + (i - scroll) as u16;
        let row = Rect { x: x0, y, width: w, height: 1 };
        match line {
            Line::Proj(pi) => {
                let p = &model[*pi];
                let open = !app.hy.saved.closed.contains(&format!("p:{}", p.key));
                let on = app.mode == Mode::Side && app.hy.cursor_proj.as_ref() == Some(&p.key);
                let hov = hovered(app, row) || on;
                let bg = if hov { crate::client::render::blend(surf, t.text, 0.10) } else { surf };
                fill(buf, row, bg);
                let s = Style::default().bg(bg);
                let mut left = vec![
                    seg(if open { "▾ " } else { "▸ " }, s.fg(t.muted)),
                    seg("▌", s.fg(p.color)),
                    seg(p.name.clone(), s.fg(t.strong).add_modifier(Modifier::BOLD)),
                ];
                if p.fresh {
                    left.push(seg(" ", s));
                    left.push(seg(" NEW ", Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)));
                }
                // Right: ● n needing you, "no git", and when folded what's inside.
                let needs = p.sessions().filter(|x| x.status == Status::Blocked).count();
                let mut c: Vec<Seg> = Vec::new();
                if needs > 0 {
                    c.push(seg(format!("● {needs}"), s.fg(t.blocked).add_modifier(Modifier::BOLD)));
                }
                if !p.git && p.kind != Kind::Ssh {
                    c.push(seg(format!("{}no git", if c.is_empty() { "" } else { "  " }), s.fg(t.muted)));
                }
                if !open {
                    let n = p.sessions().count();
                    let what = if n == 0 {
                        "empty".to_string()
                    } else {
                        let busy = p.sessions().filter(|x| x.status == Status::Working).count();
                        if busy > 0 { format!("{busy} working") } else { format!("{n} idle") }
                    };
                    c.push(seg(format!("{}{what}", if c.is_empty() { "" } else { "  " }), s.fg(t.muted)));
                }
                let cw = segs_width(&c);
                put(buf, x0 + 2, y, &left, right.saturating_sub(cw + 1));
                if !hov {
                    put(buf, right.saturating_sub(cw) + 1, y, &c, r.right());
                }
                hit(app, row, HyHit::ToggleProj(*pi));
                if hov {
                    row_menu_button(app, buf, Rect { x: r.right().saturating_sub(2), y, width: 2, height: 1 }, bg, t, HyHit::RowMenuProj(*pi));
                    let plus = Rect { x: r.right().saturating_sub(5), y, width: 3, height: 1 };
                    put(buf, plus.x, y, &[seg(" + ", Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
                    hit(app, plus, HyHit::ShellIn(*pi));
                }
            }
            Line::Sess(pi, wi, si) => {
                let s = &model[*pi].wts[*wi].sessions[*si];
                app.hy.row_y.insert(s.term, y);
                let (bg, ink, sel) = look(app, s.term, row);
                fill(buf, row, bg);
                let st = Style::default().bg(bg);
                let (gl, gc) = if s.asleep {
                    ("☾".to_string(), t.muted)
                } else if s.is_agent || s.status != Status::None {
                    (glyph(app, s.status), t.status(s.status))
                } else {
                    (app.cfg.icons.shell.clone(), t.muted)
                };
                let (gl, gc) = match &s.dev {
                    Some(d) => ("▶".to_string(), if d.ready { t.done } else { t.muted }),
                    // A bell from something with no status of its own; an agent's state says more
                    // (codex rings it when it finishes).
                    None if s.bell && !matches!(s.status, Status::Blocked | Status::Done) => ("♪".to_string(), t.blocked),
                    None => (gl, gc),
                };
                let mut gs = st.fg(ink.unwrap_or(gc));
                if s.status == Status::Blocked {
                    gs = gs.add_modifier(Modifier::BOLD);
                }
                // What it is (the icon, coloured by status) and its name; state on the right.
                let wt = &model[*pi].wts[*wi];
                let racing = !wt.main && app.hy.saved.races.iter().any(|r| r.entries.iter().any(|(_, b)| *b == wt.branch));
                let mut left = vec![seg(format!("{gl} "), gs)];
                // The agent's icon (shells have none), then its name.
                left.push(seg(if s.is_agent { format!("{} ", kind_icon(app, &s.agent, true)) } else { "  ".to_string() }, st.fg(ink.unwrap_or(t.muted))));
                if racing {
                    left.push(seg("⚑ ", st.fg(ink.unwrap_or(t.accent))));
                }
                let focused_row = Some(s.term) == focus;
                let needs = s.status == Status::Blocked && !s.asleep;
                // Finished and not looked at yet: green, like its dot.
                let done = s.is_agent && s.status == Status::Done && !s.asleep;
                let mut ns = st.fg(ink.unwrap_or(if needs { t.blocked } else if done { t.done } else if focused_row { t.strong } else { t.text }));
                if focused_row || needs || done {
                    ns = ns.add_modifier(Modifier::BOLD);
                }
                if s.is_agent && s.status == Status::Working && ink.is_none() && !s.asleep {
                    left.extend(shimmer(&s.name, app.spinner_frame(), t.working, t.strong, ns));
                } else {
                    left.push(seg(s.name.clone(), ns));
                }
                if let Some(d) = &s.dev {
                    let port = d.port.map(|p| format!(" :{p}")).unwrap_or_default();
                    let state = if d.ready { "ready" } else { "starting…" };
                    left.truncate(1);
                    left.push(seg(format!("dev{port}"), st.fg(ink.unwrap_or(t.strong)).add_modifier(Modifier::BOLD)));
                    left.push(seg(format!("  {state}"), st.fg(ink.unwrap_or(if d.ready { t.done } else { t.muted }))));
                }
                // The branch's pull request, on its first session.
                let pr = (*si == 0).then(|| model[*pi].prs.iter().find(|p| p.branch == wt.branch)).flatten();
                let tail: Vec<Seg> = if sel {
                    vec![seg(format!(" {tk} "), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD))]
                } else if let Some(p) = pr {
                    let k2 = app.hy.pr_keys.len();
                    app.hy.pr_keys.push((wt.path.clone(), p.number.to_string()));
                    let tg: Vec<Seg> = pr_tag(t, p).into_iter().map(|(x, s2)| (x, s2.bg(bg))).collect();
                    let tw = segs_width(&tg);
                    hit(app, Rect { x: right.saturating_sub(tw) + 1, y, width: tw, height: 1 }, HyHit::Pr(k2));
                    tg
                } else if s.asleep {
                    vec![seg("asleep", st.fg(ink.unwrap_or(t.muted)))]
                } else if let Some(at) = s.resume_at {
                    vec![seg(format!("⏸ limit · resumes in {}", until(at)), st.fg(ink.unwrap_or(t.working)))]
                } else if s.is_agent {
                    // branch · age (in a repo), state · age (outside one); amber when it needs you.
                    let first = if model[*pi].git && !wt.branch.is_empty() && wt.branch != s.name { wt.branch.clone() } else { state_label(s.status).to_string() };
                    let col = match s.status {
                        Status::Blocked => t.blocked,
                        Status::Done => t.done,
                        _ => t.muted,
                    };
                    let mut tail = context_tag(s, st, ink, t);
                    tail.push(seg(format!("{} · {}", truncate(&first, 18), age(s.since)), st.fg(ink.unwrap_or(col))));
                    tail
                } else {
                    vec![]
                };
                // The name comes first: when it doesn't fit, the branch gives way (the age stays).
                let room = right.saturating_sub(x0 + 6) as usize;
                let tail = if s.is_agent && !sel && pr.is_none() && !s.asleep && s.resume_at.is_none() && segs_width(&left) as usize + segs_width(&tail) as usize + 2 > room {
                    let col = if s.status == Status::Blocked { t.blocked } else { t.muted };
                    let mut tail = context_tag(s, st, ink, t);
                    tail.push(seg(age(s.since), st.fg(ink.unwrap_or(col))));
                    tail
                } else {
                    tail
                };
                let tw = segs_width(&tail);
                put(buf, x0 + 6, y, &left, right.saturating_sub(tw + 1));
                put(buf, right.saturating_sub(tw) + 1, y, &tail, r.right());
                hit(app, row, HyHit::Session(s.term));
                bar(buf, s.term, y, bg);
                if hovered(app, row) {
                    row_menu_button(app, buf, Rect { x: r.right().saturating_sub(2), y, width: 2, height: 1 }, bg, t, HyHit::RowMenuSess(s.term));
                }
                if sel {
                    hit(app, Rect { x: right.saturating_sub(tw) + 1, y, width: tw, height: 1 }, HyHit::Talk(s.term));
                }
            }
            Line::Note(text, c, term) => {
                let (bg, ink, _) = look(app, *term, row);
                // Notes follow their session's highlight, not the mouse.
                let bg = if bg == t.hov && app.hy.cursor != Some(*term) { surf } else { bg };
                fill(buf, row, bg);
                put(buf, x0 + 10, y, &[seg(truncate(text, w.saturating_sub(12) as usize), Style::default().bg(bg).fg(ink.unwrap_or(*c)).add_modifier(Modifier::ITALIC))], r.right() - 1);
                hit(app, row, HyHit::Session(*term));
            }
            Line::Race(id) => {
                let Some(race) = app.hy.saved.races.iter().find(|r| r.id == *id).cloned() else { continue };
                let bg = if hovered(app, row) { t.hov } else { surf };
                fill(buf, row, bg);
                let s = Style::default().bg(bg);
                put(
                    buf,
                    x0 + 3,
                    y,
                    &[
                        seg("⚑ race ", s.fg(t.accent).add_modifier(Modifier::BOLD)),
                        seg(truncate(&race.prompt, w.saturating_sub(18) as usize), s.fg(t.text)),
                        seg(format!("  {}", race.entries.len()), s.fg(t.muted)),
                    ],
                    r.right(),
                );
                hit(app, row, HyHit::RaceOpen(*id));
            }
            // Nothing running: one click starts a shell there.
            Line::Empty(pi) => {
                let hov = hovered(app, row);
                let bg = if hov { crate::client::render::blend(surf, t.text, 0.10) } else { surf };
                fill(buf, row, bg);
                let st = Style::default().bg(bg);
                let nk = k(app, &Action::ShellHere);
                put(
                    buf,
                    x0 + 6,
                    y,
                    &[seg("empty  ", st.fg(t.muted).add_modifier(Modifier::ITALIC)), seg(nk, st.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" new pane", st.fg(t.muted))],
                    r.right(),
                );
                hit(app, row, HyHit::ShellIn(*pi));
            }
            // A quiet divider: ── Terminals 4 ─────────
            Line::Section(kind, n) => {
                let st = Style::default().bg(surf);
                let label = format!(" {} {n} ", kind.heading());
                let left = 2u16;
                let rest = w.saturating_sub(left + label.width() as u16 + 2);
                put(
                    buf,
                    x0 + 1,
                    y,
                    &[seg("─".repeat(left as usize), st.fg(t.line)), seg(label, st.fg(t.muted)), seg("─".repeat(rest as usize), st.fg(t.line))],
                    r.right(),
                );
            }
            Line::Gap => {}
        }
    }
    let plain = Style::default().bg(surf);
    if scroll > 0 {
        put(buf, r.right() - 1, r.y, &[seg("▲", plain.fg(t.muted))], r.right());
    }
    if scroll + list_h < lines.len() {
        put(buf, r.right() - 1, r.y + list_h as u16 - 1, &[seg("▼", plain.fg(t.muted))], r.right());
    }
    // Quiet hints: new, jump, settings.
    let by = r.bottom().saturating_sub(3);
    hline(buf, x0 + 2, by, w.saturating_sub(4), t, surf);
    let mut hx = x0 + 2;
    for (key, label, h) in [(k(app, &Action::ShellHere), "new", HyHit::NewPane), (k(app, &Action::GoTo), "go to", HyHit::GoTo), (k(app, &Action::Settings), "settings", HyHit::Settings)] {
        let segs = vec![seg(key, plain.fg(t.accent).add_modifier(Modifier::BOLD)), seg(format!(" {label}"), plain.fg(t.text))];
        let sw = segs_width(&segs);
        // A narrow sidebar shows the hints that fit, whole.
        if hx + sw > r.right().saturating_sub(1) {
            break;
        }
        let hr = Rect { x: hx, y: by + 1, width: sw, height: 1 };
        let segs: Vec<Seg> = if hovered(app, hr) { segs.into_iter().map(|(x, st)| (x, st.bg(t.hov))).collect() } else { segs };
        put(buf, hx, by + 1, &segs, r.right());
        hit(app, hr, h);
        hx += sw + 4;
    }
}

pub(in crate::client) fn draw_main(app: &mut App, f: &mut Frame, area: Rect, model: &[Proj], t: &Theme) {
    // Files, Changes or a pull request replace the sessions until closed.
    if let Some(view) = app.view.take() {
        let buf = f.buffer_mut();
        match view {
            // Drawn on top of everything (see draw).
            v @ (crate::client::View::Changes(_) | crate::client::View::Pr(_) | crate::client::View::Files(_)) => {
                app.view = Some(v);
                let _ = buf;
            }
            crate::client::View::Map(v) => {
                draw_map(app, buf, area, t, &v);
                app.view = Some(crate::client::View::Map(v));
            }
        }
        if app.view.as_ref().is_some_and(|v| !matches!(v, crate::client::View::Changes(_) | crate::client::View::Pr(_) | crate::client::View::Files(_))) {
            return;
        }
    }
    let Some(focus) = app.focused() else {
        let nk = k(app, &Action::ShellHere);
        put(f.buffer_mut(), area.x + 4, area.y + 3, &[seg(format!("Nothing open. Press {} {nk} for a new session.", app.keymap.prefix.to_string().replace("C-", "Ctrl+")), Style::default().fg(t.muted))], area.right());
        return;
    };
    // Tabs, once there's more than one.
    // Which tab was on screen last, per session.
    app.hy.tick += 1;
    let (tick, cur) = (app.hy.tick, app.hy.tab);
    if let Some(tab) = app.hy.tabs.get_mut(cur) {
        tab.used = tick;
    }
    // A tab bar once the session you're on has more than one tab.
    let area = if app.session_tabs().len() > 1 {
        draw_tab_bar(app, f.buffer_mut(), Rect { height: 1, ..area }, model, t);
        Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area }
    } else {
        area
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
        draw_session(app, f, area, focus, true, false, model, t);
        return;
    };
    let leaves = layout.leaves();
    let arrange = app.hy.tabs.get(app.hy.tab).map(|tab| tab.arrange).unwrap_or_default();
    // Split with two: where you drag the line (below). Anything else: by the arrangement.
    if leaves.len() >= 3 || arrange != Arrange::Split {
        let rects = arranged(arrange, &leaves, focus, area);
        // A line in the middle of each gutter between panes side by side.
        let buf = f.buffer_mut();
        for (_, r) in &rects {
            let gx = r.right() + 1;
            if r.right() + 3 <= area.right() && rects.iter().any(|(_, o)| o.x == r.right() + 3 && o.y < r.bottom() && r.y < o.bottom()) {
                for yy in r.top()..r.bottom() {
                    if let Some(px) = buf.cell_mut((gx, yy)) {
                        px.set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
                    }
                }
            }
        }
        app.hy.leaf_rects = rects.clone();
        for (id, r) in rects {
            draw_session(app, f, r, id, id == focus, true, model, t);
        }
        return;
    }
    // Two: each in its part, a gutter between them (space │ space), the line drags.
    let rects = layout.rects(area);
    app.hy.leaf_rects = rects.clone();
    for (id, r) in rects {
        let mut r = r;
        if r.right() < area.right() {
            r.width = r.width.saturating_sub(3);
        }
        if r.bottom() < area.bottom() {
            r.height = r.height.saturating_sub(1);
        }
        draw_session(app, f, r, id, id == focus, true, model, t);
    }
    for (i, (sa, horizontal, path)) in layout.splits(area).into_iter().enumerate() {
        let ratio = layout.ratio_at(&path).unwrap_or(0.5);
        let div = crate::layout::Node::divider(sa, horizontal, ratio);
        // Side by side: the line in the middle of the three-column gutter, and the whole
        // gutter grabs it (one column is hard to hit). Stacked: a blank row (it shows when
        // you point at it). What lights up is exactly what a press grabs.
        let (div, grab) = if horizontal {
            let line = Rect { x: div.x.saturating_sub(1), ..div };
            (line, Rect { x: line.x.saturating_sub(1), width: 3, ..line })
        } else {
            (div, div)
        };
        let c = if app.hy.drag == Some(Drag::Divider(i)) || hovered(app, grab) { t.accent } else { t.line };
        let buf = f.buffer_mut();
        for yy in div.top()..div.bottom() {
            for xx in div.left()..div.right() {
                if (horizontal || c == t.accent)
                    && let Some(px) = buf.cell_mut((xx, yy)) {
                        px.set_symbol(if horizontal { "│" } else { "─" }).set_style(Style::default().fg(c).bg(t.bg));
                    }
            }
        }
        hit(app, grab, HyHit::Divider(i));
        app.hy.dividers.push((sa, horizontal, path));
    }
}

/// The ⋯ that opens a sidebar row's menu (for terminals that keep right-clicks).
pub(in crate::client) fn row_menu_button(app: &mut App, buf: &mut Buffer, r: Rect, bg: Color, t: &Theme, h: HyHit) {
    let st = if hovered(app, r) { Style::default().bg(t.btn).fg(t.strong) } else { Style::default().bg(bg).fg(t.muted) };
    put(buf, r.x, r.y, &[seg("⋯ ", st)], r.right());
    hit(app, r, h);
}

/// One chip per tab: what it shows; the one you're in is filled. A + makes another.
pub(in crate::client) fn draw_tab_bar(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    fill(buf, r, t.bg);
    let mut x = r.x + 1;
    let mut shown = 0;
    for i in app.session_tabs() {
        let tab = &app.hy.tabs[i];
        shown += 1;
        let n = tab.layout.leaves().len();
        // A session's other tabs have no row of their own: say what's in them from the pane.
        let name = find(model, tab.focus).map(|(_, _, s)| if s.title == WAITING || s.title.is_empty() { s.agent.clone() } else { format!("{} · {}", s.agent, truncate(&s.title, 18)) }).unwrap_or_else(|| {
            app.snap.terms.get(&tab.focus).map(|i| match &i.agent {
                Some(a) => a.clone(),
                None if i.is_shell() => "shell".into(),
                None => i.display_name(),
            }).unwrap_or_else(|| "…".into())
        });
        let label = if n > 1 { format!(" {shown} {name} +{} ", n - 1) } else { format!(" {shown} {name} ") };
        let w = label.width() as u16;
        if x + w + 4 > r.right() {
            break;
        }
        let cr = Rect { x, y: r.y, width: w, height: 1 };
        let on = i == app.hy.tab;
        let st = if on {
            Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
        } else if hovered(app, cr) {
            Style::default().bg(t.hov).fg(t.strong)
        } else {
            Style::default().bg(t.btn).fg(t.text)
        };
        put(buf, x, r.y, &[seg(label, st)], r.right());
        hit(app, cr, HyHit::TabPick(i));
        x += w;
        if on {
            let xr = Rect { x, y: r.y, width: 2, height: 1 };
            put(buf, x, r.y, &[seg("✕ ", st)], r.right());
            hit(app, xr, HyHit::TabClose(i));
            x += 2;
        }
        x += 1;
    }
    let pr = Rect { x, y: r.y, width: 3, height: 1 };
    put(buf, x, r.y, &[seg(" + ", if hovered(app, pr) { Style::default().bg(t.hov).fg(t.strong) } else { Style::default().bg(t.btn).fg(t.muted) })], r.right());
    hit(app, pr, HyHit::TabNew);
}

#[allow(clippy::too_many_arguments)]
pub(in crate::client) fn draw_session(app: &mut App, f: &mut Frame, r: Rect, term: TermId, focused: bool, split: bool, model: &[Proj], t: &Theme) {
    let Some(info) = app.snap.terms.get(&term).cloned() else { return };
    let found = find(model, term);
    let (title, wt, agent) = found
        .map(|(_, w, s)| (s.title.clone(), if w.main { w.branch.clone() } else { w.name.clone() }, s.agent.clone()))
        .unwrap_or_else(|| {
            // A pane beside another in a split has no sidebar row of its own: say what it is
            // from the pane itself.
            let agent = match &info.agent {
                Some(a) => a.clone(),
                None if info.is_shell() => "shell".into(),
                None => info.display_name(),
            };
            (String::new(), String::new(), agent)
        });
    let st = info.status;
    let _ = (split, &title, &wt);
    // While the sidebar has the keys, no pane shows as focused.
    let focused = focused && app.mode != Mode::Side;
    // Title bar: name and where on the left (project · branch, cut with … before the right
    // side); state and ✕ on the right. The focused pane's bar is the accent.
    let bg = if focused { t.accent } else { t.sidebar_bg };
    let ink = |c: Color| if focused { t.acc_ink } else { c };
    fill(f.buffer_mut(), Rect { height: 1, ..r }, bg);
    let mut right: Vec<Seg> = Vec::new();
    if let Some(n) = app.scroll.get(&term) {
        right.push(seg(format!("↑{n}   "), Style::default().fg(ink(t.accent)).bg(bg)));
    }
    if info.agent.is_some() {
        let u = &info.usage;
        let mut used = Vec::new();
        if let Some(c) = u.context {
            used.push(seg(format!("ctx {c:.0}%"), Style::default().fg(if focused { t.acc_ink } else { fullness(t, c) }).bg(bg)));
        }
        if let Some(c) = u.cost.filter(|c| *c >= 0.01) {
            used.push(seg(format!("{}${c:.2}", if used.is_empty() { "" } else { " · " }), Style::default().fg(ink(t.muted)).bg(bg)));
        }
        if !used.is_empty() {
            used.push(seg("   ", Style::default().bg(bg)));
            right.extend(used);
        }
        if let Some(at) = info.resume_at {
            right.push(seg(format!("⏸ limit · says continue in {}   ", until(at)), Style::default().fg(ink(t.working)).bg(bg)));
        }
        let mut s = Style::default().fg(ink(t.status(st))).bg(bg);
        if st == Status::Blocked {
            s = s.add_modifier(Modifier::BOLD);
        }
        let extra = if st == Status::Working { format!(" {}", age(info.since)) } else { String::new() };
        right.push(seg(format!("{} {}{extra}   ", glyph(app, st), state_label(st)), s));
    }
    let xr = Rect { x: r.right().saturating_sub(2), y: r.y, width: 1, height: 1 };
    right.push(seg("✕", Style::default().fg(if hovered(app, xr) { t.err } else { ink(t.muted) }).bg(bg)));
    let rw = segs_width(&right);
    let (name, place) = found
        .map(|(p, w, s)| {
            let br = if p.git && !w.branch.is_empty() { format!(" · {}", w.branch) } else { String::new() };
            let model = if s.model.is_empty() { String::new() } else { format!(" · {}", s.model) };
            // What it's on (Claude's title for the conversation, or its first task) first.
            let on = if s.is_agent && s.title != WAITING && !s.title.is_empty() && s.title != s.name { format!("{} · ", s.title) } else { String::new() };
            (s.name.clone(), format!("{on}{}{br}{model}", p.name))
        })
        .unwrap_or_else(|| {
            let br = info.branch.as_ref().map(|b| format!(" · {b}")).unwrap_or_default();
            (agent.clone(), format!("{}{br}", folder_name(&info.cwd)))
        });
    let icon = if info.agent.is_some() { format!("{} ", kind_icon(app, &agent, true)) } else { String::new() };
    let room = (r.width as usize).saturating_sub(rw as usize + 4 + icon.chars().count() + name.chars().count() + 2);
    let place = if place.chars().count() > room { format!("{}…", place.chars().take(room.saturating_sub(1)).collect::<String>()) } else { place };
    put(
        f.buffer_mut(),
        r.x + 1,
        r.y,
        &[
            seg(format!("{icon}{name}"), Style::default().fg(ink(t.strong)).bg(bg).add_modifier(Modifier::BOLD)),
            seg(format!("  {place}"), Style::default().fg(ink(t.muted)).bg(bg)),
        ],
        r.right().saturating_sub(rw + 2),
    );
    put(f.buffer_mut(), r.right().saturating_sub(rw + 1), r.y, &right, r.right());
    app.pane_frames.push((term, r));
    hit(app, Rect { x: r.right().saturating_sub(3), y: r.y, width: 3, height: 1 }, HyHit::CloseSplit(term));

    // A blank row under the bar; output starts two cells in.
    let ask = st == Status::Blocked && info.agent.is_some();
    let bot = r.bottom().saturating_sub(if ask { 2 } else { 0 });
    let top = r.y + 2;
    let inner = Rect { x: r.x + 2, y: top, width: r.width.saturating_sub(3), height: bot.saturating_sub(top) };
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

    // A scrollbar in the margin when there's history: where you are, click or drag it.
    let (cur, total) = app.history(term);
    if total > 0 && inner.height > 2 {
        let track = Rect { x: inner.right(), y: inner.y, width: 1, height: inner.height };
        let h = track.height as usize;
        let thumb = (h * h / (h + total)).clamp(1, h);
        let top = track.y + ((h - thumb) * (total - cur) / total) as u16;
        let hot = app.hy.drag == Some(Drag::Scroll(term)) || hovered(app, track);
        for yy in track.top()..track.bottom() {
            let on = yy >= top && yy < top + thumb as u16;
            let (sym, c) = if on { ("┃", if hot { t.accent } else { t.muted }) } else { ("│", t.line) };
            if let Some(px) = f.buffer_mut().cell_mut((track.x, yy)) {
                px.set_symbol(sym).set_style(Style::default().fg(c).bg(t.bg));
            }
        }
        if app.hy.drag.is_none() || app.hy.drag == Some(Drag::Scroll(term)) {
            app.hy.bar = Some((term, track, total));
        }
        hit(app, track, HyHit::ScrollBar(term));
    }

    // Scrolled up: say so, and how to get back.
    if let Some(n) = app.scroll.get(&term).copied() {
        let note = vec![
            seg(format!(" ↑ {n} lines up "), Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)),
            seg(" type, or scroll down, to go back ", Style::default().bg(t.card2).fg(t.text)),
        ];
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

    // Answer bar: the agent's own numbered choices, so a key sends the same keystroke.
    if ask && bot + 2 <= r.bottom() {
        let bot = r.bottom() - 1;
        fill(f.buffer_mut(), Rect { x: r.x, y: bot, width: r.width, height: 1 }, t.card2);
        let opts = app.answer_options(term);
        let need: u16 = opts.iter().take(4).map(|o| o.chars().count() as u16 + 5).sum();
        // In a narrow pane the label shrinks to its dot.
        let label = if r.width.saturating_sub(4 + need) >= 10 { "● answer   " } else { "● " };
        let mut x = put(f.buffer_mut(), r.x + 2, bot, &[seg(label, Style::default().fg(t.blocked).bg(t.card2).add_modifier(Modifier::BOLD))], r.right());
        for (i, o) in opts.iter().enumerate().take(4) {
            let key = char::from_digit(i as u32 + 1, 10).unwrap_or('1');
            let kind = if i == 0 { BtnKind::Primary } else { BtnKind::Normal };
            let w = segs_width(&button(t, o, &key.to_string(), kind, false));
            let br = Rect { x, y: bot, width: w, height: 1 };
            let segs = button(t, o, &key.to_string(), kind, hovered(app, br));
            x = put(f.buffer_mut(), x, bot, &segs, r.right()) + 1;
            app.hits.push((br, Hit::Button(crate::client::Btn::Answer(term, key))));
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
    let text = truncate(&msg, (panes.width.saturating_sub(12) as usize).saturating_sub(hint.width()));
    let w = text.width() as u16 + hint.width() as u16 + 7;
    if panes.height < 4 || panes.width < w + 2 {
        return;
    }
    // Top right, under the title bar: away from where you type (an agent's prompt is at
    // the bottom).
    let r = Rect { x: panes.right().saturating_sub(w + 2), y: panes.y + 1, width: w, height: 3 };
    fill(buf, r, t.card2);
    let edge = Style::default().fg(if err { t.err } else { t.done }).bg(t.card2);
    for y in r.top()..r.bottom() {
        if let Some(px) = buf.cell_mut((r.x, y)) {
            px.set_symbol("▌").set_style(edge);
        }
    }
    let st = Style::default().bg(t.card2);
    put(
        buf,
        r.x + 2,
        r.y + 1,
        &[
            seg(if err { "✕ " } else { "✓ " }, st.fg(if err { t.err } else { t.done }).add_modifier(Modifier::BOLD)),
            seg(text, st.fg(t.strong).add_modifier(Modifier::BOLD)),
            seg(hint, st.fg(t.muted)),
        ],
        r.right() - 1,
    );
    if let Some(term) = about {
        hit(app, r, HyHit::Session(term));
    }
}

/// "72% " in front of an agent's row once its context is getting full.
fn context_tag(s: &Session, st: Style, ink: Option<Color>, t: &Theme) -> Vec<Seg> {
    match s.context.filter(|c| *c >= CONTEXT_SHOWN_FROM) {
        Some(c) => vec![seg(format!("{c:.0}% "), st.fg(ink.unwrap_or(fullness(t, c))))],
        None => Vec::new(),
    }
}

/// The footer's plan limits and spend: "claude 5h 23% · week 41%   codex week 12%   $4.20 today".
fn limits_line(app: &App, t: &Theme, s: Style) -> Vec<Seg> {
    let mut out = Vec::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for (agent, limits) in &app.snap.limits {
        let live: Vec<_> = limits.iter().filter(|l| l.resets_at > now).collect();
        if live.is_empty() {
            continue;
        }
        out.push(seg(format!("{agent} "), s.fg(t.muted)));
        for (i, l) in live.iter().enumerate() {
            let sep = if i == 0 { "" } else { " · " };
            let soon = if l.used >= 70.0 { format!(" ({})", until(l.resets_at)) } else { String::new() };
            out.push(seg(format!("{sep}{} {:.0}%{soon}", l.name, l.used), s.fg(fullness(t, l.used))));
        }
        out.push(seg("   ", s));
    }
    if app.snap.spent_today >= 0.01 {
        out.push(seg(format!("${:.2} today   ", app.snap.spent_today), s.fg(t.muted)));
    }
    out
}

pub(in crate::client) fn draw_status(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let surf = t.sidebar_bg;
    fill(buf, r, surf);
    let s = Style::default().bg(surf);
    // In the sidebar: its keys (the row's own, as its menu has them). Otherwise only what
    // needs you; where you are is on each pane's title bar, and notes pop up as toasts.
    // Leader pressed: the whole bar turns the accent and says what the next key can do.
    if matches!(app.mode, Mode::Prefix { .. }) {
        fill(buf, r, t.accent);
        let ink = Style::default().bg(t.accent).fg(t.acc_ink);
        let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
        let mut row = vec![seg(format!(" {lead} "), ink.add_modifier(Modifier::BOLD | Modifier::REVERSED)), seg("  then:  ", ink)];
        for (a, what) in [
            (Action::GoTo, "go to"),
            (Action::Palette, "palette"),
            (Action::ShellHere, "new session"),
            (Action::SplitRight, "split"),
            (Action::Help, "all keys"),
        ] {
            let key = k(app, &a);
            if !key.is_empty() {
                row.push(seg(key, ink.add_modifier(Modifier::BOLD)));
                row.push(seg(format!(" {what}   "), ink));
            }
        }
        row.push(seg("Esc", ink.add_modifier(Modifier::BOLD)));
        row.push(seg(" cancel", ink));
        put(buf, r.x + 1, r.y, &row, r.right());
        return;
    }
    let left: Vec<Seg> = if app.mode == Mode::Side {
        let items = app.cursor_items();
        let keys = crate::client::menu::menu_keys(&items);
        let mut list: Vec<(String, String)> = vec![("↑↓".into(), "move".into()), ("Enter".into(), "open".into())];
        for ((label, _), k) in items.iter().zip(keys) {
            if let Some(k) = k {
                let l = label.trim_start_matches('▶').trim().trim_end_matches('…').split_whitespace().next().unwrap_or("").to_lowercase();
                list.push((k.to_string(), l));
            }
        }
        list.push(("Esc".into(), "back".into()));
        let refs: Vec<(&str, &str)> = list.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        hints(t, &refs).into_iter().map(|(x, st)| (x, if st.bg.is_none() || st.bg == Some(t.card) { st.bg(surf) } else { st })).collect()
    } else {
        let needs = model.iter().flat_map(|p| p.sessions()).filter(|x| x.status == Status::Blocked).count();
        if needs > 0 {
            vec![seg(format!("● {needs} needs you"), s.fg(t.blocked).add_modifier(Modifier::BOLD)), seg(format!("   {} inbox", k(app, &Action::Jump)), s.fg(t.muted))]
        } else {
            Vec::new()
        }
    };
    let mut right = limits_line(app, t, s);
    if let Some(host) = crate::ipc::remote() {
        right.push(seg(format!(" ⇄ {host} "), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)));
        right.push(seg(" ", s));
    }
    let rw = segs_width(&right);
    // Lined up with the panes, not under the sidebar.
    put(buf, app.hy.crumb_x.max(r.x + 1), r.y, &left, r.right().saturating_sub(rw + 2));
    hit(app, Rect { width: 60.min(r.width), ..r }, HyHit::Jump);
    put(buf, r.right().saturating_sub(rw), r.y, &right, r.right());
    // Under the sidebar: which hydra this is, and an Update button when a newer one is out.
    if app.mode != Mode::Side {
        let mut ver = vec![seg(format!("hydra {}", env!("CARGO_PKG_VERSION")), s.fg(t.muted))];
        if let Some(v) = &app.update_available {
            ver.push(seg(format!(" · {v} is out "), s.fg(t.accent)));
        }
        let end = app.hy.crumb_x.max(r.x + 1).saturating_sub(1);
        let x = put(buf, r.x + 2, r.y, &ver, end);
        if app.update_available.is_some() {
            let label = if app.updating { " updating… " } else { " Update " };
            let w = label.width() as u16;
            if x + w <= end {
                let br = Rect { x, y: r.y, width: w, height: 1 };
                let (bg, fg) = if hovered(app, br) { (t.accent, t.acc_ink) } else { (t.btn, t.strong) };
                put(buf, x, r.y, &[seg(label, Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD))], end);
                hit(app, br, HyHit::Update);
            }
        }
    }
}
