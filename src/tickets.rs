//! Tickets from your trackers (GitHub issues, Linear, Plane): listing yours, and telling a
//! tracker how far an agent has got with one.

use crate::proc::quiet;
use serde_json::{Value, json};
use std::path::Path;

/// `fix the flaky checkout test` → `fix-the-flaky-checkout`
pub fn slug(s: &str, words: usize) -> String {
    let mut out = String::new();
    for w in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(words) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    out.chars().take(40).collect::<String>().trim_end_matches('-').to_string()
}

/// A ticket from any tracker.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    /// "#57", "ENG-123"
    pub key: String,
    /// The tracker's own id, for updating it: the issue number (GitHub), the issue's id
    /// (Linear), "project id/issue id" (Plane).
    pub id: String,
    pub title: String,
    pub url: String,
    pub state: String,
    pub meta: String,
    pub body: String,
}

impl Ticket {
    /// What the agent is told.
    pub fn prompt(&self, source: &str) -> String {
        let body: String = self.body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(700).collect();
        let mut p = format!("Work on {source} ticket {}: {}.", self.key, self.title);
        if !body.is_empty() {
            p.push_str(&format!(" Details: {body}"));
        }
        if !self.url.is_empty() {
            p.push_str(&format!(" ({})", self.url));
        }
        p
    }

    pub fn branch(&self) -> String {
        let key = slug(&self.key, 3);
        let title = slug(&self.title, 4);
        if key.is_empty() { title } else { format!("{key}-{title}") }
    }
}

/// A place tickets come from. Adding a tracker is one more of these.
pub trait Source {
    /// Config name ("github", "linear", "plane").
    fn id(&self) -> &'static str;
    /// Tab label.
    fn label(&self) -> &'static str;
    /// Open tickets for you, for the repo at `dir`.
    fn list(&self, dir: &Path) -> Result<Vec<Ticket>, String>;
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}


/// An HTTP request through curl, with the secret header passed on stdin (`-H @-`) so it
/// never appears in the process list.
fn curl(method: &str, url: &str, secret_header: &str, body: Option<&str>) -> Result<Value, String> {
    use std::io::Write;
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-s", "--max-time", "20", "-X", method, url, "-H", "Content-Type: application/json", "-H", "@-"]);
    if let Some(b) = body {
        cmd.args(["-d", b]);
    }
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = quiet(&mut cmd).spawn().map_err(|_| "`curl` isn't installed or isn't on PATH".to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{secret_header}");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice(&out.stdout).map_err(|_| format!("{url} sent back something unexpected"))
}

pub struct GitHubIssues;

impl Source for GitHubIssues {
    fn id(&self) -> &'static str {
        "github"
    }
    fn label(&self) -> &'static str {
        "GitHub issues"
    }
    fn list(&self, dir: &Path) -> Result<Vec<Ticket>, String> {
        let mut cmd = std::process::Command::new("gh");
        cmd.current_dir(dir).args(["issue", "list", "--state", "open", "--limit", "50", "--json", "number,title,url,labels,assignees,updatedAt,body"]);
        let out = quiet(&mut cmd).output().map_err(|_| "`gh` isn't installed or isn't on PATH".to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string());
        }
        parse_github(&String::from_utf8_lossy(&out.stdout))
    }
}

pub fn parse_github(out: &str) -> Result<Vec<Ticket>, String> {
    let v: Value = serde_json::from_str(out).map_err(|e| e.to_string())?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let labels: Vec<String> = i.get("labels").and_then(Value::as_array).into_iter().flatten().map(|l| s(l, "name")).collect();
            let who: Vec<String> = i.get("assignees").and_then(Value::as_array).into_iter().flatten().map(|a| s(a, "login")).collect();
            Ticket {
                key: format!("#{}", i.get("number").and_then(Value::as_u64).unwrap_or(0)),
                id: i.get("number").and_then(Value::as_u64).unwrap_or(0).to_string(),
                title: s(i, "title"),
                url: s(i, "url"),
                state: if labels.is_empty() { "open".into() } else { labels.join(", ") },
                meta: if who.is_empty() { "unassigned".into() } else { format!("assigned {}", who.join(", ")) },
                body: s(i, "body"),
            }
        })
        .collect())
}

