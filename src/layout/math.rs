/// Display math layout and equation numbering

use crate::color::Color;
use crate::document::*;
use crate::typeset::FontStyle;
use crate::font::FontId;
use crate::math_layout;
use super::state::LayoutState;
use super::types::*;

use anyhow::Result;

pub(super) fn layout_display_math_data(math_data: &DisplayMathData, state: &mut LayoutState) -> Result<()> {
    let has_alignment = math_data.nodes.iter().any(|n| matches!(n, MathNode::AlignmentMark | MathNode::NewLine));
    let has_newlines = math_data.nodes.iter().any(|n| matches!(n, MathNode::NewLine));

    if has_newlines && matches!(math_data.env_type, MathEnvType::Multline) {
        layout_multline_math(&math_data.nodes, math_data.numbered, state)
    } else if has_alignment && matches!(math_data.env_type, MathEnvType::Align | MathEnvType::Gather) {
        layout_aligned_math(&math_data.nodes, math_data.numbered, state)
    } else if has_newlines && !has_alignment {
        // split/gather with newlines but no & alignment marks — treat each line as centered
        layout_aligned_math(&math_data.nodes, math_data.numbered, state)
    } else {
        layout_display_math_simple(&math_data.nodes, math_data.numbered, state)
    }
}

fn is_breakable_op(node: &MathNode) -> bool {
    match node {
        MathNode::Operator(op) => matches!(op.as_str(),
            "+" | "-" | "=" | "<" | ">" | "≤" | "≥" | "≠" | "≈"
            | "∈" | "∉" | "⊂" | "⊃" | "⊆" | "⊇" | "∼" | "≅" | "≃"
            | "→" | "←" | "↦" | "⟶" | "⟵"
            | "∧" | "∨" | "⊕" | "⊗" | "×"
        ),
        MathNode::Symbol(s) => matches!(s.as_str(),
            "+" | "-" | "=" | "<" | ">" | "≤" | "≥" | "≠" | "≈"
            | "∈" | "∉" | "⊂" | "⊃" | "⊆" | "⊇" | "∼" | "≅" | "≃"
            | "→" | "←" | "↦" | "⟶" | "⟵"
        ),
        _ => false,
    }
}

fn is_math_break_point(node: &MathNode) -> bool {
    match node {
        MathNode::Group(children) => children.iter().any(|c| is_breakable_op(c)),
        _ => is_breakable_op(node),
    }
}

