/// Diagram rendering for xy-pic (xymatrix), tikz-cd, circuitikz, and forest.
///
/// Each renderer parses its specific source format and produces a list of
/// TikzElement primitives (rects, lines, arrows, text) that the layout
/// engine already knows how to render.

use crate::tikz_render::{TikzElement, TikzRenderResult};
use crate::color::Color;
use crate::font::{self, FontId};

// ─── Common helpers ──────────────────────────────────────────────────

/// Measure text width for diagram labels
fn text_width(text: &str, font_size: f32) -> f32 {
    font::measure_text(text, FontId::Helvetica, font_size)
}

/// Strip simple LaTeX math wrappers for display: $x$ → x, \mathcal{F} → F, etc.
fn strip_math(s: &str) -> String {
    let s = s.trim();
    // Strip surrounding $...$
    let s = if s.starts_with('$') && s.ends_with('$') && s.len() >= 2 {
        &s[1..s.len() - 1]
    } else {
        s
    };
    // Strip common math commands, keep their argument
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Skip command name
            i += 1;
            let cmd_start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let _cmd = &s[cmd_start..i];
            // If followed by braced argument, extract it
            if i < bytes.len() && bytes[i] == b'{' {
                i += 1;
                let arg_start = i;
                let mut depth = 1;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'{' { depth += 1; }
                    if bytes[i] == b'}' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                result.push_str(&s[arg_start..i]);
                if i < bytes.len() { i += 1; } // skip '}'
            }
            // else skip bare command
        } else if bytes[i] == b'{' || bytes[i] == b'}' {
            i += 1;
        } else if bytes[i] == b'_' || bytes[i] == b'^' {
            // Subscript/superscript markers — skip marker, keep content
            i += 1;
            if i < bytes.len() && bytes[i] == b'{' {
                i += 1;
                let arg_start = i;
                let mut depth = 1;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'{' { depth += 1; }
                    if bytes[i] == b'}' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                result.push_str(&s[arg_start..i]);
                if i < bytes.len() { i += 1; }
            } else if i < bytes.len() {
                result.push(bytes[i] as char);
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

// ═══════════════════════════════════════════════════════════════════════
// Task 1: xymatrix — commutative diagrams (xy-pic)
// ═══════════════════════════════════════════════════════════════════════

/// An arrow in an xymatrix diagram
#[derive(Debug)]
struct XyArrow {
    /// Direction steps: positive = right/down, negative = left/up
    dr: i32, // row offset (positive = down)
    dc: i32, // col offset (positive = right)
    /// Label text (if any)
    label: Option<String>,
    /// Label position: true = above/left (^), false = below/right (_)
    label_above: bool,
}

/// A cell in the xymatrix grid
#[derive(Debug)]
struct XyCell {
    text: String,
    arrows: Vec<XyArrow>,
}

/// Parse an xymatrix body: `A \ar[r]^{f} & B \\ C \ar[u] & D \ar[l]_{g}`
fn parse_xymatrix(source: &str) -> Vec<Vec<XyCell>> {
    let mut rows: Vec<Vec<XyCell>> = Vec::new();
    let mut current_row: Vec<XyCell> = Vec::new();

    // Split on \\ for rows
    let row_strs = split_rows(source);

    for row_str in &row_strs {
        current_row.clear();
        // Split on & for columns
        let cell_strs = split_cells(row_str);

        for cell_str in &cell_strs {
            let cell = parse_xy_cell(cell_str.trim());
            current_row.push(cell);
        }
        if !current_row.is_empty() {
            rows.push(std::mem::take(&mut current_row));
        }
    }
    rows
}

/// Split source on `\\` respecting brace nesting
fn split_rows(source: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut depth = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' { depth += 1; }
        if bytes[i] == b'}' && depth > 0 { depth -= 1; }
        if depth == 0 && bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            rows.push(std::mem::take(&mut current));
            i += 2;
            continue;
        }
        current.push(bytes[i] as char);
        i += 1;
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        rows.push(current);
    }
    rows
}

/// Split source on `&` respecting brace nesting
fn split_cells(source: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut depth = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' { depth += 1; }
        if bytes[i] == b'}' && depth > 0 { depth -= 1; }
        if depth == 0 && bytes[i] == b'&' {
            cells.push(std::mem::take(&mut current));
            i += 1;
            continue;
        }
        current.push(bytes[i] as char);
        i += 1;
    }
    cells.push(current);
    cells
}