pub struct Linear {
    pub key: String,
}

impl Source for Linear {
    fn id(&self) -> &'static str {
        "linear"
    }
    fn label(&self) -> &'static str {
        "Linear"
    }
    fn list(&self, _dir: &Path) -> Result<Vec<Ticket>, String> {
        if self.key.is_empty() {
            return Err("Set LINEAR_API_KEY (Linear → Settings → API → personal key), or [tickets] linear_key in config.local.toml.".into());
        }
        let query = r#"{"query":"{ viewer { assignedIssues(first: 50, filter: { state: { type: { nin: [\"completed\", \"canceled\"] } } }, orderBy: updatedAt) { nodes { id identifier title url description priorityLabel state { name } team { key } } } } }"}"#;
        let v = curl("POST", "https://api.linear.app/graphql", &format!("Authorization: {}", self.key), Some(query))?;
        if let Some(err) = v.get("errors").and_then(|e| e.get(0)).map(|e| s(e, "message")) {
            return Err(format!("Linear: {err}"));
        }
        Ok(v.pointer("/data/viewer/assignedIssues/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|n| Ticket {
                key: s(n, "identifier"),
                id: s(n, "id"),
                title: s(n, "title"),
                url: s(n, "url"),
                state: n.get("state").map(|st| s(st, "name")).unwrap_or_default(),
                meta: format!("{} · {}", s(n, "priorityLabel"), n.get("team").map(|t| s(t, "key")).unwrap_or_default()),
                body: s(n, "description"),
            })
            .collect())
    }
}

pub struct Plane {
    pub api: String,
    pub app: String,
    pub workspace: String,
    pub key: String,
}

impl Source for Plane {
    fn id(&self) -> &'static str {
        "plane"
    }
    fn label(&self) -> &'static str {
        "Plane"
    }
    fn list(&self, _dir: &Path) -> Result<Vec<Ticket>, String> {
        if self.key.is_empty() || self.workspace.is_empty() {
            return Err("Set PLANE_API_KEY and [tickets] plane_workspace (your workspace slug); self-hosted: plane_url.".into());
        }
        let api = self.api.trim_end_matches('/');
        let header = format!("X-API-Key: {}", self.key);
        let me = curl("GET", &format!("{api}/api/v1/users/me/"), &header, None)?;
        let my_id = s(&me, "id");
        if my_id.is_empty() {
            return Err(format!("Plane: {}", s(&me, "detail").chars().take(80).collect::<String>()));
        }
        let projects = curl("GET", &format!("{api}/api/v1/workspaces/{}/projects/", self.workspace), &header, None)?;
        let list = projects.get("results").or(Some(&projects)).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for p in list.iter().take(12) {
            let (pid, ident) = (s(p, "id"), s(p, "identifier"));
            let issues = curl("GET", &format!("{api}/api/v1/workspaces/{}/projects/{pid}/issues/?per_page=100", self.workspace), &header, None)?;
            out.extend(parse_plane(&issues, &my_id, &ident, &pid, &format!("{}/{}/projects/{pid}/issues", self.app.trim_end_matches('/'), self.workspace)));
        }
        Ok(out)
    }
}

/// Plane's issues for one project: the open ones assigned to `me`.
pub fn parse_plane(v: &Value, me: &str, ident: &str, project: &str, url_base: &str) -> Vec<Ticket> {
    v.get("results")
        .or(Some(v))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| i.get("assignees").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(me))))
        .filter(|i| i.get("completed_at").is_none_or(Value::is_null) && i.get("archived_at").is_none_or(Value::is_null))
        .map(|i| {
            let seq = i.get("sequence_id").and_then(Value::as_u64).unwrap_or(0);
            Ticket {
                key: format!("{ident}-{seq}"),
                id: format!("{project}/{}", s(i, "id")),
                title: s(i, "name"),
                url: format!("{url_base}/{}", s(i, "id")),
                state: s(i, "priority"),
                meta: ident.to_string(),
                body: s(i, "description_stripped"),
            }
        })
        .collect()
}

