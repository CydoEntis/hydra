//! Updating hydra from its GitHub releases: `hydra update`, and a quiet daily check that
//! says when a newer version is out. Downloads go through the GitHub CLI while the repo is
//! private (your sign-in), and straight from GitHub once it's public.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "CydoEntis/hydra";
/// How often the background check asks GitHub.
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

/// The release build that runs on this machine.
pub fn target() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) || cfg!(all(windows, target_arch = "aarch64")) {
        // Windows on ARM runs the x64 build.
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-musl")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("aarch64-unknown-linux-musl")
    } else {
        None
    }
}

fn asset_for(target: &str) -> String {
    format!("hydra-{target}.{}", if cfg!(windows) { "zip" } else { "tar.gz" })
}

/// `1.2.3` (a leading `v` allowed) as numbers, for comparing.
fn version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim().trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

/// Whether the release `tag` is newer than this build.
pub fn is_newer(tag: &str) -> bool {
    matches!((version(tag), version(env!("CARGO_PKG_VERSION"))), (Some(a), Some(b)) if a > b)
}

fn gh_signed_in() -> bool {
    crate::proc::run(Command::new("gh").args(["auth", "status"])).is_ok()
}

/// A system tool by its usual Windows home (System32 has curl and a tar that reads zips),
/// so a Git Bash or MSYS one earlier on PATH isn't picked instead.
fn system_tool(name: &str) -> Command {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let p = Path::new(&root).join("System32").join(format!("{name}.exe"));
        if p.is_file() {
            return Command::new(p);
        }
    }
    Command::new(name)
}

/// The newest release's tag, e.g. `v0.3.0`.
pub fn latest_tag() -> Result<String, String> {
    if gh_signed_in() {
        return crate::proc::run(Command::new("gh").args(["release", "view", "-R", REPO, "--json", "tagName", "-q", ".tagName"]))
            .map(|s| s.trim().to_string());
    }
    // The releases/latest page redirects to the newest tag's page. Unlike GitHub's API it
    // has no per-address limit, which shared networks (offices, CI) run out of.
    let url = format!("https://github.com/{REPO}/releases/latest");
    let landed = crate::proc::run(system_tool("curl").args(["-fsSL", "--max-time", "20", "-o", NULL_DEVICE, "-w", "%{url_effective}", &url]))
        .map_err(|e| format!("couldn't reach GitHub: {e}"))?;
    tag_from_url(&landed).ok_or_else(|| "no release found".into())
}

const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// `v1.2.3` from `https://github.com/owner/repo/releases/tag/v1.2.3`.
fn tag_from_url(url: &str) -> Option<String> {
    let tag = url.trim().rsplit_once("/releases/tag/")?.1.trim_end_matches('/');
    version(tag).map(|_| tag.to_string())
}

fn download(tag: &str, name: &str, dir: &Path) -> Result<PathBuf> {
    let to = dir.join(name);
    let got = if gh_signed_in() {
        crate::proc::run(Command::new("gh").args(["release", "download", tag, "-R", REPO, "-p", name, "--clobber", "-D"]).arg(dir))
    } else {
        let url = format!("https://github.com/{REPO}/releases/download/{tag}/{name}");
        crate::proc::run(system_tool("curl").args(["-fsSL", "--max-time", "300", "-o"]).arg(&to).arg(&url))
    };
    got.map_err(|e| anyhow::anyhow!("downloading {name}: {e}"))?;
    Ok(to)
}

