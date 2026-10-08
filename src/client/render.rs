//! Drawing: sidebar, tab bar, panes, status bar and overlays.

use super::copy::Copy;
use super::modal;
use super::{App, Mode, PickTarget};
use crate::protocol::Status;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The smallest window hydra draws into (below it, a note to make it bigger).
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 10;

pub fn draw(app: &mut App, f: &mut Frame) {
    app.hits.clear();
    app.panes.clear();
    app.pane_frames.clear();
    let area = f.area();
    let t = app.theme.clone();
    set_palette(t.ansi, t.ansi.map(|_| t.card2));
    // The terminal's own background (its window padding) matches ours while we run.
    if let Color::Rgb(r, g, b) = t.bg
        && app.osc_bg != Some(t.bg)
        && !cfg!(test)
    {
        use std::io::Write;
        let _ = write!(std::io::stdout(), "]11;#{r:02x}{g:02x}{b:02x}");
        app.osc_bg = Some(t.bg);
    }

    // Smaller than this, nothing fits: say so instead of drawing a broken screen.
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let buf = f.buffer_mut();
        super::design::fill(buf, area, t.bg);
        let msg = "make the window bigger";
        let x = area.x + area.width.saturating_sub(msg.width() as u16) / 2;
        super::design::put(buf, x, area.y + area.height / 2, &[super::design::seg(msg, Style::default().fg(t.muted).bg(t.bg))], area.right());
        return;
    }
    if app.splash {
        super::hydra::draw_splash(app, f, area, &t);
        return;
    }
    super::hydra::draw(app, f, area, &t);
    draw_overlays(app, f, area, &t);
}

fn draw_overlays(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme) {
    let t = t.clone();
    // Copy mode holds the whole history; match by reference and clone only small fields.
    match &app.mode {
        Mode::Prefix { since } => {
            if app.cfg.ui.which_key && since.elapsed() >= Duration::from_millis(app.cfg.ui.which_key_delay_ms) {
                super::hydra::draw_keys(app, f, area, &t);
            }
        }
        Mode::Help { .. } => super::hydra::draw_keys(app, f, area, &t),
        Mode::Talk { term, input } => {
            let (term, input) = (*term, input.clone());
            super::hydra::draw_talk(app, f, area, &t, term, &input);
        }
        Mode::Confirm(c) => {
            let c = (**c).clone();
            super::menu::draw_confirm(app, f, area, &t, &c);
        }
        Mode::GoTo { query, sel } => {
            let (q, s) = (query.clone(), *sel);
            super::hydra::draw_goto(app, f, area, &t, &q, s);
        }
        Mode::History { sel } => {
            let sel = *sel;
            super::hydra::draw_history(app, f, area, &t, sel);
        }
        Mode::Memory { sel } => {
            let sel = *sel;
            super::hydra::draw_memory(app, f, area, &t, sel);
        }
        Mode::Branch(v) => {
            let v = (**v).clone();
            super::branch::draw_branches(app, f, area, &t, &v);
        }
        Mode::Find(v) => {
            let v = (**v).clone();
            super::find::draw_find(app, f, area, &t, &v);
        }
        Mode::HyMenu(m) => {
            let m = (**m).clone();
            super::menu::draw_menu(app, f, area, &t, &m);
        }
        Mode::Ideas(v) => {
            let v = (**v).clone();
            super::work::draw_ideas(app, f, area, &t, &v);
        }
        Mode::Tickets(v) => {
            let v = (**v).clone();
            super::work::draw_tickets(app, f, area, &t, &v);
        }
        Mode::RaceNew(v) => {
            let v = (**v).clone();
            super::work::draw_race_new(app, f, area, &t, &v);
        }
        Mode::Checkpoints(v) => {
            let v = (**v).clone();
            super::history::draw_checkpoints(app, f, area, &t, &v);
        }
        Mode::Chats(v) => {
            let v = (**v).clone();
            super::history::draw_chats(app, f, area, &t, &v);
        }
        Mode::Race(v) => {
            let v = (**v).clone();
            super::work::draw_race(app, f, area, &t, &v);
        }
        Mode::Ship(ask) => {
            let ask = (**ask).clone();
            super::hydra::draw_ship(app, f, area, &t, &ask);
        }
        Mode::Finder(fd) => {
            let fd = (**fd).clone();
            super::hydra::draw_finder(app, f, area, &t, &fd);
        }
        Mode::HyPane(np) => {
            let np = np.clone();
            super::hydra::draw_new_pane(app, f, area, &t, &np);
        }
        Mode::HySettings(v) => {
            let v = (**v).clone();
            super::hydra::draw_settings(app, f, area, &t, &v);
        }
        Mode::Picker { query, sel, commands } => {
            let (query, sel, commands) = (query.clone(), *sel, *commands);
            draw_picker(app, f, area, &t, &query, sel, commands)
        }
        Mode::Quick(q) => {
            let q = q.clone();
            draw_quick(app, f, area, &t, &q)
        }
        Mode::Toolbox(_) => {
            let v = toolbox_panel(app, &t);
            draw_panel(app, f, area, &t, v);
        }
        Mode::Prompt { kind, input } => {
            let (label, input, confirm) = (kind.label().to_string(), input.clone(), kind.is_confirm());
            draw_prompt(app, f, area, &t, &label, &input, confirm)
        }
        Mode::Worktrees { items, query, sel, .. } => {
            let rows = app.worktree_rows(items.as_deref(), query);
            let (loading, query, sel) = (items.is_none(), query.clone(), *sel);
            draw_worktrees(app, f, area, &t, &rows, &query, sel, loading);
        }
        _ => {}
    }
}

