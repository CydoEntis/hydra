//! The client's tests: rendering through TestBackend, keys and clicks.

use super::*;

mod basics {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn blend_mixes_rgb() {
        let c = render::blend(Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 50), 0.1);
        assert_eq!(c, Color::Rgb(20, 10, 5));
        assert_eq!(render::blend(Color::Indexed(4), Color::Rgb(1, 2, 3), 0.1), Color::Indexed(4));
    }

}

mod design_tests {
    use super::*;
    use crate::layout::{Dir, Node};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn entry(path: &str, branch: &str, main: bool) -> WorktreeEntry {
        WorktreeEntry { path: PathBuf::from(path), branch: branch.into(), main, repo: "shop-api".into() }
    }

    fn term(id: TermId, agent: Option<&str>, status: Status, cwd: &str) -> TermInfo {
        TermInfo {
            name: String::new(),
            label: String::new(),
            model: String::new(),
            dev: None,
            mem: 0,
            bell: false,
            id,
            cols: 80,
            rows: 20,
            title: String::new(),
            process: if agent.is_some() { "node".into() } else { "pwsh".into() },
            agent: agent.map(String::from),
            status,
            cwd: PathBuf::from(cwd),
            summary: String::new(),
            said: String::new(),
            branch: None,
            linked: false,
            subagents: Vec::new(),
            root: None,
            top: None,
            since: 0,
            asleep: false,
            win32_input: false,
        }
    }

    /// The main screen with a few projects and agents, rendered as text.
    pub(super) fn render_with(w: u16, h: u16) -> (String, App) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (bg_tx, _bg_rx) = mpsc::unbounded_channel();
        let cfg = Config::default();
        let mut app = App::new(cfg, tx, None, bg_tx);
        app.splash = false;
        let mut layout = Node::Leaf(1);
        layout.split(1, Dir::Right, 2);
        layout.split(2, Dir::Down, 3);
        // A project folder in this platform's own style (tests run on Windows and Linux).
        let base = std::path::Path::new(if cfg!(windows) { r"C:\code" } else { "/code" });
        let root_buf = base.join("shop-api");
        let rate_buf = root_buf.join(".wt").join("rate");
        let orders_buf = root_buf.join(".wt").join("orders");
        let root = root_buf.to_str().unwrap();
        app.snap.workspaces.push(WorkspaceInfo {
            id: 10,
            name: "shop-api".into(),
            cwd: PathBuf::from(root),
            tabs: vec![TabInfo { id: 11, name: String::new(), layout, focus: 1 }],
            active_tab: 11,
            git: Some(GitInfo {
                repo: "shop-api".into(),
                branch: "main".into(),
                dirty: 2,
                linked: false,
                root: PathBuf::from(root),
                worktrees: vec![
                    entry(root, "main", true),
                    entry(rate_buf.to_str().unwrap(), "rate-limit", false),
                    entry(orders_buf.to_str().unwrap(), "orders-migration", false),
                ],
                ahead: 0,
            }),
            worktree: false,
            color: 0,
            is_new: false,
            group: None,
        });
        app.snap.active_ws = Some(10);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let rate = rate_buf.to_str().unwrap();
        let mut claude = term(1, Some("claude"), Status::Blocked, root);
        claude.branch = Some("main".into());
        claude.subagents = vec!["Explore".into()];
        claude.summary = "Fix flaky checkout test".into();
        claude.root = Some(PathBuf::from(root));
        claude.top = Some(PathBuf::from(root));
        claude.since = now - 180;
        let mut codex = term(2, Some("codex"), Status::Working, rate);
        codex.branch = Some("rate-limit".into());
        codex.linked = true;
        codex.summary = "Rate limit /login".into();
        codex.root = Some(PathBuf::from(root));
        codex.top = Some(PathBuf::from(rate));
        codex.since = now - 120;
        let mut shell = term(3, None, Status::None, root);
        shell.root = Some(PathBuf::from(root));
        shell.top = Some(PathBuf::from(root));
        shell.branch = Some("main".into());
        app.snap.terms.insert(1, claude);
        app.snap.terms.insert(2, codex);
        app.snap.terms.insert(3, shell);
        app.hy_sync();
        let mut p = vt100::Parser::new(20, 80, 0);
        p.process(b"> fix the flaky checkout test\r\n\r\nRun npm test -- checkout?\r\n\x1b[1m\xe2\x9d\xaf 1. Yes\x1b[0m\r\n  2. Yes, and always allow\r\n  3. No");
        app.parsers.insert(1, p);
        app.parsers.insert(2, vt100::Parser::new(20, 80, 0));
        app.parsers.insert(3, vt100::Parser::new(20, 80, 0));
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            let mut line = String::new();
            for x in 0..w {
                line.push_str(buf[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        (out, app)
    }

}

mod hydra_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        app.hy_fresh();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render::draw(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string() + "\n").collect()
    }

