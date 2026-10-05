//! Where each command started in a pane's history, so you can jump between them. Shells mark
//! their prompt with OSC 133;A (fish, PowerShell through hydra, shells set up for it); the
//! line it lands on is noted in history numbering that keeps counting as lines scroll away.
//! A jump checks the line still holds what it did (the history drops its oldest lines once
//! full) and looks nearby if it moved.

use super::App;
use crate::protocol::TermId;

/// The prompt mark (OSC 133;A).
const MARK: &[u8] = b"\x1b]133;A";
/// How far a jump looks for a mark's line if the history shifted under it.
const SEARCH: usize = 200;

#[derive(Debug, Default)]
pub(super) struct Marks {
    /// Lines that have scrolled up out of the screen since the pane's copy was made.
    pushed: u64,
    /// (line in that numbering, what the line said once it was written).
    list: Vec<(u64, String)>,
}

impl App {
    /// Feed a pane's output to its screen copy, noting where prompts start.
    pub(super) fn feed(&mut self, term: TermId, data: &[u8]) {
        let Some(p) = self.parsers.get_mut(&term) else { return };
        let marks = self.marks.entry(term).or_default();
        let mut rest = data;
        loop {
            let at = find(rest, MARK);
            let (now, later) = match at {
                Some(i) => rest.split_at(i),
                None => (rest, &[][..]),
            };
            process(p, marks, now);
            if at.is_none() {
                break;
            }
            // The prompt starts where the cursor is.
            if !p.screen().alternate_screen() {
                let row = p.screen().cursor_position().0 as u64;
                let line = marks.pushed + row;
                if marks.list.last().is_none_or(|(l, _)| *l != line) {
                    marks.list.push((line, String::new()));
                }
            }
            process(p, marks, &later[..MARK.len().min(later.len())]);
            rest = &later[MARK.len().min(later.len())..];
        }
        // Note what each new mark's line says, once it says something.
        let screen = p.screen();
        let (rows, cols) = screen.size();
        for (line, text) in marks.list.iter_mut().rev().take(8) {
            if text.is_empty()
                && let Some(row) = line.checked_sub(marks.pushed).filter(|r| *r < rows as u64)
            {
                *text = screen.rows(0, cols).nth(row as usize).unwrap_or_default().trim_end().to_string();
            }
        }
    }

    /// Scroll `term` to the previous (`back`) or next command from what's on screen.
    pub(super) fn jump_prompt(&mut self, term: TermId, back: bool) {
        let (cur, total) = self.history(term);
        let Some(marks) = self.marks.get(&term) else {
            self.notify("no commands marked here (the shell doesn't mark its prompts)".into(), false);
            return;
        };
        let pushed = marks.pushed;
        // The line at the top of the view now.
        let top = pushed.saturating_sub(cur as u64);
        let pick = if back {
            marks.list.iter().rev().find(|(l, _)| *l < top)
        } else {
            marks.list.iter().find(|(l, _)| *l > top)
        };
        let Some((line, text)) = pick.cloned() else {
            if back {
                self.notify("that's the first command in the history".into(), false);
            } else {
                // Past the last: back to the bottom.
                self.scroll_to(term, 0);
            }
            return;
        };
        let want = (pushed.saturating_sub(line) as usize).min(total);
        let at = self.verify(term, want, &text, total).unwrap_or(want);
        self.scroll_to(term, at);
    }

    /// The scroll offset whose top line says `text`: `want`, or the nearest that does.
    fn verify(&mut self, term: TermId, want: usize, text: &str, total: usize) -> Option<usize> {
        if text.trim().is_empty() {
            return None;
        }
        let p = self.parsers.get_mut(&term)?;
        let keep = p.screen().scrollback();
        let mut found = None;
        for d in 0..=SEARCH {
            for off in [want.checked_add(d), want.checked_sub(d)].into_iter().flatten().filter(|o| *o <= total) {
                p.screen_mut().set_scrollback(off);
                let screen = p.screen();
                let (_, cols) = screen.size();
                if screen.rows(0, cols).next().is_some_and(|r| r.trim_end() == text) {
                    found = Some(off);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        p.screen_mut().set_scrollback(keep);
        found
    }
}

/// Process output and count the lines it scrolled up (history growth; once the history is
/// full it stops growing, so count the line feeds instead: a close guess the jump checks).
fn process(p: &mut vt100::Parser, marks: &mut Marks, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let before = history_len(p);
    let full = p.screen().alternate_screen();
    p.process(data);
    let after = history_len(p);
    marks.pushed += if after > before {
        (after - before) as u64
    } else if after == before && after > 0 && !full {
        let bottom = p.screen().cursor_position().0 + 1 >= p.screen().size().0;
        if bottom { data.iter().filter(|b| **b == b'\n').count() as u64 } else { 0 }
    } else {
        0
    };
}

fn history_len(p: &mut vt100::Parser) -> usize {
    let keep = p.screen().scrollback();
    p.screen_mut().set_scrollback(usize::MAX);
    let n = p.screen().scrollback();
    p.screen_mut().set_scrollback(keep);
    n
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
