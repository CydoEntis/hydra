//! Full-pane views: pull request, ship, map.

use super::*;

// ---- pull request view ---------------------------------------------------------------------

pub(in crate::client) fn check_glyph(t: &Theme, c: crate::client::pr::Checks) -> Seg {
    use crate::client::pr::Checks;
    match c {
        Checks::Pass => seg("✓", Style::default().fg(t.done)),
        Checks::Fail => seg("✕", Style::default().fg(t.err).add_modifier(Modifier::BOLD)),
        Checks::Pending => seg("…", Style::default().fg(t.muted)),
        Checks::None => seg("·", Style::default().fg(t.muted)),
    }
}

/// The little `#412 ✓` tag for a branch with a pull request.
pub(in crate::client) fn pr_tag(t: &Theme, pr: &crate::client::pr::PrBrief) -> Vec<Seg> {
    use crate::client::pr::Review;
    let mut v = vec![seg(format!(" #{} ", pr.number), Style::default().fg(t.muted)), check_glyph(t, pr.checks)];
    if pr.review == Review::Changes {
        v.push(seg("±", Style::default().fg(t.blocked).add_modifier(Modifier::BOLD)));
    } else if pr.review == Review::Approved {
        v.push(seg("✔", Style::default().fg(t.done)));
    }
    v
}

/// Wrap plain text to `width` columns.
pub(in crate::client) fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

