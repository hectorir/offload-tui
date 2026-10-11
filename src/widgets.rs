//! Custom widgets: gradient bars, text bars, logos, spinners.

use crate::theme::{Theme, lerp};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

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

pub fn logo(icon: &str) -> [&'static str; 4] {
    match icon {
        "action" => ["        ", "┌─────┐▪", "│  ◉  │ ", "└─────┘ "],
        "mirrorless" => ["   ▄▄▄  ", "╭───────╮", "│  (◉)  │", "╰───────╯"],
        _ => ["  ┌▀▀┐  ", "┌─┴──┴─┐", "│  ◎   │", "└──────┘"],
    }
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
    fn logos_have_consistent_height() {
        for i in ["action", "mirrorless", "camera", "zzz"] {
            assert_eq!(logo(i).len(), 4);
        }
    }
}
