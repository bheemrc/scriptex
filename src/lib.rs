//! SonicSpeedLaTeX — blazing fast LaTeX to PDF compiler
//!
//! Library entry point for both native CLI and WASM targets.
//! Supports single-file compilation via `compile_latex_core()` and
//! multi-file compilation via `compile_latex_project()`.

pub mod lexer;
pub mod parser;
pub mod document;
pub mod layout;
pub mod typeset;
pub mod pdf;
pub mod color;
pub mod error;
pub mod font;
pub mod highlight;
pub mod math_layout;
pub mod macro_expand;
pub mod hyphenate;
#[allow(dead_code)]
pub mod tikz;
pub mod tikz_render;
pub mod pgfplots;
pub mod diagrams;
pub mod bibliography;
pub mod svg_render;
#[allow(dead_code)]
pub mod xref;
#[cfg(feature = "cli")]
#[allow(dead_code)]
pub mod image_embed;
#[cfg(feature = "cli")]
#[allow(dead_code)]
pub mod font_embed;

pub mod structure;

pub mod section;
pub mod citation;
pub mod corpus;
pub mod analysis_json;

#[cfg(feature = "wasm")]
pub mod wasm_api;

use std::collections::HashMap;
use anyhow::Result;
use crate::parser::Parser;
use crate::document::{Node, EnvironmentData};
use crate::bibliography::Bibliography;

/// Auxiliary files for multi-file compilation.
/// Keys are filenames (e.g. "chapter1.tex", "refs.bib", "figure.png").
/// Values are the raw file contents as bytes.
#[derive(Default)]
pub struct ProjectFiles {
    /// TeX source files (.tex, .sty, .cls) — stored as strings
    pub tex_files: HashMap<String, String>,
    /// Binary files (images: .png, .jpg, .pdf) — stored as raw bytes
    pub binary_files: HashMap<String, Vec<u8>>,
    /// Bibliography files (.bib) — stored as strings
    pub bib_files: HashMap<String, String>,
}

impl ProjectFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a text file (tex, sty, cls, bib)
    pub fn add_text_file(&mut self, name: String, content: String) {
        let lower = name.to_lowercase();
        if lower.ends_with(".bib") {
            self.bib_files.insert(name, content);
        } else {
            self.tex_files.insert(name, content);
        }
    }

    /// Register a binary file (png, jpg, jpeg, pdf)
    pub fn add_binary_file(&mut self, name: String, data: Vec<u8>) {
        self.binary_files.insert(name, data);
    }
}

/// Core compilation pipeline: LaTeX source → PDF bytes (in-memory).
/// Single-file mode — no \input/\include resolution.
pub fn compile_latex_core(source: &str) -> Result<Vec<u8>> {
    compile_latex_project(source, &ProjectFiles::new())
}

/// Multi-file compilation pipeline: resolves \input/\include from project files,
/// processes style files, loads images, and compiles to PDF.
pub fn compile_latex_project(source: &str, project: &ProjectFiles) -> Result<Vec<u8>> {
    // Step 1: Resolve \input{} and \include{} commands using project files
    let resolved;
    let source_after_includes: &str = if source.contains("\\input") || source.contains("\\include{") {
        resolved = resolve_inputs_from_project(source, project, 0);
        &resolved
    } else {
        source
    };

    // Step 2: Extract and prepend style file definitions
    let with_styles;
    let source_with_styles: &str = if !project.tex_files.is_empty() {
        let style_defs = extract_style_definitions(source_after_includes, project);
        if !style_defs.is_empty() {
            with_styles = inject_style_definitions(source_after_includes, &style_defs);
            &with_styles
        } else {
            source_after_includes
        }
    } else {
        source_after_includes
    };

    // Step 3: Macro expansion
    let expanded;
    let effective_source: &str = if macro_expand::MacroEngine::has_macros(source_with_styles) {
        expanded = macro_expand::expand(source_with_styles);
        &expanded
    } else {
        source_with_styles
    };

    // Step 4: Lex
    let tokens = lexer::tokenize_parallel(effective_source);

    // Step 5: Parse
    let mut parser = Parser::new(tokens, effective_source);
    let mut doc = parser.parse()?;

    // Step 5b: Bibliography resolution — load .bib files, resolve citations,
    // generate reference section nodes
    let author_year_map = if !project.bib_files.is_empty() || has_bibliography_command(effective_source) {
        resolve_bibliography(&mut doc, effective_source, project)
    } else {
        HashMap::new()
    };

    // Step 6: Layout (with image data and author-year map)
    let layout_result = layout::layout_document_inner(
        &doc,
        effective_source,
        project.binary_files.clone(),
        author_year_map,
        String::new(),
    )?;

    // Step 7: Generate PDF to in-memory buffer
    let mut buf = Vec::with_capacity(1024 * 1024);
    pdf::write_pdf_to_writer(&mut buf, &layout_result, &doc, effective_source)?;

    Ok(buf)
}

