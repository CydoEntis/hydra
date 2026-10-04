//! Tasks: a worktree workspace with an agent working in it. The list shows what stage each
//! is at; the review screen shows what it changed and ships it (commit, merge, PR) or throws
//! it away.

use crate::protocol::{TermId, WsId};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Ready,
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
    /// Which file `diff` is for (or being loaded for); None: load it.
    pub diff_sel: Option<usize>,
    pub scroll: u16,
    /// An action waiting for y/n: (label, key).
    pub confirm: Option<(String, char)>,
}

/// The most of an untracked file read to show or count it.
const UNTRACKED_READ_CAP: u64 = 512 * 1024;

/// The start of a file, at most `cap` bytes, as text.
fn read_head(path: &std::path::Path, cap: u64) -> std::io::Result<String> {
    use std::io::Read;
    let mut s = Vec::new();
    std::fs::File::open(path)?.take(cap).read_to_end(&mut s)?;
    Ok(String::from_utf8_lossy(&s).into_owned())
}

/// What a diff needs, so it can be worked out off the UI thread.
#[derive(Debug, Clone)]
pub struct DiffJob {
    pub dir: std::path::PathBuf,
    pub merge_base: String,
    pub file: Option<Changed>,
}

impl Review {
    pub fn diff_job(&self) -> DiffJob {
        DiffJob { dir: self.task.dir.clone(), merge_base: self.merge_base.clone(), file: self.files.get(self.sel).cloned() }
    }
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
        let added = read_head(&dir.join(path), UNTRACKED_READ_CAP).map(|s| s.lines().count() as i64).unwrap_or(0);
        files.push(Changed { path: path.to_string(), added, removed: 0, untracked: true });
    }
    let mut r = Review { task, merge_base, files, sel: 0, diff: Vec::new(), diff_sel: Some(0), scroll: 0, confirm: None };
    r.diff = file_diff(&r);
    Ok(r)
}

/// The selected file's diff against where the task started.
pub fn file_diff(r: &Review) -> Vec<String> {
    diff_of(&r.diff_job())
}

pub fn diff_of(job: &DiffJob) -> Vec<String> {
    let Some(f) = &job.file else { return vec!["No changes yet.".into()] };
    if f.untracked {
        let body = read_head(&job.dir.join(&f.path), UNTRACKED_READ_CAP).unwrap_or_else(|_| "(binary or unreadable)".into());
        let mut out = vec![format!("new file {}", f.path)];
        out.extend(body.lines().take(2000).map(|l| format!("+{l}")));
        return out;
    }
    match git(&job.dir, &["diff", &job.merge_base, "--", &f.path]) {
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
    if !git(&t.root, &["status", "--porcelain"])?.trim().is_empty() {
        return Err(format!("{} has uncommitted changes; commit or stash them first", t.root.display()));
    }
    let msg = format!("Merge {}", t.branch);
    if let Err(e) = git(&t.root, &["merge", "--no-ff", &t.branch, "-m", &msg]) {
        // Leave the main checkout as it was, not mid-merge.
        let _ = git(&t.root, &["merge", "--abort"]);
        return Err(format!("couldn't merge {} (conflicts?): {e}", t.branch));
    }
    Ok(format!("merged {} into {}", t.branch, t.base))
}

/// Ship a branch: commit what's left, push it, and open a pull request (or, if it has one
/// already, the push updates it).
pub fn ship(t: &TaskRow) -> Result<String, String> {
    let committed = commit_all(t)?;
    git(&t.dir, &["push", "-u", "origin", &t.branch])?;
    let mut view = Command::new("gh");
    view.current_dir(&t.dir).args(["pr", "view", &t.branch, "--json", "number", "--jq", ".number"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        view.creation_flags(0x0800_0000);
    }
    if let Ok(o) = view.output()
        && o.status.success()
    {
        let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let what = if committed { "committed and pushed" } else { "pushed" };
        return Ok(format!("{what} {} to PR #{n}", t.branch));
    }
    pull_request(t)
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
