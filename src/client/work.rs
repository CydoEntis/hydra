//! Ways work starts: ideas you jot down, tickets from your trackers, races between agents,
//! and recipes. Each ends the same way: an agent on it in its own worktree.

use super::design::{BtnKind, Seg, fill, path_key, put, seg, segs_width, tilde};
use super::hydra::{HyHit, Proj, btn, dim_all, find, folder_name, hints, hit, hovered, panel, sel_row};
use super::render::truncate;
use super::{App, Bg, Mode};
use crate::protocol::{Command, QueueItem, QueueState, QueuedTicket, Status};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use serde::{Deserialize, Serialize};
use crate::proc::quiet;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;
pub use crate::tickets::{Source, Ticket, slug, sources};

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---- ideas -----------------------------------------------------------------------------------

/// An idea, waiting to become work. Lives next to config.toml (and syncs with it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Idea {
    pub text: String,
    /// The project it's for (a repo's main folder), if any.
    pub project: Option<PathBuf>,
    pub at: u64,
}

pub fn ideas_path() -> PathBuf {
    crate::config::config_path().parent().map(|d| d.join("ideas.json")).unwrap_or_else(|| PathBuf::from("ideas.json"))
}

pub fn load_ideas() -> Vec<Idea> {
    if cfg!(test) {
        return Vec::new();
    }
    std::fs::read_to_string(ideas_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_ideas(ideas: &[Idea]) {
    if cfg!(test) {
        return;
    }
    if let Ok(s) = serde_json::to_string_pretty(ideas) {
        if let Err(e) = crate::config::write_atomic(&ideas_path(), s) {
            tracing::warn!("couldn't save ideas: {e}");
        }
        crate::sync::push_soon();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdeasView {
    pub ideas: Vec<Idea>,
    pub input: String,
    pub sel: usize,
    /// Which project a new idea is for: an index into the projects, or past the end for none.
    pub tag: usize,
}

// ---- tickets ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct TicketsView {
    pub dir: PathBuf,
    /// (source id, label) per tab.
    pub tabs: Vec<(String, String)>,
    pub tab: usize,
    pub lists: Vec<Option<Result<Vec<Ticket>, String>>>,
    pub query: String,
    pub sel: usize,
}

/// The Tickets view's last tab: hydra's queue.
pub const QUEUE_TAB: &str = "queue";

impl TicketsView {
    /// On the Queue tab (its list is the server's queue, and what you type is a new task).
    pub fn on_queue(&self) -> bool {
        self.tabs.get(self.tab).is_some_and(|(id, _)| id == QUEUE_TAB)
    }

    pub fn visible(&self) -> Vec<&Ticket> {
        let Some(Some(Ok(list))) = self.lists.get(self.tab) else { return Vec::new() };
        let q = self.query.to_lowercase();
        list.iter().filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q) || t.key.to_lowercase().contains(&q)).collect()
    }
}

// ---- races -----------------------------------------------------------------------------------

/// The same task given to several agents, each in its own worktree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Race {
    pub id: u64,
    pub project: PathBuf,
    pub prompt: String,
    pub base: String,
    /// (agent, branch)
    pub entries: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RaceNew {
    pub text: String,
    pub picked: Vec<bool>,
    /// 0: the task, 1: the agents
    pub row: u8,
    pub cur: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RaceView {
    pub id: u64,
    pub sel: usize,
    /// Per entry: "+42 −7 · 3 files", once known.
    pub stats: Vec<Option<String>>,
    /// Waiting for Enter to keep the selected one and drop the rest.
    pub confirm: bool,
}

// ---- starting work ---------------------------------------------------------------------------

impl App {
    /// The command that starts `agent` on `prompt` (the quick prompt's template when known).
    pub(super) fn agent_cmd(&self, agent: &str, prompt: &str) -> String {
        let q = self.cfg.quote_for_shell(prompt);
        match self.cfg.quick.agents.iter().find(|a| a.name == agent) {
            Some(a) => a.command.replace("{prompt}", &q),
            None => format!("{agent} {q}"),
        }
    }

    /// An agent on a task: its own worktree on `branch` in a git project, else in the folder.
    pub(super) fn start_task(&mut self, project: &Path, agent: &str, prompt: &str, branch: &str) {
        let model = self.hy_model();
        let cmd = self.agent_cmd(agent, prompt);
        let p = model.iter().find(|p| path_key(&p.path) == path_key(project)).cloned();
        match p {
            Some(p) if p.git => {
                let taken: Vec<String> = p.wts.iter().map(|w| w.branch.clone()).chain(p.branches.iter().cloned()).collect();
                let mut b = if branch.is_empty() { slug(prompt, 4) } else { branch.to_string() };
                if taken.contains(&b) {
                    b = format!("{b}-{}", now() % 1000);
                }
                self.hy_start_worktree(&p, Some(cmd), Some(b));
            }
            _ => self.hy_new_session(project.to_path_buf(), Some(cmd), false),
        }
    }

    fn current_project(&self) -> Option<Proj> {
        let model = self.hy_model();
        model.iter().find(|p| self.hy.proj.as_deref() == Some(p.key.as_str())).or(model.first()).cloned()
    }

    // ---- ideas

    pub(super) fn open_ideas(&mut self) {
        self.mode = Mode::Ideas(Box::new(IdeasView { ideas: load_ideas(), input: String::new(), sel: 0, tag: 0 }));
    }

    /// The projects ideas can be tagged with, current first.
    fn idea_projects(&self) -> Vec<Proj> {
        let mut model = self.hy_model();
        if let Some(i) = model.iter().position(|p| self.hy.proj.as_deref() == Some(p.key.as_str())) {
            let cur = model.remove(i);
            model.insert(0, cur);
        }
        model
    }

    /// Ideas in display order: this project's first, then the rest by project.
    fn idea_order(&self, ideas: &[Idea]) -> Vec<usize> {
        let cur = self.hy.proj.clone().unwrap_or_default();
        let mut idx: Vec<usize> = (0..ideas.len()).collect();
        idx.sort_by_key(|&i| {
            let k = ideas[i].project.as_ref().map(|p| path_key(p)).unwrap_or_default();
            (k != cur, k, std::cmp::Reverse(ideas[i].at))
        });
        idx
    }

    pub(super) fn on_ideas_key(&mut self, mut v: IdeasView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let projects = self.idea_projects();
        let order = self.idea_order(&v.ideas);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab => v.tag = (v.tag + 1) % (projects.len() + 1),
            KeyCode::BackTab => v.tag = (v.tag + projects.len()) % (projects.len() + 1),
            KeyCode::Down => v.sel = (v.sel + 1).min(order.len().saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Delete => {
                if let Some(&i) = order.get(v.sel) {
                    v.ideas.remove(i);
                    save_ideas(&v.ideas);
                    v.sel = v.sel.min(v.ideas.len().saturating_sub(1));
                }
            }
            KeyCode::Enter if !v.input.trim().is_empty() => {
                let project = projects.get(v.tag).map(|p| p.path.clone());
                v.ideas.push(Idea { text: v.input.trim().to_string(), project, at: now() });
                save_ideas(&v.ideas);
                v.input.clear();
                self.notify("idea saved".into(), false);
            }
            KeyCode::Enter => {
                let Some(&i) = order.get(v.sel) else { return };
                let idea = v.ideas.remove(i);
                save_ideas(&v.ideas);
                let project = idea.project.clone().or_else(|| self.current_project().map(|p| p.path));
                let Some(project) = project else {
                    self.notify("start a session first (o opens a folder)".into(), true);
                    return;
                };
                let agent = self.hy_agent();
                self.mode = Mode::Normal;
                self.start_task(&project, &agent, &idea.text, &slug(&idea.text, 4));
                self.notify(format!("{agent} is on it: {}", truncate(&idea.text, 50)), false);
                return;
            }
            KeyCode::Backspace => {
                v.input.pop();
            }
            KeyCode::Char(c) if !ctrl => v.input.push(c),
            _ => {}
        }
        self.mode = Mode::Ideas(Box::new(v));
    }

    // ---- tickets

    pub(super) fn open_tickets(&mut self) {
        let srcs = sources(&self.cfg.tickets);
        let mut tabs: Vec<(String, String)> = srcs.iter().map(|s| (s.id().to_string(), s.label().to_string())).collect();
        tabs.push((QUEUE_TAB.into(), "Queue".into()));
        let proj = self.current_project();
        let dir = proj.as_ref().map(|p| p.path.clone()).unwrap_or_else(|| self.here_dir());
        // The project's own tracker opens first.
        let tab = proj
            .and_then(|p| self.cfg.tickets.projects.get(&p.name).cloned())
            .and_then(|id| tabs.iter().position(|(t, _)| *t == id))
            .unwrap_or(0);
        let n = tabs.len();
        let mut lists = vec![None; n];
        lists[n - 1] = Some(Ok(Vec::new()));
        let v = TicketsView { dir: dir.clone(), tabs, tab, lists, query: String::new(), sel: 0 };
        if !v.on_queue() {
            self.load_tickets(dir, tab);
        }
        self.mode = Mode::Tickets(Box::new(v));
    }

    /// Open the Tickets view on its Queue tab.
    pub(super) fn open_queue(&mut self) {
        self.open_tickets();
        if let Mode::Tickets(v) = &mut self.mode {
            v.tab = v.tabs.len() - 1;
        }
    }

    /// Put work in hydra's queue, for the agent new work goes to.
    fn queue_work(&mut self, dir: &Path, prompt: &str, title: &str, branch: &str, ticket: Option<QueuedTicket>) {
        let agent = self.hy_agent();
        let item = QueueItem {
            id: 0,
            project: dir.to_path_buf(),
            agent: agent.clone(),
            cmd: self.agent_cmd(&agent, prompt),
            title: title.to_string(),
            branch: branch.to_string(),
            ticket,
            state: QueueState::Waiting,
            worked: false,
        };
        self.cmd(Command::Enqueue { item });
        let busy = self.snap.queue.iter().filter(|q| !matches!(q.state, QueueState::Review(_) | QueueState::Failed(_))).count();
        let at_once = self.cfg.queue_at_once.max(1) as usize;
        let when = if busy < at_once { "starting now".to_string() } else { format!("{} ahead of it", busy + 1 - at_once) };
        self.notify(format!("queued {title} for {agent}: {when}"), false);
    }

    fn load_tickets(&self, dir: PathBuf, tab: usize) {
        let cfg = self.cfg.tickets.clone();
        self.spawn_bg(move || {
            let srcs = sources(&cfg);
            let result = srcs.get(tab).map(|s| s.list(&dir)).unwrap_or_else(|| Err("no such source".into()));
            Bg::Tickets(dir, tab, result)
        });
    }

    pub(super) fn on_tickets_key(&mut self, mut v: TicketsView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = if v.on_queue() { self.snap.queue.len() } else { v.visible().len() };
        if v.on_queue() {
            match k.code {
                // A task typed in joins the queue; with nothing typed, Enter opens the one picked.
                KeyCode::Enter if !v.query.trim().is_empty() => {
                    let task = std::mem::take(&mut v.query);
                    let title: String = task.trim().chars().take(60).collect();
                    let dir = v.dir.clone();
                    self.queue_work(&dir, task.trim(), &title, &slug(&task, 4), None);
                    self.mode = Mode::Tickets(Box::new(v));
                    return;
                }
                KeyCode::Enter => {
                    if let Some(QueueState::Running(term) | QueueState::Review(term)) = self.snap.queue.get(v.sel).map(|q| q.state.clone()) {
                        self.mode = Mode::Normal;
                        self.hy_focus(term);
                        return;
                    }
                }
                KeyCode::Delete => {
                    if let Some(q) = self.snap.queue.get(v.sel) {
                        self.cmd(Command::Dequeue { id: q.id });
                        v.sel = v.sel.min(n.saturating_sub(2));
                    }
                    self.mode = Mode::Tickets(Box::new(v));
                    return;
                }
                _ => {}
            }
        }
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let d = if k.code == KeyCode::Tab { 1 } else { v.tabs.len() - 1 };
                v.tab = (v.tab + d) % v.tabs.len();
                v.sel = 0;
                v.query.clear();
                if v.lists[v.tab].is_none() {
                    self.load_tickets(v.dir.clone(), v.tab);
                }
            }
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Char('o') if ctrl => {
                if let Some(t) = v.visible().get(v.sel) {
                    super::files::open_url(&t.url);
                }
            }
            KeyCode::Char('r') if ctrl && !v.on_queue() => {
                v.lists[v.tab] = None;
                self.load_tickets(v.dir.clone(), v.tab);
            }
            KeyCode::Char('q') if ctrl && !v.on_queue() => {
                if let Some(t) = v.visible().get(v.sel).map(|t| (*t).clone()) {
                    let (source, label) = v.tabs[v.tab].clone();
                    let ticket = QueuedTicket { source, key: t.key.clone(), id: t.id.clone(), url: t.url.clone() };
                    let dir = v.dir.clone();
                    self.queue_work(&dir, &t.prompt(&label), &format!("{} {}", t.key, t.title), &t.branch(), Some(ticket));
                    v.sel = (v.sel + 1).min(n.saturating_sub(1));
                }
            }
            KeyCode::Enter if !v.on_queue() => {
                let Some(t) = v.visible().get(v.sel).map(|t| (*t).clone()) else { return };
                let source = v.tabs[v.tab].1.clone();
                let agent = self.hy_agent();
                self.mode = Mode::Normal;
                self.start_task(&v.dir, &agent, &t.prompt(&source), &t.branch());
                self.notify(format!("{agent} is on {}", t.key), false);
                return;
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.sel = 0;
            }
            _ => {}
        }
        self.mode = Mode::Tickets(Box::new(v));
    }

    // ---- races

    pub(super) fn open_race_new(&mut self) {
        let agents = self.race_agents();
        let picked = agents.iter().enumerate().map(|(i, _)| i < 2).collect();
        self.mode = Mode::RaceNew(Box::new(RaceNew { text: String::new(), picked, row: 0, cur: 0 }));
    }

    fn race_agents(&self) -> Vec<String> {
        super::hydra::np_agents(self).into_iter().filter(|a| a != "shell" && !a.starts_with('⚙')).collect()
    }

    pub(super) fn on_race_new_key(&mut self, mut v: RaceNew, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = v.picked.len();
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => v.row = 1 - v.row,
            KeyCode::Left if v.row == 1 => v.cur = (v.cur + n - 1) % n.max(1),
            KeyCode::Right if v.row == 1 => v.cur = (v.cur + 1) % n.max(1),
            KeyCode::Char(' ') if v.row == 1 => {
                if let Some(p) = v.picked.get_mut(v.cur) {
                    *p = !*p;
                }
            }
            KeyCode::Enter => {
                if v.text.trim().is_empty() {
                    self.notify("type the task first".into(), true);
                } else if v.picked.iter().filter(|p| **p).count() < 2 {
                    self.notify("pick at least two agents to race".into(), true);
                } else {
                    return self.start_race(v);
                }
            }
            KeyCode::Backspace if v.row == 0 => {
                v.text.pop();
            }
            KeyCode::Char(c) if v.row == 0 && !ctrl => v.text.push(c),
            _ => {}
        }
        self.mode = Mode::RaceNew(Box::new(v));
    }

    fn start_race(&mut self, v: RaceNew) {
        self.mode = Mode::Normal;
        let Some(p) = self.current_project().filter(|p| p.git) else {
            self.notify("races need a git project (each agent gets a worktree)".into(), true);
            return;
        };
        let agents: Vec<String> = self.race_agents().into_iter().zip(v.picked).filter(|(_, on)| *on).map(|(a, _)| a).collect();
        let base = p.wts.iter().find(|w| w.main).map(|w| w.branch.clone()).unwrap_or_else(|| "main".into());
        let id = now();
        let stem = format!("race-{}", slug(&v.text, 3));
        let mut entries = Vec::new();
        for a in &agents {
            let branch = format!("{stem}-{a}");
            let cmd = self.agent_cmd(a, &v.text);
            self.hy_start_worktree(&p, Some(cmd), Some(branch.clone()));
            entries.push((a.clone(), branch));
        }
        self.hy.saved.races.push(Race { id, project: p.path.clone(), prompt: v.text.clone(), base, entries });
        self.hy.save();
        self.notify(format!("racing {} on \"{}\"", agents.join(" vs "), truncate(&v.text, 40)), false);
    }

    pub(super) fn open_race(&mut self, id: u64) {
        let Some(r) = self.hy.saved.races.iter().find(|r| r.id == id).cloned() else { return };
        self.mode = Mode::Race(Box::new(RaceView { id, sel: 0, stats: vec![None; r.entries.len()], confirm: false }));
        let model = self.hy_model();
        for (i, (_, branch)) in r.entries.iter().enumerate() {
            let dir = model.iter().flat_map(|p| p.wts.iter()).find(|w| &w.branch == branch).map(|w| w.path.clone());
            let base = r.base.clone();
            self.spawn_bg(move || Bg::RaceStat(id, i, dir.map(|d| diff_stat(&d, &base)).unwrap_or_else(|| "not started yet".into())));
        }
    }

    pub(super) fn on_race_key(&mut self, mut v: RaceView, k: &KeyEvent) {
        let Some(r) = self.hy.saved.races.iter().find(|r| r.id == v.id).cloned() else {
            self.mode = Mode::Normal;
            return;
        };
        let model = self.hy_model();
        let wt_of = |branch: &str| model.iter().flat_map(|p| p.wts.iter()).find(|w| w.branch == branch).cloned();
        if v.confirm {
            v.confirm = false;
            if k.code == KeyCode::Enter {
                return self.keep_race_winner(&r, v.sel);
            }
            self.mode = Mode::Race(Box::new(v));
            return;
        }
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down => v.sel = (v.sel + 1).min(r.entries.len().saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(t) = r.entries.get(v.sel).and_then(|(_, b)| wt_of(b)).and_then(|w| w.sessions.first().map(|s| s.term)) {
                    self.hy_focus(t);
                    return;
                }
            }
            KeyCode::Char('d') => {
                if let Some(w) = r.entries.get(v.sel).and_then(|(_, b)| wt_of(b)) {
                    self.mode = Mode::Normal;
                    self.open_changes(w.path);
                    return;
                }
            }
            KeyCode::Char('k') => v.confirm = true,
            _ => {}
        }
        self.mode = Mode::Race(Box::new(v));
    }

    /// Keep one entry; the other worktrees and their branches go.
    fn keep_race_winner(&mut self, r: &Race, keep: usize) {
        self.mode = Mode::Normal;
        let model = self.hy_model();
        for (i, (_, branch)) in r.entries.iter().enumerate() {
            if i == keep {
                continue;
            }
            let Some(w) = model.iter().flat_map(|p| p.wts.iter()).find(|w| &w.branch == branch).cloned() else { continue };
            let ws = w.sessions.first().and_then(|s| self.snap.locate(s.term)).map(|(ws, _)| ws.id);
            match ws {
                Some(ws) => self.cmd(Command::RemoveWorktree { ws, force: true, delete_branch: true }),
                None => {
                    let (dir, root, b) = (w.path.clone(), r.project.clone(), branch.clone());
                    self.spawn_bg(move || Bg::Done(drop_worktree(&root, &dir, &b), false));
                }
            }
        }
        self.hy.saved.races.retain(|x| x.id != r.id);
        self.hy.save();
        let (agent, branch) = r.entries[keep].clone();
        self.notify(format!("kept {agent}'s {branch}; the others are gone"), false);
    }

    // ---- recipes

    /// Run a recipe from config in a project: its own worktree (if it says so) running the
    /// first command, with the rest started beside it once the worktree exists.
    pub(super) fn run_recipe(&mut self, p: &Proj, name: &str) {
        let Some(r) = self.cfg.recipes.iter().find(|r| r.name == name).cloned() else { return };
        let Some(first) = r.run.first().cloned() else { return };
        let rest: Vec<String> = r.run[1..].to_vec();
        let main = p.wts.iter().find(|w| w.main).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
        if r.worktree && p.git {
            let branch = format!("{}-{}", slug(&r.name, 2), now() % 10_000);
            self.hy.pending_recipe = Some((branch.clone(), rest, std::time::Instant::now()));
            self.hy_start_worktree(p, Some(first), Some(branch));
        } else {
            // The last one started gets the focus, so start the first command last.
            for c in rest.iter().rev() {
                self.cmd(Command::NewWorkspace { cwd: Some(main.clone()), name: None, cmd: Some(c.clone()) });
            }
            self.hy_new_session(main, Some(first), false);
        }
        self.notify(format!("recipe {}: {}", r.name, r.run.join(" + ")), false);
    }

    /// Once a recipe's worktree exists, start the rest of its commands there.
    pub(super) fn recipe_followup(&mut self) {
        let Some((branch, rest, at)) = self.hy.pending_recipe.clone() else { return };
        if at.elapsed().as_secs() > 60 {
            self.hy.pending_recipe = None;
            return;
        }
        let model = self.hy_model();
        let Some(w) = model.iter().flat_map(|p| p.wts.iter()).find(|w| w.branch == branch && !w.sessions.is_empty()).cloned() else { return };
        self.hy.pending_recipe = None;
        for c in rest {
            self.cmd(Command::NewWorkspace { cwd: Some(w.path.clone()), name: None, cmd: Some(c) });
        }
        if let Some(first) = w.sessions.first() {
            self.cmd(Command::FocusPane { term: first.term });
        }
    }
}

