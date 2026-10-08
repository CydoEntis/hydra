//! What agents use: context and cost (Claude reports them through its status line, Codex
//! writes them to its session files), plan limits, and saying "continue" once a limit
//! lifts.

use super::*;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// How often Codex's session files are read.
const CODEX_EVERY: Duration = Duration::from_secs(30);
/// How much of the end of a Codex session file is read for its latest numbers.
const CODEX_TAIL: u64 = 512 * 1024;
/// A limit with no known reset time: try again after this long (seconds).
const RETRY_LIMIT_AFTER: u64 = 30 * 60;
/// "continue" goes this long after the reset, so the limit has really lifted (seconds).
const AFTER_RESET: u64 = 60;
/// After saying "continue", the old limit line may still be on screen this long.
const CONTINUE_SETTLES: Duration = Duration::from_secs(120);
/// The rows at the bottom of an agent's screen where its limit line shows (above its
/// input box and footer).
const LIMIT_ROWS: u16 = 14;
/// A limit this used up (percent) is the one that stopped it.
const SPENT: f32 = 95.0;
/// How far back "today" reaches for what sessions cost (seconds).
const DAY: u64 = 24 * 60 * 60;

/// The line an agent shows when a plan limit stops it.
static LIMIT_HIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(usage limit reached|you'?ve hit your (usage |session |weekly )?limit|you have hit your (usage )?limit|(5-hour|weekly|session|opus) limit reached|out of extra usage)").unwrap()
});

impl Daemon {
    /// Claude's status line said what this pane's session has used.
    pub(super) fn report_usage(&mut self, term: TermId, usage: Usage, limits: Vec<Limit>) {
        let Some(t) = self.terms.get_mut(&term) else { return };
        if let Some(cost) = usage.cost {
            let before = t.usage.cost.unwrap_or(0.0);
            // Lower than before: a new session (/clear) counting from zero.
            let added = if cost >= before { cost - before } else { cost };
            if added > 0.0 {
                let now = term::unix_now();
                self.spent.retain(|(at, _)| now.saturating_sub(*at) < DAY);
                self.spent.push((now, added));
                self.dirty = true;
            }
        }
        let agent = t.agent.clone().unwrap_or_else(|| "claude".into());
        if t.usage != usage {
            t.usage = usage;
            self.dirty = true;
        }
        if !limits.is_empty() && self.limits.get(&agent) != Some(&limits) {
            self.limits.insert(agent, limits);
            self.dirty = true;
        }
    }

    /// What sessions in hydra have cost over the last day.
    pub(super) fn spent_today(&self) -> f64 {
        let now = term::unix_now();
        self.spent.iter().filter(|(at, _)| now.saturating_sub(*at) < DAY).map(|(_, c)| c).sum()
    }

    /// Codex writes its numbers to its session file: read them now and then, off the loop.
    pub(super) fn poll_codex(&mut self) {
        if self.codex_busy || self.last_codex.elapsed() < CODEX_EVERY {
            return;
        }
        let panes: Vec<(TermId, PathBuf)> = self
            .terms
            .values()
            .filter(|t| t.agent.as_deref() == Some("codex"))
            .map(|t| (t.id, t.head.as_ref().map(|h| h.top.clone()).unwrap_or_else(|| t.cwd.clone())))
            .collect();
        if panes.is_empty() {
            return;
        }
        self.codex_busy = true;
        self.last_codex = Instant::now();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let files = codex_recent_sessions();
            let mut usage = Vec::new();
            let mut limits = None;
            for (term, dir) in panes {
                let Some(file) = files.iter().find(|(_, cwd)| cwd.as_deref().is_some_and(|c| same_path(c, &dir))).map(|(f, _)| f) else { continue };
                let (u, l) = codex_facts(&read_tail(file, CODEX_TAIL));
                if let Some(u) = u {
                    usage.push((term, u));
                }
                limits = limits.or(l);
            }
            let _ = tx.blocking_send(Ev::CodexUsage { usage, limits });
        });
    }

    pub(super) fn codex_usage(&mut self, usage: Vec<(TermId, Usage)>, limits: Option<Vec<Limit>>) {
        self.codex_busy = false;
        for (term, u) in usage {
            if let Some(t) = self.terms.get_mut(&term)
                && t.usage != u
            {
                t.usage = u;
                self.dirty = true;
            }
        }
        if let Some(l) = limits
            && self.limits.get("codex") != Some(&l)
        {
            self.limits.insert("codex".into(), l);
            self.dirty = true;
        }
    }

    /// An agent stopped by a plan limit gets "continue" once the limit resets.
    pub(super) fn auto_continue(&mut self) {
        let now = term::unix_now();
        let on = self.cfg.auto_continue;
        let mut said = Vec::new();
        for t in self.terms.values_mut() {
            let Some(agent) = t.agent.clone() else { continue };
            if !on || t.status == Status::Working {
                // Off, or it's going again (you continued it yourself).
                if t.resume_at.take().is_some() {
                    self.dirty = true;
                }
                continue;
            }
            if let Some(at) = t.resume_at {
                if now >= at {
                    t.resume_at = None;
                    t.continued = Some(Instant::now());
                    t.pending_input = Some((b"continue\r".to_vec(), Instant::now()));
                    said.push((agent, t.id));
                    self.dirty = true;
                }
                continue;
            }
            if t.continued.is_some_and(|c| c.elapsed() < CONTINUE_SETTLES) || !LIMIT_HIT.is_match(&t.tail_text(LIMIT_ROWS)) {
                continue;
            }
            let reset = self.limits.get(&agent).and_then(|ls| ls.iter().filter(|l| l.used >= SPENT).map(|l| l.resets_at).max()).filter(|r| *r > now);
            t.resume_at = Some(reset.map(|r| r + AFTER_RESET).unwrap_or(now + RETRY_LIMIT_AFTER));
            self.dirty = true;
        }
        for (agent, term) in said {
            self.broadcast(|c| c.attach, ServerMsg::Notice(format!("{agent}'s limit reset: told it to continue (pane {term})")));
        }
    }
}

