//! Right-click menus for the hydra layout: a pane, an agent row, a project, a branch or a
//! worktree. Short, hoverable, and every item does something.

use super::design::{fill, path_key, put, seg, segs_width};
use super::hydra::{HyHit, find, hit, hovered};
use super::render::truncate;
use super::{App, Mode, PromptKind};
use crate::protocol::{ClientMsg, Command, TermId};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Focus(TermId),
    Beside(TermId),
    Talk(TermId),
    Rename(TermId),
    Interrupt(TermId),
    /// End these (stops what runs in them).
    End(Vec<TermId>),
    /// + New with this project picked (index), beside or not.
    New(usize, bool),
    ShellBeside,
    StartAgent(PathBuf),
    StartShell(PathBuf),
    Files(PathBuf),
    Changes(PathBuf),
    Ship(PathBuf),
    OpenFolder(PathBuf),
    Fold(usize),
    Forget(PathBuf),
    RemoveWorktree(PathBuf),
    CloseSplit,
    CopyMode(TermId),
    Duplicate(TermId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyMenu {
    pub title: String,
    pub items: Vec<(String, Act)>,
    pub sel: usize,
    pub at: (u16, u16),
}

impl App {
    fn menu(&mut self, title: String, items: Vec<(String, Act)>, at: (u16, u16)) {
        self.mode = Mode::HyMenu(Box::new(HyMenu { title, items, sel: 0, at }));
    }

    /// Right-click on a pane's content.
    pub(super) fn menu_for_pane(&mut self, term: TermId, at: (u16, u16)) {
        if Some(term) != self.focused() {
            self.cmd(Command::FocusPane { term });
        }
        let model = self.hy_model();
        let pi = find(&model, term).and_then(|(p, ..)| model.iter().position(|x| x.key == p.key)).unwrap_or(0);
        let dir = find(&model, term).map(|(_, w, _)| w.path.clone()).unwrap_or_else(|| self.here_dir());
        let agent = find(&model, term).map(|(_, _, s)| s.agent.clone()).unwrap_or_default();
        let mut items = vec![
            ("+ New beside…".to_string(), Act::New(pi, true)),
            ("Shell beside".to_string(), Act::ShellBeside),
            (format!("Message {agent}…"), Act::Talk(term)),
            ("Files".to_string(), Act::Files(dir.clone())),
            ("Changes".to_string(), Act::Changes(dir)),
            ("Select text (copy mode)".to_string(), Act::CopyMode(term)),
        ];
        if self.hy.pair.is_some_and(|(a, b)| a == term || b == term) {
            items.push(("Close the split".into(), Act::CloseSplit));
        }
        items.push((format!("End {agent}"), Act::End(vec![term])));
        self.menu(agent, items, at);
    }

    /// Right-click on an agent (or shell) row.
    pub(super) fn menu_for_session(&mut self, term: TermId, at: (u16, u16)) {
        let model = self.hy_model();
        let Some((_, _, s)) = find(&model, term) else { return };
        let agent = s.agent.clone();
        let mut items = vec![("Open".to_string(), Act::Focus(term))];
        if self.focused().is_some_and(|f| f != term) {
            items.push(("Open beside this one".into(), Act::Beside(term)));
        }
        items.extend([
            (format!("Message {agent}…"), Act::Talk(term)),
            ("Duplicate (same task, new worktree)…".to_string(), Act::Duplicate(term)),
            ("Rename…".to_string(), Act::Rename(term)),
            ("Interrupt (Ctrl+C)".to_string(), Act::Interrupt(term)),
            (format!("End {agent}"), Act::End(vec![term])),
        ]);
        self.menu(format!("{agent} · {}", truncate(&s.title, 30)), items, at);
    }

    /// Right-click on a project row.
    pub(super) fn menu_for_project(&mut self, pi: usize, at: (u16, u16)) {
        let model = self.hy_model();
        let Some(p) = model.get(pi) else { return };
        let all: Vec<TermId> = p.sessions().map(|s| s.term).collect();
        let open = !self.hy.saved.closed.contains(&format!("p:{}", p.key));
        let mut items = vec![
            ("+ New here…".to_string(), Act::New(pi, false)),
            ("Open the folder".to_string(), Act::OpenFolder(p.path.clone())),
            (if open { "Fold" } else { "Unfold" }.to_string(), Act::Fold(pi)),
        ];
        if !all.is_empty() {
            items.push((format!("End everything here ({})", all.len()), Act::End(all)));
        }
        if self.hy.saved.known.iter().any(|k| path_key(k) == p.key) {
            items.push(("Forget this project".into(), Act::Forget(p.path.clone())));
        }
        self.menu(p.name.clone(), items, at);
    }

    /// Right-click on a branch (the repo folder) or worktree row.
    pub(super) fn menu_for_place(&mut self, key: String, at: (u16, u16)) {
        let model = self.hy_model();
        let Some(w) = model.iter().flat_map(|p| p.wts.iter()).find(|w| w.key == key).cloned() else { return };
        let all: Vec<TermId> = w.sessions.iter().map(|s| s.term).collect();
        let agent = self.hy_agent();
        let mut items = vec![
            (format!("+ {agent} here"), Act::StartAgent(w.path.clone())),
            ("+ Shell here".to_string(), Act::StartShell(w.path.clone())),
            ("Files".to_string(), Act::Files(w.path.clone())),
            ("Changes".to_string(), Act::Changes(w.path.clone())),
            ("Ship (commit, push, PR)".to_string(), Act::Ship(w.path.clone())),
        ];
        if !all.is_empty() {
            items.push((format!("End everything here ({})", all.len()), Act::End(all)));
        }
        if !w.main {
            items.push(("Remove worktree (branch kept)".into(), Act::RemoveWorktree(w.path.clone())));
        }
        let title = if w.main { format!("⎇ {}", w.branch) } else { format!("⑂ {}", w.name) };
        self.menu(title, items, at);
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
            _ => self.mode = Mode::HyMenu(Box::new(m)),
        }
    }

    pub(super) fn menu_pick(&mut self, i: usize) {
        let Mode::HyMenu(m) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
        if let Some((_, a)) = m.items.get(i).cloned() {
            self.menu_act(a);
        }
    }

    fn menu_act(&mut self, a: Act) {
        match a {
            Act::Focus(t) => self.hy_focus(t),
            Act::Beside(t) => {
                if let Some(f) = self.focused() {
                    self.hy.pair = Some((f, t));
                }
            }
            Act::Talk(t) => self.hy_talk(t, false),
            Act::Duplicate(t) => self.hy_duplicate(t),
            Act::Rename(t) => {
                if let Some((w, _)) = self.snap.locate(t) {
                    self.mode = Mode::Prompt { kind: PromptKind::RenameWorkspace(w.id), input: w.name.clone() };
                }
            }
            Act::Interrupt(t) => self.send(ClientMsg::Input { term: t, data: vec![3] }),
            Act::End(ts) => {
                for t in ts {
                    self.cmd(Command::ClosePane { term: t });
                }
            }
            Act::New(pi, beside) => self.hy_new(pi, beside),
            Act::ShellBeside => {
                self.act(crate::keys::Action::SplitRight);
            }
            Act::StartAgent(p) => {
                let agent = self.hy_agent();
                self.hy_new_session(p, Some(agent), false);
            }
            Act::StartShell(p) => self.hy_new_session(p, None, false),
            Act::Files(p) => self.open_files(p),
            Act::Changes(p) => self.open_changes(p),
            Act::Ship(p) => self.ask_ship(p),
            Act::OpenFolder(p) => {
                let _ = super::files::open_default(&p);
            }
            Act::Fold(pi) => self.on_hy_hit(HyHit::ToggleProj(pi), false),
            Act::Forget(p) => self.hy_forget(&p),
            Act::RemoveWorktree(p) => {
                let ws = self
                    .snap
                    .terms
                    .values()
                    .find(|t| t.top.as_ref().is_some_and(|x| path_key(x) == path_key(&p)))
                    .and_then(|t| self.snap.locate(t.id))
                    .map(|(w, _)| w.id);
                match ws {
                    Some(ws) => self.cmd(Command::RemoveWorktree { ws, force: false, delete_branch: false }),
                    None => {
                        let dir = p.clone();
                        self.spawn_bg(move || {
                            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(["worktree", "remove", "."]).output();
                            let r = match out {
                                Ok(o) if o.status.success() => Ok(format!("removed worktree {}", dir.display())),
                                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("git failed").to_string()),
                                Err(e) => Err(e.to_string()),
                            };
                            super::Bg::Done(r, false)
                        });
                    }
                }
            }
            Act::CloseSplit => self.hy.pair = None,
            Act::CopyMode(t) => {
                self.enter_copy(t);
            }
        }
    }
}

pub(super) fn draw_menu(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, m: &HyMenu) {
    let buf = f.buffer_mut();
    let w = m.items.iter().map(|(l, _)| l.width() as u16).max().unwrap_or(10).max(m.title.width() as u16).clamp(18, 48) + 6;
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
        let danger = matches!(act, Act::End(_) | Act::RemoveWorktree(_) | Act::Forget(_));
        let fg = if danger { t.err } else if on { t.strong } else { t.text };
        let segs = vec![seg(if i == m.sel { "›" } else { " " }, Style::default().fg(t.accent).bg(bg)), seg(format!(" {label}"), Style::default().fg(fg).bg(bg))];
        let _ = segs_width(&segs);
        put(f.buffer_mut(), row.x, yy, &segs, row.right());
        hit(app, row, HyHit::MenuPick(i));
    }
}
