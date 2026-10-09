//! The floating look's pieces: rounded cards with their title set into the top border,
//! pills (tabs, buttons, selected rows), and fading what isn't in focus.

use super::*;

/// Nerd Font half-circles that round a pill's ends.
pub(in crate::client) const CAP_L: &str = "\u{e0b6}";
pub(in crate::client) const CAP_R: &str = "\u{e0b4}";

/// Round pill ends (the `ui.pill_caps` setting), for drawing that has no `Look` at hand.
static ROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Set once a frame from the settings.
pub(in crate::client) fn set_round(on: bool) {
    ROUND.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// A pill's two ends in `bg`: half-circles that keep the ground under them, or plain cells
/// of `bg` (square ends) without a Nerd Font.
pub(in crate::client) fn ends(bg: Color) -> (Seg, Seg) {
    if ROUND.load(std::sync::atomic::Ordering::Relaxed) {
        let st = Style::default().fg(bg);
        (seg(CAP_L, st), seg(CAP_R, st))
    } else {
        let st = Style::default().bg(bg);
        (seg(" ", st), seg(" ", st))
    }
}

/// A one-row strip of `bg` across `r` with round ends (an input, a bar of buttons). The ends
/// keep the ground under them; draw the strip's text from a column in.
pub(in crate::client) fn strip(buf: &mut Buffer, r: Rect, bg: Color) {
    if r.width < 3 || !ROUND.load(std::sync::atomic::Ordering::Relaxed) {
        fill(buf, r, bg);
        return;
    }
    fill(buf, Rect { x: r.x + 1, width: r.width - 2, ..r }, bg);
    let (l, e) = ends(bg);
    put(buf, r.x, r.y, &[l], r.x + 1);
    put(buf, r.right() - 1, r.y, &[e], r.right());
}

/// `text` on a pill in the style's background, as wide as `" text "` would be.
pub(in crate::client) fn chip(text: &str, st: Style) -> Vec<Seg> {
    let (l, r) = ends(st.bg.unwrap_or(Color::Reset));
    vec![l, seg(text, st), r]
}

/// How far the inside of an unfocused card fades toward its background at "40%" (the design's 42).
const DIM_DEFAULT: f32 = 0.42;
const DIM_SUBTLE: f32 = 0.2;
const DIM_STRONG: f32 = 0.6;

/// The inside of a card: the desk mixed most of the way to the surface.
pub(in crate::client) fn pane_bg(t: &Theme) -> Color {
    blend(t.bg, t.sidebar_bg, 0.7)
}

/// A state's colour on a card or tab: working reads as plain text, the rest as their state.
pub(in crate::client) fn state_color(t: &Theme, st: Status) -> Color {
    match st {
        Status::Working => t.text,
        Status::None => t.muted,
        s => t.status(s),
    }
}

/// How the cards are laid out and drawn, from the settings.
#[derive(Clone, Copy)]
pub(in crate::client) struct Look {
    pub tiled: bool,
    pub rounded: bool,
    /// Rows between stacked cards, and columns between cards side by side.
    pub gap: u16,
    pub dim: f32,
    pub caps: bool,
}

impl Look {
    pub fn of(ui: &crate::config::Ui) -> Look {
        let tiled = ui.panes == "tiled";
        Look {
            tiled,
            rounded: ui.corners != "square",
            gap: if tiled { 0 } else { ui.gap.parse().unwrap_or(1).min(2) },
            dim: match ui.dim.as_str() {
                "off" => 0.0,
                "subtle" => DIM_SUBTLE,
                "60%" => DIM_STRONG,
                _ => DIM_DEFAULT,
            },
            caps: ui.pill_caps,
        }
    }
}

/// `segs` on a pill of `bg`, its ends rounded on `outer` (or squared off without a Nerd Font).
pub(in crate::client) fn pill(look: Look, segs: Vec<Seg>, bg: Color, outer: Color) -> Vec<Seg> {
    let (l, r) = if look.caps { (CAP_L, CAP_R) } else { (" ", " ") };
    let end = |s: &str| if look.caps { seg(s, Style::default().fg(bg).bg(outer)) } else { seg(s, Style::default().bg(bg)) };
    let mut out = vec![end(l)];
    out.extend(segs.into_iter().map(|(x, st)| (x, st.bg(bg))));
    out.push(end(r));
    out
}

/// A pill as wide as `w`: the selected row of a list, an input, the answer bar.
pub(in crate::client) fn row_pill(look: Look, buf: &mut Buffer, x: u16, y: u16, w: u16, bg: Color, outer: Color) {
    if w < 2 {
        return;
    }
    let segs = pill(look, vec![seg(" ".repeat((w - 2) as usize), Style::default())], bg, outer);
    put(buf, x, y, &segs, x + w);
}

/// A key as a little pill: the accent on a button, or ink on the accent when it's the one.
pub(in crate::client) fn keycap_pill(look: Look, t: &Theme, key: &str, on: bool, outer: Color) -> Vec<Seg> {
    let (bg, fg) = if on { (t.accent, t.acc_ink) } else { (t.btn, t.accent) };
    pill(look, vec![seg(key, Style::default().fg(fg).add_modifier(Modifier::BOLD))], bg, outer)
}

/// A button as a pill: the primary one in the accent, the others raised with their key lit.
pub(in crate::client) fn button_pill(look: Look, t: &Theme, label: &str, key: &str, primary: bool, hovered: bool, outer: Color) -> Vec<Seg> {
    let (bg, fg, kf) = if primary { (t.accent, t.acc_ink, t.acc_ink) } else { (if hovered { t.hov } else { t.btn }, t.strong, t.accent) };
    let b = Style::default().add_modifier(Modifier::BOLD);
    let mut segs = vec![seg(format!("{label} "), b.fg(fg))];
    if !key.is_empty() {
        segs.push(seg(key, b.fg(kf)));
    }
    pill(look, segs, bg, outer)
}

/// What a card shows in and around its border.
pub(in crate::client) struct Card<'a> {
    pub title: &'a str,
    /// Dim, after the title, dropped when there isn't room.
    pub sub: &'a str,
    pub state: Option<Status>,
    /// The ✕ at the right of the title border, and what a click on it does.
    pub close: Option<HyHit>,
    pub border: Color,
    pub bold: bool,
    pub title_fg: Color,
    /// The inside's ground.
    pub bg: Color,
    /// The last row inside: left, and right-aligned.
    pub foot: Vec<Seg>,
    pub foot_r: Vec<Seg>,
}

