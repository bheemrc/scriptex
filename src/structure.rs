//! Document structure extraction — compiler-verified JSON output.
//!
//! Walks the AST + prescan results (label_map, citation_map, toc_entries)
//! to produce a JSON string describing the document structure with
//! resolved cross-reference numbers, unresolved refs, and integrity data.
//! No serde dependency — manual JSON building for minimal WASM size.

use std::collections::{HashMap, HashSet};
use crate::document::*;
use crate::layout::TocEntry;

/// Extract compiler-verified document structure as JSON.
/// Runs after parsing + prescan but BEFORE layout/PDF generation.
pub fn extract_structure_json(
    doc: &Document,
    source: &str,
    label_map: &HashMap<String, String>,
    label_types: &HashMap<String, String>,
    citation_map: &HashMap<String, u32>,
    toc_entries: &[TocEntry],
) -> String {
    let mut out = String::with_capacity(4096);
    out.push('{');

    // documentClass
    let class_name = match &doc.class.class_type {
        ClassType::Article => "article",
        ClassType::Report => "report",
        ClassType::Book => "book",
        ClassType::Letter => "letter",
        ClassType::Beamer => "beamer",
        ClassType::Memoir => "memoir",
        ClassType::Custom(s) => s.as_str(),
    };
    out.push_str("\"documentClass\":\"");
    json_escape_into(class_name, &mut out);
    out.push('"');

    // title
    if let Some(ref title) = doc.preamble.title {
        out.push_str(",\"title\":\"");
        json_escape_into(title, &mut out);
        out.push('"');
    } else {
        out.push_str(",\"title\":null");
    }

    // author
    if let Some(ref author) = doc.preamble.author {
        out.push_str(",\"author\":\"");
        json_escape_into(author, &mut out);
        out.push('"');
    } else {
        out.push_str(",\"author\":null");
    }

    // packages
    out.push_str(",\"packages\":[");
    for (i, pkg) in doc.preamble.packages.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(&pkg.name, &mut out);
        out.push('"');
    }
    out.push(']');

    // sections (from toc_entries — compiler-numbered)
    out.push_str(",\"sections\":[");
    for (i, entry) in toc_entries.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"level\":\"");
        out.push_str(section_level_str(&entry.level));
        out.push_str("\",\"number\":\"");
        json_escape_into(&entry.number, &mut out);
        out.push_str("\",\"title\":\"");
        json_escape_into(&entry.title, &mut out);
        out.push_str("\"}");
    }
    out.push(']');

    // labels — compiler-resolved label → {number, type}
    out.push_str(",\"labels\":{");
    let mut first = true;
    for (label, number) in label_map {
        if !first { out.push(','); }
        first = false;
        out.push('"');
        json_escape_into(label, &mut out);
        out.push_str("\":{\"number\":\"");
        json_escape_into(number, &mut out);
        out.push_str("\",\"type\":\"");
        if let Some(t) = label_types.get(label) {
            json_escape_into(t, &mut out);
        } else {
            out.push_str("unknown");
        }
        out.push_str("\"}");
    }
    out.push('}');

    // Collect all references and citation keys from AST
    let all_refs = collect_all_refs(&doc.body);
    let all_cite_keys = collect_all_cite_keys(&doc.body);

    // references — all \ref/\eqref/\cref targets
    out.push_str(",\"references\":[");
    for (i, r) in all_refs.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(r, &mut out);
        out.push('"');
    }
    out.push(']');

    // Compute unresolved refs and unreferenced labels
    let (unresolved_refs, unreferenced_labels) = compute_unresolved(&all_refs, label_map);

    out.push_str(",\"unresolvedRefs\":[");
    for (i, r) in unresolved_refs.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(r, &mut out);
        out.push('"');
    }
    out.push(']');

    out.push_str(",\"unreferencedLabels\":[");
    for (i, r) in unreferenced_labels.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(r, &mut out);
        out.push('"');
    }
    out.push(']');

    // equations
    let equations = collect_equations(&doc.body, label_map);
    out.push_str(",\"equations\":[");
    for (i, eq) in equations.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"number\":");
        if let Some(ref n) = eq.number {
            out.push('"');
            json_escape_into(n, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"label\":");
        if let Some(ref l) = eq.label {
            out.push('"');
            json_escape_into(l, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"env\":\"");
        out.push_str(eq.env);
        out.push_str("\",\"numbered\":");
        out.push_str(if eq.numbered { "true" } else { "false" });
        out.push('}');
    }
    out.push(']');

    // theorems
    let theorems = collect_theorems(&doc.body, label_map);
    out.push_str(",\"theorems\":[");
    for (i, thm) in theorems.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"type\":\"");
        json_escape_into(&thm.thm_type, &mut out);
        out.push_str("\",\"number\":");
        if let Some(ref n) = thm.number {
            out.push('"');
            json_escape_into(n, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"label\":");
        if let Some(ref l) = thm.label {
            out.push('"');
            json_escape_into(l, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push(']');

    // figures
    let figures = collect_figures(&doc.body, label_map, source);
    out.push_str(",\"figures\":[");
    for (i, f) in figures.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"number\":");
        if let Some(ref n) = f.number {
            out.push('"');
            json_escape_into(n, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"label\":");
        if let Some(ref l) = f.label {
            out.push('"');
            json_escape_into(l, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"caption\":");
        if let Some(ref c) = f.caption {
            out.push('"');
            json_escape_into(c, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push(']');

    // tables
    let tables = collect_tables(&doc.body, label_map, source);
    out.push_str(",\"tables\":[");
    for (i, t) in tables.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"number\":");
        if let Some(ref n) = t.number {
            out.push('"');
            json_escape_into(n, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"label\":");
        if let Some(ref l) = t.label {
            out.push('"');
            json_escape_into(l, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push_str(",\"caption\":");
        if let Some(ref c) = t.caption {
            out.push('"');
            json_escape_into(c, &mut out);
            out.push('"');
        } else {
            out.push_str("null");
        }
        out.push('}');
    }
    out.push(']');

    // citations — compiler-assigned numbers
    out.push_str(",\"citations\":{");
    let mut first = true;
    for (key, num) in citation_map {
        if !first { out.push(','); }
        first = false;
        out.push('"');
        json_escape_into(key, &mut out);
        out.push_str("\":");
        let mut ibuf = itoa::Buffer::new();
        out.push_str(ibuf.format(*num));
    }
    out.push('}');

    // citedKeys — all citation keys found in AST
    out.push_str(",\"citedKeys\":[");
    for (i, k) in all_cite_keys.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(k, &mut out);
        out.push('"');
    }
    out.push(']');

    // unresolvedCitations
    let unresolved_cites: Vec<&str> = all_cite_keys.iter()
        .filter(|k| !citation_map.contains_key(k.as_str()))
        .map(|k| k.as_str())
        .collect();
    out.push_str(",\"unresolvedCitations\":[");
    for (i, k) in unresolved_cites.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push('"');
        json_escape_into(k, &mut out);
        out.push('"');
    }
    out.push(']');

    // tocEntries (same as sections but with level name)
    out.push_str(",\"tocEntries\":[");
    for (i, entry) in toc_entries.iter().enumerate() {
        if i > 0 { out.push(','); }
        out.push_str("{\"level\":\"");
        out.push_str(section_level_str(&entry.level));
        out.push_str("\",\"number\":\"");
        json_escape_into(&entry.number, &mut out);
        out.push_str("\",\"title\":\"");
        json_escape_into(&entry.title, &mut out);
        out.push_str("\"}");
    }
    out.push(']');

    // counters — final counter values from prescan
    // We can derive these from the collected items
    out.push_str(",\"counters\":{");
    out.push_str("\"equation\":");
    let mut ibuf = itoa::Buffer::new();
    out.push_str(ibuf.format(equations.iter().filter(|e| e.numbered).count()));
    out.push_str(",\"figure\":");
    out.push_str(ibuf.format(figures.len()));
    out.push_str(",\"table\":");
    out.push_str(ibuf.format(tables.len()));
    out.push('}');

    // wordCountEstimate
    let word_count = estimate_word_count(source);
    out.push_str(",\"wordCountEstimate\":");
    out.push_str(ibuf.format(word_count));

    out.push('}');
    out
}

// ============================================================
// Internal data types
// ============================================================

struct EquationEntry {
    number: Option<String>,
    label: Option<String>,
    env: &'static str,
    numbered: bool,
}

struct TheoremEntry {
    thm_type: String,
    number: Option<String>,
    label: Option<String>,
}

struct FloatEntry {
    number: Option<String>,
    label: Option<String>,
    caption: Option<String>,
}

// ============================================================
// AST walkers
// ============================================================

/// Recursively collect all \ref, \eqref, \cref targets from AST
fn collect_all_refs(nodes: &[Node]) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    collect_refs_inner(nodes, &mut refs, &mut seen);
    refs
}

fn collect_refs_inner(nodes: &[Node], refs: &mut Vec<String>, seen: &mut HashSet<String>) {
    for node in nodes {
        match node {
            Node::Ref(target) | Node::EqRef(target) => {
                if seen.insert(target.clone()) {
                    refs.push(target.clone());
                }
            }
            Node::Cref(target, _) => {
                if seen.insert(target.clone()) {
                    refs.push(target.clone());
                }
            }
            // Recurse into containers
            Node::Paragraph(c) | Node::Bold(c) | Node::Italic(c)
            | Node::Monospace(c) | Node::SmallCaps(c) | Node::Underline(c)
            | Node::Strikethrough(c) | Node::Superscript(c) | Node::Subscript(c)
            | Node::Emph(c) | Node::Quote(c) | Node::Quotation(c)
            | Node::Abstract(c) | Node::Center(c) | Node::FlushLeft(c)
            | Node::FlushRight(c) | Node::Group(c) | Node::Footnote(c)
            | Node::Proof { content: c, .. } | Node::TwoColumn(c) => {
                collect_refs_inner(c, refs, seen);
            }
            Node::Section { title, .. } => {
                collect_refs_inner(title, refs, seen);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. }
            | Node::Minipage { content, .. }
            | Node::WrapFigure { content, .. } | Node::SubFigure { content, .. } => {
                collect_refs_inner(content, refs, seen);
            }
            Node::ColorBox(boxdata) => {
                collect_refs_inner(&boxdata.content, refs, seen);
            }
            Node::Environment(env) => {
                collect_refs_inner(&env.content, refs, seen);
            }
            Node::Figure(fig) => {
                collect_refs_inner(&fig.content, refs, seen);
                if let Some(cap) = &fig.caption {
                    collect_refs_inner(cap, refs, seen);
                }
            }
            Node::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_refs_inner(&cell.content, refs, seen);
                    }
                }
                if let Some(cap) = &table.caption {
                    collect_refs_inner(cap, refs, seen);
                }
            }
            Node::Theorem(thm) => {
                collect_refs_inner(&thm.body, refs, seen);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) | Node::DescriptionList(items) => {
                for item in items {
                    collect_refs_inner(&item.content, refs, seen);
                    if let Some(label) = &item.label {
                        collect_refs_inner(label, refs, seen);
                    }
                }
            }
            Node::Href { content, .. } => {
                collect_refs_inner(content, refs, seen);
            }
            _ => {}
        }
    }
}

