//! Background process-tree scan: which agent (if any) runs under each pane's shell, and
//! the name of the most recent descendant for pane titles.

use super::Ev;
use crate::config::CompiledAgent;
use crate::protocol::TermId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Is `pid` inside the process tree under `ancestor`? Walks up the parents.
/// `Some(true)` with the pids passed (to trust next time), `Some(false)` when the walk reached
/// the top without meeting it, `None` when it can't tell (a process already exited).
pub fn descends_from(pid: u32, ancestor: u32, known: &std::collections::HashSet<u32>) -> (Option<bool>, Vec<u32>) {
    let mut sys = System::new();
    let mut chain = Vec::new();
    let mut cur = pid;
    for _ in 0..32 {
        if cur == ancestor || known.contains(&cur) {
            return (Some(true), chain);
        }
        chain.push(cur);
        let p = Pid::from_u32(cur);
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing());
        let Some(proc_) = sys.process(p) else { return (None, chain) };
        match proc_.parent() {
            Some(parent) if parent.as_u32() != cur && parent.as_u32() != 0 => cur = parent.as_u32(),
            _ => return (Some(false), chain),
        }
    }
    (None, chain)
}
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
    /// Memory used by everything running in the pane (bytes).
    pub mem: u64,
    /// The machine an ssh (or mosh, …) client in the pane is connected to.
    pub remote: Option<String>,
}

/// Programs that put the pane on another machine.
const REMOTE_CLIENTS: &[&str] = &["ssh", "mosh", "mosh-client", "autossh", "et"];

/// The host an ssh-like command line connects to: its first argument that isn't an option
/// (or an option's value), without `user@` or `ssh://`.
pub fn remote_host(args: &[String]) -> Option<String> {
    // ssh options that take a value.
    const WITH_VALUE: &[&str] = &["-b", "-B", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o", "-p", "-Q", "-R", "-S", "-W", "-w", "--ssh", "--port"];
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--" {
            return it.next().and_then(|h| clean_host(h));
        }
        if a.starts_with('-') {
            if WITH_VALUE.contains(&a.as_str()) {
                it.next();
            }
            continue;
        }
        return clean_host(a);
    }
    None
}

fn clean_host(a: &str) -> Option<String> {
    let a = a.strip_prefix("ssh://").unwrap_or(a);
    let host = a.rsplit_once('@').map_or(a, |(_, h)| h);
    let host = host.split([':', '/']).next().unwrap_or(host);
    (!host.is_empty()).then(|| host.to_string())
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
                        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_cwd(UpdateKind::Always).with_memory(),
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
            let mut mem = 0u64;
            let mut remote = None;
            let mut i = 0;
            while i < queue.len() {
                let (pid, depth) = queue[i];
                i += 1;
                if let Some(p) = sys.process(pid) {
                    mem += p.memory();
                    let name = stem(p.name());
                    if !IGNORED.contains(&name.as_str()) {
                        if remote.is_none() && depth > 0 && REMOTE_CLIENTS.contains(&name.as_str()) {
                            let args: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
                            remote = remote_host(&args);
                        }
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
            Found { term, process: deepest.map(|d| d.2).unwrap_or_default(), agent, cwd, agent_cwd, mem, remote }
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

#[cfg(test)]
mod walk_tests {
    #[test]
    fn walks_several_levels_up() {
        let me = std::process::id();
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let mut chain = vec![me];
        let mut cur = sysinfo::Pid::from_u32(me);
        while let Some(p) = sys.process(cur).and_then(|p| p.parent()) {
            chain.push(p.as_u32());
            cur = p;
            if chain.len() > 6 {
                break;
            }
        }
        eprintln!("ancestors: {chain:?}");
        let up = chain[chain.len().min(4) - 1];
        let r = super::descends_from(me, up, &Default::default());
        eprintln!("descends_from(me, {up}) = {:?}", r.0);
        assert_eq!(r.0, Some(true));
    }
}

#[cfg(test)]
mod remote_tests {
    use super::remote_host;

    fn host(line: &str) -> Option<String> {
        remote_host(&line.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn the_machine_an_ssh_command_reaches() {
        assert_eq!(host("ssh build-box").as_deref(), Some("build-box"));
        assert_eq!(host("ssh vox@10.0.0.4").as_deref(), Some("10.0.0.4"));
        assert_eq!(host("ssh -p 2222 -i ~/.ssh/key me@pi.local uptime").as_deref(), Some("pi.local"), "options and their values are skipped");
        assert_eq!(host("ssh -A -o StrictHostKeyChecking=no gpu").as_deref(), Some("gpu"));
        assert_eq!(host("ssh ssh://me@host:22").as_deref(), Some("host"));
        assert_eq!(host("mosh --ssh=ssh -p 22 me@remote").as_deref(), Some("remote"));
        assert_eq!(host("ssh -V"), None);
    }
}
