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
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let out = rt.block_on(f);
    // Don't wait on reads still blocked on a pipe (ssh's, over --remote).
    rt.shutdown_background();
    out
}

/// Send one message and wait for the reply.
pub(crate) async fn request(msg: ClientMsg) -> Result<Reply> {
    let (mut r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the seshi server" })?;
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

pub(crate) fn snapshot() -> Result<Snapshot> {
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
    if let Some(p) = std::env::var("SESHI_TERM_ID").ok().and_then(|s| s.parse().ok()) {
        return Ok(p);
    }
    let snap = snapshot()?;
    snap.active().and_then(|w| w.tab()).map(|t| t.focus).ok_or_else(|| anyhow!("no pane to target"))
}

/// Arguments that mean the agent isn't starting a new piece of work: help, version, a
/// one-shot print, picking up an earlier conversation (which lives in the folder it was in),
/// or one of its subcommands.
const NOT_NEW_WORK: [&str; 13] = ["-h", "--help", "-v", "-V", "--version", "-p", "--print", "-c", "--continue", "-r", "--resume", "-w", "--worktree"];
const SUBCOMMANDS: [&str; 20] = [
    "resume", "exec", "e", "login", "logout", "mcp", "mcp-server", "config", "update", "upgrade", "doctor", "install", "plugin", "setup-token",
    "migrate-installer", "completion", "apply", "sandbox", "debug", "app-server",
];

/// Whether `agent args…` starts new work (which gets its own worktree).
pub(crate) fn starts_new_work(args: &[String]) -> bool {
    let sub = args.first().is_some_and(|a| SUBCOMMANDS.contains(&a.as_str()));
    !sub && !args.iter().any(|a| NOT_NEW_WORK.contains(&a.as_str()) || a.starts_with("--resume=") || a.starts_with("--worktree="))
}

/// `seshi agent-dir <agent> [args…]`, run by a pane's shell just before it starts an agent:
/// prints the folder to start in (a new worktree), or nothing to start where it is. Never
/// fails: the agent starts either way.
pub fn agent_dir(cmd: &[String]) {
    let args = cmd.get(1..).unwrap_or_default();
    if std::env::var_os("SESHI_TERM_ID").is_none() || !starts_new_work(args) {
        return;
    }
    let Ok(dir) = std::env::current_dir() else { return };
    match block_on(request(ClientMsg::Command(Command::AgentWorktree { dir }))) {
        Ok(Reply::Text(path)) if !path.is_empty() => {
            eprintln!("seshi: {} gets its own worktree: {path}", cmd.first().map(String::as_str).unwrap_or("the agent"));
            println!("{path}");
        }
        Ok(_) => {}
        Err(e) => eprintln!("seshi: no worktree for it ({e:#}), so it starts here"),
    }
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
                            "id": p.id, "process": p.process, "title": p.title, "agent": p.agent, "asleep": p.asleep, "win32_input": p.win32_input, "dev": p.dev, "model": p.model, "name": p.name, "mem_mb": p.mem >> 20, "bell": p.bell,
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

/// The model the agent last answered with and the name given to the conversation, from the
/// end of a Claude transcript (JSON lines).
pub fn transcript_facts(path: &std::path::Path) -> (Option<String>, Option<String>) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return (None, None) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(512 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    transcript_facts_in(&String::from_utf8_lossy(&buf))
}

/// The name: what you called it (/rename) wins over the title Claude made up.
pub fn transcript_facts_in(text: &str) -> (Option<String>, Option<String>) {
    let (mut model, mut name, mut ai) = (None, None, None);
    for line in text.lines().rev() {
        if model.is_some() && name.is_some() {
            break;
        }
        let maybe_model = model.is_none() && line.contains("\"model\"");
        let maybe_name = name.is_none() && line.contains("\"customTitle\"");
        let maybe_ai = ai.is_none() && line.contains("\"aiTitle\"");
        if !maybe_model && !maybe_name && !maybe_ai {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let title = |k: &str| v.get(k).and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(|t| one_line(t, 80));
        if maybe_name {
            name = title("customTitle");
        }
        if maybe_ai {
            ai = title("aiTitle");
        }
        if maybe_model
            && v.get("type").and_then(Value::as_str) == Some("assistant")
            && let Some(m) = v.pointer("/message/model").and_then(Value::as_str).filter(|m| !m.starts_with('<'))
        {
            model = Some(short_model(m));
        }
    }
    (model, name.or(ai))
}

/// "claude-opus-4-5-20251101" → "opus 4.5"; other names as they are.
pub fn short_model(m: &str) -> String {
    let Some(rest) = m.strip_prefix("claude-") else { return m.to_string() };
    let parts: Vec<&str> = rest.split('-').filter(|p| p.len() < 8).collect();
    let family: Vec<&str> = parts.iter().copied().filter(|p| !p.chars().all(|c| c.is_ascii_digit())).collect();
    let version: Vec<&str> = parts.iter().copied().filter(|p| p.chars().all(|c| c.is_ascii_digit())).collect();
    let mut out = family.join(" ");
    if !version.is_empty() {
        out.push(' ');
        out.push_str(&version.join("."));
    }
    if out.is_empty() { m.to_string() } else { out }
}

/// What a wait ended on.
#[derive(Debug, Clone, PartialEq)]
pub enum Waited {
    /// The turn ended: its status then, and its last message (if hooks said).
    Turn(Status, String),
    /// The regex matched this line.
    Matched(String),
    TimedOut,
}

/// Wait on pane `term`: for its turn to end (it was working, now it isn't; a prompt that
/// never starts a turn counts as ended after a few seconds), or for `regex` on its screen.
/// `just_sent`: the turn hasn't started yet, so wait for it to start first.
pub fn wait_on(term: TermId, regex: Option<&str>, timeout: Duration, just_sent: bool) -> Result<Waited> {
    let re = regex.map(|r| regex::RegexBuilder::new(r).case_insensitive(true).build()).transpose().context("bad --regex")?;
    let start = std::time::Instant::now();
    let mut seen_working = !just_sent;
    loop {
        if let Some(re) = &re {
            if let Reply::Text(screen) = block_on(request(ClientMsg::Query(Query::Read { term })))?
                && let Some(l) = screen.lines().find(|l| re.is_match(l))
            {
                return Ok(Waited::Matched(l.trim().to_string()));
            }
        } else {
            let snap = snapshot()?;
            let t = snap.terms.get(&term).ok_or_else(|| anyhow!("pane {term} is gone"))?;
            match t.status {
                Status::Working => seen_working = true,
                s @ (Status::Blocked | Status::Done | Status::Idle) if seen_working || start.elapsed() > Duration::from_secs(8) => {
                    return Ok(Waited::Turn(s, t.said.clone()));
                }
                // An agent without hooks: no status at all, so wait on output going quiet.
                Status::None if start.elapsed() > Duration::from_secs(8) => return Ok(Waited::Turn(Status::None, String::new())),
                _ => {}
            }
        }
        if start.elapsed() >= timeout {
            return Ok(Waited::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

pub fn send_wait(pane: Option<TermId>, text: String, enter: bool, timeout: u64) -> Result<()> {
    let term = resolve_pane(pane)?;
    send(Some(term), text, enter)?;
    wait_print(term, None, timeout, true)
}

pub fn wait(pane: Option<TermId>, regex: Option<String>, timeout: u64) -> Result<()> {
    wait_print(resolve_pane(pane)?, regex, timeout, false)
}

/// `seshi wait` / `seshi send --wait`: wait, then print the reply (or the screen's end).
fn wait_print(term: TermId, regex: Option<String>, timeout: u64, just_sent: bool) -> Result<()> {
    match wait_on(term, regex.as_deref(), Duration::from_secs(timeout), just_sent)? {
        Waited::Matched(l) => println!("{l}"),
        Waited::Turn(s, said) => {
            if s == Status::Blocked {
                eprintln!("(pane {term} needs you)");
            }
            if said.trim().is_empty() {
                if let Reply::Text(screen) = block_on(request(ClientMsg::Query(Query::Read { term })))? {
                    let lines: Vec<&str> = screen.trim_end().lines().collect();
                    println!("{}", lines[lines.len().saturating_sub(30)..].join("\n"));
                }
            } else {
                println!("{}", said.trim_end());
            }
            if s == Status::Blocked {
                std::process::exit(3);
            }
        }
        Waited::TimedOut => {
            eprintln!("timed out waiting on pane {term}");
            std::process::exit(2);
        }
    }
    Ok(())
}

pub fn send(pane: Option<TermId>, text: String, enter: bool) -> Result<()> {
    let term = resolve_pane(pane)?;
    block_on(async move {
        let (_r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the seshi server" })?;
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
        let (_r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the seshi server" })?;
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

/// The session you're on, if a server is running.
pub fn focused_pane() -> Option<TermId> {
    let s = snapshot().ok()?;
    s.workspaces.iter().find(|w| Some(w.id) == s.active_ws)?.tab().map(|t| t.focus)
}

/// `seshi popup -- <command>`: a floating pane in the folder you're in.
pub fn popup(cmd: Vec<String>) -> Result<()> {
    let cmd = join_command(cmd).ok_or_else(|| anyhow!("what to run: seshi popup -- fzf"))?;
    command(Command::Popup { cmd, cwd: std::env::current_dir().ok() })
}

/// `seshi grant <pane> read,write,…|default`.
pub fn grant(term: TermId, grants: &str) -> Result<()> {
    let list = (grants.trim() != "default").then(|| grants.split(',').map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty()).collect::<Vec<_>>());
    command(Command::Grant { term, grants: list.clone() })?;
    match list {
        Some(l) if l.is_empty() => println!("pane {term} may do nothing through seshi but its own work"),
        Some(l) => println!("pane {term} may: {}", l.join(", ")),
        None => println!("pane {term} is back to the default grants ([mcp] grants in config)"),
    }
    Ok(())
}

/// `seshi ask-human "…?" -o Yes -o No`: ask the person, wait, print the answer.
pub fn ask_human(text: String, options: Vec<String>) -> Result<()> {
    let term = resolve_pane(None).context("run it in a seshi pane (the question shows there)")?;
    match block_on(request(ClientMsg::Command(Command::AskHuman { term, text, options })))? {
        Reply::Text(answer) => {
            println!("{answer}");
            Ok(())
        }
        _ => bail!("no answer"),
    }
}

/// `seshi teach [pane]`: the program running there is an agent.
pub fn teach(pane: Option<TermId>) -> Result<()> {
    let term = resolve_pane(pane)?;
    command(Command::TeachAgent { term })?;
    println!("seshi knows it as an agent now (see [[agents]] in {})", crate::config::config_path().display());
    Ok(())
}

/// Focus a pane and bring seshi's window forward (what clicking a notification does).
pub fn reveal(pane: TermId) -> Result<()> {
    command(Command::Reveal { term: pane })
}

pub fn close(pane: Option<TermId>) -> Result<()> {
    command(Command::ClosePane { term: resolve_pane(pane)? })
}

/// Whether a seshi server is running (for this socket).
pub(crate) fn server_running() -> bool {
    block_on(async { ipc::connect().await.map(|_| ()) }).is_ok()
}

pub fn kill_server(forget: bool) -> Result<()> {
    if ipc::remote().is_none() && !server_running() {
        if forget {
            crate::daemon::forget_session();
        }
        println!("no seshi server is running; nothing to stop");
        return Ok(());
    }
    match command(Command::KillServer { forget }) {
        // Another version can't be asked to stop: stop its process instead.
        Err(e) if e.chain().any(|c| c.is::<ipc::OtherVersion>()) => {
            let pid = stop_server_process()?;
            if forget {
                crate::daemon::forget_session();
            }
            println!("stopped the server (another seshi version, process {pid})");
            Ok(())
        }
        r => r,
    }
}

/// Stop this socket's server by its process, for when it's a version that can't be talked
/// to. Its saved session stays, as with `kill-server`.
fn stop_server_process() -> Result<u32> {
    let pid = server_pid().ok_or_else(|| {
        anyhow!("couldn't find the server's process; stop it yourself (on Linux/macOS: pkill -f \"seshi daemon\"; on Windows: end seshi.exe in Task Manager)")
    })?;
    let killed = if cfg!(windows) {
        crate::proc::run(std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]))
    } else {
        crate::proc::run(std::process::Command::new("kill").arg(pid.to_string()))
    };
    killed.map_err(|e| anyhow!("couldn't stop the server (process {pid}): {e}"))?;
    // Wait for it to let go of its socket, so the next start doesn't find it.
    for _ in 0..40 {
        if block_on(ipc::connect()).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(ipc::pid_file());
    Ok(pid)
}

/// This socket's server process: from the id it noted, or (servers too old to note it) the
/// one running `seshi daemon` for this socket.
fn server_pid() -> Option<u32> {
    if let Some(pid) = std::fs::read_to_string(ipc::pid_file()).ok().and_then(|s| s.trim().parse().ok()) {
        return Some(pid);
    }
    #[cfg(target_os = "linux")]
    {
        let label = std::env::var("SESHI_SOCKET").unwrap_or_else(|_| "default".into());
        let me = std::process::id();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            if pid == me {
                continue;
            }
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
            let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
            let is_daemon = args.first().is_some_and(|a| a.ends_with(b"seshi")) && args.get(1) == Some(&b"daemon".as_slice());
            if !is_daemon {
                continue;
            }
            // Its socket: SESHI_SOCKET in its environment, else the default one.
            let env = std::fs::read(entry.path().join("environ")).unwrap_or_default();
            let theirs = env
                .split(|b| *b == 0)
                .find_map(|kv| kv.strip_prefix(b"SESHI_SOCKET="))
                .map(|v| String::from_utf8_lossy(v).into_owned())
                .unwrap_or_else(|| "default".into());
            if theirs == label {
                return Some(pid);
            }
        }
    }
    // Elsewhere a server's socket can't be read off its process: only when there's exactly
    // one, and it's the default server you mean.
    #[cfg(not(target_os = "linux"))]
    if std::env::var("SESHI_SOCKET").is_err() {
        let list = if cfg!(windows) {
            crate::proc::run(std::process::Command::new("powershell").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='seshi.exe'\" | Where-Object { $_.CommandLine -match '\\sdaemon\\s*$' } | ForEach-Object { $_.ProcessId }",
            ]))
        } else {
            crate::proc::run(std::process::Command::new("pgrep").args(["-f", "seshi daemon$"]))
        };
        let pids: Vec<u32> = list.unwrap_or_default().split_whitespace().filter_map(|p| p.parse().ok()).filter(|p| *p != std::process::id()).collect();
        if let [pid] = pids[..] {
            return Some(pid);
        }
    }
    None
}

/// The workspace this command runs in (via its pane), else the active one.
fn resolve_ws(ws: Option<WsId>) -> Result<WsId> {
    if let Some(ws) = ws {
        return Ok(ws);
    }
    let snap = snapshot()?;
    let here = std::env::var("SESHI_TERM_ID").ok().and_then(|s| s.parse().ok());
    here.and_then(|t| snap.locate(t).map(|(w, _)| w.id))
        .or(snap.active_ws)
        .ok_or_else(|| anyhow!("no workspace to target"))
}

pub fn worktree(branch: String, base: Option<String>, ws: Option<WsId>, cmd: Vec<String>) -> Result<()> {
    let ws = resolve_ws(ws)?;
    command(Command::NewWorktree { ws, branch, base, cmd: join_command(cmd), split: None, from: None })
}


/// `seshi doctor`: each thing seshi relies on, ✓ or what to do about it.
pub fn doctor() -> Result<()> {
    let mut bad = 0;
    let mut line = |ok: Option<bool>, what: &str, detail: String| {
        let mark = match ok {
            Some(true) => "\x1b[32m✓\x1b[0m",
            Some(false) => {
                bad += 1;
                "\x1b[31m✕\x1b[0m"
            }
            None => "\x1b[33m·\x1b[0m",
        };
        println!(" {mark} {what:<22} {detail}");
    };
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let mut c = std::process::Command::new(cmd);
        c.args(args);
        let out = c.output().ok().filter(|o| o.status.success())?;
        Some(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string())
    };
    let on_path = |name: &str| -> Option<String> {
        let names = if cfg!(windows) { vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.ps1"), name.to_string()] } else { vec![name.to_string()] };
        std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p).find_map(|d| names.iter().map(|n| d.join(n)).find(|f| f.is_file()).map(|f| f.display().to_string()))
        })
    };

    println!("seshi {}  (protocol {})\n", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION);

    // Config
    let path = crate::config::config_path();
    match crate::config::Config::load() {
        Ok(_) if path.exists() => line(Some(true), "config", path.display().to_string()),
        Ok(_) => line(None, "config", format!("none yet (defaults); `seshi config init` writes one at {}", path.display())),
        Err(e) => line(Some(false), "config", format!("{e:#}  ({})", path.display())),
    }
    let (cfg, _) = crate::config::Config::load_or_default();

    // Server
    let server = block_on(async { ipc::open(false).await });
    match server {
        Ok(_) => line(Some(true), "server", format!("running ({})", ipc::socket_id())),
        Err(e) if format!("{e:#}").contains("protocol") => {
            line(Some(false), "server", format!("{e:#}"));
            println!("{:27}fix: close seshi, `seshi kill-server`, start it again", "");
        }
        Err(_) => line(None, "server", "not running (it starts with `seshi`)".into()),
    }

    // Tools
    match run("git", &["--version"]) {
        Some(v) => line(Some(true), "git", v),
        None => line(Some(false), "git", "not found; worktrees, Changes and branches need it".into()),
    }
    match run("gh", &["--version"]) {
        Some(v) => line(Some(true), "gh (GitHub CLI)", v),
        None => line(None, "gh (GitHub CLI)", "not found; PRs and GitHub issues use it (optional)".into()),
    }
    let shell = cfg.shell_command();
    let shell_ok = on_path(&shell[0]).is_some() || std::path::Path::new(&shell[0]).is_file();
    line(Some(shell_ok), "shell", shell.join(" "));

    // Agents
    let mut found = Vec::new();
    for a in ["claude", "codex", "gemini", "opencode", "cursor-agent", "copilot", "amp", "qwen", "aider", "grok", "auggie", "kimi"] {
        if on_path(a).is_some() {
            found.push(a);
        }
    }
    line(Some(!found.is_empty()), "agents on PATH", if found.is_empty() { "none found (claude, codex, gemini, …)".into() } else { found.join(", ") });

    // Claude: hooks and MCP
    if found.contains(&"claude") {
        let settings = std::fs::read_to_string(claude_settings()).unwrap_or_default();
        let hooked = settings.contains("hook claude");
        let stale = stale_claude_hooks();
        match (hooked, stale.first()) {
            (false, _) => line(Some(false), "claude status hooks", "missing: `seshi integrate claude` (exact working / needs-you / done)".into()),
            // Another seshi can't talk to this one's server: every agent would look idle.
            (true, Some(other)) => line(Some(false), "claude status hooks", format!("they run another seshi ({other}); `seshi integrate claude` (or restart the server) fixes it")),
            (true, None) => line(Some(true), "claude status hooks", "installed".into()),
        }
        // Any hook whose program is gone (often an old hydra's): Claude shows "hook error" on
        // every turn it runs in, and only removing or repointing it stops that.
        let mut broken = Vec::new();
        for file in claude_settings_files() {
            let Some(root) = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { continue };
            for cmd in broken_hooks_in(&root, |p| p.exists(), |n| on_path(n).is_some()) {
                broken.push(format!("{} in {}", program_of(&cmd), file.display()));
            }
        }
        match broken.first() {
            None => line(Some(true), "claude hooks run", "every hook's program is there".into()),
            Some(first) => line(
                Some(false),
                "claude hooks run",
                format!("{} missing ({first}{}); `seshi integrate claude` fixes seshi's own, remove the rest", broken.len(), if broken.len() > 1 { ", …" } else { "" }),
            ),
        }
        let home = directories::BaseDirs::new().map(|d| d.home_dir().join(".claude.json"));
        let mcp = home.and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains("\"seshi\"") && s.contains("\"mcp\""));
        line(if mcp { Some(true) } else { None }, "seshi MCP for claude", if mcp { "registered".into() } else { "not set up: `seshi integrate mcp` lets agents see each other (optional)".into() });
    }

    // Terminal
    let term = std::env::var("TERM_PROGRAM").or_else(|_| std::env::var("TERM")).unwrap_or_else(|_| if std::env::var("WT_SESSION").is_ok() { "Windows Terminal".into() } else { "unknown".into() });
    let truecolor = std::env::var("COLORTERM").is_ok_and(|c| c.contains("truecolor") || c.contains("24bit")) || std::env::var("WT_SESSION").is_ok();
    line(if truecolor { Some(true) } else { None }, "terminal", format!("{term}{}", if truecolor { ", true colour" } else { " (colours may look off without true colour)" }));
    if let Ok((w, h)) = crossterm::terminal::size() {
        line(Some(w >= 100 && h >= 30), "window size", format!("{w}×{h}{}", if w < 100 || h < 30 { " (seshi wants at least 100×30)" } else { "" }));
    }

    // Data and sync
    let data = crate::config::data_dir();
    let writable = std::fs::create_dir_all(&data).is_ok() && std::fs::write(data.join(".doctor"), b"ok").is_ok();
    let _ = std::fs::remove_file(data.join(".doctor"));
    line(Some(writable), "data folder", data.display().to_string());
    if let Some(ed) = Some(cfg.editor.clone()).filter(|e| !e.is_empty()).or_else(|| std::env::var("VISUAL").ok()).or_else(|| std::env::var("EDITOR").ok()) {
        line(Some(true), "editor", ed);
    } else {
        line(None, "editor", "none set (editor = \"code\" in config, or $EDITOR)".into());
    }

    println!();
    if bad == 0 {
        println!("All good.");
    } else {
        println!("{bad} thing(s) to fix above.");
    }
    Ok(())
}

/// `seshi dev [start|stop|restart]`: this checkout's dev server.
pub fn dev(action: &str, dir: Option<PathBuf>) -> Result<()> {
    let action = match action {
        "start" | "run" => DevAction::Start,
        "stop" => DevAction::Stop,
        "restart" => DevAction::Restart,
        other => bail!("{other}? use start, stop or restart"),
    };
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let dir = crate::gitfs::head(&dir).map(|h| h.top).unwrap_or(dir);
    command(Command::Dev { dir, action })
}

/// Run by an agent inside a pane: make a worktree and move this agent into it.
pub fn move_to_worktree(branch: String) -> Result<()> {
    let term: TermId = std::env::var("SESHI_TERM_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("run this from inside a seshi pane (an agent running in seshi)"))?;
    let branch = Some(branch.trim().to_string()).filter(|b| !b.is_empty());
    block_on(async move {
        let (mut r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the seshi server" })?;
        ipc::send(&mut w, &ClientMsg::Command(Command::MoveToWorktree { term, branch })).await?;
        loop {
            match ipc::recv_server(&mut r).await? {
                Some(ServerMsg::Notice(n)) => println!("{n}"),
                Some(ServerMsg::Reply(_)) => return Ok(()),
                Some(ServerMsg::Error(e)) => bail!(e),
                Some(_) => continue,
                None => bail!("server closed the connection"),
            }
        }
    })
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
        // Gemini CLI.
        "BeforeAgent" | "BeforeTool" | "AfterTool" => HookStatus::Working,
        "AfterAgent" => HookStatus::Done,
        "SubagentStop" => HookStatus::Same,
        "PermissionRequest" => HookStatus::Blocked,
        "Notification" => {
            let kind = payload.get("notification_type").and_then(Value::as_str).unwrap_or("");
            let msg = payload.get("message").and_then(Value::as_str).unwrap_or("").to_lowercase();
            match kind {
                "permission_prompt" | "elicitation_dialog" | "ToolPermission" => HookStatus::Blocked,
                "idle_prompt" => HookStatus::Done,
                _ if msg.contains("permission") => HookStatus::Blocked,
                _ => return None,
            }
        }
        "Stop" => HookStatus::Done,
        "SessionStart" => HookStatus::Idle,
        "SessionEnd" => HookStatus::Gone,
        // opencode (its plugin passes the event).
        "" if payload.get("type").and_then(Value::as_str).is_some_and(|t| t.contains('.')) => {
            let ty = payload.get("type").and_then(Value::as_str).unwrap_or("");
            let state = payload.pointer("/properties/status/type").and_then(Value::as_str).unwrap_or("");
            match ty {
                "session.status" if state == "busy" || state == "retry" => HookStatus::Working,
                "session.status" if state == "idle" => HookStatus::Done,
                "session.idle" => HookStatus::Done,
                "permission.asked" | "permission.updated" | "question.asked" => HookStatus::Blocked,
                "permission.replied" => HookStatus::Working,
                "session.created" => HookStatus::Idle,
                "session.deleted" => HookStatus::Gone,
                _ => return None,
            }
        }
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

/// Whether a Claude transcript has background work running that was started since your last
/// message: a background command or agent with no "finished" note for it yet.
fn background_running(path: &std::path::Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(4 * 1024 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return false;
    }
    background_running_in(&buf)
}

fn background_running_in(transcript: &str) -> bool {
    use std::sync::LazyLock;
    static STARTED: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?:running in background with ID: |agentId: )([A-Za-z0-9_-]+)").expect("valid regex"));
    static ENDED: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<task-id>([A-Za-z0-9_-]+)</task-id>.{0,800}?<status>(\w+)</status>").expect("valid regex"));
    let mut started: Vec<String> = Vec::new();
    let mut ended: Vec<String> = Vec::new();
    for line in transcript.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) == Some("user") && is_typed_prompt(&v) {
            // Your message: what ran before it is no longer this job.
            started.clear();
            ended.clear();
            continue;
        }
        started.extend(STARTED.captures_iter(line).map(|c| c[1].to_string()));
        ended.extend(ENDED.captures_iter(line).filter(|c| &c[2] != "running").map(|c| c[1].to_string()));
    }
    started.iter().any(|id| !ended.contains(id))
}

/// A user line in a Claude transcript that you typed (not a tool's result or a background
/// task's note).
fn is_typed_prompt(v: &Value) -> bool {
    let typed = |t: &str| {
        let t = t.trim_start();
        !t.is_empty() && !t.starts_with("<task-notification>") && !t.starts_with("<system-reminder>")
    };
    match v.pointer("/message/content") {
        Some(Value::String(t)) => typed(t),
        Some(Value::Array(items)) => {
            !items.iter().any(|i| i.get("type").and_then(Value::as_str) == Some("tool_result"))
                && items.iter().any(|i| i.get("type").and_then(Value::as_str) == Some("text") && i.get("text").and_then(Value::as_str).is_some_and(typed))
        }
        _ => false,
    }
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
    // Inert outside seshi, so global hooks don't bother other terminals.
    let Some(term) = std::env::var("SESHI_TERM_ID").ok().and_then(|s| s.parse::<TermId>().ok()) else {
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
    let mut event = field(&["hook_event_name"]).unwrap_or_default();
    let transcript = field(&["transcript_path"]).map(PathBuf::from);
    // A hook fired inside a subagent (it has an agent id, or its own agent-… transcript) is
    // news about that subagent, not about the session: it keeps the subagent listed as
    // running, and only a question it asks changes the session's state. Its transcript's
    // name, model and last words aren't the session's.
    let lifecycle = matches!(event.as_str(), "SubagentStart" | "SubagentStop");
    let inner_id = field(&["agent_id"]).or_else(|| {
        transcript.as_deref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).filter(|s| s.starts_with("agent-"))
    });
    let inner = !lifecycle && inner_id.is_some();
    let status = if inner && status != HookStatus::Blocked { HookStatus::Same } else { status };
    let subagent = (lifecycle || inner).then(|| Subagent {
        id: field(&["agent_id", "subagent_id", "tool_use_id"]).or(inner_id).unwrap_or_default(),
        kind: field(&["agent_type", "subagent_type", "agent_name"]).unwrap_or_else(|| "subagent".into()),
        start: event != "SubagentStop",
    });
    let session_facts = !inner;
    let prompt = field(&["prompt"]).filter(|_| session_facts).map(|p| one_line(&p, 160));
    let transcript = transcript.filter(|_| session_facts);
    let (model, name) = transcript.as_deref().map(transcript_facts).unwrap_or_default();
    // A reply that ends while work it started in the background still runs (a long build,
    // a background agent) isn't the end of the job: it wakes up when that's done.
    let status = if status == HookStatus::Done && transcript.as_deref().is_some_and(background_running) { HookStatus::Working } else { status };
    if event == "Notification"
        && let Some(kind) = field(&["notification_type"])
    {
        event = format!("Notification:{kind}");
    }
    // The agent that ran this hook (still alive, unlike this short process) vouches for it.
    let pid = {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let me = Pid::from_u32(std::process::id());
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[me]), true, ProcessRefreshKind::nothing());
        sys.process(me).and_then(|p| p.parent()).map(|p| p.as_u32()).unwrap_or(0)
    };
    let token = std::env::var("SESHI_PANE_TOKEN").unwrap_or_default();
    let said = field(&["last_assistant_message", "last-assistant-message"])
        .or_else(|| transcript.as_deref().and_then(last_assistant_text))
        .map(|s| s.trim().chars().take(2000).collect::<String>())
        .filter(|s| !s.is_empty() && session_facts);
    let session = session.filter(|_| session_facts);
    let cwd = cwd.filter(|_| session_facts);
    block_on(async move {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (_r, mut w) = ipc::open(false).await?;
            ipc::send(&mut w, &ClientMsg::Hook { term, agent: agent.to_string(), status, session, cwd, prompt, said, subagent, event, pid, token, transcript, model, name }).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await?
    })
}

/// Gemini CLI's hook events (its settings.json takes Claude's shape, with its own names).
const GEMINI_EVENTS: &[(&str, Option<&str>)] = &[
    ("BeforeAgent", None),
    ("BeforeTool", None),
    ("AfterTool", None),
    ("Notification", None),
    ("AfterAgent", None),
    ("SessionStart", None),
    ("SessionEnd", None),
];

/// A tool's settings.json under the home folder (`.gemini`, `.qwen`), or `$<env>/…`.
fn home_settings(env: &str, dir: &str) -> PathBuf {
    std::env::var_os(env)
        .map(|h| PathBuf::from(h).join(dir))
        .unwrap_or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(dir)).unwrap_or_default())
        .join("settings.json")
}

