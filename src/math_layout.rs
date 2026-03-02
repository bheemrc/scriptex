/// Math layout engine: converts MathNode tree to positioned glyphs and lines
/// Custom recursive layout for fractions, superscripts, subscripts, radicals, etc.
///
/// All coordinates are relative to the insertion point. The caller translates
/// to absolute page position.

use crate::color::Color;
use crate::document::*;
use crate::font::{self, FontId};

/// A positioned math element ready for PDF rendering
#[derive(Debug, Clone)]
pub enum MathElement {
    /// Text at position (relative to math block origin)
    Text {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
        font_id: FontId,
        color: Color,
    },
    /// Horizontal line (for fractions, sqrt overline, etc.)
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: Color,
    },
}

/// Result of laying out a math expression
#[derive(Debug, Clone)]
pub struct MathBox {
    pub width: f32,
    pub height: f32,   // above baseline
    pub depth: f32,    // below baseline (positive = extends down)
    pub elements: Vec<MathElement>,
}

impl MathBox {
    fn empty() -> Self {
        MathBox { width: 0.0, height: 0.0, depth: 0.0, elements: Vec::new() }
    }

    /// Translate all elements by (dx, dy)
    fn translate(&mut self, dx: f32, dy: f32) {
        for elem in &mut self.elements {
            match elem {
                MathElement::Text { x, y, .. } => { *x += dx; *y += dy; }
                MathElement::Line { x1, y1, x2, y2, .. } => {
                    *x1 += dx; *y1 += dy; *x2 += dx; *y2 += dy;
                }
            }
        }
    }

    fn total_height(&self) -> f32 {
        self.height + self.depth
    }
}

/// Layout a list of math nodes at the given font size
pub fn layout_math(nodes: &[MathNode], font_size: f32) -> MathBox {
    let mut result = MathBox::empty();
    let mut x = 0.0f32;

    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];

        // Check for super/subscript following the current node
        let mut base = layout_math_node(node, font_size);

        // Look ahead for ^/_ modifiers
        let mut has_sup = false;
        let mut has_sub = false;
        let mut sup_box = MathBox::empty();
        let mut sub_box = MathBox::empty();

        while i + 1 < nodes.len() {
            match &nodes[i + 1] {
                MathNode::Super(sup_nodes) if !has_sup => {
                    sup_box = layout_math(sup_nodes, font_size * 0.7);
                    has_sup = true;
                    i += 1;
                }
                MathNode::Sub(sub_nodes) if !has_sub => {
                    sub_box = layout_math(sub_nodes, font_size * 0.7);
                    has_sub = true;
                    i += 1;
                }
                _ => break,
            }
        }

        if has_sup || has_sub {
            let combined = attach_scripts(&base, if has_sup { Some(&sup_box) } else { None },
                                           if has_sub { Some(&sub_box) } else { None },
                                           font_size);
            base = combined;
        }

        // Position this math box
        base.translate(x, 0.0);
        x += base.width;
        result.height = result.height.max(base.height);
        result.depth = result.depth.max(base.depth);
        result.elements.extend(base.elements);

        i += 1;
    }

    result.width = x;
    result
}

