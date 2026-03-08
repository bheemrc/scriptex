//! JSON serialization for paper analysis and corpus data.
//!
//! Uses manual JSON building (same `json_escape_into()` pattern as structure.rs)
//! to avoid pulling in serde — keeps WASM binary small.

use crate::citation::{CitationGraph, InlineCitation, ReferenceEntry};
use crate::corpus::{PaperAnalysis, PaperCorpus};
use crate::document::CitationStyle;
use crate::section::PaperSection;

/// Escape a string for JSON output (in-place into buffer).
fn json_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let hex = format!("\\u{:04x}", c as u32);
                out.push_str(&hex);
            }
            c => out.push(c),
        }
    }
}

/// Write a JSON string field: `"key":"escaped_value"`.
fn write_str_field(out: &mut String, key: &str, value: &str) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":\"");
    json_escape_into(value, out);
    out.push('"');
}

/// Write a JSON nullable string field: `"key":"value"` or `"key":null`.
fn write_opt_str_field(out: &mut String, key: &str, value: Option<&str>) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    match value {
        Some(v) => {
            out.push('"');
            json_escape_into(v, out);
            out.push('"');
        }
        None => out.push_str("null"),
    }
}

/// Write a JSON integer field: `"key":123`.
fn write_int_field(out: &mut String, key: &str, value: usize) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    let mut ibuf = itoa::Buffer::new();
    out.push_str(ibuf.format(value));
}

/// Write a JSON float field: `"key":3.5`.
fn write_float_field(out: &mut String, key: &str, value: f32) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    // Format with 1 decimal place, strip trailing zeros
    let formatted = format!("{:.1}", value);
    out.push_str(&formatted);
}

fn citation_style_str(style: CitationStyle) -> &'static str {
    match style {
        CitationStyle::Numeric => "Numeric",
        CitationStyle::Parenthetical => "Parenthetical",
        CitationStyle::Textual => "Textual",
        CitationStyle::AuthorOnly => "AuthorOnly",
        CitationStyle::YearOnly => "YearOnly",
        CitationStyle::AltNoParen => "AltNoParen",
    }
}

// ============================================================
// Section serialization
// ============================================================

fn write_section(out: &mut String, section: &PaperSection, include_text: bool) {
    out.push('{');
    write_str_field(out, "kind", section.kind.label());
    out.push(',');
    write_str_field(out, "heading", &section.heading);
    out.push(',');
    write_opt_str_field(out, "number", section.number.as_deref());
    out.push(',');
    write_int_field(out, "textLength", section.full_text.len());
    if include_text {
        out.push(',');
        write_str_field(out, "fullText", &section.full_text);
    }
    out.push('}');
}

// ============================================================
// Reference entry serialization
// ============================================================

fn write_reference(out: &mut String, entry: &ReferenceEntry) {
    out.push('{');
    write_str_field(out, "key", &entry.key);
    out.push(',');
    // authors as joined string
    let authors_str = entry.authors.join(", ");
    write_str_field(out, "authors", &authors_str);
    out.push(',');
    write_str_field(out, "title", &entry.title);
    out.push(',');
    write_str_field(out, "year", &entry.year);
    out.push(',');
    write_opt_str_field(out, "venue", entry.venue.as_deref());
    out.push(',');
    write_opt_str_field(out, "doi", entry.doi.as_deref());
    out.push('}');
}

// ============================================================
// Inline citation serialization
// ============================================================

fn write_inline_citation(out: &mut String, cit: &InlineCitation) {
    out.push('{');
    write_str_field(out, "key", &cit.key);
    out.push(',');
    write_str_field(out, "section", cit.section.label());
    out.push(',');
    write_str_field(out, "style", citation_style_str(cit.style));
    out.push(',');
    let mut ibuf = itoa::Buffer::new();
    out.push_str("\"order\":");
    out.push_str(ibuf.format(cit.order));
    out.push('}');
}

// ============================================================
// Citation graph serialization
// ============================================================

