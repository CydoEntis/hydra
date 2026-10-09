//! `seshi mcp`: seshi as an MCP server (stdio), so an agent can see and steer the others —
//! list them, read their screens, message them, answer their prompts (only as allowed), start
//! new ones in their own worktrees, interrupt them. There is no merge, push or delete.
//!
//! Guardrails come from `[mcp]` in config: `approve` (never / safe / always) decides whether
//! prompts may be answered "yes"; `scope` (project / all) limits it to the caller's own repo.

use crate::cli;
use crate::protocol::*;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// The repo (or folder) the calling agent works in.
fn caller_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    crate::gitfs::head(&cwd).map(|h| h.main_root).unwrap_or(cwd)
}

fn root_of(t: &TermInfo) -> PathBuf {
    t.root.clone().unwrap_or_else(|| t.cwd.clone())
}

struct Server {
    cfg: crate::config::Config,
    root: PathBuf,
    me: Option<TermId>,
}

fn text(s: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": s.into() }] })
}

fn fail(s: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": s.into() }], "isError": true })
}

fn tools() -> Value {
    let id = json!({ "type": "integer", "description": "The session id (from seshi_list)." });
    json!([
        {
            "name": "seshi_list",
            "description": "List the agent and shell sessions running in seshi (in this project, unless seshi is set to allow all): id, agent, status (working / needs-you / done / idle), branch and folder, what it's on, and the question it's asking if it needs an answer.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "seshi_read",
            "description": "Read the end of a session's screen (what the agent shows right now).",
            "inputSchema": { "type": "object", "properties": { "id": id, "lines": { "type": "integer", "description": "How many lines from the bottom (default 40)." } }, "required": ["id"] }
        },
        {
            "name": "seshi_send",
            "description": "Type a message into a session's prompt and press Enter, like a person would. With wait, returns once it has finished that turn, with its reply.",
            "inputSchema": { "type": "object", "properties": {
                "id": id,
                "text": { "type": "string" },
                "wait": { "type": "boolean", "description": "Wait for its turn to end and return its reply (default false)." },
                "timeout": { "type": "integer", "description": "Seconds to wait at most (default 600)." }
            }, "required": ["id", "text"] }
        },
        {
            "name": "seshi_wait",
            "description": "Wait until a session finishes its turn (or needs an answer), or until text matching a regex shows on its screen. Returns its reply or the matching line.",
            "inputSchema": { "type": "object", "properties": {
                "id": id,
                "regex": { "type": "string", "description": "Wait for this on the screen instead of the turn ending." },
                "timeout": { "type": "integer", "description": "Seconds to wait at most (default 600)." }
            }, "required": ["id"] }
        },
        {
            "name": "seshi_answer",
            "description": "Answer a session's numbered prompt (e.g. 1 = Yes, 3 = No). Saying yes is only allowed if the user has turned that on in seshi's settings; otherwise ask the user.",
            "inputSchema": { "type": "object", "properties": { "id": id, "choice": { "type": "integer", "minimum": 1, "maximum": 9 } }, "required": ["id", "choice"] }
        },
        {
            "name": "seshi_start",
            "description": "Start a new agent on a task. In a git repo it gets its own worktree and branch, so it never edits the same files as anyone else. Returns its session id.",
            "inputSchema": { "type": "object", "properties": {
                "prompt": { "type": "string", "description": "The task." },
                "agent": { "type": "string", "description": "claude, codex, gemini, … (default: the user's first agent)." },
                "worktree": { "type": "boolean", "description": "Own worktree (default true)." }
            }, "required": ["prompt"] }
        },
        {
            "name": "seshi_interrupt",
            "description": "Interrupt what a session is doing (Ctrl+C).",
            "inputSchema": { "type": "object", "properties": { "id": id }, "required": ["id"] }
        }
    ])
}