fn layout_display_math_simple(math_nodes: &[MathNode], numbered: bool, state: &mut LayoutState) -> Result<()> {
    // LaTeX \abovedisplayskip ≈ 10-12pt for 10pt base
    state.add_vertical_space(state.current_font_size * 1.0);

    let filtered: Vec<&MathNode> = math_nodes.iter()
        .filter(|n| !matches!(n, MathNode::AlignmentMark | MathNode::NewLine))
        .collect();
    let owned: Vec<MathNode> = filtered.into_iter().cloned().collect();
    let math_box = math_layout::layout_math(&owned, state.current_font_size);

    let eq_num_width = if numbered { 40.0 } else { 0.0 };
    let avail_width = state.text_width() - eq_num_width;

    if math_box.width <= avail_width {
        let total_height = math_box.height + math_box.depth;
        state.ensure_space(total_height + state.base_font_size * 1.6);
        let cx = state.text_left() + (avail_width - math_box.width) / 2.0;
        let baseline_y = state.current_y + math_box.height;
        emit_math_elements(&math_box, cx, baseline_y, state);

        if numbered {
            emit_equation_number(state, baseline_y);
        }
        state.current_y = baseline_y + math_box.depth;
    } else {
        let font_size = state.current_font_size;
        let indent = font_size * 2.0;
        let mut break_indices: Vec<usize> = Vec::new();
        for (i, node) in owned.iter().enumerate() {
            if is_math_break_point(node) { break_indices.push(i); }
        }

        if break_indices.is_empty() {
            let total_height = math_box.height + math_box.depth;
            state.ensure_space(total_height + state.base_font_size * 1.6);
            let baseline_y = state.current_y + math_box.height;
            emit_math_elements(&math_box, state.text_left(), baseline_y, state);
            state.current_y = baseline_y + math_box.depth;
        } else {
            let mut lines: Vec<(usize, usize)> = Vec::new();
            let mut line_start = 0;
            let mut last_valid_break = 0;
            for (_bi, &break_pos) in break_indices.iter().enumerate() {
                let segment = &owned[line_start..break_pos];
                let seg_box = math_layout::layout_math(segment, font_size);
                let line_avail = if lines.is_empty() { avail_width } else { avail_width - indent };
                if seg_box.width > line_avail && last_valid_break > line_start {
                    lines.push((line_start, last_valid_break));
                    line_start = last_valid_break;
                }
                last_valid_break = break_pos;
            }
            lines.push((line_start, owned.len()));

            let mut first_line_baseline_y = 0.0f32;
            for (li, &(start, end)) in lines.iter().enumerate() {
                let segment = &owned[start..end];
                let seg_box = math_layout::layout_math(segment, font_size);
                let total_h = seg_box.height + seg_box.depth;
                let row_spacing = font_size * 0.4;
                state.ensure_space(total_h + row_spacing);
                let line_avail = if li == 0 { avail_width } else { avail_width - indent };
                let cx = if seg_box.width <= line_avail {
                    if li == 0 {
                        state.text_left() + (avail_width - seg_box.width) / 2.0
                    } else {
                        state.text_left() + avail_width - seg_box.width
                    }
                } else {
                    state.text_left() + if li > 0 { indent } else { 0.0 }
                };
                let baseline_y = state.current_y + seg_box.height;
                if li == 0 { first_line_baseline_y = baseline_y; }
                emit_math_elements(&seg_box, cx, baseline_y, state);
                state.current_y = baseline_y + seg_box.depth + row_spacing;
            }

            if numbered {
                emit_equation_number(state, first_line_baseline_y);
            }
        }
    }

    // LaTeX \belowdisplayskip ≈ 10-12pt for 10pt base
    state.add_vertical_space(state.current_font_size * 1.0);
    state.current_x = state.text_left();
    state.suppress_next_indent = true;
    Ok(())
}

/// Layout multline environment: first line left-aligned, last line right-aligned, middle centered.
fn layout_multline_math(math_nodes: &[MathNode], numbered: bool, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(state.current_font_size * 1.0);

    // Split into lines at NewLine nodes
    let mut lines: Vec<Vec<MathNode>> = vec![vec![]];
    for node in math_nodes {
        if matches!(node, MathNode::NewLine) {
            lines.push(vec![]);
        } else {
            lines.last_mut().unwrap().push(node.clone());
        }
    }
    // Remove empty trailing lines
    while lines.last().map_or(false, |l| l.is_empty()) { lines.pop(); }
    if lines.is_empty() { return Ok(()); }

    let font_size = state.current_font_size;
    let step = font_size * 1.6;
    let text_left = state.text_left();
    let text_width = state.text_width();
    let num_lines = lines.len();
    let eq_number_width = if numbered { 40.0 } else { 0.0 };
    let total_height = step * num_lines as f32;
    state.ensure_space(total_height + state.base_font_size * 1.6);

    for (line_idx, line_nodes) in lines.iter().enumerate() {
        if line_nodes.is_empty() { continue; }

        let math_box = math_layout::layout_math(line_nodes, font_size);
        let line_width = math_box.width;

        let x = if line_idx == 0 {
            // First line: left-aligned (with small indent)
            text_left + font_size * 1.5
        } else if line_idx == num_lines - 1 {
            // Last line: right-aligned (before equation number)
            (text_left + text_width - line_width - eq_number_width - font_size * 1.5).max(text_left)
        } else {
            // Middle lines: centered
            text_left + (text_width - line_width) / 2.0
        };

        emit_math_elements(&math_box, x.max(text_left), state.current_y + math_box.height, state);

        // Equation number on the last line
        if numbered && line_idx == num_lines - 1 {
            state.equation_counter += 1;
            let eq_text = format!("({})", state.equation_counter);
            let num_x = state.text_left() + state.text_width() - 30.0;
            let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
            state.all_text.push_str(&eq_text);
            state.all_elements.push(PageElement::Text {
                x: num_x, y: state.current_y + math_box.height,
                text_offset: offset, text_len: eq_text.len().min(65535) as u16,
                font_size_100: (font_size * 100.0) as u16, font_style: FontStyle::Regular,
                color: Color::BLACK, word_spacing_50: 0,
            });
        }

        state.current_y += step;
    }

    state.add_vertical_space(state.current_font_size * 1.0);
    state.current_x = state.text_left();
    state.suppress_next_indent = true;
    Ok(())
}