/// Parse a single xymatrix cell: text content + \ar[...] arrows
fn parse_xy_cell(source: &str) -> XyCell {
    let mut text = String::new();
    let mut arrows = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Check for \ar[ command
        if i + 3 < bytes.len() && &source[i..i + 3] == "\\ar" {
            i += 3;
            // Skip optional space
            while i < bytes.len() && bytes[i] == b' ' { i += 1; }
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                // Read direction chars until ]
                let mut dr: i32 = 0;
                let mut dc: i32 = 0;
                while i < bytes.len() && bytes[i] != b']' {
                    match bytes[i] {
                        b'r' => dc += 1,
                        b'l' => dc -= 1,
                        b'd' => dr += 1,
                        b'u' => dr -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                if i < bytes.len() { i += 1; } // skip ']'

                // Check for label: ^{...} or _{...}
                let mut label = None;
                let mut label_above = true;
                while i < bytes.len() && bytes[i] == b' ' { i += 1; }
                if i < bytes.len() && (bytes[i] == b'^' || bytes[i] == b'_') {
                    label_above = bytes[i] == b'^';
                    i += 1;
                    if i < bytes.len() && bytes[i] == b'{' {
                        i += 1;
                        let start = i;
                        let mut depth = 1;
                        while i < bytes.len() && depth > 0 {
                            if bytes[i] == b'{' { depth += 1; }
                            if bytes[i] == b'}' { depth -= 1; }
                            if depth > 0 { i += 1; }
                        }
                        label = Some(strip_math(&source[start..i]));
                        if i < bytes.len() { i += 1; }
                    } else if i < bytes.len() {
                        // Single char label
                        label = Some(String::from(bytes[i] as char));
                        i += 1;
                    }
                }

                arrows.push(XyArrow { dr, dc, label, label_above });
                continue;
            }
        }

        // Regular text
        text.push(bytes[i] as char);
        i += 1;
    }

    XyCell {
        text: strip_math(text.trim()),
        arrows,
    }
}

/// Render an xymatrix diagram to TikzElements
pub fn render_xymatrix(source: &str) -> TikzRenderResult {
    // Extract xymatrix body — look for \xymatrix{...}
    let body = if let Some(start) = source.find("\\xymatrix") {
        let rest = &source[start + 9..];
        // Skip optional size modifier like @R=2em@C=3em
        let rest = if let Some(b) = rest.find('{') {
            &rest[b + 1..]
        } else {
            rest
        };
        // Find matching close brace
        let mut depth = 1;
        let mut end = 0;
        for (i, &b) in rest.as_bytes().iter().enumerate() {
            if b == b'{' { depth += 1; }
            if b == b'}' {
                depth -= 1;
                if depth == 0 { end = i; break; }
            }
        }
        &rest[..end]
    } else {
        source
    };

    let rows = parse_xymatrix(body);
    if rows.is_empty() {
        return TikzRenderResult { elements: Vec::new(), width: 0.0, height: 0.0 };
    }

    let font_size = 10.0;
    let cell_w = 80.0;
    let cell_h = 50.0;
    let num_rows = rows.len();
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);

    let total_w = num_cols as f32 * cell_w;
    let total_h = num_rows as f32 * cell_h;
    let mut elements = Vec::new();

    // Render cell text
    for (ri, row) in rows.iter().enumerate() {
        for (ci, cell) in row.iter().enumerate() {
            if !cell.text.is_empty() {
                let tw = text_width(&cell.text, font_size);
                let cx = ci as f32 * cell_w + cell_w / 2.0;
                let cy = ri as f32 * cell_h + cell_h / 2.0;
                elements.push(TikzElement::Text {
                    x: cx - tw / 2.0,
                    y: cy - font_size / 2.0 + font_size * 0.8,
                    text: cell.text.clone(),
                    font_size,
                    bold: false,
                    color: Color::BLACK,
                });
            }
        }
    }

    // Render arrows
    for (ri, row) in rows.iter().enumerate() {
        for (ci, cell) in row.iter().enumerate() {
            for arrow in &cell.arrows {
                let from_x = ci as f32 * cell_w + cell_w / 2.0;
                let from_y = ri as f32 * cell_h + cell_h / 2.0;
                let to_col = ci as i32 + arrow.dc;
                let to_row = ri as i32 + arrow.dr;

                if to_col < 0 || to_row < 0 { continue; }

                let to_x = to_col as f32 * cell_w + cell_w / 2.0;
                let to_y = to_row as f32 * cell_h + cell_h / 2.0;

                // Shorten arrow to not overlap text
                let dx = to_x - from_x;
                let dy = to_y - from_y;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1.0 { continue; }
                let shorten = 15.0; // shorten by this much at each end
                let ux = dx / len;
                let uy = dy / len;
                let x1 = from_x + ux * shorten;
                let y1 = from_y + uy * shorten;
                let x2 = to_x - ux * shorten;
                let y2 = to_y - uy * shorten;

                elements.push(TikzElement::Arrow {
                    x1, y1, x2, y2,
                    width: 0.6,
                    color: Color::BLACK,
                    bidirectional: false,
                });

                // Render label
                if let Some(ref label) = arrow.label {
                    let lw = text_width(label, font_size * 0.85);
                    let mid_x = (x1 + x2) / 2.0;
                    let mid_y = (y1 + y2) / 2.0;
                    // Offset label perpendicular to the arrow
                    let offset = if arrow.label_above { -8.0 } else { 10.0 };
                    let lx;
                    let ly;
                    if dx.abs() > dy.abs() {
                        // Mostly horizontal arrow: label above/below
                        lx = mid_x - lw / 2.0;
                        ly = mid_y + offset;
                    } else {
                        // Mostly vertical arrow: label left/right
                        lx = mid_x + offset - lw / 2.0;
                        ly = mid_y;
                    }
                    elements.push(TikzElement::Text {
                        x: lx,
                        y: ly,
                        text: label.clone(),
                        font_size: font_size * 0.85,
                        bold: false,
                        color: Color::BLACK,
                    });
                }
            }
        }
    }

    TikzRenderResult { elements, width: total_w, height: total_h }
}

// ═══════════════════════════════════════════════════════════════════════
// Task 2: circuitikz — circuit diagrams
// ═══════════════════════════════════════════════════════════════════════

