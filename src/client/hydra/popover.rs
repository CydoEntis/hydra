//! Popovers beside a sidebar row (the screen isn't dimmed): a quick follow-up to an agent
//! (`m`), and why it has the status it has (`i`). A `◂` points at the row.

use super::*;
use crate::protocol::ClientMsg;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A follow-up being written: to `term`, from the sidebar (`side`: back to its cursor after)
/// or from an Inbox row (`inbox`: the Inbox's query and row, to go back to).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::client) struct Compose {
    pub term: TermId,
    pub text: String,
    pub side: bool,
    pub inbox: Option<(String, usize)>,
}

/// The text area's rows: 3 to start, growing with the text up to this many (the popover;
/// an Inbox row grows to `INBOX_ROWS`).
const POP_ROWS: u16 = 10;
pub(in crate::client) const INBOX_ROWS: u16 = 6;
/// The popovers' widths, when there's room.
const COMPOSE_W: u16 = 76;
const WHY_W: u16 = 64;

/// Word-wrapped to `width`, keeping blank lines.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            if !cur.is_empty() && cur.width() + 1 + word.width() > width {
                out.push(std::mem::take(&mut cur));
            } else if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
            while cur.width() > width {
                let head: String = cur.chars().take(width).collect();
                cur = cur.chars().skip(width).collect();
                out.push(head);
            }
        }
        out.push(cur);
    }
    out
}

/// The rows a text area `w` wide takes (its box and the keys under it).
pub(in crate::client) fn compose_rows(c: &Compose, w: u16, max: u16) -> u16 {
    (wrap(&c.text, w.saturating_sub(6) as usize).len() as u16).clamp(3, max) + 3
}

/// The text area at `at` (its top left and width), on `outer`: a rounded box of 3 to `max` rows that grows
/// with the text and scrolls past that (`↑ n more` in its top border), a word count top
/// right, and the keys under it. Returns the rows it took.
pub(in crate::client) fn compose_box(app: &mut App, buf: &mut Buffer, at: Rect, c: &Compose, max: u16, outer: Color, t: &Theme) -> u16 {
    let (x, y, w) = (at.x, at.y, at.width);
    let lines = wrap(&c.text, w.saturating_sub(6) as usize);
    let rows = (lines.len() as u16).clamp(3, max);
    let hidden = lines.len().saturating_sub(rows as usize);
    let more = if hidden > 0 { format!("↑ {hidden} more") } else { String::new() };
    let mut card_ = Card::new(t, &more);
    card_.border = t.accent;
    card_.bold = true;
    card_.title_fg = t.muted;
    card_.bg = t.card2;
    let r = Rect { x, y, width: w, height: rows + 2 };
    card(app, buf, r, &card_, t);
    let words = c.text.split_whitespace().count();
    if words > 0 {
        let wc = format!(" {words} word{} ", if words == 1 { "" } else { "s" });
        put(buf, r.right().saturating_sub(wc.width() as u16 + 2), y, &[seg(wc, Style::default().fg(t.muted).bg(outer))], r.right());
    }
    let s2 = Style::default().bg(t.card2);
    if c.text.is_empty() {
        let name = app.snap.terms.get(&c.term).map(|i| i.display_name()).unwrap_or_default();
        put(buf, x + 3, y + 1, &[seg(format!("Next prompt for {name}…"), s2.fg(t.muted).add_modifier(Modifier::ITALIC)), seg(" █", s2.fg(t.accent))], r.right() - 2);
    }
    let shown = &lines[hidden..];
    for (i, l) in shown.iter().enumerate().filter(|_| !c.text.is_empty()) {
        let mut segs = vec![seg(l.clone(), s2.fg(t.strong))];
        if i + 1 == shown.len() {
            segs.push(seg("█", s2.fg(t.accent)));
        }
        put(buf, x + 3, y + 1 + i as u16, &segs, r.right() - 2);
    }
    let keys = cap_hints(t, outer, &[("Enter", "send"), ("Shift+Enter", "new line"), ("Esc", "cancel")]);
    put(buf, x + 1, y + rows + 2, &keys, x + w);
    rows + 3
}

/// Where a popover for `term`'s row goes: just right of the sidebar, its third row level
/// with the row, kept on screen.
fn pop_rect(app: &App, screen: Rect, side: Rect, term: TermId, w: u16, h: u16) -> Rect {
    let x = side.right() + 1;
    let row = app.hy.row_y.get(&term).copied().unwrap_or(screen.y + 6);
    let y = row.saturating_sub(2).clamp(screen.y + 1, screen.bottom().saturating_sub(h + 1));
    Rect { x, y, width: w.min(screen.right().saturating_sub(x + 2)), height: h }
}