/// opencode has no command hooks: a plugin hands each event to `seshi hook opencode`.
fn opencode_plugin(uninstall: bool) -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".config")).unwrap_or_default());
    let path = base.join("opencode").join("plugins").join("seshi.js");
    if uninstall {
        let _ = std::fs::remove_file(&path);
        return Ok(path);
    }
    let exe = this_exe()?;
    let js = format!(
        "// Written by `seshi integrate opencode`: tells seshi what opencode is doing (inert outside seshi).\n\
export const Seshi = async ({{ $, directory }}) => ({{\n\
  event: async ({{ event }}) => {{\n\
    if (!process.env.SESHI_TERM_ID) return\n\
    await $`{exe} hook opencode ${{JSON.stringify({{ ...event, cwd: directory }})}}`.quiet().nothrow()\n\
  }},\n\
}})\n"
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&path, js)?;
    Ok(path)
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

/// A hook group seshi put there: tagged, or (from installs before the tag) one that runs
/// `seshi hook …` under any of the app's names.
fn is_ours(group: &Value) -> bool {
    ["_seshi", "_hydra", "_drover"].iter().any(|tag| group.get(*tag).and_then(Value::as_bool) == Some(true))
        || group.get("hooks").and_then(Value::as_array).into_iter().flatten().filter_map(|h| h.get("command").and_then(Value::as_str)).any(runs_our_hook)
}

