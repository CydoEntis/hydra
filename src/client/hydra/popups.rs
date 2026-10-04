//! Popups: shared panel helpers, jump, open project, palette, go to, history, memory.

use super::*;

// ---- overlays ------------------------------------------------------------------------------

/// Dim everything already drawn, gently: still readable behind a popup.
pub(in crate::client) fn dim_all(buf: &mut Buffer, area: Rect, t: &Theme) {
    let black = Color::Rgb(0, 0, 0);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let c = &mut buf[(x, y)];
            let bg = if c.bg == Color::Reset { t.bg } else { c.bg };
            let fg = if c.fg == Color::Reset { t.fg } else { c.fg };
            c.fg = blend(blend(fg, bg, 0.35), black, 0.2);
            c.bg = blend(bg, black, 0.2);
        }
    }
}

/// A centred panel: `card` ground, accent title bar with "Esc close" on the right.
#[allow(clippy::too_many_arguments)]
/// The one size every tool window opens at (files, search, changes, pull requests).
pub(in crate::client) fn tool_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(170);
    let h = area.height.saturating_sub(4).min(48);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::client) fn panel(app: &mut App, buf: &mut Buffer, area: Rect, w: u16, h: u16, title: &str, right: &[Seg], t: &Theme) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    hit(app, area, HyHit::Close);
    fill(buf, r, t.card);
    hit(app, r, HyHit::Noop);
    let bar = Rect { height: 1, ..r };
    fill(buf, bar, t.accent);
    let ink = Style::default().fg(t.acc_ink).bg(t.accent);
    put(buf, r.x + 2, r.y, &[seg(title, ink.add_modifier(Modifier::BOLD))], r.right());
    let right: Vec<Seg> = if right.is_empty() {
        vec![seg("Esc", ink.add_modifier(Modifier::BOLD)), seg(" close ", ink)]
    } else {
        right.iter().map(|(s, st)| (s.clone(), st.fg(t.acc_ink).bg(t.accent))).collect()
    };
    let rw = segs_width(&right);
    put(buf, r.right().saturating_sub(rw + 1), r.y, &right, r.right());
    hit(app, Rect { x: r.right().saturating_sub(rw.max(10) + 1), y: r.y, width: rw.max(10), height: 1 }, HyHit::Close);
    r
}

pub(in crate::client) fn sel_row(app: &App, buf: &mut Buffer, r: Rect, y: u16, sel: bool, t: &Theme) -> Color {
    let row = Rect { x: r.x + 1, y, width: r.width.saturating_sub(2), height: 1 };
    if sel || hovered(app, row) {
        fill(buf, row, t.hov);
        if sel {
            put(buf, r.x + 1, y, &[seg(">", Style::default().fg(t.accent).bg(t.hov).add_modifier(Modifier::BOLD))], r.right());
        }
        t.hov
    } else {
        t.card
    }
}

pub(in crate::client) fn hints(t: &Theme, pairs: &[(&str, &str)]) -> Vec<Seg> {
    cap_hints(t, t.card, pairs)
}

// Jump ----------------------------------------------------------------------------------------

pub(in crate::client) fn jump_list(model: &[Proj]) -> Vec<(Session, String, String, Color)> {
    let mut out = Vec::new();
    for st in [Status::Blocked, Status::Done] {
        for p in model {
            for w in &p.wts {
                for s in w.sessions.iter().filter(|s| s.status == st) {
                    out.push((s.clone(), p.name.clone(), w.name.clone(), p.color));
                }
            }
        }
    }
    // Nothing waiting: the most recent sessions instead of an empty box.
    if out.is_empty() {
        let mut all: Vec<(Session, String, String, Color)> =
            model.iter().flat_map(|p| p.wts.iter().flat_map(move |w| w.sessions.iter().map(move |s| (s.clone(), p.name.clone(), w.name.clone(), p.color)))).collect();
        all.sort_by_key(|(s, ..)| std::cmp::Reverse(s.since));
        out = all.into_iter().take(9).collect();
    }
    out
}

/// Your pull requests with failing checks or changes requested: (folder, PR, project, colour).
pub(in crate::client) fn jump_prs(model: &[Proj]) -> Vec<(PathBuf, crate::client::pr::PrBrief, String, Color)> {
    model
        .iter()
        .flat_map(|p| {
            p.prs.iter().filter(|pr| pr.needs_you()).map(move |pr| {
                let dir = p.wts.iter().find(|w| w.branch == pr.branch).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
                (dir, pr.clone(), p.name.clone(), p.color)
            })
        })
        .collect()
}

