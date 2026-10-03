//! State for the quick prompt, pane menu and settings screen. Drawing lives in render.rs;
//! the key handling that needs the whole app lives in mod.rs.

use crate::config::Config;
use crate::keys::Action;
use anyhow::{Context, Result};

/// Where the quick prompt sends its task.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Right,
    Down,
    Tab,
    Worktree,
    /// Type it into the focused pane's agent.
    Here,
}

impl Place {
    pub const ALL: [Place; 5] = [Place::Right, Place::Down, Place::Tab, Place::Worktree, Place::Here];

    pub fn parse(s: &str) -> Place {
        match s {
            "down" => Place::Down,
            "tab" => Place::Tab,
            "worktree" => Place::Worktree,
            "here" => Place::Here,
            _ => Place::Right,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Place::Right => "split right",
            Place::Down => "split down",
            Place::Tab => "new tab",
            Place::Worktree => "new worktree",
            Place::Here => "this pane's agent",
        }
    }

    pub fn next(self) -> Place {
        let i = Place::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Place::ALL[(i + 1) % Place::ALL.len()]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quick {
    pub text: String,
    pub agent: usize,
    pub place: Place,
}

/// A branch name made from a task: "fix the login bug" -> "q/fix-the-login-bug-3f2a".
pub fn branch_for(task: &str) -> String {
    let mut slug = String::new();
    for word in task.split_whitespace().take(6) {
        let w: String = word.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if w.is_empty() {
            continue;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w);
        if slug.len() > 32 {
            break;
        }
    }
    if slug.is_empty() {
        slug.push_str("task");
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u16 ^ d.as_secs() as u16)
        .unwrap_or(0);
    format!("q/{slug}-{nonce:04x}")
}

/// One row of the pane menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub key: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub sel: usize,
    /// Screen position to open at (mouse), else centred.
    pub at: Option<(u16, u16)>,
}

// ---- settings --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool,
    Choice(&'static [&'static str]),
    #[allow(dead_code)]
    Number { step: f64, min: f64, max: f64 },
    Int { step: i64, min: i64, max: i64 },
    Text,
    Key,
}

#[derive(Debug)]
pub struct Setting {
    pub path: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    /// Which page of the settings view it's on.
    pub cat: Cat,
    /// One plain sentence: what it changes.
    pub help: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cat {
    General,
    Sessions,
    Appearance,
    Agents,
    Projects,
    Keys,
}

impl Cat {
    pub const ALL: [Cat; 6] = [Cat::General, Cat::Sessions, Cat::Appearance, Cat::Agents, Cat::Projects, Cat::Keys];

    pub fn label(self) -> &'static str {
        match self {
            Cat::General => "General",
            Cat::Sessions => "Sessions",
            Cat::Appearance => "Appearance",
            Cat::Agents => "Agents",
            Cat::Projects => "Projects",
            Cat::Keys => "Keys",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Cat::General => "⚙",
            Cat::Sessions => "↻",
            Cat::Appearance => "◐",
            Cat::Agents => "●",
            Cat::Projects => "▌",
            Cat::Keys => "⌨",
        }
    }
}

