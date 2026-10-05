//! The daemon's own logic, driven through a real daemon with real panes (plain shells).

use super::*;
use std::time::Duration;

fn daemon() -> (Daemon, mpsc::Receiver<Ev>) {
    let (tx, rx) = mpsc::channel(4096);
    (Daemon::new(Config::default(), tx), rx)
}

fn pane(d: &mut Daemon) -> TermId {
    d.spawn(None, &std::env::temp_dir(), 80, 24).expect("a shell pane")
}

fn report(term: TermId, token: &str, pid: u32, status: HookStatus, event: &str) -> ClientMsg {
    ClientMsg::Hook {
        term,
        agent: "claude".into(),
        status,
        session: None,
        cwd: None,
        prompt: None,
        said: None,
        subagent: None,
        event: event.into(),
        pid,
        token: token.into(),
        transcript: None,
        model: None,
        name: None,
    }
}

/// Apply checked status reports as the daemon loop would, until `n` have come back (or
/// `wait` passes). Returns how many did.
fn settle_within(d: &mut Daemon, rx: &mut mpsc::Receiver<Ev>, n: usize, wait: Duration) -> usize {
    let until = Instant::now() + wait;
    let mut got = 0;
    while got < n && Instant::now() < until {
        match rx.try_recv() {
            Ok(ev @ Ev::HookChecked { .. }) => {
                d.handle(ev);
                got += 1;
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    got
}

fn settle(d: &mut Daemon, rx: &mut mpsc::Receiver<Ev>, n: usize) -> usize {
    settle_within(d, rx, n, Duration::from_secs(10))
}

fn close(d: &mut Daemon, terms: &[TermId]) {
    for t in terms {
        d.close_term(*t);
    }
}

#[test]
fn status_reports_need_the_pane_secret() {
    let (mut d, mut rx) = daemon();
    let t = pane(&mut d);
    let secret = d.terms[&t].token.clone();
    d.message(0, report(t, "not-the-secret", 0, HookStatus::Blocked, "PreToolUse"));
    d.message(0, report(t, "", 0, HookStatus::Blocked, "PreToolUse"));
    assert_eq!(settle_within(&mut d, &mut rx, 1, Duration::from_secs(1)), 0, "a report without the secret is never even checked");
    assert_eq!(d.terms[&t].status, Status::None);
    d.message(0, report(t, &secret, 0, HookStatus::Blocked, "PreToolUse"));
    assert_eq!(settle(&mut d, &mut rx, 1), 1);
    assert_eq!(d.terms[&t].status, Status::Blocked, "with the secret it counts");
    close(&mut d, &[t]);
}

#[test]
fn a_report_traced_outside_the_pane_is_dropped() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let secret = d.terms[&t].token.clone();
    // The hook thread found its process chain doesn't lead to the pane (a desktop app that
    // inherited the pane's environment, say): the report is dropped, secret or not.
    d.apply_hook(report(t, &secret, 4242, HookStatus::Blocked, "PreToolUse"), Some(false), Vec::new());
    assert_eq!(d.terms[&t].status, Status::None);
    // Traced to the pane: it counts, and the chain is remembered for next time.
    d.apply_hook(report(t, &secret, 4242, HookStatus::Blocked, "PreToolUse"), Some(true), vec![4242, 4243]);
    assert_eq!(d.terms[&t].status, Status::Blocked);
    assert!(d.terms[&t].trusted.contains(&4243));
    close(&mut d, &[t]);
}

#[test]
fn status_reports_apply_in_the_order_they_came() {
    let (mut d, mut rx) = daemon();
    let t = pane(&mut d);
    let secret = d.terms[&t].token.clone();
    d.message(0, report(t, &secret, 0, HookStatus::Working, "UserPromptSubmit"));
    d.message(0, report(t, &secret, 0, HookStatus::Blocked, "PreToolUse"));
    d.message(0, report(t, &secret, 0, HookStatus::Done, "Stop"));
    assert_eq!(settle(&mut d, &mut rx, 3), 3);
    assert_eq!(d.terms[&t].status, Status::Done, "the last report wins: done (no one is looking)");
    close(&mut d, &[t]);
}

#[test]
fn detaching_and_closing_prune_tabs_and_workspaces() {
    let (mut d, _rx) = daemon();
    let dir = std::env::temp_dir();
    d.command(0, Command::NewWorkspace { cwd: Some(dir.clone()), name: None, cmd: None, home: crate::protocol::Home::Auto }).unwrap();
    assert_eq!(d.workspaces.len(), 1);
    let first = d.workspaces[0].tabs[0].focus;
    d.command(0, Command::Split { term: first, dir: crate::layout::Dir::Right, cmd: None, cwd: None }).unwrap();
    let leaves = d.workspaces[0].tabs[0].layout.leaves();
    assert_eq!(leaves.len(), 2);
    let second = leaves.into_iter().find(|t| *t != first).unwrap();
    // Out of the tab, still running.
    d.detach(first);
    assert!(d.terms.contains_key(&first), "detach keeps the terminal");
    assert_eq!(d.workspaces[0].tabs[0].layout.leaves(), vec![second]);
    assert_eq!(d.workspaces[0].tabs[0].focus, second, "focus moves to what's left");
    // The last pane goes: the tab and the workspace go with it.
    d.remove_term(second);
    assert!(d.workspaces.is_empty(), "an emptied workspace disappears");
    assert!(!d.terms.contains_key(&second));
    close(&mut d, &[first]);
}

#[test]
fn a_spare_fits_only_its_repo_and_its_agent() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let repo = std::env::temp_dir().join("hydra-spare-repo");
    d.cfg.worktree.prewarm = "claude".into();
    d.spare = Some((repo.clone(), repo.join("wt"), t));
    assert_eq!(d.spare_fits(&repo, "claude").as_deref(), Some(""), "the agent alone");
    let with_task = format!("claude {}", d.cfg.quote_for_shell("fix the flaky test"));
    assert_eq!(d.spare_fits(&repo, &with_task).as_deref(), Some("fix the flaky test"), "the agent and a task");
    assert_eq!(d.spare_fits(&repo, "codex"), None, "another agent");
    assert_eq!(d.spare_fits(&repo, "claude --model opus"), None, "other options");
    assert_eq!(d.spare_fits(&std::env::temp_dir().join("other"), "claude"), None, "another repo");
    d.cfg.worktree.prewarm.clear();
    assert_eq!(d.spare_fits(&repo, "claude"), None, "prewarming off");
    d.spare = None;
    close(&mut d, &[t]);
}

#[test]
fn restore_gives_panes_new_ids_and_keeps_their_layout() {
    let (mut d, _rx) = daemon();
    let dir = std::env::temp_dir();
    let mut layout = Node::Leaf(70);
    layout.split(70, crate::layout::Dir::Right, 71);
    let mut panes = std::collections::BTreeMap::new();
    panes.insert(70, persist::SavedPane { cwd: Some(dir.clone()), name: "left one".into(), ..Default::default() });
    panes.insert(71, persist::SavedPane { cwd: Some(dir.clone()), model: "opus".into(), ..Default::default() });
    let saved = persist::Saved {
        workspaces: vec![persist::SavedWs {
            name: "api".into(),
            cwd: dir.clone(),
            worktree: false,
            color: None,
            group: None,
            home: Home::Loose,
            tabs: vec![persist::SavedTab { name: "main".into(), layout, focus: 71, panes }],
            active_tab: 0,
        }],
        active: 0,
        made_worktrees: Vec::new(),
    };
    d.restore(saved);
    assert_eq!(d.workspaces.len(), 1);
    assert_eq!(d.workspaces[0].home, Home::Loose, "outside projects stays outside after a restart");
    let tab = &d.workspaces[0].tabs[0];
    let leaves = tab.layout.leaves();
    assert_eq!(leaves.len(), 2, "both panes came back, side by side");
    assert!(leaves.iter().all(|t| d.terms.contains_key(t) && *t != 70 && *t != 71), "with this run's ids");
    let (left, right) = (leaves[0], leaves[1]);
    assert_eq!(tab.focus, right, "focus follows the pane it was on");
    assert_eq!(d.terms[&left].first_prompt, "left one");
    assert_eq!(d.terms[&right].model, "opus");
    close(&mut d, &leaves);
}

#[test]
fn worktree_names_that_look_like_options_are_refused() {
    let dir = std::env::temp_dir();
    for bad in ["", "-x", "a b", "--force"] {
        assert!(git::create_worktree(&dir, bad, None, "{repo_parent}/{repo}-worktrees/{branch}").is_err(), "branch {bad:?}");
    }
}

#[test]
fn closing_the_last_pane_keeps_an_open_window() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let (tx, _crx) = mpsc::channel(64);
    d.handle(Ev::Connected(1, tx, true));
    d.close_term(t);
    assert!(!d.should_exit(), "a window is still showing hydra");
    d.handle(Ev::Disconnected(1));
    assert!(d.should_exit(), "everything closed and nobody's looking: exit");
}

#[test]
fn a_new_session_remembers_where_it_belongs() {
    let (mut d, _rx) = daemon();
    let dir = std::env::temp_dir();
    d.command(0, Command::NewWorkspace { cwd: Some(dir.clone()), name: None, cmd: None, home: Home::Auto }).unwrap();
    d.command(0, Command::NewWorkspace { cwd: Some(dir.clone()), name: None, cmd: None, home: Home::Loose }).unwrap();
    assert!(matches!(&d.workspaces[0].home, Home::Project(p) if clean_path(p.clone()) == clean_path(dir.clone())), "the project of the folder it started in: {:?}", d.workspaces[0].home);
    assert_eq!(d.workspaces[1].home, Home::Loose, "a quick shell stays outside projects");
    let terms: Vec<TermId> = d.terms.keys().copied().collect();
    close(&mut d, &terms);
}