/// "+42 −7 · 3 files" for a worktree against the base branch (committed and not).
pub fn diff_stat(dir: &Path, base: &str) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(["diff", "--shortstat", base]);
    let out = quiet(&mut cmd).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if out.is_empty() {
        return "no changes yet".into();
    }
    let num = |word: &str| out.split(',').find(|p| p.contains(word)).and_then(|p| p.split_whitespace().next()?.parse::<u32>().ok()).unwrap_or(0);
    format!("+{} −{} · {} files", num("insertion"), num("deletion"), num("file"))
}

/// Remove a worktree that has nothing running in it, and its branch.
fn drop_worktree(root: &Path, dir: &Path, branch: &str) -> Result<String, String> {
    let run = |args: &[&str]| -> Result<(), String> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        let o = quiet(&mut cmd).output().map_err(|e| e.to_string())?;
        if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("git failed").to_string()) }
    };
    run(&["worktree", "remove", "--force", &dir.to_string_lossy()])?;
    run(&["branch", "-D", branch])?;
    Ok(format!("removed {branch}"))
}

// ---- drawing ---------------------------------------------------------------------------------

fn chip(t: &Theme, text: &str, on: bool, hov: bool) -> Seg {
    let st = if on {
        Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
    } else if hov {
        Style::default().bg(t.hov).fg(t.strong)
    } else {
        Style::default().bg(t.btn).fg(t.text)
    };
    seg(format!(" {text} "), st)
}