pub const SETTINGS: &[Setting] = &[
    // General
    Setting { path: "prefix", label: "Leader key", kind: Kind::Key, cat: Cat::General, help: "Press it, let go, then a key. Enter, then press the new leader." },
    Setting { path: "ui.splash", label: "Splash screen", kind: Kind::Bool, cat: Cat::General, help: "Show the hydra and what happened while you were away when you start." },
    Setting { path: "ui.mouse", label: "Mouse", kind: Kind::Bool, cat: Cat::General, help: "Click, hover, scroll and drag. Hold Shift to select text with your terminal instead." },
    Setting { path: "ui.which_key", label: "Keys after a pause", kind: Kind::Bool, cat: Cat::General, help: "After the leader key, show every shortcut if you pause." },
    Setting { path: "ui.sidebar_position", label: "Sidebar side", kind: Kind::Choice(&["left", "right"]), cat: Cat::General, help: "Which edge the sidebar sits on." },
    Setting { path: "ui.layout", label: "Layout", kind: Kind::Choice(&["hydra", "workspaces", "tree", "dock", "sidebar"]), cat: Cat::General, help: "hydra is the main design; the others are earlier layouts." },
    Setting { path: "shell", label: "Shell", kind: Kind::Text, cat: Cat::General, help: "The shell new sessions run. Empty: pwsh / powershell on Windows, $SHELL elsewhere." },
    Setting { path: "shell_integration", label: "PowerShell folder tracking", kind: Kind::Bool, cat: Cat::General, help: "Lets hydra see where PowerShell sessions cd to." },
    // Sessions
    Setting { path: "ui.attention_sort", label: "Sort sidebar by attention", kind: Kind::Bool, cat: Cat::Sessions, help: "Sessions that need you float to the top, then done, then working, then idle." },
    Setting { path: "notify.bell", label: "Bell when one needs you", kind: Kind::Bool, cat: Cat::Sessions, help: "Rings the terminal bell when a session you're not looking at needs you or finishes." },
    Setting { path: "restore.enabled", label: "Bring sessions back after a restart", kind: Kind::Bool, cat: Cat::Sessions, help: "Agents keep going in the background; after a reboot hydra rebuilds your sessions." },
    Setting { path: "restore.agents", label: "Resume agents", kind: Kind::Bool, cat: Cat::Sessions, help: "Restart agents in their last conversation (claude --resume, codex resume)." },
    Setting { path: "restore.commands", label: "Re-run commands", kind: Kind::Bool, cat: Cat::Sessions, help: "Run again the commands sessions were started with (lazygit, a dev server, ...)." },
    Setting { path: "scrollback", label: "Scrollback lines", kind: Kind::Int { step: 1000, min: 1000, max: 100_000 }, cat: Cat::Sessions, help: "How much history each session keeps for scrolling and search." },
    // Appearance
    Setting { path: "theme", label: "Theme", kind: Kind::Choice(crate::theme::BUILTIN), cat: Cat::Appearance, help: "Changes the whole app live. Agent output keeps its own colours; only the ANSI palette is themed." },
    // Agents
    Setting { path: "worktree.per_agent", label: "Own worktree per agent", kind: Kind::Bool, cat: Cat::Agents, help: "Quick-prompt agents in a git repo get their own branch and folder." },
    Setting { path: "quick.place", label: "Quick prompt opens in", kind: Kind::Choice(&["worktree", "right", "down", "tab", "here"]), cat: Cat::Agents, help: "Where an agent starts when you give it a task with the quick prompt." },
    Setting { path: "worktree.command", label: "Start in new worktrees", kind: Kind::Text, cat: Cat::Agents, help: "A command to run in every new worktree (e.g. claude). Empty: a shell." },
    Setting { path: "detection.working_window_ms", label: "Working window (ms)", kind: Kind::Int { step: 250, min: 250, max: 10_000 }, cat: Cat::Agents, help: "For agents without hooks: output this recent counts as working." },
];

pub fn in_cat(cat: Cat) -> Vec<&'static Setting> {
    SETTINGS.iter().filter(|s| s.cat == cat).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub sel: usize,
    /// Text being typed for a Text setting.
    pub editing: Option<String>,
    /// Waiting for a key press to become the leader key.
    pub capturing: bool,
}

/// The effective value of a setting (defaults included), as TOML.
pub fn current(cfg: &Config, path: &str) -> Option<toml::Value> {
    let mut v = toml::Value::try_from(cfg).ok()?;
    for part in path.split('.') {
        v = v.get(part)?.clone();
    }
    Some(v)
}

pub fn display(cfg: &Config, s: &Setting) -> String {
    match (s.kind, current(cfg, s.path)) {
        (Kind::Key, Some(toml::Value::String(k))) => {
            k.parse::<crate::keys::KeySpec>().map(|k| k.to_string()).unwrap_or(k)
        }
        (Kind::Bool, Some(toml::Value::Boolean(b))) => (if b { "on" } else { "off" }).into(),
        (Kind::Number { .. }, Some(toml::Value::Float(f))) => format!("{f:.2}"),
        (Kind::Text, Some(toml::Value::String(t))) if t.is_empty() => "(shell)".into(),
        (_, Some(toml::Value::String(t))) => t,
        (_, Some(other)) => other.to_string(),
        (_, None) => String::new(),
    }
}

