/// Minimal SVG to PDF content stream converter
///
/// Converts basic SVG elements (rect, circle, ellipse, line, polyline, polygon,
/// path, text, g) to PDF drawing operators for embedding as Form XObjects.

/// Parsed SVG document ready for PDF conversion
#[derive(Debug)]
pub struct SvgDocument {
    pub width: f32,
    pub height: f32,
    pub view_box: (f32, f32, f32, f32), // min_x, min_y, width, height
    pub elements: Vec<SvgElement>,
}

#[derive(Debug, Clone)]
pub enum SvgElement {
    Rect {
        x: f32, y: f32, width: f32, height: f32,
        rx: f32, ry: f32,
        style: SvgStyle,
    },
    Circle {
        cx: f32, cy: f32, r: f32,
        style: SvgStyle,
    },
    Ellipse {
        cx: f32, cy: f32, rx: f32, ry: f32,
        style: SvgStyle,
    },
    Line {
        x1: f32, y1: f32, x2: f32, y2: f32,
        style: SvgStyle,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        style: SvgStyle,
    },
    Polygon {
        points: Vec<(f32, f32)>,
        style: SvgStyle,
    },
    Path {
        commands: Vec<PathCommand>,
        style: SvgStyle,
    },
    Text {
        x: f32, y: f32,
        text: String,
        font_size: f32,
        style: SvgStyle,
    },
    Group {
        transform: Option<Transform>,
        children: Vec<SvgElement>,
        style: SvgStyle,
    },
}

#[derive(Debug, Clone, Default)]
pub struct SvgStyle {
    pub fill: Option<PdfColor>,
    pub stroke: Option<PdfColor>,
    pub stroke_width: f32,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub stroke_opacity: f32,
    pub no_fill: bool, // fill="none"
    pub no_stroke: bool, // stroke="none"
}

impl SvgStyle {
    fn new() -> Self {
        SvgStyle {
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            no_fill: false,
            no_stroke: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PdfColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl PdfColor {
    fn black() -> Self { PdfColor { r: 0.0, g: 0.0, b: 0.0 } }
}

#[derive(Debug, Clone)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32), // c1x, c1y, c2x, c2y, x, y
    QuadTo(f32, f32, f32, f32),             // cx, cy, x, y
    ClosePath,
}

#[derive(Debug, Clone)]
pub enum Transform {
    Translate(f32, f32),
    Scale(f32, f32),
    Rotate(f32),
    Matrix(f32, f32, f32, f32, f32, f32), // a, b, c, d, e, f
}

/// Parse an SVG string and return document structure, or None if invalid
pub fn parse_svg(svg_data: &str) -> Option<SvgDocument> {
    // Find <svg> tag and extract dimensions
    let svg_start = svg_data.find("<svg")?;
    let svg_tag_end = svg_data[svg_start..].find('>')? + svg_start;
    let svg_tag = &svg_data[svg_start..=svg_tag_end];

    let (width, height) = parse_svg_dimensions(svg_tag);
    let view_box = parse_viewbox(svg_tag).unwrap_or((0.0, 0.0, width, height));

    // Parse elements between <svg> and </svg>
    let content_start = svg_tag_end + 1;
    let content_end = svg_data.rfind("</svg>").unwrap_or(svg_data.len());
    let content = &svg_data[content_start..content_end];

    let elements = parse_elements(content);

    Some(SvgDocument {
        width,
        height,
        view_box,
        elements,
    })
}

/// Convert parsed SVG to PDF content stream bytes
pub fn svg_to_pdf_content(doc: &SvgDocument) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);

