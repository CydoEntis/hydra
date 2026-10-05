//! Right-click menus for the hydra layout: a pane, an agent row, a project, a branch or a
//! worktree. Short, hoverable, and every item does something.

use super::design::{fill, put, seg};
use super::hydra::{HyHit, find, hit, hovered};
use super::render::truncate;
use super::{App, Mode};
use crate::protocol::{Command, TermId};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Beside(TermId),
    Talk(TermId),
    /// End these (stops what runs in them).
    End(Vec<TermId>),
    Changes(PathBuf),
    /// Preset i, for this agent.
    Preset(usize, Option<TermId>),
    RenamePane(TermId),
    Split(TermId, crate::layout::Dir),
    Zoom(TermId),
    RightClicks(TermId),
    RenameProject(String, String),
    /// End everything in a project and forget it.
    CloseProject(usize),
    NewWorktree(usize),
    /// git init and a first commit, so agents there can get worktrees.
    GitInit(PathBuf),
    OpenWorktree(usize),
    Dev(PathBuf, crate::protocol::DevAction),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Confirm {
    /// The title bar ("Close pane", "changes").
    pub title: String,
    /// Dim next to the title ("dsa-tool").
    pub sub: String,
    /// What it's about (bold) and a dim detail after it.
    pub what: String,
    pub detail: String,
    /// A plain line of explanation.
    pub note: String,
    /// The button that does it, and its key ("Close", "Enter").
    pub yes: String,
    pub key: char,
    /// Red (destructive) or the accent.
    pub danger: bool,
    pub act: Act,
}

