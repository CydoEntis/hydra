//! What each action does, and the helpers they share.

use super::*;

impl App {
    pub(super) fn act(&mut self, a: Action) {
        // These read files on this machine; over ssh the files are on the other one.
        if crate::ipc::remote().is_some()
            && matches!(a, Action::Files | Action::Changes | Action::Find(_) | Action::Branches | Action::PasteImage | Action::Ship)
        {
            self.notify(format!("{} isn't available over ssh yet (agents, panes and worktrees are)", a.describe()), true);
            return;
        }
        if self.hy_act(&a) {
            return;
        }
        let ws = self.active_ws().map(|w| w.id);
        let focused = self.focused();
        match a {
            Action::PasteImage => self.paste_image(),
            Action::Find(tab) => self.open_find(tab),
            Action::Branches => self.open_branches(None),
            Action::History => self.mode = Mode::History { sel: 0 },
            Action::SpawnTab(c) => {
                if let Some(ws) = ws {
                    self.cmd(Command::NewTab { ws, name: None, cmd: Some(c) });
                }
            }
            Action::ClosePane => {
                if let Some(term) = focused {
                    self.menu_act(menu::Act::End(vec![term]));
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
            Action::CloseWorkspace => {
                if let Some(ws) = ws {
                    self.mode = Mode::Prompt { kind: PromptKind::ConfirmCloseWorkspace(ws), input: String::new() };
                }
            }
            Action::Palette => self.mode = Mode::Picker { query: String::new(), sel: 0, commands: true },
            Action::StopAgent => {
                if let Some(term) = self.target_term() {
                    self.send(ClientMsg::Input { term, data: vec![3] });
                }
            }
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
            Action::RemoveWorktree => {
                let Some(w) = self.active_ws() else { return };
                if !w.worktree && !w.git.as_ref().is_some_and(|g| g.linked) {
                    self.notify("this workspace is not a linked worktree".into(), true);
                    return;
                }
                self.mode = Mode::Prompt { kind: PromptKind::ConfirmRemoveWorktree(w.id), input: String::new() };
            }
            Action::Detach => self.quit = Some("detached".into()),
            Action::Help => self.open_keymap(None),
            Action::ReloadConfig => self.reload_config(),
            Action::PaneInfo => {
                if let Some(msg) = self.focused().and_then(|t| self.pane_info(t)) {
                    super::copy::to_clipboard(&msg);
                    self.notify(format!("{msg} (copied)"), true);
                }
            }
            Action::SendPrefix => {
                let p = self.keymap.prefix;
                let ev = KeyEvent::new(p.code, p.mods);
                self.forward_key(&ev);
            }
            Action::KillServer => {
                self.mode = Mode::Prompt { kind: PromptKind::ConfirmKillServer, input: String::new() };
            }
            Action::Update => self.ask_update(),
            Action::None => {}
            // The rest are handled by hy_act above.
            _ => {}
        }
    }



    /// Run an extension's command about the worktree you're in: hidden (its last line is
    /// shown), or in a pane beside.
    pub(super) fn run_ext(&mut self, ei: usize, ci: usize) {
        let Some(e) = self.exts.get(ei).cloned() else { return };
        let Some(c) = e.commands.get(ci).cloned() else { return };
        let term = self.focused();
        let info = term.and_then(|t| self.snap.terms.get(&t)).cloned();
        let dir = info.as_ref().map(|t| t.top.clone().unwrap_or_else(|| t.cwd.clone())).unwrap_or_else(|| self.here_dir());
        if c.background {
            let shell = self.cfg.shell_command();
            self.notify(format!("{}…", c.title), false);
            self.spawn_bg(move || {
                let vars = crate::ext::vars("command", &dir, info.as_ref());
                Bg::ExtDone(crate::ext::run(&shell, &e, &dir, &c.run, &vars))
            });
        } else {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "seshi".into());
            let t = term.map(|t| format!(" --term {t}")).unwrap_or_default();
            let cmd = format!("{} ext run {} {ci}{t}", self.cfg.quote_for_shell(&exe), self.cfg.quote_for_shell(&e.name));
            let cmd = if self.cfg.shell_command()[0].to_lowercase().contains("powershell") || self.cfg.shell_command()[0].to_lowercase().contains("pwsh") { format!("& {cmd}") } else { cmd };
            self.hy_new_session(dir, Some(cmd), true);
        }
    }

    /// Ask extensions for their worktree labels when they're due.
    pub(super) fn refresh_ext_labels(&mut self, worktrees: Vec<(String, PathBuf)>) {
        if self.exts.iter().all(|e| e.labels.is_empty()) {
            return;
        }
        let mut n = 0;
        for (ei, e) in self.exts.clone().into_iter().enumerate() {
            for (li, l) in e.labels.iter().enumerate() {
                let slot = ei * 100 + li;
                for (key, path) in &worktrees {
                    let due = self.ext_labels.get(&(key.clone(), slot)).is_none_or(|(_, at)| at.elapsed().as_secs() >= l.every.max(5));
                    if !due {
                        continue;
                    }
                    let old = self.ext_labels.get(&(key.clone(), slot)).map(|x| x.0.clone()).unwrap_or_default();
                    self.ext_labels.insert((key.clone(), slot), (old, Instant::now()));
                    let (shell, e, l, key, path) = (self.cfg.shell_command(), e.clone(), l.clone(), key.clone(), path.clone());
                    self.spawn_bg(move || {
                        let vars = crate::ext::vars("label", &path, None);
                        let text = crate::ext::run(&shell, &e, &path, &l.run, &vars).map(|o| o.lines().next().unwrap_or("").trim().chars().take(24).collect()).unwrap_or_default();
                        Bg::ExtLabel(key, slot, text)
                    });
                    n += 1;
                    // A few at a time.
                    if n >= 8 {
                        return;
                    }
                }
            }
        }
    }

    pub(super) fn reload_config(&mut self) {
        match Config::load() {
            Ok(cfg) => {
                self.keymap = cfg.keymap();
                self.theme = cfg.theme();
                // Only what the file changed: a sidebar hidden with a key stays hidden.
                if cfg.ui.sidebar != self.cfg.ui.sidebar {
                    self.sidebar = cfg.ui.sidebar;
                }
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

    /// "C-b %" for an action bound after the leader, "M-h" for a global one.
    pub(super) fn key_for(&self, a: &Action) -> String {
        if let Some(k) = self.keymap.prefixed_order.iter().find(|k| self.keymap.prefixed.get(k) == Some(a)) {
            return format!("{} {k}", self.keymap.prefix);
        }
        self.keymap.global.iter().find(|(_, x)| *x == a).map(|(k, _)| k.to_string()).unwrap_or_default()
    }

    /// Every command worth offering in the palette, plus your own spawn bindings.
    pub(super) fn palette_commands(&self) -> Vec<Action> {
        use crate::layout::Dir;
        vec![
            Action::Jump,
            Action::BrowseTree,
            Action::NewPane,
            Action::ShellHere,
            Action::SplitRight,
            Action::SplitDown,
            Action::Focus(Dir::Left),
            Action::Focus(Dir::Right),
            Action::Focus(Dir::Up),
            Action::Focus(Dir::Down),
            Action::Zoom,
            Action::ToggleSidebar,
            Action::ClosePane,
            Action::NewTab,
            Action::NextTab,
            Action::PrevTab,
            Action::CloseTab,
            Action::RenameWorkspace,
            Action::Files,
            Action::Find(0),
            Action::Find(1),
            Action::Changes,
            Action::Branches,
            Action::PullRequest,
            Action::Ship,
            Action::CopyMode,
            Action::PasteImage,
            Action::History,
            Action::Settings,
            Action::Help,
            Action::ReloadConfig,
            Action::PaneInfo,
            Action::Update,
            Action::Detach,
        ]
    }

    /// The workspace a command applies to: the one on screen.
    pub(super) fn target_ws(&self) -> Option<WsId> {
        self.snap.active_ws
    }

    /// The terminal a command applies to: the target pane's focused one.
    pub(super) fn target_term(&self) -> Option<TermId> {
        self.target_ws().and_then(|ws| self.snap.workspace(ws)).and_then(|w| w.tab()).map(|t| t.focus).or(self.focused())
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
        let key = design::path_key(&head.main_root);
        let pr = self.hy.prs.get(&key).and_then(|l| l.iter().find(|p| p.branch == head.branch)).map(|p| p.number.to_string());
        let top = head.top.clone();
        self.spawn_bg(move || {
            let changed = std::process::Command::new("git")
                .arg("-C")
                .arg(&top)
                .args(["status", "--porcelain"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
                .unwrap_or(0);
            Bg::Then(Box::new(move |app: &mut App| app.mode = Mode::Ship(Box::new(ShipAsk { task, changed, pr }))))
        });
    }

    pub(super) fn open_changes(&mut self, dir: PathBuf) {
        let head = crate::gitfs::head(&dir);
        // Not a git repo: say so, and offer the next step.
        if head.is_none() {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.mode = Mode::Confirm(Box::new(menu::Confirm {
                title: "changes".into(),
                sub: name,
                what: format!("{} isn't a git repository yet.", dir.display()),
                detail: String::new(),
                note: "Git lets you review and undo what agents change here.".into(),
                list: Vec::new(),
                yes: "Make it a git repo".into(),
                key: 'g',
                danger: false,
                act: menu::Act::GitInit(dir),
            }));
            return;
        }
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
            reviewed: Default::default(),
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

    /// A path an agent printed (Ctrl+click): relative to where it runs. Text opens in the file
    /// viewer at that file (and line); anything else in its own app.
    pub(super) fn open_path_from(&mut self, term: TermId, raw: &str, line: Option<u32>) {
        let info = self.snap.terms.get(&term);
        let bases: Vec<PathBuf> = [info.map(|t| t.cwd.clone()), info.and_then(|t| t.top.clone()), info.and_then(|t| t.root.clone())].into_iter().flatten().collect();
        let raw = raw.trim_start_matches("./").replace('/', std::path::MAIN_SEPARATOR_STR);
        let p = PathBuf::from(&raw);
        let found = if p.is_absolute() { Some(p).filter(|p| p.exists()) } else { bases.iter().map(|b| b.join(&raw)).find(|p| p.exists()) };
        let Some(path) = found else {
            self.notify(format!("no file {raw} where this pane runs"), true);
            return;
        };
        if path.is_dir() {
            self.open_files(path);
            return;
        }
        let text = files::looks_like_text(&path);
        if text {
            let dir = path.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            self.open_files(dir);
            if let Some(View::Files(v)) = &mut self.view {
                v.reveal = Some((path, line));
            }
        } else {
            let _ = files::open_default(&path);
            self.notify(format!("opening {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), false);
        }
    }

    pub(super) fn open_files(&mut self, dir: PathBuf) {
        let head = crate::gitfs::head(&dir);
        let root = head.as_ref().map(|h| h.top.clone()).unwrap_or(dir);
        let branch = head.map(|h| h.branch);
        self.view = Some(View::Files(Box::new(views::FilesTree::new(root.clone(), branch))));
        self.spawn_bg(move || Bg::Tree(root.clone(), views::scan_tree(&root), views::git_marks(&root)));
    }

    /// The pull request of a branch (or a number), in the main area.
    pub(super) fn open_pr(&mut self, dir: PathBuf, which: String) {
        self.view = Some(View::Pr(Box::new(pr::PrView { dir: dir.clone(), which: which.clone(), info: None, diff: None, tab: 0, scroll: 0 })));
        self.spawn_bg(move || Bg::Pr(which.clone(), pr::load(&dir, &which)));
    }

    /// Open a file in your editor: terminal editors inside seshi beside what you're on,
    /// others (VS Code, …) as their own window.
    pub(super) fn open_in_editor(&mut self, path: &std::path::Path) {
        self.open_in_editor_at(path, None);
    }

    /// Open a file in your editor, at a line if given.
    pub(super) fn open_in_editor_at(&mut self, path: &std::path::Path, at: Option<u32>) {
        let ed = [self.cfg.editor.clone(), std::env::var("VISUAL").unwrap_or_default(), std::env::var("EDITOR").unwrap_or_default()]
            .into_iter()
            .find(|e| !e.trim().is_empty())
            .unwrap_or_else(|| "code".into());
        let exe = ed.split_whitespace().next().unwrap_or("code");
        let name = std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let q = files::quote_path(path);
        let line = match (at, name.as_str()) {
            (None, _) => format!("{ed} {q}"),
            (Some(l), "code" | "code-insiders" | "cursor" | "windsurf") => {
                format!("{ed} -g {}", files::quote_path(std::path::Path::new(&format!("{}:{l}", path.display()))))
            }
            (Some(l), "hx" | "helix" | "zed") => format!("{ed} {}:{l}", q),
            (Some(l), _) => format!("{ed} +{l} {q}"),
        };
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.here_dir());
        if ["nvim", "vim", "vi", "hx", "helix", "nano", "micro", "kak", "emacs", "ne"].contains(&name.as_str()) {
            self.hy_new_session(dir, Some(line), true);
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
        crate::proc::quiet(&mut cmd);
        match cmd.spawn() {
            Ok(_) => self.notify(format!("opened {} in {exe}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), false),
            Err(e) => self.notify(format!("couldn't start {exe}: {e}"), true),
        }
    }

    pub(super) fn save_setting(&mut self, path: &str, value: toml_edit::Value) {
        match modal::write(path, value) {
            Ok(()) => self.reload_config(),
            Err(e) => self.notify(format!("{e:#}"), true),
        }
    }

    pub(super) fn pick_items(&self, query: &str, commands: bool) -> Vec<PickItem> {
        let q = query.to_lowercase();
        let command_items: Vec<PickItem> = if commands {
            let ext = self.exts.iter().enumerate().flat_map(|(ei, e)| {
                e.commands.iter().enumerate().map(move |(ci, c)| PickItem {
                    label: c.title.clone(),
                    detail: format!("extension · {}", e.name),
                    status: Status::None,
                    key: String::new(),
                    target: PickTarget::Ext(ei, ci),
                })
            });
            self.palette_commands()
                .into_iter()
                .map(|a| PickItem {
                    label: a.describe(),
                    detail: String::new(),
                    status: Status::None,
                    key: self.key_for(&a),
                    target: PickTarget::Command(a),
                })
                .chain(ext)
                .collect()
        } else {
            Vec::new()
        };
        // In this layout the palette is commands only; Go to finds sessions.
        if commands {
            let words: Vec<&str> = q.split_whitespace().collect();
            return command_items.into_iter().filter(|i| words.iter().all(|w| i.label.to_lowercase().contains(w))).collect();
        }
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
