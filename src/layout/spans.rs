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
                            FontDeclType::SansSerif => FontStyle::SansSerif,
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
            Node::Italic(children) => {
                let s = match style { FontStyle::Bold => FontStyle::BoldItalic, _ => FontStyle::Italic };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Emph(children) => {
                // \emph toggles: italic→upright, upright→italic (LaTeX convention)
                let s = match style {
                    FontStyle::Italic => FontStyle::Regular,
                    FontStyle::BoldItalic => FontStyle::Bold,
                    FontStyle::Bold => FontStyle::BoldItalic,
                    _ => FontStyle::Italic,
                };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Monospace(children) => {
                let mut t = String::new();
                for c in children { node_to_text_resolved(c, &mut t, source, labels); }
                out.push(StyledSpan { text: t, style: FontStyle::Monospace, color, font_size, underline: false, strikethrough: false });
            }
            Node::SansSerif(children) => {
                let sf_style = match style {
                    FontStyle::Bold | FontStyle::SansSerifBold => FontStyle::SansSerifBold,
                    FontStyle::Italic | FontStyle::SansSerifItalic => FontStyle::SansSerifItalic,
                    FontStyle::BoldItalic | FontStyle::SansSerifBoldItalic => FontStyle::SansSerifBoldItalic,
                    _ => FontStyle::SansSerif,
                };
                nodes_to_spans_sc(children, sf_style, color, font_size, base_size, smallcaps, out, source, labels, citations);
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
            Node::LaTeXLogo => {
                out.push(StyledSpan { text: "LaTeX".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::TeXLogo => {
                out.push(StyledSpan { text: "TeX".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::Rule { width: w, height: h } => {
                // Sentinel marker with encoded dimensions — rendered as filled rectangle
                out.push(StyledSpan { text: format!("\x01RULE:{:.2}:{:.2}\x01", w, h), style, color, font_size, underline: false, strikethrough: false });
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

struct StyledWord {
    text: String, style: FontStyle, color: Color, font_size: f32, width: f32,
    math: Option<math_layout::MathBox>, superscript: bool, underline: bool, strikethrough: bool,
}

/// Compute max_above/max_below metrics for a line of styled words
fn compute_line_metrics(words: &[StyledWord], text_ascent: f32, text_descent: f32, _base_font_size: f32) -> (f32, f32) {
    let mut ma = text_ascent;
    let mut mb = text_descent;
    for w in words {
        if let Some(ref math_box) = w.math {
            ma = ma.max(math_box.height);
            mb = mb.max(math_box.depth);
        } else if (w.font_size - _base_font_size).abs() > 0.5 {
            // Recompute metrics for any non-base-sized text (both larger AND smaller)
            let wfid = font::style_to_font_id(w.style);
            let wa = font::font_ascent(wfid, w.font_size);
            let wb = font::font_descent(wfid, w.font_size);
            ma = ma.max(wa);
            mb = mb.max(wb);
        }
    }
    (ma, mb)
}

/// Layout a paragraph with rich inline formatting (bold, italic, etc.).
pub(super) fn layout_rich_paragraph(children: &[Node], state: &mut LayoutState, source: &str, with_indent: bool) -> Result<()> {
    // Apply \parskip between paragraphs (LaTeX applies parskip between all paragraphs)
    if state.paragraph_skip > 0.0 {
        state.add_vertical_space(state.paragraph_skip);
    }

    let has_formatting = children.iter().any(|n| matches!(n,
        Node::Bold(_) | Node::Italic(_) | Node::Emph(_) | Node::Monospace(_) | Node::SansSerif(_)
        | Node::Colored { .. } | Node::Code(_) | Node::SmallCaps(_)
        | Node::Underline(_) | Node::InlineMath(_) | Node::Href { .. }
        | Node::Footnote(_) | Node::FontStyleDecl(_) | Node::ColorDecl(_) | Node::Group(_)
        | Node::FontSize { .. } | Node::Citation(..) | Node::Cref(..)
        | Node::Dingbat(_)
    ));

    if !has_formatting {
        // Fast path for single TextRef: zero-copy layout directly from source
        if children.len() == 1 {
            if let Node::TextRef(offset, len) = &children[0] {
                let text = source[*offset as usize..(*offset as usize + *len as usize)].trim();
                if !text.is_empty() {
                    let src_off = (text.as_ptr() as usize - source.as_ptr() as usize) as u32;
                    if !with_indent { state.suppress_next_indent = true; }
                    super::text::layout_text_content_source(text, state, src_off)?;
                }
                return Ok(());
            }
        }
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

    let font_size = state.current_font_size;
    let base_font_size = state.base_font_size;
    let line_height = if let Some(bsk) = state.baseline_skip_override {
        bsk
    } else {
        font_size * baselineskip_factor(font_size)
    };
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
                let math_box = math_layout::layout_math_inline(math, font_size);
                if math_box.width > 0.0 {
                    if let Some(last) = words.last() {
                        if last.text != " " && last.text != "\n" {
                            // TeX \thinmuskip = 3mu ≈ font_size/6
                            let thin_space = font_size / 6.0;
                            words.push(StyledWord { text: " ".to_string(), style: FontStyle::Regular, color: state.current_color, width: thin_space, math: None, superscript: false, font_size, underline: false, strikethrough: false });
                        }
                    }
                    let w = math_box.width;
                    words.push(StyledWord { text: String::new(), style: FontStyle::Regular, color: state.current_color, font_size, width: w, math: Some(math_box), superscript: false, underline: false, strikethrough: false });
                }
            }
            Node::Superscript(content) => {
                // \textsuperscript: render at 65% size with vertical offset
                let mut sup_spans = Vec::new();
                nodes_to_spans(content, state.current_font_style, state.current_color, font_size * 0.65, base_font_size, &mut sup_spans, source, labels_ref, citations_ref);
                for span in &sup_spans {
                    let sf = span.font_size;
                    let font_id = font::style_to_font_id(span.style);
                    for part in span.text.split_whitespace() {
                        let w = font::measure_text(part, font_id, sf);
                        words.push(StyledWord { text: part.to_string(), style: span.style, color: span.color, font_size, width: w, math: None, superscript: true, underline: false, strikethrough: false });
                    }
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
                    if span.text.starts_with("\x01RULE:") {
                        // Parse encoded rule dimensions: \x01RULE:w:h\x01
                        let inner = &span.text[6..span.text.len()-1]; // strip \x01RULE: and \x01
                        let parts: Vec<&str> = inner.split(':').collect();
                        let rule_w: f32 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(10.0);
                        words.push(StyledWord { text: span.text.clone(), style: span.style, color: span.color, font_size: sf, width: rule_w, math: None, superscript: false, underline: false, strikethrough: false });
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

    let base_font_id = font::style_to_font_id(FontStyle::Regular);
    let text_ascent = font::font_ascent(base_font_id, font_size);
    let text_descent = font::font_descent(base_font_id, font_size);
    state.ensure_space(step);
    let normal_start = state.text_left() + indent;
    let initial_line_x = if state.current_x > normal_start + 1.0 { state.current_x } else { normal_start };
    let first_line_used = initial_line_x - state.text_left();

    struct LineInfo { start: usize, end: usize, max_above: f32, max_below: f32, hyphen: Option<(usize, usize)> }
    let mut lines: Vec<LineInfo> = Vec::new();

    // --- Knuth-Plass style optimal line breaking ---
    // Split paragraph into segments at forced breaks (\n), then run DP on each segment.
    let mut segments: Vec<(usize, usize)> = Vec::new(); // (start, end) in words[]
    {
        let mut seg_start = 0;
        for (i, w) in words.iter().enumerate() {
            if w.text == "\n" {
                segments.push((seg_start, i));
                seg_start = i + 1;
            }
        }
        if seg_start <= words.len() {
            segments.push((seg_start, words.len()));
        }
    }

    let mut is_first_line = true;
    for (seg_start, seg_end) in &segments {
        let seg_start = *seg_start;
        let seg_end = *seg_end;
        if seg_start >= seg_end { continue; }

        // Collect break opportunities within this segment (at spaces)
        // bp[0] = seg_start (start of segment), bp[last] = seg_end (end)
        let mut bp: Vec<usize> = vec![seg_start];
        for i in seg_start..seg_end {
            if words[i].text == " " {
                bp.push(i + 1);
            }
        }
        if *bp.last().unwrap() != seg_end {
            bp.push(seg_end);
        }
        let m = bp.len();

        if m <= 2 {
            // Single word or no break opportunities — one line
            let (ma, mb) = compute_line_metrics(&words[seg_start..seg_end], text_ascent, text_descent, font_size);
            lines.push(LineInfo { start: seg_start, end: seg_end, max_above: ma, max_below: mb, hyphen: None });
            is_first_line = false;
            continue;
        }

        // Prefix sums for O(1) line width queries
        let mut prefix: Vec<f32> = Vec::with_capacity(seg_end - seg_start + 1);
        prefix.push(0.0);
        for k in seg_start..seg_end {
            prefix.push(prefix.last().unwrap() + words[k].width);
        }
        let pw = |idx: usize| -> f32 { prefix[idx - seg_start] };

        // DP: cost[i] = minimum badness to optimally break words up to bp[i]
        let mut dp_cost: Vec<f64> = vec![f64::MAX; m];
        let mut dp_from: Vec<usize> = vec![0; m];
        dp_cost[0] = 0.0;

        for j in 1..m {
            let end = bp[j];
            // Width of trailing space at end of line (if any)
            let trail_sp = if end > seg_start && words[end - 1].text == " " {
                words[end - 1].width
            } else { 0.0 };

            for a in (0..j).rev() {
                if dp_cost[a] == f64::MAX { continue; }
                let start = bp[a];

                // O(1) line width via prefix sums
                let lw = pw(end) - pw(start) - trail_sp;

                let max_w = if is_first_line && a == 0 { text_width - first_line_used } else { text_width };

                // Prune: earlier starts only make lines wider
                if lw > max_w * 1.3 && j > a + 1 { break; }

                let is_last = j == m - 1;
                let badness: f64 = if is_last {
                    if lw > max_w { let o = (lw - max_w) as f64; o * o * 1000.0 } else { 0.0 }
                } else if lw > max_w {
                    let o = (lw - max_w) as f64;
                    o * o * 1000.0
                } else {
                    let slack = (max_w - lw) as f64;
                    let ratio = slack / max_w as f64;
                    if ratio > 0.5 { slack * slack * slack + 10000.0 }
                    else { slack * slack * slack }
                };

                let total = dp_cost[a] + badness;
                if total < dp_cost[j] {
                    dp_cost[j] = total;
                    dp_from[j] = a;
                }
            }
        }

        // Reconstruct optimal break sequence
        let mut opt_breaks: Vec<usize> = Vec::new();
        let mut k = m - 1;
        while k > 0 { opt_breaks.push(k); k = dp_from[k]; }
        opt_breaks.reverse();

        // Build LineInfo from optimal breaks, with post-DP hyphenation for loose lines
        let mut prev_bp_idx = 0;
        for (bi, &b) in opt_breaks.iter().enumerate() {
            let ls = bp[prev_bp_idx];
            let le = bp[b];
            let is_last = bi == opt_breaks.len() - 1;

            // Check if line is too loose and could benefit from hyphenation
            let trail_sp = if le > seg_start && words[le - 1].text == " " { words[le - 1].width } else { 0.0 };
            let lw: f32 = words[ls..le].iter().map(|w| w.width).sum::<f32>() - trail_sp;
            let max_w = if is_first_line && prev_bp_idx == 0 { text_width - first_line_used } else { text_width };
            let slack = max_w - lw;
            let mut hyphen_info: Option<(usize, usize)> = None;

            // If slack > 0.5em and not the last line, try to pull in part of the next word
            if !is_last && slack > font_size * 0.5 && le < seg_end {
                // Find the next non-space word after this line's break
                let mut next_wi = le;
                while next_wi < seg_end && words[next_wi].text == " " { next_wi += 1; }
                if next_wi < seg_end {
                    let next_word = &words[next_wi];
                    let wb = next_word.text.as_bytes();
                    if wb.len() >= 5 && wb.iter().all(|&b| b.is_ascii_alphanumeric()) {
                        let avail = slack - font_size * 0.1;
                        let fid = font::style_to_font_id(next_word.style);
                        let hyph_w = font::measure_text("-", fid, next_word.font_size);
                        // Estimate max chars that fit based on average char width
                        let avg_cw = next_word.width / wb.len() as f32;
                        let max_chars = ((avail - hyph_w) / avg_cw).min(wb.len() as f32) as usize;
                        if max_chars >= 2 {
                            if let Some(bp) = crate::hyphenate::best_break(wb, max_chars.min(wb.len() - 2)) {
                                // Verify the prefix actually fits
                                let prefix_w = font::measure_text(&next_word.text[..bp], fid, next_word.font_size);
                                if prefix_w + hyph_w <= avail {
                                    hyphen_info = Some((next_wi, bp));
                                }
                            }
                        }
                    }
                }
            }

            let (ma, mb) = compute_line_metrics(&words[ls..le], text_ascent, text_descent, font_size);
            lines.push(LineInfo { start: ls, end: le, max_above: ma, max_below: mb, hyphen: hyphen_info });
            is_first_line = false;
            prev_bp_idx = b;
        }
    }

    // Orphan/widow control (LaTeX \clubpenalty/\widowpenalty equivalent)
    if lines.len() >= 1 {
        let remaining_space = state.cached_max_y - state.current_y;
        let first_line_h = lines[0].max_above + lines[0].max_below;
        // Orphan: prevent single line stranded at page bottom
        if remaining_space >= first_line_h && remaining_space < first_line_h + step {
            state.ensure_space(remaining_space + 1.0); // force page break
        } else {
            // Widow: prevent single last line stranded at top of new page
            // Estimate how many lines fit on the current page
            let mut space_left = remaining_space;
            let mut lines_fitting = 0usize;
            for li in &lines {
                let line_h = li.max_above + li.max_below;
                if space_left >= line_h {
                    space_left -= line_h.max(step);
                    lines_fitting += 1;
                } else {
                    break;
                }
            }
            // If all but the last line fit, push the second-to-last to next page too
            if lines_fitting == lines.len() - 1 && lines.len() >= 3 {
                // Force page break before the penultimate line by reducing remaining space
                let penultimate_h = lines[lines_fitting - 1].max_above + lines[lines_fitting - 1].max_below;
                state.ensure_space(remaining_space - penultimate_h.max(step) + 1.0);
            }
        }
    }

    let total_lines = lines.len();
    let mut first_line = true;
    let mut prev_max_below = text_descent;
    let mut prev_hyphen: Option<(usize, usize)> = None; // (word_index, break_point) from previous line
    for (line_idx, line) in lines.iter().enumerate() {
        if !first_line {
            // TeX baselineskip: distance between baselines = max(baselineskip, prev_depth + cur_height + lineskiplimit)
            // Use step as the normal baseline distance, but allow expansion for large content
            let natural_gap = prev_max_below + line.max_above;
            let effective_step = if natural_gap > step {
                natural_gap  // Large content (math, big fonts) — expand to fit
            } else {
                step  // Normal text — use standard baselineskip
            };
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
        let align = state.alignment_mode;
        // Compute content width (needed for centering/right-align and justification)
        let mut content_w = 0.0f32;
        let mut space_count = 0u32;
        for wi in line.start..line.end {
            let word = &words[wi];
            if word.text == " " {
                space_count += 1;
                content_w += word.width;
            } else if let Some((prev_wi, prev_bp)) = prev_hyphen {
                if wi == prev_wi {
                    let fid = crate::font::style_to_font_id(word.style);
                    content_w += crate::font::measure_text(&word.text[prev_bp..], fid, word.font_size);
                } else {
                    content_w += word.width;
                }
            } else {
                content_w += word.width;
            }
        }
        if align == crate::document::AlignmentMode::Justify && !is_last_line {
            if space_count > 0 {
                let slack = available - content_w;
                // Justify if line is at least 45% full (TeX justifies aggressively)
                if slack > 0.0 && slack < available * 0.55 {
                    extra_per_space = slack / space_count as f32;
                    // TeX inter-word stretch: allow up to 0.35em for long lines with few spaces
                    // but prefer tighter spacing for better text color
                    let max_stretch = if space_count <= 3 {
                        font_size * 0.35  // Few spaces — need more stretch per space
                    } else {
                        font_size * 0.25  // Many spaces — keep tighter
                    };
                    extra_per_space = extra_per_space.min(max_stretch);
                } else if slack < 0.0 && slack > -font_size * 1.5 {
                    // TeX inter-word shrink ≈ 1.11pt for 10pt (0.111em)
                    extra_per_space = slack / space_count as f32;
                    extra_per_space = extra_per_space.max(-font_size * 0.12);
                }
            }
        }
        // Adjust starting x for non-justified alignment modes
        match align {
            crate::document::AlignmentMode::Center => {
                let slack = available - content_w;
                if slack > 0.0 { line_x += slack * 0.5; }
            }
            crate::document::AlignmentMode::FlushRight => {
                let slack = available - content_w;
                if slack > 0.0 { line_x += slack; }
            }
            _ => {}
        }

        for wi in line.start..line.end {
            let word = &words[wi];
            if word.text == " " { line_x += word.width + extra_per_space; continue; }
            if word.text.starts_with("\x01RULE:") {
                // Render filled rectangle for \rule{width}{height}
                let inner = &word.text[6..word.text.len()-1];
                let parts: Vec<&str> = inner.split(':').collect();
                let rule_w: f32 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(10.0);
                let rule_h: f32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(word.font_size * 0.8);
                let ry = state.current_y - rule_h + word.font_size * 0.15; // baseline-aligned
                state.emit_rect(line_x, ry, rule_w, rule_h, Some(word.color), None);
                line_x += rule_w;
                continue;
            }
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
            // Handle suffix of a word hyphenated on the previous line
            if let Some((prev_wi, prev_bp)) = prev_hyphen {
                if wi == prev_wi {
                    let suffix = &word.text[prev_bp..];
                    if !suffix.is_empty() {
                        state.current_x = line_x;
                        state.emit_text(suffix, word.font_size, word.style, word.color);
                        let fid = crate::font::style_to_font_id(word.style);
                        line_x += crate::font::measure_text(suffix, fid, word.font_size);
                    }
                    continue;
                }
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
                // TeX superscript rise ≈ 0.45 * x-height ≈ 0.2em (not 0.35em)
                state.current_y -= word.font_size * 0.25;
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
                let wfid = font::style_to_font_id(word.style);
                let st_y = state.current_y - font::font_info(wfid).x_height as f32 * word.font_size / 2000.0;
                let st_thickness = (word.font_size * 0.05).max(0.4);
                state.emit_line(line_x, st_y, line_x + word.width, st_y, st_thickness, word.color);
            }
            line_x += word.width;
        }

        prev_hyphen = line.hyphen;
        prev_max_below = line.max_below;
        first_line = false;
    }

    state.current_y += step;
    state.current_x = state.text_left();
    Ok(())
}
