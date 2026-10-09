//! The "workspaces" layout from the design handoff: a sidebar of workspaces (one per repo)
//! → their worktrees → the agent in each; borderless panes with one-row title bars; an
//! answer bar on panes that need you; a crumb in the top bar and counts in the status line.
//! Plus the seshi-native views that replace the pane area (Changes, Files), the
//! talk-to-a-worktree modal and the + Pane menu.

use super::render::{status_icon, truncate};
use super::{App, Hit};
use crate::protocol::{Status, TermInfo};

use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

// ---- sidebar model -------------------------------------------------------------------------

/// Case- and separator-insensitive form of a path, for comparisons.
/// A path for comparing: one separator, no trailing one, and case folded only where the
/// file system ignores case (Windows).
pub(super) fn path_key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
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
    // Off the screen (a popup taller or wider than a small window): draw nothing.
    let area = buf.area;
    if y < area.top() || y >= area.bottom() || x < area.left() {
        return x;
    }
    let max_x = max_x.min(area.right());
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
    let r = r.intersection(buf.area);
    buf.set_style(r, Style::reset().bg(bg));
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            if let Some(px) = buf.cell_mut((x, y)) {
                px.set_symbol(" ");
            }
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
    // A pill: the same width as " Label key " was.
    let (l, r) = super::hydra::ends(bg);
    let mut v = vec![l];
    if key.is_empty() {
        v.push(seg(label.to_string(), Style::default().bg(bg).fg(fg).add_modifier(bold)));
    } else {
        v.push(seg(format!("{label} "), Style::default().bg(bg).fg(fg).add_modifier(bold)));
        v.push(seg(key.to_string(), Style::default().bg(bg).fg(kfg).add_modifier(Modifier::BOLD)));
    }
    v.push(r);
    v
}

/// A key drawn as a little keycap: the key on a raised ground.
pub(super) fn keycap(t: &Theme, key: &str) -> Vec<Seg> {
    super::hydra::chip(key, Style::default().bg(t.btn).fg(t.strong).add_modifier(Modifier::BOLD))
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
            out.extend(keycap(t, c));
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
        out.extend(keycap(t, k));
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

/// A title bar: name + location on the left, the given segments on the right.
pub(super) fn title_bar(buf: &mut Buffer, r: Rect, t: &Theme, left: &[Seg], right: &[Seg], focus: bool) {
    let bg = t.bg;
    fill(buf, Rect { height: 1, ..r }, bg);
    let ink = |segs: &[Seg], c: Color| -> Vec<Seg> { segs.iter().map(|(s, st)| (s.clone(), st.bg(bg).fg(c))).collect() };
    let rw = segs_width(right);
    put(buf, r.x + 1, r.y, &ink(left, if focus { t.accent } else { t.strong }), r.right().saturating_sub(rw + 2));
    put(buf, r.right().saturating_sub(rw), r.y, &ink(right, t.muted), r.right());
}

// ---- views that replace the pane area ------------------------------------------------------

/// The bottom action row of a view.
fn action_row(app: &mut App, buf: &mut Buffer, r: Rect, t: &Theme, buttons: &[(&str, &str, BtnKind, Hit)], tail: &[Seg]) {
    let y = r.bottom().saturating_sub(1);
    super::hydra::strip(buf, Rect { y, height: 1, ..r }, t.card2);
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
        if let Some(px) = buf.cell_mut((body.x + fw, yy)) {
            px.set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
        }
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
        if let Some(px) = buf.cell_mut((body.x + fw, yy)) {
            px.set_symbol("│").set_style(Style::default().fg(t.line).bg(t.bg));
        }
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
                        if let Some(px) = buf.cell_mut((body.x + fw, yy)) {
                            px.set_symbol("┃").set_style(Style::default().fg(t.accent).bg(t.bg));
                        }
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

/// The shortcut rows by category: (category, [(what it does, the actions it covers)]).
#[allow(clippy::type_complexity)]
pub(super) fn key_rows() -> Vec<(&'static str, Vec<(&'static str, Vec<crate::keys::Action>)>)> {
    use crate::keys::Action as A;
    use crate::layout::Dir::*;
    vec![
        (
            "GET AROUND",
            vec![
                ("Command palette", vec![A::Palette]),
                ("Focus the sidebar", vec![A::BrowseTree]),
                ("Focus the pane left", vec![A::Focus(Left)]),
                ("Focus the pane below", vec![A::Focus(Down)]),
                ("Focus the pane above", vec![A::Focus(Up)]),
                ("Focus the pane right", vec![A::Focus(Right)]),
                ("Inbox: what needs you", vec![A::Jump]),
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
            "START",
            vec![
                ("New session (a shell here)", vec![A::ShellHere]),
                ("Rename session", vec![A::RenameWorkspace]),
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
            ],
        ),
        (
            "APP",
            vec![
                ("Settings", vec![A::Settings]),
                ("All keys", vec![A::Help]),
                ("History", vec![A::History]),
                ("Detach", vec![A::Detach]),
                ("Reload config", vec![A::ReloadConfig]),
            ],
        ),
    ]
}

// ---- settings view -------------------------------------------------------------------------

/// A row of a settings page: a setting, or (on the Keys page) a shortcut.
#[derive(Debug, Clone)]
pub(super) enum SRow {
    Setting(&'static super::modal::Setting),
    Bind { label: &'static str, acts: Vec<crate::keys::Action> },
    /// One theme (Settings → Appearance): index into theme::BUILTIN.
    Theme(usize),
}

/// The small caps heading a settings row sits under.
pub(super) fn settings_group(row: &SRow) -> &'static str {
    match row {
        SRow::Theme(_) => "THEME",
        SRow::Bind { label, .. } => key_rows().into_iter().find(|(_, items)| items.iter().any(|(l, _)| l == label)).map(|(g, _)| g).unwrap_or("KEYS"),
        SRow::Setting(s) => match s.path {
            "prefix" | "ui.mouse" | "ui.which_key" => "INPUT",
            "ui.sidebar_position" | "ui.splash" => "LAYOUT",
            "ui.update_check" => "UPDATES",
            "shell" | "editor" | "shell_integration" | "ui.start_dir" => "SHELL",
            "ui.attention_sort" => "SIDEBAR",
            "ui.panes" | "ui.corners" | "ui.gap" | "ui.dim" | "ui.focus_border" | "ui.pill_caps" => "PANES",
            p if p.starts_with("notify.") => "ALERTS",
            "worktree.delete_with_last" | "worktree.per_agent" | "worktree.command" => "WORKTREES",
            p if p.starts_with("restore.") || p == "sleep_after" => "RESTARTS",
            "scrollback" => "HISTORY",
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
pub(super) fn settings_rows(cat: super::modal::Cat) -> Vec<SRow> {
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
                    let mut caps: Vec<Seg> = Vec::new();
                    for (i, p) in k.split('+').enumerate() {
                        if i > 0 {
                            caps.push(seg("+", Style::default().fg(t.muted)));
                        }
                        caps.extend(keycap(t, p));
                    }
                    caps
                }
                Kind::Text | Kind::Folder | Kind::Program(_) => {
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