/// Render a circuitikz diagram. Since circuitikz extends TikZ, we first
/// try the native TikZ renderer. We augment it by recognizing component
/// node shapes and drawing simplified symbols.
pub fn render_circuitikz(source: &str) -> TikzRenderResult {
    // Try standard TikZ rendering first
    let mut result = crate::tikz_render::render_tikz(source);

    if !result.elements.is_empty() {
        return result;
    }

    // Fallback: parse circuitikz-specific to-path components
    let mut elements = Vec::new();
    let font_size = 9.0;

    // Parse \draw statements with to[component] paths
    let mut named_coords: std::collections::HashMap<String, (f32, f32)> = std::collections::HashMap::new();

    // Simple pass: look for coordinates defined with \coordinate or \node
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("\\coordinate") {
            if let Some((name, x, y)) = parse_coord_def(rest) {
                named_coords.insert(name, (x, y));
            }
        }
        if let Some(rest) = trimmed.strip_prefix("\\node") {
            if let Some((name, x, y)) = parse_coord_def(rest) {
                named_coords.insert(name, (x, y));
            }
        }
    }

    // Parse \draw ... to[component] ... statements
    let statements = collect_statements(source);
    let scale = 40.0; // TikZ cm to points

    for stmt in &statements {
        let s = stmt.trim();
        if !s.starts_with("\\draw") { continue; }

        let coords = extract_path_coords(s, &named_coords);
        if coords.len() < 2 { continue; }

        // Draw lines between consecutive coordinates
        for pair in coords.windows(2) {
            let (x1, y1) = pair[0];
            let (x2, y2) = pair[1];
            let px1 = x1 * scale + 150.0;
            let py1 = -y1 * scale + 150.0; // flip y
            let px2 = x2 * scale + 150.0;
            let py2 = -y2 * scale + 150.0;

            // Check if there's a component between these coords
            if let Some(comp) = extract_component_between(s, (x1, y1), (x2, y2)) {
                let mid_x = (px1 + px2) / 2.0;
                let mid_y = (py1 + py2) / 2.0;
                render_circuit_component(&comp, mid_x, mid_y, font_size, &mut elements);
                // Draw lines from endpoints to component
                let comp_half = 15.0;
                let dx = px2 - px1;
                let dy = py2 - py1;
                let len = (dx * dx + dy * dy).sqrt();
                if len > 0.0 {
                    let ux = dx / len;
                    let uy = dy / len;
                    elements.push(TikzElement::Line {
                        x1: px1, y1: py1, x2: mid_x - ux * comp_half, y2: mid_y - uy * comp_half,
                        width: 0.6, color: Color::BLACK,
                    });
                    elements.push(TikzElement::Line {
                        x1: mid_x + ux * comp_half, y1: mid_y + uy * comp_half, x2: px2, y2: py2,
                        width: 0.6, color: Color::BLACK,
                    });
                }
            } else {
                elements.push(TikzElement::Line {
                    x1: px1, y1: py1, x2: px2, y2: py2,
                    width: 0.6, color: Color::BLACK,
                });
            }
        }
    }

    // Look for ground nodes
    for stmt in &statements {
        let s = stmt.trim();
        if s.contains("ground") || s.contains("tlground") {
            if let Some((x, y)) = extract_single_coord(s, &named_coords) {
                let px = x * scale + 150.0;
                let py = -y * scale + 150.0;
                render_ground(px, py, &mut elements);
            }
        }
    }

    if elements.is_empty() {
        // Absolute fallback: placeholder
        result.width = 200.0;
        result.height = 80.0;
        result.elements.push(TikzElement::Rect {
            x: 0.0, y: 0.0, width: 200.0, height: 80.0,
            fill: Some(Color::rgb(0.95, 0.95, 0.98)),
            stroke: Some(Color::rgb(0.6, 0.6, 0.8)),
            stroke_width: 0.5, corner_radius: 4.0,
        });
        result.elements.push(TikzElement::Text {
            x: 40.0, y: 45.0, text: "[Circuit diagram]".into(),
            font_size: 10.0, bold: false, color: Color::GRAY,
        });
        return result;
    }

    let (min_x, min_y, max_x, max_y) = bounding_box(&elements);
    let margin = 15.0;
    offset_elements(&mut elements, -(min_x - margin), -(min_y - margin));

    TikzRenderResult {
        elements,
        width: (max_x - min_x) + margin * 2.0,
        height: (max_y - min_y) + margin * 2.0,
    }
}

