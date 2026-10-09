//! Dialogs: new agent, message an agent, settings, keys.

use super::*;

/// Models to pick from for `agent` ("default" first), or none when it has no choice.
pub(in crate::client) fn np_models(app: &App, agent: &str) -> Vec<String> {
    let q = app.cfg.quick.agents.iter().find(|q| q.name == agent);
    let list: Vec<String> = match q {
        Some(q) if !q.models.is_empty() => q.models.clone(),
        _ if agent == "claude" => ["opus", "sonnet", "haiku"].map(String::from).to_vec(),
        _ => Vec::new(),
    };
    if list.is_empty() {
        return list;
    }
    std::iter::once("default".to_string()).chain(list).collect()
}

/// The command line that starts `agent` with `model` (index into np_models) on `task`.
pub(in crate::client) fn np_command(app: &App, agent: &str, model: usize, task: &str) -> Option<String> {
    if agent == "shell" {
        return None;
    }
    if let Some(p) = preset_of(app, agent) {
        let m = np_models(app, &p.agent).iter().position(|m| *m == p.model).unwrap_or(0);
        return np_command(app, &p.agent, m, &p.fill(task));
    }
    let q = app.cfg.quick.agents.iter().find(|q| q.name == agent);
    let base = q.map(|q| q.command.clone()).unwrap_or_else(|| agent.to_string());
    let task = task.trim();
    let mut cmd = if task.is_empty() {
        base.replace("{prompt}", "").trim().to_string()
    } else if base.contains("{prompt}") {
        base.replace("{prompt}", &app.cfg.quote_for_shell(task))
    } else {
        format!("{base} {}", app.cfg.quote_for_shell(task))
    };
    if let Some(m) = np_models(app, agent).get(model).filter(|_| model > 0) {
        let flag = q.map(|q| q.model_flag.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| "--model".into());
        let (first, rest) = cmd.split_once(' ').map(|(a, b)| (a.to_string(), format!(" {b}"))).unwrap_or((cmd.clone(), String::new()));
        cmd = format!("{first} {flag} {m}{rest}");
    }
    Some(cmd)
}

