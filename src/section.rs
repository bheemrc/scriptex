//! Academic paper section detection from LaTeX AST.
//!
//! Segments a parsed Document into named academic sections (Abstract,
//! Introduction, Methods, Results, etc.) by classifying section headings
//! against known patterns. Works directly with the parsed AST for
//! exact accuracy — no regex heuristics on raw text.

use crate::document::*;
use std::fmt;

/// Kind of academic paper section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Title,
    Abstract,
    Introduction,
    Background,
    RelatedWork,
    Methods,
    Results,
    Discussion,
    Conclusion,
    Acknowledgments,
    References,
    Appendix,
    Other(u32),
}

impl SectionKind {
    /// Whether this is a "key" section typically included in summaries.
    pub fn is_key_section(&self) -> bool {
        matches!(
            self,
            SectionKind::Abstract
                | SectionKind::Introduction
                | SectionKind::Methods
                | SectionKind::Results
                | SectionKind::Discussion
                | SectionKind::Conclusion
        )
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            SectionKind::Title => "Title",
            SectionKind::Abstract => "Abstract",
            SectionKind::Introduction => "Introduction",
            SectionKind::Background => "Background",
            SectionKind::RelatedWork => "Related Work",
            SectionKind::Methods => "Methods",
            SectionKind::Results => "Results",
            SectionKind::Discussion => "Discussion",
            SectionKind::Conclusion => "Conclusion",
            SectionKind::Acknowledgments => "Acknowledgments",
            SectionKind::References => "References",
            SectionKind::Appendix => "Appendix",
            SectionKind::Other(_) => "Other",
        }
    }
}

impl fmt::Display for SectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A detected academic paper section.
#[derive(Debug, Clone)]
pub struct PaperSection {
    pub kind: SectionKind,
    pub heading: String,
    pub number: Option<String>,
    pub level: Option<SectionLevel>,
    pub full_text: String,
}

/// Detect academic sections from a parsed Document AST.
///
/// Walks body nodes to identify section boundaries, classifies each heading
/// against ~40 known academic patterns, and extracts full text content.
/// Returns sections in document order.
pub fn detect_sections(doc: &Document, source: &str) -> Vec<PaperSection> {
    let mut sections = Vec::new();
    let mut other_counter = 0u32;
    let mut current: Option<SectionBuilder> = None;

    // Title from preamble
    if let Some(ref title) = doc.preamble.title {
        sections.push(PaperSection {
            kind: SectionKind::Title,
            heading: title.clone(),
            number: None,
            level: None,
            full_text: title.clone(),
        });
    }

    // Walk top-level body nodes
    walk_for_sections(
        &doc.body,
        source,
        &mut sections,
        &mut current,
        &mut other_counter,
    );

    // Flush final section
    if let Some(builder) = current {
        sections.push(builder.finish());
    }

    sections
}

// ============================================================
// Internal section builder
// ============================================================

struct SectionBuilder {
    kind: SectionKind,
    heading: String,
    number: Option<String>,
    level: Option<SectionLevel>,
    text: String,
}

impl SectionBuilder {
    fn finish(self) -> PaperSection {
        PaperSection {
            kind: self.kind,
            heading: self.heading,
            number: self.number,
            level: self.level,
            full_text: self.text,
        }
    }
}

