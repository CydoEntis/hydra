//! Saving sessions as they change, and rebuilding them after a restart; sleeping and waking agents.

use super::*;

impl Daemon {
    pub(super) fn mass_exit(&self) -> bool {
        self.natural_exits.iter().filter(|t| t.elapsed() < MASS_EXIT_WINDOW).count() >= 2
    }

    /// Write the session file if it changed. `force` skips the mass-exit guard.
    pub(super) fn persist(&mut self, force: bool) {
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

    pub(super) fn saved(&self) -> persist::Saved {
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
        // Only what hasn't started: the panes of the rest come back as ordinary sessions.
        let queue = self.queue.iter().filter(|q| matches!(q.state, QueueState::Waiting | QueueState::Starting)).map(|q| QueueItem { state: QueueState::Waiting, ..q.clone() }).collect();
        persist::Saved { workspaces, active, made_worktrees: self.made_worktrees.clone(), queue }
    }

    /// The command that brings a saved pane back: an agent's resume command, the pane's
    /// original command, or nothing (a shell).
    pub(super) fn restore_cmd(&self, p: &persist::SavedPane) -> Option<String> {
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

    pub(super) fn restore(&mut self, mut saved: persist::Saved) {
        self.made_worktrees = std::mem::take(&mut saved.made_worktrees);
        self.queue = std::mem::take(&mut saved.queue);
        self.next_queue_id = self.queue.iter().map(|q| q.id + 1).max().unwrap_or(1);
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

    /// The command that resumes an agent's conversation, if its kind supports it.
    pub(super) fn resume_cmd(&self, t: &Term) -> Option<String> {
        let def = t.agent.as_ref().and_then(|n| self.agents.iter().find(|a| &a.name == n))?;
        match (&def.resume, &t.session) {
            (Some(tpl), Some(id)) => Some(tpl.replace("{session}", id)),
            _ => def.resume_last.clone(),
        }
    }

    /// Stop agents that have sat finished or idle longer than `sleep_after` (never the
    /// focused one, and only ones that can be resumed).
    pub(super) fn sleep_idle(&mut self) {
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
    pub(super) fn wake(&mut self, old: TermId) -> Option<TermId> {
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
    pub(super) fn relocate(&mut self, old: TermId, dest: &std::path::Path) {
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
        self.poll_git_soon();
        self.dirty = true;
    }
}
