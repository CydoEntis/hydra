//! Switch the branch a checkout is on: pick one (local or remote, newest first) or type a new
//! name, and say what happens to uncommitted changes.

use super::design::{fill, put, seg, tilde};
use super::hydra::{HyHit, dim_all, hints, hit, hovered, panel};
use super::render::truncate;
use super::{App, Bg, Mode, files};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::path::{Path, PathBuf};
use crate::proc::git;

#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// As `git switch` takes it: `main`, or `origin/feature` for one only on the remote.
    pub name: String,
    pub when: String,
    pub remote: bool,
    /// Checked out in another worktree (git won't switch to it here).
    pub elsewhere: Option<PathBuf>,
}

/// What to do with uncommitted changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Carry {
    Bring,
    Stash,
    Commit,
    Discard,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Pick,
    /// There are uncommitted changes: what to do with them (the choice under the cursor).
    Changes(usize),
    /// Commit first: the message being typed.
    Message(String),
    /// Discarding: press y to really do it.
    ConfirmDiscard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BranchView {
    pub dir: PathBuf,
    pub current: String,
    pub query: String,
    pub sel: usize,
    pub list: Option<Vec<Branch>>,
    /// Files with uncommitted changes.
    pub dirty: usize,
    /// Agents and shells running in this checkout.
    pub running: usize,
    pub step: Step,
    /// The branch being switched to (or created).
    pub target: Option<(String, bool)>,
}

/// One row in the list: an existing branch, or "create <query>".
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Create(String),
    Existing(Branch),
}

impl BranchView {
    pub fn rows(&self) -> Vec<Row> {
        let Some(list) = &self.list else { return Vec::new() };
        let q = self.query.trim();
        let mut out: Vec<Row> = if q.is_empty() {
            list.iter().cloned().map(Row::Existing).collect()
        } else {
            let mut scored: Vec<(i32, &Branch)> = list.iter().filter_map(|b| files::fuzzy(q, &b.name).map(|s| (s, b))).collect();
            scored.sort_by_key(|x| std::cmp::Reverse(x.0));
            scored.into_iter().map(|(_, b)| Row::Existing(b.clone())).collect()
        };
        let exact = list.iter().any(|b| b.name == q || b.name.split_once('/').is_some_and(|(_, n)| b.remote && n == q));
        // Making a new one comes first only when nothing matches what you typed.
        if !q.is_empty() && !exact && valid_name(q) {
            out.push(Row::Create(q.to_string()));
        }
        out
    }
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && !n.contains(char::is_whitespace) && !n.contains("..") && !n.starts_with('-') && !n.ends_with('/') && !n.contains(['~', '^', ':', '?', '*', '[', '\\'])
}

/// Branches newest first (local, then remote ones with no local branch), which are checked
/// out elsewhere, and how many files have uncommitted changes.
pub fn load(dir: &Path) -> (String, Vec<Branch>, usize) {
    let current = git(dir, &["branch", "--show-current"]).unwrap_or_default().trim().to_string();
    let mut elsewhere = std::collections::HashMap::new();
    let mut wt: Option<PathBuf> = None;
    for l in git(dir, &["worktree", "list", "--porcelain"]).unwrap_or_default().lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            wt = Some(PathBuf::from(p));
        } else if let (Some(b), Some(p)) = (l.strip_prefix("branch refs/heads/"), &wt) {
            elsewhere.insert(b.to_string(), p.clone());
        }
    }
    let here = dunce(dir);
    let refs = git(dir, &["for-each-ref", "--sort=-committerdate", "--format=%(refname)\t%(committerdate:relative)", "refs/heads", "refs/remotes"]).unwrap_or_default();
    let mut list: Vec<Branch> = Vec::new();
    let mut locals = std::collections::HashSet::new();
    for l in refs.lines() {
        let Some((r, when)) = l.split_once('\t') else { continue };
        if let Some(n) = r.strip_prefix("refs/heads/") {
            locals.insert(n.to_string());
            let other = elsewhere.get(n).filter(|p| dunce(p) != here).cloned();
            list.push(Branch { name: n.into(), when: when.into(), remote: false, elsewhere: other });
        }
    }
    for l in refs.lines() {
        let Some((r, when)) = l.split_once('\t') else { continue };
        if let Some(n) = r.strip_prefix("refs/remotes/")
            && !n.ends_with("/HEAD")
            && n.split_once('/').is_some_and(|(_, short)| !locals.contains(short))
        {
            list.push(Branch { name: n.into(), when: when.into(), remote: true, elsewhere: None });
        }
    }
    let dirty = git(dir, &["status", "--porcelain"]).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).count();
    (current, list, dirty)
}

