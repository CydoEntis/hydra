//! Wire protocol between the daemon and its clients (the TUI and one-shot CLI calls).
//!
//! Frames are length-delimited MessagePack. The daemon is the single source of truth for
//! the workspace / tab / pane tree; clients render `Snapshot`s and send `Command`s.

use crate::layout::{Dir, Node};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Bump whenever a message shape changes; client and daemon refuse to talk across versions.
pub const PROTOCOL_VERSION: u32 = 18;

pub type TermId = u32;
pub type WsId = u32;
pub type TabId = u32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    /// First frame of every connection. `attach` clients get replays and live output.
    Hello { version: u32, attach: bool },
    Command(Command),
    Input { term: TermId, #[serde(with = "serde_bytes")] data: Vec<u8> },
    Resize { term: TermId, cols: u16, rows: u16 },
    /// Lifecycle event reported by an agent's hook (`hydra hook ...`).
    Hook {
        term: TermId,
        agent: String,
        status: HookStatus,
        session: Option<String>,
        cwd: Option<PathBuf>,
        /// The prompt the user just gave the agent, for its card.
        prompt: Option<String>,
        /// The agent's last message (its summary of what it did).
        said: Option<String>,
        /// A subagent starting or stopping.
        subagent: Option<Subagent>,
        /// The hook event ("Stop", "Notification:permission_prompt", …).
        event: String,
        /// The reporting process; only reports from inside the pane's own process tree count.
        pid: u32,
        /// The agent's conversation file (Claude's transcript), for moving it elsewhere.
        transcript: Option<PathBuf>,
        /// The model it's answering with right now (from the transcript).
        model: Option<String>,
        /// The name you gave the conversation in the agent (Claude's /rename).
        name: Option<String>,
    },
    Query(Query),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    NewWorkspace { cwd: Option<PathBuf>, name: Option<String>, cmd: Option<String> },
    CloseWorkspace { ws: WsId },
    RenameWorkspace { ws: WsId, name: String },
    SelectWorkspace { ws: WsId },
    NewTab { ws: WsId, name: Option<String>, cmd: Option<String> },
    CloseTab { ws: WsId, tab: TabId },
    RenameTab { ws: WsId, tab: TabId, name: String },
    SelectTab { ws: WsId, tab: TabId },
    /// `cwd`: where the new pane starts (default: where the split pane is).
    Split { term: TermId, dir: Dir, cmd: Option<String>, cwd: Option<PathBuf> },
    ClosePane { term: TermId },
    /// Focus a pane anywhere: switches workspace and tab as needed.
    FocusPane { term: TermId },
    /// You've seen a finished agent (the cursor rested on it): done → idle.
    MarkSeen { term: TermId },
    /// A checkout's dev server (from its `.hydra.toml`): start, stop or restart it.
    Dev { dir: PathBuf, action: DevAction },
    /// The agent in `term` moves into a new worktree of its repo: made now, and when its turn
    /// ends it restarts there, resumed (the agent runs `hydra worktree --move`).
    MoveToWorktree { term: TermId, branch: Option<String> },
    /// Grow (positive) or shrink the pane along `dir`'s axis by `delta` (fraction of parent).
    ResizePane { term: TermId, dir: Dir, delta: f32 },
    /// `git worktree add` next to the workspace's repo, then open it as a new workspace.
    /// `split`: open the worktree as a pane beside this one instead of as a workspace.
    /// `from`: the repo to branch from (default: the workspace's folder).
    NewWorktree {
        ws: WsId,
        branch: String,
        base: Option<String>,
        cmd: Option<String>,
        split: Option<TermId>,
        from: Option<PathBuf>,
    },
    /// Put a pane in a sidebar group (None: take it out).
    SetGroup { ws: WsId, group: Option<String> },
    /// Move a pane into another group (`to`), or into a new group named `name`.
    MovePane { term: TermId, to: Option<WsId>, name: Option<String> },
    /// Put back the pane the last automatic workspace took.
    UndoAutoWorkspace,
    /// Close a worktree workspace and `git worktree remove` its checkout.
    /// `delete_branch`: also delete the worktree's branch (discarding a task).
    RemoveWorktree { ws: WsId, force: bool, delete_branch: bool },
    /// Make a pane its own workspace, homed at the pane's current directory. A workspace's
    /// only pane re-homes the workspace instead.
    PaneToWorkspace { term: TermId },
    /// Index into the configured workspace colour palette.
    SetWorkspaceColor { ws: WsId, color: u8 },
    ReloadConfig,
    /// `forget`: also discard the saved session, so the next start is a clean slate.
    KillServer { forget: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    List,
    /// Plain-text screen contents of a terminal (the visible screen).
    Read { term: TermId },
    /// Every worktree of the repository a workspace is in.
    Worktrees { ws: WsId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookStatus {
    Working,
    Blocked,
    Done,
    Idle,
    /// The agent session ended; the pane is a plain shell again.
    Gone,
    /// Leave the status as it is (the event only carries other news, e.g. a subagent).
    Same,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subagent {
    pub id: String,
    /// "Explore", "code-reviewer", …
    pub kind: String,
    pub start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome { version: u32 },
    State(Snapshot),
    /// Sent once per terminal on attach: the recent output ring, to rebuild scrollback.
    Replay { term: TermId, cols: u16, rows: u16, #[serde(with = "serde_bytes")] data: Vec<u8> },
    Output { term: TermId, #[serde(with = "serde_bytes")] data: Vec<u8> },
    /// An agent needs attention (blocked, or finished while unfocused).
    Attention { term: TermId, status: Status },
    Reply(Reply),
    Error(String),
    /// Informational message for the status bar.
    Notice(String),
    /// A program in a pane copied text (OSC 52): put it on the user's clipboard.
    Clipboard { term: TermId, text: String },
    /// An agent started in a new repo, which became a workspace (undoable).
    AutoWorkspace(String),
    Bye,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    Ok,
    List(Snapshot),
    Text(String),
    Worktrees(Vec<WorktreeEntry>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// Branch name, or a short commit for a detached HEAD.
    pub branch: String,
    /// The repository's main checkout.
    pub main: bool,
    pub repo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Status {
    #[default]
    None,
    Idle,
    Working,
    Blocked,
    /// Finished a turn and nobody has looked yet.
    Done,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::None => "",
            Status::Idle => "idle",
            Status::Working => "working",
            Status::Blocked => "blocked",
            Status::Done => "done",
        }
    }
    /// Sort order for "what needs me": blocked first, then done, working, idle.
    pub fn urgency(self) -> u8 {
        match self {
            Status::Blocked => 0,
            Status::Done => 1,
            Status::Working => 2,
            Status::Idle => 3,
            Status::None => 4,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_ws: Option<WsId>,
    pub terms: BTreeMap<TermId, TermInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: WsId,
    pub name: String,
    pub cwd: PathBuf,
    pub tabs: Vec<TabInfo>,
    pub active_tab: TabId,
    pub git: Option<GitInfo>,
    /// A worktree hydra created; offered for removal when closed.
    pub worktree: bool,
    /// Palette index for this workspace's colour (tint, borders, sidebar marker).
    pub color: u8,
    /// Created automatically and not looked at yet (shows a NEW chip).
    pub is_new: bool,
    /// The sidebar group it's in, if the user put it in one.
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitInfo {
    pub repo: String,
    pub branch: String,
    /// Changed + untracked files.
    pub dirty: u32,
    /// A linked worktree rather than the main checkout.
    pub linked: bool,
    /// The repository's main checkout; workspaces sharing it are one repo.
    pub root: PathBuf,
    /// Every worktree of the repository, open in hydra or not.
    pub worktrees: Vec<WorktreeEntry>,
    /// For a linked worktree: commits on its branch that the main checkout's branch lacks.
    pub ahead: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DevAction {
    Start,
    Stop,
    Restart,
}

/// A pane running a checkout's dev server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevInfo {
    pub dir: PathBuf,
    pub port: Option<u16>,
    /// Its output said it's up (the `ready` pattern), or there's no pattern.
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: TabId,
    pub name: String,
    pub layout: Node,
    pub focus: TermId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermInfo {
    pub id: TermId,
    pub cols: u16,
    pub rows: u16,
    /// Title set by the program (OSC 0/2), if any.
    pub title: String,
    /// Name of the most interesting process running in the pane (the leaf of the tree).
    pub process: String,
    /// Detected or hook-reported agent name.
    pub agent: Option<String>,
    pub status: Status,
    pub cwd: PathBuf,
    /// What the agent is on: the last prompt it was given (one line), if known.
    pub summary: String,
    /// What to call it: the name you gave it in the agent (/rename), else its first prompt.
    pub name: String,
    /// The model it's using, short ("opus 4.5"), if known.
    pub model: String,
    /// It's a checkout's dev server.
    pub dev: Option<DevInfo>,
    /// Memory used by what runs in it (bytes).
    #[serde(default)]
    pub mem: u64,
    /// The agent's last message, if hooks reported it.
    pub said: String,
    /// The git branch the pane is on, if it's in a repo.
    pub branch: Option<String>,
    /// The pane is in a linked worktree.
    pub linked: bool,
    /// Subagents running right now (their kinds).
    pub subagents: Vec<String>,
    /// The repository's main checkout, if the pane is in a repo.
    pub root: Option<PathBuf>,
    /// The checkout (main or a linked worktree) the pane is in.
    pub top: Option<PathBuf>,
    /// When its status last changed (unix seconds).
    pub since: u64,
    /// Put to sleep after sitting idle; focusing it (or typing) wakes it, resumed.
    pub asleep: bool,
    /// Windows' console layer asked for native key records (win32-input-mode), so keys
    /// plain VT can't express (Ctrl+Shift+letter, Ctrl+Enter) can be sent exactly.
    pub win32_input: bool,
}

impl TermInfo {
    /// What to call this pane: the agent's name, else the folder a shell is sitting in, else
    /// the running program's title or name (`vim`, `lazygit`, `node`).
    pub fn display_name(&self) -> String {
        if let Some(a) = &self.agent {
            return a.clone();
        }
        if self.is_shell() && !self.cwd.as_os_str().is_empty() {
            return match self.cwd.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                // A drive root like C:\ has no folder name; show it whole.
                None => self.cwd.display().to_string(),
            };
        }
        // Programs often title the window with their own exe path; that says nothing.
        let path_like = self.title.contains('\\') || self.title.contains('/') || self.title.ends_with(".exe");
        if !self.title.is_empty() && !path_like {
            return self.title.clone();
        }
        self.process.clone()
    }

    /// The pane is at a shell prompt (nothing else running in it).
    pub fn is_shell(&self) -> bool {
        matches!(
            self.process.to_ascii_lowercase().as_str(),
            "pwsh" | "powershell" | "cmd" | "bash" | "zsh" | "fish" | "nu" | "sh" | "dash" | "elvish" | "xonsh"
        )
    }
}

impl Snapshot {
    pub fn workspace(&self, id: WsId) -> Option<&WorkspaceInfo> {
        self.workspaces.iter().find(|w| w.id == id)
    }
    pub fn active(&self) -> Option<&WorkspaceInfo> {
        self.active_ws.and_then(|id| self.workspace(id))
    }
    /// (workspace, tab) containing a terminal.
    pub fn locate(&self, term: TermId) -> Option<(&WorkspaceInfo, &TabInfo)> {
        self.workspaces.iter().find_map(|w| {
            w.tabs.iter().find(|t| t.layout.contains(term)).map(|t| (w, t))
        })
    }
}

impl WorkspaceInfo {
    pub fn tab(&self) -> Option<&TabInfo> {
        self.tabs.iter().find(|t| t.id == self.active_tab).or(self.tabs.first())
    }
}

pub fn encode<T: Serialize>(msg: &T) -> anyhow::Result<bytes::Bytes> {
    Ok(rmp_serde::to_vec_named(msg)?.into())
}

pub fn decode<'a, T: Deserialize<'a>>(buf: &'a [u8]) -> anyhow::Result<T> {
    Ok(rmp_serde::from_slice(buf)?)
}
