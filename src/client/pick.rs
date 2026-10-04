//! What's under the mouse in a pane: a link (Ctrl+click opens it) or a word (double-click
//! copies it), read off the screen the way herdr does.

/// A screen row as one string per cell (wide characters take their cell, their
/// continuation cell is empty).
fn cells(screen: &vt100::Screen, row: u16) -> Vec<String> {
    let (_, cols) = screen.size();
    (0..cols).map(|c| screen.cell(row, c).map(|x| x.contents().to_string()).unwrap_or_default()).collect()
}

fn text(cells: &[String], from: usize, to: usize) -> String {
    cells[from..to].concat()
}

/// The http(s) link covering `col`, with trailing punctuation and unbalanced closing
/// brackets trimmed.
pub fn url_at(screen: &vt100::Screen, row: u16, col: u16) -> Option<String> {
    let cells = cells(screen, row);
    let col = col as usize;
    let line: Vec<char> = cells.iter().map(|c| c.chars().next().unwrap_or(' ')).collect();
    let s: String = line.iter().collect();
    for scheme in ["https://", "http://"] {
        let mut from = 0;
        while let Some(i) = s[from..].find(scheme).map(|i| i + from) {
            let start = s[..i].chars().count();
            let mut end = start;
            while end < line.len() && !line[end].is_whitespace() && !"<>\"'`".contains(line[end]) {
                end += 1;
            }
            let mut url: String = line[start..end].iter().collect();
            loop {
                let last = url.chars().last();
                let unbalanced = |o: char, c: char| url.ends_with(c) && url.matches(c).count() > url.matches(o).count();
                if matches!(last, Some('.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"')) || unbalanced('(', ')') || unbalanced('[', ']') || unbalanced('{', '}') {
                    url.pop();
                } else {
                    break;
                }
            }
            let end = start + url.chars().count();
            if (start..end).contains(&col) && url.len() > scheme.len() {
                return Some(url);
            }
            from = i + scheme.len();
        }
    }
    None
}

/// The word under `col`: a link, else a run of characters between separators (spaces and
/// `|()[]{},;!` and quotes), so paths and identifiers come out whole.
/// A file path under (row, col), as agents print them: `src/main.rs`, `./a/b.png`,
/// `C:\\dev\\x.md`, `apps/x.ts:42` (the line comes back too). Only things that look like
/// paths: a separator or a file extension.
pub fn path_at(screen: &vt100::Screen, row: u16, col: u16) -> Option<(String, Option<u32>)> {
    let cells = cells(screen, row);
    let col = (col as usize).min(cells.len().saturating_sub(1));
    let sep = |c: &str| c.is_empty() || c.chars().all(|ch| ch.is_whitespace() || "()[]{},;\"'`<>|*".contains(ch));
    if sep(&cells[col]) {
        return None;
    }
    let mut a = col;
    while a > 0 && !sep(&cells[a - 1]) {
        a -= 1;
    }
    let mut b = col + 1;
    while b < cells.len() && !sep(&cells[b]) {
        b += 1;
    }
    let mut w = text(&cells, a, b).trim_end_matches(['.', ',', ':', ';', '!', '?']).to_string();
    // path:line(:col)
    let mut line = None;
    let parts: Vec<&str> = w.rsplitn(3, ':').collect();
    if parts.len() >= 2 && parts[0].chars().all(|c| c.is_ascii_digit()) && !parts[0].is_empty() {
        let (num, rest) = if parts.len() == 3 && parts[1].chars().all(|c| c.is_ascii_digit()) && !parts[1].is_empty() {
            (parts[1], parts[2].to_string())
        } else {
            (parts[0], w[..w.len() - parts[0].len() - 1].to_string())
        };
        line = num.parse().ok();
        w = rest;
    }
    let looks = w.contains('/') || w.contains('\\') || std::path::Path::new(&w).extension().is_some_and(|e| e.len() <= 8 && e.to_string_lossy().chars().all(|c| c.is_ascii_alphanumeric()));
    (looks && !w.starts_with("http") && w.len() > 1).then_some((w, line))
}

pub fn word_at(screen: &vt100::Screen, row: u16, col: u16) -> Option<String> {
    if let Some(u) = url_at(screen, row, col) {
        return Some(u);
    }
    let cells = cells(screen, row);
    let col = (col as usize).min(cells.len().saturating_sub(1));
    let sep = |c: &str| c.is_empty() || c.chars().all(|ch| ch.is_whitespace() || "|()[]{},;!\"'`<>".contains(ch));
    if sep(&cells[col]) {
        return None;
    }
    let mut a = col;
    while a > 0 && !sep(&cells[a - 1]) {
        a -= 1;
    }
    let mut b = col + 1;
    while b < cells.len() && (!sep(&cells[b]) || (cells[b].is_empty() && b > 0 && !sep(&cells[b - 1]))) {
        if cells[b].is_empty() {
            // A wide character's second cell belongs to it.
            b += 1;
            continue;
        }
        b += 1;
    }
    let w = text(&cells, a, b).trim_end_matches(['.', ':', ',']).to_string();
    (!w.is_empty()).then_some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_agents_print() {
        let mut p = vt100::Parser::new(4, 100, 0);
        p.process("• View the tileset (artifacts/undead/terrain/v2/revenant-environment.png)\r\n".as_bytes());
        p.process(b"see src/client/hydra.rs:42 and README.md. Plain words here.\r\n");
        let s = p.screen();
        assert_eq!(path_at(s, 0, 30), Some(("artifacts/undead/terrain/v2/revenant-environment.png".into(), None)), "inside the brackets, without them");
        assert_eq!(path_at(s, 1, 8), Some(("src/client/hydra.rs".into(), Some(42))), "path:line");
        assert_eq!(path_at(s, 1, 33), Some(("README.md".into(), None)), "a file name, the full stop dropped");
        assert_eq!(path_at(s, 1, 46), None, "plain words aren't paths");
    }

    #[test]
    fn links_and_words() {
        let mut p = vt100::Parser::new(3, 80, 0);
        p.process(b"see https://example.com/a(b). and src/main.rs:12 now");
        let s = p.screen();
        assert_eq!(url_at(s, 0, 10).as_deref(), Some("https://example.com/a(b)"));
        assert_eq!(url_at(s, 0, 1), None);
        assert_eq!(word_at(s, 0, 36).as_deref(), Some("src/main.rs:12"));
        assert_eq!(word_at(s, 0, 3), None, "a space");
    }
}
