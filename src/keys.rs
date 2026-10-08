//! Key specs ("ctrl+b", "alt+h", "%", "shift+tab"), bindable actions and the default keymap.

use anyhow::{Result, anyhow, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySpec {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeySpec {
    /// Normalise an incoming event so it compares equal to a parsed spec: for characters the
    /// shift state is already in the character itself ('N', '%'), so SHIFT is dropped.
    pub fn from_event(ev: &KeyEvent) -> Self {
        let mut mods = ev.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let mut code = ev.code;
        if let KeyCode::Char(c) = code {
            mods.remove(KeyModifiers::SHIFT);
            if mods.contains(KeyModifiers::CONTROL) && c.is_ascii_uppercase() {
                code = KeyCode::Char(c.to_ascii_lowercase());
            }
            // Ctrl+Space, Ctrl+@ and Ctrl+2 are the same byte (NUL) on the wire; when it
            // arrives that way (ConPTY, SSH, nested sessions) Windows reports '@' or '2'.
            if mods.contains(KeyModifiers::CONTROL) && matches!(c, '@' | '2' | '\0') {
                code = KeyCode::Char(' ');
            }
        }
        if code == KeyCode::BackTab {
            mods.remove(KeyModifiers::SHIFT);
        }
        KeySpec { code, mods }
    }
}

impl KeySpec {
    /// The config-file spelling ("ctrl+b", "alt+space", "%").
    pub fn to_config(self) -> String {
        let mut s = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            s.push_str("ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            s.push_str("alt+");
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            s.push_str("shift+");
        }
        match self.code {
            KeyCode::Char(' ') => s.push_str("space"),
            KeyCode::Char(c) => s.push(c),
            KeyCode::F(n) => s.push_str(&format!("f{n}")),
            KeyCode::Enter => s.push_str("enter"),
            KeyCode::Tab => s.push_str("tab"),
            KeyCode::BackTab => s.push_str("shift+tab"),
            KeyCode::Backspace => s.push_str("backspace"),
            KeyCode::Left => s.push_str("left"),
            KeyCode::Right => s.push_str("right"),
            KeyCode::Up => s.push_str("up"),
            KeyCode::Down => s.push_str("down"),
            KeyCode::Home => s.push_str("home"),
            KeyCode::End => s.push_str("end"),
            KeyCode::PageUp => s.push_str("pageup"),
            KeyCode::PageDown => s.push_str("pagedown"),
            KeyCode::Insert => s.push_str("insert"),
            KeyCode::Delete => s.push_str("delete"),
            _ => s.push_str("ctrl+space"),
        }
        s
    }
}

impl FromStr for KeySpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        // A lone "+" or a trailing "+" (e.g. "ctrl++") is the plus key itself.
        let (mod_part, key) = match s.strip_suffix("++") {
            Some(m) => (m, "+"),
            None if s == "+" => ("", "+"),
            None => match s.rsplit_once('+') {
                Some((m, k)) => (m, k),
                None => ("", s),
            },
        };
        let mut mods = KeyModifiers::NONE;
        for m in mod_part.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "c" | "control" => mods |= KeyModifiers::CONTROL,
                "alt" | "a" | "meta" | "m" | "opt" | "option" => mods |= KeyModifiers::ALT,
                "shift" | "s" => mods |= KeyModifiers::SHIFT,
                other => bail!("unknown modifier `{other}` in `{s}`"),
            }
        }
        let lower = key.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" | "return" | "cr" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "esc" | "escape" => KeyCode::Esc,
            "space" => KeyCode::Char(' '),
            "backspace" | "bs" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
                KeyCode::F(f[1..].parse()?)
            }
            _ => {
                let mut chars = key.chars();
                let c = chars.next().ok_or_else(|| anyhow!("empty key in `{s}`"))?;
                if chars.next().is_some() {
                    bail!("unknown key `{key}` in `{s}`");
                }
                KeyCode::Char(c)
            }
        };
        let mut spec = KeySpec { code, mods };
        // Express shift on characters through the character, matching `from_event`.
        if let KeyCode::Char(c) = spec.code {
            if spec.mods.contains(KeyModifiers::SHIFT) {
                spec.mods.remove(KeyModifiers::SHIFT);
                spec.code = KeyCode::Char(c.to_ascii_uppercase());
            }
            if spec.mods.contains(KeyModifiers::CONTROL) {
                spec.code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        if spec.code == KeyCode::Tab && spec.mods.contains(KeyModifiers::SHIFT) {
            spec = KeySpec { code: KeyCode::BackTab, mods: spec.mods - KeyModifiers::SHIFT };
        }
        Ok(spec)
    }
}

impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            write!(f, "C-")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            write!(f, "M-")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            write!(f, "S-")?;
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "Space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::BackTab => write!(f, "S-Tab"),
            other => write!(f, "{other:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SplitRight,
    SplitDown,
    SplitLeft,
    SplitUp,
    /// Split and run a command in the new pane: `spawn-right:claude`.
    Spawn(crate::layout::Dir, String),
    /// New tab running a command: `spawn-tab:lazygit`.
    SpawnTab(String),
    ClosePane,
    Focus(crate::layout::Dir),
    FocusNext,
    FocusPrev,
    Resize(crate::layout::Dir),
    Zoom,
    NewTab,
    NextTab,
    PrevTab,
    CloseTab,
    RenameTab,
    SelectTab(usize),
    NewWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    ToggleSidebar,
    Picker,
    NextAttention,
    ScrollUp,
    ScrollDown,
    CopyMode,
    Search,
    /// Prompt for a branch, create a worktree, open it; optionally run a command there.
    NewWorktree(Option<String>),
    RemoveWorktree,
    /// Search every command, workspace, agent and pane.
    Palette,
    /// Type a task; an agent starts on it.
    QuickPrompt,
    Settings,
    /// Move through the sidebar tree with the arrow keys.
    BrowseTree,
    /// Recent and project files; Enter puts the path in the prompt.
    Files,
    /// GitHub pull requests and issues, Linear tickets.
    Inbox,
    /// Hydra's queue of work (the Tickets view's Queue tab).
    Queue,
    /// The focused session's checkpoints (roll its folder back).
    Checkpoints,
    /// Search past agent chats and pick one up again.
    Chats,
    /// Each AI tool's MCP servers, skills, plugins, hooks.
    Toolbox,
    /// The + Pane menu: what to run, and in which folder.
    NewPane,
    /// Talk to the agent of the selected worktree in a modal.
    Talk,
    /// The selected worktree's changes (review and ship).
    Changes,
    /// Answer the focused agent's numbered prompt.
    Answer(char),
    /// Reply to the focused agent in words.
    Reply,
    UndoAutoWorkspace,
    /// Show the selected worktree's folder in the file manager.
    OpenFolder,
    /// Interrupt the selected worktree's agent (Ctrl+C).
    StopAgent,
    /// Sessions that need you, then finished ones, across every project.
    Jump,
    /// The folder finder: open a repo (or any folder) as a project.
    OpenProject,
    /// A new agent session in the sidebar cursor's worktree.
    NewSession,
    /// Close the split (both sessions keep running).
    CloseSplit,
    /// The pull request of this branch: checks, reviews, comments, diff.
    PullRequest,
    /// Commit, push and open (or update) the pull request, after one confirm.
    Ship,
    /// Your ideas: jot one down, or start an agent on one.
    Ideas,
    /// Give one task to several agents, each in its own worktree, and compare.
    Race,
    /// The project as a map: its folders and worktrees as boxes, coloured by status.
    Map,
    /// Save the clipboard's image to a file and paste its path (agents attach it).
    PasteImage,
    /// Find a file (0) or search the code (1).
    Find(u8),
    /// Pick a saved launch (presets).
    Presets,
    /// Switch the branch of the checkout you're in.
    Branches,
    /// Memory used by each session.
    Memory,
    /// A shell in the folder of the session you're on, straight away.
    ShellHere,
    /// The next way of laying out this tab's panes (split, grid, main and stack, columns).
    Arrange,
    /// Scroll to the command before (or after) the one at the top of the view.
    PrevPrompt,
    NextPrompt,
    /// The switcher: every project and session, type to find one.
    GoTo,
    /// What happened lately: who finished, who asked, bells, messages.
    History,
    /// Move the sidebar cursor (-1 up, 1 down); bare keys then work on the sidebar.
    SideMove(i8),
    Detach,
    Help,
    ReloadConfig,
    /// Say how the pane you're on takes the mouse and how much history it has.
    PaneInfo,
    SendPrefix,
    KillServer,
    /// Install the newest hydra and restart this window into it.
    Update,
    /// Explicitly unbind a default.
    None,
}

impl Action {
    /// Actions that keep prefix mode armed so they can be repeated (like tmux `-r`).
    pub fn repeats(&self) -> bool {
        matches!(self, Action::Resize(_) | Action::NextTab | Action::PrevTab | Action::ScrollUp | Action::ScrollDown)
    }

