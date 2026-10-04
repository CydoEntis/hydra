//! The TUI client: renders the daemon's snapshot, keeps a terminal emulator per pane, and
//! turns keys into either pane input or commands.

mod copy;
mod design;
mod files;
mod find;
mod branch;
mod hydra;
mod menu;
mod modal;
mod pick;
mod pr;
mod render;
mod tasks;
mod toolbox;
mod views;
mod work;
mod actions;
mod background;
mod input;
mod view_keys;

use crate::config::{Config, Keymap};
use crate::ipc;
use crate::keys::{self, Action, KeySpec};
use crate::layout::Dir;
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
    Quick(modal::Quick),
    Toolbox(Box<toolbox::ToolboxView>),
    Prompt { kind: PromptKind, input: String },
    Help { scroll: u16 },
    Copy(Box<copy::Copy>),
    /// Pick an existing worktree of the repo, or type a branch to create one.
    Worktrees { ws: WsId, cmd: Option<String>, items: Option<Vec<WorktreeEntry>>, query: String, sel: usize },
    /// Talking to one pane's agent in a modal.
    Talk { term: TermId, input: String },
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
    /// Find a file / search the code.
    Find(Box<find::FindView>),
    /// Switch branch.
    Branch(Box<branch::BranchView>),
    /// Memory per session (the selected row).
    Memory { sel: usize },
    /// Notification history (the selected row, newest first).
    History { sel: usize },
    /// "Close …?" with confirm / cancel.
    Confirm(Box<menu::Confirm>),
    /// The switcher: projects and sessions, typed to filter.
    GoTo { query: String, sel: usize },
}

/// The ship confirm: the branch and what shipping it will do.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ShipAsk {
    pub task: tasks::TaskRow,
    pub changed: usize,
    pub pr: Option<String>,
}

/// A view that replaces the pane area.
pub(super) enum View {
    Changes(Box<views::ChangesView>),
    Files(Box<views::FilesTree>),
    Pr(Box<pr::PrView>),
    Map(Box<hydra::MapView>),
}

/// Clickable chips and buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Btn {
    Answer(TermId, char),
    CloseView,
    /// A key inside the current view ('\n' for Enter).
    ViewKey(char),
    /// A row of the current view's list.
    Row(usize),
}

/// Results of background work (disk scans, git, gh, APIs), delivered to the event loop.
/// Windows: start a console program without flashing a console window.
#[cfg(windows)]
pub(super) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) enum Bg {
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
    /// An extension's label for a worktree, or a background command's result.
    ExtLabel(String, usize, String),
    ExtDone(Result<String, String>),
    /// Every file under a folder (Find).
    FindFiles(PathBuf, Vec<String>),
    /// A checkout's branches: (folder, current, branches, files with changes).
    Branches(PathBuf, String, Vec<branch::Branch>, usize),
    /// A branch switch finished.
    Switched(Result<String, String>),
    /// A code search's results: (folder, which search, hits).
    Grep(PathBuf, u64, Result<Vec<find::GrepHit>, String>),
    /// A file's diff for the Changes view: (folder, which file, lines).
    Diff(PathBuf, usize, Vec<String>),
    /// Slow work done; finish it on the UI thread.
    Then(Box<dyn FnOnce(&mut App) + Send>),
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

/// Save the clipboard image as a PNG under the data folder; old pastes (a week) are
/// cleared out. Returns (path, width, height).
fn clipboard_image_to_file() -> Result<(PathBuf, usize, usize), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("can't open the clipboard: {e}"))?;
    let img = cb.get_image().map_err(|_| "there's no image on the clipboard".to_string())?;
    let dir = crate::config::data_dir().join("pastes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a.as_secs() > 7 * 86_400)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = dir.join(format!("paste-{ms}.png"));
    write_png(&path, img.width as u32, img.height as u32, &img.bytes)?;
    Ok((path, img.width, img.height))
}

/// RGBA pixels to a PNG file.
fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    Ok(())
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

/// Used where a key handler replaces the view itself and must not restore the old one.
fn true_and_replace() -> bool {
    false
}

/// The repository root above `p`, or `p` itself.
fn repo_root(p: &std::path::Path) -> PathBuf {
    p.ancestors().find(|a| a.join(".git").exists()).map(|a| a.to_path_buf()).unwrap_or_else(|| p.to_path_buf())
}