fn walk_for_sections(
    nodes: &[Node],
    source: &str,
    sections: &mut Vec<PaperSection>,
    current: &mut Option<SectionBuilder>,
    other_counter: &mut u32,
) {
    for node in nodes {
        match node {
            // Explicit Abstract node → creates its own section
            Node::Abstract(content) => {
                if let Some(builder) = current.take() {
                    sections.push(builder.finish());
                }
                let mut text = String::new();
                extract_text(content, source, &mut text);
                sections.push(PaperSection {
                    kind: SectionKind::Abstract,
                    heading: "Abstract".to_string(),
                    number: None,
                    level: None,
                    full_text: text,
                });
            }

            // Section heading → start a new section
            Node::Section { level, title, .. } => {
                if let Some(builder) = current.take() {
                    sections.push(builder.finish());
                }
                let heading_text = extract_text_string(title, source);
                let (clean, num) = strip_section_number(&heading_text);
                let kind = classify_heading(&clean, other_counter);
                *current = Some(SectionBuilder {
                    kind,
                    heading: heading_text,
                    number: num,
                    level: Some(*level),
                    text: String::new(),
                });
            }

            // Appendix marker
            Node::Appendix => {
                if let Some(builder) = current.take() {
                    sections.push(builder.finish());
                }
                *current = Some(SectionBuilder {
                    kind: SectionKind::Appendix,
                    heading: "Appendix".to_string(),
                    number: None,
                    level: None,
                    text: String::new(),
                });
            }

            // Bibliography environment → References section
            Node::Environment(env) if env.name == "thebibliography" => {
                if let Some(builder) = current.take() {
                    sections.push(builder.finish());
                }
                let mut text = String::new();
                extract_text(&env.content, source, &mut text);
                sections.push(PaperSection {
                    kind: SectionKind::References,
                    heading: "References".to_string(),
                    number: None,
                    level: None,
                    full_text: text,
                });
            }

            // Recurse into two-column wrapper (sections can appear inside)
            Node::TwoColumn(c) => {
                walk_for_sections(c, source, sections, current, other_counter);
            }

            // All other nodes → accumulate text into current section
            _ => {
                if let Some(ref mut builder) = current {
                    extract_text(std::slice::from_ref(node), source, &mut builder.text);
                }
            }
        }
    }
}

// ============================================================
// Heading classification
// ============================================================

/// Section heading patterns, priority-ordered.
/// For combined headings like "Results and Discussion", the first matching
/// pattern wins — patterns are ordered so "result" matches before "discussion".
const HEADING_PATTERNS: &[(&[&str], SectionKind)] = &[
    (&["abstract"], SectionKind::Abstract),
    (
        &["introduction", "overview", "motivation"],
        SectionKind::Introduction,
    ),
    (
        &[
            "related work",
            "prior work",
            "literature review",
            "previous work",
        ],
        SectionKind::RelatedWork,
    ),
    (
        &["background", "preliminar", "prerequisit", "notation"],
        SectionKind::Background,
    ),
    (
        &[
            "method",
            "approach",
            "model",
            "framework",
            "algorithm",
            "implementation",
            "system design",
            "technical",
            "proposed",
            "our approach",
            "formulation",
            "setup",
            "design",
        ],
        SectionKind::Methods,
    ),
    (
        &[
            "result",
            "experiment",
            "evaluation",
            "empirical",
            "finding",
            "performance",
            "analysis",
        ],
        SectionKind::Results,
    ),
    (
        &[
            "discussion",
            "limitation",
            "future work",
            "broader impact",
            "societal impact",
            "ethical",
        ],
        SectionKind::Discussion,
    ),
    (
        &[
            "conclusion",
            "concluding",
            "final remark",
            "closing",
            "summary",
        ],
        SectionKind::Conclusion,
    ),
    (
        &[
            "acknowledgment",
            "acknowledgement",
            "funding",
            "grant",
        ],
        SectionKind::Acknowledgments,
    ),
    (
        &["reference", "bibliography", "works cited"],
        SectionKind::References,
    ),
    (
        &["appendix", "supplementary", "supplemental"],
        SectionKind::Appendix,
    ),
];

/// Classify a section heading into a SectionKind.
/// Uses starts_with matching against known patterns, priority-ordered.
pub fn classify_heading(heading: &str, other_counter: &mut u32) -> SectionKind {
    let lower = heading.trim().to_lowercase();

    for &(patterns, kind) in HEADING_PATTERNS {
        for pattern in patterns {
            if lower.starts_with(pattern) {
                return kind;
            }
        }
    }

    *other_counter += 1;
    SectionKind::Other(*other_counter - 1)
}

