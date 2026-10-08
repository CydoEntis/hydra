//! Hydra's queue: work waiting for an agent. Fewer than `queue_at_once` running, the next
//! starts in its own worktree; when its agent finishes, it's yours to review. A ticket's
//! tracker is told both (in progress, then review).

use super::*;
use crate::tickets::{Progress, Ticket};

/// How many other names queued work tries when its worktree's folder is already there.
const FOLDER_TRIES: usize = 8;

impl Daemon {
    pub(super) fn enqueue(&mut self, mut item: QueueItem) {
        item.id = self.next_queue_id;
        self.next_queue_id += 1;
        item.state = QueueState::Waiting;
        item.worked = false;
        self.queue.push(item);
    }

    /// Move work along: finished agents to review, closed ones out, the next ones in.
    pub(super) fn run_queue(&mut self) {
        let mut reviews = Vec::new();
        let before = self.queue.clone();
        for q in &mut self.queue {
            let QueueState::Running(term) = q.state else { continue };
            let Some(t) = self.terms.get(&term) else { continue };
            q.worked |= t.status == Status::Working;
            // Done with its turn (or waiting at its prompt again) after working on it.
            if q.worked && matches!(t.status, Status::Done | Status::Idle) {
                q.state = QueueState::Review(term);
                reviews.push(q.clone());
            }
        }
        // Closed: it's off the list (finished or not, you've dealt with it).
        self.queue.retain(|q| match q.state {
            QueueState::Running(t) | QueueState::Review(t) => self.terms.contains_key(&t),
            _ => true,
        });
        for q in reviews {
            self.mark_ticket(&q, Progress::Review);
        }
        let busy = self.queue.iter().filter(|q| matches!(q.state, QueueState::Starting | QueueState::Running(_))).count();
        let room = (self.cfg.queue_at_once.max(1) as usize).saturating_sub(busy);
        let next: Vec<u64> = self.queue.iter().filter(|q| q.state == QueueState::Waiting).take(room).map(|q| q.id).collect();
        for id in next {
            self.start_queued(id);
        }
        if self.queue != before {
            self.dirty = true;
        }
    }

    /// Make the worktree for queued work (off the loop); its agent starts once it's there.
    fn start_queued(&mut self, id: u64) {
        let Some(q) = self.queue.iter_mut().find(|q| q.id == id) else { return };
        q.state = QueueState::Starting;
        let (dir, branch, template) = (q.project.clone(), q.branch.clone(), self.cfg.worktree.dir.clone());
        let mut taken: Vec<String> = git_branches_quick(&dir);
        let first = free_branch(&branch, &taken);
        q.branch = first.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            // A folder of that name left from before: the next name.
            let mut name = first;
            let mut result = create_trusted_worktree(&dir, &name, None, &template);
            for _ in 0..FOLDER_TRIES {
                if !result.as_ref().is_err_and(|e| e.ends_with("already exists")) {
                    break;
                }
                taken.push(name);
                name = free_branch(&branch, &taken);
                result = create_trusted_worktree(&dir, &name, None, &template);
            }
            let _ = tx.blocking_send(Ev::QueueWorktree { id, result });
        });
    }

    pub(super) fn queue_worktree(&mut self, id: u64, result: Result<(PathBuf, String), String>) {
        let Some(q) = self.queue.iter().find(|q| q.id == id).cloned() else { return };
        let started = result.and_then(|(path, _)| {
            // Beside your work, not in front of it: where you are stays where you are.
            let was = self.active_ws;
            self.command(0, Command::NewWorkspace { cwd: Some(path.clone()), name: None, cmd: Some(q.cmd.clone()) }).map_err(|e| format!("{e:#}"))?;
            let ws = self.workspaces.last_mut().ok_or("no workspace")?;
            ws.worktree = true;
            let term = ws.tab().map(|t| t.focus).ok_or("no pane")?;
            self.active_ws = was;
            self.made_worktrees.push(path.clone());
            self.worktree_hook(&path, true);
            Ok(term)
        });
        // The branch it really got (another name when a folder was in the way).
        let branch = started.as_ref().ok().and_then(|t| self.terms.get(t)).and_then(|t| t.head.as_ref()).map(|h| h.branch.clone());
        let Some(item) = self.queue.iter_mut().find(|q| q.id == id) else { return };
        if let Some(b) = branch {
            item.branch = b;
        }
        match started {
            Ok(term) => {
                item.state = QueueState::Running(term);
                self.mark_ticket(&q, Progress::Started);
            }
            Err(e) => item.state = QueueState::Failed(e),
        }
        self.dirty = true;
    }

    /// Tell the ticket's tracker (in the background); the result shows as a notice.
    fn mark_ticket(&self, q: &QueueItem, progress: Progress) {
        let Some(qt) = q.ticket.clone() else { return };
        let cfg = self.cfg.tickets.clone();
        let (dir, tx) = (q.project.clone(), self.tx.clone());
        let repo = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let note = format!("branch {} in {repo}", q.branch);
        let ticket = Ticket { key: qt.key, id: qt.id, title: q.title.clone(), url: qt.url, state: String::new(), meta: String::new(), body: String::new() };
        tokio::task::spawn_blocking(move || {
            let result = crate::tickets::mark(&cfg, &qt.source, &dir, &ticket, progress, &note);
            let _ = tx.blocking_send(Ev::TicketMarked(result));
        });
    }
}

/// The repository's branches, read from its files (no git process: this runs on the loop).
fn git_branches_quick(dir: &std::path::Path) -> Vec<String> {
    let Some(head) = crate::gitfs::head(dir) else { return Vec::new() };
    let heads = head.main_root.join(".git").join("refs").join("heads");
    let mut out = Vec::new();
    let mut stack = vec![(heads.clone(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = format!("{prefix}{}", e.file_name().to_string_lossy());
            if e.path().is_dir() {
                stack.push((e.path(), format!("{name}/")));
            } else {
                out.push(name);
            }
        }
    }
    // Packed refs too.
    if let Ok(packed) = std::fs::read_to_string(head.main_root.join(".git").join("packed-refs")) {
        out.extend(packed.lines().filter_map(|l| l.split_once(" refs/heads/").map(|(_, b)| b.to_string())));
    }
    out
}

/// `branch`, or `branch-2`, `branch-3`… when it's taken.
pub(super) fn free_branch(branch: &str, taken: &[String]) -> String {
    let branch = if branch.is_empty() { "task" } else { branch };
    std::iter::once(branch.to_string()).chain((2..).map(|n| format!("{branch}-{n}"))).find(|b| !taken.contains(b)).unwrap_or_default()
}
