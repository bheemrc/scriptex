/// Theorem, proof, verbatim, code block, centered, and flush-right layout

use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};
use super::state::LayoutState;
use super::text::node_to_text;
use super::types::*;

use anyhow::Result;

pub(super) fn layout_theorem(thm: &TheoremData, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(10.0);

    let (display_title, is_numbered, thm_style) = if let Some(def) = doc.preamble.theorem_defs.iter()
        .find(|d| d.env_name == thm.env_name) {
        (def.display_title.clone(), def.numbered, def.style)
    } else {
        let mut title = thm.env_name.clone();
        if let Some(first) = title.get_mut(0..1) { first.make_ascii_uppercase(); }
        let style = match thm.env_name.as_str() {
            "definition" | "example" | "notation" | "convention" | "assumption" => TheoremStyle::Definition,
            "remark" | "note" | "observation" => TheoremStyle::Remark,
            _ => TheoremStyle::Plain,
        };
        (title, false, style)
    };

    let mut header = display_title.clone();
    if is_numbered {
        let counter_name = if let Some(def) = doc.preamble.theorem_defs.iter()
            .find(|d| d.env_name == thm.env_name) {
            def.counter.clone().unwrap_or_else(|| thm.env_name.clone())
        } else { thm.env_name.clone() };
        let count = state.theorem_counters.entry(counter_name).or_insert(0);
        *count += 1;
        let num = *count;
        if state.current_section_num > 0 {
            header.push_str(&format!(" {}.{}", state.current_section_num, num));
        } else {
            header.push_str(&format!(" {}", num));
        }
    }
    if let Some(ref name) = thm.optional_name {
        header.push_str(&format!(" ({})", name));
    }
    header.push('.');

    let label_style = match thm_style {
        TheoremStyle::Plain | TheoremStyle::Definition => FontStyle::Bold,
        TheoremStyle::Remark => FontStyle::Italic,
    };

    let font_size = state.current_font_size;
    state.ensure_space(font_size * 1.2);
    state.current_x = state.text_left();
    let header_w = font::measure_text(&header, font::style_to_font_id(label_style), font_size);
    state.emit_text(&header, font_size, label_style, Color::BLACK);
    // Continue body on same line after header
    state.current_x = state.text_left() + header_w + font_size * 0.25;
    state.suppress_next_indent = true;

    let saved_style = state.current_font_style;
    if thm_style == TheoremStyle::Plain { state.current_font_style = FontStyle::Italic; }
    super::layout_nodes(&thm.body, state, doc, source)?;
    state.current_font_style = saved_style;

    state.add_vertical_space(10.0);
    Ok(())
}

pub(super) fn layout_proof(header: Option<&str>, content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(8.0);
    let font_size = state.current_font_size;
    let header_text = match header {
        Some(h) => format!("{}.", h),
        None => "Proof.".to_string(),
    };
    state.ensure_space(font_size * 1.2);
    state.current_x = state.text_left();
    let header_w = font::measure_text(&header_text, FontId::TimesItalic, font_size);
    state.emit_text(&header_text, font_size, FontStyle::Italic, Color::BLACK);
    // Continue body on same line after header
    state.current_x = state.text_left() + header_w + font_size * 0.25;
    state.suppress_next_indent = true;

    super::layout_nodes(content, state, doc, source)?;

    // QED square — filled rectangle flush right
    let sq = font_size * 0.45;
    let qed_x = state.text_left() + state.text_width() - sq;
    let qed_y = state.current_y - sq;
    state.emit_rect(qed_x, qed_y, sq, sq, Some(Color::BLACK), None);
    state.add_vertical_space(8.0);
    Ok(())
}