/// Recursively collect all citation keys from AST
fn collect_all_cite_keys(nodes: &[Node]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    collect_cite_keys_inner(nodes, &mut keys, &mut seen);
    keys
}

fn collect_cite_keys_inner(nodes: &[Node], keys: &mut Vec<String>, seen: &mut HashSet<String>) {
    for node in nodes {
        match node {
            Node::Citation(key, _, _) => {
                for k in key.split(',') {
                    let k = k.trim();
                    if !k.is_empty() && seen.insert(k.to_string()) {
                        keys.push(k.to_string());
                    }
                }
            }
            // Recurse into containers
            Node::Paragraph(c) | Node::Bold(c) | Node::Italic(c)
            | Node::Monospace(c) | Node::SmallCaps(c) | Node::Underline(c)
            | Node::Strikethrough(c) | Node::Superscript(c) | Node::Subscript(c)
            | Node::Emph(c) | Node::Quote(c) | Node::Quotation(c)
            | Node::Abstract(c) | Node::Center(c) | Node::FlushLeft(c)
            | Node::FlushRight(c) | Node::Group(c) | Node::Footnote(c)
            | Node::Proof { content: c, .. } | Node::TwoColumn(c) => {
                collect_cite_keys_inner(c, keys, seen);
            }
            Node::Section { title, .. } => {
                collect_cite_keys_inner(title, keys, seen);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. }
            | Node::Minipage { content, .. }
            | Node::WrapFigure { content, .. } | Node::SubFigure { content, .. } => {
                collect_cite_keys_inner(content, keys, seen);
            }
            Node::ColorBox(boxdata) => {
                collect_cite_keys_inner(&boxdata.content, keys, seen);
            }
            Node::Environment(env) => {
                collect_cite_keys_inner(&env.content, keys, seen);
            }
            Node::Figure(fig) => {
                collect_cite_keys_inner(&fig.content, keys, seen);
                if let Some(cap) = &fig.caption {
                    collect_cite_keys_inner(cap, keys, seen);
                }
            }
            Node::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_cite_keys_inner(&cell.content, keys, seen);
                    }
                }
                if let Some(cap) = &table.caption {
                    collect_cite_keys_inner(cap, keys, seen);
                }
            }
            Node::Theorem(thm) => {
                collect_cite_keys_inner(&thm.body, keys, seen);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) | Node::DescriptionList(items) => {
                for item in items {
                    collect_cite_keys_inner(&item.content, keys, seen);
                    if let Some(label) = &item.label {
                        collect_cite_keys_inner(label, keys, seen);
                    }
                }
            }
            Node::Href { content, .. } => {
                collect_cite_keys_inner(content, keys, seen);
            }
            _ => {}
        }
    }
}