pub(in crate::client) fn draw_pr(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &crate::client::pr::PrView) {
    fill(buf, area, t.bg);
    let title = match &v.info {
        Some(Ok(i)) => format!("#{}  {}", i.number, i.title),
        _ => format!("pull request · {}", v.which),
    };
    crate::client::design::title_bar(
        buf,
        area,
        t,
        &[seg(title, Style::default().add_modifier(Modifier::BOLD))],
        &[seg(if v.tab == 0 { "Tab diff   " } else { "Tab overview   " }, Style::default()), seg("✕ ", Style::default())],
        true,
    );
    hit(app, Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, HyHit::ViewKey('\x1b'));
    let body = Rect { x: area.x + 2, y: area.y + 2, width: area.width.saturating_sub(4), height: area.height.saturating_sub(5) };
    let info = match &v.info {
        None => {
            put(buf, body.x, body.y, &[seg("loading…", Style::default().fg(t.muted))], body.right());
            return;
        }
        Some(Err(e)) => {
            let msg = if e.contains("no pull requests found") { "No pull request for this branch yet. Open one from Changes (d, then p)." } else { e.as_str() };
            put(buf, body.x, body.y, &[seg(msg.to_string(), Style::default().fg(t.muted))], body.right());
            return;
        }
        Some(Ok(i)) => i,
    };
    let w = body.width as usize;
    let head = |s: &str| vec![seg(s.to_string(), Style::default().fg(t.muted).add_modifier(Modifier::BOLD))];
    let mut lines: Vec<Vec<Seg>> = Vec::new();
    if v.tab == 0 {
        let state_c = match info.state.as_str() {
            "MERGED" => t.accent,
            "CLOSED" => t.err,
            _ => t.done,
        };
        lines.push(vec![
            seg(format!(" {} ", info.state.to_lowercase()), Style::default().bg(state_c).fg(t.acc_ink).add_modifier(Modifier::BOLD)),
            seg(format!("  {} → {}", info.branch, info.base), Style::default().fg(t.text)),
            seg(format!("   +{}", info.additions), Style::default().fg(t.done)),
            seg(format!(" −{}", info.deletions), Style::default().fg(t.err)),
            seg(format!(" · {} files · by {}", info.files, info.author), Style::default().fg(t.muted)),
        ]);
        lines.push(vec![]);
        if !info.checks.is_empty() {
            let bad = info.checks.iter().filter(|(_, c)| *c == crate::client::pr::Checks::Fail).count();
            lines.push(head(&format!("CHECKS  {}", if bad > 0 { format!("{bad} failing") } else { format!("{} ok", info.checks.len()) })));
            for (name, c) in &info.checks {
                lines.push(vec![check_glyph(t, *c), seg(format!(" {name}"), Style::default().fg(t.text))]);
            }
            lines.push(vec![]);
        }
        if !info.reviews.is_empty() {
            lines.push(head("REVIEWS"));
            for (who, st, text) in &info.reviews {
                let (label, c) = match st.as_str() {
                    "APPROVED" => ("approved", t.done),
                    "CHANGES_REQUESTED" => ("asked for changes", t.blocked),
                    _ => ("commented", t.muted),
                };
                lines.push(vec![seg(who.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)), seg(format!(" {label}"), Style::default().fg(c))]);
                for l in wrap_text(text, w.saturating_sub(4)) {
                    lines.push(vec![seg(format!("  {l}"), Style::default().fg(t.text))]);
                }
            }
            lines.push(vec![]);
        }
        if !info.comments.is_empty() {
            lines.push(head("COMMENTS"));
            for (who, text) in &info.comments {
                lines.push(vec![seg(who.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD))]);
                for l in wrap_text(text, w.saturating_sub(4)) {
                    lines.push(vec![seg(format!("  {l}"), Style::default().fg(t.text))]);
                }
            }
            lines.push(vec![]);
        }
        lines.push(head("DESCRIPTION"));
        let text = if info.body.trim().is_empty() { "(none)" } else { info.body.as_str() };
        for l in wrap_text(text, w) {
            lines.push(vec![seg(l, Style::default().fg(t.text))]);
        }
    } else {
        match &v.diff {
            None => lines.push(vec![seg("loading the diff…", Style::default().fg(t.muted))]),
            Some(Err(e)) => lines.push(vec![seg(e.clone(), Style::default().fg(t.err))]),
            Some(Ok(d)) => {
                let (mut old, mut new) = (0, 0);
                for l in d.lines() {
                    if let Some(rest) = l.strip_prefix("diff --git a/") {
                        let file = rest.split(" b/").next().unwrap_or(rest).to_string();
                        lines.push(vec![]);
                        lines.push(vec![seg(file, Style::default().fg(t.strong).add_modifier(Modifier::BOLD))]);
                        continue;
                    }
                    if l.starts_with("index ") || l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("new file") || l.starts_with("deleted file") {
                        continue;
                    }
                    lines.push(crate::client::design::diff_line(t, l, &mut old, &mut new));
                }
            }
        }
    }
    let start = (v.scroll as usize).min(lines.len().saturating_sub(1));
    for (i, l) in lines.iter().skip(start).take(body.height as usize).enumerate() {
        put(buf, body.x, body.y + i as u16, l, body.right());
    }
    // Actions
    let y = area.bottom().saturating_sub(1);
    fill(buf, Rect { y, height: 1, ..area }, t.card2);
    let mut x = area.x + 2;
    for (label, key, kind, c) in [
        ("Ask the agent to fix it", "f", BtnKind::Primary, 'f'),
        ("Open in browser", "o", BtnKind::Normal, 'o'),
        ("Reload", "r", BtnKind::Normal, 'r'),
    ] {
        x = btn(app, buf, x, y, label, key, kind, HyHit::ViewKey(c), area.right()) + 1;
    }
    put(buf, x + 1, y, &hints(t, &[("↑↓", "scroll"), ("Tab", if v.tab == 0 { "diff" } else { "overview" }), ("Esc", "close")]).into_iter().map(|(s, st)| (s, st.bg(t.card2))).collect::<Vec<_>>(), area.right());
}

// ---- ship ------------------------------------------------------------------------------------

