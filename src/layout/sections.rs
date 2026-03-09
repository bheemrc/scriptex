/// Section headings and table of contents layout

use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle, wrap_text};
use crate::font::{self, FontId};
use super::state::LayoutState;
use super::types::*;
use super::prescans::TocFixup;
use super::text::{node_to_text, math_to_text_buf};

use anyhow::Result;

pub(super) fn layout_section(
    level: SectionLevel, title: &[Node], numbered: bool,
    state: &mut LayoutState, _doc: &Document, source: &str,
) -> Result<()> {
    // LaTeX vertical space collapsing: if previous element added structural space,
    // take the max of previous and current spacing rather than accumulating both
    let space_before = level.spacing_before_scaled(state.base_font_size);
    if state.last_structural_vspace > 0.0 {
        let extra = (space_before - state.last_structural_vspace).max(0.0);
        state.add_vertical_space(extra);
    } else {
        state.add_vertical_space(space_before);
    }
    state.last_structural_vspace = 0.0;

    let (font_size, style) = if state.is_amsart {
        match level {
            SectionLevel::Section => (state.base_font_size, FontStyle::SmallCaps),
            SectionLevel::Subsection => (state.base_font_size, FontStyle::Bold),
            SectionLevel::Subsubsection => (state.base_font_size, FontStyle::Italic),
            _ => (level.font_size(state.base_font_size), FontStyle::Bold),
        }
    } else {
        (level.font_size(state.base_font_size), FontStyle::Bold)
    };
    let line_height = font_size * super::state::baselineskip_factor(state.base_font_size);
    // Ensure heading + spacing + at least 3 lines of body text fit on current page
    // This prevents orphaned headings near page bottoms (LaTeX \clubpenalty equivalent)
    let min_body_lines = match level {
        SectionLevel::Section | SectionLevel::Chapter | SectionLevel::Part => 2.5,
        SectionLevel::Subsection => 2.0,
        SectionLevel::Subsubsection => 1.5,
        _ => 1.0, // paragraph, subparagraph
    };
    state.ensure_space(line_height + level.spacing_after_scaled(state.base_font_size) + state.cached_line_height * min_body_lines);

    state.text_buf.clear();
    if numbered {
        let idx = (level.depth() + 1).max(0) as usize;
        if idx < state.section_counters.len() {
            state.section_counters[idx] += 1;
            for i in (idx + 1)..state.section_counters.len() { state.section_counters[i] = 0; }
        }
        let mut ibuf = itoa::Buffer::new();
        match level {
            SectionLevel::Part => {
                state.text_buf.push_str("Part ");
                state.text_buf.push_str(ibuf.format(state.section_counters[0]));
                state.text_buf.push(' ');
            }
            SectionLevel::Chapter => {
                state.text_buf.push_str(ibuf.format(state.section_counters[1]));
                state.text_buf.push(' ');
            }
            SectionLevel::Section => {
                state.current_section_num = state.section_counters[2];
                state.theorem_counters.clear();
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                state.text_buf.push_str(if state.is_amsart { ". " } else { " " });
            }
            SectionLevel::Subsection => {
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                state.text_buf.push_str(if state.is_amsart { ". " } else { " " });
            }
            SectionLevel::Subsubsection => {
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[4]));
                state.text_buf.push_str(if state.is_amsart { ". " } else { " " });
            }
            _ => {}
        }
    }

    let has_inline_math = title.iter().any(|n| matches!(n, Node::InlineMath(_)));
    let title_start = state.text_buf.len();
    for node in title { node_to_text(node, &mut state.text_buf, source); }
    // \paragraph and \subparagraph: append period after title (LaTeX convention)
    if matches!(level, SectionLevel::Paragraph | SectionLevel::Subparagraph) {
        let title_text = state.text_buf[title_start..].trim_end();
        if !title_text.ends_with('.') && !title_text.ends_with('!') && !title_text.ends_with('?') {
            state.text_buf.push('.');
        }
    }
    if state.is_amsart && matches!(level, SectionLevel::Section) {
        let title_text = state.text_buf[title_start..].to_uppercase();
        state.text_buf.truncate(title_start);
        state.text_buf.push_str(&title_text);
    }

    if level.depth() <= 3 && state.outlines.len() < 5000 {
        state.outlines.push(OutlineEntry {
            title: state.text_buf.clone(), page: state.page_bounds.len() as u32,
            y: state.current_y, level: level.depth(),
        });
    }
    if matches!(level, SectionLevel::Section) {
        state.current_section_title.clear();
        state.current_section_title.push_str(&state.text_buf);
    }
    if numbered && (state.toc_section_idx as usize) < state.toc_entries.len() {
        let idx = state.toc_section_idx as usize;
        state.toc_entries[idx].page = state.page_number;
        state.toc_entries[idx].dest_page = state.page_bounds.len() as u32;
        state.toc_entries[idx].dest_y = state.current_y;
        state.toc_section_idx += 1;
    }

    state.current_x = state.text_left();
    let run_in = matches!(level, SectionLevel::Paragraph | SectionLevel::Subparagraph);
    let centered = state.is_amsart && matches!(level, SectionLevel::Section);

    if has_inline_math && !run_in {
        layout_section_with_math(title, title_start, style, font_size, line_height, centered, state, source);
    } else {
        let full_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
        let measure_font_id = match style {
            FontStyle::Bold | FontStyle::SmallCaps => FontId::TimesBold,
            FontStyle::Italic => FontId::TimesItalic,
            _ => FontId::TimesRoman,
        };
        let measured_width = font::measure_text(full_text, measure_font_id, font_size);

        if run_in {
            let text_w = font::measure_text(full_text, FontId::TimesBold, font_size);
            state.emit_text(full_text, font_size, style, Color::BLACK);
            state.current_x += text_w + font_size * 0.5;
        } else if centered {
            let font_id = match style {
                FontStyle::SmallCaps | FontStyle::Bold => FontId::TimesBold,
                FontStyle::Italic => FontId::TimesItalic,
                _ => FontId::TimesRoman,
            };
            let text_w = font::measure_text(full_text, font_id, font_size);
            let cx = state.text_left() + (state.text_width() - text_w) / 2.0;
            state.current_x = cx;
            state.emit_text(full_text, font_size, style, Color::BLACK);
            state.current_y += line_height;
            state.current_x = state.text_left();
        } else if measured_width <= state.text_width() {
            state.emit_text(full_text, font_size, style, Color::BLACK);
            state.current_y += line_height;
            state.current_x = state.text_left();
        } else {
            let metrics = FontMetrics::new(font_size, style);
            let lines = wrap_text(full_text, &metrics, state.text_width());
            for line in &lines {
                if centered {
                    let font_id = match style {
                        FontStyle::SmallCaps | FontStyle::Bold => FontId::TimesBold,
                        _ => FontId::TimesRoman,
                    };
                    let tw = font::measure_text(line, font_id, font_size);
                    state.current_x = state.text_left() + (state.text_width() - tw) / 2.0;
                }
                state.emit_text(line, font_size, style, Color::BLACK);
                state.current_y += line_height;
                state.current_x = state.text_left();
            }
        }
    }

    if !run_in {
        let sa = level.spacing_after_scaled(state.base_font_size);
        state.add_vertical_space(sa);
        state.last_structural_vspace = sa;
        state.current_x = state.text_left();
    }
    state.suppress_next_indent = true;
    Ok(())
}

