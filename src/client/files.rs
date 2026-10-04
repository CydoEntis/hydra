//! The file finder: recently touched files (Downloads, Desktop, Documents, the project) and
//! the project's files, so a path can go straight into an agent's prompt without a trip to
//! the file manager.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
    /// Where it was found: "Downloads", "Desktop", "project", ...
    pub place: String,
}


/// Subsequence match, scoring consecutive runs and matches at word starts. `None` = no match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut prev_hit = false;
    for qc in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let mut found = false;
        while ti < t.len() {
            let c = t[ti];
            ti += 1;
            if c == qc {
                let start = ti == 1 || matches!(t[ti - 2], '/' | '\\' | '_' | '-' | '.' | ' ');
                score += 1 + if prev_hit { 4 } else { 0 } + if start { 3 } else { 0 };
                prev_hit = true;
                found = true;
                break;
            }
            prev_hit = false;
        }
        if !found {
            return None;
        }
    }
    // Prefer shorter paths when scores tie.
    Some(score * 100 - t.len().min(99) as i32)
}

fn entry(path: PathBuf, place: &str) -> Option<FileEntry> {
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(FileEntry { modified: meta.modified().ok()?, size: meta.len(), path, place: place.to_string() })
}

/// Files touched in the last few days in the usual drop zones and the project, newest first.
pub fn scan_recent(root: &Path) -> Vec<FileEntry> {
    let now = SystemTime::now();
    let fresh = |e: &FileEntry, days: u64| now.duration_since(e.modified).unwrap_or_default() < Duration::from_secs(days * 86_400);
    let mut out = Vec::new();
    if let Some(u) = directories::UserDirs::new() {
        let places = [("Downloads", u.download_dir()), ("Desktop", u.desktop_dir()), ("Documents", u.document_dir())];
        for (name, dir) in places {
            let Some(dir) = dir else { continue };
            for e in ignore::WalkBuilder::new(dir).max_depth(Some(2)).standard_filters(false).hidden(true).build().flatten().take(5000) {
                if let Some(fe) = entry(e.into_path(), name).filter(|f| fresh(f, 7)) {
                    out.push(fe);
                }
            }
        }
    }
    for e in ignore::WalkBuilder::new(root).max_depth(Some(12)).build().flatten().take(20_000) {
        if let Some(fe) = entry(e.into_path(), "project").filter(|f| fresh(f, 2)) {
            out.push(fe);
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(300);
    out
}

/// The first lines of a text file, or a one-line description of anything else.
pub fn preview(path: &Path) -> Vec<String> {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return vec!["(can't open)".into()] };
    let mut buf = vec![0u8; 2 * 1024 * 1024];
    let n = f.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    if buf.contains(&0) {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let kind = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "binary".into());
        return vec![format!("{kind} file, {}", human_size(size)), String::new(), "Enter puts its path in your prompt.".into()];
    }
    String::from_utf8_lossy(&buf).lines().take(50_000).map(|l| l.replace('\t', "    ")).collect()
}

pub fn human_size(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

pub fn ago(t: SystemTime) -> String {
    let s = SystemTime::now().duration_since(t).unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// A path ready to type into a prompt: quoted when it has spaces.
pub fn quote_path(p: &Path) -> String {
    let s = p.display().to_string();
    if s.contains(' ') { format!("\"{s}\"") } else { s }
}

/// Whether a file looks like text: no NUL in its first 8 KB (only those are read).
pub fn looks_like_text(path: &Path) -> bool {
    use std::io::Read;
    let mut head = Vec::with_capacity(8192);
    match std::fs::File::open(path).and_then(|f| f.take(8192).read_to_end(&mut head)) {
        Ok(_) => !head.contains(&0),
        Err(_) => false,
    }
}

/// Open a link in the browser (https only).
pub fn open_url(url: &str) {
    if url.starts_with("https://") {
        let _ = open_default(Path::new(url));
    }
}

/// Open a file (or folder) in its default app.
pub fn open_default(path: &Path) -> std::io::Result<()> {
    // A leading '-' would be read as an option by open / xdg-open.
    if path.as_os_str().to_string_lossy().starts_with('-') {
        return Err(std::io::Error::other("won't open a name starting with '-'"));
    }
    #[cfg(windows)]
    {
        // ShellExecute, not `cmd /C start`: cmd would treat & | ^ in a link or file
        // name as commands of its own.
        shell_open(path.as_os_str())
    }
    #[cfg(not(windows))]
    {
        open_with_tool(path)
    }
}

#[cfg(windows)]
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn shell_open(target: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::ffi::c_void;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> *mut c_void;
    }
    const SW_SHOWNORMAL: i32 = 1;
    let (op, file) = (wide("open".as_ref()), wide(target));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; null
    // window, parameters and directory are allowed.
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) } as isize;
    // ShellExecute reports success as a value above 32.
    if r > 32 { Ok(()) } else { Err(std::io::Error::other(format!("Windows couldn't open it (code {r})"))) }
}

/// Start a program without waiting for it or keeping its output.
#[cfg(not(windows))]
fn spawn_detached(mut cmd: std::process::Command) -> std::io::Result<()> {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map(|_| ())
}

#[cfg(not(windows))]
fn open_with_tool(path: &Path) -> std::io::Result<()> {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("open");
        cmd.arg(path);
    } else {
        cmd = std::process::Command::new("xdg-open");
        cmd.arg(path);
    }
    spawn_detached(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_tight_matches() {
        assert!(fuzzy("rdme", "README.md").is_some());
        assert!(fuzzy("xyz", "README.md").is_none());
        let tight = fuzzy("main", "src/main.rs").unwrap();
        let loose = fuzzy("main", "src/my_animation.rs").unwrap();
        assert!(tight > loose);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_path(Path::new("a/b.txt")), "a/b.txt");
        assert_eq!(quote_path(Path::new("my docs/b.txt")), "\"my docs/b.txt\"");
    }
}

#[cfg(all(test, windows))]
mod open_tests {
    #[test]
    fn links_reach_windows_untouched() {
        // The whole link, & and all, is one string to ShellExecute: nothing parses it.
        let url = "https://x.io/a&calc|b^c";
        let w = super::wide(url.as_ref());
        assert_eq!(String::from_utf16(&w[..w.len() - 1]).unwrap(), url);
        assert!(super::open_default(std::path::Path::new("-x")).is_err());
    }
}

#[cfg(test)]
mod sniff_tests {
    #[test]
    fn only_the_start_of_a_file_is_read() {
        let dir = std::env::temp_dir().join(format!("hydra-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (text, bin) = (dir.join("a.txt"), dir.join("b.bin"));
        // A NUL past the first 8 KB isn't looked at.
        let mut late = vec![b'a'; 10_000];
        late.push(0);
        std::fs::write(&text, &late).unwrap();
        std::fs::write(&bin, [b'x', 0, b'y']).unwrap();
        assert!(super::looks_like_text(&text));
        assert!(!super::looks_like_text(&bin));
        assert!(!super::looks_like_text(&dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