    pub fn describe(&self) -> String {
        use crate::layout::Dir::*;
        let dir = |d: &crate::layout::Dir| match d {
            Left => "left",
            Right => "right",
            Up => "up",
            Down => "down",
        };
        match self {
            Action::SplitRight => "Split right (a shell beside)".into(),
            Action::SplitDown => "Split down (a shell below)".into(),
            Action::SplitLeft => "split left".into(),
            Action::SplitUp => "split up".into(),
            Action::Spawn(d, c) => format!("run `{c}` {}", dir(d)),
            Action::SpawnTab(c) => format!("run `{c}` in new tab"),
            Action::ClosePane => "Close pane".into(),
            Action::Focus(d) => format!("Focus the pane {}", dir(d)),
            Action::FocusNext => "next split".into(),
            Action::FocusPrev => "previous pane".into(),
            Action::Resize(d) => format!("Resize the pane {}", dir(d)),
            Action::Zoom => "Zoom pane (just this one)".into(),
            Action::NewTab => "New tab".into(),
            Action::NextTab => "Next tab".into(),
            Action::PrevTab => "Previous tab".into(),
            Action::CloseTab => "Close tab".into(),
            Action::RenameTab => "rename tab".into(),
            Action::SelectTab(n) => format!("Go to tab {n}"),
            Action::NewWorkspace => "new group".into(),
            Action::CloseWorkspace => "close this pane (all its tabs)".into(),
            Action::RenameWorkspace => "Rename session".into(),
            Action::ToggleSidebar => "Show / hide the sidebar".into(),
            Action::Picker => "jump to…".into(),
            Action::NextAttention => "Next agent that needs you".into(),
            Action::ScrollUp => "scroll up".into(),
            Action::ScrollDown => "scroll down".into(),
            Action::CopyMode => "Select text with the keyboard".into(),
            Action::Search => "search history".into(),
            Action::NewWorktree(None) => "Worktrees…".into(),
            Action::NewWorktree(Some(c)) => format!("worktrees… running `{c}`"),
            Action::RemoveWorktree => "remove worktree".into(),
            Action::Palette => "Command palette".into(),
            Action::QuickPrompt => "quick prompt (start an agent on a task)".into(),
            Action::Settings => "Settings".into(),
            Action::BrowseTree => "Focus the sidebar".into(),
            Action::Files => "Files".into(),
            Action::Inbox => "Tickets (GitHub, Linear, Plane)".into(),
            Action::Queue => "Queue (work waiting for an agent)".into(),
            Action::Checkpoints => "Checkpoints (undo an agent's changes)".into(),
            Action::Chats => "Past chats (search, pick one up again)".into(),
            Action::Toolbox => "Agent tools (MCP, skills, plugins)".into(),
            Action::NewPane => "New agent (task, agent, model, worktree)".into(),
            Action::Talk => "Message an agent".into(),
            Action::Changes => "Changes (diff, review, commit)".into(),
            Action::Answer(c) => format!("answer {c}"),
            Action::Reply => "Reply to the focused agent".into(),
            Action::UndoAutoWorkspace => "undo automatic workspace".into(),
            Action::OpenFolder => "open folder".into(),
            Action::StopAgent => "stop agent (Ctrl+C)".into(),
            Action::Jump => "Inbox: what needs you, answer from there".into(),
            Action::OpenProject => "Open a folder (start a session there)".into(),
            Action::NewSession => "+ new, beside this one".into(),
            Action::CloseSplit => "close the split".into(),
            Action::PullRequest => "Pull request (checks, reviews, diff)".into(),
            Action::Ship => "Ship: commit, push, open the PR".into(),
            Action::Ideas => "Ideas: jot one down, start an agent on one".into(),
            Action::Race => "Race agents on one task".into(),
            Action::Map => "Map of the project".into(),
            Action::PasteImage => "Paste the clipboard image".into(),
            Action::Presets => "Run a preset".into(),
            Action::Branches => "Switch branch".into(),
            Action::Memory => "Memory used by each session".into(),
            Action::ShellHere => "New session (a shell here)".into(),
            Action::Arrange => "Arrange panes: split, grid, main and stack, columns".into(),
            Action::PrevPrompt => "Jump to the previous command in history".into(),
            Action::NextPrompt => "Jump to the next command in history".into(),
            Action::GoTo => "Go to a project or session".into(),
            Action::History => "What happened (notifications)".into(),
            Action::Find(0) => "Find a file".into(),
            Action::Find(_) => "Search the code".into(),
            Action::SideMove(d) if *d < 0 => "sidebar up".into(),
            Action::SideMove(_) => "sidebar down".into(),
            Action::Detach => "Detach (agents keep running)".into(),
            Action::Help => "Keys".into(),
            Action::ReloadConfig => "Reload the config file".into(),
            Action::PaneInfo => "Pane info (scrolling, mouse)".into(),
            Action::SendPrefix => "send prefix key".into(),
            Action::KillServer => "kill server".into(),
            Action::Update => "Update hydra".into(),
            Action::None => "unbound".into(),
        }
    }
}

