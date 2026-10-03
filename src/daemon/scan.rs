//! Background process-tree scan: which agent (if any) runs under each pane's shell, and
//! the name of the most recent descendant for pane titles.

use super::Ev;
use crate::config::CompiledAgent;
use crate::protocol::TermId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::sync::mpsc;

#[derive(Default)]
pub struct Shared {
    pub roots: Vec<(TermId, u32)>,
    pub agents: Vec<CompiledAgent>,
    pub interval: Duration,
}

#[derive(Debug)]
pub struct Found {
    pub term: TermId,
    pub process: String,
    pub agent: Option<String>,
    /// The shell's own working directory (accurate for shells that `chdir`, like bash).
    pub cwd: Option<std::path::PathBuf>,
    /// The agent process's working directory: where it was started, whatever the shell
    /// reported.
    pub agent_cwd: Option<std::path::PathBuf>,
}

/// Helper processes that are never what the user thinks of as "running in the pane".
const IGNORED: &[&str] = &["conhost", "openconsole", "wslhost", "cmd-shim"];

pub fn start(shared: Arc<Mutex<Shared>>, tx: mpsc::Sender<Ev>) {
    std::thread::Builder::new()
        .name("proc-scan".into())
        .spawn(move || {
            let mut sys = System::new();
            loop {
                let (roots, agents, interval) = {
                    let s = shared.lock().unwrap();
                    (s.roots.clone(), s.agents.clone(), s.interval)
                };
                if !roots.is_empty() {
                    sys.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_cwd(UpdateKind::Always),
                    );
                    let found = scan(&sys, &roots, &agents);
                    if tx.blocking_send(Ev::Scan(found)).is_err() {
                        return;
                    }
                }
                std::thread::sleep(interval.max(Duration::from_millis(200)));
            }
        })
        .expect("spawning scanner thread");
}

fn stem(name: &std::ffi::OsStr) -> String {
    let s = name.to_string_lossy().to_ascii_lowercase();
    s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
}

fn scan(sys: &System, roots: &[(TermId, u32)], agents: &[CompiledAgent]) -> Vec<Found> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, p) in sys.processes() {
        if let Some(parent) = p.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    roots
        .iter()
        .map(|&(term, root)| {
            let root = Pid::from_u32(root);
            // Breadth-first so the shallowest agent wins (an agent's own helpers are deeper).
            let mut queue = vec![(root, 0usize)];
            let mut agent = None;
            let mut agent_cwd = None;
            let mut deepest: Option<(usize, u64, String)> = None;
            let mut i = 0;
            while i < queue.len() {
                let (pid, depth) = queue[i];
                i += 1;
                if let Some(p) = sys.process(pid) {
                    let name = stem(p.name());
                    if !IGNORED.contains(&name.as_str()) {
                        if agent.is_none() && depth > 0 {
                            agent = match_agent(&name, p.cmd(), agents);
                            if agent.is_some() {
                                agent_cwd = p.cwd().map(|c| c.to_path_buf());
                            }
                        }
                        let key = (depth, p.start_time());
                        if deepest.as_ref().is_none_or(|(d, t, _)| key > (*d, *t)) {
                            deepest = Some((depth, p.start_time(), name));
                        }
                    }
                }
                for c in children.get(&pid).into_iter().flatten() {
                    if queue.len() < 512 {
                        queue.push((*c, depth + 1));
                    }
                }
            }
            let cwd = sys.process(root).and_then(|p| p.cwd()).map(|p| p.to_path_buf());
            Found { term, process: deepest.map(|d| d.2).unwrap_or_default(), agent, cwd, agent_cwd }
        })
        .collect()
}

fn match_agent(name: &str, cmd: &[std::ffi::OsString], agents: &[CompiledAgent]) -> Option<String> {
    if let Some(a) = agents.iter().find(|a| a.process.iter().any(|p| p == name)) {
        return Some(a.name.clone());
    }
    if agents.iter().all(|a| a.cmdline.is_empty()) || cmd.is_empty() {
        return None;
    }
    let line = cmd.iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>().join(" ");
    agents
        .iter()
        .find(|a| a.cmdline.iter().any(|c| line.contains(c.as_str())))
        .map(|a| a.name.clone())
}
