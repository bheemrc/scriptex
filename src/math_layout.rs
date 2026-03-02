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
    let mut current_size = font_size;

    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];

        // Handle style switches by adjusting font size
        if let MathNode::StyleSwitch(style) = node {
            current_size = match style {
                MathStyleType::Display => font_size * 1.2, // display style: larger
                MathStyleType::Text => font_size,          // text style: normal
                MathStyleType::Script => font_size * 0.7,  // script: smaller
                MathStyleType::ScriptScript => font_size * 0.5, // scriptscript: smallest
            };
            i += 1;
            continue;
        }

        // Check for super/subscript following the current node
        let mut base = layout_math_node(node, current_size);

        // Look ahead for ^/_ modifiers
        let mut has_sup = false;
        let mut has_sub = false;
        let mut sup_box = MathBox::empty();
        let mut sub_box = MathBox::empty();

        while i + 1 < nodes.len() {
            match &nodes[i + 1] {
                MathNode::Super(sup_nodes) if !has_sup => {
                    sup_box = layout_math(sup_nodes, current_size * 0.7);
                    has_sup = true;
                    i += 1;
                }
                MathNode::Sub(sub_nodes) if !has_sub => {
                    sub_box = layout_math(sub_nodes, current_size * 0.7);
                    has_sub = true;
                    i += 1;
                }
                _ => break,
            }
        }

        if has_sup || has_sub {
            let combined = attach_scripts(&base, if has_sup { Some(&sup_box) } else { None },
                                           if has_sub { Some(&sub_box) } else { None },
                                           current_size);
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
        MathNode::Number(s) => layout_text(s, font_size, FontId::TimesRoman),
        MathNode::Variable(c) => {
            let s = c.to_string();
            layout_text(&s, font_size, FontId::TimesItalic)
        }
        MathNode::Operator(s) => layout_operator(s, font_size),
        MathNode::Text(s) => layout_text(s, font_size, FontId::TimesRoman),
        MathNode::Symbol(s) => layout_symbol(s, font_size),
        MathNode::Function(name) => layout_text(name, font_size, FontId::TimesRoman),
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

        MathNode::Left(delim) => layout_delimiter(delim, font_size, font_size * 0.7, font_size * 0.2),
        MathNode::Right(delim) => layout_delimiter(delim, font_size, font_size * 0.7, font_size * 0.2),
        MathNode::DelimitedGroup { left, right, content } => layout_delimited_group(left, right, content, font_size),

        MathNode::Matrix { rows, style } => layout_matrix(rows, *style, font_size),

        MathNode::Cases { rows } => layout_cases(rows, font_size),
        MathNode::Accent { base, accent_type } => layout_accent(base, accent_type, font_size),
        MathNode::Over { content, over_type } => layout_over(content, over_type, font_size),
        MathNode::Under { content, under_type } => layout_under(content, under_type, font_size),
        MathNode::Binom { top, bottom } => layout_binom(top, bottom, font_size),
        MathNode::Overset { over, base } => layout_overset(over, base, font_size),
        MathNode::Underset { under, base } => layout_underset(under, base, font_size),
        MathNode::OperatorName(name) => layout_text(name, font_size, FontId::TimesRoman),
        MathNode::MathFont { font, content } => layout_math_font(font, content, font_size),
        MathNode::AlignmentMark => MathBox { width: 10.0, height: 0.0, depth: 0.0, elements: Vec::new() },
        MathNode::NewLine => MathBox { width: 0.0, height: font_size * 1.2, depth: 0.0, elements: Vec::new() },
        MathNode::Phantom(content) => {
            let inner = layout_math(content, font_size);
            MathBox { width: inner.width, height: inner.height, depth: inner.depth, elements: Vec::new() }
        }
        MathNode::LimitOp { name, lower, upper } => {
            // Layout like a large operator but with text name
            let name_box = layout_text(name, font_size, FontId::TimesRoman);
            let mut result = MathBox { width: 0.0, height: 0.0, depth: 0.0, elements: Vec::new() };
            let mut total_w = name_box.width;
            let sub_size = font_size * 0.65;
            let sub_box = lower.as_ref().map(|l| layout_math(l, sub_size));
            let sup_box = upper.as_ref().map(|u| layout_math(u, sub_size));
            if let Some(ref sb) = sub_box { total_w = total_w.max(sb.width); }
            if let Some(ref sb) = sup_box { total_w = total_w.max(sb.width); }

            let op_x = (total_w - name_box.width) * 0.5;
            let mut shifted_name = name_box.clone();
            shifted_name.translate(op_x, 0.0);

            result.height = name_box.height;
            result.depth = name_box.depth;
            result.elements.extend(shifted_name.elements);

            if let Some(mut sb) = sup_box {
                let sx = (total_w - sb.width) * 0.5;
                let sy = -(name_box.height + sb.depth + 1.0);
                sb.translate(sx, sy);
                result.height = name_box.height + sb.height + sb.depth + 1.0;
                result.elements.extend(sb.elements);
            }
            if let Some(mut sb) = sub_box {
                let sx = (total_w - sb.width) * 0.5;
                let sy = name_box.depth + sb.height + 1.0;
                sb.translate(sx, sy);
                result.depth = name_box.depth + sb.height + sb.depth + 1.0;
                result.elements.extend(sb.elements);
            }
            result.width = total_w;
            result
        }
        MathNode::Boxed(content) => {
            let inner = layout_math(content, font_size);
            let pad = font_size * 0.15;
            let bw = inner.width + 2.0 * pad;
            let bh = inner.height + inner.depth + 2.0 * pad;
            let mut result = MathBox {
                width: bw,
                height: inner.height + pad,
                depth: inner.depth + pad,
                elements: Vec::new(),
            };
            // Draw box frame (4 lines)
            let x0 = 0.0;
            let y_top = -(inner.height + pad);
            let x1 = bw;
            let y_bot = inner.depth + pad;
            let lw = 0.4;
            result.elements.push(MathElement::Line { x1: x0, y1: y_top, x2: x1, y2: y_top, width: lw, color: Color::BLACK }); // top
            result.elements.push(MathElement::Line { x1: x0, y1: y_bot, x2: x1, y2: y_bot, width: lw, color: Color::BLACK }); // bottom
            result.elements.push(MathElement::Line { x1: x0, y1: y_top, x2: x0, y2: y_bot, width: lw, color: Color::BLACK }); // left
            result.elements.push(MathElement::Line { x1: x1, y1: y_top, x2: x1, y2: y_bot, width: lw, color: Color::BLACK }); // right
            // Shift inner content by padding
            let mut shifted = inner;
            shifted.translate(pad, 0.0);
            result.elements.extend(shifted.elements);
            result
        }
        MathNode::StyledText(text, font_id) => layout_text(text, font_size, *font_id),
        MathNode::Substack(rows) => {
            // Vertically stacked lines, typically in subscript size
            let sub_size = font_size * 0.7;
            let line_h = sub_size * 1.3;
            let mut max_w = 0.0f32;
            let mut row_boxes: Vec<MathBox> = Vec::new();
            for row in rows {
                let mb = layout_math(row, sub_size);
                max_w = max_w.max(mb.width);
                row_boxes.push(mb);
            }
            let total_h = line_h * row_boxes.len() as f32;
            let mut result = MathBox { width: max_w, height: total_h * 0.5, depth: total_h * 0.5, elements: Vec::new() };
            let start_y = -total_h * 0.5 + sub_size;
            for (i, mut mb) in row_boxes.into_iter().enumerate() {
                let cx = (max_w - mb.width) / 2.0; // center each row
                mb.translate(cx, start_y + i as f32 * line_h);
                result.elements.extend(mb.elements);
            }
            result
        }
        MathNode::Label(_) => MathBox::empty(), // Labels are handled during prescan/layout
        MathNode::StyleSwitch(_) | MathNode::NoTag | MathNode::Tag(_) | MathNode::Intertext(_) => MathBox::empty(),
        MathNode::BigDelim { delim, size } => {
            let ds = font_size * size;
            let h = ds * 0.7;
            let d = ds * 0.2;
            layout_delimiter(delim, ds, h, d)
        }
        MathNode::VPhantom(content) => {
            let inner = layout_math(content, font_size);
            MathBox { width: 0.0, height: inner.height, depth: inner.depth, elements: Vec::new() }
        }
        MathNode::HPhantom(content) => {
            let inner = layout_math(content, font_size);
            MathBox { width: inner.width, height: 0.0, depth: 0.0, elements: Vec::new() }
        }
        MathNode::Pmod(content) => {
            // "(mod X)" with thin space before
            let thin = font_size * 0.22;
            let mut result = MathBox::empty();
            let mut x = thin; // thin space before

            // Opening paren
            let lp = layout_text("(", font_size, FontId::TimesRoman);
            let mut lp_shifted = lp.clone();
            lp_shifted.translate(x, 0.0);
            result.elements.extend(lp_shifted.elements);
            x += lp.width;

            // "mod" in upright
            let mod_box = layout_text("mod", font_size, FontId::TimesRoman);
            let mut mod_shifted = mod_box.clone();
            mod_shifted.translate(x, 0.0);
            result.elements.extend(mod_shifted.elements);
            x += mod_box.width + font_size * 0.17; // thin space after "mod"

            // Content
            let content_box = layout_math(content, font_size);
            let mut content_shifted = content_box.clone();
            content_shifted.translate(x, 0.0);
            result.height = result.height.max(content_box.height).max(mod_box.height);
            result.depth = result.depth.max(content_box.depth).max(mod_box.depth);
            result.elements.extend(content_shifted.elements);
            x += content_box.width;

            // Closing paren
            let rp = layout_text(")", font_size, FontId::TimesRoman);
            let mut rp_shifted = rp.clone();
            rp_shifted.translate(x, 0.0);
            result.elements.extend(rp_shifted.elements);
            x += rp.width;

            result.width = x;
            result
        }
        MathNode::Pod(content) => {
            // "(X)" with thin space before — like \pmod but no "mod"
            let thin = font_size * 0.22;
            let mut result = MathBox::empty();
            let mut x = thin;

            let lp = layout_text("(", font_size, FontId::TimesRoman);
            let mut lp_s = lp.clone();
            lp_s.translate(x, 0.0);
            result.elements.extend(lp_s.elements);
            x += lp.width;

            let content_box = layout_math(content, font_size);
            let mut c_s = content_box.clone();
            c_s.translate(x, 0.0);
            result.height = content_box.height.max(lp.height);
            result.depth = content_box.depth.max(lp.depth);
            result.elements.extend(c_s.elements);
            x += content_box.width;

            let rp = layout_text(")", font_size, FontId::TimesRoman);
            let mut rp_s = rp.clone();
            rp_s.translate(x, 0.0);
            result.elements.extend(rp_s.elements);
            x += rp.width;

            result.width = x;
            result
        }
        MathNode::Bmod => {
            // "mod" as binary operator with medium space on each side
            let med = font_size * 0.22;
            let mod_box = layout_text("mod", font_size, FontId::TimesRoman);
            let total = med + mod_box.width + med;
            let mut shifted = mod_box.clone();
            shifted.translate(med, 0.0);
            MathBox {
                width: total,
                height: mod_box.height,
                depth: mod_box.depth,
                elements: shifted.elements,
            }
        }
        MathNode::MathRel(content) => {
            // Relation spacing: thick space on each side (5mu ≈ 0.28em)
            let thick = font_size * 0.28;
            let inner = layout_math(content, font_size);
            let total = thick + inner.width + thick;
            let mut shifted = inner;
            shifted.translate(thick, 0.0);
            MathBox { width: total, height: shifted.height, depth: shifted.depth, elements: shifted.elements }
        }
        MathNode::MathBin(content) => {
            // Binary operator spacing: medium space on each side (4mu ≈ 0.22em)
            let med = font_size * 0.22;
            let inner = layout_math(content, font_size);
            let total = med + inner.width + med;
            let mut shifted = inner;
            shifted.translate(med, 0.0);
            MathBox { width: total, height: shifted.height, depth: shifted.depth, elements: shifted.elements }
        }
        MathNode::Rule { width, height } => {
            if *width == 0.0 {
                // Strut — invisible height spacer
                MathBox { width: 0.0, height: *height, depth: 0.0, elements: Vec::new() }
            } else {
                // Filled rectangle
                MathBox {
                    width: *width,
                    height: *height,
                    depth: 0.0,
                    elements: vec![MathElement::Line {
                        x1: 0.0, y1: -height / 2.0,
                        x2: *width, y2: -height / 2.0,
                        width: *height,
                        color: Color::BLACK,
                    }],
                }
            }
        }
        MathNode::Middle(delim) => {
            // Middle delimiter — render at current size
            let h = font_size * 0.7;
            let d = font_size * 0.2;
            layout_delimiter(delim, font_size, h, d)
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
        "(" | "[" => layout_text(op, font_size, FontId::TimesRoman),
        // Closing delimiters: no extra space
        ")" | "]" => layout_text(op, font_size, FontId::TimesRoman),
        // Punctuation: thin space after only
        "," => {
            let glyph = layout_text(op, font_size, FontId::TimesRoman);
            MathBox {
                width: glyph.width + font_size * 0.17,
                height: glyph.height,
                depth: glyph.depth,
                elements: glyph.elements,
            }
        }
        ";" | ":" => {
            let glyph = layout_text(op, font_size, FontId::TimesRoman);
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
            layout_text(&op_text, font_size, FontId::TimesRoman)
        }
    }
}

