//! One pseudoterminal and the process in it. Reading happens on a dedicated thread that
//! feeds the daemon's event loop; everything else is touched only from the loop.

use super::Ev;
use crate::config::Config;
use crate::protocol::{Status, TermId};
use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct Callbacks {
    pub title: String,
    pub title_changed: bool,
}

impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).trim().to_string();
        self.title_changed = true;
    }
}

pub struct Term {
    pub id: TermId,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pub pid: Option<u32>,
    /// The daemon's own view of the screen, for status patterns and `hydra read`.
    pub parser: vt100::Parser<Callbacks>,
    ring: VecDeque<u8>,
    ring_cap: usize,
    pub cols: u16,
    pub rows: u16,
    pub process: String,
    pub agent: Option<String>,
    pub status: Status,
    /// When the status last changed (unix seconds), for "working 3m".
    pub status_since: u64,
    /// Hooks reported for this agent session: trust them over heuristics.
    pub hooked: bool,
    pub last_output: Instant,
    pub last_input: Instant,
    /// The command the pane was started with (None for a plain shell).
    pub cmd: Option<String>,
    /// Agent session id from hooks, for resuming after a restart.
    pub session: Option<String>,
    pub cwd: PathBuf,
    /// The agent's last prompt, for its card.
    pub summary: String,
    /// The agent's last message.
    pub said: String,
    /// Git facts for `cwd`: branch, and whether it's a linked worktree.
    pub head: Option<crate::gitfs::Head>,
    /// Running subagents: (id, kind).
    pub subagents: Vec<(String, String)>,
    /// The shell reports its directory itself (OSC 7 / 9;9); trust that over process scans.
    pub cwd_reported: bool,
    /// Stopped on purpose to save memory; wakes (resumed) when focused.
    pub asleep: bool,
}

pub struct SpawnSpec<'a> {
    pub id: TermId,
    pub cmd: Option<&'a str>,
    pub cwd: &'a Path,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, PartialEq)]
enum Query {
    CursorPosition,
    Status,
    PrimaryAttributes,
    SecondaryAttributes,
}

const QUERIES: &[(&[u8], Query)] = &[
    (b"\x1b[6n", Query::CursorPosition),
    (b"\x1b[5n", Query::Status),
    (b"\x1b[0c", Query::PrimaryAttributes),
    (b"\x1b[c", Query::PrimaryAttributes),
    (b"\x1b[>0c", Query::SecondaryAttributes),
    (b"\x1b[>c", Query::SecondaryAttributes),
];

/// Earliest terminal query in `data`: (offset, length, kind).
fn find_query(data: &[u8]) -> Option<(usize, usize, &'static Query)> {
    let mut i = 0;
    while let Some(p) = data[i..].iter().position(|&b| b == 0x1b) {
        let at = i + p;
        for (seq, q) in QUERIES {
            if data[at..].starts_with(seq) {
                return Some((at, seq.len(), q));
            }
        }
        i = at + 1;
    }
    None
}

/// The last working-directory report in `data`: OSC 7 (`file://host/path`, most shells
/// and prompts) or OSC 9;9 (Windows Terminal's convention, oh-my-posh, starship).
fn find_cwd_report(data: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(data);
    let mut found = None;
    for (intro, url) in [("\x1b]7;", true), ("\x1b]9;9;", false)] {
        let mut rest = &text[..];
        while let Some(i) = rest.find(intro) {
            rest = &rest[i + intro.len()..];
            let end = rest.find(['\x07', '\x1b']).unwrap_or(rest.len());
            let body = &rest[..end];
            let path = if url { from_file_url(body) } else { Some(body.trim_matches('"').to_string()) };
            if let Some(p) = path.filter(|p| !p.is_empty()) {
                found = Some(PathBuf::from(p));
            }
        }
    }
    found
}

