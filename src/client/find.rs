//! Find a file (fuzzy) or search the code (`git grep`), then put the path in the agent's prompt
//! or open it in your editor at the line.

use super::design::{fill, put, seg, tilde};
use super::hydra::{HyHit, dim_all, hints, hit, hovered, panel};
use super::render::truncate;
use super::{App, Bg, Mode, files};
use crate::protocol::ClientMsg;
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub struct GrepHit {
    /// Relative to the search folder.
    pub path: String,
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindView {
    pub dir: PathBuf,
    /// 0: files, 1: search the code.
    pub tab: u8,
    pub query: String,
    pub sel: usize,
    /// Every file (relative paths), once listed.
    pub files: Option<Vec<String>>,
    /// The latest search's results; `seq` drops answers to older queries.
    pub hits: Option<Result<Vec<GrepHit>, String>>,
    pub seq: u64,
    pub preview: Vec<String>,
}

/// What the selection points at: (relative path, line).
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub path: String,
    pub line: Option<u32>,
}

impl FindView {
    pub fn new(dir: PathBuf, tab: u8) -> FindView {
        FindView { dir, tab, query: String::new(), sel: 0, files: None, hits: None, seq: 0, preview: Vec::new() }
    }

    /// The files tab's matches, best first.
    pub fn file_matches(&self) -> Vec<&str> {
        let Some(list) = &self.files else { return Vec::new() };
        if self.query.trim().is_empty() {
            return list.iter().take(300).map(|s| s.as_str()).collect();
        }
        let mut scored: Vec<(i32, &str)> = list.iter().filter_map(|p| files::fuzzy(&self.query, p).map(|s| (s, p.as_str()))).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        scored.into_iter().take(300).map(|(_, p)| p).collect()
    }

    pub fn targets(&self) -> Vec<Target> {
        if self.tab == 0 {
            self.file_matches().into_iter().map(|p| Target { path: p.to_string(), line: None }).collect()
        } else {
            match &self.hits {
                Some(Ok(h)) => h.iter().map(|h| Target { path: h.path.clone(), line: Some(h.line) }).collect(),
                _ => Vec::new(),
            }
        }
    }

    pub fn selected(&self) -> Option<Target> {
        self.targets().into_iter().nth(self.sel)
    }

    /// The selected file's first lines (around the hit), for the preview.
    pub fn refresh_preview(&mut self) {
        self.preview.clear();
        let Some(t) = self.selected() else { return };
        let Ok(text) = std::fs::read_to_string(self.dir.join(&t.path)) else { return };
        let from = t.line.map(|l| (l as usize).saturating_sub(6)).unwrap_or(0);
        self.preview = text.lines().skip(from).take(60).enumerate().map(|(i, l)| format!("{:>5}  {}", from + i + 1, l.replace('\t', "    "))).collect();
    }
}

fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Every file under `dir` (respecting .gitignore), relative, up to 50k.
pub fn list_files(dir: &Path) -> Vec<String> {
    ignore::WalkBuilder::new(dir)
        .hidden(true)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .take(50_000)
        .filter_map(|e| e.path().strip_prefix(dir).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
        .collect()
}

/// `git grep` for `q` (case-insensitive, fixed string) under `dir`, up to 500 hits.
pub fn grep(dir: &Path, q: &str) -> Result<Vec<GrepHit>, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(["grep", "-n", "-I", "-i", "-F", "--no-color", "--untracked", "-e", q]);
    let out = quiet(&mut cmd).output().map_err(|_| "`git` isn't installed or isn't on PATH".to_string())?;
    // git grep exits 1 when nothing matches.
    if !out.status.success() && out.status.code() != Some(1) {
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("git grep failed").to_string());
    }
    Ok(parse_grep(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_grep(out: &str) -> Vec<GrepHit> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, ':');
            let path = it.next()?.to_string();
            let line = it.next()?.parse().ok()?;
            let text = it.next()?.trim().chars().take(300).collect();
            Some(GrepHit { path, line, text })
        })
        .take(500)
        .collect()
}

