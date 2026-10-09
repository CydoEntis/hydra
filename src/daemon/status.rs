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
/// How long a "needs you" with no question anywhere on screen lasts (you typed nothing).
pub(super) const UNASKED_QUESTION_GONE_AFTER: Duration = Duration::from_secs(8);
/// A numbered choice an agent offers ("❯ 1. Yes"): a question, whatever its wording.
static CHOICES: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?m)^[\s│┃]*[❯>]?\s*1\.\s+\S").unwrap());
/// A "done" held while subagents run is let through once none has been heard from for this
/// long (one died without saying so). Background agents can run, and wait on CI, for a while.
const SUBAGENTS_QUIET_AT_MOST: Duration = Duration::from_secs(60 * 60);
/// A permission ping this soon after a "working" is Claude repeating one already answered.
const REPEATED_PROMPT_WITHIN: Duration = Duration::from_secs(5);
/// The progress indicator gone this long with no turn-end: the turn was cancelled (Esc).
/// Agents whose hooks report only the end of a turn.
const HOOKS_END_ONLY: &[&str] = &["codex"];
/// A turn starts from something you typed: only that long after it is the screen checked.
const TURN_START_WITHIN: Duration = Duration::from_secs(30);
const CANCELLED_AFTER: Duration = Duration::from_secs(2);
/// An agent read from its screen (no hooks) is done only once its screen has been still this
/// long: working ones redraw a timer every second.
const SCREEN_DONE_QUIET: Duration = Duration::from_secs(5);

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
                        t.subagent_seen = Some(Instant::now());
                        if sa.start {
                            // Its start, or news from one already running (listed once; one
                            // missed while seshi was away joins the list here).
                            if sa.id.is_empty() || !t.subagents.iter().any(|(id, _)| *id == sa.id) {
                                t.subagents.push((sa.id, sa.kind));
                            }
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
                    // An agent stays in the group of the folder it started in: only its first
                    // report places it, so its row doesn't jump as it works in other folders.
                    if let Some(c) = cwd.filter(|c| c.is_dir() && !(t.hooked && t.agent.is_some())) {
                        t.cwd = c;
                        t.cwd_reported = true;
                        t.refresh_head();
                    }
                }
                self.hook(term, agent, status, &event);
    }

    pub(super) fn hook(&mut self, term: TermId, agent: String, status: HookStatus, event: &str) {
        tracing::debug!("hook for pane {term}: {event} -> {status:?}");
        if let Some(t) = self.terms.get_mut(&term) {
            let name = if event.is_empty() { "status report".to_string() } else { event.to_string() };
            t.last_hook = Some((name, term::unix_now()));
            self.dirty = true;
        }
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
                    self.set_status(term, s, "hook: its last subagent finished");
                }
                self.dirty = true;
                return;
            }
            // Claude sometimes repeats a permission ping after you've already answered and it's
            // moved on: ignore one that comes right after a "working".
            HookStatus::Blocked
                if event == "Notification:permission_prompt" && t.last_working_hook.is_some_and(|w| w.elapsed() < REPEATED_PROMPT_WITHIN) =>
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
        self.set_status(term, new, &format!("hook: {}", if event.is_empty() { "status report" } else { event }));
        self.dirty = true;
    }

    pub(super) fn set_status(&mut self, term: TermId, new: Status, why: &str) {
        let Some(t) = self.terms.get_mut(&term) else { return };
        if t.status == new {
            return;
        }
        t.status_why = why.to_string();
        if let Some(seen) = why.strip_prefix("screen: ") {
            t.last_screen = Some((seen.to_string(), term::unix_now()));
        }
        let turn = (t.status == Status::Working && t.agent.is_some())
            .then(|| (t.agent.clone().unwrap_or_default(), t.place(), term::unix_now().saturating_sub(t.status_since)));
        t.blocked_at = (new == Status::Blocked).then(Instant::now);
        t.status = new;
        t.status_since = term::unix_now();
        self.dirty = true;
        if let Some((agent, place, secs)) = turn {
            self.note_turn(agent, place, secs);
        }
        if matches!(new, Status::Blocked | Status::Done) {
            self.broadcast(|c| c.attach, ServerMsg::Attention { term, status: new });
            // No window open: the server tells you itself.
            if !self.has_viewer()
                && let Some(t) = self.terms.get(&term)
            {
                let agent = t.agent.clone().unwrap_or_else(|| "an agent".into());
                let place = t.place();
                let summary = t.summary.trim();
                let body = if summary.is_empty() { place } else { format!("{place} · {summary}") };
                let (kind, what) = if new == Status::Blocked { (crate::alert::Kind::Needs, "needs you") } else { (crate::alert::Kind::Done, "finished") };
                crate::alert::alert(&self.cfg.notify, kind, &format!("{agent} {what}"), &body, Some(crate::reveal::link(term)));
            }
        }
    }

    /// Phone alerts for agents that have waited `phone_after` with nobody answering (or,
    /// when asked for, finished with nobody looking). Each status at most once.
    pub(super) fn phone_alerts(&mut self) {
        let n = &self.cfg.notify;
        if n.phone_topic.trim().is_empty() {
            return;
        }
        let now = term::unix_now();
        for t in self.terms.values_mut() {
            let Some(agent) = t.agent.clone() else { continue };
            let wanted = t.status == Status::Blocked || (t.status == Status::Done && n.phone_done);
            if !wanted || t.phoned == t.status_since || now.saturating_sub(t.status_since) < n.phone_after {
                continue;
            }
            t.phoned = t.status_since;
            let place = t.place();
            let (title, body) = crate::alert::phone_message(&agent, &place, t.status == Status::Blocked, &t.summary, n.phone_text);
            let cfg = n.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::alert::phone(&cfg, &title, &body) {
                    tracing::warn!("phone alert didn't go: {e}");
                }
            });
        }
    }

    pub(super) fn update_statuses(&mut self) {
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
                    changes.push((t.id, Status::Idle, "you're looking at it".to_string()));
                }
                // Subagents went quiet for good (one died without reporting): don't hold "done"
                // forever.
                if t.done_held.is_some_and(|h| h.elapsed() > SUBAGENTS_QUIET_AT_MOST)
                    && t.subagent_seen.is_none_or(|s| s.elapsed() > SUBAGENTS_QUIET_AT_MOST)
                {
                    changes.push((t.id, if Some(t.id) == focused { Status::Idle } else { Status::Done }, "subagents went quiet".to_string()));
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
                        changes.push((t.id, Status::Idle, "screen: you typed and the question went away".to_string()));
                    }
                }
                // A permission ask that auto mode (or the agent) settled by itself also sends
                // no hook: a while on, with no question on screen at all, it's over.
                if t.status == Status::Blocked
                    && t.blocked_at.is_some_and(|b| b.elapsed() > UNASKED_QUESTION_GONE_AFTER)
                    && !self.questions.iter().any(|(q, ..)| q.term == t.id)
                {
                    let text = t.tail_text(rows);
                    let asking = CHOICES.is_match(&text) || self.agents.iter().find(|a| &a.name == name).is_some_and(|d| d.blocked.iter().any(|r| r.is_match(&text)));
                    if !asking {
                        changes.push((t.id, if Some(t.id) == focused { Status::Idle } else { Status::Done }, "screen: the question settled itself".to_string()));
                    }
                }
                // The progress indicator went away and no turn-end came: cancelled (Esc).
                if t.status == Status::Working && t.done_held.is_none() && t.progress_off.is_some_and(|p| p.elapsed() > CANCELLED_AFTER) {
                    changes.push((t.id, Status::Idle, "progress bar gone, no turn end: cancelled".to_string()));
                }
                // Agents whose hooks only say a turn ended (Codex's notify): the start of the
                // next turn shows on screen only.
                if HOOKS_END_ONLY.contains(&name.as_str())
                    && matches!(t.status, Status::Idle | Status::Done)
                    && t.last_input.elapsed() < TURN_START_WITHIN
                    // Typed since it finished: not the last turn's line still on screen.
                    && t.last_input.elapsed().as_secs() < term::unix_now().saturating_sub(t.status_since)
                    && let Some(d) = self.agents.iter().find(|a| &a.name == name)
                    && let Some(r) = d.working.iter().find(|r| r.is_match(&t.tail_text(rows)))
                {
                    changes.push((t.id, Status::Working, format!("screen: matched `{}`", r.as_str())));
                }
                continue;
            }
            let def = self.agents.iter().find(|a| &a.name == name);
            let text = t.tail_text(rows);
            let blocked = def.and_then(|d| d.blocked.iter().find(|r| r.is_match(&text)));
            let working: Option<String> = match def {
                Some(d) if !d.working.is_empty() => d.working.iter().find(|r| r.is_match(&text)).map(|r| format!("screen: matched `{}`", r.as_str())),
                _ => (t.last_output.elapsed() < window && t.last_output.saturating_duration_since(t.last_input) > grace)
                    .then(|| "screen: it's printing".to_string()),
            };
            // Its working line can leave the rows looked at while it still runs (a long block
            // printed under it): working until the screen has also gone still, so it doesn't
            // flip to done (and alert) and back on every check.
            let still_going = t.status == Status::Working && t.last_output.elapsed() < SCREEN_DONE_QUIET;
            let (new, why) = if let Some(r) = blocked {
                (Status::Blocked, format!("screen: matched `{}`", r.as_str()))
            } else if let Some(why) = working {
                (Status::Working, why)
            } else if still_going {
                (Status::Working, "screen: still printing".to_string())
            } else if Some(t.id) == focused {
                (Status::Idle, "screen: quiet, and you're on it".to_string())
            } else if matches!(t.status, Status::Working | Status::Done) {
                (Status::Done, "screen: went quiet after working".to_string())
            } else {
                (Status::Idle, "screen: quiet".to_string())
            };
            if new != t.status {
                changes.push((t.id, new, why));
            }
        }
        for (id, s, why) in changes {
            if s != Status::Working
                && let Some(t) = self.terms.get_mut(&id)
            {
                if t.done_held.take().is_some() {
                    t.subagents.clear();
                }
                t.progress_off = None;
            }
            self.set_status(id, s, &why);
        }
    }
}