/// Strip section number prefix from heading text.
/// Handles: "1. Introduction", "1.2 Methods", "A. Appendix"
fn strip_section_number(heading: &str) -> (String, Option<String>) {
    let trimmed = heading.trim();
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    if len == 0 {
        return (String::new(), None);
    }

    // Try digit-based number: 1, 1., 1.2, 1.2.3
    if bytes[0].is_ascii_digit() {
        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
    }
    // Try single uppercase letter: A, A., B
    else if bytes[0].is_ascii_uppercase()
        && (len == 1 || bytes.get(1) == Some(&b'.') || bytes.get(1) == Some(&b' '))
    {
        i = 1;
        if i < len && bytes[i] == b'.' {
            i += 1;
        }
    }

    if i > 0 {
        let number = trimmed[..i].trim_end_matches('.').to_string();
        // Skip whitespace after number
        while i < len && bytes[i] == b' ' {
            i += 1;
        }
        let rest = trimmed[i..].to_string();
        (rest, if number.is_empty() { None } else { Some(number) })
    } else {
        (trimmed.to_string(), None)
    }
}

// ============================================================
// Text extraction (pub(crate) for use by citation.rs / corpus.rs)
// ============================================================

/// Extract plain text from nodes into a buffer.
pub(crate) fn extract_text(nodes: &[Node], source: &str, out: &mut String) {
    for node in nodes {
        extract_text_node(node, source, out);
    }
}

/// Extract plain text from nodes, returning a new String.
pub(crate) fn extract_text_string(nodes: &[Node], source: &str) -> String {
    let mut out = String::new();
    extract_text(nodes, source, &mut out);
    out
}

