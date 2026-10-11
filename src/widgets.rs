//! Custom widgets: gradient bars, text bars, logos, spinners.

use crate::theme::{Theme, lerp};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A smooth, gradient-filled progress bar of `width` cells.
pub fn gradient_bar(width: usize, ratio: f64, t: &Theme) -> Vec<Span<'static>> {
    let ratio = ratio.clamp(0.0, 1.0);
    let total_eighths = (ratio * width as f64 * 8.0).round() as usize;
    let full = total_eighths / 8;
    let part = total_eighths % 8;
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        let color = lerp(t.grad.0, t.grad.1, if width > 1 { i as f32 / (width - 1) as f32 } else { 1.0 });
        if i < full {
            spans.push(Span::styled("█", Style::default().fg(color)));
        } else if i == full && part > 0 {
            spans.push(Span::styled(EIGHTHS[part].to_string(), Style::default().fg(color)));
        } else {
            spans.push(Span::styled("░", Style::default().fg(t.border)));
        }
    }
    spans
}

/// Plain single-color bar (for breakdown charts).
pub fn solid_bar(width: usize, ratio: f64, color: Color) -> Span<'static> {
    let eighths = (ratio.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let mut s = "█".repeat(eighths / 8);
    if !eighths.is_multiple_of(8) {
        s.push(EIGHTHS[eighths % 8]);
    }
    Span::styled(s, Style::default().fg(color))
}

/// Height of every logo, in rows.
pub const LOGO_H: usize = 5;

/// Line art for a camera icon. `action` is a DJI Osmo Action (square body, big lens, front screen);
/// `mirrorless` is a Sony ZV-E1 (compact body, deep grip, windscreen, big E-mount lens, tally light).
pub fn logo(icon: &str) -> [&'static str; LOGO_H] {
    match icon {
        "action" | "osmo" => ["  ▁▂▁      ", "┏━━━━━━━━┓ ", "┃ ╭────╮▫┃ ", "┃ │(◉) │ ┃ ", "┗━╰────╯▬┛ "],
        "mirrorless" | "zv-e1" | "zve1" => ["   ▂▇▇▇▂    ●", "╭────────╮▗▖ ", "│ ╭────╮ │██ ", "│ │(◎) │ │██ ", "╰─╰────╯─╯▀▀ "],
        _ => ["  ┌▀▀┐   ", "┌─┴──┴──┐", "│ ╭──╮ ▫│", "│ │ ◎│  │", "└─╰──╯──┘"],
    }
}

/// Two cameras side by side, for the batch card.
pub fn logo_batch() -> [&'static str; LOGO_H] {
    [" ▁▂▁     ▂▇▇▂    ", "┏━━━━┓  ╭────╮▗  ", "┃(◉) ┃ +│(◎) │█  ", "┗━━━━┛  ╰────╯▀  ", "                 "]
}

/// Display width of a logo in cells.
pub fn logo_width(art: &[&str]) -> usize {
    art.iter().map(|l| l.chars().count()).max().unwrap_or(0)
}

fn glyph_style(c: char, t: &Theme, selected: bool) -> Style {
    match c {
        '◉' | '◎' => Style::default().fg(t.accent2).add_modifier(Modifier::BOLD), // the glass
        '●' => Style::default().fg(t.err),                                        // tally light
        '▁' | '▂' | '▃' | '▅' | '▆' | '▇' | '█' | '▗' | '▖' | '▐' | '▀' | '▄' | '▫' | '▬' | '+' => {
            Style::default().fg(if selected { t.fg } else { t.dim })
        }
        _ => Style::default().fg(if selected { t.accent } else { t.dim }),
    }
}

/// Render logo art with per-glyph colors (lens, tally light, grip/accessories, body).
pub fn logo_lines(art: &[&'static str], t: &Theme, selected: bool) -> Vec<Line<'static>> {
    art.iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = vec![];
            let mut cur = String::new();
            let mut cur_style: Option<Style> = None;
            for c in row.chars() {
                let st = glyph_style(c, t, selected);
                if cur_style != Some(st) && !cur.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut cur), cur_style.unwrap_or_default()));
                }
                cur_style = Some(st);
                cur.push(c);
            }
            if !cur.is_empty() {
                spans.push(Span::styled(cur, cur_style.unwrap_or_default()));
            }
            Line::from(spans)
        })
        .collect()
}

pub fn spinner(frame: usize) -> &'static str {
    SPINNER[frame % SPINNER.len()]
}

/// Big block-letter wordmark.
pub const WORDMARK: [&str; 3] = ["╔═╗╔═╗╔═╗╦  ╔═╗╔═╗╔╦╗", "║ ║╠╣ ╠╣ ║  ║ ║╠═╣ ║║", "╚═╝╚  ╚  ╩═╝╚═╝╩ ╩═╩╝"];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn gradient_bar_widths_and_fill() {
        let t = Theme::by_name("nord");
        assert_eq!(text(&gradient_bar(10, 0.0, &t)), "░".repeat(10));
        assert_eq!(text(&gradient_bar(10, 1.0, &t)), "█".repeat(10));
        let half = text(&gradient_bar(10, 0.5, &t));
        assert_eq!(half.chars().count(), 10);
        assert!(half.starts_with("█████"));
        assert_eq!(gradient_bar(0, 0.5, &t).len(), 0);
        assert_eq!(gradient_bar(10, 7.0, &t).len(), 10, "clamped");
    }

    #[test]
    fn solid_bar_partial_cells() {
        assert_eq!(solid_bar(4, 0.5, Color::Red).content, "██");
        assert_eq!(solid_bar(4, 0.625, Color::Red).content, "██▌");
    }

    #[test]
    fn logos_have_consistent_height_and_are_distinct() {
        for i in ["action", "mirrorless", "camera", "zzz"] {
            assert_eq!(logo(i).len(), LOGO_H);
        }
        assert_eq!(logo_batch().len(), LOGO_H);
        assert_ne!(logo("action"), logo("mirrorless"));
        assert_ne!(logo("action"), logo("camera"));
        assert_eq!(logo("osmo"), logo("action"));
        assert_eq!(logo("zv-e1"), logo("mirrorless"));
    }

    #[test]
    fn colored_logo_preserves_every_character() {
        let t = Theme::by_name("nord");
        for art in [logo("action"), logo("mirrorless"), logo("camera"), logo_batch()] {
            let lines = logo_lines(&art, &t, true);
            for (row, line) in art.iter().zip(&lines) {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(&text, row);
            }
        }
    }
}