pub(in crate::client) fn draw_jump(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let model = app.hy_model();
    let list = jump_list(&model);
    let prs = jump_prs(&model);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (list.len() as u16 + prs.len() as u16 + 12).max(10);
    let r = panel(app, buf, area, 80, h, "Jump to", &[], t);
    let mut y = r.y + 2;
    let mut i = 0;
    let waiting = list.iter().any(|(s, ..)| matches!(s.status, Status::Blocked | Status::Done));
    if !waiting {
        put(buf, r.x + 3, y, &[seg("✓ ", Style::default().fg(t.done).bg(t.card).add_modifier(Modifier::BOLD)), seg("Nothing needs you right now.", Style::default().fg(t.strong).bg(t.card))], r.right());
        y += 2;
    }
    let groups: Vec<(Option<Status>, &str)> = if waiting { vec![(Some(Status::Blocked), "NEEDS YOU"), (Some(Status::Done), "DONE · NOT REVIEWED")] } else { vec![(None, "RECENT")] };
    for (st, label) in groups {
        let rows: Vec<_> = list.iter().filter(|(s, ..)| st.is_none_or(|x| s.status == x)).collect();
        let st = st.unwrap_or(Status::None);
        if rows.is_empty() {
            continue;
        }
        put(buf, r.x + 3, y, &[seg(label, Style::default().fg(if st == Status::Blocked { t.blocked } else { t.muted }).bg(t.card).add_modifier(Modifier::BOLD))], r.right());
        y += 1;
        for (s, pname, wname, pc) in rows {
            if y >= r.bottom().saturating_sub(2) {
                break;
            }
            let bg = sel_row(app, buf, r, y, i == sel, t);
            let st_ = Style::default().bg(bg);
            let mut ts = st_.fg(t.strong);
            if i == sel {
                ts = ts.add_modifier(Modifier::BOLD);
            }
            let rseg = vec![
                seg("▌", st_.fg(*pc)),
                seg(pname.clone(), st_.fg(t.text)),
                seg(format!(" › {wname}  {}  {}", s.agent, age(s.since)), st_.fg(t.muted)),
            ];
            let rw = segs_width(&rseg);
            put(
                buf,
                r.x + 3,
                y,
                &[
                    seg(format!("{}  ", i + 1), st_.fg(t.accent).add_modifier(Modifier::BOLD)),
                    seg(format!("{} ", glyph(app, s.status)), st_.fg(t.status(s.status)).add_modifier(Modifier::BOLD)),
                    seg(s.title.clone(), ts),
                ],
                r.right().saturating_sub(rw + 3),
            );
            put(buf, r.right().saturating_sub(rw + 2), y, &rseg, r.right());
            hit(app, Rect { x: r.x + 1, y, width: r.width - 2, height: 1 }, HyHit::JumpTo(s.term));
            y += 1;
            i += 1;
        }
        y += 1;
    }
    if !prs.is_empty() && y < r.bottom().saturating_sub(3) {
        put(buf, r.x + 3, y, &[seg("PULL REQUESTS", Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::BOLD))], r.right());
        y += 1;
        for (dir, pr, pname, pc) in &prs {
            if y >= r.bottom().saturating_sub(2) {
                break;
            }
            let bg = sel_row(app, buf, r, y, i == sel, t);
            let st_ = Style::default().bg(bg);
            let mut row = vec![seg(format!("{}  ", i + 1), st_.fg(t.accent).add_modifier(Modifier::BOLD))];
            row.extend(pr_tag(t, pr).into_iter().map(|(x, s2)| (x, s2.bg(bg))));
            row.push(seg(format!("  {}", pr.title), st_.fg(t.strong)));
            let rseg = vec![seg("▌", st_.fg(*pc)), seg(pname.clone(), st_.fg(t.text)), seg(format!("  {}", pr.state_text()), st_.fg(if pr.checks == crate::client::pr::Checks::Fail { t.err } else { t.blocked }))];
            let rw = segs_width(&rseg);
            put(buf, r.x + 3, y, &row, r.right().saturating_sub(rw + 3));
            put(buf, r.right().saturating_sub(rw + 2), y, &rseg, r.right());
            let k = app.hy.pr_keys.len();
            app.hy.pr_keys.push((dir.clone(), pr.number.to_string()));
            hit(app, Rect { x: r.x + 1, y, width: r.width - 2, height: 1 }, HyHit::Pr(k));
            y += 1;
            i += 1;
        }
    }
    if list.is_empty() && prs.is_empty() {
        put(buf, r.x + 3, y, &[seg("Nothing running yet.", Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::ITALIC))], r.right());
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("1-9", "jump"), ("Enter", "jump"), ("↑↓", "choose"), ("Esc", "close")]), r.right());
}

