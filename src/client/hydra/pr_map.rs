//! Full-pane views: pull request, ship.

use super::*;

// ---- pull request view ---------------------------------------------------------------------

pub(in crate::client) fn check_glyph(t: &Theme, c: crate::client::pr::Checks) -> Seg {
    use crate::client::pr::Checks;
    match c {
        Checks::Pass => seg("✓", Style::default().fg(t.done)),
        Checks::Fail => seg("✕", Style::default().fg(t.err).add_modifier(Modifier::BOLD)),
        Checks::Pending => seg("…", Style::default().fg(t.muted)),
        Checks::None => seg("·", Style::default().fg(t.muted)),
    }
}

/// The little `#412 ✓` tag for a branch with a pull request.
pub(in crate::client) fn pr_tag(t: &Theme, pr: &crate::client::pr::PrBrief) -> Vec<Seg> {
    use crate::client::pr::Review;
    let mut v = vec![seg(format!(" #{} ", pr.number), Style::default().fg(t.muted)), check_glyph(t, pr.checks)];
    if pr.review == Review::Changes {
        v.push(seg("±", Style::default().fg(t.blocked).add_modifier(Modifier::BOLD)));
    } else if pr.review == Review::Approved {
        v.push(seg("✔", Style::default().fg(t.done)));
    }
    v
}

/// Wrap plain text to `width` columns.
pub(in crate::client) fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

pub(in crate::client) fn draw_pr(app: &mut App, buf: &mut Buffer, area: Rect, t: &Theme, v: &crate::client::pr::PrView) {
    fill(buf, area, t.bg);
    let title = match &v.info {
        Some(Ok(i)) => format!("#{}  {}", i.number, i.title),
        _ => format!("pull request · {}", v.which),
    };
    crate::client::design::title_bar(
        buf,
        area,
        t,
        &[seg(title, Style::default().add_modifier(Modifier::BOLD))],
        &[seg(if v.tab == 0 { "Tab diff   " } else { "Tab overview   " }, Style::default()), seg("✕ ", Style::default())],
        true,
    );
    hit(app, Rect { x: area.right().saturating_sub(2), y: area.y, width: 2, height: 1 }, HyHit::ViewKey('\x1b'));
    let body = Rect { x: area.x + 2, y: area.y + 2, width: area.width.saturating_sub(4), height: area.height.saturating_sub(5) };
    let info = match &v.info {
        None => {
            put(buf, body.x, body.y, &[seg("loading…", Style::default().fg(t.muted))], body.right());
            return;
        }
        Some(Err(e)) => {
            let msg = if e.contains("no pull requests found") { "No pull request for this branch yet. Open one from Changes (d, then p)." } else { e.as_str() };
            put(buf, body.x, body.y, &[seg(msg.to_string(), Style::default().fg(t.muted))], body.right());
            return;
        }
        Some(Ok(i)) => i,
    };
    let w = body.width as usize;
    let head = |s: &str| vec![seg(s.to_string(), Style::default().fg(t.muted).add_modifier(Modifier::BOLD))];
    let mut lines: Vec<Vec<Seg>> = Vec::new();
    if v.tab == 0 {
        let state_c = match info.state.as_str() {
            "MERGED" => t.accent,
            "CLOSED" => t.err,
            _ => t.done,
        };
        lines.push(vec![
            seg(format!(" {} ", info.state.to_lowercase()), Style::default().bg(state_c).fg(t.acc_ink).add_modifier(Modifier::BOLD)),
            seg(format!("  {} → {}", info.branch, info.base), Style::default().fg(t.text)),
            seg(format!("   +{}", info.additions), Style::default().fg(t.done)),
            seg(format!(" −{}", info.deletions), Style::default().fg(t.err)),
            seg(format!(" · {} files · by {}", info.files, info.author), Style::default().fg(t.muted)),
        ]);
        lines.push(vec![]);
        if !info.checks.is_empty() {
            let bad = info.checks.iter().filter(|(_, c)| *c == crate::client::pr::Checks::Fail).count();
            lines.push(head(&format!("CHECKS  {}", if bad > 0 { format!("{bad} failing") } else { format!("{} ok", info.checks.len()) })));
            for (name, c) in &info.checks {
                lines.push(vec![check_glyph(t, *c), seg(format!(" {name}"), Style::default().fg(t.text))]);
            }
            lines.push(vec![]);
        }
        if !info.reviews.is_empty() {
            lines.push(head("REVIEWS"));
            for (who, st, text) in &info.reviews {
                let (label, c) = match st.as_str() {
                    "APPROVED" => ("approved", t.done),
                    "CHANGES_REQUESTED" => ("asked for changes", t.blocked),
                    _ => ("commented", t.muted),
                };
                lines.push(vec![seg(who.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD)), seg(format!(" {label}"), Style::default().fg(c))]);
                for l in wrap_text(text, w.saturating_sub(4)) {
                    lines.push(vec![seg(format!("  {l}"), Style::default().fg(t.text))]);
                }
            }
            lines.push(vec![]);
        }
        if !info.comments.is_empty() {
            lines.push(head("COMMENTS"));
            for (who, text) in &info.comments {
                lines.push(vec![seg(who.clone(), Style::default().fg(t.strong).add_modifier(Modifier::BOLD))]);
                for l in wrap_text(text, w.saturating_sub(4)) {
                    lines.push(vec![seg(format!("  {l}"), Style::default().fg(t.text))]);
                }
            }
            lines.push(vec![]);
        }
        lines.push(head("DESCRIPTION"));
        let text = if info.body.trim().is_empty() { "(none)" } else { info.body.as_str() };
        for l in wrap_text(text, w) {
            lines.push(vec![seg(l, Style::default().fg(t.text))]);
        }
    } else {
        match &v.diff {
            None => lines.push(vec![seg("loading the diff…", Style::default().fg(t.muted))]),
            Some(Err(e)) => lines.push(vec![seg(e.clone(), Style::default().fg(t.err))]),
            Some(Ok(d)) => {
                let (mut old, mut new) = (0, 0);
                for l in d.lines() {
                    if let Some(rest) = l.strip_prefix("diff --git a/") {
                        let file = rest.split(" b/").next().unwrap_or(rest).to_string();
                        lines.push(vec![]);
                        lines.push(vec![seg(file, Style::default().fg(t.strong).add_modifier(Modifier::BOLD))]);
                        continue;
                    }
                    if l.starts_with("index ") || l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("new file") || l.starts_with("deleted file") {
                        continue;
                    }
                    lines.push(crate::client::design::diff_line(t, l, &mut old, &mut new));
                }
            }
        }
    }
    let start = (v.scroll as usize).min(lines.len().saturating_sub(1));
    for (i, l) in lines.iter().skip(start).take(body.height as usize).enumerate() {
        put(buf, body.x, body.y + i as u16, l, body.right());
    }
    // Actions
    let y = area.bottom().saturating_sub(1);
    strip(buf, Rect { y, height: 1, ..area }, t.card2);
    let mut x = area.x + 2;
    for (label, key, kind, c) in [
        ("Ask the agent to fix it", "f", BtnKind::Primary, 'f'),
        ("Open in browser", "o", BtnKind::Normal, 'o'),
        ("Reload", "r", BtnKind::Normal, 'r'),
    ] {
        x = btn(app, buf, x, y, label, key, kind, HyHit::ViewKey(c), area.right()) + 1;
    }
    put(buf, x + 1, y, &hints(t, &[("↑↓", "scroll"), ("Tab", if v.tab == 0 { "diff" } else { "overview" }), ("Esc", "close")]).into_iter().map(|(s, st)| (s, st.bg(t.card2))).collect::<Vec<_>>(), area.right());
}