/// Resolve \input{file} and \include{file} by looking up files in ProjectFiles.
fn resolve_inputs_from_project(source: &str, project: &ProjectFiles, depth: u32) -> String {
    if depth > 10 { return source.to_string(); }

    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 6 < bytes.len() {
            let rest = &source[i..];
            let (cmd, cmd_len) = if rest.starts_with("\\input") && !rest.get(6..7).map_or(false, |s| s.starts_with(|c: char| c.is_ascii_alphabetic())) {
                ("\\input", 6)
            } else if rest.starts_with("\\include{") {
                ("\\include", 8)
            } else {
                result.push(bytes[i] as char);
                i += 1;
                continue;
            };

            let mut j = cmd_len;
            while j < rest.len() && rest.as_bytes()[j] == b' ' { j += 1; }

            if j < rest.len() && rest.as_bytes()[j] == b'{' {
                j += 1;
                let start = j;
                while j < rest.len() && rest.as_bytes()[j] != b'}' { j += 1; }
                if j < rest.len() {
                    let filename = rest[start..j].trim();
                    j += 1;

                    // Try multiple filename variants
                    let candidates = [
                        filename.to_string(),
                        format!("{}.tex", filename),
                    ];

                    let mut found = false;
                    for candidate in &candidates {
                        if let Some(content) = project.tex_files.get(candidate.as_str()) {
                            if cmd == "\\include" {
                                result.push_str("\\clearpage\n");
                            }
                            let resolved = resolve_inputs_from_project(content, project, depth + 1);
                            result.push_str(&resolved);
                            if cmd == "\\include" {
                                result.push_str("\n\\clearpage\n");
                            }
                            found = true;
                            break;
                        }
                    }

                    if !found {
                        // Keep the original command if file not found
                        result.push_str(&rest[..j]);
                    }

                    i += j;
                    continue;
                }
            }
        }

        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Extract macro/command definitions from style files referenced by \usepackage
fn extract_style_definitions(source: &str, project: &ProjectFiles) -> String {
    let mut defs = String::new();

    // Find all \usepackage commands
    let mut i = 0;
    let bytes = source.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let rest = &source[i..];
            if rest.starts_with("\\usepackage") {
                let after_cmd = &rest[11..];
                // Skip optional [options]
                let mut j = 0;
                let abytes = after_cmd.as_bytes();
                while j < abytes.len() && abytes[j] == b' ' { j += 1; }
                if j < abytes.len() && abytes[j] == b'[' {
                    let mut depth = 1;
                    j += 1;
                    while j < abytes.len() && depth > 0 {
                        if abytes[j] == b'[' { depth += 1; }
                        if abytes[j] == b']' { depth -= 1; }
                        j += 1;
                    }
                }
                while j < abytes.len() && abytes[j] == b' ' { j += 1; }
                if j < abytes.len() && abytes[j] == b'{' {
                    j += 1;
                    let start = j;
                    while j < abytes.len() && abytes[j] != b'}' { j += 1; }
                    if j < abytes.len() {
                        let packages = &after_cmd[start..j];
                        // Handle comma-separated packages
                        for pkg in packages.split(',') {
                            let pkg = pkg.trim();
                            let sty_name = format!("{}.sty", pkg);
                            if let Some(sty_content) = project.tex_files.get(&sty_name) {
                                // Extract only definition commands from style files
                                extract_defs_from_sty(sty_content, &mut defs);
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    defs
}

/// Extract \newcommand, \def, \newenvironment, \DeclareMathOperator from a .sty file.
/// Conservative: skips internal LaTeX commands (with @), overly complex definitions,
/// and commands that could break the compilation pipeline.
fn extract_defs_from_sty(sty_content: &str, defs: &mut String) {
    let bytes = sty_content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip comments
        if bytes[i] == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            i += 1;
            continue;
        }

        if bytes[i] == b'\\' {
            let rest = &sty_content[i..];
            // Only extract safe, user-visible definitions
            let is_def_cmd = rest.starts_with("\\newcommand")
                || rest.starts_with("\\providecommand")
                || rest.starts_with("\\DeclareMathOperator")
                || rest.starts_with("\\newtheorem")
                || rest.starts_with("\\definecolor")
                || rest.starts_with("\\setlength")
                || rest.starts_with("\\addtolength");

            if is_def_cmd {
                let end = find_command_end(sty_content, i);
                let extracted = &sty_content[i..end];

                // Safety checks: skip if command contains internal @ macros,
                // is too long (complex definitions), or redefines core commands
                let safe = extracted.len() < 500
                    && !extracted.contains('@')
                    && !extracted.contains("\\begin{document}")
                    && !extracted.contains("\\end{document}")
                    && !extracted.contains("\\AtBeginDocument")
                    && !extracted.contains("\\AtEndDocument");

                if safe {
                    defs.push_str(extracted);
                    defs.push('\n');
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

/// Find the end of a LaTeX command with braced arguments
fn find_command_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut i = start;
    // Skip command name
    i += 1; // skip '\'
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
    if i < bytes.len() && bytes[i] == b'*' { i += 1; }

    // Process optional and required arguments
    let mut brace_groups_seen = 0;
    while i < bytes.len() && brace_groups_seen < 5 {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => { i += 1; }
            b'[' => {
                // Optional argument
                let mut depth = 1;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'\\' => { i += 1; } // skip escaped chars
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'{' => {
                // Required argument
                let mut depth = 1;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        b'\\' => { i += 1; }
                        _ => {}
                    }
                    i += 1;
                }
                brace_groups_seen += 1;
            }
            _ => break,
        }
    }
    i
}

/// Inject style definitions into the source before \begin{document}
fn inject_style_definitions(source: &str, defs: &str) -> String {
    if let Some(pos) = source.find("\\begin{document}") {
        let mut result = String::with_capacity(source.len() + defs.len() + 2);
        result.push_str(&source[..pos]);
        result.push_str("% --- Style definitions from .sty files ---\n");
        result.push_str(defs);
        result.push_str("% --- End style definitions ---\n");
        result.push_str(&source[pos..]);
        result
    } else {
        // No \begin{document} found, prepend
        let mut result = String::with_capacity(source.len() + defs.len());
        result.push_str(defs);
        result.push_str(source);
        result
    }
}

// ============================================================
// Bibliography resolution
// ============================================================

/// Check if source contains \bibliography{} or \addbibresource{} commands
fn has_bibliography_command(source: &str) -> bool {
    source.contains("\\bibliography{") || source.contains("\\addbibresource{")
}

/// Extract bibliography filenames from source (\bibliography{main} → "main.bib")
fn extract_bib_filenames(source: &str) -> Vec<String> {
    let mut filenames = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let rest = &source[i..];
            let cmd_len = if rest.starts_with("\\bibliography{") {
                14 // len of "\bibliography{"
            } else if rest.starts_with("\\addbibresource{") {
                16
            } else {
                i += 1;
                continue;
            };

            // Read the braced argument
            let after_brace = &rest[cmd_len..];
            if let Some(close) = after_brace.find('}') {
                let arg = after_brace[..close].trim();
                // \bibliography can have comma-separated filenames
                for name in arg.split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        // Add .bib extension if not present
                        if name.ends_with(".bib") {
                            filenames.push(name.to_string());
                        } else {
                            filenames.push(format!("{}.bib", name));
                        }
                    }
                }
                i += cmd_len + close + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    filenames
}

/// Recursively collect all citation keys from the AST
fn collect_citation_keys(nodes: &[Node], keys: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Citation(key, _, _) => {
                // Handle comma-separated keys
                for k in key.split(',') {
                    let k = k.trim();
                    if !k.is_empty() && !keys.contains(&k.to_string()) {
                        keys.push(k.to_string());
                    }
                }
            }
            // Recurse into containers
            Node::Paragraph(c) | Node::Bold(c) | Node::Italic(c)
            | Node::Monospace(c) | Node::SansSerif(c) | Node::SmallCaps(c) | Node::Underline(c)
            | Node::Strikethrough(c) | Node::Superscript(c) | Node::Subscript(c)
            | Node::Emph(c) | Node::Quote(c) | Node::Quotation(c)
            | Node::Abstract(c) | Node::Center(c) | Node::FlushLeft(c)
            | Node::FlushRight(c) | Node::Group(c) | Node::MBox(c) | Node::Footnote(c)
            | Node::Proof { content: c, .. } | Node::TwoColumn(c) => {
                collect_citation_keys(c, keys);
            }
            Node::Section { title, .. } => {
                collect_citation_keys(title, keys);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. }
            | Node::Minipage { content, .. }
            | Node::WrapFigure { content, .. } | Node::SubFigure { content, .. } => {
                collect_citation_keys(content, keys);
            }
            Node::ColorBox(boxdata) => {
                collect_citation_keys(&boxdata.content, keys);
            }
            Node::Environment(env) => {
                collect_citation_keys(&env.content, keys);
            }
            Node::Figure(fig) => {
                collect_citation_keys(&fig.content, keys);
                if let Some(cap) = &fig.caption {
                    collect_citation_keys(cap, keys);
                }
            }
            Node::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_citation_keys(&cell.content, keys);
                    }
                }
                if let Some(cap) = &table.caption {
                    collect_citation_keys(cap, keys);
                }
            }
            Node::Theorem(thm) => {
                collect_citation_keys(&thm.body, keys);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) | Node::DescriptionList(items) => {
                for item in items {
                    collect_citation_keys(&item.content, keys);
                    if let Some(label) = &item.label {
                        collect_citation_keys(label, keys);
                    }
                }
            }
            Node::Href { content, .. } => {
                collect_citation_keys(content, keys);
            }
            _ => {}
        }
    }
}