fn layout_aligned_math(math_nodes: &[MathNode], numbered: bool, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(state.current_font_size * 1.0);

    let mut rows: Vec<Vec<Vec<MathNode>>> = Vec::new();
    let mut current_row: Vec<Vec<MathNode>> = Vec::new();
    let mut current_col: Vec<MathNode> = Vec::new();

    // Track per-row numbering overrides: None = use default, Some(true) = force no number, Some(tag) = custom
    let mut row_tags: Vec<Option<String>> = Vec::new(); // None = default numbering, Some("") = suppress, Some(text) = custom
    let mut current_row_tag: Option<String> = None;

    for node in math_nodes {
        match node {
            MathNode::NewLine => {
                current_row.push(std::mem::take(&mut current_col));
                rows.push(std::mem::take(&mut current_row));
                row_tags.push(current_row_tag.take());
            }
            MathNode::AlignmentMark => {
                current_row.push(std::mem::take(&mut current_col));
            }
            MathNode::NoTag => {
                current_row_tag = Some(String::new()); // Empty = suppress
            }
            MathNode::Tag(text) => {
                current_row_tag = Some(text.clone());
            }
            MathNode::Intertext(_) => {
                // Flush current row first
                if !current_col.is_empty() || !current_row.is_empty() {
                    current_row.push(std::mem::take(&mut current_col));
                    rows.push(std::mem::take(&mut current_row));
                    row_tags.push(current_row_tag.take());
                }
                // Store intertext as a single-element row with special marker
                rows.push(vec![vec![node.clone()]]);
                row_tags.push(Some("\x01INTERTEXT\x01".to_string()));
            }
            _ => { current_col.push(node.clone()); }
        }
    }
    if !current_col.is_empty() || !current_row.is_empty() {
        current_row.push(current_col);
        rows.push(current_row);
        row_tags.push(current_row_tag.take());
    }
    if rows.is_empty() { return Ok(()); }

    let font_size = state.current_font_size;
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut cell_boxes: Vec<Vec<math_layout::MathBox>> = Vec::new();
    let mut col_widths = vec![0.0f32; num_cols];

    for row in &rows {
        let mut row_boxes = Vec::new();
        for (j, cell) in row.iter().enumerate() {
            let mb = math_layout::layout_math(cell, font_size);
            if j < num_cols { col_widths[j] = col_widths[j].max(mb.width); }
            row_boxes.push(mb);
        }
        cell_boxes.push(row_boxes);
    }

    let col_gap = font_size * 0.5;
    let row_spacing = font_size * 1.6;
    let total_content_width: f32 = col_widths.iter().sum::<f32>() + col_gap * (num_cols.max(1) - 1) as f32;
    let total_height = row_spacing * rows.len() as f32;
    let eq_num_width = if numbered { 40.0 } else { 0.0 };
    let avail_width = state.text_width() - eq_num_width;

    state.ensure_space(total_height + state.base_font_size * 1.6);

    let base_x = if total_content_width > avail_width {
        state.text_left()
    } else {
        state.text_left() + (avail_width - total_content_width) / 2.0
    };

    for (row_idx, row_boxes) in cell_boxes.iter().enumerate() {
        // Check for intertext row
        let tag = row_tags.get(row_idx).and_then(|t| t.as_deref());
        if tag == Some("\x01INTERTEXT\x01") {
            // Render intertext as a normal text paragraph
            if let Some(row) = rows.get(row_idx) {
                if let Some(cells) = row.first() {
                    if let Some(MathNode::Intertext(text)) = cells.first() {
                        state.current_x = state.text_left();
                        state.emit_text(text, font_size, FontStyle::Regular, Color::BLACK);
                        state.current_y += row_spacing;
                        state.current_x = state.text_left();
                    }
                }
            }
            continue;
        }

        let baseline_y = state.current_y;
        let mut col_x = base_x;
        for (j, cell_box) in row_boxes.iter().enumerate() {
            let col_w = if j < col_widths.len() { col_widths[j] } else { cell_box.width };
            let cx = if j % 2 == 0 { col_x + col_w - cell_box.width } else { col_x };
            emit_math_elements(cell_box, cx, baseline_y + cell_box.height, state);
            col_x += col_w + col_gap;
        }

        if numbered {
            let suppress = tag == Some("");
            let custom_tag = tag.filter(|t| !t.is_empty());
            if !suppress {
                let eq_text = if let Some(ct) = custom_tag {
                    format!("({})", ct)
                } else {
                    state.equation_counter += 1;
                    format!("({})", state.equation_counter)
                };
                let num_x = state.text_left() + state.text_width() - 30.0;
                let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                state.all_text.push_str(&eq_text);
                let max_h = row_boxes.iter().map(|b| b.height).fold(0.0f32, f32::max);
                state.all_elements.push(PageElement::Text {
                    x: num_x, y: baseline_y + max_h,
                    text_offset: offset, text_len: eq_text.len().min(65535) as u16,
                    font_size_100: (font_size * 100.0) as u16, font_style: FontStyle::Regular,
                    color: Color::BLACK, word_spacing_50: 0,
                });
            }
        }
        state.current_y += row_spacing;
    }

    state.add_vertical_space(state.current_font_size * 1.0);
    state.current_x = state.text_left();
    state.suppress_next_indent = true;
    Ok(())
}

