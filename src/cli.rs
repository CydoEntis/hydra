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
    let (mut r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the hydra server" })?;
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

/// `hydra wait` / `hydra send --wait`: wait, then print the reply (or the screen's end).
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
        let (_r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the hydra server" })?;
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
        let (_r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the hydra server" })?;
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

/// Focus a pane and bring hydra's window forward (what clicking a notification does).
pub fn reveal(pane: TermId) -> Result<()> {
    command(Command::Reveal { term: pane })
}

pub fn close(pane: Option<TermId>) -> Result<()> {
    command(Command::ClosePane { term: resolve_pane(pane)? })
}

pub fn kill_server(forget: bool) -> Result<()> {
    match command(Command::KillServer { forget }) {
        // Another version can't be asked to stop: stop its process instead.
        Err(e) if e.chain().any(|c| c.is::<ipc::OtherVersion>()) => {
            let pid = stop_server_process()?;
            if forget {
                crate::daemon::forget_session();
            }
            println!("stopped the server (another hydra version, process {pid})");
            Ok(())
        }
        r => r,
    }
}

/// Stop this socket's server by its process, for when it's a version that can't be talked
/// to. Its saved session stays, as with `kill-server`.
fn stop_server_process() -> Result<u32> {
    let pid = server_pid().ok_or_else(|| {
        anyhow!("couldn't find the server's process; stop it yourself (on Linux/macOS: pkill -f \"hydra daemon\"; on Windows: end hydra.exe in Task Manager)")
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
/// one running `hydra daemon` for this socket.
fn server_pid() -> Option<u32> {
    if let Some(pid) = std::fs::read_to_string(ipc::pid_file()).ok().and_then(|s| s.trim().parse().ok()) {
        return Some(pid);
    }
    #[cfg(target_os = "linux")]
    {
        let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
        let me = std::process::id();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            if pid == me {
                continue;
            }
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
            let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
            let is_daemon = args.first().is_some_and(|a| a.ends_with(b"hydra")) && args.get(1) == Some(&b"daemon".as_slice());
            if !is_daemon {
                continue;
            }
            // Its socket: HYDRA_SOCKET in its environment, else the default one.
            let env = std::fs::read(entry.path().join("environ")).unwrap_or_default();
            let theirs = env
                .split(|b| *b == 0)
                .find_map(|kv| kv.strip_prefix(b"HYDRA_SOCKET="))
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
    if std::env::var("HYDRA_SOCKET").is_err() {
        let list = if cfg!(windows) {
            crate::proc::run(std::process::Command::new("powershell").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='hydra.exe'\" | Where-Object { $_.CommandLine -match '\\sdaemon\\s*$' } | ForEach-Object { $_.ProcessId }",
            ]))
        } else {
            crate::proc::run(std::process::Command::new("pgrep").args(["-f", "hydra daemon$"]))
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
    let here = std::env::var("HYDRA_TERM_ID").ok().and_then(|s| s.parse().ok());
    here.and_then(|t| snap.locate(t).map(|(w, _)| w.id))
        .or(snap.active_ws)
        .ok_or_else(|| anyhow!("no workspace to target"))
}

pub fn worktree(branch: String, base: Option<String>, ws: Option<WsId>, cmd: Vec<String>) -> Result<()> {
    let ws = resolve_ws(ws)?;
    command(Command::NewWorktree { ws, branch, base, cmd: join_command(cmd), split: None, from: None })
}

/// `hydra ext list | new <name> | run <name> <n> [--term id]`.
pub fn ext(action: &str, name: Option<String>, index: Option<usize>, term: Option<TermId>) -> Result<()> {
    match action {
        "list" | "ls" => {
            let (exts, errors) = crate::ext::load_all();
            println!("extensions in {}\n", crate::ext::dir().display());
            if exts.is_empty() && errors.is_empty() {
                println!("none yet; `hydra ext new <name>` makes one to start from");
            }
            for e in &exts {
                println!("{}  {}", e.name, e.description);
                for c in &e.commands {
                    println!("    command: {}{}", c.title, if c.background { " (background)" } else { "" });
                }
                for l in &e.labels {
                    println!("    label: {} (every {}s)", l.run, l.every);
                }
                for ev in ["agent_start", "agent_done", "needs_you", "worktree_create", "worktree_remove"] {
                    let h = e.hooks.get(ev);
                    if !h.is_empty() {
                        println!("    on {ev}: {h}");
                    }
                }
            }
            for e in errors {
                println!("✕ {e}");
            }
            Ok(())
        }
        "new" => {
            let name = name.ok_or_else(|| anyhow!("give it a name: hydra ext new <name>"))?;
            let d = crate::ext::scaffold(&name)?;
            println!("made {}\nedit {} and it shows up in the palette (Ctrl+Space space)", d.display(), d.join(crate::ext::MANIFEST).display());
            Ok(())
        }
        // A command opened in a pane: run it here (the pane's terminal), with its context.
        "run" => {
            let name = name.ok_or_else(|| anyhow!("which extension?"))?;
            let (exts, _) = crate::ext::load_all();
            let e = exts.iter().find(|e| e.name == name).ok_or_else(|| anyhow!("no extension {name}"))?;
            let c = e.commands.get(index.unwrap_or(0)).ok_or_else(|| anyhow!("{name} has no command {}", index.unwrap_or(0)))?;
            let info = term.and_then(|t| snapshot().ok().and_then(|s| s.terms.get(&t).cloned()));
            let dir = std::env::current_dir()?;
            let (cfg, _) = crate::config::Config::load_or_default();
            let shell = cfg.shell_command();
            // In your terminal: it may ask you things.
            let mut cmd = crate::proc::shell(&shell, &crate::ext::resolve(&e.dir, &c.run), true);
            cmd.env("HYDRA_EXT_DIR", &e.dir);
            for (k, v) in crate::ext::vars("command", &dir, info.as_ref()) {
                cmd.env(k, v);
            }
            let status = cmd.status()?;
            std::process::exit(status.code().unwrap_or(1));
        }
        other => bail!("{other}? use list, new or run"),
    }
}

/// `hydra doctor`: each thing hydra relies on, ✓ or what to do about it.
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

    println!("hydra {}  (protocol {})\n", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION);

    // Config
    let path = crate::config::config_path();
    match crate::config::Config::load() {
        Ok(_) if path.exists() => line(Some(true), "config", path.display().to_string()),
        Ok(_) => line(None, "config", format!("none yet (defaults); `hydra config init` writes one at {}", path.display())),
        Err(e) => line(Some(false), "config", format!("{e:#}  ({})", path.display())),
    }
    let (cfg, _) = crate::config::Config::load_or_default();

    // Server
    let server = block_on(async { ipc::open(false).await });
    match server {
        Ok(_) => line(Some(true), "server", format!("running ({})", ipc::socket_id())),
        Err(e) if format!("{e:#}").contains("protocol") => {
            line(Some(false), "server", format!("{e:#}"));
            println!("{:27}fix: close hydra, `hydra kill-server`, start it again", "");
        }
        Err(_) => line(None, "server", "not running (it starts with `hydra`)".into()),
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
            (false, _) => line(Some(false), "claude status hooks", "missing: `hydra integrate claude` (exact working / needs-you / done)".into()),
            // Another hydra can't talk to this one's server: every agent would look idle.
            (true, Some(other)) => line(Some(false), "claude status hooks", format!("they run another hydra ({other}); `hydra integrate claude` (or restart the server) fixes it")),
            (true, None) => line(Some(true), "claude status hooks", "installed".into()),
        }
        let home = directories::BaseDirs::new().map(|d| d.home_dir().join(".claude.json"));
        let mcp = home.and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|s| s.contains("\"hydra\"") && s.contains("\"mcp\""));
        line(if mcp { Some(true) } else { None }, "hydra MCP for claude", if mcp { "registered".into() } else { "not set up: `hydra integrate mcp` lets agents see each other (optional)".into() });
    }

    // Terminal
    let term = std::env::var("TERM_PROGRAM").or_else(|_| std::env::var("TERM")).unwrap_or_else(|_| if std::env::var("WT_SESSION").is_ok() { "Windows Terminal".into() } else { "unknown".into() });
    let truecolor = std::env::var("COLORTERM").is_ok_and(|c| c.contains("truecolor") || c.contains("24bit")) || std::env::var("WT_SESSION").is_ok();
    line(if truecolor { Some(true) } else { None }, "terminal", format!("{term}{}", if truecolor { ", true colour" } else { " (colours may look off without true colour)" }));
    if let Ok((w, h)) = crossterm::terminal::size() {
        line(Some(w >= 100 && h >= 30), "window size", format!("{w}×{h}{}", if w < 100 || h < 30 { " (hydra wants at least 100×30)" } else { "" }));
    }

    // Data and sync
    let data = crate::config::data_dir();
    let writable = std::fs::create_dir_all(&data).is_ok() && std::fs::write(data.join(".doctor"), b"ok").is_ok();
    let _ = std::fs::remove_file(data.join(".doctor"));
    line(Some(writable), "data folder", data.display().to_string());
    line(None, "sync", if crate::sync::enabled() { format!("on ({})", crate::sync::dir().display()) } else { "off (`hydra sync setup` shares config and ideas)".into() });
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

/// `hydra dev [start|stop|restart]`: this checkout's dev server.
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
    let term: TermId = std::env::var("HYDRA_TERM_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("run this from inside a hydra pane (an agent running in hydra)"))?;
    let branch = Some(branch.trim().to_string()).filter(|b| !b.is_empty());
    block_on(async move {
        let (mut r, mut w) = ipc::open(false).await.context(if ipc::remote().is_some() { "over ssh" } else { "couldn't reach the hydra server" })?;
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
    let mut event = field(&["hook_event_name"]).unwrap_or_default();
    let subagent = matches!(event.as_str(), "SubagentStart" | "SubagentStop").then(|| Subagent {
        id: field(&["agent_id", "subagent_id", "tool_use_id"]).unwrap_or_default(),
        kind: field(&["agent_type", "subagent_type", "agent_name"]).unwrap_or_else(|| "subagent".into()),
        start: event == "SubagentStart",
    });
    let prompt = field(&["prompt"]).map(|p| one_line(&p, 160));
    let transcript = field(&["transcript_path"]).map(PathBuf::from);
    let (model, name) = transcript.as_deref().map(transcript_facts).unwrap_or_default();
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
    let token = std::env::var("HYDRA_PANE_TOKEN").unwrap_or_default();
    let said = field(&["last_assistant_message", "last-assistant-message"])
        .or_else(|| field(&["transcript_path"]).and_then(|p| last_assistant_text(std::path::Path::new(&p))))
        .map(|s| s.trim().chars().take(2000).collect::<String>())
        .filter(|s| !s.is_empty());
    block_on(async move {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (_r, mut w) = ipc::open(false).await?;
            ipc::send(&mut w, &ClientMsg::Hook { term, agent: agent.to_string(), status, session, cwd, prompt, said, subagent, event, pid, token, transcript, model, name }).await?;
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

/// Claude Code's settings file, where its hooks live.
fn claude_settings() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".claude")).unwrap_or_default())
        .join("settings.json")
}

/// This hydra, as hook commands name it.
fn this_exe() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().replace('\\', "/"))
}

/// Add (or with `uninstall`, remove) hydra's hooks in Claude Code's settings.
fn claude_hooks(uninstall: bool) -> Result<PathBuf> {
    let exe = this_exe()?;
    let path = claude_settings();
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(s) if s.trim().is_empty() => json!({}),
        Ok(s) => serde_json::from_str(&s).with_context(|| format!("{} isn't valid JSON; not touching it", path.display()))?,
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&path, serde_json::to_string_pretty(&root)? + "\n")?;
    Ok(path)
}

/// The commands hydra's Claude hooks run that aren't this hydra (an older install, a copy
/// that moved). Empty when they're right, or when there are none.
pub fn stale_claude_hooks() -> Vec<String> {
    let (Ok(exe), Ok(text)) = (this_exe(), std::fs::read_to_string(claude_settings())) else { return Vec::new() };
    let Ok(root) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    stale_in(&root, &format!("\"{exe}\" hook claude"))
}

/// Hydra's hook commands in Claude settings `root` other than `want`.
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

/// Point hydra's Claude hooks at this hydra if they name another one. A hook from a
/// different version can't talk to this server, and hooks fail silently by design, so
/// every agent would look idle. Run when the server starts.
pub fn refresh_claude_hooks() {
    // A side server (HYDRA_SOCKET) or a build in a source checkout mustn't take your
    // hooks from the hydra you use.
    let side = std::env::var("HYDRA_SOCKET").is_ok_and(|s| s != "default");
    let dev_build = this_exe().is_ok_and(|e| e.contains("/target/debug/") || e.contains("/target/release/"));
    if side || dev_build {
        return;
    }
    if !stale_claude_hooks().is_empty() {
        match claude_hooks(false) {
            Ok(p) => tracing::info!("pointed hydra's Claude hooks in {} at this hydra", p.display()),
            Err(e) => tracing::warn!("couldn't update hydra's Claude hooks: {e:#}"),
        }
    }
}

pub fn integrate(agent: &str, uninstall: bool) -> Result<()> {
    let exe = this_exe()?;
    match agent {
        "claude" => {
            let path = claude_hooks(uninstall)?;
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

/// Debugging: the colours a pane's program is drawing (rows with a background colour).
pub fn debug_colors(pane: TermId) -> Result<()> {
    block_on(async move {
        let (mut r, _w) = ipc::open(true).await.context("couldn't reach the hydra server")?;
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

/// `hydra allow`: show the repo's hook commands and let them run from now on.
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
    fn hooks_from_another_hydra_are_found() {
        let want = "\"/home/me/.local/bin/hydra\" hook claude";
        let settings = |cmd: &str| {
            serde_json::json!({ "hooks": {
                "Stop": [
                    { "_hydra": true, "hooks": [{ "type": "command", "command": cmd }] },
                    { "hooks": [{ "type": "command", "command": "someone-elses-hook" }] },
                ],
            }})
        };
        assert!(super::stale_in(&settings(want), want).is_empty(), "this hydra's hooks are fine; others' aren't ours to judge");
        let old = "\"/home/me/.cargo/bin/hydra\" hook claude";
        assert_eq!(super::stale_in(&settings(old), want), vec![old.to_string()]);
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
