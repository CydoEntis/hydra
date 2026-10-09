//! Short movements: the sidebar and the sheet slide in, a highlight glides to its row, a
//! toast slides in and fades. Whole cells, eased out, over well under 150 ms, so they read as
//! smooth without slowing anything down. Off (Settings › Motion), or over SSH where every
//! frame is sent down the link, things are simply where they end.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a slide takes; a glide between rows is quicker.
pub const SLIDE: Duration = Duration::from_millis(140);
pub const GLIDE: Duration = Duration::from_millis(90);
/// A toast's slide in, and the fade at the end of its time.
pub const TOAST_IN: Duration = Duration::from_millis(120);
pub const TOAST_FADE: Duration = Duration::from_millis(350);

/// A value moving from `from` to `to`, eased out (fast, then settling).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
}

impl Tween {
    pub fn new(from: f32, to: f32, dur: Duration) -> Tween {
        Tween { from, to, start: Instant::now(), dur }
    }

    pub fn at(&self, now: Instant) -> f32 {
        let p = (now.saturating_duration_since(self.start).as_secs_f32() / self.dur.as_secs_f32().max(1e-6)).min(1.0);
        self.from + (self.to - self.from) * ease_out(p)
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

/// Cubic ease-out: most of the way at once, the last bit gently.
pub fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// What's moving right now.
#[derive(Debug, Default)]
pub struct Motion {
    /// The sidebar's share of its width (0 hidden, 1 shown), and whether it was shown.
    side: Option<Tween>,
    side_was: Option<bool>,
    /// The sheet's share of its width as it opens, and whether one was open.
    sheet: Option<Tween>,
    sheet_was: bool,
    /// Per highlight: what it marked last, the row it was on, and its glide.
    glides: HashMap<&'static str, (u64, u16, Option<Tween>)>,
    /// Per tab: its panes last frame, and a new split's first-pane share as the new pane
    /// grows in.
    splits: HashMap<u64, (Vec<u64>, Option<Tween>)>,
}

impl Motion {
    /// The sidebar's share of its width now; a change of `shown` starts a slide.
    pub fn side(&mut self, shown: bool, on: bool) -> f32 {
        let now = Instant::now();
        let target = if shown { 1.0 } else { 0.0 };
        if self.side_was.is_some_and(|was| was != shown) && on {
            let from = self.side.map(|t| t.at(now)).unwrap_or(1.0 - target);
            self.side = Some(Tween::new(from, target, SLIDE));
        }
        self.side_was = Some(shown);
        match self.side {
            Some(t) if !t.done(now) => t.at(now),
            _ => {
                self.side = None;
                target
            }
        }
    }

    /// The open sheet's share of its width now: it slides in when one opens (and is gone at
    /// once when it closes).
    pub fn sheet(&mut self, open: bool, on: bool) -> f32 {
        let now = Instant::now();
        if open && !self.sheet_was && on {
            self.sheet = Some(Tween::new(0.0, 1.0, SLIDE));
        }
        self.sheet_was = open;
        match self.sheet {
            Some(t) if open && !t.done(now) => t.at(now),
            _ => {
                self.sheet = None;
                1.0
            }
        }
    }

    /// Where highlight `key` is drawn: when what it marks (`id`) changes, it glides from the
    /// row it was on to `y`. Some(row) while it's on its way; None once it's there.
    pub fn glide(&mut self, key: &'static str, id: u64, y: u16, on: bool) -> Option<u16> {
        let now = Instant::now();
        let entry = self.glides.entry(key).or_insert((id, y, None));
        if entry.0 != id {
            let from = entry.2.map(|t| t.at(now)).unwrap_or(entry.1 as f32);
            entry.2 = (on && from.round() as u16 != y).then(|| Tween::new(from, y as f32, GLIDE));
            entry.0 = id;
        }
        entry.1 = y;
        match entry.2 {
            Some(t) if !t.done(now) => Some(t.at(now).round() as u16),
            _ => {
                entry.2 = None;
                None
            }
        }
    }

    /// The first pane's share of a two-pane split in tab `tab` (`leaves`, in order), heading for
    /// `ratio`: when a second pane has just opened, it grows out of the edge it opened on.
    pub fn split(&mut self, tab: u64, leaves: &[u64], ratio: f32, on: bool) -> f32 {
        let now = Instant::now();
        let entry = self.splits.entry(tab).or_insert((leaves.to_vec(), None));
        if leaves.len() == 2 && entry.0.len() == 1 && on {
            // The new one is whichever wasn't there: first means it opened left (or above).
            let new_first = entry.0[0] != leaves[0];
            entry.1 = Some(Tween::new(if new_first { 0.0 } else { 1.0 }, ratio, SLIDE));
        }
        entry.0 = leaves.to_vec();
        match entry.1 {
            Some(t) if leaves.len() == 2 && !t.done(now) => t.at(now),
            _ => {
                entry.1 = None;
                ratio
            }
        }
    }

    /// Something that changes the layout is moving (pane sizes wait for it to stop).
    pub fn layout_moving(&self) -> bool {
        let now = Instant::now();
        self.side.is_some_and(|t| !t.done(now)) || self.sheet.is_some_and(|t| !t.done(now)) || self.splits.values().any(|(_, t)| t.is_some_and(|t| !t.done(now)))
    }

    /// Anything at all is moving (keep drawing frames).
    pub fn moving(&self) -> bool {
        let now = Instant::now();
        self.layout_moving() || self.glides.values().any(|(.., t)| t.is_some_and(|t| !t.done(now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_eases_out_and_ends_where_it_was_going() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.8, "most of the way by half time");
        let t = Tween { from: 0.0, to: 10.0, start: Instant::now() - Duration::from_secs(1), dur: SLIDE };
        assert!(t.done(Instant::now()) && t.at(Instant::now()) == 10.0);
    }

    #[test]
    fn the_sidebar_slides_only_when_it_changes_and_motion_is_on() {
        let mut m = Motion::default();
        assert_eq!(m.side(true, true), 1.0, "first sight: already there");
        let mid = m.side(false, true);
        assert!(mid > 0.0 && m.layout_moving(), "hiding: it slides out ({mid})");
        let mut m = Motion::default();
        m.side(true, false);
        assert_eq!(m.side(false, false), 0.0, "motion off: gone at once");
    }

    #[test]
    fn a_new_pane_grows_out_of_its_edge() {
        let mut m = Motion::default();
        assert_eq!(m.split(1, &[10], 0.5, true), 0.5);
        let r = m.split(1, &[10, 11], 0.5, true);
        assert!(r > 0.5 && m.layout_moving(), "opened right: the first pane starts wide ({r})");
        let mut m = Motion::default();
        m.split(1, &[10], 0.5, true);
        assert!(m.split(1, &[11, 10], 0.5, true) < 0.5, "opened left: the first (new) one starts narrow");
    }

    #[test]
    fn a_highlight_glides_to_its_new_row() {
        let mut m = Motion::default();
        assert_eq!(m.glide("side", 1, 5, true), None, "first sight: on its row");
        let y = m.glide("side", 2, 15, true).expect("moving to the next one");
        assert!((5..15).contains(&y), "on its way: {y}");
        assert!(m.moving());
        assert_eq!(Motion::default().glide("side", 1, 5, false), None);
    }
}