impl Action {
    /// The config-file spelling of an action (`"split-right"`), the inverse of `parse`.
    pub fn to_config(&self) -> Option<String> {
        use crate::layout::Dir::*;
        let d = |d: &crate::layout::Dir| match d {
            Left => "left",
            Right => "right",
            Up => "up",
            Down => "down",
        };
        Some(match self {
            Action::SplitRight => "split-right".into(),
            Action::SplitDown => "split-down".into(),
            Action::SplitLeft => "split-left".into(),
            Action::SplitUp => "split-up".into(),
            Action::Spawn(dir, c) => format!("spawn-{}:{c}", d(dir)),
            Action::SpawnTab(c) => format!("spawn-tab:{c}"),
            Action::ClosePane => "close-pane".into(),
            Action::Focus(dir) => format!("focus-{}", d(dir)),
            Action::FocusNext => "focus-next".into(),
            Action::FocusPrev => "focus-prev".into(),
            Action::Resize(dir) => format!("resize-{}", d(dir)),
            Action::Zoom => "zoom".into(),
            Action::NewTab => "new-tab".into(),
            Action::NextTab => "next-tab".into(),
            Action::PrevTab => "prev-tab".into(),
            Action::CloseTab => "close-tab".into(),
            Action::RenameTab => "rename-tab".into(),
            Action::SelectTab(n) => format!("select-tab-{n}"),
            Action::NewWorkspace => "new-workspace".into(),
            Action::CloseWorkspace => "close-workspace".into(),
            Action::RenameWorkspace => "rename-workspace".into(),
            Action::ToggleSidebar => "toggle-sidebar".into(),
            Action::Picker => "picker".into(),
            Action::NextAttention => "next-attention".into(),
            Action::ScrollUp => "scroll-up".into(),
            Action::ScrollDown => "scroll-down".into(),
            Action::CopyMode => "copy-mode".into(),
            Action::Search => "search".into(),
            Action::NewWorktree(None) => "new-worktree".into(),
            Action::NewWorktree(Some(c)) => format!("new-worktree:{c}"),
            Action::RemoveWorktree => "remove-worktree".into(),
            Action::Palette => "palette".into(),
            Action::QuickPrompt => "quick-prompt".into(),
            Action::Settings => "settings".into(),
            Action::BrowseTree => "browse-tree".into(),
            Action::Files => "files".into(),
            Action::Inbox => "inbox".into(),
            Action::Queue => "queue".into(),
            Action::Checkpoints => "checkpoints".into(),
            Action::Chats => "chats".into(),
            Action::Toolbox => "toolbox".into(),
            Action::NewPane => "new".into(),
            Action::Jump => "jump".into(),
            Action::OpenProject => "open-project".into(),
            Action::NewSession => "new-beside".into(),
            Action::CloseSplit => "close-split".into(),
            Action::PullRequest => "pr".into(),
            Action::Ship => "ship".into(),
            Action::Ideas => "ideas".into(),
            Action::Race => "race".into(),
            Action::Map => "map".into(),
            Action::PasteImage => "paste-image".into(),
            Action::Presets => "presets".into(),
            Action::Branches => "switch-branch".into(),
            Action::Memory => "memory".into(),
            Action::ShellHere => "shell-here".into(),
            Action::Arrange => "arrange".into(),
            Action::PrevPrompt => "prev-prompt".into(),
            Action::NextPrompt => "next-prompt".into(),
            Action::GoTo => "go-to".into(),
            Action::History => "history".into(),
            Action::Find(0) => "find-file".into(),
            Action::Find(_) => "search-code".into(),
            Action::SideMove(d) if *d < 0 => "side-up".into(),
            Action::SideMove(_) => "side-down".into(),
            Action::Talk => "talk".into(),
            Action::Changes => "changes".into(),
            Action::Answer(c) => format!("answer:{c}"),
            Action::Reply => "reply".into(),
            Action::UndoAutoWorkspace => "undo".into(),
            Action::OpenFolder => "open-folder".into(),
            Action::StopAgent => "stop-agent".into(),
            Action::Detach => "detach".into(),
            Action::Help => "help".into(),
            Action::ReloadConfig => "reload-config".into(),
            Action::PaneInfo => "pane-info".into(),
            Action::SendPrefix => "send-prefix".into(),
            Action::KillServer => "kill-server".into(),
            Action::Update => "update".into(),
            Action::None => return None,
        })
    }
}

