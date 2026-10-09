//! Heads-ups: two checkouts changed the same file. Both diffs side by side in the sheet, a
//! note to one of the agents, or dismissing it (it comes back if it clears and happens again).

use super::*;
use crate::client::overlap::Overlap;
use crossterm::event::{KeyCode, KeyEvent};

/// Both diffs of one file: each checkout's changes to it, one under the other.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BothView {
    pub repo: PathBuf,
    pub overlap: Overlap,
    /// (checkout, its diff of the file), once read.
    pub parts: Option<Vec<(String, String)>>,
    pub scroll: u16,
}

/// The checkouts of `repo` by name, with their folders, from the sidebar's model.
fn checkouts(model: &[Proj], repo: &Path) -> Vec<Wt> {
    model.iter().filter(|p| p.path == repo).flat_map(|p| p.wts.iter().cloned()).collect()
}

/// The agent working in checkout `name` of `repo` (the first, if several).
pub(in crate::client) fn agent_in(model: &[Proj], repo: &Path, name: &str) -> Option<TermId> {
    checkouts(model, repo).into_iter().filter(|w| w.name == name).flat_map(|w| w.sessions).find(|s| s.is_agent).map(|s| s.term)
}

/// What to tell `to` about the file `other` changed too.
fn note(o: &Overlap, other: &str) -> String {
    format!("{other} also changes {}; rebase on it before you finish", o.file)
}

/// The rows the both-diffs sheet shows: a heading, then each checkout's hunks.
fn lines(v: &BothView, t: &Theme) -> Vec<Vec<Seg>> {
    let mut out = Vec::new();
    let conf = blend(t.blocked, t.text, 0.45);
    let who = v.overlap.checkouts.join(" and ");
    out.push(vec![seg("⇆ ", Style::default().fg(conf).add_modifier(Modifier::BOLD)), seg(format!("{who} both changed {}", v.overlap.file), Style::default().fg(t.strong))]);
    out.push(vec![]);
    let Some(parts) = &v.parts else {
        out.push(vec![seg("reading both diffs…", Style::default().fg(t.muted))]);
        return out;
    };
    for (name, diff) in parts {
        out.push(vec![seg(format!("⎇ {name}"), Style::default().fg(t.done).add_modifier(Modifier::BOLD)), seg(format!("  {}", v.overlap.file), Style::default().fg(t.sky()))]);
        let (mut old, mut new) = (0, 0);
        let body: Vec<&str> = diff.lines().skip_while(|l| !l.starts_with("@@")).collect();
        if body.is_empty() {
            out.push(vec![seg("  (no changes to it any more)", Style::default().fg(t.muted).add_modifier(Modifier::ITALIC))]);
        }
        for l in body {
            out.push(crate::client::design::diff_line(t, l, &mut old, &mut new));
        }
        out.push(vec![]);
    }
    out.push(vec![seg("Merging both will conflict where they overlap. Tell one agent to rebase on the other, or keep going.", Style::default().fg(t.muted).add_modifier(Modifier::ITALIC))]);
    out
}

pub(in crate::client) fn draw_both(buf: &mut Buffer, inside: Rect, t: &Theme, v: &BothView) {
    let (x, w) = (inside.x + 2, inside.width.saturating_sub(4));
    let top = inside.y + 1;
    let room = inside.height.saturating_sub(3) as usize;
    let all = lines(v, t);
    let start = (v.scroll as usize).min(all.len().saturating_sub(room.max(1)));
    for (i, l) in all.iter().skip(start).take(room).enumerate() {
        let l: Vec<Seg> = l.iter().map(|(s, st)| (s.clone(), st.bg(t.card))).collect();
        put(buf, x, top + i as u16, &l, x + w);
    }
    let names = &v.overlap.checkouts;
    let a = names.first().cloned().unwrap_or_default();
    let b = names.get(1).cloned().unwrap_or_default();
    let (ka, kb) = (format!("tell {a}"), format!("tell {b}"));
    status_bar(buf, inside, t, &[("m", &ka), ("M", &kb), ("k", "dismiss"), ("Esc", "close")], &[]);
}

impl App {
    /// Both diffs of heads-up `o` in `repo`, in the sheet; they're read off the UI thread.
    pub(in crate::client) fn open_both(&mut self, repo: PathBuf, o: Overlap) {
        let model = self.hy_model();
        let base = crate::gitfs::main_branch(&repo);
        let dirs: Vec<(String, PathBuf, bool)> = o
            .checkouts
            .iter()
            .filter_map(|name| checkouts(&model, &repo).into_iter().find(|w| &w.name == name).map(|w| (name.clone(), w.path, w.main)))
            .collect();
        let file = o.file.clone();
        self.mode = Mode::Normal;
        self.view = Some(crate::client::View::Both(Box::new(BothView { repo: repo.clone(), overlap: o, parts: None, scroll: 0 })));
        self.spawn_bg(move || {
            let parts = dirs
                .into_iter()
                .map(|(name, dir, main)| {
                    let since = if main { Some("HEAD".to_string()) } else { base.as_deref().and_then(|b| crate::proc::git(&dir, &["merge-base", "HEAD", b]).ok()).map(|s| s.trim().to_string()) };
                    let diff = since.and_then(|s| crate::proc::git(&dir, &["diff", s.as_str(), "--", &file]).ok()).unwrap_or_default();
                    (name, diff)
                })
                .collect();
            crate::client::Bg::BothDiff(repo, parts)
        });
    }

    /// Don't say this one again (until it clears and happens again).
    pub(in crate::client) fn dismiss_heads_up(&mut self, repo: &Path, o: &Overlap) {
        self.hy.dismissed.insert((repo.to_path_buf(), o.file.clone()));
        self.notify("Dismissed. It comes back if they touch it again.".into(), false);
    }

    /// A follow-up to the agent in checkout `to`, about what `other` changed in the same file
    /// (written for you; change it or send it). `inbox`: from that Inbox place.
    pub(in crate::client) fn tell_about(&mut self, repo: &Path, o: &Overlap, to: usize, inbox: Option<(String, usize)>) {
        let model = self.hy_model();
        let (Some(name), Some(other)) = (o.checkouts.get(to), o.checkouts.get(1 - to.min(1))) else { return };
        match agent_in(&model, repo, name) {
            Some(term) => {
                self.start_compose(term, false, inbox);
                if let Mode::Compose(c) = &mut self.mode {
                    c.text = note(o, other);
                }
            }
            None => self.notify(format!("no agent is working in {name}"), true),
        }
    }

    /// Keys in the both-diffs sheet. Returns false when it closes.
    pub(in crate::client) fn on_both_key(&mut self, v: &mut BothView, k: &KeyEvent) -> bool {
        match k.code {
            KeyCode::Esc => return false,
            KeyCode::Down => v.scroll = v.scroll.saturating_add(1),
            KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageDown => v.scroll = v.scroll.saturating_add(10),
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(10),
            KeyCode::Char(c @ ('m' | 'M')) => {
                let (repo, o) = (v.repo.clone(), v.overlap.clone());
                self.tell_about(&repo, &o, usize::from(c == 'M'), None);
                return false;
            }
            KeyCode::Char('k') => {
                let (repo, o) = (v.repo.clone(), v.overlap.clone());
                self.dismiss_heads_up(&repo, &o);
                return false;
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_names_the_other_checkout_and_the_file() {
        let o = Overlap { file: "src/checkout.ts".into(), checkouts: vec!["orders".into(), "rate-limit".into()] };
        assert_eq!(note(&o, "rate-limit"), "rate-limit also changes src/checkout.ts; rebase on it before you finish");
    }
}