/// The numbered choices on a screen and the question above them.
fn prompt_on(screen: &str) -> (String, Vec<(u32, String)>) {
    let lines: Vec<&str> = screen.lines().collect();
    let mut opts = Vec::new();
    let mut first = lines.len();
    for (i, l) in lines.iter().enumerate().rev().take(20) {
        let t = l.trim().trim_matches(|c: char| "│┃ ".contains(c)).trim_start_matches(['❯', '>', ' ']).trim();
        let mut ch = t.chars();
        if let (Some(d), Some('.')) = (ch.next(), ch.next())
            && let Some(n) = d.to_digit(10)
        {
            opts.push((n, ch.as_str().trim().to_string()));
            first = i;
        }
    }
    opts.reverse();
    let question = lines[first.saturating_sub(6)..first].join(" ");
    (question, opts)
}

impl Server {
    fn snapshot(&self) -> Result<Snapshot, String> {
        match cli::block_on(cli::request(ClientMsg::Query(Query::List))) {
            Ok(Reply::List(s)) => Ok(s),
            Ok(_) => Err("unexpected reply".into()),
            Err(e) => Err(format!("{e:#}")),
        }
    }

    fn in_scope(&self, t: &TermInfo) -> bool {
        self.cfg.mcp.scope == "all" || norm(&root_of(t)) == norm(&self.root)
    }

    /// A session the caller may touch.
    fn session(&self, args: &Value) -> Result<TermInfo, String> {
        let id = args.get("id").and_then(Value::as_u64).ok_or("give the session id")? as TermId;
        let snap = self.snapshot()?;
        let t = snap.terms.get(&id).cloned().ok_or_else(|| format!("no session {id}"))?;
        if !self.in_scope(&t) {
            return Err(format!("session {id} is in another project; seshi only lets you reach sessions in {}", self.root.display()));
        }
        Ok(t)
    }

    fn screen(&self, id: TermId) -> Result<String, String> {
        match cli::block_on(cli::request(ClientMsg::Query(Query::Read { term: id }))) {
            Ok(Reply::Text(t)) => Ok(t),
            Ok(_) => Err("unexpected reply".into()),
            Err(e) => Err(format!("{e:#}")),
        }
    }

    fn call(&self, name: &str, args: &Value) -> Value {
        let r = match name {
            "seshi_list" => self.list(),
            "seshi_read" => self.read(args),
            "seshi_send" => self.send(args),
            "seshi_wait" => self.session(args).and_then(|t| {
                let regex = args.get("regex").and_then(Value::as_str).map(str::to_string);
                self.wait(t.id, regex.as_deref(), args, false)
            }),
            "seshi_answer" => self.answer(args),
            "seshi_start" => self.start(args),
            "seshi_interrupt" => self.session(args).and_then(|t| cli::send(Some(t.id), "\x03".into(), false).map(|_| format!("interrupted {}", t.id)).map_err(|e| format!("{e:#}"))),
            other => Err(format!("no tool {other}")),
        };
        match r {
            Ok(s) => text(s),
            Err(e) => fail(e),
        }
    }

    fn list(&self) -> Result<String, String> {
        let snap = self.snapshot()?;
        let mut out = Vec::new();
        for t in snap.terms.values().filter(|t| self.in_scope(t)) {
            let mut row = json!({
                "id": t.id,
                "agent": t.agent.clone().unwrap_or_else(|| if t.is_shell() { "shell".into() } else { t.display_name() }),
                "status": match t.status { Status::Blocked => "needs-you", s => s.label() },
                "branch": t.branch,
                "folder": t.top.clone().unwrap_or_else(|| t.cwd.clone()),
                "on": t.summary,
                "asleep": t.asleep,
            });
            if Some(t.id) == self.me {
                row["you"] = json!(true);
            }
            if t.status == Status::Blocked
                && let Ok(screen) = self.screen(t.id)
            {
                let (q, opts) = prompt_on(&screen);
                row["question"] = json!(q.trim());
                row["choices"] = json!(opts.iter().map(|(n, l)| format!("{n}. {l}")).collect::<Vec<_>>());
            }
            out.push(row);
        }
        Ok(serde_json::to_string_pretty(&out).unwrap_or_default())
    }

    fn read(&self, args: &Value) -> Result<String, String> {
        let t = self.session(args)?;
        let n = args.get("lines").and_then(Value::as_u64).unwrap_or(40) as usize;
        let screen = self.screen(t.id)?;
        let lines: Vec<&str> = screen.lines().collect();
        Ok(lines[lines.len().saturating_sub(n)..].join("\n").trim_end().to_string())
    }