impl FromStr for Action {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::layout::Dir::*;
        if let Some((verb, arg)) = s.split_once(':') {
            let arg = arg.trim().to_string();
            return Ok(match verb {
                "spawn-right" => Action::Spawn(Right, arg),
                "spawn-down" => Action::Spawn(Down, arg),
                "spawn-left" => Action::Spawn(Left, arg),
                "spawn-up" => Action::Spawn(Up, arg),
                "spawn-tab" => Action::SpawnTab(arg),
                "new-worktree" => Action::NewWorktree(Some(arg).filter(|a| !a.is_empty())),
                "select-tab" => Action::SelectTab(arg.parse()?),
                "answer" => Action::Answer(arg.chars().next().unwrap_or('1')),
                _ => bail!("unknown action `{s}`"),
            });
        }
        if let Some(n) = s.strip_prefix("select-tab-") {
            return Ok(Action::SelectTab(n.parse()?));
        }
        Ok(match s {
            "split-right" => Action::SplitRight,
            "split-down" => Action::SplitDown,
            "split-left" => Action::SplitLeft,
            "split-up" => Action::SplitUp,
            "close-pane" => Action::ClosePane,
            "focus-left" => Action::Focus(Left),
            "focus-right" => Action::Focus(Right),
            "focus-up" => Action::Focus(Up),
            "focus-down" => Action::Focus(Down),
            "focus-next" => Action::FocusNext,
            "focus-prev" => Action::FocusPrev,
            "resize-left" => Action::Resize(Left),
            "resize-right" => Action::Resize(Right),
            "resize-up" => Action::Resize(Up),
            "resize-down" => Action::Resize(Down),
            "zoom" => Action::Zoom,
            "new-tab" => Action::NewTab,
            "next-tab" => Action::NextTab,
            "prev-tab" => Action::PrevTab,
            "close-tab" => Action::CloseTab,
            "rename-tab" => Action::RenameTab,
            "new-workspace" => Action::NewWorkspace,
            "close-workspace" => Action::CloseWorkspace,
            "rename-workspace" => Action::RenameWorkspace,
            "toggle-sidebar" => Action::ToggleSidebar,
            "picker" => Action::Picker,
            "next-attention" => Action::NextAttention,
            "scroll-up" => Action::ScrollUp,
            "scroll-down" => Action::ScrollDown,
            "copy-mode" => Action::CopyMode,
            "search" => Action::Search,
            "new-worktree" => Action::NewWorktree(None),
            "remove-worktree" => Action::RemoveWorktree,
            "palette" | "command-palette" => Action::Palette,
            "quick-prompt" => Action::QuickPrompt,
            "settings" => Action::Settings,
            "browse-tree" | "browse" => Action::BrowseTree,
            "files" => Action::Files,
            "inbox" => Action::Inbox,
            "queue" => Action::Queue,
            "checkpoints" => Action::Checkpoints,
            "chats" => Action::Chats,
            "toolbox" => Action::Toolbox,
            "new-pane" | "new" => Action::NewPane,
            "jump" => Action::Jump,
            "open-project" => Action::OpenProject,
            "new-session" | "new-beside" => Action::NewSession,
            "close-split" => Action::CloseSplit,
            "pr" | "pull-request" => Action::PullRequest,
            "ship" => Action::Ship,
            "ideas" => Action::Ideas,
            "race" => Action::Race,
            "map" => Action::Map,
            "paste-image" => Action::PasteImage,
            "presets" => Action::Presets,
            "switch-branch" => Action::Branches,
            "memory" => Action::Memory,
            "shell-here" => Action::ShellHere,
            "arrange" | "next-layout" => Action::Arrange,
            "prev-prompt" => Action::PrevPrompt,
            "next-prompt" => Action::NextPrompt,
            "go-to" => Action::GoTo,
            "history" => Action::History,
            "find-file" => Action::Find(0),
            "search-code" => Action::Find(1),
            "side-up" => Action::SideMove(-1),
            "side-down" => Action::SideMove(1),
            "talk" => Action::Talk,
            "changes" => Action::Changes,
            "reply" => Action::Reply,
            "undo" | "undo-auto-workspace" => Action::UndoAutoWorkspace,
            "open-folder" => Action::OpenFolder,
            "stop-agent" => Action::StopAgent,
            "detach" => Action::Detach,
            "help" => Action::Help,
            "reload-config" => Action::ReloadConfig,
            "pane-info" => Action::PaneInfo,
            "send-prefix" => Action::SendPrefix,
            "kill-server" => Action::KillServer,
            "update" => Action::Update,
            "none" | "unbind" => Action::None,
            _ => bail!("unknown action `{s}`"),
        })
    }
}

/// Bindings active after the prefix key.
pub const DEFAULT_PREFIX_KEYS: &[(&str, &str)] = &[
    // Panes (as herdr)
    ("v", "split-right"),
    ("|", "split-right"),
    ("-", "split-down"),
    ("h", "focus-left"),
    ("j", "focus-down"),
    ("k", "focus-up"),
    ("l", "focus-right"),
    ("left", "focus-left"),
    ("down", "focus-down"),
    ("up", "focus-up"),
    ("right", "focus-right"),
    ("x", "close-pane"),
    ("z", "zoom"),
    ("b", "toggle-sidebar"),
    ("H", "resize-left"),
    ("J", "resize-down"),
    ("K", "resize-up"),
    ("L", "resize-right"),
    ("y", "copy-mode"),
    ("pageup", "scroll-up"),
    ("=", "arrange"),
    ("{", "prev-prompt"),
    ("}", "next-prompt"),
    ("pagedown", "scroll-down"),
    // Tabs (as herdr)
    ("c", "new-tab"),
    ("]", "next-tab"),
    ("[", "prev-tab"),
    ("X", "close-tab"),
    ("1", "select-tab-1"),
    ("2", "select-tab-2"),
    ("3", "select-tab-3"),
    ("4", "select-tab-4"),
    ("5", "select-tab-5"),
    ("6", "select-tab-6"),
    ("7", "select-tab-7"),
    ("8", "select-tab-8"),
    ("9", "select-tab-9"),
    // Getting around
    ("g", "go-to"),
    ("e", "browse-tree"),
    ("a", "jump"),
    ("p", "palette"),
    ("space", "palette"),
    (":", "palette"),
    // Starting things
    ("n", "shell-here"),
    (".", "presets"),
    // Agents
    ("m", "talk"),
    ("T", "talk"),
    ("r", "reply"),
    ("R", "rename-workspace"),
    // Code
    ("f", "files"),
    ("F", "find-file"),
    ("/", "search-code"),
    ("d", "changes"),
    ("B", "switch-branch"),
    ("P", "pr"),
    ("S", "ship"),
    ("i", "inbox"),
    ("I", "ideas"),
    ("A", "toolbox"),
    ("C", "race"),
    ("M", "map"),
    ("W", "new-worktree"),
    ("V", "paste-image"),
    ("ctrl+v", "paste-image"),
    // App
    (",", "settings"),
    ("?", "help"),
    ("U", "memory"),
    ("N", "history"),
    ("q", "detach"),
    ("ctrl+r", "reload-config"),
    ("ctrl+space", "send-prefix"),
];

