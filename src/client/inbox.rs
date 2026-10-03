//! The inbox: GitHub pull requests and issues (through the `gh` CLI) and Linear tickets
//! (through its API, when LINEAR_API_KEY is set), each one key away from becoming a task.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const TABS: [&str; 3] = ["Pull requests", "Issues", "Linear"];

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// "#57", "ABC-123"
    pub key: String,
    pub title: String,
    pub url: String,
    /// One short status line: checks, review state, ticket state.
    pub state: String,
    pub tone: Tone,
    pub meta: String,
    pub body: String,
    /// PR head branch (for checking out as a task).
    pub branch: Option<String>,
}

/// How the state should be coloured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Good,
    Bad,
    Waiting,
    Plain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InboxView {
    pub tab: usize,
    pub query: String,
    pub sel: usize,
    pub dir: PathBuf,
    /// One slot per tab: None = loading, Some(Err) = couldn't load.
    pub lists: [Option<Result<Vec<Item>, String>>; 3],
    pub scroll: u16,
}

impl InboxView {
    pub fn new(dir: PathBuf) -> InboxView {
        InboxView { tab: 0, query: String::new(), sel: 0, dir, lists: [None, None, None], scroll: 0 }
    }

    pub fn visible(&self) -> Vec<&Item> {
        let Some(Ok(list)) = &self.lists[self.tab] else { return Vec::new() };
        let q = self.query.to_lowercase();
        list.iter()
            .filter(|i| q.is_empty() || i.title.to_lowercase().contains(&q) || i.key.to_lowercase().contains(&q))
            .collect()
    }
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|_| format!("`{program}` isn't installed or isn't on PATH"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn short_date(iso: &str) -> String {
    iso.get(..10).unwrap_or(iso).to_string()
}

pub fn load_prs(dir: &Path) -> Result<Vec<Item>, String> {
    let out = run(
        dir,
        "gh",
        &[
            "pr", "list", "--limit", "50", "--json",
            "number,title,author,headRefName,isDraft,reviewDecision,statusCheckRollup,mergeable,updatedAt,url,body",
        ],
    )?;
    parse_prs(&out)
}

/// `gh pr list --json ...` output to inbox items.
pub fn parse_prs(out: &str) -> Result<Vec<Item>, String> {
    let v: Value = serde_json::from_str(out).map_err(|e| e.to_string())?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let (mut ok, mut bad, mut wait) = (0, 0, 0);
            for c in p.get("statusCheckRollup").and_then(Value::as_array).into_iter().flatten() {
                let verdict = c.get("conclusion").and_then(Value::as_str).filter(|x| !x.is_empty())
                    .or_else(|| c.get("state").and_then(Value::as_str))
                    .unwrap_or("PENDING");
                match verdict {
                    "SUCCESS" | "NEUTRAL" | "SKIPPED" => ok += 1,
                    "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" => bad += 1,
                    _ => wait += 1,
                }
            }
            let review = s(p, "reviewDecision");
            let conflict = s(p, "mergeable") == "CONFLICTING";
            let draft = p.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
            let mut parts = Vec::new();
            if draft {
                parts.push("draft".to_string());
            }
            if bad > 0 {
                parts.push(format!("{bad} check{} failing", if bad == 1 { "" } else { "s" }));
            } else if wait > 0 {
                parts.push("checks running".into());
            } else if ok > 0 {
                parts.push("checks pass".into());
            }
            match review.as_str() {
                "APPROVED" => parts.push("approved".into()),
                "CHANGES_REQUESTED" => parts.push("changes requested".into()),
                "REVIEW_REQUIRED" => parts.push("needs review".into()),
                _ => {}
            }
            if conflict {
                parts.push("merge conflict".into());
            }
            let tone = if bad > 0 || conflict || review == "CHANGES_REQUESTED" {
                Tone::Bad
            } else if wait > 0 || review == "REVIEW_REQUIRED" {
                Tone::Waiting
            } else if ok > 0 || review == "APPROVED" {
                Tone::Good
            } else {
                Tone::Plain
            };
            let author = p.get("author").map(|a| s(a, "login")).unwrap_or_default();
            Item {
                key: format!("#{}", p.get("number").and_then(Value::as_u64).unwrap_or(0)),
                title: s(p, "title"),
                url: s(p, "url"),
                state: if parts.is_empty() { "open".into() } else { parts.join(" · ") },
                tone,
                meta: format!("{author} · {} · updated {}", s(p, "headRefName"), short_date(&s(p, "updatedAt"))),
                body: s(p, "body"),
                branch: Some(s(p, "headRefName")),
            }
        })
        .collect())
}

