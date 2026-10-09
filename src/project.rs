//! Per-project settings that live in the repo, `.seshi.toml`: how to run its dev server, and
//! commands to run when a worktree is made or removed.
//!
//! ```toml
//! [dev]
//! run = "npm run dev"
//! ready = "ready in|listening on"   # a regex: the server is up once its output shows it
//! port = 3000                        # each worktree gets its own: 3000, 3001, … (as $PORT)
//!
//! [hooks]
//! on_create = "npm install"
//! on_remove = ""
//! ```

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Project {
    pub dev: Option<Dev>,
    pub hooks: Hooks,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Dev {
    pub run: String,
    pub ready: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hooks {
    pub on_create: String,
    pub on_remove: String,
}

pub const FILE: &str = ".seshi.toml";
/// What the file was called before the rename; still read when there's no `FILE`.
const OLD_FILE: &str = ".hydra.toml";

/// Longest a worktree hook may run before it's stopped.
pub const HOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// A repo's hook commands run only once the user has allowed them (`seshi allow`, like
/// direnv): a cloned repo's `.seshi.toml` can't run code by itself. Approvals are kept per
/// repo and command, in a file beside the config that is never synced.
fn allowed_path() -> PathBuf {
    crate::config::config_path().with_file_name("allowed-hooks.json")
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
struct Allowed {
    repo: String,
    command: String,
}

fn repo_key(dir: &Path) -> String {
    let root = crate::gitfs::head(dir).map(|h| h.main_root).unwrap_or_else(|| dir.to_path_buf());
    let s = root.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) { s.to_lowercase() } else { s.to_string() }
}

fn read_allowed(file: &Path) -> Vec<Allowed> {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Whether the user allowed `cmd` for the repo `dir` is in.
pub fn allowed(dir: &Path, cmd: &str) -> bool {
    allowed_in(&allowed_path(), dir, cmd)
}

fn allowed_in(file: &Path, dir: &Path, cmd: &str) -> bool {
    let repo = repo_key(dir);
    read_allowed(file).iter().any(|a| a.repo == repo && a.command == cmd.trim())
}

/// Allow these commands for the repo `dir` is in (replacing what was allowed before).
pub fn allow(dir: &Path, cmds: &[&str]) -> anyhow::Result<()> {
    allow_in(&allowed_path(), dir, cmds)
}

fn allow_in(file: &Path, dir: &Path, cmds: &[&str]) -> anyhow::Result<()> {
    use anyhow::Context;
    let repo = repo_key(dir);
    let mut list = read_allowed(file);
    list.retain(|a| a.repo != repo);
    list.extend(cmds.iter().filter(|c| !c.trim().is_empty()).map(|c| Allowed { repo: repo.clone(), command: c.trim().to_string() }));
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&list)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("saving {}", file.display()))
}

/// The settings for a checkout: its own `.seshi.toml` (or `.hydra.toml`), else the main
/// checkout's.
pub fn load(dir: &Path) -> Project {
    let main = crate::gitfs::head(dir).map(|h| h.main_root);
    for d in std::iter::once(dir.to_path_buf()).chain(main) {
        for name in [FILE, OLD_FILE] {
            if let Ok(text) = std::fs::read_to_string(d.join(name)) {
                return toml::from_str(&text).unwrap_or_default();
            }
        }
    }
    Project::default()
}

/// The port for a checkout: the base for the main one, then one more for each worktree in
/// `git worktree list` order (stable while worktrees come and go at the end).
pub fn port_for(dir: &Path, base: u16) -> u16 {
    let key = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["worktree", "list", "--porcelain"]).output();
    let list: Vec<PathBuf> = out
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.strip_prefix("worktree ").map(PathBuf::from)).collect())
        .unwrap_or_default();
    let i = list.iter().position(|p| key(p) == key(dir)).unwrap_or(0);
    base.saturating_add(i as u16)
}

/// Run a hook to the end (in `dir`, through the shell, with `vars` set). Its output's last
/// lines on failure.
pub fn run_hook(shell: &[String], dir: &Path, cmd: &str, vars: &[(&str, String)]) -> Result<(), String> {
    let mut c = crate::proc::shell(shell, cmd, false);
    c.current_dir(dir).stdin(std::process::Stdio::null());
    for (k, v) in vars {
        c.env(k, v);
    }
    crate::proc::quiet(&mut c);
    c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("couldn't run it: {e}"))?;
    // Read output on threads so a chatty hook can't fill the pipe and stall.
    fn drain<R: std::io::Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if started.elapsed() > HOOK_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("stopped after {} minutes", HOOK_TIMEOUT.as_secs() / 60));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(format!("couldn't wait for it: {e}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let text = format!("{}{}", out.join().unwrap_or_default(), err.join().unwrap_or_default());
    let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    Err(tail[tail.len().saturating_sub(2)..].join(" / "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_project_file() {
        let p: Project = toml::from_str("[dev]\nrun = \"npm run dev\"\nready = \"ready in\"\nport = 3000\n[hooks]\non_create = \"npm install\"\n").unwrap();
        assert_eq!(p.dev.as_ref().unwrap().port, Some(3000));
        assert_eq!(p.hooks.on_create, "npm install");
    }

    #[test]
    fn hooks_run_only_once_allowed() {
        let dir = std::env::temp_dir().join(format!("seshi-allow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("allowed-hooks.json");
        assert!(!allowed_in(&file, &dir, "npm install"), "nothing is allowed at first");
        allow_in(&file, &dir, &["npm install"]).unwrap();
        assert!(allowed_in(&file, &dir, "npm install"));
        assert!(!allowed_in(&file, &dir, "npm install && curl x | sh"), "a changed command asks again");
        allow_in(&file, &dir, &["pnpm i"]).unwrap();
        assert!(!allowed_in(&file, &dir, "npm install"), "allowing again replaces the old list");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
