//! The TUI client: renders the daemon's snapshot, keeps a terminal emulator per pane, and
//! turns keys into either pane input or commands.

mod copy;
mod design;
mod files;
mod hydra;
mod inbox;
mod menu;
mod modal;
mod pr;
mod render;
mod tasks;
mod toolbox;
mod views;
mod work;

use crate::config::{Config, Keymap};
use crate::ipc;
use crate::keys::{self, Action, KeySpec};
use crate::layout::{self, Dir};
use crate::protocol::*;
use crate::theme::Theme;
use anyhow::Result;
use futures_util::StreamExt;
use ratatui::crossterm::event::{
    self, Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub struct Options {
    /// Open (or switch to) a workspace for this directory on attach.
    pub open: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Normal,
    Prefix { since: Instant },
    /// Jump list (`commands: false`) or command palette (`commands: true`).
    Picker { query: String, sel: usize, commands: bool },
    Menu(modal::Menu),
    Quick(modal::Quick),
    Settings(modal::Settings),
    /// Arrow-key browsing of the sidebar tree.
    Tree { sel: usize },
    Files(Box<files::FilesView>),
    /// Task list; with `review` set, the review screen for one task.
    Tasks { sel: usize, review: Option<Box<tasks::Review>> },
    Inbox(Box<inbox::InboxView>),
    Toolbox(Box<toolbox::ToolboxView>),
    Prompt { kind: PromptKind, input: String },
    Help { scroll: u16 },
    Copy(Box<copy::Copy>),
    /// Pick an existing worktree of the repo, or type a branch to create one.
    Worktrees { ws: WsId, cmd: Option<String>, items: Option<Vec<WorktreeEntry>>, query: String, sel: usize },
    /// Talking to one pane's agent in a modal.
    Talk { term: TermId, input: String },
    NewPane(NewPane),
    /// Hydra layout: sessions that need you, then finished ones.
    Jump { sel: usize },
    /// Hydra layout: open a folder as a project.
    Finder(Box<hydra::Finder>),
    /// Hydra layout: new pane (project, worktree, what to run).
    HyPane(hydra::NewPaneHy),
    /// Hydra layout: the settings overlay.
    HySettings(Box<design::SettingsView>),
    /// Hydra layout: keyboard cursor in the sidebar; bare keys act like leader keys.
    Side,
    /// Ship this branch? (what will happen, then Enter)
    Ship(Box<ShipAsk>),
    Ideas(Box<work::IdeasView>),
    Tickets(Box<work::TicketsView>),
    RaceNew(Box<work::RaceNew>),
    Race(Box<work::RaceView>),
    /// A right-click menu (hydra layout).
    HyMenu(Box<menu::HyMenu>),
}

/// The ship confirm: the branch and what shipping it will do.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ShipAsk {
    pub task: tasks::TaskRow,
    pub changed: usize,
    pub pr: Option<String>,
}

/// The + Pane menu's state.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct NewPane {
    /// 0 = RUN, 1 = IN
    pub section: u8,
    pub run: usize,
    pub place: usize,
    /// Asking for a branch or a folder: (label, text).
    pub input: Option<(String, String)>,
}

/// A view that replaces the pane area.
pub(super) enum View {
    Changes(Box<views::ChangesView>),
    Files(Box<views::FilesTree>),
    Settings(Box<design::SettingsView>),
    Pr(Box<pr::PrView>),
    Map(Box<hydra::MapView>),
}

/// Clickable chips and buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Btn {
    Settings,
    Keys,
    Leader,
    Answer(TermId, char),
    Reply(TermId),
    Zoom(TermId),
    Close(TermId),
    CloseView,
    /// A key inside the current view ('\n' for Enter).
    ViewKey(char),
    /// A row of the current view's list.
    Row(usize),
    NewPaneRow(u8, usize),
    SettingsCat(usize),
    SettingsRow(usize),
    /// The value control of a settings row (click changes it).
    SettingsAct(usize),
}

/// Results of background work (disk scans, git, gh, APIs), delivered to the event loop.
pub(super) enum Bg {
    Recent(PathBuf, Vec<files::FileEntry>),
    Project(PathBuf, Vec<files::FileEntry>),
    Review(Result<Box<tasks::Review>, String>),
    Inbox(usize, PathBuf, Result<Vec<inbox::Item>, String>),
    Toolbox(PathBuf, Vec<toolbox::Section>),
    Changes(PathBuf, Result<Box<tasks::Review>, String>),
    Checks(PathBuf, Option<String>),
    Tree(PathBuf, Vec<views::FileNode>, std::collections::HashMap<String, char>),
    TreeRecent(PathBuf, Vec<files::FileEntry>),
    /// A finished action: its message, and whether the open review should reload.
    Done(Result<String, String>, bool),
    /// Your open pull requests in a project (by key).
    Prs(String, Vec<pr::PrBrief>),
    /// One pull request (by number or branch), and its diff.
    Pr(String, Result<pr::PrInfo, String>),
    PrDiff(String, Result<String, String>),
    /// Tickets for a folder, from the source at this tab.
    Tickets(PathBuf, usize, Result<Vec<work::Ticket>, String>),
    /// A race entry's diff stat: (race, entry, text).
    RaceStat(u64, usize, String),
    /// Pulled a shared setup from another machine.
    Synced(bool),
}

