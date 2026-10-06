//! The splash screen.

use super::*;

// ---- splash --------------------------------------------------------------------------------

pub(in crate::client) const BIG: [(char, [&str; 5]); 5] = [
    ('H', ["██  ██", "██  ██", "██████", "██  ██", "██  ██"]),
    ('Y', ["██  ██", "██  ██", " ████ ", "  ██  ", "  ██  "]),
    ('D', ["█████ ", "██  ██", "██  ██", "██  ██", "█████ "]),
    ('R', ["█████ ", "██  ██", "█████ ", "██ ██ ", "██  ██"]),
    ('A', [" ████ ", "██  ██", "██████", "██  ██", "██  ██"]),
];

pub(in crate::client) const ART: [&str; 22] = [
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡀⠀⠀⠀⠀⢠⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠻⣦⡀⠀⢸⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⣠⣦⣤⣀⣀⣤⣤⣀⡀⠀⣀⣠⡆⠀⠀⠀⠀⠀⠀⠤⠒⠛⣛⣛⣻⣿⣶⣾⣿⣦⣄⢿⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠸⠿⢿⣿⣿⣿⣯⣭⣿⣿⣿⣿⣋⣀⠀⠀⠀⠀⠀⠀⣠⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣤⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⡿⢿⣿⣿⣿⣿⣿⣓⠢⠄⢠⡾⢻⣿⣿⣿⣿⡟⠁⠀⠀⠈⠙⢿⣿⣿⣯⡻⣿⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠉⠀⠀⠀⠙⢿⣿⣿⣿⣷⣄⠁⠀⣿⣿⣿⣿⣿⡇⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣷⣄⡀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⣿⣿⣿⣷⣌⢧⠀⣿⣿⣿⣿⣿⣿⣄⠀⠀⠀⠀⢀⠉⠙⠛⠛⠿⣿⣿⣿⡆⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⣿⣿⣿⣿⡀⠠⢻⡟⢿⣿⣿⣿⣿⣧⣄⣀⠀⠘⢶⣄⣀⠀⠀⠈⢻⠿⠁⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣸⣿⣿⣿⣿⣾⠀⠀⠀⠻⣈⣙⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⣷⣦⡀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠈⠲⣄⠀⠀⣀⡤⠤⠀⠀⠀⢠⣿⣿⣿⡿⣿⠇⠀⠀⠐⠺⢉⣡⣴⣿⣿⣿⣿⣿⣿⣿⡿⢿⣿⣿⣿⣶⣿⣿⣿⣶⣶⡀⠀⠀⠀",
    "⠀⠀⠀⠀⢠⣿⣴⣿⣷⣶⣦⣤⡀⠀⢸⣿⣿⣿⠇⠏⠀⠀⠀⢀⣴⣿⣿⣿⣿⣿⠟⢿⣿⣿⣿⣷⠀⠹⣿⣿⠿⠿⠛⠻⠿⣿⠇⠀⠀⠀",
    "⠀⠀⠀⣠⣿⣿⣿⣿⣿⣿⣿⣷⣯⡂⢸⣿⣿⣿⠀⠀⠀⠀⢀⠾⣻⣿⣿⣿⠟⠀⠀⠈⣿⣿⣿⣿⡇⠀⠀⣀⣀⡀⠀⢠⡞⠉⠀⠀⠀⠀",
    "⠀⠀⢸⣟⣽⣿⣯⠀⠀⢹⣿⣿⣿⡟⠼⣿⣿⣿⣇⠀⠀⠀⠠⢰⣿⣿⣿⣿⡄⠀⠀⠀⣸⣿⣿⣿⡇⠀⢀⣤⣼⣿⣷⣾⣷⡀⠀⠀⠀⠀",
    "⠀⢀⣾⣿⡿⠟⠋⠀⠀⢸⣿⣿⣿⣿⡀⢿⣿⣿⣿⣦⠀⠀⠀⢺⣿⣿⣿⣿⣿⣄⠀⠀⣿⣿⣿⣿⡇⠐⣿⣿⣿⣿⠿⣿⣿⡿⣦⠀⠀⠀",
    "⠀⢻⣿⠏⠀⠀⠀⠀⢠⣿⣿⣿⡟⡿⠀⠀⢻⣿⣿⣿⣷⣤⡀⠘⣷⠻⣿⣿⣿⣿⣷⣼⣿⣿⣿⣿⣇⣾⣿⣿⣿⠁⠀⢼⣿⣿⣿⣆⠀⠀",
    "⠀⠀⠈⠀⠀⠀⠀⠀⢸⣿⣿⣿⡗⠁⠀⠀⠀⠙⢿⣿⣿⣿⣿⣷⣾⣆⡙⣿⣿⣿⣿⣿⣿⣿⣿⣿⠌⣾⣿⣿⣿⣆⠀⠀⠀⠉⠻⣿⡷⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣷⣄⠀⠀⠀⠀⠀⠈⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠘⣟⣿⣿⣿⡆⠀⠀⠀⠀⠙⠁⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠻⣿⣿⣿⣿⣿⣶⣤⣤⣤⣀⣠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠀⠀⠀⢈⣿⣿⣿⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⣠⣤⣤⣶⣿⣿⣿⠟⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⢀⣠⣤⣄⠀⠠⢶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⡁⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⢀⣀⠀⣠⣀⡠⠞⣿⣿⣿⣿⣶⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣴⣿⣷⣦⣄⣀⢿⡽⢻⣦",
    "⠻⠶⠾⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿⠋",
];

