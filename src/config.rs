//! `config.toml`: everything user-facing is configurable here. Every field has a default,
//! so a config file only needs the keys you want to change.

use crate::keys::{self, Action, KeySpec};
use crate::theme::{Theme, ThemeOverrides};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub const EXAMPLE: &str = include_str!("../config.example.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Key that arms command mode, tmux style.
    pub prefix: String,
    /// Teach Claude (when hydra starts it) to move itself into a worktree when asked.
    pub teach_agents: bool,
    /// Where tickets come from (Ctrl+Space i).
    pub tickets: Tickets,
    /// What agents may do through `hydra mcp`.
    pub mcp: Mcp,
    /// Named setups for + New: e.g. a worktree with claude, a dev server and lazygit.
    pub recipes: Vec<Recipe>,
    /// Saved launches: an agent, a model and a prompt, e.g. "commit and push" (Ctrl+Space .).
    pub presets: Vec<Preset>,
    /// Put agents to sleep after sitting finished or idle this long ("15m", "1h", "4h",
    /// "never"); they resume where they were when you open them.
    pub sleep_after: String,
    /// Editor for "open in editor" (Files, Changes). Empty: $VISUAL, $EDITOR, then `code`.
    /// Terminal editors (nvim, vim, hx, nano, micro, …) open inside hydra beside the agent.
    pub editor: String,
    pub theme: String,
    pub theme_overrides: ThemeOverrides,
    /// Shell for new panes. Default: pwsh (or powershell) on Windows, $SHELL elsewhere.
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    /// Teach PowerShell panes to report their directory (needed for the sidebar to follow
    /// `cd`; other shells report it on their own).
    pub shell_integration: bool,
    /// Extra environment for every pane.
    pub env: BTreeMap<String, String>,
    /// Scrollback lines kept per pane by the client.
    pub scrollback: usize,
    /// Bytes of output the daemon keeps per pane to replay on reattach.
    pub replay_bytes: usize,
    pub ui: Ui,
    pub restore: Restore,
    /// Starting an agent inside a repo that isn't a workspace yet makes it one.
    pub auto_workspace: bool,
    pub worktree: Worktree,
    pub quick: Quick,
    pub icons: Icons,
    pub keys: Keys,
    pub detection: Detection,
    pub notify: Notify,
    /// Agent definitions merged over the built-ins by `name`.
    pub agents: Vec<AgentDef>,
    /// Drop the built-in agent list and use only `agents`.
    pub agents_replace_defaults: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub sidebar: bool,
    /// "left" or "right".
    pub sidebar_position: String,
    /// Milliseconds after the prefix before the which-key popup shows (0 = immediately).
    pub which_key_delay_ms: u64,
    pub which_key: bool,
    pub mouse: bool,
    /// Animate the working icon with these frames (empty = use `icons.working`).
    pub spinner: Vec<String>,
    /// One colour per workspace, assigned in order and kept across restarts.
    pub workspace_colors: Vec<String>,
    /// The welcome screen when hydra starts.
    pub splash: bool,
    /// Sidebar sessions (and worktrees) sorted needs → done → working → idle.
    pub attention_sort: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Restore {
    /// Rebuild the saved workspaces, tabs and panes when the server starts.
    pub enabled: bool,
    /// Relaunch agents with their resume command (`claude --resume <id>`, ...).
    pub agents: bool,
    /// Re-run commands panes were started with (`spawn-right:lazygit`, `hydra split -- x`).
    pub commands: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Worktree {
    /// Where new worktrees go. `{repo}`, `{repo_parent}`, `{branch}` are substituted.
    pub dir: String,
    /// Command to start in a new worktree's first pane (e.g. "claude"). Empty = a shell.
    pub command: String,
    /// New agent panes (+ Pane, quick prompt) get their own worktree when they start in a repo.
    pub per_agent: bool,
    /// Closing the last thing running in a worktree hydra made removes its folder (the
    /// branch is kept; a worktree with uncommitted changes is left alone).
    pub delete_with_last: bool,
    /// Keep this agent (e.g. "claude") booted in a spare worktree of the repo you last
    /// used, so the next one starts instantly. Costs one idle agent. Empty: off.
    pub prewarm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Tickets {
    /// Tabs, in order: "github", "linear", "plane".
    pub sources: Vec<String>,
    /// Keys: better in config.local.toml (never synced) or the LINEAR_API_KEY /
    /// PLANE_API_KEY environment variables.
    pub linear_key: String,
    pub plane_key: String,
    /// Plane's API and web addresses (change both for self-hosted) and your workspace slug.
    pub plane_url: String,
    pub plane_app_url: String,
    pub plane_workspace: String,
    /// Which source a project opens on, by project (repo folder) name: shop-api = "linear".
    pub projects: std::collections::BTreeMap<String, String>,
}

impl Default for Tickets {
    fn default() -> Self {
        Tickets {
            sources: vec!["github".into(), "linear".into(), "plane".into()],
            linear_key: String::new(),
            plane_key: String::new(),
            plane_url: "https://api.plane.so".into(),
            plane_app_url: "https://app.plane.so".into(),
            plane_workspace: String::new(),
            projects: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Mcp {
    /// May agents answer other agents' prompts with "yes"? never | safe | always
    /// ("no" is always allowed).
    pub approve: String,
    /// "safe": the prompt must mention one of these.
    pub safe: Vec<String>,
    /// project: only sessions in the calling agent's repo; all: every session.
    pub scope: String,
}

impl Default for Mcp {
    fn default() -> Self {
        Mcp {
            approve: "never".into(),
            safe: [
                "npm test", "npm run test", "npm run lint", "pnpm test", "yarn test", "cargo test", "cargo check", "cargo clippy", "pytest",
                "go test", "git status", "git diff", "git log",
            ]
            .map(String::from)
            .to_vec(),
            scope: "project".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Recipe {
    pub name: String,
    /// Start in its own worktree (in a git project).
    pub worktree: bool,
    /// Commands: the first is the main one; the rest start beside it in the same folder.
    pub run: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    /// Which agent (a name from + New's RUN row).
    pub agent: String,
    /// One of the agent's models; empty is its default.
    pub model: String,
    /// What to ask. `{task}` is replaced with what you type; with no `{task}` the preset
    /// starts in one key, without asking.
    pub prompt: String,
    /// "send": to the agent you're on; "beside": a new one beside it, in its folder;
    /// "worktree": a new one in a new worktree.
    #[serde(rename = "where")]
    pub place: String,
}

impl Default for Preset {
    fn default() -> Self {
        Preset { name: String::new(), agent: "claude".into(), model: String::new(), prompt: String::new(), place: "beside".into() }
    }
}

impl Preset {
    pub fn asks(&self) -> bool {
        self.prompt.contains("{task}") || self.prompt.trim().is_empty()
    }

    /// The prompt with the task in it.
    pub fn fill(&self, task: &str) -> String {
        let task = task.trim();
        if self.prompt.contains("{task}") {
            self.prompt.replace("{task}", task).trim().to_string()
        } else if self.prompt.trim().is_empty() {
            task.to_string()
        } else if task.is_empty() {
            self.prompt.trim().to_string()
        } else {
            format!("{} {task}", self.prompt.trim())
        }
    }
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe { name: String::new(), worktree: true, run: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Quick {
    /// Agents the quick prompt can launch; Tab cycles through them.
    pub agents: Vec<QuickAgent>,
    /// Where a new agent goes: "right", "down", "tab", "worktree" or "here" (send to the
    /// focused pane's agent).
    pub place: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickAgent {
    pub name: String,
    /// `{prompt}` is replaced with the task, quoted for your shell.
    pub command: String,
    /// Models to choose from when starting it (the first choice is always its default).
    pub models: Vec<String>,
    /// The flag that picks one (default `--model`).
    pub model_flag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Icons {
    pub working: String,
    pub blocked: String,
    pub done: String,
    pub idle: String,
    pub shell: String,
    pub active_workspace: String,
    /// Shown before a workspace's branch (default: the Nerd Font / Powerline branch glyph).
    pub branch: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// Bindings after the prefix: key spec -> action. Merged over the defaults.
    pub prefix: BTreeMap<String, String>,
    /// Bindings that work without the prefix.
    pub global: BTreeMap<String, String>,
    /// Ignore the built-in bindings entirely.
    pub replace_defaults: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Detection {
    /// Output within this window counts as "working" for agents without hooks or patterns.
    pub working_window_ms: u64,
    /// Output this soon after a keystroke is treated as echo, not work.
    pub echo_grace_ms: u64,
    /// How often the process tree is scanned for agents.
    pub scan_interval_ms: u64,
    /// Rows from the bottom of the screen that status patterns are matched against.
    pub pattern_rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    /// Ring the terminal bell when an agent becomes blocked or finishes out of view.
    pub bell: bool,
    /// A desktop notification when an agent you're not looking at needs you or finishes
    /// (also when no hydra window is open).
    pub desktop: bool,
    /// Sounds: glass, ping, chime, pop, off, or a path to a sound file.
    pub sound_needs: String,
    pub sound_done: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDef {
    pub name: String,
    /// Executable names (without .exe), matched case-insensitively.
    pub process: Vec<String>,
    /// Substrings of the command line, for agents that run under node/bun/python.
    pub cmdline: Vec<String>,
    /// Regexes (case-insensitive) on the bottom of the screen meaning "waiting on you".
    pub blocked_patterns: Vec<String>,
    /// Regexes meaning "mid-turn". When absent, recent output is used instead.
    pub working_patterns: Vec<String>,
    /// Command that resumes a known session; `{session}` is the id hooks reported.
    pub resume: Option<String>,
    /// Command that resumes the most recent session in the pane's directory.
    pub resume_last: Option<String>,
    /// Set to false to disable a built-in agent.
    pub enabled: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: "ctrl+space".into(),
            editor: String::new(),
            sleep_after: "never".into(),
            tickets: Tickets::default(),
            teach_agents: true,
            mcp: Mcp::default(),
            recipes: Vec::new(),
            presets: Vec::new(),
            theme: "hydra".into(),
            theme_overrides: ThemeOverrides::default(),
            shell: None,
            shell_args: Vec::new(),
            shell_integration: true,
            env: BTreeMap::new(),
            scrollback: 10_000,
            replay_bytes: 2 * 1024 * 1024,
            ui: Ui::default(),
            restore: Restore::default(),
            auto_workspace: false,
            worktree: Worktree::default(),
            quick: Quick::default(),
            icons: Icons::default(),
            keys: Keys::default(),
            detection: Detection::default(),
            notify: Notify::default(),
            agents: Vec::new(),
            agents_replace_defaults: false,
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            sidebar: true,
            sidebar_position: "left".into(),
            which_key_delay_ms: 350,
            which_key: false,
            mouse: true,
            spinner: ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"].map(String::from).to_vec(),
            workspace_colors: ["#a593ff", "#5aa9ff", "#ff7ab6", "#3dd6c0", "#e8c565", "#ff9f6b", "#c792ea", "#7fd8a4"]
                .map(String::from)
                .to_vec(),
            splash: true,
            attention_sort: true,
        }
    }
}

impl Default for Quick {
    fn default() -> Self {
        let a = |name: &str, command: &str, models: &[&str], flag: &str| QuickAgent {
            name: name.into(),
            command: command.into(),
            models: models.iter().map(|m| m.to_string()).collect(),
            model_flag: flag.into(),
        };
        Quick {
            agents: vec![a("claude", "claude {prompt}", &["opus", "sonnet", "haiku"], "--model"), a("codex", "codex {prompt}", &[], "-m")],
            place: "worktree".into(),
        }
    }
}

impl Default for Restore {
    fn default() -> Self {
        Restore { enabled: true, agents: true, commands: true }
    }
}

impl Default for Worktree {
    fn default() -> Self {
        Worktree {
            dir: "{repo_parent}/{repo}-worktrees/{branch}".into(),
            command: String::new(),
            per_agent: true,
            delete_with_last: true,
            prewarm: String::new(),
        }
    }
}

impl Default for Icons {
    fn default() -> Self {
        Icons {
            working: "⠹".into(),
            blocked: "●".into(),
            done: "✓".into(),
            idle: "○".into(),
            shell: "›".into(),
            active_workspace: "▌".into(),
            branch: "\u{e0a0} ".into(),
        }
    }
}

impl Default for Detection {
    fn default() -> Self {
        Detection { working_window_ms: 1500, echo_grace_ms: 300, scan_interval_ms: 1000, pattern_rows: 12 }
    }
}

impl Default for Notify {
    fn default() -> Self {
        Notify { bell: false, desktop: true, sound_needs: "ping".into(), sound_done: "glass".into() }
    }
}

fn agent(name: &str, process: &[&str], cmdline: &[&str], working: &[&str], blocked: &[&str]) -> AgentDef {
    let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
    AgentDef {
        name: name.into(),
        process: v(process),
        cmdline: v(cmdline),
        working_patterns: v(working),
        blocked_patterns: v(blocked),
        resume: None,
        resume_last: None,
        enabled: None,
    }
}

fn resumable(mut a: AgentDef, resume: &str, last: &str) -> AgentDef {
    a.resume = Some(resume.into());
    a.resume_last = Some(last.into());
    a
}

pub fn builtin_agents() -> Vec<AgentDef> {
    vec![
        resumable(agent(
            "claude",
            &["claude"],
            &["@anthropic-ai/claude-code", "claude-code/cli"],
            // "esc to interrupt", or the spinner line: "Misting… (5s · ↓ 219 tokens)".
            &[r"esc to interrupt", r"…\s*\(\d+[smh][^)]*tokens"],
            &[
                r"Do you want to (proceed|make this edit|create|run|allow)",
                r"❯ 1\. Yes",
                r"Would you like to proceed",
                r"Do you trust the files",
                // Its question and permission dialogs.
                r"Enter to select",
                r"Esc to cancel",
            ],
        ), "claude --resume {session}", "claude --continue"),
        resumable(agent(
            "codex",
            &["codex"],
            &["@openai/codex"],
            &[r"esc to interrupt"],
            &[r"Allow command\?", r"Would you like to (run|make|apply)", r"approve this", r"Trust this folder\?", r"› 1\. Yes"],
        ), "codex resume {session}", "codex resume --last"),
        agent("gemini", &["gemini"], &["@google/gemini-cli"], &[r"esc to cancel"], &[r"Allow execution", r"Apply this change\?"]),
        resumable(agent("opencode", &["opencode"], &["opencode-ai"], &[r"esc interrupt"], &[r"Permission required"]), "opencode --session {session}", "opencode --continue"),
        agent("cursor", &["cursor-agent"], &[], &[r"ctrl\+c to stop"], &[r"Run this command\?"]),
        resumable(agent("copilot", &["copilot"], &["@github/copilot"], &[r"esc to cancel"], &[r"Do you want to"]), "copilot --resume {session}", "copilot --continue"),
        agent("amp", &["amp"], &["@sourcegraph/amp"], &[r"esc to cancel"], &[r"Allow\?"]),
        agent("qwen", &["qwen"], &["@qwen-code/qwen-code"], &[r"esc to cancel"], &[r"Allow execution"]),
        agent("aider", &["aider"], &["aider-chat", "\\aider", "/aider"], &[], &[r"\(Y\)es/\(N\)o"]),
        agent("goose", &["goose"], &[], &[], &[]),
        agent("crush", &["crush"], &[], &[], &[]),
        agent("droid", &["droid"], &[], &[], &[]),
        agent("pi", &["pi"], &["@mariozechner/pi"], &[], &[]),
        agent("kiro", &["kiro-cli", "q"], &[], &[], &[]),
        agent("grok", &["grok"], &["@vibe-kit/grok-cli", "grok-cli"], &[r"esc to interrupt"], &[r"Do you want to"]),
        agent("auggie", &["auggie"], &["@augmentcode/auggie"], &[], &[]),
        agent("kimi", &["kimi"], &["kimi-cli"], &[], &[]),
    ]
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("HYDRA_CONFIG") {
        return PathBuf::from(p);
    }
    if cfg!(windows)
        && let Some(d) = directories::BaseDirs::new() {
            return d.config_dir().join("hydra").join("config.toml");
        }
    // XDG-style on macOS too: that's where terminal people look.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("hydra").join("config.toml")
}

/// Write a file so a reader (or a crash) never sees half of it: write a temporary file
/// beside it, then swap it in.
pub fn write_atomic(path: &std::path::Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.hydra-tmp"));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Read a JSON state file; one that's there but won't parse is kept as `<name>.bak` (so a
/// bad write or a hand edit loses nothing) and the defaults are used.
pub fn read_state<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    let Ok(text) = std::fs::read_to_string(path) else { return T::default() };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let bak = path.with_file_name(format!("{name}.bak"));
            tracing::warn!("{} doesn't parse ({e}); kept it as {}", path.display(), bak.display());
            let _ = std::fs::copy(path, &bak);
            T::default()
        }
    }
}

pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "hydra")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
}

/// Lay `over` on top of `base`: tables merge key by key, anything else is replaced.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// The app used to be called drover: bring its config and saved sessions over once,
/// the first time hydra runs without its own. The old files are left in place.
pub fn migrate_from_drover() {
    if std::env::var_os("HYDRA_CONFIG").is_some() {
        return;
    }
    let new_cfg = config_path();
    let old_cfg = new_cfg.parent().and_then(|d| d.parent()).map(|d| d.join("drover").join("config.toml"));
    if let Some(old) = old_cfg
        && !new_cfg.exists()
        && old.exists()
    {
        if let Some(dir) = new_cfg.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = std::fs::read_to_string(&old) {
            let _ = std::fs::write(&new_cfg, text.replace("theme = \"drover\"", "theme = \"hydra\""));
        }
    }
    let new_data = data_dir();
    let old_data = directories::ProjectDirs::from("", "", "drover").map(|d| d.data_local_dir().to_path_buf());
    if let Some(old) = old_data
        && let Ok(entries) = std::fs::read_dir(&old)
    {
        let _ = std::fs::create_dir_all(&new_data);
        for e in entries.flatten() {
            let name = e.file_name();
            let to = new_data.join(&name);
            if name.to_string_lossy().starts_with("session-") && !to.exists() {
                let _ = std::fs::copy(e.path(), to);
            }
        }
    }
}

impl Config {
    /// `sleep_after` in seconds; None for never.
    pub fn sleep_secs(&self) -> Option<u64> {
        let s = self.sleep_after.trim().to_lowercase();
        let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
        let n: u64 = num.parse().ok().filter(|n| *n > 0)?;
        Some(match unit.trim() {
            "s" => n,
            "" | "m" | "min" => n * 60,
            "h" => n * 3600,
            _ => return None,
        })
    }

    /// Load the config file; a missing file is the defaults, a broken one is an error.
    pub fn load() -> Result<Config> {
        let path = config_path();
        let mut value: toml::Table = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        // This machine's own settings (never synced) win over the shared ones.
        let local = path.with_file_name("config.local.toml");
        if std::env::var_os("HYDRA_CONFIG").is_none()
            && let Ok(s) = std::fs::read_to_string(&local)
        {
            let over: toml::Table = toml::from_str(&s).with_context(|| format!("parsing {}", local.display()))?;
            merge(&mut value, over);
        }
        toml::Value::Table(value).try_into().with_context(|| format!("parsing {}", path.display()))
    }

    /// Load, falling back to defaults and returning the error message for display.
    pub fn load_or_default() -> (Config, Option<String>) {
        match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{e:#}"))),
        }
    }

    pub fn theme(&self) -> Theme {
        let mut t = Theme::named(&self.theme);
        t.apply(&self.theme_overrides);
        t
    }

    pub fn shell_command(&self) -> Vec<String> {
        let shell = self.shell.clone().filter(|s| !s.is_empty()).unwrap_or_else(default_shell);
        let mut v = vec![shell];
        v.extend(self.shell_args.iter().cloned());
        v
    }

    /// Quote `s` as one argument for the configured shell.
    pub fn quote_for_shell(&self, s: &str) -> String {
        let shell = self.shell_command();
        let exe = std::path::Path::new(&shell[0])
            .file_stem()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match exe.as_str() {
            "pwsh" | "powershell" | "nu" => format!("'{}'", s.replace('\'', "''")),
            "cmd" => format!("\"{}\"", s.replace('"', "\"\"").replace(['\r', '\n'], " ")),
            _ => format!("'{}'", s.replace('\'', "'\\''")),
        }
    }

    pub fn keymap(&self) -> Keymap {
        let mut warnings = Vec::new();
        let prefix = self.prefix.parse::<KeySpec>().unwrap_or_else(|e| {
            warnings.push(format!("prefix: {e}"));
            "ctrl+space".parse().unwrap()
        });
        let mut build = |defaults: &[(&str, &str)], user: &BTreeMap<String, String>| {
            let mut map: HashMap<KeySpec, Action> = HashMap::new();
            let mut order: Vec<KeySpec> = Vec::new();
            let pairs = if !self.keys.replace_defaults { defaults } else { Default::default() }
                .iter()
                .map(|(k, a)| (k.to_string(), a.to_string()))
                .chain(user.iter().map(|(k, a)| (k.clone(), a.clone())));
            for (k, a) in pairs {
                match (k.parse::<KeySpec>(), a.parse::<Action>()) {
                    (Ok(k), Ok(Action::None)) => {
                        map.remove(&k);
                    }
                    (Ok(k), Ok(a)) => {
                        if map.insert(k, a).is_none() {
                            order.push(k);
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => warnings.push(format!("key `{k}`: {e}")),
                }
            }
            order.retain(|k| map.contains_key(k));
            (map, order)
        };
        let (prefixed, prefixed_order) = build(keys::DEFAULT_PREFIX_KEYS, &self.keys.prefix);
        let (global, _) = build(keys::DEFAULT_GLOBAL_KEYS, &self.keys.global);
        Keymap { prefix, prefixed, prefixed_order, global, warnings }
    }

    pub fn agent_defs(&self) -> Vec<CompiledAgent> {
        let mut defs: Vec<AgentDef> = if self.agents_replace_defaults { Vec::new() } else { builtin_agents() };
        for user in &self.agents {
            match defs.iter_mut().find(|d| d.name == user.name) {
                Some(d) => {
                    // Non-empty user fields replace the built-in ones.
                    if !user.process.is_empty() {
                        d.process = user.process.clone();
                    }
                    if !user.cmdline.is_empty() {
                        d.cmdline = user.cmdline.clone();
                    }
                    if !user.blocked_patterns.is_empty() {
                        d.blocked_patterns = user.blocked_patterns.clone();
                    }
                    if !user.working_patterns.is_empty() {
                        d.working_patterns = user.working_patterns.clone();
                    }
                    if user.resume.is_some() {
                        d.resume = user.resume.clone();
                    }
                    if user.resume_last.is_some() {
                        d.resume_last = user.resume_last.clone();
                    }
                    d.enabled = user.enabled;
                }
                None => defs.push(user.clone()),
            }
        }
        let re = |ps: &[String]| -> Vec<Regex> {
            ps.iter().filter_map(|p| Regex::new(&format!("(?i){p}")).ok()).collect()
        };
        defs.into_iter()
            .filter(|d| d.enabled != Some(false))
            .map(|d| CompiledAgent {
                process: d.process.iter().map(|p| p.to_ascii_lowercase()).collect(),
                cmdline: d.cmdline.clone(),
                blocked: re(&d.blocked_patterns),
                working: re(&d.working_patterns),
                resume: d.resume.filter(|s| !s.is_empty()),
                resume_last: d.resume_last.filter(|s| !s.is_empty()),
                name: d.name,
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CompiledAgent {
    pub name: String,
    pub process: Vec<String>,
    pub cmdline: Vec<String>,
    pub blocked: Vec<Regex>,
    pub working: Vec<Regex>,
    pub resume: Option<String>,
    pub resume_last: Option<String>,
}

pub struct Keymap {
    pub prefix: KeySpec,
    pub prefixed: HashMap<KeySpec, Action>,
    /// Prefix bindings in definition order, for the which-key and help screens.
    pub prefixed_order: Vec<KeySpec>,
    pub global: HashMap<KeySpec, Action>,
    pub warnings: Vec<String>,
}

pub fn default_shell() -> String {
    if cfg!(windows) {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if which(candidate) {
                return candidate.into();
            }
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

fn which(exe: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn state_files_swap_in_and_keep_a_bad_copy() {
        let dir = std::env::temp_dir().join(format!("hydra-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.json");
        super::write_atomic(&f, "[1,2]").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "[1,2]");
        assert!(!dir.join(".s.json.hydra-tmp").exists(), "no temporary file left behind");
        let v: Vec<u32> = super::read_state(&f);
        assert_eq!(v, vec![1, 2]);
        std::fs::write(&f, "[1,2").unwrap();
        let v: Vec<u32> = super::read_state(&f);
        assert!(v.is_empty(), "a broken file gives the defaults");
        assert_eq!(std::fs::read_to_string(dir.join("s.json.bak")).unwrap(), "[1,2", "and is kept as .bak");
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn local_settings_win() {
        let mut base: toml::Table = toml::from_str("theme = 'hydra'\n[ui]\nmouse = true\nsplash = true\n").unwrap();
        let over: toml::Table = toml::from_str("shell = 'zsh'\n[ui]\nsplash = false\n").unwrap();
        merge(&mut base, over);
        let cfg: Config = toml::Value::Table(base).try_into().unwrap();
        assert_eq!(cfg.shell.as_deref(), Some("zsh"));
        assert!(cfg.ui.mouse && !cfg.ui.splash, "tables merge key by key");
        assert_eq!(cfg.theme, "hydra");
    }

    #[test]
    fn sleep_after_parses() {
        let mut c = Config::default();
        assert_eq!(c.sleep_secs(), None);
        for (v, want) in [("15m", Some(900)), ("1h", Some(3600)), ("30s", Some(30)), ("20", Some(1200)), ("never", None), ("0m", None)] {
            c.sleep_after = v.into();
            assert_eq!(c.sleep_secs(), want, "{v}");
        }
    }

    #[test]
    fn example_config_parses_and_binds() {
        let c: Config = toml::from_str(EXAMPLE).expect("config.example.toml must parse");
        let km = c.keymap();
        assert!(km.warnings.is_empty(), "{:?}", km.warnings);
        assert!(!c.agent_defs().is_empty());
    }

    #[test]
    fn user_keys_override_and_unbind() {
        let c: Config = toml::from_str(
            r#"
            [keys.prefix]
            x = "none"
            q = "close-pane"
            "#,
        )
        .unwrap();
        let km = c.keymap();
        assert!(!km.prefixed.contains_key(&"x".parse().unwrap()));
        assert_eq!(km.prefixed[&"q".parse().unwrap()], Action::ClosePane);
    }
}