/// Bindings active without the prefix. Empty by default so nothing is stolen from programs.
pub const DEFAULT_GLOBAL_KEYS: &[(&str, &str)] = &[];

/// A mouse event as the bytes a program that turned on mouse reporting expects, at (col,
/// row) inside its screen (0-based). None: the program didn't ask for this kind of event;
/// empty: it's the program's, but there's nothing to send.
pub fn mouse_bytes(screen: &vt100::Screen, m: &crossterm::event::MouseEvent, col: u16, row: u16) -> Option<Vec<u8>> {
    use crossterm::event::{MouseButton as B, MouseEventKind as K};
    use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as M};
    let mode = screen.mouse_protocol_mode();
    if mode == M::None {
        return None;
    }
    let mut mods = 0u32;
    if m.modifiers.contains(KeyModifiers::SHIFT) {
        mods |= 4;
    }
    if m.modifiers.contains(KeyModifiers::ALT) {
        mods |= 8;
    }
    if m.modifiers.contains(KeyModifiers::CONTROL) {
        mods |= 16;
    }
    let btn = |b: B| match b {
        B::Left => 0,
        B::Middle => 1,
        B::Right => 2,
    };
    let (code, release) = match m.kind {
        K::Down(b) => (btn(b), false),
        K::Up(_) if mode == M::Press => return Some(Vec::new()),
        K::Up(b) => (btn(b), true),
        K::Drag(b) if matches!(mode, M::ButtonMotion | M::AnyMotion) => (32 + btn(b), false),
        K::Drag(_) => return Some(Vec::new()),
        K::Moved if mode == M::AnyMotion => (35, false),
        K::Moved => return None,
        K::ScrollUp => (64, false),
        K::ScrollDown => (65, false),
        K::ScrollLeft => (66, false),
        K::ScrollRight => (67, false),
    };
    let (x, y) = (col as u32 + 1, row as u32 + 1);
    Some(match screen.mouse_protocol_encoding() {
        E::Sgr => format!("\x1b[<{};{x};{y}{}", code + mods, if release { 'm' } else { 'M' }).into_bytes(),
        enc => {
            // X10 style: release is button 3; values are offset by 32.
            let c = if release { 3 + mods } else { code + mods };
            let mut out = b"\x1b[M".to_vec();
            for v in [c, x, y] {
                let v = v + 32;
                if enc == E::Utf8 {
                    let mut b = [0; 4];
                    out.extend_from_slice(char::from_u32(v).unwrap_or(' ').encode_utf8(&mut b).as_bytes());
                } else {
                    out.push(v.min(255) as u8);
                }
            }
            out
        }
    })
}