    fn show(s: &str) {
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{s}");
        }
    }

    #[test]
    fn main_screen_follows_the_handoff() {
        let (text, mut app) = super::design_tests::render_with(160, 45);
        show(&text);
        let lines: Vec<&str> = text.lines().collect();
        assert!(!text.contains(">_ hydra") && !text.contains("Ctrl+Space"), "no logo, no keys buttons");
        let bottom = lines.iter().rev().find(|l| !l.trim().is_empty()).unwrap();
        assert!(bottom.contains("● 1 needs you") && !bottom.contains("›"), "the bottom bar: what needs you, no path (the pane's title has it): {bottom}");
        assert!(lines[1].contains("PROJECTS") && lines[1].contains("+ open"), "the sidebar's header: {}", lines[1]);
        assert!(text.contains("t new") && text.contains("g go to") && text.contains(", settings"), "quiet hints at the bottom of the sidebar");
        // Projects and their sessions, nothing in between.
        assert!(text.contains("▾ ▌shop-api") && !text.contains("BRANCHES") && !text.contains("WORKTREES"));
        assert!(!text.contains("main folder"), "no 'main folder' wording");
        assert!(!text.contains("+ open a project"), "opening a project is in the header now");
        // Rows: agent and state, then what it's on (or its question) underneath.
        assert!(text.contains("● ✻ claude") && text.contains("main · 3m"), "status, then the agent's icon and name; branch · age on the right");
        assert!(text.contains("Run npm test -- checkout?"), "the question under the agent");
        assert!(text.contains("↳ Explore"), "subagents under their agent");
        assert!(text.contains("⠋ ◇ rate") && text.contains("rate-limit · 2m"), "a worktree's session is named after it");
        assert!(!text.contains("Rate limit /login"), "under a session only its question, as in the redesign");
        assert!(text.contains("›   shell") && !text.contains("shell 2"), "a shell in the project's folder is just 'shell', no age");
        assert!(!text.contains("session"), "no 'session' wording on screen");
        for part in ["● answer", " Yes 1 ", " Always 2 ", " No 3"] {
            assert!(text.contains(part), "the answer bar has {part:?}");
        }
        assert!(!text.contains("click or press T"), "no footer under the pane");
        assert!(lines[0].contains("✻ claude  Fix flaky checkout test · shop-api · main") && lines[0].contains("● needs you"), "every pane has a title bar, at the top: {}", lines[0]);
        assert!(lines[2].contains("> fix the flaky checkout test"), "a blank row under the bar, then the output");
        // Overlays are centred over a dimmed screen.
        for (mode, needle) in [
            (Mode::Jump { sel: 0 }, "NEEDS YOU"),
            (Mode::HyPane(hydra::NewPaneHy::new(0, false)), "claude gets its own new worktree in shop-api"),
            (Mode::HyPane(hydra::NewPaneHy { place: Some(1), ..hydra::NewPaneHy::new(0, false) }), "Switches shop-api to a new branch"),
            (Mode::Help { scroll: 0 }, "search code"),
            (Mode::Talk { term: 1, input: String::new() }, "Write to claude…"),
        ] {
            app.mode = mode;
            let o = draw(&mut app, 160, 45);
            show(&o);
            assert!(o.contains(needle), "overlay shows {needle}");
        }
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 1, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" General ") && o.contains(" Sessions ") && o.contains("Sort sidebar by attention") && o.contains(" on "));
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 2, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("● Default") && o.contains("○ Tokyo Night") && o.contains("needs you, done, error"), "a row per theme, with what the swatches are");
        // The splash: the braille hydra, the wordmark, what happened, buttons.
        app.mode = Mode::Normal;
        app.splash = true;
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("⣿") && o.contains("██████"), "art and wordmark");
        assert!(o.contains("while you were away") && o.contains("1 need you") && o.contains("Resume where you left off") && o.contains("Open a folder"));
        // Small windows drop the art but keep the rest.
        let o = draw(&mut app, 100, 30);
        assert!(!o.contains("⣿") && o.contains("Open a folder"));
    }

    #[test]
    fn pull_request_view() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let info = pr::parse_info(
            r#"{"number":412,"title":"Rate limit /login","url":"u","state":"OPEN","author":{"login":"cody"},"headRefName":"rate-limit","baseRefName":"main",
            "body":"Token bucket, 5 a minute.","additions":42,"deletions":7,"changedFiles":3,
            "statusCheckRollup":[{"conclusion":"SUCCESS","name":"lint"},{"conclusion":"FAILURE","name":"test"}],
            "reviews":[{"author":{"login":"sam"},"state":"CHANGES_REQUESTED","body":"use X-Forwarded-For"}],"comments":[]}"#,
        )
        .unwrap();
        app.view = Some(View::Pr(Box::new(pr::PrView { dir: PathBuf::from("."), which: "412".into(), info: Some(Ok(info)), diff: None, tab: 0, scroll: 0 })));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("#412  Rate limit /login") && o.contains("rate-limit → main") && o.contains("+42 −7"));
        assert!(o.contains("CHECKS  1 failing") && o.contains("✕ test") && o.contains("sam asked for changes"));
        assert!(o.contains("Ask the agent to fix it f") && o.contains("Open in browser o"));
        // It opens as a tool window.
        assert!(o.contains("Esc"));
        // And a PR tag on the branch that has one.
        app.view = None;
        app.hy.prs.insert(
            app.hy_model()[0].key.clone(),
            vec![pr::PrBrief { number: 412, title: "Rate limit".into(), branch: "rate-limit".into(), checks: pr::Checks::Fail, review: pr::Review::Changes, url: String::new() }],
        );
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("#412 ✕±"), "PR tag on the worktree's session");
        app.mode = Mode::Jump { sel: 0 };
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("PULL REQUESTS") && o.contains("checks failing"), "failing PRs in Jump");
    }

    #[test]
    fn ship_confirm_says_what_happens() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let task = tasks::TaskRow {
            ws: 10,
            name: "rate-limit".into(),
            branch: "rate-limit".into(),
            base: "main".into(),
            stage: tasks::Stage::Ready,
            summary: "Rate limit /login".into(),
            dirty: 0,
            ahead: 0,
            agent: Some(2),
            dir: PathBuf::from("."),
            root: PathBuf::from("."),
        };
        app.mode = Mode::Ship(Box::new(ShipAsk { task: task.clone(), changed: 3, pr: None }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Ship rate-limit") && o.contains("commit 3 changed files as \"Rate limit /login\""));
        assert!(o.contains("push rate-limit") && o.contains("open a pull request into main") && o.contains("Ship Enter"));
        app.mode = Mode::Ship(Box::new(ShipAsk { task, changed: 0, pr: Some("412".into()) }));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("nothing new to commit") && o.contains("update pull request #412"));
    }

    #[test]
    fn ideas_tickets_and_races() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let root = app.hy_model()[0].path.clone();
        let ideas = vec![
            work::Idea { text: "dark mode for the dashboard".into(), project: Some(root.clone()), at: 0 },
            work::Idea { text: "a CLI for exports".into(), project: None, at: 0 },
        ];
        app.mode = Mode::Ideas(Box::new(work::IdeasView { ideas, input: String::new(), sel: 0, tag: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Ideas") && o.contains("for ▌shop-api") && o.contains("SHOP-API") && o.contains("✦ dark mode for the dashboard"));
        assert!(o.contains("ANY PROJECT") && o.contains("start claude on it"));

        let t = work::Ticket { key: "ENG-123".into(), title: "Checkout fails on Safari".into(), url: "u".into(), state: "Todo".into(), meta: "High · ENG".into(), body: "Steps to reproduce".into() };
        app.mode = Mode::Tickets(Box::new(work::TicketsView {
            dir: root.clone(),
            tabs: vec![("github".into(), "GitHub issues".into()), ("linear".into(), "Linear".into()), ("plane".into(), "Plane".into())],
            tab: 1,
            lists: vec![None, Some(Ok(vec![t])), Some(Err("Set PLANE_API_KEY".into()))],
            query: String::new(),
            sel: 0,
        }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" GitHub issues ") && o.contains(" Linear ") && o.contains(" Plane"));
        assert!(o.contains("ENG-123") && o.contains("Checkout fails on Safari") && o.contains("Todo · High · ENG") && o.contains("claude on it, own worktree"));

        app.mode = Mode::RaceNew(Box::new(work::RaceNew { text: "add rate limiting".into(), picked: vec![true, true, false], row: 0, cur: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Race agents") && o.contains("✓ claude") && o.contains("✓ codex") && o.contains("2 agents, each in its own worktree of shop-api"));

        app.hy.saved.races.push(work::Race {
            id: 7,
            project: root.clone(),
            prompt: "add rate limiting".into(),
            base: "main".into(),
            entries: vec![("claude".into(), "race-add-rate-limiting-claude".into()), ("codex".into(), "rate-limit".into())],
        });
        app.mode = Mode::Normal;
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("⚑ race add rate limiting") && o.contains("⚑ rate"), "race line and race worktree in the sidebar");
        app.mode = Mode::Race(Box::new(work::RaceView { id: 7, sel: 1, stats: vec![None, Some("+42 −7 · 3 files".into())], confirm: true }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Race · add rate limiting") && o.contains("+42 −7 · 3 files") && o.contains("not running"));
        assert!(o.contains("Keep codex's rate-limit and delete the other 1?"));
    }

    #[test]
    fn map_view() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.view = Some(View::Map(Box::new(hydra::MapView { proj: None, sel: 1 })));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("map  shop-api") && o.contains("▌shop-api  ●1 ⠋1"), "the project at the top");
        assert!(o.contains("⎇ main") && o.contains("⑂ rate") && o.contains("⑂ orders"), "a box per folder");
        assert!(o.contains("● claude") && o.contains("needs you 3m") && o.contains("⠋ codex") && o.contains("working 2m"));
        assert!(o.contains("┴") && (o.contains("┬") || o.contains("┼")), "boxes hang off the project");
    }

    #[test]
    fn splash_menus_and_resizing() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        // The splash waits for a button: other keys do nothing.
        app.splash = true;
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Esc));
        assert!(app.splash, "random keys don't leave the splash");
        app.on_key(key(KeyCode::Down));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("↑↓ choose   Enter open"));
        app.on_key(key(KeyCode::Enter));
        assert!(!app.splash && matches!(app.mode, Mode::HyPane(_)), "Enter picks the selected one (New)");
        app.mode = Mode::Normal;

        // Right-click menus.
        app.menu_for_session(1, (10, 10));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Message claude…") && o.contains("Rename") && o.contains("Close"));
        let Mode::HyMenu(m) = &app.mode else { panic!("a menu") };
        let talk = m.items.iter().position(|(l, _)| l.starts_with("Message")).unwrap();
        app.menu_pick(talk);
        assert!(matches!(app.mode, Mode::Talk { term: 1, .. }), "picking an item does it");
        app.mode = Mode::Normal;
        app.menu_for_project(0, (5, 5));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("Rename") && o.contains("Close") && o.contains("New worktree") && o.contains("Open worktree…"), "herdr's project menu");
        app.mode = Mode::Normal;

        // herdr's pane menu, and closing asks first.
        app.menu_for_pane(1, (40, 10));
        let o = draw(&mut app, 160, 45);
        show(&o);
        for item in ["Rename pane", "Split right", "Split down", "Send right-clicks to pane", "Close pane"] {
            assert!(o.contains(item), "pane menu has {item}");
        }
        let Mode::HyMenu(m) = &app.mode else { panic!("a menu") };
        let close = m.items.iter().position(|(l, _)| l == "Close pane").unwrap();
        app.menu_pick(close);
        assert!(matches!(&app.mode, Mode::Confirm(c) if c.title == "Close pane"), "asks before closing");
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains(" Close Enter") && o.contains(" Cancel Esc"), "a red Close and a plain Cancel");
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Normal), "Esc keeps it");
        // The sidebar: sessions right under their project, worktree sessions tagged.
        let o = draw(&mut app, 160, 45);
        assert!(!o.contains("WORKTREES") && !o.contains("BRANCHES"), "no folder headings");
        assert!(o.contains("◇ rate"), "a worktree session is named after it");

        // Resizing the sidebar, within its limits.
        let before = app.hy.side_rect.width;
        app.act(Action::Resize(crate::layout::Dir::Right));
        draw(&mut app, 160, 45);
        assert!(app.hy.side_rect.width > before, "wider");
        for _ in 0..40 {
            app.act(Action::Resize(crate::layout::Dir::Right));
        }
        assert_eq!(app.hy.saved.side_w, Some(hydra::SIDE_MAX), "but not past the max");
        for _ in 0..40 {
            app.act(Action::Resize(crate::layout::Dir::Left));
        }
        assert_eq!(app.hy.saved.side_w, Some(hydra::SIDE_MIN), "nor under the min");
    }

    #[test]
    fn bright_black_backgrounds_are_a_quiet_panel() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        // Claude draws pasted text and its diff panel on palette colour 8.
        let mut p = vt100::Parser::new(20, 80, 0);
        p.process(b"[100mpasted text[0m plain");
        app.parsers.insert(1, p);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 45)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let (_, inner) = app.panes[0];
        let buf = term.backend().buffer();
        assert_eq!(buf[(inner.x, inner.y)].bg, app.theme.card2, "not a light grey slab");
        assert_eq!(buf[(inner.x + 13, inner.y)].bg, app.theme.bg);
    }

    #[test]
    fn wheel_scrolls_history() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let mut p = vt100::Parser::new(40, 120, 1000);
        for i in 1..=100 {
            p.process(format!("line {i}
").as_bytes());
        }
        app.parsers.insert(1, p);
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("line 100") && !o.contains("line 40 "), "bottom first");
        let (_, inner) = app.panes[0];
        for _ in 0..10 {
            app.on_mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: inner.x + 5, row: inner.y + 5, modifiers: KeyModifiers::NONE });
        }
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(!o.contains("line 100"), "scrolled up: {:?}", app.scroll.get(&1));
        assert!(o.contains("↑ 30 lines up"), "and it says so");
        app.on_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT));
        app.on_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("line 100") && !o.contains("lines up"), "Shift+PageDown back to the bottom");
    }

    #[test]
    fn behaves_like_a_terminal() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let mut p = vt100::Parser::new(40, 120, 1000);
        for i in 1..=100 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        app.parsers.insert(1, p);
        draw(&mut app, 160, 45);
        // PageUp at a prompt scrolls history; a scrollbar shows where you are.
        app.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(app.scroll.get(&1).is_some_and(|n| *n > 10), "PageUp scrolled: {:?}", app.scroll.get(&1));
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("┃"), "scrollbar thumb");
        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(!app.scroll.contains_key(&1), "typing goes back to the bottom");
        // A full-screen program keeps PageUp for itself.
        if let Some(p) = app.parsers.get_mut(&1) {
            p.process(b"\x1b[?1049h");
        }
        app.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(!app.scroll.contains_key(&1), "PageUp went to the program");
        // Synchronized output holds drawing until it ends.
        app.on_server(ServerMsg::Output { term: 1, data: b"\x1b[?2026hhalf a frame".to_vec() });
        assert!(app.sync_hold_until().is_some(), "held mid update");
        app.on_server(ServerMsg::Output { term: 1, data: b"rest\x1b[?2026l".to_vec() });
        assert!(app.sync_hold_until().is_none(), "drawn once it's whole");
        // Focus reporting.
        app.on_server(ServerMsg::Output { term: 1, data: b"\x1b[?1004h".to_vec() });
        assert!(app.focus_report.contains(&1));
        // A program's clipboard copy reaches the user (and says so).
        app.on_server(ServerMsg::Clipboard { term: 1, text: "hello".into() });
        assert!(app.notice.as_ref().is_some_and(|(m, ..)| m.contains("copied 5 characters")));
    }

    #[test]
    fn rewraps_on_resize_and_follows_the_cursor_style() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        // Narrow pane: a long line wraps over several rows.
        let long = "word ".repeat(40);
        app.sizes.insert(1, (40, 20));
        app.parsers.insert(1, vt100::Parser::new(20, 40, 1000));
        app.on_server(ServerMsg::Output { term: 1, data: format!("{long}\r\nend\x1b[5 q").into_bytes() });
        let rows_of = |app: &App| {
            let sc = app.parsers[&1].screen();
            sc.rows(0, sc.size().1).filter(|l| l.contains("word")).count()
        };
        let rows_narrow = rows_of(&app);
        assert!(rows_narrow >= 5, "wrapped narrow: {rows_narrow}");
        // Wider: the same text re-wraps into fewer rows instead of being cut.
        app.panes = vec![(1, Rect { x: 0, y: 0, width: 120, height: 20 })];
        app.sync_sizes();
        let rows_wide = rows_of(&app);
        assert!(rows_wide < rows_narrow && rows_wide >= 1, "re-wrapped: {rows_wide} rows (was {rows_narrow})");
        assert!(app.parsers[&1].screen().contents().contains("end"));
        // The program asked for a blinking bar.
        assert_eq!(app.cursor_style.get(&1), Some(&5));
    }

    #[test]
    fn saves_pasted_images_as_png() {
        let p = std::env::temp_dir().join(format!("hydra-paste-{}.png", std::process::id()));
        let px: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8).collect();
        super::write_png(&p, 3, 2, &px).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 13, 10, 26, 10], "a real PNG");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn task_box_starts_an_agent_on_a_task() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        let cmd = super::hydra::np_command(&app, "claude", 1, "fix the login bug").unwrap();
        assert!(cmd.starts_with("claude --model opus ") && cmd.contains("fix the login bug"), "{cmd}");
        assert_eq!(super::hydra::np_command(&app, "claude", 0, "  ").as_deref(), Some("claude"));
        assert_eq!(super::hydra::np_command(&app, "shell", 0, "x"), None);
        app.hy_new(0, false);
        for c in "add tests".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Right));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("TASK") && o.contains("add tests"), "the task row shows what you typed");
        assert!(o.contains("Runs: claude --model opus"), "and what will run");
        app.on_key(key(KeyCode::Esc));
        app.hy_new(0, false);
        assert!(matches!(&app.mode, Mode::HyPane(np) if np.task == "add tests"), "Esc keeps the task as a draft");
    }

    #[test]
    fn presets_run_in_one_key_or_ask() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        let p = |name: &str, prompt: &str, place: &str| crate::config::Preset {
            name: name.into(),
            agent: "claude".into(),
            model: "sonnet".into(),
            prompt: prompt.into(),
            place: place.into(),
        };
        app.cfg.presets = vec![p("commit and push", "Commit everything and push.", "send"), p("review", "Review {task} for bugs.", "worktree")];
        assert_eq!(app.cfg.presets[1].fill("the auth module"), "Review the auth module for bugs.");
        let cmd = super::hydra::np_command(&app, "★ review", 0, "auth").unwrap();
        assert!(cmd.starts_with("claude --model sonnet ") && cmd.contains("Review auth for bugs."), "{cmd}");
        draw(&mut app, 160, 45);
        app.act(Action::Presets);
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("1  commit and push  · tell it") && o.contains("2  review…  · new worktree"));
        app.on_key(key(KeyCode::Char('2')));
        assert!(matches!(&app.mode, Mode::HyPane(np) if np.place == Some(0)), "a preset with {{task}} asks for it in + New");
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("★ review") && o.contains("sonnet (from the preset)"));
    }

    #[test]
    fn agent_rows_show_name_model_and_latest_prompt() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let id = *app.snap.terms.iter().find(|(_, t)| t.agent.as_deref() == Some("claude")).unwrap().0;
        let t = app.snap.terms.get_mut(&id).unwrap();
        t.name = "Fix the login flow".into();
        t.summary = "now add a test for it".into();
        t.model = "opus 4.5".into();
        t.status = Status::Working;
        app.hy_fresh();
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("✻ claude") && o.contains("shop-api · main · opus 4.5"), "status, icon and name on the row; the pane's title has the model");
        assert!(o.contains("Fix the login flow"), "its name stays");
        assert!(!o.contains("› now add a test for it"), "one line under a row, no more");
    }

    #[test]
    fn memory_view_lists_sessions_biggest_first() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let ids: Vec<TermId> = app.snap.terms.keys().copied().collect();
        for (n, id) in ids.iter().enumerate() {
            app.snap.terms.get_mut(id).unwrap().mem = ((n as u64 + 1) * 300) << 20;
        }
        app.hy_fresh();
        app.act(Action::Memory);
        let o = draw(&mut app, 160, 45);
        show(&o);
        let rows = super::hydra::memory_rows(&app);
        assert!(rows.windows(2).all(|w| w[0].3 >= w[1].3), "biggest first");
        assert!(o.contains("Memory ·") && o.contains("in all") && o.contains("MB"));
    }

    #[test]
    fn history_keeps_who_finished_asked_and_rang() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let mut next = app.snap.clone();
        let (a, b) = {
            let mut ids = next.terms.iter().filter(|(_, t)| t.status != Status::Done).map(|(id, _)| *id);
            (ids.next().unwrap(), ids.next().unwrap())
        };
        next.terms.get_mut(&a).unwrap().status = Status::Done;
        next.terms.get_mut(&a).unwrap().said = "Fixed it.\nMore".into();
        next.terms.get_mut(&b).unwrap().bell = true;
        app.got_state = true;
        app.on_server(ServerMsg::State(next));
        let texts: Vec<&str> = app.history.iter().map(|h| h.3.as_str()).collect();
        assert!(texts.iter().any(|t| t.ends_with("finished: Fixed it.")), "{texts:?}");
        assert!(texts.iter().any(|t| t.ends_with("rang the bell")), "{texts:?}");
        app.act(Action::History);
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("What happened") && o.contains("rang the bell"));
    }

    #[test]
    fn any_number_of_splits_and_tabs() {
        let (_, mut app) = super::design_tests::render_with(200, 50);
        let a = app.focused().unwrap();
        let others: Vec<TermId> = app.snap.terms.keys().copied().filter(|t| *t != a).collect();
        let (b, c) = (others[0], others[1]);
        app.hy.tabs.clear();
        app.hy_place(a, None);
        app.hy.pending_split = Some((a, Instant::now()));
        app.hy_place(b, Some(a));
        app.hy.pending_split = Some((b, Instant::now()));
        app.hy_place(c, Some(b));
        assert_eq!(app.hy.tabs.len(), 1);
        assert_eq!(app.hy.tabs[0].layout.leaves().len(), 3, "three side by side");
        // Focus stays on a (the snapshot's focus), so draw shows all three.
        app.hy.tabs[0].focus = a;
        let o = draw(&mut app, 200, 50);
        show(&o);
        assert_eq!(app.hy.leaf_rects.len(), 3);
        let widths: Vec<u16> = app.hy.leaf_rects.iter().map(|(_, r)| r.width).collect();
        assert!(widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 2, "three tile evenly: {widths:?}");
        // Close one: two left, with a line between them you can drag.
        assert!(app.hy_unshow(c));
        assert_eq!(app.hy.tabs[0].layout.leaves().len(), 2);
        draw(&mut app, 200, 50);
        assert_eq!(app.hy.dividers.len(), 1, "a divider between the two");
        let before = app.hy.leaf_rects[0].1.width;
        let path = app.hy.dividers[0].2.clone();
        app.hy.tabs[0].layout.set_ratio(&path, 0.3);
        draw(&mut app, 200, 50);
        assert!(app.hy.leaf_rects[0].1.width < before, "dragging moves it");
        // A new tab for c; the bar shows both.
        app.hy.new_tab = Some(Instant::now());
        app.hy_place(c, Some(a));
        assert_eq!(app.hy.tabs.len(), 2);
        app.hy.tab = 0;
        let o = draw(&mut app, 200, 50);
        show(&o);
        assert!(o.contains(" 1 ") && o.contains(" 2 ") && o.contains(" + "), "a tab bar");
    }

    #[test]
    fn map_opens_from_its_key() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        draw(&mut app, 160, 45);
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        app.on_key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT));
        eprintln!("view after key: {:?}", app.view.as_ref().map(std::mem::discriminant));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(matches!(app.view, Some(View::Map(_))), "the map is open");
    }

    #[test]
    fn renaming_names_the_row() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let id = *app.snap.terms.iter().find(|(_, t)| t.agent.as_deref() == Some("codex")).unwrap().0;
        app.snap.terms.get_mut(&id).unwrap().label = "Doing something".into();
        app.hy_fresh();
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("◇ Doing something"), "the name is on the row itself");
        assert_eq!(o.matches("Doing something").count(), 1, "once: on the row, not in a second line");
    }

    #[test]
    fn sidebar_by_keyboard() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        draw(&mut app, 160, 45);
        // Clicking a project puts the keys in the sidebar, on that project.
        app.on_hy_hit(hydra::HyHit::ToggleProj(0), false);
        app.on_hy_hit(hydra::HyHit::ToggleProj(0), false);
        assert!(matches!(app.mode, Mode::Side) && app.hy.cursor_proj.is_some(), "on the project row");
        draw(&mut app, 160, 45);
        // ← folds it, → opens it again.
        app.on_key(key(KeyCode::Left));
        assert!(app.hy.saved.closed.iter().any(|k| k.starts_with("p:")), "folded");
        app.on_key(key(KeyCode::Right));
        assert!(!app.hy.saved.closed.iter().any(|k| k.starts_with("p:")), "open");
        draw(&mut app, 160, 45);
        // ↓ onto its first session; ← back up to the project.
        app.on_key(key(KeyCode::Down));
        let first = app.hy.cursor.expect("on a session");
        app.on_key(key(KeyCode::Left));
        assert!(app.hy.cursor_proj.is_some() && app.hy.cursor.is_none(), "← goes to its project");
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.hy.cursor, Some(first));
        // The bottom bar says what the keys do, the row's own included.
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("x  close") && o.contains("r  rename"), "the row's keys in the bottom bar");
        // A row's letter does what its menu says: x asks to close it.
        app.on_key(key(KeyCode::Char('x')));
        assert!(matches!(&app.mode, Mode::Confirm(c) if c.title == "Close pane"), "x closes (after asking)");
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Side) && app.hy.cursor == Some(first), "saying no goes back to the sidebar");
        let next = app.side_after_close();
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Side), "after closing, the keys stay in the sidebar");
        assert_eq!(next.map(|n| n != hydra::SideItem::Sess(first)), Some(true));
        app.hy_side_set(hydra::SideItem::Sess(first));
        // Enter opens it and gives the keys back to the pane.
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Normal) && app.hy.cursor.is_none());
        // The leader in the sidebar is the leader, not "message this one".
        app.act(Action::BrowseTree);
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(matches!(app.mode, Mode::Prefix { .. }), "Ctrl+Space waits for a key: {:?}", std::mem::discriminant(&app.mode));
        app.mode = Mode::Normal;
        // Typing in the sidebar goes to the pane instead.
        app.act(Action::BrowseTree);
        assert!(matches!(app.mode, Mode::Side));
        app.on_key(key(KeyCode::Char('q')));
        assert!(matches!(app.mode, Mode::Normal), "a letter leaves the sidebar (and is typed into the pane)");
    }

    #[test]
    fn changes_outside_git_offers_git_init() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let dir = std::env::temp_dir().join(format!("hydra-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.open_changes(dir.clone());
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("isn't a git repository yet.") && o.contains(" Make it a git repo g") && o.contains(" Cancel Esc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_grouped_like_the_redesign() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.act(Action::Settings);
        let o = draw(&mut app, 160, 45);
        show(&o);
        for g in ["INPUT", "LAYOUT", "SHELL"] {
            assert!(o.contains(g), "General is grouped: {g}");
        }
        let input = o.find("INPUT").unwrap();
        let layout = o.find("LAYOUT").unwrap();
        assert!(input < layout, "INPUT before LAYOUT");
        assert!(o.contains("› Leader key"), "the first row is selected");
        // Appearance: one theme per row.
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("THEME") && o.contains("● Default") && o.contains("○ Monokai") && o.contains("○ Tokyo Night"));
        assert!(o.contains("Swatches: background, surface"));
    }

    #[test]
    fn a_session_is_grouped_by_where_it_is_now() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        // The shell cd's out of shop-api into a folder that isn't a repo.
        let elsewhere = PathBuf::from(if cfg!(windows) { r"C:\notes" } else { "/notes" });
        let t = app.snap.terms.get_mut(&3).unwrap();
        (t.cwd, t.root, t.top, t.branch) = (elsewhere.clone(), None, None, None);
        app.hy_fresh();
        let model = app.hy_model();
        let group_of = |term: TermId| model.iter().find(|p| p.sessions().any(|s| s.term == term)).map(|p| p.name.clone());
        assert_eq!(group_of(3).as_deref(), Some("notes"), "it moved to where it is");
        assert_eq!(group_of(1).as_deref(), Some("shop-api"), "the others stay");
    }

    #[test]
    fn a_note_about_a_session_is_a_way_there() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let term = *app.snap.terms.keys().next().unwrap();
        app.notify("claude finished".into(), false);
        app.notice_term = Some(term);
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("claude finished") && o.contains("click to open"));
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::Hy(hydra::HyHit::Session(term))), "clicking it opens the session");
        // Any other note isn't.
        app.notify("saved".into(), false);
        let o = draw(&mut app, 160, 45);
        assert!(o.contains("saved") && !o.contains("click to open"));
    }

    #[test]
    fn a_split_is_one_session_and_others_open_full_size() {
        let (_, mut app) = super::design_tests::render_with(200, 50);
        let a = app.focused().unwrap();
        let others: Vec<TermId> = app.snap.terms.keys().copied().filter(|t| *t != a).collect();
        let (b, c) = (others[0], others[1]);
        app.hy.tabs.clear();
        app.hy_place(a, None);
        app.hy.pending_split = Some((a, Instant::now()));
        app.hy_place(b, Some(a));
        app.hy_fresh();
        let rows: Vec<TermId> = app.hy_model().iter().flat_map(|p| p.sessions().map(|s| s.term).collect::<Vec<_>>()).collect();
        assert!(rows.contains(&a) && !rows.contains(&b), "the pane beside a isn't a session of its own: {rows:?}");
        // b needs you: a's row says so.
        app.snap.terms.get_mut(&b).unwrap().status = Status::Blocked;
        app.hy_fresh();
        let row = app.hy_model().iter().flat_map(|p| p.sessions().cloned().collect::<Vec<_>>()).find(|s| s.term == a).unwrap();
        assert_eq!(row.status, Status::Blocked);
        // Picking another session shows it alone; the split is still there to go back to.
        app.hy_place(c, Some(b));
        assert_eq!(app.hy.tabs[app.hy.tab].layout, crate::layout::Node::Leaf(c), "full size");
        assert!(app.hy.tabs.iter().any(|t| t.layout.leaves() == vec![a, b]), "the split is kept: {:?}", app.hy.tabs);
        app.hy_place(a, Some(c));
        assert_eq!(app.hy.tabs[app.hy.tab].layout.leaves(), vec![a, b], "its row brings the split back");
    }

    #[test]
    fn closing_a_split_pane_leaves_the_other() {
        let (_, mut app) = super::design_tests::render_with(200, 50);
        let a = app.focused().unwrap();
        let others: Vec<TermId> = app.snap.terms.keys().copied().filter(|t| *t != a).collect();
        let (b, c) = (others[0], others[1]);
        app.hy.tabs.clear();
        app.hy_place(a, None);
        app.hy.pending_split = Some((a, Instant::now()));
        app.hy_place(b, Some(a));
        assert_eq!(app.hy.tabs[0].layout.leaves().len(), 2);
        // b is closed; the server then focuses some other session, c.
        app.snap.terms.remove(&b);
        app.hy_place(c, Some(b));
        assert_eq!(app.hy.tabs[0].layout, crate::layout::Node::Leaf(a), "a takes the room; c doesn't slide into b's place");
        // Closing from the ✕ takes it out of the split first.
        app.hy.pending_split = Some((a, Instant::now()));
        app.snap.terms.insert(b, app.snap.terms[&c].clone());
        app.hy_place(b, Some(a));
        app.menu_do(menu::Act::End(vec![b]));
        assert_eq!(app.hy.tabs[0].layout, crate::layout::Node::Leaf(a));
    }

    #[test]
    fn copying_pops_a_toast() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.notify("Copied".into(), false);
        let o = draw(&mut app, 160, 45);
        show(&o);
        let lines: Vec<&str> = o.lines().collect();
        let at = lines.iter().position(|l| l.contains("✓ Copied")).expect("a toast");
        assert!(at < lines.len() - 2, "over the panes, above the bottom bar");
        assert!(!lines.last().unwrap().contains("Copied"), "not in the bottom bar");
    }

    #[test]
    fn go_to_a_project_or_session() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        app.act(Action::GoTo);
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Go to") && o.contains("▌shop-api") && o.contains("type a project or session"));
        for c in "rate".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("◇ rate"), "typing filters to what matches");
        let Mode::GoTo { query, sel } = app.mode.clone() else { panic!("still open") };
        let rows = hydra::goto_rows(&app.hy_model(), &query);
        assert!(matches!(rows[sel], hydra::GoRow::Sess(..)), "it lands on the matching session");
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Normal), "Enter goes there");
    }

    #[test]
    fn palette_and_keys_look_like_the_app() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.act(Action::Palette);
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Command palette") && o.contains("Go to a project or session") && !o.contains("workspace ·"), "commands only, in a panel");
        for c in "split".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("Split right") && !o.contains("Settings"), "typing narrows it");
        app.mode = Mode::Help { scroll: 0 };
        let o = draw(&mut app, 160, 45);
        show(&o);
        let a = o.lines().find(|l| l.contains("go to session")).unwrap();
        let b = o.lines().find(|l| l.contains("palette")).unwrap();
        assert_eq!(a.find("go to session"), b.find("palette"), "labels line up");
    }

    #[test]
    fn settings_keys_tab_is_current() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let cat = modal::Cat::ALL.iter().position(|c| *c == modal::Cat::Keys).unwrap();
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let o = draw(&mut app, 160, 45);
        show(&o);
        assert!(o.contains("GET AROUND") && o.contains("Go to a project or session") && o.contains("Command palette"));
    }

    #[test]
    fn working_names_shimmer() {
        use ratatui::style::{Color, Style};
        let (base, bright) = (Color::Rgb(200, 160, 0), Color::Rgb(255, 255, 255));
        let at = |frame| hydra::shimmer("claude", frame, base, bright, Style::default()).iter().map(|(_, s)| s.fg).collect::<Vec<_>>();
        assert_eq!(at(0).len(), 6, "one colour per letter");
        assert_ne!(at(4), at(7), "the bright band moves");
        assert!(at(7).contains(&Some(base)), "the rest stays the working colour");
    }

    #[test]
    fn the_split_line_drags_again_and_again() {
        let (_, mut app) = super::design_tests::render_with(200, 50);
        let a = app.focused().unwrap();
        let b = app.snap.terms.keys().copied().find(|t| *t != a).unwrap();
        app.hy.tabs.clear();
        app.hy_place(a, None);
        app.hy.pending_split = Some((a, Instant::now()));
        app.hy_place(b, Some(a));
        let mouse = |app: &mut App, kind: MouseEventKind, x: u16, y: u16| {
            app.on_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
            draw(app, 200, 50);
        };
        let line_x = |app: &App| app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Hy(hydra::HyHit::Divider(_))).then_some(r.x));
        // Both programs want the mouse (full-screen agents do).
        for t in [a, b] {
            let mut p = vt100::Parser::new(40, 90, 0);
            p.process(b"\x1b[?1002h\x1b[?1006h");
            app.parsers.insert(t, p);
        }
        draw(&mut app, 200, 50);
        let mut at = line_x(&app).expect("a divider");
        for (n, to) in [at - 30, at + 20, at - 10].into_iter().enumerate() {
            let y = 20;
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at, y);
            mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), to, y);
            mouse(&mut app, MouseEventKind::Up(MouseButton::Left), to, y);
            let now = line_x(&app).expect("still a divider");
            assert!(now.abs_diff(to) <= 1, "drag {n}: the line follows the mouse to {to}, it's at {now}");
            at = now;
        }
    }

    #[test]
    fn a_finished_agent_is_a_green_dot_even_if_it_rang() {
        let (_, mut app) = super::design_tests::render_with(120, 30);
        // codex finished and rang the bell (it does both), and you're on another session.
        let t = app.snap.terms.get_mut(&2).unwrap();
        (t.status, t.bell) = (Status::Done, true);
        app.hy_fresh();
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| render::draw(&mut app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        let row = |y: u16| (0..40u16).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        let y = (0..30).find(|&y| row(y).contains("rate")).expect("codex's row");
        assert!(row(y).contains('●') && !row(y).contains('♪'), "the done dot, not the bell: {:?}", row(y));
        let dot = (0..40u16).find(|&x| buf[(x, y)].symbol() == "●").unwrap();
        assert_eq!(buf[(dot, y)].fg, app.theme.done, "a green dot");
        let name = (0..40u16).find(|&x| buf[(x, y)].symbol() == "r").unwrap();
        assert_eq!(buf[(name, y)].fg, app.theme.done, "and green text");
    }

    #[test]
    fn only_the_part_with_the_keys_looks_focused() {
        let (_, mut app) = super::design_tests::render_with(120, 30);
        let bar_bg = |app: &mut App| {
            let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
            term.draw(|f| render::draw(app, f)).unwrap();
            let buf = term.backend().buffer().clone();
            let y = (0..30).find(|&y| (0..120).any(|x| buf[(x, y)].symbol() == "✕")).unwrap();
            let x = (0..120).rev().find(|&x| buf[(x, y)].symbol() == "✕").unwrap();
            (buf[(x, y)].bg, buf[(0u16, 29u16)].symbol().to_string(), buf.clone())
        };
        let (bg, _, _) = bar_bg(&mut app);
        assert_eq!(bg, app.theme.accent, "the focused pane's bar is the accent");
        app.act(Action::BrowseTree);
        let (bg, _, buf) = bar_bg(&mut app);
        assert_ne!(bg, app.theme.accent, "not while the sidebar has the keys");
        let side_left = (0..29).filter(|&y| buf[(0u16, y)].fg == app.theme.accent).count();
        assert!(side_left > 10, "the sidebar is outlined: {side_left} rows");
    }

    #[test]
    fn leader_then_a_key_does_it() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.cfg.ui.which_key = false;
        let lead = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);
        for (c, what) in [('g', "go to"), ('p', "palette"), ('?', "keys"), (',', "settings")] {
            app.mode = Mode::Normal;
            app.on_key(lead);
            assert!(matches!(app.mode, Mode::Prefix { .. }), "leader waits");
            let o = draw(&mut app, 160, 45);
            assert!(o.lines().last().unwrap_or("").contains("then:") && o.contains("g go to"), "the bottom bar shows leader mode");
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            assert!(!matches!(app.mode, Mode::Normal | Mode::Prefix { .. }), "leader + {c} opens {what}: {:?}", std::mem::discriminant(&app.mode));
        }
    }

    #[test]
    fn every_popup_fits_a_tiny_window() {
        use crate::layout::Dir;
        let actions = [
            Action::GoTo,
            Action::Palette,
            Action::Help,
            Action::Settings,
            Action::NewPane,
            Action::Jump,
            Action::OpenProject,
            Action::Talk,
            Action::Files,
            Action::Find(0),
            Action::Find(1),
            Action::Changes,
            Action::Branches,
            Action::Inbox,
            Action::Ideas,
            Action::Race,
            Action::Toolbox,
            Action::Map,
            Action::Memory,
            Action::History,
            Action::Presets,
            Action::RenameWorkspace,
            Action::BrowseTree,
            Action::SplitRight,
            Action::Focus(Dir::Left),
        ];
        for (w, h) in [(20, 6), (1, 1), (80, 3), (40, 10), (40, 15), (60, 12)] {
            for a in &actions {
                let (_, mut app) = super::design_tests::render_with(160, 45);
                app.act(a.clone());
                let _ = draw(&mut app, w, h);
                // And a right-click menu, and a question.
                app.mode = Mode::Normal;
                app.menu_for_session(1, (w.saturating_sub(1), h.saturating_sub(1)));
                let _ = draw(&mut app, w, h);
                app.menu_act(menu::Act::End(vec![1]));
                let _ = draw(&mut app, w, h);
            }
        }
    }

    #[test]
    fn borrowed_screens_look_like_hydra() {
        for (a, title) in [(Action::Toolbox, "Agent tools"), (Action::RenameWorkspace, "Rename"), (Action::NewWorktree(None), "Worktrees")] {
            let (_, mut app) = super::design_tests::render_with(120, 34);
            app.act(a);
            let o = draw(&mut app, 120, 34);
            show(&o);
            assert!(o.contains("Esc close"), "{title}: a hydra panel (title bar with Esc close)");
        }
    }

    #[test]
    fn quick_follow_up_from_the_sidebar() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        draw(&mut app, 160, 45);
        app.act(Action::SideMove(0));
        assert!(matches!(app.mode, Mode::Side));
        let row = app.hy.row_y[&app.hy.cursor.unwrap()];
        app.on_key(key(KeyCode::Char(' ')));
        assert!(matches!(app.mode, Mode::Talk { .. }), "Space opens the box");
        for c in "run the tests".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        let o = draw(&mut app, 160, 45);
        show(&o);
        let _ = row;
        assert!(o.contains("Message claude") && o.contains(" run the tests█"), "a centered text area with what you typed");
        assert!(o.contains("Run npm test -- checkout?"), "with the agent's question for context");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        app.on_key(key(KeyCode::Char('x')));
        assert!(matches!(&app.mode, Mode::Talk { input, .. } if input == "run the tests\nx"), "Shift+Enter: a new line");
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Side), "sent, and back on the list for the next one");
        assert!(app.notice.as_ref().is_some_and(|(m, ..)| m.starts_with("sent to")));
    }

    #[test]
    fn options_come_from_the_screen() {
        let mut p = vt100::Parser::new(10, 60, 0);
        p.process(b"Question: use WebKit or Safari?\r\n  1. WebKit build\r\n  2. Safari driver");
        assert_eq!(hydra::options(Some(&p)), vec!["WebKit build".to_string(), "Safari driver".to_string()]);
        assert_eq!(hydra::options(None), vec!["Yes", "Always", "No"]);
    }
}

