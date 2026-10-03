//! One-shot commands: scripting the multiplexer from a shell (or from an agent inside it).

use crate::ipc;
use crate::layout::Dir;
use crate::protocol::*;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) fn block_on<T>(f: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(f)
}

/// Send one message and wait for the reply.
pub(crate) async fn request(msg: ClientMsg) -> Result<Reply> {
    let (mut r, mut w) = ipc::open(false).await.context("no hydra server running")?;
    ipc::send(&mut w, &msg).await?;
    loop {
        match ipc::recv_server(&mut r).await? {
            Some(ServerMsg::Reply(reply)) => return Ok(reply),
            Some(ServerMsg::Error(e)) => bail!(e),
            // kill-server: the goodbye can overtake the reply.
            Some(ServerMsg::Bye) => return Ok(Reply::Ok),
            Some(ServerMsg::Notice(n)) => eprintln!("{n}"),
            Some(_) => continue,
            None => bail!("server closed the connection"),
        }
    }
}

fn snapshot() -> Result<Snapshot> {
    match block_on(request(ClientMsg::Query(Query::List)))? {
        Reply::List(s) => Ok(s),
        _ => bail!("unexpected reply"),
    }
}

fn command(c: Command) -> Result<()> {
    block_on(request(ClientMsg::Command(c))).map(|_| ())
}

/// An explicit pane, else the pane this command runs in, else the focused one.
fn resolve_pane(pane: Option<TermId>) -> Result<TermId> {
    if let Some(p) = pane {
        return Ok(p);
    }
    if let Some(p) = std::env::var("HYDRA_TERM_ID").ok().and_then(|s| s.parse().ok()) {
        return Ok(p);
    }
    let snap = snapshot()?;
    snap.active().and_then(|w| w.tab()).map(|t| t.focus).ok_or_else(|| anyhow!("no pane to target"))
}