fn input_row(buf: &mut Buffer, r: Rect, y: u16, t: &Theme, text: &str, placeholder: &str, focused: bool) {
    let row = Rect { x: r.x + 1, y, width: r.width - 2, height: 1 };
    fill(buf, row, t.card2);
    let s = Style::default().bg(t.card2);
    let mut segs = vec![seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(text.to_string(), s.fg(t.strong))];
    if focused {
        segs.push(seg("█", s.fg(t.accent)));
    }
    if text.is_empty() {
        segs.push(seg(format!(" {placeholder}"), s.fg(t.muted)));
    }
    put(buf, r.x + 3, y, &segs, r.right() - 2);
}

pub(super) fn draw_ideas(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &IdeasView) {
    let projects = app.idea_projects();
    let order = app.idea_order(&v.ideas);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 92, 26, "Ideas", &[], t);
    let c = Style::default().bg(t.card);
    input_row(buf, r, r.y + 2, t, &v.input, "jot an idea and press Enter", true);
    let tag = match projects.get(v.tag) {
        Some(p) => vec![seg("for ", c.fg(t.muted)), seg("▌", c.fg(p.color)), seg(p.name.clone(), c.fg(t.strong))],
        None => vec![seg("for ", c.fg(t.muted)), seg("no project in particular", c.fg(t.strong))],
    };
    let mut tag = tag;
    tag.push(seg("   Tab", c.fg(t.accent).add_modifier(Modifier::BOLD)));
    tag.push(seg(" change", c.fg(t.muted)));
    put(buf, r.x + 3, r.y + 3, &tag, r.right() - 2);
    let mut y = r.y + 5;
    let mut last_key: Option<String> = None;
    let max_y = r.bottom().saturating_sub(3);
    let start = v.sel.saturating_sub((max_y - y) as usize / 2);
    for (pos, &i) in order.iter().enumerate().skip(start) {
        if y >= max_y {
            break;
        }
        let idea = &v.ideas[i];
        let key = idea.project.as_ref().map(|p| path_key(p)).unwrap_or_default();
        if last_key.as_ref() != Some(&key) {
            let name = idea.project.as_ref().map(|p| folder_name(p)).unwrap_or_else(|| "ANY PROJECT".into());
            put(buf, r.x + 3, y, &[seg(name.to_uppercase(), c.fg(t.muted).add_modifier(Modifier::BOLD))], r.right());
            y += 1;
            last_key = Some(key);
        }
        if y >= max_y {
            break;
        }
        let sel = pos == v.sel && v.input.is_empty();
        let bg = sel_row(app, buf, r, y, sel, t);
        let st = Style::default().bg(bg);
        put(buf, r.x + 3, y, &[seg("✦ ", st.fg(t.accent)), seg(truncate(&idea.text, (r.width - 20) as usize), st.fg(t.strong))], r.right() - 12);
        put(buf, r.right() - 11, y, &[seg(super::hydra::age(idea.at), st.fg(t.muted))], r.right());
        hit(app, Rect { x: r.x + 1, y, width: r.width - 2, height: 1 }, HyHit::IdeaRow(pos));
        y += 1;
    }
    if order.is_empty() {
        put(buf, r.x + 3, y, &[seg("No ideas yet. Type one above; later, Enter on it starts an agent on it.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
    }
    let agent = app.hy_agent();
    put(
        buf,
        r.x + 3,
        r.bottom() - 2,
        &hints(t, &[("Enter", "save"), ("↑↓ Enter", &format!("start {agent} on it")), ("Del", "delete"), ("Esc", "close")]),
        r.right(),
    );
}

pub(super) fn draw_tickets(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &TicketsView) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 110, 30, &format!("Tickets · {}", tilde(&v.dir)), &[], t);
    let c = Style::default().bg(t.card);
    let mut x = r.x + 2;
    for (i, (_, label)) in v.tabs.iter().enumerate() {
        let w = label.width() as u16 + 2;
        let tr = Rect { x, y: r.y + 2, width: w, height: 1 };
        let ch = chip(t, label, i == v.tab, hovered(app, tr));
        put(buf, x, r.y + 2, &[ch], r.right());
        hit(app, tr, HyHit::TicketTab(i));
        x += w + 2;
    }
    let queued = v.on_queue();
    input_row(buf, r, r.y + 4, t, &v.query, if queued { "type a task, Enter queues it" } else { "type to filter" }, true);
    let list_x = r.x + 3;
    let preview_x = r.x + r.width * 55 / 100;
    let mut y = r.y + 6;
    if queued {
        draw_queue(app, buf, r, y, t, v.sel);
        let agent = app.hy_agent();
        let at_once = format!("{} at once", app.cfg.queue_at_once.max(1));
        put(
            buf,
            r.x + 3,
            r.bottom() - 2,
            &hints(t, &[("Enter", &format!("queue for {agent} / open")), ("Del", "remove"), ("Tab", "tickets"), ("", &at_once), ("Esc", "close")]),
            r.right(),
        );
        return;
    }
    match v.lists.get(v.tab) {
        Some(None) | None => {
            put(buf, list_x, y, &[seg("loading…", c.fg(t.muted))], r.right());
        }
        Some(Some(Err(e))) => {
            for l in super::views::wrap(e, (r.width - 6) as usize) {
                put(buf, list_x, y, &[seg(l, c.fg(t.muted))], r.right() - 2);
                y += 1;
            }
        }
        Some(Some(Ok(_))) => {
            let items = v.visible();
            let max = (r.bottom() - 3 - y) as usize;
            let start = v.sel.saturating_sub(max.saturating_sub(1));
            for (i, it) in items.iter().enumerate().skip(start).take(max) {
                let sel = i == v.sel;
                let row = Rect { x: r.x + 1, y, width: preview_x - r.x - 2, height: 1 };
                let bg = if sel || hovered(app, row) { t.hov } else { t.card };
                fill(buf, row, bg);
                let st = Style::default().bg(bg);
                if sel {
                    put(buf, r.x + 1, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
                }
                let in_queue = app.snap.queue.iter().any(|q| q.ticket.as_ref().is_some_and(|qt| qt.key == it.key && qt.source == v.tabs[v.tab].0));
                put(
                    buf,
                    list_x,
                    y,
                    &[
                        seg(format!("{:<9}", truncate(&it.key, 9)), st.fg(t.accent)),
                        seg(if in_queue { "◷ " } else { "" }, st.fg(t.working)),
                        seg(truncate(&it.title, (preview_x - list_x - 14) as usize), st.fg(t.strong)),
                    ],
                    preview_x - 1,
                );
                hit(app, row, HyHit::TicketRow(i));
                y += 1;
            }
            if items.is_empty() {
                put(buf, list_x, y, &[seg("Nothing open for you here.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
            }
            // Preview of the selected one.
            if let Some(it) = items.get(v.sel) {
                let mut py = r.y + 6;
                for l in super::views::wrap(&it.title, (r.right() - preview_x - 2) as usize).into_iter().take(3) {
                    put(buf, preview_x, py, &[seg(l, c.fg(t.strong).add_modifier(Modifier::BOLD))], r.right() - 2);
                    py += 1;
                }
                put(buf, preview_x, py, &[seg(format!("{} · {}", it.state, it.meta), c.fg(t.muted))], r.right() - 2);
                py += 2;
                for l in super::views::wrap(&it.body, (r.right() - preview_x - 2) as usize) {
                    if py >= r.bottom() - 3 {
                        break;
                    }
                    put(buf, preview_x, py, &[seg(l, c.fg(t.text))], r.right() - 2);
                    py += 1;
                }
            }
        }
    }
    let agent = app.hy_agent();
    put(
        buf,
        r.x + 3,
        r.bottom() - 2,
        &hints(t, &[("Enter", &format!("{agent} on it now")), ("Ctrl+Q", "queue it"), ("Tab", "source"), ("Ctrl+O", "open"), ("Ctrl+R", "reload"), ("Esc", "close")]),
        r.right(),
    );
}

/// The queue, in order: what's waiting, starting, running, ready for review or failed.
fn draw_queue(app: &mut App, buf: &mut Buffer, r: Rect, mut y: u16, t: &Theme, sel: usize) {
    let c = Style::default().bg(t.card);
    let queue = app.snap.queue.clone();
    if queue.is_empty() {
        put(buf, r.x + 3, y, &[seg("Nothing queued. Ctrl+Q on a ticket queues it; or type a task above.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
        return;
    }
    let max = (r.bottom() - 3 - y) as usize;
    let start = sel.saturating_sub(max.saturating_sub(1));
    for (i, q) in queue.iter().enumerate().skip(start).take(max) {
        let row = Rect { x: r.x + 1, y, width: r.width - 2, height: 1 };
        let bg = if i == sel || hovered(app, row) { t.hov } else { t.card };
        fill(buf, row, bg);
        let st = Style::default().bg(bg);
        if i == sel {
            put(buf, r.x + 1, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
        }
        let (label, col) = match &q.state {
            QueueState::Waiting => ("waiting".to_string(), t.muted),
            QueueState::Starting => ("starting…".to_string(), t.working),
            QueueState::Running(term) => match app.snap.terms.get(term).map(|x| x.status) {
                Some(Status::Blocked) => ("needs you".to_string(), t.blocked),
                _ => ("working".to_string(), t.working),
            },
            QueueState::Review(_) => ("review".to_string(), t.done),
            QueueState::Failed(e) => (format!("failed: {e}"), t.err),
        };
        let where_ = format!("  {} · {}", q.agent, q.branch);
        put(
            buf,
            r.x + 3,
            y,
            &[seg(format!("{:<11}", truncate(&label, 11)), st.fg(col).add_modifier(Modifier::BOLD)), seg(truncate(&q.title, 60), st.fg(t.strong)), seg(where_, st.fg(t.muted))],
            r.right() - 2,
        );
        if let QueueState::Failed(e) = &q.state {
            let _ = e;
        }
        hit(app, row, HyHit::TicketRow(i));
        y += 1;
    }
}

pub(super) fn draw_race_new(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &RaceNew) {
    let agents = app.race_agents();
    let proj = app.current_project();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 80, 13, "Race agents", &[], t);
    let c = Style::default().bg(t.card);
    put(buf, r.x + 3, r.y + 2, &[seg("TASK", c.fg(if v.row == 0 { t.accent } else { t.muted }).add_modifier(Modifier::BOLD))], r.right());
    input_row(buf, r, r.y + 3, t, &v.text, "what should they all do?", v.row == 0);
    put(buf, r.x + 3, r.y + 5, &[seg("AGENTS", c.fg(if v.row == 1 { t.accent } else { t.muted }).add_modifier(Modifier::BOLD))], r.right());
    let mut x = r.x + 12;
    for (i, a) in agents.iter().enumerate() {
        let on = v.picked.get(i).copied().unwrap_or(false);
        let label = format!("{} {a}", if on { "✓" } else { "·" });
        let w = label.width() as u16 + 2;
        let cr = Rect { x, y: r.y + 5, width: w, height: 1 };
        let mut ch = chip(t, &label, on, hovered(app, cr) || (v.row == 1 && v.cur == i));
        if v.row == 1 && v.cur == i && !on {
            ch.1 = ch.1.add_modifier(Modifier::UNDERLINED);
        }
        put(buf, x, r.y + 5, &[ch], r.right());
        hit(app, cr, HyHit::RaceAgent(i));
        x += w + 1;
    }
    let n = v.picked.iter().filter(|p| **p).count();
    let what = match proj {
        Some(p) if p.git => format!("{n} agents, each in its own worktree of {}. Then compare and keep the best.", p.name),
        Some(p) => format!("{} isn't a git repo; races need worktrees.", p.name),
        None => "Start a session first (o opens a folder).".into(),
    };
    put(buf, r.x + 3, r.y + 7, &[seg(what, c.fg(t.text))], r.right() - 2);
    let gx = btn(app, buf, r.x + 3, r.bottom() - 2, "Start the race", "Enter", BtnKind::Primary, HyHit::RaceGo, r.right());
    put(buf, gx + 3, r.bottom() - 2, &hints(t, &[("Tab", "task / agents"), ("Space", "pick"), ("Esc", "close")]), r.right());
}

pub(super) fn draw_race(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &RaceView) {
    let Some(race) = app.hy.saved.races.iter().find(|r| r.id == v.id).cloned() else { return };
    let model = app.hy_model();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 96, race.entries.len() as u16 + 11, &format!("Race · {}", truncate(&race.prompt, 60)), &[], t);
    let c = Style::default().bg(t.card);
    put(buf, r.x + 3, r.y + 2, &[seg(format!("from {} · same task, different agents", race.base), c.fg(t.muted))], r.right());
    for (i, (agent, branch)) in race.entries.iter().enumerate() {
        let y = r.y + 4 + i as u16;
        let bg = sel_row(app, buf, r, y, i == v.sel, t);
        let st = Style::default().bg(bg);
        let sess = model.iter().flat_map(|p| p.wts.iter()).find(|w| &w.branch == branch).and_then(|w| w.sessions.first().cloned());
        let (gl, state, col) = match &sess {
            Some(s) => (super::design::glyph(app, s.status), super::design::state_label(s.status), t.status(s.status)),
            None => ("○".into(), "not running", t.muted),
        };
        let stat = v.stats.get(i).cloned().flatten().unwrap_or_else(|| "…".into());
        put(
            buf,
            r.x + 3,
            y,
            &[
                seg(format!("{gl} "), st.fg(col).add_modifier(if sess.as_ref().is_some_and(|s| s.status == Status::Blocked) { Modifier::BOLD } else { Modifier::empty() })),
                seg(format!("{agent:<8}"), st.fg(t.strong).add_modifier(Modifier::BOLD)),
                seg(format!("{state:<10}"), st.fg(col)),
                seg(format!("  {branch}"), st.fg(t.muted)),
            ],
            r.right() - 26,
        );
        put(buf, r.right() - 25, y, &[seg(stat, st.fg(t.text))], r.right() - 2);
        hit(app, Rect { x: r.x + 1, y, width: r.width - 2, height: 1 }, HyHit::RaceRow(i));
    }
    let y = r.bottom() - 4;
    if v.confirm {
        let (agent, branch) = &race.entries[v.sel];
        put(
            buf,
            r.x + 3,
            y,
            &[seg(format!("Keep {agent}'s {branch} and delete the other {}?  ", race.entries.len() - 1), c.fg(t.blocked).add_modifier(Modifier::BOLD)), seg("Enter yes · Esc no", c.fg(t.muted))],
            r.right(),
        );
    }
    let mut x = r.x + 3;
    for (label, key, kind, ch) in [("Open", "Enter", BtnKind::Primary, '\n'), ("Changes", "d", BtnKind::Normal, 'd'), ("Keep this one", "k", BtnKind::Normal, 'k')] {
        x = btn(app, buf, x, r.bottom() - 2, label, key, kind, HyHit::RaceKey(ch), r.right()) + 1;
    }
    put(buf, x + 2, r.bottom() - 2, &hints(t, &[("↑↓", "choose"), ("Esc", "close")]), r.right());
    let _ = segs_width(&[]);
    let _ = find;
}

#[cfg(test)]
mod live {
    /// `HYDRA_PR_DIR=<clone> cargo test tickets_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn tickets_live() {
        use super::Source;
        let dir = std::path::PathBuf::from(std::env::var("HYDRA_PR_DIR").unwrap());
        let list = crate::tickets::GitHubIssues.list(&dir).unwrap();
        println!("{} open issues", list.len());
        for t in list.iter().take(3) {
            println!("{} {} -> branch {}", t.key, t.title, t.branch());
        }
    }
}
