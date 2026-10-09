//! Leader mode in the floating look: the key map (every leader key by what it acts on, typed
//! to search), its second steps, the actions list, and a new tab named in its pill.

use super::*;
use crate::layout::Dir;

/// How long typed words may get in the key map's search.
const QUERY_MAX: usize = 40;
/// Matches the key map lists while searching.
const MATCHES_SHOWN: usize = 12;

/// The key map's groups: what each key does, in the design's words.
fn groups() -> Vec<(&'static str, Vec<(Action, &'static str)>)> {
    vec![
        ("AGENTS", vec![(Action::Jump, "jump to waiting"), (Action::Talk, "talk to an agent"), (Action::Answer('1'), "answer"), (Action::NewSession, "new agent")]),
        ("PANES", vec![(Action::NewPane, "new pane"), (Action::Zoom, "zoom"), (Action::ClosePane, "close"), (Action::Arrange, "layout"), (Action::Focus(Dir::Left), "move focus")]),
        ("TABS", vec![(Action::NewTab, "new tab"), (Action::SelectTab(1), "go to tab"), (Action::RenameTab, "rename")]),
        ("PROJECT", vec![(Action::OpenProject, "open project"), (Action::Worktrees, "worktrees  ›"), (Action::Files, "files"), (Action::Changes, "changes")]),
        ("SESHI", vec![(Action::Settings, "settings"), (Action::Help, "all keys"), (Action::Detach, "quit, agents keep running")]),
    ]
}

/// The actions list: every common command with its key.
pub(in crate::client) fn action_items() -> Vec<(&'static str, Action)> {
    vec![
        ("New pane", Action::NewPane),
        ("New tab", Action::NewTab),
        ("Jump to what needs you", Action::Jump),
        ("Open project", Action::OpenProject),
        ("Talk to an agent", Action::Talk),
        ("Zoom pane", Action::Zoom),
        ("Change layout", Action::Arrange),
        ("All keys", Action::Help),
    ]
}

/// A second step's choices: key, what it does, a dim detail, the action.
fn step_items(app: &App, step: Step) -> Vec<(char, &'static str, String, Action)> {
    match step {
        Step::Worktrees => {
            let model = app.hy_model();
            let here = app.focused().and_then(|f| find(&model, f));
            let n = here.map(|(p, _, _)| p.wts.len()).unwrap_or(0);
            let branch = here.map(|(_, w, _)| w.branch.clone()).unwrap_or_default();
            vec![
                ('n', "new worktree", "own branch, new folder, new agent".into(), Action::NewWorktree(None)),
                ('s', "switch to…", format!("{n} worktree{}", if n == 1 { "" } else { "s" }), Action::GoTo),
                ('m', "merge into main", branch, Action::Ship),
                ('d', "delete worktree", "keeps the branch".into(), Action::RemoveWorktree),
            ]
        }
    }
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Worktrees => "worktrees",
    }
}

fn step_key(app: &App, step: Step) -> String {
    match step {
        Step::Worktrees => k(app, &Action::Worktrees),
    }
}

/// The key a row shows: its own binding, or the group it stands for (1 2 3, arrows, Alt+1-9).
fn key_label(app: &App, a: &Action) -> String {
    let bound = |a: &Action| app.keymap.prefixed.iter().any(|(_, b)| b == a);
    match a {
        Action::Answer(_) => ['1', '2', '3'].iter().filter(|c| bound(&Action::Answer(**c))).map(|c| c.to_string()).collect::<Vec<_>>().join(" "),
        Action::Focus(_) => "←↑↓→".into(),
        Action::SelectTab(_) => {
            if app.keymap.global.values().any(|b| *b == Action::SelectTab(1)) {
                "Alt+1-9".into()
            } else {
                "1-9".into()
            }
        }
        _ => k(app, a),
    }
}

/// Every leader binding whose description has all the typed words.
fn search(app: &App, query: &str) -> Vec<(String, String, Action)> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut out: Vec<(String, String, Action)> = Vec::new();
    for key in &app.keymap.prefixed_order {
        let Some(a) = app.keymap.prefixed.get(key) else { continue };
        if out.iter().any(|(_, _, b)| b == a) || *a == Action::None {
            continue;
        }
        let label = a.describe();
        let hay = label.to_lowercase();
        if words.iter().all(|w| hay.contains(w.as_str())) {
            out.push((crate::client::design::pretty_key(key), label, a.clone()));
        }
    }
    out
}

