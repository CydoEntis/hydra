//! Looking back: a folder's checkpoints (roll it back to after any agent turn) and your past
//! agent chats (search them, pick one up again).

use super::design::{fill, put, seg, tilde};
use super::hydra::{HyHit, age, dim_all, folder_name, hints, hit, hovered, panel};
use super::render::truncate;
use super::{App, Bg, Mode};
use crate::checkpoint::Checkpoint;
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::path::PathBuf;

/// How many chat files a search reads, newest first.
const CHATS_SEARCHED: usize = 600;
/// How many chats a list shows.
const CHATS_SHOWN: usize = 80;
/// How much of the start of a chat file is read for its folder and first message.
const HEAD_READ: u64 = 256 * 1024;
/// Text either side of a match in a chat's snippet.
const SNIPPET_AROUND: usize = 60;

#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointsView {
    /// The checkout they're for.
    pub top: PathBuf,
    pub list: Option<Result<Vec<Checkpoint>, String>>,
    pub sel: usize,
}

/// A past agent conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatHit {
    /// "claude" or "codex".
    pub agent: String,
    /// Its session id (what resuming it takes).
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    /// The text around the match; empty with no search.
    pub snippet: String,
    /// When it was last written (unix seconds).
    pub at: u64,
}

impl ChatHit {
    /// The command that picks the conversation up again.
    pub fn resume_cmd(&self) -> String {
        match self.agent.as_str() {
            "codex" => format!("codex resume {}", self.id),
            _ => format!("claude --resume {}", self.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatsView {
    pub query: String,
    /// What the list is for (the query as it was searched).
    pub searched: String,
    pub list: Option<Result<Vec<ChatHit>, String>>,
    pub sel: usize,
}

impl App {
    /// The checkpoints of the checkout the focused session is in.
    pub(super) fn open_checkpoints(&mut self) {
        let top = self.focused().and_then(|t| self.snap.terms.get(&t)).and_then(|t| t.top.clone());
        let Some(top) = top else {
            self.notify("checkpoints are for sessions in a git repo".into(), true);
            return;
        };
        self.mode = Mode::Checkpoints(Box::new(CheckpointsView { top: top.clone(), list: None, sel: 0 }));
        self.spawn_bg(move || Bg::Checkpoints(top.clone(), crate::checkpoint::list(&top).map_err(|e| format!("{e:#}"))));
    }

    pub(super) fn on_checkpoints_key(&mut self, mut v: CheckpointsView, k: &KeyEvent) {
        let n = v.list.as_ref().and_then(|l| l.as_ref().ok()).map(Vec::len).unwrap_or(0);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(cp) = v.list.as_ref().and_then(|l| l.as_ref().ok()).and_then(|l| l.get(v.sel)) {
                    self.mode = Mode::Confirm(Box::new(super::menu::Confirm {
                        title: "Go back".into(),
                        sub: folder_name(&v.top),
                        what: truncate(&cp.what, 50),
                        detail: format!("{} ago", age(cp.at)),
                        note: "Its files go back to how they were then. What's there now is kept as a checkpoint.".into(),
                        list: Vec::new(),
                        yes: "Go back".into(),
                        key: '\n',
                        danger: true,
                        act: super::menu::Act::Checkpoint(v.top.clone(), cp.commit.clone()),
                    }));
                    return;
                }
            }
            _ => {}
        }
        self.mode = Mode::Checkpoints(Box::new(v));
    }

    /// Roll a checkout back to a checkpoint (in the background).
    pub(super) fn restore_checkpoint(&mut self, top: PathBuf, commit: String) {
        self.notify("going back…".into(), false);
        self.spawn_bg(move || Bg::Done(crate::checkpoint::restore(&top, &commit).map_err(|e| format!("{e:#}")), false));
    }

    /// Your recent agent chats; type to search them all.
    pub(super) fn open_chats(&mut self) {
        self.mode = Mode::Chats(Box::new(ChatsView { query: String::new(), searched: String::new(), list: None, sel: 0 }));
        self.search_chats(String::new());
    }

    fn search_chats(&self, query: String) {
        self.spawn_bg(move || {
            let hits = search_chats(&chat_files(), &query, CHATS_SHOWN);
            Bg::Chats(query, Ok(hits))
        });
    }

    pub(super) fn on_chats_key(&mut self, mut v: ChatsView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = v.list.as_ref().and_then(|l| l.as_ref().ok()).map(Vec::len).unwrap_or(0);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            // A new search on Enter (reading every chat on each key would be slow); Enter on
            // the results picks the conversation up again.
            KeyCode::Enter if v.query != v.searched => {
                v.list = None;
                v.sel = 0;
                v.searched = v.query.clone();
                self.search_chats(v.query.clone());
            }
            KeyCode::Enter => {
                if let Some(c) = v.list.as_ref().and_then(|l| l.as_ref().ok()).and_then(|l| l.get(v.sel)).cloned() {
                    if !c.cwd.is_dir() {
                        self.notify(format!("{} isn't there any more", c.cwd.display()), true);
                    } else {
                        self.mode = Mode::Normal;
                        self.notify(format!("picking up {} again", truncate(&c.title, 40)), false);
                        self.hy_new_session(c.cwd.clone(), Some(c.resume_cmd()), false);
                        return;
                    }
                }
            }
            KeyCode::Backspace => {
                v.query.pop();
            }
            KeyCode::Char('u') if ctrl => v.query.clear(),
            KeyCode::Char(c) if !ctrl => v.query.push(c),
            _ => {}
        }
        self.mode = Mode::Chats(Box::new(v));
    }
}

/// Claude's and Codex's conversation files, newest first, with which agent wrote each.
pub fn chat_files() -> Vec<(String, PathBuf, u64)> {
    let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_default();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude")).join("projects");
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex")).join("sessions");
    let mut out = Vec::new();
    // Claude: projects/<folder>/<session>.jsonl (its subagents' files sit deeper).
    for d in std::fs::read_dir(&claude).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            push_chat(&mut out, "claude", f.path());
        }
    }
    // Codex: sessions/YYYY/MM/DD/rollout-….jsonl
    let mut stack = vec![(codex, 0)];
    while let Some((dir, depth)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if depth < 3 && e.path().is_dir() {
                stack.push((e.path(), depth + 1));
            } else {
                push_chat(&mut out, "codex", e.path());
            }
        }
    }
    out.sort_by_key(|(_, _, at)| std::cmp::Reverse(*at));
    out
}

