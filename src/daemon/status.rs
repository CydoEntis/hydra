//! Agent status: hook reports (checked on their own thread) and screen detection.

use super::*;

/// A status report waiting for its process chain to be checked.
pub(super) struct HookJob {
    pub(super) msg: ClientMsg,
    pub(super) pid: u32,
    /// The pane's own process, when there's a chain to check.
    pub(super) root: Option<u32>,
    pub(super) known: std::collections::HashSet<u32>,
}

/// One thread checks status reports' process chains, in the order they came, and hands
/// them back to the daemon loop.
pub(super) fn hook_thread(tx: mpsc::Sender<Ev>) -> std::sync::mpsc::Sender<HookJob> {
    let (jobs, rx) = std::sync::mpsc::channel::<HookJob>();
    let spawned = std::thread::Builder::new().name("hook-check".into()).spawn(move || {
        for job in rx {
            let (verdict, chain) = match job.root {
                Some(root) => scan::descends_from(job.pid, root, &job.known),
                None => (None, Vec::new()),
            };
            if tx.blocking_send(Ev::HookChecked { msg: job.msg, verdict, chain }).is_err() {
                break;
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!("couldn't start the hook thread: {e}");
    }
    jobs
}

/// How long after your last key, with the pane quiet, a question that's no longer on screen
/// counts as dismissed.
pub(super) const QUESTION_GONE_AFTER: Duration = Duration::from_secs(2);

impl Daemon {
    /// A status report whose sender checked out (`verdict`: did its process chain lead to
    /// the pane; None when it couldn't be traced and the secret decides).
    pub(super) fn apply_hook(&mut self, msg: ClientMsg, verdict: Option<bool>, chain: Vec<u32>) {
        let ClientMsg::Hook { term, agent, status, session, cwd, prompt, said, subagent, event, pid, transcript, model, name, .. } = msg else { return };
        match verdict {
            Some(false) => {
                tracing::info!("ignoring a status report for pane {term} from pid {pid} outside it");
                return;
            }
            Some(true) => {
                if let Some(t) = self.terms.get_mut(&term) {
                    t.trusted.extend(chain);
                }
            }
            None => {}
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

    pub(super) fn hook(&mut self, term: TermId, agent: String, status: HookStatus, event: &str) {
        tracing::debug!("hook for pane {term}: {event} -> {status:?}");
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

    pub(super) fn set_status(&mut self, term: TermId, new: Status) {
        let Some(t) = self.terms.get_mut(&term) else { return };
        if t.status == new {
            return;
        }
        t.blocked_at = (new == Status::Blocked).then(Instant::now);
        t.status = new;
        t.status_since = term::unix_now();
        self.dirty = true;
        if matches!(new, Status::Blocked | Status::Done) {
            self.ext_event(if new == Status::Blocked { "needs_you" } else { "agent_done" }, None, Some(term));
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

    pub(super) fn update_statuses(&mut self) {
        // Dev servers: up once their output shows the ready pattern (and which port, if it
        // prints a localhost URL).
        static URL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?:localhost|127\.0\.0\.1):(\d{2,5})").unwrap());
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
                        && let Some(p) = URL.captures(&text).and_then(|c| c[1].parse().ok())
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
                // A question dismissed with Esc sends no hook at all. Once you've typed since
                // it was asked, the pane has gone quiet, and no question shows on screen, it's
                // over.
                if t.status == Status::Blocked
                    && t.blocked_at.is_some_and(|b| t.last_input > b)
                    && t.last_input.elapsed() > QUESTION_GONE_AFTER
                    && t.last_output.elapsed() > QUESTION_GONE_AFTER
                {
                    let text = t.tail_text(rows);
                    let asking = self.agents.iter().find(|a| &a.name == name).is_some_and(|d| d.blocked.iter().any(|r| r.is_match(&text)));
                    if !asking {
                        changes.push((t.id, Status::Idle));
                    }
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
}
