//! A stand-in `tmux` so tools that open their own panes through tmux (Claude Code's agent
//! teams) open them as hydra panes. `hydra tmux-shim -- claude` runs a command with a `tmux`
//! first on its PATH (a link to hydra) and `TMUX` naming this shim; hydra, run under the
//! name `tmux`, answers those calls from the hydra server. Calls meant for a real tmux
//! (`TMUX` naming another server, `-L` / `-S`) go to the real one.
//!
//! What it answers: the commands those tools use (split-window, send-keys, display-message,
//! list-panes, kill-pane, capture-pane, select-pane -T, has-session, new-window, -V). Layout
//! and option commands succeed and change nothing. Anything else is logged (so it can be
//! added) and succeeds.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use crate::layout::Dir;
use crate::protocol::{ClientMsg, Command, Reply, Snapshot, TermId};

/// What `TMUX` starts with when it names this shim.
const MARK: &str = "hydra-tmux-shim";

fn shim_dir() -> PathBuf {
    crate::config::data_dir().join("tmux-shim")
}

/// `hydra tmux-shim [-- command…]`: run the command (default: your shell) with the shim.
#[cfg(unix)]
pub fn run(cmd: Vec<String>) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let term = std::env::var("HYDRA_TERM_ID").context("run it inside a hydra pane")?;
    let bin = shim_dir().join("bin");
    std::fs::create_dir_all(&bin)?;
    let link = bin.join("tmux");
    let exe = std::env::current_exe()?;
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&exe, &link).context("making the tmux link")?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(&path));
    let (program, args) = match cmd.split_first() {
        Some((p, a)) => (p.clone(), a.to_vec()),
        None => (std::env::var("SHELL").unwrap_or_else(|_| "sh".into()), Vec::new()),
    };
    let err = std::process::Command::new(&program)
        .args(&args)
        .env("PATH", std::env::join_paths(paths)?)
        // tmux's form: socket,server pid,session. Tools only check it's set (and we check
        // it's ours).
        .env("TMUX", format!("{MARK},{},0", std::process::id()))
        .env("TMUX_PANE", format!("%{term}"))
        .exec();
    Err(err).with_context(|| format!("starting {program}"))
}

#[cfg(not(unix))]
pub fn run(_cmd: Vec<String>) -> Result<()> {
    bail!("the tmux shim is for macOS and Linux (Claude Code's split-pane teammates don't run on Windows)")
}

/// hydra was started under the name `tmux`.
pub fn invoked_as_tmux() -> bool {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .is_some_and(|s| s == "tmux")
}

/// Answer a tmux call, or hand it to the real tmux when it isn't for us. Returns the exit
/// code.
pub fn main(args: Vec<String>) -> i32 {
    let ours = std::env::var("TMUX").is_ok_and(|t| t.starts_with(MARK));
    let other_server = args.first().is_some_and(|a| a == "-L" || a == "-S");
    if !ours || other_server {
        return real_tmux(&args);
    }
    match answer(&args) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}

/// The next `tmux` on PATH that isn't this shim.
fn real_tmux(args: &[String]) -> i32 {
    let ours = shim_dir().join("bin");
    let found = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).filter(|d| *d != ours).map(|d| d.join("tmux")).find(|c| c.is_file())
    });
    let Some(real) = found else {
        eprintln!("tmux: not installed (and this call isn't for hydra's tmux shim)");
        return 127;
    };
    std::process::Command::new(real).args(args).status().map(|s| s.code().unwrap_or(1)).unwrap_or(127)
}

fn log(line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(shim_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(shim_dir().join("calls.log")) {
        let _ = writeln!(f, "{line}");
    }
}

/// The parsed flags of one tmux command: `-x` switches and `-x value` options.
struct Flags {
    on: Vec<char>,
    vals: Vec<(char, String)>,
    rest: Vec<String>,
}

impl Flags {
    /// `with_value`: the flags that take a value for this command.
    fn parse(args: &[String], with_value: &str) -> Flags {
        let mut f = Flags { on: Vec::new(), vals: Vec::new(), rest: Vec::new() };
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" {
                f.rest.extend(args[i + 1..].iter().cloned());
                break;
            }
            if a.len() > 1 && a.starts_with('-') && f.rest.is_empty() {
                let chars: Vec<char> = a[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    let c = chars[j];
                    if with_value.contains(c) {
                        let v: String = chars[j + 1..].iter().collect();
                        let v = if v.is_empty() {
                            i += 1;
                            args.get(i).cloned().unwrap_or_default()
                        } else {
                            v
                        };
                        f.vals.push((c, v));
                        break;
                    }
                    f.on.push(c);
                    j += 1;
                }
            } else {
                f.rest.push(a.clone());
            }
            i += 1;
        }
        f
    }
    fn has(&self, c: char) -> bool {
        self.on.contains(&c)
    }
    fn val(&self, c: char) -> Option<&str> {
        self.vals.iter().rev().find(|(k, _)| *k == c).map(|(_, v)| v.as_str())
    }
    fn all(&self, c: char) -> Vec<&str> {
        self.vals.iter().filter(|(k, _)| *k == c).map(|(_, v)| v.as_str()).collect()
    }
}