/// Mix `b` into `a` by `t` (0..1). Non-RGB colours can't be mixed and fall back.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black | Color::Reset => Some((12, 12, 16)),
        Color::White => Some((240, 240, 240)),
        _ => None,
    };
    match (rgb(a), rgb(b)) {
        (Some(x), Some(y)) => {
            let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t.clamp(0.0, 1.0)).round() as u8;
            Color::Rgb(m(x.0, y.0), m(x.1, y.1), m(x.2, y.2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// Background of the row the keyboard is on: clearly stronger than a hover or the
/// active-workspace tint.
pub(super) fn focus_bg(t: &crate::theme::Theme) -> Color {
    blend(t.selection_bg, t.accent, 0.30)
}

/// Mark a list row as the selected one: an accent bar at the left edge, the focus band
/// behind every cell, and bold text. The bar takes the place of the row's leading space so
/// nothing shifts sideways when the selection moves.
pub(super) fn focus_row(line: Line<'static>, t: &crate::theme::Theme) -> Line<'static> {
    let bg = focus_bg(t);
    let mut spans: Vec<Span<'static>> = line.spans;
    if let Some(first) = spans.first_mut()
        && first.content.starts_with(' ')
    {
        first.content = first.content[1..].to_string().into();
    }
    spans.insert(0, Span::styled("▌", Style::default().fg(t.accent)));
    Line::from(
        spans
            .into_iter()
            .map(|sp| {
                let style = sp.style.bg(bg).add_modifier(Modifier::BOLD);
                Span::styled(sp.content, style)
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn status_icon(app: &App, s: Status) -> String {
    let i = &app.cfg.icons;
    match s {
        Status::Working if !app.cfg.ui.spinner.is_empty() => {
            let n = app.cfg.ui.spinner.len() as u64;
            app.cfg.ui.spinner[(app.spinner_frame() % n) as usize].clone()
        }
        Status::Working => i.working.clone(),
        Status::Blocked => i.blocked.clone(),
        Status::Done => i.done.clone(),
        Status::Idle => i.idle.clone(),
        Status::None => i.shell.clone(),
    }
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + 1 >= max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

pub(super) fn render_copy(c: &Copy, area: Rect, buf: &mut Buffer, t: &crate::theme::Theme) {
    let select = Style::default().bg(t.accent).fg(t.bg);
    let hit = Style::default().bg(t.working).fg(Color::Black);
    for row in 0..area.height {
        let li = c.top + row as usize;
        let Some(line) = c.lines.get(li) else { break };
        let y = area.y + row;
        let matches = c.matches(li);
        let mut x = area.x;
        let mut n = 0;
        for (ci, ch) in line.chars().enumerate() {
            n = ci + 1;
            let w = ch.width().unwrap_or(0) as u16;
            if w == 0 {
                continue;
            }
            if x + w > area.right() {
                break;
            }
            let mut style = Style::default().fg(t.fg);
            if matches.iter().any(|(s, e)| ci >= *s && ci < *e) {
                style = hit;
            }
            if c.selected(li, ci) {
                style = select;
            }
            if (li, ci) == c.cur {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let mut tmp = [0u8; 4];
            if let Some(px) = buf.cell_mut((x, y)) {
                px.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
            }
            x += w;
        }
        // Show the cursor (and line selections) past the end of the text.
        if x < area.right() {
            if c.cur.0 == li && c.cur.1 >= n {
                if let Some(px) = buf.cell_mut((x, y)) {
                    px.set_symbol(" ").set_style(Style::default().add_modifier(Modifier::REVERSED));
                }
            } else if c.selected(li, n)
                && let Some(px) = buf.cell_mut((x, y)) {
                    px.set_symbol(" ").set_style(select);
                }
        }
    }
    let bar = if let Some(input) = &c.input {
        Some((format!("{}{input}", if c.backward { "?" } else { "/" }), t.accent))
    } else {
        c.message.clone().map(|m| (m, t.blocked))
    };
    if let Some((text, color)) = bar
        && area.height > 0
    {
        let r = Rect { y: area.bottom() - 1, height: 1, ..area };
        Paragraph::new(Span::styled(text, Style::default().fg(t.bg).bg(color)))
            .style(Style::default().bg(color))
            .render(r, buf);
    }
}

/// The theme's terminal colours, if it has its own (set each frame from the theme).
static PALETTE: std::sync::RwLock<Option<[Color; 7]>> = std::sync::RwLock::new(None);

/// What programs' "bright black" backgrounds become: a subtle panel, not a grey slab.
static PANEL: std::sync::RwLock<Option<Color>> = std::sync::RwLock::new(None);

pub(super) fn set_palette(p: Option<[Color; 7]>, panel: Option<Color>) {
    if let Ok(mut w) = PALETTE.write() {
        *w = p;
    }
    if let Ok(mut w) = PANEL.write() {
        *w = panel;
    }
}

/// Map the basic ANSI colours (red … cyan, and gray) to the theme's.
fn themed(c: Color, pal: &Option<[Color; 7]>) -> Color {
    match (c, pal) {
        (Color::Indexed(i @ 1..=6), Some(p)) => p[i as usize - 1],
        (Color::Indexed(i @ 9..=14), Some(p)) => p[i as usize - 9],
        (Color::Indexed(8), Some(p)) => p[6],
        _ => c,
    }
}

pub(super) fn vt_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Draw a terminal screen. Cells with the default background get `default_bg` (the
/// workspace tint); programs' own background colours are left alone.
pub(super) fn render_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer, default_bg: Color) {
    let (rows, cols) = screen.size();
    let pal = PALETTE.read().map(|p| *p).unwrap_or(None);
    let panel = PANEL.read().map(|p| *p).unwrap_or(None);
    for row in 0..area.height.min(rows) {
        for col in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut fg = themed(vt_color(cell.fgcolor()), &pal);
            let mut bg = match vt_color(cell.bgcolor()) {
                Color::Reset => default_bg,
                Color::Indexed(8) if panel.is_some() => panel.unwrap_or(default_bg),
                c => c,
            };
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
                if fg == Color::Reset {
                    fg = Color::Black;
                }
                if bg == Color::Reset {
                    bg = Color::White;
                }
            }
            let mut style = Style::default().fg(fg).bg(bg);
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.dim() {
                style = style.add_modifier(Modifier::DIM);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            let contents = cell.contents();
            let c = &mut buf[(area.x + col, area.y + row)];
            c.set_symbol(if contents.is_empty() { " " } else { contents });
            c.set_style(style);
        }
    }
}

pub(super) fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn overlay_block<'a>(t: &crate::theme::Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent))
        .style(Style::default().bg(t.sidebar_bg).fg(t.fg))
        .title(Span::styled(format!(" {title} "), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))
}

fn draw_picker(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, query: &str, sel: usize, commands: bool) {
    if commands {
        let items = app.pick_items(query, true);
        return super::hydra::draw_palette(app, f, area, t, query, sel, &items);
    }
    let items = app.pick_items(query, commands);
    let r = centered(area, 80, 22);
    f.render_widget(Clear, r);
    let block = overlay_block(t, if commands { "command palette" } else { "jump to" });
    let inner = block.inner(r);
    f.render_widget(block, r);
    let input = Line::from(vec![Span::styled("› ", Style::default().fg(t.accent)), Span::raw(query.to_string())]);
    f.render_widget(Paragraph::new(input), Rect { height: 1, ..inner });
    f.set_cursor_position(Position::new(inner.x + 2 + query.width() as u16, inner.y));
    let list_h = inner.height.saturating_sub(2) as usize;
    let start = sel.saturating_sub(list_h.saturating_sub(1));
    for (i, item) in items.iter().enumerate().skip(start).take(list_h) {
        let y = inner.y + 2 + (i - start) as u16;
        let selected = i == sel;
        let selected_row = selected;
        let bg = if selected_row { focus_bg(t) } else { t.sidebar_bg };
        let (icon, color) = match &item.target {
            PickTarget::Workspace(id) => {
                ("■".to_string(), app.snap.workspace(*id).map(|w| app.ws_color(w)).unwrap_or(t.muted))
            }
            PickTarget::Pane(_) => (status_icon(app, item.status), t.status(item.status)),
            PickTarget::Command(_) => ("›".to_string(), t.accent),
            PickTarget::Ext(..) => ("◆".to_string(), t.accent),
        };
        let line = if let PickTarget::Command(_) = item.target {
            let key_w = item.key.width();
            let label_w = (inner.width as usize).saturating_sub(key_w + 5);
            let label = truncate(&item.label, label_w);
            let pad = label_w.saturating_sub(label.width());
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color).bg(bg)),
                Span::styled(label, Style::default().fg(t.fg).bg(bg)),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(format!("{} ", item.key), Style::default().fg(t.muted).bg(bg)),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color).bg(bg)),
                Span::styled(format!("{:<20} ", truncate(&item.label, 20)), Style::default().fg(t.fg).bg(bg).add_modifier(Modifier::BOLD)),
                Span::styled(truncate(&item.detail, inner.width.saturating_sub(26) as usize), Style::default().fg(t.muted).bg(bg)),
            ])
        };
        f.render_widget(Paragraph::new(if selected_row { focus_row(line, t) } else { line }).style(Style::default().bg(bg)), Rect { y, height: 1, ..inner });
    }
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  no matches", Style::default().fg(t.muted))),
            Rect { y: inner.y + 2, height: 1, ..inner },
        );
    }
}