pub(in crate::client) fn draw_ship(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, ask: &crate::client::ShipAsk) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 66, 12, &format!("Ship {}", ask.task.branch), &[], t);
    let c = Style::default().bg(t.card);
    let mut y = r.y + 2;
    let mut step = |buf: &mut Buffer, n: u8, text: String| {
        put(buf, r.x + 3, y, &[seg(format!("{n}  "), c.fg(t.accent).add_modifier(Modifier::BOLD)), seg(text, c.fg(t.text))], r.right() - 2);
        y += 1;
    };
    let msg = if ask.task.summary.is_empty() { ask.task.branch.clone() } else { ask.task.summary.clone() };
    if ask.changed > 0 {
        step(buf, 1, format!("commit {} changed file{} as \"{}\"", ask.changed, if ask.changed == 1 { "" } else { "s" }, truncate(&msg, 30)));
    } else {
        step(buf, 1, "nothing new to commit".into());
    }
    step(buf, 2, format!("push {}", ask.task.branch));
    match &ask.pr {
        Some(n) => step(buf, 3, format!("update pull request #{n}")),
        None => step(buf, 3, format!("open a pull request into {}", ask.task.base)),
    }
    put(buf, r.x + 3, y + 1, &[seg("Checks then show next to the branch in the sidebar.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
    let gx = btn(app, buf, r.x + 3, r.bottom() - 2, "Ship", "Enter", BtnKind::Primary, HyHit::ShipGo, r.right());
    put(buf, gx + 3, r.bottom() - 2, &hints(t, &[("Esc", "cancel")]), r.right());
}

// ---- map -------------------------------------------------------------------------------------

/// The Map: a project at the top, its main folder and worktrees as boxes under it, each
/// with the agents in it, coloured by what needs you.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MapView {
    /// Project key (None: the current one).
    pub proj: Option<String>,
    pub sel: usize,
}

pub(in crate::client) const BW: u16 = 28;
pub(in crate::client) const BH: u16 = 6;

pub(in crate::client) fn junction(up: bool, down: bool, left: bool, right: bool) -> &'static str {
    match (up, down, left, right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (false, true, true, true) => "┬",
        (true, false, true, true) => "┴",
        (true, true, false, false) => "│",
        (false, false, true, true) => "─",
        (false, true, true, false) => "╮",
        (false, true, false, true) => "╭",
        (true, false, true, false) => "╯",
        (true, false, false, true) => "╰",
        _ => "┼",
    }
}

pub(in crate::client) fn map_project<'a>(app: &App, model: &'a [Proj], v: &MapView) -> Option<&'a Proj> {
    let key = v.proj.clone().or_else(|| app.hy.proj.clone());
    model.iter().find(|p| Some(&p.key) == key.as_ref()).or(model.first())
}

/// The most urgent session in a box decides its colour.
pub(in crate::client) fn wt_status(w: &Wt) -> Option<&Session> {
    w.sessions.iter().filter(|s| s.is_agent).min_by_key(|s| (rank(s.status), s.term))
}

