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
    NextWorkspace,
    PrevWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    SelectWorkspace(usize),
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
    CycleWorkspaceColor,
    /// Search every command, workspace, agent and pane.
    Palette,
    /// Type a task; an agent starts on it.
    QuickPrompt,
    /// Actions for the focused pane.
    Menu,
    Settings,
    /// Move through the sidebar tree with the arrow keys.
    BrowseTree,
    ToggleDock,
    /// Move the focused pane into a workspace of its own, at its current folder.
    PaneToWorkspace,
    /// Recent and project files; Enter puts the path in the prompt.
    Files,
    /// Agent jobs by stage, with review.
    Tasks,
    /// GitHub pull requests and issues, Linear tickets.
    Inbox,
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
    /// Put the pane in a sidebar group (None: take it out of its group).
    SetGroup(Option<String>),
    /// Ask for a name and put the pane in that new group.
    NewGroup,
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
    /// Move the sidebar cursor (-1 up, 1 down); bare keys then work on the sidebar.
    SideMove(i8),
    Detach,
    Help,
    ReloadConfig,
    SendPrefix,
    KillServer,
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
            Action::SplitRight => "split right".into(),
            Action::SplitDown => "split down".into(),
            Action::SplitLeft => "split left".into(),
            Action::SplitUp => "split up".into(),
            Action::Spawn(d, c) => format!("run `{c}` {}", dir(d)),
            Action::SpawnTab(c) => format!("run `{c}` in new tab"),
            Action::ClosePane => "close split".into(),
            Action::Focus(d) => format!("focus {}", dir(d)),
            Action::FocusNext => "next split".into(),
            Action::FocusPrev => "previous pane".into(),
            Action::Resize(d) => format!("resize {}", dir(d)),
            Action::Zoom => "zoom pane".into(),
            Action::NewTab => "new tab".into(),
            Action::NextTab => "next tab".into(),
            Action::PrevTab => "previous tab".into(),
            Action::CloseTab => "close tab".into(),
            Action::RenameTab => "rename tab".into(),
            Action::SelectTab(n) => format!("tab {n}"),
            Action::NewWorkspace => "new group".into(),
            Action::NextWorkspace => "next pane".into(),
            Action::PrevWorkspace => "previous pane".into(),
            Action::CloseWorkspace => "close this pane (all its tabs)".into(),
            Action::RenameWorkspace => "rename pane".into(),
            Action::SelectWorkspace(n) => format!("group {n}"),
            Action::ToggleSidebar => "show / hide sidebar".into(),
            Action::Picker => "jump to…".into(),
            Action::NextAttention => "next agent needing you".into(),
            Action::ScrollUp => "scroll up".into(),
            Action::ScrollDown => "scroll down".into(),
            Action::CopyMode => "copy mode".into(),
            Action::Search => "search history".into(),
            Action::NewWorktree(None) => "worktrees…".into(),
            Action::NewWorktree(Some(c)) => format!("worktrees… running `{c}`"),
            Action::RemoveWorktree => "remove worktree".into(),
            Action::CycleWorkspaceColor => "pane colour".into(),
            Action::Palette => "command palette".into(),
            Action::QuickPrompt => "quick prompt (start an agent on a task)".into(),
            Action::Menu => "pane menu".into(),
            Action::Settings => "settings".into(),
            Action::BrowseTree => "browse workspaces, worktrees & agents".into(),
            Action::ToggleDock => "show / hide agent dock".into(),
            Action::PaneToWorkspace => "group…".into(),
            Action::Files => "files (recent + project)".into(),
            Action::Tasks => "tasks & review".into(),
            Action::Inbox => "tickets (GitHub, Linear, Plane)".into(),
            Action::Toolbox => "toolbox (MCP, skills, plugins)".into(),
            Action::NewPane => "+ new: an agent or a shell".into(),
            Action::Talk => "talk to this worktree's agent".into(),
            Action::Changes => "changes (review and ship)".into(),
            Action::Answer(c) => format!("answer {c}"),
            Action::Reply => "reply to the agent".into(),
            Action::UndoAutoWorkspace => "undo automatic workspace".into(),
            Action::OpenFolder => "open folder".into(),
            Action::StopAgent => "stop agent (Ctrl+C)".into(),
            Action::SetGroup(Some(g)) => format!("move to group {g}"),
            Action::SetGroup(None) => "take out of its group".into(),
            Action::NewGroup => "new group with this pane".into(),
            Action::Jump => "jump to what needs you".into(),
            Action::OpenProject => "open a project".into(),
            Action::NewSession => "+ new, beside this one".into(),
            Action::CloseSplit => "close the split".into(),
            Action::PullRequest => "pull request (checks, reviews, diff)".into(),
            Action::Ship => "ship: commit, push, open the PR".into(),
            Action::Ideas => "ideas: jot one, start an agent on one".into(),
            Action::Race => "race agents on one task".into(),
            Action::Map => "map of the project".into(),
            Action::SideMove(d) if *d < 0 => "sidebar up".into(),
            Action::SideMove(_) => "sidebar down".into(),
            Action::Detach => "detach".into(),
            Action::Help => "help".into(),
            Action::ReloadConfig => "reload config".into(),
            Action::SendPrefix => "send prefix key".into(),
            Action::KillServer => "kill server".into(),
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
            Action::NextWorkspace => "next-workspace".into(),
            Action::PrevWorkspace => "prev-workspace".into(),
            Action::CloseWorkspace => "close-workspace".into(),
            Action::RenameWorkspace => "rename-workspace".into(),
            Action::SelectWorkspace(n) => format!("select-workspace-{n}"),
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
            Action::CycleWorkspaceColor => "cycle-workspace-color".into(),
            Action::Palette => "palette".into(),
            Action::QuickPrompt => "quick-prompt".into(),
            Action::Menu => "menu".into(),
            Action::Settings => "settings".into(),
            Action::BrowseTree => "browse-tree".into(),
            Action::ToggleDock => "toggle-dock".into(),
            Action::PaneToWorkspace => "pane-to-workspace".into(),
            Action::Files => "files".into(),
            Action::Tasks => "tasks".into(),
            Action::Inbox => "inbox".into(),
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
            Action::SideMove(d) if *d < 0 => "side-up".into(),
            Action::SideMove(_) => "side-down".into(),
            Action::Talk => "talk".into(),
            Action::Changes => "changes".into(),
            Action::Answer(c) => format!("answer:{c}"),
            Action::Reply => "reply".into(),
            Action::UndoAutoWorkspace => "undo".into(),
            Action::OpenFolder => "open-folder".into(),
            Action::StopAgent => "stop-agent".into(),
            Action::NewGroup => "new-group".into(),
            Action::Detach => "detach".into(),
            Action::Help => "help".into(),
            Action::ReloadConfig => "reload-config".into(),
            Action::SendPrefix => "send-prefix".into(),
            Action::KillServer => "kill-server".into(),
            Action::SetGroup(_) | Action::None => return None,
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
                "select-workspace" => Action::SelectWorkspace(arg.parse()?),
                "answer" => Action::Answer(arg.chars().next().unwrap_or('1')),
                _ => bail!("unknown action `{s}`"),
            });
        }
        if let Some(n) = s.strip_prefix("select-tab-") {
            return Ok(Action::SelectTab(n.parse()?));
        }
        if let Some(n) = s.strip_prefix("select-workspace-") {
            return Ok(Action::SelectWorkspace(n.parse()?));
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
            "next-workspace" => Action::NextWorkspace,
            "prev-workspace" => Action::PrevWorkspace,
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
            "cycle-workspace-color" | "cycle-workspace-colour" => Action::CycleWorkspaceColor,
            "palette" | "command-palette" => Action::Palette,
            "quick-prompt" => Action::QuickPrompt,
            "menu" => Action::Menu,
            "settings" => Action::Settings,
            "browse-tree" | "browse" => Action::BrowseTree,
            "toggle-dock" => Action::ToggleDock,
            "pane-to-workspace" | "group" => Action::PaneToWorkspace,
            "new-group" => Action::NewGroup,
            "files" => Action::Files,
            "tasks" => Action::Tasks,
            "inbox" => Action::Inbox,
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
            "send-prefix" => Action::SendPrefix,
            "kill-server" => Action::KillServer,
            "none" | "unbind" => Action::None,
            _ => bail!("unknown action `{s}`"),
        })
    }
}

