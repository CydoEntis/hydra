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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

// ---- state ---------------------------------------------------------------------------------

/// What the user chose that the daemon doesn't track: projects they opened, the colour
/// order, folded rows. Saved to `hydra-ui.json` in the data folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Saved {
    /// Folders opened as projects (they stay even with nothing running).
    pub known: Vec<PathBuf>,
    /// Project keys in the order they were first seen; a project's colour is its index.
    pub order: Vec<String>,
    /// Folded sidebar rows ("p:<project key>").
    pub closed: Vec<String>,
    /// Projects whose BRANCHES list is unfolded.
    pub open_branches: Vec<String>,
    /// Races in progress.
    pub races: Vec<super::work::Race>,
    /// Sidebar width you dragged it to, and the split's share for the left (or top) half.
    pub side_w: Option<u16>,
    pub split: Option<f32>,
    /// A task typed into + New and not started yet.
    pub draft: String,
    /// Files marked reviewed in Changes: folder key -> file -> what it was like then.
    pub reviewed: HashMap<String, HashMap<String, u64>>,
}

#[derive(Debug, Default)]
pub(super) struct Hy {
    pub saved: Saved,
    /// The project of what you're on (the default for + New).
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
    /// Sidebar scroll, and whether to bring the focused row into view on the next draw.
    pub side_scroll: u16,
    pub follow: bool,
    pub side_rect: Rect,
    /// Drawn this frame: sidebar sessions in order, project keys, worktree and branch rows.
    pub visible: Vec<TermId>,
    pub proj_keys: Vec<String>,
    pub wt_keys: Vec<(String, PathBuf)>,
    pub branch_keys: Vec<(PathBuf, String)>,
    /// Your open pull requests per project (by key), refreshed every couple of minutes.
    pub prs: std::collections::HashMap<String, Vec<super::pr::PrBrief>>,
    pub pr_at: std::collections::HashMap<String, Instant>,
    /// Pull request tags and rows drawn this frame: (folder, number).
    pub pr_keys: Vec<(PathBuf, String)>,
    /// The sidebar model, built once per event / frame (see `hy_fresh`).
    pub model_cache: std::cell::RefCell<Option<Vec<Proj>>>,
    /// A recipe's worktree being made: (branch, the commands to start there, since).
    pub pending_recipe: Option<(String, Vec<String>, Instant)>,
    /// Dragging the sidebar edge or the split divider.
    pub drag: Option<Drag>,
    /// Where the split divider is (for dragging), and which way the halves go.
    pub split_rect: Option<(Rect, bool)>,
    /// The splash's selected button.
    pub splash_sel: usize,
    /// The scrollbar being dragged: (pane, track, lines of history).
    pub bar: Option<(TermId, Rect, usize)>,
    /// Where each agent row was drawn in the sidebar (for the follow-up box beside it).
    pub row_y: std::collections::HashMap<TermId, u16>,
    /// The message box opened from a sidebar row: drawn beside it, and back to the sidebar
    /// cursor after sending.
    pub talk_anchor: Option<u16>,
    pub talk_back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Drag {
    Side,
    Split,
    /// A pane's scrollbar.
    Scroll(TermId),
}

/// The sidebar's width limits.
pub(super) const SIDE_MIN: u16 = 24;
pub(super) const SIDE_MAX: u16 = 60;

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
        *self.model_cache.borrow_mut() = None;
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
    /// Project index (== count means "other folder…"), run index, row, open beside.
    pub p: usize,
    pub a: usize,
    /// 0 task, 1 run, 2 model, 3 project, 4 where, 5 open.
    pub row: u8,
    pub beside: bool,
    /// Where, once chosen: 0 a new worktree, 1 a new branch, 2 this branch, 3.. one of the
    /// project's worktrees. None: the default for what runs (agents get a worktree, shells
    /// this branch).
    pub place: Option<u8>,
    /// What to ask the agent to do (its first prompt); empty starts it waiting.
    pub task: String,
    /// The model (0: the agent's default).
    pub model: usize,
}

pub(super) const NP_ROWS: u8 = 6;

impl NewPaneHy {
    pub fn new(p: usize, beside: bool) -> NewPaneHy {
        NewPaneHy { p, a: 0, row: 0, beside, place: None, task: String::new(), model: 0 }
    }

    pub fn place_for(&self, agent: &str, per_agent: bool) -> u8 {
        self.place.unwrap_or(if agent != "shell" && per_agent { 0 } else { 2 })
    }
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
    pub asleep: bool,
    /// Subagents it's running right now.
    pub subagents: Vec<String>,
    /// The latest prompt, when it's not the title already.
    pub latest: String,
    pub model: String,
    /// A dev server, not an agent or a shell.
    pub dev: Option<crate::protocol::DevInfo>,
    /// It rang the bell and you haven't looked.
    pub bell: bool,
}

#[derive(Debug, Clone)]
pub(super) struct Wt {
    pub key: String,
    pub path: PathBuf,
    /// The worktree's folder name (or "main folder").
    pub name: String,
    /// The branch checked out there.
    pub branch: String,
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
    /// A git repo (worktrees are possible).
    pub git: bool,
    /// Recent local branches not checked out anywhere.
    pub branches: Vec<String>,
    /// Your open pull requests here.
    pub prs: Vec<super::pr::PrBrief>,
}

impl Proj {
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.wts.iter().flat_map(|w| w.sessions.iter())
    }
}

/// Attention order: needs you, done, working, idle.
/// What an agent's row says before it has been given anything to do.
pub(super) const WAITING: &str = "waiting for you";

pub(super) fn rank(s: Status) -> u8 {
    match s {
        Status::Blocked => 0,
        Status::Done => 1,
        Status::Working => 2,
        _ => 3,
    }
}

