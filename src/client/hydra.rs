//! The Hydra layout, from the design handoff: projects (git repos) in the sidebar, each
//! project's worktrees with the sessions running in them, and the focused session full size
//! (optionally split with a second). Everything else is a centred overlay over a dimmed
//! screen: jump, open a project, new pane, talk, settings and keys.
//!
//! The daemon only knows workspaces, tabs and terminals. Here every terminal is a session;
//! its project and worktree come from where it runs (`TermInfo::root` / `top`), so a shell
//! that `cd`s into another repo moves to that project by itself.

use super::design::{
    BtnKind, SRow, Seg, SettingsView, button, cap_hints, fill, glyph, key_text, keycap, keycaps, path_key, put,
    question, seg, segs_width, state_label, tilde,
};
use super::render::{blend, render_screen, truncate};
use super::{App, Hit, Mode};
use crate::keys::Action;
use crate::protocol::{Status, TermId, TermInfo};
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

// ---- state ---------------------------------------------------------------------------------

/// What the user chose that the daemon doesn't track: projects they opened, the colour
/// order, folded worktrees. Saved to `hydra-ui.json` in the data folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Saved {
    /// Folders opened as projects (they stay even with nothing running).
    pub known: Vec<PathBuf>,
    /// Project keys in the order they were first seen; a project's colour is its index.
    pub order: Vec<String>,
    /// Folded worktrees (by path key).
    pub closed: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct Hy {
    pub saved: Saved,
    /// The project shown in the sidebar (by key).
    pub proj: Option<String>,
    /// Two sessions side by side: (left, right). One of them is the focused one.
    pub pair: Option<(TermId, TermId)>,
    /// Sidebar cursor (keyboard browsing; bare keys work while it's set).
    pub cursor: Option<TermId>,
    /// Projects opened this run that haven't been looked at (NEW chip).
    pub fresh: HashSet<String>,
    /// A session about to open beside this one: pair them when it appears.
    pub pending_split: Option<(TermId, Instant)>,
    /// The focus last time the state came in.
    pub last_focus: Option<TermId>,
    /// Drawn this frame: sidebar sessions in order, project keys, worktree keys.
    pub visible: Vec<TermId>,
    pub proj_keys: Vec<String>,
    pub wt_keys: Vec<(String, PathBuf)>,
}

fn saved_path() -> PathBuf {
    // Per server, like the saved session: a test server (HYDRA_SOCKET) keeps its own.
    let name = match std::env::var("HYDRA_SOCKET") {
        Ok(l) if l != "default" => format!("hydra-ui-{l}.json"),
        _ => "hydra-ui.json".into(),
    };
    crate::config::data_dir().join(name)
}

impl Hy {
    pub fn load() -> Hy {
        if cfg!(test) {
            return Hy::default();
        }
        let saved = std::fs::read_to_string(saved_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Hy { saved, ..Default::default() }
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Ok(s) = serde_json::to_string_pretty(&self.saved) {
            let p = saved_path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, s);
        }
    }
}

/// The overlays of this layout.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Finder {
    pub q: String,
    pub sel: usize,
    /// The folder being listed and its subfolders: (name, is a git repo).
    pub dir: PathBuf,
    pub list: Vec<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct NewPaneHy {
    /// Project index, worktree index (== count means "+ new worktree"), run index, row.
    pub p: usize,
    pub w: usize,
    pub a: usize,
    pub row: u8,
}

// ---- model ---------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct Session {
    pub term: TermId,
    pub title: String,
    pub agent: String,
    pub status: Status,
    pub since: u64,
    pub question: Option<String>,
    pub is_agent: bool,
    /// Subagents it's running right now.
    pub subagents: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct Wt {
    pub key: String,
    pub path: PathBuf,
    pub name: String,
    pub main: bool,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone)]
pub(super) struct Proj {
    pub key: String,
    pub path: PathBuf,
    pub name: String,
    pub color: Color,
    pub wts: Vec<Wt>,
    pub fresh: bool,
}

impl Proj {
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.wts.iter().flat_map(|w| w.sessions.iter())
    }
}

/// Attention order: needs you, done, working, idle.
pub(super) fn rank(s: Status) -> u8 {
    match s {
        Status::Blocked => 0,
        Status::Done => 1,
        Status::Working => 2,
        _ => 3,
    }
}

fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// "40s", "3m", "1h", "2d".
pub(super) fn age(since: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(since);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// An agent's numbered choices on screen ("❯ 1. Yes", "  2. Yes, and always allow …"),
/// shortened for buttons. Falls back to Yes / Always / No.
pub(super) fn options(parser: Option<&vt100::Parser>) -> Vec<String> {
    let mut out: Vec<(u8, String)> = Vec::new();
    if let Some(p) = parser {
        let screen = p.screen();
        let (_, cols) = screen.size();
        let lines: Vec<String> = screen.rows(0, cols).collect();
        for l in lines.iter().rev().take(16) {
            let t = l.trim().trim_matches(|c: char| "│┃ ".contains(c)).trim_start_matches(['❯', '>', ' ']).trim();
            let mut chars = t.chars();
            let (Some(d), Some('.')) = (chars.next(), chars.next()) else { continue };
            let Some(n) = d.to_digit(10).filter(|n| (1..=9).contains(n)) else { continue };
            let rest = chars.as_str().trim();
            if rest.is_empty() || out.iter().any(|(k, _)| *k == n as u8) {
                continue;
            }
            let label = if rest.starts_with("Yes, and") || rest.to_lowercase().contains("always") {
                "Always".to_string()
            } else {
                let cut = rest.find([',', '(']).unwrap_or(rest.len());
                truncate(rest[..cut].trim(), 18)
            };
            out.push((n as u8, label));
        }
    }
    out.sort();
    if out.is_empty() || out[0].0 != 1 {
        return vec!["Yes".into(), "Always".into(), "No".into()];
    }
    out.into_iter().take(9).map(|(_, l)| l).collect()
}

impl App {
    /// The design's sessions: every terminal, by project and worktree.
    pub(super) fn hy_model(&self) -> Vec<Proj> {
        let mut projs: Vec<Proj> = Vec::new();
        let sort = self.cfg.ui.attention_sort;
        let order = &self.hy.saved.order;
        let add_proj = |projs: &mut Vec<Proj>, path: &Path| -> usize {
            let key = path_key(path);
            if let Some(i) = projs.iter().position(|p| p.key == key) {
                return i;
            }
            let ci = order.iter().position(|k| *k == key).unwrap_or(order.len() + projs.len());
            projs.push(Proj {
                key: key.clone(),
                path: path.to_path_buf(),
                name: folder_name(path),
                color: self.theme.project(ci, &self.cfg.ui.workspace_colors.iter().filter_map(|c| crate::theme::parse_color(c)).collect::<Vec<_>>()),
                wts: Vec::new(),
                fresh: self.hy.fresh.contains(&key),
            });
            projs.len() - 1
        };
        let add_wt = |p: &mut Proj, path: &Path, name: String, main: bool| -> usize {
            let key = path_key(path);
            if let Some(i) = p.wts.iter().position(|w| w.key == key) {
                return i;
            }
            p.wts.push(Wt { key, path: path.to_path_buf(), name, main, sessions: Vec::new() });
            p.wts.len() - 1
        };

        for w in &self.snap.workspaces {
            let leaves: Vec<TermId> = w.tabs.iter().flat_map(|t| t.layout.leaves()).collect();
            for id in &leaves {
                let Some(t) = self.snap.terms.get(id) else { continue };
                let (root, top, wname, main) = match (&t.root, &t.top) {
                    (Some(r), Some(tp)) => {
                        let main = path_key(r) == path_key(tp);
                        (r.clone(), tp.clone(), t.branch.clone().unwrap_or_else(|| folder_name(tp)), main)
                    }
                    _ => {
                        let cwd = if t.cwd.as_os_str().is_empty() { w.cwd.clone() } else { t.cwd.clone() };
                        (cwd.clone(), cwd, "folder".to_string(), true)
                    }
                };
                let pi = add_proj(&mut projs, &root);
                let wi = add_wt(&mut projs[pi], &top, wname, main);
                let title = session_title(t, if leaves.len() == 1 { &w.name } else { "" }, &top);
                let question = (t.status == Status::Blocked).then(|| self.parsers.get(id).and_then(question)).flatten();
                projs[pi].wts[wi].sessions.push(Session {
                    term: *id,
                    title,
                    agent: match &t.agent {
                        Some(a) => a.clone(),
                        None if t.is_shell() => "shell".into(),
                        None => t.display_name(),
                    },
                    status: t.status,
                    since: t.since,
                    question,
                    is_agent: t.agent.is_some(),
                    subagents: t.subagents.clone(),
                });
            }
            // The repo's other worktrees, even with nothing running in them.
            if let Some(g) = &w.git
                && let Some(p) = projs.iter_mut().find(|p| p.key == path_key(&g.root))
            {
                for e in &g.worktrees {
                    add_wt(p, &e.path, e.branch.clone(), e.main);
                }
            }
        }
        for k in &self.hy.saved.known {
            let head = crate::gitfs::head(k);
            let root = head.as_ref().map(|h| h.main_root.clone()).unwrap_or_else(|| k.clone());
            let pi = add_proj(&mut projs, &root);
            if projs[pi].wts.is_empty() {
                match head {
                    Some(h) => add_wt(&mut projs[pi], &h.top, h.branch, true),
                    None => add_wt(&mut projs[pi], k, "folder".into(), true),
                };
            }
        }
        for p in &mut projs {
            if sort {
                for w in &mut p.wts {
                    w.sessions.sort_by_key(|s| (rank(s.status), s.term));
                }
            }
            p.wts.sort_by_key(|w| {
                let worst = w.sessions.iter().map(|s| rank(s.status)).min().unwrap_or(4);
                (if sort { worst } else { 0 }, !w.main, w.name.clone())
            });
        }
        projs.sort_by_key(|p| order.iter().position(|k| *k == p.key).unwrap_or(usize::MAX));
        projs
    }

    /// Keep the remembered bits in step with a new state: project order, the selected
    /// project, the split pair.
    pub(super) fn hy_sync(&mut self) {
        let focus = self.focused();
        let model = self.hy_model();
        let mut changed = false;
        for p in &model {
            if !self.hy.saved.order.contains(&p.key) {
                self.hy.saved.order.push(p.key.clone());
                changed = true;
            }
        }
        if changed {
            self.hy.save();
        }
        let proj_of = |t: TermId| model.iter().find(|p| p.sessions().any(|s| s.term == t)).map(|p| p.key.clone());
        if focus != self.hy.last_focus {
            if let Some(f) = focus {
                if let Some(k) = proj_of(f) {
                    self.hy.fresh.remove(&k);
                    self.hy.proj = Some(k);
                }
                // A session opened "beside": pair it with the one it was opened from.
                if let Some((prev, at)) = self.hy.pending_split.take() {
                    if prev != f && at.elapsed().as_secs() < 20 && self.snap.terms.contains_key(&prev) {
                        self.hy.pair = Some((prev, f));
                    }
                } else if let Some((a, b)) = self.hy.pair {
                    // Focusing something else replaces the half that had the focus.
                    if f != a && f != b {
                        self.hy.pair = match self.hy.last_focus {
                            Some(l) if l == a => Some((f, b)),
                            Some(l) if l == b => Some((a, f)),
                            _ => None,
                        };
                    }
                }
            }
            self.hy.last_focus = focus;
        }
        if let Some((a, b)) = self.hy.pair
            && (a == b || !self.snap.terms.contains_key(&a) || !self.snap.terms.contains_key(&b))
        {
            self.hy.pair = None;
        }
        if self.hy.proj.as_ref().is_none_or(|k| !model.iter().any(|p| &p.key == k)) {
            self.hy.proj = focus.and_then(proj_of).or_else(|| model.first().map(|p| p.key.clone()));
        }
    }
}

/// What to call a session: its name if renamed, else the agent's last prompt, else where a
/// shell is (inside its worktree), else the program.
fn session_title(t: &TermInfo, name: &str, top: &Path) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    if t.agent.is_some() {
        let s = t.summary.trim();
        return if s.is_empty() { "new session".into() } else { s.to_string() };
    }
    if t.is_shell() {
        return match t.cwd.strip_prefix(top) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => folder_name(&t.cwd),
        };
    }
    t.display_name()
}