pub(in crate::client) fn draw_map(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &MapView) {
    let model = app.hy_model();
    fill(buf, area, t.bg);
    let Some(p) = map_project(app, &model, v).cloned() else {
        put(buf, area.x + 2, area.y + 2, &[seg("Open a project first (o).", Style::default().fg(t.muted))], area.right());
        return;
    };
    crate::client::design::title_bar(
        buf,
        area,
        t,
        &[seg("map  ", Style::default().add_modifier(Modifier::BOLD)), seg(p.name.clone(), Style::default())],
        &[seg("Tab next project   ", Style::default()), seg("✕ ", Style::default())],
        true,
    );
    hit(app, Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, HyHit::ViewKey('\x1b'));
    let line = |buf: &mut Buffer, x: u16, y: u16, sym: &str, c: Color| {
        if x < area.right() && y < area.bottom()
            && let Some(px) = buf.cell_mut((x, y)) {
                px.set_symbol(sym).set_style(Style::default().fg(c).bg(t.bg));
            }
    };
    // The project at the top.
    let rw = (p.name.width() as u16 + 18).max(24).min(area.width.saturating_sub(4));
    let rx = area.x + area.width.saturating_sub(rw) / 2;
    let ry = area.y + 2;
    let worst = p.sessions().filter(|s| s.is_agent).map(|s| s.status).min_by_key(|s| rank(*s));
    let rc = worst.map(|s| if rank(s) <= 1 { t.status(s) } else { t.line }).unwrap_or(t.line);
    draw_box(buf, Rect { x: rx, y: ry, width: rw, height: 3 }, rc, t);
    let c: Vec<Seg> = counts(app, t, p.sessions(), None);
    let mut head = vec![seg("▌", Style::default().fg(p.color)), seg(p.name.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)), seg("  ", Style::default())];
    head.extend(c);
    put(buf, rx + 2, ry + 1, &head, rx + rw - 1);
    let spine_x = rx + rw / 2;

    let nodes: Vec<&Wt> = p.wts.iter().collect();
    let cols = ((area.width.saturating_sub(4)) / (BW + 2)).max(1) as usize;
    let rows: Vec<&[&Wt]> = nodes.chunks(cols).collect();
    let mut bus_y = ry + 4;
    let mut idx = 0;
    let n_rows = rows.len();
    line(buf, spine_x, ry + 2, "┬", rc);
    for (ri, row) in rows.iter().enumerate() {
        if bus_y + BH + 1 >= area.bottom() {
            break;
        }
        let n = row.len() as u16;
        let row_w = n * BW + (n - 1) * 2;
        let x0 = area.x + area.width.saturating_sub(row_w) / 2;
        let centers: Vec<u16> = (0..n).map(|i| x0 + i * (BW + 2) + BW / 2).collect();
        // The spine down to this row's bus.
        for y in ry + 3..bus_y {
            if buf[(spine_x, y)].symbol() == " " {
                line(buf, spine_x, y, "│", t.line);
            }
        }
        let lo = centers.iter().copied().min().unwrap_or(spine_x).min(spine_x);
        let hi = centers.iter().copied().max().unwrap_or(spine_x).max(spine_x);
        for x in lo..=hi {
            let child = centers.contains(&x);
            let spine = x == spine_x;
            let up = spine;
            let down = child || (spine && ri + 1 < n_rows);
            line(buf, x, bus_y, junction(up, down, x > lo, x < hi), t.line);
            let _ = child;
        }
        for (i, w) in row.iter().enumerate() {
            let bx = x0 + i as u16 * (BW + 2);
            let r = Rect { x: bx, y: bus_y + 1, width: BW, height: BH };
            let sel = idx == v.sel;
            let top = wt_status(w);
            let col = if sel {
                t.accent
            } else {
                match top {
                    Some(s) if s.asleep => t.muted,
                    Some(s) if s.status == Status::Blocked => t.blocked,
                    Some(s) if s.status == Status::Done => t.done,
                    Some(s) if s.status == Status::Working => t.text,
                    _ => t.line,
                }
            };
            draw_box(buf, r, col, t);
            line(buf, r.x + BW / 2, r.y, "┴", col);
            let inner = r.right() - 1;
            let mut title = vec![
                seg(if w.main { "⎇ " } else { "⑂ " }, Style::default().fg(t.muted)),
                seg(truncate(&w.name, (BW - 6) as usize), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
            ];
            if let Some(pr) = p.prs.iter().find(|pr| pr.branch == w.branch) {
                title.extend(pr_tag(t, pr));
            }
            put(buf, r.x + 2, r.y + 1, &title, inner);
            if !w.main || !w.branch.is_empty() {
                put(buf, r.x + 2, r.y + 2, &[seg(truncate(&w.branch, (BW - 4) as usize), Style::default().fg(t.muted))], inner);
            }
            if w.sessions.is_empty() {
                put(buf, r.x + 2, r.y + 3, &[seg("no agent", Style::default().fg(t.muted))], inner);
            }
            for (y, s) in (r.y + 3..).zip(w.sessions.iter().take(2)) {
                let gl = if s.asleep { "☾".into() } else if s.is_agent { glyph(app, s.status) } else { app.cfg.icons.shell.clone() };
                let what = if s.asleep { "asleep".to_string() } else if s.is_agent { format!("{} {}", state_label(s.status), age(s.since)) } else { String::new() };
                put(
                    buf,
                    r.x + 2,
                    y,
                    &[
                        seg(format!("{gl} "), Style::default().fg(if s.is_agent { t.status(s.status) } else { t.muted })),
                        seg(format!("{:<7}", truncate(&s.agent, 7)), Style::default().fg(t.text)),
                        seg(what, Style::default().fg(if s.status == Status::Blocked { t.blocked } else { t.muted })),
                    ],
                    inner,
                );
            }
            if w.sessions.len() > 2 {
                put(buf, inner.saturating_sub(4), r.y + BH - 2, &[seg(format!("+{}", w.sessions.len() - 2), Style::default().fg(t.muted))], inner);
            }
            hit(app, r, HyHit::MapNode(idx));
            idx += 1;
        }
        bus_y += BH + 2;
    }
    let y = area.bottom().saturating_sub(1);
    fill(buf, Rect { y, height: 1, ..area }, t.card2);
    put(
        buf,
        area.x + 2,
        y,
        &cap_hints(t, t.card2, &[("←→↑↓", "choose"), ("Enter", "open"), ("Tab", "next project"), ("Esc", "close")]),
        area.right(),
    );
}

