//! Right-click menus for the hydra layout: a pane, an agent row, a project, a branch or a
//! worktree. Short, hoverable, and every item does something.

use super::design::{fill, put, seg, segs_width};
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
    pub title: String,
    pub detail: String,
    pub act: Act,
}

/// "Close pane?" — what, and confirm / cancel (like herdr's).
pub(super) fn draw_confirm(app: &mut App, f: &mut ratatui::Frame, area: ratatui::layout::Rect, t: &crate::theme::Theme, c: &Confirm) {
    use super::design::{fill, put, seg};
    use super::hydra::{HyHit, dim_all, hit, hovered};
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let w = 60.min(area.width.saturating_sub(4));
    let r = Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: area.y + area.height / 3, width: w, height: 6 };
    fill(buf, r, t.card);
    let edge = Style::default().fg(t.blocked).bg(t.card);
    for x in r.left()..r.right() {
        buf[(x, r.y)].set_symbol("─").set_style(edge);
        buf[(x, r.bottom() - 1)].set_symbol("─").set_style(edge);
    }
    for y in r.top()..r.bottom() {
        buf[(r.x, y)].set_symbol("│").set_style(edge);
        buf[(r.right() - 1, y)].set_symbol("│").set_style(edge);
    }
    for (x, y, g) in [(r.x, r.y, "┌"), (r.right() - 1, r.y, "┐"), (r.x, r.bottom() - 1, "└"), (r.right() - 1, r.bottom() - 1, "┘")] {
        buf[(x, y)].set_symbol(g).set_style(edge);
    }
    let c0 = Style::default().bg(t.card);
    put(buf, r.x + 2, r.y + 1, &[seg(c.title.clone(), c0.fg(t.blocked).add_modifier(Modifier::BOLD))], r.right() - 2);
    put(buf, r.x + 2, r.y + 2, &[seg(super::render::truncate(&c.detail, (w - 4) as usize), c0.fg(t.text))], r.right() - 2);
    let yes = " ↵ confirm ";
    let no = " esc cancel ";
    let total = (yes.chars().count() + 2 + no.chars().count()) as u16;
    let bx = r.x + (w.saturating_sub(total)) / 2;
    let yr = Rect { x: bx, y: r.y + 4, width: yes.chars().count() as u16, height: 1 };
    let nr = Rect { x: yr.right() + 2, y: r.y + 4, width: no.chars().count() as u16, height: 1 };
    let ys = Style::default().bg(t.blocked).fg(t.bg).add_modifier(Modifier::BOLD);
    let ns = if hovered(app, nr) { Style::default().bg(t.hov).fg(t.strong) } else { Style::default().bg(t.btn).fg(t.text) };
    put(buf, yr.x, yr.y, &[seg(yes, ys)], r.right());
    put(buf, nr.x, nr.y, &[seg(no, ns)], r.right());
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
    pub(super) fn menu(&mut self, title: String, items: Vec<(String, Act)>, at: (u16, u16)) {
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
        let model = self.hy_model();
        let Some((_, _, s)) = find(&model, term) else { return };
        let agent = s.agent.clone();
        let dir = find(&model, term).map(|(_, w, _)| w.path.clone());
        if let (Some(_), Some(d)) = (&s.dev, &dir) {
            use crate::protocol::DevAction;
            let items = vec![
                ("Restart".to_string(), Act::Dev(d.clone(), DevAction::Restart)),
                ("Stop".to_string(), Act::Dev(d.clone(), DevAction::Stop)),
            ];
            self.menu("dev server".into(), items, at);
            return;
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
        self.menu(format!("{agent} · {}", truncate(&s.title, 30)), items, at);
    }

    /// Right-click on a project row.
    pub(super) fn menu_for_project(&mut self, pi: usize, at: (u16, u16)) {
        let model = self.hy_model();
        let Some(p) = model.get(pi) else { return };
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
        self.menu(p.name.clone(), items, at);
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
        let ask = match &a {
            Act::End(ts) if ts.len() == 1 => {
                let name = find(&model, ts[0]).map(|(_, _, s)| if s.title == super::hydra::WAITING || s.title.is_empty() { s.agent.clone() } else { format!("{} · {}", s.agent, s.title) }).unwrap_or_else(|| "this pane".into());
                let running = self.snap.terms.get(&ts[0]).is_some_and(|t| t.status == crate::protocol::Status::Working);
                Some(("Close pane?".to_string(), format!("{name}{}", if running { " — still working" } else { "" })))
            }
            Act::End(ts) => Some(("Close all of these?".to_string(), format!("{} panes", ts.len()))),
            Act::GitInit(p) => Some((
                format!("Make {} a git repo?", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                "git init, then everything there as the first commit; new agents get their own worktrees".to_string(),
            )),
            Act::CloseProject(pi) => model.get(*pi).map(|p| {
                let n = p.sessions().count();
                ("Close project?".to_string(), format!("{} — {n} pane{}", p.name, if n == 1 { "" } else { "s" }))
            }),
            _ => None,
        };
        match ask {
            Some((title, detail)) => self.mode = Mode::Confirm(Box::new(Confirm { title, detail, act: a })),
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
                let run = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output();
                let ok = run(&["init", "-q"]).is_ok_and(|o| o.status.success())
                    && run(&["add", "-A"]).is_ok_and(|o| o.status.success())
                    && run(&["commit", "-q", "-m", "Initial commit"]).is_ok_and(|o| o.status.success());
                if ok {
                    self.notify("it's a git repo now; new agents get their own worktree".into(), false);
                } else {
                    self.notify("git init or the first commit failed (is git set up with your name and email?)".into(), true);
                }
                self.hy_fresh();
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
                    let path = p.path.clone();
                    self.hy_forget(&path);
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
                    self.cmd(Command::ClosePane { term: t });
                }
            }
            Act::Changes(p) => self.open_changes(p),
        }
    }
}