/// A hook command split into the program it starts (its first word, or the quoted path)
/// and its arguments.
fn split_command(cmd: &str) -> (&str, &str) {
    let c = cmd.trim_start();
    let (prog, args) = match c.strip_prefix('"') {
        Some(rest) => rest.split_once('"').unwrap_or((rest, "")),
        None => c.split_once(char::is_whitespace).unwrap_or((c, "")),
    };
    (prog, args.trim_start())
}

fn program_of(cmd: &str) -> &str {
    split_command(cmd).0
}

/// `"<…/seshi>" hook claude`, or the same from a hydra or drover install.
fn runs_our_hook(cmd: &str) -> bool {
    let (prog, args) = split_command(cmd);
    let stem = std::path::Path::new(prog).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(stem.as_str(), "seshi" | "hydra" | "drover") && args.starts_with("hook ")
}

/// Hook commands in Claude settings `root` whose program isn't there (a removed install, a
/// moved file): Claude reports "hook error" on every turn they run in. Paths are checked
/// with `exists`; a bare name only when it's one of the app's old names (`on_path`).
fn broken_hooks_in(root: &Value, exists: impl Fn(&std::path::Path) -> bool, on_path: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = root
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|h| h.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|h| h.get("command").and_then(Value::as_str))
        .filter(|cmd| {
            let prog = program_of(cmd);
            if prog.contains('$') || prog.starts_with('~') {
                return false;
            }
            if prog.contains('/') || prog.contains('\\') {
                !exists(std::path::Path::new(prog))
            } else {
                matches!(prog, "seshi" | "hydra" | "drover") && !on_path(prog)
            }
        })
        .map(String::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The Claude settings files whose hooks run here: yours, and this folder's project ones.
fn claude_settings_files() -> Vec<PathBuf> {
    let user = claude_settings();
    let mut files = vec![user.with_file_name("settings.local.json"), user];
    if let Ok(here) = std::env::current_dir() {
        files.push(here.join(".claude").join("settings.json"));
        files.push(here.join(".claude").join("settings.local.json"));
    }
    files
}

/// Claude Code's settings file, where its hooks live.
fn claude_settings() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".claude")).unwrap_or_default())
        .join("settings.json")
}

