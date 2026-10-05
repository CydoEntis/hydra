//! The daemon: owns every PTY and the workspace tree, survives clients detaching.
//!
//! All state lives in one event loop (`Daemon::run`). PTY reader threads, the process
//! scanner, background git work and client connections talk to it through a single
//! channel, so there is no locking around the model.

mod git;
mod persist;
mod scan;
mod term;
mod commands;
mod contain;
mod restore;
mod status;
mod worktrees;

use status::*;
use worktrees::*;

use crate::config::{CompiledAgent, Config};
use crate::ipc;
use crate::layout::Node;
use crate::protocol::*;
use anyhow::Result;
use interprocess::local_socket::traits::tokio::Listener as _;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use term::{SpawnSpec, Term};
use tokio::sync::mpsc;

type ClientId = u64;

/// How often the daemon looks at its timers (statuses, saves, git, sleep).
const TICK: Duration = Duration::from_millis(250);
/// How often each workspace's git state is read again.
const GIT_POLL_EVERY: Duration = Duration::from_secs(3);
/// How often a changed session is saved to disk.
const SAVE_EVERY: Duration = Duration::from_secs(2);
/// How often idle agents are checked for putting to sleep.
const SLEEP_CHECK_EVERY: Duration = Duration::from_secs(30);
/// A daemon nothing ever used (no panes, no clients) exits after this.
const EXIT_WHEN_UNUSED_FOR: Duration = Duration::from_secs(30);
/// Two panes ending on their own within this window means something killed them all
/// (a reboot, a logoff): their sessions are kept for the next start.
const MASS_EXIT_WINDOW: Duration = Duration::from_secs(5);

#[allow(clippy::large_enum_variant)]
pub enum Ev {
    Connected(ClientId, mpsc::Sender<ServerMsg>, bool),
    Msg(ClientId, ClientMsg),
    Disconnected(ClientId),
    Output(TermId, Vec<u8>),
    Exited(TermId),
    Scan(Vec<scan::Found>),
    Git(Vec<(WsId, Option<GitInfo>)>),
    WorktreeCreated {
        client: ClientId,
        branch: String,
        cmd: Option<String>,
        split: Option<TermId>,
        result: Result<(PathBuf, String), String>,
    },
    WorktreeRemoved { client: ClientId, path: PathBuf, result: Result<(), String> },
    /// A worktree removed (or kept) after its last pane closed.
    WorktreeAutoRemoved { client: ClientId, path: PathBuf, result: Result<(), String> },
    WorktreeList { client: ClientId, result: Result<Vec<WorktreeEntry>, String> },
    /// A worktree hook ran: what to tell people.
    HookRan(String),
    /// A spare worktree for prewarming: (repo, result).
    SpareMade(PathBuf, Result<(PathBuf, String), String>),
    /// A worktree made for an agent to move into: (client, pane, branch, result).
    MoveReady { client: ClientId, term: TermId, branch: String, result: Result<(PathBuf, String), String> },
    /// A status report, its sender checked (see `HookJob`).
    HookChecked { msg: ClientMsg, verdict: Option<bool>, chain: Vec<u32> },
}

/// How to put back what an automatic workspace changed.
enum AutoUndo {
    /// The pane moved out of `from_ws`/`from_tab` (beside `beside`).
    Moved { term: TermId, from_ws: WsId, from_tab: TabId, beside: Option<TermId> },
    /// The pane was alone, so its workspace was re-homed; restore name and folder.
    Rehomed { ws: WsId, name: String, cwd: PathBuf },
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    n(a) == n(b)
}

struct Client {
    tx: mpsc::Sender<ServerMsg>,
    attach: bool,
    /// Its queue filled up (a stalled terminal, a slow SSH link): it's dropped, and can
    /// attach again for a fresh copy of everything.
    behind: std::cell::Cell<bool>,
}

/// Messages waiting for one client before it counts as too far behind.
const CLIENT_QUEUE: usize = 8192;

impl Client {
    fn push(&self, msg: ServerMsg) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send(msg) {
            self.behind.set(true);
        }
    }
}

