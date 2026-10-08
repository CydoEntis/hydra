//! Checkpoints: the state of a checkout after each agent turn, kept as git commits under
//! `refs/hydra/checkpoints/…` (never on a branch, never in your index), so a folder can be
//! rolled back to any of them. Everything here runs git and blocks; call it off the UI
//! thread and off the server's loop.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// How many checkpoints a list shows.
pub const SHOWN: usize = 60;

/// One saved state of a checkout.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub commit: String,
    /// Unix seconds.
    pub at: u64,
    /// What the agent was on when it was taken.
    pub what: String,
    /// "3 files, +40 −12" against the one before it.
    pub change: String,
}

fn git(dir: &Path, args: &[&str], index: Option<&Path>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    if let Some(i) = index {
        cmd.env("GIT_INDEX_FILE", i);
    }
    let out = crate::proc::quiet(&mut cmd).output().context("running git")?;
    if !out.status.success() {
        bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The ref a checkout's checkpoints hang from: one per folder (worktrees share refs).
pub fn ref_for(top: &Path) -> String {
    use sha2::Digest;
    let key = top.to_string_lossy().replace('\\', "/").to_lowercase();
    let hash = sha2::Sha256::digest(key.as_bytes());
    let short: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let name: String = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
    format!("refs/hydra/checkpoints/{name}-{short}")
}

/// The tree of everything in the checkout now (tracked and untracked, not ignored), written
/// through a scratch index so yours isn't touched.
fn tree_now(top: &Path) -> Result<String> {
    let real = PathBuf::from(git(top, &["rev-parse", "--path-format=absolute", "--git-path", "index"], None)?);
    let scratch = std::env::temp_dir().join(format!("hydra-checkpoint-{}-{}", std::process::id(), rand_suffix()));
    // Starting from yours makes `add` quick (its file stats are already known).
    if real.is_file() {
        std::fs::copy(&real, &scratch)?;
    }
    let tree = git(top, &["add", "-A", "."], Some(&scratch)).and_then(|_| git(top, &["write-tree"], Some(&scratch)));
    let _ = std::fs::remove_file(&scratch);
    tree
}

fn rand_suffix() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

/// Save the checkout's state now, unless nothing changed since the last checkpoint. Returns
/// the new checkpoint's commit, if one was made.
pub fn take(top: &Path, what: &str) -> Result<Option<String>> {
    let tree = tree_now(top)?;
    let r = ref_for(top);
    let last = git(top, &["rev-parse", "--verify", "--quiet", &r], None).ok().filter(|s| !s.is_empty());
    if let Some(l) = &last
        && git(top, &["rev-parse", &format!("{l}^{{tree}}")], None).ok().as_deref() == Some(tree.as_str())
    {
        return Ok(None);
    }
    // The first one hangs off HEAD, so even it shows what the agent changed.
    let parent = last.clone().or_else(|| git(top, &["rev-parse", "--verify", "--quiet", "HEAD"], None).ok().filter(|s| !s.is_empty()));
    let msg = if what.trim().is_empty() { "agent turn".to_string() } else { what.trim().chars().take(200).collect() };
    let mut args = vec!["-c", "user.name=hydra", "-c", "user.email=hydra@localhost", "commit-tree", tree.as_str(), "-m", msg.as_str()];
    if let Some(p) = &parent {
        args.extend(["-p", p.as_str()]);
    }
    let commit = git(top, &args, None)?;
    git(top, &["update-ref", &r, &commit], None)?;
    Ok(Some(commit))
}

/// The checkout's checkpoints, newest first.
pub fn list(top: &Path) -> Result<Vec<Checkpoint>> {
    let r = ref_for(top);
    if git(top, &["rev-parse", "--verify", "--quiet", &r], None).is_err() {
        return Ok(Vec::new());
    }
    let n = SHOWN.to_string();
    // Only hydra's own commits: the chain ends at the branch it started from.
    let log = git(top, &["log", "--author=hydra", "--format=%H%x09%ct%x09%s", "--shortstat", "-n", &n, &r], None)?;
    let mut out: Vec<Checkpoint> = Vec::new();
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(at), Some(what)) if h.len() >= 40 && at.parse::<u64>().is_ok() => {
                out.push(Checkpoint { commit: h.to_string(), at: at.parse().unwrap_or(0), what: what.to_string(), change: String::new() });
            }
            _ => {
                if let Some(last) = out.last_mut() {
                    last.change = shortstat(line);
                }
            }
        }
    }
    Ok(out)
}