pub fn load_issues(dir: &Path) -> Result<Vec<Item>, String> {
    let out = run(dir, "gh", &["issue", "list", "--limit", "50", "--json", "number,title,author,labels,assignees,updatedAt,url,body"])?;
    let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let labels: Vec<String> =
                i.get("labels").and_then(Value::as_array).into_iter().flatten().map(|l| s(l, "name")).collect();
            let assignees: Vec<String> =
                i.get("assignees").and_then(Value::as_array).into_iter().flatten().map(|a| s(a, "login")).collect();
            Item {
                key: format!("#{}", i.get("number").and_then(Value::as_u64).unwrap_or(0)),
                title: s(i, "title"),
                url: s(i, "url"),
                state: if labels.is_empty() { "open".into() } else { labels.join(", ") },
                tone: Tone::Plain,
                meta: format!(
                    "{} · {} · updated {}",
                    i.get("author").map(|a| s(a, "login")).unwrap_or_default(),
                    if assignees.is_empty() { "unassigned".into() } else { format!("assigned {}", assignees.join(", ")) },
                    short_date(&s(i, "updatedAt"))
                ),
                body: s(i, "body"),
                branch: None,
            }
        })
        .collect())
}

/// Open Linear tickets assigned to you. Needs a personal API key in LINEAR_API_KEY.
pub fn load_linear(dir: &Path) -> Result<Vec<Item>, String> {
    let key = std::env::var("LINEAR_API_KEY")
        .map_err(|_| "Set LINEAR_API_KEY (Linear → Settings → API → personal key) to see your tickets here.".to_string())?;
    let query = r#"{"query":"{ viewer { assignedIssues(first: 50, filter: { state: { type: { nin: [\"completed\", \"canceled\"] } } }, orderBy: updatedAt) { nodes { identifier title url description priorityLabel updatedAt state { name type } team { key } } } } }"}"#;
    // The key goes in through stdin (`-H @-`) so it never shows up in the process list.
    let out = {
        use std::io::Write;
        let mut cmd = Command::new("curl");
        cmd.current_dir(dir)
            .args(["-s", "-X", "POST", "https://api.linear.app/graphql", "-H", "Content-Type: application/json", "-H", "@-", "-d", query])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|_| "`curl` isn't installed or isn't on PATH".to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = writeln!(stdin, "Authorization: {key}");
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let v: Value = serde_json::from_str(&out).map_err(|_| "Linear sent back something unexpected".to_string())?;
    if let Some(err) = v.get("errors").and_then(|e| e.get(0)).map(|e| s(e, "message")) {
        return Err(format!("Linear: {err}"));
    }
    let nodes = v.pointer("/data/viewer/assignedIssues/nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(nodes
        .iter()
        .map(|n| {
            let state = n.get("state").map(|st| s(st, "name")).unwrap_or_default();
            let kind = n.get("state").map(|st| s(st, "type")).unwrap_or_default();
            Item {
                key: s(n, "identifier"),
                title: s(n, "title"),
                url: s(n, "url"),
                state: state.clone(),
                tone: match kind.as_str() {
                    "started" => Tone::Waiting,
                    "unstarted" | "backlog" | "triage" => Tone::Plain,
                    _ => Tone::Plain,
                },
                meta: format!("{} · {} · updated {}", s(n, "priorityLabel"), n.get("team").map(|t| s(t, "key")).unwrap_or_default(), short_date(&s(n, "updatedAt"))),
                body: s(n, "description"),
                branch: None,
            }
        })
        .collect())
}

pub fn load(tab: usize, dir: &Path) -> Result<Vec<Item>, String> {
    match tab {
        0 => load_prs(dir),
        1 => load_issues(dir),
        _ => load_linear(dir),
    }
}

/// The task prompt for an inbox item: what to do, with a link for context.
pub fn task_prompt(tab: usize, item: &Item) -> String {
    match tab {
        0 => format!("Pull request {} \"{}\" ({}): get its checks passing and address review comments.", item.key, item.title, item.url),
        1 => format!("Fix GitHub issue {}: {} ({})", item.key, item.title, item.url),
        _ => format!("Linear ticket {}: {} ({})", item.key, item.title, item.url),
    }
}

/// Open a URL in the browser.
pub fn open_url(url: &str) {
    if url.starts_with("https://") {
        let _ = super::files::open_default(Path::new(url));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `gh pr list` output from a public repo, captured by the test harness.
    #[test]
    fn parses_live_pr_json() {
        let Ok(json) = std::fs::read_to_string(std::env::var("HYDRA_PR_SAMPLE").unwrap_or_default()) else { return };
        let items = parse_prs(&json).unwrap();
        assert!(!items.is_empty());
        for i in items.iter().take(8) {
            println!("{:<7} {:<50} {}", i.key, i.title.chars().take(50).collect::<String>(), i.state);
        }
    }
}
