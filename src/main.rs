mod alert;
mod cli;
mod client;
mod config;
mod daemon;
mod gitfs;
mod ipc;
mod mcp;
mod keys;
mod layout;
mod protocol;
mod sync;
mod theme;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// An agent-aware terminal multiplexer.
#[derive(Parser)]
#[command(name = "hydra", version, about)]
struct Args {
    /// Directory to open as a workspace (switches to it if already open).
    path: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Attach to the running server (starting it if needed).
    Attach { path: Option<PathBuf> },
    /// Run the server in the foreground (normally started automatically).
    Daemon,
    /// List workspaces, tabs and panes.
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Print a pane's screen.
    Read { pane: Option<protocol::TermId> },
    /// Type text into a pane and press Enter.
    Send {
        #[arg(long, short)]
        pane: Option<protocol::TermId>,
        /// Don't press Enter afterwards.
        #[arg(long)]
        no_enter: bool,
        /// Send key presses instead of text, e.g. `--key ctrl+space --key %` (no Enter).
        #[arg(long = "key", short = 'k')]
        keys: Vec<String>,
        text: Vec<String>,
    },
    /// Split a pane, optionally running a command in the new one.
    Split {
        #[arg(long, short)]
        pane: Option<protocol::TermId>,
        /// Split downwards instead of to the right.
        #[arg(long, short)]
        down: bool,
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Open a new workspace.
    New {
        path: Option<PathBuf>,
        #[arg(long, short)]
        name: Option<String>,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Focus a pane (switches workspace and tab).
    Focus { pane: protocol::TermId },
    /// Close a pane.
    Close { pane: Option<protocol::TermId> },
    /// Report agent lifecycle from an agent's hook. Reads the hook JSON from stdin.
    Hook {
        /// Hook dialect: claude, codex, or any name with --status.
        agent: String,
        /// Explicit status: working, blocked, done, idle, gone.
        #[arg(long)]
        status: Option<String>,
        /// Extra payload (codex `notify` passes JSON as an argument).
        payload: Option<String>,
    },
    /// Install status hooks into an agent's config.
    Integrate {
        /// claude or codex
        agent: String,
        #[arg(long)]
        uninstall: bool,
    },
    /// Config helpers.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Create a git worktree for a branch and open it as a workspace.
    Worktree {
        branch: String,
        /// Start a new branch from this ref (default: current HEAD).
        #[arg(long)]
        base: Option<String>,
        /// Workspace whose repo to use (default: this pane's, else the active one).
        #[arg(long)]
        ws: Option<protocol::WsId>,
        /// Command for the new workspace's first pane, e.g. `-- claude`.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Close a worktree workspace and remove its checkout.
    WorktreeRemove {
        #[arg(long)]
        ws: Option<protocol::WsId>,
        /// Remove even with uncommitted changes.
        #[arg(long)]
        force: bool,
    },
    /// Stop the server and every pane. The session is kept for restore unless --forget.
    KillServer {
        #[arg(long)]
        forget: bool,
    },
    /// Show a desktop notification and play the needs-you and done sounds, to check them.
    TestAlert,
    /// Run as an MCP server (stdio) so agents can see and steer the others.
    /// `hydra integrate mcp` registers it with Claude Code.
    Mcp,
    /// Share your config and ideas between machines through a private GitHub repo:
    /// `hydra sync setup [repo]`, `hydra sync` (pull + push now), `hydra sync off`.
    Sync {
        action: Option<String>,
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the config file location.
    Path,
    /// Write the annotated example config (won't overwrite).
    Init,
    /// Print the annotated example config.
    Example,
}

fn main() {
    let args = Args::parse();
    config::migrate_from_drover();
    // `hydra <dir>` opens a directory; a typo'd subcommand shouldn't silently attach.
    if let Some(p) = args.path.as_ref().or(match &args.cmd {
        Some(Cmd::Attach { path }) => path.as_ref(),
        _ => None,
    }) && !p.is_dir()
    {
        eprintln!("hydra: `{}` is not a directory or a command (see `hydra --help`)", p.display());
        std::process::exit(2);
    }
    let result = match args.cmd {
        None => client::run(client::Options { open: args.path }),
        Some(Cmd::Attach { path }) => client::run(client::Options { open: path }),
        Some(Cmd::Daemon) => daemon::run(),
        Some(Cmd::Ls { json }) => cli::ls(json),
        Some(Cmd::Read { pane }) => cli::read(pane),
        Some(Cmd::Send { pane, keys, .. }) if !keys.is_empty() => cli::send_keys(pane, keys),
        Some(Cmd::Send { pane, no_enter, text, .. }) => cli::send(pane, text.join(" "), !no_enter),
        Some(Cmd::Split { pane, down, command }) => cli::split(pane, down, command),
        Some(Cmd::New { path, name, command }) => cli::new_workspace(path, name, command),
        Some(Cmd::Focus { pane }) => cli::focus(pane),
        Some(Cmd::Close { pane }) => cli::close(pane),
        Some(Cmd::Hook { agent, status, payload }) => {
            // Hooks run inline in the agent's turn: never fail, never print.
            let _ = cli::hook(&agent, status.as_deref(), payload.as_deref());
            Ok(())
        }
        Some(Cmd::Integrate { agent, uninstall }) => cli::integrate(&agent, uninstall),
        Some(Cmd::Config { cmd }) => match cmd {
            ConfigCmd::Path => {
                println!("{}", config::config_path().display());
                Ok(())
            }
            ConfigCmd::Init => cli::config_init(),
            ConfigCmd::Example => {
                print!("{}", config::EXAMPLE);
                Ok(())
            }
        },
        Some(Cmd::Worktree { branch, base, ws, command }) => cli::worktree(branch, base, ws, command),
        Some(Cmd::WorktreeRemove { ws, force }) => cli::worktree_remove(ws, force),
        Some(Cmd::KillServer { forget }) => cli::kill_server(forget),
        Some(Cmd::Mcp) => mcp::run(),
        Some(Cmd::Sync { action, name }) => sync::command(action.as_deref(), name.as_deref()),
        Some(Cmd::TestAlert) => {
            let (cfg, _) = config::Config::load_or_default();
            println!("notification: {}", if cfg.notify.desktop { "on" } else { "off (notify.desktop)" });
            if cfg.notify.desktop {
                alert::notify("claude needs you", "hydra · this is a test");
            }
            for (what, sound) in [("needs you", &cfg.notify.sound_needs), ("done", &cfg.notify.sound_done)] {
                match alert::sound_file(sound) {
                    Some(f) => {
                        println!("{what} sound: {sound} ({})", f.display());
                        alert::play(sound);
                    }
                    None => println!("{what} sound: {sound} (off or not found)"),
                }
            }
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("hydra: {e:#}");
        std::process::exit(1);
    }
}