/// The ticket sources turned on in config, in order.
pub fn sources(cfg: &crate::config::Tickets) -> Vec<Box<dyn Source + Send>> {
    let env = |name: &str, given: &str| if given.is_empty() { std::env::var(name).unwrap_or_default() } else { given.to_string() };
    cfg.sources
        .iter()
        .filter_map(|id| -> Option<Box<dyn Source + Send>> {
            match id.as_str() {
                "github" => Some(Box::new(GitHubIssues)),
                "linear" => Some(Box::new(Linear { key: env("LINEAR_API_KEY", &cfg.linear_key) })),
                "plane" => Some(Box::new(Plane {
                    api: cfg.plane_url.clone(),
                    app: cfg.plane_app_url.clone(),
                    workspace: cfg.plane_workspace.clone(),
                    key: env("PLANE_API_KEY", &cfg.plane_key),
                })),
                _ => None,
            }
        })
        .collect()
}


/// How far a queued ticket has got, as its tracker is told.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// An agent started on it.
    Started,
    /// The agent finished: it's ready for you to review.
    Review,
}

/// Tell a ticket's tracker how far it has got: GitHub gets a comment, Linear and Plane
/// move the issue to their "in progress" state (Started) or a review state, when the team
/// has one (Review). `note` says where the work is ("branch fix-login in shop-api").
pub fn mark(cfg: &crate::config::Tickets, source: &str, dir: &Path, t: &Ticket, progress: Progress, note: &str) -> Result<String, String> {
    let env = |name: &str, given: &str| if given.is_empty() { std::env::var(name).unwrap_or_default() } else { given.to_string() };
    match source {
        "github" => {
            let body = match progress {
                Progress::Started => format!("An agent started on this ({note}), from hydra's queue."),
                Progress::Review => format!("Ready for review ({note})."),
            };
            let mut cmd = std::process::Command::new("gh");
            cmd.current_dir(dir).args(["issue", "comment", &t.id, "--body", &body]);
            let out = quiet(&mut cmd).output().map_err(|_| "`gh` isn't installed or isn't on PATH".to_string())?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string());
            }
            Ok(format!("commented on {}", t.key))
        }
        "linear" => {
            let key = env("LINEAR_API_KEY", &cfg.linear_key);
            let auth = format!("Authorization: {key}");
            let q = json!({ "query": "query($id: String!) { issue(id: $id) { team { states { nodes { id name type } } } } }", "variables": { "id": t.id } });
            let v = curl("POST", "https://api.linear.app/graphql", &auth, Some(&q.to_string()))?;
            let states: Vec<(String, String, String)> = v
                .pointer("/data/issue/team/states/nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|n| (s(n, "id"), s(n, "name"), s(n, "type")))
                .collect();
            let Some((state, name)) = pick_state(&states, progress) else { return Ok(format!("{} has no state for that; left as it is", t.key)) };
            let m = json!({ "query": "mutation($id: String!, $state: String!) { issueUpdate(id: $id, input: { stateId: $state }) { success } }", "variables": { "id": t.id, "state": state } });
            let v = curl("POST", "https://api.linear.app/graphql", &auth, Some(&m.to_string()))?;
            if v.pointer("/data/issueUpdate/success").and_then(Value::as_bool) != Some(true) {
                return Err(format!("Linear didn't move {}", t.key));
            }
            Ok(format!("moved {} to {name}", t.key))
        }
        "plane" => {
            let (project, issue) = t.id.split_once('/').ok_or("not a Plane ticket")?;
            let api = cfg.plane_url.trim_end_matches('/');
            let header = format!("X-API-Key: {}", env("PLANE_API_KEY", &cfg.plane_key));
            let base = format!("{api}/api/v1/workspaces/{}/projects/{project}", cfg.plane_workspace);
            let v = curl("GET", &format!("{base}/states/"), &header, None)?;
            let states: Vec<(String, String, String)> =
                v.get("results").or(Some(&v)).and_then(Value::as_array).into_iter().flatten().map(|n| (s(n, "id"), s(n, "name"), s(n, "group"))).collect();
            let Some((state, name)) = pick_state(&states, progress) else { return Ok(format!("{} has no state for that; left as it is", t.key)) };
            let v = curl("PATCH", &format!("{base}/issues/{issue}/"), &header, Some(&json!({ "state": state }).to_string()))?;
            if s(&v, "id").is_empty() {
                return Err(format!("Plane didn't move {}", t.key));
            }
            Ok(format!("moved {} to {name}", t.key))
        }
        other => Err(format!("can't update tickets from {other}")),
    }
}

