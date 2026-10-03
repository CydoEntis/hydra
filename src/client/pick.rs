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
