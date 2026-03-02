/// Table layout

use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};
use super::state::LayoutState;
use super::text::{node_to_text_resolved, node_to_text};

use anyhow::Result;

fn detect_cell_style(content: &[Node]) -> FontStyle {
    if content.is_empty() { return FontStyle::Regular; }
    let mut has_bold = false;
    let mut has_non_bold = false;
    for node in content {
        match node {
            Node::Bold(_) => has_bold = true,
            Node::FontStyleDecl(FontDeclType::Bold) => has_bold = true,
            Node::FontStyleDecl(FontDeclType::Italic) => return FontStyle::Italic,
            Node::Italic(_) => return FontStyle::Italic,
            Node::Text(t) if t.trim().is_empty() => {}
            Node::TextRef(_, _) => has_non_bold = true,
            _ => has_non_bold = true,
        }
    }
    if has_bold && !has_non_bold { FontStyle::Bold } else { FontStyle::Regular }
}

pub(super) fn layout_table(table: &Table, state: &mut LayoutState, _doc: &Document, source: &str) -> Result<()> {
    if table.rows.is_empty() { return Ok(()); }

    let num_data_rows = {
        let num_cols = table.columns.iter().filter(|c| !matches!(c, ColumnSpec::Separator)).count().max(1);
        let mut n = table.rows.len();
        while n > 0 && table.rows[n - 1].cells.len() < num_cols
            && table.rows[n - 1].cells.iter().all(|c| c.content.is_empty()) { n -= 1; }
        n
    };

    if table.caption.is_some() { state.table_counter += 1; }
    let tbl_num = state.table_counter;
    state.add_vertical_space(8.0);

    let data_cols: Vec<&ColumnSpec> = table.columns.iter().filter(|c| !matches!(c, ColumnSpec::Separator)).collect();
    let num_cols = data_cols.len().max(1);
    let available_width = state.text_width();
    let cell_padding = 4.0;
    let has_explicit_widths = data_cols.iter().any(|c| matches!(c, ColumnSpec::Paragraph(_)));
    let base_metrics = state.metrics();
    let font_size = state.current_font_size;
    let mut col_max_widths = vec![0.0f32; num_cols];
    let mut cell_texts: Vec<Vec<String>> = Vec::with_capacity(table.rows.len());
    let mut cell_styles: Vec<Vec<FontStyle>> = Vec::with_capacity(table.rows.len());
    let mut cell_logical_cols: Vec<Vec<(u32, u32, Option<ColumnSpec>)>> = Vec::with_capacity(table.rows.len());

    for row in &table.rows {
        let mut row_texts = Vec::with_capacity(num_cols);
        let mut row_styles = Vec::with_capacity(num_cols);
        let mut row_cols = Vec::with_capacity(num_cols);
        let mut logical_col: u32 = 0;
        for cell in &row.cells {
            if logical_col as usize >= num_cols { break; }
            let span = cell.colspan.max(1);
            let mut text = String::new();
            for node in &cell.content { node_to_text_resolved(node, &mut text, source, &state.label_map); }
            let trimmed = text.trim().to_string();
            let style = detect_cell_style(&cell.content);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };
            let w = font::measure_text(&trimmed, fid, font_size);
            if span == 1 && (logical_col as usize) < num_cols {
                if w > col_max_widths[logical_col as usize] { col_max_widths[logical_col as usize] = w; }
            }
            row_texts.push(trimmed); row_styles.push(style);
            row_cols.push((logical_col, span, cell.alignment.clone()));
            logical_col += span;
        }
        while row_texts.len() < num_cols {
            row_texts.push(String::new()); row_styles.push(FontStyle::Regular);
            row_cols.push((logical_col, 1, None)); logical_col += 1;
        }
        cell_texts.push(row_texts); cell_styles.push(row_styles); cell_logical_cols.push(row_cols);
    }

    let col_widths: Vec<f32> = if has_explicit_widths {
        let mut widths: Vec<f32> = Vec::with_capacity(num_cols);
        let mut total_specified = 0.0f32;
        let mut num_auto = 0u32;
        for col in &data_cols {
            match col {
                ColumnSpec::Paragraph(w) => { widths.push(*w); total_specified += *w; }
                _ => { widths.push(0.0); num_auto += 1; }
            }
        }
        while widths.len() < num_cols { widths.push(0.0); num_auto += 1; }
        if num_auto > 0 {
            let remaining = (available_width - total_specified).max(0.0);
            let auto_w = remaining / num_auto as f32;
            for (i, w) in widths.iter_mut().enumerate() {
                if *w == 0.0 {
                    *w = col_max_widths.get(i).copied().unwrap_or(auto_w).min(auto_w).max(cell_padding * 3.0);
                }
            }
        }
        let total: f32 = widths.iter().sum();
        if total > available_width && total > 0.0 {
            let scale = available_width / total;
            widths.iter_mut().for_each(|w| *w *= scale);
        }
        widths
    } else {
        let total_content = col_max_widths.iter().sum::<f32>() + (num_cols as f32 * cell_padding * 2.0);
        if total_content <= available_width {
            let remaining = available_width - total_content;
            let extra_per_col = remaining / num_cols as f32;
            col_max_widths.iter().map(|&w| w + cell_padding * 2.0 + extra_per_col).collect()
        } else {
            col_max_widths.iter().map(|&w| {
                let ratio = if total_content > 0.0 { w / total_content } else { 1.0 / num_cols as f32 };
                (ratio * available_width).max(cell_padding * 3.0)
            }).collect()
        }
    };

    let line_h = base_metrics.line_height();
    let actual_table_width: f32 = col_widths.iter().sum();
    let table_x = if actual_table_width < available_width {
        state.text_left() + (available_width - actual_table_width) / 2.0
    } else { state.text_left() };

    let mut wrapped_cells: Vec<Vec<Vec<String>>> = Vec::with_capacity(table.rows.len());
    let mut row_heights: Vec<f32> = Vec::with_capacity(table.rows.len());

    for (row_idx, row_texts) in cell_texts.iter().enumerate() {
        let mut row_wrapped: Vec<Vec<String>> = Vec::with_capacity(num_cols);
        let mut max_lines = 1u32;
        for (col_idx, text) in row_texts.iter().enumerate() {
            let (logical_col, span, _) = cell_logical_cols.get(row_idx).and_then(|r| r.get(col_idx)).cloned().unwrap_or((col_idx as u32, 1, None));
            let col_w = if span > 1 {
                (logical_col..logical_col + span).map(|c| col_widths.get(c as usize).copied().unwrap_or(0.0)).sum::<f32>()
            } else { col_widths.get(logical_col as usize).copied().unwrap_or(100.0) };
            let content_w = col_w - cell_padding * 2.0;
            let fid = if cell_styles[row_idx][col_idx] == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };
            let text_w = font::measure_text(text, fid, font_size);
            if text_w <= content_w + 1.0 || content_w < 20.0 {
                row_wrapped.push(vec![text.clone()]);
            } else {
                let words: Vec<&str> = text.split_whitespace().collect();
                let mut lines: Vec<String> = Vec::new();
                let mut current_line = String::new();
                let mut current_w = 0.0f32;
                let space_w = font::measure_text(" ", fid, font_size);
                for word in &words {
                    let word_w = font::measure_text(word, fid, font_size);
                    if current_line.is_empty() { current_line.push_str(word); current_w = word_w; }
                    else if current_w + space_w + word_w <= content_w {
                        current_line.push(' '); current_line.push_str(word); current_w += space_w + word_w;
                    } else {
                        lines.push(std::mem::take(&mut current_line)); current_line.push_str(word); current_w = word_w;
                    }
                }
                if !current_line.is_empty() { lines.push(current_line); }
                if lines.is_empty() { lines.push(String::new()); }
                max_lines = max_lines.max(lines.len() as u32);
                row_wrapped.push(lines);
            }
        }
        let extra = table.rows[row_idx].extra_space_before;
        let rule_sep = if table.rows[row_idx].hline_before { font_size * 0.9 } else { 0.0 };
        let rh = max_lines as f32 * line_h + cell_padding * 2.0 + extra + rule_sep;
        row_heights.push(rh);
        wrapped_cells.push(row_wrapped);
    }

    let total_row_height: f32 = row_heights.iter().take(num_data_rows).sum();
    let caption_height = if table.caption.is_some() { state.current_font_size * 0.9 * 1.2 + 4.0 } else { 0.0 };
    let total_table_height = total_row_height + caption_height + 8.0;

    let remaining_space = state.cached_max_y - state.current_y;
    let full_page_height = state.cached_max_y - state.cached_start_y;
    if total_table_height > remaining_space && total_table_height <= full_page_height { state.new_page(); }

    if let Some(caption) = &table.caption {
        state.text_buf.clear();
        state.text_buf.push_str("Table ");
        let mut ibuf = itoa::Buffer::new();
        state.text_buf.push_str(ibuf.format(tbl_num));
        state.text_buf.push_str(": ");
        for node in caption { node_to_text(node, &mut state.text_buf, source); }
        let full: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
        let cap_font_size = state.current_font_size * 0.9;
        let cap_metrics = FontMetrics::new(cap_font_size, FontStyle::Regular);
        let tw = cap_metrics.measure_text(full);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.current_x = cx;
        state.emit_text(full, cap_font_size, FontStyle::Regular, Color::DARK_GRAY);
        state.current_y += cap_metrics.line_height() + 4.0;
    }

    for row_idx in 0..num_data_rows {
        let row = &table.rows[row_idx];
        let row_height = row_heights[row_idx];
        let extra = row.extra_space_before;
        state.ensure_space(row_height);
        if extra > 0.0 { state.current_y += extra; }
        let y = state.current_y;

        let mut col_x = table_x;
        for (cell_idx, cell_lines) in wrapped_cells[row_idx].iter().enumerate() {
            let (logical_col, span, align_override) = cell_logical_cols.get(row_idx).and_then(|r| r.get(cell_idx)).cloned().unwrap_or((cell_idx as u32, 1, None));
            if logical_col as usize >= num_cols { break; }
            col_x = table_x + col_widths.iter().take(logical_col as usize).sum::<f32>();
            let col_w = if span > 1 {
                (logical_col..logical_col + span).map(|c| col_widths.get(c as usize).copied().unwrap_or(0.0)).sum::<f32>()
            } else { col_widths.get(logical_col as usize).copied().unwrap_or(100.0) };
            let cx = col_x + cell_padding;
            let cell_content_width = col_w - cell_padding * 2.0;
            let default_center = ColumnSpec::Center;
            let align = if let Some(ref ov) = align_override { ov }
                else if span > 1 { &default_center }
                else if (logical_col as usize) < data_cols.len() { data_cols[logical_col as usize] }
                else { &ColumnSpec::Left };
            let style = cell_styles[row_idx].get(cell_idx).copied().unwrap_or(FontStyle::Regular);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };

            for (line_idx, line_text) in cell_lines.iter().enumerate() {
                let display_w = font::measure_text(line_text, fid, font_size);
                let text_x = match align {
                    ColumnSpec::Center => cx + (cell_content_width - display_w) / 2.0,
                    ColumnSpec::Right => cx + cell_content_width - display_w,
                    _ => cx,
                };
                let rule_sep = if row.hline_before { font_size * 0.9 } else { 0.0 };
                let text_y = y + cell_padding + rule_sep + line_idx as f32 * line_h;
                state.current_x = text_x;
                state.current_y = text_y;
                state.emit_text(line_text, state.current_font_size, style, Color::BLACK);
            }
        }

        if row.hline_before {
            let rule_width = if row_idx == 0 { 1.2 } else { 0.8 };
            state.emit_line(table_x, y, table_x + actual_table_width, y, rule_width, Color::BLACK);
        }
        if row.hline_after {
            let line_y = y + row_height;
            let rule_width = if row_idx == num_data_rows - 1 { 1.2 } else { 0.8 };
            state.emit_line(table_x, line_y, table_x + actual_table_width, line_y, rule_width, Color::BLACK);
        }
        state.current_y = y + row_height - extra;
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();
    Ok(())
}