/// The `◂` on the popover's left edge, pointing at its row.
fn pointer(buf: &mut Buffer, r: Rect, t: &Theme) {
    put(buf, r.x, r.y + 2, &[seg("◂", Style::default().fg(t.accent).bg(t.bg).add_modifier(Modifier::BOLD))], r.x + 1);
}

/// The follow-up popover: what the agent asks (or said last), then the text area.
pub(in crate::client) fn draw_compose_pop(app: &mut App, buf: &mut Buffer, screen: Rect, side: Rect, c: &Compose, t: &Theme) {
    let inner_w = COMPOSE_W.min(screen.width.saturating_sub(side.width + 4)).saturating_sub(4);
    let rows = (wrap(&c.text, inner_w.saturating_sub(6) as usize).len() as u16).clamp(3, POP_ROWS);
    let r = pop_rect(app, screen, side, c.term, COMPOSE_W, rows + 8);
    let info = app.snap.terms.get(&c.term).cloned();
    let name = info.as_ref().map(|i| i.display_name()).unwrap_or_default();
    let title = format!("message {name}");
    let mut card_ = Card::new(t, &title).lit(t.accent);
    card_.bg = t.card;
    card_.sub = info.as_ref().and_then(|i| i.agent.as_deref()).unwrap_or("");
    card(app, buf, r, &card_, t);
    hit(app, r, HyHit::Noop);
    let model = app.hy_model();
    let asks = find(&model, c.term).and_then(|(_, _, s)| (s.status == Status::Blocked).then(|| s.question.clone()).flatten());
    let (line, col) = match asks {
        Some(q) => (q, t.blocked),
        None => (last_line(app, c.term).map(|l| format!("“{l}”")).unwrap_or_default(), t.muted),
    };
    put(buf, r.x + 3, r.y + 2, &[seg(truncate(&line, r.width.saturating_sub(6) as usize), Style::default().fg(col).bg(t.card).add_modifier(Modifier::ITALIC))], r.right() - 2);
    compose_box(app, buf, Rect { x: r.x + 2, y: r.y + 4, width: r.width.saturating_sub(4), height: 0 }, c, POP_ROWS, t.card, t);
    pointer(buf, r, t);
}

/// What an agent said last, one line.
fn last_line(app: &App, term: TermId) -> Option<String> {
    let said = app.snap.terms.get(&term)?.said.lines().rev().find(|l| !l.trim().is_empty())?.trim().to_string();
    Some(said)
}

/// Where a status came from: "hook", "screen" or "process" (the rest: you).
fn source_of(why: &str) -> &'static str {
    match why.split(':').next().unwrap_or("") {
        "hook" => "hook",
        "screen" => "screen",
        "process" => "process",
        _ => "",
    }
}

/// Why a session has its status: the hook, screen and process evidence, each with its age,
/// the one that set it bold and marked.
pub(in crate::client) fn draw_why_pop(app: &mut App, buf: &mut Buffer, screen: Rect, side: Rect, term: TermId, t: &Theme) {
    let Some(info) = app.snap.terms.get(&term).cloned() else { return };
    let r = pop_rect(app, screen, side, term, WHY_W, 9);
    let title = format!("why {} {}", glyph(app, info.status), state_label(info.status));
    let mut card_ = Card::new(t, &title).lit(t.accent);
    card_.bg = t.card;
    card(app, buf, r, &card_, t);
    hit(app, r, HyHit::Noop);
    let ago = |at: u64| if at == 0 { String::new() } else { format!("{} ago", age(at)) };
    let process = match (info.agent.as_deref(), info.pid) {
        (Some(a), 0) => format!("{a} running"),
        (Some(a), pid) => format!("{a} running · pid {pid}"),
        (None, _) => format!("{} running", if info.process.is_empty() { "shell" } else { &info.process }),
    };
    let won = source_of(&info.status_why);
    let rows = [
        ("hook", if info.why_hook.is_empty() { "none since it started".to_string() } else { info.why_hook.clone() }, ago(info.why_hook_at)),
        ("screen", if info.why_screen.is_empty() { "nothing matched yet".to_string() } else { info.why_screen.clone() }, ago(info.why_screen_at)),
        ("process", process, "now".to_string()),
    ];
    let c = Style::default().bg(t.card);
    for (i, (src, detail, when)) in rows.iter().enumerate() {
        let y = r.y + 2 + i as u16;
        let win = *src == won;
        let name = if win { c.fg(t.strong).add_modifier(Modifier::BOLD) } else { c.fg(t.muted) };
        let right = vec![seg(when.clone(), c.fg(t.muted)), seg(if win { "  ◂" } else { "   " }, c.fg(t.accent).add_modifier(Modifier::BOLD))];
        let rw = segs_width(&right);
        put(buf, r.x + 3, y, &[seg(format!("{src:<9}"), name), seg(detail.clone(), c.fg(if win { t.text } else { t.muted }))], r.right().saturating_sub(rw + 4));
        put(buf, r.right().saturating_sub(rw + 3), y, &right, r.right() - 1);
    }
    let note = if won.is_empty() && !info.status_why.is_empty() {
        format!("Set by you: {}.", info.status_why)
    } else {
        "The most specific source wins: hook, then screen, then process.".to_string()
    };
    put(buf, r.x + 3, r.y + 6, &[seg(truncate(&note, r.width.saturating_sub(6) as usize), c.fg(t.muted).add_modifier(Modifier::ITALIC))], r.right() - 2);
    pointer(buf, r, t);
}