// Open a project -----------------------------------------------------------------------------

impl Finder {
    pub fn new(start: &Path) -> Finder {
        let sep = std::path::MAIN_SEPARATOR;
        let mut f = Finder { q: format!("{}{sep}", tilde(start)), sel: 0, dir: PathBuf::new(), list: Vec::new() };
        f.refresh();
        f
    }

    /// The typed text as (folder, the part being typed in it).
    pub(in crate::client) fn split(&self) -> (PathBuf, String) {
        let sep = std::path::MAIN_SEPARATOR;
        let mut q = self.q.replace(['/', '\\'], &sep.to_string());
        if let Some(rest) = q.strip_prefix("cd ") {
            q = rest.to_string();
        }
        let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_default();
        let full = if let Some(rest) = q.strip_prefix('~') {
            format!("{}{rest}", home.display())
        } else if Path::new(&q).is_absolute() || q.get(1..2) == Some(":") {
            q
        } else {
            format!("{}{sep}{q}", home.display())
        };
        let (dir, part) = match full.rfind(sep) {
            Some(i) => (&full[..=i], full[i + 1..].to_string()),
            None => (full.as_str(), String::new()),
        };
        (normalize(Path::new(dir)), part)
    }

    /// Re-list the folder being typed in.
    pub fn refresh(&mut self) {
        let (dir, part) = self.split();
        if dir != self.dir {
            self.dir = dir.clone();
            let mut list: Vec<(String, bool)> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| !n.starts_with('.') && !n.starts_with('$'))
                        .map(|n| {
                            let repo = dir.join(&n).join(".git").exists();
                            (n, repo)
                        })
                        .collect()
                })
                .unwrap_or_default();
            list.sort_by_key(|(n, _)| n.to_lowercase());
            self.list = list;
        }
        let _ = part;
        self.sel = self.sel.min(self.rows().len().saturating_sub(1));
    }

    /// Rows shown: "." (open this folder), ".." (up), then matching subfolders.
    pub fn rows(&self) -> Vec<(String, PathBuf, bool)> {
        let (dir, part) = self.split();
        let p = part.to_lowercase();
        let fuzzy = |n: &str| {
            let mut it = p.chars().peekable();
            for c in n.to_lowercase().chars() {
                if it.peek() == Some(&c) {
                    it.next();
                }
            }
            it.peek().is_none()
        };
        let mut out = Vec::new();
        if part.is_empty() {
            out.push((".".to_string(), dir.clone(), dir.join(".git").exists()));
        }
        if dir.parent().is_some() && (part.is_empty() || "..".starts_with(&part)) {
            out.push(("..".to_string(), dir.parent().map(Path::to_path_buf).unwrap_or_default(), false));
        }
        let mut kids: Vec<_> = self.list.iter().filter(|(n, _)| part == ".." || fuzzy(n)).collect();
        kids.sort_by_key(|(n, _)| !n.to_lowercase().starts_with(&p));
        out.extend(kids.into_iter().map(|(n, repo)| (n.clone(), dir.join(n), *repo)));
        out
    }

    /// The rest of the selected name, shown greyed after what's typed.
    pub(in crate::client) fn ghost(&self) -> String {
        let (_, part) = self.split();
        let rows = self.rows();
        match rows.get(self.sel) {
            Some((n, ..)) if !part.is_empty() && n.to_lowercase().starts_with(&part.to_lowercase()) => n[part.len()..].to_string(),
            _ => String::new(),
        }
    }
}

/// Resolve `.` and `..` without touching the disk.
pub(in crate::client) fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            std::path::Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