pub(super) fn layout_algorithm(
    caption: &Option<String>,
    label: &Option<String>,
    lines: &[AlgoLine],
    line_numbered: bool,
    state: &mut LayoutState,
    doc: &Document,
) -> Result<()> {
    state.add_vertical_space(12.0);
    let font_size = state.current_font_size;
    let line_height = font_size * 1.4;
    let indent_unit = font_size * 1.2;
    let left = state.text_left();
    let width = state.text_width();

    // Line number gutter width (enough for "00:")
    let num_gutter = if line_numbered { font_size * 2.0 } else { 0.0 };

    // Algorithm counter + caption
    state.figure_counter += 1;
    let algo_num = state.figure_counter;

    // Top rule
    state.ensure_space(line_height * 2.0);
    state.emit_line(left, state.current_y, left + width, state.current_y, 0.8, Color::BLACK);
    state.current_y += line_height * 0.3;

    // Caption header: "Algorithm N: caption text"
    if let Some(cap) = caption {
        let header = format!("Algorithm {}: {}", algo_num, cap);
        state.current_x = left;
        state.emit_text(&header, font_size, FontStyle::Bold, Color::BLACK);
        state.current_y += line_height;
    } else {
        let header = format!("Algorithm {}", algo_num);
        state.current_x = left;
        state.emit_text(&header, font_size, FontStyle::Bold, Color::BLACK);
        state.current_y += line_height;
    }

    // Store label
    if let Some(ref lbl) = label {
        state.label_map.insert(lbl.clone(), algo_num.to_string());
        state.label_types.insert(lbl.clone(), "algorithm".to_string());
    }

    // Mid rule
    state.emit_line(left, state.current_y - line_height * 0.3, left + width, state.current_y - line_height * 0.3, 0.4, Color::BLACK);

    // Render algorithm lines
    let mut line_num: u32 = 0;
    let num_font_size = font_size * 0.85;
    let mut ibuf = itoa::Buffer::new();

    for line in lines {
        state.ensure_space(line_height);
        line_num += 1;

        let x = left + num_gutter + indent_unit * line.indent as f32 + 4.0;

        // Emit line number in the gutter
        if line_numbered {
            let num_str = ibuf.format(line_num);
            let num_w = font::measure_text(num_str, FontId::Helvetica, num_font_size);
            // Right-align in gutter
            state.current_x = left + num_gutter - num_w - 4.0;
            state.emit_text(num_str, num_font_size, FontStyle::Regular, Color::GRAY);
        }

        state.current_x = x;

        for token in &line.content {
            match token {
                AlgoToken::Keyword(kw) => {
                    let w = font::measure_text(kw, FontId::HelveticaBold, font_size);
                    state.emit_text(kw, font_size, FontStyle::Bold, Color::BLACK);
                    state.current_x += w;
                }
                AlgoToken::Text(t) => {
                    let w = font::measure_text(t, FontId::Helvetica, font_size);
                    state.emit_text(t, font_size, FontStyle::Regular, Color::BLACK);
                    state.current_x += w;
                }
                AlgoToken::Math(math) => {
                    let math_box = crate::math_layout::layout_math(math, font_size);
                    super::math::emit_math_elements(&math_box, state.current_x, state.current_y, state);
                    state.current_x += math_box.width;
                }
            }
        }
        state.current_y += line_height;
    }

    // Bottom rule
    state.emit_line(left, state.current_y - line_height * 0.5, left + width, state.current_y - line_height * 0.5, 0.8, Color::BLACK);
    state.add_vertical_space(12.0);
    Ok(())
}

pub(super) fn layout_verbatim(text: &str, state: &mut LayoutState) -> Result<()> {
    layout_code_block(text, None, state)
}

pub(super) fn layout_code_block(text: &str, language: Option<&str>, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(6.0);
    let font_size = state.base_font_size * 0.85;
    let metrics = FontMetrics::new(font_size, FontStyle::Monospace);
    let text_lines: Vec<&str> = text.lines().collect();
    let total_height = text_lines.len() as f32 * metrics.line_height() + 12.0;
    state.ensure_space(total_height);

    state.emit_rect(
        state.text_left() - 4.0, state.current_y - 4.0,
        state.text_width() + 8.0, total_height,
        Some(Color::rgb(0.96, 0.96, 0.96)), Some(Color::LIGHT_GRAY),
    );

    if let Some(lang) = language {
        let highlighted = crate::highlight::get_highlighter().highlight(text, lang);
        if !highlighted.is_empty() {
            for line_spans in &highlighted {
                state.current_x = state.text_left() + 4.0;
                for span in line_spans {
                    let style = if span.bold { FontStyle::Bold } else { FontStyle::Monospace };
                    let color = span.color;
                    let w = font::measure_text(&span.text, FontId::Courier, font_size);
                    let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                    state.all_text.push_str(&span.text);
                    state.all_elements.push(PageElement::Text {
                        x: state.current_x, y: state.current_y, text_offset: offset,
                        text_len: span.text.len().min(65535) as u16,
                        font_size_100: (font_size * 100.0) as u16,
                        font_style: style, color, word_spacing_50: 0,
                    });
                    state.current_x += w;
                }
                state.current_y += metrics.line_height();
            }
            state.add_vertical_space(10.0);
            state.current_x = state.text_left();
            return Ok(());
        }
    }

    for line in text_lines {
        state.current_x = state.text_left() + 4.0;
        state.emit_text(line, font_size, FontStyle::Monospace, Color::DARK_GRAY);
        state.current_y += metrics.line_height();
    }
    state.add_vertical_space(10.0);
    state.current_x = state.text_left();
    Ok(())
}