impl App {
    /// Start a follow-up to `term`; `inbox`: from that Inbox place (query, row).
    pub(in crate::client) fn start_compose(&mut self, term: TermId, side: bool, inbox: Option<(String, usize)>) {
        let agent = self.snap.terms.get(&term).is_some_and(|t| t.agent.is_some());
        if !agent {
            self.notify("only an agent takes a follow-up".into(), true);
            return;
        }
        self.mode = Mode::Compose(Box::new(Compose { term, text: String::new(), side, inbox }));
    }

    /// Back where the follow-up was started from.
    fn compose_back(&mut self, c: &Compose) {
        self.mode = match &c.inbox {
            Some((query, sel)) => Mode::GoTo { query: query.clone(), sel: *sel },
            None if c.side => Mode::Side,
            None => Mode::Normal,
        };
    }

    pub(in crate::client) fn on_compose_key(&mut self, mut c: Compose, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let newline = k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
        match k.code {
            KeyCode::Esc => return self.compose_back(&c),
            KeyCode::Enter if newline => c.text.push('\n'),
            KeyCode::Enter => {
                let text = if c.text.trim().is_empty() { "keep going".to_string() } else { c.text.trim_end().to_string() };
                self.send_follow_up(c.term, &text);
                return self.compose_back(&c);
            }
            KeyCode::Backspace => {
                c.text.pop();
            }
            KeyCode::Char('u') if ctrl => c.text.clear(),
            KeyCode::Char(ch) if !ctrl => c.text.push(ch),
            _ => {}
        }
        self.mode = Mode::Compose(Box::new(c));
    }

    /// Type `text` into the agent's prompt and press Enter (a moment later, once it has taken
    /// the text); several lines go in as one paste.
    pub(in crate::client) fn send_follow_up(&mut self, term: TermId, text: &str) {
        let bracketed = self.parsers.get(&term).is_some_and(|p| p.screen().bracketed_paste());
        let data = if text.contains('\n') && bracketed { format!("\x1b[200~{}\x1b[201~", text.replace('\n', "\r")) } else { text.replace('\n', " ") };
        self.send(ClientMsg::Input { term, data: data.into_bytes() });
        let out = self.out.clone();
        std::thread::spawn(move || {
            std::thread::sleep(ENTER_AFTER);
            let _ = out.send(ClientMsg::Input { term, data: b"\r".to_vec() });
        });
        let who = self.snap.terms.get(&term).map(|t| t.display_name()).unwrap_or_default();
        self.notify(format!("Sent to {who} · you stayed here"), false);
    }

    /// `i` again or Esc closes it; any other key closes it and does its own thing.
    pub(in crate::client) fn on_why_key(&mut self, side: bool, k: &KeyEvent) {
        self.mode = if side { Mode::Side } else { Mode::Normal };
        if !matches!(k.code, KeyCode::Esc | KeyCode::Char('i')) {
            self.on_key(*k);
        }
    }
}

/// How long after the text the follow-up's Enter goes, so the agent has taken the text.
const ENTER_AFTER: std::time::Duration = std::time::Duration::from_millis(60);

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn the_text_area_wraps_on_words_and_keeps_blank_lines() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"], "a word longer than the line breaks");
    }
}