// ---- hits ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HyHit {
    Splash,
    Proj(usize),
    OpenFolder,
    ToggleWt(usize),
    Session(TermId),
    Talk(TermId),
    NewSession(usize),
    Settings,
    Jump,
    NewPane,
    Keys,
    CloseSplit(TermId),
    /// Outside an overlay: closes it.
    Close,
    /// Inside an overlay's panel: nothing.
    Noop,
    FinderPick(usize),
    JumpTo(TermId),
    NpProj(usize),
    NpWt(usize),
    NpRun(usize),
    NpGo,
    SetTab(usize),
    SetRow(usize),
    /// Settings row, value index.
    SetVal(usize, usize),
    /// A button on the splash screen.
    SplashKey(char),
}

fn hit(app: &mut App, r: Rect, h: HyHit) {
    app.hits.push((r, Hit::Hy(h)));
}

fn hovered(app: &App, r: Rect) -> bool {
    app.hover.is_some_and(|p| r.contains(p))
}

/// A design button (" Label key "), hovered with `hov`; records its hit.
#[allow(clippy::too_many_arguments)]
fn btn(app: &mut App, buf: &mut Buffer, x: u16, y: u16, label: &str, key: &str, kind: BtnKind, h: HyHit, max_x: u16) -> u16 {
    let t = app.theme.clone();
    let w = segs_width(&button(&t, label, key, kind, false));
    let r = Rect { x, y, width: w.min(max_x.saturating_sub(x)), height: 1 };
    let segs = button(&t, label, key, kind, hovered(app, r));
    let nx = put(buf, x, y, &segs, max_x);
    hit(app, r, h);
    nx
}

/// The key bound to an action after the leader, as the design shows it ("j", "Space").
fn k(app: &App, a: &Action) -> String {
    let s = super::design::key_of(app, a);
    match s.as_str() {
        "Space" | " " => "Space".into(),
        _ => s,
    }
}

// ---- main screen ---------------------------------------------------------------------------

fn side_w(width: u16) -> u16 {
    match width {
        0..140 => 26,
        140..200 => 36,
        _ => 38,
    }
}

/// Draw the main screen; returns the pane area.
pub(super) fn draw(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) -> Rect {
    let model = app.hy_model();
    fill(f.buffer_mut(), area, t.bg);
    let sw = if app.sidebar { side_w(area.width).min(area.width / 2) } else { 0 };
    let right_side = app.cfg.ui.sidebar_position == "right";
    let mid = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(2) };
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
    draw_top(app, f.buffer_mut(), area, panes.x, &model, t);
    if sw > 0 {
        draw_side(app, f.buffer_mut(), side, &model, t);
    }
    draw_main(app, f, panes, &model, t);
    draw_status(app, f.buffer_mut(), Rect { y: area.bottom().saturating_sub(1), height: 1, ..area }, &model, t);
    panes
}

fn find(model: &[Proj], term: TermId) -> Option<(&Proj, &Wt, &Session)> {
    model.iter().find_map(|p| p.wts.iter().find_map(|w| w.sessions.iter().find(|s| s.term == term).map(|s| (p, w, s))))
}