/// A modal like everything else: what it's about, a line of explanation, the action
/// (red when it destroys something) and Cancel.
pub(super) fn draw_confirm(app: &mut App, f: &mut ratatui::Frame, area: ratatui::layout::Rect, t: &crate::theme::Theme, c: &Confirm) {
    use super::design::{put, seg};
    use super::hydra::{HyHit, dim_all, hit, hovered, panel};
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let w = (c.what.chars().count() + c.detail.chars().count() + 12).max(c.note.chars().count() + 8).clamp(58, 90) as u16;
    let r = panel(app, buf, area, w, 9, &c.title, &[], t);
    if !c.sub.is_empty() {
        put(buf, r.x + 3 + c.title.chars().count() as u16, r.y, &[seg(format!("  {}", c.sub), Style::default().bg(t.accent).fg(t.acc_ink))], r.right().saturating_sub(12));
    }
    let card = Style::default().bg(t.card);
    put(buf, r.x + 3, r.y + 2, &[seg(c.what.clone(), card.fg(t.strong).add_modifier(Modifier::BOLD)), seg(format!("   {}", c.detail), card.fg(t.muted))], r.right() - 2);
    put(buf, r.x + 3, r.y + 3, &[seg(c.note.clone(), card.fg(t.text))], r.right() - 2);
    let key = if c.key == '\n' { "Enter".to_string() } else { c.key.to_string() };
    let (yb, yf) = if c.danger { (t.danger_fill(), t.ink_on(t.danger_fill())) } else { (t.accent, t.acc_ink) };
    let yes = vec![seg(format!(" {} ", c.yes), Style::default().bg(yb).fg(yf).add_modifier(Modifier::BOLD)), seg(format!("{key} "), Style::default().bg(yb).fg(yf).add_modifier(Modifier::BOLD))];
    let yw: u16 = yes.iter().map(|(x, _)| x.chars().count() as u16).sum();
    let yr = Rect { x: r.x + 3, y: r.y + 6, width: yw, height: 1 };
    put(buf, yr.x, yr.y, &yes, r.right());
    let nr = Rect { x: yr.right() + 2, y: r.y + 6, width: 12, height: 1 };
    let nb = if hovered(app, nr) { t.hov } else { t.btn };
    put(buf, nr.x, nr.y, &[seg(" Cancel ", Style::default().bg(nb).fg(t.strong).add_modifier(Modifier::BOLD)), seg("Esc ", Style::default().bg(nb).fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
    hit(app, area, HyHit::ConfirmNo);
    hit(app, r, HyHit::Noop);
    hit(app, yr, HyHit::ConfirmYes);
    hit(app, nr, HyHit::ConfirmNo);
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyMenu {
    pub title: String,
    pub items: Vec<(String, Act)>,
    pub sel: usize,
    pub at: (u16, u16),
}

impl App {
    pub(super) fn menu(&mut self, title: String, mut items: Vec<(String, Act)>, at: (u16, u16)) {
        // The destructive item last.
        items.sort_by_key(|(_, a)| matches!(a, Act::End(_) | Act::CloseProject(_)));
        self.mode = Mode::HyMenu(Box::new(HyMenu { title, items, sel: 0, at }));
    }

    /// Right-click on a pane's content.
    pub(super) fn menu_for_pane(&mut self, term: TermId, at: (u16, u16)) {
        if Some(term) != self.focused() {
            self.cmd(Command::FocusPane { term });
        }
        let model = self.hy_model();
        let agent = find(&model, term).map(|(_, _, s)| s.agent.clone()).unwrap_or_default();
        let in_split = self.hy.tabs.get(self.hy.tab).is_some_and(|tab| tab.layout.contains(term) && tab.layout.leaves().len() > 1);
        let zoomed = self.hy.zoom == Some(term);
        let mut items = vec![
            ("Rename pane".to_string(), Act::RenamePane(term)),
            ("Split right".to_string(), Act::Split(term, crate::layout::Dir::Right)),
            ("Split down".to_string(), Act::Split(term, crate::layout::Dir::Down)),
        ];
        if in_split || zoomed {
            items.push((if zoomed { "Unzoom" } else { "Zoom" }.to_string(), Act::Zoom(term)));
        }
        items.push((
            if self.hy.right_clicks.contains(&term) { "Stop sending right-clicks to pane" } else { "Send right-clicks to pane" }.to_string(),
            Act::RightClicks(term),
        ));
        items.push(("Close pane".to_string(), Act::End(vec![term])));
        self.menu(if agent.is_empty() { "pane".into() } else { agent }, items, at);
    }

    /// Right-click on an agent (or shell) row.
    pub(super) fn menu_for_session(&mut self, term: TermId, at: (u16, u16)) {
        if let Some((title, items)) = self.session_items(term) {
            self.menu(title, items, at);
        }
    }

    /// A session's menu: its title and items, for right-click and for the sidebar's
    /// letter keys.
    pub(super) fn session_items(&self, term: TermId) -> Option<(String, Vec<(String, Act)>)> {
        let model = self.hy_model();
        let (_, _, s) = find(&model, term)?;
        let agent = s.agent.clone();
        let dir = find(&model, term).map(|(_, w, _)| w.path.clone());
        if let (Some(_), Some(d)) = (&s.dev, &dir) {
            use crate::protocol::DevAction;
            let items = vec![
                ("Restart".to_string(), Act::Dev(d.clone(), DevAction::Restart)),
                ("Stop".to_string(), Act::Dev(d.clone(), DevAction::Stop)),
            ];
            return Some(("dev server".into(), items));
        }
        let mut items = vec![(format!("Message {agent}…"), Act::Talk(term))];
        if self.focused().is_some_and(|f| f != term) {
            items.push(("Open beside".into(), Act::Beside(term)));
        }
        items.push(("Rename".to_string(), Act::RenamePane(term)));
        if let Some(d) = dir.filter(|d| crate::gitfs::head(d).is_some()) {
            items.push(("Changes".to_string(), Act::Changes(d)));
        }
        if let Some(d) = find(&model, term).filter(|(_, w, _)| !w.main && !w.sessions.iter().any(|x| x.dev.is_some())).map(|(_, w, _)| w.path.clone())
            && crate::project::load(&d).dev.is_some_and(|x| !x.run.trim().is_empty())
        {
            items.push(("▶ Run dev server here".to_string(), Act::Dev(d, crate::protocol::DevAction::Start)));
        }
        items.push(("Close".to_string(), Act::End(vec![term])));
        Some((format!("{} · {}", s.name, truncate(&agent, 30)), items))
    }

    /// Right-click on a project row.
    pub(super) fn menu_for_project(&mut self, pi: usize, at: (u16, u16)) {
        if let Some((title, items)) = self.project_items(pi) {
            self.menu(title, items, at);
        }
    }

    pub(super) fn project_items(&self, pi: usize) -> Option<(String, Vec<(String, Act)>)> {
        let model = self.hy_model();
        let p = model.get(pi)?;
        let mut items = vec![("Rename".to_string(), Act::RenameProject(p.key.clone(), p.name.clone())), ("Close".to_string(), Act::CloseProject(pi))];
        if p.git {
            items.push(("New worktree".to_string(), Act::NewWorktree(pi)));
            items.push(("Open worktree…".to_string(), Act::OpenWorktree(pi)));
        } else {
            items.push(("Make it a git repo…".to_string(), Act::GitInit(p.path.clone())));
        }
        if let Some(main) = p.wts.iter().find(|w| w.main)
            && crate::project::load(&main.path).dev.is_some_and(|d| !d.run.trim().is_empty())
            && !main.sessions.iter().any(|s| s.dev.is_some())
        {
            items.push(("▶ Run dev server".to_string(), Act::Dev(main.path.clone(), crate::protocol::DevAction::Start)));
        }
        Some((p.name.clone(), items))
    }

    /// The menu items for the sidebar cursor's row (what its letter keys do), in the
    /// menu's order.
    pub(super) fn cursor_items(&self) -> Vec<(String, Act)> {
        let mut items = match (&self.hy.cursor_proj, self.hy.cursor) {
            (Some(key), _) => self.hy_model().iter().position(|p| p.key == *key).and_then(|pi| self.project_items(pi)),
            (None, Some(t)) => self.session_items(t),
            _ => None,
        }
        .map(|(_, i)| i)
        .unwrap_or_default();
        items.sort_by_key(|(_, a)| matches!(a, Act::End(_) | Act::CloseProject(_)));
        items
    }

    pub(super) fn on_hy_menu_key(&mut self, mut m: HyMenu, k: &KeyEvent) {
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => {
                m.sel = (m.sel + 1) % m.items.len().max(1);
                self.mode = Mode::HyMenu(Box::new(m));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                m.sel = (m.sel + m.items.len().max(1) - 1) % m.items.len().max(1);
                self.mode = Mode::HyMenu(Box::new(m));
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                if let Some((_, a)) = m.items.get(m.sel).cloned() {
                    self.menu_act(a);
                }
            }
            // An item's key picks it.
            KeyCode::Char(c) if menu_keys(&m.items).contains(&Some(c.to_ascii_lowercase())) => {
                let i = menu_keys(&m.items).iter().position(|k| *k == Some(c.to_ascii_lowercase())).unwrap_or(0);
                self.mode = Mode::Normal;
                if let Some((_, a)) = m.items.get(i).cloned() {
                    self.menu_act(a);
                }
            }
            // A numbered list (presets): its number picks it.
            KeyCode::Char(c @ '1'..='9') if m.items.iter().any(|(l, _)| l.starts_with("1 ")) => {
                let i = c as usize - '1' as usize;
                if let Some((_, a)) = m.items.get(i).cloned() {
                    self.mode = Mode::Normal;
                    self.menu_act(a);
                } else {
                    self.mode = Mode::HyMenu(Box::new(m));
                }
            }
            _ => self.mode = Mode::HyMenu(Box::new(m)),
        }
    }

    pub(super) fn menu_pick(&mut self, i: usize) {
        let Mode::HyMenu(m) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
        if let Some((_, a)) = m.items.get(i).cloned() {
            self.menu_act(a);
        }
    }

    /// What a menu item does; closing things asks first.
    pub(super) fn menu_act(&mut self, a: Act) {
        let model = self.hy_model();
        let ask: Option<Confirm> = match &a {
            Act::End(ts) if ts.len() == 1 => {
                let name = find(&model, ts[0]).map(|(_, _, s)| s.name.clone()).unwrap_or_else(|| "this pane".into());
                let path = self.snap.terms.get(&ts[0]).map(|t| t.cwd.display().to_string()).unwrap_or_default();
                let running = self.snap.terms.get(&ts[0]).is_some_and(|t| t.status == crate::protocol::Status::Working);
                Some(Confirm {
                    title: "Close pane".into(),
                    sub: String::new(),
                    what: name,
                    detail: path,
                    note: if running { "It's still working; the program running in it will stop.".into() } else { "The program running in it will stop.".into() },
                    yes: "Close".into(),
                    key: '\n',
                    danger: true,
                    act: a.clone(),
                })
            }
            Act::End(ts) => Some(Confirm {
                title: "Close panes".into(),
                sub: String::new(),
                what: format!("{} panes", ts.len()),
                detail: String::new(),
                note: "The programs running in them will stop.".into(),
                yes: "Close".into(),
                key: '\n',
                danger: true,
                act: a.clone(),
            }),
            Act::GitInit(p) => Some(Confirm {
                title: "Make it a git repo".into(),
                sub: String::new(),
                what: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                detail: p.display().to_string(),
                note: "git init, then what's there as the first commit. Agents then get worktrees.".into(),
                yes: "Make it a git repo".into(),
                key: 'g',
                danger: false,
                act: a.clone(),
            }),
            Act::CloseProject(pi) => model.get(*pi).map(|p| {
                let n = p.sessions().count();
                Confirm {
                    title: "Close project".into(),
                    sub: String::new(),
                    what: p.name.clone(),
                    detail: format!("{n} pane{}", if n == 1 { "" } else { "s" }),
                    note: "Everything running in it stops; the folder stays where it is.".into(),
                    yes: "Close".into(),
                    key: '\n',
                    danger: true,
                    act: a.clone(),
                }
            }),
            _ => None,
        };
        match ask {
            Some(c) => {
                // Asked from the sidebar: come back to it afterwards, on the next row.
                self.hy.side_return = if self.mode == Mode::Side { self.side_after_close() } else { None };
                self.mode = Mode::Confirm(Box::new(c));
            }
            None => self.menu_do(a),
        }
    }

    pub(super) fn menu_do(&mut self, a: Act) {
        match a {
            Act::RenamePane(t) => {
                let cur = self.snap.terms.get(&t).map(|x| x.label.clone()).unwrap_or_default();
                self.mode = Mode::Prompt { kind: super::PromptKind::RenamePane(t), input: cur };
            }
            Act::Split(t, dir) => {
                let cwd = self.snap.terms.get(&t).map(|x| x.top.clone().unwrap_or_else(|| x.cwd.clone())).unwrap_or_else(|| self.here_dir());
                self.hy.split_dir = Some(dir);
                if self.focused() != Some(t) {
                    self.cmd(Command::FocusPane { term: t });
                }
                self.hy.pending_split = Some((t, std::time::Instant::now()));
                self.cmd(Command::NewWorkspace { cwd: Some(cwd), name: None, cmd: None });
            }
            Act::GitInit(dir) => {
                self.notify("making it a git repo…".into(), false);
                self.spawn_bg(move || {
                    let result = git_init(&dir);
                    super::Bg::Then(Box::new(move |app: &mut App| {
                        match result {
                            Ok(()) => app.notify("it's a git repo now; new agents get their own worktree".into(), false),
                            Err(e) => app.notify(e, true),
                        }
                        app.hy_fresh();
                    }))
                });
            }
            Act::Zoom(t) => self.hy.zoom = if self.hy.zoom == Some(t) { None } else { Some(t) },
            Act::RightClicks(t) => {
                if !self.hy.right_clicks.remove(&t) {
                    self.hy.right_clicks.insert(t);
                    self.notify("right-clicks go to this pane now (Shift+right-click for hydra's menu)".into(), false);
                }
            }
            Act::RenameProject(key, name) => self.mode = Mode::Prompt { kind: super::PromptKind::RenameProject(key), input: name },
            Act::CloseProject(pi) => {
                let model = self.hy_model();
                if let Some(p) = model.get(pi) {
                    for s in p.sessions() {
                        self.cmd(Command::ClosePane { term: s.term });
                    }
                }
            }
            Act::NewWorktree(pi) => {
                self.hy_new(pi, false);
                if let Mode::HyPane(np) = &mut self.mode {
                    np.place = Some(0);
                }
            }
            Act::OpenWorktree(pi) => {
                let model = self.hy_model();
                let Some(p) = model.get(pi) else { return };
                let key = super::design::path_key(&p.path);
                let ws = self.snap.workspaces.iter().find(|w| {
                    w.tabs.iter().flat_map(|t| t.layout.leaves()).any(|id| self.snap.terms.get(&id).and_then(|t| t.root.as_ref()).is_some_and(|r| super::design::path_key(r) == key))
                });
                match ws.map(|w| w.id) {
                    Some(ws) => {
                        self.send(crate::protocol::ClientMsg::Query(crate::protocol::Query::Worktrees { ws }));
                        self.mode = Mode::Worktrees { ws, cmd: None, items: None, query: String::new(), sel: 0 };
                    }
                    None => self.notify("start something in this project first".into(), true),
                }
            }
            Act::Beside(t) => {
                if let Some(f) = self.focused() {
                    self.hy_unshow(t);
                    self.hy.pending_split = Some((f, std::time::Instant::now()));
                    self.cmd(Command::FocusPane { term: t });
                }
            }
            Act::Talk(t) => self.hy_talk(t, false),
            Act::Preset(i, on) => self.hy_run_preset(i, on, None),
            Act::Dev(dir, action) => self.cmd(Command::Dev { dir, action }),
            Act::End(ts) => {
                for t in ts {
                    // Out of its split first: the pane beside it takes the room (and the
                    // focus), so nothing else slides into its place.
                    self.hy_unshow(t);
                    self.cmd(Command::ClosePane { term: t });
                }
            }
            Act::Changes(p) => self.open_changes(p),
        }
    }
}

/// What a new repo ignores when it has no .gitignore of its own, so its first commit
/// doesn't take in dependencies, build output or secrets.
const DEFAULT_GITIGNORE: &str = "node_modules/\ntarget/\ndist/\nbuild/\n.venv/\n__pycache__/\n.env\n.env.*\n*.log\n.DS_Store\n";

/// `git init`, a .gitignore when there isn't one, and a first commit (worktrees need one).
fn git_init(dir: &std::path::Path) -> Result<(), String> {
    let run = |args: &[&str]| {
        let mut c = std::process::Command::new("git");
        c.arg("-C").arg(dir).args(args);
        crate::proc::quiet(&mut c);
        match c.output() {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("git failed").to_string()),
            Err(e) => Err(format!("couldn't run git: {e}")),
        }
    };
    run(&["init", "-q"])?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, DEFAULT_GITIGNORE).map_err(|e| format!("couldn't write .gitignore: {e}"))?;
    }
    run(&["add", "-A"])?;
    run(&["commit", "-q", "-m", "Initial commit"]).map_err(|e| format!("the first commit failed ({e}); is git set up with your name and email?"))
}