fn layout_section_with_math(
    title: &[Node], title_start: usize, style: FontStyle, font_size: f32,
    line_height: f32, centered: bool, state: &mut LayoutState, source: &str,
) {
    let base_font_id = match style {
        FontStyle::SmallCaps | FontStyle::Bold => FontId::TimesBold,
        FontStyle::Italic => FontId::TimesItalic,
        _ => FontId::TimesRoman,
    };
    struct Seg { text: String, sym: bool }
    let mut segs: Vec<Seg> = Vec::new();
    let prefix = state.text_buf[..title_start].to_string();
    if !prefix.is_empty() { segs.push(Seg { text: prefix, sym: false }); }

    for node in title {
        match node {
            Node::InlineMath(math_nodes) => {
                for mn in math_nodes.iter() {
                    match mn {
                        MathNode::Symbol(s) => {
                            if let Some(first_char) = s.chars().next() {
                                if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                                    segs.push(Seg { text: String::from(byte as char), sym: true });
                                }
                            }
                        }
                        MathNode::Variable(ch) => {
                            let mut t = String::new(); t.push(*ch);
                            segs.push(Seg { text: t, sym: false });
                        }
                        _ => {
                            let mut t = String::new();
                            math_to_text_buf(std::slice::from_ref(mn), &mut t);
                            if !t.is_empty() { segs.push(Seg { text: t, sym: false }); }
                        }
                    }
                }
            }
            _ => {
                let mut t = String::new();
                node_to_text(node, &mut t, source);
                if !t.is_empty() { segs.push(Seg { text: t, sym: false }); }
            }
        }
    }

    let total_w: f32 = segs.iter().map(|s| {
        if s.sym { font::measure_text(&s.text, FontId::Symbol, font_size) }
        else { font::measure_text(&s.text, base_font_id, font_size) }
    }).sum();

    if centered {
        state.current_x = state.text_left() + (state.text_width() - total_w) / 2.0;
    }
    for seg in &segs {
        let (seg_style, seg_font) = if seg.sym { (FontStyle::Symbol, FontId::Symbol) } else { (style, base_font_id) };
        state.emit_text(&seg.text, font_size, seg_style, Color::BLACK);
        state.current_x += font::measure_text(&seg.text, seg_font, font_size);
    }
    state.current_y += line_height;
    state.current_x = state.text_left();
}

