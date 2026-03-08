/// Styled span conversion and rich paragraph layout

use std::collections::HashMap;
use crate::color::Color;
use crate::document::*;
use crate::typeset::FontStyle;
use crate::font::{self, FontId};
use crate::math_layout;
use super::state::{LayoutState, baselineskip_factor};
use super::text::{node_to_text_resolved, resolve_citations, layout_text_content, layout_text_content_no_indent};
use super::math::emit_math_elements;


use anyhow::Result;

pub(super) struct StyledSpan {
    pub text: String,
    pub style: FontStyle,
    pub color: Color,
    pub font_size: f32,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Split text into uppercase (normal size) and lowercase (uppercase at 75% size) spans for small caps
fn emit_smallcaps_spans(text: &str, style: FontStyle, color: Color, font_size: f32, out: &mut Vec<StyledSpan>) {
    let sc_size = font_size * 0.75;
    let mut seg_start = 0;
    let mut seg_is_lower = text.as_bytes().first().map_or(false, |b| b.is_ascii_lowercase());

    for (i, ch) in text.char_indices() {
        let is_lower = ch.is_ascii_lowercase();
        if is_lower != seg_is_lower && i > seg_start {
            let seg = &text[seg_start..i];
            if seg_is_lower {
                out.push(StyledSpan { text: seg.to_ascii_uppercase(), style, color, font_size: sc_size, underline: false, strikethrough: false });
            } else {
                out.push(StyledSpan { text: seg.to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            seg_start = i;
            seg_is_lower = is_lower;
        }
    }
    if seg_start < text.len() {
        let seg = &text[seg_start..];
        if seg_is_lower {
            out.push(StyledSpan { text: seg.to_ascii_uppercase(), style, color, font_size: sc_size, underline: false, strikethrough: false });
        } else {
            out.push(StyledSpan { text: seg.to_string(), style, color, font_size, underline: false, strikethrough: false });
        }
    }
}

pub(super) fn nodes_to_spans(nodes: &[Node], style: FontStyle, color: Color, font_size: f32, base_size: f32, out: &mut Vec<StyledSpan>, source: &str, labels: &HashMap<String, String>, citations: &HashMap<String, u32>) {
    nodes_to_spans_sc(nodes, style, color, font_size, base_size, false, out, source, labels, citations);
}

fn nodes_to_spans_sc(nodes: &[Node], style: FontStyle, color: Color, font_size: f32, base_size: f32, smallcaps: bool, out: &mut Vec<StyledSpan>, source: &str, labels: &HashMap<String, String>, citations: &HashMap<String, u32>) {
    let mut style = style;
    let mut color = color;
    let mut font_size = font_size;
    let mut smallcaps = smallcaps;
    for node in nodes {
        match node {
            Node::FontStyleDecl(decl) => {
                match decl {
                    FontDeclType::SmallCaps => { smallcaps = true; }
                    _ => {
                        smallcaps = false;
                        style = match decl {
                            FontDeclType::Bold => FontStyle::Bold,
                            FontDeclType::Italic => FontStyle::Italic,
                            FontDeclType::Monospace => FontStyle::Monospace,
                            FontDeclType::Regular => FontStyle::Regular,
                            FontDeclType::SmallCaps => unreachable!(),
                        };
                    }
                }
            }
            Node::ColorDecl(c) => { color = *c; }
            Node::FontSize { size, content } if content.is_empty() => {
                font_size = size.to_points(base_size);
            }
            Node::Text(s) => {
                let normalized = if s.contains('\n') { s.replace('\n', " ") } else { s.clone() };
                if smallcaps {
                    emit_smallcaps_spans(&normalized, style, color, font_size, out);
                } else {
                    out.push(StyledSpan { text: normalized, style, color, font_size, underline: false, strikethrough: false });
                }
            }
            Node::TextRef(offset, len) => {
                let raw = &source[*offset as usize..(*offset as usize + *len as usize)];
                let text = if raw.contains('\n') { raw.replace('\n', " ") } else { raw.to_string() };
                if smallcaps {
                    emit_smallcaps_spans(&text, style, color, font_size, out);
                } else {
                    out.push(StyledSpan { text, style, color, font_size, underline: false, strikethrough: false });
                }
            }
            Node::Bold(children) => {
                let s = match style { FontStyle::Italic => FontStyle::BoldItalic, _ => FontStyle::Bold };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Italic(children) | Node::Emph(children) => {
                let s = match style { FontStyle::Bold => FontStyle::BoldItalic, _ => FontStyle::Italic };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Monospace(children) => {
                let mut t = String::new();
                for c in children { node_to_text_resolved(c, &mut t, source, labels); }
                out.push(StyledSpan { text: t, style: FontStyle::Monospace, color, font_size, underline: false, strikethrough: false });
            }
            Node::Code(s) => {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Monospace, color, font_size, underline: false, strikethrough: false });
            }
            Node::SmallCaps(children) => {
                nodes_to_spans_sc(children, style, color, font_size, base_size, true, out, source, labels, citations);
            }
            Node::Underline(children) => {
                let start_idx = out.len();
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
                for span in &mut out[start_idx..] { span.underline = true; }
            }
            Node::Strikethrough(children) => {
                let start_idx = out.len();
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
                for span in &mut out[start_idx..] { span.strikethrough = true; }
            }
            Node::Dingbat(code) => {
                // ZapfDingbats character: text is a single char whose byte value is the dingbat code
                let text = String::from(char::from(*code));
                out.push(StyledSpan { text, style: FontStyle::ZapfDingbats, color, font_size, underline: false, strikethrough: false });
            }
            Node::Group(children) | Node::Superscript(children) | Node::Subscript(children) => {
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Colored { content, color: c } => {
                nodes_to_spans_sc(content, style, *c, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::FontSize { size, content } => {
                let new_size = size.to_points(base_size);
                nodes_to_spans_sc(content, style, color, new_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Paragraph(children) => {
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::NonBreakingSpace | Node::HSpace(_) => {
                out.push(StyledSpan { text: " ".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::HFill => {
                // Sentinel marker — handled in layout_rich_paragraph
                out.push(StyledSpan { text: "\x01HFILL\x01".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::LineBreak => {
                out.push(StyledSpan { text: "\n".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::InlineMath(math) => {
                inline_math_to_spans(math, color, font_size, out);
            }
            Node::Citation(key, opt, cite_style) => {
                // For spans, use numeric style (author-year handled at layout level)
                let cite_text = resolve_citations(key, opt.as_deref(), citations, *cite_style, &std::collections::HashMap::new());
                out.push(StyledSpan { text: cite_text, style, color, font_size, underline: false, strikethrough: false });
            }
            Node::Href { content, .. } => {
                let link_color = Color::from_rgb_u8(0, 0, 180);
                nodes_to_spans_sc(content, style, link_color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            _ => {
                let mut t = String::new();
                node_to_text_resolved(node, &mut t, source, labels);
                if !t.is_empty() {
                    out.push(StyledSpan { text: t, style, color, font_size, underline: false, strikethrough: false });
                }
            }
        }
    }
}

fn inline_math_to_spans(nodes: &[MathNode], color: Color, font_size: f32, out: &mut Vec<StyledSpan>) {
    for node in nodes { inline_math_node_to_spans(node, color, font_size, out); }
}

fn inline_math_node_to_spans(node: &MathNode, color: Color, font_size: f32, out: &mut Vec<StyledSpan>) {
    match node {
        MathNode::Number(s) => {
            out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Variable(c) => {
            out.push(StyledSpan { text: c.to_string(), style: FontStyle::Italic, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Operator(s) => {
            let first_char = s.chars().next().unwrap_or('?');
            if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            } else {
                out.push(StyledSpan { text: format!(" {} ", s), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Symbol(s) => {
            let first_char = s.chars().next().unwrap_or('?');
            if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            } else if first_char.is_ascii() {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Italic, color, font_size, underline: false, strikethrough: false });
            } else {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Text(s) => {
            out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Function(name) | MathNode::OperatorName(name) => {
            out.push(StyledSpan { text: name.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Space(w) => {
            if *w > 0.0 {
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Group(nodes) => { inline_math_to_spans(nodes, color, font_size, out); }
        MathNode::Phantom(_) => {}
        MathNode::Super(nodes) | MathNode::Sub(nodes) => { inline_math_to_spans(nodes, color, font_size, out); }
        MathNode::Frac { numer, denom } => {
            inline_math_to_spans(numer, color, font_size, out);
            out.push(StyledSpan { text: "/".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(denom, color, font_size, out);
        }
        MathNode::Sqrt { radicand, .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{221A}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
            out.push(StyledSpan { text: "(".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(radicand, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Left(d) | MathNode::Right(d) => {
            let first_char = d.chars().next().unwrap_or('.');
            if first_char != '.' {
                if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                    out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
                } else {
                    out.push(StyledSpan { text: d.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                }
            }
        }
        MathNode::DelimitedGroup { left, right, content } => {
            // For inline span fallback, just render as left + content + right
            let ld = left.chars().next().unwrap_or('.');
            if ld != '.' {
                out.push(StyledSpan { text: left.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
            inline_math_to_spans(content, color, font_size, out);
            let rd = right.chars().next().unwrap_or('.');
            if rd != '.' {
                out.push(StyledSpan { text: right.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Sum { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{2211}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Integral { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{222B}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Product { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{220F}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Accent { base, .. } => { inline_math_to_spans(base, color, font_size, out); }
        MathNode::Over { content, .. } | MathNode::Under { content, .. } => {
            inline_math_to_spans(content, color, font_size, out);
        }
        MathNode::Binom { top, bottom } => {
            out.push(StyledSpan { text: "(".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(top, color, font_size, out);
            out.push(StyledSpan { text: ", ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(bottom, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Overset { base, .. } | MathNode::Underset { base, .. } => {
            inline_math_to_spans(base, color, font_size, out);
        }
        MathNode::MathFont { content, .. } => { inline_math_to_spans(content, color, font_size, out); }
        MathNode::Matrix { rows, .. } => {
            for (i, row) in rows.iter().enumerate() {
                if i > 0 { out.push(StyledSpan { text: "; ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false }); }
                for (j, cell) in row.iter().enumerate() {
                    if j > 0 { out.push(StyledSpan { text: ", ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false }); }
                    inline_math_to_spans(cell, color, font_size, out);
                }
            }
        }
        MathNode::Cases { rows } => {
            for (i, (val, cond)) in rows.iter().enumerate() {
                if i > 0 { out.push(StyledSpan { text: "; ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false }); }
                inline_math_to_spans(val, color, font_size, out);
                if let Some(c) = cond {
                    out.push(StyledSpan { text: " if ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                    inline_math_to_spans(c, color, font_size, out);
                }
            }
        }
        MathNode::AlignmentMark => {
            out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::NewLine | MathNode::StyleSwitch(_) | MathNode::BigDelim { .. }
        | MathNode::Boxed(_) | MathNode::LimitOp { .. } | MathNode::NoTag | MathNode::Tag(_) | MathNode::Intertext(_)
        | MathNode::Label(_) | MathNode::Substack(_) | MathNode::StyledText(..) => {}
        MathNode::VPhantom(_) | MathNode::HPhantom(_) | MathNode::Rule { .. } => {}
        MathNode::Pmod(content) => {
            out.push(StyledSpan { text: " (mod ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(content, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Pod(content) => {
            out.push(StyledSpan { text: " (".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(content, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Bmod => {
            out.push(StyledSpan { text: " mod ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::MathRel(content) | MathNode::MathBin(content) => { inline_math_to_spans(content, color, font_size, out); }
        MathNode::Middle(d) => {
            out.push(StyledSpan { text: d.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
    }
}

/// Layout a paragraph with rich inline formatting (bold, italic, etc.).
pub(super) fn layout_rich_paragraph(children: &[Node], state: &mut LayoutState, source: &str, with_indent: bool) -> Result<()> {
    let has_formatting = children.iter().any(|n| matches!(n,
        Node::Bold(_) | Node::Italic(_) | Node::Emph(_) | Node::Monospace(_)
        | Node::Colored { .. } | Node::Code(_) | Node::SmallCaps(_)
        | Node::Underline(_) | Node::InlineMath(_) | Node::Href { .. }
        | Node::Footnote(_) | Node::FontStyleDecl(_) | Node::ColorDecl(_) | Node::Group(_)
        | Node::FontSize { .. } | Node::Citation(..) | Node::Cref(..)
        | Node::Dingbat(_)
    ));

    if !has_formatting {
        state.text_buf.clear();
        let labels: &HashMap<String, String> = unsafe { &*(&state.label_map as *const _) };
        for node in children {
            node_to_text_resolved(node, &mut state.text_buf, source, labels);
        }
        let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
        if !text.is_empty() {
            if with_indent { layout_text_content(text, state)?; }
            else { layout_text_content_no_indent(text, state)?; }
        }
        return Ok(());
    }

    struct StyledWord {
        text: String, style: FontStyle, color: Color, font_size: f32, width: f32,
        math: Option<math_layout::MathBox>, superscript: bool, underline: bool, strikethrough: bool,
    }

    let font_size = state.current_font_size;
    let base_font_size = state.base_font_size;
    let line_height = font_size * baselineskip_factor(base_font_size);
    let step = line_height * state.line_spacing;
    let space_width = crate::font::measure_text(" ", crate::font::FontId::TimesRoman, font_size);
    let text_width = state.text_width();
    let indent = if with_indent { state.paragraph_indent } else { 0.0 };

    let mut words: Vec<StyledWord> = Vec::new();
    let fn_count_before = state.footnotes.len();
    let labels_ref: &HashMap<String, String> = &state.label_map;
    let citations_ref: &HashMap<String, u32> = &state.citation_map;

    for child in children {
        match child {
            Node::Footnote(content) => {
                state.footnote_counter += 1;
                let num = state.footnote_counter;
                let num_str = format!("{}", num);
                let sup_size = font_size * 0.65;
                let w = font::measure_text(&num_str, FontId::TimesRoman, sup_size);
                words.push(StyledWord { text: num_str, style: FontStyle::Regular, color: state.current_color, font_size, width: w, math: None, superscript: true, underline: false, strikethrough: false });
                state.footnotes.push(content.clone());
            }
            Node::InlineMath(math) => {
                let math_box = math_layout::layout_math(math, font_size);
                if math_box.width > 0.0 {
                    if let Some(last) = words.last() {
                        if last.text != " " && last.text != "\n" {
                            words.push(StyledWord { text: " ".to_string(), style: FontStyle::Regular, color: state.current_color, width: space_width * 0.5, math: None, superscript: false, font_size, underline: false, strikethrough: false });
                        }
                    }
                    let w = math_box.width;
                    words.push(StyledWord { text: String::new(), style: FontStyle::Regular, color: state.current_color, font_size, width: w, math: Some(math_box), superscript: false, underline: false, strikethrough: false });
                }
            }
            Node::Cref(label, capitalize) => {
                let num = state.label_map.get(label).cloned().unwrap_or_else(|| "??".to_string());
                let type_name = state.label_types.get(label).map(|s| s.as_str()).unwrap_or("section");
                let prefix = match type_name {
                    "section" | "subsection" | "subsubsection" => if *capitalize { "Section" } else { "section" },
                    "chapter" => if *capitalize { "Chapter" } else { "chapter" },
                    "part" => if *capitalize { "Part" } else { "part" },
                    "figure" => if *capitalize { "Figure" } else { "figure" },
                    "table" => if *capitalize { "Table" } else { "table" },
                    "equation" => if *capitalize { "Equation" } else { "eq." },
                    "theorem" => if *capitalize { "Theorem" } else { "theorem" },
                    "lemma" => if *capitalize { "Lemma" } else { "lemma" },
                    "proposition" => if *capitalize { "Proposition" } else { "proposition" },
                    "corollary" => if *capitalize { "Corollary" } else { "corollary" },
                    "definition" => if *capitalize { "Definition" } else { "definition" },
                    "remark" => if *capitalize { "Remark" } else { "remark" },
                    "example" => if *capitalize { "Example" } else { "example" },
                    _ => if *capitalize { "Section" } else { "section" },
                };
                let ref_text = if type_name == "equation" && !*capitalize {
                    format!("{}\u{00A0}({})", prefix, num)
                } else {
                    format!("{}\u{00A0}{}", prefix, num)
                };
                let w = font::measure_text(&ref_text, FontId::TimesRoman, font_size);
                words.push(StyledWord { text: ref_text, style: state.current_font_style, color: state.current_color, font_size, width: w, math: None, superscript: false, underline: false, strikethrough: false });
            }
            _ => {
                let mut node_spans = Vec::new();
                let node_style = state.current_font_style;
                let node_color = state.current_color;
                nodes_to_spans(&[child.clone()], node_style, node_color, font_size, base_font_size, &mut node_spans, source, labels_ref, citations_ref);
                for span in &node_spans {
                    let sf = span.font_size;
                    let font_id = crate::font::style_to_font_id(span.style);
                    let sw = crate::font::measure_text(" ", font_id, sf);
                    if span.text == "\n" {
                        words.push(StyledWord { text: "\n".to_string(), style: span.style, color: span.color, font_size: sf, width: 0.0, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                        continue;
                    }
                    if span.text == "\x01HFILL\x01" {
                        words.push(StyledWord { text: "\x01HFILL\x01".to_string(), style: span.style, color: span.color, font_size: sf, width: 0.0, math: None, superscript: false, underline: false, strikethrough: false });
                        continue;
                    }
                    let font_id = font::style_to_font_id(span.style);
                    let parts: Vec<&str> = span.text.split_whitespace().collect();
                    let starts_with_space = span.text.starts_with(char::is_whitespace);
                    let ends_with_space = span.text.ends_with(char::is_whitespace);
                    if starts_with_space && !words.is_empty() {
                        if let Some(last) = words.last() {
                            if last.text != " " && last.text != "\n" {
                                words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                            }
                        }
                    }
                    for (i, part) in parts.iter().enumerate() {
                        if i > 0 {
                            words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                        }
                        let w = font::measure_text(part, font_id, sf);
                        words.push(StyledWord { text: part.to_string(), style: span.style, color: span.color, font_size: sf, width: w, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                    }
                    if ends_with_space && !parts.is_empty() {
                        words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                    }
                }
            }
        }
    }

    let fn_count_after = state.footnotes.len();
    for _ in fn_count_before..fn_count_after { state.reserve_footnote_space(); }

    let text_ascent = font_size * 0.75;
    let text_descent = font_size * 0.25;
    state.ensure_space(line_height);
    let normal_start = state.text_left() + indent;
    let initial_line_x = if state.current_x > normal_start + 1.0 { state.current_x } else { normal_start };
    let first_line_used = initial_line_x - state.text_left();

    struct LineInfo { start: usize, end: usize, max_above: f32, max_below: f32, hyphen: Option<(usize, usize)> }
    let mut lines: Vec<LineInfo> = Vec::new();
    let mut line_start = 0usize;
    let mut current_line_width = 0.0f32;
    let mut first_line = true;
    let mut line_max_above = text_ascent;
    let mut line_max_below = text_descent;

    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if word.text == "\n" {
            lines.push(LineInfo { start: line_start, end: i, max_above: line_max_above, max_below: line_max_below, hyphen: None });
            line_start = i + 1; current_line_width = 0.0; first_line = false;
            line_max_above = text_ascent; line_max_below = text_descent; i += 1; continue;
        }
        if word.text == " " { current_line_width += word.width; i += 1; continue; }

        let effective_max = if first_line { text_width - first_line_used } else { text_width };
        if current_line_width > 0.0 && current_line_width + word.width > effective_max {
            let mut hyphenated = false;
            if word.math.is_none() && !word.superscript && word.text.len() >= 5 {
                let remaining = effective_max - current_line_width;
                let fid = font::style_to_font_id(word.style);
                let hyphen_w = font::measure_text("-", fid, word.font_size);
                let avg_char_w = word.width / word.text.len() as f32;
                let max_prefix = ((remaining - hyphen_w) / avg_char_w) as usize;
                if max_prefix >= 2 {
                    if let Some(bp) = crate::hyphenate::best_break(word.text.as_bytes(), max_prefix) {
                        let prefix_w = font::measure_text(&word.text[..bp], fid, word.font_size);
                        if prefix_w + hyphen_w <= remaining {
                            lines.push(LineInfo { start: line_start, end: i + 1, max_above: line_max_above, max_below: line_max_below, hyphen: Some((i, bp)) });
                            line_start = i; current_line_width = 0.0; first_line = false;
                            line_max_above = text_ascent; line_max_below = text_descent;
                            if word.font_size > font_size + 0.5 {
                                line_max_above = line_max_above.max(word.font_size * 0.75);
                                line_max_below = line_max_below.max(word.font_size * 0.25);
                            }
                            let suffix_w = font::measure_text(&word.text[bp..], fid, word.font_size);
                            current_line_width = suffix_w; i += 1; hyphenated = true;
                        }
                    }
                }
            }
            if !hyphenated {
                lines.push(LineInfo { start: line_start, end: i, max_above: line_max_above, max_below: line_max_below, hyphen: None });
                line_start = i; current_line_width = 0.0; first_line = false;
                line_max_above = text_ascent; line_max_below = text_descent; continue;
            } else { continue; }
        }

        if let Some(ref math_box) = word.math {
            line_max_above = line_max_above.max(math_box.height);
            line_max_below = line_max_below.max(math_box.depth);
        } else if word.font_size > font_size + 0.5 {
            line_max_above = line_max_above.max(word.font_size * 0.75);
            line_max_below = line_max_below.max(word.font_size * 0.25);
        }
        current_line_width += word.width; i += 1;
    }
    if line_start < words.len() {
        lines.push(LineInfo { start: line_start, end: words.len(), max_above: line_max_above, max_below: line_max_below, hyphen: None });
    }

    // Orphan control: prevent single line stranded at page bottom
    // (LaTeX \clubpenalty=150 equivalent)
    if lines.len() >= 2 {
        let remaining_space = state.cached_max_y - state.current_y;
        let first_line_h = lines[0].max_above + lines[0].max_below;
        // Fire only if exactly one line fits (not two or more)
        if remaining_space >= first_line_h && remaining_space < first_line_h + step {
            state.ensure_space(remaining_space + 1.0); // force page break
        }
    }

    let total_lines = lines.len();
    let mut first_line = true;
    let mut prev_max_below = text_descent;
    for (line_idx, line) in lines.iter().enumerate() {
        if !first_line {
            let effective_step = (prev_max_below + line.max_above).max(step);
            state.current_y += effective_step;
            state.ensure_space(line_height);
        } else if line.max_above > text_ascent {
            state.current_y += line.max_above - text_ascent;
        }

        let mut line_x = if first_line { initial_line_x } else { state.text_left() };

        // Justification: compute extra space per word gap for non-last lines
        let is_last_line = line_idx == total_lines - 1;
        let available = if first_line { text_width - first_line_used } else { text_width };
        let mut extra_per_space = 0.0f32;
        if !is_last_line {
            let mut content_w = 0.0f32;
            let mut space_count = 0u32;
            for wi in line.start..line.end {
                let word = &words[wi];
                if word.text == " " {
                    space_count += 1;
                    content_w += word.width;
                } else {
                    content_w += word.width;
                }
            }
            if space_count > 0 {
                let slack = available - content_w;
                // Justify if line is at least 55% full (TeX-like threshold)
                if slack > 0.0 && slack < available * 0.45 {
                    extra_per_space = slack / space_count as f32;
                    // Allow up to 0.6em stretch per space for professional justification
                    extra_per_space = extra_per_space.min(font_size * 0.6);
                } else if slack < 0.0 && slack > -font_size * 1.5 {
                    // Allow slight compression for overfull lines
                    extra_per_space = slack / space_count as f32;
                    extra_per_space = extra_per_space.max(-font_size * 0.1);
                }
            }
        }

        for wi in line.start..line.end {
            let word = &words[wi];
            if word.text == " " { line_x += word.width + extra_per_space; continue; }
            if word.text == "\x01HFILL\x01" {
                // Compute remaining content width after HFill
                let mut remaining_w = 0.0f32;
                for rw in (wi + 1)..line.end {
                    remaining_w += words[rw].width;
                }
                let right_edge = state.text_left() + text_width;
                line_x = (right_edge - remaining_w).max(line_x);
                continue;
            }
            if let Some((hyph_wi, bp)) = line.hyphen {
                if wi == hyph_wi {
                    state.current_x = line_x;
                    state.text_buf.clear();
                    state.text_buf.push_str(&word.text[..bp]);
                    state.text_buf.push('-');
                    let hyph: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
                    state.emit_text(hyph, word.font_size, word.style, word.color);
                    continue;
                }
            }
            state.current_x = line_x;
            if let Some(ref math_box) = word.math {
                let saved_x = state.current_x;
                let saved_y = state.current_y;
                emit_math_elements(math_box, line_x, state.current_y, state);
                state.current_x = saved_x;
                state.current_y = saved_y;
            } else if word.superscript {
                let sup_size = word.font_size * 0.65;
                let saved_y = state.current_y;
                state.current_y -= word.font_size * 0.35;
                state.emit_text(&word.text, sup_size, word.style, word.color);
                state.current_y = saved_y;
            } else {
                state.emit_text(&word.text, word.font_size, word.style, word.color);
            }
            if word.underline && word.text != " " && !word.text.is_empty() {
                let ul_y = state.current_y + word.font_size * 0.15;
                let ul_thickness = (word.font_size * 0.05).max(0.4);
                state.emit_line(line_x, ul_y, line_x + word.width, ul_y, ul_thickness, word.color);
            }
            if word.strikethrough && word.text != " " && !word.text.is_empty() {
                let st_y = state.current_y - word.font_size * 0.25;
                let st_thickness = (word.font_size * 0.05).max(0.4);
                state.emit_line(line_x, st_y, line_x + word.width, st_y, st_thickness, word.color);
            }
            line_x += word.width;
        }

        if let Some((hyph_wi, bp)) = line.hyphen {
            let _ = (hyph_wi, bp);
        }
        prev_max_below = line.max_below;
        first_line = false;
    }

    state.current_y += step;
    state.current_x = state.text_left();
    Ok(())
}