/// The pull request checks for a branch, if it has a pull request: "✓ checks 14/14" or
/// "✕ 2 of 14 checks failing".
fn pr_checks(dir: &std::path::Path, branch: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.current_dir(dir).args(["pr", "view", branch, "--json", "statusCheckRollup"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let checks = v.get("statusCheckRollup")?.as_array()?;
    if checks.is_empty() {
        return None;
    }
    let verdict = |c: &serde_json::Value| {
        c.get("conclusion").and_then(|x| x.as_str()).filter(|x| !x.is_empty()).or_else(|| c.get("state").and_then(|x| x.as_str())).unwrap_or("PENDING").to_string()
    };
    let bad = checks.iter().filter(|c| matches!(verdict(c).as_str(), "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT")).count();
    let ok = checks.iter().filter(|c| matches!(verdict(c).as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED")).count();
    Some(if bad > 0 {
        format!("✕ {bad} of {} checks failing", checks.len())
    } else if ok == checks.len() {
        format!("✓ checks {ok}/{}", checks.len())
    } else {
        format!("⠹ checks {ok}/{}", checks.len())
    })
}

/// Remove a worktree that isn't open as a workspace.
fn remove_worktree_dir(dir: &std::path::Path) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["worktree", "remove", "--force", "."])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() { Ok(format!("removed {}", dir.display())) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Four hex digits, different each call (for branch names like `claude-3f2a`).
fn short_id() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(0);
    format!("{:04x}", n & 0xffff)
}

/// Used where a key handler replaces the view itself and must not restore the old one.
fn true_and_replace() -> bool {
    false
}

/// The repository root above `p`, or `p` itself.
fn repo_root(p: &std::path::Path) -> PathBuf {
    p.ancestors().find(|a| a.join(".git").exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| p.to_path_buf())
}

/// A row of the sidebar tree: repos > workspaces and worktrees > agents.
#[derive(Debug, Clone)]
pub(super) enum TreeRow {
    Repo { name: String },
    Workspace { ws: WsId, depth: u8, label: String },
    /// A pane: agents first, then shells (with the folder they're in).
    Agent { term: TermId, depth: u8 },
    NewWorkspace,
    /// A worktree of the repo that isn't open in hydra.
    Closed { entry: WorktreeEntry, depth: u8 },
    NewWorktree { ws: WsId, depth: u8 },
}

/// A comparable form of a path (case- and separator-insensitive), cheap enough to run
/// every frame.
fn path_key(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// A row in the worktree picker.
#[derive(Debug, Clone)]
enum WtRow {
    Existing(WorktreeEntry),
    Create(String),
}

#[derive(Debug, Clone, PartialEq)]
enum PromptKind {
    RenameTab(WsId, TabId),
    RenameWorkspace(WsId),
    NewWorkspace,
    NewGroupFor(WsId),
    ConfirmCloseWorkspace(WsId),
    ConfirmKillServer,
    ConfirmRemoveWorktree(WsId),
}

impl PromptKind {
    fn label(&self) -> &'static str {
        match self {
            PromptKind::RenameTab(..) => "Rename tab",
            PromptKind::RenameWorkspace(_) => "Rename workspace",
            PromptKind::NewWorkspace => "New pane: name it (empty = its folder)",
            PromptKind::NewGroupFor(_) => "New group: name it",
            PromptKind::ConfirmCloseWorkspace(_) => "Close workspace and all its panes? (y/n)",
            PromptKind::ConfirmKillServer => "Kill the server and every pane? (y/n)",
            PromptKind::ConfirmRemoveWorktree(_) => "Remove this worktree? (y / f = force / n)",
        }
    }
    fn is_confirm(&self) -> bool {
        matches!(
            self,
            PromptKind::ConfirmCloseWorkspace(_) | PromptKind::ConfirmKillServer | PromptKind::ConfirmRemoveWorktree(_)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    Workspace(WsId),
    Tab(WsId, TabId),
    NewTab,
    Pane(TermId),
    MenuItem(usize),
    TreeRow(usize),
    AddPane,
    SideRow(usize),
    SideTalk(usize),
    Button(Btn),
    Hy(hydra::HyHit),
}

/// A row in the jump picker.
#[derive(Debug, Clone)]
struct PickItem {
    label: String,
    detail: String,
    status: Status,
    /// Shortcut shown on the right (commands only).
    key: String,
    target: PickTarget,
}

#[derive(Debug, Clone)]
enum PickTarget {
    Workspace(WsId),
    Pane(TermId),
    Command(Action),
}

pub struct App {
    cfg: Config,
    theme: Theme,
    keymap: Keymap,
    notice: Option<(String, Instant, bool)>,
    snap: Snapshot,
    got_state: bool,
    parsers: HashMap<TermId, vt100::Parser>,
    scroll: HashMap<TermId, usize>,
    sizes: HashMap<TermId, (u16, u16)>,
    mode: Mode,
    zoomed: HashSet<TabId>,
    sidebar: bool,
    dock: bool,
    hits: Vec<(Rect, Hit)>,
    /// Inner rects of the panes drawn last frame.
    panes: Vec<(TermId, Rect)>,
    /// Outer rects of the panes drawn last frame (for neighbour search).
    pane_frames: Vec<(TermId, Rect)>,
    out: mpsc::UnboundedSender<ClientMsg>,
    quit: Option<String>,
    dirty: bool,
    started: Instant,
    open: Option<PathBuf>,
    /// Where a left-button press started, for drag-to-select.
    drag: Option<(TermId, Position)>,
    bg: mpsc::UnboundedSender<Bg>,
    view: Option<View>,
    /// Workspaces (repo keys) folded shut in the sidebar.
    collapsed: HashSet<String>,
    /// Where the mouse is, for hover highlights.
    hover: Option<Position>,
    /// The status line's message can be undone with `u`.
    undo_hint: bool,
    /// The last click, for double-click detection.
    last_click: Option<(Hit, Instant)>,
    /// A sidebar row a context menu or browse targets.
    side_target: Option<usize>,
    /// The welcome screen is up; which of its panes is selected.
    splash: bool,
    /// The hydra layout's own state.
    hy: hydra::Hy,
    /// The terminal window has focus (for alerts about the session you're looking at).
    window_focused: bool,
    /// The background we told the terminal to use (OSC 11), so its padding matches.
    osc_bg: Option<ratatui::style::Color>,
}

pub fn run(opts: Options) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    rt.block_on(run_async(opts))
}

async fn run_async(opts: Options) -> Result<()> {
    let (mut reader, mut writer) = ipc::open_or_spawn(true).await?;
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMsg>();
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ipc::send(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });

    let (cfg, err) = Config::load_or_default();
    let (bg_tx, mut bg_rx) = mpsc::unbounded_channel::<Bg>();
    let mut app = App::new(cfg, out_tx, opts.open, bg_tx);
    if crate::sync::enabled() {
        app.spawn_bg(|| Bg::Synced(crate::sync::pull().unwrap_or(false)));
    }
    if let Some(e) = err {
        app.notify(format!("config error: {e}"), true);
    }

    let mut terminal = ratatui::init();
    if app.cfg.ui.mouse {
        let _ = execute!(std::io::stdout(), event::EnableMouseCapture);
    }
    let _ = execute!(std::io::stdout(), event::EnableBracketedPaste, event::EnableFocusChange);

    let result = app.event_loop(&mut terminal, &mut reader, &mut bg_rx).await;

    let _ = execute!(
        std::io::stdout(),
        event::DisableMouseCapture,
        event::DisableBracketedPaste,
        event::DisableFocusChange
    );
    // Give the terminal its own background back.
    {
        use std::io::Write;
        let _ = write!(std::io::stdout(), "]111");
    }
    ratatui::restore();
    result?;
    if let Some(reason) = app.quit {
        println!("[{reason}]");
    }
    Ok(())
}

impl App {
    fn new(cfg: Config, out: mpsc::UnboundedSender<ClientMsg>, open: Option<PathBuf>, bg_tx: mpsc::UnboundedSender<Bg>) -> App {
        let keymap = cfg.keymap();
        let mut app = App {
            theme: cfg.theme(),
            sidebar: cfg.ui.sidebar,
            dock: cfg.ui.dock,
            keymap,
            cfg,
            notice: None,
            snap: Snapshot::default(),
            got_state: false,
            parsers: HashMap::new(),
            scroll: HashMap::new(),
            sizes: HashMap::new(),
            mode: Mode::Normal,
            zoomed: HashSet::new(),
            hits: Vec::new(),
            panes: Vec::new(),
            pane_frames: Vec::new(),
            out,
            quit: None,
            dirty: true,
            started: Instant::now(),
            open,
            drag: None,
            bg: bg_tx,
            view: None,
            collapsed: HashSet::new(),
            hover: None,
            undo_hint: false,
            last_click: None,
            side_target: None,
            splash: false,
            hy: hydra::Hy::load(),
            window_focused: true,
            osc_bg: None,
        };
        app.splash = app.cfg.ui.splash;
        if !crate::theme::BUILTIN.contains(&app.cfg.theme.as_str()) {
            app.keymap.warnings.push(format!(
                "unknown theme `{}` (have: {})",
                app.cfg.theme,
                crate::theme::BUILTIN.join(", ")
            ));
        }
        if !app.keymap.warnings.is_empty() {
            app.notify(format!("config: {}", app.keymap.warnings.join("; ")), true);
        }
        app
    }

    async fn event_loop(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        reader: &mut ipc::Reader,
        bg_rx: &mut mpsc::UnboundedReceiver<Bg>,
    ) -> Result<()> {
        let mut events = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        let mut last_draw = Instant::now() - Duration::from_secs(1);
        let mut last_frame = 0u64;
        loop {
            tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(ev)) => self.on_event(ev),
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                },
                Some(b) = bg_rx.recv() => {
                    self.on_bg(b);
                    self.dirty = true;
                }
                msg = ipc::recv_server(reader) => match msg {
                    Ok(Some(m)) => self.on_server(m),
                    _ => {
                        self.quit.get_or_insert_with(|| "server exited".into());
                    }
                },
                _ = tick.tick() => {
                    let frame = self.spinner_frame();
                    if frame != last_frame && self.any_working() {
                        last_frame = frame;
                        self.dirty = true;
                    }
                    if matches!(self.mode, Mode::Prefix { .. }) {
                        self.dirty = true;
                    }
                    if self.notice.as_ref().is_some_and(|(_, at, _)| at.elapsed() > Duration::from_secs(5)) {
                        self.notice = None;
                        self.dirty = true;
                    }
                }
            }
            if self.quit.is_some() {
                return Ok(());
            }
            if self.dirty && last_draw.elapsed() >= Duration::from_millis(12) {
                self.dirty = false;
                last_draw = Instant::now();
                terminal.draw(|f| render::draw(self, f))?;
                self.sync_sizes();
            }
        }
    }

    fn notify(&mut self, msg: String, error: bool) {
        self.notice = Some((msg, Instant::now(), error));
        self.dirty = true;
    }

    fn send(&self, msg: ClientMsg) {
        let _ = self.out.send(msg);
    }

    fn cmd(&self, c: Command) {
        self.send(ClientMsg::Command(c));
    }

    fn spinner_frame(&self) -> u64 {
        (self.started.elapsed().as_millis() / 90) as u64
    }

    fn any_working(&self) -> bool {
        self.snap.terms.values().any(|t| t.status == Status::Working)
    }

    // ---- model helpers -------------------------------------------------------------

    fn active_ws(&self) -> Option<&WorkspaceInfo> {
        self.snap.active()
    }

    fn active_tab(&self) -> Option<&TabInfo> {
        self.active_ws().and_then(|w| w.tab())
    }

    fn focused(&self) -> Option<TermId> {
        self.active_tab().map(|t| t.focus)
    }

    fn is_zoomed(&self) -> bool {
        self.active_tab().is_some_and(|t| self.zoomed.contains(&t.id) && t.layout.leaves().len() > 1)
    }

    fn new_parser(&self, rows: u16, cols: u16) -> vt100::Parser {
        vt100::Parser::new(rows.max(1), cols.max(1), self.cfg.scrollback)
    }

    /// Resize panes whose drawn size differs from what the daemon last heard.
    fn sync_sizes(&mut self) {
        let panes = self.panes.clone();
        for (term, r) in panes {
            let want = (r.width.max(2), r.height.max(2));
            if self.sizes.get(&term) == Some(&want) {
                continue;
            }
            self.sizes.insert(term, want);
            if let Some(p) = self.parsers.get_mut(&term) {
                p.screen_mut().set_size(want.1, want.0);
            }
            self.send(ClientMsg::Resize { term, cols: want.0, rows: want.1 });
        }
    }

    // ---- server messages -----------------------------------------------------------

    fn on_server(&mut self, msg: ServerMsg) {
        self.dirty = true;
        match msg {
            ServerMsg::State(s) => {
                self.parsers.retain(|id, _| s.terms.contains_key(id));
                self.scroll.retain(|id, _| s.terms.contains_key(id));
                for (id, t) in &s.terms {
                    if !self.parsers.contains_key(id) {
                        let p = self.new_parser(t.rows, t.cols);
                        self.parsers.insert(*id, p);
                    }
                    // Trust the daemon's size for panes we aren't showing.
                    if !self.panes.iter().any(|(p, _)| p == id) {
                        self.sizes.insert(*id, (t.cols, t.rows));
                    }
                }
                let first = !self.got_state;
                self.got_state = true;
                self.snap = s;
                if first {
                    self.on_first_state();
                }
                if self.cfg.ui.layout == "hydra" {
                    self.hy_sync();
                }
            }
            ServerMsg::Replay { term, cols, rows, data } => {
                let mut p = self.new_parser(rows, cols);
                p.process(&data);
                self.parsers.insert(term, p);
                self.sizes.insert(term, (cols, rows));
            }
            ServerMsg::Output { term, data } => {
                if let Some(p) = self.parsers.get_mut(&term) {
                    p.process(&data);
                } else if let Some(t) = self.snap.terms.get(&term) {
                    let mut p = self.new_parser(t.rows, t.cols);
                    p.process(&data);
                    self.parsers.insert(term, p);
                }
            }
            ServerMsg::Attention { term, status } => {
                if self.focused() == Some(term) && self.window_focused {
                    return;
                }
                let (title, body) = self.alert_text(term, status);
                let kind = if status == Status::Blocked { crate::alert::Kind::Needs } else { crate::alert::Kind::Done };
                crate::alert::alert(&self.cfg.notify, kind, &title, &body);
                if self.focused() == Some(term) {
                    return;
                }
                if self.cfg.notify.bell {
                    use std::io::Write;
                    let _ = std::io::stdout().write_all(b"\x07");
                    let _ = std::io::stdout().flush();
                }
                let who = self.describe_term(term);
                let what = if status == Status::Blocked { "needs you" } else { "finished" };
                self.notify(format!("{who} {what}"), false);
            }
            ServerMsg::Error(e) => self.notify(e, true),
            ServerMsg::Notice(n) => {
                self.undo_hint = false;
                self.notify(n, false)
            }
            ServerMsg::AutoWorkspace(n) => {
                self.notify(n, false);
                self.undo_hint = true;
                if let Some((_, at, _)) = &mut self.notice {
                    // Leave time to undo.
                    *at = Instant::now() + Duration::from_secs(10);
                }
            }
            ServerMsg::Bye => self.quit = Some("server exited".into()),
            ServerMsg::Reply(Reply::Worktrees(list)) => {
                if let Mode::Worktrees { items, .. } = &mut self.mode {
                    *items = Some(list);
                }
            }
            ServerMsg::Welcome { .. } | ServerMsg::Reply(_) => {}
        }
    }

    fn on_first_state(&mut self) {
        let open = self.open.take();
        if let Some(dir) = &open {
            let dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if let Some(w) = self.snap.workspaces.iter().find(|w| same_dir(&w.cwd, &dir)) {
                self.cmd(Command::SelectWorkspace { ws: w.id });
                return;
            }
        }
        if self.snap.workspaces.is_empty() || open.is_some() {
            let cwd = open.or_else(|| std::env::current_dir().ok());
            self.cmd(Command::NewWorkspace { cwd, name: None, cmd: None });
        }
    }

    fn describe_term(&self, term: TermId) -> String {
        let name = self.snap.terms.get(&term).map(|t| t.display_name().to_string()).unwrap_or_default();
        match self.snap.locate(term) {
            Some((w, t)) => {
                let idx = w.tabs.iter().position(|x| x.id == t.id).unwrap_or(0) + 1;
                format!("{name} ({} › {idx})", w.name)
            }
            None => name,
        }
    }

    // ---- input ---------------------------------------------------------------------

    fn on_event(&mut self, ev: Event) {
        self.dirty = true;
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Paste(s) => self.on_paste(s),
            Event::FocusGained => self.window_focused = true,
            Event::FocusLost => self.window_focused = false,
            Event::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    fn on_paste(&mut self, s: String) {
        match &mut self.mode {
            Mode::Copy(c) => {
                if let Some(input) = &mut c.input {
                    input.push_str(s.lines().next().unwrap_or(""));
                }
            }
            Mode::Quick(q) => q.text.push_str(&s.replace("\r\n", "\n")),
            Mode::Files(v) => v.query.push_str(s.lines().next().unwrap_or("")),
            Mode::Inbox(v) => v.query.push_str(s.lines().next().unwrap_or("")),
            Mode::Toolbox(v) => v.query.push_str(s.lines().next().unwrap_or("")),
            Mode::Prompt { input, .. } | Mode::Picker { query: input, .. } | Mode::Worktrees { query: input, .. } => {
                input.push_str(s.lines().next().unwrap_or(""));
            }
            _ => {
                let Some(term) = self.focused() else { return };
                let bracketed = self.parsers.get(&term).is_some_and(|p| p.screen().bracketed_paste());
                let body = s.replace("\r\n", "\r").replace('\n', "\r");
                let data = if bracketed { format!("\x1b[200~{body}\x1b[201~") } else { body };
                self.scroll.remove(&term);
                self.send(ClientMsg::Input { term, data: data.into_bytes() });
            }
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        let spec = KeySpec::from_event(&k);
        if self.splash {
            self.on_hy_splash_key(&k);
            return;
        }
        if let Mode::Copy(c) = &mut self.mode {
            match c.key(&k) {
                copy::Outcome::Stay => {}
                copy::Outcome::Exit => self.mode = Mode::Normal,
                copy::Outcome::Yank(text) => self.yank(text),
            }
            return;
        }
        match self.mode.clone() {
            Mode::Normal => {
                if spec == self.keymap.prefix {
                    self.mode = Mode::Prefix { since: Instant::now() };
                } else if let Some(a) = self.keymap.global.get(&spec).cloned() {
                    self.act(a);
                } else if self.view.is_some() {
                    self.on_view_key(&k);
                } else {
                    self.forward_key(&k);
                }
            }
            Mode::Talk { term, input } => self.on_talk_key(term, input, &k),
            Mode::NewPane(np) => self.on_new_pane_key(np, &k),
            Mode::Jump { sel } => self.on_jump_key(sel, &k),
            Mode::Finder(fd) => self.on_finder_key(*fd, &k),
            Mode::HyPane(np) => self.on_hy_pane_key(np, &k),
            Mode::HySettings(_) => self.hy_settings_key(&k),
            Mode::Side => self.on_side_key(&k),
            Mode::Ideas(v) => self.on_ideas_key(*v, &k),
            Mode::Tickets(v) => self.on_tickets_key(*v, &k),
            Mode::RaceNew(v) => self.on_race_new_key(*v, &k),
            Mode::Race(v) => self.on_race_key(*v, &k),
            Mode::HyMenu(m) => self.on_hy_menu_key(*m, &k),
            Mode::Ship(ask) => {
                self.mode = Mode::Normal;
                if k.code == KeyCode::Enter {
                    let task = ask.task.clone();
                    self.notify(format!("shipping {}…", task.branch), false);
                    self.spawn_bg(move || Bg::Done(tasks::ship(&task), false));
                }
            }
            Mode::Prefix { .. } => {
                self.mode = Mode::Normal;
                if k.code == KeyCode::Esc {
                    return;
                }
                if let Some(a) = self.keymap.prefixed.get(&spec).cloned() {
                    let repeat = a.repeats();
                    self.act(a);
                    if repeat && self.mode == Mode::Normal {
                        self.mode = Mode::Prefix { since: Instant::now() - Duration::from_secs(60) };
                    }
                }
            }
            Mode::Help { scroll } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.mode = Mode::Help { scroll: scroll + 1 },
                KeyCode::Up | KeyCode::Char('k') => self.mode = Mode::Help { scroll: scroll.saturating_sub(1) },
                KeyCode::PageDown => self.mode = Mode::Help { scroll: scroll + 10 },
                KeyCode::PageUp => self.mode = Mode::Help { scroll: scroll.saturating_sub(10) },
                _ => self.mode = Mode::Normal,
            },
            Mode::Picker { mut query, mut sel, commands } => {
                let n = self.pick_items(&query, commands).len();
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        if let Some(item) = self.pick_items(&query, commands).get(sel) {
                            match item.target.clone() {
                                PickTarget::Workspace(ws) => self.cmd(Command::SelectWorkspace { ws }),
                                PickTarget::Pane(term) => self.cmd(Command::FocusPane { term }),
                                PickTarget::Command(a) => self.act(a),
                            }
                        }
                        return;
                    }
                    KeyCode::Down | KeyCode::Tab => sel = (sel + 1).min(n.saturating_sub(1)),
                    KeyCode::Char('n') if ctrl => sel = (sel + 1).min(n.saturating_sub(1)),
                    KeyCode::Up | KeyCode::BackTab => sel = sel.saturating_sub(1),
                    KeyCode::Char('p') if ctrl => sel = sel.saturating_sub(1),
                    KeyCode::Backspace => {
                        query.pop();
                        sel = 0;
                    }
                    KeyCode::Char(c) if !ctrl => {
                        query.push(c);
                        sel = 0;
                    }
                    _ => {}
                }
                self.mode = Mode::Picker { query, sel, commands };
            }
            Mode::Menu(mut m) => {
                let pick = match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                        m.sel = (m.sel + 1) % m.items.len().max(1);
                        None
                    }
                    KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                        m.sel = (m.sel + m.items.len().max(1) - 1) % m.items.len().max(1);
                        None
                    }
                    KeyCode::Enter => m.items.get(m.sel).map(|i| i.action.clone()),
                    // The item's own shortcut works inside the menu, without the leader.
                    _ => self.keymap.prefixed.get(&spec).filter(|a| m.items.iter().any(|i| &i.action == *a)).cloned(),
                };
                match pick {
                    Some(a) => {
                        self.mode = Mode::Normal;
                        self.act(a);
                    }
                    None => self.mode = Mode::Menu(m),
                }
            }
            Mode::Tree { mut sel } if self.cfg.ui.layout == "workspaces" => {
                let n = self.side_rows().len().max(1);
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Down | KeyCode::Char('j') => sel = (sel + 1) % n,
                    KeyCode::Up | KeyCode::Char('k') => sel = (sel + n - 1) % n,
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        self.mode = Mode::Normal;
                        self.on_button(Hit::SideRow(sel), false);
                        return;
                    }
                    KeyCode::Char('T') => {
                        self.act(Action::Talk);
                        return;
                    }
                    KeyCode::Char('d') => {
                        self.act(Action::Changes);
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Char('f') => {
                        let dir = self.target_path();
                        self.mode = Mode::Normal;
                        if let Some(d) = dir {
                            self.open_files(d);
                        }
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::Tree { sel };
            }
            Mode::Tree { mut sel } => {
                let n = self.tree_rows().len().max(1);
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => sel = (sel + 1) % n,
                    KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => sel = (sel + n - 1) % n,
                    KeyCode::Home | KeyCode::Char('g') => sel = 0,
                    KeyCode::End | KeyCode::Char('G') => sel = n - 1,
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        self.mode = Mode::Normal;
                        self.activate_tree_row(sel);
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::Tree { sel };
            }
            Mode::Quick(q) => self.on_quick_key(q, &k),
            Mode::Files(v) => self.on_files_key(*v, &k),
            Mode::Tasks { sel, review } => self.on_tasks_key(sel, review, &k),
            Mode::Inbox(v) => self.on_inbox_key(*v, &k),
            Mode::Toolbox(v) => self.on_toolbox_key(*v, &k),
            Mode::Settings(s) => self.on_settings_key(s, &k),
            Mode::Worktrees { ws, cmd, items, mut query, mut sel } => {
                let rows = self.worktree_rows(items.as_deref(), &query);
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        return;
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        match rows.get(sel) {
                            Some(WtRow::Existing(w)) => self.open_worktree(w, cmd),
                            Some(WtRow::Create(text)) => {
                                let mut words = text.split_whitespace();
                                if let Some(branch) = words.next().map(str::to_string) {
                                    let base = words.next().map(str::to_string);
                                    self.notify(format!("creating worktree {branch}…"), false);
                                    self.cmd(Command::NewWorktree { ws, branch, base, cmd, split: None, from: None });
                                }
                            }
                            None => {}
                        }
                        return;
                    }
                    KeyCode::Down | KeyCode::Tab => sel = (sel + 1).min(rows.len().saturating_sub(1)),
                    KeyCode::Char('n') if ctrl => sel = (sel + 1).min(rows.len().saturating_sub(1)),
                    KeyCode::Up | KeyCode::BackTab => sel = sel.saturating_sub(1),
                    KeyCode::Char('p') if ctrl => sel = sel.saturating_sub(1),
                    KeyCode::Backspace => {
                        query.pop();
                        sel = 0;
                    }
                    KeyCode::Char(c) if !ctrl => {
                        query.push(c);
                        sel = 0;
                    }
                    _ => {}
                }
                self.mode = Mode::Worktrees { ws, cmd, items, query, sel };
            }
            Mode::Prompt { kind, mut input } => {
                if kind.is_confirm() {
                    self.mode = Mode::Normal;
                    let yes = matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter);
                    let force = matches!(k.code, KeyCode::Char('f') | KeyCode::Char('F'));
                    match kind {
                        PromptKind::ConfirmCloseWorkspace(ws) if yes => self.cmd(Command::CloseWorkspace { ws }),
                        PromptKind::ConfirmKillServer if yes => self.cmd(Command::KillServer { forget: false }),
                        PromptKind::ConfirmRemoveWorktree(ws) if yes || force => {
                            self.notify("removing worktree…".into(), false);
                            self.cmd(Command::RemoveWorktree { ws, force, delete_branch: false });
                        }
                        _ => {}
                    }
                    return;
                }
                match k.code {
                    KeyCode::Esc => self.mode = Mode::Normal,
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        self.submit_prompt(kind, input);
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.mode = Mode::Prompt { kind, input };
                    }
                    KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.mode = Mode::Prompt { kind, input: String::new() };
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        self.mode = Mode::Prompt { kind, input };
                    }
                    _ => self.mode = Mode::Prompt { kind, input },
                }
            }
            Mode::Copy(_) => unreachable!("handled above"),
        }
    }

    fn submit_prompt(&mut self, kind: PromptKind, input: String) {
        let input = input.trim().to_string();
        match kind {
            PromptKind::RenameTab(ws, tab) => self.cmd(Command::RenameTab { ws, tab, name: input }),
            PromptKind::RenameWorkspace(ws) if !input.is_empty() => {
                self.cmd(Command::RenameWorkspace { ws, name: input })
            }
            PromptKind::NewWorkspace => {
                // A group starts with a shell where you are.
                let name = (!input.is_empty()).then_some(input);
                self.cmd(Command::NewWorkspace { cwd: Some(self.here_dir()), name, cmd: None });
            }
            PromptKind::NewGroupFor(ws)
                if !input.is_empty() => {
                    self.collapsed.remove(&App::group_key(&input));
                    self.cmd(Command::SetGroup { ws, group: Some(input) });
                }
            _ => {}
        }
    }

    /// Rows for the worktree picker: matching worktrees, then "create <query>" when the
    /// query isn't already an existing branch.
    fn worktree_rows(&self, items: Option<&[WorktreeEntry]>, query: &str) -> Vec<WtRow> {
        let q = query.trim().to_lowercase();
        let items = items.unwrap_or_default();
        let mut rows: Vec<WtRow> = items
            .iter()
            .filter(|w| q.is_empty() || w.branch.to_lowercase().contains(&q) || w.path.to_string_lossy().to_lowercase().contains(&q))
            .cloned()
            .map(WtRow::Existing)
            .collect();
        let first = q.split_whitespace().next().unwrap_or("");
        if !first.is_empty() && !items.iter().any(|w| w.branch.to_lowercase() == first) {
            rows.push(WtRow::Create(query.trim().to_string()));
        }
        rows
    }

    /// Switch to the workspace already open on this worktree, or open one.
    fn open_worktree(&mut self, w: &WorktreeEntry, cmd: Option<String>) {
        if let Some(open) = self.snap.workspaces.iter().find(|x| same_dir(&x.cwd, &w.path)) {
            self.cmd(Command::SelectWorkspace { ws: open.id });
            if let Some(cmd) = cmd {
                self.cmd(Command::NewTab { ws: open.id, name: None, cmd: Some(cmd) });
            }
            return;
        }
        let name = if w.main { w.repo.clone() } else { format!("{}:{}", w.repo, w.branch) };
        self.cmd(Command::NewWorkspace { cwd: Some(w.path.clone()), name: Some(name), cmd });
    }

    /// The colour of a workspace, from the configured palette.
    fn ws_color(&self, w: &WorkspaceInfo) -> ratatui::style::Color {
        let pal = &self.cfg.ui.workspace_colors;
        if pal.is_empty() {
            return self.theme.accent;
        }
        crate::theme::parse_color(&pal[w.color as usize % pal.len()]).unwrap_or(self.theme.accent)
    }

    fn yank(&mut self, text: String) {
        self.mode = Mode::Normal;
        if text.is_empty() {
            return;
        }
        let lines = text.lines().count().max(1);
        let native = copy::to_clipboard(&text);
        let how = if native { "" } else { " (via terminal)" };
        self.notify(format!("copied {} chars, {lines} line(s){how}", text.chars().count()), false);
    }

    fn enter_copy(&mut self, term: TermId) -> bool {
        let scroll = self.scroll.get(&term).copied().unwrap_or(0);
        let Some(p) = self.parsers.get_mut(&term) else { return false };
        let mut c = copy::Copy::new(term, p, scroll);
        if let Some((_, r)) = self.panes.iter().find(|(t, _)| *t == term) {
            c.height = r.height as usize;
            c.width = r.width as usize;
        }
        self.mode = Mode::Copy(Box::new(c));
        true
    }

    fn forward_key(&mut self, k: &KeyEvent) {
        let Some(term) = self.focused() else { return };
        let app_cursor = self.parsers.get(&term).is_some_and(|p| p.screen().application_cursor());
        let data = keys::encode(k, app_cursor);
        if data.is_empty() {
            return;
        }
        self.scroll.remove(&term);
        if let Some(p) = self.parsers.get_mut(&term) {
            p.screen_mut().set_scrollback(0);
        }
        self.send(ClientMsg::Input { term, data });
    }

    /// The pane under `pos` and the cell inside it.
    fn pane_cell(&self, pos: Position) -> Option<(TermId, u16, u16)> {
        let (term, r) = self.panes.iter().find(|(_, r)| r.contains(pos))?;
        Some((*term, pos.y - r.y, pos.x - r.x))
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        // Dragging the sidebar edge or the split divider.
        if let Some(d) = self.hy.drag {
            match m.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    match d {
                        hydra::Drag::Side => {
                            let side = self.hy.side_rect;
                            let w = if self.cfg.ui.sidebar_position == "right" { side.right().saturating_sub(m.column) } else { m.column.saturating_sub(side.x) };
                            self.hy.saved.side_w = Some(w.clamp(hydra::SIDE_MIN, hydra::SIDE_MAX));
                        }
                        hydra::Drag::Split => {
                            if let Some((r, stack)) = self.hy.split_rect {
                                let f = if stack {
                                    (m.row.saturating_sub(r.y)) as f32 / r.height.max(1) as f32
                                } else {
                                    (m.column.saturating_sub(r.x)) as f32 / r.width.max(1) as f32
                                };
                                self.hy.saved.split = Some(f.clamp(0.2, 0.8));
                            }
                        }
                    }
                    self.dirty = true;
                    return;
                }
                MouseEventKind::Up(_) => {
                    self.hy.drag = None;
                    self.hy.save();
                    self.dirty = true;
                    return;
                }
                _ => {}
            }
        }
        if self.cfg.ui.layout == "hydra" && m.kind == MouseEventKind::Down(MouseButton::Right) && !self.splash {
            let hit = self.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
            let at = (m.column, m.row);
            match hit {
                Some(Hit::Hy(hydra::HyHit::Session(t))) => self.menu_for_session(t, at),
                Some(Hit::Hy(hydra::HyHit::ToggleProj(pi))) => self.menu_for_project(pi, at),
                Some(Hit::Hy(hydra::HyHit::Wt(i))) => {
                    if let Some((key, _)) = self.hy.wt_keys.get(i).cloned() {
                        self.menu_for_place(key, at);
                    }
                }
                _ => {
                    if let Some((term, _)) = self.pane_frames.iter().find(|(_, r)| r.contains(pos)).copied() {
                        self.menu_for_pane(term, at);
                    }
                }
            }
            self.dirty = true;
            return;
        }
        if m.kind == MouseEventKind::Moved {
            if self.hover != Some(pos) {
                self.hover = Some(pos);
                self.dirty = true;
            }
            return;
        }
        if m.kind == MouseEventKind::Down(MouseButton::Left) && !matches!(self.mode, Mode::Menu(_)) {
            let hit = self.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
            if let Some(h @ Hit::Hy(hh)) = hit {
                let double = self.last_click.is_some_and(|(prev, at)| prev == h && at.elapsed() < Duration::from_millis(400));
                self.last_click = Some((h, Instant::now()));
                if matches!(self.mode, Mode::Prefix { .. } | Mode::Side) {
                    self.mode = Mode::Normal;
                }
                self.on_hy_hit(hh, double);
                self.dirty = true;
                return;
            }
            // The splash waits for one of its buttons.
            if self.splash {
                return;
            }
            if let Some(h @ (Hit::AddPane | Hit::SideRow(_) | Hit::SideTalk(_) | Hit::Button(_))) = hit {
                let double = self.last_click.is_some_and(|(prev, at)| prev == h && at.elapsed() < Duration::from_millis(400));
                self.last_click = Some((h, Instant::now()));
                if matches!(self.mode, Mode::Prefix { .. } | Mode::Help { .. } | Mode::Tree { .. }) {
                    self.mode = Mode::Normal;
                }
                if self.on_button(h, double) {
                    self.dirty = true;
                    return;
                }
            }
            if matches!(self.mode, Mode::Talk { .. } | Mode::NewPane(_)) {
                self.mode = Mode::Normal;
                return;
            }
        }
        if m.kind == MouseEventKind::Down(MouseButton::Right)
            && let Some(Hit::SideRow(i)) = self.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h)
        {
            self.side_target = Some(i);
            let items = [Action::Talk, Action::Changes, Action::Files, Action::OpenFolder, Action::StopAgent, Action::RemoveWorktree]
                .into_iter()
                .map(|a| modal::MenuItem { label: a.describe(), key: self.key_for(&a), action: a })
                .collect();
            self.mode = Mode::Menu(modal::Menu { items, sel: 0, at: Some((m.column, m.row)) });
            return;
        }
        if self.on_mouse_select(&m, pos) {
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some((term, _)) = self.pane_frames.iter().find(|(_, r)| r.contains(pos)).copied() {
                    if Some(term) != self.focused() {
                        self.cmd(Command::FocusPane { term });
                    }
                    self.open_menu(Some((m.column, m.row)));
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Mode::Menu(menu) = &self.mode {
                    // Clicking a menu row picks it; anywhere else closes the menu.
                    let hit = self.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
                    let action = match hit {
                        Some(Hit::MenuItem(i)) => menu.items.get(i).map(|i| i.action.clone()),
                        _ => None,
                    };
                    self.mode = Mode::Normal;
                    if let Some(a) = action {
                        self.act(a);
                    }
                    return;
                }
                if !matches!(self.mode, Mode::Normal) {
                    self.mode = Mode::Normal;
                    return;
                }
                if let Some((term, _, _)) = self.pane_cell(pos) {
                    self.drag = Some((term, pos));
                }
                let hit = self.hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, h)| *h);
                match hit {
                    Some(Hit::Workspace(ws)) => self.cmd(Command::SelectWorkspace { ws }),
                    Some(Hit::Tab(ws, tab)) => {
                        self.view = None;
                        self.cmd(Command::SelectTab { ws, tab });
                    }
                    Some(Hit::NewTab) => self.act(Action::NewTab),
                    Some(Hit::Pane(term)) => self.cmd(Command::FocusPane { term }),
                    Some(Hit::MenuItem(_)) => {}
                    Some(Hit::TreeRow(i)) => self.activate_tree_row(i),
                    Some(Hit::AddPane | Hit::SideRow(_) | Hit::SideTalk(_) | Hit::Button(_) | Hit::Hy(_)) => {}
                    None => {
                        if let Some((term, _)) = self.pane_frames.iter().find(|(_, r)| r.contains(pos))
                            && Some(*term) != self.focused() {
                                self.cmd(Command::FocusPane { term: *term });
                            }
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if self.hy.side_rect.contains(pos) {
                    self.hy.side_scroll = if up { self.hy.side_scroll.saturating_sub(3) } else { self.hy.side_scroll + 3 };
                    self.dirty = true;
                    return;
                }
                let Some((term, _)) = self.pane_frames.iter().find(|(_, r)| r.contains(pos)).copied() else {
                    return;
                };
                let alt = self.parsers.get(&term).is_some_and(|p| p.screen().alternate_screen());
                if alt {
                    // Full-screen programs scroll themselves; send them arrow keys.
                    let key = if up { b"\x1b[A" } else { b"\x1b[B" };
                    self.send(ClientMsg::Input { term, data: key.repeat(3) });
                } else {
                    self.scroll_by(term, if up { 3 } else { -3 });
                }
            }
            _ => {}
        }
    }

    /// Drag to select (enters copy mode), release to copy; clicks and wheel inside copy mode.
    /// Returns true when the event was consumed.
    fn on_mouse_select(&mut self, m: &MouseEvent, pos: Position) -> bool {
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Mode::Copy(c) = &mut self.mode {
                    if let Some((r_term, r)) = self.panes.iter().find(|(t, _)| *t == c.term) {
                        let _ = r_term;
                        // Dragging past the edge scrolls.
                        if pos.y < r.y {
                            c.scroll(-1);
                        } else if pos.y >= r.bottom() {
                            c.scroll(1);
                        }
                        let row = pos.y.clamp(r.y, r.bottom().saturating_sub(1)) - r.y;
                        let col = pos.x.clamp(r.x, r.right().saturating_sub(1)) - r.x;
                        let at = c.at_cell(row, col);
                        if c.anchor.is_none() {
                            c.anchor = Some(c.cur);
                        }
                        c.move_to(at);
                    }
                    return true;
                }
                let Some((term, start)) = self.drag else { return false };
                if start == pos || !matches!(self.mode, Mode::Normal) || !self.enter_copy(term) {
                    return false;
                }
                let Some((_, r)) = self.panes.iter().find(|(t, _)| *t == term).copied() else { return true };
                if let Mode::Copy(c) = &mut self.mode {
                    c.mouse = true;
                    let s = c.at_cell(start.y.saturating_sub(r.y), start.x.saturating_sub(r.x));
                    c.cur = s;
                    c.anchor = Some(s);
                    let row = pos.y.clamp(r.y, r.bottom().saturating_sub(1)) - r.y;
                    let col = pos.x.clamp(r.x, r.right().saturating_sub(1)) - r.x;
                    let at = c.at_cell(row, col);
                    c.move_to(at);
                }
                true
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag = None;
                if let Mode::Copy(c) = &self.mode
                    && c.mouse
                {
                    let text = c.selected_text();
                    self.yank(text);
                    return true;
                }
                false
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Mode::Copy(c) = &mut self.mode else { return false };
                let Some((_, r)) = self.panes.iter().find(|(t, _)| *t == c.term).copied() else { return false };
                if !r.contains(pos) {
                    self.mode = Mode::Normal;
                    return true;
                }
                let at = c.at_cell(pos.y - r.y, pos.x - r.x);
                c.anchor = None;
                c.move_to(at);
                true
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Mode::Copy(c) = &mut self.mode else { return false };
                c.scroll(if m.kind == MouseEventKind::ScrollUp { -3 } else { 3 });
                true
            }
            _ => false,
        }
    }

    fn scroll_by(&mut self, term: TermId, delta: i32) {
        let Some(p) = self.parsers.get_mut(&term) else { return };
        let cur = self.scroll.get(&term).copied().unwrap_or(0) as i32;
        let want = (cur + delta).max(0) as usize;
        p.screen_mut().set_scrollback(want);
        // vt100 clamps to the history it has.
        let actual = p.screen().scrollback();
        if actual == 0 {
            self.scroll.remove(&term);
        } else {
            self.scroll.insert(term, actual);
        }
    }

    // ---- actions -------------------------------------------------------------------

    fn act(&mut self, a: Action) {
        if self.cfg.ui.layout == "hydra" && self.hy_act(&a) {
            return;
        }
        let ws = self.active_ws().map(|w| w.id);
        let focused = self.focused();
        match a {
            Action::Jump => self.mode = Mode::Picker { query: String::new(), sel: 0, commands: false },
            Action::OpenProject => self.act(Action::NewWorkspace),
            Action::NewSession => self.act(Action::NewPane),
            Action::CloseSplit => self.act(Action::ClosePane),
            Action::Ship => {
                if let Some(dir) = self.target_path() {
                    self.ask_ship(dir);
                }
            }
            Action::Ideas => self.open_ideas(),
            Action::Race => self.open_race_new(),
            Action::Map => self.open_map(),
            Action::PullRequest => {
                if let Some(dir) = self.target_path()
                    && let Some(h) = crate::gitfs::head(&dir)
                {
                    self.open_pr(h.top, h.branch);
                }
            }
            Action::SideMove(d) => self.act(Action::Focus(if d < 0 { Dir::Up } else { Dir::Down })),
            Action::SplitRight => self.split(Dir::Right, None),
            Action::SplitDown => self.split(Dir::Down, None),
            Action::SplitLeft => self.split(Dir::Left, None),
            Action::SplitUp => self.split(Dir::Up, None),
            Action::Spawn(d, c) => self.split(d, Some(c)),
            Action::SpawnTab(c) => {
                if let Some(ws) = ws {
                    self.cmd(Command::NewTab { ws, name: None, cmd: Some(c) });
                }
            }
            Action::ClosePane => {
                if let Some(term) = focused {
                    self.cmd(Command::ClosePane { term });
                }
            }
            Action::Focus(d) => {
                if let Some(term) = focused
                    && let Some(n) = layout::neighbor(&self.pane_frames, term, d) {
                        if let Some(t) = self.active_tab().map(|t| t.id) {
                            self.zoomed.remove(&t);
                        }
                        self.cmd(Command::FocusPane { term: n });
                    }
            }
            Action::FocusNext | Action::FocusPrev => {
                let Some(tab) = self.active_tab() else { return };
                let leaves = tab.layout.leaves();
                let i = leaves.iter().position(|t| Some(*t) == focused).unwrap_or(0);
                let n = leaves.len();
                let next = if a == Action::FocusNext { (i + 1) % n } else { (i + n - 1) % n };
                self.cmd(Command::FocusPane { term: leaves[next] });
            }
            Action::Resize(d) => {
                if let Some(term) = focused {
                    self.cmd(Command::ResizePane { term, dir: d, delta: 0.05 });
                }
            }
            Action::Zoom => {
                if let Some(t) = self.active_tab().map(|t| t.id)
                    && !self.zoomed.remove(&t) {
                        self.zoomed.insert(t);
                    }
            }
            Action::NewTab => {
                if let Some(ws) = ws {
                    self.cmd(Command::NewTab { ws, name: None, cmd: None });
                }
            }
            Action::NextTab | Action::PrevTab | Action::SelectTab(_) => {
                let Some(w) = self.active_ws() else { return };
                let n = w.tabs.len();
                let i = w.tabs.iter().position(|t| t.id == w.active_tab).unwrap_or(0);
                let target = match a {
                    Action::NextTab => (i + 1) % n,
                    Action::PrevTab => (i + n - 1) % n,
                    Action::SelectTab(k) if k >= 1 && k <= n => k - 1,
                    _ => return,
                };
                self.cmd(Command::SelectTab { ws: w.id, tab: w.tabs[target].id });
            }
            Action::CloseTab => {
                if let (Some(ws), Some(tab)) = (ws, self.active_tab().map(|t| t.id)) {
                    self.cmd(Command::CloseTab { ws, tab });
                }
            }
            Action::RenameTab => {
                if let (Some(ws), Some(tab)) = (ws, self.active_tab()) {
                    self.mode = Mode::Prompt { kind: PromptKind::RenameTab(ws, tab.id), input: tab.name.clone() };
                }
            }
            Action::NewWorkspace => {
                self.mode = Mode::Prompt { kind: PromptKind::NewWorkspace, input: String::new() };
            }
            Action::SetGroup(group) => {
                if let Some(ws) = self.target_ws() {
                    self.cmd(Command::SetGroup { ws, group });
                }
            }
            Action::NewGroup => {
                if let Some(ws) = self.target_ws() {
                    self.mode = Mode::Prompt { kind: PromptKind::NewGroupFor(ws), input: String::new() };
                }
            }
            Action::NextWorkspace | Action::PrevWorkspace | Action::SelectWorkspace(_) => {
                let list = &self.snap.workspaces;
                let n = list.len();
                if n == 0 {
                    return;
                }
                let i = list.iter().position(|w| Some(w.id) == ws).unwrap_or(0);
                let target = match a {
                    Action::NextWorkspace => (i + 1) % n,
                    Action::PrevWorkspace => (i + n - 1) % n,
                    Action::SelectWorkspace(k) if k >= 1 && k <= n => k - 1,
                    _ => return,
                };
                self.cmd(Command::SelectWorkspace { ws: list[target].id });
            }
            Action::CloseWorkspace => {
                if let Some(ws) = ws {
                    self.mode = Mode::Prompt { kind: PromptKind::ConfirmCloseWorkspace(ws), input: String::new() };
                }
            }
            Action::RenameWorkspace => {
                if let Some(w) = self.active_ws() {
                    self.mode = Mode::Prompt { kind: PromptKind::RenameWorkspace(w.id), input: w.name.clone() };
                }
            }
            Action::ToggleSidebar => self.sidebar = !self.sidebar,
            Action::Picker => self.mode = Mode::Picker { query: String::new(), sel: 0, commands: false },
            Action::Palette => self.mode = Mode::Picker { query: String::new(), sel: 0, commands: true },
            Action::QuickPrompt => {
                let agent = 0;
                let place = modal::Place::parse(&self.cfg.quick.place);
                self.mode = Mode::Quick(modal::Quick { text: String::new(), agent, place });
            }
            Action::Menu => self.open_menu(None),
            Action::BrowseTree => {
                if self.cfg.ui.layout != "tree" {
                    // No tree on screen: the jump list does the same job.
                    self.mode = Mode::Picker { query: String::new(), sel: 0, commands: false };
                    return;
                }
                self.sidebar = true;
                if self.cfg.ui.layout == "workspaces" {
                    let rows = self.side_rows();
                    let sel = rows.iter().position(|r| matches!(r, design::SideRow::Pane { primary: true, .. })).unwrap_or(0);
                    self.mode = Mode::Tree { sel };
                    return;
                }
                let rows = self.tree_rows();
                let sel = rows
                    .iter()
                    .position(|r| matches!(r, TreeRow::Workspace { ws: w, .. } if Some(*w) == ws))
                    .unwrap_or(0);
                self.mode = Mode::Tree { sel };
            }
            Action::ToggleDock => self.dock = !self.dock,
            Action::Files if self.cfg.ui.layout == "workspaces" => {
                let dir = self.target_path().unwrap_or_else(|| self.here_dir());
                self.open_files(dir);
            }
            Action::NewPane => {
                let place = 0;
                self.mode = Mode::NewPane(NewPane { section: 0, run: 0, place, input: None });
            }
            Action::Talk => match self.target_term() {
                Some(term) => self.mode = Mode::Talk { term, input: String::new() },
                None => self.notify("nothing is running in that worktree".into(), true),
            },
            Action::Changes => {
                if let Some(dir) = self.target_path() {
                    self.open_changes(dir);
                }
            }
            Action::Answer(c) => if let Some(term) = focused { self.send(ClientMsg::Input { term, data: c.to_string().into_bytes() }) },
            Action::Reply => {
                self.mode = Mode::Quick(modal::Quick { text: String::new(), agent: 0, place: modal::Place::Here });
            }
            Action::UndoAutoWorkspace => {
                self.undo_hint = false;
                self.notice = None;
                self.cmd(Command::UndoAutoWorkspace);
            }
            Action::OpenFolder => {
                if let Some(p) = self.target_path() {
                    let _ = files::open_default(&p);
                }
            }
            Action::StopAgent => {
                if let Some(term) = self.target_term() {
                    self.send(ClientMsg::Input { term, data: vec![3] });
                }
            }
            Action::Files => {
                let root = self.here_dir();
                let mut v = files::FilesView::new(repo_root(&root));
                v.root = repo_root(&root);
                let r = v.root.clone();
                self.spawn_bg(move || Bg::Recent(r.clone(), files::scan_recent(&r)));
                self.mode = Mode::Files(Box::new(v));
            }
            Action::Tasks => self.mode = Mode::Tasks { sel: 0, review: None },
            Action::Inbox => {
                let dir = repo_root(&self.here_dir());
                let d = dir.clone();
                self.spawn_bg(move || Bg::Inbox(0, d.clone(), inbox::load(0, &d)));
                self.mode = Mode::Inbox(Box::new(inbox::InboxView::new(dir)));
            }
            Action::Toolbox => {
                let dir = repo_root(&self.here_dir());
                let d = dir.clone();
                self.spawn_bg(move || Bg::Toolbox(d.clone(), toolbox::scan(&d)));
                self.mode = Mode::Toolbox(Box::new(toolbox::ToolboxView::new(dir)));
            }
            Action::PaneToWorkspace => {
                // Choose a group for this pane (or a new one), in a centered menu.
                let Some(ws) = self.target_ws() else { return };
                let current = self.snap.workspace(ws).and_then(|w| w.group.clone());
                let mut groups: Vec<String> = self.snap.workspaces.iter().filter_map(|w| w.group.clone()).collect();
                groups.dedup();
                let mut items: Vec<modal::MenuItem> = groups
                    .into_iter()
                    .filter(|g| Some(g) != current.as_ref())
                    .map(|g| modal::MenuItem { label: format!("move to {g}"), key: String::new(), action: Action::SetGroup(Some(g)) })
                    .collect();
                items.push(modal::MenuItem { label: "new group…".into(), key: String::new(), action: Action::NewGroup });
                if current.is_some() {
                    items.push(modal::MenuItem { label: "take out of group".into(), key: String::new(), action: Action::SetGroup(None) });
                }
                self.mode = Mode::Menu(modal::Menu { items, sel: 0, at: None });
            }
            Action::Settings => {
                if self.cfg.ui.layout == "workspaces" {
                    self.view = Some(View::Settings(Box::new(design::SettingsView { cat: 0, sel: 0, editing: None, capturing: false, scroll: 0 })));
                } else {
                    self.mode = Mode::Settings(modal::Settings { sel: 0, editing: None, capturing: false });
                }
            }
            Action::NextAttention => self.next_attention(),
            Action::ScrollUp | Action::ScrollDown => {
                if let Some(term) = focused {
                    let half = self.sizes.get(&term).map(|s| s.1 / 2).unwrap_or(10).max(1) as i32;
                    self.scroll_by(term, if a == Action::ScrollUp { half } else { -half });
                }
            }
            Action::CopyMode | Action::Search => {
                if let Some(term) = focused
                    && self.enter_copy(term)
                    && a == Action::Search
                    && let Mode::Copy(c) = &mut self.mode
                {
                    c.input = Some(String::new());
                    c.backward = true;
                }
            }
            Action::NewWorktree(cmd) => {
                let Some(w) = self.active_ws() else { return };
                if w.git.is_none() {
                    let here = focused.and_then(|id| self.snap.terms.get(&id)).map(|t| t.cwd.clone());
                    let hint = match here {
                        Some(h) if h != w.cwd => format!(
                            "workspace {} isn't a git repo — {} M makes this pane's folder a workspace",
                            w.name, self.keymap.prefix
                        ),
                        _ => format!("{} is not in a git repository", w.cwd.display()),
                    };
                    self.notify(hint, true);
                    return;
                }
                let ws = w.id;
                self.send(ClientMsg::Query(Query::Worktrees { ws }));
                self.mode = Mode::Worktrees { ws, cmd, items: None, query: String::new(), sel: 0 };
            }
            Action::CycleWorkspaceColor => {
                if let Some(w) = self.active_ws() {
                    let n = self.cfg.ui.workspace_colors.len().clamp(1, 255) as u8;
                    self.cmd(Command::SetWorkspaceColor { ws: w.id, color: (w.color + 1) % n });
                }
            }
            Action::RemoveWorktree => {
                let Some(w) = self.active_ws() else { return };
                if !w.worktree && !w.git.as_ref().is_some_and(|g| g.linked) {
                    self.notify("this workspace is not a linked worktree".into(), true);
                    return;
                }
                self.mode = Mode::Prompt { kind: PromptKind::ConfirmRemoveWorktree(w.id), input: String::new() };
            }
            Action::Detach => self.quit = Some("detached".into()),
            Action::Help => self.mode = Mode::Help { scroll: 0 },
            Action::ReloadConfig => self.reload_config(),
            Action::SendPrefix => {
                let p = self.keymap.prefix;
                let ev = KeyEvent::new(p.code, p.mods);
                self.forward_key(&ev);
            }
            Action::KillServer => {
                self.mode = Mode::Prompt { kind: PromptKind::ConfirmKillServer, input: String::new() };
            }
            Action::None => {}
        }
    }

    fn split(&mut self, dir: Dir, cmd: Option<String>) {
        if let Some(term) = self.focused() {
            if let Some(t) = self.active_tab().map(|t| t.id) {
                self.zoomed.remove(&t);
            }
            self.cmd(Command::Split { term, dir, cmd, cwd: None });
        }
    }

    fn next_attention(&mut self) {
        let mut waiting: Vec<&TermInfo> = self
            .snap
            .terms
            .values()
            .filter(|t| matches!(t.status, Status::Blocked | Status::Done))
            .collect();
        if waiting.is_empty() {
            self.notify("no agent is waiting on you".into(), false);
            return;
        }
        waiting.sort_by_key(|t| (t.status.urgency(), t.id));
        let cur = self.focused();
        let i = waiting.iter().position(|t| Some(t.id) == cur).map(|i| i + 1).unwrap_or(0);
        let term = waiting[i % waiting.len()].id;
        self.cmd(Command::FocusPane { term });
    }

    fn reload_config(&mut self) {
        match Config::load() {
            Ok(cfg) => {
                self.keymap = cfg.keymap();
                self.theme = cfg.theme();
                self.sidebar = cfg.ui.sidebar;
                self.dock = cfg.ui.dock;
                let mouse_changed = cfg.ui.mouse != self.cfg.ui.mouse;
                self.cfg = cfg;
                if mouse_changed {
                    let _ = if self.cfg.ui.mouse {
                        execute!(std::io::stdout(), event::EnableMouseCapture)
                    } else {
                        execute!(std::io::stdout(), event::DisableMouseCapture)
                    };
                }
                self.cmd(Command::ReloadConfig);
                if self.keymap.warnings.is_empty() {
                    self.notify("config reloaded".into(), false);
                } else {
                    self.notify(format!("config: {}", self.keymap.warnings.join("; ")), true);
                }
            }
            Err(e) => self.notify(format!("config error: {e:#}"), true),
        }
    }

    /// The sidebar tree. Workspaces of one repository are grouped under it with the repo's
    /// other worktrees; agents sit under the workspace they run in.
    pub(super) fn tree_rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        let mut done: Vec<String> = Vec::new();
        let active = self.snap.active_ws;
        let agents_of = |w: &WorkspaceInfo| -> Vec<TermId> {
            let mut v: Vec<&TermInfo> =
                w.tabs.iter().flat_map(|t| t.layout.leaves()).filter_map(|id| self.snap.terms.get(&id)).collect();
            v.sort_by_key(|t| (t.status.urgency(), t.id));
            v.into_iter().map(|t| t.id).collect()
        };
        for w in &self.snap.workspaces {
            let Some(g) = &w.git else {
                rows.push(TreeRow::Workspace { ws: w.id, depth: 0, label: w.name.clone() });
                rows.extend(agents_of(w).into_iter().map(|term| TreeRow::Agent { term, depth: 1 }));
                continue;
            };
            let root = path_key(&g.root);
            if done.contains(&root) {
                continue;
            }
            done.push(root.clone());
            let members: Vec<&WorkspaceInfo> = self
                .snap
                .workspaces
                .iter()
                .filter(|m| m.git.as_ref().is_some_and(|mg| path_key(&mg.root) == root))
                .collect();
            let open: Vec<String> = members.iter().map(|m| path_key(&m.cwd)).collect();
            let closed: Vec<&WorktreeEntry> = g.worktrees.iter().filter(|e| !open.contains(&path_key(&e.path))).collect();
            let has_active = members.iter().any(|m| Some(m.id) == active);
            if members.len() == 1 && closed.is_empty() {
                rows.push(TreeRow::Workspace { ws: w.id, depth: 0, label: w.name.clone() });
                rows.extend(agents_of(w).into_iter().map(|term| TreeRow::Agent { term, depth: 1 }));
                if has_active {
                    rows.push(TreeRow::NewWorktree { ws: w.id, depth: 1 });
                }
                continue;
            }
            rows.push(TreeRow::Repo { name: g.repo.clone() });
            for m in &members {
                // Under its repo, a worktree workspace is best named by its branch.
                let mg = m.git.as_ref().unwrap();
                let generated = m.name == mg.repo || m.name == format!("{}:{}", mg.repo, mg.branch);
                let label = if generated { mg.branch.clone() } else { m.name.clone() };
                rows.push(TreeRow::Workspace { ws: m.id, depth: 1, label });
                rows.extend(agents_of(m).into_iter().map(|term| TreeRow::Agent { term, depth: 2 }));
            }
            rows.extend(closed.into_iter().map(|e| TreeRow::Closed { entry: e.clone(), depth: 1 }));
            rows.push(TreeRow::NewWorktree { ws: members[0].id, depth: 1 });
        }
        rows.push(TreeRow::NewWorkspace);
        rows
    }

    fn activate_tree_row(&mut self, i: usize) {
        let Some(row) = self.tree_rows().get(i).cloned() else { return };
        match row {
            TreeRow::Repo { .. } => {}
            TreeRow::Workspace { ws, .. } => self.cmd(Command::SelectWorkspace { ws }),
            TreeRow::Agent { term, .. } => self.cmd(Command::FocusPane { term }),
            TreeRow::Closed { entry, .. } => self.open_worktree(&entry, None),
            TreeRow::NewWorkspace => self.act(Action::NewWorkspace),
            TreeRow::NewWorktree { ws, .. } => {
                self.send(ClientMsg::Query(Query::Worktrees { ws }));
                self.mode = Mode::Worktrees { ws, cmd: None, items: None, query: String::new(), sel: 0 };
            }
        }
    }

    /// "C-b %" for an action bound after the leader, "M-h" for a global one.
    fn key_for(&self, a: &Action) -> String {
        if let Some(k) = self.keymap.prefixed_order.iter().find(|k| self.keymap.prefixed.get(k) == Some(a)) {
            return format!("{} {k}", self.keymap.prefix);
        }
        self.keymap.global.iter().find(|(_, x)| *x == a).map(|(k, _)| k.to_string()).unwrap_or_default()
    }

    /// Every command worth offering in the palette, plus your own spawn bindings.
    fn palette_commands(&self) -> Vec<Action> {
        use crate::layout::Dir;
        let mut list = vec![
            Action::QuickPrompt,
            Action::SplitRight,
            Action::SplitDown,
            Action::NewTab,
            Action::Files,
            Action::Tasks,
            Action::Inbox,
            Action::Toolbox,
            Action::NewWorkspace,
            Action::PaneToWorkspace,
            Action::NewWorktree(None),
            Action::Picker,
            Action::NextAttention,
            Action::Zoom,
            Action::ClosePane,
            Action::CloseTab,
            Action::RenameTab,
            Action::RenameWorkspace,
            Action::CloseWorkspace,
            Action::RemoveWorktree,
            Action::CycleWorkspaceColor,
            Action::CopyMode,
            Action::Search,
            Action::Focus(Dir::Left),
            Action::Focus(Dir::Right),
            Action::Focus(Dir::Up),
            Action::Focus(Dir::Down),
            Action::NextTab,
            Action::PrevTab,
            Action::NextWorkspace,
            Action::PrevWorkspace,
            Action::ToggleSidebar,
            Action::Menu,
            Action::Settings,
            Action::Help,
            Action::ReloadConfig,
            Action::Detach,
            Action::KillServer,
        ];
        for a in self.keymap.prefixed.values().chain(self.keymap.global.values()) {
            if matches!(a, Action::Spawn(..) | Action::SpawnTab(_) | Action::NewWorktree(Some(_))) && !list.contains(a) {
                list.push(a.clone());
            }
        }
        list
    }

    fn open_menu(&mut self, at: Option<(u16, u16)>) {
        let Some(w) = self.active_ws() else { return };
        let linked = w.worktree || w.git.as_ref().is_some_and(|g| g.linked);
        let git = w.git.is_some();
        let mut actions = vec![
            Action::QuickPrompt,
            Action::Files,
            Action::SplitRight,
            Action::SplitDown,
            Action::Zoom,
            Action::CopyMode,
            Action::Search,
            Action::RenameTab,
            Action::CycleWorkspaceColor,
            Action::PaneToWorkspace,
        ];
        if git {
            actions.push(Action::NewWorktree(None));
        }
        if linked {
            actions.push(Action::RemoveWorktree);
        }
        actions.extend([Action::ClosePane, Action::CloseTab, Action::Settings]);
        let items = actions
            .into_iter()
            .map(|a| modal::MenuItem { label: a.describe(), key: self.key_for(&a), action: a })
            .collect();
        self.mode = Mode::Menu(modal::Menu { items, sel: 0, at });
    }

    /// The pane (sidebar item) a command applies to: the browse cursor's or a
    /// right-clicked row's, else the one on screen.
    fn target_ws(&self) -> Option<WsId> {
        let rows = self.side_rows();
        let idx = match self.mode {
            Mode::Tree { sel } if self.cfg.ui.layout == "workspaces" => Some(sel),
            _ => self.side_target,
        };
        match idx.and_then(|i| rows.get(i)) {
            Some(design::SideRow::Pane { ws, .. } | design::SideRow::Detail { ws, .. }) => Some(*ws),
            _ => self.snap.active_ws,
        }
    }

    /// The terminal a command applies to: the target pane's focused one.
    fn target_term(&self) -> Option<TermId> {
        self.target_ws().and_then(|ws| self.snap.workspace(ws)).and_then(|w| w.tab()).map(|t| t.focus).or(self.focused())
    }

    /// The checkout (or folder) the target pane is in.
    fn target_path(&self) -> Option<PathBuf> {
        let t = self.target_term().and_then(|id| self.snap.terms.get(&id))?;
        Some(crate::gitfs::head(&t.cwd).map(|h| h.top).unwrap_or_else(|| t.cwd.clone()))
    }

    /// The agents + Shell offered by + Pane.
    pub(super) fn new_pane_runs(&self) -> Vec<String> {
        let mut v: Vec<String> = self.cfg.quick.agents.iter().map(|a| a.name.clone()).collect();
        v.push("Shell".into());
        v
    }

    /// Where + Pane can open: (label, detail). An agent in a repo gets its own worktree
    /// by default, so agents never step on each other.
    pub(super) fn new_pane_places(&self, run: usize) -> Vec<(String, String)> {
        let here = self.here_dir();
        let folder = design::tilde(&here);
        let agent = run < self.cfg.quick.agents.len();
        let in_git = crate::gitfs::head(&here).is_some();
        let mut v = Vec::new();
        if agent && in_git && self.cfg.worktree.per_agent {
            v.push(("new worktree".to_string(), format!("{folder} · own branch")));
            v.push(("same checkout".to_string(), folder.clone()));
        } else {
            v.push(("this folder".to_string(), folder));
        }
        v.push(("another folder…".into(), "pick a folder".into()));
        v
    }

    /// The ship confirm for the branch at `dir`.
    pub(super) fn ask_ship(&mut self, dir: PathBuf) {
        let Some(head) = crate::gitfs::head(&dir) else {
            self.notify("not a git repo".into(), true);
            return;
        };
        let base = if head.linked { crate::gitfs::main_branch(&head.main_root).unwrap_or_else(|| "main".into()) } else { String::new() };
        if !head.linked && crate::gitfs::main_branch(&head.main_root).is_some_and(|m| m == head.branch) {
            self.notify(format!("you're on {}: ship works from a branch (start an agent with + New to get one)", head.branch), true);
            return;
        }
        let term = self
            .snap
            .terms
            .values()
            .filter(|t| t.agent.is_some() && t.top.as_ref().is_some_and(|p| design::path_key(p) == design::path_key(&head.top)))
            .map(|t| (t.id, t.summary.clone()))
            .next();
        let task = tasks::TaskRow {
            ws: self.snap.active_ws.unwrap_or(0),
            name: head.branch.clone(),
            branch: head.branch.clone(),
            base: if base.is_empty() { "main".into() } else { base },
            stage: tasks::Stage::Ready,
            summary: term.as_ref().map(|(_, s)| s.clone()).unwrap_or_default(),
            dirty: 0,
            ahead: 0,
            agent: term.map(|(id, _)| id),
            dir: head.top.clone(),
            root: head.main_root.clone(),
        };
        let changed = std::process::Command::new("git")
            .arg("-C")
            .arg(&head.top)
            .args(["status", "--porcelain"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        let key = design::path_key(&head.main_root);
        let pr = self.hy.prs.get(&key).and_then(|l| l.iter().find(|p| p.branch == head.branch)).map(|p| p.number.to_string());
        self.mode = Mode::Ship(Box::new(ShipAsk { task, changed, pr }));
    }

    fn open_changes(&mut self, dir: PathBuf) {
        let head = crate::gitfs::head(&dir);
        let top = head.as_ref().map(|h| h.top.clone()).unwrap_or_else(|| dir.clone());
        let linked = head.as_ref().is_some_and(|h| h.linked);
        let branch = head.as_ref().map(|h| h.branch.clone()).unwrap_or_default();
        let base = match &head {
            Some(h) if h.linked => crate::gitfs::main_branch(&h.main_root).unwrap_or_else(|| "main".into()),
            _ => branch.clone(),
        };
        // The pane working in this checkout, preferring one with an agent.
        let mut here: Vec<&TermInfo> = self
            .snap
            .terms
            .values()
            .filter(|t| crate::gitfs::head(&t.cwd).is_some_and(|h| design::path_key(&h.top) == design::path_key(&top)))
            .collect();
        here.sort_by_key(|t| (t.agent.is_none(), t.status.urgency()));
        let info = here.first().map(|t| (*t).clone());
        let term = info.as_ref().map(|i| i.id);
        let ws = term.and_then(|t| self.snap.locate(t).map(|(w, _)| w.id)).or(self.snap.active_ws);
        let task = tasks::TaskRow {
            ws: ws.unwrap_or(0),
            name: branch.clone(),
            branch: branch.clone(),
            base,
            stage: tasks::Stage::Ready,
            summary: info.as_ref().map(|i| i.summary.clone()).unwrap_or_default(),
            dirty: 0,
            ahead: 0,
            agent: term,
            dir: top.clone(),
            root: head.as_ref().map(|h| h.main_root.clone()).unwrap_or_else(|| top.clone()),
        };
        self.view = Some(View::Changes(Box::new(views::ChangesView {
            dir: top.clone(),
            review: None,
            error: if head.is_none() { Some(format!("{} isn't in a git repository.", design::tilde(&dir))) } else { None },
            agent: info.as_ref().and_then(|i| i.agent.clone()).unwrap_or_default(),
            said: info.as_ref().map(|i| i.said.clone()).unwrap_or_default(),
            term,
            ws,
            checks: None,
            linked,
            confirm: None,
        })));
        if head.is_none() {
            return;
        }
        let d = top.clone();
        self.spawn_bg(move || Bg::Changes(d, tasks::load_review(task).map(Box::new)));
        let (d, b) = (top, branch);
        self.spawn_bg(move || Bg::Checks(d.clone(), pr_checks(&d, &b)));
    }

    fn open_files(&mut self, dir: PathBuf) {
        let head = crate::gitfs::head(&dir);
        let root = head.as_ref().map(|h| h.top.clone()).unwrap_or(dir);
        let branch = head.map(|h| h.branch);
        self.view = Some(View::Files(Box::new(views::FilesTree::new(root.clone(), branch))));
        self.spawn_bg(move || Bg::Tree(root.clone(), views::scan_tree(&root), views::git_marks(&root)));
    }

    /// Keys while a view replaces the pane area (single letters act directly).
    fn on_view_key(&mut self, k: &KeyEvent) {
        let Some(view) = self.view.take() else { return };
        match view {
            View::Changes(mut v) => {
                if self.on_changes_key(&mut v, k) {
                    self.view = Some(View::Changes(v));
                }
            }
            View::Files(mut v) => {
                if self.on_files_tree_key(&mut v, k) {
                    self.view = Some(View::Files(v));
                }
            }
            View::Settings(mut v) => {
                if self.on_settings_view_key(&mut v, k) {
                    self.view = Some(View::Settings(v));
                }
            }
            View::Pr(mut v) => {
                if self.on_pr_key(&mut v, k) {
                    self.view = Some(View::Pr(v));
                }
            }
            View::Map(mut v) => {
                if self.on_map_key(&mut v, k) && self.view.is_none() {
                    self.view = Some(View::Map(v));
                }
            }
        }
    }

    /// The pull request of a branch (or a number), in the main area.
    pub(super) fn open_pr(&mut self, dir: PathBuf, which: String) {
        self.view = Some(View::Pr(Box::new(pr::PrView { dir: dir.clone(), which: which.clone(), info: None, diff: None, tab: 0, scroll: 0 })));
        self.spawn_bg(move || Bg::Pr(which.clone(), pr::load(&dir, &which)));
    }

    /// Returns false when the view closes.
    fn on_pr_key(&mut self, v: &mut pr::PrView, k: &KeyEvent) -> bool {
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Tab | KeyCode::BackTab => {
                v.tab = 1 - v.tab;
                v.scroll = 0;
                if v.tab == 1 && v.diff.is_none() {
                    let (dir, which) = (v.dir.clone(), v.which.clone());
                    self.spawn_bg(move || Bg::PrDiff(which.clone(), pr::diff(&dir, &which)));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => v.scroll = v.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = v.scroll.saturating_add(15),
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(15),
            KeyCode::Char('o') => {
                if let Some(Ok(i)) = &v.info {
                    inbox::open_url(&i.url);
                }
            }
            KeyCode::Char('r') => {
                v.info = None;
                v.diff = None;
                let (dir, which) = (v.dir.clone(), v.which.clone());
                self.spawn_bg(move || Bg::Pr(which.clone(), pr::load(&dir, &which)));
            }
            KeyCode::Char('f') => {
                let Some(Ok(info)) = &v.info else { return true };
                // The agent working in this branch's folder.
                let key = design::path_key(&v.dir);
                let term = self
                    .snap
                    .terms
                    .values()
                    .filter(|t| t.agent.is_some() && t.top.as_ref().is_some_and(|p| design::path_key(p) == key))
                    .map(|t| t.id)
                    .next();
                match term {
                    Some(term) => {
                        self.mode = Mode::Talk { term, input: info.fix_prompt() };
                        return false;
                    }
                    None => self.notify("no agent is working in this branch; start one with + New".into(), true),
                }
            }
            _ => {}
        }
        true
    }

    /// Open a file in your editor: terminal editors inside hydra beside what you're on,
    /// others (VS Code, …) as their own window.
    pub(super) fn open_in_editor(&mut self, path: &std::path::Path) {
        let ed = [self.cfg.editor.clone(), std::env::var("VISUAL").unwrap_or_default(), std::env::var("EDITOR").unwrap_or_default()]
            .into_iter()
            .find(|e| !e.trim().is_empty())
            .unwrap_or_else(|| "code".into());
        let exe = ed.split_whitespace().next().unwrap_or("code");
        let name = std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let line = format!("{ed} {}", files::quote_path(path));
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.here_dir());
        if ["nvim", "vim", "vi", "hx", "helix", "nano", "micro", "kak", "emacs", "ne"].contains(&name.as_str()) {
            if self.cfg.ui.layout == "hydra" {
                self.hy_new_session(dir, Some(line), true);
            } else if let Some(term) = self.focused() {
                self.cmd(Command::Split { term, dir: Dir::Right, cmd: Some(line), cwd: Some(dir) });
            }
            self.view = None;
            return;
        }
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", &line]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", &line]);
            c
        };
        cmd.current_dir(&dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.spawn() {
            Ok(_) => self.notify(format!("opened {} in {exe}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), false),
            Err(e) => self.notify(format!("couldn't start {exe}: {e}"), true),
        }
    }

    /// Returns false when the view closes.
    fn on_changes_key(&mut self, v: &mut views::ChangesView, k: &KeyEvent) -> bool {
        let c = match k.code {
            KeyCode::Enter => '\n',
            KeyCode::Char(c) => c,
            KeyCode::Esc => '\x1b',
            _ => '\0',
        };
        if let Some((_, action)) = v.confirm.take() {
            if (c == 'y' || c == '\n')
                && let Some(r) = &v.review {
                    let task = r.task.clone();
                    match action {
                        'm' => self.spawn_bg(move || Bg::Done(tasks::merge(&task), true)),
                        'p' => self.spawn_bg(move || Bg::Done(tasks::pull_request(&task), true)),
                        'd' => {
                            if let Some(ws) = self.snap.workspaces.iter().find(|w| design::path_key(&w.cwd) == design::path_key(&v.dir)).map(|w| w.id) {
                                self.cmd(Command::RemoveWorktree { ws, force: true, delete_branch: true });
                            } else {
                                let d = v.dir.clone();
                                self.spawn_bg(move || Bg::Done(remove_worktree_dir(&d), false));
                            }
                            self.notify("discarding…".into(), false);
                            return false;
                        }
                        _ => {}
                    }
                    self.notify("working on it…".into(), false);
                }
            return true;
        }
        let Some(r) = v.review.as_mut() else { return c != '\x1b' };
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                r.sel = (r.sel + 1).min(r.files.len().saturating_sub(1));
                r.diff = tasks::file_diff(r);
                r.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                r.sel = r.sel.saturating_sub(1);
                r.diff = tasks::file_diff(r);
                r.scroll = 0;
            }
            KeyCode::PageDown | KeyCode::Char(' ') => r.scroll = r.scroll.saturating_add(15),
            KeyCode::PageUp => r.scroll = r.scroll.saturating_sub(15),
            KeyCode::Char('c') => {
                let task = r.task.clone();
                self.spawn_bg(move || Bg::Done(tasks::commit(&task), true));
            }
            KeyCode::Char('e') => {
                if let Some(f) = r.files.get(r.sel) {
                    let p = v.dir.join(&f.path);
                    self.open_in_editor(&p);
                }
            }
            KeyCode::Char('v') => {
                let (dir, branch) = (v.dir.clone(), r.task.branch.clone());
                self.open_pr(dir, branch);
                return true_and_replace();
            }
            KeyCode::Char('m') if v.linked => v.confirm = Some((format!("Merge {} into {}?", r.task.branch, r.task.base), 'm')),
            KeyCode::Char('p') => v.confirm = Some((format!("Push {} and open a pull request?", r.task.branch), 'p')),
            KeyCode::Char('d') if v.linked => v.confirm = Some((format!("Throw away {} (folder and branch)?", r.task.branch), 'd')),
            KeyCode::Char('r') => {
                if let Some(term) = v.term {
                    self.cmd(Command::FocusPane { term });
                    self.mode = Mode::Quick(modal::Quick { text: String::new(), agent: 0, place: modal::Place::Here });
                    return false;
                }
                self.notify("no agent runs in this worktree".into(), true);
            }
            _ => {}
        }
        true
    }

    /// Returns false when the view closes.
    fn on_files_tree_key(&mut self, v: &mut views::FilesTree, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if v.filtering {
            match k.code {
                KeyCode::Esc => {
                    v.filtering = false;
                    v.filter.clear();
                }
                KeyCode::Enter => v.filtering = false,
                KeyCode::Backspace => {
                    v.filter.pop();
                }
                KeyCode::Char(c) if !ctrl => v.filter.push(c),
                KeyCode::Down | KeyCode::Up => {
                    let n = v.visible().len();
                    Self::list_move(&mut v.sel, n, k);
                }
                _ => {}
            }
            v.sel = v.sel.min(v.visible().len().saturating_sub(1));
            v.refresh_preview();
            return true;
        }
        let node = v.selected();
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Char('/') => {
                v.filtering = true;
                v.sel = 0;
            }
            KeyCode::Tab => {
                v.recent = !v.recent;
                v.sel = 0;
                if v.recent && v.recent_list.is_none() {
                    let r = v.root.clone();
                    self.spawn_bg(move || Bg::TreeRecent(r.clone(), files::scan_recent(&r)));
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(n) = node.filter(|n| n.is_dir) {
                    v.expanded.insert(n.path);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(n) = node {
                    if n.is_dir && v.expanded.remove(&n.path) {
                    } else if let Some(parent) = n.path.parent().map(|p| p.to_path_buf()) {
                        v.expanded.remove(&parent);
                        if let Some(i) = v.visible().iter().position(|(_, x)| x.path == parent) {
                            v.sel = i;
                        }
                    }
                }
            }
            KeyCode::Enter => match node {
                Some(n) if n.is_dir => {
                    if !v.expanded.remove(&n.path) {
                        v.expanded.insert(n.path);
                    }
                }
                Some(n) => {
                    if let Some(term) = self.focused() {
                        self.send(ClientMsg::Input { term, data: format!("{} ", files::quote_path(&n.path)).into_bytes() });
                        self.notify(format!("inserted {}", n.path.display()), false);
                    }
                    return false;
                }
                None => {}
            },
            KeyCode::Char('o') => {
                if let Some(n) = node {
                    let _ = files::open_default(&n.path);
                }
            }
            KeyCode::Char('e') => {
                if let Some(n) = node.filter(|n| !n.is_dir) {
                    self.open_in_editor(&n.path);
                    return self.view.is_some() || !matches!(self.cfg.ui.layout.as_str(), "hydra");
                }
            }
            KeyCode::Char('y') => {
                if let Some(n) = node {
                    copy::to_clipboard(&n.path.display().to_string());
                    self.notify(format!("copied {}", n.path.display()), false);
                }
            }
            KeyCode::Char('d') => {
                let root = v.root.clone();
                self.open_changes(root);
                if let (Some(n), Some(View::Changes(c))) = (node, &mut self.view) {
                    c.error = None;
                    let _ = n;
                }
                return true_and_replace();
            }
            KeyCode::Char('j') => {
                let n = v.visible().len();
                v.sel = (v.sel + 1).min(n.saturating_sub(1));
            }
            KeyCode::Char('k') => v.sel = v.sel.saturating_sub(1),
            _ => {
                let n = v.visible().len();
                Self::list_move(&mut v.sel, n, k);
            }
        }
        v.refresh_preview();
        true
    }

    fn on_talk_key(&mut self, term: TermId, mut input: String, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('o') if ctrl || input.is_empty() => {
                self.cmd(Command::FocusPane { term });
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                if !input.is_empty() {
                    let data = std::mem::take(&mut input).into_bytes();
                    self.send(ClientMsg::Input { term, data });
                    let out = self.out.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(60)).await;
                        let _ = out.send(ClientMsg::Input { term, data: b"\r".to_vec() });
                    });
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if !ctrl => input.push(c),
            _ => {}
        }
        self.mode = Mode::Talk { term, input };
    }

    fn on_new_pane_key(&mut self, mut np: NewPane, k: &KeyEvent) {
        let runs = self.new_pane_runs().len();
        if let Some((label, mut text)) = np.input.take() {
            match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    np.input = Some((label, text));
                    self.submit_new_pane(np);
                    return;
                }
                KeyCode::Tab => {
                    text = complete_dir(&text);
                    np.input = Some((label, text));
                }
                KeyCode::Backspace => {
                    text.pop();
                    np.input = Some((label, text));
                }
                KeyCode::Char(c) => {
                    text.push(c);
                    np.input = Some((label, text));
                }
                _ => np.input = Some((label, text)),
            }
            self.mode = Mode::NewPane(np);
            return;
        }
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => np.section = 1 - np.section,
            KeyCode::Down | KeyCode::Char('j') => {
                if np.section == 0 {
                    np.run = (np.run + 1).min(runs - 1);
                    np.place = 0;
                } else {
                    np.place = (np.place + 1).min(self.new_pane_places(np.run).len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if np.section == 0 {
                    np.run = np.run.saturating_sub(1);
                    np.place = 0;
                } else {
                    np.place = np.place.saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                self.submit_new_pane(np);
                return;
            }
            _ => {}
        }
        self.mode = Mode::NewPane(np);
    }

    fn submit_new_pane(&mut self, mut np: NewPane) {
        let runs = self.new_pane_runs();
        let places = self.new_pane_places(np.run);
        let run = runs.get(np.run).cloned().unwrap_or_default();
        let cmd = self
            .cfg
            .quick
            .agents
            .iter()
            .find(|a| a.name == run)
            .map(|a| a.command.replace("{prompt}", "").trim().to_string());
        let place = places.get(np.place).map(|p| p.0.clone()).unwrap_or_default();
        let ws = self.active_ws().map(|w| w.id);
        let here = self.here_dir();
        self.mode = Mode::Normal;
        self.view = None;
        // A new pane is a new full screen of its own; splitting is something you ask for.
        match place.as_str() {
            "new worktree" => {
                if let Some(ws) = ws {
                    let branch = format!("{}-{}", run.to_lowercase(), short_id());
                    self.notify(format!("{run} gets its own worktree: {branch}…"), false);
                    self.cmd(Command::NewWorktree { ws, branch, base: None, cmd, split: None, from: Some(here) });
                }
            }
            "another folder…" => match np.input.take() {
                Some((_, folder)) if !folder.trim().is_empty() => {
                    let path = expand_home(folder.trim());
                    if !path.is_dir() {
                        self.notify(format!("not a folder: {}", path.display()), true);
                        np.input = Some(("folder".into(), folder));
                        self.mode = Mode::NewPane(np);
                        return;
                    }
                    self.cmd(Command::NewWorkspace { cwd: Some(path), name: None, cmd });
                }
                _ => {
                    let start = format!("{}{}", here.display(), std::path::MAIN_SEPARATOR);
                    np.input = Some(("folder".into(), start));
                    self.mode = Mode::NewPane(np);
                }
            },
            _ => self.cmd(Command::NewWorkspace { cwd: Some(here), name: None, cmd }),
        }
    }

    /// Keys in the settings view. Returns false when it closes.
    fn on_settings_view_key(&mut self, v: &mut design::SettingsView, k: &KeyEvent) -> bool {
        use modal::{Cat, Kind};
        let cat = Cat::ALL[v.cat.min(Cat::ALL.len() - 1)];
        let rows = design::settings_rows(self, cat);
        let row = rows.get(v.sel).cloned();
        // Waiting for a key: the new leader, or a new key for a shortcut.
        if v.capturing {
            v.capturing = false;
            if k.code == KeyCode::Esc {
                return true;
            }
            let spec = KeySpec::from_event(k);
            match row {
                Some(design::SRow::Setting(s)) => self.save_setting(s.path, spec.to_config().into()),
                Some(design::SRow::Bind { acts, .. }) if acts.len() == 1 => {
                    let act = &acts[0];
                    let Some(name) = act.to_config() else { return true };
                    let new = spec.to_config();
                    // Unbind the old keys, then bind the new one.
                    let old: Vec<String> = self
                        .keymap
                        .prefixed_order
                        .iter()
                        .filter(|key| self.keymap.prefixed.get(key) == Some(act))
                        .map(|key| key.to_config())
                        .filter(|key| *key != new)
                        .collect();
                    let mut result = Ok(());
                    for o in old {
                        result = result.and_then(|_| modal::write_at(&["keys", "prefix", &o], "none".into()));
                    }
                    result = result.and_then(|_| modal::write_at(&["keys", "prefix", &new], name.into()));
                    match result {
                        Ok(()) => {
                            self.reload_config();
                            self.notify(format!("{} is now {} {}", act.describe(), self.keymap.prefix, spec), false);
                        }
                        Err(e) => self.notify(format!("{e:#}"), true),
                    }
                }
                _ => {}
            }
            return true;
        }
        if let Some(mut text) = v.editing.take() {
            match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Some(design::SRow::Setting(s)) = row {
                        self.save_setting(s.path, text.trim().into());
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                    v.editing = Some(text);
                }
                KeyCode::Char(c) => {
                    text.push(c);
                    v.editing = Some(text);
                }
                _ => v.editing = Some(text),
            }
            return true;
        }
        let step = |app: &mut App, dir: i64| {
            if let Some(design::SRow::Setting(s)) = &row
                && let Some(val) = modal::step(&app.cfg, s, dir)
            {
                app.save_setting(s.path, val);
            }
        };
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Tab => {
                v.cat = (v.cat + 1) % Cat::ALL.len();
                v.sel = 0;
            }
            KeyCode::BackTab => {
                v.cat = (v.cat + Cat::ALL.len() - 1) % Cat::ALL.len();
                v.sel = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => v.sel = (v.sel + 1).min(rows.len().saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => v.sel = v.sel.saturating_sub(1),
            KeyCode::Left | KeyCode::Char('h') => step(self, -1),
            KeyCode::Right | KeyCode::Char('l') => step(self, 1),
            KeyCode::Char('o') => {
                let _ = files::open_default(&crate::config::config_path());
            }
            KeyCode::Enter | KeyCode::Char(' ') => match &row {
                Some(design::SRow::Setting(s)) => match s.kind {
                    Kind::Text => {
                        let cur = modal::current(&self.cfg, s.path).and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default();
                        v.editing = Some(cur);
                    }
                    Kind::Key => v.capturing = true,
                    _ => step(self, 1),
                },
                Some(design::SRow::Bind { acts, .. }) if acts.len() == 1 => v.capturing = true,
                Some(design::SRow::Bind { .. }) => self.notify("that row has several keys: change them in the config file (o)".into(), false),
                Some(design::SRow::Project(p)) => {
                    let p = p.clone();
                    self.hy_forget(&p);
                }
                None => {}
            },
            _ => {}
        }
        true
    }

    /// "claude needs you" / "shop-api · Fix flaky checkout test".
    fn alert_text(&self, term: TermId, status: Status) -> (String, String) {
        let t = self.snap.terms.get(&term);
        let agent = t.and_then(|t| t.agent.clone()).unwrap_or_else(|| "an agent".into());
        let what = if status == Status::Blocked { "needs you" } else { "finished" };
        let place = t
            .and_then(|t| t.top.as_ref().or(Some(&t.cwd)).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let summary = t.map(|t| t.summary.trim().to_string()).unwrap_or_default();
        let body = if summary.is_empty() { place } else { format!("{place} · {summary}") };
        (format!("{agent} {what}"), body)
    }

    /// Clicks on chips and buttons. Returns true if handled.
    fn on_button(&mut self, hit: Hit, double: bool) -> bool {
        match hit {
            Hit::AddPane => self.act(Action::NewPane),
            Hit::SideRow(i) => {
                let rows = self.side_rows();
                match rows.get(i).cloned() {
                    Some(design::SideRow::Group { name, .. }) => {
                        let key = App::group_key(&name);
                        if !self.collapsed.remove(&key) {
                            self.collapsed.insert(key);
                        }
                    }
                    Some(design::SideRow::Pane { ws, .. } | design::SideRow::Detail { ws, .. }) => {
                        if double {
                            let dir = self.snap.workspace(ws).and_then(|w| w.tab()).and_then(|t| self.snap.terms.get(&t.focus)).map(|t| t.cwd.clone());
                            if let Some(dir) = dir {
                                self.open_changes(dir);
                            }
                        } else {
                            self.view = None;
                            self.cmd(Command::SelectWorkspace { ws });
                        }
                    }
                    None => {}
                }
                if matches!(self.mode, Mode::Tree { .. }) {
                    self.mode = Mode::Normal;
                }
            }
            Hit::SideTalk(i) => {
                if let Some(design::SideRow::Pane { ws, .. }) = self.side_rows().get(i).cloned()
                    && let Some(w) = self.snap.workspace(ws)
                {
                    // Talk to the pane's agent (its most urgent one).
                    let mut agents: Vec<&TermInfo> = w
                        .tabs
                        .iter()
                        .flat_map(|t| t.layout.leaves())
                        .filter_map(|id| self.snap.terms.get(&id))
                        .filter(|t| t.agent.is_some())
                        .collect();
                    agents.sort_by_key(|t| t.status.urgency());
                    if let Some(t) = agents.first() {
                        self.mode = Mode::Talk { term: t.id, input: String::new() };
                    }
                }
            }
            Hit::Button(b) => match b {
                Btn::Settings => self.act(Action::Settings),
                Btn::Keys => self.act(Action::Help),
                Btn::Leader => self.mode = Mode::Prefix { since: Instant::now() - Duration::from_secs(5) },
                Btn::Answer(term, c) => self.send(ClientMsg::Input { term, data: c.to_string().into_bytes() }),
                Btn::Reply(term) => {
                    self.cmd(Command::FocusPane { term });
                    self.mode = Mode::Quick(modal::Quick { text: String::new(), agent: 0, place: modal::Place::Here });
                }
                Btn::Zoom(term) => {
                    self.cmd(Command::FocusPane { term });
                    if let Some(t) = self.snap.locate(term).map(|(_, t)| t.id)
                        && !self.zoomed.remove(&t) {
                            self.zoomed.insert(t);
                        }
                }
                Btn::Close(term) => self.cmd(Command::ClosePane { term }),
                Btn::CloseView => self.view = None,
                Btn::ViewKey(c) => {
                    let code = match c {
                        '\n' => KeyCode::Enter,
                        c => KeyCode::Char(c),
                    };
                    self.on_view_key(&KeyEvent::new(code, KeyModifiers::NONE));
                }
                Btn::Row(i) => match &mut self.view {
                    Some(View::Changes(v)) => {
                        let rows = v.rows();
                        if let (Some(views::ChangesRow::File(fi, _)), Some(r)) = (rows.get(i), v.review.as_mut()) {
                            r.sel = *fi;
                            r.diff = tasks::file_diff(r);
                            r.scroll = 0;
                        }
                    }
                    Some(View::Files(v)) => {
                        v.sel = i;
                        v.refresh_preview();
                        if double {
                            self.on_view_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                    Some(View::Map(_)) => {}
                    Some(View::Pr(_)) => {}
                    Some(View::Settings(_)) => {}
                    None => {}
                },
                Btn::SettingsCat(i) => {
                    if let Some(View::Settings(v)) = &mut self.view {
                        v.cat = i;
                        v.sel = 0;
                        v.editing = None;
                        v.capturing = false;
                    }
                }
                Btn::SettingsRow(i) => {
                    if let Some(View::Settings(v)) = &mut self.view {
                        let again = v.sel == i;
                        v.sel = i;
                        if again || double {
                            self.on_view_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                }
                Btn::SettingsAct(i) => {
                    if let Some(View::Settings(v)) = &mut self.view {
                        v.sel = i;
                    }
                    self.on_view_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
                Btn::NewPaneRow(sec, i) => {
                    if let Mode::NewPane(np) = &mut self.mode {
                        let again = np.section == sec && (if sec == 0 { np.run } else { np.place }) == i;
                        np.section = sec;
                        if sec == 0 {
                            np.run = i;
                        } else {
                            np.place = i;
                        }
                        if again || double {
                            let np = np.clone();
                            self.submit_new_pane(np);
                        }
                    }
                }
            },
            _ => return false,
        }
        true
    }

    /// Where the user is: the focused pane's folder, else the workspace's.
    fn here_dir(&self) -> PathBuf {
        self.focused()
            .and_then(|id| self.snap.terms.get(&id))
            .map(|t| t.cwd.clone())
            .filter(|p| p.is_dir())
            .or_else(|| self.active_ws().map(|w| w.cwd.clone()))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn spawn_bg(&self, f: impl FnOnce() -> Bg + Send + 'static) {
        let tx = self.bg.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(f());
        });
    }

    fn on_bg(&mut self, b: Bg) {
        let b = match b {
            Bg::Prs(key, list) => {
                self.hy.prs.insert(key, list);
                self.dirty = true;
                return;
            }
            Bg::Pr(which, info) => {
                if let Some(View::Pr(v)) = &mut self.view
                    && v.which == which
                {
                    v.info = Some(info);
                }
                self.dirty = true;
                return;
            }
            Bg::Tickets(dir, tab, list) => {
                if let Mode::Tickets(v) = &mut self.mode
                    && v.dir == dir
                    && let Some(slot) = v.lists.get_mut(tab)
                {
                    *slot = Some(list);
                }
                self.dirty = true;
                return;
            }
            Bg::RaceStat(id, i, text) => {
                if let Mode::Race(v) = &mut self.mode
                    && v.id == id
                    && let Some(slot) = v.stats.get_mut(i)
                {
                    *slot = Some(text);
                }
                self.dirty = true;
                return;
            }
            Bg::Synced(changed) => {
                if changed {
                    self.reload_config();
                    self.notify("pulled your setup from another machine".into(), false);
                }
                return;
            }
            Bg::PrDiff(which, d) => {
                if let Some(View::Pr(v)) = &mut self.view
                    && v.which == which
                {
                    v.diff = Some(d);
                }
                self.dirty = true;
                return;
            }
            b => b,
        };
        match (b, &mut self.mode) {
            (Bg::Recent(root, list), Mode::Files(v)) if v.root == root => {
                v.recent = Some(list);
                Self::refresh_preview(v);
            }
            (Bg::Project(root, list), Mode::Files(v)) if v.root == root => {
                v.project = Some(list);
                Self::refresh_preview(v);
            }
            (Bg::Review(r), Mode::Tasks { review, .. }) => match r {
                Ok(mut r) => {
                    // Keep the reader's place when reloading the same task.
                    if let Some(old) = review.as_ref().filter(|o| o.task.ws == r.task.ws) {
                        r.sel = old.sel.min(r.files.len().saturating_sub(1));
                        r.diff = tasks::file_diff(&r);
                    }
                    *review = Some(r);
                }
                Err(e) => self.notify(e, true),
            },
            (Bg::Inbox(tab, dir, list), Mode::Inbox(v)) if v.dir == dir => v.lists[tab] = Some(list),
            (Bg::Toolbox(dir, sections), Mode::Toolbox(v)) if v.project == dir => v.sections = Some(sections),
            (Bg::Changes(dir, r), _) => {
                if let Some(View::Changes(v)) = &mut self.view
                    && v.dir == dir
                {
                    match r {
                        Ok(r) => {
                            let keep = v.review.as_ref().map(|o| o.sel);
                            let mut r = *r;
                            if let Some(sel) = keep {
                                r.sel = sel.min(r.files.len().saturating_sub(1));
                                r.diff = tasks::file_diff(&r);
                            }
                            v.review = Some(r);
                        }
                        Err(e) => v.error = Some(e),
                    }
                }
            }
            (Bg::Checks(dir, c), _) => {
                if let Some(View::Changes(v)) = &mut self.view
                    && v.dir == dir
                {
                    v.checks = c;
                }
            }
            (Bg::Tree(root, nodes, git), _) => {
                let working = self
                    .snap
                    .terms
                    .values()
                    .filter(|t| matches!(t.status, Status::Working | Status::Blocked))
                    .find(|t| crate::gitfs::head(&t.cwd).is_some_and(|h| design::path_key(&h.top) == design::path_key(&root)))
                    .and_then(|t| t.agent.clone());
                if let Some(View::Files(v)) = &mut self.view
                    && v.root == root
                {
                    // Open the folders that hold changes; an agent working here is editing them.
                    for rel in git.keys() {
                        let mut p = root.join(rel);
                        while let Some(parent) = p.parent().map(|x| x.to_path_buf()) {
                            if parent == root {
                                break;
                            }
                            v.expanded.insert(parent.clone());
                            p = parent;
                        }
                    }
                    if let Some(a) = working {
                        v.editing = git.keys().map(|k| (k.clone(), a.clone())).collect();
                    }
                    v.all = nodes;
                    v.git = git;
                    v.loading = false;
                    v.refresh_preview();
                }
            }
            (Bg::TreeRecent(root, list), _) => {
                if let Some(View::Files(v)) = &mut self.view
                    && v.root == root
                {
                    v.recent_list = Some(list);
                    v.refresh_preview();
                }
            }
            (Bg::Done(result, reload), _) => {
                match result {
                    Ok(msg) => self.notify(msg, false),
                    Err(e) => self.notify(e, true),
                }
                if let (true, Some(View::Changes(v))) = (reload, &self.view)
                    && let Some(r) = &v.review
                {
                    let (task, d) = (r.task.clone(), v.dir.clone());
                    self.spawn_bg(move || Bg::Changes(d, tasks::load_review(task).map(Box::new)));
                }
                if let (true, Mode::Tasks { review: Some(r), .. }) = (reload, &self.mode) {
                    let task = r.task.clone();
                    self.spawn_bg(move || Bg::Review(tasks::load_review(task).map(Box::new)));
                }
            }
            _ => {} // the panel it was for has closed
        }
    }

    /// Generic list movement shared by the panels. Returns true if the key was handled.
    fn list_move(sel: &mut usize, len: usize, k: &KeyEvent) -> bool {
        let last = len.saturating_sub(1);
        match k.code {
            KeyCode::Down => *sel = (*sel + 1).min(last),
            KeyCode::Up => *sel = sel.saturating_sub(1),
            KeyCode::PageDown => *sel = (*sel + 10).min(last),
            KeyCode::PageUp => *sel = sel.saturating_sub(10),
            KeyCode::Home => *sel = 0,
            KeyCode::End => *sel = last,
            _ => return false,
        }
        true
    }

    fn refresh_preview(v: &mut files::FilesView) {
        let path = v.visible().get(v.sel).map(|e| e.path.clone());
        if v.preview.as_ref().map(|(p, _)| Some(p)) == Some(path.as_ref()) {
            return;
        }
        v.preview = path.map(|p| {
            let lines = files::preview(&p);
            (p, lines)
        });
    }

    fn on_files_key(&mut self, mut v: files::FilesView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let picked = v.visible().get(v.sel).map(|e| e.path.clone());
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                if let (Some(p), Some(term)) = (picked, self.focused()) {
                    let text = format!("{} ", files::quote_path(&p));
                    self.send(ClientMsg::Input { term, data: text.into_bytes() });
                    self.notify(format!("inserted {}", p.display()), false);
                }
                return;
            }
            KeyCode::Char('o') if ctrl => {
                if let Some(p) = picked {
                    match files::open_default(&p) {
                        Ok(()) => self.notify(format!("opened {}", p.display()), false),
                        Err(e) => self.notify(format!("couldn't open: {e}"), true),
                    }
                }
            }
            KeyCode::Char('f') if ctrl => {
                if let Some(p) = picked {
                    let _ = files::reveal(&p);
                }
            }
            KeyCode::Char('y') if ctrl => {
                if let Some(p) = picked {
                    copy::to_clipboard(&p.display().to_string());
                    self.notify(format!("copied {}", p.display()), false);
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                v.tab = 1 - v.tab;
                v.sel = 0;
                if v.tab == 1 && v.project.is_none() {
                    let r = v.root.clone();
                    self.spawn_bg(move || Bg::Project(r.clone(), files::scan_project(&r)));
                }
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.sel = 0;
            }
            _ => {
                let n = v.visible().len();
                Self::list_move(&mut v.sel, n, k);
            }
        }
        Self::refresh_preview(&mut v);
        self.mode = Mode::Files(Box::new(v));
    }

    fn on_tasks_key(&mut self, mut sel: usize, review: Option<Box<tasks::Review>>, k: &KeyEvent) {
        let Some(mut r) = review else {
            let rows = tasks::rows(&self.snap);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.mode = Mode::Normal;
                    return;
                }
                KeyCode::Enter => {
                    if let Some(task) = rows.get(sel).cloned() {
                        if self.cfg.ui.layout == "workspaces" {
                            self.mode = Mode::Normal;
                            self.open_changes(task.dir);
                            return;
                        }
                        self.spawn_bg(move || Bg::Review(tasks::load_review(task).map(Box::new)));
                    }
                }
                KeyCode::Char('o') => {
                    if let Some(t) = rows.get(sel) {
                        self.cmd(Command::SelectWorkspace { ws: t.ws });
                        self.mode = Mode::Normal;
                        return;
                    }
                }
                KeyCode::Char('n') => {
                    self.mode = Mode::Quick(modal::Quick { text: String::new(), agent: 0, place: modal::Place::Worktree });
                    return;
                }
                KeyCode::Char('j') => sel = (sel + 1).min(rows.len().saturating_sub(1)),
                KeyCode::Char('k') => sel = sel.saturating_sub(1),
                _ => {
                    Self::list_move(&mut sel, rows.len(), k);
                }
            }
            self.mode = Mode::Tasks { sel, review: None };
            return;
        };

        if let Some((_, action)) = r.confirm.take() {
            if matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) {
                let task = r.task.clone();
                match action {
                    'm' => self.spawn_bg(move || Bg::Done(tasks::merge(&task), true)),
                    'p' => self.spawn_bg(move || Bg::Done(tasks::pull_request(&task), true)),
                    'x' => {
                        self.cmd(Command::RemoveWorktree { ws: task.ws, force: true, delete_branch: true });
                        self.notify(format!("discarding {}…", task.name), false);
                        self.mode = Mode::Tasks { sel: 0, review: None };
                        return;
                    }
                    _ => {}
                }
                self.notify("working on it…".into(), false);
            }
            self.mode = Mode::Tasks { sel, review: Some(r) };
            return;
        }
        let page = 15u16;
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Tasks { sel, review: None };
                return;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                r.sel = (r.sel + 1).min(r.files.len().saturating_sub(1));
                r.diff = tasks::file_diff(&r);
                r.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                r.sel = r.sel.saturating_sub(1);
                r.diff = tasks::file_diff(&r);
                r.scroll = 0;
            }
            KeyCode::PageDown | KeyCode::Char(' ') => r.scroll = r.scroll.saturating_add(page),
            KeyCode::PageUp => r.scroll = r.scroll.saturating_sub(page),
            KeyCode::Char('c') => {
                let task = r.task.clone();
                self.spawn_bg(move || Bg::Done(tasks::commit(&task), true));
            }
            KeyCode::Char('m') => r.confirm = Some((format!("Merge {} into {}?", r.task.branch, r.task.base), 'm')),
            KeyCode::Char('p') => r.confirm = Some((format!("Push {} and open a pull request?", r.task.branch), 'p')),
            KeyCode::Char('x') => {
                r.confirm = Some((format!("Throw away {} (its folder and branch)?", r.task.name), 'x'))
            }
            KeyCode::Char('o') => {
                self.cmd(Command::SelectWorkspace { ws: r.task.ws });
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('r') => {
                // Reply: focus the task's agent and open the prompt aimed at it.
                if let Some(term) = r.task.agent {
                    self.cmd(Command::FocusPane { term });
                    self.mode = Mode::Quick(modal::Quick { text: String::new(), agent: 0, place: modal::Place::Here });
                } else {
                    self.notify("this task has no agent running".into(), true);
                    self.mode = Mode::Tasks { sel, review: Some(r) };
                }
                return;
            }
            _ => {}
        }
        self.mode = Mode::Tasks { sel, review: Some(r) };
    }

    fn on_inbox_key(&mut self, mut v: inbox::InboxView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let item = v.visible().get(v.sel).map(|i| (*i).clone());
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                if let Some(i) = item {
                    inbox::open_url(&i.url);
                    self.notify(format!("opened {} in the browser", i.key), false);
                }
            }
            KeyCode::Char('t') if ctrl => {
                let Some(i) = item else {
                    self.mode = Mode::Inbox(Box::new(v));
                    return;
                };
                let text = inbox::task_prompt(v.tab, &i);
                if let (0, Some(branch)) = (v.tab, i.branch.clone()) {
                    // A PR: check its branch out as a task with the agent already briefed.
                    let ws = self.active_ws().map(|w| w.id);
                    let agent = self.cfg.quick.agents.first().cloned();
                    if let (Some(ws), Some(agent)) = (ws, agent) {
                        let cmd = agent.command.replace("{prompt}", &self.cfg.quote_for_shell(&text));
                        let dir = v.dir.clone();
                        let out = self.out.clone();
                        self.spawn_bg(move || {
                            let fetched = std::process::Command::new("git").arg("-C").arg(&dir).args(["fetch", "origin", &branch]).output();
                            if !fetched.is_ok_and(|o| o.status.success()) {
                                return Bg::Done(Err(format!("couldn't fetch {branch}")), false);
                            }
                            let _ = out.send(ClientMsg::Command(Command::NewWorktree { ws, branch: branch.clone(), base: None, cmd: Some(cmd), split: None, from: None }));
                            Bg::Done(Ok(format!("checking out {branch} as a task…")), false)
                        });
                        self.mode = Mode::Normal;
                        return;
                    }
                }
                self.mode = Mode::Quick(modal::Quick { text, agent: 0, place: modal::Place::Worktree });
                return;
            }
            KeyCode::Char('r') if ctrl => {
                v.lists[v.tab] = None;
                let (tab, d) = (v.tab, v.dir.clone());
                self.spawn_bg(move || Bg::Inbox(tab, d.clone(), inbox::load(tab, &d)));
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let n = inbox::TABS.len();
                v.tab = if k.code == KeyCode::Tab { (v.tab + 1) % n } else { (v.tab + n - 1) % n };
                v.sel = 0;
                v.scroll = 0;
                if v.lists[v.tab].is_none() {
                    let (tab, d) = (v.tab, v.dir.clone());
                    self.spawn_bg(move || Bg::Inbox(tab, d.clone(), inbox::load(tab, &d)));
                }
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.sel = 0;
            }
            _ => {
                let n = v.visible().len();
                if Self::list_move(&mut v.sel, n, k) {
                    v.scroll = 0;
                }
            }
        }
        self.mode = Mode::Inbox(Box::new(v));
    }

    fn on_toolbox_key(&mut self, mut v: toolbox::ToolboxView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                if let Some(item) = v.selected() {
                    match files::open_default(&item.source) {
                        Ok(()) => self.notify(format!("opened {}", item.source.display()), false),
                        Err(e) => self.notify(format!("couldn't open: {e}"), true),
                    }
                }
            }
            KeyCode::Char('r') if ctrl => {
                v.sections = None;
                let d = v.project.clone();
                self.spawn_bg(move || Bg::Toolbox(d.clone(), toolbox::scan(&d)));
            }
            KeyCode::Backspace => {
                v.query.pop();
                v.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                v.query.push(c);
                v.sel = 0;
            }
            _ => {
                let n = v.item_count();
                if Self::list_move(&mut v.sel, n, k) {
                    v.scroll = 0;
                }
            }
        }
        self.mode = Mode::Toolbox(Box::new(v));
    }

    fn on_quick_key(&mut self, mut q: modal::Quick, k: &KeyEvent) {
        let agents = self.cfg.quick.agents.len().max(1);
        let newline = k.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter if newline => q.text.push('\n'),
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                self.submit_quick(q);
                return;
            }
            KeyCode::Tab => q.agent = (q.agent + 1) % agents,
            KeyCode::BackTab => q.place = q.place.next(),
            KeyCode::Backspace => {
                q.text.pop();
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => q.text.clear(),
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => q.text.push(c),
            _ => {}
        }
        self.mode = Mode::Quick(q);
    }

    fn submit_quick(&mut self, q: modal::Quick) {
        let task = q.text.trim().to_string();
        if q.place == modal::Place::Here {
            let Some(term) = self.focused() else { return };
            if task.is_empty() {
                return;
            }
            let body = task.replace('\n', "\r");
            let data = if task.contains('\n') { format!("\x1b[200~{body}\x1b[201~") } else { body };
            self.send(ClientMsg::Input { term, data: data.into_bytes() });
            // Enter as a separate write, a beat later, so the agent reads the text first.
            let out = self.out.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                let _ = out.send(ClientMsg::Input { term, data: b"\r".to_vec() });
            });
            return;
        }
        let Some(agent) = self.cfg.quick.agents.get(q.agent).cloned() else {
            self.notify("no agents configured under [quick]".into(), true);
            return;
        };
        let prompt = if task.is_empty() { String::new() } else { self.cfg.quote_for_shell(&task) };
        let cmd = agent.command.replace("{prompt}", &prompt).trim().to_string();
        let Some(ws) = self.active_ws().map(|w| w.id) else { return };
        match q.place {
            modal::Place::Right | modal::Place::Down => {
                let dir = if q.place == modal::Place::Right { Dir::Right } else { Dir::Down };
                self.split(dir, Some(cmd));
            }
            modal::Place::Tab => self.cmd(Command::NewTab { ws, name: None, cmd: Some(cmd) }),
            modal::Place::Worktree => {
                if crate::gitfs::head(&self.here_dir()).is_none() {
                    // Not in a repo: no worktree to make; open beside instead.
                    self.split(Dir::Right, Some(cmd));
                    return;
                }
                let branch = modal::branch_for(&task);
                self.notify(format!("creating worktree {branch}…"), false);
                let (split, from) = (self.focused(), Some(self.here_dir()));
                self.cmd(Command::NewWorktree { ws, branch, base: None, cmd: Some(cmd), split, from });
            }
            modal::Place::Here => {}
        }
    }

    fn on_settings_key(&mut self, mut s: modal::Settings, k: &KeyEvent) {
        let n = modal::SETTINGS.len();
        let setting = &modal::SETTINGS[s.sel.min(n - 1)];
        if s.capturing {
            if k.code != KeyCode::Esc {
                let spec = KeySpec::from_event(k);
                self.save_setting(setting.path, spec.to_config().into());
            }
            s.capturing = false;
            self.mode = Mode::Settings(s);
            return;
        }
        if let Some(mut text) = s.editing.take() {
            match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.save_setting(setting.path, text.trim().into()),
                KeyCode::Backspace => {
                    text.pop();
                    s.editing = Some(text);
                }
                KeyCode::Char(c) => {
                    text.push(c);
                    s.editing = Some(text);
                }
                _ => s.editing = Some(text),
            }
            self.mode = Mode::Settings(s);
            return;
        }
        let change = |app: &mut App, dir: i64| {
            if let Some(v) = modal::step(&app.cfg, setting, dir) {
                app.save_setting(setting.path, v);
            }
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => s.sel = (s.sel + 1) % n,
            KeyCode::Up | KeyCode::Char('k') => s.sel = (s.sel + n - 1) % n,
            KeyCode::Left | KeyCode::Char('h') => change(self, -1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => change(self, 1),
            KeyCode::Enter => match setting.kind {
                modal::Kind::Text => {
                    let cur = modal::current(&self.cfg, setting.path)
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    s.editing = Some(cur);
                }
                modal::Kind::Key => s.capturing = true,
                _ => change(self, 1),
            },
            _ => {}
        }
        self.mode = Mode::Settings(s);
    }

    fn save_setting(&mut self, path: &str, value: toml_edit::Value) {
        match modal::write(path, value) {
            Ok(()) => self.reload_config(),
            Err(e) => self.notify(format!("{e:#}"), true),
        }
    }

    fn pick_items(&self, query: &str, commands: bool) -> Vec<PickItem> {
        let q = query.to_lowercase();
        let command_items: Vec<PickItem> = if commands {
            self.palette_commands()
                .into_iter()
                .map(|a| PickItem {
                    label: a.describe(),
                    detail: String::new(),
                    status: Status::None,
                    key: self.key_for(&a),
                    target: PickTarget::Command(a),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut agents: Vec<PickItem> = self
            .snap
            .terms
            .values()
            .filter(|t| t.agent.is_some())
            .map(|t| PickItem {
                label: t.display_name().to_string(),
                detail: self.describe_term(t.id),
                status: t.status,
                key: String::new(),
                target: PickTarget::Pane(t.id),
            })
            .collect();
        agents.sort_by_key(|i| i.status.urgency());
        let workspaces = self.snap.workspaces.iter().map(|w| PickItem {
            label: w.name.clone(),
            detail: match &w.git {
                Some(g) => format!("workspace · {} · {}", g.branch, w.cwd.display()),
                None => format!("workspace · {}", w.cwd.display()),
            },
            status: Status::None,
            key: String::new(),
            target: PickTarget::Workspace(w.id),
        });
        let shells = self.snap.terms.values().filter(|t| t.agent.is_none()).map(|t| PickItem {
            label: t.display_name().to_string(),
            detail: self.describe_term(t.id),
            status: Status::None,
            key: String::new(),
            target: PickTarget::Pane(t.id),
        });
        command_items
            .into_iter()
            .chain(agents)
            .chain(workspaces)
            .chain(shells)
            .filter(|i| q.is_empty() || i.label.to_lowercase().contains(&q) || i.detail.to_lowercase().contains(&q))
            .collect()
    }
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a == b
}

/// Complete a directory path to the longest unambiguous prefix (Tab in the new-workspace prompt).
fn complete_dir(input: &str) -> String {
    let expanded = expand_home(input);
    let typed = expanded.to_string_lossy().into_owned();
    let ends_sep = typed.ends_with(['/', '\\']);
    let (dir, prefix) = if ends_sep || typed.is_empty() {
        (PathBuf::from(if typed.is_empty() { "." } else { &typed }), String::new())
    } else {
        let p = PathBuf::from(&typed);
        let parent = p.parent().map(|x| x.to_path_buf()).filter(|x| !x.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from("."));
        (parent, p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    };
    let Ok(entries) = std::fs::read_dir(&dir) else { return input.to_string() };
    let lower = prefix.to_lowercase();
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().starts_with(&lower) && (prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort();
    let Some(first) = names.first() else { return input.to_string() };
    // Longest common prefix (case-insensitive, keeping the first match's spelling).
    let mut common = first.chars().count();
    for n in &names[1..] {
        common = common.min(first.chars().zip(n.chars()).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count());
    }
    let stem: String = first.chars().take(common).collect();
    let sep = std::path::MAIN_SEPARATOR;
    let base = if ends_sep || typed.is_empty() {
        typed.clone()
    } else {
        let cut = typed.len() - prefix.len();
        typed[..cut].to_string()
    };
    let mut out = format!("{base}{stem}");
    if names.len() == 1 {
        out.push(sep);
    }
    out
}

fn expand_home(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(d) = directories::BaseDirs::new() {
            return d.home_dir().join(rest.trim_start_matches(['/', '\\']));
        }
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn blend_mixes_rgb() {
        let c = render::blend(Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 50), 0.1);
        assert_eq!(c, Color::Rgb(20, 10, 5));
        assert_eq!(render::blend(Color::Indexed(4), Color::Rgb(1, 2, 3), 0.1), Color::Indexed(4));
    }

    #[test]
    fn completes_directories() {
        let root = std::env::temp_dir().join(format!("hydra-complete-{}", std::process::id()));
        for d in ["alpha-one", "alpha-two", "beta"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let sep = std::path::MAIN_SEPARATOR;
        let base = format!("{}{sep}", root.display());
        assert_eq!(complete_dir(&format!("{base}be")), format!("{base}beta{sep}"));
        assert_eq!(complete_dir(&format!("{base}al")), format!("{base}alpha-"));
        assert_eq!(complete_dir(&format!("{base}zz")), format!("{base}zz"));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::layout::Node;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Panes are washed with their workspace's colour; programs' own colours survive.
    #[test]
    fn panes_are_tinted_with_the_workspace_colour() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (bg_tx, _bg_rx) = mpsc::unbounded_channel();
        let mut cfg = Config::default();
        // The tint belongs to the classic layouts; the design keeps panes untinted.
        cfg.ui.layout = "tree".into();
        cfg.ui.workspace_tint = 0.10;
        let mut app = App::new(cfg, tx, None, bg_tx);
        app.splash = false;
        app.snap.workspaces.push(WorkspaceInfo {
            id: 1,
            name: "api".into(),
            cwd: PathBuf::from("."),
            tabs: vec![TabInfo { id: 2, name: String::new(), layout: Node::Leaf(3), focus: 3 }],
            active_tab: 2,
            git: None,
            worktree: false,
            color: 2,
            is_new: false,
            group: None,
        });
        app.snap.active_ws = Some(1);
        let mut p = vt100::Parser::new(10, 40, 0);
        p.process(b"plain \x1b[44mblue-bg\x1b[0m");
        app.parsers.insert(3, p);

        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let buf = term.backend().buffer();
        let (_, inner) = app.panes[0];
        let ws_color = crate::theme::parse_color(&app.cfg.ui.workspace_colors[2]).unwrap();
        let tint = render::blend(app.theme.bg, ws_color, app.cfg.ui.workspace_tint);
        assert_eq!(buf[(inner.x, inner.y)].bg, tint, "default background takes the tint");
        assert_eq!(buf[(inner.x + 6, inner.y)].bg, ratatui::style::Color::Indexed(4), "program backgrounds are left alone");
        assert_eq!(buf[(inner.x + 20, inner.y + 5)].bg, tint, "empty cells are tinted too");
        assert_eq!(buf[(inner.x - 1, inner.y)].fg, ws_color, "focused border is the workspace colour");
    }
}

#[cfg(test)]
mod design_tests {
    use super::*;
    use crate::layout::{Dir, Node};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn entry(path: &str, branch: &str, main: bool) -> WorktreeEntry {
        WorktreeEntry { path: PathBuf::from(path), branch: branch.into(), main, repo: "shop-api".into() }
    }

    fn term(id: TermId, agent: Option<&str>, status: Status, cwd: &str) -> TermInfo {
        TermInfo {
            id,
            cols: 80,
            rows: 20,
            title: String::new(),
            process: if agent.is_some() { "node".into() } else { "pwsh".into() },
            agent: agent.map(String::from),
            status,
            cwd: PathBuf::from(cwd),
            summary: String::new(),
            said: String::new(),
            branch: None,
            linked: false,
            subagents: Vec::new(),
            root: None,
            top: None,
            since: 0,
            asleep: false,
        }
    }

    /// The design's main screen, rendered as text.
    pub(super) fn render(w: u16, h: u16) -> (String, App) {
        render_with("workspaces", w, h)
    }

    pub(super) fn render_with(which: &str, w: u16, h: u16) -> (String, App) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (bg_tx, _bg_rx) = mpsc::unbounded_channel();
        let mut cfg = Config::default();
        cfg.ui.layout = which.into();
        let mut app = App::new(cfg, tx, None, bg_tx);
        app.splash = false;
        let mut layout = Node::Leaf(1);
        layout.split(1, Dir::Right, 2);
        layout.split(2, Dir::Down, 3);
        let root = r"C:\code\shop-api";
        app.snap.workspaces.push(WorkspaceInfo {
            id: 10,
            name: "shop-api".into(),
            cwd: PathBuf::from(root),
            tabs: vec![TabInfo { id: 11, name: String::new(), layout, focus: 1 }],
            active_tab: 11,
            git: Some(GitInfo {
                repo: "shop-api".into(),
                branch: "main".into(),
                dirty: 2,
                linked: false,
                root: PathBuf::from(root),
                worktrees: vec![
                    entry(root, "main", true),
                    entry(r"C:\code\shop-api\.wt\rate", "rate-limit", false),
                    entry(r"C:\code\shop-api\.wt\orders", "orders-migration", false),
                ],
                ahead: 0,
            }),
            worktree: false,
            color: 0,
            is_new: false,
            group: None,
        });
        app.snap.active_ws = Some(10);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let rate = r"C:\code\shop-api\.wt\rate";
        let mut claude = term(1, Some("claude"), Status::Blocked, root);
        claude.branch = Some("main".into());
        claude.subagents = vec!["Explore".into()];
        claude.summary = "Fix flaky checkout test".into();
        claude.root = Some(PathBuf::from(root));
        claude.top = Some(PathBuf::from(root));
        claude.since = now - 180;
        let mut codex = term(2, Some("codex"), Status::Working, rate);
        codex.branch = Some("rate-limit".into());
        codex.linked = true;
        codex.summary = "Rate limit /login".into();
        codex.root = Some(PathBuf::from(root));
        codex.top = Some(PathBuf::from(rate));
        codex.since = now - 120;
        let mut shell = term(3, None, Status::None, root);
        shell.root = Some(PathBuf::from(root));
        shell.top = Some(PathBuf::from(root));
        shell.branch = Some("main".into());
        app.snap.terms.insert(1, claude);
        app.snap.terms.insert(2, codex);
        app.snap.terms.insert(3, shell);
        if which == "hydra" {
            app.hy_sync();
        }
        let mut p = vt100::Parser::new(20, 80, 0);
        p.process(b"> fix the flaky checkout test\r\n\r\nRun npm test -- checkout?\r\n\x1b[1m\xe2\x9d\xaf 1. Yes\x1b[0m\r\n  2. Yes, and always allow\r\n  3. No");
        app.parsers.insert(1, p);
        app.parsers.insert(2, vt100::Parser::new(20, 80, 0));
        app.parsers.insert(3, vt100::Parser::new(20, 80, 0));
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            let mut line = String::new();
            for x in 0..w {
                line.push_str(buf[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        (out, app)
    }

    #[test]
    fn main_screen_matches_the_design() {
        let (text, _app) = render(160, 45);
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{text}");
        }
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with(" >_ hydra"), "top bar: {}", lines[0]);
        assert!(lines[0].contains("▌shop-api  ›  claude"), "crumb: {}", lines[0]);
        assert!(lines[0].trim_end().ends_with("+ Pane n"), "+ Pane button: {}", lines[0]);
        assert!(!text.contains("GROUPS") && !text.contains("WORKSPACES"), "no heading");
        assert!(text.contains("▌● shop-api"), "the pane, named, with its most urgent agent");
        assert!(text.contains("claude +1"), "and how many agents it holds");
        assert!(text.contains("Run npm test -- checkout?"), "the question under it");
        assert!(text.contains("↳ Explore"), "its subagent");
        assert!(text.contains("+ Pane"), "a way to start a new pane");
        assert!(lines[1].contains(" 1 ● claude"), "the tab strip: {}", lines[1]);
        assert!(text.contains("● answer   Yes 1   Always 2   No 3   Reply r"), "answer bar");
        assert!(text.contains("codex  rate-limit"), "pane title shows the worktree");
        assert!(lines[44].contains("1 need you") && lines[44].contains("1 working"), "status: {}", lines[44]);
        assert!(lines[44].contains("1 pane"));
        assert!(lines[44].contains("Keys ?") && lines[44].contains("Ctrl+Space"));
    }

    #[test]
    fn narrow_screen_drops_the_agent_from_the_crumb() {
        let (text, _) = render(100, 30);
        let first = text.lines().next().unwrap();
        assert!(first.contains("▌shop-api") && !first.contains("›  claude"), "{first}");
        // 26-column sidebar: no room for the question line.
        assert!(!text.contains("Run npm test -- checkout?\n") || text.lines().any(|l| l.starts_with("  ├")));
    }
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render::draw(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string() + "\n").collect()
    }

    #[test]
    fn keys_and_new_pane_are_centered_modals() {
        let (_, mut app) = super::design_tests::render(160, 45);
        app.mode = Mode::Help { scroll: 0 };
        let keys = draw(&mut app, 160, 45);
        app.mode = Mode::NewPane(NewPane { section: 0, run: 0, place: 0, input: None });
        let pane = draw(&mut app, 160, 45);
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{keys}\n{pane}");
        }
        for cat in ["AGENTS", "PANES & TABS", "SPLITS", "CODE", "HYDRA"] {
            assert!(keys.contains(cat), "missing category {cat}");
        }
        let focus = keys.lines().find(|l| l.contains("move focus")).unwrap();
        assert!(focus.contains(" ← ") && focus.contains(" → "), "a keycap per arrow: {focus}");
        assert!(keys.contains(" 1   2   3  answer its prompt"), "a keycap per answer key");
        assert!(pane.contains("New pane") && pane.contains("RUN") && pane.contains("WHERE"));
        // Keys are drawn as caps: the key sits on the button ground, not the card's.
        let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
        app.mode = Mode::Help { scroll: 0 };
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let buf = term.backend().buffer();
        let (y, line) = keys.lines().enumerate().find(|(_, l)| l.contains("give an agent a task")).unwrap();
        let x = line.chars().take_while(|c| *c != 'q').count() as u16;
        assert_eq!(buf[(x, y as u16)].bg, app.theme.btn, "the q key is a keycap");
        // Centered: the window's title row starts well away from the left edge.
        let title = pane.lines().find(|l| l.contains("New pane")).unwrap();
        let x = title.find("New pane").unwrap();
        assert!(x > 40 && x < 90, "title at column {x}");
    }
}

#[cfg(test)]
mod settings_splash_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &mut App) -> String {
        let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
        term.draw(|f| render::draw(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..45).map(|y| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string() + "\n").collect()
    }

    #[test]
    fn splash_and_settings_render() {
        let (_, mut app) = super::design_tests::render(160, 45);
        app.splash = true;
        let splash = draw(&mut app);
        app.splash = false;
        app.view = Some(View::Settings(Box::new(design::SettingsView { cat: 0, sel: 1, editing: None, capturing: false, scroll: 0 })));
        let settings = draw(&mut app);
        app.view = Some(View::Settings(Box::new(design::SettingsView { cat: 2, sel: 0, editing: None, capturing: false, scroll: 0 })));
        let keys = draw(&mut app);
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{splash}\n{settings}\n{keys}");
        }
        assert!(splash.contains("██████") && splash.contains("many heads, one body"));
        assert!(splash.contains("Jump to what needs you") && splash.contains("Open shop-api"));
        for page in ["General", "Sessions", "Appearance", "Agents", "Projects", "Keys"] {
            assert!(settings.contains(page), "page {page}");
        }
        assert!(settings.contains("Leader key") && settings.contains("Splash screen") && settings.contains("Press it, let go"));
        assert!(keys.contains("Theme") && keys.contains("Changes the whole app live"));
    }
}