/// A row in the worktree picker.
#[derive(Debug, Clone)]
enum WtRow {
    Existing(WorktreeEntry),
    Create(String),
}

#[derive(Debug, Clone, PartialEq)]
enum PromptKind {
    RenamePane(TermId),
    RenameProject(String),
    RenameTab(WsId, TabId),
    NewWorkspace,
    ConfirmCloseWorkspace(WsId),
    ConfirmKillServer,
    ConfirmRemoveWorktree(WsId),
}

impl PromptKind {
    fn label(&self) -> &'static str {
        match self {
            PromptKind::RenamePane(_) => "Rename pane (empty: automatic)",
            PromptKind::RenameProject(_) => "Rename project (empty: its folder name)",
            PromptKind::RenameTab(..) => "Rename tab",
            PromptKind::NewWorkspace => "New pane: name it (empty = its folder)",
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
    Pane(TermId),
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
    /// An extension's command: (extension, command).
    Ext(usize, usize),
}

pub struct App {
    cfg: Config,
    theme: Theme,
    keymap: Keymap,
    notice: Option<(String, Instant, bool)>,
    /// What happened lately, newest last: (unix secs, pane, kind, text). Kinds: '!' needs
    /// you, '✓' finished, '♪' bell, 'i' a message, 'x' an error.
    pub(super) history: std::collections::VecDeque<(u64, Option<TermId>, char, String)>,
    /// Extensions, and the labels they put on worktree rows: (worktree key, label n) ->
    /// (text, when it was asked for).
    pub(super) exts: Vec<crate::ext::Ext>,
    /// When the right button last went down (menus open on press or release, once).
    right_down: Option<Instant>,
    pub(super) ext_labels: HashMap<(String, usize), (String, Instant)>,
    snap: Snapshot,
    got_state: bool,
    parsers: HashMap<TermId, vt100::Parser>,
    scroll: HashMap<TermId, usize>,
    sizes: HashMap<TermId, (u16, u16)>,
    mode: Mode,
    zoomed: HashSet<TabId>,
    sidebar: bool,
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
    /// Where the mouse is, for hover highlights.
    hover: Option<Position>,
    /// The status line's message can be undone with `u`.
    undo_hint: bool,
    /// The last click, for double-click detection.
    last_click: Option<(Hit, Instant)>,
    /// The welcome screen is up; which of its panes is selected.
    splash: bool,
    /// The hydra layout's own state.
    hy: hydra::Hy,
    /// The terminal window has focus (for alerts about the session you're looking at).
    window_focused: bool,
    /// The background we told the terminal to use (OSC 11), so its padding matches.
    osc_bg: Option<ratatui::style::Color>,
    /// Panes in the middle of a synchronized update (mode 2026): since when. Not drawn
    /// until it ends (or 100 ms pass), so redraws don't flicker.
    sync_hold: HashMap<TermId, Instant>,
    /// Panes whose program asked to hear about focus (mode 1004).
    focus_report: HashSet<TermId>,
    /// The focus we last told programs about: (pane, window focused).
    focus_sent: Option<(TermId, bool)>,
    /// The last left-click in a pane, for double-click.
    pane_click: Option<(Position, Instant)>,
    /// Recent raw output per pane, to re-wrap the screen when its width changes.
    raw: HashMap<TermId, std::collections::VecDeque<u8>>,
    /// The cursor style each pane's program asked for (DECSCUSR 0-6), and the one we set.
    cursor_style: HashMap<TermId, u8>,
    cursor_sent: u8,
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
    // A panic must leave the shell as it found it: ratatui's own hook leaves the alternate
    // screen and raw mode; this one also turns off what hydra turned on.
    let earlier = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        earlier(info);
    }));
    if app.cfg.ui.mouse {
        let _ = execute!(std::io::stdout(), event::EnableMouseCapture);
    }
    let _ = execute!(std::io::stdout(), event::EnableBracketedPaste, event::EnableFocusChange);

    let result = app.event_loop(&mut terminal, &mut reader, &mut bg_rx).await;

    restore_terminal();
    result?;
    if let Some(reason) = app.quit {
        println!("[{reason}]");
    }
    Ok(())
}

