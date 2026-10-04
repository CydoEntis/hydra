//! What keys, clicks and actions do in this layout.

use super::*;

// ---- behaviour -----------------------------------------------------------------------------


pub(in crate::client) const WT_NAMES: [&str; 16] = [
    "quick-fox", "calm-heron", "bright-owl", "steady-elk", "brave-wren", "swift-lynx", "keen-otter", "bold-raven",
    "wise-badger", "lucky-hare", "sly-marten", "warm-finch", "deep-pike", "grey-wolf", "red-kite", "tall-crane",
];

impl App {
    pub(in crate::client) fn hy_agent(&self) -> String {
        self.cfg.quick.agents.first().map(|a| a.name.clone()).unwrap_or_else(|| "claude".into())
    }

    /// Where a session that just got the focus shows: beside the one it was opened from, in
    /// a new tab, in the tab that already shows it, or in place of the one you were on.
    pub(in crate::client) fn hy_place(&mut self, f: TermId, prev: Option<TermId>) {
        use crate::layout::{Dir, Node};
        let beside = self
            .hy
            .pending_split
            .take()
            .filter(|(p, at)| *p != f && at.elapsed().as_secs() < 20 && self.snap.terms.contains_key(p))
            .map(|(p, _)| p);
        let new_tab = self.hy.new_tab.take().is_some_and(|at| at.elapsed().as_secs() < 60);
        let take_out = |tabs: &mut Vec<HyTab>, f: TermId| {
            for tab in tabs.iter_mut() {
                if tab.layout.contains(f)
                    && tab.layout.leaves().len() > 1
                    && let Some(l) = tab.layout.clone().remove(f)
                {
                    tab.layout = l;
                    tab.focus = tab.layout.first_leaf();
                }
            }
            tabs.retain(|t| !(t.layout == Node::Leaf(f)));
        };
        if new_tab {
            take_out(&mut self.hy.tabs, f);
            self.hy.tabs.push(HyTab { layout: Node::Leaf(f), focus: f });
            self.hy.tab = self.hy.tabs.len() - 1;
            return;
        }
        if let Some(p) = beside {
            take_out(&mut self.hy.tabs, f);
            let i = match self.hy.tabs.iter().position(|t| t.layout.contains(p)) {
                Some(i) => i,
                None => {
                    self.hy.tabs.push(HyTab { layout: Node::Leaf(p), focus: p });
                    self.hy.tabs.len() - 1
                }
            };
            // Beside a wide one, under a tall one.
            let wide = self.hy.leaf_rects.iter().find(|(id, _)| *id == p).is_none_or(|(_, r)| r.width >= r.height * 3);
            let dir = self.hy.split_dir.take().unwrap_or(if wide { Dir::Right } else { Dir::Down });
            let tab = &mut self.hy.tabs[i];
            tab.layout.split(p, dir, f);
            tab.focus = f;
            self.hy.tab = i;
            return;
        }
        if let Some(i) = self.hy.tabs.iter().position(|t| t.layout.contains(f)) {
            self.hy.tab = i;
            self.hy.tabs[i].focus = f;
            return;
        }
        if self.hy.tabs.is_empty() {
            self.hy.tabs.push(HyTab { layout: Node::Leaf(f), focus: f });
            self.hy.tab = 0;
            return;
        }
        // The one you were on was closed: the rest of its split stays as it is, and the focus
        // goes to what's left there (not to whatever the server picked).
        let tab = &mut self.hy.tabs[self.hy.tab];
        if let Some(gone) = prev.filter(|o| tab.layout.contains(*o) && !self.snap.terms.contains_key(o))
            && tab.layout.leaves().len() > 1
            && let Some(rest) = tab.layout.clone().remove(gone)
        {
            tab.layout = rest;
            tab.focus = tab.layout.first_leaf();
            let to = tab.focus;
            self.cmd(Command::FocusPane { term: to });
            return;
        }
        let tab = &mut self.hy.tabs[self.hy.tab];
        // Not shown anywhere: it takes the place of the one you were on.
        let old = prev.filter(|o| tab.layout.contains(*o)).unwrap_or(tab.focus);
        let swapped = tab.layout.map_leaves(&mut |id| Some(if id == old { f } else { id }));
        tab.layout = swapped.filter(|l| l.contains(f)).unwrap_or(Node::Leaf(f));
        tab.focus = f;
    }

    /// Stop showing `t` beside the others (it keeps running). False when it's on its own.
    pub(in crate::client) fn hy_unshow(&mut self, t: TermId) -> bool {
        let Some(i) = self.hy.tabs.iter().position(|tab| tab.layout.contains(t)) else { return false };
        let tab = &mut self.hy.tabs[i];
        if tab.layout.leaves().len() < 2 {
            return false;
        }
        if let Some(l) = tab.layout.clone().remove(t) {
            tab.layout = l;
            tab.focus = tab.layout.first_leaf();
            let to = tab.focus;
            if self.focused() == Some(t) {
                self.cmd(Command::FocusPane { term: to });
            }
        }
        true
    }

    /// Message an agent: a small box beside its sidebar row when it has one, else centered.
    /// From the sidebar cursor, sending returns there so the next one is a key away.
    pub(in crate::client) fn hy_talk(&mut self, term: TermId, from_side: bool) {
        self.hy.talk_anchor = self.hy.row_y.get(&term).copied();
        self.hy.talk_back = from_side;
        self.mode = Mode::Talk { term, input: String::new() };
    }

    /// Show a session: it becomes the focused one.
    pub(in crate::client) fn hy_focus(&mut self, term: TermId) {
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.cmd(Command::FocusPane { term });
    }

    /// Start a session in `cwd` running `cmd` (None: a shell), beside the focused one or
    /// on its own.
    pub(in crate::client) fn hy_new_session(&mut self, cwd: PathBuf, cmd: Option<String>, beside: bool) {
        if beside && let Some(f) = self.focused() {
            self.hy.pending_split = Some((f, Instant::now()));
        }
        self.hy.cursor = None;
        self.mode = Mode::Normal;
        self.cmd(Command::NewWorkspace { cwd: Some(cwd), name: None, cmd });
    }