mod settings_splash_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(app: &mut App) -> String {
        let mut term = Terminal::new(TestBackend::new(160, 45)).unwrap();
        term.draw(|f| render::draw(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..45).map(|y| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string() + "\n").collect()
    }

    #[test]
    fn splash_and_settings_render() {
        let (_, mut app) = super::design_tests::render_with(160, 45);
        app.splash = true;
        let splash = draw(&mut app);
        app.splash = false;
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 0, sel: 1, editing: None, capturing: false, scroll: 0 }));
        let settings = draw(&mut app);
        app.mode = Mode::HySettings(Box::new(design::SettingsView { cat: 2, sel: 0, editing: None, capturing: false, scroll: 0 }));
        let keys = draw(&mut app);
        if std::env::var("HYDRA_SHOW").is_ok() {
            println!("{splash}\n{settings}\n{keys}");
        }
        assert!(splash.contains("██████") && splash.contains("many heads, one body"));
        assert!(splash.contains("Resume where you left off") && splash.contains("New: an agent or a shell") && splash.contains("Open a folder"));
        for page in ["General", "Sessions", "Appearance", "Agents", "Keys"] {
            assert!(settings.contains(page), "page {page}");
        }
        assert!(settings.contains("Leader key") && settings.contains("Splash screen"));
        assert!(keys.contains("Default") && keys.contains("Monokai"), "a row per theme");
    }
}
