//! The saved session: enough to rebuild every workspace, tab and pane after the server (or
//! the machine) restarts. Written atomically, a few seconds after the model settles.

use crate::layout::Node;
use crate::protocol::TermId;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub workspaces: Vec<SavedWs>,
    /// Index into `workspaces`.
    pub active: usize,
    /// Worktrees hydra created (the only ones it may remove by itself).
    #[serde(default)]
    pub made_worktrees: Vec<PathBuf>,
    /// Queued work not started yet.
    #[serde(default)]
    pub queue: Vec<crate::protocol::QueueItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedWs {
    pub name: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub color: Option<u8>,
    #[serde(default)]
    pub group: Option<String>,
    pub tabs: Vec<SavedTab>,
    pub active_tab: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedTab {
    pub name: String,
    /// Uses the pane ids of the previous run; remapped on restore.
    pub layout: Node,
    pub focus: TermId,
    pub panes: BTreeMap<TermId, SavedPane>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedPane {
    pub cwd: Option<PathBuf>,
    /// The command the pane was started with, if not a plain shell.
    pub cmd: Option<String>,
    pub agent: Option<String>,
    /// Agent session id reported by hooks, for `resume`.
    pub session: Option<String>,
    /// Finished and not looked at yet.
    #[serde(default)]
    pub unseen: bool,
    /// What it was called (its /rename, else its first prompt) and its model.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
}

pub fn path() -> PathBuf {
    let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
    crate::config::data_dir().join(format!("session-{label}.json"))
}

pub fn load() -> Option<Saved> {
    let s = std::fs::read_to_string(path()).ok()?;
    match serde_json::from_str(&s) {
        Ok(saved) => Some(saved),
        Err(e) => {
            tracing::warn!("ignoring unreadable session file: {e}");
            None
        }
    }
}

pub fn save(saved: &Saved) -> Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(saved)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn forget() {
    if cfg!(test) {
        return;
    }
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_dir_all(output_dir());
}

/// Where each pane's recent output is kept, so its history is still there after the server
/// restarts (an update, a reboot): one file per pane, named by its id.
pub fn output_dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("hydra-test-output-{}", std::process::id()));
    }
    let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
    crate::config::data_dir().join(format!("output-{label}"))
}

fn output_file(term: TermId) -> PathBuf {
    output_dir().join(format!("{term}.bin"))
}

/// Keep `bytes` as pane `term`'s output, and drop the files of panes not in `live`.
pub fn save_outputs(outputs: &[(TermId, Vec<u8>)], live: &[TermId]) -> Result<()> {
    let dir = output_dir();
    std::fs::create_dir_all(&dir)?;
    for (term, bytes) in outputs {
        let path = output_file(*term);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    for e in std::fs::read_dir(&dir)?.flatten() {
        let id = e.path().file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<TermId>().ok());
        if id.is_none_or(|id| !live.contains(&id)) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(())
}

/// A pane's kept output, taken (its file goes: the pane it's restored into saves its own).
pub fn take_output(term: TermId) -> Option<Vec<u8>> {
    let path = output_file(term);
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(bytes)
}
