//! Pull requests through `gh`: your open PRs with their checks and reviews (for the sidebar
//! and Jump), and one PR in full (checks, reviews, comments, description, diff).

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Checks {
    None,
    Pass,
    Fail,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Review {
    None,
    Approved,
    Changes,
}

/// One of your open pull requests.
#[derive(Debug, Clone, PartialEq)]
pub struct PrBrief {
    pub number: u64,
    pub title: String,
    pub branch: String,
    pub checks: Checks,
    pub review: Review,
    pub url: String,
}

impl PrBrief {
    /// Something for you to do: checks failing or changes requested.
    pub fn needs_you(&self) -> bool {
        self.checks == Checks::Fail || self.review == Review::Changes
    }

    pub fn state_text(&self) -> &'static str {
        match (self.checks, self.review) {
            (Checks::Fail, _) => "checks failing",
            (_, Review::Changes) => "changes requested",
            (Checks::Pending, _) => "checks running",
            (_, Review::Approved) => "approved",
            (Checks::Pass, _) => "checks pass",
            _ => "open",
        }
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// `gh <args>` in `dir`.
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    crate::proc::run(std::process::Command::new("gh").current_dir(dir).args(args))
}

/// One check's verdict: SUCCESS, FAILURE, PENDING, …
fn verdict(c: &Value) -> String {
    c.get("conclusion")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .or_else(|| c.get("state").and_then(Value::as_str))
        .unwrap_or("PENDING")
        .to_uppercase()
}

fn check_state(v: &str) -> Checks {
    match v {
        "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => Checks::Fail,
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Checks::Pass,
        _ => Checks::Pending,
    }
}

/// All of a PR's checks rolled into one.
pub fn rollup(checks: &Value) -> Checks {
    let Some(list) = checks.as_array().filter(|l| !l.is_empty()) else { return Checks::None };
    let states: Vec<Checks> = list.iter().map(|c| check_state(&verdict(c))).collect();
    if states.contains(&Checks::Fail) {
        Checks::Fail
    } else if states.contains(&Checks::Pending) {
        Checks::Pending
    } else {
        Checks::Pass
    }
}

fn review(decision: &str) -> Review {
    match decision {
        "APPROVED" => Review::Approved,
        "CHANGES_REQUESTED" => Review::Changes,
        _ => Review::None,
    }
}

pub fn parse_mine(out: &str) -> Result<Vec<PrBrief>, String> {
    let v: Value = serde_json::from_str(out).map_err(|e| e.to_string())?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|p| PrBrief {
            number: p.get("number").and_then(Value::as_u64).unwrap_or(0),
            title: s(p, "title"),
            branch: s(p, "headRefName"),
            checks: p.get("statusCheckRollup").map(rollup).unwrap_or(Checks::None),
            review: review(&s(p, "reviewDecision")),
            url: s(p, "url"),
        })
        .collect())
}

/// Your open pull requests in the repo at `dir`.
pub fn list_mine(dir: &Path) -> Result<Vec<PrBrief>, String> {
    let out = run(
        dir,
        &["pr", "list", "--author", "@me", "--state", "open", "--limit", "30", "--json", "number,title,headRefName,statusCheckRollup,reviewDecision,url"],
    )?;
    parse_mine(&out)
}

/// One PR in full.
#[derive(Debug, Clone, Default)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub author: String,
    pub branch: String,
    pub base: String,
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    /// (name, verdict), failing first.
    pub checks: Vec<(String, Checks)>,
    /// (who, APPROVED / CHANGES_REQUESTED / COMMENTED, what they wrote)
    pub reviews: Vec<(String, String, String)>,
    /// (who, what)
    pub comments: Vec<(String, String)>,
}

impl PrInfo {
    /// A message for the agent: what's failing and what reviewers asked for.
    pub fn fix_prompt(&self) -> String {
        let mut parts = vec![format!("On PR #{} ({}):", self.number, self.title)];
        let failing: Vec<&str> = self.checks.iter().filter(|(_, c)| *c == Checks::Fail).map(|(n, _)| n.as_str()).collect();
        if !failing.is_empty() {
            parts.push(format!("fix the failing checks ({}).", failing.join(", ")));
        }
        let asks: Vec<String> = self
            .reviews
            .iter()
            .filter(|(_, st, body)| st == "CHANGES_REQUESTED" || !body.trim().is_empty())
            .map(|(who, _, body)| format!("{who}: {}", body.trim().replace('\n', " ")))
            .chain(self.comments.iter().map(|(who, body)| format!("{who}: {}", body.trim().replace('\n', " "))))
            .collect();
        if !asks.is_empty() {
            parts.push(format!("Address the review comments: {}", asks.join(" | ")));
        }
        if parts.len() == 1 {
            parts.push("check it over and fix anything that's off.".into());
        }
        parts.join(" ")
    }
}