// ---- ship ------------------------------------------------------------------------------------

pub(in crate::client) fn draw_ship(app: &mut App, f: &mut Frame, area: Rect, t: &Theme, ask: &crate::client::ShipAsk) {
    let buf = f.buffer_mut();
    dim_all(buf, area, t);
    let r = panel(app, buf, area, 66, 12, &format!("Ship {}", ask.task.branch), &[], t);
    let c = Style::default().bg(t.card);
    let mut y = r.y + 2;
    let mut step = |buf: &mut Buffer, n: u8, text: String| {
        put(buf, r.x + 3, y, &[seg(format!("{n}  "), c.fg(t.accent).add_modifier(Modifier::BOLD)), seg(text, c.fg(t.text))], r.right() - 2);
        y += 1;
    };
    let msg = if ask.task.summary.is_empty() { ask.task.branch.clone() } else { ask.task.summary.clone() };
    if ask.changed > 0 {
        step(buf, 1, format!("commit {} changed file{} as \"{}\"", ask.changed, if ask.changed == 1 { "" } else { "s" }, truncate(&msg, 30)));
    } else {
        step(buf, 1, "nothing new to commit".into());
    }
    step(buf, 2, format!("push {}", ask.task.branch));
    match &ask.pr {
        Some(n) => step(buf, 3, format!("update pull request #{n}")),
        None => step(buf, 3, format!("open a pull request into {}", ask.task.base)),
    }
    put(buf, r.x + 3, y + 1, &[seg("Checks then show next to the branch in the sidebar.", c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
    let gx = btn(app, buf, r.x + 3, r.bottom() - 2, "Ship", "Enter", BtnKind::Primary, HyHit::ShipGo, r.right());
    put(buf, gx + 3, r.bottom() - 2, &hints(t, &[("Esc", "cancel")]), r.right());
}
