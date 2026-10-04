//! Worktrees: making and trusting them, spares kept warm, hooks, cleanup.

use super::*;

/// The repository a folder belongs to: its main checkout (worktrees resolve to the repo
/// they were made from). `None` outside git.
pub(super) fn repo_of(dir: &std::path::Path) -> Option<PathBuf> {
    let top = dir.ancestors().find(|a| a.join(".git").exists())?;
    let dot = top.join(".git");
    if dot.is_dir() {
        return Some(top.to_path_buf());
    }
    // A linked worktree: ".git" is a file "gitdir: <main>/.git/worktrees/<name>".
    let text = std::fs::read_to_string(&dot).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let main_git = gitdir.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git"))?;
    main_git.parent().map(|p| p.to_path_buf())
}

/// Where the spare worktree is noted, so one left behind by a crash is cleaned up.
pub(super) fn spare_file() -> PathBuf {
    let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
    crate::config::data_dir().join(format!("spare-{label}.txt"))
}

pub(super) fn save_spare(path: Option<&std::path::Path>) {
    if cfg!(test) {
        return;
    }
    match path {
        Some(p) => {
            let _ = std::fs::write(spare_file(), p.to_string_lossy().as_bytes());
        }
        None => {
            let _ = std::fs::remove_file(spare_file());
        }
    }
}

/// Remove a spare worktree and its branch (it was never used, so nothing is lost).
pub(super) fn drop_spare_dir(path: &std::path::Path) {
    let branch = crate::gitfs::head(path).map(|h| h.branch);
    let repo = crate::gitfs::head(path).map(|h| h.main_root);
    let _ = git::remove_worktree(path, true, false);
    if let (Some(repo), Some(b)) = (repo, branch.filter(|b| b.starts_with("spare-"))) {
        let _ = std::process::Command::new("git").arg("-C").arg(repo).args(["branch", "-D", &b]).output();
    }
}

/// Claude asks "do you trust this folder?" for every new folder, and every new worktree is
/// one. A worktree of a repo you already trust is trusted the same way (never otherwise).
pub(super) fn trust_like_repo(repo: &std::path::Path, worktree: &std::path::Path) {
    if let Some(home) = directories::BaseDirs::new() {
        trust_in(&home.home_dir().join(".claude.json"), repo, worktree);
    }
}

/// Make a worktree and trust it like its repo, both on the calling (background) thread:
/// Claude's file can be megabytes, and the trust must be in place before an agent starts
/// there.
pub(super) fn create_trusted_worktree(dir: &std::path::Path, branch: &str, base: Option<&str>, template: &str) -> Result<(PathBuf, String), String> {
    let made = git::create_worktree(dir, branch, base, template).map_err(|e| format!("{e:#}"))?;
    if let Some(h) = crate::gitfs::head(&made.0) {
        trust_like_repo(&h.main_root, &made.0);
    }
    Ok(made)
}

pub(super) fn trust_in(file: &std::path::Path, repo: &std::path::Path, worktree: &std::path::Path) {
    // Claude rewrites this file too: if it changed while we worked, start again from its
    // version rather than replace it.
    for _ in 0..3 {
        match trust_once(file, repo, worktree) {
            Trust::Raced => continue,
            Trust::Done => return,
        }
    }
    tracing::warn!("{} kept changing; didn't mark {} trusted", file.display(), worktree.display());
}

pub(super) enum Trust {
    Done,
    /// The file changed between reading it and swapping ours in.
    Raced,
}