/// The pane this call is about: `-t %N` (or a bare number), else the caller's.
fn target(f: &Flags) -> Result<TermId> {
    let t = f.val('t').map(str::to_string).or_else(|| std::env::var("TMUX_PANE").ok()).context("no target pane")?;
    // `session:window.%N` or `%N` or `:.%N`: the pane part.
    let pane = t.rsplit(['.', ':']).next().unwrap_or(&t);
    pane.trim_start_matches('%').parse().with_context(|| format!("can't find pane: {t}"))
}

fn snapshot() -> Result<Snapshot> {
    crate::cli::snapshot()
}

fn command(c: Command) -> Result<()> {
    crate::cli::block_on(crate::cli::request(ClientMsg::Command(c))).map(|_| ())
}

fn answer(args: &[String]) -> Result<String> {
    let Some((cmd, rest)) = args.split_first() else { return Ok(String::new()) };
    log(&format!("tmux {}", args.join(" ")));
    match cmd.as_str() {
        "-V" => Ok("tmux 3.4".into()),
        "display-message" | "display" => {
            let f = Flags::parse(rest, "tcF");
            let fmt = f.val('F').map(str::to_string).or_else(|| f.rest.first().cloned()).unwrap_or_default();
            if !f.has('p') {
                return Ok(String::new());
            }
            let snap = snapshot()?;
            Ok(expand(&fmt, &snap, target(&f)?))
        }
        "split-window" | "splitw" => {
            let f = Flags::parse(rest, "tcelpF");
            let from = target(&f)?;
            let before = snapshot()?;
            let cmd = (!f.rest.is_empty()).then(|| f.rest.join(" "));
            let cwd = f.val('c').map(PathBuf::from);
            // tmux's -h is side by side.
            let dir = if f.has('h') { Dir::Right } else { Dir::Down };
            let env = f.all('e');
            let cmd = match (cmd, env.is_empty()) {
                (Some(c), false) => Some(format!("{} {c}", env_prefix(&env))),
                (c, _) => c,
            };
            command(Command::Split { term: from, dir, cmd, cwd })?;
            let after = snapshot()?;
            let new = after.terms.keys().copied().filter(|id| !before.terms.contains_key(id)).max().context("the new pane didn't appear")?;
            remember(new);
            if f.has('d') {
                command(Command::FocusPane { term: from })?;
            }
            Ok(if f.has('P') { expand(f.val('F').unwrap_or("#{pane_id}"), &after, new) } else { String::new() })
        }
        "new-window" | "neww" => {
            let f = Flags::parse(rest, "tcnFe");
            let before = snapshot()?;
            let cmd = (!f.rest.is_empty()).then(|| f.rest.join(" "));
            let cwd = f.val('c').map(PathBuf::from).or_else(|| std::env::current_dir().ok());
            command(Command::NewWorkspace { cwd, name: f.val('n').map(str::to_string), cmd })?;
            let after = snapshot()?;
            let new = after.terms.keys().copied().filter(|id| !before.terms.contains_key(id)).max().context("the new pane didn't appear")?;
            remember(new);
            Ok(if f.has('P') { expand(f.val('F').unwrap_or("#{pane_id}"), &after, new) } else { String::new() })
        }
        "send-keys" | "send" => {
            let f = Flags::parse(rest, "tN");
            let term = target(&f)?;
            let times: usize = f.val('N').and_then(|n| n.parse().ok()).unwrap_or(1);
            let mut data = Vec::new();
            for k in &f.rest {
                data.extend(if f.has('l') { k.as_bytes().to_vec() } else { key_bytes(k) });
            }
            let data = data.repeat(times);
            crate::cli::block_on(async move {
                let (_r, mut w) = crate::ipc::open(false).await?;
                crate::ipc::send(&mut w, &ClientMsg::Input { term, data }).await
            })?;
            Ok(String::new())
        }
        "kill-pane" | "killp" => {
            let f = Flags::parse(rest, "t");
            command(Command::ClosePane { term: target(&f)? })?;
            Ok(String::new())
        }
        // The panes this shim opened (a team's), when the tool cleans up.
        "kill-window" | "killw" | "kill-session" => {
            for t in remembered() {
                let _ = command(Command::ClosePane { term: t });
            }
            let _ = std::fs::remove_file(shim_dir().join("panes"));
            Ok(String::new())
        }
        "select-pane" | "selectp" => {
            let f = Flags::parse(rest, "tTP");
            if let Some(title) = f.val('T') {
                command(Command::RenamePane { term: target(&f)?, name: title.to_string() })?;
            }
            // Focus stays with you: a tool picking a pane doesn't move your view.
            Ok(String::new())
        }
        "list-panes" | "lsp" => {
            let f = Flags::parse(rest, "tF");
            let snap = snapshot()?;
            let fmt = f.val('F').unwrap_or("#{pane_index}: [#{pane_width}x#{pane_height}] #{pane_id}");
            Ok(snap.terms.keys().map(|t| expand(fmt, &snap, *t)).collect::<Vec<_>>().join("\n"))
        }
        "list-sessions" | "ls" => {
            let f = Flags::parse(rest, "F");
            let snap = snapshot()?;
            let any = snap.terms.keys().next().copied().unwrap_or(0);
            Ok(expand(f.val('F').unwrap_or("#{session_name}: 1 windows"), &snap, any))
        }
        "list-windows" | "lsw" => {
            let f = Flags::parse(rest, "tF");
            let snap = snapshot()?;
            let fmt = f.val('F').unwrap_or("#{window_index}: #{window_name}");
            Ok(snap.workspaces.iter().filter_map(|w| w.tab().map(|t| expand(fmt, &snap, t.focus))).collect::<Vec<_>>().join("\n"))
        }
        "capture-pane" | "capturep" => {
            let f = Flags::parse(rest, "tSEb");
            if !f.has('p') {
                bail!("capture-pane: only -p is supported");
            }
            match crate::cli::block_on(crate::cli::request(ClientMsg::Query(crate::protocol::Query::Read { term: target(&f)? })))? {
                Reply::Text(t) => Ok(t.trim_end().to_string()),
                _ => Ok(String::new()),
            }
        }
        "has-session" | "has" => Ok(String::new()),
        // Layout and options: hydra places panes itself; nothing to change.
        "select-layout" | "selectl" | "resize-pane" | "resizep" | "set-option" | "set" | "set-window-option" | "setw" | "refresh-client"
        | "rename-window" | "renamew" | "select-window" | "selectw" | "bind-key" | "bind" | "source-file" | "source" => Ok(String::new()),
        "show-options" | "show" | "show-window-options" | "showw" => {
            let f = Flags::parse(rest, "t");
            Ok(match f.rest.first().map(String::as_str) {
                Some("pane-base-index" | "base-index") if f.has('v') => "0".into(),
                Some(o @ ("pane-base-index" | "base-index")) => format!("{o} 0"),
                _ => String::new(),
            })
        }
        other => {
            log(&format!("  (not handled: {other})"));
            Ok(String::new())
        }
    }
}

