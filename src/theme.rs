//! Built-in themes plus per-key overrides from config. Pane contents keep the program's own
//! colours; the theme only paints hydra's chrome (sidebar, title bars, status line).
//!
//! The roles follow the design handoff: `surf` for chrome, `card`/`card2` for panels and
//! action rows, `btn`/`hov` for buttons and the hovered row, `acc` (lime in the default
//! theme) for focus, `needs` (amber) only for "needs you".

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Theme {
    /// Ground, and the default background of terminal cells.
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    /// Focus, primary worktree, primary buttons, keys.
    pub accent: Color,
    pub border: Color,
    pub border_active: Color,
    /// Sidebar, status line, unfocused title bars (`surf` in the design).
    pub sidebar_bg: Color,
    pub selection_bg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub working: Color,
    /// "Needs you". Nothing else uses this colour.
    pub blocked: Color,
    pub done: Color,
    pub idle: Color,
    /// Modal and menu panels.
    pub card: Color,
    /// Answer bar, action rows, modal input.
    pub card2: Color,
    /// Normal button ground.
    pub btn: Color,
    /// Hovered row or button, keyboard cursor row.
    pub hov: Color,
    /// Dividers and tree glyphs.
    pub line: Color,
    /// Secondary text.
    pub text: Color,
    /// Names and titles.
    pub strong: Color,
    /// Text on the accent colour.
    pub acc_ink: Color,
    /// Discard, failures.
    pub err: Color,
    /// Project colours (violet, sky, pink, teal), when the theme has its own.
    pub ws: Option<[Color; 4]>,
    /// Terminal colours for agent output (red, green, yellow, blue, magenta, cyan, gray),
    /// when the theme has its own; otherwise programs keep the terminal's palette.
    pub ansi: Option<[Color; 7]>,
}

/// Every field optional, as hex strings ("#89b4fa") or names ("red", "reset", "123").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeOverrides(pub BTreeMap<String, String>);

pub const BUILTIN: &[&str] = &[
    "hydra",
    "papercolor-dark",
    "tango-dark",
    "monokai",
    "tokyo-night",
    "catppuccin-mocha",
    "catppuccin-latte",
    "gruvbox",
    "nord",
    "dracula",
    "mono",
];

/// The themes from the design handoff, in its order (shown as chips in Settings).
pub const DESIGN: &[(&str, &str)] = &[
    ("hydra", "Default"),
    ("papercolor-dark", "PaperColor Dark"),
    ("tango-dark", "Tango Dark"),
    ("monokai", "Monokai"),
    ("tokyo-night", "Tokyo Night"),
];

fn hex(s: &str) -> Color {
    parse_color(s).unwrap_or(Color::Reset)
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        if h.len() == 6 {
            let v = u32::from_str_radix(h, 16).ok()?;
            return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return None;
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Color::Indexed(n));
    }
    s.parse::<Color>().ok()
}