fn render_circuit_component(comp: &str, cx: f32, cy: f32, font_size: f32, elements: &mut Vec<TikzElement>) {
    match comp {
        "R" | "resistor" | "european resistor" => {
            // Zig-zag resistor symbol (simplified as rectangle)
            let w = 28.0;
            let h = 10.0;
            elements.push(TikzElement::Rect {
                x: cx - w / 2.0, y: cy - h / 2.0, width: w, height: h,
                fill: None, stroke: Some(Color::BLACK),
                stroke_width: 0.6, corner_radius: 0.0,
            });
        }
        "C" | "capacitor" | "european capacitor" => {
            // Two parallel plates
            let gap = 4.0;
            let plate_h = 14.0;
            elements.push(TikzElement::Line {
                x1: cx - gap / 2.0, y1: cy - plate_h / 2.0,
                x2: cx - gap / 2.0, y2: cy + plate_h / 2.0,
                width: 1.5, color: Color::BLACK,
            });
            elements.push(TikzElement::Line {
                x1: cx + gap / 2.0, y1: cy - plate_h / 2.0,
                x2: cx + gap / 2.0, y2: cy + plate_h / 2.0,
                width: 1.5, color: Color::BLACK,
            });
        }
        "L" | "inductor" | "cute inductor" | "american inductor" => {
            // Simple coil (series of bumps - approximated with arcs/text)
            let w = 28.0;
            let h = 10.0;
            elements.push(TikzElement::Rect {
                x: cx - w / 2.0, y: cy - h / 2.0, width: w, height: h,
                fill: None, stroke: Some(Color::BLACK),
                stroke_width: 0.6, corner_radius: 5.0,
            });
        }
        "V" | "voltage source" | "european voltage source" => {
            // Circle with + and -
            let r = 10.0;
            elements.push(TikzElement::Rect {
                x: cx - r, y: cy - r, width: r * 2.0, height: r * 2.0,
                fill: None, stroke: Some(Color::BLACK),
                stroke_width: 0.6, corner_radius: r,
            });
            elements.push(TikzElement::Text {
                x: cx - 3.0, y: cy - 1.0, text: "+".into(),
                font_size, bold: false, color: Color::BLACK,
            });
            elements.push(TikzElement::Text {
                x: cx - 2.0, y: cy + 8.0, text: "-".into(),
                font_size, bold: false, color: Color::BLACK,
            });
        }
        "I" | "current source" => {
            let r = 10.0;
            elements.push(TikzElement::Rect {
                x: cx - r, y: cy - r, width: r * 2.0, height: r * 2.0,
                fill: None, stroke: Some(Color::BLACK),
                stroke_width: 0.6, corner_radius: r,
            });
        }
        "D" | "diode" | "empty diode" => {
            // Triangle + line
            let s = 10.0;
            elements.push(TikzElement::Line {
                x1: cx - s, y1: cy - s / 2.0, x2: cx - s, y2: cy + s / 2.0,
                width: 0.6, color: Color::BLACK,
            });
            elements.push(TikzElement::Line {
                x1: cx - s, y1: cy - s / 2.0, x2: cx + s / 2.0, y2: cy,
                width: 0.6, color: Color::BLACK,
            });
            elements.push(TikzElement::Line {
                x1: cx - s, y1: cy + s / 2.0, x2: cx + s / 2.0, y2: cy,
                width: 0.6, color: Color::BLACK,
            });
            elements.push(TikzElement::Line {
                x1: cx + s / 2.0, y1: cy - s / 2.0, x2: cx + s / 2.0, y2: cy + s / 2.0,
                width: 0.6, color: Color::BLACK,
            });
        }
        _ => {
            // Generic box with label
            let w = 28.0;
            let h = 14.0;
            elements.push(TikzElement::Rect {
                x: cx - w / 2.0, y: cy - h / 2.0, width: w, height: h,
                fill: Some(Color::rgb(0.95, 0.95, 1.0)),
                stroke: Some(Color::BLACK),
                stroke_width: 0.6, corner_radius: 0.0,
            });
            let tw = text_width(comp, font_size * 0.8);
            elements.push(TikzElement::Text {
                x: cx - tw / 2.0, y: cy + font_size * 0.3,
                text: comp.to_string(),
                font_size: font_size * 0.8, bold: false, color: Color::BLACK,
            });
        }
    }
}

fn render_ground(x: f32, y: f32, elements: &mut Vec<TikzElement>) {
    let w1 = 14.0;
    let w2 = 9.0;
    let w3 = 4.0;
    let gap = 3.0;
    elements.push(TikzElement::Line {
        x1: x - w1 / 2.0, y1: y, x2: x + w1 / 2.0, y2: y,
        width: 0.8, color: Color::BLACK,
    });
    elements.push(TikzElement::Line {
        x1: x - w2 / 2.0, y1: y + gap, x2: x + w2 / 2.0, y2: y + gap,
        width: 0.8, color: Color::BLACK,
    });
    elements.push(TikzElement::Line {
        x1: x - w3 / 2.0, y1: y + gap * 2.0, x2: x + w3 / 2.0, y2: y + gap * 2.0,
        width: 0.8, color: Color::BLACK,
    });
}

/// Collect semicolon-terminated statements from TikZ source
fn collect_statements(source: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut current = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('%') { continue; }
        current.push(' ');
        current.push_str(trimmed);
        if trimmed.ends_with(';') {
            stmts.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        stmts.push(current);
    }
    stmts
}

/// Parse a coordinate definition from a \coordinate or \node statement
fn parse_coord_def(rest: &str) -> Option<(String, f32, f32)> {
    // Look for (name) at (x,y)
    let name_start = rest.find('(')? + 1;
    let name_end = rest[name_start..].find(')')? + name_start;
    let name = rest[name_start..name_end].trim().to_string();

    let at_pos = rest.find("at")?;
    let coord_start = rest[at_pos..].find('(')? + at_pos + 1;
    let coord_end = rest[coord_start..].find(')')? + coord_start;
    let coord_str = &rest[coord_start..coord_end];
    let parts: Vec<&str> = coord_str.split(',').collect();
    if parts.len() >= 2 {
        let x = parts[0].trim().parse::<f32>().ok()?;
        let y = parts[1].trim().parse::<f32>().ok()?;
        Some((name, x, y))
    } else {
        None
    }
}