fn write_citation_graph(out: &mut String, graph: &CitationGraph) {
    out.push('{');

    // references array
    out.push_str("\"references\":[");
    for (i, entry) in graph.references.iter().enumerate() {
        if i > 0 { out.push(','); }
        write_reference(out, entry);
    }
    out.push(']');

    // inlineCitations array
    out.push_str(",\"inlineCitations\":[");
    for (i, cit) in graph.inline_citations().iter().enumerate() {
        if i > 0 { out.push(','); }
        write_inline_citation(out, cit);
    }
    out.push(']');

    // sectionCounts object
    out.push_str(",\"sectionCounts\":{");
    let counts = graph.section_citation_counts();
    for (i, (kind, count)) in counts.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        out.push_str(kind.label());
        out.push_str("\":");
        let mut ibuf = itoa::Buffer::new();
        out.push_str(ibuf.format(*count));
    }
    out.push('}');

    // importance array (top 50 to keep JSON reasonable)
    out.push_str(",\"importance\":[");
    let importance = graph.reference_importance();
    let limit = importance.len().min(50);
    for (i, (key, score)) in importance[..limit].iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('{');
        write_str_field(out, "key", key);
        out.push(',');
        write_float_field(out, "score", *score);
        out.push('}');
    }
    out.push(']');

    // clusters array
    out.push_str(",\"clusters\":[");
    let clusters = graph.co_citation_clusters();
    for (i, cluster) in clusters.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('[');
        for (j, key) in cluster.iter().enumerate() {
            if j > 0 { out.push(','); }
            out.push('"');
            json_escape_into(key, out);
            out.push('"');
        }
        out.push(']');
    }
    out.push(']');

    // unresolved keys
    out.push_str(",\"unresolved\":[");
    for (i, key) in graph.unresolved().iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(key, out);
        out.push('"');
    }
    out.push(']');

    // dominantStyle
    out.push_str(",\"dominantStyle\":");
    match graph.dominant_style() {
        Some(style) => {
            out.push('"');
            out.push_str(citation_style_str(style));
            out.push('"');
        }
        None => out.push_str("null"),
    }

    // summary counts
    out.push(',');
    write_int_field(out, "referenceCount", graph.reference_count());
    out.push(',');
    write_int_field(out, "inlineCount", graph.inline_count());

    out.push('}');
}

// ============================================================
// Public API
// ============================================================

/// Serialize a PaperAnalysis to JSON.
///
/// When `detailed` is true, section full text is included.
/// When false, only `textLength` is provided (saves bandwidth).
pub fn paper_analysis_to_json(analysis: &PaperAnalysis, detailed: bool) -> String {
    let mut out = String::with_capacity(4096);
    write_paper_analysis(&mut out, analysis, detailed);
    out
}

/// Serialize a PaperCorpus to JSON.
///
/// Includes per-paper analysis plus cross-paper shared references.
pub fn corpus_to_json(corpus: &PaperCorpus, detailed: bool) -> String {
    let mut out = String::with_capacity(8192);
    out.push('{');

    // papers array
    out.push_str("\"papers\":[");
    for (i, paper) in corpus.papers.iter().enumerate() {
        if i > 0 { out.push(','); }
        write_paper_analysis(&mut out, paper, detailed);
    }
    out.push(']');

    // sharedReferences array
    out.push_str(",\"sharedReferences\":[");
    let shared = corpus.shared_references();
    for (i, (paper_a, paper_b, keys)) in shared.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('{');

        let mut ibuf = itoa::Buffer::new();
        out.push_str("\"paperA\":");
        out.push_str(ibuf.format(*paper_a));

        out.push_str(",\"paperB\":");
        let mut ibuf2 = itoa::Buffer::new();
        out.push_str(ibuf2.format(*paper_b));

        out.push_str(",\"sharedKeys\":[");
        for (j, key) in keys.iter().enumerate() {
            if j > 0 { out.push(','); }
            out.push('"');
            json_escape_into(key, &mut out);
            out.push('"');
        }
        out.push(']');

        out.push(',');
        write_int_field(&mut out, "count", keys.len());

        out.push('}');
    }
    out.push(']');

    // paperCount for convenience
    out.push(',');
    write_int_field(&mut out, "paperCount", corpus.len());

    out.push('}');
    out
}