fn layout_math_node(node: &MathNode, font_size: f32) -> MathBox {
    match node {
        MathNode::Number(s) => layout_text(s, font_size, FontId::Helvetica),
        MathNode::Variable(c) => {
            let s = c.to_string();
            layout_text(&s, font_size, FontId::HelveticaOblique)
        }
        MathNode::Operator(s) => layout_operator(s, font_size),
        MathNode::Text(s) => layout_text(s, font_size, FontId::Helvetica),
        MathNode::Symbol(s) => layout_symbol(s, font_size),
        MathNode::Function(name) => layout_text(name, font_size, FontId::Helvetica),
        MathNode::Space(pts) => {
            MathBox {
                width: *pts,
                height: 0.0,
                depth: 0.0,
                elements: Vec::new(),
            }
        }
        MathNode::Group(nodes) => layout_math(nodes, font_size),

        MathNode::Frac { numer, denom } => layout_fraction(numer, denom, font_size),
        MathNode::Sqrt { index, radicand } => layout_sqrt(index.as_deref(), radicand, font_size),
        MathNode::Super(nodes) => {
            // Standalone superscript (no base)
            let mut sup = layout_math(nodes, font_size * 0.7);
            let shift = font_size * 0.4;
            sup.translate(0.0, -shift);
            sup.height = (sup.height + shift).max(0.0);
            sup
        }
        MathNode::Sub(nodes) => {
            let mut sub = layout_math(nodes, font_size * 0.7);
            let shift = font_size * 0.2;
            sub.translate(0.0, shift);
            sub.depth = (sub.depth + shift).max(0.0);
            sub
        }

        MathNode::Sum { lower, upper } => layout_large_op("\u{2211}", lower, upper, font_size),
        MathNode::Integral { lower, upper } => layout_large_op("\u{222B}", lower, upper, font_size),
        MathNode::Product { lower, upper } => layout_large_op("\u{220F}", lower, upper, font_size),

        MathNode::Left(delim) => layout_delimiter(delim, font_size),
        MathNode::Right(delim) => layout_delimiter(delim, font_size),

        MathNode::Matrix { rows, style } => layout_matrix(rows, *style, font_size),

        MathNode::Cases { rows } => layout_cases(rows, font_size),
        MathNode::Accent { base, accent_type } => layout_accent(base, accent_type, font_size),
        MathNode::Over { content, over_type } => layout_over(content, over_type, font_size),
        MathNode::Under { content, under_type } => layout_under(content, under_type, font_size),
        MathNode::Binom { top, bottom } => layout_binom(top, bottom, font_size),
        MathNode::Overset { over, base } => layout_overset(over, base, font_size),
        MathNode::Underset { under, base } => layout_underset(under, base, font_size),
        MathNode::OperatorName(name) => layout_text(name, font_size, FontId::Helvetica),
        MathNode::MathFont { font, content } => layout_math_font(font, content, font_size),
        MathNode::AlignmentMark => MathBox { width: 10.0, height: 0.0, depth: 0.0, elements: Vec::new() },
        MathNode::NewLine => MathBox { width: 0.0, height: font_size * 1.2, depth: 0.0, elements: Vec::new() },
        MathNode::Phantom(content) => {
            let inner = layout_math(content, font_size);
            MathBox { width: inner.width, height: inner.height, depth: inner.depth, elements: Vec::new() }
        }
        MathNode::StyleSwitch(_) => MathBox::empty(),
        MathNode::BigDelim { delim, size } => {
            let ds = font_size * size;
            layout_delimiter(delim, ds / 1.2) // undo the 1.2 in layout_delimiter
        }
    }
}

fn layout_text(text: &str, font_size: f32, font_id: FontId) -> MathBox {
    let width = font::measure_text(text, font_id, font_size);
    let info = font::font_info(font_id);
    let height = info.ascent as f32 * font_size * 0.001;
    let depth = (-info.descent as f32) * font_size * 0.001;

    MathBox {
        width,
        height,
        depth,
        elements: vec![MathElement::Text {
            x: 0.0,
            y: 0.0,
            text: text.to_string(),
            font_size,
            font_id,
            color: Color::BLACK,
        }],
    }
}

fn layout_operator(op: &str, font_size: f32) -> MathBox {
    let thin = font_size * 0.22;
    // Check if the operator contains a Symbol-font character (non-ASCII)
    if let Some(ch) = op.chars().next() {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let sym_width = font::char_width_pt(FontId::Symbol, sym_byte, font_size);
            let total_width = thin + sym_width.max(font_size * 0.4) + thin;
            let info = font::font_info(FontId::Symbol);
            return MathBox {
                width: total_width,
                height: info.ascent as f32 * font_size * 0.001,
                depth: (-info.descent as f32) * font_size * 0.001,
                elements: vec![MathElement::Text {
                    x: thin,
                    y: 0.0,
                    text: String::from(sym_byte as char),
                    font_size,
                    font_id: FontId::Symbol,
                    color: Color::BLACK,
                }],
            };
        }
    }
    // ASCII operators — different spacing rules per TeX math classes
    match op {
        // Opening delimiters: no extra space
        "(" | "[" => layout_text(op, font_size, FontId::Helvetica),
        // Closing delimiters: no extra space
        ")" | "]" => layout_text(op, font_size, FontId::Helvetica),
        // Punctuation: thin space after only
        "," => {
            let glyph = layout_text(op, font_size, FontId::Helvetica);
            MathBox {
                width: glyph.width + font_size * 0.17,
                height: glyph.height,
                depth: glyph.depth,
                elements: glyph.elements,
            }
        }
        ";" | ":" => {
            let glyph = layout_text(op, font_size, FontId::Helvetica);
            MathBox {
                width: glyph.width + font_size * 0.22,
                height: glyph.height,
                depth: glyph.depth,
                elements: glyph.elements,
            }
        }
        // Binary/relation operators: thin space on both sides
        _ => {
            let op_text = format!(" {} ", op);
            layout_text(&op_text, font_size, FontId::Helvetica)
        }
    }
}