struct Daemon {
    cfg: Config,
    agents: Vec<CompiledAgent>,
    terms: HashMap<TermId, Term>,
    workspaces: Vec<WorkspaceInfo>,
    active_ws: Option<WsId>,
    next_id: u32,
    clients: HashMap<ClientId, Client>,
    tx: mpsc::Sender<Ev>,
    /// To the hook thread (see `hook_thread`).
    hook_jobs: std::sync::mpsc::Sender<HookJob>,
    scan: Arc<Mutex<scan::Shared>>,
    dirty: bool,
    had_terms: bool,
    empty_since: Instant,
    /// Background git work in flight; the server stays up until it lands.
    pending_ops: u32,
    /// Questions agents asked you: the question, who's waiting for the answer, and the
    /// pane's status before it asked.
    questions: Vec<(HumanQuestion, ClientId, Status)>,
    git_busy: bool,
    last_git: Instant,
    last_save: Instant,
    last_saved: String,
    /// Worktrees hydra created; closing the last thing in one removes it.
    made_worktrees: Vec<PathBuf>,
    /// Extensions (their hooks run here).
    exts: Vec<crate::ext::Ext>,
    /// Extra environment for the next pane spawned (a dev server's PORT).
    next_env: Vec<(String, String)>,
    /// The prewarmed agent: (repo, its worktree, its pane), and whether one is being made.
    spare: Option<(PathBuf, PathBuf, TermId)>,
    spare_making: bool,
    /// A spare being handed over: the next pane spawned in this folder is it.
    adopt: Option<(PathBuf, TermId)>,
    /// The spare's folder, handed over: its on_create hook already ran.
    adopted: Option<PathBuf>,
    last_sleep_check: Instant,
    /// The last automatic workspace move, for undo.
    auto_undo: Option<AutoUndo>,
    /// Panes whose process ended on its own, recently. Several at once means a crash,
    /// logoff or reboot is tearing things down, which must not overwrite the saved session.
    natural_exits: Vec<Instant>,
}

pub fn run() -> Result<()> {
    init_logging();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    rt.block_on(async {
        // Refuse to start a second daemon on the same socket.
        if ipc::connect().await.is_ok() {
            anyhow::bail!("a hydra daemon is already running on {}", ipc::socket_id());
        }
        let listener = ipc::listen()?;
        tracing::info!("daemon listening on {}", ipc::socket_id());
        // Lets `hydra kill-server` from another version stop it (it can't talk to this one).
        let _ = std::fs::write(ipc::pid_file(), std::process::id().to_string());
        let (tx, rx) = mpsc::channel::<Ev>(4096);

        let accept_tx = tx.clone();
        tokio::spawn(async move {
            let mut next: ClientId = 1;
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        let id = next;
                        next += 1;
                        tokio::spawn(serve(id, stream, accept_tx.clone()));
                    }
                    Err(e) => {
                        tracing::error!("accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        if let Ok(p) = std::fs::read_to_string(spare_file()) {
            let p = PathBuf::from(p.trim());
            tokio::task::spawn_blocking(move || drop_spare_dir(&p));
            save_spare(None);
        }
        let (cfg, err) = Config::load_or_default();
        if let Some(e) = err {
            tracing::warn!("config: {e}");
        }
        let mut d = Daemon::new(cfg, tx);
        if d.cfg.restore.enabled
            && let Some(saved) = persist::load()
        {
            d.restore(saved);
            d.last_saved = serde_json::to_string(&d.saved()).unwrap_or_default();
        }
        d.run(rx).await;
        Ok(())
    })
}

fn init_logging() {
    let dir = crate::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("daemon.log")) {
        let _ = tracing_subscriber::fmt()
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("HYDRA_LOG").unwrap_or_else(|_| "info".into()),
            )
            .try_init();
    }
}

async fn serve(id: ClientId, stream: interprocess::local_socket::tokio::Stream, ev: mpsc::Sender<Ev>) {
    let (mut r, mut w) = ipc::framed(stream);
    let attach = match ipc::recv_client(&mut r).await {
        Ok(Some(ClientMsg::Hello { attach, .. })) => attach,
        _ => return,
    };
    if ipc::send(&mut w, &ServerMsg::Welcome { version: PROTOCOL_VERSION }).await.is_err() {
        return;
    }
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(CLIENT_QUEUE);
    if ev.send(Ev::Connected(id, tx, attach)).await.is_err() {
        return;
    }
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let bye = matches!(msg, ServerMsg::Bye);
            if ipc::send(&mut w, &msg).await.is_err() || bye {
                break;
            }
        }
    });
    while let Ok(Some(msg)) = ipc::recv_client(&mut r).await {
        if ev.send(Ev::Msg(id, msg)).await.is_err() {
            break;
        }
    }
    let _ = ev.send(Ev::Disconnected(id)).await;
    writer.abort();
}

/// Discard the saved session (for `kill-server --forget` when the server couldn't be asked).
pub fn forget_session() {
    persist::forget();
}

