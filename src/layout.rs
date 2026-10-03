//! The split tree each tab is made of. Shared by the daemon (which owns it) and the client
//! (which lays it out against its own viewport).

use crate::protocol::TermId;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Leaf(TermId),
    /// `horizontal`: children side by side (`a` left of `b`); otherwise `a` above `b`.
    Split { horizontal: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    pub fn contains(&self, t: TermId) -> bool {
        match self {
            Node::Leaf(id) => *id == t,
            Node::Split { a, b, .. } => a.contains(t) || b.contains(t),
        }
    }

    pub fn leaves(&self) -> Vec<TermId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<TermId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn first_leaf(&self) -> TermId {
        match self {
            Node::Leaf(id) => *id,
            Node::Split { a, .. } => a.first_leaf(),
        }
    }

    /// Replace leaf `target` with a split holding it and `new`, `new` on the `dir` side.
    pub fn split(&mut self, target: TermId, dir: Dir, new: TermId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let (a, b) = match dir {
                    Dir::Right | Dir::Down => (Node::Leaf(target), Node::Leaf(new)),
                    Dir::Left | Dir::Up => (Node::Leaf(new), Node::Leaf(target)),
                };
                *self = Node::Split {
                    horizontal: dir.horizontal(),
                    ratio: 0.5,
                    a: Box::new(a),
                    b: Box::new(b),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, dir, new) || b.split(target, dir, new),
        }
    }

    /// Remove a leaf, collapsing its parent split. Returns `None` when the tree becomes empty.
    pub fn remove(self, target: TermId) -> Option<Node> {
        match self {
            Node::Leaf(id) if id == target => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { horizontal, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split { horizontal, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(n), None) | (None, Some(n)) => Some(n),
                (None, None) => None,
            },
        }
    }

    /// Rebuild the tree with new leaf ids; leaves mapped to `None` are dropped.
    pub fn map_leaves(&self, f: &mut impl FnMut(TermId) -> Option<TermId>) -> Option<Node> {
        match self {
            Node::Leaf(id) => f(*id).map(Node::Leaf),
            Node::Split { horizontal, ratio, a, b } => match (a.map_leaves(f), b.map_leaves(f)) {
                (Some(a), Some(b)) => {
                    Some(Node::Split { horizontal: *horizontal, ratio: *ratio, a: Box::new(a), b: Box::new(b) })
                }
                (Some(n), None) | (None, Some(n)) => Some(n),
                (None, None) => None,
            },
        }
    }

    /// Move the divider of the nearest split on `dir`'s axis that encloses `target`.
    pub fn resize(&mut self, target: TermId, dir: Dir, delta: f32) -> bool {
        let Node::Split { horizontal, ratio, a, b } = self else {
            return false;
        };
        let in_a = a.contains(target);
        if !in_a && !b.contains(target) {
            return false;
        }
        // Prefer the deepest matching split.
        let child = if in_a { a } else { b };
        if child.resize(target, dir, delta) {
            return true;
        }
        if *horizontal != dir.horizontal() {
            return false;
        }
        let sign = if matches!(dir, Dir::Right | Dir::Down) { 1.0 } else { -1.0 };
        *ratio = (*ratio + sign * delta).clamp(0.1, 0.9);
        true
    }

    /// Every split in the tree laid out in `area`: (its area, side by side?, its ratio's
    /// path from the root: false = into `a`, true = into `b`).
    pub fn splits(&self, area: Rect) -> Vec<(Rect, bool, Vec<bool>)> {
        let mut out = Vec::new();
        self.collect_splits(area, &mut Vec::new(), &mut out);
        out
    }

    fn collect_splits(&self, area: Rect, path: &mut Vec<bool>, out: &mut Vec<(Rect, bool, Vec<bool>)>) {
        if let Node::Split { horizontal, ratio, a, b } = self {
            out.push((area, *horizontal, path.clone()));
            let (ra, rb) = split_rect(area, *horizontal, *ratio);
            path.push(false);
            a.collect_splits(ra, path, out);
            path.pop();
            path.push(true);
            b.collect_splits(rb, path, out);
            path.pop();
        }
    }

    /// Set the ratio of the split at `path` (from `splits`).
    pub fn set_ratio(&mut self, path: &[bool], to: f32) {
        match (self, path.split_first()) {
            (Node::Split { ratio, .. }, None) => *ratio = to.clamp(0.1, 0.9),
            (Node::Split { a, b, .. }, Some((side, rest))) => (if *side { b } else { a }).set_ratio(rest, to),
            _ => {}
        }
    }

    /// Where the divider of a split laid out in `area` is.
    pub fn divider(area: Rect, horizontal: bool, ratio: f32) -> Rect {
        let (ra, _) = split_rect(area, horizontal, ratio);
        if horizontal {
            Rect { x: ra.right().saturating_sub(1), width: 1, ..area }
        } else {
            Rect { y: ra.bottom().saturating_sub(1), height: 1, ..area }
        }
    }

    /// The ratio of the split at `path`.
    pub fn ratio_at(&self, path: &[bool]) -> Option<f32> {
        match (self, path.split_first()) {
            (Node::Split { ratio, .. }, None) => Some(*ratio),
            (Node::Split { a, b, .. }, Some((side, rest))) => (if *side { b } else { a }).ratio_at(rest),
            _ => None,
        }
    }

    pub fn rects(&self, area: Rect) -> Vec<(TermId, Rect)> {
        let mut out = Vec::new();
        self.layout(area, &mut out);
        out
    }

    fn layout(&self, area: Rect, out: &mut Vec<(TermId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, area)),
            Node::Split { horizontal, ratio, a, b } => {
                let (ra, rb) = split_rect(area, *horizontal, *ratio);
                a.layout(ra, out);
                b.layout(rb, out);
            }
        }
    }
}