fn write_paper_analysis(out: &mut String, analysis: &PaperAnalysis, detailed: bool) {
    out.push('{');

    write_str_field(out, "id", &analysis.id);
    out.push(',');
    write_str_field(out, "source", &analysis.source_name);
    out.push(',');
    write_opt_str_field(out, "title", analysis.title.as_deref());
    out.push(',');
    write_opt_str_field(out, "author", analysis.author.as_deref());

    // sections array
    out.push_str(",\"sections\":[");
    for (i, section) in analysis.sections.iter().enumerate() {
        if i > 0 { out.push(','); }
        write_section(out, section, detailed);
    }
    out.push(']');

    // citationGraph object
    out.push_str(",\"citationGraph\":");
    write_citation_graph(out, &analysis.citation_graph);

    out.push('}');
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_analyze(source: &str, name: &str, id: usize) -> PaperAnalysis {
        let effective = if crate::macro_expand::MacroEngine::has_macros(source) {
            crate::macro_expand::expand(source)
        } else {
            source.to_string()
        };
        let tokens = crate::lexer::tokenize_parallel(&effective);
        let mut parser = crate::parser::Parser::new(tokens, &effective);
        let doc = parser.parse().unwrap();
        crate::corpus::analyze_document(&doc, &effective, &[], name, id)
    }

    #[test]
    fn test_single_paper_json() {
        let source = r#"\documentclass{article}
\title{Test Paper}
\author{Alice}
\begin{document}
\maketitle
\begin{abstract}
This paper presents findings.
\end{abstract}
\section{Introduction}
Intro text \cite{ref1}.
\section{Methods}
Method text \cite{ref1,ref2}.
\begin{thebibliography}{9}
\bibitem{ref1} Author1. Title1. 2020.
\bibitem{ref2} Author2. Title2. 2021.
\end{thebibliography}
\end{document}"#;

        let analysis = parse_and_analyze(source, "test.tex", 0);
        let json = paper_analysis_to_json(&analysis, false);

        // Verify it's valid JSON structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));

        // Verify key fields are present
        assert!(json.contains("\"id\":\"[P1]\""));
        assert!(json.contains("\"source\":\"test.tex\""));
        assert!(json.contains("\"title\":\"Test Paper\""));
        assert!(json.contains("\"author\":\"Alice\""));
        assert!(json.contains("\"sections\":["));
        assert!(json.contains("\"citationGraph\":{"));
        assert!(json.contains("\"references\":["));
        assert!(json.contains("\"inlineCitations\":["));
        assert!(json.contains("\"sectionCounts\":{"));
        assert!(json.contains("\"importance\":["));
        assert!(json.contains("\"clusters\":["));
        assert!(json.contains("\"referenceCount\":"));
        assert!(json.contains("\"inlineCount\":"));

        // fullText should NOT be present (detailed=false)
        assert!(!json.contains("\"fullText\":"));
        // textLength should be present
        assert!(json.contains("\"textLength\":"));
    }

    #[test]
    fn test_single_paper_json_detailed() {
        let source = r#"\documentclass{article}
\begin{document}
\section{Introduction}
Some introduction text here.
\end{document}"#;

        let analysis = parse_and_analyze(source, "test.tex", 0);
        let json = paper_analysis_to_json(&analysis, true);

        // fullText should be present (detailed=true)
        assert!(json.contains("\"fullText\":"));
    }

    #[test]
    fn test_corpus_json() {
        let source1 = r#"\documentclass{article}
\title{Paper One}
\begin{document}
\section{Introduction}
\cite{shared_ref} \cite{unique1}
\begin{thebibliography}{9}
\bibitem{shared_ref} Shared Reference
\bibitem{unique1} Unique 1
\end{thebibliography}
\end{document}"#;

        let source2 = r#"\documentclass{article}
\title{Paper Two}
\begin{document}
\section{Introduction}
\cite{shared_ref} \cite{unique2}
\begin{thebibliography}{9}
\bibitem{shared_ref} Shared Reference
\bibitem{unique2} Unique 2
\end{thebibliography}
\end{document}"#;

        let a1 = parse_and_analyze(source1, "paper1.tex", 0);
        let a2 = parse_and_analyze(source2, "paper2.tex", 1);
        let corpus = PaperCorpus::from_analyses(vec![a1, a2]);
        let json = corpus_to_json(&corpus, false);

        // Verify corpus structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"papers\":["));
        assert!(json.contains("\"sharedReferences\":["));
        assert!(json.contains("\"paperCount\":2"));
        assert!(json.contains("\"sharedKeys\":["));
        assert!(json.contains("shared_ref"));
    }

    #[test]
    fn test_json_escaping() {
        let source = r#"\documentclass{article}
\title{A "Quoted" Title with \backslash}
\begin{document}
\end{document}"#;

        let analysis = parse_and_analyze(source, "test.tex", 0);
        let json = paper_analysis_to_json(&analysis, false);

        // Should contain escaped quotes
        assert!(json.contains("\\\"Quoted\\\""));
    }

    #[test]
    fn test_null_fields() {
        let source = r#"\documentclass{article}
\begin{document}
Hello world.
\end{document}"#;

        let analysis = parse_and_analyze(source, "test.tex", 0);
        let json = paper_analysis_to_json(&analysis, false);

        // Title and author should be null
        assert!(json.contains("\"title\":null"));
        assert!(json.contains("\"author\":null"));
    }
}