/// Wrap text into lines of at most `width` columns (by character, keeping explicit newlines).
fn wrap_chars(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut w = 0;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(String::new());
            w = 0;
            continue;
        }
        let cw = ch.width().unwrap_or(0);
        if w + cw > width.max(1) {
            lines.push(String::new());
            w = 0;
        }
        lines.last_mut().unwrap().push(ch);
        w += cw;
    }
    lines
}

fn draw_quick(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, q: &modal::Quick) {
    use super::design::{fill, put, seg};
    let agent = if q.place == modal::Place::Here {
        app.focused().and_then(|id| app.snap.terms.get(&id)).map(|ti| ti.display_name().to_string()).unwrap_or_default()
    } else {
        app.cfg.quick.agents.get(q.agent).map(|a| a.name.clone()).unwrap_or_else(|| "?".into())
    };
    let ws = app.active_ws();
    let ws_name = ws.map(|w| w.name.clone()).unwrap_or_default();
    let buf = f.buffer_mut();
    super::hydra::dim_all(buf, area, t);
    let r = super::hydra::panel(app, buf, area, 76, 13, "Quick prompt", &[], t);
    let c = Style::default().bg(t.card);
    let chip = |s: String| seg(format!(" {s} "), Style::default().fg(t.strong).bg(t.btn).add_modifier(Modifier::BOLD));
    let head = vec![chip(agent), seg("  in  ", c.fg(t.muted)), chip(q.place.label().into()), seg("  of  ", c.fg(t.muted)), chip(ws_name)];
    put(buf, r.x + 3, r.y + 2, &head, r.right().saturating_sub(2));
    let text_area = Rect { x: r.x + 2, y: r.y + 4, width: r.width.saturating_sub(4), height: r.height.saturating_sub(7) };
    fill(buf, text_area, t.card2);
    let lines = wrap_chars(&q.text, text_area.width.saturating_sub(4) as usize);
    let visible = text_area.height as usize;
    let skip = lines.len().saturating_sub(visible);
    let s2 = Style::default().bg(t.card2);
    for (i, l) in lines.iter().skip(skip).enumerate() {
        let prefix = if i == 0 && skip == 0 { "› " } else { "  " };
        put(buf, text_area.x + 1, text_area.y + i as u16, &[seg(prefix, s2.fg(t.accent)), seg(l.clone(), s2.fg(t.strong))], text_area.right());
    }
    if q.text.is_empty() {
        put(buf, text_area.x + 1, text_area.y, &[seg("› ", s2.fg(t.accent)), seg("describe the task…", s2.fg(t.muted))], text_area.right());
    }
    let last = lines.last().map(|l| l.width()).unwrap_or(0) as u16;
    let row = (lines.len() - skip).saturating_sub(1) as u16;
    f.set_cursor_position(Position::new(text_area.x + 3 + last, text_area.y + row));
    let keys = super::hydra::hints(t, &[("Enter", "start"), ("Tab", "agent"), ("Shift+Tab", "where"), ("Alt+Enter", "new line"), ("Esc", "cancel")]);
    put(f.buffer_mut(), r.x + 3, r.bottom().saturating_sub(2), &keys, r.right());
}