pub(super) fn trust_once(file: &std::path::Path, repo: &std::path::Path, worktree: &std::path::Path) -> Trust {
    let file = file.to_path_buf();
    let Ok(text) = std::fs::read_to_string(&file) else { return Trust::Done };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { return Trust::Done };
    let key = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_string();
    let projects = v.get("projects");
    let trusted = |k: &str| projects.and_then(|p| p.get(k)).and_then(|e| e.get("hasTrustDialogAccepted")).and_then(|b| b.as_bool()) == Some(true);
    let (rk, wk) = (key(repo), key(worktree));
    // Claude's keys may differ in the drive letter's case.
    let repo_ok = trusted(&rk) || projects.and_then(|p| p.as_object()).is_some_and(|m| m.iter().any(|(k, e)| k.eq_ignore_ascii_case(&rk) && e.get("hasTrustDialogAccepted").and_then(|b| b.as_bool()) == Some(true)));
    if !repo_ok || trusted(&wk) {
        return Trust::Done;
    }
    let Some(map) = v.get_mut("projects").and_then(|p| p.as_object_mut()) else { return Trust::Done };
    let entry = map.entry(wk).or_insert_with(|| serde_json::json!({}));
    if let Some(o) = entry.as_object_mut() {
        o.insert("hasTrustDialogAccepted".into(), serde_json::Value::Bool(true));
    }
    // Write beside and swap, so a reader never sees half a file; check it's still the
    // version we read right before swapping.
    let tmp = file.with_extension("json.hydra-tmp");
    let Ok(s) = serde_json::to_string_pretty(&v) else { return Trust::Done };
    if std::fs::write(&tmp, s).is_err() {
        return Trust::Done;
    }
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        let _ = std::fs::remove_file(&tmp);
        return Trust::Raced;
    }
    if let Err(e) = std::fs::rename(&tmp, &file) {
        tracing::warn!("couldn't update {}: {e}", file.display());
        let _ = std::fs::remove_file(&tmp);
    }
    Trust::Done
}

/// Claude files conversations under `~/.claude/projects/<folder slug>/<session>.jsonl`, the
/// slug being the folder with every non-alphanumeric character as `-`. Put a copy where a
/// resume in `dest` will look.
pub(super) fn copy_claude_transcript(src: &std::path::Path, session: &str, dest: &std::path::Path) {
    let Some(projects) = src.parent().and_then(|p| p.parent()) else { return };
    let slug: String = dest.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let dir = projects.join(slug);
    let to = dir.join(format!("{session}.jsonl"));
    if !to.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::copy(src, &to);
    }
}

impl Daemon {
    /// Heuristic statuses for agents without hooks, and "seen" bookkeeping for everyone.
    /// Whether the spare can be what `cmd` asks for in `repo`: the same agent, with at most a
    /// task after it (returned, unquoted; empty for none).
    pub(super) fn spare_fits(&self, repo: &std::path::Path, cmd: &str) -> Option<String> {
        let warm = self.cfg.worktree.prewarm.trim();
        let (spare_repo, _, term) = self.spare.as_ref()?;
        if warm.is_empty() || !same_path(spare_repo, repo) || !self.terms.contains_key(term) {
            return None;
        }
        let cmd = cmd.trim();
        if cmd == warm {
            return Some(String::new());
        }
        let rest = cmd.strip_prefix(warm)?.strip_prefix(' ')?;
        // Only a task after the agent: undo the shell quoting and check it quotes back.
        let inner = rest.strip_prefix('\'')?.strip_suffix('\'')?;
        let task = inner.replace("''", "'").replace("'\\''", "'");
        (self.cfg.quote_for_shell(&task) == rest).then_some(task)
    }

    /// Get a spare ready in `repo`, if prewarming is on and there isn't one there already.
    pub(super) fn prewarm(&mut self, repo: PathBuf) {
        if self.cfg.worktree.prewarm.trim().is_empty() || self.spare_making {
            return;
        }
        if let Some((r, path, term)) = self.spare.take() {
            if same_path(&r, &repo) && self.terms.contains_key(&term) {
                self.spare = Some((r, path, term));
                return;
            }
            // Warm for the repo you're using now instead.
            self.close_term(term);
            let p = path.clone();
            tokio::task::spawn_blocking(move || drop_spare_dir(&p));
        }
        self.spare_making = true;
        let template = self.cfg.worktree.dir.clone();
        let tx = self.tx.clone();
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let name = format!("spare-{:04x}", n & 0xffff);
        tokio::task::spawn_blocking(move || {
            let result = create_trusted_worktree(&repo, &name, None, &template);
            let _ = tx.blocking_send(Ev::SpareMade(repo, result));
        });
    }

