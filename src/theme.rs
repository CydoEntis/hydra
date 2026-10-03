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
            working: hex(c[10]),
            blocked: hex(c[11]),
            done: hex(c[12]),
            idle: hex(c[13]),
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
            working: h(7),
            blocked: h(11),
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
                    "#eeeeee", "#00afaf", "#1c1c1c", "#ffaf00", "#5faf00", "#af005f", "#3a3a3a", "#5f5faf",
                ],
                ["#af87d7", "#5fafd7", "#ff5faf", "#5f8787"],
                ["#af005f", "#5faf00", "#d7af5f", "#5fafd7", "#af87d7", "#00afaf", "#808080"],
            ),
            "tango-dark" => Theme::design(
                [
                    "#2e3436", "#eeeeec", "#252a2b", "#363c3e", "#41474a", "#555753", "#9a9c97", "#d3d7cf",
                    "#eeeeec", "#729fcf", "#2e3436", "#fcaf3e", "#73d216", "#ef2929", "#4a5052", "#204a87",
                ],
                ["#ad7fa8", "#729fcf", "#e9b96e", "#34e2e2"],
                ["#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#888a85"],
            ),
            "monokai" => Theme::design(
                [
                    "#272822", "#f8f8f2", "#1e1f1c", "#2f302a", "#3e3d32", "#49483e", "#8f8a72", "#cfcfc2",
                    "#f8f8f2", "#66d9ef", "#272822", "#fd971f", "#a6e22e", "#f92672", "#3e3d32", "#55544a",
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
                "#eff1f5", "#4c4f69", "#8c8fa1", "#1e66f5", "#bcc0cc", "#1e66f5", "#e6e9ef",
                "#ccd0da", "#1e66f5", "#eff1f5", "#df8e1d", "#d20f39", "#40a02b", "#8c8fa1",
            ]),
            "gruvbox" => Theme::classic([
                "#282828", "#ebdbb2", "#928374", "#fabd2f", "#504945", "#fabd2f", "#1d2021",
                "#3c3836", "#fabd2f", "#282828", "#fe8019", "#fb4934", "#83a598", "#b8bb26",
            ]),
            "nord" => Theme::classic([
                "#2e3440", "#eceff4", "#616e88", "#88c0d0", "#434c5e", "#88c0d0", "#272c36",
                "#3b4252", "#88c0d0", "#2e3440", "#ebcb8b", "#bf616a", "#81a1c1", "#a3be8c",
            ]),
            "dracula" => Theme::classic([
                "#282a36", "#f8f8f2", "#6272a4", "#bd93f9", "#44475a", "#bd93f9", "#21222c",
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
                working: Color::Gray,
                blocked: Color::Yellow,
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
