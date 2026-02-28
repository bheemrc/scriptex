/// Typesetting metrics and font measurement utilities
/// Uses approximate metrics for built-in fonts (no external font files needed)

use crate::document::FontSizeSpec;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Monospace,
    SmallCaps,
}

#[derive(Debug, Clone, Copy)]
pub struct FontMetrics {
    pub style: FontStyle,
    pub size: f32,
}

impl FontMetrics {
    pub fn new(size: f32, style: FontStyle) -> Self {
        FontMetrics { style, size }
    }

    /// Average character width for the font
    pub fn char_width(&self) -> f32 {
        let base = match self.style {
            FontStyle::Monospace => 0.6,
            FontStyle::SmallCaps => 0.55,
            FontStyle::Bold | FontStyle::BoldItalic => 0.56,
            _ => 0.5,
        };
        self.size * base
    }

    /// Width of a specific character (approximate)
    pub fn measure_char(&self, ch: char) -> f32 {
        let ratio = match ch {
            'W' | 'M' => 0.85,
            'w' | 'm' => 0.75,
            'i' | 'l' | 'j' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' => 0.28,
            'f' | 't' | 'r' => 0.35,
            ' ' => 0.25,
            'A'..='Z' => 0.65,
            'a'..='z' => 0.48,
            '0'..='9' => 0.5,
            _ => 0.5,
        };
        let style_factor = match self.style {
            FontStyle::Bold | FontStyle::BoldItalic => 1.05,
            FontStyle::Monospace => 1.0, // fixed width
            FontStyle::SmallCaps => 0.85,
            _ => 1.0,
        };
        if self.style == FontStyle::Monospace {
            self.size * 0.6 // all chars same width
        } else {
            self.size * ratio * style_factor
        }
    }

    /// Measure text width
    pub fn measure_text(&self, text: &str) -> f32 {
        text.chars().map(|c| self.measure_char(c)).sum()
    }

    /// Line height (leading)
    pub fn line_height(&self) -> f32 {
        self.size * 1.2
    }

    /// Ascent above baseline
    pub fn ascent(&self) -> f32 {
        self.size * 0.8
    }

    /// Descent below baseline
    pub fn descent(&self) -> f32 {
        self.size * 0.2
    }

    /// Cap height
    pub fn cap_height(&self) -> f32 {
        self.size * 0.7
    }

    /// x-height
    pub fn x_height(&self) -> f32 {
        self.size * 0.45
    }
}

impl Default for FontMetrics {
    fn default() -> Self {
        FontMetrics {
            style: FontStyle::Regular,
            size: 10.0,
        }
    }
}

/// Fast word width measurement using average char width
/// This is much faster than per-char measurement for typical text
#[inline]
fn fast_word_width(word: &str, avg_char_width: f32) -> f32 {
    word.len() as f32 * avg_char_width
}

/// Word wrapping: breaks text into lines that fit within max_width
/// Optimized for speed with fast width estimation
pub fn wrap_text(text: &str, metrics: &FontMetrics, max_width: f32) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    // Fast average char width for this font
    let avg_width = metrics.size * match metrics.style {
        FontStyle::Monospace => 0.6,
        FontStyle::Bold | FontStyle::BoldItalic => 0.52,
        _ => 0.48,
    };
    let space_width = metrics.size * 0.25;

    // Estimate chars per line for capacity hint
    let chars_per_line = (max_width / avg_width) as usize;
    let estimated_lines = text.len() / chars_per_line.max(1) + 1;

    let mut lines = Vec::with_capacity(estimated_lines);
    let mut current_line = String::with_capacity(chars_per_line + 10);
    let mut current_width: f32 = 0.0;

    for word in text.split_whitespace() {
        let word_width = fast_word_width(word, avg_width);

        if current_line.is_empty() {
            current_line.push_str(word);
            current_width = word_width;
        } else if current_width + space_width + word_width <= max_width {
            current_line.push(' ');
            current_line.push_str(word);
            current_width += space_width + word_width;
        } else {
            lines.push(std::mem::take(&mut current_line));
            current_line = String::with_capacity(chars_per_line + 10);
            current_line.push_str(word);
            current_width = word_width;
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

/// Knuth-Plass style line breaking for high-quality justification
pub fn optimal_line_breaks(text: &str, metrics: &FontMetrics, max_width: f32) -> Vec<(usize, usize)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }

    let n = words.len();
    let space_width = metrics.measure_char(' ');

    // DP for optimal breaks
    let mut cost = vec![f64::MAX; n + 1];
    let mut breaks = vec![0usize; n + 1];
    cost[0] = 0.0;

    for j in 1..=n {
        let mut line_width: f32 = 0.0;
        for i in (0..j).rev() {
            let word_width = metrics.measure_text(words[i]);
            if i == j - 1 {
                line_width = word_width;
            } else {
                line_width += space_width + word_width;
            }

            if line_width > max_width && i < j - 1 {
                break;
            }

            let penalty = if j == n {
                0.0 // no penalty for last line
            } else {
                let slack = (max_width - line_width) as f64;
                slack * slack * slack // cubic penalty
            };

            let total = cost[i] + penalty;
            if total < cost[j] {
                cost[j] = total;
                breaks[j] = i;
            }
        }
    }

    // Reconstruct break positions
    let mut result = Vec::new();
    let mut j = n;
    while j > 0 {
        let i = breaks[j];
        result.push((i, j));
        j = i;
    }
    result.reverse();
    result
}