/// `-e KEY=VALUE`s as a prefix for the command.
fn env_prefix(env: &[&str]) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    format!("env {}", env.iter().map(|e| quote(e)).collect::<Vec<_>>().join(" "))
}

/// tmux key names as the bytes a terminal sends; anything else as text.
fn key_bytes(k: &str) -> Vec<u8> {
    let named: Option<&[u8]> = match k {
        "Enter" | "C-m" | "KPEnter" => Some(b"\r"),
        "Escape" | "Esc" | "C-[" => Some(b"\x1b"),
        "Tab" | "C-i" => Some(b"\t"),
        "BSpace" => Some(b"\x7f"),
        "Space" => Some(b" "),
        "Up" => Some(b"\x1b[A"),
        "Down" => Some(b"\x1b[B"),
        "Right" => Some(b"\x1b[C"),
        "Left" => Some(b"\x1b[D"),
        "Home" => Some(b"\x1b[H"),
        "End" => Some(b"\x1b[F"),
        "PageUp" | "PPage" => Some(b"\x1b[5~"),
        "PageDown" | "NPage" => Some(b"\x1b[6~"),
        "DC" | "Delete" => Some(b"\x1b[3~"),
        _ => None,
    };
    if let Some(b) = named {
        return b.to_vec();
    }
    // C-x: the control character.
    if let Some(c) = k.strip_prefix("C-").filter(|c| c.len() == 1).and_then(|c| c.chars().next()) {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() {
            return vec![c as u8 - b'a' + 1];
        }
    }
    if let Some(rest) = k.strip_prefix("M-").filter(|c| !c.is_empty()) {
        let mut v = vec![0x1b];
        v.extend(key_bytes(rest));
        return v;
    }
    k.as_bytes().to_vec()
}