fn push_chat(out: &mut Vec<(String, PathBuf, u64)>, agent: &str, p: PathBuf) {
    if p.extension().is_none_or(|e| e != "jsonl") {
        return;
    }
    let at = p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    out.push((agent.to_string(), p, at));
}

/// The chats among `files` that mention `query` (all of them, with no query), up to `limit`.
pub fn search_chats(files: &[(String, PathBuf, u64)], query: &str, limit: usize) -> Vec<ChatHit> {
    let q = query.trim();
    let find = (!q.is_empty()).then(|| regex::bytes::RegexBuilder::new(&regex::escape(q)).case_insensitive(true).build().ok()).flatten();
    let mut out = Vec::new();
    for (agent, path, at) in files.iter().take(CHATS_SEARCHED) {
        let snippet = match &find {
            None => String::new(),
            Some(re) => {
                let Ok(bytes) = std::fs::read(path) else { continue };
                match said_match(&bytes, re) {
                    Some(s) => s,
                    None => continue,
                }
            }
        };
        let Some(mut hit) = chat_facts_of(agent, path) else { continue };
        hit.at = *at;
        hit.snippet = snippet;
        out.push(hit);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// The first match in something you or the agent said, as a snippet.
fn said_match(bytes: &[u8], re: &regex::bytes::Regex) -> Option<String> {
    re.find_iter(bytes).find_map(|m| {
        let start = bytes[..m.start()].iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
        let end = bytes[m.end()..].iter().position(|b| *b == b'\n').map(|i| m.end() + i).unwrap_or(bytes.len());
        let line = String::from_utf8_lossy(&bytes[start..end]);
        if !is_said(&line) {
            return None;
        }
        let at = m.start() - start;
        let from = at.saturating_sub(SNIPPET_AROUND);
        let from = (0..=from).rev().find(|i| line.is_char_boundary(*i)).unwrap_or(0);
        let text: String = line[from..].chars().take(SNIPPET_AROUND * 2 + (m.end() - m.start())).collect();
        // JSON escapes, made readable.
        let clean = text.replace("\\n", " ").replace("\\\"", "\"").replace("\\t", " ");
        Some(clean.split_whitespace().collect::<Vec<_>>().join(" "))
    })
}

/// A chat file's facts from its start (and, for Claude, its name from the end): big files
/// aren't read whole.
fn chat_facts_of(agent: &str, path: &std::path::Path) -> Option<ChatHit> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(HEAD_READ).read_to_end(&mut head).ok()?;
    let mut hit = chat_facts(agent, &String::from_utf8_lossy(&head))?;
    if agent == "claude"
        && let Some(name) = crate::cli::transcript_facts(path).1
    {
        hit.title = name;
    }
    Some(hit)
}

/// A line of what you or the agent said (not tool plumbing or metadata).
fn is_said(line: &str) -> bool {
    line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"") || line.contains("\"user_message\"") || line.contains("\"agent_message\"")
}