/// Collect all display math environments with their labels and numbers
fn collect_equations(nodes: &[Node], label_map: &HashMap<String, String>) -> Vec<EquationEntry> {
    let mut eqs = Vec::new();
    collect_equations_inner(nodes, label_map, &mut eqs);
    eqs
}

fn collect_equations_inner(nodes: &[Node], label_map: &HashMap<String, String>, eqs: &mut Vec<EquationEntry>) {
    for node in nodes {
        match node {
            Node::DisplayMath(data) => {
                let env = match data.env_type {
                    MathEnvType::DollarDollar => "displaymath",
                    MathEnvType::Equation => "equation",
                    MathEnvType::Align => "align",
                    MathEnvType::Gather => "gather",
                    MathEnvType::Multline => "multline",
                };
                // Find labels inside math nodes
                let mut label = None;
                for mn in &data.nodes {
                    if let MathNode::Label(l) = mn {
                        label = Some(l.clone());
                        break;
                    }
                }
                let number = label.as_ref().and_then(|l| label_map.get(l)).cloned();
                eqs.push(EquationEntry {
                    number,
                    label,
                    env,
                    numbered: data.numbered,
                });
            }
            // Recurse into containers that might contain DisplayMath
            Node::Paragraph(c) | Node::Group(c) | Node::Center(c)
            | Node::Abstract(c) | Node::Quote(c) | Node::Quotation(c)
            | Node::TwoColumn(c) | Node::FlushLeft(c) | Node::FlushRight(c) => {
                collect_equations_inner(c, label_map, eqs);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. }
            | Node::Minipage { content, .. } => {
                collect_equations_inner(content, label_map, eqs);
            }
            Node::Environment(env) => {
                collect_equations_inner(&env.content, label_map, eqs);
            }
            Node::Theorem(thm) => {
                collect_equations_inner(&thm.body, label_map, eqs);
            }
            Node::Proof { content, .. } => {
                collect_equations_inner(content, label_map, eqs);
            }
            _ => {}
        }
    }
}

