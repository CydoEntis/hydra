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
    assert_eq!(d.terms[&t].status_why, "hook: PreToolUse", "and it says what set it");
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
fn a_permission_ask_settled_without_you_stops_needing_you() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let secret = d.terms[&t].token.clone();
    d.apply_hook(report(t, &secret, 0, HookStatus::Blocked, "Notification:permission_prompt"), None, Vec::new());
    assert_eq!(d.terms[&t].status, Status::Blocked);
    let long_ago = Instant::now() - super::status::UNASKED_QUESTION_GONE_AFTER - Duration::from_secs(1);
    // The question still on screen: it still needs you.
    d.terms.get_mut(&t).unwrap().parser.process(&b"\r\n".repeat(30));
    d.terms.get_mut(&t).unwrap().parser.process(b"\r\n Do you want to proceed?\r\n \xe2\x9d\xaf 1. Yes\r\n   2. No\r\n");
    d.terms.get_mut(&t).unwrap().blocked_at = Some(long_ago);
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Blocked, "asking on screen");
    // Auto mode allowed it and the screen moved on, with no hook to say so: over.
    d.terms.get_mut(&t).unwrap().parser.process(b"\x1b[2J\x1b[H* Waiting for 1 dynamic workflow to finish\r\n");
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Done, "nothing asked on screen");
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
    let repo = std::env::temp_dir().join("seshi-spare-repo");
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
fn an_agent_typed_into_a_shell_gets_its_own_worktree() {
    let tmp = std::env::temp_dir().join(format!("seshi-agent-wt-{}", std::process::id()));
    let repo = tmp.join("shop");
    std::fs::create_dir_all(repo.join("web")).unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap().status.success(), "git {args:?}");
    git(&["init", "-q"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first"]);
    std::fs::write(repo.join("web").join("keep"), "").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "web"]);
    let made = worktrees::agent_worktree(&repo.join("web"), "{repo_parent}/{repo}-worktrees/{branch}").unwrap();
    assert!(made.ends_with("web") && made.is_dir(), "in the same subfolder of the new worktree: {}", made.display());
    let head = crate::gitfs::head(&made).unwrap();
    assert!(head.linked && crate::gitfs::WT_NAMES.contains(&head.branch.as_str()), "a linked worktree on a made-up branch: {head:?}");
    let again = worktrees::agent_worktree(&repo, "{repo_parent}/{repo}-worktrees/{branch}").unwrap();
    let other = crate::gitfs::head(&again).unwrap().branch;
    assert_ne!(other, head.branch, "the next one gets its own branch");
    // Closing them: a merged branch goes with its worktree, one with work of its own stays.
    let wt = |dir: &std::path::Path, args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success(), "git {args:?}");
    wt(&again, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "work"]);
    git::remove_worktree(&crate::gitfs::head(&made).unwrap().top, false, false).unwrap();
    git::remove_worktree(&again, false, false).unwrap();
    assert!(git::delete_if_merged(&repo, &head.branch), "nothing on it that main doesn't have: deleted");
    assert!(!git::delete_if_merged(&repo, &other), "a commit main doesn't have: kept");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn only_a_pane_shell_asks_for_an_agent_worktree() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let (tx, mut crx) = mpsc::channel(64);
    d.handle(Ev::Connected(1, tx, false));
    d.handle(Ev::Msg(1, ClientMsg::Command(Command::AgentWorktree { dir: std::env::current_dir().unwrap() })));
    assert!(matches!(crx.try_recv(), Ok(ServerMsg::Reply(Reply::Text(p))) if p.is_empty()), "not from a pane: start where it is");
    d.terms.get_mut(&t).unwrap().agent = Some("claude".into());
    let (tx, mut crx) = mpsc::channel(64);
    d.handle(Ev::Connected(2, tx, false));
    let token = d.terms[&t].token.clone();
    d.handle(Ev::From(2, t, token));
    d.handle(Ev::Msg(2, ClientMsg::Command(Command::AgentWorktree { dir: std::env::current_dir().unwrap() })));
    assert!(matches!(crx.try_recv(), Ok(ServerMsg::Reply(Reply::Text(p))) if p.is_empty()), "an agent's own agents work where it does");
    close(&mut d, &[t]);
}