    // Apply viewBox transform: scale from viewBox coordinates to width/height
    let vb = doc.view_box;
    if vb.2 > 0.0 && vb.3 > 0.0 {
        let sx = doc.width / vb.2;
        let sy = doc.height / vb.3;
        let tx = -vb.0 * sx;
        let ty = -vb.1 * sy;
        if (sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001
            || tx.abs() > 0.001 || ty.abs() > 0.001
        {
            write_f(&mut buf, sx);
            buf.extend_from_slice(b" 0 0 ");
            write_f(&mut buf, sy);
            buf.push(b' ');
            write_f(&mut buf, tx);
            buf.push(b' ');
            write_f(&mut buf, ty);
            buf.extend_from_slice(b" cm\n");
        }
    }

    for elem in &doc.elements {
        render_element(&mut buf, elem);
    }

    buf
}

fn render_element(buf: &mut Vec<u8>, elem: &SvgElement) {
    match elem {
        SvgElement::Rect { x, y, width, height, rx, ry, style } => {
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            if *rx > 0.0 || *ry > 0.0 {
                render_rounded_rect(buf, *x, *y, *width, *height, rx.max(*ry));
            } else {
                write_f(buf, *x); buf.push(b' ');
                write_f(buf, *y); buf.push(b' ');
                write_f(buf, *width); buf.push(b' ');
                write_f(buf, *height);
                buf.extend_from_slice(b" re\n");
            }
            paint(buf, style);
            buf.extend_from_slice(b"Q\n");
        }

        SvgElement::Circle { cx, cy, r, style } => {
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            render_ellipse_path(buf, *cx, *cy, *r, *r);
            paint(buf, style);
            buf.extend_from_slice(b"Q\n");
        }

        SvgElement::Ellipse { cx, cy, rx, ry, style } => {
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            render_ellipse_path(buf, *cx, *cy, *rx, *ry);
            paint(buf, style);
            buf.extend_from_slice(b"Q\n");
        }

        SvgElement::Line { x1, y1, x2, y2, style } => {
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            write_f(buf, *x1); buf.push(b' '); write_f(buf, *y1);
            buf.extend_from_slice(b" m\n");
            write_f(buf, *x2); buf.push(b' '); write_f(buf, *y2);
            buf.extend_from_slice(b" l\nS\nQ\n");
        }

        SvgElement::Polyline { points, style } => {
            if points.is_empty() { return; }
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            write_f(buf, points[0].0); buf.push(b' '); write_f(buf, points[0].1);
            buf.extend_from_slice(b" m\n");
            for &(x, y) in &points[1..] {
                write_f(buf, x); buf.push(b' '); write_f(buf, y);
                buf.extend_from_slice(b" l\n");
            }
            buf.extend_from_slice(b"S\nQ\n");
        }

        SvgElement::Polygon { points, style } => {
            if points.is_empty() { return; }
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            write_f(buf, points[0].0); buf.push(b' '); write_f(buf, points[0].1);
            buf.extend_from_slice(b" m\n");
            for &(x, y) in &points[1..] {
                write_f(buf, x); buf.push(b' '); write_f(buf, y);
                buf.extend_from_slice(b" l\n");
            }
            buf.extend_from_slice(b"h\n");
            paint(buf, style);
            buf.extend_from_slice(b"Q\n");
        }

        SvgElement::Path { commands, style } => {
            buf.extend_from_slice(b"q\n");
            apply_style(buf, style);
            for cmd in commands {
                match cmd {
                    PathCommand::MoveTo(x, y) => {
                        write_f(buf, *x); buf.push(b' '); write_f(buf, *y);
                        buf.extend_from_slice(b" m\n");
                    }
                    PathCommand::LineTo(x, y) => {
                        write_f(buf, *x); buf.push(b' '); write_f(buf, *y);
                        buf.extend_from_slice(b" l\n");
                    }
                    PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                        write_f(buf, *c1x); buf.push(b' '); write_f(buf, *c1y); buf.push(b' ');
                        write_f(buf, *c2x); buf.push(b' '); write_f(buf, *c2y); buf.push(b' ');
                        write_f(buf, *x); buf.push(b' '); write_f(buf, *y);
                        buf.extend_from_slice(b" c\n");
                    }
                    PathCommand::QuadTo(cx, cy, x, y) => {
                        // PDF doesn't have quadratic curves; approximate with cubic
                        // Not perfect without current point, but close enough
                        // We emit as cubic: the caller should convert properly
                        write_f(buf, *cx); buf.push(b' '); write_f(buf, *cy); buf.push(b' ');
                        write_f(buf, *cx); buf.push(b' '); write_f(buf, *cy); buf.push(b' ');
                        write_f(buf, *x); buf.push(b' '); write_f(buf, *y);
                        buf.extend_from_slice(b" c\n");
                    }
                    PathCommand::ClosePath => {
                        buf.extend_from_slice(b"h\n");
                    }
                }
            }
            paint(buf, style);
            buf.extend_from_slice(b"Q\n");
        }

        SvgElement::Text { x, y, text, font_size, style } => {
            buf.extend_from_slice(b"q\n");
            if let Some(c) = &style.fill {
                write_f(buf, c.r); buf.push(b' ');
                write_f(buf, c.g); buf.push(b' ');
                write_f(buf, c.b);
                buf.extend_from_slice(b" rg\n");
            } else if !style.no_fill {
                buf.extend_from_slice(b"0 0 0 rg\n");
            }
            buf.extend_from_slice(b"BT\n/F1 ");
            write_f(buf, *font_size);
            buf.extend_from_slice(b" Tf\n");
            write_f(buf, *x); buf.push(b' '); write_f(buf, *y);
            buf.extend_from_slice(b" Td\n(");
            // Escape PDF string
            for &b_val in text.as_bytes() {
                match b_val {
                    b'(' => buf.extend_from_slice(b"\\("),
                    b')' => buf.extend_from_slice(b"\\)"),
                    b'\\' => buf.extend_from_slice(b"\\\\"),
                    _ => buf.push(b_val),
                }
            }
            buf.extend_from_slice(b") Tj\nET\nQ\n");
        }

        SvgElement::Group { transform, children, style: _ } => {
            buf.extend_from_slice(b"q\n");
            if let Some(t) = transform {
                match t {
                    Transform::Translate(tx, ty) => {
                        buf.extend_from_slice(b"1 0 0 1 ");
                        write_f(buf, *tx); buf.push(b' ');
                        write_f(buf, *ty);
                        buf.extend_from_slice(b" cm\n");
                    }
                    Transform::Scale(sx, sy) => {
                        write_f(buf, *sx);
                        buf.extend_from_slice(b" 0 0 ");
                        write_f(buf, *sy);
                        buf.extend_from_slice(b" 0 0 cm\n");
                    }
                    Transform::Rotate(angle) => {
                        let rad = angle * std::f32::consts::PI / 180.0;
                        let cos_a = rad.cos();
                        let sin_a = rad.sin();
                        write_f(buf, cos_a); buf.push(b' ');
                        write_f(buf, sin_a); buf.push(b' ');
                        write_f(buf, -sin_a); buf.push(b' ');
                        write_f(buf, cos_a);
                        buf.extend_from_slice(b" 0 0 cm\n");
                    }
                    Transform::Matrix(a, b_val, c, d, e, f) => {
                        write_f(buf, *a); buf.push(b' ');
                        write_f(buf, *b_val); buf.push(b' ');
                        write_f(buf, *c); buf.push(b' ');
                        write_f(buf, *d); buf.push(b' ');
                        write_f(buf, *e); buf.push(b' ');
                        write_f(buf, *f);
                        buf.extend_from_slice(b" cm\n");
                    }
                }
            }
            for child in children {
                render_element(buf, child);
            }
            buf.extend_from_slice(b"Q\n");
        }
    }
}

