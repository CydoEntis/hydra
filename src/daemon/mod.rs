//! The daemon: owns every PTY and the workspace tree, survives clients detaching.
//!
//! All state lives in one event loop (`Daemon::run`). PTY reader threads, the process
//! scanner, background git work and client connections talk to it through a single
//! channel, so there is no locking around the model.

mod git;
mod persist;
mod scan;
mod term;

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

pub enum Ev {
    Connected(ClientId, mpsc::UnboundedSender<ServerMsg>, bool),
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
    /// A worktree made for an agent to move into: (client, pane, branch, result).
    MoveReady { client: ClientId, term: TermId, branch: String, result: Result<(PathBuf, String), String> },
}

/// How to put back what an automatic workspace changed.
enum AutoUndo {
    /// The pane moved out of `from_ws`/`from_tab` (beside `beside`).
    Moved { term: TermId, from_ws: WsId, from_tab: TabId, beside: Option<TermId> },
    /// The pane was alone, so its workspace was re-homed; restore name and folder.
    Rehomed { ws: WsId, name: String, cwd: PathBuf },
}

/// The repository a folder belongs to: its main checkout (worktrees resolve to the repo
/// they were made from). `None` outside git.
fn repo_of(dir: &std::path::Path) -> Option<PathBuf> {
    let top = dir.ancestors().find(|a| a.join(".git").exists())?;
    let dot = top.join(".git");
    if dot.is_dir() {
        return Some(top.to_path_buf());
    }
    // A linked worktree: ".git" is a file "gitdir: <main>/.git/worktrees/<name>".
    let text = std::fs::read_to_string(&dot).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let main_git = gitdir.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git"))?;
    main_git.parent().map(|p| p.to_path_buf())
}

/// Claude asks "do you trust this folder?" for every new folder, and every new worktree is
/// one. A worktree of a repo you already trust is trusted the same way (never otherwise).
fn trust_like_repo(repo: &std::path::Path, worktree: &std::path::Path) {
    if let Some(home) = directories::BaseDirs::new() {
        trust_in(&home.home_dir().join(".claude.json"), repo, worktree);
    }
}