#[test]
fn an_agent_stopped_by_a_limit_is_told_to_continue_when_it_resets() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let now = term::unix_now();
    d.limits.insert("claude".into(), vec![Limit { name: "5h".into(), used: 100.0, resets_at: now + 600 }, Limit { name: "week".into(), used: 40.0, resets_at: now + 90_000 }]);
    {
        let p = d.terms.get_mut(&t).unwrap();
        (p.agent, p.status) = (Some("claude".into()), Status::Idle);
        p.parser.process(&b"\r\n".repeat(30));
        p.parser.process(b"\r\n  \xe2\x8e\xbf  Claude usage limit reached. Your limit will reset at 3pm\r\n> ");
    }
    d.auto_continue();
    assert_eq!(d.terms[&t].resume_at, Some(now + 600 + 60), "a minute after the spent limit resets");
    assert!(d.snapshot().terms[&t].resume_at.is_some(), "and the window says so");
    d.auto_continue();
    assert!(d.terms[&t].pending_input.is_none(), "not before");
    d.terms.get_mut(&t).unwrap().resume_at = Some(now - 1);
    d.auto_continue();
    assert_eq!(d.terms[&t].pending_input.as_ref().map(|(b, _)| b.as_slice()), Some(&b"continue\r"[..]), "then it's told to go on");
    d.terms.get_mut(&t).unwrap().pending_input = None;
    d.auto_continue();
    assert!(d.terms[&t].resume_at.is_none(), "the old line still on screen doesn't start another wait");
    // Off in Settings: nothing.
    d.cfg.auto_continue = false;
    d.terms.get_mut(&t).unwrap().continued = None;
    d.auto_continue();
    assert!(d.terms[&t].resume_at.is_none());
    close(&mut d, &[t]);
}

#[test]
fn claude_reports_what_its_session_used() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let token = d.terms[&t].token.clone();
    d.terms.get_mut(&t).unwrap().agent = Some("claude".into());
    let usage = |cost: f64| Usage { context: Some(30.0), cost: Some(cost) };
    let limits = vec![Limit { name: "5h".into(), used: 23.0, resets_at: term::unix_now() + 100 }];
    d.handle(Ev::Msg(1, ClientMsg::Usage { term: t, token: "wrong".into(), usage: usage(9.0), limits: Vec::new() }));
    assert_eq!(d.terms[&t].usage, Usage::default(), "only with the pane's secret");
    d.handle(Ev::Msg(1, ClientMsg::Usage { term: t, token: token.clone(), usage: usage(1.0), limits: limits.clone() }));
    d.handle(Ev::Msg(1, ClientMsg::Usage { term: t, token: token.clone(), usage: usage(1.5), limits }));
    // /clear: a new session, counting from zero again.
    d.handle(Ev::Msg(1, ClientMsg::Usage { term: t, token, usage: usage(0.25), limits: Vec::new() }));
    let snap = d.snapshot();
    assert_eq!(snap.terms[&t].usage.context, Some(30.0));
    assert!((snap.spent_today - 1.75).abs() < 1e-9, "1.50 then 0.25 after the clear: {}", snap.spent_today);
    assert_eq!(snap.limits[0].0, "claude");
    close(&mut d, &[t]);
}



#[test]
fn a_panes_history_survives_the_server_restarting() {
    let (mut d, _rx) = daemon();
    d.cfg.restore.enabled = true;
    d.command(0, Command::NewWorkspace { cwd: Some(std::env::temp_dir()), name: None, cmd: None }).unwrap();
    let t = *d.terms.keys().next().unwrap();
    d.terms.get_mut(&t).unwrap().ring_push_for_test(b"line from before the restart\r\n");
    let saved = d.saved();
    d.save_outputs(true);
    // A new server: the pane comes back, with what it showed above what it shows now.
    let (mut d2, _rx2) = daemon();
    d2.cfg.restore.enabled = true;
    d2.restore(saved);
    let new = *d2.terms.keys().next().unwrap();
    let replay = String::from_utf8_lossy(&d2.terms[&new].replay()).into_owned();
    assert!(replay.contains("line from before the restart") && replay.contains("seshi restarted here"), "{replay}");
    let mut p = vt100::Parser::new(24, 80, 1000);
    p.process(&d2.terms[&new].replay());
    p.screen_mut().set_scrollback(usize::MAX);
    assert!(p.screen().scrollback() >= 24, "the old screen went up into history: {}", p.screen().scrollback());
    assert!(persist::take_output(t).is_none(), "taken once");
    close(&mut d, &[t]);
    close(&mut d2, &[new]);
}

#[test]
fn closing_the_last_pane_keeps_an_open_window() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let (tx, _crx) = mpsc::channel(64);
    d.handle(Ev::Connected(1, tx, true));
    d.close_term(t);
    assert!(!d.should_exit(), "a window is still showing seshi");
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
    // The agent's `seshi ask-human` is a client waiting for the reply.
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