fn apply_style(buf: &mut Vec<u8>, style: &SvgStyle) {
    if let Some(c) = &style.fill {
        if !style.no_fill {
            write_f(buf, c.r); buf.push(b' ');
            write_f(buf, c.g); buf.push(b' ');
            write_f(buf, c.b);
            buf.extend_from_slice(b" rg\n");
        }
    }
    if let Some(c) = &style.stroke {
        if !style.no_stroke {
            write_f(buf, c.r); buf.push(b' ');
            write_f(buf, c.g); buf.push(b' ');
            write_f(buf, c.b);
            buf.extend_from_slice(b" RG\n");
        }
    }
    if style.stroke_width != 1.0 {
        write_f(buf, style.stroke_width);
        buf.extend_from_slice(b" w\n");
    }
}

fn paint(buf: &mut Vec<u8>, style: &SvgStyle) {
    let has_fill = !style.no_fill && (style.fill.is_some() || (!style.no_fill && style.stroke.is_none() && !style.no_stroke));
    let has_stroke = !style.no_stroke && style.stroke.is_some();
    match (has_fill, has_stroke) {
        (true, true) => buf.extend_from_slice(b"B\n"),   // fill + stroke
        (true, false) => buf.extend_from_slice(b"f\n"),  // fill only
        (false, true) => buf.extend_from_slice(b"S\n"),  // stroke only
        (false, false) => {
            // Default SVG: fill black if no stroke specified
            if !style.no_fill {
                buf.extend_from_slice(b"f\n");
            }
        }
    }
}

/// Approximate an ellipse with 4 cubic Bézier curves
fn render_ellipse_path(buf: &mut Vec<u8>, cx: f32, cy: f32, rx: f32, ry: f32) {
    // Magic number for circular Bézier approximation: 4*(sqrt(2)-1)/3 ≈ 0.5523
    let k: f32 = 0.5523;
    let kx = rx * k;
    let ky = ry * k;

    // Start at right
    write_f(buf, cx + rx); buf.push(b' '); write_f(buf, cy);
    buf.extend_from_slice(b" m\n");
    // Top-right quadrant
    write_f(buf, cx + rx); buf.push(b' '); write_f(buf, cy - ky); buf.push(b' ');
    write_f(buf, cx + kx); buf.push(b' '); write_f(buf, cy - ry); buf.push(b' ');
    write_f(buf, cx); buf.push(b' '); write_f(buf, cy - ry);
    buf.extend_from_slice(b" c\n");
    // Top-left quadrant
    write_f(buf, cx - kx); buf.push(b' '); write_f(buf, cy - ry); buf.push(b' ');
    write_f(buf, cx - rx); buf.push(b' '); write_f(buf, cy - ky); buf.push(b' ');
    write_f(buf, cx - rx); buf.push(b' '); write_f(buf, cy);
    buf.extend_from_slice(b" c\n");
    // Bottom-left quadrant
    write_f(buf, cx - rx); buf.push(b' '); write_f(buf, cy + ky); buf.push(b' ');
    write_f(buf, cx - kx); buf.push(b' '); write_f(buf, cy + ry); buf.push(b' ');
    write_f(buf, cx); buf.push(b' '); write_f(buf, cy + ry);
    buf.extend_from_slice(b" c\n");
    // Bottom-right quadrant
    write_f(buf, cx + kx); buf.push(b' '); write_f(buf, cy + ry); buf.push(b' ');
    write_f(buf, cx + rx); buf.push(b' '); write_f(buf, cy + ky); buf.push(b' ');
    write_f(buf, cx + rx); buf.push(b' '); write_f(buf, cy);
    buf.extend_from_slice(b" c\n");
    buf.extend_from_slice(b"h\n");
}

fn render_rounded_rect(buf: &mut Vec<u8>, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k: f32 = 0.5523 * r;

    // Start at top-left + r
    write_f(buf, x + r); buf.push(b' '); write_f(buf, y);
    buf.extend_from_slice(b" m\n");
    // Top edge
    write_f(buf, x + w - r); buf.push(b' '); write_f(buf, y);
    buf.extend_from_slice(b" l\n");
    // Top-right corner
    write_f(buf, x + w - r + k); buf.push(b' '); write_f(buf, y); buf.push(b' ');
    write_f(buf, x + w); buf.push(b' '); write_f(buf, y + r - k); buf.push(b' ');
    write_f(buf, x + w); buf.push(b' '); write_f(buf, y + r);
    buf.extend_from_slice(b" c\n");
    // Right edge
    write_f(buf, x + w); buf.push(b' '); write_f(buf, y + h - r);
    buf.extend_from_slice(b" l\n");
    // Bottom-right corner
    write_f(buf, x + w); buf.push(b' '); write_f(buf, y + h - r + k); buf.push(b' ');
    write_f(buf, x + w - r + k); buf.push(b' '); write_f(buf, y + h); buf.push(b' ');
    write_f(buf, x + w - r); buf.push(b' '); write_f(buf, y + h);
    buf.extend_from_slice(b" c\n");
    // Bottom edge
    write_f(buf, x + r); buf.push(b' '); write_f(buf, y + h);
    buf.extend_from_slice(b" l\n");
    // Bottom-left corner
    write_f(buf, x + r - k); buf.push(b' '); write_f(buf, y + h); buf.push(b' ');
    write_f(buf, x); buf.push(b' '); write_f(buf, y + h - r + k); buf.push(b' ');
    write_f(buf, x); buf.push(b' '); write_f(buf, y + h - r);
    buf.extend_from_slice(b" c\n");
    // Left edge
    write_f(buf, x); buf.push(b' '); write_f(buf, y + r);
    buf.extend_from_slice(b" l\n");
    // Top-left corner
    write_f(buf, x); buf.push(b' '); write_f(buf, y + r - k); buf.push(b' ');
    write_f(buf, x + r - k); buf.push(b' '); write_f(buf, y); buf.push(b' ');
    write_f(buf, x + r); buf.push(b' '); write_f(buf, y);
    buf.extend_from_slice(b" c\n");
    buf.extend_from_slice(b"h\n");
}