fn layout_symbol(symbol: &str, font_size: f32) -> MathBox {
    // Special handling for dot patterns (not in standard fonts)
    if let Some(ch) = symbol.chars().next() {
        match ch {
            '\u{22EE}' => { // vdots — vertical dots
                let dot = "\u{00B7}"; // middle dot
                let dot_w = font::measure_text(dot, FontId::Helvetica, font_size);
                let spacing = font_size * 0.35;
                let mut elems = Vec::new();
                for i in 0..3 {
                    elems.push(MathElement::Text {
                        x: 0.0, y: -spacing + i as f32 * spacing,
                        text: dot.to_string(), font_size, font_id: FontId::Helvetica, color: Color::BLACK,
                    });
                }
                return MathBox { width: dot_w.max(font_size * 0.3), height: font_size * 0.7, depth: font_size * 0.3, elements: elems };
            }
            '\u{22F1}' => { // ddots — diagonal dots (down-right)
                let dot = "\u{00B7}";
                let spacing = font_size * 0.35;
                let mut elems = Vec::new();
                for i in 0..3 {
                    elems.push(MathElement::Text {
                        x: i as f32 * spacing * 0.6, y: -spacing + i as f32 * spacing,
                        text: dot.to_string(), font_size, font_id: FontId::Helvetica, color: Color::BLACK,
                    });
                }
                return MathBox { width: spacing * 1.5, height: font_size * 0.7, depth: font_size * 0.3, elements: elems };
            }
            '\u{22F0}' => { // iddots — anti-diagonal dots (up-right)
                let dot = "\u{00B7}";
                let spacing = font_size * 0.35;
                let mut elems = Vec::new();
                for i in 0..3 {
                    elems.push(MathElement::Text {
                        x: i as f32 * spacing * 0.6, y: spacing - i as f32 * spacing,
                        text: dot.to_string(), font_size, font_id: FontId::Helvetica, color: Color::BLACK,
                    });
                }
                return MathBox { width: spacing * 1.5, height: font_size * 0.7, depth: font_size * 0.3, elements: elems };
            }
            _ => {}
        }
    }

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
    // Fallback to Times-Roman for unrecognized symbols
    let width = font::measure_text(symbol, FontId::TimesRoman, font_size);
    let info = font::font_info(FontId::TimesRoman);
    MathBox {
        width: width.max(font_size * 0.5),
        height: info.ascent as f32 * font_size * 0.001,
        depth: (-info.descent as f32) * font_size * 0.001,
        elements: vec![MathElement::Text {
            x: 0.0,
            y: 0.0,
            text: symbol.to_string(),
            font_size,
            font_id: FontId::TimesRoman,
            color: Color::BLACK,
        }],
    }
}

