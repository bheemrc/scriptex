/// List, description list, and bibliography layout

use crate::color::Color;
use crate::document::*;
use crate::typeset::FontStyle;
use crate::font::{self, FontId};
use super::state::LayoutState;
use super::spans::layout_rich_paragraph;

use anyhow::Result;

/// Check if list item content is effectively empty (only whitespace/invisible nodes)
fn is_item_empty(content: &[Node], source: &str) -> bool {
    content.iter().all(|n| match n {
        Node::Text(s) => s.trim().is_empty(),
        Node::TextRef(off, len) => source[*off as usize..(*off as usize + *len as usize)].trim().is_empty(),
        Node::Label(_) | Node::HSpace(_) | Node::VSpace(_) | Node::NonBreakingSpace => true,
        Node::Paragraph(children) | Node::Group(children) => is_item_empty(children, source),
        _ => false,
    })
}

pub(super) fn layout_list(
    items: &[ListItem], state: &mut LayoutState, doc: &Document,
    numbered: bool, source: &str,
) -> Result<()> {
    let saved_indent = state.indent;
    let saved_para_indent = state.paragraph_indent;
    let depth = state.list_depth;
    state.list_depth += 1;
    // LaTeX default indent: ~25pt for enumerate, ~18pt for itemize
    let list_indent = if numbered { 22.0 } else { 18.0 };
    state.set_indent(state.indent + list_indent);
    state.paragraph_indent = 0.0;
    // LaTeX \topsep: 8pt + \partopsep(2pt) for 10pt, scales with base size
    let base = state.base_font_size;
    let topsep = if depth == 0 { base * 0.8 } else { base * 0.3 };
    state.add_vertical_space(topsep);

    for (i, item) in items.iter().enumerate() {
        // Skip empty items (only whitespace or invisible nodes)
        if item.label.is_none() && is_item_empty(&item.content, source) { continue; }

        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        let fs = state.current_font_size;
        let marker_gap = fs * 0.3; // gap between marker and text
        if numbered {
            state.text_buf.clear();
            if let Some(ref custom_label) = item.label {
                // Use custom label from enumitem package
                for node in custom_label {
                    super::text::node_to_text(node, &mut state.text_buf, source);
                }
            } else {
                match depth {
                    0 => {
                        let mut ibuf = itoa::Buffer::new();
                        state.text_buf.push_str(ibuf.format(i + 1));
                        state.text_buf.push('.');
                    }
                    1 => {
                        state.text_buf.push('(');
                        state.text_buf.push((b'a' + (i as u8).min(25)) as char);
                        state.text_buf.push(')');
                    }
                    2 => {
                        let roman = to_roman_lower(i + 1);
                        state.text_buf.push_str(&roman);
                        state.text_buf.push('.');
                    }
                    _ => {
                        state.text_buf.push((b'A' + (i as u8).min(25)) as char);
                        state.text_buf.push('.');
                    }
                }
            }
            let marker: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            // Right-align numbered marker before text boundary
            let marker_w = font::measure_text(marker, FontId::TimesRoman, fs);
            state.current_x = state.text_left() - marker_gap - marker_w;
            state.emit_text(marker, fs, FontStyle::Regular, Color::BLACK);
        } else if let Some(ref custom_label) = item.label {
            // Custom label via \item[label] in itemize
            state.text_buf.clear();
            for node in custom_label {
                super::text::node_to_text(node, &mut state.text_buf, source);
            }
            let marker: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            let marker_w = font::measure_text(marker, FontId::TimesRoman, fs);
            state.current_x = state.text_left() - marker_gap - marker_w;
            state.emit_text(marker, fs, FontStyle::Regular, Color::BLACK);
        } else {
            // LaTeX itemize bullets: level 0 = filled circle (textbullet),
            // level 1 = en-dash, level 2 = filled small triangle, level 3+ = centered dot
            let bullet_base_x = state.text_left() - fs * 1.0; // bullet area starts ~1em before text
            let by = state.current_y - fs * 0.3; // vertical center of x-height (above baseline)
            match depth {
                0 => {
                    // Filled circle bullet (•) - radius ~1.5pt for 10pt font
                    let bullet_r = fs * 0.17;
                    let bx = bullet_base_x + bullet_r + fs * 0.3;
                    state.emit_rounded_rect(bx - bullet_r, by - bullet_r, bullet_r * 2.0, bullet_r * 2.0, Some(Color::BLACK), None, bullet_r);
                }
                1 => {
                    // En-dash (–) for second level
                    let dash_w = fs * 0.5;
                    let dash_x = bullet_base_x + fs * 0.2;
                    state.emit_line(dash_x, by, dash_x + dash_w, by, 0.5, Color::BLACK);
                }
                _ => {
                    // Small filled circle for deeper levels
                    let bullet_r = fs * 0.10;
                    let bx = bullet_base_x + bullet_r + fs * 0.3;
                    state.emit_rounded_rect(bx - bullet_r, by - bullet_r, bullet_r * 2.0, bullet_r * 2.0, Some(Color::BLACK), None, bullet_r);
                }
            }
        }
        state.current_x = state.text_left();

        let mut inline_end = item.content.len();
        for (j, node) in item.content.iter().enumerate() {
            if !super::is_inline_node(node) { inline_end = j; break; }
        }
        if inline_end > 0 {
            layout_rich_paragraph(&item.content[..inline_end], state, source, false)?;
        }
        if inline_end < item.content.len() {
            super::layout_nodes(&item.content[inline_end..], state, doc, source)?;
        }
        // LaTeX \itemsep + \parsep: ~4pt for 10pt at depth 0, ~2pt for nested
        if i + 1 < items.len() {
            let itemsep = if depth == 0 { base * 0.4 } else { base * 0.2 };
            state.add_vertical_space(itemsep);
        }
    }

    state.list_depth = depth;
    state.paragraph_indent = saved_para_indent;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    state.add_vertical_space(topsep);
    state.suppress_next_indent = true;
    Ok(())
}

