//! Desktop notifications and sounds, using each system's own tools so nothing extra is
//! needed: a Windows toast through PowerShell, `osascript` on macOS, `notify-send` on Linux;
//! sounds through PowerShell's media player, `afplay`, or `paplay` / `pw-play`.
//!
//! Everything runs on a short-lived thread and failures are ignored: an alert that can't be
//! shown must never get in the way. A notification can carry a `hydra://` link (see
//! `reveal`): clicking it takes you to the session.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What happened, for picking the sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Needs,
    Done,
}

/// The built-in sound names (anything else is a path to a sound file).
pub const SOUNDS: &[&str] = &["glass", "ping", "chime", "pop", "off"];

/// Show a desktop notification and/or play the sound for `kind`, as configured. `link`:
/// where clicking the notification goes.
pub fn alert(cfg: &crate::config::Notify, kind: Kind, title: &str, body: &str, link: Option<String>) {
    let sound = match kind {
        Kind::Needs => cfg.sound_needs.clone(),
        Kind::Done => cfg.sound_done.clone(),
    };
    if cfg.desktop {
        let (title, body) = (title.to_string(), body.to_string());
        // Its own thread: on Linux it waits for the click.
        std::thread::spawn(move || notify(&title, &body, link.as_deref()));
    }
    std::thread::spawn(move || play(&sound));
}

fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::proc::quiet(cmd)
}

/// A desktop notification. Blocks until the tool returns (on Linux, with a link, until the
/// notification is clicked or goes away).
pub fn notify(title: &str, body: &str, link: Option<&str>) {
    if cfg!(windows) {
        // Windows PowerShell (not pwsh) can load the WinRT toast types. Text goes in through
        // the environment, so nothing in it is ever run.
        const TOAST: &str = "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
$x = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
$t = $x.GetElementsByTagName('text'); \
$t.Item(0).AppendChild($x.CreateTextNode($env:HYDRA_TITLE)) > $null; \
$t.Item(1).AppendChild($x.CreateTextNode($env:HYDRA_BODY)) > $null; \
$a = $x.CreateElement('audio'); $a.SetAttribute('silent', 'true'); $x.DocumentElement.AppendChild($a) > $null; \
if ($env:HYDRA_LINK) { $x.DocumentElement.SetAttribute('activationType', 'protocol'); $x.DocumentElement.SetAttribute('launch', $env:HYDRA_LINK) }; \
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))";
        let _ = quiet(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", TOAST]))
            .env("HYDRA_TITLE", title)
            .env("HYDRA_BODY", body)
            .env("HYDRA_LINK", link.unwrap_or_default())
            .status();
    } else if cfg!(target_os = "macos") {
        // macOS's own notifications can't run anything when clicked; terminal-notifier can.
        if let (Some(link), Ok(exe)) = (link, std::env::current_exe())
            && on_path("terminal-notifier")
        {
            let sh = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
            let run = format!("{} reveal {}", sh(&exe.to_string_lossy()), sh(link));
            if quiet(Command::new("terminal-notifier").args(["-title", title, "-message", body, "-execute", &run])).status().is_ok_and(|s| s.success()) {
                return;
            }
        }
        // AppleScript strings: escape backslashes and quotes.
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("display notification \"{}\" with title \"{}\"", q(body), q(title));
        let _ = quiet(Command::new("osascript").args(["-e", &script])).status();
    } else {
        // With a link: an action the notification runs when clicked, and wait to hear it.
        // notify-send before 0.7.9 doesn't know --action; then a plain one.
        if let (Some(link), Ok(exe)) = (link, std::env::current_exe()) {
            let out = crate::proc::quiet(Command::new("notify-send").args(["--app-name=hydra", "--action=default=Open", "--wait", "--", title, body]))
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            if let Ok(out) = out
                && out.status.success()
            {
                if String::from_utf8_lossy(&out.stdout).trim() == "default" {
                    let _ = quiet(Command::new(exe).args(["reveal", link])).status();
                }
                return;
            }
        }
        let _ = quiet(Command::new("notify-send").args(["--app-name=hydra", "--", title, body])).status();
    }
}

/// Whether a program is on PATH.
fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
}

/// The file behind a sound name on this system (or the path itself for a custom sound).
pub fn sound_file(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("off") {
        return None;
    }
    if name.contains(['/', '\\']) || name.contains('.') {
        let p = PathBuf::from(name);
        return p.exists().then_some(p);
    }
    let lower = name.to_lowercase();
    let file = if cfg!(windows) {
        let f = match lower.as_str() {
            "glass" => "Windows Notify System Generic.wav",
            "ping" => "Windows Ding.wav",
            "chime" => "Windows Notify Messaging.wav",
            "pop" => "Windows Notify Email.wav",
            _ => return None,
        };
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        windir.join("Media").join(f)
    } else if cfg!(target_os = "macos") {
        let f = match lower.as_str() {
            "glass" => "Glass",
            "ping" => "Ping",
            "chime" => "Hero",
            "pop" => "Pop",
            _ => return None,
        };
        PathBuf::from(format!("/System/Library/Sounds/{f}.aiff"))
    } else {
        let f = match lower.as_str() {
            "glass" => "complete",
            "ping" => "bell",
            "chime" => "message-new-instant",
            "pop" => "message",
            _ => return None,
        };
        PathBuf::from(format!("/usr/share/sounds/freedesktop/stereo/{f}.oga"))
    };
    file.exists().then_some(file)
}

/// Play a sound by name (see `SOUNDS`) or file path. Blocks until it has played.
pub fn play(name: &str) {
    let Some(file) = sound_file(name) else { return };
    if cfg!(windows) {
        // MediaPlayer handles wav, mp3 and wma; it plays asynchronously, so wait a moment.
        const PLAY: &str = "Add-Type -AssemblyName PresentationCore; $p = New-Object System.Windows.Media.MediaPlayer; \
$p.Open([Uri]$env:HYDRA_SOUND); $p.Volume = 1; $p.Play(); Start-Sleep -Milliseconds 2500";
        let _ = quiet(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", PLAY]))
            .env("HYDRA_SOUND", &file)
            .status();
    } else if cfg!(target_os = "macos") {
        let _ = quiet(Command::new("afplay").arg(&file)).status();
    } else {
        for player in ["paplay", "pw-play"] {
            if quiet(Command::new(player).arg(&file)).status().is_ok_and(|s| s.success()) {
                return;
            }
        }
        let _ = quiet(Command::new("canberra-gtk-play").arg("-f").arg(&file)).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_names() {
        assert!(sound_file("off").is_none() && sound_file("").is_none());
        assert!(sound_file("no-such-sound").is_none());
        assert!(sound_file("/definitely/missing.wav").is_none(), "a custom file must exist");
        if cfg!(windows) {
            assert!(sound_file("ping").is_some_and(|p| p.ends_with("Windows Ding.wav")));
        }
    }
}
