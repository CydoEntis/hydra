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
    d.command(0, Command::NewWorkspace { cwd: Some(dir.clone()), name: None, cmd: None }).unwrap();
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
            tabs: vec![persist::SavedTab { name: "main".into(), layout, focus: 71, panes }],
            active_tab: 0,
        }],
        active: 0,
        made_worktrees: Vec::new(),
    };
    d.restore(saved);
    assert_eq!(d.workspaces.len(), 1);
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
fn a_turn_waiting_on_background_agents_stays_working() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let hook = |status: HookStatus, event: &str, subagent: Option<Subagent>| {
        let mut m = report(t, "", 0, status, event);
        if let ClientMsg::Hook { subagent: s, .. } = &mut m {
            *s = subagent;
        }
        m
    };
    let agent = |start: bool| Some(Subagent { id: "a1".into(), kind: "general-purpose".into(), start });
    d.apply_hook(hook(HookStatus::Working, "UserPromptSubmit", None), Some(true), Vec::new());
    d.apply_hook(hook(HookStatus::Working, "SubagentStart", agent(true)), Some(true), Vec::new());
    // Claude's own turn ends ("waiting for 1 background agent"): still working.
    d.apply_hook(hook(HookStatus::Done, "Stop", None), Some(true), Vec::new());
    assert_eq!(d.terms[&t].status, Status::Working);
    // Long past the old three minutes, the agent is still at it (its own tool use): still
    // working, still listed once.
    let ago = |mins: u64| Instant::now().checked_sub(Duration::from_secs(mins * 60)).unwrap();
    d.terms.get_mut(&t).unwrap().done_held = Some(ago(20));
    d.apply_hook(hook(HookStatus::Same, "PreToolUse", agent(true)), Some(true), Vec::new());
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Working, "held while the agent runs");
    assert_eq!(d.terms[&t].subagents.len(), 1, "listed once");
    // Nothing from it for over an hour: it died without saying; the turn is done.
    let term = d.terms.get_mut(&t).unwrap();
    (term.done_held, term.subagent_seen) = (Some(ago(70)), Some(ago(70)));
    d.update_statuses();
    assert_ne!(d.terms[&t].status, Status::Working, "not held forever");
    let terms: Vec<TermId> = d.terms.keys().copied().collect();
    close(&mut d, &terms);
}

#[test]
fn an_agent_asks_you_and_gets_your_answer() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    // The agent's `hydra ask-human` is a client waiting for the reply.
    let (tx, mut asker) = mpsc::channel(8);
    d.handle(Ev::Connected(5, tx, false));
    let ask = Command::AskHuman { term: t, text: "Deploy to staging?".into(), options: vec!["Yes".into(), "No".into()] };
    assert!(!d.command(5, ask).unwrap(), "the reply waits for you");
    assert_eq!(d.terms[&t].status, Status::Blocked, "it needs you");
    let q = d.snapshot().questions;
    assert_eq!((q.len(), q[0].text.as_str()), (1, "Deploy to staging?"));
    // You answer No: the asker gets it, the pane is back at work.
    d.command(0, Command::AnswerHuman { id: q[0].id, choice: 1 }).unwrap();
    assert!(matches!(asker.try_recv(), Ok(ServerMsg::Reply(Reply::Text(a))) if a == "No"));
    assert_eq!(d.terms[&t].status, Status::Working);
    assert!(d.snapshot().questions.is_empty());
    let terms: Vec<TermId> = d.terms.keys().copied().collect();
    close(&mut d, &terms);
}

#[test]
fn a_pane_does_what_its_grants_allow() {
    let (mut d, _rx) = daemon();
    let (a, b) = (pane(&mut d), pane(&mut d));
    // An agent in pane a, with a's secret.
    let (tx, mut out) = mpsc::channel(16);
    d.handle(Ev::Connected(7, tx, false));
    let token = d.terms[&a].token.clone();
    d.handle(Ev::From(7, a, token));
    let err = |out: &mut mpsc::Receiver<ServerMsg>| matches!(out.try_recv(), Ok(ServerMsg::Error(_)));
    // Reading another pane: allowed by default.
    d.message(7, ClientMsg::Query(Query::Read { term: b }));
    assert!(matches!(out.try_recv(), Ok(ServerMsg::Reply(Reply::Text(_)))), "read is a default grant");
    // Answering b's prompt: not without respond.
    d.terms.get_mut(&b).unwrap().status = Status::Blocked;
    d.message(7, ClientMsg::Input { term: b, data: b"1".to_vec() });
    assert!(err(&mut out), "respond isn't a default grant");
    // Closing b: not without admin; and it can't grant itself more.
    d.message(7, ClientMsg::Command(Command::ClosePane { term: b }));
    assert!(err(&mut out) && d.terms.contains_key(&b), "admin isn't a default grant");
    d.message(7, ClientMsg::Command(Command::Grant { term: a, grants: Some(vec!["admin".into()]) }));
    assert!(err(&mut out), "grants are yours to give");
    // You give it admin: now it may.
    d.message(0, ClientMsg::Command(Command::Grant { term: a, grants: Some(vec!["admin".into()]) }));
    d.message(7, ClientMsg::Command(Command::ClosePane { term: b }));
    assert!(!d.terms.contains_key(&b), "closed once allowed");
    let terms: Vec<TermId> = d.terms.keys().copied().collect();
    close(&mut d, &terms);
}