pub(super) fn folder_name(p: &Path) -> String {
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
    /// Every terminal, by project (repo) and worktree.
    /// Forget the cached model (things may have changed).
    pub(super) fn hy_fresh(&self) {
        *self.hy.model_cache.borrow_mut() = None;
    }

    /// Every terminal, by project (repo) and worktree. Cached until the next event.
    pub(super) fn hy_model(&self) -> Vec<Proj> {
        if let Some(m) = self.hy.model_cache.borrow().as_ref() {
            return m.clone();
        }
        let m = self.hy_model_build();
        *self.hy.model_cache.borrow_mut() = Some(m.clone());
        m
    }

    fn hy_model_build(&self) -> Vec<Proj> {
        let mut projs: Vec<Proj> = Vec::new();
        let sort = self.cfg.ui.attention_sort;
        let order = &self.hy.saved.order;
        let fallback: Vec<Color> = self.cfg.ui.workspace_colors.iter().filter_map(|c| crate::theme::parse_color(c)).collect();
        let add_proj = |projs: &mut Vec<Proj>, path: &Path, git: bool| -> usize {
            let key = path_key(path);
            if let Some(i) = projs.iter().position(|p| p.key == key) {
                projs[i].git |= git;
                return i;
            }
            let ci = order.iter().position(|k| *k == key).unwrap_or(order.len() + projs.len());
            projs.push(Proj {
                key: key.clone(),
                path: path.to_path_buf(),
                name: folder_name(path),
                color: self.theme.project(ci, &fallback),
                wts: Vec::new(),
                fresh: self.hy.fresh.contains(&key),
                git,
                branches: Vec::new(),
                prs: Vec::new(),
            });
            projs.len() - 1
        };
        let add_wt = |p: &mut Proj, path: &Path, branch: String, main: bool| -> usize {
            let key = path_key(path);
            if let Some(i) = p.wts.iter().position(|w| w.key == key) {
                return i;
            }
            // The repo folder goes by the branch it's on; a worktree by its folder.
            let name = if !branch.is_empty() && (main || folder_name(path).starts_with("spare-")) { branch.clone() } else { folder_name(path) };
            p.wts.push(Wt { key, path: path.to_path_buf(), name, branch, main, sessions: Vec::new() });
            p.wts.len() - 1
        };

        for w in &self.snap.workspaces {
            let leaves: Vec<TermId> = w.tabs.iter().flat_map(|t| t.layout.leaves()).collect();
            for id in &leaves {
                let Some(t) = self.snap.terms.get(id) else { continue };
                let (root, top, branch, main, git) = match (&t.root, &t.top) {
                    (Some(r), Some(tp)) => {
                        let main = path_key(r) == path_key(tp);
                        (r.clone(), tp.clone(), t.branch.clone().unwrap_or_default(), main, true)
                    }
                    _ => {
                        let cwd = if t.cwd.as_os_str().is_empty() { w.cwd.clone() } else { t.cwd.clone() };
                        (cwd.clone(), cwd, String::new(), true, false)
                    }
                };
                let pi = add_proj(&mut projs, &root, git);
                let wi = add_wt(&mut projs[pi], &top, branch, main);
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
                    asleep: t.asleep,
                    subagents: t.subagents.clone(),
                    dev: t.dev.clone(),
                    bell: t.bell,
                    latest: if t.agent.is_some() && !t.name.trim().is_empty() && t.name.trim() != t.summary.trim() { t.summary.trim().to_string() } else { String::new() },
                    model: t.model.clone(),
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
            let pi = add_proj(&mut projs, &root, head.is_some());
            if projs[pi].wts.is_empty() {
                match head {
                    Some(h) => add_wt(&mut projs[pi], &h.top, h.branch, true),
                    None => add_wt(&mut projs[pi], k, String::new(), true),
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
                (!w.main, if sort { worst } else { 0 }, w.name.clone())
            });
            p.prs = self.hy.prs.get(&p.key).cloned().unwrap_or_default();
        }
        projs.sort_by_key(|p| order.iter().position(|k| *k == p.key).unwrap_or(usize::MAX));
        projs
    }

    /// Keep the remembered bits in step with a new state: project order, the current
    /// project, the split pair, the branch lists.
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
            self.hy.follow = true;
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
        self.recipe_followup();
        // Your pull requests, every two minutes per repo.
        if !cfg!(test) {
            for p in model.iter().filter(|p| p.git) {
                if self.hy.pr_at.get(&p.key).is_some_and(|t| t.elapsed().as_secs() < 120) {
                    continue;
                }
                self.hy.pr_at.insert(p.key.clone(), Instant::now());
                let (tx, key, dir) = (self.bg.clone(), p.key.clone(), p.path.clone());
                std::thread::spawn(move || {
                    if let Ok(list) = super::pr::list_mine(&dir) {
                        let _ = tx.send(super::Bg::Prs(key, list));
                    }
                });
            }
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
        // Its name (from /rename, else the first thing it was asked) stays put; the latest
        // prompt shows under it.
        let s = if t.name.trim().is_empty() { t.summary.trim() } else { t.name.trim() };
        return if s.is_empty() { WAITING.into() } else { s.to_string() };
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
    /// Fold / unfold a project.
    ToggleProj(usize),
    /// The + on a project row: new, in that project.
    NewIn(usize),
    OpenFolder,
    /// A main folder or worktree row (index into `wt_keys`).
    Wt(usize),
    Session(TermId),
    Talk(TermId),
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
    NpRun(usize),
    NpBeside(usize),
    NpWhere(usize),
    NpGo,
    NpTask,
    NpModel(usize),
    SetTab(usize),
    SetRow(usize),
    /// Settings row, value index.
    SetVal(usize, usize),
    /// A button on the splash screen.
    SplashKey(char),
    /// A pull request tag or row (index into `pr_keys`).
    Pr(usize),
    /// A key for the view in the main area (Files, Changes, PR).
    ViewKey(char),
    /// The Ship button.
    ShipGo,
    IdeaRow(usize),
    TicketTab(usize),
    TicketRow(usize),
    RaceAgent(usize),
    RaceGo,
    RaceRow(usize),
    RaceKey(char),
    /// A race line in the sidebar (race id).
    RaceOpen(u64),
    /// A box on the Map.
    MapNode(usize),
    /// A right-click menu item.
    MenuPick(usize),
    /// The sidebar's edge (drag to resize).
    SideEdge,
    /// The split's divider (drag to resize).
    SplitEdge,
    /// A pane's scrollbar (click or drag).
    ScrollBar(TermId),
    FindTab(u8),
    FindRow(usize),
    BranchRow(usize),
    MemRow(usize),
    HistRow(usize),
    BranchChoice(usize),
}

pub(super) fn hit(app: &mut App, r: Rect, h: HyHit) {
    app.hits.push((r, Hit::Hy(h)));
}

pub(super) fn hovered(app: &App, r: Rect) -> bool {
    app.hover.is_some_and(|p| r.contains(p))
}

/// A design button (" Label key "), hovered with `hov`; records its hit.
#[allow(clippy::too_many_arguments)]
pub(super) fn btn(app: &mut App, buf: &mut Buffer, x: u16, y: u16, label: &str, key: &str, kind: BtnKind, h: HyHit, max_x: u16) -> u16 {
    let t = app.theme.clone();
    let w = segs_width(&button(&t, label, key, kind, false));
    let r = Rect { x, y, width: w.min(max_x.saturating_sub(x)), height: 1 };
    let segs = button(&t, label, key, kind, hovered(app, r));
    let nx = put(buf, x, y, &segs, max_x);
    hit(app, r, h);
    nx
}

/// The key bound to an action after the leader, as the design shows it ("j", "Space").
pub(super) fn k(app: &App, a: &Action) -> String {
    let s = super::design::key_of(app, a);
    match s.as_str() {
        "Space" | " " => "Space".into(),
        _ => s,
    }
}

// ---- main screen ---------------------------------------------------------------------------

fn side_w(width: u16) -> u16 {
    match width {
        0..140 => 30,
        140..200 => 38,
        _ => 44,
    }
}

/// Draw the main screen; returns the pane area.
pub(super) fn draw(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) -> Rect {
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
        // The edge between sidebar and panes: drag it.
        let ex = if right_side { side.x.saturating_sub(1) } else { side.right() };
        let edge = Rect { x: ex, y: side.y, width: 1, height: side.height };
        let c = if app.hy.drag == Some(Drag::Side) || hovered(app, edge) { t.accent } else { t.line };
        for yy in edge.top()..edge.bottom() {
            f.buffer_mut()[(ex, yy)].set_symbol("│").set_style(Style::default().fg(c).bg(t.bg));
        }
        hit(app, edge, HyHit::SideEdge);
    }
    // A little air between the panes and everything around them.
    let panes = Rect { x: panes.x + 1, y: panes.y + 1, width: panes.width.saturating_sub(2), height: panes.height.saturating_sub(1) };
    draw_main(app, f, panes, &model, t);
    draw_status(app, f.buffer_mut(), Rect { y: area.bottom().saturating_sub(1), height: 1, ..area }, &model, t);
    panes
}

pub(super) fn find(model: &[Proj], term: TermId) -> Option<(&Proj, &Wt, &Session)> {
    model.iter().find_map(|p| p.wts.iter().find_map(|w| w.sessions.iter().find(|s| s.term == term).map(|s| (p, w, s))))
}

fn draw_top(app: &mut App, buf: &mut Buffer, area: Rect, crumb_x: u16, model: &[Proj], t: &Theme) {
    let y = area.y;
    let x = put(buf, area.x + 1, y, &[seg(">_ hydra", Style::default().fg(t.accent).bg(t.bg).add_modifier(Modifier::BOLD))], area.right());
    hit(app, Rect { x: area.x, y, width: x - area.x, height: 1 }, HyHit::Splash);
    let _ = model;
    let jx = area.right().saturating_sub(1);
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
                seg(if w.main { format!("⎇ {}", w.branch) } else { format!("⑂ {}", w.name) }, Style::default().fg(t.text)),
                seg("  ›  ", Style::default().fg(t.muted)),
                seg(format!("{}  ", s.agent), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)),
                seg(if s.is_agent { format!("{} {}", glyph(app, s.status), state_label(s.status)) } else { String::new() }, Style::default().fg(t.status(s.status))),
                seg(if s.is_agent && s.title != WAITING { format!("  ·  {}", s.title) } else { String::new() }, Style::default().fg(t.text)),
            ],
            jx.saturating_sub(2),
        );
    }
}