/// tmux's `#{name}` (and `#D #P #S #W #T`) for a pane.
fn expand(fmt: &str, snap: &Snapshot, term: TermId) -> String {
    let t = snap.terms.get(&term);
    let ws = snap.workspaces.iter().position(|w| w.tabs.iter().any(|tab| tab.layout.contains(term)));
    let value = |name: &str| -> String {
        match name {
            "pane_id" => format!("%{term}"),
            "pane_index" => ws
                .and_then(|i| snap.workspaces[i].tabs.iter().find(|tab| tab.layout.contains(term)))
                .and_then(|tab| tab.layout.leaves().iter().position(|l| *l == term))
                .unwrap_or(0)
                .to_string(),
            "pane_title" => t.map(|t| if t.label.is_empty() { t.title.clone() } else { t.label.clone() }).unwrap_or_default(),
            "pane_current_path" => t.map(|t| t.cwd.display().to_string()).unwrap_or_default(),
            "pane_current_command" => t.map(|t| t.process.clone()).unwrap_or_default(),
            "pane_width" => t.map(|t| t.cols.to_string()).unwrap_or_default(),
            "pane_height" => t.map(|t| t.rows.to_string()).unwrap_or_default(),
            "pane_active" => (ws.and_then(|i| snap.workspaces[i].tab()).is_some_and(|tab| tab.focus == term) as u8).to_string(),
            "pane_dead" | "pane_in_mode" | "window_zoomed_flag" => "0".into(),
            "session_name" => "hydra".into(),
            "session_id" => "$0".into(),
            "session_attached" | "session_windows" => "1".into(),
            "window_id" => format!("@{}", ws.map(|i| snap.workspaces[i].id).unwrap_or(0)),
            "window_index" => ws.unwrap_or(0).to_string(),
            "window_name" => ws.map(|i| snap.workspaces[i].name.clone()).unwrap_or_default(),
            "window_active" => "1".into(),
            "version" => "3.4".into(),
            "pid" => std::process::id().to_string(),
            _ => {
                log(&format!("  (format variable not known: {name})"));
                String::new()
            }
        }
    };
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '#' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('{') => {
                let name: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.push_str(&value(&name));
            }
            Some('D') => out.push_str(&value("pane_id")),
            Some('P') => out.push_str(&value("pane_index")),
            Some('S') => out.push_str(&value("session_name")),
            Some('W') => out.push_str(&value("window_name")),
            Some('T') => out.push_str(&value("pane_title")),
            Some('I') => out.push_str(&value("window_index")),
            Some('#') => out.push('#'),
            Some(o) => {
                out.push('#');
                out.push(o);
            }
            None => out.push('#'),
        }
    }
    out
}

/// The panes this shim opened, so a tool's kill-session closes its own panes only.
fn remember(term: TermId) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(shim_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(shim_dir().join("panes")) {
        let _ = writeln!(f, "{term}");
    }
}

fn remembered() -> Vec<TermId> {
    std::fs::read_to_string(shim_dir().join("panes")).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn flags_as_tmux_reads_them() {
        let f = Flags::parse(&s(&["-h", "-d", "-P", "-F", "#{pane_id}", "-t", "%3", "-c/tmp", "claude", "--agent"]), "tcelpF");
        assert!(f.has('h') && f.has('d') && f.has('P'));
        assert_eq!((f.val('F'), f.val('t'), f.val('c')), (Some("#{pane_id}"), Some("%3"), Some("/tmp")));
        assert_eq!(f.rest, s(&["claude", "--agent"]), "the command keeps its own flags");
        let f = Flags::parse(&s(&["-dPF#{pane_id}"]), "tF");
        assert!(f.has('d') && f.has('P') && f.val('F') == Some("#{pane_id}"), "flags bundled together");
    }

    #[test]
    fn keys_become_terminal_bytes() {
        assert_eq!(key_bytes("Enter"), b"\r");
        assert_eq!(key_bytes("C-c"), vec![3]);
        assert_eq!(key_bytes("M-x"), b"\x1bx");
        assert_eq!(key_bytes("claude --resume"), b"claude --resume");
    }

    #[test]
    fn formats_fill_in_a_pane() {
        use crate::layout::Node;
        use crate::protocol::{TabInfo, WorkspaceInfo};
        let mut snap = Snapshot::default();
        snap.workspaces.push(WorkspaceInfo {
            id: 7,
            name: "team".into(),
            cwd: PathBuf::from("/code"),
            tabs: vec![TabInfo { id: 1, name: String::new(), layout: Node::Leaf(4), focus: 4 }],
            active_tab: 1,
            git: None,
            worktree: false,
            color: 0,
            is_new: false,
            group: None,
        });
        assert_eq!(expand("#{pane_id} #{session_name} #{window_id} #D ##", &snap, 4), "%4 hydra @7 %4 #");
    }
}