fn emit_equation_number(state: &mut LayoutState, baseline_y: f32) {
    state.equation_counter += 1;
    let eq_text = format!("({})", state.equation_counter);
    // Right-align equation number flush to right margin
    let num_w = crate::font::measure_text(&eq_text, crate::font::FontId::TimesRoman, state.current_font_size);
    let num_x = state.text_left() + state.text_width() - num_w;
    let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
    state.all_text.push_str(&eq_text);
    state.all_elements.push(PageElement::Text {
        x: num_x, y: baseline_y,
        text_offset: offset, text_len: eq_text.len().min(65535) as u16,
        font_size_100: (state.current_font_size * 100.0) as u16,
        font_style: FontStyle::Regular, color: Color::BLACK, word_spacing_50: 0,
    });
}

pub(super) fn emit_math_elements(math_box: &math_layout::MathBox, cx: f32, baseline_y: f32, state: &mut LayoutState) {
    for elem in &math_box.elements {
        match elem {
            math_layout::MathElement::Text { x, y, text, font_size, font_id, color } => {
                let style = match font_id {
                    FontId::TimesRoman => FontStyle::TimesRoman,
                    FontId::TimesItalic => FontStyle::TimesItalic,
                    FontId::TimesBold => FontStyle::TimesBold,
                    FontId::Courier => FontStyle::Monospace,
                    FontId::Symbol => FontStyle::Symbol,
                    FontId::ZapfDingbats => FontStyle::ZapfDingbats,
                    _ => FontStyle::Regular,
                };
                let abs_x = cx + x;
                let abs_y = baseline_y + y;
                let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                state.all_text.push_str(text);
                state.all_elements.push(PageElement::Text {
                    x: abs_x, y: abs_y, text_offset: offset,
                    text_len: text.len().min(65535) as u16,
                    font_size_100: (*font_size * 100.0) as u16,
                    font_style: style, color: *color, word_spacing_50: 0,
                });
            }
            math_layout::MathElement::Line { x1, y1, x2, y2, width, color } => {
                state.emit_line(cx + x1, baseline_y + y1, cx + x2, baseline_y + y2, *width, *color);
            }
        }
    }
}