fn from_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let raw = path.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = (raw[i] == b'%' && i + 2 < raw.len())
            .then(|| std::str::from_utf8(&raw[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match hex {
            Some(b) => {
                bytes.push(b);
                i += 3;
            }
            None => {
                bytes.push(raw[i]);
                i += 1;
            }
        }
    }
    let s = String::from_utf8_lossy(&bytes).into_owned();
    // "/C:/Users/x" -> "C:/Users/x"
    let b = s.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[2] == b':' {
        return Some(s[1..].to_string());
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_reports() {
        assert_eq!(find_cwd_report(b"x\x1b]7;file://host/home/me/a%20b\x07y"), Some(PathBuf::from("/home/me/a b")));
        assert_eq!(find_cwd_report(b"\x1b]7;file://pc/C:/dev/x\x1b\\"), Some(PathBuf::from("C:/dev/x")));
        assert_eq!(find_cwd_report(b"\x1b]9;9;\"C:\\dev\\y\"\x07"), Some(PathBuf::from(r"C:\dev\y")));
        assert_eq!(find_cwd_report(b"plain output"), None);
    }

    #[test]
    fn queries() {
        assert_eq!(find_query(b"ab\x1b[6ncd").map(|q| (q.0, q.1)), Some((2, 4)));
        assert_eq!(find_query(b"\x1b[31m\x1b[c").map(|q| q.0), Some(5));
        assert!(find_query(b"\x1b[1;5A").is_none());
    }
}

/// Build the argv for a pane: a plain shell, or a command run inside one so PATH lookups,
/// `.cmd` shims and aliases work, and the pane falls back to the shell when it exits.
/// PowerShell doesn't tell the terminal where it is. This wraps whatever prompt the user's
/// profile set up so each prompt also reports the directory (OSC 9;9, the Windows Terminal
/// convention), which keeps the sidebar, splits and restores in the right folder. Runs after
/// the profile; uses no double quotes so it survives Windows argument quoting.
const PWSH_CWD_HOOK: &str = "$global:__hydraPrompt = $function:prompt; \
function global:prompt { $l = $executionContext.SessionState.Path.CurrentLocation; \
if ($l.Provider.Name -eq 'FileSystem') { [Console]::Write([char]27 + ']9;9;' + [char]34 + $l.ProviderPath + [char]34 + [char]7) }; \
& $global:__hydraPrompt }";

fn argv(cfg: &Config, cmd: Option<&str>) -> Vec<String> {
    let mut shell = cfg.shell_command();
    let cmd = cmd.filter(|c| !c.trim().is_empty());
    let exe = Path::new(&shell[0])
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match (exe.as_str(), cmd) {
        ("pwsh" | "powershell", cmd) if cfg.shell_integration => {
            let script = match cmd {
                Some(c) => format!("{PWSH_CWD_HOOK}; {c}"),
                None => PWSH_CWD_HOOK.to_string(),
            };
            shell.extend(["-NoExit".into(), "-Command".into(), script]);
        }
        (_, None) => {}
        ("pwsh" | "powershell", Some(c)) => shell.extend(["-NoExit".into(), "-Command".into(), c.into()]),
        ("cmd", Some(c)) => shell.extend(["/K".into(), c.into()]),
        ("nu", Some(c)) => shell.extend(["-e".into(), c.into()]),
        (_, Some(c)) => {
            let sh = shell[0].clone();
            shell.extend(["-ic".into(), format!("{c}; exec {sh}")]);
        }
    }
    shell
}

impl Term {
    pub fn spawn(cfg: &Config, spec: SpawnSpec, tx: mpsc::Sender<Ev>) -> Result<Term> {
        let pty = native_pty_system();
        let size = PtySize { rows: spec.rows.max(2), cols: spec.cols.max(2), pixel_width: 0, pixel_height: 0 };
        let pair = pty.openpty(size).context("opening pty")?;

        let args = argv(cfg, spec.cmd);
        let mut cmd = CommandBuilder::new(&args[0]);
        cmd.args(&args[1..]);
        if spec.cwd.is_dir() {
            cmd.cwd(spec.cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "hydra");
        cmd.env("HYDRA", "1");
        cmd.env("HYDRA_TERM_ID", spec.id.to_string());
        if let Ok(sock) = std::env::var("HYDRA_SOCKET") {
            cmd.env("HYDRA_SOCKET", sock);
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("spawning {args:?}"))?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let id = spec.id;

        let out_tx = tx.clone();
        std::thread::Builder::new().name(format!("pty-read-{id}")).spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out_tx.blocking_send(Ev::Output(id, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = out_tx.blocking_send(Ev::Exited(id));
        })?;
        // ConPTY keeps the output pipe open after the child exits, so wait on the child too.
        std::thread::Builder::new().name(format!("pty-wait-{id}")).spawn(move || {
            let _ = child.wait();
            let _ = tx.blocking_send(Ev::Exited(id));
        })?;

        let process = Path::new(&args[0])
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let now = Instant::now();
        Ok(Term {
            id,
            master: pair.master,
            writer,
            killer,
            pid,
            parser: vt100::Parser::new_with_callbacks(size.rows, size.cols, 0, Callbacks::default()),
            ring: VecDeque::new(),
            ring_cap: cfg.replay_bytes.max(64 * 1024),
            cols: size.cols,
            rows: size.rows,
            process,
            agent: None,
            status: Status::None,
            status_since: unix_now(),
            hooked: false,
            last_output: now,
            last_input: now,
            cmd: spec.cmd.filter(|c| !c.trim().is_empty()).map(str::to_string),
            session: None,
            cwd: spec.cwd.to_path_buf(),
            summary: String::new(),
            said: String::new(),
            head: crate::gitfs::head(spec.cwd),
            subagents: Vec::new(),
            cwd_reported: false,
            asleep: false,
        })
    }

    /// Returns true when the reported working directory changed.
    pub fn output(&mut self, data: &[u8]) -> bool {
        self.process_answering_queries(data);
        let mut cwd_changed = false;
        if let Some(dir) = find_cwd_report(data)
            && dir != self.cwd
        {
            self.cwd = dir;
            cwd_changed = true;
        }
        if cwd_changed {
            self.cwd_reported = true;
        }
        self.ring.extend(data);
        let excess = self.ring.len().saturating_sub(self.ring_cap);
        if excess > 0 {
            self.ring.drain(..excess);
        }
        self.last_output = Instant::now();
        cwd_changed
    }

    /// Feed output to the emulator, answering terminal queries as we reach them. The daemon
    /// is the terminal as far as the program is concerned, and some programs block until
    /// answered: ConPTY itself sends `ESC[6n` at startup and waits for the cursor report.
    fn process_answering_queries(&mut self, data: &[u8]) {
        let mut rest = data;
        while let Some((at, len, query)) = find_query(rest) {
            self.parser.process(&rest[..at + len]);
            let reply = match query {
                Query::CursorPosition => {
                    let (row, col) = self.parser.screen().cursor_position();
                    format!("\x1b[{};{}R", row + 1, col + 1).into_bytes()
                }
                Query::Status => b"\x1b[0n".to_vec(),
                Query::PrimaryAttributes => b"\x1b[?62;22c".to_vec(),
                Query::SecondaryAttributes => b"\x1b[>0;10;1c".to_vec(),
            };
            let _ = self.writer.write_all(&reply);
            let _ = self.writer.flush();
            rest = &rest[at + len..];
        }
        self.parser.process(rest);
    }

    pub fn replay(&self) -> Vec<u8> {
        let (a, b) = self.ring.as_slices();
        // Restore the screen we know is current after replaying history, so a ring that
        // starts mid-escape-sequence or spans resizes still ends on the right picture.
        let mut out = Vec::with_capacity(a.len() + b.len() + 4096);
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");
        out.extend(self.parser.screen().state_formatted());
        out
    }

    pub fn input(&mut self, data: &[u8]) {
        self.last_input = Instant::now();
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(2));
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Re-read the git branch for the current folder. Returns true if it changed.
    pub fn refresh_head(&mut self) -> bool {
        let head = crate::gitfs::head(&self.cwd);
        let changed = head != self.head;
        self.head = head;
        changed
    }

    /// Stop everything the pane runs: the shell and what it started (an agent is usually a
    /// child of the shell, and on Windows it would otherwise keep the terminal open). Runs
    /// on its own thread so nothing waits on it.
    pub fn kill_tree(&mut self) {
        let mut killer = self.killer.clone_killer();
        let pid = self.pid;
        std::thread::spawn(move || {
            if let Some(pid) = pid {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .creation_flags(0x0800_0000)
                        .status();
                }
                #[cfg(not(windows))]
                {
                    // The pane's shell leads its own process group.
                    let _ = std::process::Command::new("kill")
                        .args(["-KILL", &format!("-{pid}")])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                }
            }
            let _ = killer.kill();
        });
    }

    pub fn title(&self) -> &str {
        &self.parser.callbacks().title
    }

    /// The bottom `n` rows of the screen as text, for status patterns.
    pub fn tail_text(&self, n: u16) -> String {
        let screen = self.parser.screen();
        let rows: Vec<String> = screen.rows(0, self.cols).collect();
        let start = rows.len().saturating_sub(n as usize);
        rows[start..].join("\n")
    }
}

/// Seconds since the unix epoch.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