fn layout_fraction(numer: &[MathNode], denom: &[MathNode], font_size: f32) -> MathBox {
    let frac_size = font_size * 0.8; // TeX \textstyle fraction uses ~80% of current size
    let mut num_box = layout_math(numer, frac_size);
    let mut den_box = layout_math(denom, frac_size);

    let rule_thickness = (font_size * 0.04).max(0.4);
    let padding = font_size * 0.15; // horizontal padding around content
    let total_width = num_box.width.max(den_box.width) + padding * 2.0;

    // TeX math axis: fraction bar sits at the math axis (~half x-height)
    let axis = font_size * 0.25;
    // Minimum gap between content and bar (TeX sigma8 / sigma11)
    let num_gap = (font_size * 0.12).max(rule_thickness);
    let den_gap = (font_size * 0.12).max(rule_thickness);

    // Position numerator: bottom of numerator box should be at least num_gap above the bar
    let num_x = (total_width - num_box.width) / 2.0;
    let num_shift_up = axis + rule_thickness / 2.0 + num_gap + num_box.depth;
    num_box.translate(num_x, -num_shift_up);

    // Position denominator: top of denominator box should be at least den_gap below the bar
    let den_x = (total_width - den_box.width) / 2.0;
    let den_shift_down = -axis + rule_thickness / 2.0 + den_gap + den_box.height;
    den_box.translate(den_x, den_shift_down);

    let height = num_shift_up + num_box.height;
    let depth = den_shift_down + den_box.depth;

    let mut elements = Vec::with_capacity(num_box.elements.len() + den_box.elements.len() + 1);
    elements.extend(num_box.elements);
    elements.extend(den_box.elements);
    // Fraction bar at math axis
    elements.push(MathElement::Line {
        x1: 0.0,
        y1: -axis,
        x2: total_width,
        y2: -axis,
        width: rule_thickness,
        color: Color::BLACK,
    });

    MathBox { width: total_width, height, depth, elements }
}