pub(in crate::client) fn draw_box(buf: &mut Buffer, r: Rect, c: Color, t: &Theme) {
    let st = Style::default().fg(c).bg(t.bg);
    for x in r.x..r.right() {
        if let Some(px) = buf.cell_mut((x, r.y)) {
            px.set_symbol("─").set_style(st);
        }
        if let Some(px) = buf.cell_mut((x, r.bottom() - 1)) {
            px.set_symbol("─").set_style(st);
        }
    }
    for y in r.y..r.bottom() {
        if let Some(px) = buf.cell_mut((r.x, y)) {
            px.set_symbol("│").set_style(st);
        }
        if let Some(px) = buf.cell_mut((r.right() - 1, y)) {
            px.set_symbol("│").set_style(st);
        }
    }
    if let Some(px) = buf.cell_mut((r.x, r.y)) {
        px.set_symbol("╭");
    }
    if let Some(px) = buf.cell_mut((r.right() - 1, r.y)) {
        px.set_symbol("╮");
    }
    if let Some(px) = buf.cell_mut((r.x, r.bottom() - 1)) {
        px.set_symbol("╰");
    }
    if let Some(px) = buf.cell_mut((r.right() - 1, r.bottom() - 1)) {
        px.set_symbol("╯");
    }
}

impl App {
    pub(in crate::client) fn open_map(&mut self) {
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.view = Some(crate::client::View::Map(Box::new(MapView { proj: self.hy.proj.clone(), sel: 0 })));
    }

    /// Returns false when the map closes.
    pub(in crate::client) fn on_map_key(&mut self, v: &mut MapView, k: &KeyEvent) -> bool {
        let model = self.hy_model();
        let Some(p) = map_project(self, &model, v).cloned() else { return k.code != KeyCode::Esc };
        let n = p.wts.len();
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Down | KeyCode::Char('j') => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Up | KeyCode::Char('k') => v.sel = v.sel.saturating_sub(1),
            KeyCode::Tab => {
                let i = model.iter().position(|x| x.key == p.key).unwrap_or(0);
                v.proj = model.get((i + 1) % model.len().max(1)).map(|x| x.key.clone());
                v.sel = 0;
            }
            KeyCode::Enter => {
                if let Some(w) = p.wts.get(v.sel) {
                    match w.sessions.iter().min_by_key(|s| (rank(s.status), s.term)) {
                        Some(s) => self.hy_focus(s.term),
                        None if w.main => self.hy_new_session(w.path.clone(), None, false),
                        None => {
                            let agent = self.hy_agent();
                            self.hy_new_session(w.path.clone(), Some(agent), false);
                        }
                    }
                    return false;
                }
            }
            _ => {}
        }
        true
    }
}
