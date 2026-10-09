//! The Seshi layout, from the design handoff: projects (git repos) in the sidebar, each
//! project's worktrees with the sessions running in them, and the focused session full size
//! (optionally split with a second). Everything else is a centred overlay over a dimmed
//! screen: jump, open a folder, new pane, talk, settings and keys.
//!
//! The daemon only knows workspaces, tabs and terminals. Here every terminal is a session;
//! its project and worktree come from where it runs (`TermInfo::root` / `top`), so a shell
//! that `cd`s into another repo moves to that project by itself.

use super::design::{
    BtnKind, SRow, Seg, SettingsView, button, cap_hints, fill, glyph, key_text, keycaps, path_key, put,
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

use super::design::SettingsView as SView;
use crate::keys::KeySpec;
use crate::protocol::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ---- state ---------------------------------------------------------------------------------

/// What the user chose that the daemon doesn't track: the colour order, folded rows.
/// Saved to `seshi-ui.json` in the data folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Saved {
    /// Project keys in the order they were first seen; a project's colour is its index.
    pub order: Vec<String>,
    /// Folded sidebar rows ("p:<project key>").
    pub closed: Vec<String>,
    /// Sessions in the order you dragged them to (within a group; what needs you still
    /// comes first).
    pub session_order: Vec<TermId>,
    /// Sessions you dragged into another group: the session, that group's folder, and
    /// whether it's a git repo. Kept while the session lives.
    pub placed: Vec<(TermId, PathBuf, bool)>,
    /// Projects whose BRANCHES list is unfolded.
    pub open_branches: Vec<String>,
    /// Sidebar width you dragged it to, and the split's share for the left (or top) half.
    pub side_w: Option<u16>,
    pub split: Option<f32>,
    /// A task typed into + New and not started yet.
    pub draft: String,
    /// Projects you renamed: key -> name.
    pub names: HashMap<String, String>,
    /// Files marked reviewed in Changes: folder key -> file -> what it was like then.
    pub reviewed: HashMap<String, HashMap<String, u64>>,
}

#[derive(Debug, Default)]
pub(super) struct Hy {
    /// A question asked from the sidebar: where its cursor goes after (the row below the
    /// one being closed), so the keys stay in the sidebar.
    pub side_return: Option<SideItem>,
    pub saved: Saved,
    /// The project of what you're on (the default for + New).
    pub proj: Option<String>,
    /// Two sessions side by side: (left, right). One of them is the focused one.
    /// Tabs: each shows one session or any number side by side.
    pub tabs: Vec<HyTab>,
    /// Shown alone for now, out of its split (Zoom).
    pub zoom: Option<TermId>,
    /// Panes whose program gets right-clicks (no seshi menu there; Shift+right-click for it).
    pub right_clicks: HashSet<TermId>,
    /// Which way the next "split" goes.
    pub split_dir: Option<crate::layout::Dir>,
    /// Where the bottom bar's "where you are" starts (lined up with the panes).
    pub crumb_x: u16,
    /// Where the file preview was drawn (wheel scrolls it; paging uses its height).
    pub preview_rect: Rect,
    pub tab: usize,
    /// The next session that opens goes in a new tab (Ctrl+Space w), until this time.
    /// A new tab was asked for in this session: the next session to show goes in it.
    pub new_tab: Option<(Instant, TermId)>,
    /// Counts frames, for `HyTab::used`.
    pub tick: u64,
    /// A tab's ✕ clicked once with an agent in it: a second click closes it.
    pub close_armed: Option<(usize, Instant)>,
    /// Where the sessions were drawn last frame, and the splits between them.
    pub leaf_rects: Vec<(TermId, Rect)>,
    pub dividers: Vec<(Rect, bool, Vec<bool>)>,
    /// Sidebar cursor (keyboard browsing; bare keys work while it's set).
    pub cursor: Option<TermId>,
    /// The sidebar cursor on a project row instead (its key).
    pub cursor_proj: Option<String>,
    /// What the sidebar cursor walks over, top to bottom.
    pub side_items: Vec<SideItem>,
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
    /// The sidebar model, built once per event / frame (see `hy_fresh`).
    pub model_cache: std::cell::RefCell<Option<Vec<Proj>>>,
    /// A recipe's worktree being made: (branch, the commands to start there, since).
    pub pending_recipe: Option<(String, Vec<String>, Instant)>,
    /// Dragging the sidebar edge or the split divider.
    pub drag: Option<Drag>,
    /// The group being dragged in the sidebar (its key).
    pub drag_key: Option<String>,
    /// The splash's selected button.
    pub splash_sel: usize,
    /// The scrollbar being dragged: (pane, track, lines of history).
    pub bar: Option<(TermId, Rect, usize)>,
    /// Where each agent row was drawn in the sidebar (for the follow-up box beside it).
    pub row_y: std::collections::HashMap<TermId, u16>,
}

