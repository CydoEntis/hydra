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
    let _ = std::fs::remove_file(path());
}