#[cfg(test)]
mod hydra_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render::draw(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string() + "\n").collect()
    }

    fn show(s: &str) {
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{s}");
        }
    }

    #[test]
    fn main_screen_follows_the_handoff() {
        let (text, mut app) = super::design_tests::render_with("hydra", 160, 45);
        show(&text);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with(" >_ hydra"), "top bar: {}", lines[0]);
        assert!(lines[0].contains("shop-api  ›  ⎇ main  ›  claude  ● needs you  ·  Fix flaky checkout test"), "crumb: {}", lines[0]);
        assert!(lines[2].contains("+ New n") && lines[2].contains("Jump ●1"), "+ New and Jump at the top of the sidebar: {}", lines[2]);
        // One tree: project → BRANCHES (what runs in the repo folder) → WORKTREES.
        assert!(text.contains("▾ ▌shop-api") && text.contains("BRANCHES") && text.contains("⎇ main") && text.contains("WORKTREES"));
        assert!(!text.contains("main folder"), "no 'main folder' wording");
        assert!(text.contains("+ open a project"));
        let branches = text.find("BRANCHES").unwrap();
        let worktrees = text.find("WORKTREES").unwrap();
        assert!(branches < worktrees, "branches first");
        // Rows: agent and state, then what it's on (or its question) underneath.
        assert!(text.contains("● claude  needs you") && text.contains("3m"));
        assert!(text.contains("Run npm test -- checkout?"), "the question under the agent");
        assert!(text.contains("↳ Explore"), "subagents under their agent");
        assert!(text.contains("⑂ rate · rate-limit") && text.contains("⠋ codex  working") && text.contains("Rate limit /login"));
        assert!(text.contains("⑂ orders") && text.contains("nothing running"), "worktrees with nothing running");
        assert!(text.contains("› shell  shop-api") && !text.contains("shell 2"), "shells: where they are, no age");
        assert!(!text.contains("session"), "no 'session' wording on screen");
        assert!(text.contains("● claude is waiting") && text.contains(" Yes 1 ") && text.contains(" Always 2 ") && text.contains(" No 3 "));
        assert!(!text.contains("click or press T"), "no footer under the pane");
        assert!(lines[2].contains("> fix the flaky checkout test"), "one pane: no title bar of its own, a row of air under the top bar");
        assert!(lines[44].contains("● 1 need you") && !lines[44].contains("across"), "status: {}", lines[44]);
        // Overlays are centred over a dimmed screen.
        for (mode, needle) in [
            (Mode::Jump { sel: 0 }, "NEEDS YOU"),
            (Mode::HyPane(hydra::NewPaneHy::new(0, false)), "claude gets its own new worktree in shop-api"),
            (Mode::HyPane(hydra::NewPaneHy { place: Some(1), ..hydra::NewPaneHy::new(0, false) }), "Switches shop-api to a new branch"),
            (Mode::Help { scroll: 0 }, "PANES & CODE"),
            (Mode::Talk { term: 1, input: String::new() }, "Message claude"),
        ] {
            app.mode = mode;
            let o = draw(&mut app, 160, 45);
            show(&o);
            assert!(o.contains(needle), "overlay shows {needle}");
        }
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 1, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" General ") && o.contains(" Sessions ") && o.contains("Sort sidebar by attention") && o.contains(" on "));
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 2, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" Default ") && o.contains(" Tokyo Night") && o.contains("needs you"));
        // The splash: the braille hydra, the wordmark, what happened, buttons.
        app.mode = Mode::Normal;
        app.splash = true;
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("⣿") && o.contains("██████"), "art and wordmark");
        assert!(o.contains("while you were away") && o.contains("1 need you") && o.contains("Open shop-api Enter"));
        // Small windows drop the art but keep the rest.
        let o = draw(&mut app, 100, 30);
        assert!(!o.contains("⣿") && o.contains("Jump to what needs you"));
    }

    #[test]
    fn pull_request_view() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        let info = pr::parse_info(
            r#"{"number":412,"title":"Rate limit /login","url":"u","state":"OPEN","author":{"login":"cody"},"headRefName":"rate-limit","baseRefName":"main",
            "body":"Token bucket, 5 a minute.","additions":42,"deletions":7,"changedFiles":3,
            "statusCheckRollup":[{"conclusion":"SUCCESS","name":"lint"},{"conclusion":"FAILURE","name":"test"}],
            "reviews":[{"author":{"login":"sam"},"state":"CHANGES_REQUESTED","body":"use X-Forwarded-For"}],"comments":[]}"#,
        )
        .unwrap();
        app.view = Some(View::Pr(Box::new(pr::PrView { dir: PathBuf::from("."), which: "412".into(), info: Some(Ok(info)), diff: None, tab: 0, scroll: 0 })));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("#412  Rate limit /login") && o.contains("rate-limit → main") && o.contains("+42 −7"));
        assert!(o.contains("CHECKS  1 failing") && o.contains("✕ test") && o.contains("sam asked for changes"));
        assert!(o.contains("Ask the agent to fix it f") && o.contains("Open in browser o"));
        // The sidebar stays; the view takes the main area.
        assert!(o.contains("⎇ main"));
        // And a PR tag on the branch that has one.
        app.view = None;
        app.hy.prs.insert(
            app.hy_model()[0].key.clone(),
            vec![pr::PrBrief { number: 412, title: "Rate limit".into(), branch: "rate-limit".into(), checks: pr::Checks::Fail, review: pr::Review::Changes, url: String::new() }],
        );
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("#412 ✕±"), "PR tag on the worktree row");
        app.mode = Mode::Jump { sel: 0 };
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("PULL REQUESTS") && o.contains("checks failing"), "failing PRs in Jump");
    }

    #[test]
    fn ship_confirm_says_what_happens() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        let task = tasks::TaskRow {
            ws: 10,
            name: "rate-limit".into(),
            branch: "rate-limit".into(),
            base: "main".into(),
            stage: tasks::Stage::Ready,
            summary: "Rate limit /login".into(),
            dirty: 0,
            ahead: 0,
            agent: Some(2),
            dir: PathBuf::from("."),
            root: PathBuf::from("."),
        };
        app.mode = Mode::Ship(Box::new(ShipAsk { task: task.clone(), changed: 3, pr: None }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Ship rate-limit") && o.contains("commit 3 changed files as \"Rate limit /login\""));
        assert!(o.contains("push rate-limit") && o.contains("open a pull request into main") && o.contains("Ship Enter"));
        app.mode = Mode::Ship(Box::new(ShipAsk { task, changed: 0, pr: Some("412".into()) }));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("nothing new to commit") && o.contains("update pull request #412"));
    }

    #[test]
    fn ideas_tickets_and_races() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        let root = app.hy_model()[0].path.clone();
        let ideas = vec![
            work::Idea { text: "dark mode for the dashboard".into(), project: Some(root.clone()), at: 0 },
            work::Idea { text: "a CLI for exports".into(), project: None, at: 0 },
        ];
        app.mode = Mode::Ideas(Box::new(work::IdeasView { ideas, input: String::new(), sel: 0, tag: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Ideas") && o.contains("for ▌shop-api") && o.contains("SHOP-API") && o.contains("✦ dark mode for the dashboard"));
        assert!(o.contains("ANY PROJECT") && o.contains("start claude on it"));

        let t = work::Ticket { key: "ENG-123".into(), title: "Checkout fails on Safari".into(), url: "u".into(), state: "Todo".into(), meta: "High · ENG".into(), body: "Steps to reproduce".into() };
        app.mode = Mode::Tickets(Box::new(work::TicketsView {
            dir: root.clone(),
            tabs: vec![("github".into(), "GitHub issues".into()), ("linear".into(), "Linear".into()), ("plane".into(), "Plane".into())],
            tab: 1,
            lists: vec![None, Some(Ok(vec![t])), Some(Err("Set PLANE_API_KEY".into()))],
            query: String::new(),
            sel: 0,
        }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" GitHub issues ") && o.contains(" Linear ") && o.contains(" Plane"));
        assert!(o.contains("ENG-123") && o.contains("Checkout fails on Safari") && o.contains("Todo · High · ENG") && o.contains("claude on it, own worktree"));

        app.mode = Mode::RaceNew(Box::new(work::RaceNew { text: "add rate limiting".into(), picked: vec![true, true, false], row: 0, cur: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Race agents") && o.contains("✓ claude") && o.contains("✓ codex") && o.contains("2 agents, each in its own worktree of shop-api"));

        app.hy.saved.races.push(work::Race {
            id: 7,
            project: root.clone(),
            prompt: "add rate limiting".into(),
            base: "main".into(),
            entries: vec![("claude".into(), "race-add-rate-limiting-claude".into()), ("codex".into(), "rate-limit".into())],
        });
        app.mode = Mode::Normal;
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("⚑ race add rate limiting") && o.contains("⚑ rate"), "race line and race worktree in the sidebar");
        app.mode = Mode::Race(Box::new(work::RaceView { id: 7, sel: 1, stats: vec![None, Some("+42 −7 · 3 files".into())], confirm: true }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Race · add rate limiting") && o.contains("+42 −7 · 3 files") && o.contains("not running"));
        assert!(o.contains("Keep codex's rate-limit and delete the other 1?"));
    }

    #[test]
    fn map_view() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        app.view = Some(View::Map(Box::new(hydra::MapView { proj: None, sel: 1 })));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("map  shop-api") && o.contains("▌shop-api  ●1 ⠋1"), "the project at the top");
        assert!(o.contains("⎇ main") && o.contains("⑂ rate") && o.contains("⑂ orders"), "a box per folder");
        assert!(o.contains("● claude") && o.contains("needs you 3m") && o.contains("⠋ codex") && o.contains("working 2m"));
        assert!(o.contains("┴") && (o.contains("┬") || o.contains("┼")), "boxes hang off the project");
    }

    #[test]
    fn splash_menus_and_resizing() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        // The splash waits for a button: other keys do nothing.
        app.splash = true;
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Esc));
        assert!(app.splash, "random keys don't leave the splash");
        app.on_key(key(KeyCode::Right));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("←→ choose   Enter open"));
        app.on_key(key(KeyCode::Enter));
        assert!(!app.splash && matches!(app.mode, Mode::Jump { .. }), "Enter picks the selected button (Jump)");
        app.mode = Mode::Normal;

        // Right-click menus.
        app.menu_for_session(1, (10, 10));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Message claude…") && o.contains("Interrupt (Ctrl+C)") && o.contains("End claude"));
        let Mode::HyMenu(m) = &app.mode else { panic!("a menu") };
        let talk = m.items.iter().position(|(l, _)| l.starts_with("Message")).unwrap();
        app.menu_pick(talk);
        assert!(matches!(app.mode, Mode::Talk { term: 1, .. }), "picking an item does it");
        app.mode = Mode::Normal;
        app.menu_for_project(0, (5, 5));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("+ New here…") && o.contains("End everything here (3)"));
        app.mode = Mode::Normal;
        let wt = app.hy_model()[0].wts.iter().find(|w| !w.main).unwrap().key.clone();
        app.menu_for_place(wt, (5, 5));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("+ claude here") && o.contains("Remove worktree (branch kept)"));
        app.mode = Mode::Normal;

        // Resizing the sidebar, within its limits.
        let before = app.hy.side_rect.width;
        app.act(Action::Resize(crate::layout::Dir::Right));
        draw(&mut app, 160, 45);
        assert!(app.hy.side_rect.width > before, "wider");
        for _ in 0..40 {
            app.act(Action::Resize(crate::layout::Dir::Right));
        }
        assert_eq!(app.hy.saved.side_w, Some(hydra::SIDE_MAX), "but not past the max");
        for _ in 0..40 {
            app.act(Action::Resize(crate::layout::Dir::Left));
        }
        assert_eq!(app.hy.saved.side_w, Some(hydra::SIDE_MIN), "nor under the min");
    }

    #[test]
    fn bright_black_backgrounds_are_a_quiet_panel() {
        let (_, mut app) = super::design_tests::render_with("hydra", 160, 45);
        // Claude draws pasted text and its diff panel on palette colour 8.
        let mut p = vt100::Parser::new(20, 80, 0);
        p.process(b"[100mpasted text[0m plain");
        app.parsers.insert(1, p);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 45)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let (_, inner) = app.panes[0];
        let buf = term.backend().buffer();
        assert_eq!(buf[(inner.x, inner.y)].bg, app.theme.card2, "not a light grey slab");
        assert_eq!(buf[(inner.x + 13, inner.y)].bg, app.theme.bg);
    }

    #[test]
    fn options_come_from_the_screen() {
        let mut p = vt100::Parser::new(10, 60, 0);
        p.process(b"Question: use WebKit or Safari?\r\n  1. WebKit build\r\n  2. Safari driver");
        assert_eq!(hydra::options(Some(&p)), vec!["WebKit build".to_string(), "Safari driver".to_string()]);
        assert_eq!(hydra::options(None), vec!["Yes", "Always", "No"]);
    }
}