/// Extract coordinates from a \draw path
fn extract_path_coords(stmt: &str, named: &std::collections::HashMap<String, (f32, f32)>) -> Vec<(f32, f32)> {
    let mut coords = Vec::new();
    let bytes = stmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b')' { i += 1; }
            let inner = &stmt[start..i];
            if i < bytes.len() { i += 1; }
            // Try numeric (x,y)
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() >= 2 {
                if let (Ok(x), Ok(y)) = (parts[0].trim().parse::<f32>(), parts[1].trim().parse::<f32>()) {
                    coords.push((x, y));
                    continue;
                }
            }
            // Try named coordinate
            if let Some(&(x, y)) = named.get(inner.trim()) {
                coords.push((x, y));
            }
        } else {
            i += 1;
        }
    }
    coords
}

/// Extract component name from a to[...] segment
fn extract_component_between(stmt: &str, _from: (f32, f32), _to: (f32, f32)) -> Option<String> {
    // Find "to[component_name" in stmt
    let to_pos = stmt.find("to[")?;
    let start = to_pos + 3;
    let rest = &stmt[start..];
    let end = rest.find(']').unwrap_or(rest.len());
    let component = rest[..end].trim();
    // Extract first word (the component type), ignoring options like l=$R$
    let comp = component.split(',').next()?.trim();
    let comp = comp.split('=').next()?.trim();
    if comp.is_empty() { None } else { Some(comp.to_string()) }
}

/// Extract a single coordinate from a statement
fn extract_single_coord(stmt: &str, named: &std::collections::HashMap<String, (f32, f32)>) -> Option<(f32, f32)> {
    let coords = extract_path_coords(stmt, named);
    coords.into_iter().next()
}

// ═══════════════════════════════════════════════════════════════════════
// Task 3: forest — tree diagrams
// ═══════════════════════════════════════════════════════════════════════

/// A node in a forest tree
#[derive(Debug)]
struct ForestNode {
    label: String,
    children: Vec<ForestNode>,
}

/// Parse forest bracket notation: [root [child1] [child2 [grandchild]]]
fn parse_forest_tree(source: &str) -> Option<ForestNode> {
    let source = source.trim();
    let bytes = source.as_bytes();
    let mut pos = 0;

    // Skip to first '['
    while pos < bytes.len() && bytes[pos] != b'[' { pos += 1; }
    if pos >= bytes.len() { return None; }

    parse_forest_node(source, &mut pos)
}

fn parse_forest_node(source: &str, pos: &mut usize) -> Option<ForestNode> {
    let bytes = source.as_bytes();
    if *pos >= bytes.len() || bytes[*pos] != b'[' { return None; }
    *pos += 1; // skip '['

    // Read label text until we hit '[' (child) or ']' (end) or ',' (options)
    let mut label = String::new();
    let mut children = Vec::new();

    while *pos < bytes.len() {
        match bytes[*pos] {
            b'[' => {
                // Child node
                if let Some(child) = parse_forest_node(source, pos) {
                    children.push(child);
                }
            }
            b']' => {
                *pos += 1;
                break;
            }
            b',' => {
                // Forest options — skip until next '[' or ']'
                *pos += 1;
                // Skip options (key=value pairs) until we see a child bracket or closing bracket
                let mut depth = 0;
                while *pos < bytes.len() {
                    match bytes[*pos] {
                        b'[' if depth == 0 => break,
                        b']' if depth == 0 => break,
                        b'{' => { depth += 1; *pos += 1; }
                        b'}' => { depth -= 1; *pos += 1; }
                        _ => { *pos += 1; }
                    }
                }
            }
            b'\\' => {
                // Skip LaTeX commands in label
                *pos += 1;
                while *pos < bytes.len() && bytes[*pos].is_ascii_alphabetic() {
                    *pos += 1;
                }
                // If followed by {arg}, skip that too
                if *pos < bytes.len() && bytes[*pos] == b'{' {
                    *pos += 1;
                    let start = *pos;
                    let mut d = 1;
                    while *pos < bytes.len() && d > 0 {
                        if bytes[*pos] == b'{' { d += 1; }
                        if bytes[*pos] == b'}' { d -= 1; }
                        if d > 0 { *pos += 1; }
                    }
                    label.push_str(&source[start..*pos]);
                    if *pos < bytes.len() { *pos += 1; }
                }
            }
            _ => {
                label.push(bytes[*pos] as char);
                *pos += 1;
            }
        }
    }

    Some(ForestNode {
        label: label.trim().to_string(),
        children,
    })
}

/// Layout a tree: compute (x, y) positions for each node using a simple
/// top-down algorithm.
struct TreeLayout {
    positions: Vec<(f32, f32, String)>, // (x, y, label)
    edges: Vec<(f32, f32, f32, f32)>,   // parent(x,y) -> child(x,y)
}

fn layout_tree(root: &ForestNode, font_size: f32) -> TreeLayout {
    let h_gap = 20.0;  // horizontal gap between siblings
    let v_gap = 35.0;  // vertical gap between levels

    let mut layout = TreeLayout { positions: Vec::new(), edges: Vec::new() };
    let mut next_x = 0.0_f32;

    fn compute_positions(
        node: &ForestNode, depth: f32, next_x: &mut f32,
        font_size: f32, h_gap: f32, v_gap: f32,
        layout: &mut TreeLayout,
    ) -> f32 {
        let y = depth * v_gap;
        let label_w = text_width(&node.label, font_size).max(20.0);

        if node.children.is_empty() {
            let x = *next_x + label_w / 2.0;
            *next_x += label_w + h_gap;
            layout.positions.push((x, y, node.label.clone()));
            x
        } else {
            let mut child_xs = Vec::new();
            for child in &node.children {
                let cx = compute_positions(child, depth + 1.0, next_x, font_size, h_gap, v_gap, layout);
                child_xs.push(cx);
            }
            // Center parent over children
            let x = (child_xs[0] + child_xs[child_xs.len() - 1]) / 2.0;
            layout.positions.push((x, y, node.label.clone()));

            // Add edges to children
            let child_offset = layout.positions.len() - 1 - node.children.len();
            for (i, child) in node.children.iter().enumerate() {
                let _ = child;
                let (cx, cy, _) = layout.positions[child_offset + i];
                layout.edges.push((x, y + font_size + 2.0, cx, cy));
            }
            x
        }
    }

    compute_positions(root, 0.0, &mut next_x, font_size, h_gap, v_gap, &mut layout);
    layout
}

