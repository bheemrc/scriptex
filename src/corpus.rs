//! Paper corpus assembly and LLM-optimized comparison output.
//!
//! Assembles multiple analyzed papers into a `PaperCorpus` for
//! cross-paper analysis. Provides query methods and generates
//! structured markdown comparison tables.

use crate::bibliography::BibEntry;
use crate::citation::{CitationGraph, ReferenceEntry};
use crate::document::Document;
use crate::section::{self, PaperSection, SectionKind};

/// Level of detail for comparison output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailLevel {
    /// ~600 chars per paper: title, authors, abstract snippet
    AbstractsOnly,
    /// ~1200 chars per paper: key sections (intro, methods, results, conclusion)
    KeySections,
    /// Unlimited: all section text included
    Full,
}

/// Analysis result for a single paper.
#[derive(Debug)]
pub struct PaperAnalysis {
    /// Paper identifier (e.g., "[P1]")
    pub id: String,
    /// Source filename or URL
    pub source_name: String,
    /// Title from preamble
    pub title: Option<String>,
    /// Author from preamble
    pub author: Option<String>,
    /// Detected academic sections
    pub sections: Vec<PaperSection>,
    /// Citation graph with adjacency lists
    pub citation_graph: CitationGraph,
}

impl PaperAnalysis {
    /// Get the abstract section text, if detected.
    pub fn abstract_text(&self) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| s.kind == SectionKind::Abstract)
            .map(|s| s.full_text.as_str())
    }

    /// Get a section by kind.
    pub fn section(&self, kind: SectionKind) -> Option<&PaperSection> {
        self.sections.iter().find(|s| s.kind == kind)
    }
}

/// Analyze a parsed Document into a PaperAnalysis.
///
/// This is the core analysis function. It runs section detection and
/// builds the citation graph from the AST in a single pass.
pub fn analyze_document(
    doc: &Document,
    source: &str,
    bib_entries: &[BibEntry],
    name: &str,
    id: usize,
) -> PaperAnalysis {
    let sections = section::detect_sections(doc, source);
    let citation_graph = CitationGraph::build(doc, source, bib_entries);

    PaperAnalysis {
        id: format!("[P{}]", id + 1),
        source_name: name.to_string(),
        title: doc.preamble.title.clone(),
        author: doc.preamble.author.clone(),
        sections,
        citation_graph,
    }
}

/// A corpus of analyzed papers with cross-paper query support.
#[derive(Debug)]
pub struct PaperCorpus {
    pub papers: Vec<PaperAnalysis>,
}

impl PaperCorpus {
    /// Build a corpus from already-analyzed papers.
    pub fn from_analyses(papers: Vec<PaperAnalysis>) -> Self {
        PaperCorpus { papers }
    }

    /// Number of papers in the corpus.
    pub fn len(&self) -> usize {
        self.papers.len()
    }

    /// Whether the corpus is empty.
    pub fn is_empty(&self) -> bool {
        self.papers.is_empty()
    }

    // ============================================================
    // Query methods
    // ============================================================

    /// Get abstracts for all papers: (id, abstract_text).
    pub fn abstracts(&self) -> Vec<(&str, Option<&str>)> {
        self.papers
            .iter()
            .map(|p| (p.id.as_str(), p.abstract_text()))
            .collect()
    }

    /// Get a specific section kind across all papers.
    pub fn sections(&self, kind: SectionKind) -> Vec<(&str, Option<&PaperSection>)> {
        self.papers
            .iter()
            .map(|p| (p.id.as_str(), p.section(kind)))
            .collect()
    }

    /// Get all references across all papers: (paper_id, references).
    pub fn all_references(&self) -> Vec<(&str, &[ReferenceEntry])> {
        self.papers
            .iter()
            .map(|p| (p.id.as_str(), p.citation_graph.references.as_slice()))
            .collect()
    }

    /// Find papers that cite a reference matching the given key substring.
    pub fn papers_citing(&self, key_pattern: &str) -> Vec<&str> {
        self.papers
            .iter()
            .filter(|p| {
                p.citation_graph
                    .unique_keys()
                    .iter()
                    .any(|k| k.contains(key_pattern))
            })
            .map(|p| p.id.as_str())
            .collect()
    }