/// This seshi, as hook commands name it.
fn this_exe() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().replace('\\', "/"))
}

/// Context, cost and plan limits from the JSON Claude Code hands its status line.
pub(crate) fn statusline_facts(json: &str) -> (Usage, Vec<Limit>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return (Usage::default(), Vec::new()) };
    let usage = Usage { context: v["context_window"]["used_percentage"].as_f64().map(|p| p as f32), cost: v["cost"]["total_cost_usd"].as_f64() };
    let limits = [("five_hour", "5h"), ("seven_day", "week")]
        .iter()
        .filter_map(|(k, name)| {
            let l = &v["rate_limits"][k];
            Some(Limit { name: name.to_string(), used: l["used_percentage"].as_f64()? as f32, resets_at: l["resets_at"].as_u64()? })
        })
        .collect();
    (usage, limits)
}

/// Where the status line you had before seshi's is kept (seshi's runs it after its own).
fn statusline_before() -> PathBuf {
    crate::config::data_dir().join("claude-statusline.txt")
}

/// Whether a status line command is seshi's (or the hydra's it was renamed from).
fn is_our_statusline(cmd: &str) -> bool {
    (cmd.contains("seshi") || cmd.contains("hydra")) && cmd.trim_end().ends_with(" statusline")
}

/// Make Claude's status line seshi's (or with `uninstall`, put yours back). Yours keeps
/// showing: seshi's runs it with the same input and prints what it prints.
fn statusline_setting(root: &mut Value, exe: &str, uninstall: bool, kept: &std::path::Path) -> Result<()> {
    let cur = root.get("statusLine").and_then(|s| s.get("command")).and_then(Value::as_str).map(String::from);
    let ours = cur.as_deref().is_some_and(is_our_statusline);
    let before = std::fs::read_to_string(kept).unwrap_or_default();
    let obj = root.as_object_mut().ok_or_else(|| anyhow!("settings.json is not an object"))?;
    if uninstall {
        if ours {
            match before.trim() {
                "" => {
                    obj.remove("statusLine");
                }
                b => obj["statusLine"]["command"] = json!(b),
            }
        }
        let _ = std::fs::remove_file(kept);
        return Ok(());
    }
    if !ours {
        if let Some(dir) = kept.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(kept, cur.unwrap_or_default())?;
    }
    let line = obj.entry("statusLine").or_insert_with(|| json!({ "type": "command" }));
    if !line.is_object() {
        *line = json!({ "type": "command" });
    }
    line["command"] = json!(format!("\"{exe}\" statusline"));
    Ok(())
}