    /// A new worktree of a project running `cmd`: on `branch` if given (an existing one),
    /// else on a new branch with a generated name.
    pub(in crate::client) fn hy_new_worktree(&mut self, proj: &Proj, cmd: Option<String>, beside: bool, branch: Option<String>) {
        let taken: HashSet<String> = proj.wts.iter().flat_map(|w| [w.name.clone(), w.branch.clone()]).chain(proj.branches.iter().cloned()).collect();
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as usize).unwrap_or(0);
        let branch = branch.unwrap_or_else(|| {
            (0..WT_NAMES.len())
                .map(|i| WT_NAMES[(n + i) % WT_NAMES.len()].to_string())
                .find(|b| !taken.contains(b))
                .unwrap_or_else(|| format!("{}-{}", WT_NAMES[n % WT_NAMES.len()], n % 1000))
        });
        let Some(ws) = self.active_ws().map(|w| w.id).or_else(|| self.snap.workspaces.first().map(|w| w.id)) else {
            self.notify("start something first".into(), true);
            return;
        };
        if beside && let Some(f) = self.focused() {
            self.hy.pending_split = Some((f, Instant::now()));
        }
        self.mode = Mode::Normal;
        self.notify(format!("new worktree {branch} in {}", proj.name), false);
        self.cmd(Command::NewWorktree { ws, branch, base: None, cmd, split: None, from: Some(proj.path.clone()) });
    }

    /// A new worktree running `cmd` (on `branch` if given), full screen.
    pub(in crate::client) fn hy_start_worktree(&mut self, proj: &Proj, cmd: Option<String>, branch: Option<String>) {
        self.hy_new_worktree(proj, cmd, false, branch);
    }

    /// Open a folder as a project: switch to it, or start a session there if nothing runs.
    pub(in crate::client) fn hy_open_project(&mut self, path: PathBuf) {
        self.mode = Mode::Normal;
        self.splash = false;
        let root = crate::gitfs::head(&path).map(|h| h.main_root).unwrap_or_else(|| path.clone());
        let key = path_key(&root);
        let model = self.hy_model();
        let existing = model.iter().find(|p| p.key == key);
        let found = existing.and_then(|p| p.sessions().min_by_key(|s| (rank(s.status), s.term)).map(|s| (p.name.clone(), s.term)));
        let is_new = existing.is_none();
        if !self.hy.saved.known.iter().any(|k| path_key(k) == key) {
            self.hy.saved.known.push(root.clone());
            self.hy.save();
        }
        self.hy.proj = Some(key.clone());
        match found {
            Some((name, term)) => {
                self.hy_focus(term);
                self.notify(format!("Switched to {name}"), false);
            }
            None => {
                if is_new {
                    self.hy.fresh.insert(key);
                }
                self.hy_new_session(root.clone(), None, false);
                self.notify(format!("Opened {} with a shell; + New starts an agent", folder_name(&root)), false);
            }
        }
    }

    pub(in crate::client) fn hy_open_finder(&mut self) {
        let model = self.hy_model();
        let start = model
            .iter()
            .find(|p| self.hy.proj.as_deref() == Some(p.key.as_str()))
            .and_then(|p| p.path.parent().map(Path::to_path_buf))
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_default();
        self.mode = Mode::Finder(Box::new(Finder::new(&start)));
    }

    /// The + New chooser, for the current project (or the sidebar cursor's).
    pub(in crate::client) fn hy_open_new_pane(&mut self, beside: bool) {
        let model = self.hy_model();
        let key = self.hy.cursor.and_then(|c| model.iter().find(|p| p.sessions().any(|s| s.term == c)).map(|p| p.key.clone())).or(self.hy.proj.clone());
        let p = model.iter().position(|p| Some(&p.key) == key.as_ref()).unwrap_or(0);
        self.hy_new(p, beside);
    }

    pub(in crate::client) fn hy_settings(&mut self) {
        self.mode = Mode::HySettings(Box::new(SView { cat: 0, sel: 0, editing: None, capturing: false, scroll: 0 }));
    }

    /// Move the sidebar cursor; bare keys then work on the sidebar until Esc or Enter.
    pub(in crate::client) fn hy_side_move(&mut self, d: i8) {
        let items = self.hy.side_items.clone();
        if items.is_empty() {
            return;
        }
        let cur = match (&self.hy.cursor_proj, self.hy.cursor.or(self.focused())) {
            (Some(k), _) => items.iter().position(|i| *i == SideItem::Proj(k.clone())),
            (None, Some(t)) => items.iter().position(|i| *i == SideItem::Sess(t)),
            _ => None,
        };
        let next = match (cur, d) {
            (None, _) => 0,
            (Some(i), 0) => i,
            (Some(i), d) if d < 0 => i.saturating_sub(1),
            (Some(i), _) => (i + 1).min(items.len() - 1),
        };
        self.hy_side_set(items[next].clone());
    }

    /// The sidebar row to land on after the cursor's row goes: the next one, else the one
    /// before.
    pub(in crate::client) fn side_after_close(&self) -> Option<SideItem> {
        let items = &self.hy.side_items;
        let cur = match (&self.hy.cursor_proj, self.hy.cursor) {
            (Some(k), _) => items.iter().position(|i| *i == SideItem::Proj(k.clone())),
            (None, Some(t)) => items.iter().position(|i| *i == SideItem::Sess(t)),
            _ => None,
        }?;
        items.get(cur + 1).or_else(|| cur.checked_sub(1).and_then(|i| items.get(i))).cloned()
    }

    /// A confirm answered (`act` when yes): do it, and go back to the sidebar if it was
    /// asked from there.
    pub(in crate::client) fn confirm_done(&mut self, act: Option<crate::client::menu::Act>) {
        let back = self.hy.side_return.take();
        let yes = act.is_some();
        if let Some(a) = act {
            self.menu_do(a);
        }
        if let Some(item) = back {
            if yes {
                self.hy_side_set(item);
            } else {
                self.hy_side_move(0);
            }
        }
    }

    /// Put the sidebar cursor on a row (and the keys in the sidebar).
    pub(in crate::client) fn hy_side_set(&mut self, item: SideItem) {
        self.hy.follow = true;
        self.mode = Mode::Side;
        match item {
            SideItem::Proj(k) => {
                self.hy.cursor_proj = Some(k);
                self.hy.cursor = None;
            }
            SideItem::Sess(t) => {
                self.hy.cursor_proj = None;
                self.hy.cursor = Some(t);
                // Resting on a finished agent counts as seeing it.
                if self.snap.terms.get(&t).is_some_and(|i| i.status == Status::Done) {
                    self.cmd(Command::MarkSeen { term: t });
                }
            }
        }
    }

    pub(in crate::client) fn hy_side_leave(&mut self) {
        self.hy.cursor = None;
        self.hy.cursor_proj = None;
        self.mode = Mode::Normal;
    }

    /// Fold or unfold a project (true: open it).
    pub(in crate::client) fn hy_fold(&mut self, key: &str, open: Option<bool>) {
        let k = format!("p:{key}");
        let is_open = !self.hy.saved.closed.contains(&k);
        let want = open.unwrap_or(!is_open);
        if want != is_open {
            if want {
                self.hy.saved.closed.retain(|c| *c != k);
            } else {
                self.hy.saved.closed.push(k);
            }
            self.hy.save();
        }
    }

    /// Actions that work differently in this layout. Returns true if handled.
    pub(in crate::client) fn hy_act(&mut self, a: &Action) -> bool {
        let focused = self.focused();
        match a {
            Action::Settings => self.hy_settings(),
            Action::NewPane => self.hy_open_new_pane(false),
            // A plain shell in the project's own folder (the repo itself, not a worktree).
            Action::ShellHere => {
                let dir = self
                    .hy
                    .cursor
                    .or(self.focused())
                    .and_then(|t| self.snap.terms.get(&t))
                    .map(|t| t.root.clone().unwrap_or_else(|| t.cwd.clone()))
                    .or_else(|| self.hy_model().iter().find(|p| Some(&p.key) == self.hy.proj.as_ref()).map(|p| p.path.clone()))
                    .unwrap_or_else(|| self.here_dir());
                self.hy.cursor = None;
                self.hy_new_session(dir, None, false);
            }
            Action::Jump | Action::Picker => self.mode = Mode::Jump { sel: 0 },
            Action::OpenProject => self.hy_open_finder(),
            Action::Talk | Action::Reply => {
                let from_side = *a == Action::Talk && self.hy.cursor.is_some();
                let term = if *a == Action::Talk { self.hy.cursor.or(focused) } else { focused };
                match term {
                    Some(term) => self.hy_talk(term, from_side),
                    None => self.notify("nothing to message".into(), true),
                }
            }
            // In a split: just this one, and back. On its own: hide the sidebar.
            Action::Zoom => match focused {
                Some(f) if self.hy.tabs.get(self.hy.tab).is_some_and(|t| t.layout.contains(f) && t.layout.leaves().len() > 1) => {
                    self.hy.zoom = if self.hy.zoom == Some(f) { None } else { Some(f) };
                }
                _ => self.sidebar = !self.sidebar,
            },
            Action::Focus(d) => match focused.and_then(|f| crate::layout::neighbor(&self.hy.leaf_rects, f, *d)) {
                Some(to) => self.cmd(Command::FocusPane { term: to }),
                // Past the left edge: the sidebar.
                None if *d == crate::layout::Dir::Left && self.sidebar => self.hy_side_move(0),
                None => {}
            },
            Action::FocusNext | Action::FocusPrev => {
                let shown: Vec<TermId> = self.hy.leaf_rects.iter().map(|(id, _)| *id).collect();
                if let Some(f) = focused
                    && shown.len() > 1
                {
                    let i = shown.iter().position(|x| *x == f).unwrap_or(0);
                    let n = shown.len();
                    let to = if *a == Action::FocusNext { shown[(i + 1) % n] } else { shown[(i + n - 1) % n] };
                    self.cmd(Command::FocusPane { term: to });
                }
            }
            Action::NewTab => {
                self.hy.new_tab = Some(Instant::now());
                self.mode = Mode::Jump { sel: 0 };
                self.notify("pick what the new tab shows (or + New for something new)".into(), false);
            }
            Action::NextTab | Action::PrevTab => {
                let n = self.hy.tabs.len();
                if n > 1 {
                    self.hy.tab = if *a == Action::NextTab { (self.hy.tab + 1) % n } else { (self.hy.tab + n - 1) % n };
                    let to = self.hy.tabs[self.hy.tab].focus;
                    self.cmd(Command::FocusPane { term: to });
                }
            }
            Action::GoTo => self.mode = Mode::GoTo { query: String::new(), sel: 0 },
            Action::SelectTab(n) => {
                if let Some(i) = n.checked_sub(1)
                    && let Some(tab) = self.hy.tabs.get(i)
                {
                    let to = tab.focus;
                    self.hy.tab = i;
                    self.cmd(Command::FocusPane { term: to });
                }
            }
            // A tab is a view: closing it keeps its sessions (they're in the sidebar).
            Action::CloseTab => {
                if self.hy.tabs.len() > 1 {
                    let i = self.hy.tab.min(self.hy.tabs.len() - 1);
                    self.hy.tabs.remove(i);
                    self.hy.tab = self.hy.tab.min(self.hy.tabs.len() - 1);
                    let to = self.hy.tabs[self.hy.tab].focus;
                    self.cmd(Command::FocusPane { term: to });
                } else {
                    self.notify("it's the only tab".into(), false);
                }
            }
            Action::ToggleSidebar => self.sidebar = !self.sidebar,
            Action::RenameWorkspace => {
                if let Some(t) = self.hy.cursor.or(focused) {
                    self.menu_do(crate::client::menu::Act::RenamePane(t));
                }
            }
            Action::SplitRight | Action::SplitDown | Action::SplitLeft | Action::SplitUp | Action::Spawn(..) => {
                self.hy.split_dir = Some(match a {
                    Action::SplitDown | Action::SplitUp => crate::layout::Dir::Down,
                    Action::Spawn(d, _) => *d,
                    _ => crate::layout::Dir::Right,
                });
                let cwd = focused.and_then(|f| self.snap.terms.get(&f)).map(|t| t.cwd.clone()).unwrap_or_else(|| self.here_dir());
                let cmd = match a {
                    Action::Spawn(_, c) => Some(c.clone()),
                    _ => None,
                };
                self.hy_new_session(cwd, cmd, true);
            }
            Action::NewSession => self.hy_open_new_pane(true),
            Action::CloseSplit => {
                if let Some(f) = focused
                    && !self.hy_unshow(f)
                {
                    self.notify("nothing split here".into(), false);
                }
            }
            Action::SideMove(d) => self.hy_side_move(*d),
            Action::Resize(d) => {
                use crate::layout::Dir;
                let grow = matches!(d, Dir::Right | Dir::Down);
                let many = self.hy.tabs.get(self.hy.tab).is_some_and(|tab| tab.layout.leaves().len() > 1);
                if let (true, Some(f)) = (many, focused) {
                    let tab = self.hy.tab;
                    self.hy.tabs[tab].layout.resize(f, *d, 0.05);
                } else {
                    let w = self.hy.saved.side_w.unwrap_or(self.hy.side_rect.width.max(SIDE_MIN));
                    let w = if grow { w + 2 } else { w.saturating_sub(2) };
                    self.hy.saved.side_w = Some(w.clamp(SIDE_MIN, SIDE_MAX));
                }
                self.hy.save();
            }
            Action::Ideas => self.open_ideas(),
            Action::Map => self.open_map(),
            Action::Inbox => self.open_tickets(),
            Action::Race => self.open_race_new(),
            Action::Ship => {
                let dir = self.hy_target_dir();
                self.hy.cursor = None;
                self.ask_ship(dir);
            }
            Action::Files | Action::Changes | Action::PullRequest => {
                let dir = self.hy_target_dir();
                self.hy.cursor = None;
                self.mode = Mode::Normal;
                match a {
                    Action::Files => self.open_files(dir),
                    Action::Changes => self.open_changes(dir),
                    _ => match crate::gitfs::head(&dir) {
                        Some(h) => self.open_pr(h.top, h.branch),
                        None => self.notify("not a git repo".into(), true),
                    },
                }
            }
            Action::BrowseTree => self.hy_side_move(0),
            _ => return false,
        }
        true
    }

    /// Keys while the sidebar cursor is up: arrows move, Enter opens, other keys act as
    /// if the leader had been pressed.
    /// Keys while the sidebar has them: ↑↓ move, →← open or fold (← from a session goes to
    /// its project), Enter opens, Space messages, Esc goes back to the pane. Anything you
    /// type goes to the pane you're on.
    pub(in crate::client) fn on_side_key(&mut self, k: &KeyEvent) {
        let spec = KeySpec::from_event(k);
        let proj = self.hy.cursor_proj.clone();
        let model = self.hy_model();
        if spec == self.keymap.prefix {
            self.mode = Mode::Prefix { since: Instant::now() };
            return;
        }
        let plain = !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') if !k.modifiers.contains(KeyModifiers::CONTROL) => self.hy_side_move(-1),
            KeyCode::Down | KeyCode::Char('j') if !k.modifiers.contains(KeyModifiers::CONTROL) => self.hy_side_move(1),
            KeyCode::Home => {
                if let Some(first) = self.hy.side_items.first().cloned() {
                    self.hy_side_set(first);
                }
            }
            KeyCode::End => {
                if let Some(last) = self.hy.side_items.last().cloned() {
                    self.hy_side_set(last);
                }
            }
            KeyCode::Right => match &proj {
                Some(key) => self.hy_fold(key, Some(true)),
                None => {
                    // Into the pane.
                    if let Some(c) = self.hy.cursor {
                        self.hy_focus(c);
                    }
                    self.hy_side_leave();
                }
            },
            KeyCode::Left => match (&proj, self.hy.cursor) {
                (Some(key), _) => self.hy_fold(key, Some(false)),
                (None, Some(t)) => {
                    if let Some(p) = model.iter().find(|p| p.sessions().any(|s| s.term == t)) {
                        self.hy_side_set(SideItem::Proj(p.key.clone()));
                    }
                }
                _ => {}
            },
            KeyCode::Char(' ') if plain => match (&proj, self.hy.cursor) {
                (Some(key), _) => {
                    if let Some(pi) = model.iter().position(|p| p.key == *key) {
                        self.hy_side_leave();
                        self.hy_new(pi, false);
                    }
                }
                (None, Some(c)) => self.hy_talk(c, true),
                _ => {}
            },
            KeyCode::Enter => match (&proj, self.hy.cursor) {
                (Some(key), _) => self.hy_fold(key, None),
                (None, c) => {
                    if let Some(c) = c {
                        self.hy_focus(c);
                    }
                    self.hy_side_leave();
                }
            },
            KeyCode::Esc => self.hy_side_leave(),
            _ if spec == self.keymap.prefix => self.mode = Mode::Prefix { since: Instant::now() },
            // A letter from the row's menu does that (x close, r rename, m message, …).
            KeyCode::Char(c) if plain && crate::client::menu::menu_keys(&self.cursor_items()).contains(&Some(c)) => {
                let items = self.cursor_items();
                if let Some(i) = crate::client::menu::menu_keys(&items).iter().position(|k| *k == Some(c))
                    && let Some((_, act)) = items.get(i).cloned()
                {
                    self.menu_act(act);
                }
            }
            // Anything else is typing: leave the sidebar and hand the key over.
            _ => {
                self.hy_side_leave();
                if let Some(term) = self.focused() {
                    let data = crate::keys::encode(k, self.parsers.get(&term).is_some_and(|p| p.screen().application_cursor()));
                    if !data.is_empty() {
                        self.send(crate::protocol::ClientMsg::Input { term, data });
                    }
                }
            }
        }
    }

    pub(in crate::client) fn on_jump_key(&mut self, sel: usize, k: &KeyEvent) {
        let model = self.hy_model();
        let list = jump_list(&model);
        let prs = jump_prs(&model);
        let total = list.len() + prs.len();
        let go = |app: &mut App, i: usize| {
            if let Some((s, ..)) = list.get(i) {
                app.hy_focus(s.term);
            } else if let Some((dir, pr, ..)) = prs.get(i - list.len().min(i)) {
                app.mode = Mode::Normal;
                app.open_pr(dir.clone(), pr.number.to_string());
            }
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('j') => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::Jump { sel: (sel + 1).min(total.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::Jump { sel: sel.saturating_sub(1) },
            KeyCode::Enter if sel < total => go(self, sel),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' && (c as usize - '1' as usize) < total => go(self, c as usize - '1' as usize),
            _ => {}
        }
    }

    /// The folder Files / Changes / PR act on: the sidebar cursor's, else the focused one's.
    pub(in crate::client) fn hy_target_dir(&self) -> PathBuf {
        self.hy
            .cursor
            .or(self.focused())
            .and_then(|t| self.snap.terms.get(&t))
            .map(|t| t.top.clone().unwrap_or_else(|| t.cwd.clone()))
            .unwrap_or_else(|| self.here_dir())
    }

    pub(in crate::client) fn finder_enter(&mut self, mut fd: Finder) {
        let sep = std::path::MAIN_SEPARATOR;
        let rows = fd.rows();
        match rows.get(fd.sel).cloned() {
            None => {
                let (dir, part) = fd.split();
                let p = dir.join(part);
                if p.is_dir() {
                    self.hy_open_project(p);
                } else {
                    self.notify(format!("no folder {}", tilde(&p)), true);
                    self.mode = Mode::Finder(Box::new(fd));
                }
            }
            Some((n, p, repo)) if n == "." || repo => self.hy_open_project(p),
            Some((_, p, _)) => {
                fd.q = format!("{}{sep}", tilde(&p).trim_end_matches(sep));
                fd.sel = 0;
                fd.refresh();
                self.mode = Mode::Finder(Box::new(fd));
            }
        }
    }

    pub(in crate::client) fn on_finder_key(&mut self, mut fd: Finder, k: &KeyEvent) {
        let sep = std::path::MAIN_SEPARATOR;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => return self.finder_enter(fd),
            KeyCode::Tab => {
                if let Some((n, p, _)) = fd.rows().get(fd.sel).cloned()
                    && n != "."
                {
                    fd.q = format!("{}{sep}", tilde(&p).trim_end_matches(sep));
                    fd.sel = 0;
                }
            }
            KeyCode::Down => fd.sel = (fd.sel + 1).min(fd.rows().len().saturating_sub(1)),
            KeyCode::Up => fd.sel = fd.sel.saturating_sub(1),
            KeyCode::Backspace => {
                fd.q.pop();
                fd.sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                fd.q.push(if c == '/' || c == '\\' { sep } else { c });
                fd.sel = 0;
            }
            _ => {}
        }
        fd.refresh();
        self.mode = Mode::Finder(Box::new(fd));
    }

    /// Start what the chooser says: an agent gets its own worktree in a git project, a
    /// shell (or an agent outside git) runs in the project's folder.
    pub(in crate::client) fn hy_pane_go(&mut self, np: &NewPaneHy) {
        let model = self.hy_model();
        let Some(p) = model.get(np.p).cloned() else {
            self.hy_open_finder();
            return;
        };
        let agents = np_agents(self);
        let agent = agents.get(np.a).cloned().unwrap_or_else(|| "shell".into());
        let main = p.wts.iter().find(|w| w.main).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
        let cmd = np_command(self, &agent, np.model, &np.task);
        if !self.hy.saved.draft.is_empty() {
            self.hy.saved.draft.clear();
            self.hy.save();
        }
        if let Some(recipe) = agent.strip_prefix("⚙ ") {
            self.mode = Mode::Normal;
            self.run_recipe(&p, recipe);
            return;
        }
        if !p.git {
            return self.hy_new_session(main, cmd, np.beside);
        }
        match np.place_for(&agent, self.cfg.worktree.per_agent) {
            0 => self.hy_new_worktree(&p, cmd, np.beside, None),
            1 => {
                // A new branch right in the repo folder (no worktree).
                let taken: HashSet<String> = p.wts.iter().map(|w| w.branch.clone()).collect();
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as usize).unwrap_or(0);
                let name = (0..WT_NAMES.len()).map(|i| WT_NAMES[(n + i) % WT_NAMES.len()].to_string()).find(|b| !taken.contains(b)).unwrap_or_else(|| format!("branch-{}", n % 10_000));
                let (pname, beside) = (p.name.clone(), np.beside);
                self.spawn_bg(move || {
                    let mut git = std::process::Command::new("git");
                    git.arg("-C").arg(&main).args(["switch", "-c", &name]);
                    crate::proc::quiet(&mut git);
                    let out = git.output();
                    crate::client::Bg::Then(Box::new(move |app: &mut App| match out {
                        Ok(o) if o.status.success() => {
                            app.notify(format!("{pname} is on a new branch, {name}"), false);
                            app.hy_new_session(main, cmd, beside);
                        }
                        Ok(o) => app.notify(format!("couldn't make a branch: {}", String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("git failed")), true),
                        Err(e) => app.notify(format!("couldn't run git: {e}"), true),
                    }))
                });
            }
            n @ 3.. => match p.wts.iter().filter(|w| !w.main).nth(n as usize - 3) {
                Some(w) => self.hy_new_session(w.path.clone(), cmd, np.beside),
                None => self.hy_new_session(main, cmd, np.beside),
            },
            _ => self.hy_new_session(main, cmd, np.beside),
        }
    }

    pub(in crate::client) fn on_goto_key(&mut self, mut query: String, mut sel: usize, k: &KeyEvent) {
        let model = self.hy_model();
        let rows = goto_rows(&model, &query);
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down => sel = (sel + 1).min(rows.len().saturating_sub(1)),
            KeyCode::Up => sel = sel.saturating_sub(1),
            KeyCode::PageDown => sel = (sel + 10).min(rows.len().saturating_sub(1)),
            KeyCode::PageUp => sel = sel.saturating_sub(10),
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                match rows.get(sel) {
                    Some(GoRow::Sess(_, t)) => self.hy_focus(*t),
                    // A project: its most urgent session, or a shell there.
                    Some(GoRow::Proj(pi)) => match model[*pi].sessions().min_by_key(|s| (rank(s.status), s.term)) {
                        Some(s) => self.hy_focus(s.term),
                        None => self.on_hy_hit(HyHit::ShellIn(*pi), false),
                    },
                    None => {}
                }
                return;
            }
            KeyCode::Backspace => {
                query.pop();
                sel = 0;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                query.push(c);
                // Land on the first session that matches, when there is one.
                let rows = goto_rows(&model, &query);
                sel = rows.iter().position(|r| matches!(r, GoRow::Sess(..))).unwrap_or(0);
            }
            _ => {}
        }
        self.mode = Mode::GoTo { query, sel };
    }

    pub(in crate::client) fn on_history_key(&mut self, sel: usize, k: &KeyEvent) {
        let n = self.history.len();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::History { sel: (sel + 1).min(n.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::History { sel: sel.saturating_sub(1) },
            KeyCode::Char('c') => {
                self.history.clear();
                self.mode = Mode::History { sel: 0 };
            }
            KeyCode::Enter => {
                let term = self.history.iter().rev().nth(sel).and_then(|h| h.1);
                match term.filter(|t| self.snap.terms.contains_key(t)) {
                    Some(t) => {
                        self.mode = Mode::Normal;
                        self.hy_focus(t);
                    }
                    None => self.mode = Mode::History { sel },
                }
            }
            _ => self.mode = Mode::History { sel },
        }
    }

    pub(in crate::client) fn on_memory_key(&mut self, sel: usize, k: &KeyEvent) {
        let rows = memory_rows(self);
        let n = rows.len();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down => self.mode = Mode::Memory { sel: (sel + 1).min(n.saturating_sub(1)) },
            KeyCode::Up => self.mode = Mode::Memory { sel: sel.saturating_sub(1) },
            KeyCode::Enter => {
                if let Some(r) = rows.get(sel) {
                    self.mode = Mode::Normal;
                    self.hy_focus(r.0);
                }
            }
            KeyCode::Char('x') => {
                if let Some(r) = rows.get(sel) {
                    self.cmd(Command::ClosePane { term: r.0 });
                    self.notify(format!("ended {} ({} freed)", r.1, mb(r.3)), false);
                }
                self.mode = Mode::Memory { sel: sel.min(n.saturating_sub(2)) };
            }
            _ => self.mode = Mode::Memory { sel },
        }
    }

    /// Ctrl+Space . : your presets, numbered, for the agent you're on.
    pub(in crate::client) fn hy_presets(&mut self) {
        if self.cfg.presets.is_empty() {
            self.notify("no presets yet: add [[presets]] to your config (see the example config)".into(), true);
            return;
        }
        let on = self.hy.cursor.or(self.focused());
        let items = self
            .cfg
            .presets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let how = match p.place.as_str() {
                    "send" => "tell it",
                    "worktree" => "new worktree",
                    _ => "beside",
                };
                (format!("{}  {}{}  · {how}", i + 1, p.name, if p.asks() { "…" } else { "" }), crate::client::menu::Act::Preset(i, on))
            })
            .collect();
        self.menu("Presets".into(), items, (self.hy.side_rect.right() + 4, 3));
    }

    /// Run preset `i` for the agent `on` (its folder, or the agent itself).
    pub(in crate::client) fn hy_run_preset(&mut self, i: usize, on: Option<TermId>, task: Option<String>) {
        let Some(p) = self.cfg.presets.get(i).cloned() else { return };
        let model = self.hy_model();
        let at = on.and_then(|t| self.snap.terms.get(&t)).map(|t| (t.top.clone().unwrap_or_else(|| t.cwd.clone()), t.root.clone()));
        let pi = at
            .as_ref()
            .and_then(|(top, root)| model.iter().position(|m| path_key(&m.path) == path_key(root.as_ref().unwrap_or(top))))
            .unwrap_or(0);
        if p.asks() && task.is_none() {
            // Ask for the task in + New, with the preset picked.
            let mut np = NewPaneHy::new(pi, p.place == "beside");
            np.a = np_agents(self).iter().position(|a| *a == format!("★ {}", p.name)).unwrap_or(0);
            np.place = Some(if p.place == "worktree" { 0 } else { 2 });
            self.mode = Mode::HyPane(np);
            return;
        }
        let task = task.unwrap_or_default();
        self.mode = Mode::Normal;
        match (p.place.as_str(), on, at) {
            ("send", Some(term), _) => self.send_message(term, &p.fill(&task)),
            ("worktree", _, _) | (_, None, _) | (_, _, None) => {
                let cmd = np_command(self, &format!("★ {}", p.name), 0, &task);
                match model.get(pi) {
                    Some(proj) if proj.git => self.hy_new_worktree(proj, cmd, false, None),
                    Some(proj) => self.hy_new_session(proj.path.clone(), cmd, false),
                    None => self.notify("open a project first".into(), true),
                }
            }
            (_, Some(_), Some((top, _))) => {
                let cmd = np_command(self, &format!("★ {}", p.name), 0, &task);
                self.hy_new_session(top, cmd, true);
            }
        }
    }

    /// + New for project `p`, with the task you didn't start last time.
    pub(in crate::client) fn hy_new(&mut self, p: usize, beside: bool) {
        let mut np = NewPaneHy::new(p, beside);
        np.task = self.hy.saved.draft.clone();
        self.mode = Mode::HyPane(np);
    }

    pub(in crate::client) fn on_hy_pane_key(&mut self, mut np: NewPaneHy, k: &KeyEvent) {
        let model = self.hy_model();
        let nproj = model.len() + 1;
        let agents = np_agents(self);
        let nag = agents.len();
        let agent = agents.get(np.a).cloned().unwrap_or_default();
        let nmodels = if agent.starts_with('★') { 0 } else { np_models(self, &agent).len() };
        let nplaces = model.get(np.p).map(|p| np_places(p).len()).unwrap_or(3);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let d: i32 = match k.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => 0,
        };
        let cyc = |v: usize, d: i32, n: usize| (v as i32 + d).rem_euclid(n.max(1) as i32) as usize;
        // Rows with nothing to choose are skipped.
        let skip = |r: u8| r == 2 && nmodels == 0;
        let step = |np: &mut NewPaneHy, by: u8| {
            np.row = (np.row + by) % NP_ROWS;
            if skip(np.row) {
                np.row = (np.row + by) % NP_ROWS;
            }
        };
        match k.code {
            KeyCode::Esc => {
                // The task stays for next time.
                if self.hy.saved.draft != np.task {
                    self.hy.saved.draft = np.task.clone();
                    self.hy.save();
                }
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                np.beside |= k.modifiers.contains(KeyModifiers::SHIFT);
                return self.hy_pane_go(&np);
            }
            KeyCode::Tab | KeyCode::Down => step(&mut np, 1),
            KeyCode::BackTab | KeyCode::Up => step(&mut np, NP_ROWS - 1),
            // Typing on the task row.
            KeyCode::Backspace if np.row == 0 => {
                if ctrl {
                    let keep = np.task.trim_end().rfind(' ').map(|i| i + 1).unwrap_or(0);
                    np.task.truncate(keep);
                } else {
                    np.task.pop();
                }
            }
            KeyCode::Char('u') if ctrl && np.row == 0 => np.task.clear(),
            KeyCode::Char(c) if np.row == 0 && !ctrl => np.task.push(c),
            KeyCode::Char('b') if !ctrl => np.beside = !np.beside,
            _ if d != 0 => match np.row {
                0 => {}
                1 => {
                    np.a = cyc(np.a, d, nag);
                    np.model = 0;
                }
                2 => np.model = cyc(np.model, d, nmodels),
                3 => {
                    np.p = cyc(np.p, d, nproj);
                    np.place = np.place.filter(|p| *p < 3);
                }
                4 => {
                    let cur = np.place_for(&agent, self.cfg.worktree.per_agent) as usize;
                    np.place = Some(cyc(cur, d, nplaces) as u8);
                }
                _ => np.beside = !np.beside,
            },
            _ => {}
        }
        self.mode = Mode::HyPane(np);
    }

    /// Keys on the splash: move between its buttons, Enter picks one, or a button's own key.
    /// Nothing else leaves it.
    pub(in crate::client) fn on_hy_splash_key(&mut self, k: &KeyEvent) {
        let opts = splash_options(self);
        let n = opts.len().max(1);
        match k.code {
            KeyCode::Up | KeyCode::Left | KeyCode::BackTab | KeyCode::Char('k') => self.hy.splash_sel = (self.hy.splash_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Right | KeyCode::Tab | KeyCode::Char('j') => self.hy.splash_sel = (self.hy.splash_sel + 1) % n,
            KeyCode::Enter => {
                if let Some((_, _, c)) = opts.get(self.hy.splash_sel.min(n - 1)) {
                    self.hy_splash_action(*c);
                }
            }
            KeyCode::Char(c) if opts.iter().any(|(_, _, k)| *k == c) || c == '?' || c == ',' => self.hy_splash_action(c),
            _ => {}
        }
    }

    pub(in crate::client) fn hy_splash_action(&mut self, c: char) {
        self.splash = false;
        self.hy.splash_sel = 0;
        match c {
            'n' => self.hy_open_new_pane(false),
            'o' => self.hy_open_finder(),
            ',' => self.hy_settings(),
            '?' => self.mode = Mode::Help { scroll: 0 },
            // r: resume, just the app as you left it.
            _ => {}
        }
    }

    /// Forget a project the user opened (sessions in it keep running).
    pub(in crate::client) fn hy_forget(&mut self, p: &Path) {
        let key = path_key(p);
        self.hy.saved.known.retain(|k| path_key(k) != key);
        self.hy.save();
        self.notify(format!("forgot {}", folder_name(p)), false);
    }

    /// Clicks on this layout's chips, rows and buttons.
    pub(in crate::client) fn on_hy_hit(&mut self, h: HyHit, double: bool) {
        match h {
            HyHit::ToggleProj(i) => {
                if let Some(key) = self.hy.proj_keys.get(i).cloned() {
                    self.hy_fold(&key, None);
                    // The keys follow you into the sidebar.
                    self.hy_side_set(SideItem::Proj(key));
                }
            }
            HyHit::NewIn(i) => self.hy_new(i, false),
            HyHit::OpenFolder => self.hy_open_finder(),
            // Opens it, and the arrows keep walking the sidebar until you type or press Enter.
            HyHit::Session(t) => {
                self.hy_focus(t);
                self.hy_side_set(SideItem::Sess(t));
            }
            HyHit::Talk(t) => self.hy_talk(t, false),
            HyHit::Settings => self.hy_settings(),
            HyHit::Jump => self.mode = Mode::Jump { sel: 0 },
            HyHit::NewPane => self.hy_open_new_pane(false),
            // The ✕ on a pane: close it (after asking).
            HyHit::CloseSplit(t) => self.menu_act(crate::client::menu::Act::End(vec![t])),
            HyHit::Divider(i) => self.hy.drag = Some(Drag::Divider(i)),
            HyHit::ShellIn(pi) => {
                if let Some(p) = self.hy_model().get(pi) {
                    let dir = p.wts.iter().find(|w| w.main).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
                    self.hy_new_session(dir, None, false);
                }
            }
            HyHit::RowMenuSess(t) => {
                let at = self.hover.map(|p| (p.x, p.y)).unwrap_or((0, 0));
                self.menu_for_session(t, at);
            }
            HyHit::RowMenuProj(i) => {
                let at = self.hover.map(|p| (p.x, p.y)).unwrap_or((0, 0));
                self.menu_for_project(i, at);
            }
            HyHit::ConfirmYes => {
                if let Mode::Confirm(c) = std::mem::replace(&mut self.mode, Mode::Normal) {
                    self.confirm_done(Some(c.act));
                }
            }
            HyHit::ConfirmNo => {
                self.mode = Mode::Normal;
                self.confirm_done(None);
            }
            HyHit::TabPick(i) => {
                if let Some(tab) = self.hy.tabs.get(i) {
                    let to = tab.focus;
                    self.hy.tab = i;
                    self.cmd(Command::FocusPane { term: to });
                }
            }
            HyHit::TabClose(i) => {
                if i < self.hy.tabs.len() && self.hy.tabs.len() > 1 {
                    self.hy.tabs.remove(i);
                    self.hy.tab = self.hy.tab.min(self.hy.tabs.len() - 1);
                    let to = self.hy.tabs[self.hy.tab].focus;
                    self.cmd(Command::FocusPane { term: to });
                }
            }
            HyHit::TabNew => self.act(Action::NewTab),
            HyHit::Close => self.mode = Mode::Normal,
            HyHit::Noop => {}
            HyHit::FinderPick(i) => {
                if let Mode::Finder(fd) = &self.mode {
                    let mut fd = (**fd).clone();
                    fd.sel = i;
                    self.finder_enter(fd);
                }
            }
            HyHit::JumpTo(t) => self.hy_focus(t),
            HyHit::NpTask => {
                if let Mode::HyPane(np) = &mut self.mode {
                    np.row = 0;
                }
            }
            HyHit::NpProj(i) | HyHit::NpRun(i) | HyHit::NpBeside(i) | HyHit::NpWhere(i) | HyHit::NpModel(i) => {
                if let Mode::HyPane(np) = &mut self.mode {
                    match h {
                        HyHit::NpWhere(_) => {
                            np.place = Some(i as u8);
                            np.row = 4;
                        }
                        HyHit::NpRun(_) => {
                            if np.a != i {
                                np.model = 0;
                            }
                            np.a = i;
                            np.row = 1;
                        }
                        HyHit::NpModel(_) => {
                            np.model = i;
                            np.row = 2;
                        }
                        HyHit::NpProj(_) => {
                            np.p = i;
                            np.place = np.place.filter(|p| *p < 3);
                            np.row = 3;
                        }
                        _ => {
                            np.beside = i == 1;
                            np.row = 5;
                        }
                    }
                }
            }
            HyHit::NpGo => {
                if let Mode::HyPane(np) = &self.mode {
                    let np = np.clone();
                    self.hy_pane_go(&np);
                }
            }
            HyHit::SetTab(i) => {
                if let Mode::HySettings(v) = &mut self.mode {
                    v.cat = i;
                    v.sel = 0;
                    v.editing = None;
                    v.capturing = false;
                }
            }
            HyHit::SetRow(i) | HyHit::SetVal(i, _) => {
                let Mode::HySettings(v) = &mut self.mode else { return };
                let again = v.sel == i;
                v.sel = i;
                let cat = crate::client::modal::Cat::ALL[v.cat.min(5)];
                let row = crate::client::design::settings_rows(self, cat).get(i).cloned();
                let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                match (h, row) {
                    (HyHit::SetVal(_, vi), Some(row @ SRow::Setting(_))) if vi != usize::MAX && chips_for(self, &row).is_some() => set_chip(self, &row, vi),
                    (HyHit::SetVal(..), Some(SRow::Project(p))) => self.hy_forget(&p),
                    (HyHit::SetVal(..), Some(_)) => self.hy_settings_key(&enter),
                    (HyHit::SetRow(_), Some(_)) if again || double => self.hy_settings_key(&enter),
                    _ => {}
                }
            }
            HyHit::SplashKey(c) => {
                self.splash = false;
                self.hy_splash_action(c);
            }
            HyHit::IdeaRow(i) => {
                if let Mode::Ideas(v) = &mut self.mode {
                    let again = v.sel == i && v.input.is_empty();
                    v.sel = i;
                    v.input.clear();
                    if again || double {
                        let v = (**v).clone();
                        self.on_ideas_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::TicketTab(i) => {
                if let Mode::Tickets(v) = &self.mode {
                    let mut v = (**v).clone();
                    let n = v.tabs.len();
                    v.tab = (i + n - 1) % n;
                    self.on_tickets_key(v, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                }
            }
            HyHit::TicketRow(i) => {
                if let Mode::Tickets(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        let v = (**v).clone();
                        self.on_tickets_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::RaceAgent(i) => {
                if let Mode::RaceNew(v) = &mut self.mode {
                    v.row = 1;
                    v.cur = i;
                    if let Some(p) = v.picked.get_mut(i) {
                        *p = !*p;
                    }
                }
            }
            HyHit::RaceGo => {
                if let Mode::RaceNew(v) = &self.mode {
                    let v = (**v).clone();
                    self.on_race_new_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            HyHit::RaceRow(i) => {
                if let Mode::Race(v) = &mut self.mode {
                    v.sel = i;
                    v.confirm = false;
                }
            }
            HyHit::RaceKey(c) => {
                if let Mode::Race(v) = &self.mode {
                    let v = (**v).clone();
                    let code = if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) };
                    self.on_race_key(v, &KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            HyHit::RaceOpen(id) => self.open_race(id),
            HyHit::MenuPick(i) => self.menu_pick(i),
            HyHit::HistRow(i) => {
                if let Mode::History { sel } = &mut self.mode {
                    let again = *sel == i;
                    *sel = i;
                    if again || double {
                        self.on_history_key(i, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::GoTo => self.mode = Mode::GoTo { query: String::new(), sel: 0 },
            HyHit::GoPick(i) => {
                if let Mode::GoTo { query, sel } = &mut self.mode {
                    let again = *sel == i;
                    *sel = i;
                    let q = query.clone();
                    if again || double {
                        self.on_goto_key(q, i, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::MemRow(i) => {
                if let Mode::Memory { sel } = &mut self.mode {
                    let again = *sel == i;
                    *sel = i;
                    if again || double {
                        self.on_memory_key(i, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::BranchRow(i) => {
                if let Mode::Branch(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        let v = (**v).clone();
                        self.on_branch_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::BranchChoice(i) => {
                if let Mode::Branch(v) = &self.mode {
                    let v = (**v).clone();
                    self.branch_choose(v, i);
                }
            }
            HyHit::FindTab(i) => {
                if let Mode::Find(v) = &self.mode
                    && v.tab != i
                {
                    let v = (**v).clone();
                    self.on_find_key(v, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                }
            }
            HyHit::FindRow(i) => {
                if let Mode::Find(v) = &mut self.mode {
                    let again = v.sel == i;
                    v.sel = i;
                    v.refresh_preview();
                    if again || double {
                        let v = (**v).clone();
                        self.on_find_key(v, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::SideEdge => self.hy.drag = Some(Drag::Side),
            HyHit::ScrollBar(term) => {
                self.hy.drag = Some(Drag::Scroll(term));
                if let (Some((t, r, total)), Some(pos)) = (self.hy.bar, self.hover)
                    && t == term
                {
                    let from_bottom = r.bottom().saturating_sub(pos.y + 1) as usize;
                    self.scroll_to(term, (from_bottom * total / r.height.max(1) as usize).min(total));
                }
            }
            HyHit::MapNode(i) => {
                if let Some(crate::client::View::Map(v)) = &mut self.view {
                    let again = v.sel == i;
                    v.sel = i;
                    if again || double {
                        self.on_view_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
            }
            HyHit::ShipGo => {
                if let Mode::Ship(ask) = std::mem::replace(&mut self.mode, Mode::Normal) {
                    let task = ask.task.clone();
                    self.notify(format!("shipping {}…", task.branch), false);
                    self.spawn_bg(move || crate::client::Bg::Done(crate::client::tasks::ship(&task), false));
                }
            }
            HyHit::Pr(i) => {
                if let Some((dir, n)) = self.hy.pr_keys.get(i).cloned() {
                    self.mode = Mode::Normal;
                    self.open_pr(dir, n);
                }
            }
            HyHit::ViewKey(c) => {
                let code = match c {
                    '\x1b' => KeyCode::Esc,
                    '\n' => KeyCode::Enter,
                    c => KeyCode::Char(c),
                };
                self.on_view_key(&KeyEvent::new(code, KeyModifiers::NONE));
            }
        }
    }

    /// A key for the settings overlay (shared with clicks).
    pub(in crate::client) fn hy_settings_key(&mut self, k: &KeyEvent) {
        let Mode::HySettings(v) = self.mode.clone() else { return };
        let mut v = *v;
        let open = self.on_settings_view_key(&mut v, k);
        if !open {
            self.mode = Mode::Normal;
        } else if matches!(self.mode, Mode::HySettings(_)) {
            self.mode = Mode::HySettings(Box::new(v));
        }
    }
}