/// Undo everything hydra turned on in the terminal: mouse, paste and focus reporting, its
/// cursor shape and background colour, the alternate screen and raw mode. Safe to run twice.
fn restore_terminal() {
    let _ = execute!(
        std::io::stdout(),
        event::DisableMouseCapture,
        event::DisableBracketedPaste,
        event::DisableFocusChange
    );
    // Give the terminal its own background and cursor back.
    let _ = execute!(std::io::stdout(), crossterm::cursor::SetCursorStyle::DefaultUserShape);
    {
        use std::io::Write;
        let _ = write!(std::io::stdout(), "]111");
    }
    ratatui::restore();
}

impl App {
    fn new(cfg: Config, out: mpsc::UnboundedSender<ClientMsg>, open: Option<PathBuf>, bg_tx: mpsc::UnboundedSender<Bg>) -> App {
        let keymap = cfg.keymap();
        let mut app = App {
            theme: cfg.theme(),
            sidebar: cfg.ui.sidebar,
            keymap,
            cfg,
            notice: None,
            history: Default::default(),
            exts: if cfg!(test) { Vec::new() } else { crate::ext::load_all().0 },
            ext_labels: HashMap::new(),
            right_down: None,
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
            hover: None,
            undo_hint: false,
            last_click: None,
            splash: false,
            hy: hydra::Hy::load(),
            window_focused: true,
            osc_bg: None,
            sync_hold: HashMap::new(),
            focus_report: HashSet::new(),
            focus_sent: None,
            pane_click: None,
            raw: HashMap::new(),
            cursor_style: HashMap::new(),
            cursor_sent: 0,
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
        let mut last_draw = crate::clock::ago(Duration::from_secs(1));
        let mut last_frame = 0u64;
        let mut last_fresh = Instant::now();
        // At most one frame per 12 ms, but never wait longer than that to show new output.
        let frame = Duration::from_millis(12);
        loop {
            let hold = self.sync_hold_until();
            let next_draw = tokio::time::Instant::from_std((last_draw + frame).max(hold.unwrap_or(last_draw)));
            tokio::select! {
                _ = tokio::time::sleep_until(next_draw), if self.dirty => {}
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
                    // One message that didn't decode: skip it rather than quit.
                    Err(e) if ipc::is_decode_error(&e) => tracing::warn!("skipped a message from the server: {e:#}"),
                    _ => {
                        self.quit.get_or_insert_with(|| "server exited".into());
                    }
                },
                _ = tick.tick() => {
                    // What agents ask (read off their screens) can change without a state
                    // message; look again now and then.
                    if last_fresh.elapsed() >= Duration::from_secs(1) {
                        last_fresh = Instant::now();
                        self.hy_fresh();
                        self.dirty = true;
                    }
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
            self.report_focus();
            self.sync_cursor_style();
            self.request_diff();
            if self.dirty && last_draw.elapsed() >= frame && self.sync_hold_until().is_none_or(|h| Instant::now() >= h) {
                self.dirty = false;
                last_draw = Instant::now();
                terminal.draw(|f| render::draw(self, f))?;
                self.sync_sizes();
            }
        }
    }

    fn notify(&mut self, msg: String, error: bool) {
        self.remember(None, if error { 'x' } else { 'i' }, msg.clone());
        self.notice = Some((msg, Instant::now(), error));
        self.dirty = true;
    }

    /// Add to the notification history (Ctrl+Space N).
    pub(super) fn remember(&mut self, term: Option<TermId>, kind: char, text: String) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        // The same thing twice in a row is one entry.
        if self.history.back().is_some_and(|(_, t, k, x)| *t == term && *k == kind && *x == text) {
            return;
        }
        self.history.push_back((now, term, kind, text));
        while self.history.len() > 300 {
            self.history.pop_front();
        }
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
            let old = self.sizes.insert(term, want);
            if old.is_some_and(|o| o.0 != want.0) && self.raw.get(&term).is_some_and(|r| !r.is_empty()) {
                // A new width: replay recent output into a fresh screen so text re-wraps
                // instead of being cut (history included).
                let mut p = self.new_parser(want.1, want.0);
                if let Some(raw) = self.raw.get_mut(&term) {
                    p.process(raw.make_contiguous());
                }
                self.parsers.insert(term, p);
                self.scroll.remove(&term);
            } else if let Some(p) = self.parsers.get_mut(&term) {
                p.screen_mut().set_size(want.1, want.0);
            }
            self.send(ClientMsg::Resize { term, cols: want.0, rows: want.1 });
        }
    }