    /// Detect shared references between papers.
    ///
    /// Returns (paper_idx_a, paper_idx_b, shared_keys) for all pairs
    /// that share at least one reference key. Useful for building a
    /// paper similarity graph.
    pub fn shared_references(&self) -> Vec<(usize, usize, Vec<String>)> {
        let n = self.papers.len();
        let mut result = Vec::new();

        // Collect key sets per paper
        let key_sets: Vec<std::collections::HashSet<&str>> = self
            .papers
            .iter()
            .map(|p| p.citation_graph.unique_keys().into_iter().collect())
            .collect();

        // Compare all pairs — O(n^2 * k) where k = avg keys per paper
        for i in 0..n {
            for j in (i + 1)..n {
                let shared: Vec<String> = key_sets[i]
                    .intersection(&key_sets[j])
                    .map(|k| k.to_string())
                    .collect();
                if !shared.is_empty() {
                    result.push((i, j, shared));
                }
            }
        }

        result.sort_by(|a, b| b.2.len().cmp(&a.2.len()));
        result
    }

    // ============================================================
    // Comparison output
    // ============================================================

    /// Generate LLM-optimized markdown comparison of all papers.
    pub fn to_comparison_markdown(&self, detail: DetailLevel) -> String {
        let mut out = String::with_capacity(4096);

        // Header
        out.push_str("# Paper Corpus Comparison\n");
        out.push_str(&format!("{} papers analyzed\n\n", self.papers.len()));

        // Papers table
        out.push_str("## Papers\n");
        out.push_str("| ID | Title | Authors | Sections | Refs | Citations |\n");
        out.push_str("|---|---|---|---|---|---|\n");
        for p in &self.papers {
            let title = p.title.as_deref().unwrap_or("Untitled");
            let author = p.author.as_deref().unwrap_or("Unknown");
            let sec_count = p.sections.len();
            let ref_count = p.citation_graph.reference_count();
            let cite_count = p.citation_graph.inline_count();
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                p.id,
                truncate(title, 60),
                truncate(author, 40),
                sec_count,
                ref_count,
                cite_count
            ));
        }
        out.push('\n');

        // Abstracts
        out.push_str("## Abstracts\n");
        for p in &self.papers {
            if let Some(abstract_text) = p.abstract_text() {
                out.push_str(&format!("### {} Abstract\n", p.id));
                let max_chars = match detail {
                    DetailLevel::AbstractsOnly => 600,
                    DetailLevel::KeySections => 800,
                    DetailLevel::Full => usize::MAX,
                };
                out.push_str(&truncate(abstract_text.trim(), max_chars));
                out.push_str("\n\n");
            }
        }

        // Key sections (if detail >= KeySections)
        if detail != DetailLevel::AbstractsOnly {
            let key_kinds = [
                SectionKind::Introduction,
                SectionKind::Methods,
                SectionKind::Results,
                SectionKind::Conclusion,
            ];

            for kind in &key_kinds {
                let has_any = self.papers.iter().any(|p| p.section(*kind).is_some());
                if !has_any {
                    continue;
                }

                out.push_str(&format!("## {}\n", kind));
                for p in &self.papers {
                    if let Some(sec) = p.section(*kind) {
                        out.push_str(&format!("### {} {}\n", p.id, kind));
                        let max_chars = match detail {
                            DetailLevel::KeySections => 1200,
                            DetailLevel::Full => usize::MAX,
                            _ => 600,
                        };
                        out.push_str(&truncate(sec.full_text.trim(), max_chars));
                        out.push_str("\n\n");
                    }
                }
            }
        }

        // Citation summary
        out.push_str("## Citation Summary\n");
        for p in &self.papers {
            let graph = &p.citation_graph;
            let format_name = graph
                .dominant_style()
                .map(|s| format!("{:?}", s))
                .unwrap_or_else(|| "Unknown".to_string());

            out.push_str(&format!(
                "**{}**: {} references, {} inline citations, format: {}\n\n",
                p.id,
                graph.reference_count(),
                graph.inline_count(),
                format_name
            ));

            let counts = graph.section_citation_counts();
            if !counts.is_empty() {
                out.push_str("| Section | Citations |\n");
                out.push_str("|---|---|\n");
                for (kind, count) in &counts {
                    out.push_str(&format!("| {} | {} |\n", kind, count));
                }
                out.push('\n');
            }

            if !graph.unresolved().is_empty() {
                out.push_str(&format!(
                    "Unresolved: {}\n\n",
                    graph.unresolved().join(", ")
                ));
            }
        }

        // Cross-paper shared references
        let shared = self.shared_references();
        if !shared.is_empty() {
            out.push_str("## Shared References\n");
            for (i, j, keys) in &shared {
                out.push_str(&format!(
                    "- {} and {} share {} references: {}\n",
                    self.papers[*i].id,
                    self.papers[*j].id,
                    keys.len(),
                    keys.iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
                if keys.len() > 5 {
                    out.push_str(&format!("  ... and {} more\n", keys.len() - 5));
                }
            }
            out.push('\n');
        }

        out
    }
}