fn layout_symbol(symbol: &str, font_size: f32) -> MathBox {
    // Try to map Unicode symbol to PDF Symbol font encoding
    if let Some(ch) = symbol.chars().next() {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let width = font::char_width_pt(FontId::Symbol, sym_byte, font_size);
            let info = font::font_info(FontId::Symbol);
            return MathBox {
                width: width.max(font_size * 0.4),
                height: info.ascent as f32 * font_size * 0.001,
                depth: (-info.descent as f32) * font_size * 0.001,
                elements: vec![MathElement::Text {
                    x: 0.0,
                    y: 0.0,
                    text: String::from(sym_byte as char),
                    font_size,
                    font_id: FontId::Symbol,
                    color: Color::BLACK,
                }],
            };
        }
    }
    // Fallback to Helvetica for unrecognized symbols
    let width = font::measure_text(symbol, FontId::Helvetica, font_size);
    let info = font::font_info(FontId::Helvetica);
    MathBox {
        width: width.max(font_size * 0.5),
        height: info.ascent as f32 * font_size * 0.001,
        depth: (-info.descent as f32) * font_size * 0.001,
        elements: vec![MathElement::Text {
            x: 0.0,
            y: 0.0,
            text: symbol.to_string(),
            font_size,
            font_id: FontId::Helvetica,
            color: Color::BLACK,
        }],
    }
}

fn layout_fraction(numer: &[MathNode], denom: &[MathNode], font_size: f32) -> MathBox {
    let frac_size = font_size * 0.85;
    let mut num_box = layout_math(numer, frac_size);
    let mut den_box = layout_math(denom, frac_size);

    let total_width = num_box.width.max(den_box.width) + font_size * 0.2;
    let bar_y = font_size * 0.3; // fraction bar at ~x-height/2
    let gap = font_size * 0.15;
    let rule_thickness = font_size * 0.04;

    // Center numerator above bar
    let num_x = (total_width - num_box.width) / 2.0;
    let num_y = -(bar_y + gap + num_box.depth);
    num_box.translate(num_x, num_y);

    // Center denominator below bar
    let den_x = (total_width - den_box.width) / 2.0;
    let den_y = -(bar_y - gap - den_box.height) + den_box.height;
    den_box.translate(den_x, den_y);

    let height = bar_y + gap + num_box.total_height();
    let depth = gap + den_box.total_height() - bar_y + rule_thickness;

    let mut elements = Vec::with_capacity(num_box.elements.len() + den_box.elements.len() + 1);
    elements.extend(num_box.elements);
    elements.extend(den_box.elements);
    // Fraction bar
    elements.push(MathElement::Line {
        x1: 0.0,
        y1: -bar_y,
        x2: total_width,
        y2: -bar_y,
        width: rule_thickness,
        color: Color::BLACK,
    });

    MathBox { width: total_width, height, depth, elements }
}