pub(in crate::client) fn draw_new_pane(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, np: &NewPaneHy) {
    let model = app.hy_model();
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 86, 21, "New", &[], t);
    let lab = |buf: &mut Buffer, y: u16, text: &str, row: u8| {
        put(buf, r.x + 3, y, &[seg(text, Style::default().fg(if np.row == row { t.accent } else { t.muted }).bg(t.card).add_modifier(Modifier::BOLD))], r.right());
    };
    // A row of chips; the selected one is filled. Long rows scroll to keep it in view.
    let chips = |app: &mut App, buf: &mut Buffer, y: u16, items: &[String], cur: usize, mk: fn(usize) -> HyHit| {
        // The list can shrink under the selection (a session closed while this is open).
        let cur = cur.min(items.len().saturating_sub(1));
        let mut start = 0;
        let room = (r.right() - 2).saturating_sub(r.x + 14) as usize;
        while start < cur && items[start..=cur].iter().map(|s| s.width() + 3).sum::<usize>() > room {
            start += 1;
        }
        let mut x = r.x + 14;
        if start > 0 {
            put(buf, x - 2, y, &[seg("‹", Style::default().fg(t.muted).bg(t.card))], r.right());
        }
        for (i, it) in items.iter().enumerate().skip(start) {
            let txt = format!(" {it} ");
            if x + txt.width() as u16 > r.right() - 2 {
                put(buf, r.right() - 2, y, &[seg("›", Style::default().fg(t.muted).bg(t.card))], r.right());
                break;
            }
            let on = i == cur;
            let cr = Rect { x, y, width: txt.width() as u16, height: 1 };
            let st = if on {
                Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD)
            } else if hovered(app, cr) {
                Style::default().bg(t.hov).fg(t.strong)
            } else {
                Style::default().bg(t.btn).fg(t.text)
            };
            put(buf, x, y, &chip(it, st), r.right());
            hit(app, cr, mk(i));
            x += txt.width() as u16 + 1;
        }
    };
    let agents = np_agents(app);
    let agent = agents.get(np.a).cloned().unwrap_or_default();
    let muted = Style::default().fg(t.muted).bg(t.card);
    // The task: typed straight away, it's the agent's first prompt.
    lab(buf, r.y + 2, "TASK", 0);
    let tb = Rect { x: r.x + 14, y: r.y + 2, width: r.right().saturating_sub(r.x + 17), height: 1 };
    strip(buf, tb, t.card2);
    let s2 = Style::default().bg(t.card2);
    let shown = {
        let w = tb.width.saturating_sub(3) as usize;
        let n = np.task.chars().count();
        if n > w { np.task.chars().skip(n - w).collect::<String>() } else { np.task.clone() }
    };
    let mut ts = vec![seg(format!(" {shown}"), s2.fg(t.strong))];
    if np.row == 0 {
        ts.push(seg("█", s2.fg(t.accent)));
    }
    if np.task.is_empty() {
        let ph = match preset_of(app, &agent) {
            Some(p) if !p.asks() => format!(" (the preset says: {})", truncate(&p.prompt, 50)),
            Some(_) => " what should it do?".to_string(),
            None if agent == "shell" || agent.starts_with('⚙') => " (not used for this)".to_string(),
            None => " what should it do? (optional)".to_string(),
        };
        ts.push(seg(ph, s2.fg(t.muted)));
    }
    put(buf, tb.x, tb.y, &ts, tb.right());
    hit(app, tb, HyHit::NpTask);
    lab(buf, r.y + 4, "RUN", 1);
    chips(app, buf, r.y + 4, &agents, np.a, HyHit::NpRun);
    lab(buf, r.y + 6, "MODEL", 2);
    let models = if agent.starts_with('★') { Vec::new() } else { np_models(app, &agent) };
    if let Some(p) = preset_of(app, &agent) {
        let m = if p.model.is_empty() { "its default".to_string() } else { p.model.clone() };
        put(buf, r.x + 14, r.y + 6, &[seg(format!("{m} (from the preset)"), muted)], r.right());
    } else if models.is_empty() {
        put(buf, r.x + 14, r.y + 6, &[seg("its default", muted)], r.right());
    } else {
        chips(app, buf, r.y + 6, &models, np.model.min(models.len() - 1), HyHit::NpModel);
    }
    let mut projs: Vec<String> = model.iter().map(|p| p.name.clone()).collect();
    projs.push("other folder…".into());
    lab(buf, r.y + 8, "PROJECT", 3);
    chips(app, buf, r.y + 8, &projs, np.p, HyHit::NpProj);
    let place = np.place_for(&agent, app.cfg.worktree.per_agent);
    let proj = model.get(np.p);
    lab(buf, r.y + 10, "WHERE", 4);
    if proj.is_some_and(|p| p.git) && !agent.starts_with('⚙') {
        chips(app, buf, r.y + 10, &np_places(proj.unwrap()), place as usize, HyHit::NpWhere);
    } else {
        put(buf, r.x + 14, r.y + 10, &[seg(if agent.starts_with('⚙') { "as the recipe says" } else { "in the folder" }, muted)], r.right());
    }
    lab(buf, r.y + 12, "OPEN", 5);
    chips(app, buf, r.y + 12, &["full screen".into(), "beside this".into()], np.beside as usize, HyHit::NpBeside);

    // What will happen, in words.
    let branch = proj.and_then(|p| p.wts.iter().find(|w| w.main)).map(|w| w.branch.clone()).unwrap_or_default();
    let others = proj.and_then(|p| p.wts.iter().find(|w| w.main)).map(|w| w.sessions.len()).unwrap_or(0);
    let what = match proj {
        None => "Pick a folder; it becomes a project.".to_string(),
        Some(p) if p.git && !agent.starts_with('⚙') && place >= 3 => {
            let w = p.wts.iter().filter(|w| !w.main).nth(place as usize - 3);
            format!("{agent} runs in the {} worktree, on {}.", w.map(|w| w.name.as_str()).unwrap_or("?"), w.map(|w| w.branch.as_str()).unwrap_or("?"))
        }
        Some(p) if p.git && !agent.starts_with('⚙') && place == 0 => format!("{agent} gets its own new worktree in {}, on a new branch named for you.", p.name),
        Some(p) if p.git && !agent.starts_with('⚙') && place == 1 => format!(
            "Switches {} to a new branch, then starts {agent} there.{}",
            p.name,
            if others > 0 { format!(" The {others} already running in it will be on that branch too.") } else { String::new() }
        ),
        Some(p) if p.git && !agent.starts_with('⚙') => format!("{agent} runs in {} on {branch}.", p.name),
        Some(p) if agent == "shell" => format!("A shell in {}.", p.name),
        Some(p) if agent.starts_with('⚙') => {
            let r = app.cfg.recipes.iter().find(|r| Some(r.name.as_str()) == agent.strip_prefix("⚙ "));
            match r {
                Some(r) => format!("{}{}", if r.worktree && p.git { "A new worktree running " } else { "Runs " }, r.run.join(" + ")),
                None => String::new(),
            }
        }
        Some(p) if !p.git => format!("{} isn't a git repo, so {agent} runs in the folder.", p.name),
        Some(p) if app.cfg.worktree.per_agent => format!("{agent} gets its own new worktree in {}, named for you.", p.name),
        Some(p) => format!("{agent} runs in {}.", p.name),
    };
    let c = Style::default().bg(t.card);
    let what = match np_command(app, &agent, np.model, &np.task) {
        Some(cmd) if !np.task.trim().is_empty() || np.model > 0 => format!("{what}  Runs: {cmd}"),
        _ => what,
    };
    for (i, l) in crate::client::views::wrap(&what, (r.width - 6) as usize).into_iter().take(3).enumerate() {
        put(buf, r.x + 3, r.y + 14 + i as u16, &[seg(l, c.fg(t.text))], r.right() - 2);
    }
    let gx = btn(app, buf, r.x + 3, r.y + 18, if np.task.trim().is_empty() { "Open" } else { "Start" }, "Enter", BtnKind::Primary, HyHit::NpGo, r.right());
    put(buf, gx + 3, r.y + 18, &hints(t, &[("↑↓", "rows"), ("←→", "choose"), ("Esc", "close, keeps the task")]), r.right());
}