/// `●1 ✓1 ⠹2`: counts of a list of sessions by state.
pub(super) fn counts<'a>(app: &App, t: &Theme, list: impl Iterator<Item = &'a Session>, ink: Option<Color>) -> Vec<Seg> {
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

/// A line of the sidebar tree.
#[derive(Debug, Clone)]
enum Line {
    Proj(usize),
    /// A heading: BRANCHES or WORKTREES.
    Label(&'static str),
    /// The branch the repo folder is on, or a linked worktree (pi, wi).
    Place(usize, usize),
    /// An agent or shell (pi, wi, si).
    Sess(usize, usize, usize),
    /// A dim line under an agent: what it's on, its question, a subagent. Shares the
    /// agent's highlight.
    Note(String, Color, TermId),
    /// A race in this project (race id).
    Race(u64),
    /// Nothing running in a folder project.
    Empty,
    OpenProject,
    Gap,
}

/// What a sidebar row shows of a session: the agent and its state on top, what it's working
/// on (or asking) underneath.
fn session_lines(s: &Session, t: &Theme, out: &mut Vec<Line>) {
    let notes_c = t.muted;
    if let Some(q) = &s.question {
        out.push(Line::Note(q.clone(), blend(t.blocked, t.sidebar_bg, 0.25), s.term));
    } else if s.is_agent && s.title != WAITING {
        out.push(Line::Note(s.title.clone(), t.text, s.term));
        if !s.latest.is_empty() {
            out.push(Line::Note(format!("› {}", s.latest), notes_c, s.term));
        }
    }
    for sub in &s.subagents {
        out.push(Line::Note(format!("↳ {sub}"), notes_c, s.term));
    }
}

fn side_lines(app: &App, model: &[Proj], t: &Theme) -> Vec<Line> {
    let mut out = Vec::new();
    for (pi, p) in model.iter().enumerate() {
        out.push(Line::Proj(pi));
        if app.hy.saved.closed.contains(&format!("p:{}", p.key)) {
            out.push(Line::Gap);
            continue;
        }
        for r in app.hy.saved.races.iter().filter(|r| path_key(&r.project) == p.key) {
            out.push(Line::Race(r.id));
        }
        if !p.git {
            let mut any = false;
            for (wi, w) in p.wts.iter().enumerate() {
                for (si, s) in w.sessions.iter().enumerate() {
                    any = true;
                    out.push(Line::Sess(pi, wi, si));
                    session_lines(s, t, &mut out);
                }
            }
            if !any {
                out.push(Line::Empty);
            }
            out.push(Line::Gap);
            continue;
        }
        // BRANCHES: what runs in the repo folder itself, under the branch it's on.
        if let Some(wi) = p.wts.iter().position(|w| w.main) {
            out.push(Line::Label("BRANCHES"));
            out.push(Line::Place(pi, wi));
            for (si, s) in p.wts[wi].sessions.iter().enumerate() {
                out.push(Line::Sess(pi, wi, si));
                session_lines(s, t, &mut out);
            }
        }
        // WORKTREES: everything started in its own worktree.
        let linked: Vec<usize> = (0..p.wts.len()).filter(|&i| !p.wts[i].main).collect();
        if !linked.is_empty() {
            out.push(Line::Gap);
            out.push(Line::Label("WORKTREES"));
            for (n, wi) in linked.into_iter().enumerate() {
                if n > 0 {
                    out.push(Line::Gap);
                }
                out.push(Line::Place(pi, wi));
                for (si, s) in p.wts[wi].sessions.iter().enumerate() {
                    out.push(Line::Sess(pi, wi, si));
                    session_lines(s, t, &mut out);
                }
            }
        }
        out.push(Line::Gap);
        out.push(Line::Gap);
    }
    out.push(Line::OpenProject);
    out
}

/// The session a sidebar line stands for, if any.
fn line_term(model: &[Proj], l: &Line) -> Option<TermId> {
    match l {
        Line::Sess(pi, wi, si) => Some(model[*pi].wts[*wi].sessions[*si].term),
        _ => None,
    }
}

fn draw_side(app: &mut App, buf: &mut Buffer, r: Rect, model: &[Proj], t: &Theme) {
    let surf = t.sidebar_bg;
    fill(buf, r, surf);
    app.hy.side_rect = r;
    let lines = side_lines(app, model, t);
    let focus = app.focused();
    let split = app.hy.pair.map(|(a, b)| if Some(a) == focus { b } else { a });
    // + New and Jump, where you'll see them.
    let nk = k(app, &Action::NewPane);
    let jk = k(app, &Action::Jump);
    let needs = model.iter().flat_map(|p| p.sessions()).filter(|s| s.status == Status::Blocked).count();
    let bx = btn(app, buf, r.x + 2, r.y + 1, "+ New", &nk, BtnKind::Primary, HyHit::NewPane, r.right());
    let jlabel = if needs > 0 { format!("Jump ●{needs}") } else { "Jump".to_string() };
    let jb = button(t, &jlabel, &jk, BtnKind::Normal, false);
    let jr = Rect { x: bx + 1, y: r.y + 1, width: segs_width(&jb), height: 1 };
    let jb: Vec<Seg> = if needs > 0 {
        jb.into_iter().map(|(x, st)| (x, st.bg(t.blocked).fg(t.bg))).collect()
    } else {
        button(t, &jlabel, &jk, BtnKind::Normal, hovered(app, jr))
    };
    put(buf, jr.x, jr.y, &jb, r.right());
    hit(app, jr, HyHit::Jump);
    let r = Rect { y: r.y + 3, height: r.height.saturating_sub(3), ..r };
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
    app.hy.row_y.clear();
    app.hy.proj_keys = model.iter().map(|p| p.key.clone()).collect();
    let tk = k(app, &Action::Talk);
    let (x0, w) = (r.x, r.width);
    let right = r.right().saturating_sub(2);

    // A session's highlight: focused (filled), in the split, under the cursor or mouse.
    let look = |app: &App, term: TermId, row: Rect| -> (Color, Option<Color>, bool) {
        let prim = Some(term) == focus;
        let sel = !prim && (app.hy.cursor == Some(term) || hovered(app, row));
        let bg = if prim {
            t.accent
        } else if Some(term) == split {
            t.card2
        } else if sel {
            t.hov
        } else {
            surf
        };
        (bg, prim.then_some(t.acc_ink), sel)
    };

    for (i, line) in lines.iter().enumerate().skip(scroll).take(list_h) {
        let y = r.y + (i - scroll) as u16;
        let row = Rect { x: x0, y, width: w, height: 1 };
        let plain = Style::default().bg(surf);
        match line {
            Line::Proj(pi) => {
                let p = &model[*pi];
                let open = !app.hy.saved.closed.contains(&format!("p:{}", p.key));
                let hov = hovered(app, row);
                let bg = if hov { t.hov } else { surf };
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
                let mut c: Vec<Seg> = counts(app, t, p.sessions(), None).into_iter().map(|(x, st)| (x, st.bg(bg))).collect();
                if hov {
                    c.push(seg(" + ", Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)));
                }
                let cw = segs_width(&c);
                put(buf, x0 + 1, y, &left, right.saturating_sub(cw + 1));
                put(buf, right.saturating_sub(cw) + 1, y, &c, r.right());
                hit(app, row, HyHit::ToggleProj(*pi));
                if hov {
                    hit(app, Rect { x: right.saturating_sub(2), y, width: 3, height: 1 }, HyHit::NewIn(*pi));
                }
            }
            Line::Label(text) => {
                put(buf, x0 + 3, y, &[seg(*text, plain.fg(t.muted).add_modifier(Modifier::BOLD))], r.right());
            }
            Line::Place(pi, wi) => {
                let p = &model[*pi];
                let wt = &p.wts[*wi];
                let idx = app.hy.wt_keys.len();
                app.hy.wt_keys.push((wt.key.clone(), wt.path.clone()));
                let hov = hovered(app, row);
                let bg = if hov { t.hov } else { surf };
                fill(buf, row, bg);
                let s = Style::default().bg(bg);
                let racing = app.hy.saved.races.iter().any(|r| r.entries.iter().any(|(_, b)| *b == wt.branch));
                let (icon, name) = if wt.main { ("⎇ ", wt.branch.clone()) } else { (if racing { "⚑ " } else { "⑂ " }, wt.name.clone()) };
                let tag = p.prs.iter().find(|pr| pr.branch == wt.branch).map(|pr| (pr_tag(t, pr), pr.number));
                let tail: Vec<Seg> = match &tag {
                    Some((tg, _)) => tg.iter().map(|(x, st)| (x.clone(), st.bg(bg))).collect(),
                    None if wt.sessions.is_empty() && hov => vec![seg(format!("+ {}", if wt.main { "shell".into() } else { app.hy_agent() }), s.fg(t.accent))],
                    None if wt.sessions.is_empty() => vec![seg("nothing running", s.fg(t.muted))],
                    None => vec![],
                };
                let tw = segs_width(&tail);
                let mut label = vec![seg(icon, s.fg(if racing { t.accent } else { t.muted })), seg(name, s.fg(t.text).add_modifier(Modifier::BOLD))];
                // A worktree on a branch of another name says so.
                if !wt.main && !wt.branch.is_empty() && wt.branch != wt.name && tag.is_none() {
                    label.push(seg(format!(" · {}", wt.branch), s.fg(t.muted)));
                }
                put(buf, x0 + 3, y, &label, right.saturating_sub(tw + 1));
                put(buf, right.saturating_sub(tw) + 1, y, &tail, r.right());
                hit(app, row, HyHit::Wt(idx));
                if let Some((tg, n)) = tag {
                    let k2 = app.hy.pr_keys.len();
                    app.hy.pr_keys.push((wt.path.clone(), n.to_string()));
                    let tw = segs_width(&tg);
                    hit(app, Rect { x: right.saturating_sub(tw) + 1, y, width: tw, height: 1 }, HyHit::Pr(k2));
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
                    None if s.bell && s.status != Status::Blocked => ("♪".to_string(), t.blocked),
                    None => (gl, gc),
                };
                let mut gs = st.fg(ink.unwrap_or(gc));
                if s.status == Status::Blocked {
                    gs = gs.add_modifier(Modifier::BOLD);
                }
                // Agent and state on the left; age (or the talk chip) on the right.
                let mut left = vec![seg(format!("{gl} "), gs), seg(s.agent.clone(), st.fg(ink.unwrap_or(t.strong)).add_modifier(Modifier::BOLD))];
                if let Some(d) = &s.dev {
                    let port = d.port.map(|p| format!(" :{p}")).unwrap_or_default();
                    let state = if d.ready { "ready" } else { "starting…" };
                    left.truncate(1);
                    left.push(seg(format!("dev{port}"), st.fg(ink.unwrap_or(t.strong)).add_modifier(Modifier::BOLD)));
                    left.push(seg(format!("  {state}"), st.fg(ink.unwrap_or(if d.ready { t.done } else { t.muted }))));
                } else if !s.model.is_empty() && w > 34 {
                    left.push(seg(format!(" {}", s.model), st.fg(ink.unwrap_or(t.muted))));
                }
                if s.asleep {
                    left.push(seg("  asleep", st.fg(ink.unwrap_or(t.muted))));
                } else if s.is_agent {
                    left.push(seg(format!("  {}", state_label(s.status)), st.fg(ink.unwrap_or(gc))));
                } else {
                    left.push(seg(format!("  {}", truncate(&s.title, w.saturating_sub(16) as usize)), st.fg(ink.unwrap_or(t.muted))));
                }
                let tail: Vec<Seg> = if sel {
                    vec![seg(format!(" {tk} "), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD))]
                } else if s.is_agent && !s.asleep {
                    vec![seg(age(s.since), st.fg(ink.unwrap_or(t.muted)))]
                } else {
                    vec![]
                };
                let tw = segs_width(&tail);
                put(buf, x0 + 5, y, &left, right.saturating_sub(tw + 1));
                put(buf, right.saturating_sub(tw) + 1, y, &tail, r.right());
                hit(app, row, HyHit::Session(s.term));
                if sel {
                    hit(app, Rect { x: right.saturating_sub(tw) + 1, y, width: tw, height: 1 }, HyHit::Talk(s.term));
                }
            }
            Line::Note(text, c, term) => {
                let (bg, ink, _) = look(app, *term, row);
                // Notes follow their session's highlight, not the mouse.
                let bg = if bg == t.hov && app.hy.cursor != Some(*term) { surf } else { bg };
                fill(buf, row, bg);
                put(buf, x0 + 7, y, &[seg(truncate(text, w.saturating_sub(9) as usize), Style::default().bg(bg).fg(ink.unwrap_or(*c)))], r.right() - 1);
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
            Line::Empty => {
                put(buf, x0 + 5, y, &[seg("nothing running", plain.fg(t.muted))], r.right());
            }
            Line::OpenProject => {
                let bg = if hovered(app, row) { t.hov } else { surf };
                fill(buf, row, bg);
                let ok = k(app, &Action::OpenProject);
                put(
                    buf,
                    x0 + 2,
                    y,
                    &[seg("+ open a project", Style::default().fg(t.muted).bg(bg)), seg(format!("  {ok}"), Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))],
                    r.right(),
                );
                hit(app, row, HyHit::OpenFolder);
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
    let by = r.bottom().saturating_sub(3);
    hline(buf, x0 + 1, by, w.saturating_sub(2), t, surf);
    let sk = k(app, &Action::Settings);
    btn(app, buf, x0 + 2, by + 1, "Settings", &sk, BtnKind::Ghost, HyHit::Settings, r.right());
}

fn draw_main(app: &mut App, f: &mut Frame, area: Rect, model: &[Proj], t: &Theme) {
    // Files, Changes or a pull request replace the sessions until closed.
    if let Some(view) = app.view.take() {
        let buf = f.buffer_mut();
        match view {
            super::View::Changes(v) => {
                super::design::draw_changes(app, buf, area, t, &v);
                app.view = Some(super::View::Changes(v));
            }
            super::View::Files(v) => {
                super::design::draw_files(app, buf, area, t, &v);
                app.view = Some(super::View::Files(v));
            }
            super::View::Pr(v) => {
                draw_pr(app, buf, area, t, &v);
                app.view = Some(super::View::Pr(v));
            }
            super::View::Map(v) => {
                draw_map(app, buf, area, t, &v);
                app.view = Some(super::View::Map(v));
            }
            other => app.view = Some(other),
        }
        if app.view.is_some() {
            return;
        }
    }
    let Some(focus) = app.focused() else {
        let nk = k(app, &Action::NewPane);
        put(f.buffer_mut(), area.x + 4, area.y + 3, &[seg(format!("Nothing open. Press {} {nk} to start an agent or a shell.", app.keymap.prefix.to_string().replace("C-", "Ctrl+")), Style::default().fg(t.muted))], area.right());
        return;
    };
    let pair = app.hy.pair.filter(|(a, b)| *a == focus || *b == focus);
    match pair {
        Some((a, b)) => {
            let stack = area.width < 140 - side_w(140);
            let ratio = app.hy.saved.split.unwrap_or(0.5).clamp(0.2, 0.8);
            let (ra, rb, div) = if stack {
                let h = (area.height as f32 * ratio) as u16;
                (Rect { height: h, ..area }, Rect { y: area.y + h + 1, height: area.height - h - 1, ..area }, Rect { y: area.y + h, height: 1, ..area })
            } else {
                let lw = (area.width as f32 * ratio) as u16;
                (Rect { width: lw, ..area }, Rect { x: area.x + lw + 1, width: area.width - lw - 1, ..area }, Rect { x: area.x + lw, width: 1, ..area })
            };
            app.hy.split_rect = Some((area, stack));
            let c = if app.hy.drag == Some(Drag::Split) || hovered(app, div) { t.accent } else { t.line };
            let buf = f.buffer_mut();
            for yy in div.top()..div.bottom() {
                for xx in div.left()..div.right() {
                    buf[(xx, yy)].set_symbol(if stack { "─" } else { "│" }).set_style(Style::default().fg(c).bg(t.bg));
                }
            }
            hit(app, div, HyHit::SplitEdge);
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
    let (title, wt, agent) = found
        .map(|(_, w, s)| (s.title.clone(), if w.main { w.branch.clone() } else { w.name.clone() }, s.agent.clone()))
        .unwrap_or_default();
    let st = info.status;
    let bar = split;
    // Title bar (split only; on its own the top bar already says all this).
    let bg = if focused { t.accent } else { t.sidebar_bg };
    if bar {
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
    }
    app.pane_frames.push((term, r));
    if split {
        hit(app, Rect { x: r.right().saturating_sub(2), y: r.y, width: 2, height: 1 }, HyHit::CloseSplit(term));
    }

    let ask = st == Status::Blocked && info.agent.is_some();
    let foot = 0;
    let bot = r.bottom().saturating_sub(foot + if ask { 2 } else { 0 });
    let top = r.y + bar as u16;
    let _ = top;
    let inner = Rect { x: r.x + 2, y: top, width: r.width.saturating_sub(3), height: bot.saturating_sub(top) };
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
            f.buffer_mut()[(track.x, yy)].set_symbol(sym).set_style(Style::default().fg(c).bg(t.bg));
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
pub(super) fn panel(app: &mut App, buf: &mut Buffer, area: Rect, w: u16, h: u16, title: &str, right: &[Seg], t: &Theme) -> Rect {
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

pub(super) fn sel_row(app: &App, buf: &mut Buffer, r: Rect, y: u16, sel: bool, t: &Theme) -> Color {
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

pub(super) fn hints(t: &Theme, pairs: &[(&str, &str)]) -> Vec<Seg> {
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

/// Your pull requests with failing checks or changes requested: (folder, PR, project, colour).
pub(super) fn jump_prs(model: &[Proj]) -> Vec<(PathBuf, super::pr::PrBrief, String, Color)> {
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

pub(super) fn draw_jump(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let model = app.hy_model();
    let list = jump_list(&model);
    let prs = jump_prs(&model);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (list.len() as u16 + prs.len() as u16 + 12).max(10);
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
            let rseg = vec![seg("▌", st_.fg(*pc)), seg(pname.clone(), st_.fg(t.text)), seg(format!("  {}", pr.state_text()), st_.fg(if pr.checks == super::pr::Checks::Fail { t.err } else { t.blocked }))];
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
    v.extend(app.cfg.recipes.iter().filter(|r| !r.run.is_empty()).map(|r| format!("⚙ {}", r.name)));
    v.extend(app.cfg.presets.iter().map(|p| format!("★ {}", p.name)));
    v
}

pub(super) fn preset_of<'a>(app: &'a App, run: &str) -> Option<&'a crate::config::Preset> {
    let name = run.strip_prefix("★ ")?;
    app.cfg.presets.iter().find(|p| p.name == name)
}

/// Where a new agent can go in a git project: the three kinds, then its worktrees by name.
pub(super) fn np_places(p: &Proj) -> Vec<String> {
    let mut v: Vec<String> = ["new worktree", "new branch", "this branch"].map(String::from).to_vec();
    v.extend(p.wts.iter().filter(|w| !w.main).map(|w| format!("⌥ {}", w.name)));
    v
}

/// Every session with what it uses, biggest first: (term, label, where, bytes, asleep).
pub(super) fn memory_rows(app: &App) -> Vec<(TermId, String, String, u64, bool)> {
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

pub(super) fn draw_history(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let items: Vec<(u64, Option<TermId>, char, String)> = app.history.iter().rev().cloned().collect();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (items.len() as u16 + 7).clamp(10, area.height.saturating_sub(4));
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

pub(super) fn mb(bytes: u64) -> String {
    let m = bytes as f64 / (1u64 << 20) as f64;
    if m >= 1024.0 { format!("{:.1} GB", m / 1024.0) } else { format!("{m:.0} MB") }
}

pub(super) fn draw_memory(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let rows = memory_rows(app);
    let total: u64 = rows.iter().map(|r| r.3).sum();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let h = (rows.len() as u16 + 8).clamp(10, area.height.saturating_sub(4));
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

/// Models to pick from for `agent` ("default" first), or none when it has no choice.
pub(super) fn np_models(app: &App, agent: &str) -> Vec<String> {
    let q = app.cfg.quick.agents.iter().find(|q| q.name == agent);
    let list: Vec<String> = match q {
        Some(q) if !q.models.is_empty() => q.models.clone(),
        _ if agent == "claude" => ["opus", "sonnet", "haiku"].map(String::from).to_vec(),
        _ => Vec::new(),
    };
    if list.is_empty() {
        return list;
    }
    std::iter::once("default".to_string()).chain(list).collect()
}

/// The command line that starts `agent` with `model` (index into np_models) on `task`.
pub(super) fn np_command(app: &App, agent: &str, model: usize, task: &str) -> Option<String> {
    if agent == "shell" {
        return None;
    }
    if let Some(p) = preset_of(app, agent) {
        let m = np_models(app, &p.agent).iter().position(|m| *m == p.model).unwrap_or(0);
        return np_command(app, &p.agent, m, &p.fill(task));
    }
    let q = app.cfg.quick.agents.iter().find(|q| q.name == agent);
    let base = q.map(|q| q.command.clone()).unwrap_or_else(|| agent.to_string());
    let task = task.trim();
    let mut cmd = if task.is_empty() {
        base.replace("{prompt}", "").trim().to_string()
    } else if base.contains("{prompt}") {
        base.replace("{prompt}", &app.cfg.quote_for_shell(task))
    } else {
        format!("{base} {}", app.cfg.quote_for_shell(task))
    };
    if let Some(m) = np_models(app, agent).get(model).filter(|_| model > 0) {
        let flag = q.map(|q| q.model_flag.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| "--model".into());
        let (first, rest) = cmd.split_once(' ').map(|(a, b)| (a.to_string(), format!(" {b}"))).unwrap_or((cmd.clone(), String::new()));
        cmd = format!("{first} {flag} {m}{rest}");
    }
    Some(cmd)
}

pub(super) fn draw_new_pane(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, np: &NewPaneHy) {
    let model = app.hy_model();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 86, 21, "New", &[], t);
    let lab = |buf: &mut Buffer, y: u16, text: &str, row: u8| {
        put(buf, r.x + 3, y, &[seg(text, Style::default().fg(if np.row == row { t.accent } else { t.muted }).bg(t.card).add_modifier(Modifier::BOLD))], r.right());
    };
    // A row of chips; the selected one is filled. Long rows scroll to keep it in view.
    let chips = |app: &mut App, buf: &mut Buffer, y: u16, items: &[String], cur: usize, mk: fn(usize) -> HyHit| {
        let mut start = 0;
        let room = (r.right() - 2).saturating_sub(r.x + 14) as usize;
        while start < cur && items[start..=cur].iter().map(|s| s.width() + 3).sum::<usize>() > room {
            start += 1;
        }
        let mut x = r.x + 14;
        if start > 0 {
            put(buf, x - 2, y, &[seg("‹", Style::default().fg(t.muted).bg(t.card))], r.right());
        }
        for (i, it) in items.iter().enumerate().skip(start) {
            let txt = format!(" {it} ");
            if x + txt.width() as u16 > r.right() - 2 {
                put(buf, r.right() - 2, y, &[seg("›", Style::default().fg(t.muted).bg(t.card))], r.right());
                break;
            }
            let on = i == cur;
            let cr = Rect { x, y, width: txt.width() as u16, height: 1 };
            let st = if on {
                Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
            } else if hovered(app, cr) {
                Style::default().bg(t.hov).fg(t.strong)
            } else {
                Style::default().bg(t.btn).fg(t.text)
            };
            put(buf, x, y, &[seg(txt.clone(), st)], r.right());
            hit(app, cr, mk(i));
            x += txt.width() as u16 + 1;
        }
    };
    let agents = np_agents(app);
    let agent = agents.get(np.a).cloned().unwrap_or_default();
    let muted = Style::default().fg(t.muted).bg(t.card);
    // The task: typed straight away, it's the agent's first prompt.
    lab(buf, r.y + 2, "TASK", 0);
    let tb = Rect { x: r.x + 14, y: r.y + 2, width: r.right().saturating_sub(r.x + 17), height: 1 };
    fill(buf, tb, t.card2);
    let s2 = Style::default().bg(t.card2);
    let shown = {
        let w = tb.width.saturating_sub(3) as usize;
        let n = np.task.chars().count();
        if n > w { np.task.chars().skip(n - w).collect::<String>() } else { np.task.clone() }
    };
    let mut ts = vec![seg(format!(" {shown}"), s2.fg(t.strong))];
    if np.row == 0 {
        ts.push(seg("█", s2.fg(t.accent)));
    }
    if np.task.is_empty() {
        let ph = match preset_of(app, &agent) {
            Some(p) if !p.asks() => format!(" (the preset says: {})", truncate(&p.prompt, 50)),
            Some(_) => " what should it do?".to_string(),
            None if agent == "shell" || agent.starts_with('⚙') => " (not used for this)".to_string(),
            None => " what should it do? (optional)".to_string(),
        };
        ts.push(seg(ph, s2.fg(t.muted)));
    }
    put(buf, tb.x, tb.y, &ts, tb.right());
    hit(app, tb, HyHit::NpTask);
    lab(buf, r.y + 4, "RUN", 1);
    chips(app, buf, r.y + 4, &agents, np.a, HyHit::NpRun);
    lab(buf, r.y + 6, "MODEL", 2);
    let models = if agent.starts_with('★') { Vec::new() } else { np_models(app, &agent) };
    if let Some(p) = preset_of(app, &agent) {
        let m = if p.model.is_empty() { "its default".to_string() } else { p.model.clone() };
        put(buf, r.x + 14, r.y + 6, &[seg(format!("{m} (from the preset)"), muted)], r.right());
    } else if models.is_empty() {
        put(buf, r.x + 14, r.y + 6, &[seg("its default", muted)], r.right());
    } else {
        chips(app, buf, r.y + 6, &models, np.model.min(models.len() - 1), HyHit::NpModel);
    }
    let mut projs: Vec<String> = model.iter().map(|p| p.name.clone()).collect();
    projs.push("other folder…".into());
    lab(buf, r.y + 8, "PROJECT", 3);
    chips(app, buf, r.y + 8, &projs, np.p, HyHit::NpProj);
    let place = np.place_for(&agent, app.cfg.worktree.per_agent);
    let proj = model.get(np.p);
    lab(buf, r.y + 10, "WHERE", 4);
    if proj.is_some_and(|p| p.git) && !agent.starts_with('⚙') {
        chips(app, buf, r.y + 10, &np_places(proj.unwrap()), place as usize, HyHit::NpWhere);
    } else {
        put(buf, r.x + 14, r.y + 10, &[seg(if agent.starts_with('⚙') { "as the recipe says" } else { "in the folder" }, muted)], r.right());
    }
    lab(buf, r.y + 12, "OPEN", 5);
    chips(app, buf, r.y + 12, &["full screen".into(), "beside this".into()], np.beside as usize, HyHit::NpBeside);

    // What will happen, in words.
    let branch = proj.and_then(|p| p.wts.iter().find(|w| w.main)).map(|w| w.branch.clone()).unwrap_or_default();
    let others = proj.and_then(|p| p.wts.iter().find(|w| w.main)).map(|w| w.sessions.len()).unwrap_or(0);
    let what = match proj {
        None => "Pick a folder; it becomes a project.".to_string(),
        Some(p) if p.git && !agent.starts_with('⚙') && place >= 3 => {
            let w = p.wts.iter().filter(|w| !w.main).nth(place as usize - 3);
            format!("{agent} runs in the {} worktree, on {}.", w.map(|w| w.name.as_str()).unwrap_or("?"), w.map(|w| w.branch.as_str()).unwrap_or("?"))
        }
        Some(p) if p.git && !agent.starts_with('⚙') && place == 0 => format!("{agent} gets its own new worktree in {}, on a new branch named for you.", p.name),
        Some(p) if p.git && !agent.starts_with('⚙') && place == 1 => format!(
            "Switches {} to a new branch, then starts {agent} there.{}",
            p.name,
            if others > 0 { format!(" The {others} already running in it will be on that branch too.") } else { String::new() }
        ),
        Some(p) if p.git && !agent.starts_with('⚙') => format!("{agent} runs in {} on {branch}.", p.name),
        Some(p) if agent == "shell" => format!("A shell in {}.", p.name),
        Some(p) if agent.starts_with('⚙') => {
            let r = app.cfg.recipes.iter().find(|r| Some(r.name.as_str()) == agent.strip_prefix("⚙ "));
            match r {
                Some(r) => format!("{}{}", if r.worktree && p.git { "A new worktree running " } else { "Runs " }, r.run.join(" + ")),
                None => String::new(),
            }
        }
        Some(p) if !p.git => format!("{} isn't a git repo, so {agent} runs in the folder.", p.name),
        Some(p) if app.cfg.worktree.per_agent => format!("{agent} gets its own new worktree in {}, named for you.", p.name),
        Some(p) => format!("{agent} runs in {}.", p.name),
    };
    let c = Style::default().bg(t.card);
    let what = match np_command(app, &agent, np.model, &np.task) {
        Some(cmd) if !np.task.trim().is_empty() || np.model > 0 => format!("{what}  Runs: {cmd}"),
        _ => what,
    };
    for (i, l) in super::views::wrap(&what, (r.width - 6) as usize).into_iter().take(3).enumerate() {
        put(buf, r.x + 3, r.y + 14 + i as u16, &[seg(l, c.fg(t.text))], r.right() - 2);
    }
    let gx = btn(app, buf, r.x + 3, r.y + 18, if np.task.trim().is_empty() { "Open" } else { "Start" }, "Enter", BtnKind::Primary, HyHit::NpGo, r.right());
    put(buf, gx + 3, r.y + 18, &hints(t, &[("↑↓", "rows"), ("←→", "choose"), ("Esc", "close, keeps the task")]), r.right());
}

// Talk ----------------------------------------------------------------------------------------

pub(super) fn draw_talk(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, term: TermId, input: &str) {
    let model = app.hy_model();
    let found = find(&model, term).map(|(_, _, s)| s.clone());
    let (title, agent) = found.as_ref().map(|s| (s.title.clone(), s.agent.clone())).unwrap_or_default();
    let status = found.as_ref().map(|s| s.status).unwrap_or(Status::None);
    // What it last said or asks, for context.
    let context = found.as_ref().and_then(|s| s.question.clone()).or_else(|| {
        app.parsers.get(&term).and_then(|p| {
            let sc = p.screen();
            let (_, cols) = sc.size();
            sc.rows(0, cols)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.chars().all(|c| "─━-_ ".contains(c)) && !l.starts_with('❯') && !l.starts_with('>'))
                .last()
        })
    });
    let lines: Vec<&str> = if input.is_empty() { vec![""] } else { input.split('\n').collect() };
    let shown = lines.len().min(5) as u16;
    let buf = f.buffer_mut();
    // Beside its row in the sidebar (the view behind stays as it is), else centered.
    let side = app.hy.side_rect;
    let r = match app.hy.talk_anchor {
        Some(y) if side.width > 0 && area.right() > side.right() + 40 => {
            let w = (area.right() - side.right() - 3).min(84);
            let h = 5 + shown;
            let y = y.min(area.bottom().saturating_sub(h + 1)).max(area.y + 1);
            let r = Rect { x: side.right() + 1, y, width: w, height: h };
            hit(app, area, HyHit::Close);
            fill(buf, r, t.card);
            let edge = Style::default().fg(t.accent).bg(t.card);
            for yy in r.top()..r.bottom() {
                buf[(r.x, yy)].set_symbol("▌").set_style(edge);
            }
            hit(app, r, HyHit::Noop);
            r
        }
        _ => {
            dim_all(buf, area, t);
            let r = panel(app, buf, area, 96, 6 + shown, "", &[], t);
            fill(buf, Rect { height: 1, ..r }, t.card);
            r
        }
    };
    let c = Style::default().bg(t.card);
    let mut head = vec![
        seg(format!("{} ", glyph(app, status)), c.fg(t.status(status)).add_modifier(Modifier::BOLD)),
        seg(agent.clone(), c.fg(t.strong).add_modifier(Modifier::BOLD)),
    ];
    if title != WAITING && !title.is_empty() {
        head.push(seg(format!("  {title}"), c.fg(t.muted)));
    }
    put(buf, r.x + 2, r.y, &head, r.right() - 1);
    if let Some(cx) = context {
        let col = if status == Status::Blocked { t.blocked } else { t.text };
        put(buf, r.x + 2, r.y + 1, &[seg(truncate(&cx, (r.width - 4) as usize), c.fg(col))], r.right() - 1);
    }
    let box_ = Rect { x: r.x + 1, y: r.y + 2, width: r.width - 2, height: shown };
    fill(buf, box_, t.card2);
    let s = Style::default().bg(t.card2);
    let first = lines.len().saturating_sub(5);
    for (i, l) in lines[first..].iter().enumerate() {
        let last = i + first + 1 == lines.len();
        let mut segs = vec![seg(if i + first == 0 { "› " } else { "  " }, s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(l.to_string(), s.fg(t.strong))];
        if last {
            segs.push(seg("█", s.fg(t.accent)));
            if input.is_empty() {
                segs.push(seg(format!(" message {agent}…"), s.fg(t.muted)));
            }
        }
        put(buf, r.x + 2, box_.y + i as u16, &segs, r.right() - 2);
    }
    put(
        buf,
        r.x + 2,
        r.bottom() - 2,
        &hints(t, &[("Enter", "send"), ("Shift+Enter", "new line"), ("Esc", "close")]),
        r.right() - 1,
    );
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
    let sel = app.hy.splash_sel.min(3);
    for (i, (l, key, _, c)) in btns.into_iter().enumerate() {
        // The selected button is the bright one; arrows, Tab and the mouse move it.
        let kind = if i == sel { BtnKind::Primary } else { BtnKind::Normal };
        let b = button(t, l, key, kind, false);
        let br = Rect { x, y, width: segs_width(&b), height: 1 };
        let b = button(t, l, key, kind, hovered(app, br));
        put(f.buffer_mut(), x, y, &b, area.right());
        if i == sel {
            put(f.buffer_mut(), x, y + 1, &[seg("▔".repeat(br.width as usize), Style::default().fg(t.accent).bg(t.bg))], area.right());
        }
        hit(app, br, HyHit::SplashKey(c));
        x += br.width + 3;
    }
    put(
        f.buffer_mut(),
        center(44),
        y + 3,
        &[seg("←→ choose   Enter open   or press a button's key", Style::default().fg(t.muted).bg(t.bg))],
        area.right(),
    );
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
    pub(super) fn hy_agent(&self) -> String {
        self.cfg.quick.agents.first().map(|a| a.name.clone()).unwrap_or_else(|| "claude".into())
    }

    /// Message an agent: a small box beside its sidebar row when it has one, else centered.
    /// From the sidebar cursor, sending returns there so the next one is a key away.
    pub(super) fn hy_talk(&mut self, term: TermId, from_side: bool) {
        self.hy.talk_anchor = self.hy.row_y.get(&term).copied();
        self.hy.talk_back = from_side;
        self.mode = Mode::Talk { term, input: String::new() };
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

    /// A new worktree of a project running `cmd`: on `branch` if given (an existing one),
    /// else on a new branch with a generated name.
    fn hy_new_worktree(&mut self, proj: &Proj, cmd: Option<String>, beside: bool, branch: Option<String>) {
        let taken: HashSet<String> = proj.wts.iter().flat_map(|w| [w.name.clone(), w.branch.clone()]).chain(proj.branches.iter().cloned()).collect();
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as usize).unwrap_or(0);
        let branch = branch.unwrap_or_else(|| {
            (0..WT_NAMES.len())
                .map(|i| WT_NAMES[(n + i) % WT_NAMES.len()].to_string())
                .find(|b| !taken.contains(b))
                .unwrap_or_else(|| format!("{}-{}", WT_NAMES[n % WT_NAMES.len()], n % 1000))
        });
        let Some(ws) = self.active_ws().map(|w| w.id).or_else(|| self.snap.workspaces.first().map(|w| w.id)) else {
            self.notify("start something first".into(), true);
            return;
        };
        if beside && let Some(f) = self.focused() {
            self.hy.pending_split = Some((f, Instant::now()));
        }
        self.mode = Mode::Normal;
        self.notify(format!("new worktree {branch} in {}", proj.name), false);
        self.cmd(Command::NewWorktree { ws, branch, base: None, cmd, split: None, from: Some(proj.path.clone()) });
    }

    /// A new worktree running `cmd` (on `branch` if given), full screen.
    pub(super) fn hy_start_worktree(&mut self, proj: &Proj, cmd: Option<String>, branch: Option<String>) {
        self.hy_new_worktree(proj, cmd, false, branch);
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

    /// The + New chooser, for the current project (or the sidebar cursor's).
    fn hy_open_new_pane(&mut self, beside: bool) {
        let model = self.hy_model();
        let key = self.hy.cursor.and_then(|c| model.iter().find(|p| p.sessions().any(|s| s.term == c)).map(|p| p.key.clone())).or(self.hy.proj.clone());
        let p = model.iter().position(|p| Some(&p.key) == key.as_ref()).unwrap_or(0);
        self.hy_new(p, beside);
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
        self.hy.follow = true;
        self.mode = Mode::Side;
        // Resting on a finished agent counts as seeing it.
        let t = vis[next];
        if self.snap.terms.get(&t).is_some_and(|i| i.status == Status::Done) {
            self.cmd(Command::MarkSeen { term: t });
        }
    }

    /// Actions that work differently in this layout. Returns true if handled.
    pub(super) fn hy_act(&mut self, a: &Action) -> bool {
        let focused = self.focused();
        match a {
            Action::Settings => self.hy_settings(),
            Action::NewPane => self.hy_open_new_pane(false),
            Action::Jump | Action::Picker => self.mode = Mode::Jump { sel: 0 },
            Action::OpenProject => self.hy_open_finder(),
            Action::Talk | Action::Reply => {
                let from_side = *a == Action::Talk && self.hy.cursor.is_some();
                let term = if *a == Action::Talk { self.hy.cursor.or(focused) } else { focused };
                match term {
                    Some(term) => self.hy_talk(term, from_side),
                    None => self.notify("nothing to message".into(), true),
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
            Action::NewSession => self.hy_open_new_pane(true),
            Action::CloseSplit => {
                if self.hy.pair.take().is_none() {
                    self.notify("no split open".into(), false);
                }
            }
            Action::SideMove(d) => self.hy_side_move(*d),
            Action::Resize(d) => {
                use crate::layout::Dir;
                let grow = matches!(d, Dir::Right | Dir::Down);
                if self.hy.pair.is_some() {
                    let r = self.hy.saved.split.unwrap_or(0.5) + if grow { 0.05 } else { -0.05 };
                    self.hy.saved.split = Some(r.clamp(0.2, 0.8));
                } else {
                    let w = self.hy.saved.side_w.unwrap_or(self.hy.side_rect.width.max(SIDE_MIN));
                    let w = if grow { w + 2 } else { w.saturating_sub(2) };
                    self.hy.saved.side_w = Some(w.clamp(SIDE_MIN, SIDE_MAX));
                }
                self.hy.save();
            }
            Action::Ideas => self.open_ideas(),
            Action::Map => self.open_map(),
            Action::Inbox => self.open_tickets(),
            Action::Race => self.open_race_new(),
            Action::Ship => {
                let dir = self.hy_target_dir();
                self.hy.cursor = None;
                self.ask_ship(dir);
            }
            Action::Files | Action::Changes | Action::PullRequest => {
                let dir = self.hy_target_dir();
                self.hy.cursor = None;
                self.mode = Mode::Normal;
                match a {
                    Action::Files => self.open_files(dir),
                    Action::Changes => self.open_changes(dir),
                    _ => match crate::gitfs::head(&dir) {
                        Some(h) => self.open_pr(h.top, h.branch),
                        None => self.notify("not a git repo".into(), true),
                    },
                }
            }
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
            KeyCode::Char(' ') => {
                if let Some(c) = self.hy.cursor {
                    self.hy_talk(c, true);
                }
            }
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
        let model = self.hy_model();
        let list = jump_list(&model);
        let prs = jump_prs(&model);
        let total = list.len() + prs.len();
        let go = |app: &mut App, i: usize| {
            if let Some((s, ..)) = list.get(i) {
                app.hy_focus(s.term);
            } else if let Some((dir, pr, ..)) = prs.get(i - list.len().min(i)) {
                app.mode = Mode::Normal;
                app.open_pr(dir.clone(), pr.number.to_string());
            }
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('j') => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::Jump { sel: (sel + 1).min(total.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::Jump { sel: sel.saturating_sub(1) },
            KeyCode::Enter if sel < total => go(self, sel),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' && (c as usize - '1' as usize) < total => go(self, c as usize - '1' as usize),
            _ => {}
        }
    }

    /// The folder Files / Changes / PR act on: the sidebar cursor's, else the focused one's.
    fn hy_target_dir(&self) -> PathBuf {
        self.hy
            .cursor
            .or(self.focused())
            .and_then(|t| self.snap.terms.get(&t))
            .map(|t| t.top.clone().unwrap_or_else(|| t.cwd.clone()))
            .unwrap_or_else(|| self.here_dir())
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

    /// Start what the chooser says: an agent gets its own worktree in a git project, a
    /// shell (or an agent outside git) runs in the project's folder.
    fn hy_pane_go(&mut self, np: &NewPaneHy) {
        let model = self.hy_model();
        let Some(p) = model.get(np.p).cloned() else {
            self.hy_open_finder();
            return;
        };
        let agents = np_agents(self);
        let agent = agents.get(np.a).cloned().unwrap_or_else(|| "shell".into());
        let main = p.wts.iter().find(|w| w.main).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
        let cmd = np_command(self, &agent, np.model, &np.task);
        if !self.hy.saved.draft.is_empty() {
            self.hy.saved.draft.clear();
            self.hy.save();
        }
        if let Some(recipe) = agent.strip_prefix("⚙ ") {
            self.mode = Mode::Normal;
            self.run_recipe(&p, recipe);
            return;
        }
        if !p.git {
            return self.hy_new_session(main, cmd, np.beside);
        }
        match np.place_for(&agent, self.cfg.worktree.per_agent) {
            0 => self.hy_new_worktree(&p, cmd, np.beside, None),
            1 => {
                // A new branch right in the repo folder (no worktree).
                let taken: HashSet<String> = p.wts.iter().map(|w| w.branch.clone()).collect();
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as usize).unwrap_or(0);
                let name = (0..WT_NAMES.len()).map(|i| WT_NAMES[(n + i) % WT_NAMES.len()].to_string()).find(|b| !taken.contains(b)).unwrap_or_else(|| format!("branch-{}", n % 10_000));
                let out = std::process::Command::new("git").arg("-C").arg(&main).args(["switch", "-c", &name]).output();
                match out {
                    Ok(o) if o.status.success() => {
                        self.notify(format!("{} is on a new branch, {name}", p.name), false);
                        self.hy_new_session(main, cmd, np.beside);
                    }
                    Ok(o) => self.notify(format!("couldn't make a branch: {}", String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("git failed")), true),
                    Err(e) => self.notify(format!("couldn't run git: {e}"), true),
                }
            }
            n @ 3.. => match p.wts.iter().filter(|w| !w.main).nth(n as usize - 3) {
                Some(w) => self.hy_new_session(w.path.clone(), cmd, np.beside),
                None => self.hy_new_session(main, cmd, np.beside),
            },
            _ => self.hy_new_session(main, cmd, np.beside),
        }
    }

    pub(super) fn on_history_key(&mut self, sel: usize, k: &KeyEvent) {
        let n = self.history.len();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::History { sel: (sel + 1).min(n.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::History { sel: sel.saturating_sub(1) },
            KeyCode::Char('c') => {
                self.history.clear();
                self.mode = Mode::History { sel: 0 };
            }
            KeyCode::Enter => {
                let term = self.history.iter().rev().nth(sel).and_then(|h| h.1);
                match term.filter(|t| self.snap.terms.contains_key(t)) {
                    Some(t) => {
                        self.mode = Mode::Normal;
                        self.hy_focus(t);
                    }
                    None => self.mode = Mode::History { sel },
                }
            }
            _ => self.mode = Mode::History { sel },
        }
    }

    pub(super) fn on_memory_key(&mut self, sel: usize, k: &KeyEvent) {
        let rows = memory_rows(self);
        let n = rows.len();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::Memory { sel: (sel + 1).min(n.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::Memory { sel: sel.saturating_sub(1) },
            KeyCode::Enter => {
                if let Some(r) = rows.get(sel) {
                    self.mode = Mode::Normal;
                    self.hy_focus(r.0);
                }
            }
            KeyCode::Char('x') => {
                if let Some(r) = rows.get(sel) {
                    self.cmd(Command::ClosePane { term: r.0 });
                    self.notify(format!("ended {} ({} freed)", r.1, mb(r.3)), false);
                }
                self.mode = Mode::Memory { sel: sel.min(n.saturating_sub(2)) };
            }
            _ => self.mode = Mode::Memory { sel },
        }
    }

    /// Ctrl+Space . : your presets, numbered, for the agent you're on.
    pub(super) fn hy_presets(&mut self) {
        if self.cfg.presets.is_empty() {
            self.notify("no presets yet: add [[presets]] to your config (see the example config)".into(), true);
            return;
        }
        let on = self.hy.cursor.or(self.focused());
        let items = self
            .cfg
            .presets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let how = match p.place.as_str() {
                    "send" => "tell it",
                    "worktree" => "new worktree",
                    _ => "beside",
                };
                (format!("{}  {}{}  · {how}", i + 1, p.name, if p.asks() { "…" } else { "" }), super::menu::Act::Preset(i, on))
            })
            .collect();
        self.menu("Presets".into(), items, (self.hy.side_rect.right() + 4, 3));
    }

    /// Run preset `i` for the agent `on` (its folder, or the agent itself).
    pub(super) fn hy_run_preset(&mut self, i: usize, on: Option<TermId>, task: Option<String>) {
        let Some(p) = self.cfg.presets.get(i).cloned() else { return };
        let model = self.hy_model();
        let at = on.and_then(|t| self.snap.terms.get(&t)).map(|t| (t.top.clone().unwrap_or_else(|| t.cwd.clone()), t.root.clone()));
        let pi = at
            .as_ref()
            .and_then(|(top, root)| model.iter().position(|m| path_key(&m.path) == path_key(root.as_ref().unwrap_or(top))))
            .unwrap_or(0);
        if p.asks() && task.is_none() {
            // Ask for the task in + New, with the preset picked.
            let mut np = NewPaneHy::new(pi, p.place == "beside");
            np.a = np_agents(self).iter().position(|a| *a == format!("★ {}", p.name)).unwrap_or(0);
            np.place = Some(if p.place == "worktree" { 0 } else { 2 });
            self.mode = Mode::HyPane(np);
            return;
        }
        let task = task.unwrap_or_default();
        self.mode = Mode::Normal;
        match (p.place.as_str(), on, at) {
            ("send", Some(term), _) => self.send_message(term, &p.fill(&task)),
            ("worktree", _, _) | (_, None, _) | (_, _, None) => {
                let cmd = np_command(self, &format!("★ {}", p.name), 0, &task);
                match model.get(pi) {
                    Some(proj) if proj.git => self.hy_new_worktree(proj, cmd, false, None),
                    Some(proj) => self.hy_new_session(proj.path.clone(), cmd, false),
                    None => self.notify("open a project first".into(), true),
                }
            }
            (_, Some(_), Some((top, _))) => {
                let cmd = np_command(self, &format!("★ {}", p.name), 0, &task);
                self.hy_new_session(top, cmd, true);
            }
        }
    }

    /// + New for project `p`, with the task you didn't start last time.
    pub(super) fn hy_new(&mut self, p: usize, beside: bool) {
        let mut np = NewPaneHy::new(p, beside);
        np.task = self.hy.saved.draft.clone();
        self.mode = Mode::HyPane(np);
    }

    /// + New filled in like an agent that's running: same project, agent, model and task.
    pub(super) fn hy_duplicate(&mut self, term: TermId) {
        let model = self.hy_model();
        let Some(pi) = model.iter().position(|p| p.wts.iter().any(|w| w.sessions.iter().any(|s| s.term == term))) else { return };
        let Some((_, _, s)) = find(&model, term) else { return };
        let mut np = NewPaneHy::new(pi, false);
        np.a = np_agents(self).iter().position(|a| *a == s.agent).unwrap_or(0);
        if let Some(t) = self.snap.terms.get(&term) {
            np.task = t.summary.clone();
        }
        np.place = Some(0);
        self.mode = Mode::HyPane(np);
    }

    pub(super) fn on_hy_pane_key(&mut self, mut np: NewPaneHy, k: &KeyEvent) {
        let model = self.hy_model();
        let nproj = model.len() + 1;
        let agents = np_agents(self);
        let nag = agents.len();
        let agent = agents.get(np.a).cloned().unwrap_or_default();
        let nmodels = if agent.starts_with('★') { 0 } else { np_models(self, &agent).len() };
        let nplaces = model.get(np.p).map(|p| np_places(p).len()).unwrap_or(3);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let d: i32 = match k.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => 0,
        };
        let cyc = |v: usize, d: i32, n: usize| (v as i32 + d).rem_euclid(n.max(1) as i32) as usize;
        // Rows with nothing to choose are skipped.
        let skip = |r: u8| r == 2 && nmodels == 0;
        let step = |np: &mut NewPaneHy, by: u8| {
            np.row = (np.row + by) % NP_ROWS;
            if skip(np.row) {
                np.row = (np.row + by) % NP_ROWS;
            }
        };
        match k.code {
            KeyCode::Esc => {
                // The task stays for next time.
                if self.hy.saved.draft != np.task {
                    self.hy.saved.draft = np.task.clone();
                    self.hy.save();
                }
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                np.beside |= k.modifiers.contains(KeyModifiers::SHIFT);
                return self.hy_pane_go(&np);
            }
            KeyCode::Tab | KeyCode::Down => step(&mut np, 1),
            KeyCode::BackTab | KeyCode::Up => step(&mut np, NP_ROWS - 1),
            // Typing on the task row.
            KeyCode::Backspace if np.row == 0 => {
                if ctrl {
                    let keep = np.task.trim_end().rfind(' ').map(|i| i + 1).unwrap_or(0);
                    np.task.truncate(keep);
                } else {
                    np.task.pop();
                }
            }
            KeyCode::Char('u') if ctrl && np.row == 0 => np.task.clear(),
            KeyCode::Char(c) if np.row == 0 && !ctrl => np.task.push(c),
            KeyCode::Char('b') if !ctrl => np.beside = !np.beside,
            _ if d != 0 => match np.row {
                0 => {}
                1 => {
                    np.a = cyc(np.a, d, nag);
                    np.model = 0;
                }
                2 => np.model = cyc(np.model, d, nmodels),
                3 => {
                    np.p = cyc(np.p, d, nproj);
                    np.place = np.place.filter(|p| *p < 3);
                }
                4 => {
                    let cur = np.place_for(&agent, self.cfg.worktree.per_agent) as usize;
                    np.place = Some(cyc(cur, d, nplaces) as u8);
                }
                _ => np.beside = !np.beside,
            },
            _ => {}
        }
        self.mode = Mode::HyPane(np);
    }

    /// Keys on the splash: move between its buttons, Enter picks one, or a button's own key.
    /// Nothing else leaves it.
    pub(super) fn on_hy_splash_key(&mut self, k: &KeyEvent) {
        const KEYS: [char; 4] = ['\n', 'j', 'o', ','];
        match k.code {
            KeyCode::Left | KeyCode::Up | KeyCode::BackTab => self.hy.splash_sel = (self.hy.splash_sel + 3) % 4,
            KeyCode::Right | KeyCode::Down | KeyCode::Tab => self.hy.splash_sel = (self.hy.splash_sel + 1) % 4,
            KeyCode::Enter => {
                let c = KEYS[self.hy.splash_sel.min(3)];
                self.hy_splash_action(c);
            }
            KeyCode::Char(c @ ('j' | 'o' | ',' | '?')) => self.hy_splash_action(c),
            _ => {}
        }
    }

    fn hy_splash_action(&mut self, c: char) {
        self.splash = false;
        self.hy.splash_sel = 0;
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
            HyHit::ToggleProj(i) => {
                if let Some(key) = self.hy.proj_keys.get(i).map(|k| format!("p:{k}")) {
                    if let Some(pos) = self.hy.saved.closed.iter().position(|k| *k == key) {
                        self.hy.saved.closed.remove(pos);
                    } else {
                        self.hy.saved.closed.push(key);
                    }
                    self.hy.save();
                }
            }
            HyHit::NewIn(i) => self.hy_new(i, false),
            HyHit::Wt(i) => {
                let Some((key, path)) = self.hy.wt_keys.get(i).cloned() else { return };
                let model = self.hy_model();
                let wt = model.iter().flat_map(|p| p.wts.iter()).find(|w| w.key == key).cloned();
                match wt {
                    Some(w) if !w.sessions.is_empty() => {
                        let best = w.sessions.iter().min_by_key(|s| (rank(s.status), s.term)).map(|s| s.term);
                        if let Some(t) = best {
                            self.hy_focus(t);
                        }
                    }
                    // Nothing running: a shell in the main folder, your agent in a worktree.
                    Some(w) if w.main => self.hy_new_session(path, None, false),
                    _ => {
                        let agent = self.hy_agent();
                        self.hy_new_session(path, Some(agent), false);
                    }
                }
            }
            HyHit::OpenFolder => self.hy_open_finder(),
            HyHit::Session(t) => self.hy_focus(t),
            HyHit::Talk(t) => self.hy_talk(t, false),
            HyHit::Settings => self.hy_settings(),
            HyHit::Jump => self.mode = Mode::Jump { sel: 0 },
            HyHit::NewPane => self.hy_open_new_pane(false),
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
            HyHit::NpTask => {
                if let Mode::HyPane(np) = &mut self.mode {
                    np.row = 0;
                }
            }
            HyHit::NpProj(i) | HyHit::NpRun(i) | HyHit::NpBeside(i) | HyHit::NpWhere(i) | HyHit::NpModel(i) => {
                if let Mode::HyPane(np) = &mut self.mode {
                    match h {
                        HyHit::NpWhere(_) => {
                            np.place = Some(i as u8);
                            np.row = 4;
                        }
                        HyHit::NpRun(_) => {
                            if np.a != i {
                                np.model = 0;
                            }
                            np.a = i;
                            np.row = 1;
                        }
                        HyHit::NpModel(_) => {
                            np.model = i;
                            np.row = 2;
                        }
                        HyHit::NpProj(_) => {
                            np.p = i;
                            np.place = np.place.filter(|p| *p < 3);
                            np.row = 3;
                        }
                        _ => {
                            np.beside = i == 1;
                            np.row = 5;
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
            HyHit::IdeaRow(i) => {
                if let Mode::Ideas(v) = &mut self.mode {
                    let again = v.sel == i && v.input.is_empty();
                    v.sel = i;
                    v.input.clear();
                    if again || double {
                        let v = (**v).clone();
                        self.on_ideas_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::TicketTab(i) => {
                if let Mode::Tickets(v) = &self.mode {
                    let mut v = (**v).clone();
                    let n = v.tabs.len();
                    v.tab = (i + n - 1) % n;
                    self.on_tickets_key(v, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                }
            }
            HyHit::TicketRow(i) => {
                if let Mode::Tickets(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        let v = (**v).clone();
                        self.on_tickets_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::RaceAgent(i) => {
                if let Mode::RaceNew(v) = &mut self.mode {
                    v.row = 1;
                    v.cur = i;
                    if let Some(p) = v.picked.get_mut(i) {
                        *p = !*p;
                    }
                }
            }
            HyHit::RaceGo => {
                if let Mode::RaceNew(v) = &self.mode {
                    let v = (**v).clone();
                    self.on_race_new_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            HyHit::RaceRow(i) => {
                if let Mode::Race(v) = &mut self.mode {
                    v.sel = i;
                    v.confirm = false;
                }
            }
            HyHit::RaceKey(c) => {
                if let Mode::Race(v) = &self.mode {
                    let v = (**v).clone();
                    let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
                    self.on_race_key(v, &KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            HyHit::RaceOpen(id) => self.open_race(id),
            HyHit::MenuPick(i) => self.menu_pick(i),
            HyHit::HistRow(i) => {
                if let Mode::History { sel } = &mut self.mode {
                    let again = *sel == i;
                    *sel = i;
                    if again || double {
                        self.on_history_key(i, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::MemRow(i) => {
                if let Mode::Memory { sel } = &mut self.mode {
                    let again = *sel == i;
                    *sel = i;
                    if again || double {
                        self.on_memory_key(i, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::BranchRow(i) => {
                if let Mode::Branch(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        let v = (**v).clone();
                        self.on_branch_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::BranchChoice(i) => {
                if let Mode::Branch(v) = &self.mode {
                    let v = (**v).clone();
                    self.branch_choose(v, i);
                }
            }
            HyHit::FindTab(i) => {
                if let Mode::Find(v) = &self.mode
                    && v.tab != i
                {
                    let v = (**v).clone();
                    self.on_find_key(v, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                }
            }
            HyHit::FindRow(i) => {
                if let Mode::Find(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    v.refresh_preview();
                    if again || double {
                        let v = (**v).clone();
                        self.on_find_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::SideEdge => self.hy.drag = Some(Drag::Side),
            HyHit::SplitEdge => self.hy.drag = Some(Drag::Split),
            HyHit::ScrollBar(term) => {
                self.hy.drag = Some(Drag::Scroll(term));
                if let (Some((t, r, total)), Some(pos)) = (self.hy.bar, self.hover)
                    && t == term
                {
                    let from_bottom = r.bottom().saturating_sub(pos.y + 1) as usize;
                    self.scroll_to(term, (from_bottom * total / r.height.max(1) as usize).min(total));
                }
            }
            HyHit::MapNode(i) => {
                if let Some(super::View::Map(v)) = &mut self.view {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        self.on_view_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::ShipGo => {
                if let Mode::Ship(ask) = std::mem::replace(&mut self.mode, Mode::Normal) {
                    let task = ask.task.clone();
                    self.notify(format!("shipping {}…", task.branch), false);
                    self.spawn_bg(move || super::Bg::Done(super::tasks::ship(&task), false));
                }
            }
            HyHit::Pr(i) => {
                if let Some((dir, n)) = self.hy.pr_keys.get(i).cloned() {
                    self.mode = Mode::Normal;
                    self.open_pr(dir, n);
                }
            }
            HyHit::ViewKey(c) => {
                let code = match c {
                    '\x1b' => KeyCode::Esc,
                    '\n' => KeyCode::Enter,
                    c => KeyCode::Char(c),
                };
                self.on_view_key(&KeyEvent::new(code, KeyModifiers::NONE));
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

// ---- pull request view ---------------------------------------------------------------------

fn check_glyph(t: &Theme, c: super::pr::Checks) -> Seg {
    use super::pr::Checks;
    match c {
        Checks::Pass => seg("✓", Style::default().fg(t.done)),
        Checks::Fail => seg("✕", Style::default().fg(t.err).add_modifier(Modifier::BOLD)),
        Checks::Pending => seg("…", Style::default().fg(t.muted)),
        Checks::None => seg("·", Style::default().fg(t.muted)),
    }
}

/// The little `#412 ✓` tag for a branch with a pull request.
fn pr_tag(t: &Theme, pr: &super::pr::PrBrief) -> Vec<Seg> {
    use super::pr::Review;
    let mut v = vec![seg(format!(" #{} ", pr.number), Style::default().fg(t.muted)), check_glyph(t, pr.checks)];
    if pr.review == Review::Changes {
        v.push(seg("±", Style::default().fg(t.blocked).add_modifier(Modifier::BOLD)));
    } else if pr.review == Review::Approved {
        v.push(seg("✔", Style::default().fg(t.done)));
    }
    v
}

/// Wrap plain text to `width` columns.
fn wrap_text(s: &str, width: usize) -> Vec<String> {
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

pub(super) fn draw_pr(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &super::pr::PrView) {
    fill(buf, area, t.bg);
    let title = match &v.info {
        Some(Ok(i)) => format!("#{}  {}", i.number, i.title),
        _ => format!("pull request · {}", v.which),
    };
    super::design::title_bar(
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
            let bad = info.checks.iter().filter(|(_, c)| *c == super::pr::Checks::Fail).count();
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
                    lines.push(super::design::diff_line(t, l, &mut old, &mut new));
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

pub(super) fn draw_ship(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, ask: &super::ShipAsk) {
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

const BW: u16 = 28;
const BH: u16 = 6;

fn junction(up: bool, down: bool, left: bool, right: bool) -> &'static str {
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

fn map_project<'a>(app: &App, model: &'a [Proj], v: &MapView) -> Option<&'a Proj> {
    let key = v.proj.clone().or_else(|| app.hy.proj.clone());
    model.iter().find(|p| Some(&p.key) == key.as_ref()).or(model.first())
}

/// The most urgent session in a box decides its colour.
fn wt_status(w: &Wt) -> Option<&Session> {
    w.sessions.iter().filter(|s| s.is_agent).min_by_key(|s| (rank(s.status), s.term))
}

pub(super) fn draw_map(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &MapView) {
    let model = app.hy_model();
    fill(buf, area, t.bg);
    let Some(p) = map_project(app, &model, v).cloned() else {
        put(buf, area.x + 2, area.y + 2, &[seg("Open a project first (o).", Style::default().fg(t.muted))], area.right());
        return;
    };
    super::design::title_bar(
        buf,
        area,
        t,
        &[seg("map  ", Style::default().add_modifier(Modifier::BOLD)), seg(p.name.clone(), Style::default())],
        &[seg("Tab next project   ", Style::default()), seg("✕ ", Style::default())],
        true,
    );
    hit(app, Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, HyHit::ViewKey('\x1b'));
    let line = |buf: &mut Buffer, x: u16, y: u16, sym: &str, c: Color| {
        if x < area.right() && y < area.bottom() {
            buf[(x, y)].set_symbol(sym).set_style(Style::default().fg(c).bg(t.bg));
        }
    };
    // The project at the top.
    let rw = (p.name.width() as u16 + 18).clamp(24, area.width.saturating_sub(4));
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

fn draw_box(buf: &mut Buffer, r: Rect, c: Color, t: &Theme) {
    let st = Style::default().fg(c).bg(t.bg);
    for x in r.x..r.right() {
        buf[(x, r.y)].set_symbol("─").set_style(st);
        buf[(x, r.bottom() - 1)].set_symbol("─").set_style(st);
    }
    for y in r.y..r.bottom() {
        buf[(r.x, y)].set_symbol("│").set_style(st);
        buf[(r.right() - 1, y)].set_symbol("│").set_style(st);
    }
    buf[(r.x, r.y)].set_symbol("╭");
    buf[(r.right() - 1, r.y)].set_symbol("╮");
    buf[(r.x, r.bottom() - 1)].set_symbol("╰");
    buf[(r.right() - 1, r.bottom() - 1)].set_symbol("╯");
}

impl App {
    pub(super) fn open_map(&mut self) {
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.view = Some(super::View::Map(Box::new(MapView { proj: self.hy.proj.clone(), sel: 0 })));
    }

    /// Returns false when the map closes.
    pub(super) fn on_map_key(&mut self, v: &mut MapView, k: &KeyEvent) -> bool {
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