impl App {
    pub(super) fn open_find(&mut self, tab: u8) {
        let dir = self.find_dir();
        let v = FindView::new(dir.clone(), tab);
        self.mode = Mode::Find(Box::new(v));
        self.spawn_bg(move || Bg::FindFiles(dir.clone(), list_files(&dir)));
    }

    /// The folder to search: the focused agent's worktree (or the cursor's), else here.
    fn find_dir(&self) -> PathBuf {
        self.hy
            .cursor
            .or(self.focused())
            .and_then(|t| self.snap.terms.get(&t))
            .map(|t| t.top.clone().unwrap_or_else(|| t.cwd.clone()))
            .unwrap_or_else(|| self.here_dir())
    }

    fn find_search(&mut self, v: &mut FindView) {
        v.seq += 1;
        v.hits = None;
        if v.query.trim().len() < 2 {
            v.hits = Some(Ok(Vec::new()));
            return;
        }
        let (dir, q, g) = (v.dir.clone(), v.query.clone(), v.seq);
        self.spawn_bg(move || Bg::Grep(dir.clone(), g, grep(&dir, &q)));
    }

    pub(super) fn on_find_key(&mut self, mut v: FindView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = v.targets().len();
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                v.tab = 1 - v.tab;
                v.sel = 0;
                if v.tab == 1 {
                    self.find_search(&mut v);
                }
            }
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::PageDown => v.sel = (v.sel + 15).min(n.saturating_sub(1)),
            KeyCode::PageUp => v.sel = v.sel.saturating_sub(15),
            KeyCode::Enter => {
                // The path (and line) into the agent's prompt, ready to keep typing.
                if let (Some(t), Some(term)) = (v.selected(), self.focused()) {
                    let text = match t.line {
                        Some(l) => format!("{}:{l} ", t.path),
                        None => format!("{} ", t.path),
                    };
                    self.send(ClientMsg::Input { term, data: text.into_bytes() });
                    self.notify(format!("put {} in the prompt", t.path), false);
                }
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('e') if ctrl => {
                if let Some(t) = v.selected() {
                    let p = v.dir.join(&t.path);
                    self.mode = Mode::Normal;
                    self.open_in_editor_at(&p, t.line);
                    return;
                }
            }
            KeyCode::Char('y') if ctrl => {
                if let Some(t) = v.selected() {
                    super::copy::to_clipboard(&t.path);
                    self.notify(format!("copied {}", t.path), false);
                }
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.sel = 0;
                if v.tab == 1 {
                    self.find_search(&mut v);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.sel = 0;
                if v.tab == 1 {
                    self.find_search(&mut v);
                }
            }
            _ => {}
        }
        v.refresh_preview();
        self.mode = Mode::Find(Box::new(v));
    }
}