/// `seshi statusline`, Claude Code's status line command: hands what the session has used
/// to seshi (inside a seshi pane), then shows the status line you had before, if any.
pub fn statusline() {
    use std::io::Write;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let term = std::env::var("SESHI_TERM_ID").ok().and_then(|s| s.parse::<TermId>().ok());
    if let (Some(term), Ok(token)) = (term, std::env::var("SESHI_PANE_TOKEN")) {
        let (usage, limits) = statusline_facts(&input);
        let msg = ClientMsg::Usage { term, token, usage, limits };
        // Never hold up Claude's screen for it; a report that doesn't get there is skipped.
        let sent = block_on(async {
            tokio::time::timeout(Duration::from_millis(800), async {
                let (_r, mut w) = ipc::open(false).await?;
                ipc::send(&mut w, &msg).await
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("timed out")))
        });
        if let Err(e) = sent {
            tracing::debug!("status line report: {e:#}");
        }
    }
    let before = std::fs::read_to_string(statusline_before()).unwrap_or_default();
    let before = before.trim();
    if before.is_empty() || is_our_statusline(before) {
        return;
    }
    let mut cmd = shell_line(before);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::inherit()).stderr(std::process::Stdio::null());
    if let Ok(mut child) = crate::proc::quiet(&mut cmd).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let _ = child.wait();
    }
}