pub(in crate::client) fn draw_finder(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, fd: &Finder) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 92, 24, "Open a project", &[], t);
    let input = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: 1 };
    fill(buf, input, t.card2);
    let s = Style::default().bg(t.card2);
    // A long path shows its end, where you're typing.
    let room = r.width.saturating_sub(10) as usize;
    let ghost = fd.ghost();
    let q = if fd.q.width() + ghost.width() > room {
        let tail: String = fd.q.chars().rev().take(room.saturating_sub(ghost.width() + 1)).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    } else {
        fd.q.clone()
    };
    put(
        buf,
        r.x + 3,
        input.y,
        &[seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(q, s.fg(t.strong)), seg(ghost, s.fg(t.muted)), seg("█", s.fg(t.accent))],
        r.right() - 1,
    );
    let c = Style::default().bg(t.card);
    put(
        buf,
        r.x + 3,
        r.y + 3,
        &[seg("type a path, ", c.fg(t.muted)), seg("cd ..", c.fg(t.text)), seg(" works too · fuzzy: ", c.fg(t.muted)), seg("~\\c\\sa", c.fg(t.text)), seg(" finds ~\\code\\shop-api", c.fg(t.muted))],
        r.right(),
    );
    put(buf, r.x + 3, r.y + 5, &[seg(tilde(&fd.dir), c.fg(t.muted).add_modifier(Modifier::BOLD))], r.right());
    let model = app.hy_model();
    let rows = fd.rows();
    let max = (r.height.saturating_sub(9)) as usize;
    let start = fd.sel.saturating_sub(max.saturating_sub(1));
    for (i, (n, path, repo)) in rows.iter().enumerate().skip(start).take(max) {
        let y = r.y + 6 + (i - start) as u16;
        let sel = i == fd.sel;
        let bg = sel_row(app, buf, r, y, sel, t);
        let st = Style::default().bg(bg);
        let open = model.iter().any(|p| path_key(&p.path) == path_key(path));
        let (icon, label) = match n.as_str() {
            "." => ("◇ ", format!("open {} here", folder_name(path))),
            ".." => ("↰ ", format!(".. (up to {})", tilde(path))),
            _ => (if *repo { "◆ " } else { "▸ " }, n.clone()),
        };
        let mut ls = st.fg(if sel { t.strong } else { t.fg });
        if sel {
            ls = ls.add_modifier(Modifier::BOLD);
        }
        put(buf, r.x + 3, y, &[seg(icon, st.fg(if *repo || n == "." { t.accent } else { t.muted })), seg(label, ls)], r.right() - 30);
        let tag = if open {
            "open project"
        } else if n == "." {
            if *repo { "git repo · opens as a project" } else { "opens as a project" }
        } else if *repo {
            "git repo · opens as a project"
        } else if n == ".." {
            ""
        } else {
            "folder"
        };
        put(buf, r.right().saturating_sub(3 + tag.width() as u16), y, &[seg(tag, st.fg(if open { t.accent } else { t.muted }))], r.right());
        hit(app, Rect { x: r.x + 1, y, width: r.width - 2, height: 1 }, HyHit::FinderPick(i));
    }
    if rows.is_empty() {
        put(buf, r.x + 3, r.y + 6, &[seg("nothing matches here", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Tab", "complete"), ("Enter", "open"), ("↑↓", "choose"), ("Backspace", "edit"), ("Esc", "close")]), r.right());
}

// New pane ----------------------------------------------------------------------------------

pub(in crate::client) fn np_agents(app: &App) -> Vec<String> {
    let mut v: Vec<String> = app.cfg.quick.agents.iter().map(|a| a.name.clone()).collect();
    for a in ["claude", "codex", "gemini"] {
        if !v.iter().any(|x| x == a) {
            v.push(a.into());
        }
    }
    v.push("shell".into());
    v.extend(app.cfg.recipes.iter().filter(|r| !r.run.is_empty()).map(|r| format!("⚙ {}", r.name)));
    v.extend(app.cfg.presets.iter().map(|p| format!("★ {}", p.name)));
    v
}

pub(in crate::client) fn preset_of<'a>(app: &'a App, run: &str) -> Option<&'a crate::config::Preset> {
    let name = run.strip_prefix("★ ")?;
    app.cfg.presets.iter().find(|p| p.name == name)
}

/// Where a new agent can go in a git project: the three kinds, then its worktrees by name.
pub(in crate::client) fn np_places(p: &Proj) -> Vec<String> {
    let mut v: Vec<String> = ["new worktree", "new branch", "this branch"].map(String::from).to_vec();
    v.extend(p.wts.iter().filter(|w| !w.main).map(|w| format!("⌥ {}", w.name)));
    v
}

/// The command palette, as the other popups: type words, ↑↓, Enter; each with its key.
pub(in crate::client) fn draw_palette(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, query: &str, sel: usize, items: &[crate::client::PickItem]) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (items.len() as u16 + 8).clamp(12, 30);
    let r = panel(app, buf, area, 80, h, "Command palette", &[], t);
    let q = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: 1 };
    fill(buf, q, t.card2);
    let s2 = Style::default().bg(t.card2);
    let mut qs = vec![seg("› ", s2.fg(t.accent).add_modifier(Modifier::BOLD)), seg(query.to_string(), s2.fg(t.strong)), seg("█", s2.fg(t.accent))];
    if query.is_empty() {
        qs.push(seg(" what do you want to do?", s2.fg(t.muted)));
    }
    put(buf, r.x + 3, q.y, &qs, r.right() - 2);
    let list = Rect { x: r.x + 1, y: r.y + 4, width: r.width - 2, height: r.height.saturating_sub(7) };
    let start = sel.saturating_sub(list.height.saturating_sub(1) as usize);
    for (i, item) in items.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let rr = Rect { y, height: 1, ..list };
        let on = i == sel;
        let bg = if on || hovered(app, rr) { t.hov } else { t.card };
        fill(buf, rr, bg);
        let st = Style::default().bg(bg);
        if on {
            put(buf, rr.x + 1, y, &[seg("›", st.fg(t.accent).add_modifier(Modifier::BOLD))], rr.right());
        }
        let ls = st.fg(if on { t.strong } else { t.text });
        put(buf, rr.x + 3, y, &[seg(item.label.clone(), if on { ls.add_modifier(Modifier::BOLD) } else { ls })], rr.right().saturating_sub(14));
        let key = match &item.target {
            crate::client::PickTarget::Command(a) => key_text(app, std::slice::from_ref(a)).split("  ").next().unwrap_or("").to_string(),
            _ => String::new(),
        };
        let caps = keycaps(t, &key, bg);
        let kw = segs_width(&caps);
        put(buf, rr.right().saturating_sub(kw + 2), y, &caps, rr.right());
    }
    if items.is_empty() {
        put(buf, list.x + 2, list.y, &[seg("nothing matches", Style::default().bg(t.card).fg(t.muted).add_modifier(Modifier::ITALIC))], list.right());
    }
    let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("↑↓", "move"), ("Enter", "do it"), ("Esc", "close")]), r.right());
    let note = format!("keys go after {lead}");
    put(buf, r.right().saturating_sub(note.width() as u16 + 3), r.bottom() - 2, &[seg(note, Style::default().bg(t.card).fg(t.muted))], r.right());
}