fn layout_sqrt(index: Option<&[MathNode]>, radicand: &[MathNode], font_size: f32) -> MathBox {
    let mut inner = layout_math(radicand, font_size);
    let rule_thickness = (font_size * 0.04).max(0.4);
    let overline_gap = font_size * 0.1;
    let overline_y = inner.height + overline_gap;

    // Scale radical sign to match content height
    let content_total = inner.height + inner.depth;
    // The radical glyph should be tall enough to cover the content
    let radical_font_size = (content_total + overline_gap).max(font_size) * 1.1;
    let radical_width = radical_font_size * 0.42; // proportional to radical size

    let mut elements = Vec::new();

    // For very tall content, draw the radical as lines instead of a glyph
    if content_total > font_size * 2.0 {
        // Extensible radical: diagonal line + vertical line
        let bottom_y = inner.depth;
        let top_y = -overline_y;
        let tick_x = radical_width * 0.3;
        let join_x = radical_width * 0.7;

        // Small tick at bottom-left
        elements.push(MathElement::Line {
            x1: 0.0, y1: bottom_y - (bottom_y - top_y) * 0.3,
            x2: tick_x, y2: bottom_y,
            width: rule_thickness, color: Color::BLACK,
        });
        // Diagonal from tick bottom to top of radical
        elements.push(MathElement::Line {
            x1: tick_x, y1: bottom_y,
            x2: join_x, y2: top_y,
            width: rule_thickness * 1.5, color: Color::BLACK,
        });
        // Vertical connection to overline
        elements.push(MathElement::Line {
            x1: join_x, y1: top_y,
            x2: radical_width, y2: top_y,
            width: rule_thickness, color: Color::BLACK,
        });
    } else {
        // Glyph-based radical using Symbol font (byte 0xD6 = √)
        // Vertically center the radical glyph
        let glyph_center_offset = radical_font_size * 0.15;
        let target_center = (inner.height - inner.depth) / 2.0;
        let y_shift = glyph_center_offset - target_center;
        elements.push(MathElement::Text {
            x: 0.0,
            y: y_shift,
            text: String::from(0xD6 as char),
            font_size: radical_font_size,
            font_id: FontId::Symbol,
            color: Color::BLACK,
        });
    }

    // Overline bar above content
    elements.push(MathElement::Line {
        x1: radical_width,
        y1: -overline_y,
        x2: radical_width + inner.width + font_size * 0.1,
        y2: -overline_y,
        width: rule_thickness,
        color: Color::BLACK,
    });

    // Inner content shifted right past the radical
    inner.translate(radical_width, 0.0);
    elements.extend(inner.elements);

    let total_width = radical_width + inner.width + font_size * 0.15;
    let mut result = MathBox {
        width: total_width,
        height: overline_y + rule_thickness,
        depth: inner.depth,
        elements,
    };

    // Optional index (n-th root)
    if let Some(idx_nodes) = index {
        let mut idx = layout_math(idx_nodes, font_size * 0.5);
        idx.translate(font_size * 0.05, -(overline_y * 0.5));
        result.elements.extend(idx.elements);
        // Widen if index extends left
        result.width = result.width.max(idx.width + radical_width);
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
    let limit_size = font_size * 0.6;
    let limit_gap = font_size * 0.08;

    // Use Symbol font for large operators
    let (op_text, op_font, op_width) = if let Some(ch) = symbol.chars().next() {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let w = font::char_width_pt(FontId::Symbol, sym_byte, op_size).max(font_size * 0.8);
            (String::from(sym_byte as char), FontId::Symbol, w)
        } else {
            let w = font::measure_text(symbol, FontId::TimesRoman, op_size).max(font_size * 0.8);
            (symbol.to_string(), FontId::TimesRoman, w)
        }
    } else {
        let w = font::measure_text(symbol, FontId::Helvetica, op_size).max(font_size * 0.8);
        (symbol.to_string(), FontId::Helvetica, w)
    };

    // Compute the overall column width (max of operator, upper limit, lower limit)
    let upper_box_opt = upper.as_ref().map(|u| layout_math(u, limit_size));
    let lower_box_opt = lower.as_ref().map(|l| layout_math(l, limit_size));

    let max_width = op_width
        .max(upper_box_opt.as_ref().map_or(0.0, |b| b.width))
        .max(lower_box_opt.as_ref().map_or(0.0, |b| b.width));

    // Center the operator symbol in the column
    let op_x = (max_width - op_width) / 2.0;

    let mut elements = vec![MathElement::Text {
        x: op_x,
        y: 0.0,
        text: op_text,
        font_size: op_size,
        font_id: op_font,
        color: Color::BLACK,
    }];

    let op_ascent = op_size * 0.7;
    let op_descent = op_size * 0.2;
    let mut height = op_ascent;
    let mut depth = op_descent;

    // Upper limit: centered above the operator
    if let Some(mut upper_box) = upper_box_opt {
        let ux = (max_width - upper_box.width) / 2.0;
        let uy = -(op_ascent + limit_gap + upper_box.depth);
        upper_box.translate(ux, uy);
        height = op_ascent + limit_gap + upper_box.total_height();
        elements.extend(upper_box.elements);
    }

    // Lower limit: centered below the operator
    if let Some(mut lower_box) = lower_box_opt {
        let lx = (max_width - lower_box.width) / 2.0;
        let ly = op_descent + limit_gap + lower_box.height;
        lower_box.translate(lx, ly);
        depth = op_descent + limit_gap + lower_box.total_height();
        elements.extend(lower_box.elements);
    }

    // Add spacing after operator
    let width = max_width + font_size * 0.2;

    MathBox { width, height, depth, elements }
}