/// A command line run the way Claude Code runs its own: by bash (Git Bash on Windows, where
/// cmd stands in when there's none).
fn shell_line(line: &str) -> std::process::Command {
    if cfg!(windows) {
        let bash = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH")
            .map(PathBuf::from)
            .or_else(|| ["C:/Program Files/Git/bin/bash.exe", "C:/Program Files (x86)/Git/bin/bash.exe"].iter().map(PathBuf::from).find(|p| p.is_file()));
        match bash {
            Some(b) => {
                let mut c = std::process::Command::new(b);
                c.args(["-c", line]);
                c
            }
            None => {
                let mut c = std::process::Command::new("cmd");
                c.args(["/C", line]);
                c
            }
        }
    } else {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", line]);
        c
    }
}

/// Add (or with `uninstall`, remove) seshi's hooks in Claude Code's settings.
fn claude_hooks(uninstall: bool) -> Result<PathBuf> {
    json_hooks(&claude_settings(), "claude", CLAUDE_EVENTS, 5, uninstall)
}

/// Add (or remove) seshi's hooks in a Claude-style settings.json (Claude Code, Gemini CLI,
/// Qwen Code): one tagged group per event running `seshi hook <agent>`, `timeout` in the
/// tool's own unit. The rest of the file is kept, with a backup beside it.
fn json_hooks(path: &std::path::Path, agent: &str, events: &[(&str, Option<&str>)], timeout: u64, uninstall: bool) -> Result<PathBuf> {
    let exe = this_exe()?;
    let path = path.to_path_buf();
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(s) if s.trim().is_empty() => json!({}),
        Ok(s) => serde_json::from_str(&s).with_context(|| format!("{} isn't valid JSON; not touching it", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    if path.exists() {
        std::fs::copy(&path, path.with_extension("json.seshi-bak"))?;
    }
    let hooks = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings.json is not an object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or_else(|| anyhow!("`hooks` is not an object"))?;
    for (event, matcher) in events {
        let groups = hooks.entry(*event).or_insert_with(|| json!([]));
        let Some(arr) = groups.as_array_mut() else { continue };
        arr.retain(|g| !is_ours(g));
        if !uninstall {
            let mut g = json!({
                "_seshi": true,
                "hooks": [{ "type": "command", "command": format!("\"{exe}\" hook {agent}"), "timeout": timeout }],
            });
            if let Some(m) = matcher {
                g["matcher"] = json!(m);
            }
            arr.push(g);
        }
    }
    hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    if agent == "claude" {
        statusline_setting(&mut root, &exe, uninstall, &statusline_before())?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&path, serde_json::to_string_pretty(&root)? + "\n")?;
    Ok(path)
}

/// The commands seshi's Claude hooks run that aren't this seshi (an older install, a copy
/// that moved). Empty when they're right, or when there are none.
pub fn stale_claude_hooks() -> Vec<String> {
    let (Ok(exe), Ok(text)) = (this_exe(), std::fs::read_to_string(claude_settings())) else { return Vec::new() };
    let Ok(root) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    stale_in(&root, &format!("\"{exe}\" hook claude"))
}

/// Seshi's hook commands in Claude settings `root` other than `want`.
fn stale_in(root: &Value, want: &str) -> Vec<String> {
    let mut stale: Vec<String> = root
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|h| h.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter(|g| is_ours(g))
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter_map(|h| h.get("command").and_then(Value::as_str))
        .filter(|c| *c != want)
        .map(String::from)
        .collect();
    stale.sort();
    stale.dedup();
    stale
}

/// Seshi's Claude hooks are in, but not its status line (installed before there was one).
fn needs_statusline() -> bool {
    let Ok(text) = std::fs::read_to_string(claude_settings()) else { return false };
    let Ok(root) = serde_json::from_str::<Value>(&text) else { return false };
    let hooked = root.get("hooks").and_then(Value::as_object).is_some_and(|h| h.values().filter_map(Value::as_array).flatten().any(is_ours));
    let cmd = root.get("statusLine").and_then(|s| s.get("command")).and_then(Value::as_str).unwrap_or_default();
    hooked && !is_our_statusline(cmd)
}

/// Point seshi's Claude hooks at this seshi if they name another one. A hook from a
/// different version can't talk to this server, and hooks fail silently by design, so
/// every agent would look idle. Run when the server starts.
pub fn refresh_claude_hooks() {
    // A side server (SESHI_SOCKET) or a build in a source checkout mustn't take your
    // hooks from the seshi you use.
    let side = std::env::var("SESHI_SOCKET").is_ok_and(|s| s != "default");
    let dev_build = this_exe().is_ok_and(|e| e.contains("/target/debug/") || e.contains("/target/release/"));
    if side || dev_build {
        return;
    }
    if !stale_claude_hooks().is_empty() || needs_statusline() {
        match claude_hooks(false) {
            Ok(p) => tracing::info!("pointed seshi's Claude hooks in {} at this seshi", p.display()),
            Err(e) => tracing::warn!("couldn't update seshi's Claude hooks: {e:#}"),
        }
    }
}

/// Codex's config file (`$CODEX_HOME/config.toml`, else `~/.codex/config.toml`).
fn codex_config() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".codex")).unwrap_or_default())
        .join("config.toml")
}

enum CodexNotify {
    Set,
    Removed,
    /// Someone else's notify is there (shown as written).
    Theirs(String),
}