fn layout_sqrt(index: Option<&[MathNode]>, radicand: &[MathNode], font_size: f32) -> MathBox {
    let mut inner = layout_math(radicand, font_size);
    let radical_width = font_size * 0.5;
    let overline_gap = font_size * 0.1;
    let rule_thickness = font_size * 0.04;

    // Radical symbol — use Symbol font (byte 0xD6 = √)
    let radical_height = inner.height + overline_gap;

    let mut elements = Vec::new();
    // Radical sign using Symbol font encoding
    elements.push(MathElement::Text {
        x: 0.0,
        y: 0.0,
        text: String::from(0xD6 as char),
        font_size: font_size * 1.2,
        font_id: FontId::Symbol,
        color: Color::BLACK,
    });

    // Overline
    elements.push(MathElement::Line {
        x1: radical_width,
        y1: -(inner.height + overline_gap),
        x2: radical_width + inner.width + font_size * 0.1,
        y2: -(inner.height + overline_gap),
        width: rule_thickness,
        color: Color::BLACK,
    });

    // Inner content
    inner.translate(radical_width, 0.0);
    elements.extend(inner.elements);

    let mut result = MathBox {
        width: radical_width + inner.width + font_size * 0.15,
        height: radical_height + rule_thickness,
        depth: inner.depth,
        elements,
    };

    // Optional index (n-th root)
    if let Some(idx_nodes) = index {
        let mut idx = layout_math(idx_nodes, font_size * 0.5);
        idx.translate(font_size * 0.05, -(radical_height * 0.5));
        result.elements.extend(idx.elements);
    }

    result
}

fn attach_scripts(base: &MathBox, sup: Option<&MathBox>, sub: Option<&MathBox>, font_size: f32) -> MathBox {
    let mut elements = base.elements.clone();
    let mut width = base.width;
    let mut height = base.height;
    let mut depth = base.depth;

    let script_x = base.width + font_size * 0.05;

    if let Some(sup_box) = sup {
        let shift_up = font_size * 0.35;
        let mut s = sup_box.clone();
        s.translate(script_x, -shift_up);
        width = width.max(script_x + s.width);
        height = height.max(shift_up + s.height);
        elements.extend(s.elements);
    }

    if let Some(sub_box) = sub {
        let shift_down = font_size * 0.15;
        let mut s = sub_box.clone();
        s.translate(script_x, shift_down);
        width = width.max(script_x + s.width);
        depth = depth.max(shift_down + s.depth);
        elements.extend(s.elements);
    }

    MathBox { width, height, depth, elements }
}

fn layout_large_op(
    symbol: &str,
    lower: &Option<Vec<MathNode>>,
    upper: &Option<Vec<MathNode>>,
    font_size: f32,
) -> MathBox {
    let op_size = font_size * 1.5;

    // Use Symbol font for large operators
    let (op_text, op_font, op_width) = if let Some(ch) = symbol.chars().next() {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let w = font::char_width_pt(FontId::Symbol, sym_byte, op_size).max(font_size * 0.8);
            (String::from(sym_byte as char), FontId::Symbol, w)
        } else {
            let w = font::measure_text(symbol, FontId::Helvetica, op_size).max(font_size * 0.8);
            (symbol.to_string(), FontId::Helvetica, w)
        }
    } else {
        let w = font::measure_text(symbol, FontId::Helvetica, op_size).max(font_size * 0.8);
        (symbol.to_string(), FontId::Helvetica, w)
    };

    let mut elements = vec![MathElement::Text {
        x: 0.0,
        y: 0.0,
        text: op_text,
        font_size: op_size,
        font_id: op_font,
        color: Color::BLACK,
    }];

    let mut height = op_size * 0.7;
    let mut depth = op_size * 0.2;
    let mut width = op_width;

    // Upper limit
    if let Some(upper_nodes) = upper {
        let mut upper_box = layout_math(upper_nodes, font_size * 0.6);
        let ux = (op_width - upper_box.width) / 2.0;
        let uy = -(height + font_size * 0.1);
        upper_box.translate(ux.max(0.0), uy);
        height += upper_box.total_height() + font_size * 0.1;
        width = width.max(upper_box.width);
        elements.extend(upper_box.elements);
    }

    // Lower limit
    if let Some(lower_nodes) = lower {
        let mut lower_box = layout_math(lower_nodes, font_size * 0.6);
        let lx = (op_width - lower_box.width) / 2.0;
        let ly = depth + font_size * 0.1 + lower_box.height;
        lower_box.translate(lx.max(0.0), ly);
        depth += lower_box.total_height() + font_size * 0.1;
        width = width.max(lower_box.width);
        elements.extend(lower_box.elements);
    }

    // Add spacing after operator
    width += font_size * 0.2;

    MathBox { width, height, depth, elements }
}