/// Layout a `\left...\right` delimited group: lay out content, measure its height,
/// then scale delimiters to match.
fn layout_delimited_group(left: &str, right: &str, content: &[MathNode], font_size: f32) -> MathBox {
    let inner = layout_math(content, font_size);

    // Minimum delimiter size is standard text height
    let content_height = inner.height.max(font_size * 0.7);
    let content_depth = inner.depth.max(font_size * 0.2);
    let extra = font_size * 0.1; // slight extra clearance

    let mut left_box = layout_delimiter(left, font_size, content_height + extra, content_depth + extra);
    let mut right_box = layout_delimiter(right, font_size, content_height + extra, content_depth + extra);

    // Position: left delimiter, then content, then right delimiter
    let gap = font_size * 0.08;
    let mut x = 0.0;
    // Left delimiter is already at x=0
    x += left_box.width + gap;

    let mut inner_shifted = inner;
    inner_shifted.translate(x, 0.0);
    x += inner_shifted.width + gap;

    right_box.translate(x, 0.0);
    x += right_box.width;

    let height = content_height.max(left_box.height).max(right_box.height);
    let depth = content_depth.max(left_box.depth).max(right_box.depth);

    let mut elements = Vec::with_capacity(
        left_box.elements.len() + inner_shifted.elements.len() + right_box.elements.len(),
    );
    elements.extend(left_box.elements);
    elements.extend(inner_shifted.elements);
    elements.extend(right_box.elements);

    MathBox { width: x, height, depth, elements }
}