// Talk ----------------------------------------------------------------------------------------

pub(in crate::client) fn draw_talk(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, term: TermId, input: &str) {
    let model = app.hy_model();
    let found = find(&model, term).map(|(_, _, s)| s.clone());
    let (title, agent) = found.as_ref().map(|s| (s.title.clone(), s.agent.clone())).unwrap_or_default();
    let status = found.as_ref().map(|s| s.status).unwrap_or(Status::None);
    // What it last said or asks, for context.
    let context = found.as_ref().and_then(|s| s.question.clone()).or_else(|| {
        app.parsers.get(&term).and_then(|p| {
            let sc = p.screen();
            let (_, cols) = sc.size();
            sc.rows(0, cols)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.chars().all(|c| "─━-_ ".contains(c)) && !l.starts_with('❯') && !l.starts_with('>'))
                .last()
        })
    });
    // A text area, centered: wraps as you type and grows up to a point.
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let w = area.width.saturating_sub(8).min(110);
    let tw = w.saturating_sub(7) as usize;
    let mut rows: Vec<String> = Vec::new();
    for para in input.split('\n') {
        let chars: Vec<char> = para.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
        }
        for chunk in chars.chunks(tw.max(8)) {
            rows.push(chunk.iter().collect());
        }
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    let look = Look::of(&app.cfg.ui);
    let box_h = (rows.len() as u16).clamp(1, 8);
    let ctx = context.as_deref().map(|c| crate::client::views::wrap(c, w.saturating_sub(8) as usize)).unwrap_or_default();
    let ctx_h = ctx.len().min(3) as u16;
    let h = 2 + 1 + ctx_h + 2 + box_h + 4;
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + area.width.saturating_sub(w) / 2, y: area.y + area.height.saturating_sub(h) / 2, width: w, height: h };
    hit(app, area, HyHit::Close);
    hit(app, r, HyHit::Noop);
    let model = app.hy_model();
    let sub = find(&model, term).map(|(p, wt, _)| if p.git && !wt.branch.is_empty() { format!("{} · {}", p.name, wt.branch) } else { p.name.clone() }).unwrap_or_default();
    let name = found.as_ref().map(|s| s.name.clone()).unwrap_or(agent.clone());
    let mut c = Card::new(t, &name).lit(t.accent);
    c.sub = &sub;
    c.bg = t.card;
    c.state = (status != Status::None).then_some(status);
    c.close = Some(HyHit::Close);
    let cs = Style::default().bg(t.card);
    let hot = cs.fg(t.accent).add_modifier(Modifier::BOLD);
    c.foot = vec![seg("Enter", hot), seg(" send   ", cs.fg(t.text)), seg("Shift+Enter", hot), seg(" new line   ", cs.fg(t.text)), seg("Esc", hot), seg(" close", cs.fg(t.text))];
    card(app, buf, r, &c, t);
    let mut y = r.y + 2;
    if title != WAITING && !title.is_empty() {
        put(buf, r.x + 4, y, &[seg(title.clone(), cs.fg(t.muted))], r.right() - 4);
        y += 1;
    }
    let col = if status == Status::Blocked { t.strong } else { t.text };
    for l in ctx.iter().take(3) {
        let mut st = cs.fg(col);
        if status == Status::Blocked {
            st = st.add_modifier(Modifier::BOLD);
        }
        put(buf, r.x + 4, y, &[seg(l.clone(), st)], r.right() - 4);
        y += 1;
    }
    // What you're writing, in a pill (a taller box once it wraps).
    let by = r.bottom().saturating_sub(4 + box_h);
    let first = rows.len().saturating_sub(box_h as usize);
    let s = Style::default().bg(t.card2);
    for (i, l) in rows[first..].iter().enumerate() {
        let yy = by + i as u16;
        if box_h == 1 {
            row_pill(look, buf, r.x + 2, yy, r.width.saturating_sub(4), t.card2, t.card);
        } else {
            fill(buf, Rect { x: r.x + 3, y: yy, width: r.width.saturating_sub(6), height: 1 }, t.card2);
        }
        let last = first + i + 1 == rows.len();
        let mut segs = vec![seg(if i == 0 { "› " } else { "  " }, s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(l.clone(), s.fg(t.strong))];
        if last {
            segs.push(seg("█", s.fg(t.accent)));
        }
        if input.is_empty() {
            segs.push(seg(format!(" Write to {agent}…"), s.fg(t.muted)));
        }
        put(buf, r.x + 4, yy, &segs, r.right() - 4);
    }
}