fn to_roman_lower(mut n: usize) -> String {
    let mut s = String::new();
    for &(val, sym) in &[(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"),
        (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"),
        (5, "v"), (4, "iv"), (1, "i")] {
        while n >= val { s.push_str(sym); n -= val; }
    }
    s
}

pub(super) fn layout_description_list(
    items: &[ListItem], state: &mut LayoutState, doc: &Document, source: &str,
) -> Result<()> {
    // Match topsep for itemize lists
    let topsep = state.base_font_size * 0.8;
    state.add_vertical_space(topsep);
    for item in items {
        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        let saved_indent = state.indent;
        let mut inline_end = item.content.len();
        for (j, node) in item.content.iter().enumerate() {
            if !super::is_inline_node(node) { inline_end = j; break; }
        }

        // Combine bold label + inline content into one paragraph
        if let Some(label) = &item.label {
            let mut combined = Vec::with_capacity(label.len() + inline_end + 2);
            combined.push(Node::Bold(label.clone()));
            combined.push(Node::Text(" ".to_string()));
            combined.extend_from_slice(&item.content[..inline_end]);
            layout_rich_paragraph(&combined, state, source, false)?;
        } else if inline_end > 0 {
            layout_rich_paragraph(&item.content[..inline_end], state, source, false)?;
        }
        if inline_end < item.content.len() {
            state.set_indent(state.indent + state.base_font_size * 2.0);
            state.current_x = state.text_left();
            for node in &item.content[inline_end..] {
                if super::is_inline_node(node) {
                    layout_rich_paragraph(std::slice::from_ref(node), state, source, false)?;
                } else {
                    super::layout_node(node, state, doc, source)?;
                }
            }
            state.set_indent(saved_indent);
        }
        // LaTeX \itemsep ≈ 4pt for 10pt
        state.add_vertical_space(state.base_font_size * 0.4);
    }
    state.add_vertical_space(topsep);
    state.current_x = state.text_left();
    Ok(())
}

pub(super) fn layout_bibliography(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Space before bibliography: ~2 baselineskips
    let base = state.base_font_size;
    state.add_vertical_space(base * 2.4);
    state.ensure_space(base * 4.0);
    if state.is_amsart {
        let heading = "References";
        let heading_size = state.current_font_size * 1.2;
        let heading_w = font::measure_text(heading, FontId::TimesRoman, heading_size);
        state.current_x = state.text_left() + (state.text_width() - heading_w) * 0.5;
        state.emit_text(heading, heading_size, FontStyle::SmallCaps, Color::BLACK);
        state.current_y += heading_size * 1.2 + base * 0.8;
    } else {
        let heading = "References";
        let heading_size = state.current_font_size * 1.44;
        state.current_x = state.text_left();
        state.emit_text(heading, heading_size, FontStyle::Bold, Color::BLACK);
        state.current_y += heading_size * 1.2 + base * 1.0;
    }

    let mut bib_num = 0u32;
    let mut entry_nodes: Vec<&Node> = Vec::new();
    let indent = if state.is_amsart { base * 2.0 } else { base * 2.4 };

    for node in nodes {
        if let Node::BibItem(_key) = node {
            if bib_num > 0 && !entry_nodes.is_empty() {
                layout_bib_entry(bib_num, &entry_nodes, state, doc, source, indent)?;
                entry_nodes.clear();
            }
            bib_num += 1;
        } else if bib_num > 0 {
            entry_nodes.push(node);
        }
    }
    if bib_num > 0 && !entry_nodes.is_empty() {
        layout_bib_entry(bib_num, &entry_nodes, state, doc, source, indent)?;
    }
    state.add_vertical_space(base * 0.8);
    Ok(())
}

fn layout_bib_entry(num: u32, nodes: &[&Node], state: &mut LayoutState, doc: &Document, source: &str, indent: f32) -> Result<()> {
    state.ensure_space(state.current_font_size * 1.2);
    let font_size = if state.is_amsart { state.current_font_size * 0.85 } else { state.current_font_size * 0.9 };

    let mut ibuf = itoa::Buffer::new();
    let marker = format!("[{}]", ibuf.format(num));
    let marker_w = font::measure_text(&marker, FontId::TimesRoman, font_size);
    let marker_x = state.text_left() + indent - marker_w - 4.0;
    state.current_x = marker_x.max(state.text_left());
    state.emit_text(&marker, font_size, FontStyle::Regular, Color::BLACK);

    let saved_indent = state.indent;
    let saved_font_size = state.current_font_size;
    state.set_indent(state.text_left() + indent);
    state.current_x = state.text_left() + indent;
    state.current_font_size = font_size;

    let para_nodes = merge_adjacent_text(nodes, source);
    let para = Node::Paragraph(para_nodes);
    super::layout_node(&para, state, doc, source)?;

    state.current_y += font_size * 0.3;
    state.current_font_size = saved_font_size;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    Ok(())
}

fn merge_adjacent_text(nodes: &[&Node], source: &str) -> Vec<Node> {
    let mut result: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut text_buf = String::new();
    for node in nodes {
        match node {
            Node::Text(s) => text_buf.push_str(s),
            Node::TextRef(offset, len) => {
                text_buf.push_str(&source[*offset as usize..(*offset as usize + *len as usize)]);
            }
            Node::Group(children) if children.len() == 1 => {
                if let Some(text) = extract_simple_text(&children[0], source) {
                    text_buf.push_str(&text);
                } else {
                    flush_text_buf(&mut text_buf, &mut result);
                    result.push((*node).clone());
                }
            }
            Node::NonBreakingSpace => text_buf.push(' '),
            Node::EnDash => text_buf.push('\u{2013}'),
            Node::EmDash => text_buf.push('\u{2014}'),
            Node::Ellipsis => text_buf.push_str("\u{2026}"),
            Node::Ampersand => text_buf.push('&'),
            Node::Percent => text_buf.push('%'),
            Node::Dollar => text_buf.push('$'),
            Node::Hash => text_buf.push('#'),
            Node::Underscore => text_buf.push('_'),
            Node::Tilde => text_buf.push('~'),
            Node::LeftQuote => text_buf.push('\u{2018}'),
            Node::RightQuote => text_buf.push('\u{2019}'),
            Node::LeftDoubleQuote => text_buf.push('\u{201C}'),
            Node::RightDoubleQuote => text_buf.push('\u{201D}'),
            _ => {
                flush_text_buf(&mut text_buf, &mut result);
                result.push((*node).clone());
            }
        }
    }
    flush_text_buf(&mut text_buf, &mut result);
    result
}

fn flush_text_buf(buf: &mut String, result: &mut Vec<Node>) {
    if !buf.is_empty() { result.push(Node::Text(std::mem::take(buf))); }
}

fn extract_simple_text(node: &Node, source: &str) -> Option<String> {
    match node {
        Node::Text(s) => Some(s.clone()),
        Node::TextRef(offset, len) => Some(source[*offset as usize..(*offset as usize + *len as usize)].to_string()),
        _ => None,
    }
}