// --- SVG XML Parsing helpers (simple string-based, no XML library) ---

fn parse_svg_dimensions(svg_tag: &str) -> (f32, f32) {
    let w = attr_value(svg_tag, "width").and_then(|v| parse_length(v)).unwrap_or(300.0);
    let h = attr_value(svg_tag, "height").and_then(|v| parse_length(v)).unwrap_or(150.0);
    (w, h)
}

fn parse_viewbox(tag: &str) -> Option<(f32, f32, f32, f32)> {
    let vb = attr_value(tag, "viewBox")?;
    let nums: Vec<f32> = vb.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() >= 4 {
        Some((nums[0], nums[1], nums[2], nums[3]))
    } else {
        None
    }
}

fn parse_length(s: &str) -> Option<f32> {
    let s = s.trim();
    // Strip units and convert to points
    if let Some(v) = s.strip_suffix("px") {
        v.trim().parse().ok()
    } else if let Some(v) = s.strip_suffix("pt") {
        v.trim().parse().ok()
    } else if let Some(v) = s.strip_suffix("mm") {
        v.trim().parse::<f32>().ok().map(|v| v * 2.8346)
    } else if let Some(v) = s.strip_suffix("cm") {
        v.trim().parse::<f32>().ok().map(|v| v * 28.346)
    } else if let Some(v) = s.strip_suffix("in") {
        v.trim().parse::<f32>().ok().map(|v| v * 72.0)
    } else if let Some(v) = s.strip_suffix("em") {
        v.trim().parse::<f32>().ok().map(|v| v * 12.0)
    } else if s.ends_with('%') {
        // Percentage: treat as fraction of default
        None
    } else {
        s.parse().ok()
    }
}

/// Extract attribute value from an XML tag string
fn attr_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    // Try name="value" and name='value'
    for quote in ['"', '\''] {
        let pattern = format!("{}={}", name, quote);
        if let Some(start) = tag.find(&pattern) {
            let val_start = start + pattern.len();
            if let Some(end) = tag[val_start..].find(quote) {
                return Some(&tag[val_start..val_start + end]);
            }
        }
        // Also try with space before =
        let pattern2 = format!("{} ={}", name, quote);
        if let Some(start) = tag.find(&pattern2) {
            let val_start = start + pattern2.len();
            if let Some(end) = tag[val_start..].find(quote) {
                return Some(&tag[val_start..val_start + end]);
            }
        }
    }
    None
}