/// Bindings active after the prefix key.
pub const DEFAULT_PREFIX_KEYS: &[(&str, &str)] = &[
    // Sessions (the design's keys)
    ("up", "side-up"),
    ("down", "side-down"),
    ("j", "jump"),
    ("T", "talk"),
    ("t", "talk"),
    ("r", "reply"),
    ("1", "answer:1"),
    ("2", "answer:2"),
    ("3", "answer:3"),
    ("n", "new"),
    ("p", "new-beside"),
    ("o", "open-project"),
    ("x", "close-split"),
    ("X", "close-pane"),
    ("$", "rename-workspace"),
    ("a", "next-attention"),
    ("q", "quick-prompt"),
    // Panes
    ("%", "split-right"),
    ("|", "split-right"),
    ("\"", "split-down"),
    ("-", "split-down"),
    ("left", "focus-left"),
    ("right", "focus-right"),
    ("z", "zoom"),
    ("[", "copy-mode"),
    ("/", "search"),
    ("pageup", "scroll-up"),
    ("pagedown", "scroll-down"),
    // Code
    ("d", "changes"),
    ("P", "pr"),
    ("s", "ship"),
    ("I", "ideas"),
    ("c", "race"),
    ("M", "map"),
    ("f", "files"),
    ("v", "tasks"),
    ("i", "inbox"),
    ("b", "toolbox"),
    ("W", "new-worktree"),
    ("R", "remove-worktree"),
    // App
    ("space", "palette"),
    (":", "palette"),
    ("m", "menu"),
    ("e", "browse-tree"),
    ("B", "toggle-sidebar"),
    (",", "settings"),
    ("S", "settings"),
    ("?", "help"),
    ("u", "undo"),
    ("D", "detach"),
    ("ctrl+r", "reload-config"),
    ("ctrl+space", "send-prefix"),
];

/// Bindings active without the prefix. Empty by default so nothing is stolen from programs.
pub const DEFAULT_GLOBAL_KEYS: &[(&str, &str)] = &[];

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