/// A key for each item: the first letter of its label not taken yet (j and k move).
/// Numbered lists (presets) use their numbers.
pub(super) fn menu_keys(items: &[(String, Act)]) -> Vec<Option<char>> {
    let mut used: Vec<char> = vec!['j', 'k'];
    items
        .iter()
        .map(|(l, _)| {
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
    let w = m.items.iter().map(|(l, _)| l.width() as u16).max().unwrap_or(10).max(m.title.width() as u16).clamp(18, 48) + 10;
    let h = m.items.len() as u16 + 3;
    let x = m.at.0.min(area.right().saturating_sub(w + 1)).max(area.x);
    let y = if m.at.1 + h < area.bottom() { m.at.1 + 1 } else { m.at.1.saturating_sub(h) }.max(area.y);
    let r = Rect { x, y, width: w, height: h };
    // Clicking anywhere else closes it.
    hit(app, area, HyHit::Close);
    fill(buf, r, t.card);
    for xx in r.x..r.right() {
        buf[(xx, r.y)].set_symbol("─").set_style(Style::default().fg(t.line).bg(t.card));
        buf[(xx, r.bottom() - 1)].set_symbol("─").set_style(Style::default().fg(t.line).bg(t.card));
    }
    for yy in r.y..r.bottom() {
        buf[(r.x, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.card));
        buf[(r.right() - 1, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.card));
    }
    buf[(r.x, r.y)].set_symbol("╭");
    buf[(r.right() - 1, r.y)].set_symbol("╮");
    buf[(r.x, r.bottom() - 1)].set_symbol("╰");
    buf[(r.right() - 1, r.bottom() - 1)].set_symbol("╯");
    hit(app, r, HyHit::Noop);
    put(buf, r.x + 2, r.y, &[seg(format!(" {} ", truncate(&m.title, (w - 6) as usize)), Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::BOLD))], r.right() - 1);
    for (i, (label, act)) in m.items.iter().enumerate() {
        let yy = r.y + 1 + i as u16;
        let row = Rect { x: r.x + 1, y: yy, width: w - 2, height: 1 };
        let on = i == m.sel || hovered(app, row);
        let bg = if on { t.hov } else { t.card };
        fill(f.buffer_mut(), row, bg);
        let danger = matches!(act, Act::End(_) | Act::CloseProject(_));
        let fg = if danger { t.err } else if on { t.strong } else { t.text };
        let segs = vec![seg(if i == m.sel { "›" } else { " " }, Style::default().fg(t.accent).bg(bg)), seg(format!(" {label}"), Style::default().fg(fg).bg(bg))];
        put(f.buffer_mut(), row.x, yy, &segs, row.right().saturating_sub(4));
        if let Some(k) = keys.get(i).copied().flatten() {
            let cap = vec![seg(format!(" {k} "), Style::default().fg(t.accent).bg(t.btn).add_modifier(Modifier::BOLD))];
            let cw = segs_width(&cap);
            put(f.buffer_mut(), row.right().saturating_sub(cw + 1), yy, &cap, row.right());
        }
        hit(app, row, HyHit::MenuPick(i));
    }
}