/// Generate thebibliography environment nodes from a Bibliography
fn generate_bibliography_nodes(bib: &Bibliography) -> Node {
    let mut content: Vec<Node> = Vec::new();

    let entries = bib.entries_in_order();
    for entry in &entries {
        content.push(Node::BibItem(entry.key.clone()));
        let formatted = bib.format_entry(entry);
        content.push(Node::Text(formatted));
    }

    Node::Environment(Box::new(EnvironmentData {
        name: "thebibliography".to_string(),
        args: vec![],
        content,
    }))
}

/// Full bibliography resolution: load .bib files, collect citations,
/// assign numbers, and inject reference section into the document.
/// Returns author-year map for natbib-style citations.
fn resolve_bibliography(doc: &mut crate::document::Document, source: &str, project: &ProjectFiles) -> HashMap<String, (String, String)> {
    let mut bib = Bibliography::new();

    // 1. Load .bib files referenced by \bibliography{} or \addbibresource{}
    let referenced_files = extract_bib_filenames(source);
    let mut loaded_any = false;

    for filename in &referenced_files {
        // Try exact match first, then case-insensitive, then just the basename
        let content = project.bib_files.get(filename)
            .or_else(|| {
                // Try without path prefix (e.g. "refs/main.bib" → "main.bib")
                let basename = filename.rsplit('/').next().unwrap_or(filename);
                project.bib_files.get(basename)
            })
            .or_else(|| {
                // Try any bib file that ends with this name
                project.bib_files.iter()
                    .find(|(k, _)| k.ends_with(filename.as_str()))
                    .map(|(_, v)| v)
            });

        if let Some(bib_content) = content {
            if bib.parse_bib_content(bib_content).is_ok() {
                loaded_any = true;
            }
        }
    }

    // If no specific files referenced, load all available .bib files
    if !loaded_any && !project.bib_files.is_empty() {
        for (_name, content) in &project.bib_files {
            let _ = bib.parse_bib_content(content);
        }
    }

    if bib.entries.is_empty() {
        return HashMap::new();
    }

    // 2. Collect all citation keys from the AST in document order
    let mut citation_keys = Vec::new();
    collect_citation_keys(&doc.body, &mut citation_keys);

    // 3. Register citations and assign numbers
    for key in &citation_keys {
        bib.register_citation(key);
    }
    bib.assign_numbers();

    // 4. Get author-year map for natbib-style citations
    let ay_map = bib.author_year_map();

    // 5. Check if document already has a thebibliography environment
    let has_manual_bib = doc.body.iter().any(|node| {
        matches!(node, Node::Environment(env) if env.name == "thebibliography")
    });

    // 6. If no manual bibliography, generate one and append to body
    if !has_manual_bib && !bib.cite_order.is_empty() {
        let bib_node = generate_bibliography_nodes(&bib);
        doc.body.push(bib_node);
    }

    ay_map
}

