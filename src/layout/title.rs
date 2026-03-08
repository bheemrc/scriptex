/// Title page layout (standard article and amsart)

use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle, wrap_text};
use crate::font::{self, FontId};
use super::state::LayoutState;

use anyhow::Result;

pub(super) fn layout_title(state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    if state.is_amsart {
        return layout_title_amsart(state, doc, source);
    }

    state.add_vertical_space(40.0);

    if let Some(title) = &doc.preamble.title {
        let size = state.base_font_size * 1.728;
        let metrics = FontMetrics::new(size, FontStyle::Bold);
        let segments: Vec<&str> = title.split("\\\\").collect();
        for segment in &segments {
            let segment = segment.trim();
            if segment.is_empty() { continue; }
            let lines = wrap_text(segment, &metrics, state.text_width());
            for line in &lines {
                let tw = metrics.measure_text(line);
                let cx = state.text_left() + (state.text_width() - tw) / 2.0;
                state.ensure_space(metrics.line_height());
                state.current_x = cx;
                state.emit_text(line, size, FontStyle::Bold, Color::BLACK);
                state.current_y += metrics.line_height();
            }
        }
        state.add_vertical_space(12.0);
    }

    if let Some(author) = &doc.preamble.author {
        let size = state.base_font_size * 1.2;
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        let para_width = state.text_width();
        // Split by \and or , for multi-author, then word-wrap each line
        let parts: Vec<&str> = if author.contains("\\and") {
            author.split("\\and").map(|s| s.trim()).collect()
        } else {
            vec![author.as_str()]
        };
        for part in &parts {
            let tw = metrics.measure_text(part);
            if tw <= para_width {
                let cx = state.text_left() + (para_width - tw) / 2.0;
                state.ensure_space(metrics.line_height());
                state.current_x = cx;
                state.emit_text(part, size, FontStyle::Regular, Color::BLACK);
                state.current_y += metrics.line_height();
            } else {
                // Word-wrap long author names
                super::environments::layout_centered_text(part, state)?;
            }
        }
        state.add_vertical_space(6.0);
    }

    if let Some(date) = &doc.preamble.date {
        let size = state.base_font_size;
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        let tw = metrics.measure_text(date);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.ensure_space(metrics.line_height());
        state.current_x = cx;
        state.emit_text(date, size, FontStyle::Regular, Color::DARK_GRAY);
        state.current_y += metrics.line_height();
    }

    state.add_vertical_space(30.0);
    Ok(())
}

fn layout_title_amsart(state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(60.0);

    if let Some(title) = &doc.preamble.title {
        let size = state.base_font_size * 1.2;
        let upper_title = title.to_uppercase();
        let metrics = FontMetrics::new(size, FontStyle::Bold);
        let segments: Vec<&str> = upper_title.split("\\\\").collect();
        for segment in &segments {
            let segment = segment.trim();
            if segment.is_empty() { continue; }
            let lines = wrap_text(segment, &metrics, state.text_width());
            for line in &lines {
                let tw = metrics.measure_text(line);
                let cx = state.text_left() + (state.text_width() - tw) / 2.0;
                state.ensure_space(metrics.line_height());
                state.current_x = cx;
                state.emit_text(line, size, FontStyle::Bold, Color::BLACK);
                state.current_y += metrics.line_height();
            }
        }
        state.add_vertical_space(16.0);
    }

    if let Some(author) = &doc.preamble.author {
        let size = state.base_font_size;
        let upper_author = author.to_uppercase();
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        let parts: Vec<&str> = upper_author.split("\\AND").collect();
        for part in &parts {
            let part = part.trim();
            if part.is_empty() { continue; }
            let tw = metrics.measure_text(part);
            let cx = state.text_left() + (state.text_width() - tw) / 2.0;
            state.ensure_space(metrics.line_height());
            state.current_x = cx;
            state.emit_text(part, size, FontStyle::Regular, Color::BLACK);
            state.current_y += metrics.line_height();
        }
        state.add_vertical_space(10.0);
    }

    // Deferred abstract
    if state.deferred_abstract_idx.is_some() {
        for node in &doc.body {
            if let Node::Abstract(content) = node {
                state.add_vertical_space(6.0);
                let saved_indent = state.indent;
                let saved_right = state.right_indent;
                state.set_right_indent(36.0);
                state.set_indent(state.indent + 36.0);
                state.current_x = state.text_left();
                let saved_size = state.current_font_size;
                let abs_size = state.base_font_size * 0.9;
                state.current_font_size = abs_size;
                let prefix = "Abstract. ";
                let prefix_w = font::measure_text(prefix, FontId::TimesBold, abs_size);
                state.emit_text(prefix, abs_size, FontStyle::SmallCaps, Color::BLACK);
                state.current_x += prefix_w;
                super::layout_nodes(content, state, doc, source)?;
                state.current_font_size = saved_size;
                state.set_right_indent(saved_right);
                state.set_indent(saved_indent);
                state.current_x = state.text_left();
                state.add_vertical_space(6.0);
                break;
            }
        }
        state.deferred_abstract_idx = None;
    }

    // First-page footer items
    {
        let fn_size = state.base_font_size * 0.7;
        let fn_lh = fn_size * 1.4;
        let mut footer_lines: Vec<(String, FontStyle)> = Vec::new();
        if let Some(date) = &doc.preamble.date {
            footer_lines.push((format!("Date: {}.", date.trim_end_matches('.')), FontStyle::Italic));
        }
        if let Some((year, text)) = &doc.preamble.subjclass {
            footer_lines.push((format!("{} Mathematics Subject Classification. {}.", year, text.trim_end_matches('.')), FontStyle::Italic));
        }
        if let Some(kw) = &doc.preamble.keywords {
            footer_lines.push((format!("Key words and phrases. {}.", kw.trim_end_matches('.')), FontStyle::Italic));
        }

        if !footer_lines.is_empty() {
            let total_h = footer_lines.len() as f32 * fn_lh + 12.0;
            let orig_max_y = state.page_setup.height - state.page_setup.margin_bottom - state.page_setup.footer_height;
            let footer_y = orig_max_y - total_h;
            state.cached_max_y = footer_y - 10.0;

            state.emit_line(
                state.page_setup.margin_left, footer_y,
                state.page_setup.margin_left + state.page_setup.text_width() * 0.3, footer_y,
                0.4, Color::GRAY,
            );

            let text_w = state.page_setup.text_width();
            let mut y = footer_y + 8.0;
            for (text, style) in &footer_lines {
                let metrics = FontMetrics::new(fn_size, *style);
                let lines = wrap_text(text, &metrics, text_w);
                for line in &lines {
                    state.current_x = state.page_setup.margin_left;
                    state.current_y = y;
                    state.emit_text(line, fn_size, *style, Color::BLACK);
                    y += fn_lh;
                }
            }
        }
    }

    Ok(())
}