#[allow(clippy::too_many_arguments)]
fn draw_worktrees(
    app: &mut App,
    f: &mut Frame,
    area: Rect,
    t: &crate::theme::Theme,
    rows: &[super::WtRow],
    query: &str,
    sel: usize,
    loading: bool,
) {
    use super::design::{put, seg};
    let buf = f.buffer_mut();
    let (r, list, start) = super::hydra::query_list(app, buf, area, t, "Worktrees", 84, rows.len().max(12), query, "pick one, or type a new branch", sel);
    let c = Style::default().bg(t.card);
    if loading {
        put(buf, list.x + 2, list.y, &[seg("reading worktrees…", c.fg(t.muted).add_modifier(Modifier::ITALIC))], list.right());
    } else if rows.is_empty() {
        put(buf, list.x + 2, list.y, &[seg("type a branch name to make a worktree", c.fg(t.muted).add_modifier(Modifier::ITALIC))], list.right());
    }
    for (i, row) in rows.iter().enumerate().skip(start).take(list.height as usize) {
        let y = list.y + (i - start) as u16;
        let rr = Rect { y, height: 1, ..list };
        let on = i == sel;
        let st = super::hydra::list_row(app, f.buffer_mut(), rr, on, t);
        let line = match row {
            super::WtRow::Existing(w) => {
                let open = app.snap.workspaces.iter().any(|x| super::same_dir(&x.cwd, &w.path));
                let mut l = vec![
                    seg(if open { "● " } else { "○ " }, st.fg(if open { t.accent } else { t.muted })),
                    seg(truncate(&w.branch, 28), st.fg(if on { t.strong } else { t.text }).add_modifier(Modifier::BOLD)),
                ];
                if w.main {
                    l.push(seg("  main", st.fg(t.accent)));
                }
                l.push(seg(format!("   {}", truncate(&w.path.display().to_string(), rr.width.saturating_sub(42) as usize)), st.fg(t.muted)));
                l
            }
            super::WtRow::Create(text) => vec![seg("+ ", st.fg(t.done)), seg("new worktree ", st.fg(t.text)), seg(text.clone(), st.fg(t.done).add_modifier(Modifier::BOLD))],
        };
        put(f.buffer_mut(), rr.x + 3, y, &line, rr.right().saturating_sub(1));
    }
    let keys = super::hydra::hints(t, &[("↑↓", "move"), ("Enter", "open"), ("Esc", "close")]);
    put(f.buffer_mut(), r.x + 3, r.bottom().saturating_sub(2), &keys, r.right());
}