impl<'a> Card<'a> {
    pub fn new(t: &Theme, title: &'a str) -> Card<'a> {
        Card { title, sub: "", state: None, close: None, border: t.line, bold: false, title_fg: t.text, bg: pane_bg(t), foot: Vec::new(), foot_r: Vec::new() }
    }

    /// The focused look: the border and title in `c`, bold.
    pub fn lit(mut self, c: Color) -> Card<'a> {
        self.border = c;
        self.title_fg = c;
        self.bold = true;
        self
    }
}

/// Draw a card: rounded border on the desk, title in the top border with a break around it,
/// state and ✕ on the right of it, a footer row inside. Returns the inside (within the border).
pub(in crate::client) fn card(app: &mut App, buf: &mut Buffer, r: Rect, c: &Card, t: &Theme) -> Rect {
    let look = Look::of(&app.cfg.ui);
    let r = r.intersection(buf.area);
    if r.width < 4 || r.height < 2 {
        return Rect { width: 0, height: 0, ..r };
    }
    fill(buf, Rect { x: r.x + 1, y: r.y + 1, width: r.width - 2, height: r.height - 2 }, c.bg);
    let mut b = Style::default().fg(c.border);
    if c.bold {
        b = b.add_modifier(Modifier::BOLD);
    }
    let (tl, tr, bl, br) = if look.rounded { ("╭", "╮", "╰", "╯") } else { ("┌", "┐", "└", "┘") };
    let (x, y, right, bottom) = (r.x, r.y, r.right() - 1, r.bottom() - 1);
    // The line runs through the middle of its cells: those cells take the card's ground, so
    // the fill reaches the line instead of stopping half a cell short of it. The corners keep
    // the desk's, so a rounded corner still looks round.
    let edge = b.bg(c.bg);
    for xx in x + 1..right {
        buf[(xx, y)].set_symbol("─").set_style(edge);
        buf[(xx, bottom)].set_symbol("─").set_style(edge);
    }
    for yy in y + 1..bottom {
        buf[(x, yy)].set_symbol("│").set_style(edge);
        buf[(right, yy)].set_symbol("│").set_style(edge);
    }
    buf[(x, y)].set_symbol(tl).set_style(b);
    buf[(right, y)].set_symbol(tr).set_style(b);
    buf[(x, bottom)].set_symbol(bl).set_style(b);
    buf[(right, bottom)].set_symbol(br).set_style(b);

    // The title border: " title ─ sub ───── ● needs you ─ ✕ ─".
    let desk = Style::default();
    let (l_x, r_x) = (x + 3, r.right().saturating_sub(4));
    let span = r_x.saturating_sub(l_x);
    let tag = |short: bool| -> Vec<Seg> {
        let mut out = Vec::new();
        if let Some(st) = c.state {
            let mut s = desk.fg(state_color(t, st));
            if st == Status::Blocked {
                s = s.add_modifier(Modifier::BOLD);
            }
            let label = if short { String::new() } else { format!(" {}", state_label(st)) };
            out.push(seg(format!(" {}{label} ", glyph(app, st)), s));
        }
        if c.state.is_some() && c.close.is_some() {
            out.push(seg("─", b));
        }
        if c.close.is_some() {
            out.push(seg(" ✕ ", desk.fg(if c.bold { t.text } else { t.muted })));
        }
        out
    };
    let title_w = if c.title.is_empty() { 0 } else { c.title.width() as u16 + 2 };
    let mut right_segs = tag(false);
    if title_w + 3 + segs_width(&right_segs) > span {
        right_segs = tag(true);
    }
    let rw = segs_width(&right_segs);
    if rw > 0 && rw <= span {
        let rx = r_x + 1 - rw;
        put(buf, rx, y, &right_segs, r_x + 1);
        if let Some(h) = c.close {
            hit(app, Rect { x: r_x + 1 - 3, y, width: 3, height: 1 }, h);
        }
    }
    if !c.title.is_empty() {
        let max_t = span.saturating_sub(rw + 4) as usize;
        let title = truncate(c.title, max_t.max(1));
        let mut ts = desk.fg(c.title_fg).add_modifier(Modifier::BOLD);
        if !c.bold && c.title_fg == c.border {
            ts = ts.remove_modifier(Modifier::BOLD);
        }
        let mut segs = vec![seg(format!(" {title} "), ts)];
        if !c.sub.is_empty() && (title.width() + c.sub.width() + 8) < span.saturating_sub(rw) as usize {
            segs.push(seg("─", b));
            segs.push(seg(format!(" {} ", c.sub), desk.fg(t.muted)));
        }
        put(buf, l_x - 1, y, &segs, r_x.saturating_sub(rw));
    }
    let inside = Rect { x: x + 1, y: y + 1, width: r.width - 2, height: r.height - 2 };
    if r.height >= 4 {
        let fy = bottom - 1;
        let fr = segs_width(&c.foot_r);
        if !c.foot_r.is_empty() && fr + 4 < r.width {
            let segs: Vec<Seg> = c.foot_r.iter().map(|(s, st)| (s.clone(), if st.bg.is_none() { st.bg(c.bg) } else { *st })).collect();
            put(buf, r_x + 1 - fr, fy, &segs, r_x + 1);
        }
        let segs: Vec<Seg> = c.foot.iter().map(|(s, st)| (s.clone(), if st.bg.is_none() { st.bg(c.bg) } else { *st })).collect();
        put(buf, l_x, fy, &segs, r_x.saturating_sub(fr + 1).max(l_x));
    }
    inside
}

/// Fade everything inside `r` toward its own ground by `amount`, keeping colours recognisable.
pub(in crate::client) fn dim_inside(buf: &mut Buffer, r: Rect, amount: f32, t: &Theme) {
    if amount <= 0.0 {
        return;
    }
    let r = r.intersection(buf.area);
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            let c = &mut buf[(x, y)];
            let bg = if c.bg == Color::Reset { t.bg } else { c.bg };
            let fg = if c.fg == Color::Reset { t.fg } else { c.fg };
            c.fg = blend(fg, bg, amount);
        }
    }
}
