//! Keep your hydra setup the same on every machine through a private GitHub repo. The
//! config folder itself is the git repo; only config.toml and ideas.json are shared.
//! Anything machine-specific goes in config.local.toml, which is never synced and wins
//! over config.toml.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const IGNORE: &str = "# hydra sync: only these are shared between machines\n*\n!.gitignore\n!config.toml\n!ideas.json\n";

pub fn dir() -> PathBuf {
    crate::config::config_path().parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

/// Sync is on: the config folder is a git repo with a remote.
pub fn enabled() -> bool {
    !cfg!(test) && dir().join(".git").exists() && git(&["remote", "get-url", "origin"]).is_ok()
}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir()).args(args);
    let out = quiet(&mut cmd).output().map_err(|_| format!("`{program}` isn't installed or isn't on PATH"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string())
    }
}

fn git(args: &[&str]) -> Result<String, String> {
    run("git", args)
}

fn host() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "this machine".into())
}

/// Turn sync on: use the private repo `name` if it exists (another machine set it up), or
/// create it from this machine's setup.
pub fn setup(name: &str) -> Result<String> {
    let d = dir();
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join(".gitignore"), IGNORE)?;
    if !d.join(".git").exists() {
        git(&["init", "-q", "-b", "main"]).map_err(anyhow::Error::msg)?;
    }
    let url = run("gh", &["repo", "view", name, "--json", "url", "--jq", ".url"]);
    match url {
        Ok(url) => {
            // Someone (you, elsewhere) already shares a setup: take it, keeping a backup.
            let cfg = d.join("config.toml");
            if cfg.exists() {
                std::fs::copy(&cfg, d.join("config.toml.before-sync")).context("backing up config.toml")?;
            }
            let _ = git(&["remote", "remove", "origin"]);
            git(&["remote", "add", "origin", &format!("{url}.git")]).map_err(anyhow::Error::msg)?;
            git(&["fetch", "-q", "origin"]).map_err(anyhow::Error::msg)?;
            git(&["reset", "-q", "--hard", "origin/main"]).map_err(anyhow::Error::msg)?;
            let _ = git(&["branch", "-q", "--set-upstream-to=origin/main", "main"]);
            Ok(format!("now using your shared setup from {url} (your old config is in config.toml.before-sync)"))
        }
        Err(_) => {
            if !d.join("config.toml").exists() {
                std::fs::write(d.join("config.toml"), "# hydra config (shared between your machines)\n")?;
            }
            git(&["add", "-A"]).map_err(anyhow::Error::msg)?;
            let _ = git(&["commit", "-q", "-m", &format!("hydra setup from {}", host())]);
            run("gh", &["repo", "create", name, "--private", "--source", ".", "--remote", "origin", "--push"]).map_err(anyhow::Error::msg)?;
            Ok(format!("created the private repo {name}; run `hydra sync setup {name}` on your other machines"))
        }
    }
}

/// Bring in changes from your other machines. Returns true if anything changed.
pub fn pull() -> Result<bool, String> {
    let before = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    git(&["pull", "-q", "--rebase", "--autostash", "origin", "main"])?;
    Ok(git(&["rev-parse", "HEAD"]).unwrap_or_default() != before)
}

/// Share this machine's changes.
pub fn push() -> Result<(), String> {
    git(&["add", "-A"])?;
    if git(&["diff", "--cached", "--quiet"]).is_err() {
        git(&["commit", "-q", "-m", &format!("hydra: from {}", host())])?;
    }
    if git(&["push", "-q", "origin", "main"]).is_err() {
        // Another machine pushed first: take theirs, then ours on top.
        git(&["pull", "-q", "--rebase", "--autostash", "origin", "main"])?;
        git(&["push", "-q", "origin", "main"])?;
    }
    Ok(())
}

static PUSHING: AtomicBool = AtomicBool::new(false);

/// Push a moment from now (changes made close together go up together).
pub fn push_soon() {
    if !enabled() || PUSHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        PUSHING.store(false, Ordering::SeqCst);
        let _ = push();
    });
}

/// `hydra sync [setup <name> | off]`
pub fn command(action: Option<&str>, name: Option<&str>) -> Result<()> {
    match action {
        Some("setup") => {
            println!("{}", setup(name.unwrap_or("hydra-config"))?);
        }
        Some("off") => {
            let g = dir().join(".git");
            if g.exists() {
                std::fs::remove_dir_all(&g).context("removing the sync folder")?;
            }
            println!("sync is off on this machine (the GitHub repo is untouched)");
        }
        Some(other) => bail!("unknown `{other}`: use `hydra sync`, `hydra sync setup [repo]` or `hydra sync off`"),
        None => {
            if !enabled() {
                println!("sync is off. `hydra sync setup` shares your config and ideas through a private GitHub repo.");
                return Ok(());
            }
            let changed = pull().map_err(anyhow::Error::msg)?;
            push().map_err(anyhow::Error::msg)?;
            let url = git(&["remote", "get-url", "origin"]).unwrap_or_default();
            println!("in sync with {url}{}", if changed { " (pulled changes from another machine)" } else { "" });
        }
    }
    Ok(())
}