/// Truncate text to max_chars, adding "..." if truncated.
fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        let mut end = max_chars;
        // Don't break in the middle of a word
        while end > 0 && !text.as_bytes()[end].is_ascii_whitespace() {
            end -= 1;
        }
        if end == 0 {
            end = max_chars;
        }
        format!("{}...", &text[..end])
    }
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
        analyze_document(&doc, &effective, &[], name, id)
    }

    #[test]
    fn test_paper_analysis() {
        let source = r#"\documentclass{article}
\title{Test Paper}
\author{Alice}
\begin{document}
\maketitle
\begin{abstract}
This paper presents our findings.
\end{abstract}
\section{Introduction}
Intro text \cite{ref1}.
\section{Methods}
Method description \cite{ref1,ref2}.
\section{Results}
Results here \cite{ref2}.
\section{Conclusion}
We conclude.
\end{document}"#;

        let analysis = parse_and_analyze(source, "test.tex", 0);

        assert_eq!(analysis.id, "[P1]");
        assert_eq!(analysis.title.as_deref(), Some("Test Paper"));
        assert!(analysis.abstract_text().is_some());
        assert!(analysis.section(SectionKind::Introduction).is_some());
        assert!(analysis.section(SectionKind::Methods).is_some());
        assert_eq!(analysis.citation_graph.inline_count(), 4);
    }

    #[test]
    fn test_corpus_comparison() {
        let source1 = r#"\documentclass{article}
\title{Paper One}
\author{Alice}
\begin{document}
\begin{abstract}First paper abstract.\end{abstract}
\section{Introduction}
\cite{shared_ref}
\end{document}"#;

        let source2 = r#"\documentclass{article}
\title{Paper Two}
\author{Bob}
\begin{document}
\begin{abstract}Second paper abstract.\end{abstract}
\section{Introduction}
\cite{shared_ref} \cite{unique_ref}
\end{document}"#;

        let a1 = parse_and_analyze(source1, "paper1.tex", 0);
        let a2 = parse_and_analyze(source2, "paper2.tex", 1);
        let corpus = PaperCorpus::from_analyses(vec![a1, a2]);

        assert_eq!(corpus.len(), 2);

        // Both papers should have abstracts
        let abstracts = corpus.abstracts();
        assert_eq!(abstracts.len(), 2);
        assert!(abstracts[0].1.is_some());
        assert!(abstracts[1].1.is_some());

        // shared_ref should be found in both papers
        let shared = corpus.shared_references();
        assert!(!shared.is_empty());
        assert!(shared[0].2.contains(&"shared_ref".to_string()));

        // papers_citing should find both papers
        let citing = corpus.papers_citing("shared_ref");
        assert_eq!(citing.len(), 2);

        // Comparison markdown should be non-empty and contain key sections
        let md = corpus.to_comparison_markdown(DetailLevel::AbstractsOnly);
        assert!(md.contains("Paper Corpus Comparison"));
        assert!(md.contains("[P1]"));
        assert!(md.contains("[P2]"));
        assert!(md.contains("Citation Summary"));
    }

    #[test]
    fn test_detail_levels() {
        let source = r#"\documentclass{article}
\title{Test}
\begin{document}
\begin{abstract}Abstract text here.\end{abstract}
\section{Introduction}
Intro.
\section{Methods}
Methods.
\end{document}"#;

        let a = parse_and_analyze(source, "test.tex", 0);
        let corpus = PaperCorpus::from_analyses(vec![a]);

        let abstracts_only = corpus.to_comparison_markdown(DetailLevel::AbstractsOnly);
        let key_sections = corpus.to_comparison_markdown(DetailLevel::KeySections);
        let full = corpus.to_comparison_markdown(DetailLevel::Full);

        // KeySections and Full should have section content; AbstractsOnly should not
        assert!(key_sections.contains("## Introduction"));
        assert!(full.contains("## Introduction"));
        // AbstractsOnly should still have abstracts
        assert!(abstracts_only.contains("Abstract"));
    }
}