fn sha256_hex(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// `hydra update`: install the newest release over this one. `check` only says whether
/// there is one; `force` reinstalls it even when it's this version.
pub fn run(check: bool, force: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let tag = latest_tag().map_err(|e| anyhow::anyhow!(e))?;
    remember(&tag);
    if !is_newer(&tag) && !force {
        println!("hydra {current} is the latest version.");
        return Ok(());
    }
    if check {
        println!("hydra {} is out (you have {current}). Run: hydra update", tag.trim_start_matches('v'));
        return Ok(());
    }
    let Some(target) = target() else { bail!("there's no release build for this machine; build from source") };
    let asset = asset_for(target);
    let tmp = std::env::temp_dir().join(format!("hydra-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let result = install(&tag, target, &asset, &tmp, true);
    let _ = std::fs::remove_dir_all(&tmp);
    let exe = result?;
    println!("Updated hydra {current} -> {} ({}).", tag.trim_start_matches('v'), exe.display());
    println!("Your running sessions still use the old version until the server restarts:");
    println!("  hydra kill-server, then hydra (agents stop, and resume when it starts)");
    Ok(())
}

/// Install the newest release over this one without printing (from inside the app, whose
/// screen printing would garble). Returns the version installed and where it went.
pub fn install_latest() -> Result<(String, PathBuf)> {
    let tag = latest_tag().map_err(|e| anyhow::anyhow!(e))?;
    remember(&tag);
    let Some(target) = target() else { bail!("there's no release build for this machine; build from source") };
    let asset = asset_for(target);
    let tmp = std::env::temp_dir().join(format!("hydra-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let result = install(&tag, target, &asset, &tmp, false);
    let _ = std::fs::remove_dir_all(&tmp);
    Ok((tag.trim_start_matches('v').to_string(), result?))
}

fn install(tag: &str, target: &str, asset: &str, tmp: &Path, say: bool) -> Result<PathBuf> {
    if say {
        println!("Downloading hydra {} for {target}...", tag.trim_start_matches('v'));
    }
    let archive = download(tag, asset, tmp)?;
    let sums = std::fs::read_to_string(download(tag, "sha256sums.txt", tmp)?)?;
    let expected = sums
        .lines()
        .find_map(|l| l.split_once("  ").filter(|(_, n)| n.trim() == asset).map(|(h, _)| h.trim().to_lowercase()))
        .with_context(|| format!("no checksum for {asset}"))?;
    if sha256_hex(&archive)? != expected {
        bail!("checksum mismatch for {asset}; nothing was changed");
    }
    crate::proc::run(system_tool("tar").arg("-xf").arg(&archive).arg("-C").arg(tmp)).map_err(|e| anyhow::anyhow!("unpacking: {e}"))?;
    let name = if cfg!(windows) { "hydra.exe" } else { "hydra" };
    let new = tmp.join(format!("hydra-{target}")).join(name);
    if !new.is_file() {
        bail!("the download had no {name}");
    }
    let exe = std::env::current_exe().context("finding this hydra")?;
    replace_exe(&new, &exe)?;
    Ok(exe)
}

/// Put `new` where `exe` is. Windows won't overwrite a running program but lets it be
/// renamed, so the old one steps aside (and is cleaned up on a later start).
fn replace_exe(new: &Path, exe: &Path) -> Result<()> {
    if cfg!(windows) {
        tidy_beside(exe);
        let old = free_old_path(exe);
        std::fs::rename(exe, &old).with_context(|| format!("moving the old {} aside", exe.display()))?;
        if let Err(e) = std::fs::copy(new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(e).context("putting the new hydra in place");
        }
    } else {
        let staged = exe.with_file_name(".hydra.new");
        std::fs::copy(new, &staged).context("staging the new hydra")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&staged, exe).context("putting the new hydra in place")?;
    }
    Ok(())
}

/// A name for the old program that's free. An older one moved aside can still be running
/// (an agent's `hydra mcp` keeps going until its session ends), so it can't always be
/// removed; the next one gets a number.
fn free_old_path(exe: &Path) -> PathBuf {
    std::iter::once(exe.with_file_name("hydra.old.exe"))
        .chain((2..).map(|n| exe.with_file_name(format!("hydra.old-{n}.exe"))))
        .find(|p| !p.exists())
        .unwrap_or_else(|| exe.with_file_name("hydra.old.exe"))
}

/// Remove the old programs updates moved aside beside `exe`, the ones nothing runs any more.
fn tidy_beside(exe: &Path) {
    let Some(dir) = exe.parent() else { return };
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("hydra.old") && name.ends_with(".exe") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Remove the old programs an update moved aside (Windows), once nothing runs them.
pub fn tidy() {
    if cfg!(windows)
        && let Ok(exe) = std::env::current_exe()
    {
        tidy_beside(&exe);
    }
}

// ---- the daily check ----------------------------------------------------------------------

fn state_path() -> PathBuf {
    crate::config::data_dir().join("update-check.json")
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Checked {
    at: u64,
    latest: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn remember(tag: &str) {
    let c = Checked { at: now(), latest: tag.to_string() };
    if let Ok(s) = serde_json::to_string(&c) {
        let _ = crate::config::write_atomic(&state_path(), s);
    }
}

/// A newer release's version, asking GitHub at most once a day (otherwise the answer from
/// the last time). Slow (network): call it off the UI thread.
pub fn newer_release() -> Option<String> {
    let last: Checked = crate::config::read_state(&state_path());
    let tag = if now().saturating_sub(last.at) < CHECK_EVERY_SECS && !last.latest.is_empty() {
        last.latest
    } else {
        let t = latest_tag().ok()?;
        remember(&t);
        t
    };
    is_newer(&tag).then(|| tag.trim_start_matches('v').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(version("1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(version("nonsense"), None);
        assert!(is_newer("v999.0.0"));
        assert!(!is_newer(&format!("v{}", env!("CARGO_PKG_VERSION"))), "the same version isn't newer");
        assert!(!is_newer("v0.0.1"));
    }

    #[test]
    fn the_latest_tag_comes_from_where_github_redirects() {
        assert_eq!(tag_from_url("https://github.com/CydoEntis/hydra/releases/tag/v0.3.1\n").as_deref(), Some("v0.3.1"));
        assert_eq!(tag_from_url("https://github.com/CydoEntis/hydra/releases"), None, "no release yet");
        assert_eq!(tag_from_url("https://github.com/CydoEntis/hydra/releases/tag/nightly"), None);
    }

    #[test]
    fn this_machine_has_a_build() {
        let t = target().expect("a release target for the platforms we build");
        assert!(asset_for(t).starts_with("hydra-"));
    }

    #[test]
    fn swapping_the_program_keeps_a_way_back() {
        let dir = std::env::temp_dir().join(format!("hydra-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(if cfg!(windows) { "hydra.exe" } else { "hydra" });
        let new = dir.join("new-build");
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace_exe(&new, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        if cfg!(windows) {
            assert_eq!(std::fs::read_to_string(exe.with_file_name("hydra.old.exe")).unwrap(), "old", "the old one is moved aside");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An older program moved aside is still running (an agent's `hydra mcp`): the update
    /// steps past it instead of failing.
    #[cfg(windows)]
    #[test]
    fn an_old_program_still_running_doesnt_stop_an_update() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("hydra-swap-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("hydra.exe");
        let new = dir.join("new-build");
        std::fs::write(&exe, "current").unwrap();
        std::fs::write(&new, "new").unwrap();
        std::fs::write(dir.join("hydra.old.exe"), "running").unwrap();
        // Open with no delete sharing, the way a running program holds its file.
        const FILE_SHARE_READ: u32 = 1;
        let held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(dir.join("hydra.old.exe")).unwrap();
        replace_exe(&new, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("hydra.old-2.exe")).unwrap(), "current", "the next free name");
        drop(held);
        tidy_beside(&exe);
        assert!(!dir.join("hydra.old.exe").exists() && !dir.join("hydra.old-2.exe").exists(), "both tidied once nothing runs them");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