/// Point Codex's top-level `notify` at seshi (or take seshi's out), keeping the rest of the
/// file. Another program's notify is left alone.
fn codex_notify(path: &std::path::Path, exe: &str, uninstall: bool) -> Result<CodexNotify> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().with_context(|| format!("{} has a syntax error; not touching it", path.display()))?;
    let ours = |v: &toml_edit::Item| v.as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some("hook")) && a.iter().any(|x| x.as_str().is_some_and(|s| s.contains("seshi") || s.contains("hydra"))));
    if let Some(cur) = doc.get("notify")
        && !ours(cur)
    {
        return Ok(CodexNotify::Theirs(cur.to_string().trim().to_string()));
    }
    if uninstall {
        doc.remove("notify");
    } else {
        let mut a = toml_edit::Array::new();
        for part in [exe, "hook", "codex"] {
            a.push(part);
        }
        // Top-level keys go before the tables.
        doc.insert("notify", toml_edit::value(a));
    }
    if path.exists() {
        std::fs::copy(path, path.with_extension("toml.seshi-bak"))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(path, doc.to_string())?;
    Ok(if uninstall { CodexNotify::Removed } else { CodexNotify::Set })
}

pub fn integrate(agent: &str, uninstall: bool) -> Result<()> {
    let exe = this_exe()?;
    match agent {
        "claude" => {
            let path = claude_hooks(uninstall)?;
            if uninstall {
                println!("removed seshi hooks from {}", path.display());
            } else {
                println!("installed seshi hooks into {}", path.display());
                println!("they only act inside seshi panes (SESHI_TERM_ID), so other terminals are unaffected.");
            }
            Ok(())
        }
        "mcp" => {
            // Claude Code: register for every project (user scope).
            let args = ["mcp", "add", "--scope", "user", "seshi", "--", exe.as_str(), "mcp"];
            let added = std::process::Command::new(if cfg!(windows) { "claude.cmd" } else { "claude" })
                .args(args)
                .status()
                .or_else(|_| std::process::Command::new("claude").args(args).status());
            match added {
                Ok(s) if s.success() => println!("added the seshi MCP server to Claude Code (all projects)"),
                _ => println!("Claude Code: run  claude mcp add --scope user seshi -- \"{exe}\" mcp"),
            }
            println!("\nCodex: add to ~/.codex/config.toml\n\n[mcp_servers.seshi]\ncommand = \"{exe}\"\nargs = [\"mcp\"]\n");
            println!("Agents can then list, read, message and start sessions. Whether they may approve");
            println!("prompts is up to you: seshi Settings → Agents (never, by default).");
            Ok(())
        }
        "gemini" | "qwen" => {
            let (path, events, timeout) = if agent == "gemini" {
                // Gemini counts its timeout in milliseconds.
                (home_settings("GEMINI_CLI_HOME", ".gemini"), GEMINI_EVENTS, 5000)
            } else {
                // Qwen Code copies Claude's events (and seconds).
                (home_settings("QWEN_HOME", ".qwen"), CLAUDE_EVENTS, 10)
            };
            let path = json_hooks(&path, agent, events, timeout, uninstall)?;
            println!("{} seshi hooks {} {}", if uninstall { "removed" } else { "installed" }, if uninstall { "from" } else { "into" }, path.display());
            Ok(())
        }
        "opencode" => {
            let path = opencode_plugin(uninstall)?;
            println!("{} seshi's opencode plugin: {}", if uninstall { "removed" } else { "wrote" }, path.display());
            Ok(())
        }
        "codex" => {
            let path = codex_config();
            match codex_notify(&path, &exe, uninstall)? {
                CodexNotify::Set => println!("set seshi as Codex's notify in {}: it reports finished turns", path.display()),
                CodexNotify::Removed => println!("removed seshi's notify from {}", path.display()),
                CodexNotify::Theirs(other) => println!(
                    "{} already has notify = {other}; not touching it. To add seshi, make it run:\n  \"{exe}\" hook codex",
                    path.display()
                ),
            }
            println!("Working / needs-you come from Codex's screen.");
            Ok(())
        }
        other => bail!("no integration for `{other}`; any agent can call `seshi hook {other} --status <working|blocked|done|idle>`"),
    }
}