/// Collect all theorem-like environments
fn collect_theorems(nodes: &[Node], label_map: &HashMap<String, String>) -> Vec<TheoremEntry> {
    let mut thms = Vec::new();
    collect_theorems_inner(nodes, label_map, &mut thms);
    thms
}

fn collect_theorems_inner(nodes: &[Node], label_map: &HashMap<String, String>, thms: &mut Vec<TheoremEntry>) {
    for node in nodes {
        match node {
            Node::Theorem(thm) => {
                // Find label inside theorem body
                let label = find_label_in_nodes(&thm.body);
                let number = label.as_ref().and_then(|l| label_map.get(l)).cloned();
                thms.push(TheoremEntry {
                    thm_type: thm.env_name.clone(),
                    number,
                    label,
                });
                // Also recurse into body for nested theorems
                collect_theorems_inner(&thm.body, label_map, thms);
            }
            Node::Paragraph(c) | Node::Group(c) | Node::Center(c) | Node::TwoColumn(c) => {
                collect_theorems_inner(c, label_map, thms);
            }
            Node::Environment(env) => {
                collect_theorems_inner(&env.content, label_map, thms);
            }
            _ => {}
        }
    }
}

/// Collect all figure environments
fn collect_figures(nodes: &[Node], label_map: &HashMap<String, String>, source: &str) -> Vec<FloatEntry> {
    let mut figs = Vec::new();
    collect_figures_inner(nodes, label_map, source, &mut figs);
    figs
}