/// A working agent's name: the working colour with a bright band sweeping across it.
pub(in crate::client) fn shimmer(text: &str, frame: u64, base: Color, bright: Color, st: Style) -> Vec<Seg> {
    const WIDTH: f32 = 3.0;
    let chars: Vec<char> = text.chars().collect();
    let span = chars.len() as u64 + 8;
    let pos = (frame % span) as f32 - 4.0;
    chars
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let k = (1.0 - (i as f32 - pos).abs() / WIDTH).max(0.0);
            seg(ch.to_string(), st.fg(crate::client::render::blend(base, bright, k * 0.8)))
        })
        .collect()
}

/// The go-to switcher's rows: projects and their sessions, those matching `q`.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::client) enum GoRow {
    Proj(usize),
    Sess(usize, TermId),
}

pub(in crate::client) fn goto_rows(model: &[Proj], q: &str) -> Vec<GoRow> {
    let q = q.trim();
    let hit = |text: &str| q.is_empty() || crate::client::files::fuzzy(q, text).is_some();
    let mut out = Vec::new();
    for (pi, p) in model.iter().enumerate() {
        let proj_hit = hit(&p.name);
        let mut sess: Vec<&Session> = p.sessions().collect();
        sess.sort_by_key(|s| (rank(s.status), s.term));
        let sess: Vec<&Session> = sess.into_iter().filter(|s| proj_hit || hit(&format!("{} {} {} {}", p.name, s.name, s.agent, s.title))).collect();
        if !proj_hit && sess.is_empty() {
            continue;
        }
        out.push(GoRow::Proj(pi));
        out.extend(sess.into_iter().map(|s| GoRow::Sess(pi, s.term)));
    }
    out
}