fn who(v: &Value) -> String {
    v.get("author").map(|a| s(a, "login")).unwrap_or_default()
}

pub fn parse_info(out: &str) -> Result<PrInfo, String> {
    let p: Value = serde_json::from_str(out).map_err(|e| e.to_string())?;
    let mut checks: Vec<(String, Checks)> = p
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let name = [s(c, "name"), s(c, "context"), s(c, "workflowName")].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "check".into());
            (name, check_state(&verdict(c)))
        })
        .collect();
    checks.sort_by_key(|(_, c)| match c {
        Checks::Fail => 0,
        Checks::Pending => 1,
        _ => 2,
    });
    let num = |k: &str| p.get(k).and_then(Value::as_u64).unwrap_or(0);
    Ok(PrInfo {
        number: num("number"),
        title: s(&p, "title"),
        url: s(&p, "url"),
        state: s(&p, "state"),
        author: who(&p),
        branch: s(&p, "headRefName"),
        base: s(&p, "baseRefName"),
        body: s(&p, "body"),
        additions: num("additions"),
        deletions: num("deletions"),
        files: num("changedFiles"),
        checks,
        reviews: p.get("reviews").and_then(Value::as_array).into_iter().flatten().map(|r| (who(r), s(r, "state"), s(r, "body"))).collect(),
        comments: p.get("comments").and_then(Value::as_array).into_iter().flatten().map(|c| (who(c), s(c, "body"))).collect(),
    })
}

/// A PR by number or branch name.
pub fn load(dir: &Path, which: &str) -> Result<PrInfo, String> {
    let out = run(
        dir,
        &[
            "pr", "view", which, "--json",
            "number,title,url,state,author,headRefName,baseRefName,body,additions,deletions,changedFiles,statusCheckRollup,reviews,comments",
        ],
    )?;
    parse_info(&out)
}

pub fn diff(dir: &Path, which: &str) -> Result<String, String> {
    run(dir, &["pr", "diff", which, "--color", "never"])
}

/// The PR view's state.
#[derive(Debug, Clone)]
pub struct PrView {
    pub dir: PathBuf,
    /// A number or a branch.
    pub which: String,
    pub info: Option<Result<PrInfo, String>>,
    pub diff: Option<Result<String, String>>,
    /// 0 overview, 1 diff
    pub tab: u8,
    pub scroll: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_up_checks_and_reviews() {
        let out = r#"[{"number":412,"title":"Rate limit","headRefName":"rate-limit","reviewDecision":"CHANGES_REQUESTED","url":"u",
            "statusCheckRollup":[{"conclusion":"SUCCESS"},{"conclusion":"FAILURE","name":"test"}]},
            {"number":7,"title":"Docs","headRefName":"docs","reviewDecision":"","url":"u","statusCheckRollup":[{"state":"PENDING"}]}]"#;
        let prs = parse_mine(out).unwrap();
        assert_eq!(prs[0].checks, Checks::Fail);
        assert_eq!(prs[0].review, Review::Changes);
        assert!(prs[0].needs_you());
        assert_eq!(prs[1].checks, Checks::Pending);
        assert!(!prs[1].needs_you());
        let info = parse_info(
            r#"{"number":412,"title":"Rate limit","url":"u","state":"OPEN","author":{"login":"cody"},"headRefName":"rate-limit","baseRefName":"main",
            "body":"b","additions":42,"deletions":7,"changedFiles":3,
            "statusCheckRollup":[{"conclusion":"SUCCESS","name":"lint"},{"conclusion":"FAILURE","name":"test"}],
            "reviews":[{"author":{"login":"sam"},"state":"CHANGES_REQUESTED","body":"use X-Forwarded-For"}],"comments":[]}"#,
        )
        .unwrap();
        assert_eq!(info.checks[0], ("test".to_string(), Checks::Fail), "failing first");
        let p = info.fix_prompt();
        assert!(p.contains("failing checks (test)") && p.contains("sam: use X-Forwarded-For"), "{p}");
    }
}

#[cfg(test)]
mod live {
    /// Against a real repo: `HYDRA_PR_DIR=<clone> HYDRA_PR=<n> cargo test pr_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn pr_live() {
        let dir = std::path::PathBuf::from(std::env::var("HYDRA_PR_DIR").unwrap());
        let n = std::env::var("HYDRA_PR").unwrap();
        let info = super::load(&dir, &n).unwrap();
        println!("#{} {} [{}] {} → {} +{} −{} files {}", info.number, info.title, info.state, info.branch, info.base, info.additions, info.deletions, info.files);
        println!("checks: {:?}", info.checks.iter().take(5).collect::<Vec<_>>());
        println!("reviews: {} comments: {}", info.reviews.len(), info.comments.len());
        let d = super::diff(&dir, &n).unwrap();
        println!("diff: {} lines", d.lines().count());
        println!("fix prompt: {}", info.fix_prompt().chars().take(200).collect::<String>());
    }
}