/// A key for each item: the first letter of its label not taken yet (j and k move).
/// Numbered lists (presets) use their numbers.
pub(super) fn menu_keys(items: &[(String, Act)]) -> Vec<Option<char>> {
    let mut used: Vec<char> = vec!['j', 'k'];
    // Closing is always x.
    let closes = |a: &Act| matches!(a, Act::End(_) | Act::CloseProject(_));
    if items.iter().any(|(_, a)| closes(a)) {
        used.push('x');
    }
    items
        .iter()
        .map(|(l, a)| {
            if closes(a) {
                return Some('x');
            }
            if l.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return None;
            }
            let c = l.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).find(|c| !used.contains(c))?;
            used.push(c);
            Some(c)
        })
        .collect()
}

pub(super) fn draw_menu(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, m: &HyMenu) {
    let buf = f.buffer_mut();
    let keys = menu_keys(&m.items);
    let danger = |a: &Act| matches!(a, Act::End(_) | Act::CloseProject(_));
    // The destructive item sits last, after a blank row.
    let gap = m.items.iter().any(|(_, a)| danger(a)) && m.items.len() > 1;
    let w = m.items.iter().map(|(l, _)| l.width() as u16).max().unwrap_or(10).max(m.title.width() as u16).clamp(16, 44) + 10;
    let h = m.items.len() as u16 + 3 + gap as u16;
    let x = m.at.0.min(area.right().saturating_sub(w + 1)).max(area.x);
    let y = if m.at.1 + h < area.bottom() { m.at.1 + 1 } else { m.at.1.saturating_sub(h) }.max(area.y);
    let r = Rect { x, y, width: w, height: h + 1 };
    // Clicking anywhere else closes it.
    hit(app, area, HyHit::Close);
    // Its own surface and an outline, so it stands off whatever is under it.
    let bgm = t.card2;
    fill(buf, r, bgm);
    let edge = Style::default().fg(super::render::blend(t.line, t.text, 0.35)).bg(bgm);
    for xx in r.x..r.right() {
        if let Some(px) = buf.cell_mut((xx, r.y)) {
            px.set_symbol("─").set_style(edge);
        }
        if let Some(px) = buf.cell_mut((xx, r.bottom() - 1)) {
            px.set_symbol("─").set_style(edge);
        }
    }
    for yy in r.y..r.bottom() {
        if let Some(px) = buf.cell_mut((r.x, yy)) {
            px.set_symbol("│").set_style(edge);
        }
        if let Some(px) = buf.cell_mut((r.right() - 1, yy)) {
            px.set_symbol("│").set_style(edge);
        }
    }
    for (cx, cy, g) in [(r.x, r.y, "╭"), (r.right() - 1, r.y, "╮"), (r.x, r.bottom() - 1, "╰"), (r.right() - 1, r.bottom() - 1, "╯")] {
        if let Some(px) = buf.cell_mut((cx, cy)) {
            px.set_symbol(g).set_style(edge);
        }
    }
    hit(app, r, HyHit::Noop);
    put(buf, r.x + 2, r.y, &[seg(format!(" {} ", truncate(&m.title, (w - 6) as usize)), Style::default().fg(t.text).bg(bgm).add_modifier(Modifier::BOLD))], r.right() - 2);
    let mut yy = r.y + 2;
    for (i, (label, act)) in m.items.iter().enumerate() {
        if gap && danger(act) && i + 1 == m.items.len() {
            yy += 1;
        }
        let row = Rect { x: r.x + 1, y: yy, width: w - 2, height: 1 };
        let on = i == m.sel || hovered(app, row);
        let bg = if on { t.hov } else { bgm };
        fill(f.buffer_mut(), row, bg);
        if i == m.sel {
            put(f.buffer_mut(), row.x + 1, yy, &[seg("›", Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], row.right());
        }
        let mut ls = Style::default().fg(if danger(act) { t.err } else { t.strong }).bg(bg);
        if i == m.sel {
            ls = ls.add_modifier(Modifier::BOLD);
        }
        put(f.buffer_mut(), row.x + 3, yy, &[seg(label.clone(), ls)], row.right().saturating_sub(4));
        if let Some(k) = keys.get(i).copied().flatten() {
            put(f.buffer_mut(), row.right().saturating_sub(3), yy, &[seg(k.to_string(), Style::default().fg(t.accent).bg(bg).add_modifier(Modifier::BOLD))], row.right());
        }
        hit(app, row, HyHit::MenuPick(i));
        yy += 1;
    }
}