pub(super) fn layout_table_of_contents(state: &mut LayoutState) -> Result<()> {
    let base = state.base_font_size;
    state.add_vertical_space(base * 1.0);
    let heading_size = base * 1.44;
    state.ensure_space(heading_size * 1.2);
    state.current_x = state.text_left();
    state.emit_text("Contents", heading_size, FontStyle::Bold, Color::BLACK);
    state.current_y += heading_size * 1.2 + base * 0.6;
    state.emit_line(state.text_left(), state.current_y, state.text_left() + state.text_width(), state.current_y, 0.5, Color::BLACK);
    state.current_y += base * 0.8;

    let entries = std::mem::take(&mut state.toc_entries);
    // Dot leaders: use evenly spaced dots like LaTeX
    let single_dot_w = font::measure_text(".", FontId::TimesRoman, base * 0.9);
    let dot_spacing = single_dot_w * 3.5; // TeX uses ~4.5pt spacing between dot centers
    let page_num_width = font::measure_text("000", FontId::TimesRoman, base);

    for (toc_idx, entry) in entries.iter().enumerate() {
        let depth = entry.level.depth();
        let indent = match depth { d if d <= 1 => 0.0, 2 => base * 1.5, 3 => base * 3.0, _ => base * 4.5 };
        let font_size = match depth { d if d <= 1 => base, 2 => base * 0.95, _ => base * 0.9 };
        let style = if depth <= 1 { FontStyle::Bold } else { FontStyle::Regular };
        let line_height = font_size * 1.4;

        state.ensure_space(line_height);
        let x = state.text_left() + indent;
        let right_edge = state.text_left() + state.text_width();

        state.text_buf.clear();
        if !entry.number.is_empty() {
            state.text_buf.push_str(&entry.number);
            state.text_buf.push(' ');
        }
        state.text_buf.push_str(&entry.title);

        let text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
        let measure_font = if depth <= 1 { FontId::TimesBold } else { FontId::TimesRoman };
        let text_w = font::measure_text(text, measure_font, font_size);
        let available = state.text_width() - indent;

        state.current_x = x;
        if text_w <= available - page_num_width - 10.0 {
            state.emit_text(text, font_size, style, Color::BLACK);
            let after_text_x = x + text_w + base * 0.4;
            let dot_end = right_edge - page_num_width - base * 0.4;
            // Align dots to a grid so they line up across TOC entries (LaTeX convention)
            let grid_start = state.text_left(); // align to left margin
            let first_dot_x = {
                let raw = after_text_x + dot_spacing * 0.5;
                let n = ((raw - grid_start) / dot_spacing).ceil();
                grid_start + n * dot_spacing
            };
            if dot_end > first_dot_x {
                let mut dx = first_dot_x;
                while dx < dot_end {
                    state.current_x = dx;
                    state.emit_text(".", base * 0.9, FontStyle::Regular, Color::BLACK);
                    dx += dot_spacing;
                }
            }
            let page_x = right_edge - page_num_width;
            state.current_x = page_x;
            let text_offset = state.all_text.len() as u32;
            let elem_idx = state.all_elements.len() as u32;
            state.emit_text("   ", font_size, FontStyle::Regular, Color::BLACK);
            state.toc_fixups.push(TocFixup { elem_idx, text_offset, toc_idx: toc_idx as u32 });
        } else {
            let metrics = FontMetrics::new(font_size, style);
            let truncated_avail = available - page_num_width - 10.0;
            if truncated_avail > 0.0 {
                let lines = wrap_text(text, &metrics, truncated_avail);
                if let Some(first) = lines.first() { state.emit_text(first, font_size, style, Color::BLACK); }
            }
        }

        // Create clickable link for TOC entry pointing to section destination
        if entry.dest_page > 0 || entry.dest_y > 0.0 {
            let link_width = (right_edge - x).min(state.text_width());
            state.links.push(LinkAnnotation {
                page: state.page_bounds.len() as u32,
                x, y: state.current_y - font_size * 0.8,
                width: link_width, height: font_size * 1.2,
                url: String::new(),
                dest_page: Some(entry.dest_page),
                dest_y: entry.dest_y,
            });
        }

        state.current_y += line_height;
        state.current_x = state.text_left();
        if depth <= 1 { state.current_y += base * 0.2; }
    }

    state.toc_entries = entries;
    state.add_vertical_space(base * 1.6);
    state.emit_line(state.text_left(), state.current_y, state.text_left() + state.text_width(), state.current_y, 0.3, Color::LIGHT_GRAY);
    state.add_vertical_space(base * 1.2);
    Ok(())
}