/// Render a forest tree diagram
pub fn render_forest(source: &str) -> TikzRenderResult {
    let root = match parse_forest_tree(source) {
        Some(r) => r,
        None => {
            return TikzRenderResult {
                elements: vec![
                    TikzElement::Rect {
                        x: 0.0, y: 0.0, width: 200.0, height: 60.0,
                        fill: Some(Color::rgb(0.95, 0.95, 0.98)),
                        stroke: Some(Color::rgb(0.6, 0.6, 0.8)),
                        stroke_width: 0.5, corner_radius: 4.0,
                    },
                    TikzElement::Text {
                        x: 45.0, y: 35.0, text: "[Tree diagram]".into(),
                        font_size: 10.0, bold: false, color: Color::GRAY,
                    },
                ],
                width: 200.0,
                height: 60.0,
            };
        }
    };

    let font_size = 9.0;
    let tree = layout_tree(&root, font_size);

    let mut elements = Vec::new();

    // Draw edges first (below nodes)
    for &(x1, y1, x2, y2) in &tree.edges {
        elements.push(TikzElement::Line {
            x1, y1, x2, y2,
            width: 0.5, color: Color::DARK_GRAY,
        });
    }

    // Draw nodes
    for (x, y, label) in &tree.positions {
        if !label.is_empty() {
            let tw = text_width(label, font_size);
            let pad = 4.0;
            // Draw background box
            elements.push(TikzElement::Rect {
                x: x - tw / 2.0 - pad, y: *y - 1.0,
                width: tw + pad * 2.0, height: font_size + pad,
                fill: Some(Color::WHITE), stroke: Some(Color::rgb(0.7, 0.7, 0.7)),
                stroke_width: 0.4, corner_radius: 2.0,
            });
            // Draw text
            elements.push(TikzElement::Text {
                x: x - tw / 2.0,
                y: y + font_size * 0.8,
                text: label.clone(),
                font_size,
                bold: false,
                color: Color::BLACK,
            });
        }
    }

    let (min_x, min_y, max_x, max_y) = bounding_box(&elements);
    let margin = 15.0;
    offset_elements(&mut elements, -(min_x - margin), -(min_y - margin));

    TikzRenderResult {
        elements,
        width: (max_x - min_x) + margin * 2.0,
        height: (max_y - min_y) + margin * 2.0,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Task 4: tikz-cd — commutative diagrams
// ═══════════════════════════════════════════════════════════════════════

/// An arrow in a tikz-cd diagram
#[derive(Debug)]
struct CdArrow {
    dr: i32,
    dc: i32,
    label: Option<String>,
    label_above: bool,
    dashed: bool,
    hook: bool,
    two_heads: bool,
    maps_to: bool,
    no_head: bool,
}

/// A cell in a tikz-cd grid
#[derive(Debug)]
struct CdCell {
    text: String,
    arrows: Vec<CdArrow>,
}

/// Parse a tikz-cd body
fn parse_tikzcd(source: &str) -> Vec<Vec<CdCell>> {
    let mut rows: Vec<Vec<CdCell>> = Vec::new();
    let row_strs = split_rows(source);

    for row_str in &row_strs {
        let cell_strs = split_cells(row_str);
        let mut row = Vec::new();
        for cell_str in &cell_strs {
            row.push(parse_cd_cell(cell_str.trim()));
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    rows
}

/// Parse a tikz-cd cell
fn parse_cd_cell(source: &str) -> CdCell {
    let mut text = String::new();
    let mut arrows = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Check for \arrow or \ar command
        let is_arrow_cmd = (i + 6 <= bytes.len() && &source[i..i + 6] == "\\arrow")
            || (i + 3 <= bytes.len() && &source[i..i + 3] == "\\ar");

        if is_arrow_cmd {
            let cmd_len = if i + 6 <= bytes.len() && &source[i..i + 6] == "\\arrow" { 6 } else { 3 };
            i += cmd_len;
            while i < bytes.len() && bytes[i] == b' ' { i += 1; }

            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                // Read arrow specification until ]
                let spec_start = i;
                let mut depth = 1;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'[' { depth += 1; }
                    if bytes[i] == b']' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                let spec = &source[spec_start..i];
                if i < bytes.len() { i += 1; } // skip ']'

                let arrow = parse_cd_arrow_spec(spec);
                arrows.push(arrow);
                continue;
            }
        }

        text.push(bytes[i] as char);
        i += 1;
    }

    CdCell {
        text: strip_math(text.trim()),
        arrows,
    }
}

/// Parse a tikz-cd arrow specification like "r", "d", "r, \"f\"", "dr, hook"
fn parse_cd_arrow_spec(spec: &str) -> CdArrow {
    let mut dr: i32 = 0;
    let mut dc: i32 = 0;
    let mut label = None;
    let mut label_above = true;
    let mut dashed = false;
    let mut hook = false;
    let mut two_heads = false;
    let mut maps_to = false;
    let mut no_head = false;

    // Split on commas for options
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();

    for (idx, part) in parts.iter().enumerate() {
        let p = part.trim();
        if idx == 0 {
            // First part is direction: r, l, u, d, or combinations like dr, rr, etc.
            for ch in p.chars() {
                match ch {
                    'r' => dc += 1,
                    'l' => dc -= 1,
                    'd' => dr += 1,
                    'u' => dr -= 1,
                    _ => {}
                }
            }
        } else if p == "dashed" || p == "dotted" {
            dashed = true;
        } else if p == "hook" || p.starts_with("hook") {
            hook = true;
        } else if p == "two heads" || p == "twoheadrightarrow" {
            two_heads = true;
        } else if p.starts_with("maps to") || p == "mapsto" {
            maps_to = true;
        } else if p == "no head" || p == "dash" {
            no_head = true;
        } else if p.starts_with('"') || p.starts_with('\'') {
            // Quoted label
            let label_text = p.trim_matches(|c: char| c == '"' || c == '\'' || c == '{' || c == '}');
            label = Some(strip_math(label_text));
            // Check for position marker
            if p.ends_with('\'') {
                label_above = false;
            }
        } else if p.starts_with("description") || p.starts_with("near start") || p.starts_with("near end") {
            // Position modifiers — ignore for now
        }
        // Check for label with ^ or _
        let _ = maps_to;
        let _ = hook;
        let _ = two_heads;
    }

    CdArrow { dr, dc, label, label_above, dashed, hook, two_heads, maps_to, no_head }
}

/// Render a tikz-cd diagram
pub fn render_tikzcd(source: &str) -> TikzRenderResult {
    let rows = parse_tikzcd(source);
    if rows.is_empty() {
        return TikzRenderResult { elements: Vec::new(), width: 0.0, height: 0.0 };
    }

    let font_size = 10.0;
    let cell_w = 80.0;
    let cell_h = 50.0;
    let num_rows = rows.len();
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);

    let total_w = num_cols as f32 * cell_w;
    let total_h = num_rows as f32 * cell_h;
    let mut elements = Vec::new();

    // Render cell text
    for (ri, row) in rows.iter().enumerate() {
        for (ci, cell) in row.iter().enumerate() {
            if !cell.text.is_empty() {
                let tw = text_width(&cell.text, font_size);
                let cx = ci as f32 * cell_w + cell_w / 2.0;
                let cy = ri as f32 * cell_h + cell_h / 2.0;
                elements.push(TikzElement::Text {
                    x: cx - tw / 2.0,
                    y: cy - font_size / 2.0 + font_size * 0.8,
                    text: cell.text.clone(),
                    font_size,
                    bold: false,
                    color: Color::BLACK,
                });
            }
        }
    }

    // Render arrows
    for (ri, row) in rows.iter().enumerate() {
        for (ci, cell) in row.iter().enumerate() {
            for arrow in &cell.arrows {
                let from_x = ci as f32 * cell_w + cell_w / 2.0;
                let from_y = ri as f32 * cell_h + cell_h / 2.0;
                let to_col = ci as i32 + arrow.dc;
                let to_row = ri as i32 + arrow.dr;

                if to_col < 0 || to_row < 0 { continue; }
                if to_col >= num_cols as i32 || to_row >= num_rows as i32 { continue; }

                let to_x = to_col as f32 * cell_w + cell_w / 2.0;
                let to_y = to_row as f32 * cell_h + cell_h / 2.0;

                let dx = to_x - from_x;
                let dy = to_y - from_y;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1.0 { continue; }
                let shorten = 15.0;
                let ux = dx / len;
                let uy = dy / len;
                let x1 = from_x + ux * shorten;
                let y1 = from_y + uy * shorten;
                let x2 = to_x - ux * shorten;
                let y2 = to_y - uy * shorten;

                if arrow.no_head {
                    // Just a line, no arrowhead
                    if arrow.dashed {
                        // Draw dashed line as segments
                        draw_dashed_line(x1, y1, x2, y2, 0.5, Color::BLACK, &mut elements);
                    } else {
                        elements.push(TikzElement::Line {
                            x1, y1, x2, y2, width: 0.5, color: Color::BLACK,
                        });
                    }
                } else if arrow.dashed {
                    draw_dashed_line(x1, y1, x2, y2, 0.5, Color::BLACK, &mut elements);
                    // Add arrowhead
                    add_arrowhead(x2, y2, ux, uy, Color::BLACK, &mut elements);
                } else {
                    elements.push(TikzElement::Arrow {
                        x1, y1, x2, y2,
                        width: 0.5, color: Color::BLACK,
                        bidirectional: false,
                    });
                }

                // Hook marker at start
                if arrow.hook {
                    let hook_len = 3.0;
                    let px = -uy; // perpendicular
                    let py = ux;
                    elements.push(TikzElement::Line {
                        x1: x1 - px * hook_len, y1: y1 - py * hook_len,
                        x2: x1 + px * hook_len, y2: y1 + py * hook_len,
                        width: 0.5, color: Color::BLACK,
                    });
                }

                // Maps-to marker at start
                if arrow.maps_to {
                    let bar_len = 3.0;
                    let px = -uy;
                    let py = ux;
                    elements.push(TikzElement::Line {
                        x1: x1 + px * bar_len, y1: y1 + py * bar_len,
                        x2: x1 - px * bar_len, y2: y1 - py * bar_len,
                        width: 0.8, color: Color::BLACK,
                    });
                }

                // Two-heads marker at end
                if arrow.two_heads {
                    let back = 4.0;
                    let hx = x2 - ux * back;
                    let hy = y2 - uy * back;
                    add_arrowhead(hx, hy, ux, uy, Color::BLACK, &mut elements);
                }

                // Render label
                if let Some(ref label_text) = arrow.label {
                    let lw = text_width(label_text, font_size * 0.85);
                    let mid_x = (x1 + x2) / 2.0;
                    let mid_y = (y1 + y2) / 2.0;
                    let offset = if arrow.label_above { -8.0 } else { 10.0 };
                    let lx;
                    let ly;
                    if dx.abs() > dy.abs() {
                        lx = mid_x - lw / 2.0;
                        ly = mid_y + offset;
                    } else {
                        lx = mid_x + offset - lw / 2.0;
                        ly = mid_y;
                    }
                    elements.push(TikzElement::Text {
                        x: lx, y: ly,
                        text: label_text.clone(),
                        font_size: font_size * 0.85,
                        bold: false,
                        color: Color::BLACK,
                    });
                }
            }
        }
    }

    TikzRenderResult { elements, width: total_w, height: total_h }
}

// ─── Drawing helpers ─────────────────────────────────────────────────

fn draw_dashed_line(x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color, elements: &mut Vec<TikzElement>) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 { return; }
    let dash_len = 4.0;
    let gap_len = 3.0;
    let ux = dx / len;
    let uy = dy / len;
    let mut t = 0.0;
    while t < len {
        let t_end = (t + dash_len).min(len);
        elements.push(TikzElement::Line {
            x1: x1 + ux * t, y1: y1 + uy * t,
            x2: x1 + ux * t_end, y2: y1 + uy * t_end,
            width, color,
        });
        t += dash_len + gap_len;
    }
}