fn extract_text_node(node: &Node, source: &str, out: &mut String) {
    match node {
        Node::Text(s) => out.push_str(s),
        Node::TextRef(off, len) => {
            let s = *off as usize;
            let e = s + *len as usize;
            if e <= source.len() {
                out.push_str(&source[s..e]);
            }
        }
        Node::TextParagraph(off, len) => {
            let s = *off as usize;
            let e = s + *len as usize;
            if e <= source.len() {
                out.push_str(&source[s..e]);
            }
            out.push('\n');
        }
        Node::Paragraph(c)
        | Node::Bold(c)
        | Node::Italic(c)
        | Node::Monospace(c)
        | Node::SmallCaps(c)
        | Node::Underline(c)
        | Node::Strikethrough(c)
        | Node::Superscript(c)
        | Node::Subscript(c)
        | Node::Emph(c)
        | Node::Quote(c)
        | Node::Quotation(c)
        | Node::Abstract(c)
        | Node::Center(c)
        | Node::FlushLeft(c)
        | Node::FlushRight(c)
        | Node::Group(c)
        | Node::Footnote(c)
        | Node::Proof { content: c, .. }
        | Node::TwoColumn(c) => extract_text(c, source, out),
        Node::Section { title, .. } => {
            extract_text(title, source, out);
            out.push('\n');
        }
        Node::Colored { content, .. }
        | Node::FontSize { content, .. }
        | Node::Minipage { content, .. }
        | Node::WrapFigure { content, .. }
        | Node::SubFigure { content, .. } => extract_text(content, source, out),
        Node::ColorBox(boxdata) => extract_text(&boxdata.content, source, out),
        Node::Environment(env) => extract_text(&env.content, source, out),
        Node::Figure(fig) => {
            if let Some(cap) = &fig.caption {
                extract_text(cap, source, out);
            }
        }
        Node::Table(table) => {
            for row in &table.rows {
                for (i, cell) in row.cells.iter().enumerate() {
                    if i > 0 {
                        out.push('\t');
                    }
                    extract_text(&cell.content, source, out);
                }
                out.push('\n');
            }
        }
        Node::Theorem(thm) => extract_text(&thm.body, source, out),
        Node::ItemizeList(items)
        | Node::EnumerateList(items)
        | Node::DescriptionList(items) => {
            for item in items {
                extract_text(&item.content, source, out);
                out.push('\n');
            }
        }
        Node::Href { content, .. } => extract_text(content, source, out),
        Node::Citation(key, _, _) => {
            out.push('[');
            out.push_str(key);
            out.push(']');
        }
        Node::InlineMath(_) | Node::DisplayMath(_) => out.push_str("[math]"),
        Node::LineBreak => out.push('\n'),
        Node::NonBreakingSpace | Node::HSpace(_) => out.push(' '),
        Node::EnDash => out.push_str("--"),
        Node::EmDash => out.push_str("---"),
        Node::Ellipsis => out.push_str("..."),
        Node::Verbatim(s) | Node::Code(s) => out.push_str(s),
        _ => {}
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_heading() {
        let mut counter = 0u32;
        assert_eq!(
            classify_heading("Introduction", &mut counter),
            SectionKind::Introduction
        );
        assert_eq!(
            classify_heading("Methods and Materials", &mut counter),
            SectionKind::Methods
        );
        assert_eq!(
            classify_heading("Related Work", &mut counter),
            SectionKind::RelatedWork
        );
        assert_eq!(
            classify_heading("Experimental Results", &mut counter),
            SectionKind::Results
        );
        assert_eq!(
            classify_heading("Discussion", &mut counter),
            SectionKind::Discussion
        );
        assert_eq!(
            classify_heading("Conclusion and Future Work", &mut counter),
            SectionKind::Conclusion
        );
        assert_eq!(
            classify_heading("Acknowledgments", &mut counter),
            SectionKind::Acknowledgments
        );
        assert_eq!(
            classify_heading("Dataset Details", &mut counter),
            SectionKind::Other(0)
        );
        assert_eq!(
            classify_heading("Hyperparameter Tuning", &mut counter),
            SectionKind::Other(1)
        );
    }

    #[test]
    fn test_strip_section_number() {
        assert_eq!(
            strip_section_number("1. Introduction"),
            ("Introduction".to_string(), Some("1".to_string()))
        );
        assert_eq!(
            strip_section_number("1.2 Methods"),
            ("Methods".to_string(), Some("1.2".to_string()))
        );
        assert_eq!(
            strip_section_number("A. Appendix"),
            ("Appendix".to_string(), Some("A".to_string()))
        );
        assert_eq!(
            strip_section_number("Introduction"),
            ("Introduction".to_string(), None)
        );
        assert_eq!(
            strip_section_number("3.1.2 Subsection"),
            ("Subsection".to_string(), Some("3.1.2".to_string()))
        );
    }

    #[test]
    fn test_detect_sections_basic() {
        let source = r#"\documentclass{article}
\title{Test Paper}
\begin{document}
\maketitle
\begin{abstract}
This is the abstract.
\end{abstract}
\section{Introduction}
Intro text here.
\section{Methods}
Methods description.
\section{Results}
Some results.
\section{Conclusion}
Final thoughts.
\end{document}"#;

        let tokens = crate::lexer::tokenize_parallel(source);
        let mut parser = crate::parser::Parser::new(tokens, source);
        let doc = parser.parse().unwrap();
        let sections = detect_sections(&doc, source);

        let kinds: Vec<SectionKind> = sections.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&SectionKind::Title));
        assert!(kinds.contains(&SectionKind::Abstract));
        assert!(kinds.contains(&SectionKind::Introduction));
        assert!(kinds.contains(&SectionKind::Methods));
        assert!(kinds.contains(&SectionKind::Results));
        assert!(kinds.contains(&SectionKind::Conclusion));
    }

    #[test]
    fn test_section_kind_display() {
        assert_eq!(format!("{}", SectionKind::Methods), "Methods");
        assert_eq!(format!("{}", SectionKind::RelatedWork), "Related Work");
        assert_eq!(format!("{}", SectionKind::Other(3)), "Other");
    }
}