fn draw_prompt(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, label: &str, input: &str, confirm: bool) {
    use super::design::{fill, put, seg};
    let buf = f.buffer_mut();
    super::hydra::dim_all(buf, area, t);
    let title = label.trim().trim_end_matches(':');
    let mut chars = title.chars();
    let title: String = chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
    let r = super::hydra::panel(app, buf, area, 64, 7, &title, &[], t);
    if confirm {
        let keys = super::hydra::hints(t, &[("y", "yes"), ("n", "no")]);
        put(buf, r.x + 3, r.y + 3, &keys, r.right());
        return;
    }
    let field = Rect { x: r.x + 2, y: r.y + 2, width: r.width.saturating_sub(4), height: 1 };
    fill(buf, field, t.card2);
    let s2 = Style::default().bg(t.card2);
    put(buf, field.x + 1, field.y, &[seg("› ", s2.fg(t.accent).add_modifier(Modifier::BOLD)), seg(input.to_string(), s2.fg(t.strong)), seg("█", s2.fg(t.accent))], field.right());
    let keys = super::hydra::hints(t, &[("Enter", "save"), ("Esc", "cancel")]);
    put(buf, r.x + 3, r.bottom().saturating_sub(2), &keys, r.right());
}

// ---- panels: files, tasks, inbox, toolbox -------------------------------------------------

