//! Per-project settings that live in the repo, `.hydra.toml`: how to run its dev server, and
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

pub const FILE: &str = ".hydra.toml";

/// The settings for a checkout: its own `.hydra.toml`, else the main checkout's.
pub fn load(dir: &Path) -> Project {
    let main = crate::gitfs::head(dir).map(|h| h.main_root);
    for d in std::iter::once(dir.to_path_buf()).chain(main) {
        if let Ok(text) = std::fs::read_to_string(d.join(FILE)) {
            return toml::from_str(&text).unwrap_or_default();
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
    let exe = Path::new(&shell[0]).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut c = std::process::Command::new(&shell[0]);
    match exe.as_str() {
        "pwsh" | "powershell" => c.args(["-NoProfile", "-NonInteractive", "-Command", cmd]),
        "cmd" => c.args(["/C", cmd]),
        _ => c.args(["-c", cmd]),
    };
    c.current_dir(dir).stdin(std::process::Stdio::null());
    for (k, v) in vars {
        c.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().map_err(|e| format!("couldn't run it: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
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
}