impl Daemon {
    /// A daemon with no panes yet; its process scanner and hook thread are running.
    fn new(cfg: Config, tx: mpsc::Sender<Ev>) -> Daemon {
        contain::children_die_with_us();
        // Clicking a notification opens its hydra:// link (Windows needs to be told how).
        if cfg.notify.desktop && !cfg!(test) {
            crate::reveal::register();
        }
        // Hooks naming another hydra (an older install) can't reach this server.
        if !cfg!(test) {
            crate::cli::refresh_claude_hooks();
        }
        let agents = cfg.agent_defs();
        let scan = Arc::new(Mutex::new(scan::Shared {
            roots: Vec::new(),
            agents: agents.clone(),
            interval: Duration::from_millis(cfg.detection.scan_interval_ms),
        }));
        scan::start(scan.clone(), tx.clone());
        let hook_jobs = hook_thread(tx.clone());
        Daemon {
            cfg,
            agents,
            terms: HashMap::new(),
            workspaces: Vec::new(),
            active_ws: None,
            next_id: 1,
            clients: HashMap::new(),
            hook_jobs,
            tx,
            scan,
            dirty: false,
            had_terms: false,
            empty_since: Instant::now(),
            pending_ops: 0,
            questions: Vec::new(),
            git_busy: false,
            last_git: crate::clock::ago(GIT_POLL_EVERY),
            last_save: Instant::now(),
            last_saved: String::new(),
            made_worktrees: Vec::new(),
            next_env: Vec::new(),
            // Tests don't load the user's extensions.
            exts: if cfg!(test) { Vec::new() } else { crate::ext::load_all().0 },
            spare: None,
            spare_making: false,
            adopt: None,
            adopted: None,
            last_sleep_check: Instant::now(),
            natural_exits: Vec::new(),
            auto_undo: None,
        }
    }