// Settings ------------------------------------------------------------------------------------

/// The values a settings row offers as chips, and which one is current.
pub(in crate::client) fn chips_for(app: &App, row: &SRow) -> Option<(Vec<String>, Option<usize>)> {
    use crate::client::modal::Kind;
    let SRow::Setting(s) = row else { return None };
    let cur = crate::client::modal::current(&app.cfg, s.path);
    match s.kind {
        Kind::Bool => {
            let on = cur.and_then(|v| v.as_bool()).unwrap_or(false);
            Some((vec!["on".into(), "off".into()], Some(if on { 0 } else { 1 })))
        }
        Kind::Choice(opts) if s.path == "theme" => {
            let names: Vec<String> = crate::theme::DESIGN.iter().map(|(_, l)| l.to_string()).collect();
            let curname = cur.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let i = crate::theme::DESIGN.iter().position(|(n, _)| *n == curname || (curname == "drover" && *n == "hydra"));
            let _ = opts;
            Some((names, i))
        }
        Kind::Choice(opts) => {
            let curs = cur.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            Some((opts.iter().map(|o| o.to_string()).collect(), opts.iter().position(|o| *o == curs)))
        }
        Kind::Program(cands) => {
            let curs = cur.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let opts = crate::client::modal::program_options(cands, &curs);
            let i = if curs.is_empty() { Some(0) } else { opts.iter().position(|o| *o == curs) };
            Some((opts, i))
        }
        _ => None,
    }
}

