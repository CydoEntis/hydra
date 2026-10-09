//! Commands from clients and the CLI.

use super::*;

impl Daemon {
    /// Apply a command. `Ok(false)` means the reply is sent later (background git work).
    pub(super) fn command(&mut self, client: ClientId, cmd: Command) -> Result<bool> {
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
                self.poll_git_soon();
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
                self.poll_git_soon();
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
                let port = dev.port.map(|base| self.known_port(&dir, base).unwrap_or_else(|| crate::project::port_for(&dir, base)));
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
                    let result = create_trusted_worktree(&dir, &b, None, &template);
                    let _ = tx.blocking_send(Ev::MoveReady { client, term, branch: b, result });
                });
                return Ok(false);
            }
            Command::RenamePane { term, name } => {
                if let Some(t) = self.terms.get_mut(&term) {
                    t.label = name.trim().chars().take(60).collect();
                }
            }
            Command::MarkSeen { term } => {
                if self.terms.get(&term).is_some_and(|t| t.status == Status::Done) {
                    self.set_status(term, Status::Idle);
                }
                if let Some(t) = self.terms.get_mut(&term) {
                    t.bell = false;
                }
            }
            Command::Reveal { term } => {
                self.command(client, Command::FocusPane { term })?;
                self.broadcast(|c| c.attach, ServerMsg::Raise);
            }
            Command::FocusPane { term } if self.terms.get(&term).is_some_and(|t| t.asleep) => {
                self.wake(term);
            }
            Command::FocusPane { term } => {
                let (ws, tab) = self.locate(term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                if let Some(t) = self.terms.get_mut(&term) {
                    t.bell = false;
                }
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
                let repo = crate::gitfs::head(&dir).map(|h| h.main_root);
                if base.is_none()
                    && let (Some(repo), Some(c)) = (&repo, &cmd)
                    && let Some(task) = self.spare_fits(repo, c)
                    && let Some((_, path, term)) = self.spare.take()
                {
                    // The spare takes the branch name asked for (off the loop; the reply waits
                    // for it), and the task if there is one.
                    save_spare(None);
                    if let Some(t) = self.terms.get_mut(&term)
                        && !task.is_empty()
                    {
                        t.pending_input = Some((format!("{task}\r").into_bytes(), Instant::now() + Duration::from_secs(20)));
                    }
                    self.adopt = Some((path.clone(), term));
                    self.adopted = Some(path.clone());
                    self.pending_ops += 1;
                    let tx = self.tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let renamed = std::process::Command::new("git").arg("-C").arg(&path).args(["branch", "-m", &branch]).output().is_ok_and(|o| o.status.success());
                        if !renamed {
                            tracing::warn!("couldn't rename the spare worktree's branch to {branch}");
                        }
                        let _ = tx.blocking_send(Ev::WorktreeCreated { client, branch, cmd, split, result: Ok((path, String::new())) });
                    });
                    self.prewarm(repo.clone());
                    return Ok(false);
                }
                if let Some(repo) = repo {
                    self.prewarm(repo);
                }
                let tx = self.tx.clone();
                self.pending_ops += 1;
                tokio::task::spawn_blocking(move || {
                    let result =
                        create_trusted_worktree(&dir, &branch, base.as_deref(), &template);
                    let _ = tx.blocking_send(Ev::WorktreeCreated { client, branch, cmd, split, result });
                });
                return Ok(false);
            }
            Command::AgentWorktree { dir } => {
                // Only for a shell starting an agent: not from outside a pane, and not for an
                // agent an agent starts (it works where its parent does).
                let dir = clean_path(dir);
                let from = self.clients.get(&client).and_then(|c| c.from).and_then(|t| self.terms.get(&t));
                let wanted = self.cfg.worktree.per_agent
                    && from.is_some_and(|t| t.agent.is_none())
                    && crate::gitfs::head(&dir).is_some_and(|h| !h.linked);
                if !wanted {
                    self.send(client, ServerMsg::Reply(Reply::Text(String::new())));
                    return Ok(false);
                }
                let template = self.cfg.worktree.dir.clone();
                let tx = self.tx.clone();
                self.pending_ops += 1;
                tokio::task::spawn_blocking(move || {
                    let result = agent_worktree(&dir, &template);
                    let _ = tx.blocking_send(Ev::AgentWorktreeMade { client, result });
                });
                return Ok(false);
            }
            Command::Enqueue { item } => self.enqueue(item),
            Command::Dequeue { id } => self.queue.retain(|q| q.id != id),
            Command::CloseWorktree { path } => {
                let path = clean_path(path);
                let head = crate::gitfs::head(&path).filter(|h| h.linked).ok_or_else(|| anyhow::anyhow!("{} is not a linked git worktree", path.display()))?;
                let terms: Vec<TermId> = self.terms.values().filter(|t| t.head.as_ref().is_some_and(|h| same_path(&h.top, &head.top))).map(|t| t.id).collect();
                for t in terms {
                    self.close_term(t);
                }
                self.made_worktrees.retain(|m| !same_path(m, &head.top));
                let tx = self.tx.clone();
                self.pending_ops += 1;
                let hook = self.hook_job(&head.top, false);
                self.ext_event("worktree_remove", Some(&head.top), None);
                let top = head.top;
                tokio::task::spawn_blocking(move || {
                    if let Some(h) = hook {
                        let _ = h();
                    }
                    // Give the closed programs a moment to let go of the folder (Windows locks it).
                    std::thread::sleep(Duration::from_millis(800));
                    let result = git::remove_worktree(&top, false, true).map_err(|e| format!("{e:#}"));
                    let _ = tx.blocking_send(Ev::WorktreeRemoved { client, path: top, result });
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
                self.ext_event("worktree_remove", Some(&path), None);
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
                self.poll_git_soon();
            }
            Command::UndoAutoWorkspace => {
                let undo = self.auto_undo.take().ok_or_else(|| anyhow::anyhow!("nothing to undo"))?;
                self.undo_auto(undo)?;
            }
            Command::Popup { cmd, cwd } => {
                let cwd = cwd.map(clean_path).filter(|p| p.is_dir()).unwrap_or_else(home);
                let (cols, rows) = self.guess_size();
                self.next_once = true;
                // Most of the screen; the client sizes it exactly once it's drawn.
                let term = self.spawn(Some(&cmd), &cwd, cols.saturating_mul(4) / 5, rows.saturating_mul(3) / 4)?;
                if let Some(t) = self.terms.get_mut(&term) {
                    t.popup = true;
                }
                self.dirty = true;
            }
            Command::TeachAgent { term } => {
                let pid = self.terms.get(&term).and_then(|t| t.pid).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                let (program, args) = scan::leaf_program(pid).ok_or_else(|| anyhow::anyhow!("nothing is running in that pane"))?;
                let def = crate::config::agent_from_command(&program, &args)
                    .ok_or_else(|| anyhow::anyhow!("{program} has nothing to recognise it by; add it under [[agents]] in config.toml"))?;
                crate::config::add_agent(&def)?;
                let name = def.name.clone();
                self.command(client, Command::ReloadConfig)?;
                self.broadcast(|c| c.attach, ServerMsg::Notice(format!("{name} is an agent now: seshi shows when it's working, needs you or done")));
            }
            Command::AskHuman { term, text, options } => {
                let options = if options.is_empty() { vec!["Yes".to_string(), "No".to_string()] } else { options };
                let id = self.next() as u64;
                let before = self.terms.get(&term).map(|t| t.status).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?;
                self.questions.push((HumanQuestion { id, term, text, options }, client, before));
                // It needs you: the sidebar, a notification, the Inbox.
                self.set_status(term, Status::Blocked);
                self.dirty = true;
                return Ok(false);
            }
            Command::AnswerHuman { id, choice } => {
                let i = self.questions.iter().position(|(q, ..)| q.id == id).ok_or_else(|| anyhow::anyhow!("that question was already answered"))?;
                let (q, asker, _) = self.questions.remove(i);
                let answer = q.options.get(choice).cloned().ok_or_else(|| anyhow::anyhow!("no answer {choice}"))?;
                self.send(asker, ServerMsg::Reply(Reply::Text(answer)));
                // Back to work with the answer.
                self.set_status(q.term, Status::Working);
                self.dirty = true;
            }
            Command::Grant { term, grants } => {
                let known = ["read", "write", "start", "respond", "admin"];
                if let Some(bad) = grants.iter().flatten().find(|g| !known.contains(&g.as_str())) {
                    anyhow::bail!("no such grant: {bad} (read, write, start, respond, admin)");
                }
                self.terms.get_mut(&term).ok_or_else(|| anyhow::anyhow!("no pane {term}"))?.grants = grants;
            }
            Command::ReloadConfig => {
                self.exts = crate::ext::load_all().0;
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
                    self.save_outputs(true);
                }
                for t in self.terms.values_mut() {
                    t.kill_tree();
                }
                if let Some((_, path, _)) = self.spare.take() {
                    // Shutting down, so there's nothing else to serve: let the spare's agent
                    // release its folder, then remove it before the process ends.
                    std::thread::sleep(Duration::from_millis(300));
                    drop_spare_dir(&path);
                    save_spare(None);
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
    pub(super) fn ws_repo(&self, w: &WorkspaceInfo) -> PathBuf {
        w.git.as_ref().map(|g| g.root.clone()).or_else(|| repo_of(&w.cwd)).unwrap_or_else(|| w.cwd.clone())
    }

    /// An agent just started in `term`. If its folder is inside a repo that its workspace
    /// isn't, move the pane to that repo's workspace, creating it if needed.
    pub(super) fn auto_workspace(&mut self, term: TermId) {
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
        self.poll_git_soon();
        self.dirty = true;
        self.broadcast(|c| c.attach, ServerMsg::AutoWorkspace(msg));
    }

    pub(super) fn tab_focus(&self, ws: WsId, tab: TabId) -> Option<TermId> {
        self.workspaces.iter().find(|w| w.id == ws)?.tabs.iter().find(|t| t.id == tab).map(|t| t.focus)
    }

    pub(super) fn undo_auto(&mut self, undo: AutoUndo) -> Result<()> {
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
        self.poll_git_soon();
        Ok(())
    }
}
