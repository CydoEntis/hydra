//! Two checkouts of one repo changing the same file: agents in different worktrees can't
//! see each other's edits, so it only shows up as a conflict at merge time. Caught here
//! while it's cheap to sort out.

use std::collections::BTreeMap;
use std::path::Path;

/// One file changed in more than one checkout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Overlap {
    /// Repo-relative, with `/`.
    pub file: String,
    /// The checkouts that changed it (their names), sorted.
    pub checkouts: Vec<String>,
}

/// Files changed in a checkout: since it left `base` (committed or not) and new files git
/// doesn't ignore. The main checkout (`base` None) counts what isn't committed yet.
pub fn changed_files(dir: &Path, base: Option<&str>) -> Vec<String> {
    let since = match base {
        Some(b) => match crate::proc::git(dir, &["merge-base", "HEAD", b]) {
            Ok(mb) => mb.trim().to_string(),
            Err(_) => return Vec::new(),
        },
        None => "HEAD".to_string(),
    };
    let mut files: Vec<String> = Vec::new();
    for args in [&["diff", "--name-only", since.as_str()][..], &["ls-files", "--others", "--exclude-standard"][..]] {
        if let Ok(out) = crate::proc::git(dir, args) {
            files.extend(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from));
        }
    }
    files.sort();
    files.dedup();
    files
}

/// The files more than one checkout changed, from each checkout's (name, changed files).
pub fn find(changes: &[(String, Vec<String>)]) -> Vec<Overlap> {
    let mut by_file: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, files) in changes {
        for f in files {
            let who = by_file.entry(f.as_str()).or_default();
            if !who.contains(name) {
                who.push(name.clone());
            }
        }
    }
    by_file
        .into_iter()
        .filter(|(_, who)| who.len() > 1)
        .map(|(file, mut checkouts)| {
            checkouts.sort();
            Overlap { file: file.to_string(), checkouts }
        })
        .collect()
}

/// "orders and rate-limit both changed checkout.ts" (and how many more files).
pub fn say(overlaps: &[Overlap]) -> Option<String> {
    let first = overlaps.first()?;
    let who = match first.checkouts.as_slice() {
        [a, b] => format!("{a} and {b} both"),
        many => format!("{} checkouts", many.len()),
    };
    let name = first.file.rsplit('/').next().unwrap_or(&first.file);
    let more = match overlaps.len() - 1 {
        0 => String::new(),
        n => format!(" (and {n} more file{})", if n == 1 { "" } else { "s" }),
    };
    Some(format!("{who} changed {name}{more}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, files: &[&str]) -> (String, Vec<String>) {
        (name.to_string(), files.iter().map(|f| f.to_string()).collect())
    }

    #[test]
    fn a_file_two_checkouts_changed_is_an_overlap() {
        let o = find(&[c("orders", &["src/checkout.ts", "README.md"]), c("rate-limit", &["src/checkout.ts", "src/limit.ts"]), c("main folder", &[])]);
        assert_eq!(o, vec![Overlap { file: "src/checkout.ts".into(), checkouts: vec!["orders".into(), "rate-limit".into()] }]);
        assert_eq!(say(&o).as_deref(), Some("orders and rate-limit both changed checkout.ts"));
    }

    #[test]
    fn several_files_and_checkouts_are_summed_up() {
        let o = find(&[c("a", &["x.rs", "y.rs"]), c("b", &["x.rs", "y.rs"]), c("c", &["x.rs"])]);
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].checkouts, vec!["a", "b", "c"]);
        assert_eq!(say(&o).as_deref(), Some("3 checkouts changed x.rs (and 1 more file)"));
        assert!(find(&[c("a", &["x.rs"]), c("b", &["y.rs"])]).is_empty(), "different files: nothing to say");
    }

    #[test]
    fn a_worktree_counts_its_commits_and_the_main_folder_its_edits() {
        let root = std::env::temp_dir().join(format!("seshi-overlap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, args: &[&str]| crate::proc::git(dir, args).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("checkout.ts"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "one"]);
        let wt = root.join("orders");
        git(&repo, &["worktree", "add", "-q", "-b", "orders", wt.to_str().unwrap()]);
        // The worktree commits a change and adds a new file; the main folder edits without committing.
        std::fs::write(wt.join("checkout.ts"), "b").unwrap();
        git(&wt, &["commit", "-qam", "two"]);
        std::fs::write(wt.join("new.ts"), "c").unwrap();
        std::fs::write(repo.join("checkout.ts"), "d").unwrap();
        assert_eq!(changed_files(&wt, Some("main")), vec!["checkout.ts", "new.ts"]);
        assert_eq!(changed_files(&repo, None), vec!["checkout.ts"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