fn dunce(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Do it: deal with the changes, then switch (or create and switch).
pub fn switch(dir: &Path, name: &str, create: bool, carry: Option<Carry>, message: &str) -> Result<String, String> {
    let mut note = String::new();
    match carry {
        Some(Carry::Stash) => {
            git(dir, &["stash", "push", "-u", "-m", &format!("seshi: before switching to {name}")])?;
            note = " (your changes are stashed: `git stash pop` brings them back)".into();
        }
        Some(Carry::Commit) => {
            git(dir, &["add", "-A"])?;
            git(dir, &["commit", "-m", message])?;
            note = " (your changes were committed first)".into();
        }
        Some(Carry::Discard) => {
            git(dir, &["reset", "--hard"])?;
            git(dir, &["clean", "-fd"])?;
            note = " (uncommitted changes were thrown away)".into();
        }
        Some(Carry::Bring) => note = " (bringing your changes along)".into(),
        None => {}
    }
    let r = if create {
        git(dir, &["switch", "-c", name])
    } else if let Some((_, short)) = name.split_once('/').filter(|_| git(dir, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{name}")]).is_ok()) {
        // A remote branch: a local one that tracks it.
        git(dir, &["switch", "--track", "-c", short, name]).map(|_| String::new()).or_else(|_| git(dir, &["switch", short]))
    } else {
        git(dir, &["switch", name])
    };
    match r {
        Ok(_) => Ok(format!("now on {}{note}", name.split_once('/').filter(|_| !create && name.contains('/')).map(|(_, s)| s).unwrap_or(name))),
        Err(e) if carry == Some(Carry::Bring) => Err(format!("{e} — your changes clash with that branch; stash or commit them instead")),
        Err(e) => Err(e),
    }
}

const CHOICES: [(&str, &str, Carry); 4] = [
    ("Bring them along", "they move onto the new branch, if they don't clash", Carry::Bring),
    ("Stash them", "put away; `git stash pop` brings them back", Carry::Stash),
    ("Commit them first", "on the branch you're leaving", Carry::Commit),
    ("Discard them", "thrown away for good", Carry::Discard),
];

impl App {
    /// Ctrl+Space g: switch the branch of the checkout you're in.
    pub(super) fn open_branches(&mut self, dir: Option<PathBuf>) {
        let term = self.hy.cursor.or(self.focused()).and_then(|t| self.snap.terms.get(&t));
        let Some(dir) = dir.or_else(|| term.and_then(|t| t.top.clone())) else {
            self.notify("not in a git repo".into(), true);
            return;
        };
        let key = |p: &Path| dunce(p);
        let running = self.snap.terms.values().filter(|t| t.top.as_deref().is_some_and(|p| key(p) == key(&dir))).count();
        let v = BranchView { dir: dir.clone(), current: String::new(), query: String::new(), sel: 0, list: None, dirty: 0, running, step: Step::Pick, target: None };
        self.mode = Mode::Branch(Box::new(v));
        self.spawn_bg(move || {
            let (current, list, dirty) = load(&dir);
            Bg::Branches(dir.clone(), current, list, dirty)
        });
    }

    fn branch_go(&mut self, v: BranchView, carry: Option<Carry>, message: String) {
        let Some((name, create)) = v.target.clone() else { return };
        let dir = v.dir.clone();
        self.mode = Mode::Normal;
        self.notify(format!("switching to {name}…"), false);
        self.spawn_bg(move || Bg::Switched(switch(&dir, &name, create, carry, &message)));
    }

    pub(super) fn on_branch_key(&mut self, mut v: BranchView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match v.step.clone() {
            Step::Pick => {
                let rows = v.rows();
                match k.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Down => v.sel = (v.sel + 1).min(rows.len().saturating_sub(1)),
                    KeyCode::Up => v.sel = v.sel.saturating_sub(1),
                    KeyCode::PageDown => v.sel = (v.sel + 12).min(rows.len().saturating_sub(1)),
                    KeyCode::PageUp => v.sel = v.sel.saturating_sub(12),
                    KeyCode::Enter => match rows.get(v.sel) {
                        Some(Row::Existing(b)) if b.name == v.current => {
                            self.notify(format!("already on {}", b.name), false);
                        }
                        Some(Row::Existing(b)) if b.elsewhere.is_some() => {
                            let p = b.elsewhere.as_ref().map(|p| tilde(p)).unwrap_or_default();
                            self.notify(format!("{} is open in another worktree ({p}); open that instead", b.name), true);
                        }
                        Some(row) => {
                            v.target = Some(match row {
                                Row::Create(n) => (n.clone(), true),
                                Row::Existing(b) => (b.name.clone(), false),
                            });
                            if v.dirty == 0 {
                                return self.branch_go(v, None, String::new());
                            }
                            v.step = Step::Changes(0);
                        }
                        None => {}
                    },
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
            }
            Step::Changes(i) => match k.code {
                KeyCode::Esc => v.step = Step::Pick,
                KeyCode::Down => v.step = Step::Changes((i + 1) % 4),
                KeyCode::Up => v.step = Step::Changes((i + 3) % 4),
                KeyCode::Char(c @ '1'..='4') => return self.branch_choose(v, c as usize - '1' as usize),
                KeyCode::Enter => return self.branch_choose(v, i),
                _ => {}
            },
            Step::Message(mut m) => match k.code {
                KeyCode::Esc => v.step = Step::Changes(2),
                KeyCode::Enter if !m.trim().is_empty() => return self.branch_go(v, Some(Carry::Commit), m),
                KeyCode::Backspace => {
                    m.pop();
                    v.step = Step::Message(m);
                }
                KeyCode::Char(c) if !ctrl => {
                    m.push(c);
                    v.step = Step::Message(m);
                }
                _ => {}
            },
            Step::ConfirmDiscard => match k.code {
                KeyCode::Char('y') => return self.branch_go(v, Some(Carry::Discard), String::new()),
                _ => v.step = Step::Changes(3),
            },
        }
        self.mode = Mode::Branch(Box::new(v));
    }

    pub(super) fn branch_choose(&mut self, mut v: BranchView, i: usize) {
        match CHOICES.get(i).map(|c| c.2) {
            Some(Carry::Commit) => {
                v.step = Step::Message(format!("WIP before switching to {}", v.target.as_ref().map(|t| t.0.as_str()).unwrap_or("")));
                self.mode = Mode::Branch(Box::new(v));
            }
            Some(Carry::Discard) => {
                v.step = Step::ConfirmDiscard;
                self.mode = Mode::Branch(Box::new(v));
            }
            Some(c) => self.branch_go(v, Some(c), String::new()),
            None => self.mode = Mode::Branch(Box::new(v)),
        }
    }
}

pub(super) fn draw_branches(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &BranchView) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let title = if v.current.is_empty() { format!("Switch branch · {}", tilde(&v.dir)) } else { format!("Switch branch · {} · on {}", tilde(&v.dir), v.current) };
    let r = panel(app, buf, area, 96, 30, &title, &[], t);
    let c = Style::default().bg(t.card);
    let muted = c.fg(t.muted);
    // Warnings first: what else this touches.
    let mut y = r.y + 2;
    if v.running > 1 {
        put(buf, r.x + 3, y, &[seg(format!("{} sessions run in this folder; they'll all be on the new branch.", v.running), c.fg(t.blocked))], r.right() - 2);
        y += 1;
    }
    if v.dirty > 0 && v.step == Step::Pick {
        put(buf, r.x + 3, y, &[seg(format!("{} file(s) with uncommitted changes; you'll choose what happens to them.", v.dirty), muted)], r.right() - 2);
        y += 1;
    }
    y += 1;
    match &v.step {
        Step::Pick => {
            let q = Rect { x: r.x + 1, y, width: r.width - 2, height: 1 };
            fill(buf, q, t.card2);
            let s = Style::default().bg(t.card2);
            let mut segs = vec![seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(v.query.clone(), s.fg(t.strong)), seg("█", s.fg(t.accent))];
            if v.query.is_empty() {
                segs.push(seg(" type to filter, or a new branch name", s.fg(t.muted)));
            }
            put(buf, r.x + 3, y, &segs, r.right() - 2);
            let list = Rect { x: r.x + 1, y: y + 2, width: r.width - 2, height: r.bottom().saturating_sub(y + 5) };
            let rows = v.rows();
            if v.list.is_none() {
                put(buf, list.x + 2, list.y, &[seg("reading branches…", muted.add_modifier(Modifier::ITALIC))], list.right());
            }
            let start = v.sel.saturating_sub(list.height.saturating_sub(1) as usize);
            for (i, row) in rows.iter().enumerate().skip(start).take(list.height as usize) {
                let yy = list.y + (i - start) as u16;
                let rr = Rect { y: yy, height: 1, ..list };
                let on = i == v.sel;
                let bg = if on || hovered(app, rr) { t.hov } else { t.card };
                fill(buf, rr, bg);
                let st = Style::default().bg(bg);
                if on {
                    put(buf, rr.x, yy, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], rr.right());
                }
                let segs = match row {
                    Row::Create(n) => vec![seg("+ new branch ", st.fg(t.accent).add_modifier(Modifier::BOLD)), seg(n.clone(), st.fg(t.strong).add_modifier(Modifier::BOLD)), seg("  from here", st.fg(t.muted))],
                    Row::Existing(b) => {
                        let mut s = vec![seg(truncate(&b.name, 50), st.fg(if b.elsewhere.is_some() { t.muted } else { t.strong }).add_modifier(if on { Modifier::BOLD } else { Modifier::empty() }))];
                        if b.name == v.current {
                            s.push(seg("  ● you're here", st.fg(t.done)));
                        } else if let Some(p) = &b.elsewhere {
                            s.push(seg(format!("  open in {}", truncate(&files_name(p), 24)), st.fg(t.muted)));
                        } else if b.remote {
                            s.push(seg("  remote", st.fg(t.muted)));
                        }
                        s
                    }
                };
                put(buf, rr.x + 2, yy, &segs, rr.right().saturating_sub(16));
                if let Row::Existing(b) = row {
                    put(buf, rr.right().saturating_sub(15), yy, &[seg(truncate(&b.when, 14), st.fg(t.muted))], rr.right());
                }
                hit(app, rr, HyHit::BranchRow(i));
            }
            put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", "switch"), ("↑↓", "choose"), ("Esc", "close")]), r.right() - 1);
        }
        Step::Changes(sel) => {
            let to = v.target.as_ref().map(|t| t.0.clone()).unwrap_or_default();
            put(buf, r.x + 3, y, &[seg(format!("{} file(s) have uncommitted changes. Before switching to ", v.dirty), c.fg(t.text)), seg(to, c.fg(t.strong).add_modifier(Modifier::BOLD)), seg(":", c.fg(t.text))], r.right() - 2);
            for (i, (label, what, carry)) in CHOICES.iter().enumerate() {
                let yy = y + 2 + i as u16 * 2;
                let rr = Rect { x: r.x + 1, y: yy, width: r.width - 2, height: 1 };
                let on = i == *sel;
                let bg = if on || hovered(app, rr) { t.hov } else { t.card };
                fill(buf, rr, bg);
                let st = Style::default().bg(bg);
                let lc = if *carry == Carry::Discard { t.blocked } else { t.strong };
                put(buf, rr.x + 2, yy, &[seg(format!(" {} ", i + 1), Style::default().bg(t.btn).fg(t.accent).add_modifier(Modifier::BOLD)), seg(format!("  {label}"), st.fg(lc).add_modifier(Modifier::BOLD)), seg(format!("  {what}"), st.fg(t.muted))], rr.right());
                hit(app, rr, HyHit::BranchChoice(i));
            }
            put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("1-4", "choose"), ("Esc", "back")]), r.right() - 1);
        }
        Step::Message(m) => {
            put(buf, r.x + 3, y, &[seg("Commit message:", c.fg(t.text))], r.right() - 2);
            let q = Rect { x: r.x + 1, y: y + 2, width: r.width - 2, height: 1 };
            fill(buf, q, t.card2);
            let s = Style::default().bg(t.card2);
            put(buf, r.x + 3, y + 2, &[seg(m.clone(), s.fg(t.strong)), seg("█", s.fg(t.accent))], r.right() - 2);
            put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", "commit and switch"), ("Esc", "back")]), r.right() - 1);
        }
        Step::ConfirmDiscard => {
            put(buf, r.x + 3, y, &[seg(format!("Throw away the changes in {} file(s)? This can't be undone.", v.dirty), c.fg(t.blocked).add_modifier(Modifier::BOLD))], r.right() - 2);
            put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("y", "yes, discard them"), ("any other key", "back")]), r.right() - 1);
        }
    }
}