/// Who, where and what a chat file is: its session id, folder and a title (its name, else
/// the first thing you asked).
pub fn chat_facts(agent: &str, text: &str) -> Option<ChatHit> {
    let mut id = String::new();
    let mut cwd = None;
    let mut first = String::new();
    for line in text.lines().take(400) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if agent == "codex" {
            if v["type"] == "session_meta" {
                id = v["payload"]["id"].as_str().unwrap_or_default().to_string();
                cwd = v["payload"]["cwd"].as_str().map(PathBuf::from);
            } else if first.is_empty() && v["payload"]["type"] == "user_message" {
                first = v["payload"]["message"].as_str().unwrap_or_default().to_string();
            }
        } else {
            if id.is_empty() {
                id = v["sessionId"].as_str().unwrap_or_default().to_string();
            }
            if cwd.is_none() {
                cwd = v["cwd"].as_str().map(PathBuf::from);
            }
            if first.is_empty() && v["type"] == "user" && v["isMeta"] != true {
                first = match &v["message"]["content"] {
                    serde_json::Value::String(s) => s.clone(),
                    c => c.as_array().into_iter().flatten().find(|b| b["type"] == "text").and_then(|b| b["text"].as_str()).unwrap_or_default().to_string(),
                };
                // Commands and their output aren't what it's about.
                if first.starts_with('<') {
                    first.clear();
                }
            }
        }
        if !id.is_empty() && cwd.is_some() && !first.is_empty() && agent == "codex" {
            break;
        }
    }
    let name = if agent == "claude" { crate::cli::transcript_facts_in(text).1 } else { None };
    let title = name.unwrap_or(first).split_whitespace().collect::<Vec<_>>().join(" ");
    if id.is_empty() {
        return None;
    }
    Some(ChatHit { agent: agent.to_string(), id, cwd: cwd?, title: if title.is_empty() { "(no messages)".into() } else { title }, snippet: String::new(), at: 0 })
}

