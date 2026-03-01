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
        MathNode::Operator(s) => {
            let op_text = format!(" {} ", s);
            layout_text(&op_text, font_size, FontId::Helvetica)
        }
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

        MathNode::Accent { base, accent_type } => layout_accent(base, accent_type, font_size),
        MathNode::Over { content, over_type } => layout_over(content, over_type, font_size),
        MathNode::Under { content, under_type } => layout_under(content, under_type, font_size),
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

fn layout_symbol(symbol: &str, font_size: f32) -> MathBox {
    // Try to render symbols using the Symbol font or as text
    // Many Greek/math symbols are in the WinAnsi range or Symbol font
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

    // Radical symbol
    let radical_text = "\u{221A}";
    let radical_height = inner.height + overline_gap;

    let mut elements = Vec::new();
    // Radical sign
    elements.push(MathElement::Text {
        x: 0.0,
        y: 0.0,
        text: radical_text.to_string(),
        font_size: font_size * 1.2,
        font_id: FontId::Helvetica,
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
    let op_width = font::measure_text(symbol, FontId::Helvetica, op_size).max(font_size * 0.8);

    let mut elements = vec![MathElement::Text {
        x: 0.0,
        y: 0.0,
        text: symbol.to_string(),
        font_size: op_size,
        font_id: FontId::Helvetica,
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
    let text = match delim {
        "(" | ")" | "[" | "]" | "|" | "." => delim.to_string(),
        "\\{" => "{".to_string(),
        "\\}" => "}".to_string(),
        "\\langle" => "\u{27E8}".to_string(),
        "\\rangle" => "\u{27E9}".to_string(),
        "\\lfloor" => "\u{230A}".to_string(),
        "\\rfloor" => "\u{230B}".to_string(),
        "\\lceil" => "\u{2308}".to_string(),
        "\\rceil" => "\u{2309}".to_string(),
        _ => delim.to_string(),
    };

    if text == "." {
        // Invisible delimiter
        return MathBox { width: 0.0, height: font_size * 0.7, depth: font_size * 0.2, elements: Vec::new() };
    }

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
    let delim_pad = font_size * 0.2;
    let content_width = total_width + delim_pad * 2.0;

    match style {
        MatrixStyle::Parenthesized => {
            elements.push(MathElement::Text {
                x: -delim_pad, y: 0.0, text: "(".to_string(),
                font_size: total_height * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
            });
            elements.push(MathElement::Text {
                x: total_width + delim_pad * 0.5, y: 0.0, text: ")".to_string(),
                font_size: total_height * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        MatrixStyle::Bracketed => {
            elements.push(MathElement::Text {
                x: -delim_pad, y: 0.0, text: "[".to_string(),
                font_size: total_height * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
            });
            elements.push(MathElement::Text {
                x: total_width + delim_pad * 0.5, y: 0.0, text: "]".to_string(),
                font_size: total_height * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
            });
        }
        MatrixStyle::VerticalBar => {
            elements.push(MathElement::Line {
                x1: -delim_pad, y1: -(total_height / 2.0),
                x2: -delim_pad, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: total_width + delim_pad, y1: -(total_height / 2.0),
                x2: total_width + delim_pad, y2: total_height / 2.0,
                width: font_size * 0.04, color: Color::BLACK,
            });
        }
        _ => {}
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
            x: base_box.width * 0.3, y: accent_y, text: "\u{0302}".to_string(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Bar => MathElement::Line {
            x1: 0.0, y1: accent_y, x2: base_box.width, y2: accent_y,
            width: font_size * 0.04, color: Color::BLACK,
        },
        AccentType::Vec => MathElement::Text {
            x: base_box.width * 0.2, y: accent_y, text: "\u{2192}".to_string(),
            font_size: font_size * 0.6, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Tilde => MathElement::Text {
            x: base_box.width * 0.2, y: accent_y, text: "~".to_string(),
            font_size: font_size * 0.8, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::Dot => MathElement::Text {
            x: base_box.width * 0.4, y: accent_y, text: "\u{00B7}".to_string(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        AccentType::DDot => MathElement::Text {
            x: base_box.width * 0.25, y: accent_y, text: "\u{00A8}".to_string(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
        },
        _ => MathElement::Text {
            x: 0.0, y: accent_y, text: String::new(),
            font_size, font_id: FontId::Helvetica, color: Color::BLACK,
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

// MatrixStyle Clone/Copy already derived in document.rs