/// A row the sidebar cursor can rest on.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SideItem {
    Proj(String),
    Sess(TermId),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct HyTab {
    pub layout: crate::layout::Node,
    /// The one you were on in it.
    pub focus: TermId,
    /// How its panes are laid out.
    /// The session (sidebar row) it's a tab of. A session's tabs share its row; the tab
    /// bar shows the tabs of the session you're on, once it has more than one.
    pub owner: TermId,
    /// When it was last on screen (higher is later): a session's row brings that tab back.
    pub used: u64,
    /// What you called it; empty: named after what runs in it.
    pub name: String,
}

/// A tab being renamed in its pill.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TabName {
    pub tab: usize,
    pub name: String,
}

/// The second step of a leader key that opens one (`w` worktrees).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Worktrees,
}

/// The key map: every leader key by what it acts on, filtered as you type.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct KeyMap {
    pub query: String,
    /// Typing searches (Tab); otherwise a key runs what it's bound to.
    pub searching: bool,
    pub step: Option<Step>,
    /// The selected match while searching.
    pub sel: usize,
}

/// Where each of three or more panes goes in `area`: rows of up to three, the same size.
/// Panes side by side leave a 3-column gutter (a line in its middle), stacked ones a row.
pub(super) fn arranged(leaves: &[TermId], area: Rect) -> Vec<(TermId, Rect)> {
    let n = leaves.len();
    let mut out = Vec::new();
    let rows = if n <= 3 { 1 } else if n <= 6 { 2 } else { n.div_ceil(3) };
    let per = n.div_ceil(rows).max(1);
    let rh = (area.height + 1) / rows as u16;
    for (ri, chunk) in leaves.chunks(per).enumerate() {
        let y = area.y + ri as u16 * rh;
        let h = if ri + 1 == rows { area.bottom() - y } else { rh - 1 };
        let cols = chunk.len() as u16;
        let cw = (area.width + 3) / cols;
        for (ci, id) in chunk.iter().enumerate() {
            let x = area.x + ci as u16 * cw;
            let w = if ci as u16 + 1 == cols { area.right() - x } else { cw - 3 };
            out.push((*id, Rect { x, y, width: w, height: h }));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Drag {
    Side,
    /// A divider between sessions (index into `dividers`).
    Divider(usize),
    /// A pane's scrollbar.
    Scroll(TermId),
    /// A group's header in the sidebar (its index when pressed; moved yet?): dragged onto
    /// another group it takes that place, released where it was it folds.
    Group(usize, bool),
    /// A session's row (moved yet?): dragged onto another in its group it takes that place,
    /// released where it was it opens.
    Session(TermId, bool),
    /// A tab's pill (its index now): dragged onto another tab it takes that place.
    Tab(usize),
}

/// The sidebar's width limits.
pub(super) const SIDE_MIN: u16 = 24;
pub(super) const SIDE_MAX: u16 = 60;

fn saved_path() -> PathBuf {
    // Per server, like the saved session: a test server (SESHI_SOCKET) keeps its own.
    let name = match std::env::var("SESHI_SOCKET") {
        Ok(l) if l != "default" => format!("seshi-ui-{l}.json"),
        _ => "seshi-ui.json".into(),
    };
    crate::config::data_dir().join(name)
}

impl Hy {
    pub fn load() -> Hy {
        if cfg!(test) {
            return Hy::default();
        }
        let saved = crate::config::read_state(&saved_path());
        Hy { saved, ..Default::default() }
    }

    pub fn save(&self) {
        *self.model_cache.borrow_mut() = None;
        if cfg!(test) {
            return;
        }
        if let Ok(s) = serde_json::to_string_pretty(&self.saved)
            && let Err(e) = crate::config::write_atomic(&saved_path(), s)
        {
            tracing::warn!("couldn't save seshi's state: {e}");
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
    /// Picking a folder for this setting (Settings → Start folder) instead of opening one.
    pub for_setting: Option<&'static str>,
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
    /// What the row says: your name for it, else its worktree, else repo/branch on a
    /// feature branch, else the folder it's in.
    pub name: String,
    pub title: String,
    pub agent: String,
    pub status: Status,
    pub since: u64,
    pub question: Option<String>,
    pub is_agent: bool,
    pub asleep: bool,
    /// Subagents it's running right now.
    pub subagents: Vec<String>,
    pub model: String,
    /// It rang the bell and you haven't looked.
    pub bell: bool,
    /// How full its context is, 0–100, when it says.
    pub context: Option<f32>,
    /// Stopped by a plan limit: seshi says "continue" then (unix seconds).
    pub resume_at: Option<u64>,
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
    /// The sidebar section it's in.
    pub kind: Kind,
}

/// The sidebar's sections, in order: each holds one type of session, grouped by where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    /// Coding agents, by repo or folder.
    Agents,
    /// Plain terminals, by repo or folder.
    Terminals,
    /// Sessions on another machine (ssh, mosh), by machine.
    Ssh,
}

impl Kind {
    pub fn heading(self) -> &'static str {
        match self {
            Kind::Agents => "Agents",
            Kind::Terminals => "Terminals",
            Kind::Ssh => "SSH",
        }
    }

    /// Which section a session belongs in.
    pub fn of(t: &TermInfo) -> Kind {
        if t.remote.is_some() {
            Kind::Ssh
        } else if t.agent.is_some() {
            Kind::Agents
        } else {
            Kind::Terminals
        }
    }
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

/// A split is one session: the pane it was split from keeps its row, the panes opened
/// beside it don't get rows of their own. The row shows the most urgent of them, so an agent
/// in a split that needs you still says so (and what it asks).
pub(super) fn fold_splits(projs: &mut [Proj], tabs: &[HyTab]) {
    let mut owners: Vec<TermId> = tabs.iter().map(|t| t.owner).collect();
    owners.sort_unstable();
    owners.dedup();
    for owner in owners {
        // Everything in the session's tabs, besides the session itself.
        let beside: Vec<TermId> = tabs.iter().filter(|t| t.owner == owner).flat_map(|t| t.layout.leaves()).filter(|id| *id != owner).collect();
        let has_row = projs.iter().flat_map(|p| p.sessions()).any(|s| s.term == owner);
        if beside.is_empty() || !has_row {
            continue;
        }
        let mut members: Vec<Session> = Vec::new();
        for p in projs.iter_mut() {
            for w in &mut p.wts {
                w.sessions.retain(|s| {
                    let gone = beside.contains(&s.term);
                    if gone {
                        members.push(s.clone());
                    }
                    !gone
                });
            }
        }
        let Some(row) = projs.iter_mut().flat_map(|p| p.wts.iter_mut()).flat_map(|w| w.sessions.iter_mut()).find(|s| s.term == owner) else {
            continue;
        };
        if let Some(m) = members.into_iter().filter(|m| rank(m.status) < rank(row.status)).min_by_key(|m| rank(m.status)) {
            row.status = m.status;
            row.since = m.since;
            row.question = m.question;
        }
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

/// How long until `at` (unix seconds): "1h20m", "5m", "now".
pub(super) fn until(at: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = at.saturating_sub(now);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 if (s % 3600) / 60 > 0 => format!("{}h{}m", s / 3600, (s % 3600) / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// How full a context (or used a limit) is worth showing, and in what colour.
pub(super) fn fullness(t: &crate::theme::Theme, pct: f32) -> Color {
    if pct >= 90.0 {
        t.blocked
    } else if pct >= 70.0 {
        t.working
    } else {
        t.muted
    }
}

/// A context this full is shown on the sidebar row (below it, only on the pane's bar).
pub(super) const CONTEXT_SHOWN_FROM: f32 = 50.0;

/// An agent's numbered choices on screen ("❯ 1. Yes", "  2. Yes, and always allow …"),
/// shortened for buttons. Falls back to Yes / Always / No.
pub(super) fn options(parser: Option<&vt100::Parser>) -> Vec<String> {
    let found = choices(parser);
    if found.is_empty() {
        return vec!["Yes".into(), "Always".into(), "No".into()];
    }
    found
}

/// The numbered choices on an agent's screen, shortened for buttons; none when it isn't
/// asking anything there.
pub(super) fn choices(parser: Option<&vt100::Parser>) -> Vec<String> {
    let mut out: Vec<(u8, String)> = Vec::new();
    if let Some(p) = parser {
        let screen = p.screen();
        let (_, cols) = screen.size();
        let lines: Vec<String> = screen.rows(0, cols).collect();
        for l in lines.iter().rev().filter(|l| !l.trim().is_empty()).take(16) {
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
    if out.first().is_none_or(|(n, _)| *n != 1) {
        return Vec::new();
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
        let add_proj = |projs: &mut Vec<Proj>, path: &Path, git: bool, kind: Kind| -> usize {
            let key = path_key(path);
            if let Some(i) = projs.iter().position(|p| p.key == key && p.kind == kind) {
                projs[i].git |= git;
                return i;
            }
            let ci = order.iter().position(|k| *k == key).unwrap_or(order.len() + projs.len());
            projs.push(Proj {
                key: key.clone(),
                path: path.to_path_buf(),
                name: self.hy.saved.names.get(&key).cloned().unwrap_or_else(|| folder_name(path)),
                color: self.theme.project(ci, &fallback),
                wts: Vec::new(),
                fresh: self.hy.fresh.contains(&key),
                git,
                branches: Vec::new(),
                kind,
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
                let placed = self.hy.saved.placed.iter().find(|(x, ..)| x == id);
                let (root, top, branch, main, git) = match (&t.root, &t.top) {
                    // Dragged into another group: it stays there.
                    _ if let Some((_, p, g)) = placed => {
                        let branch = if t.root.as_ref().is_some_and(|r| path_key(r) == path_key(p)) { t.branch.clone().unwrap_or_default() } else { String::new() };
                        (p.clone(), p.clone(), branch, true, *g)
                    }
                    (Some(r), Some(tp)) => {
                        let main = path_key(r) == path_key(tp);
                        (r.clone(), tp.clone(), t.branch.clone().unwrap_or_default(), main, true)
                    }
                    _ => {
                        let cwd = if t.cwd.as_os_str().is_empty() { w.cwd.clone() } else { t.cwd.clone() };
                        (cwd.clone(), cwd, String::new(), true, false)
                    }
                };
                let kind = Kind::of(t);
                let (name, pi, wi) = match &t.remote {
                    // On another machine: grouped by that machine (the local folder means
                    // nothing there).
                    Some(host) => {
                        let key = format!("ssh:{host}");
                        let pi = match projs.iter().position(|p| p.kind == Kind::Ssh && p.key == key) {
                            Some(i) => i,
                            None => {
                                projs.push(Proj {
                                    key: key.clone(),
                                    path: cwd_of(t, w),
                                    name: host.clone(),
                                    color: self.theme.muted,
                                    wts: vec![Wt { key, path: cwd_of(t, w), name: String::new(), branch: String::new(), main: true, sessions: Vec::new() }],
                                    fresh: false,
                                    git: false,
                                    branches: Vec::new(),
                                    kind,
                                });
                                projs.len() - 1
                            }
                        };
                        let what = t.label.trim().to_string();
                        (if what.is_empty() { t.process.clone() } else { what }, pi, 0)
                    }
                    None => {
                        let name = row_name(t, &root, &top, main, git);
                        let pi = add_proj(&mut projs, &root, git, kind);
                        let wi = add_wt(&mut projs[pi], &top, branch, main);
                        (name, pi, wi)
                    }
                };
                let title = session_title(t, if leaves.len() == 1 { &w.name } else { "" }, &top);
                // What it asks: its ask-human question, else what its screen asks.
                let question = (t.status == Status::Blocked)
                    .then(|| self.snap.questions.iter().find(|q| q.term == *id).map(|q| q.text.clone()).or_else(|| self.parsers.get(id).and_then(question)))
                    .flatten();
                projs[pi].wts[wi].sessions.push(Session {
                    term: *id,
                    name,
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
                    bell: t.bell,
                    model: t.model.clone(),
                    context: t.usage.context,
                    resume_at: t.resume_at,
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
        fold_splits(&mut projs, &self.hy.tabs);
        // Groups come from where sessions are: one left with none (its only pane is inside a
        // split elsewhere) goes.
        projs.retain(|p| p.sessions().next().is_some());
        for p in &mut projs {
            if sort {
                for w in &mut p.wts {
                    let order = &self.hy.saved.session_order;
                    w.sessions.sort_by_key(|s| (rank(s.status), order.iter().position(|t| *t == s.term).unwrap_or(usize::MAX), s.term));
                }
            }
            p.wts.sort_by_key(|w| {
                let worst = w.sessions.iter().map(|s| rank(s.status)).min().unwrap_or(4);
                (!w.main, if sort { worst } else { 0 }, w.name.clone())
            });
        }
        projs.sort_by_key(|p| order.iter().position(|k| *k == p.key).unwrap_or(usize::MAX));
        // Attention first: a project with something that needs you goes to the top.
        projs.sort_by_key(|p| !p.sessions().any(|s| s.status == Status::Blocked));
        // Sections: agents, then terminals, then other machines (each keeps the order above).
        projs.sort_by_key(|p| p.kind);
        // The same name twice in a project: number them (shell 1, shell 2), oldest first.
        for p in &mut projs {
            let mut seen: HashMap<String, Vec<TermId>> = HashMap::new();
            for s in p.wts.iter().flat_map(|w| w.sessions.iter()) {
                seen.entry(s.name.clone()).or_default().push(s.term);
            }
            for w in &mut p.wts {
                for s in &mut w.sessions {
                    if let Some(ids) = seen.get(&s.name).filter(|ids| ids.len() > 1) {
                        let mut ids = ids.clone();
                        ids.sort();
                        let n = ids.iter().position(|i| *i == s.term).unwrap_or(0) + 1;
                        s.name = format!("{} {n}", s.name);
                    }
                }
            }
        }
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
        // You closed the last session: a shell in your home folder takes its place, so there's
        // always somewhere to type (and closing a project never closes the app).
        if focus.is_none() && self.hy.last_focus.is_some() && self.snap.terms.is_empty() {
            self.hy.last_focus = None;
            self.splash = false;
            let home = self.cfg.start_dir().or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())).unwrap_or_else(std::env::temp_dir);
            self.hy_new_session(home, None, false);
            return;
        }
        let proj_of = |t: TermId| model.iter().find(|p| p.sessions().any(|s| s.term == t)).map(|p| p.key.clone());
        if focus != self.hy.last_focus {
            self.hy.follow = true;
            if let Some(f) = focus {
                if let Some(k) = proj_of(f) {
                    self.hy.fresh.remove(&k);
                    self.hy.proj = Some(k);
                }
                let prev = self.hy.last_focus;
                self.hy_place(f, prev);
                // Which panes share a split may have changed, and the sidebar folds those.
                self.hy_fresh();
            }
            self.hy.last_focus = focus;
        }
        // Sessions that ended leave their tabs; empty tabs go.
        let alive = |t: TermId| self.snap.terms.contains_key(&t);
        let mut tabs: Vec<HyTab> = self
            .hy
            .tabs
            .iter()
            .cloned()
            .filter_map(|tab| {
                let layout = tab.layout.map_leaves(&mut |id| alive(id).then_some(id))?;
                let focus = if layout.contains(tab.focus) { tab.focus } else { layout.first_leaf() };
                Some(HyTab { layout, focus, ..tab })
            })
            .collect();
        tabs.dedup_by(|a, b| a.layout == b.layout);
        // A session that ended with other tabs left: the first of those becomes the session.
        let gone: Vec<TermId> = tabs.iter().map(|t| t.owner).filter(|o| !alive(*o)).collect();
        for o in gone {
            if let Some(next) = tabs.iter().find(|t| t.owner == o).map(|t| t.layout.first_leaf()) {
                for t in tabs.iter_mut().filter(|t| t.owner == o) {
                    t.owner = next;
                }
            }
        }
        if tabs != self.hy.tabs {
            self.hy_fresh();
        }
        self.hy.tabs = tabs;
        self.hy.tab = self.hy.tab.min(self.hy.tabs.len().saturating_sub(1));
        if self.hy.proj.as_ref().is_none_or(|k| !model.iter().any(|p| &p.key == k)) {
            self.hy.proj = focus.and_then(proj_of).or_else(|| model.first().map(|p| p.key.clone()));
        }
        self.recipe_followup();
    }
}

/// What to call a session: its name if renamed, else the agent's last prompt, else where a
/// shell is (inside its worktree), else the program.
/// A session's row name: what you renamed it to; else the worktree it's in; else the
/// folder under the project it's in; else (the project's own folder) its agent or "shell".
/// Where a pane is: what it reported, else its workspace's folder.
fn cwd_of(t: &TermInfo, w: &crate::protocol::WorkspaceInfo) -> PathBuf {
    if t.cwd.as_os_str().is_empty() { w.cwd.clone() } else { t.cwd.clone() }
}

fn row_name(t: &TermInfo, root: &Path, top: &Path, main: bool, git: bool) -> String {
    if !t.label.trim().is_empty() {
        return t.label.trim().to_string();
    }
    if git && !main {
        return folder_name(top);
    }
    let cwd = if t.cwd.as_os_str().is_empty() { top } else { t.cwd.as_path() };
    // An agent by its name; a program (vim, a harness seshi doesn't know) by its own.
    let kind = t.agent.clone().unwrap_or_else(|| if t.is_shell() || t.process.is_empty() { "shell".into() } else { t.process.clone() });
    match cwd.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string().replace('\\', "/"),
        // The project's own folder: the agent's (or shell's) name.
        Ok(_) => kind,
        Err(_) => folder_name(cwd),
    }
}

/// The icon for what runs in a pane.
pub(super) fn kind_icon(app: &App, agent: &str, is_agent: bool) -> String {
    if !is_agent {
        return app.cfg.icons.shell.clone();
    }
    match agent {
        "claude" => "✻",
        "codex" => "◇",
        "gemini" => "✦",
        "opencode" => "◈",
        "cursor" => "▲",
        "copilot" => "◎",
        "aider" => "◆",
        "grok" => "✕",
        _ => "◆",
    }
    .to_string()
}

fn session_title(t: &TermInfo, name: &str, top: &Path) -> String {
    if !t.label.trim().is_empty() {
        return t.label.trim().to_string();
    }
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
    /// Fold / unfold a project.
    ToggleProj(usize),
    Session(TermId),
    /// An answer chip in the Inbox: send this key to that agent.
    InboxAnswer(TermId, char),
    Settings,
    CloseSplit(TermId),
    Divider(usize),
    /// The ⋯ on a hovered sidebar row: its menu.
    RowMenuSess(TermId),
    /// Start a shell in this project (its "nothing running" line).
    ShellIn(usize),
    RowMenuProj(usize),
    ConfirmYes,
    ConfirmNo,
    TabPick(usize),
    /// The ✕ on the current tab's pill.
    TabClose(usize),
    TabNew,
    /// Outside an overlay: closes it.
    Close,
    /// Inside an overlay's panel: nothing.
    Noop,
    FinderPick(usize),
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
    /// A right-click menu item.
    MenuPick(usize),
    /// The sidebar's edge (drag to resize).
    SideEdge,
    /// The sidebar's empty space: the keys go to it.
    SideFocus,
    /// Install the newer seshi (in Settings › General).
    Update,
    /// The sidebar's "a actions".
    Actions,
    /// A row of the actions list.
    ActionRow(usize),
    /// The leader pill (leader mode is on): the key map.
    Leader,
    /// A key in the key map (its row in the shown list).
    KeyRow(usize),
    /// A pane's scrollbar (click or drag).
    ScrollBar(TermId),
    FindTab(u8),
    FindRow(usize),
    BranchRow(usize),
    GoPick(usize),
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

mod behaviour;
mod card;
mod dialogs;
mod leader;
mod popups;
mod screen;
mod splash;

pub(in crate::client) use self::{card::*, dialogs::*, leader::*, popups::*, screen::*, splash::*};


#[cfg(test)]
mod arrange_tests {
    use super::arranged;
    use ratatui::layout::Rect;

    #[test]
    fn many_panes_tile_in_rows() {
        let area = Rect { x: 0, y: 0, width: 200, height: 50 };
        let r = arranged(&[1, 2, 3], area);
        assert!(r.iter().all(|(_, x)| x.height == 50), "three fit in one row: {r:?}");
        assert!(r[0].1.width.abs_diff(r[2].1.width) <= 3, "the same size: {r:?}");
        let r = arranged(&[1, 2, 3, 4], area);
        assert!(r[2].1.y > 0 && r[0].1.y == 0, "four take two rows: {r:?}");
    }
}