pub fn ls(as_json: bool) -> Result<()> {
    let snap = snapshot()?;
    if as_json {
        let ws: Vec<Value> = snap
            .workspaces
            .iter()
            .map(|w| {
                json!({
                    "id": w.id, "name": w.name, "cwd": w.cwd, "active": Some(w.id) == snap.active_ws,
                    "git": w.git, "worktree": w.worktree, "color": w.color,
                    "tabs": w.tabs.iter().map(|t| json!({
                        "id": t.id, "name": t.name, "active": t.id == w.active_tab, "focus": t.focus,
                        "panes": t.layout.leaves().iter().filter_map(|id| snap.terms.get(id)).map(|p| json!({
                            "id": p.id, "process": p.process, "title": p.title, "agent": p.agent, "asleep": p.asleep,
                            "status": p.status.label(), "cols": p.cols, "rows": p.rows, "cwd": p.cwd,
                        })).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&ws)?);
        return Ok(());
    }
    for w in &snap.workspaces {
        let mark = if Some(w.id) == snap.active_ws { "*" } else { " " };
        let git = w.git.as_ref().map(|g| format!("  [{}{}]", g.branch, if g.dirty > 0 { format!(" ±{}", g.dirty) } else { String::new() })).unwrap_or_default();
        println!("{mark} {} {}{git}  ({})", w.id, w.name, w.cwd.display());
        for (i, t) in w.tabs.iter().enumerate() {
            let mark = if t.id == w.active_tab { "*" } else { " " };
            println!("  {mark} tab {} {}", i + 1, t.name);
            for id in t.layout.leaves() {
                let Some(p) = snap.terms.get(&id) else { continue };
                let focus = if id == t.focus { ">" } else { " " };
                let agent = p.agent.as_deref().map(|a| format!("  [{a}: {}]", p.status.label())).unwrap_or_default();
                println!("    {focus} pane {id}  {}{agent}", p.process);
            }
        }
    }
    Ok(())
}

pub fn read(pane: Option<TermId>) -> Result<()> {
    let term = resolve_pane(pane)?;
    match block_on(request(ClientMsg::Query(Query::Read { term })))? {
        Reply::Text(t) => {
            println!("{}", t.trim_end());
            Ok(())
        }
        _ => bail!("unexpected reply"),
    }
}

pub fn send(pane: Option<TermId>, text: String, enter: bool) -> Result<()> {
    let term = resolve_pane(pane)?;
    block_on(async move {
        let (_r, mut w) = ipc::open(false).await.context("no hydra server running")?;
        let data = if text.contains('\n') { format!("\x1b[200~{text}\x1b[201~") } else { text };
        ipc::send(&mut w, &ClientMsg::Input { term, data: data.into_bytes() }).await?;
        if enter {
            // A separate write so the program reads the text before the Enter.
            tokio::time::sleep(Duration::from_millis(30)).await;
            ipc::send(&mut w, &ClientMsg::Input { term, data: b"\r".to_vec() }).await?;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        Ok(())
    })
}

/// Send key presses (`ctrl+space`, `%`, `enter`), encoded the way a terminal would.
pub fn send_keys(pane: Option<TermId>, keys: Vec<String>) -> Result<()> {
    let term = resolve_pane(pane)?;
    let mut chunks = Vec::new();
    for k in &keys {
        let spec: crate::keys::KeySpec = k.parse()?;
        let ev = ratatui::crossterm::event::KeyEvent::new(spec.code, spec.mods);
        chunks.push(crate::keys::encode(&ev, false));
    }
    block_on(async move {
        let (_r, mut w) = ipc::open(false).await.context("no hydra server running")?;
        for data in chunks {
            ipc::send(&mut w, &ClientMsg::Input { term, data }).await?;
            // One key per beat, like a person typing; lets modes change between keys.
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        Ok(())
    })
}

fn join_command(parts: Vec<String>) -> Option<String> {
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub fn split(pane: Option<TermId>, down: bool, command: Vec<String>) -> Result<()> {
    let term = resolve_pane(pane)?;
    let dir = if down { Dir::Down } else { Dir::Right };
    self::command(Command::Split { term, dir, cmd: join_command(command), cwd: None })
}

pub fn new_workspace(path: Option<PathBuf>, name: Option<String>, cmd: Vec<String>) -> Result<()> {
    let cwd = path.or_else(|| std::env::current_dir().ok()).map(|p| p.canonicalize().unwrap_or(p));
    command(Command::NewWorkspace { cwd, name, cmd: join_command(cmd) })
}

pub fn focus(pane: TermId) -> Result<()> {
    command(Command::FocusPane { term: pane })
}

pub fn close(pane: Option<TermId>) -> Result<()> {
    command(Command::ClosePane { term: resolve_pane(pane)? })
}

pub fn kill_server(forget: bool) -> Result<()> {
    command(Command::KillServer { forget })
}

/// The workspace this command runs in (via its pane), else the active one.
fn resolve_ws(ws: Option<WsId>) -> Result<WsId> {
    if let Some(ws) = ws {
        return Ok(ws);
    }
    let snap = snapshot()?;
    let here = std::env::var("HYDRA_TERM_ID").ok().and_then(|s| s.parse().ok());
    here.and_then(|t| snap.locate(t).map(|(w, _)| w.id))
        .or(snap.active_ws)
        .ok_or_else(|| anyhow!("no workspace to target"))
}

pub fn worktree(branch: String, base: Option<String>, ws: Option<WsId>, cmd: Vec<String>) -> Result<()> {
    let ws = resolve_ws(ws)?;
    command(Command::NewWorktree { ws, branch, base, cmd: join_command(cmd), split: None, from: None })
}

pub fn worktree_remove(ws: Option<WsId>, force: bool) -> Result<()> {
    command(Command::RemoveWorktree { ws: resolve_ws(ws)?, force, delete_branch: false })
}

pub fn config_init() -> Result<()> {
    let path = crate::config::config_path();
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, crate::config::EXAMPLE)?;
    println!("wrote {}", path.display());
    Ok(())
}

// ---- hooks -------------------------------------------------------------------------

/// Map a Claude Code (or Codex, same dialect) hook payload to a status.
pub fn status_from_hook(payload: &Value) -> Option<HookStatus> {
    let event = payload.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("");
    Some(match event {
        "PreToolUse" if tool == "AskUserQuestion" => HookStatus::Blocked,
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "SubagentStart" => HookStatus::Working,
        "SubagentStop" => HookStatus::Same,
        "PermissionRequest" => HookStatus::Blocked,
        "Notification" => {
            let kind = payload.get("notification_type").and_then(Value::as_str).unwrap_or("");
            let msg = payload.get("message").and_then(Value::as_str).unwrap_or("").to_lowercase();
            match kind {
                "permission_prompt" | "elicitation_dialog" => HookStatus::Blocked,
                "idle_prompt" => HookStatus::Done,
                _ if msg.contains("permission") => HookStatus::Blocked,
                _ => return None,
            }
        }
        "Stop" => HookStatus::Done,
        "SessionStart" => HookStatus::Idle,
        "SessionEnd" => HookStatus::Gone,
        // Codex `notify`.
        _ if payload.get("type").and_then(Value::as_str) == Some("agent-turn-complete") => HookStatus::Done,
        _ => return None,
    })
}

/// The last thing the assistant wrote, from a Claude Code transcript (JSON lines).
fn last_assistant_text(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // Only the tail matters; transcripts can be large.
    let start = len.saturating_sub(512 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    buf.lines().rev().find_map(|line| {
        let v: Value = serde_json::from_str(line).ok()?;
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return None;
        }
        let content = v.pointer("/message/content")?.as_array()?;
        let text: Vec<&str> = content
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect();
        (!text.is_empty()).then(|| text.join("\n"))
    })
}

/// Collapse whitespace and cut to `max` characters.
fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn parse_status(s: &str) -> Option<HookStatus> {
    Some(match s {
        "working" | "busy" | "running" => HookStatus::Working,
        "blocked" | "waiting" | "input" => HookStatus::Blocked,
        "done" | "finished" => HookStatus::Done,
        "idle" => HookStatus::Idle,
        "gone" | "exit" | "end" => HookStatus::Gone,
        _ => return None,
    })
}

pub fn hook(agent: &str, status: Option<&str>, payload: Option<&str>) -> Result<()> {
    // Inert outside hydra, so global hooks don't bother other terminals.
    let Some(term) = std::env::var("HYDRA_TERM_ID").ok().and_then(|s| s.parse::<TermId>().ok()) else {
        return Ok(());
    };
    let mut payload_json = Value::Null;
    let status = match status {
        Some(s) => parse_status(s).ok_or_else(|| anyhow!("unknown status {s}"))?,
        None => {
            let raw = match payload {
                Some(p) => p.to_string(),
                None => {
                    let mut s = String::new();
                    std::io::stdin().take(1 << 20).read_to_string(&mut s)?;
                    s
                }
            };
            payload_json = serde_json::from_str(&raw).unwrap_or(Value::Null);
            match status_from_hook(&payload_json) {
                Some(s) => s,
                None => return Ok(()),
            }
        }
    };
    let field = |names: &[&str]| names.iter().find_map(|n| payload_json.get(*n)?.as_str().map(str::to_string));
    let session = field(&["session_id", "thread-id", "thread_id", "conversation_id"]);
    let cwd = field(&["cwd"]).map(PathBuf::from);
    let event = field(&["hook_event_name"]).unwrap_or_default();
    let subagent = matches!(event.as_str(), "SubagentStart" | "SubagentStop").then(|| Subagent {
        id: field(&["agent_id", "subagent_id", "tool_use_id"]).unwrap_or_default(),
        kind: field(&["agent_type", "subagent_type", "agent_name"]).unwrap_or_else(|| "subagent".into()),
        start: event == "SubagentStart",
    });
    let prompt = field(&["prompt"]).map(|p| one_line(&p, 160));
    let said = field(&["last_assistant_message", "last-assistant-message"])
        .or_else(|| field(&["transcript_path"]).and_then(|p| last_assistant_text(std::path::Path::new(&p))))
        .map(|s| s.trim().chars().take(2000).collect::<String>())
        .filter(|s| !s.is_empty());
    block_on(async move {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (_r, mut w) = ipc::open(false).await?;
            ipc::send(&mut w, &ClientMsg::Hook { term, agent: agent.to_string(), status, session, cwd, prompt, said, subagent }).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?
    })
}

const CLAUDE_EVENTS: &[(&str, Option<&str>)] = &[
    ("UserPromptSubmit", None),
    ("PreToolUse", None),
    ("PostToolUse", None),
    ("PermissionRequest", None),
    ("Notification", None),
    ("Stop", None),
    ("SubagentStart", None),
    ("SubagentStop", None),
    ("SessionStart", None),
    ("SessionEnd", None),
];

fn is_ours(group: &Value) -> bool {
    ["_hydra", "_drover"].iter().any(|tag| group.get(*tag).and_then(Value::as_bool) == Some(true))
}

pub fn integrate(agent: &str, uninstall: bool) -> Result<()> {
    let exe = std::env::current_exe()?.to_string_lossy().replace('\\', "/");
    match agent {
        "claude" => {
            let dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| {
                directories::BaseDirs::new().map(|d| d.home_dir().join(".claude")).unwrap_or_default()
            });
            let path = dir.join("settings.json");
            let mut root: Value = match std::fs::read_to_string(&path) {
                Ok(s) if s.trim().is_empty() => json!({}),
                Ok(s) => serde_json::from_str(&s)
                    .with_context(|| format!("{} isn't valid JSON; not touching it", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
                Err(e) => return Err(e.into()),
            };
            if path.exists() {
                std::fs::copy(&path, path.with_extension("json.hydra-bak"))?;
            }
            let hooks = root
                .as_object_mut()
                .ok_or_else(|| anyhow!("settings.json is not an object"))?
                .entry("hooks")
                .or_insert_with(|| json!({}));
            let hooks = hooks.as_object_mut().ok_or_else(|| anyhow!("`hooks` is not an object"))?;
            for (event, matcher) in CLAUDE_EVENTS {
                let groups = hooks.entry(*event).or_insert_with(|| json!([]));
                let Some(arr) = groups.as_array_mut() else { continue };
                arr.retain(|g| !is_ours(g));
                if !uninstall {
                    let mut g = json!({
                        "_hydra": true,
                        "hooks": [{ "type": "command", "command": format!("\"{exe}\" hook claude"), "timeout": 5 }],
                    });
                    if let Some(m) = matcher {
                        g["matcher"] = json!(m);
                    }
                    arr.push(g);
                }
            }
            hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&path, serde_json::to_string_pretty(&root)? + "\n")?;
            if uninstall {
                println!("removed hydra hooks from {}", path.display());
            } else {
                println!("installed hydra hooks into {}", path.display());
                println!("they only act inside hydra panes (HYDRA_TERM_ID), so other terminals are unaffected.");
            }
            Ok(())
        }
        "mcp" => {
            // Claude Code: register for every project (user scope).
            let args = ["mcp", "add", "--scope", "user", "hydra", "--", exe.as_str(), "mcp"];
            let added = std::process::Command::new(if cfg!(windows) { "claude.cmd" } else { "claude" })
                .args(args)
                .status()
                .or_else(|_| std::process::Command::new("claude").args(args).status());
            match added {
                Ok(s) if s.success() => println!("added the hydra MCP server to Claude Code (all projects)"),
                _ => println!("Claude Code: run  claude mcp add --scope user hydra -- \"{exe}\" mcp"),
            }
            println!("\nCodex: add to ~/.codex/config.toml\n\n[mcp_servers.hydra]\ncommand = \"{exe}\"\nargs = [\"mcp\"]\n");
            println!("Agents can then list, read, message and start sessions. Whether they may approve");
            println!("prompts is up to you: hydra Settings → Agents (never, by default).");
            Ok(())
        }
        "codex" => {
            println!("Add this line to ~/.codex/config.toml (top level):\n");
            println!("notify = [\"{exe}\", \"hook\", \"codex\"]\n");
            println!("Codex then reports finished turns; working/blocked come from screen detection.");
            Ok(())
        }
        other => bail!("no integration for `{other}`; any agent can call `hydra hook {other} --status <working|blocked|done|idle>`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hook_mapping() {
        let s = |v: Value| status_from_hook(&v);
        assert_eq!(s(json!({"hook_event_name": "UserPromptSubmit"})), Some(HookStatus::Working));
        assert_eq!(s(json!({"hook_event_name": "Stop"})), Some(HookStatus::Done));
        assert_eq!(
            s(json!({"hook_event_name": "Notification", "notification_type": "permission_prompt"})),
            Some(HookStatus::Blocked)
        );
        assert_eq!(s(json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion"})), Some(HookStatus::Blocked));
        assert_eq!(s(json!({"type": "agent-turn-complete"})), Some(HookStatus::Done));
        assert_eq!(s(json!({"hook_event_name": "Notification", "notification_type": "auth_success"})), None);
    }
}

/// Debugging: the colours a pane's program is drawing (rows with a background colour).
pub fn debug_colors(pane: TermId) -> Result<()> {
    block_on(async move {
        let (mut r, _w) = ipc::open(true).await.context("no hydra server running")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while let Ok(Ok(Some(msg))) = tokio::time::timeout_at(deadline, ipc::recv_server(&mut r)).await {
            if let ServerMsg::Replay { term, cols, rows, data } = msg
                && term == pane
            {
                let mut p = vt100::Parser::new(rows, cols, 0);
                p.process(&data);
                let s = p.screen();
                println!("alternate screen: {}  mouse: {:?} / {:?}", s.alternate_screen(), s.mouse_protocol_mode(), s.mouse_protocol_encoding());
                let mut h = vt100::Parser::new(rows, cols, 100_000);
                h.process(&data);
                h.screen_mut().set_scrollback(usize::MAX);
                println!("history lines: {} (replay {} bytes)", h.screen().scrollback(), data.len());
                for row in 0..rows {
                    let mut seen: Vec<String> = Vec::new();
                    for col in 0..cols {
                        let Some(c) = s.cell(row, col) else { continue };
                        if c.bgcolor() == vt100::Color::Default && !c.inverse() {
                            continue;
                        }
                        let d = format!("fg={:?} bg={:?} inv={} dim={}", c.fgcolor(), c.bgcolor(), c.inverse(), c.dim());
                        if !seen.contains(&d) {
                            seen.push(d);
                        }
                    }
                    if !seen.is_empty() {
                        let text: String = s.rows(0, cols).nth(row as usize).unwrap_or_default().chars().take(40).collect();
                        println!("row {row:>3} {text:<40} {}", seen.join(" | "));
                    }
                }
                return Ok(());
            }
        }
        anyhow::bail!("no pane {pane}")
    })
}