#[test]
fn a_popup_floats_in_no_workspace_and_goes_when_done() {
    let (mut d, mut rx) = daemon();
    let cmd = if cfg!(windows) { "cmd /c exit" } else { "true" };
    d.command(0, Command::Popup { cmd: cmd.into(), cwd: Some(std::env::temp_dir()) }).unwrap();
    let (&id, t) = d.terms.iter().next().expect("the popup's pane");
    assert!(t.popup && d.workspaces.is_empty(), "floating, in no workspace");
    assert!(d.snapshot().terms[&id].popup);
    // Its command exits: the pane goes.
    let until = Instant::now() + Duration::from_secs(10);
    while d.terms.contains_key(&id) && Instant::now() < until {
        if let Ok(ev) = rx.try_recv() {
            d.handle(ev);
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(!d.terms.contains_key(&id), "gone when its command is done");
}

#[test]
fn an_agent_stays_in_the_folder_it_started_in() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    let secret = d.terms[&t].token.clone();
    let base = std::env::temp_dir().join(format!("seshi-pin-{}", std::process::id()));
    let (home, other) = (base.join("home"), base.join("other"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let at = |cwd: &std::path::Path, status: HookStatus, event: &str| match report(t, &secret, 0, status, event) {
        ClientMsg::Hook { term, agent, status, session, prompt, said, subagent, event, pid, token, transcript, model, name, .. } => {
            ClientMsg::Hook { term, agent, status, session, cwd: Some(cwd.to_path_buf()), prompt, said, subagent, event, pid, token, transcript, model, name }
        }
        _ => unreachable!(),
    };
    // Its first report places it.
    d.apply_hook(at(&home, HookStatus::Working, "UserPromptSubmit"), Some(true), Vec::new());
    assert_eq!(d.terms[&t].cwd, home);
    // Working in another folder doesn't move it.
    d.apply_hook(at(&other, HookStatus::Working, "PreToolUse"), Some(true), Vec::new());
    assert_eq!(d.terms[&t].cwd, home, "its row stays in the group it started in");
    close(&mut d, &[t]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn an_agent_read_from_its_screen_isnt_done_while_its_screen_still_moves() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    {
        // Codex with its notify pointed elsewhere: no hooks, only its screen.
        let p = d.terms.get_mut(&t).unwrap();
        (p.agent, p.hooked, p.status) = (Some("codex".into()), false, Status::Working);
        p.parser.process(b"\xe2\x80\xa2 Working (3s \xe2\x80\xa2 esc to interrupt)\r\n");
        // A long block printed under it pushes the working line out of the rows looked at.
        for i in 0..40 {
            p.parser.process(format!("+ added line {i}\r\n").as_bytes());
        }
        p.last_output = Instant::now();
    }
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Working, "still printing: not done, and no alert");
    // The screen goes still: now it's done.
    d.terms.get_mut(&t).unwrap().last_output = Instant::now() - Duration::from_secs(10);
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Done);
    close(&mut d, &[t]);
}

#[test]
fn codex_shows_working_from_its_screen_after_a_turn() {
    let (mut d, _rx) = daemon();
    let t = pane(&mut d);
    {
        let p = d.terms.get_mut(&t).unwrap();
        // Codex finished a turn (its notify hook), ten seconds ago.
        (p.agent, p.hooked, p.status) = (Some("codex".into()), true, Status::Done);
        p.status_since = term::unix_now() - 10;
        // You typed the next prompt, and it's working on it.
        p.last_input = Instant::now();
        // At the bottom of the screen, where it is in Codex.
        p.parser.process(&b"\r\n".repeat(30));
        p.parser.process(b"\xe2\x80\xa2 Working (3s \xe2\x80\xa2 esc to interrupt)\r\n\r\n> Ask Codex to do anything");
    }
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Working, "seen on its screen: its hooks never say a turn started");
    // Without anything typed since the turn ended, the old line on screen doesn't count.
    {
        let p = d.terms.get_mut(&t).unwrap();
        p.status = Status::Done;
        p.status_since = term::unix_now();
        p.last_input = Instant::now() - Duration::from_secs(5);
    }
    d.update_statuses();
    assert_eq!(d.terms[&t].status, Status::Done);
    close(&mut d, &[t]);
}

#[tokio::test]
async fn a_phone_alert_goes_once_after_the_wait() {
    let (mut d, _rx) = daemon();
    // A server nothing listens on: the alert is tried and fails quietly.
    d.cfg.notify.phone_topic = "seshi-test".into();
    d.cfg.notify.phone_server = "http://127.0.0.1:9".into();
    d.cfg.notify.phone_after = 60;
    let t = pane(&mut d);
    let now = term::unix_now();
    let tm = d.terms.get_mut(&t).unwrap();
    tm.agent = Some("claude".into());
    tm.status = Status::Blocked;
    tm.status_since = now - 10;
    d.phone_alerts();
    assert_eq!(d.terms[&t].phoned, 0, "still inside the wait: you may be at your desk");
    d.terms.get_mut(&t).unwrap().status_since = now - 120;
    d.phone_alerts();
    assert_eq!(d.terms[&t].phoned, now - 120, "waited long enough: it goes");
    d.terms.get_mut(&t).unwrap().status = Status::Done;
    d.phone_alerts();
    assert_eq!(d.terms[&t].phoned, now - 120, "finished: only when asked for (phone_done)");
    close(&mut d, &[t]);
}
