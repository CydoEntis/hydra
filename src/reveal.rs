//! Clicking a notification takes you to its session. Notifications carry a link
//! (`seshi://session/12?socket=default`); opening it runs `seshi reveal <link>`, which
//! focuses that session and asks the window showing seshi to come to the front.

use anyhow::{Context, Result, bail};
#[cfg(not(windows))]
use std::process::Command;

use crate::protocol::TermId;

const SCHEME: &str = "seshi";

/// This server's name (`SESHI_SOCKET`), which the link carries so the right one is reached.
fn socket_label() -> String {
    std::env::var("SESHI_SOCKET").unwrap_or_else(|_| "default".into())
}

/// The link to a session on this server.
pub fn link(term: TermId) -> String {
    format!("{SCHEME}://session/{term}?socket={}", encode(&socket_label()))
}

/// The session and server a link points at. A bare pane number works too.
fn parse(target: &str) -> Option<(TermId, Option<String>)> {
    let target = target.trim();
    if let Ok(term) = target.parse() {
        return Some((term, None));
    }
    let rest = target.strip_prefix(&format!("{SCHEME}://session/"))?;
    let (term, query) = rest.split_once('?').unwrap_or((rest, ""));
    let term = term.trim_end_matches('/').parse().ok()?;
    let socket = query.split('&').find_map(|kv| kv.strip_prefix("socket=")).map(decode).filter(|s| !s.is_empty());
    Some((term, socket))
}

/// Letters, digits, `-`, `_` and `.` as they are; anything else as `%XX`.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `seshi reveal <link or pane>`: focus the session and bring its window forward.
pub fn run(target: &str) -> Result<()> {
    let Some((term, socket)) = parse(target) else { bail!("not a seshi link or pane number: {target}") };
    if let Some(s) = socket {
        // SAFETY: set once at startup, before any thread is started.
        unsafe { std::env::set_var("SESHI_SOCKET", s) };
    }
    // Clicking the notification gave this process the right to bring a window forward;
    // pass it on to the seshi window, which does it when the server asks.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY);
    }
    crate::cli::reveal(term).context("couldn't reach that session")
}

/// Bring the terminal window this seshi runs in to the front (the server asked: a
/// notification for one of its sessions was clicked). Best effort, off the UI thread.
pub fn raise_window() {
    #[cfg(windows)]
    raise_windows();
    #[cfg(not(windows))]
    std::thread::spawn(raise_unix);
}

#[cfg(windows)]
fn raise_windows() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GA_ROOTOWNER, GetAncestor, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    // SAFETY: plain window calls on handles Windows gives us; a stale one just fails.
    unsafe {
        let console = GetConsoleWindow();
        if console.is_null() {
            return;
        }
        // In Windows Terminal the console window is a hidden stand-in owned by the real one.
        let owner = GetAncestor(console, GA_ROOTOWNER);
        let window = if owner.is_null() { console } else { owner };
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
    }
}

#[cfg(not(windows))]
fn raise_unix() {
    let quiet = |c: &mut Command| crate::proc::quiet(c).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if cfg!(target_os = "macos") {
        // The terminal app seshi runs in, by the name it gives itself.
        let app = match std::env::var("TERM_PROGRAM").unwrap_or_default().as_str() {
            "Apple_Terminal" => "Terminal",
            "iTerm.app" => "iTerm",
            "WezTerm" => "WezTerm",
            "ghostty" => "Ghostty",
            "vscode" => "Visual Studio Code",
            _ => return,
        };
        quiet(Command::new("open").args(["-a", app]));
        return;
    }
    // Hyprland (Omarchy): the window whose program is one of this process's ancestors.
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        && let Ok(out) = crate::proc::run(Command::new("hyprctl").args(["clients", "-j"]))
        && let Ok(clients) = serde_json::from_str::<Vec<serde_json::Value>>(&out)
    {
        let ancestors = ancestors(std::process::id());
        let found = clients.iter().find(|c| c.get("pid").and_then(|p| p.as_u64()).is_some_and(|p| ancestors.contains(&(p as u32))));
        if let Some(addr) = found.and_then(|c| c.get("address")).and_then(|a| a.as_str()) {
            quiet(Command::new("hyprctl").args(["dispatch", "focuswindow", &format!("address:{addr}")]));
        }
        return;
    }
    // X11 terminals say which window they are.
    if let Ok(id) = std::env::var("WINDOWID") {
        quiet(Command::new("xdotool").args(["windowactivate", &id]));
    }
}

/// This process and its parents, up to init (Linux's /proc).
#[cfg(not(windows))]
fn ancestors(mut pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    while pid > 1 && out.len() < 64 {
        out.push(pid);
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { break };
        // "pid (name) state ppid …": the name may hold spaces, so read after the last ')'.
        let Some(ppid) = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)).and_then(|p| p.parse().ok()) else { break };
        pid = ppid;
    }
    out
}

/// Windows: make `seshi://` links open this seshi (per user, no admin), through a console
/// with no window so clicking a notification doesn't flash one.
#[cfg(windows)]
pub fn register() {
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW};
    let Ok(exe) = std::env::current_exe() else { return };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let set = |path: &str, name: Option<&str>, value: &str| {
        let (path, value) = (wide(path), wide(value));
        let name = name.map(wide);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: valid, NUL-terminated wide strings; the key is closed before returning.
        unsafe {
            if RegCreateKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut key, std::ptr::null_mut()) != 0 {
                return;
            }
            RegSetValueExW(key, name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32);
            RegCloseKey(key);
        }
    };
    let base = format!(r"Software\Classes\{SCHEME}");
    set(&base, None, "URL:seshi");
    set(&base, Some("URL Protocol"), "");
    set(&format!(r"{base}\shell\open\command"), None, &format!("conhost.exe --headless \"{}\" reveal \"%1\"", exe.display()));
}

#[cfg(not(windows))]
pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip() {
        let l = format!("{SCHEME}://session/12?socket={}", encode("my demo"));
        assert_eq!(l, "seshi://session/12?socket=my%20demo");
        assert_eq!(parse(&l), Some((12, Some("my demo".into()))));
        assert_eq!(parse("seshi://session/7/"), Some((7, None)));
        assert_eq!(parse("7"), Some((7, None)), "a pane number works too");
        assert_eq!(parse("seshi://nope/7"), None);
        assert_eq!(parse("https://example.com/session/7"), None);
        assert_eq!(decode("100%"), "100%", "a stray % stays");
    }
}