pub(in crate::client) fn draw_goto(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, query: &str, sel: usize) {
    let model = app.hy_model();
    let rows = goto_rows(&model, query);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (rows.len() as u16 + 8).max(12).min(area.height.saturating_sub(4));
    let r = panel(app, buf, area, 92, h, "Go to", &[], t);
    let c = Style::default().bg(t.card);
    let q = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: 1 };
    fill(buf, q, t.card2);
    let s2 = Style::default().bg(t.card2);
    let mut qs = vec![seg("› ", s2.fg(t.accent).add_modifier(Modifier::BOLD)), seg(query.to_string(), s2.fg(t.strong)), seg("█", s2.fg(t.accent))];
    if query.is_empty() {
        qs.push(seg(" type a project or session", s2.fg(t.muted)));
    }
    put(buf, r.x + 3, q.y, &qs, r.right() - 2);
    let list = Rect { x: r.x + 1, y: r.y + 4, width: r.width - 2, height: r.height.saturating_sub(7) };
    let start = sel.saturating_sub(list.height.saturating_sub(1) as usize);
    for (i, row) in rows.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let rr = Rect { y, height: 1, ..list };
        let on = i == sel;
        let bg = if on || hovered(app, rr) { t.hov } else { t.card };
        fill(buf, rr, bg);
        let st = Style::default().bg(bg);
        if on {
            put(buf, rr.x + 1, y, &[seg("›", st.fg(t.accent).add_modifier(Modifier::BOLD))], rr.right());
        }
        match row {
            GoRow::Proj(pi) => {
                let p = &model[*pi];
                put(buf, rr.x + 3, y, &[seg("▌", st.fg(p.color)), seg(p.name.clone(), st.fg(t.strong).add_modifier(Modifier::BOLD))], rr.right());
                let meta: Vec<Seg> = counts(app, t, p.sessions(), None).into_iter().map(|(x, s)| (x, s.bg(bg))).collect();
                let meta = if p.sessions().count() == 0 { vec![seg("empty", st.fg(t.muted))] } else { meta };
                let mw = segs_width(&meta);
                put(buf, rr.right().saturating_sub(mw + 2), y, &meta, rr.right());
            }
            GoRow::Sess(pi, term) => {
                let Some((_, w, s)) = find(&model, *term) else { continue };
                let icon = if s.is_agent { format!("{} ", kind_icon(app, &s.agent, true)) } else { "  ".into() };
                let gl = if s.is_agent { glyph(app, s.status) } else { app.cfg.icons.shell.clone() };
                let gc = if s.is_agent { t.status(s.status) } else { t.muted };
                put(buf, rr.x + 6, y, &[seg(format!("{gl} "), st.fg(gc)), seg(icon, st.fg(t.muted)), seg(s.name.clone(), st.fg(if on { t.strong } else { t.text }))], rr.right());
                let meta = if model[*pi].git && !w.branch.is_empty() { format!("{} · {}", w.branch, state_label(s.status)) } else { state_label(s.status).to_string() };
                let col = if s.status == Status::Blocked { t.blocked } else { t.muted };
                let mw = meta.width() as u16;
                put(buf, rr.right().saturating_sub(mw + 2), y, &[seg(meta, st.fg(col))], rr.right());
            }
        }
        hit(app, rr, HyHit::GoPick(i));
    }
    if rows.is_empty() {
        put(buf, list.x + 2, list.y, &[seg("nothing matches", c.fg(t.muted).add_modifier(Modifier::ITALIC))], list.right());
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("↑↓", "move"), ("Enter", "go"), ("Esc", "close")]), r.right());
}

/// Every session with what it uses, biggest first: (term, label, where, bytes, asleep).
pub(in crate::client) fn memory_rows(app: &App) -> Vec<(TermId, String, String, u64, bool)> {
    let model = app.hy_model();
    let mut v: Vec<(TermId, String, String, u64, bool)> = model
        .iter()
        .flat_map(|p| p.wts.iter().map(move |w| (p, w)))
        .flat_map(|(p, w)| w.sessions.iter().map(move |s| (p, w, s)))
        .map(|(p, w, s)| {
            let mem = app.snap.terms.get(&s.term).map(|t| t.mem).unwrap_or(0);
            let label = if s.dev.is_some() { "▶ dev".to_string() } else { s.agent.clone() };
            let place = if w.main { p.name.clone() } else { format!("{} / {}", p.name, w.name) };
            (s.term, label, place, mem, s.asleep)
        })
        .collect();
    v.sort_by_key(|r| std::cmp::Reverse(r.3));
    v
}

