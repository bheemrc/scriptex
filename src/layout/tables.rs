/// Table layout

use crate::color::Color;
use crate::document::*;
use crate::typeset::FontStyle;
use crate::font::{self, FontId};
use crate::math_layout;
use super::state::LayoutState;
use super::text::node_to_text_resolved;
use super::math::emit_math_elements;
use super::spans;

use anyhow::Result;

/// Check if cell content contains inline math or dingbats (requires special font handling)
fn cell_has_math(content: &[Node]) -> bool {
    fn check(nodes: &[Node]) -> bool {
        for n in nodes {
            match n {
                Node::InlineMath(_) | Node::Dingbat(_) => return true,
                Node::Bold(c) | Node::Italic(c) | Node::Emph(c) | Node::Group(c)
                | Node::Underline(c) | Node::Monospace(c) | Node::SmallCaps(c)
                | Node::Strikethrough(c) | Node::Paragraph(c) => { if check(c) { return true; } }
                Node::Colored { content, .. } | Node::FontSize { content, .. } => { if check(content) { return true; } }
                _ => {}
            }
        }
        false
    }
    check(content)
}

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
    // Track vertical separator positions: separator_positions[i] = true means draw a vertical line before data column i
    // Also track trailing separator
    let mut separator_before = vec![false; num_cols + 1]; // +1 for trailing separator
    {
        let mut data_idx = 0usize;
        let mut prev_was_sep = false;
        for col in &table.columns {
            match col {
                ColumnSpec::Separator => { separator_before[data_idx] = true; prev_was_sep = true; }
                _ => { data_idx += 1; prev_was_sep = false; }
            }
        }
        if prev_was_sep && data_idx <= num_cols { separator_before[data_idx] = true; }
    }
    let available_width = state.text_width();
    let cell_padding = 5.0; // ~0.5em padding for readable cell spacing
    let has_explicit_widths = data_cols.iter().any(|c| matches!(c, ColumnSpec::Paragraph(_)));
    let base_metrics = state.metrics();
    let font_size = state.current_font_size;
    let mut col_max_widths = vec![0.0f32; num_cols];
    let mut cell_texts: Vec<Vec<String>> = Vec::with_capacity(table.rows.len());
    let mut cell_styles: Vec<Vec<FontStyle>> = Vec::with_capacity(table.rows.len());
    let mut cell_logical_cols: Vec<Vec<(u32, u32, Option<ColumnSpec>)>> = Vec::with_capacity(table.rows.len());
    // Pre-computed math boxes for cells with inline math (row_idx, cell_idx) → MathBox
    let mut cell_math: Vec<Vec<Option<math_layout::MathBox>>> = Vec::with_capacity(table.rows.len());
    // Track multirow coverage: (row_idx, logical_col) → true if covered by a rowspan from above
    let mut rowspan_covered: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    // Track multirow origins: (origin_row, cell_idx) → (rowspan, logical_col)
    let mut multirow_origins: Vec<(usize, usize, u32, u32)> = Vec::new();

    for (row_idx, row) in table.rows.iter().enumerate() {
        let mut row_texts = Vec::with_capacity(num_cols);
        let mut row_styles = Vec::with_capacity(num_cols);
        let mut row_cols = Vec::with_capacity(num_cols);
        let mut row_math: Vec<Option<math_layout::MathBox>> = Vec::with_capacity(num_cols);
        let mut logical_col: u32 = 0;
        // Skip columns covered by rowspan from above
        while rowspan_covered.contains(&(row_idx, logical_col as usize)) && (logical_col as usize) < num_cols {
            // Insert empty placeholder for covered cell
            row_texts.push(String::new()); row_styles.push(FontStyle::Regular);
            row_cols.push((logical_col, 1, None)); row_math.push(None);
            logical_col += 1;
        }
        for (cell_idx, cell) in row.cells.iter().enumerate() {
            // Skip columns covered by rowspan
            while rowspan_covered.contains(&(row_idx, logical_col as usize)) && (logical_col as usize) < num_cols {
                row_texts.push(String::new()); row_styles.push(FontStyle::Regular);
                row_cols.push((logical_col, 1, None)); row_math.push(None);
                logical_col += 1;
            }
            if logical_col as usize >= num_cols { break; }
            let span = cell.colspan.max(1);
            let has_math = cell_has_math(&cell.content);
            let mut text = String::new();
            for node in &cell.content { node_to_text_resolved(node, &mut text, source, &state.label_map); }
            let trimmed = text.trim().to_string();
            let style = detect_cell_style(&cell.content);
            let fid = if style == FontStyle::Bold { FontId::TimesBold } else { FontId::TimesRoman };
            // For cells with math, compute math box for accurate width
            let (w, math_box) = if has_math {
                let mb = layout_cell_with_math(&cell.content, font_size, source, &state.label_map);
                let w = mb.width;
                (w, Some(mb))
            } else {
                (font::measure_text(&trimmed, fid, font_size), None)
            };
            if span == 1 && (logical_col as usize) < num_cols {
                if w > col_max_widths[logical_col as usize] { col_max_widths[logical_col as usize] = w; }
            }
            row_texts.push(trimmed); row_styles.push(style);
            row_cols.push((logical_col, span, cell.alignment.clone()));
            row_math.push(math_box);
            // Track multirow: mark subsequent rows as covered
            if cell.rowspan > 1 {
                multirow_origins.push((row_idx, cell_idx, cell.rowspan, logical_col));
                for r in 1..cell.rowspan as usize {
                    for c in 0..span as usize {
                        rowspan_covered.insert((row_idx + r, logical_col as usize + c));
                    }
                }
            }
            logical_col += span;
        }
        while row_texts.len() < num_cols {
            row_texts.push(String::new()); row_styles.push(FontStyle::Regular);
            row_cols.push((logical_col, 1, None)); logical_col += 1;
            row_math.push(None);
        }
        cell_texts.push(row_texts); cell_styles.push(row_styles); cell_logical_cols.push(row_cols);
        cell_math.push(row_math);
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
        // Improved auto-calculation: use min widths + proportional distribution
        let min_col_width = cell_padding * 4.0; // absolute minimum
        let total_content = col_max_widths.iter().sum::<f32>() + (num_cols as f32 * cell_padding * 2.0);
        if total_content <= available_width {
            // Everything fits — distribute remaining space proportionally to content
            let remaining = available_width - total_content;
            if total_content > 0.0 {
                col_max_widths.iter().map(|&w| {
                    let base = w + cell_padding * 2.0;
                    let share = (w / total_content) * remaining;
                    base + share
                }).collect()
            } else {
                vec![available_width / num_cols as f32; num_cols]
            }
        } else {
            // Overflow — give each column at least min_width, then distribute proportionally
            let total_min = min_col_width * num_cols as f32;
            let distributable = (available_width - total_min).max(0.0);
            let total_max_content = col_max_widths.iter().sum::<f32>().max(1.0);
            col_max_widths.iter().map(|&w| {
                let share = (w / total_max_content) * distributable;
                (min_col_width + share).max(min_col_width)
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
            let fid = if cell_styles[row_idx][col_idx] == FontStyle::Bold { FontId::TimesBold } else { FontId::TimesRoman };
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
        let rh = max_lines as f32 * line_h * state.array_stretch + cell_padding * 2.0 + extra + rule_sep;
        row_heights.push(rh);
        wrapped_cells.push(row_wrapped);
    }

    let total_row_height: f32 = row_heights.iter().take(num_data_rows).sum();
    let caption_height = if table.caption.is_some() { state.current_font_size * 1.2 + 10.0 } else { 0.0 };
    let total_table_height = total_row_height + caption_height + 8.0;

    let remaining_space = state.cached_max_y - state.current_y;
    let full_page_height = state.cached_max_y - state.cached_start_y;
    if total_table_height > remaining_space && total_table_height <= full_page_height { state.new_page(); }

    if let Some(caption) = &table.caption {
        let cap_font_size = state.current_font_size;
        state.current_y += 6.0;

        // Bold "Table N: " prefix
        let mut ibuf = itoa::Buffer::new();
        let prefix = format!("Table {}: ", ibuf.format(tbl_num));
        let prefix_width = font::measure_text(&prefix, FontId::TimesBold, cap_font_size);

        // Pre-measure caption text to decide centering
        state.text_buf.clear();
        let label_map: &std::collections::HashMap<String, String> = unsafe { &*(&state.label_map as *const _) };
        for node in caption.iter() { node_to_text_resolved(node, &mut state.text_buf, source, label_map); }
        let cap_text_width = font::measure_text(state.text_buf.as_str(), FontId::TimesRoman, cap_font_size);
        let total_width = prefix_width + cap_text_width;

        // Center if caption fits on one line
        if total_width <= state.text_width() {
            let cx = state.text_left() + (state.text_width() - total_width) / 2.0;
            state.current_x = cx;
        } else {
            state.current_x = state.text_left();
        }

        state.emit_text(&prefix, cap_font_size, FontStyle::Bold, Color::BLACK);
        state.current_x += prefix_width;

        // Rich paragraph layout for caption body (supports bold, italic, math, etc.)
        let saved_para_indent = state.paragraph_indent;
        state.paragraph_indent = 0.0;
        spans::layout_rich_paragraph(caption, state, source, false)?;
        state.paragraph_indent = saved_para_indent;
    }

    for row_idx in 0..num_data_rows {
        let row = &table.rows[row_idx];
        let row_height = row_heights[row_idx];
        let extra = row.extra_space_before;
        state.ensure_space(row_height);
        if extra > 0.0 { state.current_y += extra; }
        let y = state.current_y;
        let rule_sep = if row.hline_before { font_size * 0.9 } else { 0.0 };

        let mut col_x = table_x;
        for (cell_idx, cell_lines) in wrapped_cells[row_idx].iter().enumerate() {
            let (logical_col, span, align_override) = cell_logical_cols.get(row_idx).and_then(|r| r.get(cell_idx)).cloned().unwrap_or((cell_idx as u32, 1, None));
            if logical_col as usize >= num_cols { break; }
            // Skip cells covered by multirow from a previous row
            if rowspan_covered.contains(&(row_idx, logical_col as usize)) { continue; }
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
            let fid = if style == FontStyle::Bold { FontId::TimesBold } else { FontId::TimesRoman };

            // Check if this cell is the origin of a multirow span — vertically center content
            let multirow_y_offset = multirow_origins.iter()
                .find(|(or, _ci, _rs, lc)| *or == row_idx && *lc == logical_col)
                .map(|&(origin_row, _ci, rspan, _lc)| {
                    let total_h: f32 = (origin_row..origin_row + rspan as usize)
                        .map(|r| row_heights.get(r).copied().unwrap_or(0.0))
                        .sum();
                    (total_h - row_height) / 2.0
                })
                .unwrap_or(0.0);

            // Use pre-computed math box if available (for cells with inline math/dingbats)
            if let Some(Some(ref math_box)) = cell_math.get(row_idx).and_then(|r| r.get(cell_idx)) {
                let display_w = math_box.width;
                let text_x = match align {
                    ColumnSpec::Center => cx + (cell_content_width - display_w) / 2.0,
                    ColumnSpec::Right => cx + cell_content_width - display_w,
                    _ => cx,
                };
                let text_y = y + cell_padding + rule_sep + multirow_y_offset;
                emit_math_elements(math_box, text_x, text_y + math_box.height, state);
            } else {
                for (line_idx, line_text) in cell_lines.iter().enumerate() {
                    let display_w = font::measure_text(line_text, fid, font_size);
                    let text_x = match align {
                        ColumnSpec::Center => cx + (cell_content_width - display_w) / 2.0,
                        ColumnSpec::Right => cx + cell_content_width - display_w,
                        _ => cx,
                    };
                    let text_y = y + cell_padding + rule_sep + line_idx as f32 * line_h + multirow_y_offset;
                    state.current_x = text_x;
                    state.current_y = text_y;
                    state.emit_text(line_text, state.current_font_size, style, Color::BLACK);
                }
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
        // Render cmidrules (partial horizontal rules)
        for &(start_col, end_col) in &row.cmidrules {
            let s = (start_col.max(1) - 1) as usize; // convert to 0-based
            let e = end_col as usize;
            let x1 = table_x + col_widths.iter().take(s).sum::<f32>();
            let x2 = table_x + col_widths.iter().take(e.min(num_cols)).sum::<f32>();
            state.emit_line(x1, y, x2, y, 0.6, Color::BLACK);
        }
        // Render vertical column separators
        {
            let mut vx = table_x;
            for ci in 0..=num_cols {
                if separator_before[ci] {
                    state.emit_line(vx, y, vx, y + row_height, 0.4, Color::BLACK);
                }
                if ci < num_cols { vx += col_widths[ci]; }
            }
        }
        state.current_y = y + row_height - extra;
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();
    Ok(())
}

/// Layout a cell that contains inline math/dingbats as a horizontal sequence of text + math boxes
fn layout_cell_with_math(content: &[Node], font_size: f32, source: &str, label_map: &std::collections::HashMap<String, String>) -> math_layout::MathBox {
    let mut result = math_layout::MathBox { width: 0.0, height: font_size, depth: 0.0, elements: Vec::new() };
    let mut x = 0.0f32;
    layout_cell_nodes(&mut result, &mut x, content, font_size, source, label_map);
    result.width = x;
    result
}

fn layout_cell_nodes(result: &mut math_layout::MathBox, x: &mut f32, nodes: &[Node], font_size: f32, source: &str, label_map: &std::collections::HashMap<String, String>) {
    for node in nodes {
        match node {
            Node::InlineMath(math_nodes) => {
                let mb = math_layout::layout_math(math_nodes, font_size);
                for elem in &mb.elements {
                    let shifted = match elem {
                        math_layout::MathElement::Text { x: ex, y, text, font_size: fs, font_id, color } => {
                            math_layout::MathElement::Text { x: ex + *x, y: *y, text: text.clone(), font_size: *fs, font_id: *font_id, color: *color }
                        }
                        math_layout::MathElement::Line { x1, y1, x2, y2, width, color } => {
                            math_layout::MathElement::Line { x1: x1 + *x, y1: *y1, x2: x2 + *x, y2: *y2, width: *width, color: *color }
                        }
                    };
                    result.elements.push(shifted);
                }
                result.height = result.height.max(mb.height);
                result.depth = result.depth.max(mb.depth);
                *x += mb.width;
            }
            Node::Dingbat(code) => {
                let text = String::from(char::from(*code));
                let tw = font::char_width_1000(FontId::ZapfDingbats, *code) as f32 * font_size / 1000.0;
                result.elements.push(math_layout::MathElement::Text {
                    x: *x, y: 0.0, text, font_size, font_id: FontId::ZapfDingbats, color: Color::BLACK,
                });
                *x += tw;
            }
            // Recurse into wrapper nodes
            Node::Bold(c) | Node::Italic(c) | Node::Emph(c) | Node::Group(c)
            | Node::Underline(c) | Node::Monospace(c) | Node::SmallCaps(c)
            | Node::Strikethrough(c) | Node::Paragraph(c) | Node::Superscript(c) | Node::Subscript(c) => {
                layout_cell_nodes(result, x, c, font_size, source, label_map);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. } => {
                layout_cell_nodes(result, x, content, font_size, source, label_map);
            }
            _ => {
                let mut text = String::new();
                node_to_text_resolved(node, &mut text, source, label_map);
                let text = text.trim_matches(|c: char| c == '\n').to_string();
                if !text.is_empty() {
                    let tw = font::measure_text(&text, FontId::TimesRoman, font_size);
                    result.elements.push(math_layout::MathElement::Text {
                        x: *x, y: 0.0, text, font_size, font_id: FontId::TimesRoman, color: Color::BLACK,
                    });
                    *x += tw;
                }
            }
        }
    }
}