fn draw_top(app: &mut App, buf: &mut Buffer, area: Rect, crumb_x: u16, model: &[Proj], t: &Theme) {
    let y = area.y;
    let x = put(buf, area.x + 1, y, &[seg(">_ hydra", Style::default().fg(t.accent).bg(t.bg).add_modifier(Modifier::BOLD))], area.right());
    hit(app, Rect { x: area.x, y, width: x - area.x, height: 1 }, HyHit::Splash);
    let needs = model.iter().flat_map(|p| p.sessions()).filter(|s| s.status == Status::Blocked).count();
    // Right: Jump (amber when something needs you) and + Pane.
    let pane_key = k(app, &Action::NewPane);
    let pb = button(t, "+ Pane", &pane_key, BtnKind::Primary, false);
    let px = area.right().saturating_sub(segs_width(&pb));
    let jk = k(app, &Action::Jump);
    let (jbg, jfg, kfg) = if needs > 0 { (t.blocked, t.bg, t.bg) } else { (t.btn, t.strong, t.accent) };
    let jb = vec![
        seg(" Jump ", Style::default().bg(jbg).fg(jfg).add_modifier(Modifier::BOLD)),
        seg(format!("{}{jk} ", if needs > 0 { format!("●{needs} ") } else { String::new() }), Style::default().bg(jbg).fg(kfg).add_modifier(Modifier::BOLD)),
    ];
    let jx = px.saturating_sub(1 + segs_width(&jb));
    put(buf, jx, y, &jb, px);
    hit(app, Rect { x: jx, y, width: segs_width(&jb), height: 1 }, HyHit::Jump);
    btn(app, buf, px, y, "+ Pane", &pane_key, BtnKind::Primary, HyHit::NewPane, area.right());
    // Crumb: ▌project › worktree › session
    if let Some((p, w, s)) = app.focused().and_then(|f| find(model, f)) {
        put(
            buf,
            crumb_x,
            y,
            &[
                seg("▌", Style::default().fg(p.color)),
                seg(p.name.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
                seg("  ›  ", Style::default().fg(t.muted)),
                seg(w.name.clone(), Style::default().fg(t.text)),
                seg("  ›  ", Style::default().fg(t.muted)),
                seg(s.title.clone(), Style::default().fg(t.text)),
            ],
            jx.saturating_sub(2),
        );
    }
}

/// `●1 ✓1 ⠹2`: counts of a list of sessions by state.
fn counts<'a>(app: &App, t: &Theme, list: impl Iterator<Item = &'a Session>, ink: Option<Color>) -> Vec<Seg> {
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

fn hline(buf: &mut Buffer, x: u16, y: u16, w: u16, t: &Theme, bg: Color) {
    for i in 0..w {
        buf[(x + i, y)].set_symbol("─").set_style(Style::default().fg(t.line).bg(bg));
    }
}

fn draw_side(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let surf = t.sidebar_bg;
    fill(buf, r, surf);
    let (x0, w) = (r.x, r.width);
    let right = r.right().saturating_sub(1);
    put(buf, x0 + 2, r.y, &[seg("PROJECTS", Style::default().fg(t.muted).bg(surf).add_modifier(Modifier::BOLD))], r.right());
    let mut y = r.y + 1;
    app.hy.proj_keys.clear();
    for (i, p) in model.iter().enumerate() {
        if y + 8 >= r.bottom() {
            break;
        }
        let cur = app.hy.proj.as_deref() == Some(p.key.as_str());
        let row = Rect { x: x0, y, width: w, height: 1 };
        let bg = if cur { t.btn } else if hovered(app, row) { t.hov } else { surf };
        fill(buf, row, bg);
        let s = Style::default().bg(bg);
        let mut left = vec![seg("▌", s.fg(p.color)), seg(p.name.clone(), if cur { s.fg(t.strong).add_modifier(Modifier::BOLD) } else { s.fg(t.strong) })];
        if p.fresh {
            left.push(seg(" ", s));
            left.push(seg(" NEW ", Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)));
        }
        let c: Vec<Seg> = counts(app, t, p.sessions(), None).into_iter().map(|(x, st)| (x, st.bg(bg))).collect();
        let cw = segs_width(&c);
        put(buf, x0 + 1, y, &left, right.saturating_sub(cw + 1));
        put(buf, right.saturating_sub(cw), y, &c, right);
        app.hy.proj_keys.push(p.key.clone());
        hit(app, row, HyHit::Proj(i));
        y += 1;
    }
    let ok = k(app, &Action::OpenProject);
    let row = Rect { x: x0, y, width: w, height: 1 };
    let bg = if hovered(app, row) { t.hov } else { surf };
    fill(buf, row, bg);
    put(buf, x0 + 2, y, &[seg("+ open a folder", Style::default().fg(t.muted).bg(bg)), seg(format!("  {ok}"), Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], r.right());
    hit(app, row, HyHit::OpenFolder);
    y += 2;
    hline(buf, x0 + 1, y, w.saturating_sub(2), t, surf);
    y += 1;

    app.hy.visible.clear();
    app.hy.wt_keys.clear();
    let bottom = r.bottom().saturating_sub(4);
    let Some(p) = model.iter().find(|p| app.hy.proj.as_deref() == Some(p.key.as_str())) else { return };
    put(
        buf,
        x0 + 2,
        y,
        &[seg("WORKTREES", Style::default().fg(t.muted).bg(surf).add_modifier(Modifier::BOLD)), seg(format!("  {}", p.name), Style::default().fg(p.color).bg(surf))],
        r.right(),
    );
    y += 2;
    let focus = app.focused();
    let split = app.hy.pair.map(|(a, b)| if Some(a) == focus { b } else { a });
    let nk = k(app, &Action::NewSession);
    let tk = k(app, &Action::Talk);
    for wt in &p.wts {
        if y >= bottom {
            break;
        }
        let open = !app.hy.saved.closed.contains(&wt.key);
        let has_focus = wt.sessions.iter().any(|s| Some(s.term) == focus);
        let wi = app.hy.wt_keys.len();
        app.hy.wt_keys.push((wt.key.clone(), wt.path.clone()));
        let row = Rect { x: x0, y, width: w, height: 1 };
        let bg = if hovered(app, row) { t.hov } else { surf };
        fill(buf, row, bg);
        let c: Vec<Seg> = counts(app, t, wt.sessions.iter(), None).into_iter().map(|(x, st)| (x, st.bg(bg))).collect();
        let cw = segs_width(&c);
        put(
            buf,
            x0 + 1,
            y,
            &[
                seg(if open { "▾ " } else { "▸ " }, Style::default().fg(t.muted).bg(bg)),
                seg(wt.name.clone(), Style::default().fg(if has_focus { t.accent } else { t.strong }).bg(bg).add_modifier(Modifier::BOLD)),
            ],
            right.saturating_sub(cw + 1),
        );
        put(buf, right.saturating_sub(cw), y, &c, right);
        hit(app, row, HyHit::ToggleWt(wi));
        y += 1;
        if !open {
            y += 1;
            continue;
        }
        if wt.sessions.is_empty() && y < bottom {
            put(buf, x0 + 5, y, &[seg("no sessions yet", Style::default().fg(t.muted).bg(surf).add_modifier(Modifier::ITALIC))], r.right());
            y += 1;
        }
        for s in &wt.sessions {
            if y >= bottom {
                break;
            }
            let prim = Some(s.term) == focus;
            let in_split = Some(s.term) == split;
            let row = Rect { x: x0, y, width: w, height: 1 };
            let sel = !prim && (app.hy.cursor == Some(s.term) || hovered(app, row));
            let bg = if prim { t.accent } else if in_split { t.card2 } else if sel { t.hov } else { surf };
            let ink = prim.then_some(t.acc_ink);
            fill(buf, row, bg);
            let st = Style::default().bg(bg);
            let right_segs = if sel {
                vec![seg(format!(" {tk} "), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)), seg(format!(" {}", s.agent), st.fg(t.muted))]
            } else {
                let when = if s.is_agent { format!(" {}", age(s.since)) } else { String::new() };
                vec![seg(format!("{}{when}", s.agent), st.fg(ink.unwrap_or(t.muted)))]
            };
            let rw = segs_width(&right_segs);
            let gl = if s.is_agent || s.status != Status::None { glyph(app, s.status) } else { app.cfg.icons.shell.clone() };
            let mut gs = st.fg(ink.unwrap_or(if s.is_agent { t.status(s.status) } else { t.muted }));
            if s.status == Status::Blocked {
                gs = gs.add_modifier(Modifier::BOLD);
            }
            let title_fg = ink.unwrap_or(t.text);
            let mut ts = st.fg(title_fg);
            if prim {
                ts = ts.add_modifier(Modifier::BOLD);
            }
            let room = w.saturating_sub(8 + rw) as usize;
            put(buf, x0 + 3, y, &[seg(format!("{gl} "), gs), seg(truncate(&s.title, room), ts)], right.saturating_sub(rw + 1));
            put(buf, right.saturating_sub(rw), y, &right_segs, right);
            hit(app, row, HyHit::Session(s.term));
            if sel {
                hit(app, Rect { x: right.saturating_sub(rw), y, width: 3, height: 1 }, HyHit::Talk(s.term));
            }
            app.hy.visible.push(s.term);
            y += 1;
            // A session that needs you shows its question under it.
            if let Some(q) = &s.question
                && y < bottom
            {
                put(buf, x0 + 5, y, &[seg(truncate(q, w.saturating_sub(7) as usize), Style::default().fg(blend(t.blocked, surf, 0.35)).bg(surf))], right);
                y += 1;
            }
            // And the subagents it's running.
            for sub in &s.subagents {
                if y >= bottom {
                    break;
                }
                put(buf, x0 + 5, y, &[seg(format!("↳ {}", truncate(sub, w.saturating_sub(9) as usize)), Style::default().fg(t.muted).bg(surf))], right);
                y += 1;
            }
        }
        if y < bottom {
            let row = Rect { x: x0, y, width: w, height: 1 };
            let bg = if hovered(app, row) { t.hov } else { surf };
            fill(buf, row, bg);
            put(buf, x0 + 5, y, &[seg("+ session", Style::default().fg(t.muted).bg(bg)), seg(format!("  {nk}"), Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], r.right());
            hit(app, row, HyHit::NewSession(wi));
            y += 1;
        }
        y += 1;
    }
    let by = r.bottom().saturating_sub(3);
    hline(buf, x0 + 1, by, w.saturating_sub(2), t, surf);
    let sk = k(app, &Action::Settings);
    btn(app, buf, x0 + 2, by + 1, "Settings", &sk, BtnKind::Ghost, HyHit::Settings, r.right());
}

fn draw_main(app: &mut App, f: &mut Frame, area: Rect, model: &[Proj], t: &Theme) {
    let Some(focus) = app.focused() else {
        put(f.buffer_mut(), area.x + 4, area.y + 3, &[seg(format!("No session open. Press {} n in a worktree to start one.", app.keymap.prefix.to_string().replace("C-", "Ctrl+")), Style::default().fg(t.muted))], area.right());
        return;
    };
    let pair = app.hy.pair.filter(|(a, b)| *a == focus || *b == focus);
    match pair {
        Some((a, b)) => {
            let stack = area.width < 140 - side_w(140);
            let (ra, rb, div) = if stack {
                let h = area.height / 2;
                (Rect { height: h, ..area }, Rect { y: area.y + h + 1, height: area.height - h - 1, ..area }, Rect { y: area.y + h, height: 1, ..area })
            } else {
                let lw = area.width / 2;
                (Rect { width: lw, ..area }, Rect { x: area.x + lw + 1, width: area.width - lw - 1, ..area }, Rect { x: area.x + lw, width: 1, ..area })
            };
            let buf = f.buffer_mut();
            for yy in div.top()..div.bottom() {
                for xx in div.left()..div.right() {
                    buf[(xx, yy)].set_symbol(if stack { "─" } else { "│" }).set_style(Style::default().fg(t.line).bg(t.bg));
                }
            }
            draw_session(app, f, ra, a, a == focus, true, model, t);
            draw_session(app, f, rb, b, b == focus, true, model, t);
        }
        None => draw_session(app, f, area, focus, true, false, model, t),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_session(app: &mut App, f: &mut Frame, r: Rect, term: TermId, focused: bool, split: bool, model: &[Proj], t: &Theme) {
    let Some(info) = app.snap.terms.get(&term).cloned() else { return };
    let found = find(model, term);
    let (title, wt, agent) = found.map(|(_, w, s)| (s.title.clone(), w.name.clone(), s.agent.clone())).unwrap_or_default();
    let st = info.status;
    // Title bar: agent, session, worktree · state, ✕ when split.
    let bg = if focused { t.accent } else { t.sidebar_bg };
    fill(f.buffer_mut(), Rect { height: 1, ..r }, bg);
    let ink = |c: Color| if focused { t.acc_ink } else { c };
    let mut right: Vec<Seg> = Vec::new();
    if info.agent.is_some() {
        let mut s = Style::default().fg(ink(t.status(st))).bg(bg);
        if st == Status::Blocked {
            s = s.add_modifier(Modifier::BOLD);
        }
        let extra = if st == Status::Working { format!(" {}", age(info.since)) } else { String::new() };
        right.push(seg(format!("{} {}{extra}  ", glyph(app, st), state_label(st)), s));
    }
    if let Some(n) = app.scroll.get(&term) {
        right.push(seg(format!("↑{n}  "), Style::default().fg(ink(t.accent)).bg(bg)));
    }
    if split {
        right.push(seg("✕ ", Style::default().fg(ink(t.muted)).bg(bg)));
    }
    let rw = segs_width(&right);
    put(
        f.buffer_mut(),
        r.x + 1,
        r.y,
        &[
            seg(format!("{agent}  "), Style::default().fg(ink(t.strong)).bg(bg).add_modifier(Modifier::BOLD)),
            seg(title.clone(), Style::default().fg(ink(t.strong)).bg(bg)),
            seg(format!("  {wt}"), Style::default().fg(ink(t.muted)).bg(bg)),
        ],
        r.right().saturating_sub(rw + 2),
    );
    put(f.buffer_mut(), r.right().saturating_sub(rw), r.y, &right, r.right());
    app.pane_frames.push((term, r));
    if split {
        hit(app, Rect { x: r.right().saturating_sub(2), y: r.y, width: 2, height: 1 }, HyHit::CloseSplit(term));
    }

    let ask = st == Status::Blocked && info.agent.is_some();
    let foot = if focused && info.agent.is_some() && r.height > 12 { 3 } else { 0 };
    let bot = r.bottom().saturating_sub(foot + if ask { 2 } else { 0 });
    let inner = Rect { x: r.x + 2, y: r.y + 1, width: r.width.saturating_sub(3), height: bot.saturating_sub(r.y + 1) };
    app.panes.push((term, inner));
    app.hits.push((inner, Hit::Pane(term)));
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

    // Answer bar: the agent's own numbered choices, so a key sends the same keystroke.
    if ask && bot + 2 <= r.bottom() {
        fill(f.buffer_mut(), Rect { x: r.x, y: bot, width: r.width, height: 2 }, t.card2);
        let mut x = put(
            f.buffer_mut(),
            r.x + 2,
            bot,
            &[seg(format!("● {agent} is waiting   "), Style::default().fg(t.blocked).bg(t.card2).add_modifier(Modifier::BOLD))],
            r.right(),
        );
        let opts = options(app.parsers.get(&term));
        for (i, o) in opts.iter().enumerate().take(4) {
            let key = char::from_digit(i as u32 + 1, 10).unwrap_or('1');
            let kind = if i == 0 { BtnKind::Primary } else { BtnKind::Normal };
            let w = segs_width(&button(t, o, &key.to_string(), kind, false));
            let br = Rect { x, y: bot, width: w, height: 1 };
            let segs = button(t, o, &key.to_string(), kind, hovered(app, br));
            x = put(f.buffer_mut(), x, bot, &segs, r.right()) + 1;
            app.hits.push((br, Hit::Button(super::Btn::Answer(term, key))));
        }
        let rk = k(app, &Action::Reply);
        x += 1;
        let w = segs_width(&button(t, "Reply…", &rk, BtnKind::Normal, false));
        let br = Rect { x, y: bot, width: w, height: 1 };
        let segs = button(t, "Reply…", &rk, BtnKind::Normal, hovered(app, br));
        put(f.buffer_mut(), x, bot, &segs, r.right());
        hit(app, br, HyHit::Talk(term));
    }

    // Focused footer: a message box (opens Talk), and where this is.
    if foot > 0 {
        let fy = r.bottom() - 3;
        hline(f.buffer_mut(), r.x, fy, r.width, t, t.bg);
        let tk = k(app, &Action::Talk);
        put(
            f.buffer_mut(),
            r.x + 2,
            fy + 1,
            &[
                seg("› ", Style::default().fg(t.muted)),
                seg("█", Style::default().fg(t.fg)),
                seg(format!("   click or press {tk} to message {agent}"), Style::default().fg(t.muted).add_modifier(Modifier::ITALIC)),
            ],
            r.right(),
        );
        hit(app, Rect { x: r.x, y: fy + 1, width: r.width, height: 1 }, HyHit::Talk(term));
        let where_ = format!("{} · {agent}", tilde(&info.cwd));
        put(f.buffer_mut(), r.x + 2, fy + 2, &[seg(truncate(&where_, r.width.saturating_sub(4) as usize), Style::default().fg(t.muted))], r.right());
    }
}

fn draw_status(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let surf = t.sidebar_bg;
    fill(buf, r, surf);
    let all: Vec<&Session> = model.iter().flat_map(|p| p.sessions()).collect();
    let n = |st: Status| all.iter().filter(|s| s.status == st).count();
    let s = Style::default().bg(surf);
    let left = match &app.notice {
        Some((msg, at, err)) if at.elapsed().as_millis() < 4500 => vec![
            seg(if *err { "✕ " } else { "✓ " }, s.fg(if *err { t.err } else { t.done }).add_modifier(Modifier::BOLD)),
            seg(msg.clone(), s.fg(t.strong)),
        ],
        _ => vec![
            seg(format!("{} {} need you", app.cfg.icons.blocked, n(Status::Blocked)), s.fg(t.blocked).add_modifier(Modifier::BOLD)),
            seg("   ", s),
            seg(format!("{} {} done", app.cfg.icons.done, n(Status::Done)), s.fg(t.done)),
            seg("   ", s),
            seg(format!("{} {} working", glyph(app, Status::Working), n(Status::Working)), s.fg(t.text)),
            seg(format!("   across {} project{}", model.len(), if model.len() == 1 { "" } else { "s" }), s.fg(t.muted)),
        ],
    };
    let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let hk = k(app, &Action::Help);
    let right = vec![
        seg(" Keys ", Style::default().bg(t.btn).fg(t.strong).add_modifier(Modifier::BOLD)),
        seg(format!("{hk} "), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)),
        seg(" ", s),
        seg(format!(" {lead} "), Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)),
    ];
    let rw = segs_width(&right);
    put(buf, r.x + 1, r.y, &left, r.right().saturating_sub(rw + 2));
    hit(app, Rect { width: 60.min(r.width), ..r }, HyHit::Jump);
    put(buf, r.right().saturating_sub(rw), r.y, &right, r.right());
    hit(app, Rect { x: r.right().saturating_sub(rw), width: rw, ..r }, HyHit::Keys);
}

// ---- overlays ------------------------------------------------------------------------------

/// Dim everything already drawn: fg 60% toward its bg, then fg and bg 45% toward black.
pub(super) fn dim_all(buf: &mut Buffer, area: Rect, t: &Theme) {
    let black = Color::Rgb(0, 0, 0);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let c = &mut buf[(x, y)];
            let bg = if c.bg == Color::Reset { t.bg } else { c.bg };
            let fg = if c.fg == Color::Reset { t.fg } else { c.fg };
            c.fg = blend(blend(fg, bg, 0.6), black, 0.45);
            c.bg = blend(bg, black, 0.45);
        }
    }
}