// ============================================================
// Structure extraction (no layout or PDF generation)
// ============================================================

/// Extract compiler-verified document structure as JSON.
/// Runs steps 1-5b + prescans but skips layout and PDF generation.
pub fn compile_latex_structure(source: &str) -> Result<String> {
    compile_latex_project_structure(source, &ProjectFiles::new())
}

/// Extract structure from a multi-file project.
pub fn compile_latex_project_structure(source: &str, project: &ProjectFiles) -> Result<String> {
    // Steps 1-3: resolve inputs, styles, macros (same as compile_latex_project)
    let resolved;
    let source_after_includes: &str = if source.contains("\\input") || source.contains("\\include{") {
        resolved = resolve_inputs_from_project(source, project, 0);
        &resolved
    } else {
        source
    };

    let with_styles;
    let source_with_styles: &str = if !project.tex_files.is_empty() {
        let style_defs = extract_style_definitions(source_after_includes, project);
        if !style_defs.is_empty() {
            with_styles = inject_style_definitions(source_after_includes, &style_defs);
            &with_styles
        } else {
            source_after_includes
        }
    } else {
        source_after_includes
    };

    let expanded;
    let effective_source: &str = if macro_expand::MacroEngine::has_macros(source_with_styles) {
        expanded = macro_expand::expand(source_with_styles);
        &expanded
    } else {
        source_with_styles
    };

    // Step 4-5: Lex + Parse
    let tokens = lexer::tokenize_parallel(effective_source);
    let mut parser = Parser::new(tokens, effective_source);
    let mut doc = parser.parse()?;

    // Step 5b: Bibliography resolution
    let _author_year_map = if !project.bib_files.is_empty() || has_bibliography_command(effective_source) {
        resolve_bibliography(&mut doc, effective_source, project)
    } else {
        HashMap::new()
    };

    // Prescans (same as layout entry, but without running layout)
    let (label_map, citation_map, label_types) = layout::collect_labels(&doc.body, &doc);
    let toc_entries = layout::collect_toc_entries(&doc.body, effective_source);

    // Serialize to JSON
    Ok(structure::extract_structure_json(
        &doc, effective_source,
        &label_map, &label_types, &citation_map, &toc_entries,
    ))
}