fn layout_delimiter(delim: &str, font_size: f32) -> MathBox {
    if delim == "." {
        // Invisible delimiter
        return MathBox { width: 0.0, height: font_size * 0.7, depth: font_size * 0.2, elements: Vec::new() };
    }

    // Check if delimiter maps to Symbol font
    let unicode_ch = match delim {
        "\\langle" => Some('\u{27E8}'),
        "\\rangle" => Some('\u{27E9}'),
        "\\lfloor" => Some('\u{230A}'),
        "\\rfloor" => Some('\u{230B}'),
        "\\lceil" => Some('\u{2308}'),
        "\\rceil" => Some('\u{2309}'),
        _ => None,
    };

    if let Some(ch) = unicode_ch {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let ds = font_size * 1.2;
            let width = font::char_width_pt(FontId::Symbol, sym_byte, ds);
            let info = font::font_info(FontId::Symbol);
            return MathBox {
                width,
                height: info.ascent as f32 * ds * 0.001,
                depth: (-info.descent as f32) * ds * 0.001,
                elements: vec![MathElement::Text {
                    x: 0.0, y: 0.0,
                    text: String::from(sym_byte as char),
                    font_size: ds,
                    font_id: FontId::Symbol,
                    color: Color::BLACK,
                }],
            };
        }
    }

    let text = match delim {
        "(" | ")" | "[" | "]" | "|" => delim.to_string(),
        "\\{" => "{".to_string(),
        "\\}" => "}".to_string(),
        _ => delim.to_string(),
    };

    layout_text(&text, font_size * 1.2, FontId::Helvetica)
}