pub(super) fn layout_centered(content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    for node in content {
        match node {
            Node::Paragraph(children) => {
                state.text_buf.clear();
                for child in children { node_to_text(child, &mut state.text_buf, source); }
                let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            Node::TextParagraph(offset, len) | Node::TextRef(offset, len) => {
                let text = source[*offset as usize..(*offset as usize + *len as usize)].trim();
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            Node::Text(s) => {
                let text = s.trim();
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            _ => { super::layout_node(node, state, doc, source)?; }
        }
    }
    Ok(())
}

fn layout_centered_text(text: &str, state: &mut LayoutState) -> Result<()> {
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let line_h = font_size * 1.2;
    let fid = font::style_to_font_id(font_style);
    let space_width = font::measure_text(" ", fid, font_size);
    let para_width = state.text_width();

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut current_width: f32 = 0.0;
    while pos < len && bytes[pos] <= b' ' { pos += 1; }
    let mut line_start = pos;

    while pos < len {
        let word_start = pos;
        pos = match memchr::memchr2(b' ', b'\n', &bytes[pos..]) { Some(o) => pos + o, None => len };
        let word_width = font::measure_text(&text[word_start..pos], fid, font_size);
        if current_width > 0.0 && current_width + space_width + word_width > para_width {
            let line = text[line_start..word_start].trim_end();
            if !line.is_empty() {
                state.ensure_space(line_h);
                let tw = font::measure_text(line, fid, font_size);
                state.current_x = state.text_left() + (para_width - tw) / 2.0;
                state.emit_text(line, font_size, font_style, color);
                state.current_y += line_h * state.line_spacing;
            }
            line_start = word_start;
            current_width = word_width;
        } else {
            if current_width > 0.0 { current_width += space_width; }
            current_width += word_width;
        }
        if pos < len { pos += 1; while pos < len && bytes[pos] <= b' ' { pos += 1; } }
    }
    let remaining = text[line_start..].trim_end();
    if !remaining.is_empty() {
        state.ensure_space(line_h);
        let tw = font::measure_text(remaining, fid, font_size);
        state.current_x = state.text_left() + (para_width - tw) / 2.0;
        state.emit_text(remaining, font_size, font_style, color);
        state.current_y += line_h * state.line_spacing;
    }
    state.add_vertical_space(font_size * 0.2);
    Ok(())
}

pub(super) fn layout_flush_right(content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    for node in content {
        match node {
            Node::Paragraph(children) => {
                state.text_buf.clear();
                for child in children { node_to_text(child, &mut state.text_buf, source); }
                let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            Node::TextParagraph(offset, len) | Node::TextRef(offset, len) => {
                let text = source[*offset as usize..(*offset as usize + *len as usize)].trim();
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            Node::Text(s) => {
                let text = s.trim();
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            _ => { super::layout_node(node, state, doc, source)?; }
        }
    }
    Ok(())
}

fn layout_right_aligned_text(text: &str, state: &mut LayoutState) -> Result<()> {
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let line_h = font_size * 1.2;
    let fid = font::style_to_font_id(font_style);
    let space_width = font::measure_text(" ", fid, font_size);
    let para_width = state.text_width();

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut current_width: f32 = 0.0;
    while pos < len && bytes[pos] <= b' ' { pos += 1; }
    let mut line_start = pos;

    while pos < len {
        let word_start = pos;
        pos = match memchr::memchr2(b' ', b'\n', &bytes[pos..]) { Some(o) => pos + o, None => len };
        let word_width = font::measure_text(&text[word_start..pos], fid, font_size);
        if current_width > 0.0 && current_width + space_width + word_width > para_width {
            let line = text[line_start..word_start].trim_end();
            if !line.is_empty() {
                state.ensure_space(line_h);
                let tw = font::measure_text(line, fid, font_size);
                state.current_x = state.text_left() + para_width - tw;
                state.emit_text(line, font_size, font_style, color);
                state.current_y += line_h * state.line_spacing;
            }
            line_start = word_start;
            current_width = word_width;
        } else {
            if current_width > 0.0 { current_width += space_width; }
            current_width += word_width;
        }
        if pos < len { pos += 1; while pos < len && bytes[pos] <= b' ' { pos += 1; } }
    }
    let remaining = text[line_start..].trim_end();
    if !remaining.is_empty() {
        state.ensure_space(line_h);
        let tw = font::measure_text(remaining, fid, font_size);
        state.current_x = state.text_left() + para_width - tw;
        state.emit_text(remaining, font_size, font_style, color);
        state.current_y += line_h * state.line_spacing;
    }
    state.add_vertical_space(font_size * 0.2);
    Ok(())
}