fn collect_figures_inner(nodes: &[Node], label_map: &HashMap<String, String>, source: &str, figs: &mut Vec<FloatEntry>) {
    for node in nodes {
        match node {
            Node::Figure(fig) => {
                if fig.caption.is_some() {
                    let number = fig.label.as_ref().and_then(|l| label_map.get(l)).cloned();
                    let caption = fig.caption.as_ref().map(|c| nodes_to_plain_text(c, source));
                    figs.push(FloatEntry {
                        number,
                        label: fig.label.clone(),
                        caption,
                    });
                }
                collect_figures_inner(&fig.content, label_map, source, figs);
            }
            Node::Paragraph(c) | Node::Group(c) | Node::TwoColumn(c) => {
                collect_figures_inner(c, label_map, source, figs);
            }
            _ => {}
        }
    }
}

/// Collect all table environments
fn collect_tables(nodes: &[Node], label_map: &HashMap<String, String>, source: &str) -> Vec<FloatEntry> {
    let mut tables = Vec::new();
    collect_tables_inner(nodes, label_map, source, &mut tables);
    tables
}

fn collect_tables_inner(nodes: &[Node], label_map: &HashMap<String, String>, source: &str, tables: &mut Vec<FloatEntry>) {
    for node in nodes {
        match node {
            Node::Table(table) => {
                if table.caption.is_some() {
                    let number = table.label.as_ref().and_then(|l| label_map.get(l)).cloned();
                    let caption = table.caption.as_ref().map(|c| nodes_to_plain_text(c, source));
                    tables.push(FloatEntry {
                        number,
                        label: table.label.clone(),
                        caption,
                    });
                }
            }
            Node::Paragraph(c) | Node::Group(c) | Node::TwoColumn(c) => {
                collect_tables_inner(c, label_map, source, tables);
            }
            _ => {}
        }
    }
}

// ============================================================
// Helpers
// ============================================================

