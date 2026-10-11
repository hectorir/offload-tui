//! Color themes. Truecolor where supported, graceful 256-color fallback (e.g. macOS Terminal.app).

use ratatui::style::Color;
use std::sync::OnceLock;

pub const NAMES: [&str; 5] = ["catppuccin", "nord", "gruvbox", "dracula", "mono"];

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub accent2: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    pub border: Color,
    pub sel_bg: Color,
    /// RGB endpoints for gradient bars.
    pub grad: ((u8, u8, u8), (u8, u8, u8)),
}

fn truecolor() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var("COLORTERM").map(|v| v.contains("truecolor") || v.contains("24bit")).unwrap_or(false))
}

/// Nearest xterm-256 color for an RGB triple.
pub fn to_256(r: u8, g: u8, b: u8) -> Color {
    let q = |v: u8| ((v as f32 / 255.0) * 5.0).round() as u8;
    Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
}

pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    if truecolor() { Color::Rgb(r, g, b) } else { to_256(r, g, b) }
}

pub fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

impl Theme {
    pub fn by_name(name: &str) -> Theme {
        match name.to_lowercase().as_str() {
            "nord" => Theme {
                name: "nord",
                fg: rgb(0xd8, 0xde, 0xe9),
                dim: rgb(0x6c, 0x77, 0x8f),
                accent: rgb(0x88, 0xc0, 0xd0),
                accent2: rgb(0xb4, 0x8e, 0xad),
                ok: rgb(0xa3, 0xbe, 0x8c),
                warn: rgb(0xeb, 0xcb, 0x8b),
                err: rgb(0xbf, 0x61, 0x6a),
                border: rgb(0x4c, 0x56, 0x6a),
                sel_bg: rgb(0x3b, 0x42, 0x52),
                grad: ((0x5e, 0x81, 0xac), (0x88, 0xc0, 0xd0)),
            },
            "gruvbox" => Theme {
                name: "gruvbox",
                fg: rgb(0xeb, 0xdb, 0xb2),
                dim: rgb(0x92, 0x83, 0x74),
                accent: rgb(0xfa, 0xbd, 0x2f),
                accent2: rgb(0xfe, 0x80, 0x19),
                ok: rgb(0xb8, 0xbb, 0x26),
                warn: rgb(0xfa, 0xbd, 0x2f),
                err: rgb(0xfb, 0x49, 0x34),
                border: rgb(0x50, 0x49, 0x45),
                sel_bg: rgb(0x3c, 0x38, 0x36),
                grad: ((0xd7, 0x99, 0x21), (0xb8, 0xbb, 0x26)),
            },
            "dracula" => Theme {
                name: "dracula",
                fg: rgb(0xf8, 0xf8, 0xf2),
                dim: rgb(0x62, 0x72, 0xa4),
                accent: rgb(0xbd, 0x93, 0xf9),
                accent2: rgb(0xff, 0x79, 0xc6),
                ok: rgb(0x50, 0xfa, 0x7b),
                warn: rgb(0xf1, 0xfa, 0x8c),
                err: rgb(0xff, 0x55, 0x55),
                border: rgb(0x44, 0x47, 0x5a),
                sel_bg: rgb(0x44, 0x47, 0x5a),
                grad: ((0xbd, 0x93, 0xf9), (0xff, 0x79, 0xc6)),
            },
            "mono" => Theme {
                name: "mono",
                fg: Color::Reset,
                dim: Color::DarkGray,
                accent: Color::White,
                accent2: Color::Gray,
                ok: Color::White,
                warn: Color::Gray,
                err: Color::White,
                border: Color::DarkGray,
                sel_bg: Color::DarkGray,
                grad: ((0xcc, 0xcc, 0xcc), (0xff, 0xff, 0xff)),
            },
            _ => Theme {
                name: "catppuccin",
                fg: rgb(0xcd, 0xd6, 0xf4),
                dim: rgb(0x6c, 0x70, 0x86),
                accent: rgb(0x89, 0xb4, 0xfa),
                accent2: rgb(0xcb, 0xa6, 0xf7),
                ok: rgb(0xa6, 0xe3, 0xa1),
                warn: rgb(0xf9, 0xe2, 0xaf),
                err: rgb(0xf3, 0x8b, 0xa8),
                border: rgb(0x45, 0x47, 0x5a),
                sel_bg: rgb(0x31, 0x32, 0x44),
                grad: ((0x89, 0xb4, 0xfa), (0xa6, 0xe3, 0xa1)),
            },
        }
    }

    pub fn next(&self) -> Theme {
        let i = NAMES.iter().position(|n| *n == self.name).unwrap_or(0);
        Theme::by_name(NAMES[(i + 1) % NAMES.len()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_resolves_and_cycles() {
        let mut t = Theme::by_name("catppuccin");
        let mut seen = vec![];
        for _ in 0..NAMES.len() {
            seen.push(t.name);
            t = t.next();
        }
        assert_eq!(seen, NAMES);
        assert_eq!(t.name, "catppuccin", "full cycle returns to start");
        assert_eq!(Theme::by_name("bogus").name, "catppuccin");
    }

    #[test]
    fn indexed_fallback_in_range() {
        assert_eq!(to_256(0, 0, 0), Color::Indexed(16));
        assert_eq!(to_256(255, 255, 255), Color::Indexed(231));
    }
}