pub(super) fn draw_checkpoints(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &CheckpointsView) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 100, 26, &format!("Checkpoints · {}", tilde(&v.top)), &[], t);
    let c = Style::default().bg(t.card);
    put(buf, r.x + 3, r.y + 2, &[seg("Saved after each agent turn. Pick one to put the folder back to how it was then.", c.fg(t.muted))], r.right() - 2);
    let mut y = r.y + 4;
    match &v.list {
        None => {
            put(buf, r.x + 3, y, &[seg("loading…", c.fg(t.muted))], r.right());
        }
        Some(Err(e)) => {
            put(buf, r.x + 3, y, &[seg(e.clone(), c.fg(t.err))], r.right() - 2);
        }
        Some(Ok(list)) if list.is_empty() => {
            put(buf, r.x + 3, y, &[seg("None yet: one is saved when an agent here finishes a turn and something changed.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
        }
        Some(Ok(list)) => {
            let max = (r.bottom() - 3 - y) as usize;
            let start = v.sel.saturating_sub(max.saturating_sub(1));
            for (i, cp) in list.iter().enumerate().skip(start).take(max) {
                let row = Rect { x: r.x + 1, y, width: r.width - 2, height: 1 };
                let bg = if i == v.sel || hovered(app, row) { t.hov } else { t.card };
                fill(buf, row, bg);
                let st = Style::default().bg(bg);
                if i == v.sel {
                    put(buf, r.x + 1, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
                }
                put(
                    buf,
                    r.x + 3,
                    y,
                    &[seg(format!("{:>4} ago  ", age(cp.at)), st.fg(t.muted)), seg(truncate(&cp.what, 58), st.fg(t.strong)), seg(format!("  {}", cp.change), st.fg(t.muted))],
                    r.right() - 2,
                );
                hit(app, row, HyHit::HistoryRow(i));
                y += 1;
            }
        }
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", "go back to it"), ("↑↓", "pick"), ("Esc", "close")]), r.right());
}

pub(super) fn draw_chats(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &ChatsView) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 120, 32, "Past chats", &[], t);
    let c = Style::default().bg(t.card);
    super::work::input_row(buf, r, r.y + 2, t, &v.query, "search every Claude and Codex chat (Enter)", true);
    let mut y = r.y + 4;
    match &v.list {
        None => {
            put(buf, r.x + 3, y, &[seg("searching…", c.fg(t.muted))], r.right());
        }
        Some(Err(e)) => {
            put(buf, r.x + 3, y, &[seg(e.clone(), c.fg(t.err))], r.right() - 2);
        }
        Some(Ok(list)) if list.is_empty() => {
            put(buf, r.x + 3, y, &[seg("No chats match.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
        }
        Some(Ok(list)) => {
            let per = if v.searched.is_empty() { 1 } else { 2 };
            let max = ((r.bottom() - 3 - y) as usize) / per;
            let start = v.sel.saturating_sub(max.saturating_sub(1));
            for (i, ch) in list.iter().enumerate().skip(start).take(max) {
                let row = Rect { x: r.x + 1, y, width: r.width - 2, height: per as u16 };
                let bg = if i == v.sel || hovered(app, row) { t.hov } else { t.card };
                fill(buf, row, bg);
                let st = Style::default().bg(bg);
                if i == v.sel {
                    put(buf, r.x + 1, y, &[seg(">", st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
                }
                put(
                    buf,
                    r.x + 3,
                    y,
                    &[
                        seg(format!("{:>4} ago  ", age(ch.at)), st.fg(t.muted)),
                        seg(format!("{:<7}", ch.agent), st.fg(t.accent)),
                        seg(format!("{:<18}", truncate(&folder_name(&ch.cwd), 17)), st.fg(t.text)),
                        seg(truncate(&ch.title, 70), st.fg(t.strong)),
                    ],
                    r.right() - 2,
                );
                if per == 2 {
                    put(buf, r.x + 15, y + 1, &[seg(truncate(&ch.snippet, (r.width - 18) as usize), st.fg(t.muted))], r.right() - 2);
                }
                hit(app, row, HyHit::HistoryRow(i));
                y += per as u16;
            }
        }
    }
    let enter = if v.query != v.searched { "search" } else { "pick it up again" };
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("Enter", enter), ("↑↓", "pick"), ("Esc", "close")]), r.right());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_chats_are_found_by_what_was_said() {
        let dir = std::env::temp_dir().join(format!("seshi-chats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("c1.jsonl");
        std::fs::write(
            &claude,
            r#"{"type":"user","sessionId":"abc-123","cwd":"C:\\code\\shop","message":{"role":"user","content":"Fix the flaky checkout test"}}
{"type":"assistant","sessionId":"abc-123","message":{"content":[{"type":"text","text":"The race is in the payment webhook handler."}]}}
"#,
        )
        .unwrap();
        let codex = dir.join("rollout-x.jsonl");
        std::fs::write(
            &codex,
            r#"{"type":"session_meta","payload":{"id":"019-xyz","cwd":"/home/me/api"}}
{"type":"event_msg","payload":{"type":"user_message","message":"add rate limiting to login"}}
"#,
        )
        .unwrap();
        let files = vec![("claude".to_string(), claude, 20), ("codex".to_string(), codex, 10)];
        let all = search_chats(&files, "", 10);
        assert_eq!(all.iter().map(|h| (h.agent.as_str(), h.id.as_str(), h.title.as_str())).collect::<Vec<_>>(), [("claude", "abc-123", "Fix the flaky checkout test"), ("codex", "019-xyz", "add rate limiting to login")]);
        let found = search_chats(&files, "WEBHOOK", 10);
        assert_eq!(found.len(), 1, "by what the agent said too, any case");
        assert!(found[0].snippet.contains("payment webhook handler"), "{}", found[0].snippet);
        assert_eq!(found[0].resume_cmd(), "claude --resume abc-123");
        assert_eq!(search_chats(&files, "rate limiting", 10)[0].resume_cmd(), "codex resume 019-xyz");
        assert!(search_chats(&files, "nowhere", 10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live {
    /// `cargo test chats_live -- --ignored --nocapture`: search your own chats.
    #[test]
    #[ignore]
    fn chats_live() {
        let files = super::chat_files();
        let t = std::time::Instant::now();
        let hits = super::search_chats(&files, &std::env::var("SESHI_CHAT_QUERY").unwrap_or_default(), 10);
        println!("{} files, {} hits in {:?}", files.len(), hits.len(), t.elapsed());
        for h in hits {
            println!("{} {} {} | {} | {}", h.agent, h.id, h.cwd.display(), h.title.chars().take(60).collect::<String>(), h.snippet.chars().take(80).collect::<String>());
        }
    }
}