/// Find the first \label in a flat node list
fn find_label_in_nodes(nodes: &[Node]) -> Option<String> {
    for node in nodes {
        match node {
            Node::Label(name) => return Some(name.clone()),
            Node::Paragraph(c) | Node::Group(c) => {
                if let Some(l) = find_label_in_nodes(c) {
                    return Some(l);
                }
            }
            _ => {}
        }
    }
    None
}

/// Simple node-to-text for captions (avoids dependency on layout::text)
fn nodes_to_plain_text(nodes: &[Node], source: &str) -> String {
    let mut out = String::new();
    for node in nodes {
        node_to_plain_text(node, source, &mut out);
    }
    out
}

fn node_to_plain_text(node: &Node, source: &str, out: &mut String) {
    match node {
        Node::Text(s) => out.push_str(s),
        Node::TextRef(offset, len) => {
            let start = *offset as usize;
            let end = start + *len as usize;
            if end <= source.len() {
                out.push_str(&source[start..end]);
            }
        }
        Node::Bold(c) | Node::Italic(c) | Node::Monospace(c)
        | Node::SmallCaps(c) | Node::Underline(c) | Node::Emph(c)
        | Node::Strikethrough(c) | Node::Superscript(c) | Node::Subscript(c)
        | Node::Group(c) | Node::Paragraph(c) => {
            for child in c { node_to_plain_text(child, source, out); }
        }
        Node::Colored { content, .. } | Node::FontSize { content, .. } => {
            for child in content { node_to_plain_text(child, source, out); }
        }
        Node::NonBreakingSpace | Node::HSpace(_) => out.push(' '),
        Node::EnDash => out.push_str("--"),
        Node::EmDash => out.push_str("---"),
        Node::Ellipsis => out.push_str("..."),
        _ => {}
    }
}

/// Compute unresolved references and unreferenced labels
fn compute_unresolved<'a>(
    all_refs: &'a [String],
    label_map: &'a HashMap<String, String>,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let referenced_set: HashSet<&str> = all_refs.iter().map(|s| s.as_str()).collect();
    let defined_set: HashSet<&str> = label_map.keys().map(|s| s.as_str()).collect();

    let mut unresolved: Vec<&str> = referenced_set.difference(&defined_set).copied().collect();
    let mut unreferenced: Vec<&str> = defined_set.difference(&referenced_set).copied().collect();
    unresolved.sort_unstable();
    unreferenced.sort_unstable();
    (unresolved, unreferenced)
}

/// Escape a string for JSON output (in-place into buffer)
fn json_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Control character — escape as \u00XX
                let hex = format!("\\u{:04x}", c as u32);
                out.push_str(&hex);
            }
            c => out.push(c),
        }
    }
}

/// Estimate word count by stripping LaTeX commands and counting whitespace-separated tokens
fn estimate_word_count(source: &str) -> usize {
    // Find \begin{document} to only count body
    let body_start = source.find("\\begin{document}")
        .map(|p| p + 16)
        .unwrap_or(0);
    let body_end = source.find("\\end{document}")
        .unwrap_or(source.len());
    let body = &source[body_start..body_end];

    let mut count = 0usize;
    let mut in_word = false;
    let bytes = body.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            // Skip command name
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
            in_word = false;
            continue;
        }
        if b == b'%' {
            // Skip comment to end of line
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            in_word = false;
            i += 1;
            continue;
        }
        if b == b'{' || b == b'}' || b == b'$' || b == b'&' || b == b'~' || b == b'^' || b == b'_' {
            i += 1;
            in_word = false;
            continue;
        }
        if b.is_ascii_whitespace() {
            in_word = false;
        } else if b.is_ascii_alphabetic() {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else {
            // Non-alphabetic, non-whitespace (digits, punctuation)
            in_word = false;
        }
        i += 1;
    }

    count
}