/// The value one step left (`dir` -1) or right (+1) of the current one.
pub fn step(cfg: &Config, s: &Setting, dir: i64) -> Option<toml_edit::Value> {
    let cur = current(cfg, s.path)?;
    Some(match s.kind {
        Kind::Bool => (!cur.as_bool()?).into(),
        Kind::Choice(opts) => {
            let i = opts.iter().position(|o| Some(*o) == cur.as_str()).unwrap_or(0) as i64;
            let n = opts.len() as i64;
            opts[((i + dir).rem_euclid(n)) as usize].into()
        }
        Kind::Number { step, min, max } => {
            let v = cur.as_float().unwrap_or(0.0) + step * dir as f64;
            ((v.clamp(min, max) * 100.0).round() / 100.0).into()
        }
        Kind::Int { step, min, max } => (cur.as_integer().unwrap_or(0) + step * dir).clamp(min, max).into(),
        Kind::Text | Kind::Key => return None,
    })
}

/// Write one setting into config.toml, keeping the rest of the file (comments included).
pub fn write(path: &str, value: toml_edit::Value) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    write_at(&parts, value)
}

/// Write a value at a path of keys (a key may contain dots, like the `.` shortcut).
pub fn write_at(parts: &[&str], value: toml_edit::Value) -> Result<()> {
    let file = crate::config::config_path();
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("reading config"),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().context("config.toml has a syntax error; fix it first")?;
    let (leaf, tables) = parts.split_last().context("empty setting path")?;
    let mut table = doc.as_table_mut();
    for part in tables {
        if !table.contains_key(part) {
            // Implicit: `[keys.prefix]` without an empty `[keys]` above it.
            let mut new = toml_edit::Table::new();
            new.set_implicit(true);
            table.insert(part, toml_edit::Item::Table(new));
        }
        table = table[*part].as_table_mut().with_context(|| format!("`{part}` in config.toml is not a table"))?;
    }
    table.insert(leaf, toml_edit::value(value));
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, doc.to_string()).context("writing config")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_safe() {
        let b = branch_for("Fix the login bug, please!");
        assert!(b.starts_with("q/fix-the-login-bug-please-"), "{b}");
        assert!(branch_for("   ").starts_with("q/task-"));
    }

    #[test]
    fn settings_step_and_display() {
        let cfg = Config::default();
        for s in SETTINGS.iter().filter(|s| s.path != "shell") {
            // `shell` is unset by default (hydra picks one), so it has no value yet.
            assert!(current(&cfg, s.path).is_some(), "missing setting {}", s.path);
        }
        let lines = SETTINGS.iter().find(|s| s.path == "scrollback").unwrap();
        assert_eq!(step(&cfg, lines, 1).unwrap().as_integer(), Some(cfg.scrollback as i64 + 1000));
        let side = SETTINGS.iter().find(|s| s.path == "ui.sidebar_position").unwrap();
        assert_eq!(step(&cfg, side, 1).unwrap().as_str(), Some("right"));
        assert_eq!(step(&cfg, side, -1).unwrap().as_str(), Some("right"));
        let leader = SETTINGS.iter().find(|s| s.path == "prefix").unwrap();
        assert_eq!(display(&cfg, leader), "C-Space");
    }

    #[test]
    fn write_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("hydra-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, "# my notes\ntheme = \"nord\"\n\n[ui]\n# tint comment\nsidebar = true\n").unwrap();
        // SAFETY: tests touching HYDRA_CONFIG run in this one test only.
        unsafe { std::env::set_var("HYDRA_CONFIG", &file) };
        write("ui.workspace_tint", 0.2.into()).unwrap();
        write("worktree.command", "claude".into()).unwrap();
        let out = std::fs::read_to_string(&file).unwrap();
        unsafe { std::env::remove_var("HYDRA_CONFIG") };
        assert!(out.contains("# my notes") && out.contains("# tint comment"), "{out}");
        assert!(out.contains("workspace_tint = 0.2"), "{out}");
        assert!(out.contains("[worktree]") && out.contains("command = \"claude\""), "{out}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
