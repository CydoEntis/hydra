//! Recipes from config: one choice in the new-agent dialog that starts several commands,
//! in their own worktree if the recipe says so.

use super::App;
use super::hydra::Proj;
use crate::protocol::Command;

/// `fix the flaky checkout test` → `fix-the-flaky-checkout`
fn slug(s: &str, words: usize) -> String {
    let mut out = String::new();
    for w in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(words) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    out.chars().take(40).collect::<String>().trim_end_matches('-').to_string()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl App {
    /// Run a recipe from config in a project: its own worktree (if it says so) running the
    /// first command, with the rest started beside it once the worktree exists.
    pub(super) fn run_recipe(&mut self, p: &Proj, name: &str) {
        let Some(r) = self.cfg.recipes.iter().find(|r| r.name == name).cloned() else { return };
        let Some(first) = r.run.first().cloned() else { return };
        let rest: Vec<String> = r.run[1..].to_vec();
        let main = p.wts.iter().find(|w| w.main).map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
        if r.worktree && p.git {
            let branch = format!("{}-{}", slug(&r.name, 2), now() % 10_000);
            self.hy.pending_recipe = Some((branch.clone(), rest, std::time::Instant::now()));
            self.hy_start_worktree(p, Some(first), Some(branch));
        } else {
            // The last one started gets the focus, so start the first command last.
            for c in rest.iter().rev() {
                self.cmd(Command::NewWorkspace { cwd: Some(main.clone()), name: None, cmd: Some(c.clone()) });
            }
            self.hy_new_session(main, Some(first), false);
        }
        self.notify(format!("recipe {}: {}", r.name, r.run.join(" + ")), false);
    }

    /// Once a recipe's worktree exists, start the rest of its commands there.
    pub(super) fn recipe_followup(&mut self) {
        let Some((branch, rest, at)) = self.hy.pending_recipe.clone() else { return };
        if at.elapsed().as_secs() > 60 {
            self.hy.pending_recipe = None;
            return;
        }
        let model = self.hy_model();
        let Some(w) = model.iter().flat_map(|p| p.wts.iter()).find(|w| w.branch == branch && !w.sessions.is_empty()).cloned() else { return };
        self.hy.pending_recipe = None;
        for c in rest {
            self.cmd(Command::NewWorkspace { cwd: Some(w.path.clone()), name: None, cmd: Some(c) });
        }
        if let Some(first) = w.sessions.first() {
            self.cmd(Command::FocusPane { term: first.term });
        }
    }
}