    // ---- server messages -----------------------------------------------------------

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
        let _ = lines;
        self.notify(format!("Copied{how}"), false);
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

    /// Keep the last ~768 KiB a pane printed (for re-wrapping on resize).
    fn keep_raw(&mut self, term: TermId, data: &[u8], replace: bool) {
        const CAP: usize = 768 * 1024;
        let ring = self.raw.entry(term).or_default();
        if replace {
            ring.clear();
        }
        ring.extend(data.iter().copied());
        if ring.len() > CAP {
            let extra = ring.len() - CAP;
            ring.drain(..extra);
        }
    }

    /// Show the focused program's cursor style (bar, block, underline) in the real terminal.
    fn sync_cursor_style(&mut self) {
        use crossterm::cursor::SetCursorStyle as S;
        let want = self.focused().and_then(|t| self.cursor_style.get(&t).copied()).unwrap_or(0);
        if want == self.cursor_sent {
            return;
        }
        self.cursor_sent = want;
        let style = match want {
            1 => S::BlinkingBlock,
            2 => S::SteadyBlock,
            3 => S::BlinkingUnderScore,
            4 => S::SteadyUnderScore,
            5 => S::BlinkingBar,
            6 => S::SteadyBar,
            _ => S::DefaultUserShape,
        };
        if !cfg!(test) {
            let _ = execute!(std::io::stdout(), style);
        }
    }

    /// The clipboard's image, saved as a PNG; its path is pasted into the focused pane
    /// (Claude Code, Codex and friends attach an image given by path).
    pub(super) fn paste_image(&mut self) {
        let Some(term) = self.focused() else { return };
        // Reading the clipboard and encoding the PNG can take a while for a big image.
        self.spawn_bg(move || {
            let made = clipboard_image_to_file();
            Bg::Then(Box::new(move |app: &mut App| match made {
                Ok((path, w, h)) => {
                    let text = files::quote_path(&path);
                    let bracketed = app.parsers.get(&term).is_some_and(|p| p.screen().bracketed_paste());
                    let data = if bracketed { format!("\x1b[200~{text} \x1b[201~") } else { format!("{text} ") };
                    app.send(ClientMsg::Input { term, data: data.into_bytes() });
                    app.notify(format!("pasted the clipboard image ({w}×{h}) as a file"), false);
                }
                Err(e) => app.notify(e, true),
            }))
        });
    }

    /// When to draw a pane that's mid synchronized update: its start + 100 ms, if that's
    /// still ahead.
    fn sync_hold_until(&self) -> Option<Instant> {
        let limit = Duration::from_millis(100);
        self.panes
            .iter()
            .filter_map(|(t, _)| self.sync_hold.get(t))
            .map(|at| *at + limit)
            .filter(|until| *until > Instant::now())
            .max()
    }

    /// Programs that asked (mode 1004) hear when their pane or the window gains or loses
    /// focus: `ESC [ I` / `ESC [ O`.
    fn report_focus(&mut self) {
        let now = self.focused().map(|t| (t, self.window_focused));
        if now == self.focus_sent {
            return;
        }
        if let Some((t, true)) = self.focus_sent
            && self.focus_report.contains(&t)
            && now != Some((t, true))
        {
            self.send(ClientMsg::Input { term: t, data: b"\x1b[O".to_vec() });
        }
        if let Some((t, true)) = now
            && self.focus_report.contains(&t)
        {
            self.send(ClientMsg::Input { term: t, data: b"\x1b[I".to_vec() });
        }
        self.focus_sent = now;
    }

    /// (offset from the bottom, lines of history) for a pane.
    pub(super) fn history(&mut self, term: TermId) -> (usize, usize) {
        let Some(p) = self.parsers.get_mut(&term) else { return (0, 0) };
        let cur = p.screen().scrollback();
        p.screen_mut().set_scrollback(usize::MAX);
        let total = p.screen().scrollback();
        p.screen_mut().set_scrollback(cur);
        (cur, total)
    }

    pub(super) fn scroll_to(&mut self, term: TermId, offset: usize) {
        let cur = self.scroll.get(&term).copied().unwrap_or(0) as i32;
        self.scroll_by(term, offset as i32 - cur);
    }

    // ---- actions -------------------------------------------------------------------

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

}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a == b
}

#[cfg(test)]
mod tests;
