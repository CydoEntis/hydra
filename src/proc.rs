//! Running other programs: without a console window flashing up on Windows, with their
//! output or their error as text, and command lines through the user's shell.

use std::path::Path;
use std::process::Command;

/// Windows: start a console program without opening a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Don't open a console window for this program (Windows; nothing elsewhere).
/// Whether a program is on PATH (with Windows' own extensions).
pub fn on_path(tool: &str) -> bool {
    let exts: &[&str] = if cfg!(windows) { &["exe", "cmd", "bat", "com"] } else { &[""] };
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| exts.iter().any(|e| if e.is_empty() { d.join(tool).is_file() } else { d.join(format!("{tool}.{e}")).is_file() }))
    })
}

pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run to the end: its output on success, else the first line of what it said went wrong.
pub fn run(cmd: &mut Command) -> Result<String, String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = quiet(cmd).output().map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        // git says "warning: …" before the "error:" / "fatal:" line that matters.
        let line = err
            .lines()
            .find(|l| l.starts_with("error") || l.starts_with("fatal"))
            .or_else(|| err.lines().find(|l| !l.trim().is_empty()))
            .unwrap_or("")
            .trim()
            .to_string();
        Err(if line.is_empty() { format!("{program} failed") } else { line })
    }
}

/// `git -C <dir> <args>`. Its messages are kept in English, since callers match some of
/// them.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C"))
}

/// A command line run through the user's shell (`shell` as from `Config::shell_command`).
/// PowerShell needs `&` to run a quoted path. `interactive`: it runs in a terminal and may
/// ask the user things; otherwise PowerShell is told not to.
pub fn shell(shell: &[String], line: &str, interactive: bool) -> Command {
    let program = shell.first().map(String::as_str).unwrap_or("sh");
    let exe = Path::new(program).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut c = Command::new(program);
    match exe.as_str() {
        "pwsh" | "powershell" => {
            let line = if line.starts_with('"') { format!("& {line}") } else { line.to_string() };
            c.arg("-NoProfile");
            if !interactive {
                c.arg("-NonInteractive");
            }
            c.args(["-Command", &line]);
        }
        "cmd" => {
            c.args(["/C", line]);
        }
        _ => {
            c.args(["-c", line]);
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(c: &Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn shell_lines_for_each_shell() {
        let pwsh = vec!["pwsh.exe".to_string()];
        assert_eq!(args(&shell(&pwsh, "npm i", false)), ["-NoProfile", "-NonInteractive", "-Command", "npm i"]);
        assert_eq!(args(&shell(&pwsh, "\"C:/x y/run.ps1\"", true)), ["-NoProfile", "-Command", "& \"C:/x y/run.ps1\""]);
        assert_eq!(args(&shell(&["cmd.exe".to_string()], "dir", false)), ["/C", "dir"]);
        assert_eq!(args(&shell(&["/bin/zsh".to_string()], "ls", false)), ["-c", "ls"]);
    }

    #[test]
    fn errors_are_the_first_thing_it_said() {
        let dir = std::env::temp_dir();
        let e = git(&dir.join("hydra-no-such-repo"), &["status"]).unwrap_err();
        assert!(!e.is_empty() && !e.contains('\n'), "{e}");
    }
}