fn add_arrowhead(x: f32, y: f32, ux: f32, uy: f32, color: Color, elements: &mut Vec<TikzElement>) {
    let arr_len = 6.0;
    let spread = 0.35;
    let angle = uy.atan2(ux);
    let a1x = x - arr_len * (angle - spread).cos();
    let a1y = y - arr_len * (angle - spread).sin();
    let a2x = x - arr_len * (angle + spread).cos();
    let a2y = y - arr_len * (angle + spread).sin();
    elements.push(TikzElement::Line { x1: x, y1: y, x2: a1x, y2: a1y, width: 0.5, color });
    elements.push(TikzElement::Line { x1: x, y1: y, x2: a2x, y2: a2y, width: 0.5, color });
}

/// Compute bounding box of elements
fn bounding_box(elements: &[TikzElement]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for elem in elements {
        match elem {
            TikzElement::Rect { x, y, width, height, .. } => {
                min_x = min_x.min(*x);
                min_y = min_y.min(*y);
                max_x = max_x.max(x + width);
                max_y = max_y.max(y + height);
            }
            TikzElement::Line { x1, y1, x2, y2, .. } | TikzElement::Arrow { x1, y1, x2, y2, .. } => {
                min_x = min_x.min(*x1).min(*x2);
                min_y = min_y.min(*y1).min(*y2);
                max_x = max_x.max(*x1).max(*x2);
                max_y = max_y.max(*y1).max(*y2);
            }
            TikzElement::Text { x, y, font_size, text, .. } => {
                min_x = min_x.min(*x);
                min_y = min_y.min(y - font_size);
                let tw = text_width(text, *font_size);
                max_x = max_x.max(x + tw);
                max_y = max_y.max(*y + 2.0);
            }
        }
    }

    if min_x > max_x { (0.0, 0.0, 100.0, 100.0) } else { (min_x, min_y, max_x, max_y) }
}