    async fn run(&mut self, mut rx: mpsc::Receiver<Ev>) {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                Some(ev) = rx.recv() => {
                    self.handle(ev);
                    // Drain whatever else is queued before broadcasting state once.
                    while let Ok(ev) = rx.try_recv() {
                        self.handle(ev);
                    }
                }
                _ = tick.tick() => {
                    self.update_statuses();
                    if self.last_sleep_check.elapsed() >= SLEEP_CHECK_EVERY {
                        self.last_sleep_check = Instant::now();
                        self.sleep_idle();
                    }
                    self.poll_git();
                    if self.last_save.elapsed() > SAVE_EVERY {
                        self.last_save = Instant::now();
                        self.persist(false);
                    }
                }
            }
            if self.dirty {
                self.dirty = false;
                self.sync_scan_roots();
                let snap = self.snapshot();
                self.broadcast(|_| true, ServerMsg::State(snap));
            }
            if self.should_exit() {
                tracing::info!("no panes left; exiting");
                // The user closed everything, so start fresh next time. A mass exit is a
                // crash or shutdown: leave the saved session for restore.
                if !self.mass_exit() {
                    persist::forget();
                }
                self.broadcast(|_| true, ServerMsg::Bye);
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = std::fs::remove_file(ipc::pid_file());
                std::process::exit(0);
            }
        }
    }

    /// Look at git again on the next tick (something just changed it).
    fn poll_git_soon(&mut self) {
        self.last_git = crate::clock::ago(GIT_POLL_EVERY);
    }

    fn should_exit(&self) -> bool {
        // A window still showing hydra (empty projects, the splash) keeps it open: closing
        // the last pane mustn't close the app under you.
        if !self.terms.is_empty() || self.pending_ops > 0 || self.clients.values().any(|c| c.attach) {
            return false;
        }
        // Exit once the last pane is closed and no window is left, or if nobody ever started one.
        self.had_terms || (self.clients.is_empty() && self.empty_since.elapsed() > EXIT_WHEN_UNUSED_FOR)
    }

    fn broadcast(&self, filter: impl Fn(&Client) -> bool, msg: ServerMsg) {
        for c in self.clients.values().filter(|c| filter(c)) {
            c.push(msg.clone());
        }
    }

    /// Let go of clients whose queue filled up; their connection closes and they can attach
    /// again.
    fn drop_lagging_clients(&mut self) {
        self.clients.retain(|id, c| {
            if c.behind.get() {
                tracing::warn!("client {id} fell too far behind; dropping it");
            }
            !c.behind.get()
        });
    }

    fn send(&self, client: ClientId, msg: ServerMsg) {
        if let Some(c) = self.clients.get(&client) {
            c.push(msg);
        }
    }

    // ---- persistence ---------------------------------------------------------------

    /// The first palette colour no workspace is using (cycling once all are taken).
    fn free_color(&self) -> u8 {
        let n = self.cfg.ui.workspace_colors.len().clamp(1, 255) as u8;
        (0..n)
            .find(|c| !self.workspaces.iter().any(|w| w.color == *c))
            .unwrap_or((self.workspaces.len() % n as usize) as u8)
    }

    fn poll_git(&mut self) {
        if self.git_busy || self.workspaces.is_empty() || self.last_git.elapsed() < GIT_POLL_EVERY {
            return;
        }
        for t in self.terms.values_mut() {
            if t.refresh_head() {
                self.dirty = true;
            }
        }
        self.git_busy = true;
        self.last_git = Instant::now();
        let dirs: Vec<(WsId, PathBuf)> = self.workspaces.iter().map(|w| (w.id, w.cwd.clone())).collect();
        let dir_of: HashMap<WsId, PathBuf> = dirs.iter().cloned().collect();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let mut res: Vec<(WsId, Option<GitInfo>)> = dirs.into_iter().map(|(id, dir)| (id, git::status(&dir))).collect();
            // One worktree listing per repository, shared by its workspaces.
            let mut lists: HashMap<PathBuf, Vec<WorktreeEntry>> = HashMap::new();
            for (_, info) in res.iter_mut() {
                let Some(g) = info else { continue };
                let list = lists
                    .entry(g.root.clone())
                    .or_insert_with(|| git::list_worktrees(&g.root).unwrap_or_default());
                g.worktrees = list.clone();
            }
            for (id, info) in res.iter_mut() {
                let Some(g) = info.as_mut().filter(|g| g.linked) else { continue };
                if let (Some(base), Some(dir)) = (
                    g.worktrees.iter().find(|e| e.main).map(|e| e.branch.clone()),
                    dir_of.get(id),
                ) {
                    g.ahead = git::ahead_of(dir, &base);
                }
            }
            let _ = tx.blocking_send(Ev::Git(res));
        });
    }

    // ---- events --------------------------------------------------------------------

    fn handle(&mut self, ev: Ev) {
        self.drop_lagging_clients();
        match ev {
            Ev::Connected(id, tx, attach) => {
                let c = Client { tx, attach, behind: std::cell::Cell::new(false) };
                if attach {
                    c.push(ServerMsg::State(self.snapshot()));
                    for t in self.terms.values() {
                        c.push(ServerMsg::Replay { term: t.id, cols: t.cols, rows: t.rows, data: t.replay() });
                    }
                }
                self.clients.insert(id, c);
            }
            Ev::Disconnected(id) => {
                self.clients.remove(&id);
                // The agent that asked went away (cancelled, killed): its question goes too.
                let gone: Vec<(TermId, Status)> = self.questions.iter().filter(|(_, c, _)| *c == id).map(|(q, _, s)| (q.term, *s)).collect();
                if !gone.is_empty() {
                    self.questions.retain(|(_, c, _)| *c != id);
                    for (term, before) in gone {
                        self.set_status(term, before);
                    }
                    self.dirty = true;
                }
            }
            Ev::Msg(id, msg) => self.message(id, msg),
            Ev::Output(tid, data) => {
                let Some(t) = self.terms.get_mut(&tid) else { return };
                if t.output(&data) {
                    t.refresh_head();
                    self.dirty = true;
                }
                if std::mem::take(&mut t.parser.callbacks_mut().title_changed) {
                    self.dirty = true;
                }
                if let Some(text) = t.parser.callbacks_mut().copied.take() {
                    self.broadcast(|c| c.attach, ServerMsg::Clipboard { term: tid, text });
                }
                // A bell from a pane you aren't looking at marks it until you do.
                if self.terms.get_mut(&tid).is_some_and(|t| std::mem::take(&mut t.parser.callbacks_mut().rang)) {
                    let seen = self.focused_term() == Some(tid) && self.has_viewer();
                    if let Some(t) = self.terms.get_mut(&tid)
                        && !seen
                        && !t.bell
                    {
                        t.bell = true;
                        self.dirty = true;
                    }
                }
                self.broadcast(|c| c.attach, ServerMsg::Output { term: tid, data });
            }
            Ev::Exited(tid) => {
                // A sleeping agent's process was stopped on purpose; its pane stays.
                if self.terms.get(&tid).is_some_and(|t| t.asleep) {
                    return;
                }
                if self.terms.contains_key(&tid) {
                    self.natural_exits.retain(|t| t.elapsed() < MASS_EXIT_WINDOW);
                    self.natural_exits.push(Instant::now());
                    self.remove_term(tid);
                }
            }
            Ev::Scan(found) => {
                let mut started = Vec::new();
                for f in found {
                    let Some(t) = self.terms.get_mut(&f.term) else { continue };
                    // Redraw for memory only when it moved by a few MB.
                    if t.mem.abs_diff(f.mem) > 4 << 20 {
                        t.mem = f.mem;
                        self.dirty = true;
                    }
                    if t.process != f.process && !f.process.is_empty() {
                        t.process = f.process;
                        self.dirty = true;
                    }
                    if t.remote != f.remote {
                        t.remote = f.remote;
                        self.dirty = true;
                    }
                    if !t.cwd_reported
                        && let Some(c) = f.cwd.filter(|c| c.is_dir() && *c != t.cwd)
                    {
                        t.cwd = c;
                        t.refresh_head();
                        self.dirty = true;
                    }
                    // Hooks own the agent identity while they're reporting.
                    if t.hooked && t.agent.is_some() {
                        continue;
                    }
                    if t.agent != f.agent {
                        if t.agent.is_none() && f.agent.is_some() {
                            // Where the agent really runs, even if the shell never said.
                            if let Some(c) = f.agent_cwd.clone().filter(|c| c.is_dir()) {
                                t.cwd = c;
                                t.refresh_head();
                                // The shell's process folder is stale (PowerShell never moves
                                // it); don't let the next scan put it back.
                                t.cwd_reported = true;
                            }
                            started.push(f.term);
                        }
                        t.status = if f.agent.is_some() { Status::Idle } else { Status::None };
                        t.status_since = term::unix_now();
                        if f.agent.is_none() {
                            t.session = None;
                            t.summary.clear();
                        }
                        t.agent = f.agent;
                        t.hooked = false;
                        self.dirty = true;
                    }
                }
                for term in &started {
                    self.ext_event("agent_start", None, Some(*term));
                }
                if self.cfg.auto_workspace {
                    for term in started {
                        self.auto_workspace(term);
                    }
                }
            }
            Ev::Git(results) => {
                self.git_busy = false;
                for (id, info) in results {
                    if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id)
                        && w.git != info
                    {
                        w.git = info;
                        self.dirty = true;
                    }
                }
            }
            Ev::WorktreeCreated { client, branch, cmd, split, result } => {
                self.pending_ops -= 1;
                let as_pane = split.filter(|t| self.terms.contains_key(t));
                let opened = result.map_err(anyhow::Error::msg).and_then(|(path, repo)| {
                    match as_pane {
                        // Beside the pane it was asked from, in the same workspace.
                        Some(term) => {
                            let dir = if self.terms.get(&term).is_some_and(|t| t.cols >= 100) {
                                crate::layout::Dir::Right
                            } else {
                                crate::layout::Dir::Down
                            };
                            self.command(client, Command::Split { term, dir, cmd, cwd: Some(path.clone()) })?;
                        }
                        None => {
                            let _ = &repo;
                            self.command(client, Command::NewWorkspace { cwd: Some(path.clone()), name: None, cmd })?;
                        }
                    }
                    Ok(path)
                });
                if let Ok(p) = &opened {
                    self.made_worktrees.push(p.clone());
                    if !self.adopted.take().is_some_and(|a| same_path(&a, p)) {
                        self.worktree_hook(p, true);
                    }
                }
                match opened {
                    Ok(path) => {
                        if as_pane.is_none()
                            && let Some(w) = self.workspaces.last_mut()
                        {
                            w.worktree = true;
                        }
                        self.poll_git_soon();
                        self.send(client, ServerMsg::Notice(format!("worktree {branch} at {}", path.display())));
                        self.send(client, ServerMsg::Reply(Reply::Ok));
                    }
                    Err(e) => self.send(client, ServerMsg::Error(format!("{e:#}"))),
                }
                self.dirty = true;
            }
            Ev::HookRan(msg) => self.broadcast(|_| true, ServerMsg::Notice(msg)),
            Ev::HookChecked { msg, verdict, chain } => self.apply_hook(msg, verdict, chain),
            Ev::SpareMade(repo, result) => {
                self.spare_making = false;
                match result {
                    Ok((path, _)) => {
                        save_spare(Some(&path));
                        self.worktree_hook(&path, true);
                        let cmd = self.cfg.worktree.prewarm.clone();
                        let (cols, rows) = self.guess_size();
                        match self.spawn(Some(&cmd), &path, cols, rows) {
                            Ok(term) => {
                                if let Some(t) = self.terms.get_mut(&term) {
                                    t.spare = true;
                                }
                                self.spare = Some((repo, path, term));
                            }
                            Err(e) => {
                                tracing::warn!("prewarm: {e:#}");
                                tokio::task::spawn_blocking(move || drop_spare_dir(&path));
                            }
                        }
                    }
                    Err(e) => tracing::warn!("prewarm: {e}"),
                }
            }
            Ev::WorktreeList { client, result } => match result {
                Ok(list) => self.send(client, ServerMsg::Reply(Reply::Worktrees(list))),
                Err(e) => self.send(client, ServerMsg::Error(e)),
            },
            Ev::MoveReady { client, term, branch, result } => {
                self.pending_ops -= 1;
                match result {
                    Ok((path, _)) => {
                        self.made_worktrees.push(path.clone());
                        self.worktree_hook(&path, true);
                        if let Some(t) = self.terms.get_mut(&term) {
                            t.pending_move = Some(path.clone());
                        }
                        self.poll_git_soon();
                        self.send(
                            client,
                            ServerMsg::Notice(format!(
                                "Worktree `{branch}` is ready at {}. Finish this turn; hydra then restarts you there, in this same conversation, and every later edit happens in that folder.",
                                path.display()
                            )),
                        );
                        self.send(client, ServerMsg::Reply(Reply::Ok));
                    }
                    Err(e) => self.send(client, ServerMsg::Error(e)),
                }
                self.dirty = true;
            }
            Ev::WorktreeAutoRemoved { client, path, result } => {
                self.pending_ops -= 1;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let msg = match result {
                    Ok(()) => format!("removed worktree {name} (its branch is kept)"),
                    Err(_) => format!("kept worktree {name}: it has uncommitted changes"),
                };
                self.send(client, ServerMsg::Notice(msg));
                self.dirty = true;
            }
            Ev::WorktreeRemoved { client, path, result } => {
                self.pending_ops -= 1;
                match result {
                    Ok(()) => {
                        self.send(client, ServerMsg::Notice(format!("removed worktree {}", path.display())));
                        self.send(client, ServerMsg::Reply(Reply::Ok));
                    }
                    Err(e) => self.send(client, ServerMsg::Error(e)),
                }
            }
        }
    }

    fn message(&mut self, client: ClientId, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello { .. } => {}
            ClientMsg::Input { term, data } => {
                if self.terms.get(&term).is_some_and(|t| t.asleep) {
                    // Wake it, and hand over what was typed once it's ready.
                    if let Some(new) = self.wake(term)
                        && let Some(t) = self.terms.get_mut(&new)
                    {
                        t.pending_input = Some((data, Instant::now() + Duration::from_secs(8)));
                    }
                } else if let Some(t) = self.terms.get_mut(&term) {
                    t.input(&data);
                }
            }
            ClientMsg::Resize { term, cols, rows } => {
                if let Some(t) = self.terms.get_mut(&term)
                    && (t.cols, t.rows) != (cols, rows)
                {
                    t.resize(cols, rows);
                    self.dirty = true;
                }
            }
            ClientMsg::Hook { term, ref token, pid, .. } => {
                // Only the pane's own processes may report its status. They carry its secret;
                // when the chain can be traced it must also lead back to the pane, so a desktop
                // app that inherited the pane's environment still can't. The chain is walked on
                // the hook thread (it's slow on Windows), in arrival order.
                let Some(pane) = self.terms.get(&term) else { return };
                if !term::same_secret(&pane.token, token) {
                    tracing::info!("ignoring a status report for pane {term} without its secret");
                    return;
                }
                let job = HookJob { root: pane.pid.filter(|_| pid != 0), pid, known: pane.trusted.clone(), msg };
                if self.hook_jobs.send(job).is_err() {
                    tracing::error!("the hook thread is gone; status reports are lost");
                }
            }
            ClientMsg::Command(cmd) => {
                match self.command(client, cmd) {
                    Ok(true) => self.send(client, ServerMsg::Reply(Reply::Ok)),
                    Ok(false) => {} // answered when the background work finishes
                    Err(e) => self.send(client, ServerMsg::Error(format!("{e:#}"))),
                }
                self.dirty = true;
            }
            ClientMsg::Query(q) => {
                let reply = match q {
                    Query::List => ServerMsg::Reply(Reply::List(self.snapshot())),
                    Query::Read { term } => match self.terms.get(&term) {
                        Some(t) => ServerMsg::Reply(Reply::Text(t.parser.screen().contents())),
                        None => ServerMsg::Error(format!("no pane {term}")),
                    },
                    Query::Worktrees { ws } => {
                        let Some(dir) = self.workspaces.iter().find(|w| w.id == ws).map(|w| w.cwd.clone()) else {
                            self.send(client, ServerMsg::Error(format!("no workspace {ws}")));
                            return;
                        };
                        let tx = self.tx.clone();
                        tokio::task::spawn_blocking(move || {
                            let result = git::list_worktrees(&dir).map_err(|e| format!("{e:#}"));
                            let _ = tx.blocking_send(Ev::WorktreeList { client, result });
                        });
                        return;
                    }
                };
                self.send(client, reply);
            }
        }
    }

    fn has_viewer(&self) -> bool {
        self.clients.values().any(|c| c.attach)
    }

    fn focused_term(&self) -> Option<TermId> {
        let ws = self.active_ws.and_then(|id| self.workspaces.iter().find(|w| w.id == id))?;
        ws.tab().map(|t| t.focus)
    }

    /// Run every extension's hook for `event` (in the background, quietly; failures are
    /// logged).
    fn ext_event(&self, event: &str, dir: Option<&std::path::Path>, term: Option<TermId>) {
        let jobs: Vec<(crate::ext::Ext, String)> =
            self.exts.iter().filter(|e| !e.hooks.get(event).trim().is_empty()).map(|e| (e.clone(), e.hooks.get(event).to_string())).collect();
        if jobs.is_empty() {
            return;
        }
        let info = term.and_then(|t| self.snapshot_term(t));
        let dir = dir.map(|d| d.to_path_buf()).or_else(|| term.and_then(|t| self.terms.get(&t)).map(|t| t.head.as_ref().map(|h| h.top.clone()).unwrap_or_else(|| t.cwd.clone()))).unwrap_or_else(home);
        let shell = self.cfg.shell_command();
        let event = event.to_string();
        tokio::task::spawn_blocking(move || {
            let vars = crate::ext::vars(&event, &dir, info.as_ref());
            for (e, cmd) in jobs {
                if let Err(err) = crate::ext::run(&shell, &e, &dir, &cmd, &vars) {
                    tracing::warn!("extension {} on {event}: {err}", e.name);
                }
            }
        });
    }

    /// One pane as clients see it.
    fn snapshot_term(&self, term: TermId) -> Option<TermInfo> {
        self.snapshot().terms.get(&term).cloned()
    }

    fn snapshot(&self) -> Snapshot {
        let terms: BTreeMap<TermId, TermInfo> = self
            .terms
            .values()
            .filter(|t| !t.spare)
            .map(|t| {
                (t.id, TermInfo {
                    id: t.id,
                    cols: t.cols,
                    rows: t.rows,
                    title: t.title().to_string(),
                    process: t.process.clone(),
                    remote: t.remote.clone(),
                    agent: t.agent.clone(),
                    status: t.status,
                    cwd: t.cwd.clone(),
                    summary: t.summary.clone(),
                    name: if t.name.is_empty() { t.first_prompt.clone() } else { t.name.clone() },
                    model: t.model.clone(),
                    label: t.label.clone(),
                    dev: t.dev.as_ref().map(|(d, _)| d.clone()),
                    mem: t.mem,
                    bell: t.bell,
                    said: t.said.clone(),
                    branch: t.head.as_ref().map(|h| h.branch.clone()),
                    linked: t.head.as_ref().is_some_and(|h| h.linked),
                    root: t.head.as_ref().map(|h| h.main_root.clone()),
                    top: t.head.as_ref().map(|h| h.top.clone()),
                    since: t.status_since,
                    asleep: t.asleep,
                    win32_input: t.win32_input,
                    subagents: t.subagents.iter().map(|(_, k)| k.clone()).collect(),
                })
            })
            .collect();
        Snapshot {
            workspaces: self.workspaces.clone(),
            active_ws: self.active_ws,
            terms,
            questions: self.questions.iter().map(|(q, ..)| q.clone()).collect(),
        }
    }

    fn sync_scan_roots(&self) {
        let roots = self.terms.values().filter_map(|t| t.pid.map(|p| (t.id, p))).collect();
        self.scan.lock().unwrap().roots = roots;
    }

    fn next(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Claude, started by hydra, learns how to move itself into a worktree (and may run
    /// just that command without asking).
    fn teach(&self, cmd: &str) -> String {
        let mut words = cmd.split_whitespace();
        let first = words.next().unwrap_or("");
        let is_claude = std::path::Path::new(first).file_stem().is_some_and(|s| s.eq_ignore_ascii_case("claude"));
        if !self.cfg.teach_agents || !is_claude || cmd.contains("--append-system-prompt") {
            return cmd.to_string();
        }
        let note = "You are running inside Hydra, a terminal where one person runs many coding agents. If the user asks you to do the work in a worktree (or on a separate branch so you don't touch their checkout), run `hydra worktree --move <short-branch-name>` with the Bash tool before editing anything, then end your turn: Hydra creates the worktree and restarts you there in this same conversation.";
        let rest = cmd[first.len()..].trim_start();
        format!(
            "{first} --append-system-prompt {} --allowedTools {} {rest}",
            self.cfg.quote_for_shell(note),
            self.cfg.quote_for_shell("Bash(hydra worktree:*)")
        )
        .trim_end()
        .to_string()
    }

    fn spawn(&mut self, cmd: Option<&str>, cwd: &std::path::Path, cols: u16, rows: u16) -> Result<TermId> {
        let taught = cmd.map(|c| self.teach(c));
        let cmd = taught.as_deref();
        let id = self.next();
        if let Some((dir, term)) = self.adopt.take() {
            if same_path(&dir, cwd) && self.terms.contains_key(&term) {
                if let Some(t) = self.terms.get_mut(&term) {
                    t.spare = false;
                    t.resize(cols, rows);
                }
                return Ok(term);
            }
            self.adopt = Some((dir, term));
        }
        let env = std::mem::take(&mut self.next_env);
        let t = Term::spawn(&self.cfg, SpawnSpec { id, cmd, cwd, cols, rows, env: &env }, self.tx.clone())?;
        self.terms.insert(id, t);
        self.had_terms = true;
        Ok(id)
    }

    /// Size for a brand-new full-tab pane: whatever the focused pane has, as a guess the
    /// client corrects immediately.
    fn guess_size(&self) -> (u16, u16) {
        self.focused_term()
            .and_then(|t| self.terms.get(&t))
            .map(|t| (t.cols, t.rows))
            .unwrap_or((120, 32))
    }

    fn ws_mut(&mut self, id: WsId) -> Result<&mut WorkspaceInfo> {
        self.workspaces.iter_mut().find(|w| w.id == id).ok_or_else(|| anyhow::anyhow!("no workspace {id}"))
    }

    fn locate(&self, term: TermId) -> Option<(WsId, TabId)> {
        self.workspaces
            .iter()
            .find_map(|w| w.tabs.iter().find(|t| t.layout.contains(term)).map(|t| (w.id, t.id)))
    }

    fn close_term(&mut self, term: TermId) {
        if let Some(t) = self.terms.get_mut(&term) {
            t.kill_tree();
        }
        self.remove_term(term);
    }

    /// Drop a terminal and prune the tree: empty tabs and workspaces disappear.
    fn remove_term(&mut self, term: TermId) {
        // Closing a terminal can block on Windows until its programs let go; never here.
        if let Some(t) = self.terms.remove(&term) {
            std::thread::spawn(move || drop(t));
        }
        self.detach(term);
        if self.terms.is_empty() {
            self.empty_since = Instant::now();
        }
    }

    /// Take a pane out of whatever tab holds it, pruning emptied tabs and workspaces. The
    /// terminal itself keeps running.
    fn detach(&mut self, term: TermId) {
        self.dirty = true;
        for w in &mut self.workspaces {
            let mut i = 0;
            while i < w.tabs.len() {
                let tab = &mut w.tabs[i];
                if !tab.layout.contains(term) {
                    i += 1;
                    continue;
                }
                let layout = std::mem::replace(&mut tab.layout, Node::Leaf(0));
                match layout.remove(term) {
                    Some(l) => {
                        if tab.focus == term {
                            tab.focus = l.first_leaf();
                        }
                        tab.layout = l;
                        i += 1;
                    }
                    None => {
                        let removed = w.tabs.remove(i).id;
                        if w.active_tab == removed {
                            let next = i.min(w.tabs.len().saturating_sub(1));
                            w.active_tab = w.tabs.get(next).map(|t| t.id).unwrap_or(0);
                        }
                    }
                }
            }
        }
        let before = self.workspaces.iter().position(|w| Some(w.id) == self.active_ws);
        self.workspaces.retain(|w| !w.tabs.is_empty());
        if self.active_ws.is_none_or(|id| !self.workspaces.iter().any(|w| w.id == id)) {
            let i = before.unwrap_or(0).min(self.workspaces.len().saturating_sub(1));
            self.active_ws = self.workspaces.get(i).map(|w| w.id);
        }
    }
}

/// Strip Windows' verbatim prefix (`\\?\C:\...`) that `canonicalize` adds; shells choke on it.
pub(crate) fn clean_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

#[cfg(test)]
mod logic_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn worktrees_of_trusted_repos_are_trusted() {
        let dir = std::env::temp_dir().join(format!("hydra-trust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("claude.json");
        std::fs::write(&f, r#"{"projects":{"c:/code/app":{"hasTrustDialogAccepted":true}},"other":1}"#).unwrap();
        super::trust_in(&f, std::path::Path::new("C:/code/app"), std::path::Path::new("C:/code/app-worktrees/x"));
        super::trust_in(&f, std::path::Path::new("C:/code/nope"), std::path::Path::new("C:/code/nope-wt"));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["projects"]["C:/code/app-worktrees/x"]["hasTrustDialogAccepted"], true);
        assert!(v["projects"].get("C:/code/nope-wt").is_none(), "never trusts what you didn't");
        assert_eq!(v["other"], 1, "the rest of the file is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