    fn send(&self, args: &Value) -> Result<String, String> {
        let t = self.session(args)?;
        if Some(t.id) == self.me {
            return Err("that's you".into());
        }
        let msg = args.get("text").and_then(Value::as_str).ok_or("give the text")?;
        // Text sent to a session that's waiting on a question would answer it; that goes
        // through seshi_answer and the user's approval rules instead.
        if t.status == Status::Blocked && self.cfg.mcp.approve != "always" {
            return Err(format!("session {} is waiting on a question; use seshi_answer (the user's approval rules apply) or ask the user", t.id));
        }
        cli::send(Some(t.id), msg.to_string(), true).map_err(|e| format!("{e:#}"))?;
        if args.get("wait").and_then(Value::as_bool) == Some(true) {
            return self.wait(t.id, None, args, true);
        }
        Ok(format!("sent to {} ({})", t.id, t.agent.unwrap_or_default()))
    }

    fn wait(&self, term: TermId, regex: Option<&str>, args: &Value, just_sent: bool) -> Result<String, String> {
        if Some(term) == self.me {
            return Err("that's you".into());
        }
        let secs = args.get("timeout").and_then(Value::as_u64).unwrap_or(600);
        match cli::wait_on(term, regex, std::time::Duration::from_secs(secs), just_sent).map_err(|e| format!("{e:#}"))? {
            cli::Waited::Matched(l) => Ok(format!("matched: {l}")),
            cli::Waited::TimedOut => Err(format!("still going after {secs}s")),
            cli::Waited::Turn(s, said) => {
                let reply = if said.trim().is_empty() {
                    let screen = self.screen(term)?;
                    let lines: Vec<&str> = screen.trim_end().lines().collect();
                    lines[lines.len().saturating_sub(30)..].join("\n")
                } else {
                    said
                };
                let state = match s {
                    Status::Blocked => "needs an answer (see seshi_read / seshi_answer)",
                    _ => "finished",
                };
                Ok(format!("{term} {state}:\n{reply}"))
            }
        }
    }

    fn answer(&self, args: &Value) -> Result<String, String> {
        let t = self.session(args)?;
        let choice = args.get("choice").and_then(Value::as_u64).filter(|c| (1..=9).contains(c)).ok_or("choice must be 1-9")? as u32;
        let screen = self.screen(t.id)?;
        let (question, opts) = prompt_on(&screen);
        let label = opts.iter().find(|(n, _)| *n == choice).map(|(_, l)| l.clone()).unwrap_or_default();
        let saying_no = label.to_lowercase().starts_with("no");
        if !saying_no {
            match self.cfg.mcp.approve.as_str() {
                "always" => {}
                "safe" => {
                    let q = question.to_lowercase();
                    if !self.cfg.mcp.safe.iter().any(|s| !s.is_empty() && q.contains(&s.to_lowercase())) {
                        return Err(format!("\"{}\" isn't on the user's safe list, so seshi won't approve it. Ask the user.", question.trim()));
                    }
                }
                _ => return Err("The user hasn't let agents approve prompts (seshi Settings → Agents). Ask the user to answer it.".into()),
            }
        }
        cli::send(Some(t.id), choice.to_string(), false).map_err(|e| format!("{e:#}"))?;
        Ok(format!("answered {choice}{} on session {}", if label.is_empty() { String::new() } else { format!(" ({label})") }, t.id))
    }