/// What a two-column panel shows: a filterable list on the left, details on the right.
struct PanelView {
    title: String,
    tabs: Vec<String>,
    tab: usize,
    /// None hides the search line (lists that don't filter).
    query: Option<String>,
    rows: Vec<Line<'static>>,
    sel: Option<usize>,
    detail: Vec<Line<'static>>,
    detail_scroll: u16,
    footer: Line<'static>,
    /// Shown instead of rows when there are none (also used for "loading…").
    empty: String,
    left_pct: u16,
}

fn draw_panel(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, v: PanelView) {
    use super::design::{fill, put, seg};
    let w = (area.width.saturating_mul(92) / 100).clamp(60.min(area.width), area.width);
    let h = (area.height.saturating_mul(88) / 100).clamp(16.min(area.height), area.height);
    let title = {
        let mut cs = v.title.chars();
        cs.next().map(|c| c.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default()
    };
    let buf = f.buffer_mut();
    super::hydra::dim_all(buf, area, t);
    let r = super::hydra::panel(app, buf, area, w, h, &title, &[], t);
    let c = Style::default().bg(t.card);
    let inner = Rect { x: r.x + 2, y: r.y + 2, width: r.width.saturating_sub(4), height: r.height.saturating_sub(3) };
    let mut y = inner.y;
    if !v.tabs.is_empty() {
        let mut x = inner.x + 1;
        for (i, name) in v.tabs.iter().enumerate() {
            let on = i == v.tab;
            let txt = format!(" {name} ");
            let st = if on { Style::default().bg(t.accent).fg(t.acc_ink).add_modifier(Modifier::BOLD) } else { c.fg(t.text) };
            x = put(buf, x, y, &[seg(txt, st)], inner.right()) + 2;
        }
        put(buf, x, y, &[seg("Tab", c.fg(t.accent).add_modifier(Modifier::BOLD)), seg(" switches", c.fg(t.muted))], inner.right());
        y += 2;
    }
    if let Some(q) = &v.query {
        let field = Rect { x: inner.x, y, width: inner.width, height: 1 };
        fill(buf, field, t.card2);
        let s2 = Style::default().bg(t.card2);
        let mut qs = vec![seg("› ", s2.fg(t.accent).add_modifier(Modifier::BOLD)), seg(q.clone(), s2.fg(t.strong)), seg("█", s2.fg(t.accent))];
        if q.is_empty() {
            qs.push(seg(" type to filter", s2.fg(t.muted)));
        }
        put(buf, field.x + 1, y, &qs, field.right());
        y += 2;
    }
    let body = Rect { y, height: inner.bottom().saturating_sub(y + 1), ..inner };
    let left_w = body.width * v.left_pct / 100;
    let left = Rect { width: left_w, ..body };
    let right = Rect { x: body.x + left_w + 2, width: body.width.saturating_sub(left_w + 2), ..body };
    for yy in body.y..body.bottom() {
        if let Some(px) = buf.cell_mut((body.x + left_w, yy)) {
            px.set_symbol("│").set_style(Style::default().fg(t.line).bg(t.card));
        }
    }
    if v.rows.is_empty() {
        f.render_widget(
            Paragraph::new(v.empty.clone()).style(c.fg(t.muted)).wrap(ratatui::widgets::Wrap { trim: false }),
            Rect { height: body.height.min(6), ..left },
        );
    }
    let hgt = left.height as usize;
    let sel = v.sel.unwrap_or(0);
    let start = sel.saturating_sub(hgt.saturating_sub(1));
    for (i, row) in v.rows.into_iter().enumerate().skip(start).take(hgt) {
        let ry = left.y + (i - start) as u16;
        let on = Some(i) == v.sel;
        let bg = if on { t.hov } else { t.card };
        let rr = Rect { y: ry, height: 1, ..left };
        fill(f.buffer_mut(), rr, bg);
        let row = Line::from(row.spans.into_iter().map(|sp| { let st = sp.style.bg(bg); Span::styled(sp.content, st) }).collect::<Vec<_>>());
        f.render_widget(Paragraph::new(row).style(Style::default().bg(bg)), rr);
    }
    f.render_widget(Paragraph::new(v.detail).style(c.fg(t.text)).scroll((v.detail_scroll, 0)).wrap(ratatui::widgets::Wrap { trim: false }), right);
    f.render_widget(Paragraph::new(v.footer).style(c), Rect { x: r.x + 3, y: r.bottom().saturating_sub(2), width: r.width.saturating_sub(6), height: 1 });
}

pub(super) fn hint(t: &crate::theme::Theme, pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (k, what)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(t.border)));
        }
        spans.push(Span::styled(k.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {what}"), Style::default().fg(t.muted)));
    }
    Line::from(spans)
}

