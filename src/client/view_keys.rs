//! Keys inside the full-pane views and popups that aren't seshi's own.

use super::*;

impl App {
    /// Keys while a view replaces the pane area (single letters act directly).
    pub(super) fn on_view_key(&mut self, k: &KeyEvent) {
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

    /// Returns false when the view closes.
    pub(super) fn on_pr_key(&mut self, v: &mut pr::PrView, k: &KeyEvent) -> bool {
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
                    files::open_url(&i.url);
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

    /// Returns false when the view closes.
    pub(super) fn on_changes_key(&mut self, v: &mut views::ChangesView, k: &KeyEvent) -> bool {
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
                        'm' => {
                            let dir = task.dir.clone();
                            self.spawn_bg(move || Bg::Merged(tasks::merge(&task), dir));
                        }
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
        let order = v.order();
        let dir_key = design::path_key(&v.dir);
        let Some(r) = v.review.as_mut() else { return c != '\x1b' };
        let pos = order.iter().position(|i| *i == r.sel).unwrap_or(0);
        let go = |r: &mut tasks::Review, to: usize| {
            if let Some(i) = order.get(to) {
                r.sel = *i;
                r.diff_sel = None;
                r.scroll = 0;
            }
        };
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => go(r, (pos + 1).min(order.len().saturating_sub(1))),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => go(r, pos.saturating_sub(1)),
            // Mark reviewed (or not): it sinks, and the next one to look at comes up.
            KeyCode::Char('x') => {
                let Some(file) = r.files.get(r.sel).map(|f| f.path.clone()) else { return true };
                let marks = self.hy.saved.reviewed.entry(dir_key).or_default();
                let now = !v.reviewed.contains(&file);
                if now {
                    marks.insert(file.clone(), views::fingerprint(&v.dir, &file));
                    v.reviewed.insert(file.clone());
                } else {
                    marks.remove(&file);
                    v.reviewed.remove(&file);
                }
                self.hy.save();
                let order = v.order();
                if let Some(r) = v.review.as_mut() {
                    let next = if now { order.iter().copied().find(|i| !v.reviewed.contains(&r.files[*i].path)) } else { None };
                    if let Some(i) = next.or_else(|| order.iter().copied().find(|i| r.files[*i].path == file)) {
                        r.sel = i;
                        r.diff_sel = None;
                        r.scroll = 0;
                    }
                    if now && v.reviewed.len() == r.files.len() {
                        self.notify("all reviewed".into(), false);
                    }
                }
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
            KeyCode::Char('m') if v.linked => v.confirm = Some((format!("Merge {} into {}, then close it and remove its worktree and branch?", r.task.branch, r.task.base), 'm')),
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
    pub(super) fn on_files_tree_key(&mut self, v: &mut views::FilesTree, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.hy.preview_rect.height.saturating_sub(4).max(5) as usize;
        // Editing the file in the preview.
        if let Some(ed) = v.edit.as_mut() {
            match k.code {
                KeyCode::Char('s') if ctrl => match ed.save() {
                    Ok(views::Saved::Done) => self.notify(format!("saved {}", ed.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), false),
                    Ok(views::Saved::ChangedOnDisk) => {
                        self.notify("the file changed since you opened it (an agent?): Ctrl+S again overwrites it, Esc keeps theirs".into(), true)
                    }
                    Err(e) => self.notify(e, true),
                },
                KeyCode::Esc if ed.dirty && !ed.warned => {
                    ed.warned = true;
                    self.notify("unsaved changes: Ctrl+S saves, Esc again throws them away".into(), true);
                }
                KeyCode::Esc => {
                    v.edit = None;
                    // Show what's on disk now.
                    v.preview = None;
                    v.refresh_preview();
                    return true;
                }
                KeyCode::Up => ed.go(-1, 0),
                KeyCode::Down => ed.go(1, 0),
                KeyCode::Left => ed.go(0, -1),
                KeyCode::Right => ed.go(0, 1),
                KeyCode::PageUp => ed.go(-(page as isize), 0),
                KeyCode::PageDown => ed.go(page as isize, 0),
                KeyCode::Home => ed.home(),
                KeyCode::End => ed.end(),
                KeyCode::Enter => ed.newline(),
                KeyCode::Backspace => ed.backspace(),
                KeyCode::Delete => ed.delete(),
                KeyCode::Tab => {
                    for _ in 0..4 {
                        ed.insert(' ');
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    ed.warned = false;
                    ed.insert(c);
                }
                _ => {}
            }
            // Keep the cursor on screen.
            let row = ed.row;
            if row < v.scroll {
                v.scroll = row;
            } else if row >= v.scroll + page {
                v.scroll = row + 1 - page;
            }
            return true;
        }
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
        let lines = v.preview.as_ref().map(|(_, l)| l.len()).unwrap_or(0);
        // In the preview: the arrows read the file; ← (or Esc) goes back to the tree.
        if v.in_preview {
            match k.code {
                KeyCode::Left | KeyCode::Esc | KeyCode::Char('h') => v.in_preview = false,
                KeyCode::Down | KeyCode::Char('j') => v.scroll = (v.scroll + 1).min(lines.saturating_sub(1)),
                KeyCode::Up | KeyCode::Char('k') => v.scroll = v.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = (v.scroll + page).min(lines.saturating_sub(1)),
                KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(page),
                KeyCode::Home | KeyCode::Char('g') => v.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => v.scroll = lines.saturating_sub(page),
                // Everything else (i, e, y, o, d, Enter) works as from the tree.
                _ => {
                    v.in_preview = false;
                    let keep = self.on_files_tree_key(v, k);
                    if v.edit.is_none() && !matches!(k.code, KeyCode::Enter) {
                        v.in_preview = true;
                    }
                    return keep;
                }
            }
            return true;
        }
        match k.code {
            KeyCode::Right if node.as_ref().is_some_and(|n| !n.is_dir) && lines > 0 => v.in_preview = true,
            KeyCode::Esc => return false,
            // Read the preview: page, or a line at a time with Shift.
            KeyCode::PageDown => v.scroll = (v.scroll + page).min(lines.saturating_sub(1)),
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(page),
            KeyCode::Down if k.modifiers.contains(KeyModifiers::SHIFT) => v.scroll = (v.scroll + 1).min(lines.saturating_sub(1)),
            KeyCode::Up if k.modifiers.contains(KeyModifiers::SHIFT) => v.scroll = v.scroll.saturating_sub(1),
            // Edit it right here.
            KeyCode::Char('i') => {
                if let Some(n) = node.as_ref().filter(|n| !n.is_dir) {
                    match views::Edit::open(&n.path) {
                        Ok(mut ed) => {
                            ed.row = v.scroll.min(ed.lines.len().saturating_sub(1));
                            v.edit = Some(ed);
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
            }
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
                    return self.view.is_some();
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

    /// Type a message into an agent and press Enter. Several lines go as one paste, so they
    /// arrive as one message.
    pub(super) fn send_message(&mut self, term: TermId, text: &str) {
        let bracketed = self.parsers.get(&term).is_some_and(|p| p.screen().bracketed_paste());
        let data = if text.contains('\n') && bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text.replace('\n', " ") };
        self.send(ClientMsg::Input { term, data: data.into_bytes() });
        // Enter a moment later, so the program has taken the text first.
        let out = self.out.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            let _ = out.send(ClientMsg::Input { term, data: b"\r".to_vec() });
        });
        let who = self.snap.terms.get(&term).map(|t| t.display_name()).unwrap_or_default();
        self.notify(format!("sent to {who}"), false);
    }

    pub(super) fn on_talk_key(&mut self, term: TermId, mut input: String, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // From the sidebar, done means back to the sidebar cursor (next agent: ↓ Space).
        let back = if self.hy.talk_back && self.hy.cursor.is_some() { Mode::Side } else { Mode::Normal };
        match k.code {
            KeyCode::Esc => {
                self.mode = back;
                return;
            }
            KeyCode::Enter if k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => input.push('\n'),
            KeyCode::Char('o') if ctrl || input.is_empty() => {
                self.cmd(Command::FocusPane { term });
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                if !input.trim().is_empty() {
                    let text = std::mem::take(&mut input);
                    self.send_message(term, &text);
                }
                self.mode = back;
                return;
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if !ctrl => input.push(c),
            _ => {}
        }
        self.mode = Mode::Talk { term, input };
    }

    /// Keys in the settings view. Returns false when it closes.
    pub(super) fn on_settings_view_key(&mut self, v: &mut design::SettingsView, k: &KeyEvent) -> bool {
        use modal::{Cat, Kind};
        let cat = Cat::ALL[v.cat.min(Cat::ALL.len() - 1)];
        let rows = design::settings_rows(cat);
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
            if let Some(design::SRow::Theme(i)) = &row
                && let Some(name) = crate::theme::BUILTIN.get(*i)
            {
                app.save_setting("theme", (*name).into());
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
                    // A folder: pick it in the folder browser (Esc comes back here).
                    Kind::Folder => {
                        let cur = self.cfg.start_dir().or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())).unwrap_or_default();
                        let mut fd = hydra::Finder::new(&cur);
                        fd.for_setting = Some(s.path);
                        self.mode = Mode::Finder(Box::new(fd));
                        return true;
                    }
                    Kind::Text | Kind::Program(_) => {
                        let cur = modal::current(&self.cfg, s.path).and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default();
                        v.editing = Some(cur);
                    }
                    Kind::Key => v.capturing = true,
                    _ => step(self, 1),
                },
                Some(design::SRow::Bind { acts, .. }) if acts.len() == 1 => v.capturing = true,
                Some(design::SRow::Bind { .. }) => self.notify("that row has several keys: change them in the config file (o)".into(), false),
                Some(design::SRow::Theme(_)) => step(self, 1),
                None => {}
            },
            _ => {}
        }
        true
    }

    /// Generic list movement shared by the panels. Returns true if the key was handled.
    pub(super) fn list_move(sel: &mut usize, len: usize, k: &KeyEvent) -> bool {
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

    pub(super) fn on_toolbox_key(&mut self, mut v: toolbox::ToolboxView, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            // This project, or everywhere.
            KeyCode::Tab | KeyCode::BackTab => {
                v.everywhere = !v.everywhere;
                v.sel = 0;
            }
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

    pub(super) fn on_quick_key(&mut self, mut q: modal::Quick, k: &KeyEvent) {
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

    pub(super) fn submit_quick(&mut self, q: modal::Quick) {
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

    pub(super) fn submit_prompt(&mut self, kind: PromptKind, input: String) {
        let input = input.trim().to_string();
        match kind {
            PromptKind::RenamePane(term) => self.cmd(Command::RenamePane { term, name: input }),
            PromptKind::RenameProject(key) => {
                if input.is_empty() {
                    self.hy.saved.names.remove(&key);
                } else {
                    self.hy.saved.names.insert(key, input);
                }
                self.hy.save();
            }
            PromptKind::RenameTab(ws, tab) => self.cmd(Command::RenameTab { ws, tab, name: input }),
            PromptKind::NewWorkspace => {
                // A group starts with a shell where you are.
                let name = (!input.is_empty()).then_some(input);
                self.cmd(Command::NewWorkspace { cwd: Some(self.here_dir()), name, cmd: None });
            }
            _ => {}
        }
    }

    /// Rows for the worktree picker: matching worktrees, then "create <query>" when the
    /// query isn't already an existing branch.
    pub(super) fn worktree_rows(&self, items: Option<&[WorktreeEntry]>, query: &str) -> Vec<WtRow> {
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
    pub(super) fn open_worktree(&mut self, w: &WorktreeEntry, cmd: Option<String>) {
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
}