/// " 3 files changed, 40 insertions(+), 12 deletions(-)" → "3 files, +40 −12".
fn shortstat(line: &str) -> String {
    let num = |word: &str| line.split(',').find(|p| p.contains(word)).and_then(|p| p.split_whitespace().next()).map(String::from);
    let mut out = num("file").map(|n| format!("{n} file{}", if n == "1" { "" } else { "s" })).unwrap_or_default();
    if let Some(n) = num("insertion") {
        out.push_str(&format!(", +{n}"));
    }
    if let Some(n) = num("deletion") {
        out.push_str(&format!(" −{n}"));
    }
    out
}

/// Put the checkout's files back as they were at `commit`: changed files restored, files
/// made since removed (ignored ones are left alone). What's there now is saved as a
/// checkpoint first, so this can be undone too. Your branch and staged changes stay.
pub fn restore(top: &Path, commit: &str) -> Result<String> {
    if commit.starts_with('-') {
        bail!("not a checkpoint: {commit}");
    }
    take(top, "before going back to an earlier checkpoint")?;
    let now = tree_now(top)?;
    let gone: Vec<String> = git(top, &["diff", "--name-only", "--diff-filter=A", "--no-renames", commit, &now], None)?.lines().map(String::from).collect();
    for f in &gone {
        let _ = std::fs::remove_file(top.join(f));
    }
    git(top, &["checkout", commit, "--", "."], None)?;
    // `checkout <commit> -- .` stages what it writes: put the index back to HEAD's for those.
    let _ = git(top, &["reset", "-q", "--", "."], None);
    let short: String = commit.chars().take(8).collect();
    Ok(format!("rolled back to checkpoint {short}{}", if gone.is_empty() { String::new() } else { format!(" ({} new file{} removed)", gone.len(), if gone.len() == 1 { "" } else { "s" }) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_goes_back_to_how_it_was_after_a_turn() {
        let dir = std::env::temp_dir().join(format!("hydra-cp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args, None).unwrap();
        g(&["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "build/\n").unwrap();
        g(&["add", "."]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]);
        // Turn 1: edits a file and adds one.
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        let first = take(&dir, "turn one").unwrap().expect("a checkpoint");
        assert_eq!(take(&dir, "nothing changed").unwrap(), None, "no change: no checkpoint");
        assert_eq!(g(&["status", "--porcelain"]).lines().count(), 2, "your index and branch are untouched");
        // Turn 2 makes a mess.
        std::fs::write(dir.join("a.txt"), "broken\n").unwrap();
        std::fs::write(dir.join("c.txt"), "junk\n").unwrap();
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::write(dir.join("build").join("out"), "ignored\n").unwrap();
        take(&dir, "turn two").unwrap();
        let cps = list(&dir).unwrap();
        assert_eq!(cps.iter().map(|c| c.what.as_str()).collect::<Vec<_>>(), ["turn two", "turn one"]);
        assert_eq!(cps[1].change, "2 files, +2 −1", "turn one against the commit it started from");
        // Back to after turn one.
        restore(&dir, &first).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap().trim(), "two");
        assert!(dir.join("b.txt").exists() && !dir.join("c.txt").exists(), "made since: removed");
        assert!(dir.join("build").join("out").exists(), "ignored files are left alone");
        assert_eq!(g(&["diff", "--cached", "--name-only"]), "", "nothing staged by it");
        assert_eq!(list(&dir).unwrap()[0].what, "turn two", "what it went back from is still a checkpoint, so going back can be undone");
        // Changes made since the last checkpoint are saved before going back.
        std::fs::write(dir.join("a.txt"), "hand edit\n").unwrap();
        restore(&dir, &first).unwrap();
        assert_eq!(list(&dir).unwrap()[0].what, "before going back to an earlier checkpoint");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
