/// Syntax highlighting module using syntect
/// Provides code highlighting for lstlisting, minted, and verbatim environments

use crate::color::Color;
use syntect::highlighting::{ThemeSet, Style, FontStyle as SynFontStyle};
use syntect::parsing::SyntaxSet;
use syntect::easy::HighlightLines;

/// A colored text span from syntax highlighting
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub text: String,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
}

/// Highlighter wrapping syntect state
pub struct Highlighter {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl Highlighter {
    pub fn new() -> Self {
        Highlighter {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    /// Highlight a block of code, returning colored spans per line
    pub fn highlight(&self, code: &str, language: &str) -> Vec<Vec<HighlightSpan>> {
        let theme = &self.theme_set.themes["InspiredGitHub"];

        // Find syntax definition
        let syntax = self.syntax_set
            .find_syntax_by_token(language)
            .or_else(|| self.syntax_set.find_syntax_by_extension(language))
            .or_else(|| {
                // Try common aliases
                let lang_lower = language.to_lowercase();
                match lang_lower.as_str() {
                    "python" | "py" | "python3" => self.syntax_set.find_syntax_by_extension("py"),
                    "javascript" | "js" => self.syntax_set.find_syntax_by_extension("js"),
                    "typescript" | "ts" => self.syntax_set.find_syntax_by_extension("ts"),
                    "c++" | "cpp" | "cxx" => self.syntax_set.find_syntax_by_extension("cpp"),
                    "c" => self.syntax_set.find_syntax_by_extension("c"),
                    "rust" | "rs" => self.syntax_set.find_syntax_by_extension("rs"),
                    "java" => self.syntax_set.find_syntax_by_extension("java"),
                    "go" | "golang" => self.syntax_set.find_syntax_by_extension("go"),
                    "ruby" | "rb" => self.syntax_set.find_syntax_by_extension("rb"),
                    "html" => self.syntax_set.find_syntax_by_extension("html"),
                    "css" => self.syntax_set.find_syntax_by_extension("css"),
                    "json" => self.syntax_set.find_syntax_by_extension("json"),
                    "xml" => self.syntax_set.find_syntax_by_extension("xml"),
                    "yaml" | "yml" => self.syntax_set.find_syntax_by_extension("yml"),
                    "sql" => self.syntax_set.find_syntax_by_extension("sql"),
                    "bash" | "sh" | "shell" => self.syntax_set.find_syntax_by_extension("sh"),
                    "latex" | "tex" => self.syntax_set.find_syntax_by_extension("tex"),
                    "r" => self.syntax_set.find_syntax_by_extension("r"),
                    "matlab" | "m" => self.syntax_set.find_syntax_by_extension("m"),
                    "haskell" | "hs" => self.syntax_set.find_syntax_by_extension("hs"),
                    "scala" => self.syntax_set.find_syntax_by_extension("scala"),
                    "swift" => self.syntax_set.find_syntax_by_extension("swift"),
                    "kotlin" | "kt" => self.syntax_set.find_syntax_by_extension("kt"),
                    "php" => self.syntax_set.find_syntax_by_extension("php"),
                    "perl" | "pl" => self.syntax_set.find_syntax_by_extension("pl"),
                    "lua" => self.syntax_set.find_syntax_by_extension("lua"),
                    "markdown" | "md" => self.syntax_set.find_syntax_by_extension("md"),
                    _ => None,
                }
            })
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut result = Vec::new();

        for line in code.lines() {
            let ranges = highlighter.highlight_line(line, &self.syntax_set);
            match ranges {
                Ok(ranges) => {
                    let spans: Vec<HighlightSpan> = ranges.iter().map(|(style, text)| {
                        HighlightSpan {
                            text: text.to_string(),
                            color: syntect_to_color(style),
                            bold: style.font_style.contains(SynFontStyle::BOLD),
                            italic: style.font_style.contains(SynFontStyle::ITALIC),
                        }
                    }).collect();
                    result.push(spans);
                }
                Err(_) => {
                    // Fallback: plain text
                    result.push(vec![HighlightSpan {
                        text: line.to_string(),
                        color: Color::DARK_GRAY,
                        bold: false,
                        italic: false,
                    }]);
                }
            }
        }

        result
    }

    /// Highlight code returning flat spans (no line structure)
    /// More efficient for rendering where we handle newlines ourselves
    pub fn highlight_flat(&self, code: &str, language: &str) -> Vec<HighlightSpan> {
        let lines = self.highlight(code, language);
        let mut flat = Vec::with_capacity(code.len() / 10);
        for line_spans in lines {
            flat.extend(line_spans);
        }
        flat
    }
}

fn syntect_to_color(style: &Style) -> Color {
    Color::from_rgb_u8(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    )
}

// Lazy-initialized global highlighter
use std::sync::OnceLock;
static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();

pub fn get_highlighter() -> &'static Highlighter {
    HIGHLIGHTER.get_or_init(Highlighter::new)
}