/// What each shown row of the key map runs, in the order its rows are numbered.
pub(in crate::client) fn keymap_actions(app: &App, km: &KeyMap) -> Vec<Action> {
    if let Some(step) = km.step {
        return step_items(app, step).into_iter().map(|(_, _, _, a)| a).collect();
    }
    if !km.query.is_empty() {
        return search(app, &km.query).into_iter().take(MATCHES_SHOWN).map(|(_, _, a)| a).collect();
    }
    groups().into_iter().flat_map(|(_, items)| items.into_iter().map(|(a, _)| a)).collect()
}

/// The key map over a dimmed screen: a sky-bordered card with a search pill, the five
/// groups of keys (or the matches, or a second step), and what to do next at its foot.
pub(in crate::client) fn draw_keymap(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, km: &KeyMap) {
    let look = Look::of(&app.cfg.ui);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    hit(app, area, HyHit::Close);
    let w = 112.min(area.width.saturating_sub(4));
    let h = (if km.step.is_some() { 17 } else { 24 }).min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let mut c = Card::new(t, &lead).lit(t.sky());
    c.title_fg = t.accent;
    c.bg = t.card;
    card(app, buf, r, &c, t);
    hit(app, r, HyHit::Noop);
    let cs = Style::default().bg(t.card);
    // The search pill.
    let sy = r.y + 2;
    row_pill(look, buf, r.x + 3, sy, w.saturating_sub(6), t.card2, t.card);
    let ps = Style::default().bg(t.card2);
    let mut typed = vec![seg("› ", ps.fg(t.accent).add_modifier(Modifier::BOLD))];
    if let Some(step) = km.step {
        typed.push(seg(step_key(app, step), ps.fg(t.strong).add_modifier(Modifier::BOLD)));
        typed.push(seg(format!("  {}", step_name(step)), ps.fg(t.muted)));
    } else {
        typed.push(seg(km.query.clone(), ps.fg(t.strong).add_modifier(Modifier::BOLD)));
    }
    typed.push(seg("█", ps.fg(t.accent)));
    if km.step.is_none() && km.query.is_empty() {
        let hint = if km.searching { " type words to search" } else { " press a key, or Tab to search" };
        typed.push(seg(hint, ps.fg(t.muted).add_modifier(Modifier::ITALIC)));
    }
    let back = if km.step.is_some() || !km.query.is_empty() { "Backspace back" } else if km.searching { "Tab keys" } else { "Esc cancel" };
    let bw = back.width() as u16;
    put(buf, r.x + 5, sy, &typed, r.right().saturating_sub(bw + 8));
    put(buf, r.right().saturating_sub(bw + 6), sy, &[seg(back, ps.fg(t.muted))], r.right().saturating_sub(5));

    let needs = app.hy_model().iter().flat_map(|p| p.sessions()).filter(|s| s.status == Status::Blocked).count();
    let mut row = 0usize;
    if let Some(step) = km.step {
        let model = app.hy_model();
        let proj = app.focused().and_then(|f| find(&model, f)).map(|(p, _, _)| p.name.clone()).unwrap_or_default();
        put(buf, r.x + 4, r.y + 5, &[seg(format!("{} IN {}", step_name(step).to_uppercase(), proj.to_uppercase()), cs.fg(t.muted).add_modifier(Modifier::BOLD))], r.right() - 2);
        for (i, (key, label, detail, _)) in step_items(app, step).into_iter().enumerate() {
            let y = r.y + 7 + i as u16 * 2;
            if y >= r.bottom().saturating_sub(2) {
                break;
            }
            let rr = Rect { x: r.x + 3, y, width: w.saturating_sub(6), height: 1 };
            let sel = i == km.sel || hovered(app, rr);
            if sel {
                row_pill(look, buf, rr.x, y, rr.width, t.hov, t.card);
            }
            let bg = if sel { t.hov } else { t.card };
            let st = Style::default().bg(bg);
            put(buf, r.x + 5, y, &keycap_pill(look, t, &key.to_string(), i == km.sel, bg), r.right());
            put(buf, r.x + 11, y, &[seg(label, st.fg(if sel { t.strong } else { t.text }).add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }))], r.x + 31);
            put(buf, r.x + 32, y, &[seg(detail, st.fg(t.muted))], r.right() - 3);
            hit(app, rr, HyHit::KeyRow(i));
        }
        let path = vec![
            seg(lead.clone(), cs.fg(t.accent).add_modifier(Modifier::BOLD)),
            seg("  ›  ", cs.fg(t.muted)),
            seg(step_key(app, step), cs.fg(t.accent).add_modifier(Modifier::BOLD)),
            seg("  ›  ", cs.fg(t.muted)),
            seg("_", cs.fg(t.muted)),
            seg("   the path so far", cs.fg(t.muted).add_modifier(Modifier::ITALIC)),
        ];
        put(buf, r.x + 4, r.bottom() - 2, &path, r.right() - 2);
        return;
    }
    if !km.query.is_empty() {
        let found = search(app, &km.query);
        if found.is_empty() {
            put(buf, r.x + 5, r.y + 5, &[seg("No key does that. Esc, or Backspace to change the words.", cs.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 3);
        }
        for (i, (key, label, _)) in found.into_iter().take(MATCHES_SHOWN).enumerate() {
            let y = r.y + 5 + i as u16;
            if y >= r.bottom().saturating_sub(2) {
                break;
            }
            let rr = Rect { x: r.x + 3, y, width: w.saturating_sub(6), height: 1 };
            let sel = i == km.sel || hovered(app, rr);
            if sel {
                row_pill(look, buf, rr.x, y, rr.width, t.hov, t.card);
            }
            let bg = if sel { t.hov } else { t.card };
            let kx = put(buf, r.x + 5, y, &keycap_pill(look, t, &key, i == km.sel, bg), r.right());
            put(buf, kx.max(r.x + 14) + 1, y, &[seg(label, Style::default().bg(bg).fg(if sel { t.strong } else { t.text }))], r.right() - 3);
            hit(app, rr, HyHit::KeyRow(i));
        }
        put(buf, r.x + 4, r.bottom() - 2, &[seg("Enter runs the selected one. ↑↓ to choose.", cs.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
        return;
    }
    let col_w = w.saturating_sub(8) / 3;
    for (gi, (name, items)) in groups().into_iter().enumerate() {
        let (col, grow) = (gi as u16 % 3, gi as u16 / 3);
        let gx = r.x + 4 + col * col_w;
        let gy = r.y + 5 + grow * 9;
        if gy + 1 >= r.bottom() {
            row += items.len();
            continue;
        }
        put(buf, gx, gy, &[seg(name, cs.fg(t.muted).add_modifier(Modifier::BOLD))], gx + col_w);
        for (i, (a, label)) in items.into_iter().enumerate() {
            let y = gy + 2 + i as u16;
            if y >= r.bottom().saturating_sub(2) {
                row += 1;
                continue;
            }
            let key = key_label(app, &a);
            let caps = if key.is_empty() { vec![seg("·", cs.fg(t.line))] } else { keycap_pill(look, t, &key, false, t.card) };
            let kx = put(buf, gx, y, &caps, gx + col_w);
            let lx = (kx + 1).max(gx + 8);
            let lend = put(buf, lx, y, &[seg(label, cs.fg(t.text))], gx + col_w);
            let extra = match a {
                Action::Jump if needs > 0 => Some((format!("● {needs}"), t.blocked)),
                Action::Settings if app.update_available.is_some() => Some(("●".to_string(), t.accent)),
                _ => None,
            };
            if let Some((e, ec)) = extra {
                put(buf, lend + 1, y, &[seg(e, cs.fg(ec).add_modifier(Modifier::BOLD))], gx + col_w);
            }
            hit(app, Rect { x: gx, y, width: col_w.saturating_sub(1), height: 1 }, HyHit::KeyRow(row));
            row += 1;
        }
    }
    put(
        buf,
        r.x + 4,
        r.bottom() - 2,
        &[seg("Keys with ", cs.fg(t.muted).add_modifier(Modifier::ITALIC)), seg("›", cs.fg(t.text)), seg(" open a second step.", cs.fg(t.muted).add_modifier(Modifier::ITALIC))],
        r.right() - 2,
    );
}

/// The actions list: a card at the bottom left, over the sidebar's foot.
pub(in crate::client) fn draw_actions(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, sel: usize) {
    let look = Look::of(&app.cfg.ui);
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    hit(app, area, HyHit::Close);
    let items = action_items();
    let w = 40.min(area.width.saturating_sub(4));
    let h = (items.len() as u16 + 5).min(area.height.saturating_sub(2));
    let side = grid(app, area).side;
    let x = if side.width > 0 { side.x + 1 } else { area.x + 3 };
    let y = area.bottom().saturating_sub(4 + h).max(area.y);
    let r = Rect { x, y, width: w, height: h };
    let mut c = Card::new(t, "Actions").lit(t.accent);
    c.bg = t.card;
    card(app, buf, r, &c, t);
    hit(app, r, HyHit::Noop);
    let needs = app.hy_model().iter().flat_map(|p| p.sessions()).filter(|s| s.status == Status::Blocked).count();
    for (i, (label, a)) in items.iter().enumerate() {
        let yy = r.y + 2 + i as u16;
        if yy >= r.bottom().saturating_sub(3) {
            break;
        }
        let rr = Rect { x: r.x + 2, y: yy, width: w.saturating_sub(4), height: 1 };
        let on = i == sel || hovered(app, rr);
        if on {
            row_pill(look, buf, rr.x, yy, rr.width, t.hov, t.card);
        }
        let bg = if on { t.hov } else { t.card };
        let st = Style::default().bg(bg);
        let mut ls = st.fg(if on { t.strong } else { t.text });
        if on {
            ls = ls.add_modifier(Modifier::BOLD);
        }
        let end = put(buf, r.x + 4, yy, &[seg(*label, ls)], r.right() - 6);
        if *a == Action::Jump && needs > 0 {
            put(buf, end + 1, yy, &[seg(format!("● {needs}"), st.fg(t.blocked).add_modifier(Modifier::BOLD))], r.right() - 6);
        }
        let key = k(app, a);
        let kw = key.width() as u16;
        put(buf, r.right().saturating_sub(4 + kw), yy, &[seg(key, st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right() - 4);
        hit(app, rr, HyHit::ActionRow(i));
    }
    let lead = app.keymap.prefix.to_string().replace("C-", "Ctrl+");
    let cs = Style::default().bg(t.card);
    put(
        buf,
        r.x + 4,
        r.bottom() - 2,
        &[seg("or ", cs.fg(t.muted).add_modifier(Modifier::ITALIC)), seg(lead, cs.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" + key, anywhere", cs.fg(t.muted).add_modifier(Modifier::ITALIC))],
        r.right() - 2,
    );
}

/// A new tab before anything runs in it: its card, with what to start there.
pub(in crate::client) fn draw_new_tab(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, nt: &NewTab) {
    let look = Look::of(&app.cfg.ui);
    let model = app.hy_model();
    let here = find(&model, nt.owner);
    let proj = here.map(|(p, _, _)| p.name.clone()).unwrap_or_else(|| app.snap.terms.get(&nt.owner).map(|i| folder_name(&i.cwd)).unwrap_or_default());
    let dir = app.snap.terms.get(&nt.owner).map(|i| tilde(&i.root.clone().unwrap_or_else(|| i.cwd.clone()))).unwrap_or_default();
    let title = if nt.name.is_empty() { nt.fallback.clone() } else { nt.name.clone() };
    let mut c = Card::new(t, &title).lit(t.accent);
    c.foot = vec![seg(dir, Style::default().fg(t.muted))];
    let buf = f.buffer_mut();
    card(app, buf, area, &c, t);
    let bg = c.bg;
    let cx = area.x + area.width / 2;
    let cy = (area.y + area.height / 2).saturating_sub(4);
    let centre = |buf: &mut Buffer, y: u16, segs: &[Seg]| -> u16 {
        let x = cx.saturating_sub(segs_width(segs) / 2);
        put(buf, x, y, segs, area.right().saturating_sub(2));
        x
    };
    let st = Style::default().bg(bg);
    centre(buf, cy, &[seg(format!("New tab in {proj}"), st.fg(t.strong).add_modifier(Modifier::BOLD))]);
    let hint = if nt.naming { format!("Type a name, then Enter. Esc keeps \"{}\".", nt.fallback) } else { "Pick what runs in it. Backspace renames, Esc cancels.".to_string() };
    centre(buf, cy + 1, &[seg(hint, st.fg(t.muted).add_modifier(Modifier::ITALIC))]);
    let choices = [("claude", 'c', true), ("codex", 'x', false), ("shell", 's', false)];
    let mut segs: Vec<Seg> = Vec::new();
    let mut spans: Vec<(u16, u16, char)> = Vec::new();
    for (i, (label, key, primary)) in choices.iter().enumerate() {
        let b = button_pill(look, t, label, &key.to_string(), *primary, false, bg);
        let at = segs_width(&segs);
        spans.push((at, segs_width(&b), *key));
        segs.extend(b);
        if i + 1 < choices.len() {
            segs.push(seg("  ", st));
        }
    }
    let x0 = centre(buf, cy + 4, &segs);
    for (at, w, key) in spans {
        let br = Rect { x: x0 + at, y: cy + 4, width: w, height: 1 };
        if hovered(app, br) && key != 'c' {
            let label = choices.iter().find(|c| c.1 == key).map(|c| c.0).unwrap_or("");
            put(buf, br.x, br.y, &button_pill(look, t, label, &key.to_string(), false, true, bg), br.right());
        }
        hit(app, br, HyHit::NewTabPick(key));
    }
    let mv = vec![seg("or move a pane here  ", st.fg(t.muted)), seg("m", st.fg(t.accent).add_modifier(Modifier::BOLD))];
    let mx = centre(buf, cy + 6, &mv);
    hit(app, Rect { x: mx, y: cy + 6, width: segs_width(&mv), height: 1 }, HyHit::NewTabPick('m'));
}

impl App {
    /// The leader was pressed `since` and the key map shows by itself now.
    pub(in crate::client) fn keymap_shown(&self, since: Instant) -> bool {
        self.cfg.ui.which_key && since.elapsed() >= std::time::Duration::from_millis(self.cfg.ui.which_key_delay_ms)
    }

    /// Open the key map (from the leader's `?`, or its pause).
    pub(in crate::client) fn open_keymap(&mut self, step: Option<Step>) {
        self.mode = Mode::KeyMap(Box::new(KeyMap { query: String::new(), searching: false, step, sel: 0 }));
    }

    /// Run what the key map's row `i` stands for.
    pub(in crate::client) fn keymap_run(&mut self, km: &KeyMap, i: usize) {
        if let Some(a) = keymap_actions(self, km).get(i).cloned() {
            self.mode = Mode::Normal;
            self.act(a);
        }
    }

    pub(in crate::client) fn on_keymap_key(&mut self, mut km: KeyMap, k: &KeyEvent) {
        let n = keymap_actions(self, &km).len();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                if km.query.pop().is_none() && km.step.take().is_none() {
                    self.mode = Mode::Normal;
                    return;
                }
                km.sel = 0;
                self.mode = Mode::KeyMap(Box::new(km));
            }
            KeyCode::Up | KeyCode::Down if km.step.is_some() || !km.query.is_empty() => {
                km.sel = if k.code == KeyCode::Up { km.sel.saturating_sub(1) } else { (km.sel + 1).min(n.saturating_sub(1)) };
                self.mode = Mode::KeyMap(Box::new(km));
            }
            KeyCode::Enter if km.step.is_some() || !km.query.is_empty() => {
                let sel = km.sel;
                self.keymap_run(&km, sel);
            }
            KeyCode::Tab if km.step.is_none() => {
                km.searching = !km.searching;
                if !km.searching {
                    km.query.clear();
                }
                km.sel = 0;
                self.mode = Mode::KeyMap(Box::new(km));
            }
            KeyCode::Char(c) if km.step.is_some() => {
                let Some(step) = km.step else { return };
                if let Some(i) = step_items(self, step).iter().position(|(key, ..)| *key == c) {
                    self.keymap_run(&km, i);
                } else {
                    self.mode = Mode::KeyMap(Box::new(km));
                }
            }
            // A key runs what it's bound to (unless you're searching); anything else searches.
            _ => {
                let spec = KeySpec::from_event(k);
                if !km.searching
                    && km.query.is_empty()
                    && let Some(a) = self.keymap.prefixed.get(&spec).cloned()
                {
                    self.mode = Mode::Normal;
                    self.act(a);
                    return;
                }
                if let KeyCode::Char(c) = k.code
                    && !k.modifiers.contains(KeyModifiers::CONTROL)
                    && km.query.chars().count() < QUERY_MAX
                {
                    km.query.push(c);
                    km.searching = true;
                    km.sel = 0;
                }
                self.mode = Mode::KeyMap(Box::new(km));
            }
        }
    }

    pub(in crate::client) fn on_actions_key(&mut self, sel: usize, k: &KeyEvent) {
        let items = action_items();
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Up => self.mode = Mode::Actions { sel: sel.saturating_sub(1) },
            KeyCode::Down => self.mode = Mode::Actions { sel: (sel + 1).min(items.len() - 1) },
            KeyCode::Enter => self.actions_run(sel),
            _ => {
                let spec = KeySpec::from_event(k);
                match items.iter().position(|(_, a)| self.keymap.prefixed.get(&spec) == Some(a)) {
                    Some(i) => self.actions_run(i),
                    None if self.keymap.prefixed.get(&spec) == Some(&Action::Actions) => self.mode = Mode::Normal,
                    None => {}
                }
            }
        }
    }

    pub(in crate::client) fn actions_run(&mut self, i: usize) {
        if let Some((_, a)) = action_items().into_iter().nth(i) {
            self.mode = Mode::Normal;
            self.act(a);
        }
    }

    /// Start naming a new tab of the session you're on.
    pub(in crate::client) fn new_tab_start(&mut self) {
        let Some(owner) = self.hy.tabs.get(self.hy.tab).map(|t| t.owner).or(self.focused()) else {
            self.notify("open a session first".into(), true);
            return;
        };
        let n = self.session_tabs().len();
        self.mode = Mode::NewTab(Box::new(NewTab { owner, name: String::new(), naming: true, tab: None, fallback: format!("tab {}", n + 1) }));
    }

    /// Rename the tab you're on, in its pill.
    pub(in crate::client) fn rename_tab_start(&mut self) {
        let i = self.hy.tab;
        if let Some(tab) = self.hy.tabs.get(i) {
            let (owner, name) = (tab.owner, tab.name.clone());
            self.mode = Mode::NewTab(Box::new(NewTab { owner, name, naming: true, tab: Some(i), fallback: String::new() }));
        }
    }

    pub(in crate::client) fn on_new_tab_key(&mut self, mut nt: NewTab, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if nt.naming {
            match k.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if let Some(i) = nt.tab {
                        if k.code == KeyCode::Enter
                            && let Some(tab) = self.hy.tabs.get_mut(i)
                        {
                            tab.name = nt.name.trim().to_string();
                        }
                        self.mode = Mode::Normal;
                        return;
                    }
                    // Esc keeps the name it would have had.
                    if k.code == KeyCode::Esc || nt.name.trim().is_empty() {
                        nt.name = nt.fallback.clone();
                    }
                    nt.naming = false;
                }
                KeyCode::Backspace => {
                    nt.name.pop();
                }
                KeyCode::Char(c) if !ctrl && nt.name.chars().count() < QUERY_MAX => nt.name.push(c),
                _ => {}
            }
            self.mode = Mode::NewTab(Box::new(nt));
            return;
        }
        match k.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                nt.naming = true;
                self.mode = Mode::NewTab(Box::new(nt));
            }
            KeyCode::Enter => self.new_tab_pick(nt, 'c'),
            KeyCode::Char(c @ ('c' | 'x' | 's' | 'm')) => self.new_tab_pick(nt, c),
            _ => self.mode = Mode::NewTab(Box::new(nt)),
        }
    }

    /// Start what goes in the new tab: claude (c), codex (x), a shell (s), or move the pane
    /// you're on into it (m).
    pub(in crate::client) fn new_tab_pick(&mut self, nt: NewTab, c: char) {
        let name = if nt.name.trim().is_empty() { nt.fallback.clone() } else { nt.name.trim().to_string() };
        self.mode = Mode::Normal;
        if c == 'm' {
            let Some(f) = self.focused() else { return };
            let Some(i) = self.hy.tabs.iter().position(|t| t.layout.contains(f) && t.layout.leaves().len() > 1) else {
                self.notify("move a pane from a split: this one is on its own".into(), false);
                return;
            };
            if let Some(rest) = self.hy.tabs[i].layout.clone().remove(f) {
                self.hy.tabs[i].layout = rest;
                self.hy.tabs[i].focus = self.hy.tabs[i].layout.first_leaf();
            }
            self.hy.tabs.push(HyTab { layout: crate::layout::Node::Leaf(f), focus: f, arrange: Arrange::Split, owner: nt.owner, used: 0, name });
            self.hy.tab = self.hy.tabs.len() - 1;
            return;
        }
        let dir = self.snap.terms.get(&nt.owner).map(|t| t.root.clone().unwrap_or_else(|| t.cwd.clone())).unwrap_or_else(|| self.here_dir());
        self.hy.new_tab = Some((Instant::now(), nt.owner));
        self.hy.new_tab_name = Some(name);
        let cmd = match c {
            'c' => Some("claude".to_string()),
            'x' => Some("codex".to_string()),
            _ => None,
        };
        self.hy_new_session(dir, cmd, false);
    }
}
