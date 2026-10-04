//! Cheap git facts read straight from the filesystem (no `git` process): which checkout a
//! folder is in, its branch, and whether it's a linked worktree. Fast enough to call for
//! every pane whenever its folder changes.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    /// The checkout's top folder (where `.git` is).
    pub top: PathBuf,
    /// The current branch, or a short commit when detached.
    pub branch: String,
    /// A linked worktree (`git worktree add`), not the main checkout.
    pub linked: bool,
    /// The main checkout of the repository.
    pub main_root: PathBuf,
}

fn read_head(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref:") {
        Some(r) => r.trim().trim_start_matches("refs/heads/").to_string(),
        None => head.chars().take(7).collect(),
    })
}

/// The checkout `dir` is inside, if any.
pub fn head(dir: &Path) -> Option<Head> {
    let top = dir.ancestors().find(|a| a.join(".git").exists())?.to_path_buf();
    let dot = top.join(".git");
    if dot.is_dir() {
        return Some(Head { branch: read_head(&dot)?, linked: false, main_root: top.clone(), top });
    }
    // A linked worktree: ".git" is a file "gitdir: <main>/.git/worktrees/<name>".
    let text = std::fs::read_to_string(&dot).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() { gitdir } else { top.join(gitdir) };
    // Only <repo>/.git/worktrees/<name> is a linked worktree; a submodule's git dir
    // (<super>/.git/modules/<name>) makes it a repo of its own.
    if gitdir.parent().and_then(|p| p.file_name()).is_none_or(|n| n != "worktrees") {
        return Some(Head { branch: read_head(&gitdir)?, linked: false, main_root: top.clone(), top });
    }
    let main_git = gitdir.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git"))?;
    let main_root = main_git.parent()?.to_path_buf();
    Some(Head { branch: read_head(&gitdir)?, linked: true, main_root, top })
}

/// The branch the main checkout is on (the base for a worktree's changes).
pub fn main_branch(main_root: &Path) -> Option<String> {
    read_head(&main_root.join(".git"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_submodule_is_its_own_repo() {
        let root = std::env::temp_dir().join(format!("hydra-submod-{}", std::process::id()));
        let modgit = root.join("super").join(".git").join("modules").join("lib");
        let sub = root.join("super").join("lib");
        std::fs::create_dir_all(&modgit).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(modgit.join("HEAD"), "ref: refs/heads/main
").unwrap();
        std::fs::write(sub.join(".git"), "gitdir: ../.git/modules/lib
").unwrap();
        let h = super::head(&sub).unwrap();
        assert!(!h.linked, "not a linked worktree");
        assert_eq!(h.main_root, sub);
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;

    #[test]
    fn reads_main_and_linked_checkouts() {
        let base = std::env::temp_dir().join(format!("hydra-gitfs-{}", std::process::id()));
        let main = base.join("repo");
        let wt = base.join("repo-feat");
        std::fs::create_dir_all(main.join(".git").join("worktrees").join("feat")).unwrap();
        std::fs::create_dir_all(main.join("src")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(main.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(main.join(".git").join("worktrees").join("feat").join("HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", main.join(".git").join("worktrees").join("feat").display())).unwrap();

        let h = head(&main.join("src")).unwrap();
        assert_eq!((h.branch.as_str(), h.linked), ("main", false));
        assert_eq!(h.top, main);
        let h = head(&wt).unwrap();
        assert_eq!((h.branch.as_str(), h.linked), ("feat/x", true));
        assert_eq!(h.main_root, main);
        assert_eq!(main_branch(&h.main_root).as_deref(), Some("main"));
        assert!(head(&std::env::temp_dir().join("hydra-nowhere-x")).is_none() || true);
        let _ = std::fs::remove_dir_all(base);
    }
}