    /// A checkout's dev-server port from the worktree lists already in the snapshot (the
    /// same order `git worktree list` gives), without running git.
    pub(super) fn known_port(&self, dir: &std::path::Path, base: u16) -> Option<u16> {
        self.workspaces.iter().filter_map(|w| w.git.as_ref()).find_map(|g| {
            let i = g.worktrees.iter().position(|wt| same_path(&wt.path, dir))?;
            Some(base.saturating_add(i as u16))
        })
    }

    /// A worktree hook (`on_create` / `on_remove` in the repo's .hydra.toml) as a job to run.
    pub(super) fn hook_job(&self, dir: &std::path::Path, create: bool) -> Option<impl FnOnce() -> String + Send + 'static> {
        let proj = crate::project::load(dir);
        let cmd = if create { proj.hooks.on_create } else { proj.hooks.on_remove };
        if cmd.trim().is_empty() {
            return None;
        }
        let shell = self.cfg.shell_command();
        let dir = dir.to_path_buf();
        let base = proj.dev.and_then(|d| d.port);
        let allowed = crate::project::allowed(&dir, &cmd);
        Some(move || {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !allowed {
                return format!("{name}: this repo's {} wants to run `{cmd}`; run `hydra allow` in the repo to let it", crate::project::FILE);
            }
            let mut vars = vec![("HYDRA_WORKTREE", dir.display().to_string())];
            if let Some(h) = crate::gitfs::head(&dir) {
                vars.push(("HYDRA_BRANCH", h.branch.clone()));
                vars.push(("HYDRA_REPO", h.main_root.display().to_string()));
            }
            if let Some(b) = base {
                vars.push(("PORT", crate::project::port_for(&dir, b).to_string()));
            }
            let what = if create { "on_create" } else { "on_remove" };
            match crate::project::run_hook(&shell, &dir, &cmd, &vars) {
                Ok(()) => format!("{name}: {what} hook done ({cmd})"),
                Err(e) => format!("{name}: {what} hook failed: {e}"),
            }
        })
    }

    /// Run a worktree hook in the background and say how it went.
    pub(super) fn worktree_hook(&self, dir: &std::path::Path, create: bool) {
        self.ext_event(if create { "worktree_create" } else { "worktree_remove" }, Some(dir), None);
        if let Some(job) = self.hook_job(dir, create) {
            let tx = self.tx.clone();
            tokio::task::spawn_blocking(move || {
                let _ = tx.blocking_send(Ev::HookRan(job()));
            });
        }
    }

    /// The linked worktrees these terminals are in.
    pub(super) fn worktrees_of(&self, terms: &[TermId]) -> Vec<PathBuf> {
        terms
            .iter()
            .filter_map(|t| self.terms.get(t))
            .filter_map(|t| t.head.as_ref().filter(|h| h.linked).map(|h| h.top.clone()))
            .collect()
    }

    /// After you close something: remove worktrees hydra made that nothing runs in any
    /// more. Git refuses if there are uncommitted changes, and the branch is always kept.
    pub(super) fn cleanup_worktrees(&mut self, client: ClientId, tops: Vec<PathBuf>) {
        if !self.cfg.worktree.delete_with_last {
            return;
        }
        for top in tops {
            if !self.made_worktrees.iter().any(|m| same_path(m, &top)) {
                continue;
            }
            if self.terms.values().any(|t| t.head.as_ref().is_some_and(|h| same_path(&h.top, &top))) {
                continue;
            }
            self.made_worktrees.retain(|m| !same_path(m, &top));
            let tx = self.tx.clone();
            self.pending_ops += 1;
            tokio::task::spawn_blocking(move || {
                // Give the closed programs a moment to let go of the folder (Windows locks it).
                std::thread::sleep(Duration::from_millis(800));
                let result = git::remove_worktree(&top, false, false).map_err(|e| format!("{e:#}"));
                let _ = tx.blocking_send(Ev::WorktreeAutoRemoved { client, path: top, result });
            });
        }
    }
}