pub(super) fn draw_find(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &FindView) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 140, 36, &format!("Find · {}", tilde(&v.dir)), &[], t);
    let c = Style::default().bg(t.card);
    // Tabs
    let mut x = r.x + 2;
    for (i, label) in ["Files", "Search code"].iter().enumerate() {
        let on = i as u8 == v.tab;
        let txt = format!(" {label} ");
        let tr = Rect { x, y: r.y + 2, width: txt.width() as u16, height: 1 };
        let st = if on {
            Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
        } else if hovered(app, tr) {
            Style::default().bg(t.hov).fg(t.strong)
        } else {
            Style::default().bg(t.btn).fg(t.text)
        };
        put(buf, x, r.y + 2, &[seg(txt.clone(), st)], r.right());
        hit(app, tr, HyHit::FindTab(i as u8));
        x += tr.width + 2;
    }
    // Query
    let qrow = Rect { x: r.x + 1, y: r.y + 4, width: r.width - 2, height: 1 };
    fill(buf, qrow, t.card2);
    let s = Style::default().bg(t.card2);
    let ph = if v.tab == 0 { " type part of a file name" } else { " type text to search for (2+ characters)" };
    let mut q = vec![seg("› ", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(v.query.clone(), s.fg(t.strong)), seg("█", s.fg(t.accent))];
    if v.query.is_empty() {
        q.push(seg(ph, s.fg(t.muted)));
    }
    put(buf, r.x + 3, qrow.y, &q, r.right() - 2);
    // List (left) and preview (right)
    let list = Rect { x: r.x + 1, y: r.y + 6, width: r.width * 45 / 100, height: r.height.saturating_sub(9) };
    let prev = Rect { x: list.right() + 2, y: list.y, width: r.right().saturating_sub(list.right() + 4), height: list.height };
    let targets = v.targets();
    let status = match (v.tab, &v.files, &v.hits) {
        (0, None, _) => Some("listing files…".to_string()),
        (1, _, None) => Some("searching…".to_string()),
        (1, _, Some(Err(e))) => Some(e.clone()),
        _ if targets.is_empty() => Some("nothing matches".to_string()),
        _ => None,
    };
    if let Some(msg) = status {
        put(buf, list.x + 2, list.y, &[seg(msg, c.fg(t.muted).add_modifier(Modifier::ITALIC))], list.right());
    }
    let start = v.sel.saturating_sub(list.height.saturating_sub(1) as usize);
    let hits: Vec<GrepHit> = match &v.hits {
        Some(Ok(h)) if v.tab == 1 => h.clone(),
        _ => Vec::new(),
    };
    for (i, tg) in targets.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let row = Rect { y, height: 1, ..list };
        let on = i == v.sel;
        let bg = if on || hovered(app, row) { t.hov } else { t.card };
        fill(buf, row, bg);
        let st = Style::default().bg(bg);
        if on {
            put(buf, row.x, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], row.right());
        }
        let segs = match tg.line {
            None => {
                let (dir, name) = tg.path.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n.to_string())).unwrap_or((String::new(), tg.path.clone()));
                vec![seg(name, st.fg(t.strong).add_modifier(if on { Modifier::BOLD } else { Modifier::empty() })), seg(format!("  {dir}"), st.fg(t.muted))]
            }
            Some(l) => {
                let text = hits.get(i).map(|h| h.text.clone()).unwrap_or_default();
                vec![seg(format!("{}:{l}  ", truncate(&tg.path, 34)), st.fg(t.accent)), seg(text, st.fg(t.text))]
            }
        };
        put(buf, row.x + 2, y, &segs, row.right());
        hit(app, row, HyHit::FindRow(i));
    }
    for xx in [list.right() + 1] {
        for yy in list.top()..list.bottom() {
            buf[(xx, yy)].set_symbol("│").set_style(Style::default().fg(t.line).bg(t.card));
        }
    }
    let hl = v.selected().and_then(|tg| tg.line);
    for (i, l) in v.preview.iter().take(prev.height as usize).enumerate() {
        let is_hit = hl.is_some_and(|h| l.trim_start().starts_with(&format!("{h} ")));
        let st = if is_hit { Style::default().bg(t.card2).fg(t.strong) } else { c.fg(t.text) };
        put(buf, prev.x, prev.y + i as u16, &[seg(truncate(l, prev.width as usize), st)], prev.right());
    }
    let n = targets.len();
    put(
        buf,
        r.x + 3,
        r.bottom() - 2,
        &hints(t, &[("Enter", "into the prompt"), ("Ctrl+E", "editor"), ("Ctrl+Y", "copy path"), ("Tab", "files / search"), ("Esc", "close")]),
        r.right() - 12,
    );
    put(buf, r.right().saturating_sub(12), r.bottom() - 2, &[seg(format!("{n} found"), c.fg(t.muted))], r.right() - 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_output_and_fuzzy_files() {
        let h = parse_grep("src/main.rs:12:fn main() {\nREADME.md:3:Hydra: many heads\n");
        assert_eq!(h[0], GrepHit { path: "src/main.rs".into(), line: 12, text: "fn main() {".into() });
        assert_eq!(h[1].text, "Hydra: many heads");
        let mut v = FindView::new(PathBuf::from("."), 0);
        v.files = Some(vec!["src/client/hydra.rs".into(), "src/main.rs".into(), "README.md".into()]);
        v.query = "hyd".into();
        assert_eq!(v.file_matches(), vec!["src/client/hydra.rs"]);
    }
}