/// A centred panel: `card` ground, accent title bar with "Esc close" on the right.
#[allow(clippy::too_many_arguments)]
fn panel(app: &mut App, buf: &mut Buffer, area: Rect, w: u16, h: u16, title: &str, right: &[Seg], t: &Theme) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    hit(app, area, HyHit::Close);
    fill(buf, r, t.card);
    hit(app, r, HyHit::Noop);
    let bar = Rect { height: 1, ..r };
    fill(buf, bar, t.accent);
    let ink = Style::default().fg(t.acc_ink).bg(t.accent);
    put(buf, r.x + 1, r.y, &[seg(title, ink.add_modifier(Modifier::BOLD))], r.right());
    let right: Vec<Seg> = if right.is_empty() {
        vec![seg("Esc", ink.add_modifier(Modifier::BOLD)), seg(" close ", ink)]
    } else {
        right.iter().map(|(s, st)| (s.clone(), st.fg(t.acc_ink).bg(t.accent))).collect()
    };
    let rw = segs_width(&right);
    put(buf, r.right().saturating_sub(rw), r.y, &right, r.right());
    hit(app, Rect { x: r.right().saturating_sub(rw.max(10)), y: r.y, width: rw.max(10), height: 1 }, HyHit::Close);
    r
}

fn sel_row(app: &App, buf: &mut Buffer, r: Rect, y: u16, sel: bool, t: &Theme) -> Color {
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

fn hints(t: &Theme, pairs: &[(&str, &str)]) -> Vec<Seg> {
    cap_hints(t, t.card, pairs)
}

// Jump ----------------------------------------------------------------------------------------

pub(super) fn jump_list(model: &[Proj]) -> Vec<(Session, String, String, Color)> {
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
    out
}

pub(super) fn draw_jump(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let model = app.hy_model();
    let list = jump_list(&model);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (list.len() as u16 + 10).max(10);
    let r = panel(app, buf, area, 80, h, "Jump to", &[], t);
    let mut y = r.y + 2;
    let mut i = 0;
    for (st, label) in [(Status::Blocked, "NEEDS YOU"), (Status::Done, "DONE · NOT REVIEWED")] {
        let rows: Vec<_> = list.iter().filter(|(s, ..)| s.status == st).collect();
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
    if list.is_empty() {
        put(buf, r.x + 3, r.y + 3, &[seg("Nothing needs you right now.", Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::ITALIC))], r.right());
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
    fn split(&self) -> (PathBuf, String) {
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
    fn ghost(&self) -> String {
        let (_, part) = self.split();
        let rows = self.rows();
        match rows.get(self.sel) {
            Some((n, ..)) if !part.is_empty() && n.to_lowercase().starts_with(&part.to_lowercase()) => n[part.len()..].to_string(),
            _ => String::new(),
        }
    }
}

/// Resolve `.` and `..` without touching the disk.
fn normalize(p: &Path) -> PathBuf {
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

pub(super) fn draw_finder(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, fd: &Finder) {
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

pub(super) fn np_agents(app: &App) -> Vec<String> {
    let mut v: Vec<String> = app.cfg.quick.agents.iter().map(|a| a.name.clone()).collect();
    for a in ["claude", "codex", "gemini"] {
        if !v.iter().any(|x| x == a) {
            v.push(a.into());
        }
    }
    v.push("shell".into());
    v
}

pub(super) fn draw_new_pane(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, np: &NewPaneHy) {
    let model = app.hy_model();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let Some(p) = model.get(np.p) else {
        let r = panel(app, buf, area, 64, 8, "New pane", &[], t);
        put(buf, r.x + 3, r.y + 3, &[seg("Open a project first (o).", Style::default().fg(t.muted).bg(t.card))], r.right());
        return;
    };
    let nwt = p.wts.len();
    let r = panel(app, buf, area, 64, 12 + nwt as u16 + 1, "New pane", &[], t);
    let lab = |buf: &mut Buffer, y: u16, text: &str, row: u8| {
        put(buf, r.x + 3, y, &[seg(text, Style::default().fg(if np.row == row { t.accent } else { t.muted }).bg(t.card).add_modifier(Modifier::BOLD))], r.right());
    };
    // PROJECT chips
    lab(buf, r.y + 2, "PROJECT", 0);
    let mut x = r.x + 14;
    for (i, pp) in model.iter().enumerate() {
        let on = i == np.p;
        let txt = format!(" {} ", pp.name);
        let st = if on { Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD) } else { Style::default().bg(t.btn).fg(t.text) };
        if x + txt.width() as u16 >= r.right() {
            break;
        }
        put(buf, x, r.y + 2, &[seg(txt.clone(), st)], r.right());
        hit(app, Rect { x, y: r.y + 2, width: txt.width() as u16, height: 1 }, HyHit::NpProj(i));
        x += txt.width() as u16 + 1;
    }
    // WORKTREE rows
    lab(buf, r.y + 4, "WORKTREE", 1);
    for i in 0..=nwt {
        let y = r.y + 4 + i as u16;
        let sel = i == np.w;
        let row = Rect { x: r.x + 13, y, width: r.width - 14, height: 1 };
        let bg = if sel || hovered(app, row) { t.hov } else { t.card };
        if bg == t.hov {
            fill(buf, row, t.hov);
        }
        if sel {
            put(buf, r.x + 13, y, &[seg(">", Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], r.right());
        }
        let st = Style::default().bg(bg);
        match p.wts.get(i) {
            Some(w) => {
                let mut ns = st.fg(t.strong);
                if sel {
                    ns = ns.add_modifier(Modifier::BOLD);
                }
                put(buf, r.x + 15, y, &[seg(w.name.clone(), ns)], r.right());
                let c: Vec<Seg> = counts(app, t, w.sessions.iter(), None).into_iter().map(|(x, s)| (x, s.bg(bg))).collect();
                put(buf, r.right().saturating_sub(2 + segs_width(&c)), y, &c, r.right());
            }
            None => {
                put(
                    buf,
                    r.x + 15,
                    y,
                    &[seg("+ new worktree", st.fg(t.accent).add_modifier(Modifier::BOLD)), seg("   own branch, named for you", st.fg(t.muted))],
                    r.right(),
                );
            }
        }
        hit(app, row, HyHit::NpWt(i));
    }
    // RUN chips
    let ay = r.y + 5 + nwt as u16 + 1;
    lab(buf, ay, "RUN", 2);
    let mut x = r.x + 14;
    for (i, a) in np_agents(app).iter().enumerate() {
        let on = i == np.a;
        let txt = format!(" {a} ");
        let st = if on { Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD) } else { Style::default().bg(t.btn).fg(t.text) };
        put(buf, x, ay, &[seg(txt.clone(), st)], r.right());
        hit(app, Rect { x, y: ay, width: txt.width() as u16, height: 1 }, HyHit::NpRun(i));
        x += txt.width() as u16 + 1;
    }
    let same = app.focused().and_then(|fo| find(&model, fo)).is_some_and(|(fp, ..)| fp.key == p.key);
    let note = if same { "Opens beside the focused pane.".to_string() } else { format!("Switches to {} and opens there.", p.name) };
    put(buf, r.x + 3, ay + 2, &[seg(note, Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::ITALIC))], r.right());
    let gx = btn(app, buf, r.x + 3, ay + 4, "Open", "Enter", BtnKind::Primary, HyHit::NpGo, r.right());
    put(buf, gx + 3, ay + 4, &hints(t, &[("Tab", "next row"), ("←→ ↑↓", "choose"), ("Esc", "close")]), r.right());
}

// Talk ----------------------------------------------------------------------------------------

pub(super) fn draw_talk(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, term: TermId, input: &str) {
    let model = app.hy_model();
    let (title, agent) = find(&model, term).map(|(_, _, s)| (s.title.clone(), s.agent.clone())).unwrap_or_default();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let ink = Style::default();
    let r = panel(
        app,
        buf,
        area,
        96,
        8,
        &format!("Message {agent} · {title}"),
        &[seg("Enter", ink.add_modifier(Modifier::BOLD)), seg(" send   ", ink), seg("Esc", ink.add_modifier(Modifier::BOLD)), seg(" close ", ink)],
        t,
    );
    // The last two things on its screen, for context.
    if let Some(p) = app.parsers.get(&term) {
        let screen = p.screen();
        let (_, cols) = screen.size();
        let lines: Vec<String> = screen.rows(0, cols).map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect();
        for (i, l) in lines.iter().rev().take(2).rev().enumerate() {
            put(buf, r.x + 3, r.y + 2 + i as u16, &[seg(truncate(l.trim(), (r.width - 6) as usize), Style::default().fg(t.text).bg(t.card))], r.right() - 2);
        }
    }
    let row = Rect { x: r.x + 1, y: r.y + 5, width: r.width - 2, height: 1 };
    fill(buf, row, t.card2);
    let s = Style::default().bg(t.card2);
    let mut segs = vec![seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(input.to_string(), s.fg(t.strong)), seg("█", s.fg(t.accent))];
    if input.is_empty() {
        segs.push(seg(" type a message…", s.fg(t.muted)));
    }
    put(buf, r.x + 3, row.y, &segs, r.right() - 2);
}

// Settings ------------------------------------------------------------------------------------

/// The values a settings row offers as chips, and which one is current.
fn chips_for(app: &App, row: &SRow) -> Option<(Vec<String>, Option<usize>)> {
    use super::modal::Kind;
    let SRow::Setting(s) = row else { return None };
    let cur = super::modal::current(&app.cfg, s.path);
    match s.kind {
        Kind::Bool => {
            let on = cur.and_then(|v| v.as_bool()).unwrap_or(false);
            Some((vec!["on".into(), "off".into()], Some(if on { 0 } else { 1 })))
        }
        Kind::Choice(opts) if s.path == "theme" => {
            let names: Vec<String> = crate::theme::DESIGN.iter().map(|(_, l)| l.to_string()).collect();
            let curname = cur.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let i = crate::theme::DESIGN.iter().position(|(n, _)| *n == curname || (curname == "drover" && *n == "hydra"));
            let _ = opts;
            Some((names, i))
        }
        Kind::Choice(opts) => {
            let curs = cur.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            Some((opts.iter().map(|o| o.to_string()).collect(), opts.iter().position(|o| *o == curs)))
        }
        _ => None,
    }
}

/// Apply a chip click: the setting row's `vi`-th value.
pub(super) fn set_chip(app: &mut App, row: &SRow, vi: usize) {
    use super::modal::Kind;
    let SRow::Setting(s) = row else { return };
    let v: toml_edit::Value = match s.kind {
        Kind::Bool => (vi == 0).into(),
        Kind::Choice(_) if s.path == "theme" => match crate::theme::DESIGN.get(vi) {
            Some((n, _)) => (*n).into(),
            None => return,
        },
        Kind::Choice(opts) => match opts.get(vi) {
            Some(o) => (*o).into(),
            None => return,
        },
        _ => return,
    };
    app.save_setting(s.path, v);
}

pub(super) fn draw_settings(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &SettingsView) {
    use super::modal::Cat;
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 100, 26, "Settings", &[], t);
    let c = Style::default().bg(t.card);
    // Tabs
    let mut x = r.x + 2;
    for (i, cat) in Cat::ALL.iter().enumerate() {
        let on = i == v.cat;
        let txt = format!(" {} ", cat.label());
        let st = if on { Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD) } else { c.fg(t.text) };
        let tr = Rect { x, y: r.y + 2, width: txt.width() as u16, height: 1 };
        let st = if !on && hovered(app, tr) { st.bg(t.hov) } else { st };
        put(buf, x, r.y + 2, &[seg(txt.clone(), st)], r.right());
        hit(app, tr, HyHit::SetTab(i));
        x += txt.width() as u16 + 2;
    }
    hline(buf, r.x + 1, r.y + 3, r.width - 2, t, t.card);
    let cat = Cat::ALL[v.cat.min(Cat::ALL.len() - 1)];
    let rows = super::design::settings_rows(app, cat);
    let lx = r.x + 4;
    let vx = r.x + 46;
    let list_top = r.y + 5;
    let list_h = r.height.saturating_sub(12) as usize;
    let start = v.sel.saturating_sub(list_h.saturating_sub(1));
    for (i, row) in rows.iter().enumerate().skip(start).take(list_h) {
        let y = list_top + (i - start) as u16;
        let sel = i == v.sel;
        let bg = sel_row(app, buf, r, y, sel, t);
        let st = Style::default().bg(bg);
        let label = match row {
            SRow::Setting(s) => s.label.to_string(),
            SRow::Bind { label, .. } => label.to_string(),
            SRow::Project(p) => tilde(p),
        };
        let mut ls = st.fg(t.strong);
        if sel {
            ls = ls.add_modifier(Modifier::BOLD);
        }
        put(buf, lx, y, &[seg(truncate(&label, (vx - lx - 2) as usize), ls)], vx - 1);
        hit(app, Rect { x: r.x + 1, y, width: vx - r.x - 2, height: 1 }, HyHit::SetRow(i));
        // The value: chips where the design has them, else the control.
        if let Some((opts, cur)) = chips_for(app, row) {
            let theme_row = matches!(row, SRow::Setting(s) if s.path == "theme");
            let mut cx = if theme_row { lx + 12 } else { vx };
            for (vi, o) in opts.iter().enumerate() {
                let on = Some(vi) == cur;
                let txt = format!(" {o} ");
                if cx + txt.width() as u16 >= r.right() - 1 {
                    break;
                }
                let s2 = if on { Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD) } else { Style::default().bg(t.btn).fg(t.text) };
                put(buf, cx, y, &[seg(txt.clone(), s2)], r.right() - 1);
                hit(app, Rect { x: cx, y, width: txt.width() as u16, height: 1 }, HyHit::SetVal(i, vi));
                cx += txt.width() as u16 + 1;
            }
        } else if let SRow::Project(_) = row {
            put(buf, vx, y, &[seg(" forget ", Style::default().bg(t.btn).fg(t.text))], r.right() - 1);
            hit(app, Rect { x: vx, y, width: 8, height: 1 }, HyHit::SetVal(i, 0));
        } else {
            let ctrl: Vec<Seg> = super::design::control(app, t, row, v, sel).into_iter().map(|(x, s)| (x, if s.bg.is_none() { s.bg(bg) } else { s })).collect();
            put(buf, vx, y, &ctrl, r.right() - 1);
            hit(app, Rect { x: vx, y, width: segs_width(&ctrl).max(1), height: 1 }, HyHit::SetVal(i, usize::MAX));
        }
    }
    if rows.is_empty() {
        let msg = if cat == Cat::Projects { "No projects opened yet. Press o to open one." } else { "Nothing to change here." };
        put(buf, lx, list_top, &[seg(msg, c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
    }
    // What the selected row does.
    let help = match rows.get(v.sel) {
        Some(SRow::Setting(s)) => s.help.to_string(),
        Some(SRow::Bind { acts, .. }) if acts.len() > 1 => "Several keys; change them in the config file (o).".into(),
        Some(SRow::Bind { .. }) => "Enter, then press the new key.".into(),
        Some(SRow::Project(_)) => "Forget this project (its sessions keep running). Projects with sessions show up by themselves.".into(),
        None => String::new(),
    };
    let hy = r.bottom().saturating_sub(6);
    put(buf, lx, hy, &[seg(truncate(&help, (r.width - 8) as usize), c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
    // Appearance: swatches of the current theme.
    if cat == Cat::Appearance {
        let mut sx = lx;
        for (n, col) in [("bg", t.bg), ("surface", t.sidebar_bg), ("accent", t.accent), ("needs you", t.blocked), ("done", t.done), ("error", t.err)] {
            fill(buf, Rect { x: sx, y: hy - 2, width: 4, height: 1 }, col);
            put(buf, sx + 5, hy - 2, &[seg(n, c.fg(t.muted))], r.right());
            sx += 6 + n.width() as u16 + 3;
        }
    }
    put(buf, lx, r.bottom() - 4, &hints(t, &[("↑↓", "move"), ("Enter", "change"), ("←→", "change"), ("Tab", "next section"), ("Esc", "close")]), r.right());
    put(buf, lx, r.bottom() - 2, &[seg(tilde(&crate::config::config_path()), c.fg(t.muted)), seg("   o", c.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" open file", c.fg(t.muted))], r.right());
}

// Keys ----------------------------------------------------------------------------------------

pub(super) fn draw_keys(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) {
    use crate::layout::Dir;
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    #[allow(clippy::type_complexity)]
    let cols: [(&str, Vec<(Vec<Action>, &str)>); 4] = [
        (
            "MOVE",
            vec![
                (vec![Action::SideMove(-1), Action::SideMove(1)], "sidebar"),
                (vec![Action::Jump], "jump to needs-you"),
                (vec![Action::Answer('1'), Action::Answer('2'), Action::Answer('3')], "answer agent"),
                (vec![Action::NextAttention], "next that needs you"),
                (vec![Action::Focus(Dir::Left), Action::Focus(Dir::Right)], "other split"),
                (vec![Action::Palette], "find anything"),
            ],
        ),
        (
            "DO",
            vec![
                (vec![Action::Talk], "message"),
                (vec![Action::Reply], "reply"),
                (vec![Action::NewSession], "new session"),
                (vec![Action::NewPane], "new pane"),
                (vec![Action::OpenProject], "open project"),
                (vec![Action::CloseSplit], "close split"),
                (vec![Action::ClosePane], "end session"),
                (vec![Action::RenameWorkspace], "rename session"),
                (vec![Action::QuickPrompt], "quick task"),
            ],
        ),
        (
            "PANES & CODE",
            vec![
                (vec![Action::SplitRight], "shell beside"),
                (vec![Action::Zoom], "hide sidebar"),
                (vec![Action::CopyMode], "copy mode"),
                (vec![Action::Search], "search history"),
                (vec![Action::Changes], "changes"),
                (vec![Action::Files], "files"),
                (vec![Action::Tasks], "tasks & review"),
                (vec![Action::Inbox], "inbox"),
                (vec![Action::Toolbox], "toolbox"),
            ],
        ),
        (
            "APP",
            vec![
                (vec![Action::Settings], "settings"),
                (vec![Action::Help], "this list"),
                (vec![Action::Menu], "menu"),
                (vec![Action::UndoAutoWorkspace], "undo"),
                (vec![Action::Detach], "detach"),
                (vec![Action::ReloadConfig], "reload config"),
                (vec![Action::SendPrefix], "send the leader"),
            ],
        ),
    ];
    let tallest = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0) as u16;
    let r = panel(app, buf, area, 116, tallest + 9, "Keys", &[], t);
    let cw = (r.width - 6) / 4;
    for (ci, (head, items)) in cols.iter().enumerate() {
        let cx = r.x + 3 + ci as u16 * cw;
        put(buf, cx, r.y + 2, &[seg(*head, Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::BOLD))], cx + cw);
        let kw = items.iter().map(|(a, _)| segs_width(&keycaps(t, &key_text(app, a), t.card))).max().unwrap_or(3).min(14);
        for (j, (acts, label)) in items.iter().enumerate() {
            let y = r.y + 3 + j as u16;
            let caps = keycaps(t, &key_text(app, acts), t.card);
            let pad = kw.saturating_sub(segs_width(&caps));
            let mut row = vec![seg(" ".repeat(pad as usize), Style::default().bg(t.card))];
            if caps.is_empty() {
                row.push(seg("·", Style::default().fg(t.line).bg(t.card)));
            }
            row.extend(caps);
            row.push(seg(format!(" {label}"), Style::default().fg(t.text).bg(t.card)));
            put(buf, cx, y, &row, cx + cw - 1);
        }
    }
    let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let mut foot = vec![seg("In a terminal: ", Style::default().fg(t.muted).bg(t.card))];
    foot.extend(lead.split('+').map(|p| keycap(t, p)).flat_map(|c| [c, seg("+", Style::default().fg(t.muted).bg(t.card))]).collect::<Vec<_>>());
    foot.pop();
    foot.push(seg(", then the key.   In the sidebar (↑↓) and on the splash, just the key.", Style::default().fg(t.muted).bg(t.card)));
    put(buf, r.x + 3, r.bottom() - 3, &foot, r.right() - 2);
    put(
        buf,
        r.x + 3,
        r.bottom() - 2,
        &[seg("Everything is also clickable: projects, worktrees, sessions, answer buttons, Jump, + Pane, Settings.", Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::ITALIC))],
        r.right() - 2,
    );
}

// ---- splash --------------------------------------------------------------------------------

const BIG: [(char, [&str; 5]); 5] = [
    ('H', ["██  ██", "██  ██", "██████", "██  ██", "██  ██"]),
    ('Y', ["██  ██", "██  ██", " ████ ", "  ██  ", "  ██  "]),
    ('D', ["█████ ", "██  ██", "██  ██", "██  ██", "█████ "]),
    ('R', ["█████ ", "██  ██", "█████ ", "██ ██ ", "██  ██"]),
    ('A', [" ████ ", "██  ██", "██████", "██  ██", "██  ██"]),
];

const ART: [&str; 22] = [
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡀⠀⠀⠀⠀⢠⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠻⣦⡀⠀⢸⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⣠⣦⣤⣀⣀⣤⣤⣀⡀⠀⣀⣠⡆⠀⠀⠀⠀⠀⠀⠤⠒⠛⣛⣛⣻⣿⣶⣾⣿⣦⣄⢿⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠸⠿⢿⣿⣿⣿⣯⣭⣿⣿⣿⣿⣋⣀⠀⠀⠀⠀⠀⠀⣠⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣤⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⡿⢿⣿⣿⣿⣿⣿⣓⠢⠄⢠⡾⢻⣿⣿⣿⣿⡟⠁⠀⠀⠈⠙⢿⣿⣿⣯⡻⣿⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠉⠀⠀⠀⠙⢿⣿⣿⣿⣷⣄⠁⠀⣿⣿⣿⣿⣿⡇⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣷⣄⡀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⣿⣿⣿⣷⣌⢧⠀⣿⣿⣿⣿⣿⣿⣄⠀⠀⠀⠀⢀⠉⠙⠛⠛⠿⣿⣿⣿⡆⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⣿⣿⣿⣿⡀⠠⢻⡟⢿⣿⣿⣿⣿⣧⣄⣀⠀⠘⢶⣄⣀⠀⠀⠈⢻⠿⠁⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣸⣿⣿⣿⣿⣾⠀⠀⠀⠻⣈⣙⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⣷⣦⡀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠈⠲⣄⠀⠀⣀⡤⠤⠀⠀⠀⢠⣿⣿⣿⡿⣿⠇⠀⠀⠐⠺⢉⣡⣴⣿⣿⣿⣿⣿⣿⣿⡿⢿⣿⣿⣿⣶⣿⣿⣿⣶⣶⡀⠀⠀⠀",
    "⠀⠀⠀⠀⢠⣿⣴⣿⣷⣶⣦⣤⡀⠀⢸⣿⣿⣿⠇⠏⠀⠀⠀⢀⣴⣿⣿⣿⣿⣿⠟⢿⣿⣿⣿⣷⠀⠹⣿⣿⠿⠿⠛⠻⠿⣿⠇⠀⠀⠀",
    "⠀⠀⠀⣠⣿⣿⣿⣿⣿⣿⣿⣷⣯⡂⢸⣿⣿⣿⠀⠀⠀⠀⢀⠾⣻⣿⣿⣿⠟⠀⠀⠈⣿⣿⣿⣿⡇⠀⠀⣀⣀⡀⠀⢠⡞⠉⠀⠀⠀⠀",
    "⠀⠀⢸⣟⣽⣿⣯⠀⠀⢹⣿⣿⣿⡟⠼⣿⣿⣿⣇⠀⠀⠀⠠⢰⣿⣿⣿⣿⡄⠀⠀⠀⣸⣿⣿⣿⡇⠀⢀⣤⣼⣿⣷⣾⣷⡀⠀⠀⠀⠀",
    "⠀⢀⣾⣿⡿⠟⠋⠀⠀⢸⣿⣿⣿⣿⡀⢿⣿⣿⣿⣦⠀⠀⠀⢺⣿⣿⣿⣿⣿⣄⠀⠀⣿⣿⣿⣿⡇⠐⣿⣿⣿⣿⠿⣿⣿⡿⣦⠀⠀⠀",
    "⠀⢻⣿⠏⠀⠀⠀⠀⢠⣿⣿⣿⡟⡿⠀⠀⢻⣿⣿⣿⣷⣤⡀⠘⣷⠻⣿⣿⣿⣿⣷⣼⣿⣿⣿⣿⣇⣾⣿⣿⣿⠁⠀⢼⣿⣿⣿⣆⠀⠀",
    "⠀⠀⠈⠀⠀⠀⠀⠀⢸⣿⣿⣿⡗⠁⠀⠀⠀⠙⢿⣿⣿⣿⣿⣷⣾⣆⡙⣿⣿⣿⣿⣿⣿⣿⣿⣿⠌⣾⣿⣿⣿⣆⠀⠀⠀⠉⠻⣿⡷⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣷⣄⠀⠀⠀⠀⠀⠈⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠘⣟⣿⣿⣿⡆⠀⠀⠀⠀⠙⠁⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠻⣿⣿⣿⣿⣿⣶⣤⣤⣤⣀⣠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠀⠀⠀⢈⣿⣿⣿⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⣠⣤⣤⣶⣿⣿⣿⠟⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⢀⣠⣤⣄⠀⠠⢶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⢀⣀⠀⣠⣀⡠⠞⣿⣿⣿⣿⣶⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣴⣿⣷⣦⣄⣀⢿⡽⢻⣦",
    "⠻⠶⠾⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠋",
];

/// The splash: the hydra, the wordmark, what happened while you were away, and buttons.
pub(super) fn draw_splash(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) {
    let model = app.hy_model();
    let buf = f.buffer_mut();
    fill(buf, area, t.bg);
    let all: Vec<&Session> = model.iter().flat_map(|p| p.sessions()).collect();
    let n = |st: Status| all.iter().filter(|s| s.status == st).count();
    let art_w = ART[0].chars().count() as u16;
    // The art needs 22 rows; small windows get the wordmark alone.
    let show_art = area.height >= 22 + 16 && area.width >= art_w + 4;
    let body_h = if show_art { 23 } else { 0 } + 18;
    let mut y = area.y + area.height.saturating_sub(body_h) / 2;
    let center = |w: u16| area.x + area.width.saturating_sub(w) / 2;
    let teal = t.teal();
    if show_art {
        let ax = center(art_w);
        for (r, line) in ART.iter().enumerate() {
            let col = blend(t.accent, teal, r as f32 / (ART.len() - 1) as f32);
            for (ci, ch) in line.chars().enumerate() {
                if ch == '\u{2800}' || ch == ' ' {
                    continue;
                }
                buf[(ax + ci as u16, y + r as u16)].set_char(ch).set_style(Style::default().fg(col).bg(t.bg));
            }
        }
        y += ART.len() as u16 + 1;
    }
    // HYDRA in block letters, accent → teal left to right.
    let lw = 5 * 6 + 4 * 2;
    let lx = center(lw);
    for (k, (_, rows)) in BIG.iter().enumerate() {
        for (r, row) in rows.iter().enumerate() {
            for (ci, ch) in row.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let col = k as u16 * 8 + ci as u16;
                let c = blend(t.accent, teal, col as f32 / lw as f32);
                buf[(lx + col, y + r as u16)].set_char(ch).set_style(Style::default().fg(c).bg(t.bg));
            }
        }
    }
    y += 6;
    let tag = "many heads, one body · your agents keep going when you leave";
    put(buf, center(tag.width() as u16), y, &[seg(tag, Style::default().fg(t.muted).bg(t.bg).add_modifier(Modifier::ITALIC))], area.right());
    y += 2;
    let away = vec![
        seg("while you were away   ", Style::default().fg(t.muted)),
        seg(format!("{} {} need you", app.cfg.icons.blocked, n(Status::Blocked)), Style::default().fg(t.blocked).add_modifier(Modifier::BOLD)),
        seg("   ", Style::default()),
        seg(format!("{} {} still working", glyph(app, Status::Working), n(Status::Working)), Style::default().fg(t.text)),
        seg("   ", Style::default()),
        seg(format!("{} {} finished", app.cfg.icons.done, n(Status::Done)), Style::default().fg(t.done)),
    ];
    put(buf, center(segs_width(&away)), y, &away, area.right());
    y += 3;
    let last = app.focused().and_then(|fo| find(&model, fo)).map(|(p, w, s)| (p.name.clone(), p.color, w.name.clone(), s.title.clone()));
    let open_label = format!("Open {}", last.as_ref().map(|l| l.0.clone()).unwrap_or_else(|| "hydra".into()));
    let btns: [(&str, &str, BtnKind, char); 4] = [
        (&open_label, "Enter", BtnKind::Primary, '\n'),
        ("Jump to what needs you", "j", BtnKind::Normal, 'j'),
        ("Open a folder", "o", BtnKind::Normal, 'o'),
        ("Settings", ",", BtnKind::Ghost, ','),
    ];
    let total: u16 = btns.iter().map(|(l, k, kind, _)| segs_width(&button(t, l, k, *kind, false))).sum::<u16>() + 3 * 3;
    let mut x = center(total);
    for (l, key, kind, c) in btns {
        let b = button(t, l, key, kind, false);
        let br = Rect { x, y, width: segs_width(&b), height: 1 };
        let b = if kind == BtnKind::Ghost && !hovered(app, br) {
            b.into_iter().map(|(s, st)| (s, st.bg(t.card))).collect()
        } else {
            button(t, l, key, kind, hovered(app, br))
        };
        put(f.buffer_mut(), x, y, &b, area.right());
        hit(app, br, HyHit::SplashKey(c));
        x += br.width + 3;
    }
    // Status line: version and where you were.
    let sy = area.bottom().saturating_sub(1);
    let buf = f.buffer_mut();
    fill(buf, Rect { y: sy, height: 1, ..area }, t.sidebar_bg);
    let s = Style::default().bg(t.sidebar_bg);
    let mut left = vec![seg(format!("hydra {}", env!("CARGO_PKG_VERSION")), s.fg(t.muted))];
    if let Some((pn, pc, wn, title)) = last {
        left.push(seg("   last: ", s.fg(t.muted)));
        left.push(seg("▌", s.fg(pc)));
        left.push(seg(pn, s.fg(t.strong).add_modifier(Modifier::BOLD)));
        left.push(seg(format!(" › {wn} › {title}"), s.fg(t.text)));
    }
    put(buf, area.x + 1, sy, &left, area.right().saturating_sub(10));
    let rr = vec![seg("?", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" keys ", s.fg(t.muted))];
    let rw = segs_width(&rr);
    put(buf, area.right().saturating_sub(rw), sy, &rr, area.right());
    hit(app, Rect { x: area.right().saturating_sub(rw), y: sy, width: rw, height: 1 }, HyHit::SplashKey('?'));
}

// ---- behaviour -----------------------------------------------------------------------------

use super::design::SettingsView as SView;
use crate::keys::KeySpec;
use crate::protocol::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const WT_NAMES: [&str; 16] = [
    "quick-fox", "calm-heron", "bright-owl", "steady-elk", "brave-wren", "swift-lynx", "keen-otter", "bold-raven",
    "wise-badger", "lucky-hare", "sly-marten", "warm-finch", "deep-pike", "grey-wolf", "red-kite", "tall-crane",
];

impl App {
    fn hy_agent(&self) -> String {
        self.cfg.quick.agents.first().map(|a| a.name.clone()).unwrap_or_else(|| "claude".into())
    }

    /// Show a session: it becomes the focused one.
    pub(super) fn hy_focus(&mut self, term: TermId) {
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.cmd(Command::FocusPane { term });
    }

    /// Start a session in `cwd` running `cmd` (None: a shell), beside the focused one or
    /// on its own.
    pub(super) fn hy_new_session(&mut self, cwd: PathBuf, cmd: Option<String>, beside: bool) {
        if beside && let Some(f) = self.focused() {
            self.hy.pending_split = Some((f, Instant::now()));
        }
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.cmd(Command::NewWorkspace { cwd: Some(cwd), name: None, cmd });
    }

    /// A new worktree of a project with a generated name, running `cmd` there.
    fn hy_new_worktree(&mut self, proj: &Proj, cmd: Option<String>, beside: bool) {
        let taken: HashSet<String> = proj.wts.iter().map(|w| w.name.clone()).collect();
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as usize).unwrap_or(0);
        let branch = (0..WT_NAMES.len())
            .map(|i| WT_NAMES[(n + i) % WT_NAMES.len()].to_string())
            .find(|b| !taken.contains(b))
            .unwrap_or_else(|| format!("{}-{}", WT_NAMES[n % WT_NAMES.len()], n % 1000));
        let Some(ws) = self.active_ws().map(|w| w.id).or_else(|| self.snap.workspaces.first().map(|w| w.id)) else {
            self.notify("start a session first".into(), true);
            return;
        };
        if beside && let Some(f) = self.focused() {
            self.hy.pending_split = Some((f, Instant::now()));
        }
        self.mode = Mode::Normal;
        self.notify(format!("new worktree {branch} in {}", proj.name), false);
        self.cmd(Command::NewWorktree { ws, branch, base: None, cmd, split: None, from: Some(proj.path.clone()) });
    }

    /// Open a folder as a project: switch to it, or start a session there if nothing runs.
    pub(super) fn hy_open_project(&mut self, path: PathBuf) {
        self.mode = Mode::Normal;
        self.splash = false;
        let root = crate::gitfs::head(&path).map(|h| h.main_root).unwrap_or_else(|| path.clone());
        let key = path_key(&root);
        let model = self.hy_model();
        let existing = model.iter().find(|p| p.key == key);
        let found = existing.and_then(|p| p.sessions().min_by_key(|s| (rank(s.status), s.term)).map(|s| (p.name.clone(), s.term)));
        let is_new = existing.is_none();
        if !self.hy.saved.known.iter().any(|k| path_key(k) == key) {
            self.hy.saved.known.push(root.clone());
            self.hy.save();
        }
        self.hy.proj = Some(key.clone());
        match found {
            Some((name, term)) => {
                self.hy_focus(term);
                self.notify(format!("Switched to {name}"), false);
            }
            None => {
                if is_new {
                    self.hy.fresh.insert(key);
                }
                let agent = self.hy_agent();
                self.hy_new_session(root.clone(), Some(agent), false);
                self.notify(format!("Opened {} as a project with a new session", folder_name(&root)), false);
            }
        }
    }

    fn hy_open_finder(&mut self) {
        let model = self.hy_model();
        let start = model
            .iter()
            .find(|p| self.hy.proj.as_deref() == Some(p.key.as_str()))
            .and_then(|p| p.path.parent().map(Path::to_path_buf))
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_default();
        self.mode = Mode::Finder(Box::new(Finder::new(&start)));
    }

    fn hy_open_new_pane(&mut self) {
        let model = self.hy_model();
        let p = model.iter().position(|p| self.hy.proj.as_deref() == Some(p.key.as_str())).unwrap_or(0);
        let w = self
            .focused()
            .and_then(|f| model.get(p).and_then(|pp| pp.wts.iter().position(|w| w.sessions.iter().any(|s| s.term == f))))
            .unwrap_or(0);
        self.mode = Mode::HyPane(NewPaneHy { p, w, a: 0, row: 0 });
    }

    fn hy_settings(&mut self) {
        self.mode = Mode::HySettings(Box::new(SView { cat: 0, sel: 0, editing: None, capturing: false, scroll: 0 }));
    }

    /// Move the sidebar cursor; bare keys then work on the sidebar until Esc or Enter.
    fn hy_side_move(&mut self, d: i8) {
        let vis = self.hy.visible.clone();
        if vis.is_empty() {
            return;
        }
        let cur = self.hy.cursor.or(self.focused());
        let i = cur.and_then(|c| vis.iter().position(|t| *t == c));
        let next = match (i, d) {
            (None, _) => 0,
            (Some(i), 0) => i,
            (Some(i), d) if d < 0 => i.saturating_sub(1),
            (Some(i), _) => (i + 1).min(vis.len() - 1),
        };
        self.hy.cursor = Some(vis[next]);
        self.mode = Mode::Side;
    }

    /// Actions that work differently in this layout. Returns true if handled.
    pub(super) fn hy_act(&mut self, a: &Action) -> bool {
        let focused = self.focused();
        match a {
            Action::Settings => self.hy_settings(),
            Action::NewPane => self.hy_open_new_pane(),
            Action::Jump | Action::Picker => self.mode = Mode::Jump { sel: 0 },
            Action::OpenProject => self.hy_open_finder(),
            Action::Talk | Action::Reply => {
                let term = if *a == Action::Talk { self.hy.cursor.or(focused) } else { focused };
                match term {
                    Some(term) => self.mode = Mode::Talk { term, input: String::new() },
                    None => self.notify("no session to message".into(), true),
                }
            }
            Action::Zoom => self.sidebar = !self.sidebar,
            Action::Focus(_) | Action::FocusNext | Action::FocusPrev => {
                if let (Some((a, b)), Some(f)) = (self.hy.pair, focused) {
                    let other = if f == a { b } else { a };
                    self.cmd(Command::FocusPane { term: other });
                }
            }
            Action::SplitRight | Action::SplitDown | Action::SplitLeft | Action::SplitUp | Action::Spawn(..) => {
                let cwd = focused.and_then(|f| self.snap.terms.get(&f)).map(|t| t.cwd.clone()).unwrap_or_else(|| self.here_dir());
                let cmd = match a {
                    Action::Spawn(_, c) => Some(c.clone()),
                    _ => None,
                };
                self.hy_new_session(cwd, cmd, true);
            }
            Action::NewSession => {
                let model = self.hy_model();
                let target = self.hy.cursor.or(focused);
                let path = target
                    .and_then(|t| find(&model, t).map(|(_, w, _)| w.path.clone()))
                    .or_else(|| model.iter().find(|p| self.hy.proj.as_deref() == Some(p.key.as_str())).and_then(|p| p.wts.first().map(|w| w.path.clone())))
                    .unwrap_or_else(|| self.here_dir());
                let agent = self.hy_agent();
                self.hy_new_session(path, Some(agent), false);
            }
            Action::CloseSplit => {
                if self.hy.pair.take().is_none() {
                    self.notify("no split open".into(), false);
                }
            }
            Action::SideMove(d) => self.hy_side_move(*d),
            Action::BrowseTree => self.hy_side_move(0),
            _ => return false,
        }
        true
    }

    /// Keys while the sidebar cursor is up: arrows move, Enter opens, other keys act as
    /// if the leader had been pressed.
    pub(super) fn on_side_key(&mut self, k: &KeyEvent) {
        let spec = KeySpec::from_event(k);
        match k.code {
            KeyCode::Up => self.hy_side_move(-1),
            KeyCode::Down => self.hy_side_move(1),
            KeyCode::Enter => {
                if let Some(c) = self.hy.cursor {
                    self.hy_focus(c);
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Esc => {
                self.hy.cursor = None;
                self.mode = Mode::Normal;
            }
            _ if spec == self.keymap.prefix => self.mode = Mode::Prefix { since: Instant::now() },
            _ => {
                let Some(a) = self.keymap.prefixed.get(&spec).cloned() else { return };
                self.mode = Mode::Normal;
                self.act(a);
                if self.mode == Mode::Normal && self.hy.cursor.is_some() {
                    self.mode = Mode::Side;
                }
            }
        }
    }

    pub(super) fn on_jump_key(&mut self, sel: usize, k: &KeyEvent) {
        let list = jump_list(&self.hy_model());
        let pick = |i: usize| list.get(i).map(|(s, ..)| s.term);
        match k.code {
            KeyCode::Esc | KeyCode::Char('j') => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::Jump { sel: (sel + 1).min(list.len().saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::Jump { sel: sel.saturating_sub(1) },
            KeyCode::Enter => {
                if let Some(t) = pick(sel) {
                    self.hy_focus(t);
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                if let Some(t) = pick(c as usize - '1' as usize) {
                    self.hy_focus(t);
                }
            }
            _ => {}
        }
    }

    fn finder_enter(&mut self, mut fd: Finder) {
        let sep = std::path::MAIN_SEPARATOR;
        let rows = fd.rows();
        match rows.get(fd.sel).cloned() {
            None => {
                let (dir, part) = fd.split();
                let p = dir.join(part);
                if p.is_dir() {
                    self.hy_open_project(p);
                } else {
                    self.notify(format!("no folder {}", tilde(&p)), true);
                    self.mode = Mode::Finder(Box::new(fd));
                }
            }
            Some((n, p, repo)) if n == "." || repo => self.hy_open_project(p),
            Some((_, p, _)) => {
                fd.q = format!("{}{sep}", tilde(&p).trim_end_matches(sep));
                fd.sel = 0;
                fd.refresh();
                self.mode = Mode::Finder(Box::new(fd));
            }
        }
    }

    pub(super) fn on_finder_key(&mut self, mut fd: Finder, k: &KeyEvent) {
        let sep = std::path::MAIN_SEPARATOR;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => return self.finder_enter(fd),
            KeyCode::Tab => {
                if let Some((n, p, _)) = fd.rows().get(fd.sel).cloned()
                    && n != "."
                {
                    fd.q = format!("{}{sep}", tilde(&p).trim_end_matches(sep));
                    fd.sel = 0;
                }
            }
            KeyCode::Down => fd.sel = (fd.sel + 1).min(fd.rows().len().saturating_sub(1)),
            KeyCode::Up => fd.sel = fd.sel.saturating_sub(1),
            KeyCode::Backspace => {
                fd.q.pop();
                fd.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                fd.q.push(if c == '/' || c == '\\' { sep } else { c });
                fd.sel = 0;
            }
            _ => {}
        }
        fd.refresh();
        self.mode = Mode::Finder(Box::new(fd));
    }

    fn hy_pane_go(&mut self, np: &NewPaneHy) {
        let model = self.hy_model();
        let Some(p) = model.get(np.p).cloned() else {
            self.mode = Mode::Normal;
            return;
        };
        let agents = np_agents(self);
        let agent = agents.get(np.a).cloned().unwrap_or_else(|| "shell".into());
        let cmd = (agent != "shell").then_some(agent);
        let same = self.focused().and_then(|f| find(&model, f)).is_some_and(|(fp, ..)| fp.key == p.key);
        match p.wts.get(np.w) {
            Some(w) => self.hy_new_session(w.path.clone(), cmd, same),
            None => self.hy_new_worktree(&p, cmd, same),
        }
    }

    pub(super) fn on_hy_pane_key(&mut self, mut np: NewPaneHy, k: &KeyEvent) {
        let model = self.hy_model();
        let nwt = model.get(np.p).map(|p| p.wts.len()).unwrap_or(0) + 1;
        let nag = np_agents(self).len();
        let nproj = model.len().max(1);
        let (dx, dy): (i32, i32) = match k.code {
            KeyCode::Left => (-1, 0),
            KeyCode::Right => (1, 0),
            KeyCode::Up => (0, -1),
            KeyCode::Down => (0, 1),
            _ => (0, 0),
        };
        let cyc = |v: usize, d: i32, n: usize| (v as i32 + d).rem_euclid(n as i32) as usize;
        match k.code {
            KeyCode::Esc | KeyCode::Char('p') => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => return self.hy_pane_go(&np),
            KeyCode::Tab => np.row = (np.row + 1) % 3,
            KeyCode::BackTab => np.row = (np.row + 2) % 3,
            _ if np.row == 1 && dy != 0 => np.w = cyc(np.w, dy, nwt),
            _ if np.row == 0 && (dx != 0 || dy != 0) => {
                np.p = cyc(np.p, dx + dy, nproj);
                np.w = 0;
            }
            _ if np.row == 2 && (dx != 0 || dy != 0) => np.a = cyc(np.a, dx + dy, nag),
            _ => {}
        }
        self.mode = Mode::HyPane(np);
    }

    /// Keys on the splash: its buttons' keys; anything else goes on to the app.
    pub(super) fn on_hy_splash_key(&mut self, k: &KeyEvent) {
        self.splash = false;
        if let KeyCode::Char(c) = k.code {
            self.hy_splash_action(c);
        }
    }

    fn hy_splash_action(&mut self, c: char) {
        self.splash = false;
        match c {
            'j' => self.mode = Mode::Jump { sel: 0 },
            'o' => self.hy_open_finder(),
            ',' => self.hy_settings(),
            '?' => self.mode = Mode::Help { scroll: 0 },
            _ => {}
        }
    }

    /// Forget a project the user opened (sessions in it keep running).
    pub(super) fn hy_forget(&mut self, p: &Path) {
        let key = path_key(p);
        self.hy.saved.known.retain(|k| path_key(k) != key);
        self.hy.save();
        self.notify(format!("forgot {}", folder_name(p)), false);
    }

    /// Clicks on this layout's chips, rows and buttons.
    pub(super) fn on_hy_hit(&mut self, h: HyHit, double: bool) {
        match h {
            HyHit::Splash => self.splash = true,
            HyHit::Proj(i) => {
                let Some(key) = self.hy.proj_keys.get(i).cloned() else { return };
                self.hy.proj = Some(key.clone());
                let model = self.hy_model();
                let first = model
                    .iter()
                    .find(|p| p.key == key)
                    .and_then(|p| p.sessions().min_by_key(|s| (rank(s.status), s.term)).map(|s| s.term));
                if let Some(t) = first {
                    self.hy_focus(t);
                }
            }
            HyHit::OpenFolder => self.hy_open_finder(),
            HyHit::ToggleWt(i) => {
                if let Some((key, _)) = self.hy.wt_keys.get(i).cloned() {
                    if let Some(pos) = self.hy.saved.closed.iter().position(|k| *k == key) {
                        self.hy.saved.closed.remove(pos);
                    } else {
                        self.hy.saved.closed.push(key);
                    }
                    self.hy.save();
                }
            }
            HyHit::Session(t) => self.hy_focus(t),
            HyHit::Talk(t) => self.mode = Mode::Talk { term: t, input: String::new() },
            HyHit::NewSession(i) => {
                if let Some((_, path)) = self.hy.wt_keys.get(i).cloned() {
                    let agent = self.hy_agent();
                    self.hy_new_session(path, Some(agent), false);
                }
            }
            HyHit::Settings => self.hy_settings(),
            HyHit::Jump => self.mode = Mode::Jump { sel: 0 },
            HyHit::NewPane => self.hy_open_new_pane(),
            HyHit::Keys => self.mode = Mode::Help { scroll: 0 },
            HyHit::CloseSplit(t) => {
                if let Some((a, b)) = self.hy.pair.take()
                    && Some(t) == self.focused()
                {
                    let other = if t == a { b } else { a };
                    self.cmd(Command::FocusPane { term: other });
                }
            }
            HyHit::Close => self.mode = Mode::Normal,
            HyHit::Noop => {}
            HyHit::FinderPick(i) => {
                if let Mode::Finder(fd) = &self.mode {
                    let mut fd = (**fd).clone();
                    fd.sel = i;
                    self.finder_enter(fd);
                }
            }
            HyHit::JumpTo(t) => self.hy_focus(t),
            HyHit::NpProj(i) | HyHit::NpWt(i) | HyHit::NpRun(i) => {
                if let Mode::HyPane(np) = &mut self.mode {
                    match h {
                        HyHit::NpProj(_) => {
                            np.p = i;
                            np.w = 0;
                            np.row = 0;
                        }
                        HyHit::NpWt(_) => {
                            np.w = i;
                            np.row = 1;
                        }
                        _ => {
                            np.a = i;
                            np.row = 2;
                        }
                    }
                }
            }
            HyHit::NpGo => {
                if let Mode::HyPane(np) = &self.mode {
                    let np = np.clone();
                    self.hy_pane_go(&np);
                }
            }
            HyHit::SetTab(i) => {
                if let Mode::HySettings(v) = &mut self.mode {
                    v.cat = i;
                    v.sel = 0;
                    v.editing = None;
                    v.capturing = false;
                }
            }
            HyHit::SetRow(i) | HyHit::SetVal(i, _) => {
                let Mode::HySettings(v) = &mut self.mode else { return };
                let again = v.sel == i;
                v.sel = i;
                let cat = super::modal::Cat::ALL[v.cat.min(5)];
                let row = super::design::settings_rows(self, cat).get(i).cloned();
                let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                match (h, row) {
                    (HyHit::SetVal(_, vi), Some(row @ SRow::Setting(_))) if vi != usize::MAX && chips_for(self, &row).is_some() => set_chip(self, &row, vi),
                    (HyHit::SetVal(..), Some(SRow::Project(p))) => self.hy_forget(&p),
                    (HyHit::SetVal(..), Some(_)) => self.hy_settings_key(&enter),
                    (HyHit::SetRow(_), Some(_)) if again || double => self.hy_settings_key(&enter),
                    _ => {}
                }
            }
            HyHit::SplashKey(c) => {
                self.splash = false;
                self.hy_splash_action(c);
            }
        }
    }

    /// A key for the settings overlay (shared with clicks).
    pub(super) fn hy_settings_key(&mut self, k: &KeyEvent) {
        let Mode::HySettings(v) = self.mode.clone() else { return };
        let mut v = *v;
        let open = self.on_settings_view_key(&mut v, k);
        if !open {
            self.mode = Mode::Normal;
        } else if matches!(self.mode, Mode::HySettings(_)) {
            self.mode = Mode::HySettings(Box::new(v));
        }
    }
}
