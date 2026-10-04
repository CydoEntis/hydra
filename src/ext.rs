//! Extensions: a folder in `<config>/extensions/<name>/` with a `hydra-ext.toml` that adds
//! commands to the palette, labels to worktree rows, and commands run when things happen.
//!
//! ```toml
//! name = "deploy"
//! description = "Preview deploys per worktree"
//!
//! [[commands]]              # palette entries (Ctrl+Space space)
//! title = "Deploy a preview"
//! run = "./deploy.sh"       # `./` is the extension's folder
//! background = true         # run hidden and show its last line (else: in a pane beside)
//!
//! [[labels]]                # a short label on every worktree row in the sidebar
//! run = "./status.sh"       # its first line of output is the label
//! every = 60                # seconds
//!
//! [hooks]                   # run when things happen
//! on_agent_start = ""
//! on_agent_done = "./notify.sh"
//! on_needs_you = ""
//! on_worktree_create = ""
//! on_worktree_remove = ""
//! ```
//!
//! Every command runs in the worktree it's about, through your shell, with HYDRA_EXT_DIR,
//! HYDRA_EVENT, HYDRA_WORKTREE, HYDRA_BRANCH, HYDRA_REPO, HYDRA_TERM_ID, HYDRA_AGENT,
//! HYDRA_STATUS, HYDRA_PROMPT and HYDRA_SAID set when they apply.

