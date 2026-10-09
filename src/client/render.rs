//! Drawing: sidebar, tab bar, panes, status bar and overlays.

use super::copy::Copy;
use super::{App, Mode, PickTarget};
use crate::protocol::Status;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The smallest window seshi draws into (below it, a note to make it bigger).
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 10;

pub fn draw(app: &mut App, f: &mut Frame) {
    app.hits.clear();
    app.panes.clear();
    app.pane_frames.clear();
    let area = f.area();
    let t = app.theme.clone();
    set_palette(t.ansi, t.ansi.map(|_| t.card2));
    super::hydra::set_round(app.cfg.ui.pill_caps);
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
            if app.keymap_shown(*since) {
                let km = super::hydra::KeyMap { query: String::new(), searching: false, step: None, sel: 0 };
                super::hydra::draw_keymap(app, f, area, &t, &km);
            }
        }
        Mode::KeyMap(km) => {
            let km = (**km).clone();
            super::hydra::draw_keymap(app, f, area, &t, &km);
        }
        Mode::Actions { sel } => {
            let sel = *sel;
            super::hydra::draw_actions(app, f, area, &t, sel);
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

fn draw_picker(app: &mut App, f: &mut Frame, area: Rect, t: &crate::theme::Theme, query: &str, sel: usize, commands: bool) {
    if commands {
        let items = app.pick_items(query, true);
        return super::hydra::draw_palette(app, f, area, t, query, sel, &items);
    }
    let items = app.pick_items(query, commands);
    let r = centered(area, 80, 22);
    let mut c = super::hydra::Card::new(t, "jump to").lit(t.accent);
    c.bg = t.card;
    let inside = super::hydra::card(app, f.buffer_mut(), r, &c, t);
    let inner = Rect { x: inside.x + 1, width: inside.width.saturating_sub(2), ..inside };
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
    use super::design::{put, seg};
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
    super::hydra::strip(buf, field, t.card2);
    let s2 = Style::default().bg(t.card2);
    put(buf, field.x + 1, field.y, &[seg("› ", s2.fg(t.accent).add_modifier(Modifier::BOLD)), seg(input.to_string(), s2.fg(t.strong)), seg("█", s2.fg(t.accent))], field.right());
    let keys = super::hydra::hints(t, &[("Enter", "save"), ("Esc", "cancel")]);
    put(buf, r.x + 3, r.bottom().saturating_sub(2), &keys, r.right());
}

// ---- panels: files, tasks, inbox, toolbox -------------------------------------------------





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