fn trust_in(file: &std::path::Path, repo: &std::path::Path, worktree: &std::path::Path) {
    let file = file.to_path_buf();
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let key = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_string();
    let projects = v.get("projects");
    let trusted = |k: &str| projects.and_then(|p| p.get(k)).and_then(|e| e.get("hasTrustDialogAccepted")).and_then(|b| b.as_bool()) == Some(true);
    let (rk, wk) = (key(repo), key(worktree));
    // Claude's keys may differ in the drive letter's case.
    let repo_ok = trusted(&rk) || projects.and_then(|p| p.as_object()).is_some_and(|m| m.iter().any(|(k, e)| k.eq_ignore_ascii_case(&rk) && e.get("hasTrustDialogAccepted").and_then(|b| b.as_bool()) == Some(true)));
    if !repo_ok || trusted(&wk) {
        return;
    }
    let Some(map) = v.get_mut("projects").and_then(|p| p.as_object_mut()) else { return };
    let entry = map.entry(wk).or_insert_with(|| serde_json::json!({}));
    if let Some(o) = entry.as_object_mut() {
        o.insert("hasTrustDialogAccepted".into(), serde_json::Value::Bool(true));
    }
    // Write beside and swap, so a reader never sees half a file.
    let tmp = file.with_extension("json.hydra-tmp");
    if let Ok(s) = serde_json::to_string_pretty(&v)
        && std::fs::write(&tmp, s).is_ok()
    {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// Claude files conversations under `~/.claude/projects/<folder slug>/<session>.jsonl`, the
/// slug being the folder with every non-alphanumeric character as `-`. Put a copy where a
/// resume in `dest` will look.
fn copy_claude_transcript(src: &std::path::Path, session: &str, dest: &std::path::Path) {
    let Some(projects) = src.parent().and_then(|p| p.parent()) else { return };
    let slug: String = dest.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let dir = projects.join(slug);
    let to = dir.join(format!("{session}.jsonl"));
    if !to.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::copy(src, &to);
    }
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    n(a) == n(b)
}

struct Client {
    tx: mpsc::UnboundedSender<ServerMsg>,
    attach: bool,
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
    scan: Arc<Mutex<scan::Shared>>,
    dirty: bool,
    had_terms: bool,
    empty_since: Instant,
    /// Background git work in flight; the server stays up until it lands.
    pending_ops: u32,
    git_busy: bool,
    last_git: Instant,
    last_save: Instant,
    last_saved: String,
    /// Worktrees hydra created; closing the last thing in one removes it.
    made_worktrees: Vec<PathBuf>,
    /// Extra environment for the next pane spawned (a dev server's PORT).
    next_env: Vec<(String, String)>,
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

        let (cfg, err) = Config::load_or_default();
        if let Some(e) = err {
            tracing::warn!("config: {e}");
        }
        let agents = cfg.agent_defs();
        let scan = Arc::new(Mutex::new(scan::Shared {
            roots: Vec::new(),
            agents: agents.clone(),
            interval: Duration::from_millis(cfg.detection.scan_interval_ms),
        }));
        scan::start(scan.clone(), tx.clone());

        let mut d = Daemon {
            cfg,
            agents,
            terms: HashMap::new(),
            workspaces: Vec::new(),
            active_ws: None,
            next_id: 1,
            clients: HashMap::new(),
            tx,
            scan,
            dirty: false,
            had_terms: false,
            empty_since: Instant::now(),
            pending_ops: 0,
            git_busy: false,
            last_git: Instant::now() - Duration::from_secs(60),
            last_save: Instant::now(),
            last_saved: String::new(),
            made_worktrees: Vec::new(),
            next_env: Vec::new(),
            last_sleep_check: Instant::now(),
            natural_exits: Vec::new(),
            auto_undo: None,
        };
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
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();
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

impl Daemon {
    async fn run(&mut self, mut rx: mpsc::Receiver<Ev>) {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
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
                    if self.last_sleep_check.elapsed() >= Duration::from_secs(30) {
                        self.last_sleep_check = Instant::now();
                        self.sleep_idle();
                    }
                    self.poll_git();
                    if self.last_save.elapsed() > Duration::from_secs(2) {
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
                std::process::exit(0);
            }
        }
    }

    fn should_exit(&self) -> bool {
        if !self.terms.is_empty() || self.pending_ops > 0 {
            return false;
        }
        // Exit when the last pane closes, or if nobody ever started one.
        self.had_terms || (self.clients.is_empty() && self.empty_since.elapsed() > Duration::from_secs(30))
    }

    fn broadcast(&self, filter: impl Fn(&Client) -> bool, msg: ServerMsg) {
        for c in self.clients.values().filter(|c| filter(c)) {
            let _ = c.tx.send(msg.clone());
        }
    }

    fn send(&self, client: ClientId, msg: ServerMsg) {
        if let Some(c) = self.clients.get(&client) {
            let _ = c.tx.send(msg);
        }
    }

    // ---- persistence ---------------------------------------------------------------

    fn mass_exit(&self) -> bool {
        self.natural_exits.iter().filter(|t| t.elapsed() < Duration::from_secs(5)).count() >= 2
    }

    /// Write the session file if it changed. `force` skips the mass-exit guard.
    fn persist(&mut self, force: bool) {
        if !self.cfg.restore.enabled || (!force && self.mass_exit()) {
            return;
        }
        let saved = self.saved();
        let Ok(text) = serde_json::to_string(&saved) else { return };
        if text == self.last_saved {
            return;
        }
        match persist::save(&saved) {
            Ok(()) => self.last_saved = text,
            Err(e) => tracing::warn!("saving session: {e:#}"),
        }
    }

    fn saved(&self) -> persist::Saved {
        let pane = |term: &Term| persist::SavedPane {
            cwd: Some(term.cwd.clone()),
            cmd: term.cmd.clone(),
            agent: term.agent.clone(),
            session: term.agent.as_ref().and(term.session.clone()),
            unseen: term.status == Status::Done,
            name: if term.name.is_empty() { term.first_prompt.clone() } else { term.name.clone() },
            model: term.model.clone(),
        };
        let workspaces = self
            .workspaces
            .iter()
            .map(|w| persist::SavedWs {
                name: w.name.clone(),
                cwd: w.cwd.clone(),
                worktree: w.worktree,
                color: Some(w.color),
                group: w.group.clone(),
                active_tab: w.tabs.iter().position(|t| t.id == w.active_tab).unwrap_or(0),
                tabs: w
                    .tabs
                    .iter()
                    .map(|t| persist::SavedTab {
                        name: t.name.clone(),
                        layout: t.layout.clone(),
                        focus: t.focus,
                        panes: t
                            .layout
                            .leaves()
                            .iter()
                            .filter_map(|id| self.terms.get(id))
                            .map(|term| (term.id, pane(term)))
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let active = self.workspaces.iter().position(|w| Some(w.id) == self.active_ws).unwrap_or(0);
        persist::Saved { workspaces, active, made_worktrees: self.made_worktrees.clone() }
    }

    /// The command that brings a saved pane back: an agent's resume command, the pane's
    /// original command, or nothing (a shell).
    fn restore_cmd(&self, p: &persist::SavedPane) -> Option<String> {
        if self.cfg.restore.agents
            && let Some(def) = p.agent.as_ref().and_then(|n| self.agents.iter().find(|a| &a.name == n))
        {
            if let (Some(tpl), Some(id)) = (&def.resume, &p.session) {
                return Some(tpl.replace("{session}", id));
            }
            if let Some(tpl) = &def.resume_last {
                return Some(tpl.clone());
            }
        }
        if self.cfg.restore.commands { p.cmd.clone() } else { None }
    }

    fn restore(&mut self, mut saved: persist::Saved) {
        self.made_worktrees = std::mem::take(&mut saved.made_worktrees);
        let mut restored = 0;
        for sw in saved.workspaces {
            let mut tabs = Vec::new();
            for st in sw.tabs {
                let mut map = HashMap::new();
                for old in st.layout.leaves() {
                    let pane = st.panes.get(&old).cloned().unwrap_or_default();
                    let cwd = pane.cwd.clone().filter(|p| p.is_dir()).unwrap_or_else(|| sw.cwd.clone());
                    let cmd = self.restore_cmd(&pane);
                    match self.spawn(cmd.as_deref(), &cwd, 120, 32) {
                        Ok(new) => {
                            if let Some(t) = self.terms.get_mut(&new) {
                                // Keep the original identity so the next save matches this one.
                                t.cmd = pane.cmd.clone();
                                t.session = pane.session.clone();
                                t.agent = pane.agent.clone();
                                t.restore_unseen = pane.unseen;
                                t.first_prompt = pane.name.clone();
                                t.model = pane.model.clone();
                            }
                            map.insert(old, new);
                            restored += 1;
                        }
                        Err(e) => tracing::warn!("restoring pane in {}: {e:#}", cwd.display()),
                    }
                }
                let Some(layout) = st.layout.map_leaves(&mut |old| map.get(&old).copied()) else { continue };
                let focus = map.get(&st.focus).copied().unwrap_or_else(|| layout.first_leaf());
                let id = self.next();
                tabs.push(TabInfo { id, name: st.name, layout, focus });
            }
            if tabs.is_empty() {
                continue;
            }
            let active_tab = tabs[sw.active_tab.min(tabs.len() - 1)].id;
            let id = self.next();
            let color = sw.color.unwrap_or_else(|| self.free_color());
            self.workspaces.push(WorkspaceInfo {
                id,
                // Older sessions named panes after their folder; treat that as no name.
                name: if sw.cwd.file_name().is_some_and(|n| n.to_string_lossy() == sw.name) { String::new() } else { sw.name },
                cwd: sw.cwd,
                tabs,
                active_tab,
                git: None,
                worktree: sw.worktree,
                color,
                is_new: false,
                group: sw.group.clone(),
            });
        }
        self.active_ws = self.workspaces.get(saved.active).or(self.workspaces.first()).map(|w| w.id);
        self.dirty = true;
        tracing::info!("restored {} workspaces, {restored} panes", self.workspaces.len());
    }

    /// The first palette colour no workspace is using (cycling once all are taken).
    fn free_color(&self) -> u8 {
        let n = self.cfg.ui.workspace_colors.len().clamp(1, 255) as u8;
        (0..n)
            .find(|c| !self.workspaces.iter().any(|w| w.color == *c))
            .unwrap_or((self.workspaces.len() % n as usize) as u8)
    }

    fn poll_git(&mut self) {
        if self.git_busy || self.workspaces.is_empty() || self.last_git.elapsed() < Duration::from_secs(3) {
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
        match ev {
            Ev::Connected(id, tx, attach) => {
                if attach {
                    let _ = tx.send(ServerMsg::State(self.snapshot()));
                    for t in self.terms.values() {
                        let _ = tx.send(ServerMsg::Replay { term: t.id, cols: t.cols, rows: t.rows, data: t.replay() });
                    }
                }
                self.clients.insert(id, Client { tx, attach });
            }
            Ev::Disconnected(id) => {
                self.clients.remove(&id);
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
                self.broadcast(|c| c.attach, ServerMsg::Output { term: tid, data });
            }
            Ev::Exited(tid) => {
                // A sleeping agent's process was stopped on purpose; its pane stays.
                if self.terms.get(&tid).is_some_and(|t| t.asleep) {
                    return;
                }
                if self.terms.contains_key(&tid) {
                    self.natural_exits.retain(|t| t.elapsed() < Duration::from_secs(5));
                    self.natural_exits.push(Instant::now());
                    self.remove_term(tid);
                }
            }
            Ev::Scan(found) => {
                let mut started = Vec::new();
                for f in found {
                    let Some(t) = self.terms.get_mut(&f.term) else { continue };
                    if t.process != f.process && !f.process.is_empty() {
                        t.process = f.process;
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
                    if let Some(h) = crate::gitfs::head(p) {
                        trust_like_repo(&h.main_root, p);
                    }
                    self.worktree_hook(p, true);
                }
                match opened {
                    Ok(path) => {
                        if as_pane.is_none()
                            && let Some(w) = self.workspaces.last_mut()
                        {
                            w.worktree = true;
                        }
                        self.last_git = Instant::now() - Duration::from_secs(60);
                        self.send(client, ServerMsg::Notice(format!("worktree {branch} at {}", path.display())));
                        self.send(client, ServerMsg::Reply(Reply::Ok));
                    }
                    Err(e) => self.send(client, ServerMsg::Error(format!("{e:#}"))),
                }
                self.dirty = true;
            }
            Ev::HookRan(msg) => self.broadcast(|_| true, ServerMsg::Notice(msg)),
            Ev::WorktreeList { client, result } => match result {
                Ok(list) => self.send(client, ServerMsg::Reply(Reply::Worktrees(list))),
                Err(e) => self.send(client, ServerMsg::Error(e)),
            },
            Ev::MoveReady { client, term, branch, result } => {
                self.pending_ops -= 1;
                match result {
                    Ok((path, _)) => {
                        self.made_worktrees.push(path.clone());
                        let repo = self.terms.get(&term).and_then(|t| t.head.as_ref().map(|h| h.main_root.clone()));
                        if let Some(repo) = repo {
                            trust_like_repo(&repo, &path);
                        }
                        self.worktree_hook(&path, true);
                        if let Some(t) = self.terms.get_mut(&term) {
                            t.pending_move = Some(path.clone());
                        }
                        self.last_git = Instant::now() - Duration::from_secs(60);
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
            ClientMsg::Hook { term, agent, status, session, cwd, prompt, said, subagent, event, pid, transcript, model, name } => {
                // Only the pane's own processes may report its status (a desktop app that
                // inherited the pane's environment can't).
                if let Some(tp) = self.terms.get(&term).and_then(|t| t.pid)
                    && pid != 0
                {
                    let known = self.terms.get(&term).map(|t| t.trusted.clone()).unwrap_or_default();
                    match scan::descends_from(pid, tp, &known) {
                        (Some(true), chain) => {
                            if let Some(t) = self.terms.get_mut(&term) {
                                t.trusted.extend(chain);
                            }
                        }
                        // Outside the pane, or a chain that can't be traced back to it (the
                        // reporter sends its live parent, so a real one always can be).
                        (_, _) => {
                            tracing::info!("ignoring a status report for pane {term} from pid {pid} outside it");
                            return;
                        }
                    }
                }
                if let Some(t) = self.terms.get_mut(&term) {
                    if let Some(sa) = subagent {
                        if sa.start {
                            t.subagents.push((sa.id, sa.kind));
                        } else if let Some(i) = t.subagents.iter().position(|(id, _)| !sa.id.is_empty() && *id == sa.id) {
                            t.subagents.remove(i);
                        } else {
                            t.subagents.pop();
                        }
                        self.dirty = true;
                    }
                    // A turn over (or a new session) has no subagents left; a "done" while some
                    // still run is held instead (see `hook`).
                    if matches!(status, HookStatus::Gone | HookStatus::Idle) {
                        t.subagents.clear();
                    }
                    if let Some(s) = said {
                        t.said = s;
                    }
                    if let Some(tr) = transcript.filter(|p| p.is_file()) {
                        t.transcript = Some(tr);
                    }
                    if let Some(p) = prompt.filter(|p| !p.trim().is_empty()) {
                        if t.first_prompt.is_empty() {
                            t.first_prompt = p.clone();
                        }
                        t.summary = p;
                    }
                    if let Some(m) = model {
                        t.model = m;
                    }
                    if let Some(n) = name {
                        t.name = n;
                    }
                    // Ids end up in a shell command on restore: keep them boring.
                    if let Some(id) = session.filter(|s| {
                        !s.is_empty() && s.len() < 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                    }) {
                        t.session = Some(id);
                    }
                    if let Some(c) = cwd.filter(|c| c.is_dir()) {
                        t.cwd = c;
                        t.cwd_reported = true;
                        t.refresh_head();
                    }
                }
                self.hook(term, agent, status, &event);
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

    fn hook(&mut self, term: TermId, agent: String, status: HookStatus, event: &str) {
        let focused = self.focused_term() == Some(term) && self.has_viewer();
        if matches!(status, HookStatus::Done | HookStatus::Idle)
            && let Some(dest) = self.terms.get_mut(&term).and_then(|t| t.pending_move.take())
        {
            self.relocate(term, &dest);
            return;
        }
        let Some(t) = self.terms.get_mut(&term) else { return };
        let now = Instant::now();
        let new = match status {
            HookStatus::Same => {
                // A subagent finished: if the turn was waiting on it, it's done now.
                if t.subagents.is_empty() && t.done_held.take().is_some() {
                    let s = if focused { Status::Idle } else { Status::Done };
                    self.set_status(term, s);
                }
                self.dirty = true;
                return;
            }
            // Claude sometimes repeats a permission ping after you've already answered and it's
            // moved on: ignore one that comes right after a "working".
            HookStatus::Blocked
                if event == "Notification:permission_prompt" && t.last_working_hook.is_some_and(|w| w.elapsed() < Duration::from_secs(5)) =>
            {
                return;
            }
            // Done while subagents still run: hold it until they finish (or 3 minutes pass).
            HookStatus::Done if !t.subagents.is_empty() => {
                t.done_held = Some(now);
                self.dirty = true;
                return;
            }
            // After a restart, a session that was done and unseen comes back done.
            HookStatus::Idle if std::mem::take(&mut t.restore_unseen) && !focused => Status::Done,
            HookStatus::Gone => {
                t.hooked = false;
                t.agent = None;
                t.session = None;
                t.summary.clear();
                t.status = Status::None;
                t.status_since = term::unix_now();
                self.dirty = true;
                return;
            }
            HookStatus::Working => Status::Working,
            HookStatus::Blocked => Status::Blocked,
            HookStatus::Idle => Status::Idle,
            HookStatus::Done if focused => Status::Idle,
            HookStatus::Done => Status::Done,
        };
        t.hooked = true;
        if !agent.is_empty() {
            t.agent = Some(agent);
        }
        if new == Status::Working {
            t.last_working_hook = Some(now);
            t.done_held = None;
            t.progress_off = None;
        }
        if new != Status::Working {
            t.progress_off = None;
        }
        self.set_status(term, new);
        self.dirty = true;
    }

    fn set_status(&mut self, term: TermId, new: Status) {
        let Some(t) = self.terms.get_mut(&term) else { return };
        if t.status == new {
            return;
        }
        t.status = new;
        t.status_since = term::unix_now();
        self.dirty = true;
        if matches!(new, Status::Blocked | Status::Done) {
            self.broadcast(|c| c.attach, ServerMsg::Attention { term, status: new });
            // No window open: the server tells you itself.
            if !self.has_viewer()
                && let Some(t) = self.terms.get(&term)
            {
                let agent = t.agent.clone().unwrap_or_else(|| "an agent".into());
                let place = t.head.as_ref().map(|h| h.top.clone()).unwrap_or_else(|| t.cwd.clone());
                let place = place.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let summary = t.summary.trim();
                let body = if summary.is_empty() { place } else { format!("{place} · {summary}") };
                let (kind, what) = if new == Status::Blocked { (crate::alert::Kind::Needs, "needs you") } else { (crate::alert::Kind::Done, "finished") };
                crate::alert::alert(&self.cfg.notify, kind, &format!("{agent} {what}"), &body);
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

    /// Heuristic statuses for agents without hooks, and "seen" bookkeeping for everyone.
    /// A worktree hook (`on_create` / `on_remove` in the repo's .hydra.toml) as a job to run.
    fn hook_job(&self, dir: &std::path::Path, create: bool) -> Option<impl FnOnce() -> String + Send + 'static> {
        let proj = crate::project::load(dir);
        let cmd = if create { proj.hooks.on_create } else { proj.hooks.on_remove };
        if cmd.trim().is_empty() {
            return None;
        }
        let shell = self.cfg.shell_command();
        let dir = dir.to_path_buf();
        let base = proj.dev.and_then(|d| d.port);
        Some(move || {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut vars = vec![("HYDRA_WORKTREE", dir.display().to_string())];
            if let Some(h) = crate::gitfs::head(&dir) {
                vars.push(("HYDRA_BRANCH", h.branch.clone()));
                vars.push(("HYDRA_REPO", h.main_root.display().to_string()));
            }
            if let Some(b) = base {
                vars.push(("PORT", crate::project::port_for(&dir, b).to_string()));
            }
            let what = if create { "on_create" } else { "on_remove" };
            match crate::project::run_hook(&shell, &dir, &cmd, &vars) {
                Ok(()) => format!("{name}: {what} hook done ({cmd})"),
                Err(e) => format!("{name}: {what} hook failed: {e}"),
            }
        })
    }

    /// Run a worktree hook in the background and say how it went.
    fn worktree_hook(&self, dir: &std::path::Path, create: bool) {
        if let Some(job) = self.hook_job(dir, create) {
            let tx = self.tx.clone();
            tokio::task::spawn_blocking(move || {
                let _ = tx.blocking_send(Ev::HookRan(job()));
            });
        }
    }

    fn update_statuses(&mut self) {
        // Dev servers: up once their output shows the ready pattern (and which port, if it
        // prints a localhost URL).
        for t in self.terms.values_mut() {
            if let Some((d, re)) = &mut t.dev
                && !d.ready
            {
                let text = {
                    let screen = t.parser.screen();
                    screen.contents()
                };
                if re.as_ref().is_some_and(|r| r.is_match(&text)) {
                    d.ready = true;
                    if d.port.is_none()
                        && let Some(p) = regex::Regex::new(r"(?:localhost|127\.0\.0\.1):(\d{2,5})").ok().and_then(|r| r.captures(&text)).and_then(|c| c[1].parse().ok())
                    {
                        d.port = Some(p);
                    }
                    self.dirty = true;
                }
            }
        }
        // Messages typed to a sleeping agent: deliver once it's ready (its hooks say idle) or
        // after a few seconds.
        for t in self.terms.values_mut() {
            let ready = matches!(t.status, Status::Idle | Status::Done) && t.hooked;
            if let Some((data, by)) = t.pending_input.take() {
                if ready || Instant::now() >= by {
                    // Text, then Enter a beat later, so it lands as a message, not a paste.
                    if data.len() > 1 && data.ends_with(b"\r") {
                        t.input(&data[..data.len() - 1]);
                        t.pending_input = Some((b"\r".to_vec(), Instant::now()));
                    } else {
                        t.input(&data);
                    }
                } else {
                    t.pending_input = Some((data, by));
                }
            }
        }
        let focused = self.has_viewer().then(|| self.focused_term()).flatten();
        let window = Duration::from_millis(self.cfg.detection.working_window_ms);
        let grace = Duration::from_millis(self.cfg.detection.echo_grace_ms);
        let rows = self.cfg.detection.pattern_rows;
        let mut changes = Vec::new();
        for t in self.terms.values() {
            let Some(name) = &t.agent else { continue };
            if t.hooked {
                if t.status == Status::Done && Some(t.id) == focused {
                    changes.push((t.id, Status::Idle));
                }
                // Subagents never reported back: don't hold "done" forever.
                if t.done_held.is_some_and(|h| h.elapsed() > Duration::from_secs(180)) {
                    changes.push((t.id, if Some(t.id) == focused { Status::Idle } else { Status::Done }));
                }
                // The progress indicator went away and no turn-end came: cancelled (Esc).
                if t.status == Status::Working && t.done_held.is_none() && t.progress_off.is_some_and(|p| p.elapsed() > Duration::from_secs(2)) {
                    changes.push((t.id, Status::Idle));
                }
                continue;
            }
            let def = self.agents.iter().find(|a| &a.name == name);
            let text = t.tail_text(rows);
            let blocked = def.is_some_and(|d| d.blocked.iter().any(|r| r.is_match(&text)));
            let working = match def {
                Some(d) if !d.working.is_empty() => d.working.iter().any(|r| r.is_match(&text)),
                _ => {
                    t.last_output.elapsed() < window
                        && t.last_output.saturating_duration_since(t.last_input) > grace
                }
            };
            let new = if blocked {
                Status::Blocked
            } else if working {
                Status::Working
            } else if Some(t.id) == focused {
                Status::Idle
            } else if matches!(t.status, Status::Working | Status::Done) {
                Status::Done
            } else {
                Status::Idle
            };
            if new != t.status {
                changes.push((t.id, new));
            }
        }
        for (id, s) in changes {
            if s != Status::Working
                && let Some(t) = self.terms.get_mut(&id)
            {
                if t.done_held.take().is_some() {
                    t.subagents.clear();
                }
                t.progress_off = None;
            }
            self.set_status(id, s);
        }
    }

    fn snapshot(&self) -> Snapshot {
        let terms: BTreeMap<TermId, TermInfo> = self
            .terms
            .values()
            .map(|t| {
                (t.id, TermInfo {
                    id: t.id,
                    cols: t.cols,
                    rows: t.rows,
                    title: t.title().to_string(),
                    process: t.process.clone(),
                    agent: t.agent.clone(),
                    status: t.status,
                    cwd: t.cwd.clone(),
                    summary: t.summary.clone(),
                    name: if t.name.is_empty() { t.first_prompt.clone() } else { t.name.clone() },
                    model: t.model.clone(),
                    dev: t.dev.as_ref().map(|(d, _)| d.clone()),
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
        Snapshot { workspaces: self.workspaces.clone(), active_ws: self.active_ws, terms }
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

    /// Apply a command. `Ok(false)` means the reply is sent later (background git work).
    fn command(&mut self, client: ClientId, cmd: Command) -> Result<bool> {
        match cmd {
            Command::NewWorkspace { cwd, name, cmd } => {
                let cwd = cwd.map(clean_path).filter(|p| p.is_dir()).unwrap_or_else(home);
                let (cols, rows) = self.guess_size();
                let term = self.spawn(cmd.as_deref(), &cwd, cols, rows)?;
                let id = self.next();
                let tab = self.next();
                let color = self.free_color();
                // No name: the sidebar shows where the pane is until the user renames it.
                let name = name.filter(|n| !n.is_empty()).unwrap_or_default();
                self.workspaces.push(WorkspaceInfo {
                    id,
                    name,
                    cwd,
                    tabs: vec![TabInfo { id: tab, name: String::new(), layout: Node::Leaf(term), focus: term }],
                    active_tab: tab,
                    git: None,
                    worktree: false,
                    color,
                    is_new: false,
                    group: None,
                });
                self.active_ws = Some(id);
                self.last_git = Instant::now() - Duration::from_secs(60);
            }
            Command::CloseWorkspace { ws } => {
                let terms: Vec<TermId> = self
                    .workspaces
                    .iter()
                    .filter(|w| w.id == ws)
                    .flat_map(|w| w.tabs.iter().flat_map(|t| t.layout.leaves()))
                    .collect();
                let tops = self.worktrees_of(&terms);
                for t in terms {
                    self.close_term(t);
                }
                self.cleanup_worktrees(client, tops);
            }
            Command::RenameWorkspace { ws, name } => self.ws_mut(ws)?.name = name,
            Command::SetGroup { ws, group } => self.ws_mut(ws)?.group = group.filter(|g| !g.trim().is_empty()),
            Command::SetWorkspaceColor { ws, color } => self.ws_mut(ws)?.color = color,
            Command::SelectWorkspace { ws } => {
                self.ws_mut(ws)?.is_new = false;
                self.active_ws = Some(ws);
            }
            Command::NewTab { ws, name, cmd } => {
                let cwd = self.ws_mut(ws)?.cwd.clone();
                let (cols, rows) = self.guess_size();
                let term = self.spawn(cmd.as_deref(), &cwd, cols, rows)?;
                let id = self.next();
                let w = self.ws_mut(ws)?;
                w.tabs.push(TabInfo { id, name: name.unwrap_or_default(), layout: Node::Leaf(term), focus: term });
                w.active_tab = id;
                self.active_ws = Some(ws);
            }
            Command::CloseTab { ws, tab } => {
                let terms: Vec<TermId> = self
                    .ws_mut(ws)?
                    .tabs
                    .iter()
                    .filter(|t| t.id == tab)
                    .flat_map(|t| t.layout.leaves())
                    .collect();
                for t in terms {
                    self.close_term(t);
                }
            }
            Command::RenameTab { ws, tab, name } => {
                if let Some(t) = self.ws_mut(ws)?.tabs.iter_mut().find(|t| t.id == tab) {
                    t.name = name;
                }
            }
            Command::SelectTab { ws, tab } => {
                let w = self.ws_mut(ws)?;
                if w.tabs.iter().any(|t| t.id == tab) {
                    w.active_tab = tab;
                }
                self.active_ws = Some(ws);
            }
            Command::Split { term, dir, cmd, cwd } => {
                let (ws, tab) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                // New panes start where asked, else where the pane being split is.
                let ws_cwd = self.ws_mut(ws)?.cwd.clone();
                let cwd = cwd
                    .map(clean_path)
                    .filter(|c| c.is_dir())
                    .or_else(|| self.terms.get(&term).map(|t| t.cwd.clone()).filter(|c| c.is_dir()))
                    .unwrap_or(ws_cwd);
                let (cols, rows) = self.terms.get(&term).map(|t| (t.cols, t.rows)).unwrap_or((80, 24));
                let (cols, rows) = if dir.horizontal() { (cols / 2, rows) } else { (cols, rows / 2) };
                let new = self.spawn(cmd.as_deref(), &cwd, cols, rows)?;
                let w = self.ws_mut(ws)?;
                if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tab) {
                    t.layout.split(term, dir, new);
                    t.focus = new;
                }
            }
            Command::ClosePane { term } => {
                let tops = self.worktrees_of(&[term]);
                self.close_term(term);
                self.cleanup_worktrees(client, tops);
            }
            Command::PaneToWorkspace { term } => {
                let (ws, _) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                let cwd = self.terms.get(&term).map(|t| t.cwd.clone()).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                let name = cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| cwd.display().to_string());
                let alone = self.ws_mut(ws)?.tabs.iter().map(|t| t.layout.leaves().len()).sum::<usize>() == 1;
                if alone {
                    // Nothing to move: the workspace simply moves to where its pane is.
                    let w = self.ws_mut(ws)?;
                    w.cwd = cwd;
                    w.name = name;
                    w.git = None;
                    w.worktree = false;
                } else {
                    self.detach(term);
                    let id = self.next();
                    let tab = self.next();
                    let color = self.free_color();
                    self.workspaces.push(WorkspaceInfo {
                        id,
                        name,
                        cwd,
                        tabs: vec![TabInfo { id: tab, name: String::new(), layout: Node::Leaf(term), focus: term }],
                        active_tab: tab,
                        git: None,
                        worktree: false,
                        color,
                        is_new: false,
                        group: None,
                    });
                    self.active_ws = Some(id);
                }
                self.last_git = Instant::now() - Duration::from_secs(60);
            }
            Command::Dev { dir, action } => {
                let key = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
                let running: Vec<TermId> = self.terms.values().filter(|t| t.dev.as_ref().is_some_and(|(d, _)| key(&d.dir) == key(&dir))).map(|t| t.id).collect();
                if matches!(action, DevAction::Stop | DevAction::Restart) {
                    if running.is_empty() && action == DevAction::Stop {
                        anyhow::bail!("no dev server is running in {}", dir.display());
                    }
                    for t in running.iter().copied() {
                        self.close_term(t);
                    }
                } else if !running.is_empty() {
                    anyhow::bail!("its dev server is already running");
                }
                if action == DevAction::Stop {
                    self.dirty = true;
                    return Ok(true);
                }
                let proj = crate::project::load(&dir);
                let dev = proj.dev.filter(|d| !d.run.trim().is_empty()).ok_or_else(|| {
                    anyhow::anyhow!("no dev server set up: add [dev] run = \"...\" to {} in the repo", crate::project::FILE)
                })?;
                let port = dev.port.map(|base| crate::project::port_for(&dir, base));
                self.next_env = port.map(|p| vec![("PORT".to_string(), p.to_string())]).unwrap_or_default();
                // Its own tab in the checkout's workspace, so it never takes screen space.
                let ws = self
                    .workspaces
                    .iter()
                    .find(|w| w.tabs.iter().flat_map(|t| t.layout.leaves()).any(|id| self.terms.get(&id).and_then(|t| t.head.as_ref()).is_some_and(|h| key(&h.top) == key(&dir))))
                    .map(|w| w.id);
                let (cols, rows) = self.guess_size();
                let term = self.spawn(Some(&dev.run), &dir, cols, rows)?;
                let ready_re = Some(dev.ready.trim()).filter(|r| !r.is_empty()).and_then(|r| regex::RegexBuilder::new(r).case_insensitive(true).build().ok());
                if let Some(t) = self.terms.get_mut(&term) {
                    t.dev = Some((DevInfo { dir: dir.clone(), port, ready: ready_re.is_none() }, ready_re));
                }
                let id = self.next();
                let tab = TabInfo { id, name: "dev".into(), layout: Node::Leaf(term), focus: term };
                match ws.and_then(|ws| self.workspaces.iter_mut().find(|w| w.id == ws)) {
                    Some(w) => w.tabs.push(tab),
                    None => {
                        let ws = self.next();
                        let (color, _) = (self.workspaces.len() as u8, ());
                        self.workspaces.push(WorkspaceInfo {
                            id: ws,
                            name: format!("dev · {}", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                            cwd: dir.clone(),
                            tabs: vec![tab],
                            active_tab: id,
                            git: None,
                            worktree: false,
                            color,
                            is_new: false,
                            group: None,
                        });
                    }
                }
                self.dirty = true;
                return Ok(true);
            }
            Command::MoveToWorktree { term, branch } => {
                let t = self.terms.get(&term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                if t.agent.is_none() {
                    anyhow::bail!("only an agent can move into a worktree");
                }
                if self.resume_cmd(t).is_none() {
                    anyhow::bail!("this agent can't be resumed elsewhere (no session id yet; try after its first turn)");
                }
                let dir = t.head.as_ref().map(|h| h.main_root.clone()).unwrap_or_else(|| t.cwd.clone());
                let branch = branch.unwrap_or_else(|| {
                    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
                    format!("{}-{:04x}", t.agent.clone().unwrap_or_else(|| "agent".into()), n & 0xffff)
                });
                let template = self.cfg.worktree.dir.clone();
                let tx = self.tx.clone();
                self.pending_ops += 1;
                let b = branch.clone();
                tokio::task::spawn_blocking(move || {
                    let result = git::create_worktree(&dir, &b, None, &template).map_err(|e| format!("{e:#}"));
                    let _ = tx.blocking_send(Ev::MoveReady { client, term, branch: b, result });
                });
                return Ok(false);
            }
            Command::MarkSeen { term } => {
                if self.terms.get(&term).is_some_and(|t| t.status == Status::Done) {
                    self.set_status(term, Status::Idle);
                }
            }
            Command::FocusPane { term } if self.terms.get(&term).is_some_and(|t| t.asleep) => {
                self.wake(term);
            }
            Command::FocusPane { term } => {
                let (ws, tab) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                let w = self.ws_mut(ws)?;
                w.active_tab = tab;
                w.is_new = false;
                if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tab) {
                    t.focus = term;
                }
                self.active_ws = Some(ws);
                if self.terms.get(&term).is_some_and(|t| t.status == Status::Done) {
                    self.set_status(term, Status::Idle);
                }
            }
            Command::ResizePane { term, dir, delta } => {
                let (ws, tab) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                if let Some(t) = self.ws_mut(ws)?.tabs.iter_mut().find(|t| t.id == tab) {
                    t.layout.resize(term, dir, delta);
                }
            }
            Command::NewWorktree { ws, branch, base, cmd, split, from } => {
                let ws_dir = self.ws_mut(ws)?.cwd.clone();
                let dir = from.map(clean_path).filter(|d| d.is_dir()).unwrap_or(ws_dir);
                let template = self.cfg.worktree.dir.clone();
                let cmd = cmd.or_else(|| Some(self.cfg.worktree.command.clone()).filter(|c| !c.is_empty()));
                let tx = self.tx.clone();
                self.pending_ops += 1;
                tokio::task::spawn_blocking(move || {
                    let result =
                        git::create_worktree(&dir, &branch, base.as_deref(), &template).map_err(|e| format!("{e:#}"));
                    let _ = tx.blocking_send(Ev::WorktreeCreated { client, branch, cmd, split, result });
                });
                return Ok(false);
            }
            Command::RemoveWorktree { ws, force, delete_branch } => {
                let w = self.ws_mut(ws)?;
                let path = w.cwd.clone();
                if !w.worktree && !w.git.as_ref().is_some_and(|g| g.linked) {
                    anyhow::bail!("{} is not a linked git worktree", path.display());
                }
                let terms: Vec<TermId> = w.tabs.iter().flat_map(|t| t.layout.leaves()).collect();
                for t in terms {
                    self.close_term(t);
                }
                let tx = self.tx.clone();
                self.pending_ops += 1;
                let hook = self.hook_job(&path, false);
                tokio::task::spawn_blocking(move || {
                    if let Some(h) = hook {
                        let _ = h();
                    }
                    let result = git::remove_worktree(&path, force, delete_branch).map_err(|e| format!("{e:#}"));
                    let _ = tx.blocking_send(Ev::WorktreeRemoved { client, path, result });
                });
                return Ok(false);
            }
            Command::MovePane { term, to, name } => {
                let (from_ws, _) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                if to == Some(from_ws) {
                    return Ok(true);
                }
                let cwd = self.terms.get(&term).map(|t| t.cwd.clone()).unwrap_or_else(home);
                self.detach(term);
                match to.filter(|id| self.workspaces.iter().any(|w| w.id == *id)) {
                    // Into an existing group: beside its focused pane.
                    Some(target) => {
                        let w = self.ws_mut(target)?;
                        let tab_id = w.active_tab;
                        match w.tabs.iter_mut().find(|t| t.id == tab_id) {
                            Some(t) => {
                                let beside = t.focus;
                                t.layout.split(beside, crate::layout::Dir::Right, term);
                                t.focus = term;
                            }
                            None => {
                                let id = self.next_id;
                                self.next_id += 1;
                                let w = self.ws_mut(target)?;
                                w.tabs.push(TabInfo { id, name: String::new(), layout: Node::Leaf(term), focus: term });
                                w.active_tab = id;
                            }
                        }
                        self.active_ws = Some(target);
                    }
                    // A new group holding just this pane.
                    None => {
                        let id = self.next();
                        let tab = self.next();
                        let color = self.free_color();
                        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                            cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "group".into())
                        });
                        self.workspaces.push(WorkspaceInfo {
                            id,
                            name,
                            cwd,
                            tabs: vec![TabInfo { id: tab, name: String::new(), layout: Node::Leaf(term), focus: term }],
                            active_tab: tab,
                            git: None,
                            worktree: false,
                            color,
                            is_new: false,
                            group: None,
                        });
                        self.active_ws = Some(id);
                    }
                }
                self.last_git = Instant::now() - Duration::from_secs(60);
            }
            Command::UndoAutoWorkspace => {
                let undo = self.auto_undo.take().ok_or_else(|| anyhow::anyhow!("nothing to undo"))?;
                self.undo_auto(undo)?;
            }
            Command::ReloadConfig => {
                self.cfg = Config::load()?;
                self.agents = self.cfg.agent_defs();
                let mut s = self.scan.lock().unwrap();
                s.agents = self.agents.clone();
                s.interval = Duration::from_millis(self.cfg.detection.scan_interval_ms);
            }
            Command::KillServer { forget } => {
                tracing::info!("kill-server requested (forget: {forget})");
                if forget {
                    persist::forget();
                } else {
                    self.persist(true);
                }
                for t in self.terms.values_mut() {
                    t.kill_tree();
                }
                self.broadcast(|_| true, ServerMsg::Bye);
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    std::process::exit(0);
                });
            }
        }
        Ok(true)
    }

    /// The repo (main checkout) a workspace stands for, or its folder outside git.
    fn ws_repo(&self, w: &WorkspaceInfo) -> PathBuf {
        w.git.as_ref().map(|g| g.root.clone()).or_else(|| repo_of(&w.cwd)).unwrap_or_else(|| w.cwd.clone())
    }

    /// An agent just started in `term`. If its folder is inside a repo that its workspace
    /// isn't, move the pane to that repo's workspace, creating it if needed.
    fn auto_workspace(&mut self, term: TermId) {
        let Some(cwd) = self.terms.get(&term).map(|t| t.cwd.clone()) else { return };
        let Some(repo) = repo_of(&cwd) else { return };
        let Some((ws, tab)) = self.locate(term) else { return };
        let Some(cur) = self.workspaces.iter().find(|w| w.id == ws) else { return };
        if same_path(&self.ws_repo(cur), &repo) {
            return;
        }
        let agent = self.terms.get(&term).and_then(|t| t.agent.clone()).unwrap_or_else(|| "an agent".into());
        let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| repo.display().to_string());
        let alone = cur.tabs.iter().map(|t| t.layout.leaves().len()).sum::<usize>() == 1;
        let target = self.workspaces.iter().find(|w| w.id != ws && same_path(&self.ws_repo(w), &repo)).map(|w| w.id);
        
        let msg = match (target, alone) {
            // The repo already has a workspace: the pane joins it.
            (Some(target), _) => {
                let beside = self.tab_focus(ws, tab).filter(|f| *f != term);
                self.detach(term);
                if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == target) {
                    let id = self.next_id;
                    self.next_id += 1;
                    w.tabs.push(TabInfo { id, name: String::new(), layout: Node::Leaf(term), focus: term });
                    w.active_tab = id;
                }
                self.active_ws = Some(target);
                self.auto_undo = Some(AutoUndo::Moved { term, from_ws: ws, from_tab: tab, beside });
                format!("You started {agent} in {}, so it joined {name}.", cwd.display())
            }
            // Its own workspace was just this pane: re-home that workspace.
            (None, true) => {
                let w = self.workspaces.iter_mut().find(|w| w.id == ws).unwrap();
                self.auto_undo = Some(AutoUndo::Rehomed { ws, name: w.name.clone(), cwd: w.cwd.clone() });
                w.name = name.clone();
                w.cwd = repo.clone();
                w.git = None;
                w.is_new = true;
                format!("You started {agent} in {}, so it became a workspace.", cwd.display())
            }
            // Otherwise a new workspace for the repo, holding the pane.
            (None, false) => {
                let beside = self.tab_focus(ws, tab).filter(|f| *f != term);
                self.detach(term);
                let id = self.next();
                let tid = self.next();
                let color = self.free_color();
                self.workspaces.push(WorkspaceInfo {
                    id,
                    name,
                    cwd: repo.clone(),
                    tabs: vec![TabInfo { id: tid, name: String::new(), layout: Node::Leaf(term), focus: term }],
                    active_tab: tid,
                    git: None,
                    worktree: false,
                    color,
                    is_new: true,
                    group: None,
                });
                self.active_ws = Some(id);
                self.auto_undo = Some(AutoUndo::Moved { term, from_ws: ws, from_tab: tab, beside });
                format!("You started {agent} in {}, so it became a workspace.", cwd.display())
            }
        };
        self.last_git = Instant::now() - Duration::from_secs(60);
        self.dirty = true;
        self.broadcast(|c| c.attach, ServerMsg::AutoWorkspace(msg));
    }

    fn tab_focus(&self, ws: WsId, tab: TabId) -> Option<TermId> {
        self.workspaces.iter().find(|w| w.id == ws)?.tabs.iter().find(|t| t.id == tab).map(|t| t.focus)
    }

    fn undo_auto(&mut self, undo: AutoUndo) -> Result<()> {
        match undo {
            AutoUndo::Rehomed { ws, name, cwd } => {
                let w = self.ws_mut(ws)?;
                w.name = name;
                w.cwd = cwd;
                w.git = None;
                w.is_new = false;
            }
            AutoUndo::Moved { term, from_ws, from_tab, beside } => {
                if !self.terms.contains_key(&term) {
                    anyhow::bail!("that pane has closed");
                }
                self.detach(term);
                match self.workspaces.iter_mut().find(|w| w.id == from_ws) {
                    Some(w) => {
                        let tab = w.tabs.iter_mut().find(|t| t.id == from_tab);
                        match (tab, beside) {
                            (Some(t), Some(b)) if t.layout.contains(b) => {
                                t.layout.split(b, crate::layout::Dir::Right, term);
                                t.focus = term;
                                w.active_tab = from_tab;
                            }
                            _ => {
                                let id = self.next_id;
                                self.next_id += 1;
                                w.tabs.push(TabInfo { id, name: String::new(), layout: Node::Leaf(term), focus: term });
                                w.active_tab = id;
                            }
                        }
                        self.active_ws = Some(from_ws);
                    }
                    None => anyhow::bail!("its old workspace has closed"),
                }
            }
        }
        self.last_git = Instant::now() - Duration::from_secs(60);
        Ok(())
    }

    /// The command that resumes an agent's conversation, if its kind supports it.
    fn resume_cmd(&self, t: &Term) -> Option<String> {
        let def = t.agent.as_ref().and_then(|n| self.agents.iter().find(|a| &a.name == n))?;
        match (&def.resume, &t.session) {
            (Some(tpl), Some(id)) => Some(tpl.replace("{session}", id)),
            _ => def.resume_last.clone(),
        }
    }

    /// Stop agents that have sat finished or idle longer than `sleep_after` (never the
    /// focused one, and only ones that can be resumed).
    fn sleep_idle(&mut self) {
        let Some(after) = self.cfg.sleep_secs() else { return };
        let now = term::unix_now();
        // The one on screen stays awake; with no window open, nobody is looking.
        let focused = if self.has_viewer() { self.focused_term() } else { None };
        let sleepy: Vec<TermId> = self
            .terms
            .values()
            .filter(|t| !t.asleep && t.agent.is_some() && Some(t.id) != focused)
            .filter(|t| matches!(t.status, Status::Idle | Status::Done) && now.saturating_sub(t.status_since) >= after)
            .filter(|t| self.resume_cmd(t).is_some())
            .map(|t| t.id)
            .collect();
        for id in sleepy {
            if let Some(t) = self.terms.get_mut(&id) {
                tracing::info!("putting {} to sleep", id);
                t.asleep = true;
                t.kill_tree();
                self.dirty = true;
            }
        }
    }

    /// Bring a sleeping agent back: start its resume command in the same spot.
    fn wake(&mut self, old: TermId) -> Option<TermId> {
        let t = self.terms.get(&old)?;
        let cmd = self.resume_cmd(t).or_else(|| t.cmd.clone());
        let (cwd, cols, rows) = (t.cwd.clone(), t.cols, t.rows);
        let (agent, session, orig) = (t.agent.clone(), t.session.clone(), t.cmd.clone());
        let new = match self.spawn(cmd.as_deref(), &cwd, cols, rows) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("waking {old}: {e:#}");
                return None;
            }
        };
        if let Some(n) = self.terms.get_mut(&new) {
            n.agent = agent;
            n.session = session;
            n.cmd = orig;
        }
        for w in &mut self.workspaces {
            for tab in &mut w.tabs {
                if tab.layout.contains(old) {
                    if let Some(l) = tab.layout.map_leaves(&mut |id| Some(if id == old { new } else { id })) {
                        tab.layout = l;
                    }
                    if tab.focus == old {
                        tab.focus = new;
                    }
                }
            }
        }
        self.terms.remove(&old);
        if let Some((ws, tab)) = self.locate(new) {
            if let Ok(w) = self.ws_mut(ws) {
                w.active_tab = tab;
            }
            self.active_ws = Some(ws);
        }
        self.dirty = true;
        Some(new)
    }

    /// Restart an agent in another folder, in the same conversation: its transcript is put
    /// where the agent looks for that folder (Claude files them by folder), then it resumes
    /// in place of the old pane and is told where it now is.
    fn relocate(&mut self, old: TermId, dest: &std::path::Path) {
        let Some(t) = self.terms.get(&old) else { return };
        if let (Some(src), Some(session)) = (&t.transcript, &t.session) {
            copy_claude_transcript(src, session, dest);
        }
        let cmd = self.resume_cmd(t);
        let (cols, rows) = (t.cols, t.rows);
        let (agent, session, orig) = (t.agent.clone(), t.session.clone(), t.cmd.clone());
        if let Some(t) = self.terms.get_mut(&old) {
            t.kill_tree();
        }
        let new = match self.spawn(cmd.as_deref(), dest, cols, rows) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("moving {old} to {}: {e:#}", dest.display());
                return;
            }
        };
        if let Some(n) = self.terms.get_mut(&new) {
            n.agent = agent;
            n.session = session;
            n.cmd = orig;
            let note = format!(
                "You've been moved into the git worktree at {}. Your conversation continues here; make every further change in this folder. Carry on with the task.",
                dest.display()
            );
            // Only once it's up (its hooks say so), never into a startup question.
            n.pending_input = Some((format!("{note}\r").into_bytes(), Instant::now() + Duration::from_secs(45)));
        }
        for w in &mut self.workspaces {
            for tab in &mut w.tabs {
                if tab.layout.contains(old) {
                    if let Some(l) = tab.layout.map_leaves(&mut |id| Some(if id == old { new } else { id })) {
                        tab.layout = l;
                    }
                    if tab.focus == old {
                        tab.focus = new;
                    }
                }
            }
            if w.tabs.iter().any(|tab| tab.layout.contains(new)) {
                w.cwd = dest.to_path_buf();
            }
        }
        self.remove_term(old);
        self.last_git = Instant::now() - Duration::from_secs(60);
        self.dirty = true;
    }

    /// The linked worktrees these terminals are in.
    fn worktrees_of(&self, terms: &[TermId]) -> Vec<PathBuf> {
        terms
            .iter()
            .filter_map(|t| self.terms.get(t))
            .filter_map(|t| t.head.as_ref().filter(|h| h.linked).map(|h| h.top.clone()))
            .collect()
    }

    /// After you close something: remove worktrees hydra made that nothing runs in any
    /// more. Git refuses if there are uncommitted changes, and the branch is always kept.
    fn cleanup_worktrees(&mut self, client: ClientId, tops: Vec<PathBuf>) {
        if !self.cfg.worktree.delete_with_last {
            return;
        }
        for top in tops {
            if !self.made_worktrees.iter().any(|m| same_path(m, &top)) {
                continue;
            }
            if self.terms.values().any(|t| t.head.as_ref().is_some_and(|h| same_path(&h.top, &top))) {
                continue;
            }
            self.made_worktrees.retain(|m| !same_path(m, &top));
            let tx = self.tx.clone();
            self.pending_ops += 1;
            tokio::task::spawn_blocking(move || {
                // Give the closed programs a moment to let go of the folder (Windows locks it).
                std::thread::sleep(Duration::from_millis(800));
                let result = git::remove_worktree(&top, false, false).map_err(|e| format!("{e:#}"));
                let _ = tx.blocking_send(Ev::WorktreeAutoRemoved { client, path: top, result });
            });
        }
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