fn split_rect(area: Rect, horizontal: bool, ratio: f32) -> (Rect, Rect) {
    let total = if horizontal { area.width } else { area.height };
    let first = if total < 2 {
        total
    } else {
        ((total as f32 * ratio).round() as u16).clamp(1, total - 1)
    };
    if horizontal {
        (
            Rect { width: first, ..area },
            Rect { x: area.x + first, width: area.width - first, ..area },
        )
    } else {
        (
            Rect { height: first, ..area },
            Rect { y: area.y + first, height: area.height - first, ..area },
        )
    }
}

/// The pane adjacent to `from` in direction `dir`, preferring the largest shared edge.
pub fn neighbor(rects: &[(TermId, Rect)], from: TermId, dir: Dir) -> Option<TermId> {
    let (_, f) = rects.iter().find(|(id, _)| *id == from)?;
    let overlap = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1).saturating_sub(a0.max(b0));
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, r)| {
            let (gap, shared) = match dir {
                Dir::Right if r.x >= f.right() => (r.x - f.right(), overlap(f.y, f.bottom(), r.y, r.bottom())),
                Dir::Left if r.right() <= f.x => (f.x - r.right(), overlap(f.y, f.bottom(), r.y, r.bottom())),
                Dir::Down if r.y >= f.bottom() => (r.y - f.bottom(), overlap(f.x, f.right(), r.x, r.right())),
                Dir::Up if r.bottom() <= f.y => (f.y - r.bottom(), overlap(f.x, f.right(), r.x, r.right())),
                _ => return None,
            };
            (shared > 0).then_some((gap, std::cmp::Reverse(shared), *id))
        })
        .min()
        .map(|(_, _, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_remove_roundtrip() {
        let mut n = Node::Leaf(1);
        assert!(n.split(1, Dir::Right, 2));
        assert!(n.split(2, Dir::Down, 3));
        assert_eq!(n.leaves(), vec![1, 2, 3]);
        let n = n.remove(2).unwrap();
        assert_eq!(n.leaves(), vec![1, 3]);
        let n = n.remove(1).unwrap();
        assert_eq!(n, Node::Leaf(3));
        assert!(n.remove(3).is_none());
    }

    #[test]
    fn neighbors() {
        let mut n = Node::Leaf(1);
        n.split(1, Dir::Right, 2);
        n.split(2, Dir::Down, 3);
        let r = n.rects(Rect::new(0, 0, 100, 40));
        assert_eq!(neighbor(&r, 1, Dir::Right), Some(2));
        assert_eq!(neighbor(&r, 3, Dir::Up), Some(2));
        assert_eq!(neighbor(&r, 3, Dir::Left), Some(1));
        assert_eq!(neighbor(&r, 1, Dir::Left), None);
    }

    #[test]
    fn resize_moves_divider() {
        let mut n = Node::Leaf(1);
        n.split(1, Dir::Right, 2);
        assert!(n.resize(1, Dir::Right, 0.1));
        let Node::Split { ratio, .. } = n else { panic!() };
        assert!((ratio - 0.6).abs() < 1e-6);
        let mut n = Node::Leaf(1);
        n.split(1, Dir::Right, 2);
        assert!(!n.resize(1, Dir::Down, 0.1));
    }
}