/// Keys plain VT can't express exactly (Ctrl+Shift+letter, Ctrl+Enter, Ctrl+Tab, Ctrl with
/// symbols) — sent as Windows key records when ConPTY asked for them. Everything else keeps
/// its usual encoding (so Claude's Shift+Enter, Ctrl+C and friends behave as before).
pub fn wants_win32(ev: &KeyEvent) -> bool {
    let m = ev.modifiers;
    let (ctrl, alt, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
    match ev.code {
        // Ctrl+Alt+symbol is AltGr typing a character: leave it.
        KeyCode::Char(c) if ctrl && alt && !c.is_ascii_alphabetic() => false,
        KeyCode::Char(' ') => false,
        KeyCode::Char(c) => ctrl && (shift || !c.is_ascii_alphabetic()),
        KeyCode::Enter | KeyCode::Tab | KeyCode::Esc => ctrl,
        KeyCode::Backspace => ctrl && shift,
        _ => false,
    }
}

/// A key as Windows' win32-input-mode records (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`), down then up.
pub fn encode_win32(ev: &KeyEvent) -> Option<Vec<u8>> {
    let m = ev.modifiers;
    let (ctrl, alt, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
    let (vk, uc, enhanced): (u16, u32, bool) = match ev.code {
        KeyCode::Char(c) => {
            let up = c.to_ascii_uppercase();
            let vk = if up.is_ascii_alphanumeric() {
                up as u16
            } else {
                match c {
                    ' ' => 0x20,
                    '-' | '_' => 0xBD,
                    '=' | '+' => 0xBB,
                    '[' | '{' => 0xDB,
                    ']' | '}' => 0xDD,
                    '\\' | '|' => 0xDC,
                    ';' | ':' => 0xBA,
                    '\'' | '"' => 0xDE,
                    ',' | '<' => 0xBC,
                    '.' | '>' => 0xBE,
                    '/' | '?' => 0xBF,
                    '`' | '~' => 0xC0,
                    '!' => 0x31,
                    '@' => 0x32,
                    '#' => 0x33,
                    '$' => 0x34,
                    '%' => 0x35,
                    '^' => 0x36,
                    '&' => 0x37,
                    '*' => 0x38,
                    '(' => 0x39,
                    ')' => 0x30,
                    _ => return None,
                }
            };
            let uc = if ctrl && up.is_ascii_alphabetic() { up as u32 - 0x40 } else { c as u32 };
            (vk, uc, false)
        }
        KeyCode::Enter => (0x0D, 13, false),
        KeyCode::Tab | KeyCode::BackTab => (0x09, 9, false),
        KeyCode::Backspace => (0x08, if ctrl { 0x7f } else { 8 }, false),
        KeyCode::Esc => (0x1B, 27, false),
        KeyCode::Up => (0x26, 0, true),
        KeyCode::Down => (0x28, 0, true),
        KeyCode::Left => (0x25, 0, true),
        KeyCode::Right => (0x27, 0, true),
        KeyCode::Home => (0x24, 0, true),
        KeyCode::End => (0x23, 0, true),
        KeyCode::PageUp => (0x21, 0, true),
        KeyCode::PageDown => (0x22, 0, true),
        KeyCode::Insert => (0x2D, 0, true),
        KeyCode::Delete => (0x2E, 0, true),
        KeyCode::F(n @ 1..=12) => (0x6F + n as u16, 0, false),
        _ => return None,
    };
    let mut cs = 0u32;
    if shift || ev.code == KeyCode::BackTab {
        cs |= 0x10; // SHIFT_PRESSED
    }
    if ctrl {
        cs |= 0x08; // LEFT_CTRL_PRESSED
    }
    if alt {
        cs |= 0x02; // LEFT_ALT_PRESSED
    }
    if enhanced {
        cs |= 0x100; // ENHANCED_KEY
    }
    let rec = |down: u8| format!("\x1b[{vk};0;{uc};{down};{cs};1_");
    Some(format!("{}{}", rec(1), rec(0)).into_bytes())
}

/// The last cursor style a program asked for (DECSCUSR `CSI n SP q`, 0-6) in `data`.
pub fn last_cursor_style(data: &[u8]) -> Option<u8> {
    let mut found = None;
    let mut i = 0;
    while i + 3 < data.len() + 1 {
        if data[i] == 0x1b && data.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            let mut n: Option<u8> = None;
            if let Some(d) = data.get(j).filter(|d| d.is_ascii_digit()) {
                n = Some(d - b'0');
                j += 1;
            }
            if data.get(j) == Some(&b' ') && data.get(j + 1) == Some(&b'q') {
                found = Some(n.unwrap_or(0).min(6));
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Encode a key press as the bytes a terminal program expects.
pub fn encode(ev: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    let m = ev.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);
    // xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl
    let param = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let csi_mod = |base: &str, fin: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{fin}").into_bytes()
        } else {
            base.as_bytes().to_vec()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 { format!("\x1b[{n};{param}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let mut out = match ev.code {
        KeyCode::Char(c) => {
            // AltGr arrives as Ctrl+Alt on Windows; treat non-letters as plain text.
            if ctrl && alt && !c.is_ascii_alphabetic() {
                let mut b = [0; 4];
                return c.encode_utf8(&mut b).as_bytes().to_vec();
            }
            if ctrl {
                let b = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    ' ' | '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '?' | '8' => 0x7f,
                    other => {
                        let mut b = [0; 4];
                        return other.encode_utf8(&mut b).as_bytes().to_vec();
                    }
                };
                vec![b]
            } else {
                let mut b = [0; 4];
                c.encode_utf8(&mut b).as_bytes().to_vec()
            }
        }
        // Shift+Enter as Meta+Enter: what agent CLIs (Claude Code, Codex) read as "newline".
        KeyCode::Enter if shift => return b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return if app_cursor && param == 1 { b"\x1bOA".to_vec() } else { csi_mod("\x1b[A", 'A') },
        KeyCode::Down => return if app_cursor && param == 1 { b"\x1bOB".to_vec() } else { csi_mod("\x1b[B", 'B') },
        KeyCode::Right => return if app_cursor && param == 1 { b"\x1bOC".to_vec() } else { csi_mod("\x1b[C", 'C') },
        KeyCode::Left => return if app_cursor && param == 1 { b"\x1bOD".to_vec() } else { csi_mod("\x1b[D", 'D') },
        KeyCode::Home => return if app_cursor && param == 1 { b"\x1bOH".to_vec() } else { csi_mod("\x1b[H", 'H') },
        KeyCode::End => return if app_cursor && param == 1 { b"\x1bOF".to_vec() } else { csi_mod("\x1b[F", 'F') },
        KeyCode::Insert => return tilde(2),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let fin = (b'P' + n - 1) as char;
            return if param > 1 { format!("\x1b[1;{param}{fin}").into_bytes() } else { format!("\x1bO{fin}").into_bytes() };
        }
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            return tilde(code);
        }
        _ => return Vec::new(),
    };
    if alt {
        out.insert(0, 0x1b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn parse_and_match() {
        let spec: KeySpec = "ctrl+b".parse().unwrap();
        assert_eq!(spec, KeySpec::from_event(&ev(KeyCode::Char('b'), KeyModifiers::CONTROL)));
        let spec: KeySpec = "%".parse().unwrap();
        assert_eq!(spec, KeySpec::from_event(&ev(KeyCode::Char('%'), KeyModifiers::SHIFT)));
        let spec: KeySpec = "shift+n".parse().unwrap();
        assert_eq!(spec, KeySpec::from_event(&ev(KeyCode::Char('N'), KeyModifiers::SHIFT)));
        let spec: KeySpec = "N".parse().unwrap();
        assert_eq!(spec, KeySpec::from_event(&ev(KeyCode::Char('N'), KeyModifiers::SHIFT)));
        let space: KeySpec = "ctrl+space".parse().unwrap();
        assert_eq!(space, KeySpec::from_event(&ev(KeyCode::Char(' '), KeyModifiers::CONTROL)));
        assert_eq!(space, KeySpec::from_event(&ev(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
        assert_eq!(space, KeySpec::from_event(&ev(KeyCode::Char('2'), KeyModifiers::CONTROL)));
        let spec: KeySpec = "alt+left".parse().unwrap();
        assert_eq!(spec, KeySpec::from_event(&ev(KeyCode::Left, KeyModifiers::ALT)));
        assert_eq!("ctrl++".parse::<KeySpec>().unwrap().code, KeyCode::Char('+'));
        // Windows key records for what VT can't say; the rest unchanged.
        let k = |code, m| KeyEvent::new(code, m);
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert!(wants_win32(&k(KeyCode::Char('A'), cs)));
        assert!(wants_win32(&k(KeyCode::Enter, KeyModifiers::CONTROL)));
        assert!(!wants_win32(&k(KeyCode::Char('c'), KeyModifiers::CONTROL)), "Ctrl+C stays ^C");
        assert!(!wants_win32(&k(KeyCode::Enter, KeyModifiers::SHIFT)), "Shift+Enter stays the agents' newline");
        assert!(!wants_win32(&k(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT)), "AltGr typing");
        assert_eq!(encode_win32(&k(KeyCode::Char('A'), cs)).unwrap(), b"\x1b[65;0;1;1;24;1_\x1b[65;0;1;0;24;1_".to_vec());
        assert_eq!(encode_win32(&k(KeyCode::Up, KeyModifiers::NONE)).unwrap(), b"\x1b[38;0;0;1;256;1_\x1b[38;0;0;0;256;1_".to_vec());
        // Cursor styles.
        assert_eq!(last_cursor_style(b"x\x1b[2 qy\x1b[6 q"), Some(6));
        assert_eq!(last_cursor_style(b"\x1b[ q"), Some(0));
        assert_eq!(last_cursor_style(b"\x1b[2J"), None);
        // Mouse: nothing until the program asks; then SGR or X10 like any terminal.
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let ev = |kind| MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE };
        let mut p = vt100::Parser::new(10, 40, 0);
        assert_eq!(mouse_bytes(p.screen(), &ev(MouseEventKind::ScrollUp), 4, 2), None);
        p.process(b"[?1002h[?1006h");
        assert_eq!(mouse_bytes(p.screen(), &ev(MouseEventKind::ScrollUp), 4, 2), Some(b"[<64;5;3M".to_vec()));
        assert_eq!(mouse_bytes(p.screen(), &ev(MouseEventKind::Down(MouseButton::Left)), 0, 0), Some(b"[<0;1;1M".to_vec()));
        assert_eq!(mouse_bytes(p.screen(), &ev(MouseEventKind::Up(MouseButton::Left)), 0, 0), Some(b"[<0;1;1m".to_vec()));
        assert_eq!(mouse_bytes(p.screen(), &ev(MouseEventKind::Moved), 0, 0), None, "plain motion only with any-motion mode");
        let mut x10 = vt100::Parser::new(10, 40, 0);
        x10.process(b"[?1000h");
        assert_eq!(mouse_bytes(x10.screen(), &ev(MouseEventKind::ScrollDown), 0, 0), Some(vec![0x1b, b'[', b'M', 32 + 65, 33, 33]));
        for (_, a) in DEFAULT_PREFIX_KEYS {
            let act: Action = a.parse().unwrap();
            assert_eq!(act.to_config().unwrap().parse::<Action>().unwrap(), act, "{a}");
        }
        for s in ["ctrl+b", "ctrl+space", "alt+a", "%", "N", "f5", "shift+tab", "ctrl+left"] {
            let k: KeySpec = s.parse().unwrap();
            assert_eq!(k.to_config().parse::<KeySpec>().unwrap(), k, "{s}");
        }
        for (k, a) in DEFAULT_PREFIX_KEYS {
            k.parse::<KeySpec>().unwrap();
            a.parse::<Action>().unwrap();
        }
    }

    #[test]
    fn encoding() {
        assert_eq!(encode(&ev(KeyCode::Char('c'), KeyModifiers::CONTROL), false), vec![3]);
        assert_eq!(encode(&ev(KeyCode::Char('x'), KeyModifiers::ALT), false), b"\x1bx");
        assert_eq!(encode(&ev(KeyCode::Up, KeyModifiers::NONE), true), b"\x1bOA");
        assert_eq!(encode(&ev(KeyCode::Up, KeyModifiers::CONTROL), true), b"\x1b[1;5A");
        assert_eq!(encode(&ev(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT), false), b"@");
    }
}