/// The splash: the hydra, the wordmark, what happened while you were away, and buttons.
/// The splash's choices: (label, key shown, action key).
pub(in crate::client) fn splash_options(app: &App) -> Vec<(String, String, char)> {
    let mut v = Vec::new();
    if !app.snap.terms.is_empty() {
        let n = app.snap.terms.len();
        v.push((format!("Resume where you left off  ·  {n} running"), "r".to_string(), 'r'));
    }
    v.push(("New session".to_string(), "n".to_string(), 'n'));
    v
}

pub(in crate::client) fn draw_splash(app: &mut App, f: &mut Frame, area: Rect, t: &Theme) {
    let model = app.hy_model();
    let buf = f.buffer_mut();
    fill(buf, area, t.bg);
    let all: Vec<&Session> = model.iter().flat_map(|p| p.sessions()).collect();
    let n = |st: Status| all.iter().filter(|s| s.status == st).count();
    let art_w = ART[0].chars().count() as u16;
    // The art needs 22 rows; small windows get the wordmark alone.
    let show_art = area.height >= 22 + 16 && area.width >= art_w + 4;
    let body_h = if show_art { 23 } else { 0 } + 18;
    let mut y = area.y + area.height.saturating_sub(body_h) / 2;
    let center = |w: u16| area.x + area.width.saturating_sub(w) / 2;
    let teal = t.teal();
    if show_art {
        let ax = center(art_w);
        for (r, line) in ART.iter().enumerate() {
            let col = blend(t.accent, teal, r as f32 / (ART.len() - 1) as f32);
            for (ci, ch) in line.chars().enumerate() {
                if ch == '\u{2800}' || ch == ' ' {
                    continue;
                }
                if let Some(px) = buf.cell_mut((ax + ci as u16, y + r as u16)) {
                    px.set_char(ch).set_style(Style::default().fg(col).bg(t.bg));
                }
            }
        }
        y += ART.len() as u16 + 1;
    }
    // HYDRA in block letters, accent → teal left to right.
    let lw = 5 * 6 + 4 * 2;
    let lx = center(lw);
    for (k, (_, rows)) in BIG.iter().enumerate() {
        for (r, row) in rows.iter().enumerate() {
            for (ci, ch) in row.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let col = k as u16 * 8 + ci as u16;
                let c = blend(t.accent, teal, col as f32 / lw as f32);
                if let Some(px) = buf.cell_mut((lx + col, y + r as u16)) {
                    px.set_char(ch).set_style(Style::default().fg(c).bg(t.bg));
                }
            }
        }
    }
    y += 6;
    let tag = "many heads, one body · your agents keep going when you leave";
    put(buf, center(tag.width() as u16), y, &[seg(tag, Style::default().fg(t.muted).bg(t.bg).add_modifier(Modifier::ITALIC))], area.right());
    y += 2;
    let away = vec![
        seg("while you were away   ", Style::default().fg(t.muted)),
        seg(format!("{} {} need you", app.cfg.icons.blocked, n(Status::Blocked)), Style::default().fg(t.blocked).add_modifier(Modifier::BOLD)),
        seg("   ", Style::default()),
        seg(format!("{} {} still working", glyph(app, Status::Working), n(Status::Working)), Style::default().fg(t.text)),
        seg("   ", Style::default()),
        seg(format!("{} {} finished", app.cfg.icons.done, n(Status::Done)), Style::default().fg(t.done)),
    ];
    // Nothing running, nothing to report: no line of zeros.
    if !app.snap.terms.is_empty() {
        put(buf, center(segs_width(&away)), y, &away, area.right());
    }
    y += 3;
    let last = app.focused().and_then(|fo| find(&model, fo)).map(|(p, w, s)| (p.name.clone(), p.color, w.name.clone(), s.title.clone()));
    // A short list, one under the other: Resume (when there's something to go back to),
    // New. ↑↓ choose, Enter or the letter picks.
    let opts = splash_options(app);
    let wmax = opts.iter().map(|(l, ..)| l.width() as u16).max().unwrap_or(10) + 10;
    let x = center(wmax);
    let sel = app.hy.splash_sel.min(opts.len().saturating_sub(1));
    for (i, (label, key, c)) in opts.iter().enumerate() {
        let yy = y + i as u16 * 2;
        let br = Rect { x, y: yy, width: wmax, height: 1 };
        let on = i == sel;
        let hov = hovered(app, br);
        let (bg, fg, kf) = if on { (t.accent, t.acc_ink, t.acc_ink) } else if hov { (t.hov, t.strong, t.accent) } else { (t.btn, t.strong, t.accent) };
        fill(f.buffer_mut(), br, bg);
        put(f.buffer_mut(), x + 2, yy, &[seg(label.clone(), Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD))], br.right());
        let kw = key.width() as u16;
        put(f.buffer_mut(), br.right().saturating_sub(kw + 2), yy, &[seg(key.clone(), Style::default().bg(bg).fg(kf).add_modifier(Modifier::BOLD))], br.right());
        hit(app, br, HyHit::SplashKey(*c));
    }
    let hy = y + opts.len() as u16 * 2 + 1;
    put(f.buffer_mut(), center(36), hy, &[seg("↑↓ choose   Enter open   or its key", Style::default().fg(t.muted).bg(t.bg))], area.right());
    // Status line: version and where you were.
    let sy = area.bottom().saturating_sub(1);
    let buf = f.buffer_mut();
    fill(buf, Rect { y: sy, height: 1, ..area }, t.sidebar_bg);
    let s = Style::default().bg(t.sidebar_bg);
    let mut left = vec![seg(format!("hydra {}", env!("CARGO_PKG_VERSION")), s.fg(t.muted))];
    if let Some(v) = &app.update_available {
        left.push(seg(format!("   {v} is out: Ctrl+Space p, then Update hydra"), s.fg(t.accent).add_modifier(Modifier::BOLD)));
    }
    if let Some((pn, pc, wn, title)) = last {
        left.push(seg("   last: ", s.fg(t.muted)));
        left.push(seg("▌", s.fg(pc)));
        left.push(seg(pn, s.fg(t.strong).add_modifier(Modifier::BOLD)));
        left.push(seg(format!(" › {wn} › {title}"), s.fg(t.text)));
    }
    put(buf, area.x + 1, sy, &left, area.right().saturating_sub(10));
    let rr = vec![seg("?", s.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" keys ", s.fg(t.muted))];
    let rw = segs_width(&rr);
    put(buf, area.right().saturating_sub(rw), sy, &rr, area.right());
    hit(app, Rect { x: area.right().saturating_sub(rw), y: sy, width: rw, height: 1 }, HyHit::SplashKey('?'));
}