/// Layout a delimiter glyph scaled to the given content height and depth.
/// For tall content, delimiters are drawn with lines (extensible).
fn layout_delimiter(delim: &str, font_size: f32, content_height: f32, content_depth: f32) -> MathBox {
    if delim == "." {
        // Invisible delimiter
        return MathBox { width: 0.0, height: content_height, depth: content_depth, elements: Vec::new() };
    }

    let total_size = content_height + content_depth;

    // Threshold: if content is taller than ~1.5x base font, draw extensible delimiters with lines
    let use_extensible = total_size > font_size * 1.8;

    if use_extensible {
        return layout_extensible_delimiter(delim, font_size, content_height, content_depth);
    }

    // For normal-sized content, scale the glyph to fit
    // The delimiter font size is chosen so the glyph covers content_height + content_depth
    let target_size = total_size * 1.1; // slight scaling
    let ds = target_size.max(font_size); // at least base font size

    // Check if delimiter maps to Symbol font
    let unicode_ch = match delim {
        "\\langle" => Some('\u{27E8}'),
        "\\rangle" => Some('\u{27E9}'),
        "\\lfloor" => Some('\u{230A}'),
        "\\rfloor" => Some('\u{230B}'),
        "\\lceil" => Some('\u{2308}'),
        "\\rceil" => Some('\u{2309}'),
        "\\|" => Some('\u{2225}'), // double vertical bar
        _ => None,
    };

    if let Some(ch) = unicode_ch {
        if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
            let width = font::char_width_pt(FontId::Symbol, sym_byte, ds);
            // Center vertically: shift so glyph center aligns with math axis
            let glyph_center_offset = ds * 0.2; // approximate: glyph center is above baseline
            let target_center = (content_height - content_depth) / 2.0;
            let y_shift = glyph_center_offset - target_center;
            return MathBox {
                width,
                height: content_height,
                depth: content_depth,
                elements: vec![MathElement::Text {
                    x: 0.0, y: y_shift,
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

    let width = font::measure_text(&text, FontId::TimesRoman, ds);
    let glyph_center_offset = ds * 0.2;
    let target_center = (content_height - content_depth) / 2.0;
    let y_shift = glyph_center_offset - target_center;

    MathBox {
        width,
        height: content_height,
        depth: content_depth,
        elements: vec![MathElement::Text {
            x: 0.0, y: y_shift,
            text,
            font_size: ds,
            font_id: FontId::TimesRoman,
            color: Color::BLACK,
        }],
    }
}

/// Draw an extensible delimiter using lines for tall content.
/// Produces vertical lines + decorative caps for brackets/braces/parens.
fn layout_extensible_delimiter(delim: &str, font_size: f32, content_height: f32, content_depth: f32) -> MathBox {
    let top = -content_height;
    let bottom = content_depth;
    let line_w = font_size * 0.04;
    let cap_len = font_size * 0.2;
    let width = cap_len + font_size * 0.1;

    let mut elements = Vec::new();

    match delim {
        "(" => {
            // Left parenthesis: curved vertical line (approximate with straight + caps)
            let mid_x = width * 0.6;
            elements.push(MathElement::Line {
                x1: mid_x, y1: top, x2: mid_x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            // Top curve cap
            elements.push(MathElement::Line {
                x1: mid_x, y1: top, x2: mid_x + cap_len * 0.5, y2: top, width: line_w, color: Color::BLACK,
            });
            // Bottom curve cap
            elements.push(MathElement::Line {
                x1: mid_x, y1: bottom, x2: mid_x + cap_len * 0.5, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        ")" => {
            let mid_x = width * 0.4;
            elements.push(MathElement::Line {
                x1: mid_x, y1: top, x2: mid_x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: mid_x, y1: top, x2: mid_x - cap_len * 0.5, y2: top, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: mid_x, y1: bottom, x2: mid_x - cap_len * 0.5, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "[" | "\\lfloor" | "\\lceil" => {
            // Left bracket: vertical line + horizontal caps
            let x = width * 0.5;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            if delim != "\\lfloor" {
                elements.push(MathElement::Line {
                    x1: x, y1: top, x2: x + cap_len, y2: top, width: line_w, color: Color::BLACK,
                });
            }
            if delim != "\\lceil" {
                elements.push(MathElement::Line {
                    x1: x, y1: bottom, x2: x + cap_len, y2: bottom, width: line_w, color: Color::BLACK,
                });
            }
        }
        "]" | "\\rfloor" | "\\rceil" => {
            let x = width * 0.5;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            if delim != "\\rfloor" {
                elements.push(MathElement::Line {
                    x1: x, y1: top, x2: x - cap_len, y2: top, width: line_w, color: Color::BLACK,
                });
            }
            if delim != "\\rceil" {
                elements.push(MathElement::Line {
                    x1: x, y1: bottom, x2: x - cap_len, y2: bottom, width: line_w, color: Color::BLACK,
                });
            }
        }
        "\\{" => {
            // Left brace: vertical line + center point + caps
            let x = width * 0.6;
            let mid_y = (top + bottom) / 2.0;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: mid_y - font_size * 0.1, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: mid_y + font_size * 0.1, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            // Center cusp
            elements.push(MathElement::Line {
                x1: x, y1: mid_y - font_size * 0.1, x2: x - cap_len * 0.5, y2: mid_y, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x - cap_len * 0.5, y1: mid_y, x2: x, y2: mid_y + font_size * 0.1, width: line_w, color: Color::BLACK,
            });
            // Top/bottom caps
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x + cap_len * 0.5, y2: top, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: bottom, x2: x + cap_len * 0.5, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "\\}" => {
            let x = width * 0.4;
            let mid_y = (top + bottom) / 2.0;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: mid_y - font_size * 0.1, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: mid_y + font_size * 0.1, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: mid_y - font_size * 0.1, x2: x + cap_len * 0.5, y2: mid_y, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x + cap_len * 0.5, y1: mid_y, x2: x, y2: mid_y + font_size * 0.1, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x - cap_len * 0.5, y2: top, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x, y1: bottom, x2: x - cap_len * 0.5, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "\\langle" => {
            // Left angle bracket: two diagonal lines meeting at the left
            let mid_y = (top + bottom) / 2.0;
            let right_x = width * 0.8;
            let left_x = width * 0.2;
            elements.push(MathElement::Line {
                x1: right_x, y1: top, x2: left_x, y2: mid_y, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: left_x, y1: mid_y, x2: right_x, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "\\rangle" => {
            let mid_y = (top + bottom) / 2.0;
            let left_x = width * 0.2;
            let right_x = width * 0.8;
            elements.push(MathElement::Line {
                x1: left_x, y1: top, x2: right_x, y2: mid_y, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: right_x, y1: mid_y, x2: left_x, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "|" => {
            let x = width * 0.5;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        "\\|" => {
            let x1 = width * 0.35;
            let x2 = width * 0.65;
            elements.push(MathElement::Line {
                x1, y1: top, x2: x1, y2: bottom, width: line_w, color: Color::BLACK,
            });
            elements.push(MathElement::Line {
                x1: x2, y1: top, x2, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
        _ => {
            // Fallback: just a vertical line
            let x = width * 0.5;
            elements.push(MathElement::Line {
                x1: x, y1: top, x2: x, y2: bottom, width: line_w, color: Color::BLACK,
            });
        }
    }

    MathBox { width, height: content_height, depth: content_depth, elements }
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
        AccentType::Breve => {
            // Draw breve as a small arc (3 line segments approximating a "⌣" shape)
            let cx = base_box.width * 0.5;
            let w = base_box.width * 0.5;
            let h = font_size * 0.08;
            let lw = font_size * 0.05;
            let y0 = accent_y + font_size * 0.15;
            base_box.elements.push(MathElement::Line { x1: cx - w * 0.5, y1: y0, x2: cx - w * 0.15, y2: y0 + h, width: lw, color: Color::BLACK });
            base_box.elements.push(MathElement::Line { x1: cx - w * 0.15, y1: y0 + h, x2: cx + w * 0.15, y2: y0 + h, width: lw, color: Color::BLACK });
            base_box.elements.push(MathElement::Line { x1: cx + w * 0.15, y1: y0 + h, x2: cx + w * 0.5, y2: y0, width: lw, color: Color::BLACK });
            base_box.height += font_size * 0.2;
            return base_box;
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
        MathFontType::Blackboard => FontId::TimesBold,         // \mathbb — bold serif
        MathFontType::Calligraphic | MathFontType::Script => FontId::TimesItalic, // \mathcal
        MathFontType::Fraktur => FontId::TimesBold,            // \mathfrak approximation
        MathFontType::SansSerif => FontId::Helvetica,          // \mathsf stays sans-serif
        MathFontType::BoldMath => FontId::TimesBold,           // \mathbf
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