use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "hydra-ext.toml";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Ext {
    pub name: String,
    pub description: String,
    pub commands: Vec<ExtCommand>,
    pub labels: Vec<ExtLabel>,
    pub hooks: ExtHooks,
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExtCommand {
    pub title: String,
    pub run: String,
    pub background: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExtLabel {
    pub run: String,
    pub every: u64,
}

impl Default for ExtLabel {
    fn default() -> Self {
        ExtLabel { run: String::new(), every: 60 }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExtHooks {
    pub on_agent_start: String,
    pub on_agent_done: String,
    pub on_needs_you: String,
    pub on_worktree_create: String,
    pub on_worktree_remove: String,
}

impl ExtHooks {
    pub fn get(&self, event: &str) -> &str {
        match event {
            "agent_start" => &self.on_agent_start,
            "agent_done" => &self.on_agent_done,
            "needs_you" => &self.on_needs_you,
            "worktree_create" => &self.on_worktree_create,
            "worktree_remove" => &self.on_worktree_remove,
            _ => "",
        }
    }
}

pub fn dir() -> PathBuf {
    crate::config::config_path().parent().map(|p| p.join("extensions")).unwrap_or_else(|| PathBuf::from("extensions"))
}

/// Every extension, by folder name; broken manifests are skipped (with why).
pub fn load_all() -> (Vec<Ext>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir()) else { return (out, errors) };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join(MANIFEST).is_file()).collect();
    dirs.sort();
    for d in dirs {
        let text = std::fs::read_to_string(d.join(MANIFEST)).unwrap_or_default();
        match toml::from_str::<Ext>(&text) {
            Ok(mut e) => {
                if e.name.trim().is_empty() {
                    e.name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                }
                e.dir = d;
                out.push(e);
            }
            Err(err) => errors.push(format!("{}: {err}", d.join(MANIFEST).display())),
        }
    }
    (out, errors)
}

/// `./x` at the start of a command means the extension's folder.
pub fn resolve(ext_dir: &Path, cmd: &str) -> String {
    match cmd.trim().strip_prefix("./") {
        Some(rest) => {
            let p = ext_dir.join(rest.split_whitespace().next().unwrap_or(""));
            let args = rest.split_once(char::is_whitespace).map(|(_, a)| format!(" {a}")).unwrap_or_default();
            format!("\"{}\"{args}", p.display())
        }
        None => cmd.to_string(),
    }
}

/// Run an extension command to the end in `cwd` and return its output's first (label) or
/// last (result) line.
pub fn run(shell: &[String], ext: &Ext, cwd: &Path, cmd: &str, vars: &[(String, String)]) -> Result<String, String> {
    let mut c = crate::proc::shell(shell, &resolve(&ext.dir, cmd), false);
    c.current_dir(if cwd.is_dir() { cwd } else { &ext.dir }).stdin(std::process::Stdio::null());
    c.env("HYDRA_EXT_DIR", &ext.dir);
    for (k, v) in vars {
        c.env(k, v);
    }
    crate::proc::quiet(&mut c);
    let out = c.output().map_err(|e| format!("couldn't run {cmd}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().chain(text.lines().rev()).find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string();
        Err(last)
    }
}

/// The environment for something about `dir` (and maybe a pane).
pub fn vars(event: &str, dir: &Path, term: Option<&crate::protocol::TermInfo>) -> Vec<(String, String)> {
    let mut v = vec![("HYDRA_EVENT".to_string(), event.to_string()), ("HYDRA_WORKTREE".to_string(), dir.display().to_string())];
    if let Some(h) = crate::gitfs::head(dir) {
        v.push(("HYDRA_BRANCH".into(), h.branch));
        v.push(("HYDRA_REPO".into(), h.main_root.display().to_string()));
    }
    if let Some(t) = term {
        v.push(("HYDRA_TERM_ID".into(), t.id.to_string()));
        v.push(("HYDRA_AGENT".into(), t.agent.clone().unwrap_or_default()));
        v.push(("HYDRA_STATUS".into(), t.status.label().to_string()));
        v.push(("HYDRA_PROMPT".into(), t.summary.clone()));
        v.push(("HYDRA_SAID".into(), t.said.chars().take(4000).collect()));
    }
    v
}

/// `hydra ext new <name>`: a starter extension to edit.
pub fn scaffold(name: &str) -> anyhow::Result<PathBuf> {
    let d = dir().join(name);
    anyhow::ensure!(!d.exists(), "{} already exists", d.display());
    std::fs::create_dir_all(&d)?;
    let script = if cfg!(windows) { "hello.ps1" } else { "hello.sh" };
    let manifest = format!(
        r#"name = "{name}"
description = "What it does"

# Palette entries (Ctrl+Space space). `./` is this folder.
[[commands]]
title = "{name}: say hello"
run = "./{script}"
background = true

# A label on every worktree row; the first line it prints.
# [[labels]]
# run = "git log -1 --format=%cr"
# every = 120

# Run when things happen (HYDRA_EVENT, HYDRA_WORKTREE, HYDRA_AGENT, HYDRA_SAID, ...).
[hooks]
on_agent_done = ""
on_needs_you = ""
"#
    );
    std::fs::write(d.join(MANIFEST), manifest)?;
    let body = if cfg!(windows) {
        "Write-Output \"hello from $env:HYDRA_EXT_DIR, on $env:HYDRA_BRANCH\"\n"
    } else {
        "#!/bin/sh\necho \"hello from $HYDRA_EXT_DIR, on $HYDRA_BRANCH\"\n"
    };
    std::fs::write(d.join(script), body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(d.join(script), std::fs::Permissions::from_mode(0o755));
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_paths() {
        let e: Ext = toml::from_str("name = \"x\"\n[[commands]]\ntitle = \"Go\"\nrun = \"./go.sh --fast\"\n[[labels]]\nrun = \"echo hi\"\n[hooks]\non_agent_done = \"./done.sh\"\n").unwrap();
        assert_eq!(e.commands[0].title, "Go");
        assert_eq!(e.labels[0].every, 60);
        assert_eq!(e.hooks.get("agent_done"), "./done.sh");
        let r = resolve(Path::new("/ext/x"), "./go.sh --fast");
        assert!(r.ends_with("go.sh\" --fast") && r.starts_with('"'), "{r}");
        assert_eq!(resolve(Path::new("/ext/x"), "lazygit"), "lazygit");
    }
}