/// WCAG contrast between two colours (1 to 21).
pub fn contrast(a: Color, b: Color) -> f64 {
    let l = |c: Color| match c {
        Color::Rgb(r, g, b) => {
            let ch = |v: u8| {
                let s = v as f64 / 255.0;
                if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
        }
        _ => 0.5,
    };
    let (x, y) = (l(a), l(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Mix `b` into `a` by `t`; colours that aren't RGB can't be mixed and fall back.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t.clamp(0.0, 1.0)).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

impl Theme {
    /// The fill of a destructive button (Close): the error red, a little deeper when its
    /// text wouldn't read on it otherwise.
    pub fn danger_fill(&self) -> Color {
        let mut c = self.err;
        for _ in 0..6 {
            if contrast(self.ink_on(c), c) >= 4.5 {
                break;
            }
            c = mix(c, Color::Rgb(0, 0, 0), 0.08);
        }
        c
    }

    /// Text to put on a filled `c`: the theme's background or its strongest text, whichever
    /// reads better (light themes need dark ink on orange, dark themes light).
    pub fn ink_on(&self, c: Color) -> Color {
        // WCAG relative luminance and contrast.
        let l = |c: Color| match c {
            Color::Rgb(r, g, b) => {
                let ch = |v: u8| {
                    let s = v as f64 / 255.0;
                    if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
                };
                0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
            }
            _ => 0.5,
        };
        let contrast = |a: f64, b: f64| (a.max(b) + 0.05) / (a.min(b) + 0.05);
        let fill = l(c);
        if contrast(l(self.bg), fill) >= contrast(l(self.strong), fill) { self.bg } else { self.strong }
    }

    /// A theme from its 14 classic colours; the newer roles are derived from them.
    fn classic(c: [&str; 14]) -> Theme {
        let (bg, fg, muted, border) = (hex(c[0]), hex(c[1]), hex(c[2]), hex(c[4]));
        Theme {
            bg,
            fg,
            muted,
            accent: hex(c[3]),
            border,
            border_active: hex(c[5]),
            sidebar_bg: hex(c[6]),
            selection_bg: hex(c[7]),
            tab_active_bg: hex(c[8]),
            tab_active_fg: hex(c[9]),
            // Working yellow, needs you red, done green, idle dim.
            working: hex(c[10]),
            blocked: hex(c[11]),
            done: hex(c[13]),
            idle: muted,
            card: mix(bg, fg, 0.05),
            card2: mix(bg, fg, 0.11),
            btn: mix(bg, fg, 0.14),
            hov: hex(c[7]),
            line: border,
            text: mix(muted, fg, 0.5),
            strong: fg,
            acc_ink: bg,
            err: hex(c[11]),
            ws: None,
            ansi: None,
        }
    }

    /// A theme from the design handoff's roles.
    /// `c`: bg fg surf card card2 line dim text strong acc accInk needs ok err btn hov.
    fn design(c: [&str; 16], ws: [&str; 4], a: [&str; 7]) -> Theme {
        let h = |i: usize| hex(c[i]);
        Theme {
            bg: h(0),
            fg: h(1),
            muted: h(6),
            accent: h(9),
            border: h(5),
            border_active: h(9),
            sidebar_bg: h(2),
            selection_bg: h(15),
            tab_active_bg: h(9),
            tab_active_fg: h(10),
            // Working yellow, needs you red, done green, idle dim.
            working: h(11),
            blocked: h(13),
            done: h(12),
            idle: h(6),
            card: h(3),
            card2: h(4),
            btn: h(14),
            hov: h(15),
            line: h(5),
            text: h(7),
            strong: h(8),
            acc_ink: h(10),
            err: h(13),
            ws: Some(ws.map(hex)),
            ansi: Some(a.map(hex)),
        }
    }

    pub fn named(name: &str) -> Theme {
        match name {
            // The design handoff's palette ("drover" is the app's old name).
            "hydra" | "drover" => Theme::design(
                [
                    "#070b10", "#c9d1d9", "#0c131b", "#0f1821", "#18242f", "#1f2c3a", "#71808f", "#a7b4c2",
                    "#f2f6f8", "#c3f53c", "#0a1204", "#ffb547", "#7fd962", "#ff6b6b", "#1d2a37", "#2a3a4c",
                ],
                ["#a593ff", "#5aa9ff", "#ff7ab6", "#3dd6c0"],
                ["#ff6b6b", "#7fd962", "#e8c565", "#5aa9ff", "#a593ff", "#3dd6c0", "#71808f"],
            ),
            "papercolor-dark" => Theme::design(
                [
                    "#1c1c1c", "#d0d0d0", "#262626", "#303030", "#3a3a3a", "#444444", "#808080", "#b2b2b2",
                    "#eeeeee", "#00afaf", "#1c1c1c", "#ffaf00", "#5faf00", "#ff5f87", "#3a3a3a", "#5f5faf",
                ],
                ["#af87d7", "#5fafd7", "#ff5faf", "#5f8787"],
                ["#ff5f87", "#5faf00", "#d7af5f", "#5fafd7", "#af87d7", "#00afaf", "#808080"],
            ),
            "tango-dark" => Theme::design(
                [
                    "#2e3436", "#eeeeec", "#252a2b", "#363c3e", "#41474a", "#555753", "#9a9c97", "#d3d7cf",
                    "#eeeeec", "#8ae234", "#2e3436", "#fcaf3e", "#73d216", "#ff5c5c", "#4a5052", "#204a87",
                ],
                ["#ad7fa8", "#729fcf", "#e9b96e", "#34e2e2"],
                ["#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#888a85"],
            ),
            "monokai" => Theme::design(
                [
                    "#272822", "#f8f8f2", "#1e1f1c", "#2f302a", "#3e3d32", "#49483e", "#8f8a72", "#cfcfc2",
                    "#f8f8f2", "#a6e22e", "#272822", "#fd971f", "#a6e22e", "#f92672", "#3e3d32", "#55544a",
                ],
                ["#ae81ff", "#66d9ef", "#f92672", "#a1efe4"],
                ["#f92672", "#a6e22e", "#e6db74", "#66d9ef", "#ae81ff", "#a1efe4", "#75715e"],
            ),
            "tokyo-night" => Theme::design(
                [
                    "#1a1b26", "#c0caf5", "#16161e", "#1f2335", "#292e42", "#292e42", "#7a82ad", "#a9b1d6",
                    "#e0e6ff", "#7aa2f7", "#1a1b26", "#e0af68", "#9ece6a", "#f7768e", "#292e42", "#3b4261",
                ],
                ["#bb9af7", "#7dcfff", "#f7768e", "#73daca"],
                ["#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#565f89"],
            ),
            "catppuccin-mocha" => Theme::classic([
                "#1e1e2e", "#cdd6f4", "#6c7086", "#89b4fa", "#45475a", "#89b4fa", "#181825",
                "#313244", "#89b4fa", "#1e1e2e", "#f9e2af", "#f38ba8", "#89dceb", "#a6e3a1",
            ]),
            "catppuccin-latte" => Theme::classic([
                "#eff1f5", "#303446", "#6c6f85", "#7a2fd8", "#bcc0cc", "#7a2fd8", "#e6e9ef",
                "#ccd0da", "#7a2fd8", "#eff1f5", "#8a5a00", "#d20f39", "#1e5ad8", "#2f8a1f",
            ]),
            "gruvbox" => Theme::classic([
                "#282828", "#ebdbb2", "#928374", "#fabd2f", "#504945", "#fabd2f", "#1d2021",
                "#3c3836", "#fabd2f", "#282828", "#fe8019", "#fb4934", "#83a598", "#b8bb26",
            ]),
            "nord" => Theme::classic([
                "#2e3440", "#eceff4", "#8390a8", "#a3be8c", "#434c5e", "#a3be8c", "#272c36",
                "#3b4252", "#a3be8c", "#2e3440", "#ebcb8b", "#e0707a", "#81a1c1", "#a3be8c",
            ]),
            "dracula" => Theme::classic([
                "#282a36", "#f8f8f2", "#7a8ac0", "#bd93f9", "#44475a", "#bd93f9", "#21222c",
                "#44475a", "#bd93f9", "#282a36", "#f1fa8c", "#ff5555", "#8be9fd", "#50fa7b",
            ]),
            "mono" => Theme {
                bg: Color::Reset,
                fg: Color::Reset,
                muted: Color::DarkGray,
                accent: Color::White,
                border: Color::DarkGray,
                border_active: Color::White,
                sidebar_bg: Color::Reset,
                selection_bg: Color::DarkGray,
                tab_active_bg: Color::White,
                tab_active_fg: Color::Black,
                working: Color::Yellow,
                blocked: Color::Red,
                done: Color::Green,
                idle: Color::DarkGray,
                card: Color::Black,
                card2: Color::DarkGray,
                btn: Color::DarkGray,
                hov: Color::DarkGray,
                line: Color::DarkGray,
                text: Color::Gray,
                strong: Color::White,
                acc_ink: Color::Black,
                err: Color::Red,
                ws: None,
                ansi: None,
            },
            _ => Theme::named("hydra"),
        }
    }

    pub fn apply(&mut self, o: &ThemeOverrides) {
        for (k, v) in &o.0 {
            let Some(c) = parse_color(v) else { continue };
            let slot = match k.replace('-', "_").as_str() {
                "bg" => &mut self.bg,
                "fg" => &mut self.fg,
                "muted" | "dim" => &mut self.muted,
                "accent" | "acc" => &mut self.accent,
                "border" => &mut self.border,
                "border_active" => &mut self.border_active,
                "sidebar_bg" | "surf" => &mut self.sidebar_bg,
                "selection_bg" => &mut self.selection_bg,
                "tab_active_bg" => &mut self.tab_active_bg,
                "tab_active_fg" => &mut self.tab_active_fg,
                "working" => &mut self.working,
                "blocked" | "needs" => &mut self.blocked,
                "done" | "ok" => &mut self.done,
                "idle" => &mut self.idle,
                "card" => &mut self.card,
                "card2" => &mut self.card2,
                "btn" => &mut self.btn,
                "hov" => &mut self.hov,
                "line" => &mut self.line,
                "text" => &mut self.text,
                "strong" => &mut self.strong,
                "acc_ink" => &mut self.acc_ink,
                "err" => &mut self.err,
                _ => continue,
            };
            *slot = c;
        }
    }

    /// A project's colour by its index: the theme's four, then yellow and dim.
    pub fn project(&self, i: usize, fallback: &[Color]) -> Color {
        match (&self.ws, &self.ansi) {
            (Some(ws), Some(a)) => [ws[0], ws[1], ws[2], ws[3], a[2], self.muted][i % 6],
            _ if !fallback.is_empty() => fallback[i % fallback.len()],
            _ => self.accent,
        }
    }

    /// The colour the splash gradients run to (teal).
    pub fn teal(&self) -> Color {
        self.ws.map(|w| w[3]).unwrap_or(self.done)
    }

    pub fn status(&self, s: crate::protocol::Status) -> Color {
        use crate::protocol::Status::*;
        match s {
            Working => self.working,
            Blocked => self.blocked,
            Done => self.done,
            Idle => self.idle,
            None => self.muted,
        }
    }
}

#[cfg(test)]
mod audit {
    use super::Theme;
    use ratatui::style::Color;

    fn lum(c: Color) -> Option<f64> {
        let Color::Rgb(r, g, b) = c else { return None };
        let ch = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        Some(0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b))
    }

    pub fn contrast(a: Color, b: Color) -> f64 {
        match (lum(a), lum(b)) {
            (Some(x), Some(y)) => (x.max(y) + 0.05) / (x.min(y) + 0.05),
            _ => 21.0,
        }
    }

    fn mix(a: Color, b: Color, t: f32) -> Color {
        super::mix(a, b, t)
    }

    /// Every theme: text readable on every surface it's drawn on, and the colours that mean
    /// different things look different.
    #[test]
    fn every_theme_reads_well() {
        let names = ["hydra", "papercolor-dark", "tango-dark", "monokai", "tokyo-night", "catppuccin-mocha", "catppuccin-latte", "gruvbox", "nord", "dracula"];
        let mut bad = Vec::new();
        for n in names {
            let t = Theme::named(n);
            let active = mix(t.sidebar_bg, t.accent, 0.16);
            let hover = mix(t.sidebar_bg, t.text, 0.10);
            let checks: Vec<(&str, Color, Color, f64)> = vec![
                ("text on bg", t.text, t.bg, 4.5),
                ("strong on bg", t.strong, t.bg, 7.0),
                ("muted on bg", t.muted, t.bg, 3.0),
                ("muted on sidebar", t.muted, t.sidebar_bg, 3.0),
                ("text on card", t.text, t.card, 4.5),
                ("text on card2", t.text, t.card2, 4.0),
                ("strong on btn", t.strong, t.btn, 4.5),
                ("accent on btn (keys)", t.accent, t.btn, 3.0),
                ("ink on accent", t.acc_ink, t.accent, 4.5),
                ("strong on open row", t.strong, active, 7.0),
                ("muted on open row", t.muted, active, 2.6),
                ("strong on hover", t.strong, hover, 7.0),
                ("strong on hov (menus)", t.strong, t.hov, 4.5),
                ("blocked on sidebar", t.blocked, t.sidebar_bg, 3.0),
                ("done on sidebar", t.done, t.sidebar_bg, 3.0),
                ("working on sidebar", t.working, t.sidebar_bg, 4.5),
                ("err on card", t.err, t.card, 3.0),
                ("ink on the red Close button", t.ink_on(t.danger_fill()), t.danger_fill(), 4.5),
            ];
            for (what, fg, bg, min) in checks {
                let c = contrast(fg, bg);
                if c < min {
                    bad.push(format!("{n:18} {what:24} {c:.2} (wants {min})"));
                }
            }
            // Different meanings, different colours.
            for (what, a, b) in [
                ("accent vs blocked", t.accent, t.blocked),
                ("done vs blocked", t.done, t.blocked),
                // Needs you is red like errors on purpose; working and done must differ.
                ("working vs done", t.working, t.done),
                ("working vs blocked", t.working, t.blocked),
            ] {
                if a == b || contrast(a, b) < 1.15 && hue_near(a, b) {
                    bad.push(format!("{n:18} {what:24} too alike"));
                }
            }
            // Open and hover rows must differ from the sidebar and each other.
            if contrast(active, t.sidebar_bg) < 1.12 || contrast(hover, t.sidebar_bg) < 1.08 {
                bad.push(format!("{n:18} open/hover rows barely show ({:.2} / {:.2})", contrast(active, t.sidebar_bg), contrast(hover, t.sidebar_bg)));
            }
        }
        for b in &bad {
            eprintln!("{b}");
        }
        assert!(bad.is_empty(), "{} theme problems", bad.len());
    }

    fn hue_near(a: Color, b: Color) -> bool {
        let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else { return false };
        let d = |x: u8, y: u8| (x as i32 - y as i32).abs();
        d(r1, r2) + d(g1, g2) + d(b1, b2) < 90
    }
}
