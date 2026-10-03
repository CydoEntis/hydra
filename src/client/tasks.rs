//! Tasks: a worktree workspace with an agent working in it. The list shows what stage each
//! is at; the review screen shows what it changed and ships it (commit, merge, PR) or throws
//! it away.

use crate::protocol::{Snapshot, Status, TermId, WsId};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    NeedsYou,
    Ready,
    Working,
    Idle,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::NeedsYou => "needs you",
            Stage::Ready => "ready for review",
            Stage::Working => "working",
            Stage::Idle => "no changes yet",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub ws: WsId,
    pub name: String,
    pub branch: String,
    pub base: String,
    pub stage: Stage,
    pub summary: String,
    pub dirty: u32,
    pub ahead: u32,
    pub agent: Option<TermId>,
    pub dir: PathBuf,
    pub root: PathBuf,
}

/// Every linked-worktree workspace, most urgent first.
pub fn rows(snap: &Snapshot) -> Vec<TaskRow> {
    let mut out: Vec<TaskRow> = snap
        .workspaces
        .iter()
        .filter_map(|w| {
            let g = w.git.as_ref().filter(|g| g.linked)?;
            let terms: Vec<_> =
                w.tabs.iter().flat_map(|t| t.layout.leaves()).filter_map(|id| snap.terms.get(&id)).collect();
            let agent = terms.iter().filter(|t| t.agent.is_some()).min_by_key(|t| t.status.urgency());
            let status = agent.map(|a| a.status).unwrap_or(Status::None);
            let stage = match status {
                Status::Blocked => Stage::NeedsYou,
                Status::Working => Stage::Working,
                _ if g.dirty > 0 || g.ahead > 0 => Stage::Ready,
                _ => Stage::Idle,
            };
            let base = g.worktrees.iter().find(|e| e.main).map(|e| e.branch.clone()).unwrap_or_else(|| "main".into());
            let name = if w.name == format!("{}:{}", g.repo, g.branch) { g.branch.clone() } else { w.name.clone() };
            Some(TaskRow {
                ws: w.id,
                name,
                branch: g.branch.clone(),
                base,
                stage,
                summary: agent.map(|a| a.summary.clone()).unwrap_or_default(),
                dirty: g.dirty,
                ahead: g.ahead,
                agent: agent.map(|a| a.id),
                dir: w.cwd.clone(),
                root: g.root.clone(),
            })
        })
        .collect();
    out.sort_by_key(|r| (r.stage, r.ws));
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Changed {
    pub path: String,
    pub added: i64,
    pub removed: i64,
    pub untracked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub task: TaskRow,
    /// The commit the task branched from.
    pub merge_base: String,
    pub files: Vec<Changed>,
    pub sel: usize,
    pub diff: Vec<String>,
    pub scroll: u16,
    /// An action waiting for y/n: (label, key).
    pub confirm: Option<(String, char)>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("running git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("git failed");
        return Err(format!("git {}: {}", args.first().unwrap_or(&""), first.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Everything the task changed since it branched: commits plus uncommitted and new files.
pub fn load_review(task: TaskRow) -> Result<Review, String> {
    let dir = &task.dir;
    let merge_base = git(dir, &["merge-base", "HEAD", &task.base])?.trim().to_string();
    let mut files: Vec<Changed> = git(dir, &["diff", "--numstat", &merge_base])?
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let added = it.next()?.parse().unwrap_or(0);
            let removed = it.next()?.parse().unwrap_or(0);
            Some(Changed { added, removed, path: it.next()?.to_string(), untracked: false })
        })
        .collect();
    for path in git(dir, &["ls-files", "--others", "--exclude-standard"])?.lines().filter(|l| !l.is_empty()) {
        let added = std::fs::read_to_string(dir.join(path)).map(|s| s.lines().count() as i64).unwrap_or(0);
        files.push(Changed { path: path.to_string(), added, removed: 0, untracked: true });
    }
    let mut r = Review { task, merge_base, files, sel: 0, diff: Vec::new(), scroll: 0, confirm: None };
    r.diff = file_diff(&r);
    Ok(r)
}

/// The selected file's diff against where the task started.
pub fn file_diff(r: &Review) -> Vec<String> {
    let Some(f) = r.files.get(r.sel) else { return vec!["No changes yet.".into()] };
    if f.untracked {
        let body = std::fs::read_to_string(r.task.dir.join(&f.path)).unwrap_or_else(|_| "(binary or unreadable)".into());
        let mut out = vec![format!("new file {}", f.path)];
        out.extend(body.lines().take(2000).map(|l| format!("+{l}")));
        return out;
    }
    match git(&r.task.dir, &["diff", &r.merge_base, "--", &f.path]) {
        Ok(d) => d.lines().skip_while(|l| !l.starts_with("@@")).take(4000).map(|l| l.replace('\t', "    ")).collect(),
        Err(e) => vec![e],
    }
}

fn commit_all(t: &TaskRow) -> Result<bool, String> {
    if git(&t.dir, &["status", "--porcelain"])?.trim().is_empty() {
        return Ok(false);
    }
    git(&t.dir, &["add", "-A"])?;
    let msg = if t.summary.is_empty() { t.name.clone() } else { t.summary.clone() };
    git(&t.dir, &["commit", "-m", &msg])?;
    Ok(true)
}

pub fn commit(t: &TaskRow) -> Result<String, String> {
    Ok(if commit_all(t)? { format!("committed {} on {}", t.name, t.branch) } else { "nothing to commit".into() })
}

/// Commit what's left, then merge the task branch into the main checkout's branch.
pub fn merge(t: &TaskRow) -> Result<String, String> {
    commit_all(t)?;
    let msg = format!("Merge {}", t.branch);
    git(&t.root, &["merge", "--no-ff", &t.branch, "-m", &msg])?;
    Ok(format!("merged {} into {}", t.branch, t.base))
}

/// Commit what's left, push the branch and open a pull request with `gh`.
pub fn pull_request(t: &TaskRow) -> Result<String, String> {
    commit_all(t)?;
    git(&t.dir, &["push", "-u", "origin", &t.branch])?;
    let mut cmd = Command::new("gh");
    cmd.current_dir(&t.dir).args(["pr", "create", "--fill", "--head", &t.branch]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("running gh: {e} (is the GitHub CLI installed?)"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh pr create failed").to_string());
    }
    Ok(format!("opened {}", text.lines().last().unwrap_or("the pull request")))
}