/// Debugging: the colours a pane's program is drawing (rows with a background colour).
pub fn debug_colors(pane: TermId) -> Result<()> {
    block_on(async move {
        let (mut r, _w) = ipc::open(true).await.context("couldn't reach the seshi server")?;
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

/// `seshi allow`: show the repo's hook commands and let them run from now on.
pub fn allow(dir: Option<std::path::PathBuf>) -> Result<()> {
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let p = crate::project::load(&dir);
    let cmds = [p.hooks.on_create.as_str(), p.hooks.on_remove.as_str()];
    if cmds.iter().all(|c| c.trim().is_empty()) {
        println!("no hooks in {} here; nothing to allow", crate::project::FILE);
        return Ok(());
    }
    crate::project::allow(&dir, &cmds)?;
    for (what, c) in [("on_create", cmds[0]), ("on_remove", cmds[1])] {
        if !c.trim().is_empty() {
            println!("allowed {what}: {c}");
        }
    }
    println!("(if the commands change, they won't run until you allow them again)");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hydras_status_line_keeps_yours() {
        let kept = std::env::temp_dir().join(format!("seshi-statusline-{}.txt", std::process::id()));
        let mut root = serde_json::json!({ "statusLine": { "type": "command", "command": "chm statusline", "padding": 1 } });
        super::statusline_setting(&mut root, "C:/x/seshi.exe", false, &kept).unwrap();
        assert_eq!(root["statusLine"]["command"], "\"C:/x/seshi.exe\" statusline");
        assert_eq!(root["statusLine"]["padding"], 1, "your settings for it stay");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "chm statusline", "and yours still runs after seshi's");
        super::statusline_setting(&mut root, "C:/y/seshi.exe", false, &kept).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "chm statusline", "installing again doesn't lose yours");
        super::statusline_setting(&mut root, "C:/y/seshi.exe", true, &kept).unwrap();
        assert_eq!(root["statusLine"]["command"], "chm statusline", "uninstalling puts yours back");
        let mut none = serde_json::json!({});
        super::statusline_setting(&mut none, "C:/x/seshi.exe", false, &kept).unwrap();
        super::statusline_setting(&mut none, "C:/x/seshi.exe", true, &kept).unwrap();
        assert!(none.get("statusLine").is_none(), "none before: none after");
    }

    #[test]
    fn claude_says_what_it_used_through_its_status_line() {
        let json = r#"{"cost":{"total_cost_usd":1.25},"context_window":{"context_window_size":200000,"used_percentage":42},
            "rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}}"#;
        let (u, l) = super::statusline_facts(json);
        assert_eq!(u, crate::protocol::Usage { context: Some(42.0), cost: Some(1.25) });
        assert_eq!(l.iter().map(|x| (x.name.as_str(), x.used, x.resets_at)).collect::<Vec<_>>(), [("5h", 23.5, 1738425600), ("week", 41.2, 1738857600)]);
        let (u, l) = super::statusline_facts(r#"{"context_window":{"used_percentage":null}}"#);
        assert_eq!((u, l.len()), (crate::protocol::Usage::default(), 0), "early in a session, or not on a plan: nothing yet");
    }

    #[test]
    fn only_new_work_gets_a_worktree() {
        let w = |a: &[&str]| super::starts_new_work(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(w(&[]) && w(&["fix the login page"]) && w(&["--model", "opus"]) && w(&["update the readme"]), "a new session or a task");
        assert!(!w(&["--continue"]) && !w(&["-r"]) && !w(&["resume"]), "an earlier conversation stays in its folder");
        assert!(!w(&["--version"]) && !w(&["-p", "what's 2+2"]) && !w(&["mcp", "list"]) && !w(&["update"]), "not work: help, one-shots, subcommands");
    }

    #[test]
    fn a_reply_with_background_work_running_isnt_the_end() {
        use serde_json::json;
        let line = |v: serde_json::Value| v.to_string();
        let you = line(json!({"type": "user", "message": {"content": "run the long build"}}));
        let started = line(json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "Command running in background with ID: b9ib9wh3g. Output is being written to: x"}]}}));
        let ended = line(json!({"type": "user", "message": {"content": "<task-notification>\n<task-id>b9ib9wh3g</task-id>\n<status>completed</status>\n</task-notification>"}}));
        let reply = line(json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Waiting for the build."}]}}));
        let t = |lines: &[&String]| lines.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n");
        assert!(super::background_running_in(&t(&[&you, &started, &reply])), "the build still runs: not done");
        assert!(!super::background_running_in(&t(&[&you, &started, &reply, &ended, &reply])), "it finished: done");
        assert!(!super::background_running_in(&t(&[&started, &reply, &you, &reply])), "started before your last message: not this job");
        let agent = line(json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": [{"type": "text", "text": "Async agent launched successfully.\nagentId: a90856b7402e3540a"}]}]}}));
        assert!(super::background_running_in(&t(&[&you, &agent, &reply])), "a background agent counts too");
    }

    #[test]
    fn gemini_qwen_and_opencode_events_mean_states() {
        use crate::protocol::HookStatus as H;
        let st = |v: serde_json::Value| super::status_from_hook(&v);
        assert_eq!(st(serde_json::json!({"hook_event_name": "BeforeAgent"})), Some(H::Working), "gemini: a turn starts");
        assert_eq!(st(serde_json::json!({"hook_event_name": "AfterAgent"})), Some(H::Done));
        assert_eq!(st(serde_json::json!({"hook_event_name": "Notification", "notification_type": "ToolPermission"})), Some(H::Blocked));
        assert_eq!(st(serde_json::json!({"hook_event_name": "Stop"})), Some(H::Done), "qwen speaks claude's events");
        assert_eq!(st(serde_json::json!({"type": "session.status", "properties": {"status": {"type": "busy"}}})), Some(H::Working), "opencode");
        assert_eq!(st(serde_json::json!({"type": "permission.asked", "properties": {}})), Some(H::Blocked));
        assert_eq!(st(serde_json::json!({"type": "session.idle", "properties": {}})), Some(H::Done));
        assert_eq!(st(serde_json::json!({"type": "agent-turn-complete"})), Some(H::Done), "codex unchanged");
    }

    #[test]
    fn codex_notify_is_set_and_others_kept() {
        let file = std::env::temp_dir().join(format!("seshi-codex-{}.toml", std::process::id()));
        std::fs::write(&file, "model = \"gpt-5\"\n\n[tui]\nnotifications = true\n").unwrap();
        assert!(matches!(super::codex_notify(&file, "/bin/seshi", false).unwrap(), super::CodexNotify::Set));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(r#"notify = ["/bin/seshi", "hook", "codex"]"#) && text.contains("[tui]") && text.contains("gpt-5"), "{text}");
        assert!(text.find("notify") < text.find("[tui]"), "a top-level key, before the tables: {text}");
        // Run again with seshi elsewhere: replaced (it's ours).
        super::codex_notify(&file, "/usr/local/bin/seshi", false).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("/usr/local/bin/seshi"));
        // Someone else's notify stays.
        std::fs::write(&file, "notify = [\"my-notifier\"]\n").unwrap();
        assert!(matches!(super::codex_notify(&file, "/bin/seshi", false).unwrap(), super::CodexNotify::Theirs(_)));
        assert!(std::fs::read_to_string(&file).unwrap().contains("my-notifier"));
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(file.with_extension("toml.seshi-bak"));
    }

    #[test]
    fn hooks_from_another_hydra_are_found() {
        let want = "\"/home/me/.local/bin/seshi\" hook claude";
        let settings = |cmd: &str| {
            serde_json::json!({ "hooks": {
                "Stop": [
                    { "_hydra": true, "hooks": [{ "type": "command", "command": cmd }] },
                    { "hooks": [{ "type": "command", "command": "someone-elses-hook" }] },
                ],
            }})
        };
        assert!(super::stale_in(&settings(want), want).is_empty(), "this seshi's hooks are fine; others' aren't ours to judge");
        let old = "\"/home/me/.cargo/bin/seshi\" hook claude";
        assert_eq!(super::stale_in(&settings(old), want), vec![old.to_string()]);
        // An old install's hook without the tag is still ours (so it gets repointed).
        let untagged = serde_json::json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "/home/me/.local/bin/hydra hook claude" }] }] } });
        assert_eq!(super::stale_in(&untagged, want), vec!["/home/me/.local/bin/hydra hook claude".to_string()]);
        assert!(!super::runs_our_hook("hydra-lint check"), "only the app's own hook");
    }

    #[test]
    fn hooks_whose_program_is_gone_are_found() {
        let root = serde_json::json!({ "hooks": {
            "Stop": [{ "hooks": [
                { "type": "command", "command": "\"/home/me/.local/bin/hydra\" hook claude" },
                { "type": "command", "command": "/usr/bin/notify-send done" },
                { "type": "command", "command": "hydra hook claude" },
                { "type": "command", "command": "npx something" },
                { "type": "command", "command": "$HOME/bin/x" },
            ] }],
        }});
        let exists = |p: &std::path::Path| p == std::path::Path::new("/usr/bin/notify-send");
        let broken = super::broken_hooks_in(&root, exists, |_| false);
        assert_eq!(broken, vec!["\"/home/me/.local/bin/hydra\" hook claude".to_string(), "hydra hook claude".to_string()]);
    }

    #[test]
    fn model_and_name_from_a_transcript() {
        assert_eq!(super::short_model("claude-opus-4-5-20251101"), "opus 4.5");
        assert_eq!(super::short_model("claude-fable-5-1"), "fable 5.1");
        assert_eq!(super::short_model("gpt-5.5"), "gpt-5.5");
        let t = [
            r#"{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[]}}"#,
            r#"{"type":"ai-title","aiTitle":"Fix the login flow","sessionId":"s"}"#,
            r#"{"type":"user","message":{"content":"say \"model\" and \"customTitle\""}}"#,
        ]
        .join("\n");
        assert_eq!(super::transcript_facts_in(&t), (Some("sonnet 5.5".into()), Some("Fix the login flow".into())));
        let t = format!("{t}\n{}", r#"{"type":"custom-title","customTitle":"auth rewrite","sessionId":"s"}"#);
        assert_eq!(super::transcript_facts_in(&t).1.as_deref(), Some("auth rewrite"), "your /rename wins");
    }

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