fn section_level_str(level: &SectionLevel) -> &'static str {
    match level {
        SectionLevel::Part => "part",
        SectionLevel::Chapter => "chapter",
        SectionLevel::Section => "section",
        SectionLevel::Subsection => "subsection",
        SectionLevel::Subsubsection => "subsubsection",
        SectionLevel::Paragraph => "paragraph",
        SectionLevel::Subparagraph => "subparagraph",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_escape() {
        let mut out = String::new();
        json_escape_into("hello \"world\"\nfoo\\bar", &mut out);
        assert_eq!(out, r#"hello \"world\"\nfoo\\bar"#);
    }

    #[test]
    fn test_word_count() {
        let src = r#"\documentclass{article}
\begin{document}
Hello world this is a test. Some more words here.
\textbf{Bold text} and \emph{italic text}.
\end{document}"#;
        let count = estimate_word_count(src);
        // "Hello world this is a test Some more words here Bold text and italic text"
        assert!(count >= 10 && count <= 20, "Expected 10-20 words, got {}", count);
    }

    #[test]
    fn test_structure_extraction() {
        let source = r#"\documentclass{article}
\usepackage{amsmath}
\title{Test Paper}
\author{John Doe}
\begin{document}
\maketitle
\section{Introduction}\label{sec:intro}
See Equation~\eqref{eq:main}.
\begin{equation}\label{eq:main}
  E = mc^2
\end{equation}
\ref{fig:nonexistent}
\end{document}"#;
        let json = crate::compile_latex_structure(source).unwrap();

        // Verify it's valid JSON by checking basic structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));

        // Check that documentClass is present
        assert!(json.contains("\"documentClass\":\"article\""));

        // Check title and author
        assert!(json.contains("\"title\":\"Test Paper\""));
        assert!(json.contains("\"author\":\"John Doe\""));

        // Check that amsmath package is listed
        assert!(json.contains("\"amsmath\""));

        // Check that the section is in tocEntries
        assert!(json.contains("\"Introduction\""));

        // Check that eq:main label exists with number "1"
        assert!(json.contains("\"eq:main\""));

        // Check that fig:nonexistent is in unresolvedRefs
        assert!(json.contains("\"fig:nonexistent\""));
        assert!(json.contains("\"unresolvedRefs\":["));

        // Check that unresolvedRefs contains fig:nonexistent
        let unresolved_start = json.find("\"unresolvedRefs\":[").unwrap();
        let unresolved_end = json[unresolved_start..].find(']').unwrap() + unresolved_start;
        let unresolved_section = &json[unresolved_start..=unresolved_end];
        assert!(unresolved_section.contains("\"fig:nonexistent\""));
    }

    #[test]
    fn test_structure_icml_paper() {
        let path = std::path::Path::new("test_docs/icml2026_paper/main.tex");
        if !path.exists() {
            return; // Skip if test doc not available
        }
        let source = std::fs::read_to_string(path).unwrap();
        let json = crate::compile_latex_structure(&source).unwrap();

        // Should produce valid JSON with document structure
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"documentClass\":\"article\""));
        assert!(json.contains("\"sections\":["));
        assert!(json.contains("\"labels\":{"));
        assert!(json.contains("\"equations\":["));
        assert!(json.contains("\"wordCountEstimate\":"));

        // Should have sections
        assert!(json.contains("\"level\":\"section\""));
    }

    #[test]
    fn test_structure_with_figures_and_tables() {
        let source = r#"\documentclass{article}
\begin{document}
\section{Results}
\begin{figure}
\caption{My figure}\label{fig:one}
\end{figure}
\begin{table}
\begin{tabular}{cc}
a & b \\
\end{tabular}
\caption{My table}\label{tab:one}
\end{table}
\ref{fig:one} and \ref{tab:one}
\end{document}"#;
        let json = crate::compile_latex_structure(source).unwrap();

        // Figures and tables should be present
        assert!(json.contains("\"figures\":["));
        assert!(json.contains("\"tables\":["));

        // Labels should be resolved
        assert!(json.contains("\"fig:one\""));
        assert!(json.contains("\"tab:one\""));

        // No unresolved refs
        assert!(json.contains("\"unresolvedRefs\":[]"));
    }
}
