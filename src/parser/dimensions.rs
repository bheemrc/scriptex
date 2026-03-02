use crate::lexer::TokenKind;
use super::Parser;

impl<'a> Parser<'a> {
    /// Read a TeX dimension without braces (e.g., after \vskip: "10pt", "-2.5mm plus 1fil")
    pub(super) fn read_tex_dimension_text(&mut self) -> String {
        self.skip_whitespace_and_comments();
        let mut text = String::new();
        // Read optional sign
        if self.current().kind == TokenKind::Text || self.current().kind == TokenKind::Command {
            let t = self.current().text(self.source);
            if t == "-" || t == "+" { text.push_str(t); self.advance(); }
        }
        // Read number + unit tokens until non-dimension token
        loop {
            match self.current().kind {
                TokenKind::Text => {
                    let t = self.current().text(self.source);
                    // Part of a dimension: digits, dots, units
                    if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+')
                        || ["pt", "mm", "cm", "in", "em", "ex", "sp", "bp", "dd", "pc", "mu", "fil", "fill"].contains(&t)
                    {
                        text.push_str(t);
                        self.advance();
                    } else if t == "plus" || t == "minus" {
                        // Glue component — stop at main dimension
                        break;
                    } else {
                        break;
                    }
                }
                TokenKind::Space => {
                    // Space might separate number from unit
                    if text.is_empty() || text.chars().last().map_or(false, |c| c.is_ascii_digit() || c == '.') {
                        self.advance();
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        text
    }

    pub(super) fn parse_dimension(&self, text: &str) -> Option<f32> {
        let text = text.trim();
        // Try to parse dimension with unit
        let (num_str, unit) = if text.ends_with("pt") {
            (&text[..text.len()-2], "pt")
        } else if text.ends_with("mm") {
            (&text[..text.len()-2], "mm")
        } else if text.ends_with("cm") {
            (&text[..text.len()-2], "cm")
        } else if text.ends_with("in") {
            (&text[..text.len()-2], "in")
        } else if text.ends_with("em") {
            (&text[..text.len()-2], "em")
        } else if text.ends_with("ex") {
            (&text[..text.len()-2], "ex")
        } else if text.ends_with("bp") {
            (&text[..text.len()-2], "bp")
        } else if text.ends_with("pc") {
            (&text[..text.len()-2], "pc")
        } else {
            (text, "pt")
        };

        let value: f32 = num_str.trim().parse().ok()?;
        let points = match unit {
            "pt" => value,
            "mm" => value * 2.83465,
            "cm" => value * 28.3465,
            "in" => value * 72.0,
            "em" => value * 10.0,
            "ex" => value * 5.0,
            "bp" => value,
            "pc" => value * 12.0,
            _ => value,
        };
        Some(points)
    }

    /// Parse a dimension that may include \textwidth, \linewidth, \columnwidth factors.
    /// E.g. "0.48\textwidth" → 0.48 * default_textwidth, "5cm" → normal dimension.
    pub(super) fn parse_dimension_with_textwidth(&self, text: &str, default_textwidth: f32) -> f32 {
        let text = text.trim();
        // Check for factor * \textwidth pattern
        for keyword in &["\\textwidth", "\\linewidth", "\\columnwidth", "\\hsize"] {
            if let Some(idx) = text.find(keyword) {
                let factor_str = text[..idx].trim();
                let factor: f32 = if factor_str.is_empty() {
                    1.0
                } else {
                    factor_str.parse().unwrap_or(1.0)
                };
                return factor * default_textwidth;
            }
        }
        self.parse_dimension(text).unwrap_or(300.0)
    }
}

/// Parse a simple dimension string like "3mm", "0.5pt", "4mm" to points.
pub(super) fn parse_dimension_simple(s: &str) -> Option<f32> {
    let s = s.trim();
    let (num, unit) = if s.ends_with("mm") {
        (s[..s.len()-2].trim(), "mm")
    } else if s.ends_with("cm") {
        (s[..s.len()-2].trim(), "cm")
    } else if s.ends_with("pt") {
        (s[..s.len()-2].trim(), "pt")
    } else if s.ends_with("in") {
        (s[..s.len()-2].trim(), "in")
    } else if s.ends_with("em") {
        (s[..s.len()-2].trim(), "em")
    } else if s.ends_with("ex") {
        (s[..s.len()-2].trim(), "ex")
    } else {
        (s, "pt")
    };
    let val: f32 = num.parse().ok()?;
    let pts = match unit {
        "mm" => val * 2.8346,
        "cm" => val * 28.346,
        "in" => val * 72.0,
        "em" => val * 10.0,
        "ex" => val * 4.3,
        _ => val,
    };
    Some(pts)
}