fn toolbox_panel(app: &App, t: &crate::theme::Theme) -> PanelView {
    use super::toolbox::Row;
    let Mode::Toolbox(v) = &app.mode else { unreachable!() };
    let mut rows = Vec::new();
    let mut sel_row = None;
    let mut item_i = 0;
    if let Some(sections) = &v.sections {
        for row in v.rows() {
            match row {
                Row::Header(si) => {
                    let s = &sections[si];
                    rows.push(Line::from(vec![
                        Span::styled(format!(" {} ", s.tool), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{} ({})", s.title, s.items.len()), Style::default().fg(t.muted).add_modifier(Modifier::BOLD)),
                    ]));
                }
                Row::Item(si, ii) => {
                    let it = &sections[si].items[ii];
                    if item_i == v.sel {
                        sel_row = Some(rows.len());
                    }
                    item_i += 1;
                    let (dot, c) = if it.enabled { ("●", t.idle) } else { ("○", t.muted) };
                    rows.push(Line::from(vec![
                        Span::styled(format!("   {dot} "), Style::default().fg(c)),
                        Span::styled(format!("{} ", truncate(&it.name, 34)), Style::default().fg(if it.enabled { t.fg } else { t.muted })),
                        Span::styled(it.scope.clone(), Style::default().fg(t.muted)),
                    ]));
                }
            }
        }
    }
    let mut detail = Vec::new();
    if let Some(it) = v.selected() {
        detail.push(Line::from(Span::styled(it.name.clone(), Style::default().fg(t.fg).add_modifier(Modifier::BOLD))));
        detail.push(Line::from(vec![
            Span::styled(if it.enabled { "on" } else { "off" }, Style::default().fg(if it.enabled { t.idle } else { t.muted })),
            Span::styled(format!(" · {}", it.scope), Style::default().fg(t.muted)),
        ]));
        detail.push(Line::raw(""));
        detail.extend(it.detail.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(t.fg)))));
        detail.push(Line::raw(""));
        detail.push(Line::from(Span::styled(format!("defined in {}", it.source.display()), Style::default().fg(t.muted))));
    }
    PanelView {
        title: format!("agent tools · {}", truncate(&v.project.display().to_string(), 50)),
        tabs: vec!["This project".into(), "Everywhere".into()],
        tab: v.everywhere as usize,
        query: Some(v.query.clone()),
        sel: sel_row,
        rows,
        detail,
        detail_scroll: v.scroll,
        footer: hint(t, &[("Tab", "this project / everywhere"), ("Enter", "open its config file"), ("^R", "rescan"), ("Esc", "close")]),
        empty: if v.sections.is_none() { "  reading configs…".into() } else { "  Nothing set up here (or nothing matches).".into() },
        left_pct: 50,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_row_marks_without_shifting() {
        let t = crate::theme::Theme::named("catppuccin-mocha");
        let row = Line::from(vec![Span::raw(" fix-totals"), Span::raw("  ready")]);
        let width = row.width();
        let marked = focus_row(row, &t);
        assert_eq!(marked.width(), width, "selection must not move the text");
        assert_eq!(marked.spans[0].content, "▌");
        assert!(marked.spans.iter().all(|s| s.style.bg == Some(focus_bg(&t))));
    }
}