// ============================================================
// Paper analysis (section detection + citation graph)
// ============================================================

/// Analyze a single LaTeX paper: detect academic sections and build
/// a citation graph with accurate section attribution.
pub fn analyze_paper(source: &str) -> Result<corpus::PaperAnalysis> {
    analyze_paper_project(source, &ProjectFiles::new())
}

/// Analyze a multi-file LaTeX paper.
pub fn analyze_paper_project(source: &str, project: &ProjectFiles) -> Result<corpus::PaperAnalysis> {
    let (doc, effective_source, bib) = preprocess_and_parse(source, project)?;
    Ok(corpus::analyze_document(&doc, &effective_source, &bib.entries, "", 0))
}

/// Analyze multiple papers and assemble into a corpus.
pub fn analyze_papers(sources: &[(&str, &str)]) -> Result<corpus::PaperCorpus> {
    let mut analyses = Vec::with_capacity(sources.len());
    for (i, (name, source)) in sources.iter().enumerate() {
        let (doc, effective_source, bib) = preprocess_and_parse(source, &ProjectFiles::new())?;
        analyses.push(corpus::analyze_document(&doc, &effective_source, &bib.entries, name, i));
    }
    Ok(corpus::PaperCorpus::from_analyses(analyses))
}

/// Shared preprocessing + parsing pipeline for analysis functions.
fn preprocess_and_parse(
    source: &str,
    project: &ProjectFiles,
) -> Result<(crate::document::Document, String, crate::bibliography::Bibliography)> {
    // Step 1: Resolve inputs
    let resolved;
    let s1: &str = if source.contains("\\input") || source.contains("\\include{") {
        resolved = resolve_inputs_from_project(source, project, 0);
        &resolved
    } else {
        source
    };

    // Step 2: Style definitions
    let with_styles;
    let s2: &str = if !project.tex_files.is_empty() {
        let style_defs = extract_style_definitions(s1, project);
        if !style_defs.is_empty() {
            with_styles = inject_style_definitions(s1, &style_defs);
            &with_styles
        } else {
            s1
        }
    } else {
        s1
    };

    // Step 3: Macro expansion
    let expanded;
    let effective_source: &str = if macro_expand::MacroEngine::has_macros(s2) {
        expanded = macro_expand::expand(s2);
        &expanded
    } else {
        s2
    };

    // Step 4-5: Lex + Parse
    let tokens = lexer::tokenize_parallel(effective_source);
    let mut parser = Parser::new(tokens, effective_source);
    let doc = parser.parse()?;

    // Load bibliography
    let mut bib = bibliography::Bibliography::new();
    if !project.bib_files.is_empty() || has_bibliography_command(effective_source) {
        let referenced_files = extract_bib_filenames(effective_source);
        for filename in &referenced_files {
            let content = project.bib_files.get(filename)
                .or_else(|| {
                    let basename = filename.rsplit('/').next().unwrap_or(filename);
                    project.bib_files.get(basename)
                });
            if let Some(bib_content) = content {
                let _ = bib.parse_bib_content(bib_content);
            }
        }
        if bib.entries.is_empty() {
            for (_, content) in &project.bib_files {
                let _ = bib.parse_bib_content(content);
            }
        }
    }

    Ok((doc, effective_source.to_string(), bib))
}