/// Codex's sessions folder (`$CODEX_HOME/sessions`, else `~/.codex/sessions`).
fn codex_sessions() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".codex")))?;
    Some(home.join("sessions"))
}

/// The session files of Codex's last two days with sessions (`YYYY/MM/DD/rollout-….jsonl`),
/// newest first, each with the folder it ran in.
fn codex_recent_sessions() -> Vec<(PathBuf, Option<PathBuf>)> {
    let Some(root) = codex_sessions() else { return Vec::new() };
    let sorted = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v.reverse();
        v
    };
    let days: Vec<PathBuf> = sorted(&root).iter().flat_map(|y| sorted(y)).flat_map(|m| sorted(&m)).take(2).collect();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = days
        .iter()
        .flat_map(|d| sorted(d))
        .filter(|f| f.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|(_, f)| {
        let cwd = codex_session_cwd(&f);
        (f, cwd)
    }).collect()
}

/// The folder a Codex session ran in, from its first line.
fn codex_session_cwd(file: &Path) -> Option<PathBuf> {
    use std::io::Read;
    static CWD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""cwd":("(?:[^"\\]|\\.)*")"#).unwrap());
    let mut head = vec![0; 16 * 1024];
    let n = std::fs::File::open(file).ok()?.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]);
    let quoted = CWD.captures(&text)?.get(1)?.as_str().to_string();
    serde_json::from_str::<String>(&quoted).ok().map(PathBuf::from)
}

/// The last `n` bytes of a file, as text.
fn read_tail(file: &Path, n: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(file) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(n)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// A limit window's name from its length in minutes.
fn window_name(minutes: u64) -> String {
    match minutes {
        300 => "5h".into(),
        10080 => "week".into(),
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// Context and plan limits from the end of a Codex session file: its last `token_count`.
pub(super) fn codex_facts(tail: &str) -> (Option<Usage>, Option<Vec<Limit>>) {
    let Some(v) = tail.lines().rev().filter(|l| l.contains("\"token_count\"")).find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()) else {
        return (None, None);
    };
    let p = &v["payload"];
    let info = &p["info"];
    let window = info["model_context_window"].as_f64().filter(|w| *w > 0.0);
    let used = info["last_token_usage"]["total_tokens"].as_f64();
    let usage = window.zip(used).map(|(w, u)| Usage { context: Some((u / w * 100.0).min(100.0) as f32), cost: None });
    let limits: Vec<Limit> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| {
            let l = &p["rate_limits"][k];
            Some(Limit { name: window_name(l["window_minutes"].as_u64()?), used: l["used_percent"].as_f64()? as f32, resets_at: l["resets_at"].as_u64()? })
        })
        .collect();
    (usage, (!limits.is_empty()).then_some(limits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_numbers_come_from_its_session_file() {
        let tail = r#"{"type":"event_msg","payload":{"type":"agent_message"}}
{"timestamp":"x","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":64600},"model_context_window":258400},"rate_limits":{"primary":{"used_percent":12.0,"window_minutes":300,"resets_at":1791322893},"secondary":{"used_percent":40.0,"window_minutes":10080,"resets_at":1791900000}}}}
{"type":"response_item","payload":{}}"#;
        let (u, l) = codex_facts(tail);
        assert_eq!(u.and_then(|u| u.context).map(|c| c.round()), Some(25.0));
        let l = l.unwrap();
        assert_eq!((l[0].name.as_str(), l[1].name.as_str(), l[1].used), ("5h", "week", 40.0));
        assert_eq!(codex_facts("nothing here"), (None, None));
    }

    #[test]
    fn a_limit_line_is_told_from_talk_about_limits() {
        for hit in ["Claude usage limit reached. Your limit will reset at 3pm", "You've hit your limit · resets 3pm", "■ You've hit your usage limit. Try again at 4:05 PM."] {
            assert!(LIMIT_HIT.is_match(hit), "{hit}");
        }
        for talk in ["I'll add a usage meter and a limit bar", "rate limits are per minute"] {
            assert!(!LIMIT_HIT.is_match(talk), "{talk}");
        }
    }
}