fn parse_elements(content: &str) -> Vec<SvgElement> {
    let mut elements = Vec::new();
    let mut pos = 0;
    let bytes = content.as_bytes();

    while pos < bytes.len() {
        // Skip to next '<'
        match memchr::memchr(b'<', &bytes[pos..]) {
            Some(offset) => pos += offset,
            None => break,
        }

        // Skip comments <!-- ... -->
        if content[pos..].starts_with("<!--") {
            if let Some(end) = content[pos..].find("-->") {
                pos += end + 3;
                continue;
            }
            break;
        }

        // Skip processing instructions, CDATA, DOCTYPE
        if content[pos..].starts_with("<?") || content[pos..].starts_with("<!")  {
            if let Some(end) = memchr::memchr(b'>', &bytes[pos..]) {
                pos += end + 1;
                continue;
            }
            break;
        }

        // Skip closing tags
        if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
            if let Some(end) = memchr::memchr(b'>', &bytes[pos..]) {
                pos += end + 1;
                continue;
            }
            break;
        }

        // Find tag name
        let tag_start = pos + 1;
        let mut tag_name_end = tag_start;
        while tag_name_end < bytes.len() && bytes[tag_name_end] != b' '
            && bytes[tag_name_end] != b'>' && bytes[tag_name_end] != b'/'
            && bytes[tag_name_end] != b'\n' && bytes[tag_name_end] != b'\t'
        {
            tag_name_end += 1;
        }
        if tag_name_end >= bytes.len() { break; }

        let tag_name = &content[tag_start..tag_name_end];

        // Find end of opening tag (could be self-closing />)
        let tag_end = match find_tag_end(content, pos) {
            Some(e) => e,
            None => break,
        };
        let is_self_closing = tag_end > 0 && bytes[tag_end - 1] == b'/';
        let tag_str = &content[pos..=tag_end];

        match tag_name {
            "rect" => {
                if let Some(elem) = parse_rect(tag_str) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "circle" => {
                if let Some(elem) = parse_circle(tag_str) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "ellipse" => {
                if let Some(elem) = parse_ellipse(tag_str) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "line" => {
                if let Some(elem) = parse_line_element(tag_str) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "polyline" => {
                if let Some(elem) = parse_polyline(tag_str, false) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "polygon" => {
                if let Some(elem) = parse_polyline(tag_str, true) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "path" => {
                if let Some(elem) = parse_path_element(tag_str) {
                    elements.push(elem);
                }
                pos = tag_end + 1;
            }
            "text" => {
                // Parse text element — need content between tags
                pos = tag_end + 1;
                if !is_self_closing {
                    if let Some(close_pos) = content[pos..].find("</text>") {
                        let text_content = &content[pos..pos + close_pos];
                        // Strip any inner <tspan> tags to get raw text
                        let clean_text = strip_tags(text_content);
                        let x = attr_f32(tag_str, "x").unwrap_or(0.0);
                        let y = attr_f32(tag_str, "y").unwrap_or(0.0);
                        let font_size = attr_f32(tag_str, "font-size")
                            .or_else(|| style_attr_f32(tag_str, "font-size"))
                            .unwrap_or(12.0);
                        let style = parse_style(tag_str);
                        elements.push(SvgElement::Text {
                            x, y, text: clean_text, font_size, style,
                        });
                        pos += close_pos + 7; // skip </text>
                    }
                }
            }
            "g" => {
                pos = tag_end + 1;
                if !is_self_closing {
                    // Find matching </g> — handle nesting
                    if let Some((inner, end_pos)) = find_matching_close(content, pos, "g") {
                        let transform = attr_value(tag_str, "transform")
                            .and_then(parse_transform);
                        let style = parse_style(tag_str);
                        let children = parse_elements(inner);
                        elements.push(SvgElement::Group { transform, children, style });
                        pos = end_pos;
                    }
                }
            }
            // Skip: defs, style, metadata, title, desc, clipPath, mask, filter, etc.
            _ => {
                pos = tag_end + 1;
                // If not self-closing, skip to matching close tag
                if !is_self_closing {
                    let close_tag = format!("</{}>", tag_name);
                    if let Some(close_pos) = content[pos..].find(&close_tag) {
                        pos += close_pos + close_tag.len();
                    }
                }
            }
        }
    }

    elements
}

fn find_tag_end(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut i = start;
    let mut in_quote = false;
    let mut quote_char = b'"';
    while i < bytes.len() {
        if in_quote {
            if bytes[i] == quote_char {
                in_quote = false;
            }
        } else {
            match bytes[i] {
                b'"' | b'\'' => {
                    in_quote = true;
                    quote_char = bytes[i];
                }
                b'>' => return Some(i),
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn find_matching_close<'a>(content: &'a str, start: usize, tag: &str) -> Option<(&'a str, usize)> {
    let open_tag = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);
    let mut depth = 1;
    let mut pos = start;

    while pos < content.len() && depth > 0 {
        // Find next < that could be open or close of our tag
        let next_open = content[pos..].find(&open_tag);
        let next_close = content[pos..].find(&close_tag);

        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                // Check if this open tag is actually our tag (not e.g. <gradient when tag=g)
                let after = pos + o + open_tag.len();
                if after < content.len() {
                    let ch = content.as_bytes()[after];
                    if ch == b' ' || ch == b'>' || ch == b'/' || ch == b'\n' || ch == b'\t' {
                        depth += 1;
                    }
                }
                pos += o + 1;
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    let inner = &content[start..pos + c];
                    return Some((inner, pos + c + close_tag.len()));
                }
                pos += c + close_tag.len();
            }
            (Some(o), None) => {
                pos += o + 1;
            }
            (None, None) => break,
        }
    }
    None
}

fn strip_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' { in_tag = true; }
        else if ch == '>' { in_tag = false; }
        else if !in_tag { result.push(ch); }
    }
    result.trim().to_string()
}

fn parse_rect(tag: &str) -> Option<SvgElement> {
    let x = attr_f32(tag, "x").unwrap_or(0.0);
    let y = attr_f32(tag, "y").unwrap_or(0.0);
    let w = attr_f32(tag, "width")?;
    let h = attr_f32(tag, "height")?;
    let rx = attr_f32(tag, "rx").unwrap_or(0.0);
    let ry = attr_f32(tag, "ry").unwrap_or(0.0);
    Some(SvgElement::Rect { x, y, width: w, height: h, rx, ry, style: parse_style(tag) })
}

fn parse_circle(tag: &str) -> Option<SvgElement> {
    let cx = attr_f32(tag, "cx").unwrap_or(0.0);
    let cy = attr_f32(tag, "cy").unwrap_or(0.0);
    let r = attr_f32(tag, "r")?;
    Some(SvgElement::Circle { cx, cy, r, style: parse_style(tag) })
}

fn parse_ellipse(tag: &str) -> Option<SvgElement> {
    let cx = attr_f32(tag, "cx").unwrap_or(0.0);
    let cy = attr_f32(tag, "cy").unwrap_or(0.0);
    let rx = attr_f32(tag, "rx")?;
    let ry = attr_f32(tag, "ry")?;
    Some(SvgElement::Ellipse { cx, cy, rx, ry, style: parse_style(tag) })
}

fn parse_line_element(tag: &str) -> Option<SvgElement> {
    let x1 = attr_f32(tag, "x1").unwrap_or(0.0);
    let y1 = attr_f32(tag, "y1").unwrap_or(0.0);
    let x2 = attr_f32(tag, "x2").unwrap_or(0.0);
    let y2 = attr_f32(tag, "y2").unwrap_or(0.0);
    let mut style = parse_style(tag);
    // Lines default to stroke=black if no stroke set
    if style.stroke.is_none() && !style.no_stroke {
        style.stroke = Some(PdfColor::black());
    }
    Some(SvgElement::Line { x1, y1, x2, y2, style })
}

fn parse_polyline(tag: &str, is_polygon: bool) -> Option<SvgElement> {
    let points_str = attr_value(tag, "points")?;
    let points = parse_points(points_str);
    if points.is_empty() { return None; }
    let style = parse_style(tag);
    if is_polygon {
        Some(SvgElement::Polygon { points, style })
    } else {
        Some(SvgElement::Polyline { points, style })
    }
}

fn parse_points(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    nums.chunks(2).filter_map(|c| {
        if c.len() == 2 { Some((c[0], c[1])) } else { None }
    }).collect()
}

fn parse_path_element(tag: &str) -> Option<SvgElement> {
    let d = attr_value(tag, "d")?;
    let commands = parse_path_data(d);
    if commands.is_empty() { return None; }
    Some(SvgElement::Path { commands, style: parse_style(tag) })
}

/// Parse SVG path data string into PathCommands
fn parse_path_data(d: &str) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    let mut cur_x: f32 = 0.0;
    let mut cur_y: f32 = 0.0;
    let mut start_x: f32 = 0.0;
    let mut start_y: f32 = 0.0;

    let tokens = tokenize_path(d);
    let mut i = 0;

    while i < tokens.len() {
        match tokens[i] {
            PathToken::Command(cmd) => {
                i += 1;
                match cmd {
                    b'M' => {
                        // Absolute moveto; subsequent pairs are implicit LineTo
                        let mut first = true;
                        while i + 1 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let x = tokens[i].as_f32();
                            let y = tokens[i + 1].as_f32();
                            i += 2;
                            if first {
                                commands.push(PathCommand::MoveTo(x, y));
                                start_x = x; start_y = y;
                                first = false;
                            } else {
                                commands.push(PathCommand::LineTo(x, y));
                            }
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'm' => {
                        let mut first = true;
                        while i + 1 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let dx = tokens[i].as_f32();
                            let dy = tokens[i + 1].as_f32();
                            i += 2;
                            cur_x += dx; cur_y += dy;
                            if first {
                                commands.push(PathCommand::MoveTo(cur_x, cur_y));
                                start_x = cur_x; start_y = cur_y;
                                first = false;
                            } else {
                                commands.push(PathCommand::LineTo(cur_x, cur_y));
                            }
                        }
                    }
                    b'L' => {
                        while i + 1 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let x = tokens[i].as_f32();
                            let y = tokens[i + 1].as_f32();
                            i += 2;
                            commands.push(PathCommand::LineTo(x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'l' => {
                        while i + 1 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let dx = tokens[i].as_f32();
                            let dy = tokens[i + 1].as_f32();
                            i += 2;
                            cur_x += dx; cur_y += dy;
                            commands.push(PathCommand::LineTo(cur_x, cur_y));
                        }
                    }
                    b'H' => {
                        while i < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            cur_x = tokens[i].as_f32();
                            i += 1;
                            commands.push(PathCommand::LineTo(cur_x, cur_y));
                        }
                    }
                    b'h' => {
                        while i < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            cur_x += tokens[i].as_f32();
                            i += 1;
                            commands.push(PathCommand::LineTo(cur_x, cur_y));
                        }
                    }
                    b'V' => {
                        while i < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            cur_y = tokens[i].as_f32();
                            i += 1;
                            commands.push(PathCommand::LineTo(cur_x, cur_y));
                        }
                    }
                    b'v' => {
                        while i < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            cur_y += tokens[i].as_f32();
                            i += 1;
                            commands.push(PathCommand::LineTo(cur_x, cur_y));
                        }
                    }
                    b'C' => {
                        while i + 5 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let c1x = tokens[i].as_f32();
                            let c1y = tokens[i+1].as_f32();
                            let c2x = tokens[i+2].as_f32();
                            let c2y = tokens[i+3].as_f32();
                            let x = tokens[i+4].as_f32();
                            let y = tokens[i+5].as_f32();
                            i += 6;
                            commands.push(PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'c' => {
                        while i + 5 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let c1x = cur_x + tokens[i].as_f32();
                            let c1y = cur_y + tokens[i+1].as_f32();
                            let c2x = cur_x + tokens[i+2].as_f32();
                            let c2y = cur_y + tokens[i+3].as_f32();
                            let x = cur_x + tokens[i+4].as_f32();
                            let y = cur_y + tokens[i+5].as_f32();
                            i += 6;
                            commands.push(PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'S' | b's' => {
                        // Smooth cubic — for simplicity, treat control point as current
                        let relative = cmd == b's';
                        while i + 3 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let (c2x, c2y, x, y) = if relative {
                                (cur_x + tokens[i].as_f32(), cur_y + tokens[i+1].as_f32(),
                                 cur_x + tokens[i+2].as_f32(), cur_y + tokens[i+3].as_f32())
                            } else {
                                (tokens[i].as_f32(), tokens[i+1].as_f32(),
                                 tokens[i+2].as_f32(), tokens[i+3].as_f32())
                            };
                            i += 4;
                            // Reflect previous control point (simplified: use cur as c1)
                            commands.push(PathCommand::CubicTo(cur_x, cur_y, c2x, c2y, x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'Q' => {
                        while i + 3 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let cx = tokens[i].as_f32();
                            let cy = tokens[i+1].as_f32();
                            let x = tokens[i+2].as_f32();
                            let y = tokens[i+3].as_f32();
                            i += 4;
                            // Convert quadratic to cubic
                            let c1x = cur_x + 2.0/3.0 * (cx - cur_x);
                            let c1y = cur_y + 2.0/3.0 * (cy - cur_y);
                            let c2x = x + 2.0/3.0 * (cx - x);
                            let c2y = y + 2.0/3.0 * (cy - y);
                            commands.push(PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'q' => {
                        while i + 3 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let cx = cur_x + tokens[i].as_f32();
                            let cy = cur_y + tokens[i+1].as_f32();
                            let x = cur_x + tokens[i+2].as_f32();
                            let y = cur_y + tokens[i+3].as_f32();
                            i += 4;
                            let c1x = cur_x + 2.0/3.0 * (cx - cur_x);
                            let c1y = cur_y + 2.0/3.0 * (cy - cur_y);
                            let c2x = x + 2.0/3.0 * (cx - x);
                            let c2y = y + 2.0/3.0 * (cy - y);
                            commands.push(PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'T' | b't' => {
                        // Smooth quadratic — simplified: line to endpoint
                        let relative = cmd == b't';
                        while i + 1 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let (x, y) = if relative {
                                (cur_x + tokens[i].as_f32(), cur_y + tokens[i+1].as_f32())
                            } else {
                                (tokens[i].as_f32(), tokens[i+1].as_f32())
                            };
                            i += 2;
                            commands.push(PathCommand::LineTo(x, y));
                            cur_x = x; cur_y = y;
                        }
                    }
                    b'A' | b'a' => {
                        // Arc — approximate as line to endpoint (proper arc conversion is complex)
                        let relative = cmd == b'a';
                        while i + 6 < tokens.len() && matches!(tokens[i], PathToken::Number(_)) {
                            let _rx = tokens[i].as_f32();
                            let _ry = tokens[i+1].as_f32();
                            let _rot = tokens[i+2].as_f32();
                            let _large = tokens[i+3].as_f32();
                            let _sweep = tokens[i+4].as_f32();
                            let x = tokens[i+5].as_f32();
                            let y = tokens[i+6].as_f32();
                            i += 7;
                            let (ax, ay) = if relative {
                                (cur_x + x, cur_y + y)
                            } else {
                                (x, y)
                            };
                            commands.push(PathCommand::LineTo(ax, ay));
                            cur_x = ax; cur_y = ay;
                        }
                    }
                    b'Z' | b'z' => {
                        commands.push(PathCommand::ClosePath);
                        cur_x = start_x;
                        cur_y = start_y;
                    }
                    _ => {}
                }
            }
            PathToken::Number(_) => {
                // Stray number without command — skip
                i += 1;
            }
        }
    }

    commands
}

#[derive(Debug)]
enum PathToken {
    Command(u8),
    Number(f32),
}

impl PathToken {
    fn as_f32(&self) -> f32 {
        match self {
            PathToken::Number(n) => *n,
            PathToken::Command(_) => 0.0,
        }
    }
}

fn tokenize_path(d: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' | b',' => { i += 1; }
            b'M' | b'm' | b'L' | b'l' | b'H' | b'h' | b'V' | b'v'
            | b'C' | b'c' | b'S' | b's' | b'Q' | b'q' | b'T' | b't'
            | b'A' | b'a' | b'Z' | b'z' => {
                tokens.push(PathToken::Command(bytes[i]));
                i += 1;
            }
            b'0'..=b'9' | b'-' | b'+' | b'.' => {
                let start = i;
                if bytes[i] == b'-' || bytes[i] == b'+' { i += 1; }
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                // Handle scientific notation
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') { i += 1; }
                    while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                }
                if let Ok(n) = d[start..i].parse::<f32>() {
                    tokens.push(PathToken::Number(n));
                }
            }
            _ => { i += 1; }
        }
    }

    tokens
}

fn parse_style(tag: &str) -> SvgStyle {
    let mut style = SvgStyle::new();

    // Check inline style attribute first
    if let Some(style_str) = attr_value(tag, "style") {
        for decl in style_str.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                let prop = prop.trim();
                let val = val.trim();
                match prop {
                    "fill" => {
                        if val == "none" { style.no_fill = true; }
                        else if let Some(c) = parse_color(val) { style.fill = Some(c); }
                    }
                    "stroke" => {
                        if val == "none" { style.no_stroke = true; }
                        else if let Some(c) = parse_color(val) { style.stroke = Some(c); }
                    }
                    "stroke-width" => {
                        if let Some(w) = parse_length(val) { style.stroke_width = w; }
                    }
                    "opacity" => {
                        if let Ok(o) = val.parse() { style.opacity = o; }
                    }
                    "fill-opacity" => {
                        if let Ok(o) = val.parse() { style.fill_opacity = o; }
                    }
                    "stroke-opacity" => {
                        if let Ok(o) = val.parse() { style.stroke_opacity = o; }
                    }
                    _ => {}
                }
            }
        }
    }

    // Check individual attributes (override style)
    if let Some(fill) = attr_value(tag, "fill") {
        if fill == "none" { style.no_fill = true; }
        else if let Some(c) = parse_color(fill) { style.fill = Some(c); }
    }
    if let Some(stroke) = attr_value(tag, "stroke") {
        if stroke == "none" { style.no_stroke = true; }
        else if let Some(c) = parse_color(stroke) { style.stroke = Some(c); }
    }
    if let Some(sw) = attr_value(tag, "stroke-width") {
        if let Some(w) = parse_length(sw) { style.stroke_width = w; }
    }
    if let Some(op) = attr_value(tag, "opacity") {
        if let Ok(o) = op.parse() { style.opacity = o; }
    }

    style
}

fn parse_color(s: &str) -> Option<PdfColor> {
    let s = s.trim();
    if s.starts_with('#') {
        let hex = &s[1..];
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
            return Some(PdfColor { r, g, b });
        } else if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? as f32 / 15.0;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? as f32 / 15.0;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? as f32 / 15.0;
            return Some(PdfColor { r, g, b });
        }
    } else if s.starts_with("rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len()-1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let parse_comp = |s: &str| -> Option<f32> {
                let s = s.trim();
                if s.ends_with('%') {
                    s[..s.len()-1].parse::<f32>().ok().map(|v| v / 100.0)
                } else {
                    s.parse::<f32>().ok().map(|v| v / 255.0)
                }
            };
            let r = parse_comp(parts[0])?;
            let g = parse_comp(parts[1])?;
            let b = parse_comp(parts[2])?;
            return Some(PdfColor { r, g, b });
        }
    } else {
        // Named colors (common ones)
        return match s {
            "black" => Some(PdfColor { r: 0.0, g: 0.0, b: 0.0 }),
            "white" => Some(PdfColor { r: 1.0, g: 1.0, b: 1.0 }),
            "red" => Some(PdfColor { r: 1.0, g: 0.0, b: 0.0 }),
            "green" => Some(PdfColor { r: 0.0, g: 0.502, b: 0.0 }),
            "blue" => Some(PdfColor { r: 0.0, g: 0.0, b: 1.0 }),
            "yellow" => Some(PdfColor { r: 1.0, g: 1.0, b: 0.0 }),
            "cyan" => Some(PdfColor { r: 0.0, g: 1.0, b: 1.0 }),
            "magenta" => Some(PdfColor { r: 1.0, g: 0.0, b: 1.0 }),
            "gray" | "grey" => Some(PdfColor { r: 0.502, g: 0.502, b: 0.502 }),
            "darkgray" | "darkgrey" => Some(PdfColor { r: 0.663, g: 0.663, b: 0.663 }),
            "lightgray" | "lightgrey" => Some(PdfColor { r: 0.827, g: 0.827, b: 0.827 }),
            "orange" => Some(PdfColor { r: 1.0, g: 0.647, b: 0.0 }),
            "purple" => Some(PdfColor { r: 0.502, g: 0.0, b: 0.502 }),
            "brown" => Some(PdfColor { r: 0.647, g: 0.165, b: 0.165 }),
            "navy" => Some(PdfColor { r: 0.0, g: 0.0, b: 0.502 }),
            "teal" => Some(PdfColor { r: 0.0, g: 0.502, b: 0.502 }),
            "olive" => Some(PdfColor { r: 0.502, g: 0.502, b: 0.0 }),
            "maroon" => Some(PdfColor { r: 0.502, g: 0.0, b: 0.0 }),
            "silver" => Some(PdfColor { r: 0.753, g: 0.753, b: 0.753 }),
            "lime" => Some(PdfColor { r: 0.0, g: 1.0, b: 0.0 }),
            "aqua" => Some(PdfColor { r: 0.0, g: 1.0, b: 1.0 }),
            "fuchsia" => Some(PdfColor { r: 1.0, g: 0.0, b: 1.0 }),
            "none" | "transparent" => None,
            _ => None,
        };
    }
    None
}