fn layout_matrix(
    rows: &[Vec<Vec<MathNode>>],
    style: MatrixStyle,
    font_size: f32,
) -> MathBox {
    if rows.is_empty() {
        return MathBox::empty();
    }

    let cell_size = font_size * 0.85;
    let col_gap = font_size * 0.8;
    let row_gap = font_size * 0.5;

    // Layout all cells
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut cell_boxes: Vec<Vec<MathBox>> = Vec::new();
    let mut col_widths = vec![0.0f32; num_cols];

    for row in rows {
        let mut row_boxes = Vec::new();
        for (j, cell) in row.iter().enumerate() {
            let cell_box = layout_math(cell, cell_size);
            if j < num_cols {
                col_widths[j] = col_widths[j].max(cell_box.width);
            }
            row_boxes.push(cell_box);
        }
        cell_boxes.push(row_boxes);
    }

    let row_height = font_size * 1.2;
    let total_width: f32 = col_widths.iter().sum::<f32>() + col_gap * (num_cols.max(1) - 1) as f32;
    let total_height = row_height * rows.len() as f32 + row_gap * (rows.len().max(1) - 1) as f32;

    let mut elements = Vec::new();
    let mut y = -(total_height / 2.0 - font_size * 0.3); // center vertically

    for row_boxes in &cell_boxes {
        let mut x = 0.0;
        for (j, cell_box) in row_boxes.iter().enumerate() {
            let col_w = if j < col_widths.len() { col_widths[j] } else { cell_box.width };
            let cx = x + (col_w - cell_box.width) / 2.0; // center in column
            let mut shifted = cell_box.clone();
            shifted.translate(cx, y);
            elements.extend(shifted.elements);
            x += col_w + col_gap;
        }
        y += row_height + row_gap;
    }

    // Add delimiters based on style
    let delim_size = total_height * 0.8;
    let delim_pad = font_size * 0.15; // gap between delimiter and content
    let has_delimiters = !matches!(style, MatrixStyle::Plain);
    // Approximate delimiter glyph width (parenthesis/bracket at large size)
    let delim_width = if has_delimiters { delim_size * 0.35 } else { 0.0 };

    // Shift all cell content right to make room for the left delimiter
    if has_delimiters {
        let shift = delim_width + delim_pad;
        for elem in &mut elements {
            match elem {
                MathElement::Text { x, .. } => *x += shift,
                MathElement::Line { x1, x2, .. } => { *x1 += shift; *x2 += shift; }
            }
        }
    }

    let content_start = if has_delimiters { delim_width + delim_pad } else { 0.0 };
    let content_width = content_start + total_width + if has_delimiters { delim_pad + delim_width } else { 0.0 };

    match style {
        MatrixStyle::Parenthesized => {
            elements.push(MathElement::Text {
                x: 0.0, y: 0.0, text: "(".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
            elements.push(MathElement::Text {
                x: content_start + total_width + delim_pad, y: 0.0, text: ")".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        MatrixStyle::Bracketed => {
            elements.push(MathElement::Text {
                x: 0.0, y: 0.0, text: "[".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
            elements.push(MathElement::Text {
                x: content_start + total_width + delim_pad, y: 0.0, text: "]".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        MatrixStyle::Braced => {
            elements.push(MathElement::Text {
                x: 0.0, y: 0.0, text: "{".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
            elements.push(MathElement::Text {
                x: content_start + total_width + delim_pad, y: 0.0, text: "}".to_string(),
                font_size: delim_size, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        MatrixStyle::VerticalBar => {
            elements.push(MathElement::Line {
                x1: delim_width * 0.5, y1: -(total_height / 2.0),
                x2: delim_width * 0.5, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: content_start + total_width + delim_pad + delim_width * 0.5, y1: -(total_height / 2.0),
                x2: content_start + total_width + delim_pad + delim_width * 0.5, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
        }
        MatrixStyle::DoubleBar => {
            let bar_gap = font_size * 0.08;
            elements.push(MathElement::Line {
                x1: delim_width * 0.4, y1: -(total_height / 2.0),
                x2: delim_width * 0.4, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: delim_width * 0.4 + bar_gap, y1: -(total_height / 2.0),
                x2: delim_width * 0.4 + bar_gap, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
            let rx = content_start + total_width + delim_pad;
            elements.push(MathElement::Line {
                x1: rx + delim_width * 0.4, y1: -(total_height / 2.0),
                x2: rx + delim_width * 0.4, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: rx + delim_width * 0.4 + bar_gap, y1: -(total_height / 2.0),
                x2: rx + delim_width * 0.4 + bar_gap, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
        }
        MatrixStyle::Plain => {}
    }

    MathBox {
        width: content_width,
        height: total_height / 2.0 + font_size * 0.3,
        depth: total_height / 2.0 - font_size * 0.3,
        elements,
    }
}

fn layout_accent(base: &[MathNode], accent_type: &AccentType, font_size: f32) -> MathBox {
    let mut base_box = layout_math(base, font_size);
    let accent_y = -(base_box.height + font_size * 0.05);

    let accent_elem = match accent_type {
        AccentType::Hat => MathElement::Text {
            x: base_box.width * 0.2, y: accent_y, text: "^".to_string(),
            font_size: font_size * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Bar => MathElement::Line {
            x1: 0.0, y1: accent_y, x2: base_box.width, y2: accent_y,
            width: font_size * 0.04, color: Color::BLACK,
        },
        AccentType::Vec => MathElement::Text {
            // Use Symbol font → (arrow right, 0xAE)
            x: base_box.width * 0.1, y: accent_y,
            text: String::from(0xAE as char),
            font_size: font_size * 0.6, font_id: FontId::Symbol, color: Color::BLACK,
        },
        AccentType::Tilde => MathElement::Text {
            // Use Symbol font ~ (0x7E) for a proper tilde
            x: base_box.width * 0.15, y: accent_y,
            text: String::from(0x7E as char),
            font_size: font_size * 0.8, font_id: FontId::Symbol, color: Color::BLACK,
        },
        AccentType::Dot => MathElement::Text {
            x: base_box.width * 0.4, y: accent_y, text: "\u{00B7}".to_string(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::DDot => MathElement::Text {
            x: base_box.width * 0.25, y: accent_y, text: "\u{00A8}".to_string(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Breve => MathElement::Text {
            x: base_box.width * 0.15, y: accent_y, text: "\u{00A8}".to_string(),
            font_size: font_size * 0.6, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Check => MathElement::Text {
            // Inverted hat / caron
            x: base_box.width * 0.2, y: accent_y + font_size * 0.15,
            text: "v".to_string(),
            font_size: font_size * 0.5, font_id: FontId::Helvetica, color: Color::BLACK,
        },
    };

    base_box.height += font_size * 0.2;
    base_box.elements.push(accent_elem);
    base_box
}

fn layout_over(content: &[MathNode], over_type: &OverType, font_size: f32) -> MathBox {
    let mut content_box = layout_math(content, font_size);
    let y = -(content_box.height + font_size * 0.05);

    match over_type {
        OverType::Line => {
            content_box.elements.push(MathElement::Line {
                x1: 0.0, y1: y, x2: content_box.width, y2: y,
                width: font_size * 0.04, color: Color::BLACK,
            });
        }
        OverType::Brace => {
            content_box.elements.push(MathElement::Text {
                x: 0.0, y: y - font_size * 0.1, text: "\u{23DE}".to_string(),
                font_size: font_size * 0.5, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        OverType::Arrow => {
            content_box.elements.push(MathElement::Text {
                x: 0.0, y: y, text: "\u{2192}".to_string(),
                font_size: font_size * 0.6, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
    }

    content_box.height += font_size * 0.15;
    content_box
}

fn layout_under(content: &[MathNode], under_type: &UnderType, font_size: f32) -> MathBox {
    let mut content_box = layout_math(content, font_size);
    let y = content_box.depth + font_size * 0.05;

    match under_type {
        UnderType::Line => {
            content_box.elements.push(MathElement::Line {
                x1: 0.0, y1: y, x2: content_box.width, y2: y,
                width: font_size * 0.04, color: Color::BLACK,
            });
        }
        UnderType::Brace => {
            content_box.elements.push(MathElement::Text {
                x: 0.0, y: y + font_size * 0.1, text: "\u{23DF}".to_string(),
                font_size: font_size * 0.5, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
    }

    content_box.depth += font_size * 0.15;
    content_box
}

fn layout_cases(rows: &[(Vec<MathNode>, Option<Vec<MathNode>>)], font_size: f32) -> MathBox {
    if rows.is_empty() {
        return MathBox::empty();
    }

    let row_height = font_size * 1.4;
    let col_gap = font_size * 1.0;
    let brace_width = font_size * 0.5;

    // Layout all rows
    let mut value_boxes: Vec<MathBox> = Vec::new();
    let mut cond_boxes: Vec<Option<MathBox>> = Vec::new();
    let mut max_val_w = 0.0f32;
    let mut max_cond_w = 0.0f32;

    for (value, cond) in rows {
        let vb = layout_math(value, font_size);
        max_val_w = max_val_w.max(vb.width);
        value_boxes.push(vb);
        if let Some(c) = cond {
            let cb = layout_math(c, font_size);
            max_cond_w = max_cond_w.max(cb.width);
            cond_boxes.push(Some(cb));
        } else {
            cond_boxes.push(None);
        }
    }

    let total_height = row_height * rows.len() as f32;
    let content_width = max_val_w + col_gap + max_cond_w;
    let total_width = brace_width + content_width;

    let mut elements = Vec::new();

    // Left brace using Symbol font
    if let Some(sym_byte) = font::unicode_to_symbol_byte('{' as char) {
        let _ = sym_byte; // '{' is not in Symbol; use Helvetica
    }
    elements.push(MathElement::Text {
        x: 0.0, y: 0.0,
        text: "{".to_string(),
        font_size: total_height * 0.7,
        font_id: FontId::Helvetica,
        color: Color::BLACK,
    });

    // Layout rows
    let start_y = -(total_height / 2.0 - font_size * 0.3);
    for (i, (vb, cb)) in value_boxes.iter().zip(cond_boxes.iter()).enumerate() {
        let y = start_y + i as f32 * row_height;

        let mut shifted_v = vb.clone();
        shifted_v.translate(brace_width, y);
        elements.extend(shifted_v.elements);

        if let Some(cond_box) = cb {
            let mut shifted_c = cond_box.clone();
            shifted_c.translate(brace_width + max_val_w + col_gap, y);
            elements.extend(shifted_c.elements);
        }
    }

    MathBox {
        width: total_width,
        height: total_height / 2.0 + font_size * 0.3,
        depth: total_height / 2.0 - font_size * 0.3,
        elements,
    }
}

fn layout_binom(top: &[MathNode], bottom: &[MathNode], font_size: f32) -> MathBox {
    let inner_size = font_size * 0.85;
    let mut top_box = layout_math(top, inner_size);
    let mut bot_box = layout_math(bottom, inner_size);

    let inner_width = top_box.width.max(bot_box.width) + font_size * 0.2;
    let gap = font_size * 0.15;
    let center_y = font_size * 0.3;

    // Center top above center
    let top_x = (inner_width - top_box.width) / 2.0;
    top_box.translate(top_x, -(center_y + gap + top_box.depth));

    // Center bottom below center
    let bot_x = (inner_width - bot_box.width) / 2.0;
    bot_box.translate(bot_x, -(center_y - gap - bot_box.height) + bot_box.height);

    let height = center_y + gap + top_box.total_height();
    let depth = gap + bot_box.total_height() - center_y;
    let paren_size = (height + depth) * 0.8;

    let mut elements = Vec::with_capacity(top_box.elements.len() + bot_box.elements.len() + 2);

    // Left paren
    elements.push(MathElement::Text {
        x: -font_size * 0.15, y: 0.0,
        text: "(".to_string(),
        font_size: paren_size,
        font_id: FontId::Helvetica,
        color: Color::BLACK,
    });

    elements.extend(top_box.elements);
    elements.extend(bot_box.elements);

    // Right paren
    elements.push(MathElement::Text {
        x: inner_width + font_size * 0.05, y: 0.0,
        text: ")".to_string(),
        font_size: paren_size,
        font_id: FontId::Helvetica,
        color: Color::BLACK,
    });

    let paren_w = font_size * 0.3;
    MathBox { width: inner_width + paren_w * 2.0, height, depth, elements }
}

fn layout_overset(over: &[MathNode], base: &[MathNode], font_size: f32) -> MathBox {
    let mut base_box = layout_math(base, font_size);
    let mut over_box = layout_math(over, font_size * 0.6);

    let total_width = base_box.width.max(over_box.width);
    let gap = font_size * 0.1;

    // Center over above base
    let over_x = (total_width - over_box.width) / 2.0;
    let over_y = -(base_box.height + gap + over_box.depth);
    over_box.translate(over_x, over_y);

    let base_x = (total_width - base_box.width) / 2.0;
    base_box.translate(base_x, 0.0);

    let height = base_box.height + gap + over_box.total_height();

    let mut elements = Vec::new();
    elements.extend(base_box.elements);
    elements.extend(over_box.elements);

    MathBox { width: total_width, height, depth: base_box.depth, elements }
}

fn layout_underset(under: &[MathNode], base: &[MathNode], font_size: f32) -> MathBox {
    let mut base_box = layout_math(base, font_size);
    let mut under_box = layout_math(under, font_size * 0.6);

    let total_width = base_box.width.max(under_box.width);
    let gap = font_size * 0.1;

    let under_x = (total_width - under_box.width) / 2.0;
    let under_y = base_box.depth + gap + under_box.height;
    under_box.translate(under_x, under_y);

    let base_x = (total_width - base_box.width) / 2.0;
    base_box.translate(base_x, 0.0);

    let depth = base_box.depth + gap + under_box.total_height();

    let mut elements = Vec::new();
    elements.extend(base_box.elements);
    elements.extend(under_box.elements);

    MathBox { width: total_width, height: base_box.height, depth, elements }
}

fn layout_math_font(font: &MathFontType, content: &[MathNode], font_size: f32) -> MathBox {
    // Map math fonts to closest available Standard 14 font
    let font_id = match font {
        MathFontType::Blackboard => FontId::HelveticaBold,
        MathFontType::Calligraphic | MathFontType::Script => FontId::HelveticaOblique,
        MathFontType::Fraktur => FontId::HelveticaBoldOblique,
        MathFontType::SansSerif => FontId::Helvetica,
        MathFontType::BoldMath => FontId::HelveticaBold,
    };

    // Re-layout content with the target font
    let mut result = MathBox::empty();
    let mut x = 0.0f32;
    for node in content {
        let mut mb = match node {
            MathNode::Variable(c) => layout_text(&c.to_string(), font_size, font_id),
            MathNode::Text(s) => layout_text(s, font_size, font_id),
            _ => layout_math_node(node, font_size),
        };
        mb.translate(x, 0.0);
        x += mb.width;
        result.height = result.height.max(mb.height);
        result.depth = result.depth.max(mb.depth);
        result.elements.extend(mb.elements);
    }
    result.width = x;
    result
}

// MatrixStyle Clone/Copy already derived in document.rs