/// Offset all elements by (dx, dy)
fn offset_elements(elements: &mut [TikzElement], dx: f32, dy: f32) {
    for elem in elements.iter_mut() {
        match elem {
            TikzElement::Rect { x, y, .. } => { *x += dx; *y += dy; }
            TikzElement::Line { x1, y1, x2, y2, .. } => {
                *x1 += dx; *y1 += dy; *x2 += dx; *y2 += dy;
            }
            TikzElement::Arrow { x1, y1, x2, y2, .. } => {
                *x1 += dx; *y1 += dy; *x2 += dx; *y2 += dy;
            }
            TikzElement::Text { x, y, .. } => { *x += dx; *y += dy; }
        }
    }
}

/// Detect which diagram type a tikz-tagged verbatim node contains, and
/// dispatch to the appropriate renderer.  Returns `None` if it's not one
/// of the special diagram types (caller should fall through to generic TikZ).
pub fn try_render_diagram(env_name: &str, source: &str) -> Option<TikzRenderResult> {
    match env_name {
        "tikzcd" => Some(render_tikzcd(source)),
        "circuitikz" => Some(render_circuitikz(source)),
        "forest" => Some(render_forest(source)),
        "xymatrix" | "xy" => Some(render_xymatrix(source)),
        _ => {
            // Check for \xymatrix inside the source (e.g. inside a tikzpicture or $$ block)
            if source.contains("\\xymatrix") {
                Some(render_xymatrix(source))
            } else {
                None
            }
        }
    }
}