pub(in crate::client) fn draw_history(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let items: Vec<(u64, Option<TermId>, char, String)> = app.history.iter().rev().cloned().collect();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (items.len() as u16 + 7).max(10).min(area.height.saturating_sub(4));
    let r = panel(app, buf, area, 104, h, "What happened", &[], t);
    let c = Style::default().bg(t.card);
    let list = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: r.height.saturating_sub(5) };
    if items.is_empty() {
        put(buf, list.x + 2, list.y, &[seg("nothing yet: agents finishing, asking and ringing show up here", c.fg(t.muted))], list.right());
    }
    let start = sel.saturating_sub(list.height.saturating_sub(1) as usize);
    for (i, (at, term, kind, text)) in items.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let row = Rect { y, height: 1, ..list };
        let on = i == sel;
        let bg = if on || hovered(app, row) { t.hov } else { t.card };
        fill(buf, row, bg);
        let st = Style::default().bg(bg);
        if on {
            put(buf, row.x, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], row.right());
        }
        let (g, gc) = match kind {
            '!' => ("●", t.blocked),
            '✓' => ("✓", t.done),
            '♪' => ("♪", t.blocked),
            'x' => ("✕", t.err),
            _ => ("·", t.muted),
        };
        let gone = term.is_some_and(|tm| !app.snap.terms.contains_key(&tm));
        put(
            buf,
            row.x + 2,
            y,
            &[
                seg(format!("{:>4}  ", age(*at)), st.fg(t.muted)),
                seg(format!("{g} "), st.fg(gc).add_modifier(Modifier::BOLD)),
                seg(truncate(text, (row.width as usize).saturating_sub(18)), st.fg(if gone { t.muted } else { t.text })),
            ],
            row.right() - 1,
        );
        hit(app, row, HyHit::HistRow(i));
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", "go to it"), ("c", "clear"), ("Esc", "close")]), r.right() - 1);
}

pub(in crate::client) fn mb(bytes: u64) -> String {
    let m = bytes as f64 / (1u64 << 20) as f64;
    if m >= 1024.0 { format!("{:.1} GB", m / 1024.0) } else { format!("{m:.0} MB") }
}

pub(in crate::client) fn draw_memory(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let rows = memory_rows(app);
    let total: u64 = rows.iter().map(|r| r.3).sum();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (rows.len() as u16 + 8).max(10).min(area.height.saturating_sub(4));
    let r = panel(app, buf, area, 92, h, &format!("Memory · {} in all", mb(total)), &[], t);
    let c = Style::default().bg(t.card);
    let top = rows.first().map(|r| r.3).unwrap_or(1).max(1);
    let list = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: r.height.saturating_sub(5) };
    if rows.is_empty() {
        put(buf, list.x + 2, list.y, &[seg("nothing running", c.fg(t.muted))], list.right());
    }
    let start = sel.saturating_sub(list.height.saturating_sub(1) as usize);
    for (i, (term, label, place, mem, asleep)) in rows.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let row = Rect { y, height: 1, ..list };
        let on = i == sel;
        let bg = if on || hovered(app, row) { t.hov } else { t.card };
        fill(buf, row, bg);
        let st = Style::default().bg(bg);
        if on {
            put(buf, row.x, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], row.right());
        }
        put(buf, row.x + 2, y, &[seg(truncate(label, 12), st.fg(t.strong).add_modifier(Modifier::BOLD)), seg(format!("  {}", truncate(place, 30)), st.fg(t.muted))], row.x + 48);
        // A bar against the biggest.
        let bw = 26u16;
        let filled = ((*mem as f64 / top as f64) * bw as f64).round() as u16;
        let bar: String = "█".repeat(filled as usize) + &"░".repeat((bw - filled.min(bw)) as usize);
        let col = if *mem > 2 << 30 { t.err } else if *mem > 1 << 30 { t.blocked } else { t.accent };
        put(buf, row.x + 50, y, &[seg(bar, st.fg(col))], row.right());
        let txt = if *asleep { "asleep".to_string() } else { mb(*mem) };
        put(buf, row.right().saturating_sub(10), y, &[seg(format!("{txt:>9}"), st.fg(t.text))], row.right());
        hit(app, row, HyHit::MemRow(i));
        let _ = term;
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", "open"), ("x", "end it"), ("Esc", "close")]), r.right() - 1);
}