/// The tracker state for `progress` from (id, name, kind) where kind is the tracker's
/// group ("started" in both Linear and Plane): in progress is the first started state not
/// about review, review the started state named for it (none: stay where it is).
pub fn pick_state(states: &[(String, String, String)], progress: Progress) -> Option<(String, String)> {
    let started = states.iter().filter(|(_, _, kind)| kind == "started");
    let review = |n: &str| n.to_lowercase().contains("review");
    match progress {
        Progress::Started => started.clone().find(|(_, n, _)| n.to_lowercase().contains("progress")).or_else(|| started.clone().find(|(_, n, _)| !review(n))),
        Progress::Review => started.clone().find(|(_, n, _)| review(n)),
    }
    .map(|(id, name, _)| (id.clone(), name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_prompts() {
        assert_eq!(slug("Fix the flaky checkout test!", 4), "fix-the-flaky-checkout");
        let t = Ticket { key: "ENG-123".into(), id: "x".into(), title: "Checkout fails on Safari".into(), url: "https://x".into(), state: "".into(), meta: "".into(), body: "Steps:\n1. open".into() };
        assert_eq!(t.branch(), "eng-123-checkout-fails-on-safari");
        let p = t.prompt("Linear");
        assert!(p.starts_with("Work on Linear ticket ENG-123: Checkout fails on Safari.") && p.contains("Steps: 1. open") && p.ends_with("(https://x)"));
    }

    #[test]
    fn plane_issues_for_me() {
        let v: Value = serde_json::from_str(
            r#"{"results":[
                {"id":"a","name":"Mine","sequence_id":12,"assignees":["me"],"priority":"high","description_stripped":"do it","completed_at":null},
                {"id":"b","name":"Theirs","sequence_id":13,"assignees":["you"],"priority":"low"},
                {"id":"c","name":"Done","sequence_id":14,"assignees":["me"],"completed_at":"2026-01-01"}]}"#,
        )
        .unwrap();
        let t = parse_plane(&v, "me", "WEB", "p1", "https://app.plane.so/team/projects/p1/issues");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].key, "WEB-12");
        assert_eq!(t[0].url, "https://app.plane.so/team/projects/p1/issues/a");
        assert_eq!(t[0].id, "p1/a", "what updating it needs");
    }

    #[test]
    fn a_tickets_state_follows_the_work() {
        let st = |list: &[(&str, &str)]| list.iter().map(|(n, k)| (format!("id-{n}"), n.to_string(), k.to_string())).collect::<Vec<_>>();
        let team = st(&[("Todo", "unstarted"), ("In Progress", "started"), ("In Review", "started"), ("Done", "completed")]);
        assert_eq!(pick_state(&team, Progress::Started).map(|s| s.1).as_deref(), Some("In Progress"));
        assert_eq!(pick_state(&team, Progress::Review).map(|s| s.1).as_deref(), Some("In Review"));
        let plain = st(&[("Todo", "unstarted"), ("Doing", "started"), ("Done", "completed")]);
        assert_eq!(pick_state(&plain, Progress::Started).map(|s| s.1).as_deref(), Some("Doing"));
        assert_eq!(pick_state(&plain, Progress::Review), None, "no review state: it stays where it is");
    }

    #[test]
    fn github_issues_parse() {
        let t = parse_github(r#"[{"number":57,"title":"Crash","url":"u","labels":[{"name":"bug"}],"assignees":[],"body":"b"}]"#).unwrap();
        assert_eq!((t[0].key.as_str(), t[0].state.as_str(), t[0].meta.as_str()), ("#57", "bug", "unassigned"));
    }
}