fn parse_transform(s: &str) -> Option<Transform> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("translate(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = inner.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        let tx = nums.first().copied().unwrap_or(0.0);
        let ty = nums.get(1).copied().unwrap_or(0.0);
        return Some(Transform::Translate(tx, ty));
    }
    if let Some(inner) = s.strip_prefix("scale(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = inner.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        let sx = nums.first().copied().unwrap_or(1.0);
        let sy = nums.get(1).copied().unwrap_or(sx);
        return Some(Transform::Scale(sx, sy));
    }
    if let Some(inner) = s.strip_prefix("rotate(").and_then(|s| s.strip_suffix(')')) {
        let angle: f32 = inner.split(|c: char| c == ',' || c.is_whitespace())
            .next()?.parse().ok()?;
        return Some(Transform::Rotate(angle));
    }
    if let Some(inner) = s.strip_prefix("matrix(").and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = inner.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() >= 6 {
            return Some(Transform::Matrix(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]));
        }
    }
    None
}

fn attr_f32(tag: &str, name: &str) -> Option<f32> {
    attr_value(tag, name).and_then(|v| parse_length(v))
}

fn style_attr_f32(tag: &str, prop: &str) -> Option<f32> {
    let style_str = attr_value(tag, "style")?;
    for decl in style_str.split(';') {
        if let Some((p, v)) = decl.split_once(':') {
            if p.trim() == prop {
                return parse_length(v.trim());
            }
        }
    }
    None
}

/// Write f32 with 2 decimal places
fn write_f(buf: &mut Vec<u8>, val: f32) {
    use std::io::Write;
    write!(buf, "{:.2}", val).unwrap();
}

/// Check if file data looks like SVG
pub fn is_svg_data(data: &[u8]) -> bool {
    // Check first 256 bytes for SVG markers
    let check_len = data.len().min(512);
    let prefix = &data[..check_len];

    // Skip BOM if present
    let start = if prefix.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    let text = match std::str::from_utf8(&prefix[start..]) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let trimmed = text.trim_start();
    trimmed.starts_with("<?xml") || trimmed.starts_with("<svg") || trimmed.starts_with("<!DOCTYPE svg")
}

/// Extract SVG viewBox/width/height to get native dimensions (in points)
pub fn svg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(data).ok()?;
    let doc = parse_svg(text)?;
    // Return dimensions as integer points
    Some((doc.width.max(1.0) as u32, doc.height.max(1.0) as u32))
}