    fn start(&self, args: &Value) -> Result<String, String> {
        let prompt = args.get("prompt").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).ok_or("give the task")?;
        let agent = args
            .get("agent")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| self.cfg.quick.agents.first().map(|a| a.name.clone()))
            .unwrap_or_else(|| "claude".into());
        let q = self.cfg.quote_for_shell(prompt);
        let cmd = match self.cfg.quick.agents.iter().find(|a| a.name == agent) {
            Some(a) => a.command.replace("{prompt}", &q),
            // Only agents seshi knows, by plain name: anything else would run as a command.
            None if known_agent(&self.cfg, &agent) => format!("{agent} {q}"),
            None => {
                let mut names: Vec<String> = self.cfg.quick.agents.iter().map(|a| a.name.clone()).collect();
                names.extend(self.cfg.agent_defs().into_iter().map(|d| d.name));
                names.dedup();
                return Err(format!("unknown agent \"{agent}\"; use one of: {}", names.join(", ")));
            }
        };
        let before = self.snapshot()?;
        let had: Vec<TermId> = before.terms.keys().copied().collect();
        let focus = before.active().and_then(|w| w.tab()).map(|t| t.focus);
        let worktree = args.get("worktree").and_then(Value::as_bool).unwrap_or(true) && crate::gitfs::head(&self.root).is_some();
        let branch = slug(prompt);
        let c = if worktree {
            let ws = before.active_ws.or_else(|| before.workspaces.first().map(|w| w.id)).ok_or("seshi has nothing open")?;
            let branch = if before.terms.values().any(|t| t.branch.as_deref() == Some(branch.as_str())) { format!("{branch}-{}", had.len()) } else { branch.clone() };
            Command::NewWorktree { ws, branch, base: None, cmd: Some(cmd), split: None, from: Some(self.root.clone()) }
        } else {
            Command::NewWorkspace { cwd: Some(self.root.clone()), name: None, cmd: Some(cmd) }
        };
        cli::block_on(cli::request(ClientMsg::Command(c))).map_err(|e| format!("{e:#}"))?;
        // Wait for it to appear, then give the user back the session they were on.
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let Ok(now) = self.snapshot() else { continue };
            if let Some(new) = now.terms.keys().copied().find(|id| !had.contains(id)) {
                if let Some(f) = focus.filter(|f| now.terms.contains_key(f)) {
                    let _ = cli::block_on(cli::request(ClientMsg::Command(Command::FocusPane { term: f })));
                }
                let place = if worktree { format!("its own worktree ({branch})") } else { self.root.display().to_string() };
                return Ok(format!("started {agent} as session {new} in {place}"));
            }
        }
        Ok(format!("asked seshi to start {agent}; it hasn't appeared yet"))
    }
}

fn slug(s: &str) -> String {
    let words: Vec<String> = s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(4).map(|w| w.to_ascii_lowercase()).collect();
    let s = words.join("-");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() { "agent-task".into() } else { s.trim_end_matches('-').to_string() }
}

/// Run the server on stdin / stdout until stdin closes.
pub fn run() -> Result<()> {
    let (cfg, _) = crate::config::Config::load_or_default();
    let server = Server { cfg, root: caller_root(), me: std::env::var("SESHI_TERM_ID").ok().and_then(|s| s.parse().ok()) };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no reply
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let reply = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "seshi", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "seshi runs your user's coding agents side by side. Use seshi_list to see them, seshi_read to see a screen, seshi_send to message one, seshi_start to start a helper in its own worktree. Don't answer prompts unless the user allows it."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(server.call(name, &args))
            }
            _ => Err(json!({ "code": -32601, "message": format!("no method {method}") })),
        };
        let resp = match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        writeln!(out, "{resp}")?;
        out.flush()?;
    }
    Ok(())
}

/// A built-in or configured agent kind, named plainly (letters, digits, - _ .).
fn known_agent(cfg: &crate::config::Config, name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && cfg.agent_defs().iter().any(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_prompt() {
        let screen = "• Read test/checkout.spec.ts\n\nRun npm test -- checkout?\n❯ 1. Yes\n  2. Yes, and always allow npm test\n  3. No, tell it what to do instead\n";
        let (q, opts) = prompt_on(screen);
        assert!(q.contains("Run npm test -- checkout?"));
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[2], (3, "No, tell it what to do instead".into()));
        assert_eq!(slug("Fix the flaky checkout test, please"), "fix-the-flaky-checkout");
    }

    #[test]
    fn only_known_agents_start() {
        let cfg = crate::config::Config::default();
        assert!(known_agent(&cfg, "claude") && known_agent(&cfg, "codex"));
        assert!(!known_agent(&cfg, "curl x|sh;"), "a command isn't an agent");
        assert!(!known_agent(&cfg, "not-an-agent"));
        assert!(!known_agent(&cfg, ""));
    }
}