/// Apply a chip click: the setting row's `vi`-th value.
pub(in crate::client) fn set_chip(app: &mut App, row: &SRow, vi: usize) {
    use crate::client::modal::Kind;
    let SRow::Setting(s) = row else { return };
    let v: toml_edit::Value = match s.kind {
        Kind::Bool => (vi == 0).into(),
        Kind::Choice(_) if s.path == "theme" => match crate::theme::DESIGN.get(vi) {
            Some((n, _)) => (*n).into(),
            None => return,
        },
        Kind::Choice(opts) => match opts.get(vi) {
            Some(o) => (*o).into(),
            None => return,
        },
        Kind::Program(cands) => {
            let curs = crate::client::modal::current(&app.cfg, s.path).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            match crate::client::modal::program_options(cands, &curs).get(vi) {
                Some(o) if o == "default" => "".into(),
                Some(o) => o.as_str().into(),
                None => return,
            }
        }
        _ => return,
    };
    app.save_setting(s.path, v);
}

pub(in crate::client) fn draw_settings(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, v: &SettingsView) {
    use crate::client::design::{settings_group, theme_label};
    use crate::client::modal::Cat;
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let look = Look::of(&app.cfg.ui);
    let r = panel(app, buf, area, 84, 40.min(area.height.saturating_sub(2)), "Settings", &[], t);
    let c = Style::default().bg(t.card);
    // Tabs: the current one a pill; General carries a dot when an update is ready.
    let mut x = r.x + 3;
    for (i, cat) in Cat::ALL.iter().enumerate() {
        let on = i == v.cat;
        let dot = *cat == Cat::General && app.update_available.is_some();
        let mut segs = vec![seg(format!(" {} ", cat.label()), Style::default().fg(if on { t.acc_ink } else { t.text }).add_modifier(if on { Modifier::BOLD } else { Modifier::empty() }))];
        if dot {
            segs.push(seg("● ", Style::default().fg(if on { t.acc_ink } else { t.accent }).add_modifier(Modifier::BOLD)));
        }
        let w = segs_width(&segs) + 2;
        let tr = Rect { x, y: r.y + 2, width: w, height: 1 };
        let segs = if on {
            pill(look, segs, t.accent, t.card)
        } else {
            let bg = if hovered(app, tr) { t.hov } else { t.card };
            let mut out = vec![seg(" ", c.bg(bg))];
            out.extend(segs.into_iter().map(|(s, st)| (s, st.bg(bg))));
            out.push(seg(" ", c.bg(bg)));
            out
        };
        put(buf, x, r.y + 2, &segs, r.right());
        hit(app, tr, HyHit::SetTab(i));
        x += w + 1;
    }
    // Which seshi this is, and the way to the next one.
    let mut ver = vec![seg(format!("seshi {}", env!("CARGO_PKG_VERSION")), c.fg(t.muted))];
    if let Some(new) = &app.update_available {
        ver.push(seg(format!(" → {new}  "), c.fg(t.accent).add_modifier(Modifier::BOLD)));
    }
    // Under the tabs, at the right.
    let vw = segs_width(&ver);
    let update = app.update_available.as_ref().map(|_| button_pill(look, t, if app.updating { "updating…" } else { "Update now" }, "", true, false, t.card));
    let uw = update.as_ref().map(|u| segs_width(u)).unwrap_or(0);
    let vx = r.right().saturating_sub(vw + uw + 4);
    let vy = r.y + 4;
    put(buf, vx, vy, &ver, r.right() - 3);
    if let Some(u) = update {
        put(buf, vx + vw, vy, &u, r.right() - 3);
        hit(app, Rect { x: vx + vw, y: vy, width: uw, height: 1 }, HyHit::Update);
    }
    hline(buf, r.x + 3, r.y + 3, r.width.saturating_sub(6), t, t.card);
    let cat = Cat::ALL[v.cat.min(Cat::ALL.len() - 1)];
    let rows = crate::client::design::settings_rows(cat);
    let lx = r.x + 6;
    let vx = r.x + 34;
    // What the selected row does (shown under it).
    let help = match rows.get(v.sel) {
        Some(SRow::Setting(s)) => s.help.to_string(),
        Some(SRow::Bind { acts, .. }) if acts.len() > 1 => "Several keys; change them in the config file (o).".into(),
        Some(SRow::Bind { .. }) => "Enter, then press the new key.".into(),
        Some(SRow::Theme(_)) | None => String::new(),
    };
    // Lines: headings, rows with a blank between them, a wider gap between groups. The
    // selected row's help sits in its own line at the bottom, so nothing moves.
    enum L {
        Head(&'static str),
        Row(usize),
        Blank,
    }
    let mut lines: Vec<L> = Vec::new();
    let mut last = "";
    for (i, row) in rows.iter().enumerate() {
        let g = settings_group(row);
        if g != last {
            if !last.is_empty() {
                lines.push(L::Blank);
            }
            lines.push(L::Head(g));
            lines.push(L::Blank);
            last = g;
        }
        lines.push(L::Row(i));
        lines.push(L::Blank);
    }
    let top = r.y + 5;
    // Appearance keeps room at the foot for its preview.
    let preview = cat == Cat::Appearance && r.height >= 30;
    let h = r.height.saturating_sub(if preview { 16 } else { 11 }) as usize;
    let at = lines.iter().position(|l| matches!(l, L::Row(i) if *i == v.sel)).unwrap_or(0);
    let start = (at + 2).saturating_sub(h);
    for (k, line) in lines.iter().enumerate().skip(start).take(h) {
        let y = top + (k - start) as u16;
        match line {
            L::Blank => {}
            L::Head(g) => {
                put(buf, r.x + 4, y, &[seg(*g, c.fg(t.muted).add_modifier(Modifier::BOLD))], r.right());
            }
            L::Row(i) => {
                let row = &rows[*i];
                let sel = *i == v.sel;
                let rr = Rect { x: r.x + 3, y, width: r.width.saturating_sub(6), height: 1 };
                let bg = if sel || hovered(app, rr) { t.hov } else { t.card };
                if bg != t.card {
                    row_pill(look, buf, rr.x, y, rr.width, bg, t.card);
                }
                let st = Style::default().bg(bg);
                if sel {
                    put(buf, r.x + 4, y, &[seg("›", st.fg(t.accent).add_modifier(Modifier::BOLD))], r.right());
                }
                let mut ls = st.fg(if sel { t.strong } else { t.text });
                if sel {
                    ls = ls.add_modifier(Modifier::BOLD);
                }
                hit(app, Rect { x: r.x + 1, y, width: vx - r.x - 2, height: 1 }, HyHit::SetRow(*i));
                match row {
                    // One theme: ● if it's yours, its name, eight swatches.
                    SRow::Theme(ti) => {
                        let name = crate::theme::BUILTIN[*ti];
                        let cur = if app.cfg.theme.is_empty() { crate::theme::DEFAULT } else { app.cfg.theme.as_str() };
                        let mine = cur == name || (cur == "drover" && name == "hydra");
                        put(buf, lx, y, &[seg(if mine { "●" } else { "○" }, st.fg(if mine { t.accent } else { t.muted })), seg(format!(" {}", theme_label(name)), ls)], vx - 1);
                        let th = Theme::named(name);
                        let a = th.ansi.unwrap_or([th.err, th.done, th.blocked, th.accent, th.accent, th.accent, th.muted]);
                        let mut sx = vx;
                        for col in [th.bg, th.sidebar_bg, th.accent, th.blocked, th.done, th.err, a[3], a[4]] {
                            fill(buf, Rect { x: sx, y, width: 3, height: 1 }, col);
                            sx += 4;
                        }
                        hit(app, rr, HyHit::SetVal(*i, 0));
                    }
                    _ => {
                        let label = match row {
                            SRow::Setting(s) => s.label.to_string(),
                            SRow::Bind { label, .. } => label.to_string(),
                            SRow::Theme(_) => String::new(),
                        };
                        put(buf, lx, y, &[seg(truncate(&label, (vx - lx - 2) as usize), ls)], vx - 1);
                        if let Some((opts, cur)) = chips_for(app, row).filter(|_| !(sel && v.editing.is_some())) {
                            let mut cx = vx;
                            for (vi, o) in opts.iter().enumerate() {
                                let on = Some(vi) == cur;
                                // The chosen one a pill; the others plain, the same width.
                                let segs = if on { pill(look, vec![seg(o.clone(), Style::default().fg(t.acc_ink).add_modifier(Modifier::BOLD))], t.accent, bg) } else { vec![seg(format!(" {o} "), st.fg(t.muted))] };
                                let w = segs_width(&segs);
                                if cx + w >= r.right() - 3 {
                                    break;
                                }
                                put(buf, cx, y, &segs, r.right() - 3);
                                hit(app, Rect { x: cx, y, width: w, height: 1 }, HyHit::SetVal(*i, vi));
                                cx += w + 1;
                            }
                        } else {
                            let ctrl: Vec<Seg> = crate::client::design::control(app, t, row, v, sel).into_iter().map(|(x, s)| (x, if s.bg.is_none() { s.bg(bg) } else { s })).collect();
                            put(buf, vx, y, &ctrl, r.right() - 1);
                            hit(app, Rect { x: vx, y, width: segs_width(&ctrl).max(1), height: 1 }, HyHit::SetVal(*i, usize::MAX));
                        }
                    }
                }
            }
        }
    }
    if rows.is_empty() {
        put(buf, lx, top, &[seg("Nothing to change here.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right());
    }
    // How panes look with these settings: one focused, one faded.
    if preview {
        let py = r.bottom().saturating_sub(10);
        put(buf, r.x + 4, py, &[seg("PREVIEW", c.fg(t.muted).add_modifier(Modifier::BOLD))], r.right());
        let mini = |app: &mut App, buf: &mut Buffer, mx: u16, focus: bool| {
            let label = if focus { "focused" } else { "dimmed" };
            let mut card_look = Card::new(t, "");
            card_look.bg = if focus { pane_bg(t) } else { t.card };
            if focus {
                card_look = card_look.lit(match app.cfg.ui.focus_border.as_str() {
                    "bright" => t.strong,
                    "none" => t.line,
                    _ => t.accent,
                });
            }
            let mr = Rect { x: mx, y: py + 1, width: 18, height: 3 };
            card(app, buf, mr, &card_look, t);
            let fg = if focus { t.strong } else { blend(t.text, t.card, Look::of(&app.cfg.ui).dim) };
            let mut st = Style::default().bg(card_look.bg).fg(fg);
            if focus {
                st = st.add_modifier(Modifier::BOLD);
            }
            put(buf, mx + 2, py + 2, &[seg(label, st)], mr.right() - 1);
        };
        mini(app, buf, lx, true);
        mini(app, buf, lx + 20, false);
    }
    // The selected row's help, always in the same place.
    hline(buf, r.x + 3, r.bottom() - 5, r.width.saturating_sub(6), t, t.card);
    if !help.is_empty() {
        put(buf, r.x + 3, r.bottom() - 4, &[seg(truncate(&help, (r.width - 6) as usize), c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
    }
    put(buf, r.x + 3, r.bottom() - 2, &hints(t, &[("↑↓", "move"), ("←→", "change"), ("Tab", "section")]), r.right());
    let file = vec![seg("config.toml  ", c.fg(t.muted)), seg("o", c.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" open", c.fg(t.muted))];
    let fw = segs_width(&file);
    put(buf, r.right().saturating_sub(fw + 3), r.bottom() - 2, &file, r.right());
}