fn files_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success(), "git {args:?}");
    }

    #[test]
    fn switching_with_changes_stashed_or_brought_along() {
        let dir = std::env::temp_dir().join(format!("seshi-branch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q", "-b", "main"]);
        run(&dir, &["config", "user.email", "t@t"]);
        run(&dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "1").unwrap();
        run(&dir, &["add", "-A"]);
        run(&dir, &["commit", "-qm", "one"]);
        run(&dir, &["branch", "feature"]);
        std::fs::write(dir.join("a.txt"), "2").unwrap();
        let (current, list, dirty) = load(&dir);
        assert_eq!((current.as_str(), dirty), ("main", 1));
        assert!(list.iter().any(|b| b.name == "feature"));
        let v = BranchView { dir: dir.clone(), current, query: "new-thing".into(), sel: 0, list: Some(list), dirty, running: 0, step: Step::Pick, target: None };
        assert_eq!(v.rows()[0], Row::Create("new-thing".into()), "a name that doesn't exist offers to create it");
        let v2 = BranchView { query: "feat".into(), ..v.clone() };
        assert!(matches!(&v2.rows()[0], Row::Existing(b) if b.name == "feature"), "a match comes before making a new one");
        assert_eq!(v2.rows().last(), Some(&Row::Create("feat".into())));
        assert!(switch(&dir, "feature", false, Some(Carry::Stash), "").unwrap().contains("stashed"));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "1", "changes put away");
        run(&dir, &["stash", "pop", "-q"]);
        switch(&dir, "new-thing", true, Some(Carry::Bring), "").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "2", "changes came along");
        assert_eq!(load(&dir).0, "new-thing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
