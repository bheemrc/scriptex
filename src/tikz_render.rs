/// Native Rust TikZ renderer: parses TikZ source and produces positioned
/// drawing primitives (rectangles, lines, text labels) without shelling out to pdflatex.
///
/// Supports:
/// - Style definitions: phase/.style={rectangle, draw, fill=blue!20, ...}
/// - Relative positioning: right=of mcr, below=2cm of mcr
/// - \node[options] (name) at (x,y) {text};
/// - \draw[->] (from) -- (to) / |- (to) / -| (to);
/// - \coordinate (name) at (x,y);
/// - xcolor names with !intensity mixing
/// - minimum width/height, text width, inner sep
/// - Multi-line node text via \\
/// - xshift, yshift
/// - fit nodes: fit=(a)(b)(c)
/// - node distance (global and per-axis)

use crate::color::Color;
use crate::font::{self, FontId};
use std::collections::HashMap;

/// A rendered TikZ element ready for layout
#[derive(Debug)]
pub enum TikzElement {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
        corner_radius: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: Color,
    },
    Arrow {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: Color,
        bidirectional: bool,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        font_size: f32,
        bold: bool,
        color: Color,
    },
}

/// Result of rendering a TikZ picture
pub struct TikzRenderResult {
    pub elements: Vec<TikzElement>,
    pub width: f32,
    pub height: f32,
}

/// A parsed style definition from tikzpicture options
#[derive(Debug, Clone, Default)]
struct TikzStyle {
    shape: Option<Shape>,
    fill_color: Option<Color>,
    draw_color: Option<Color>,
    text_color: Option<Color>,
    minimum_width: Option<f32>,
    minimum_height: Option<f32>,
    inner_sep: Option<f32>,
    font_size: Option<f32>,
    text_width: Option<f32>,
    rounded_corners: bool,
    dashed: bool,
    // Edge style properties
    arrow: bool,
    thick: bool,
    bold_font: bool,
}

/// Relative positioning anchor
#[derive(Debug, Clone)]
enum RelativePos {
    RightOf(String, f32),  // (anchor_name, explicit_distance or 0 for default)
    LeftOf(String, f32),
    AboveOf(String, f32),
    BelowOf(String, f32),
}

/// Internal parsed node
struct TikzNode {
    name: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    text: Vec<String>,
    shape: Shape,
    fill_color: Option<Color>,
    draw_color: Option<Color>,
    text_color: Color,
    minimum_width: f32,
    minimum_height: f32,
    inner_sep: f32,
    font_size: f32,
    bold: bool,
    relative_to: Option<RelativePos>,
    xshift: f32,
    yshift: f32,
    fit_nodes: Vec<String>,  // for fit= option
    label: Option<(String, String)>, // (position, text) for label= option
    positioned: bool,  // whether coordinates have been resolved
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Rectangle,
    Circle,
    Ellipse,
    Diamond,
    RoundedRect,
    Star,
    Trapezium,
    Cloud,
    None,
}

/// Path segment type between two points
#[derive(Debug, Clone, Copy, PartialEq)]
enum PathType {
    Straight,    // --
    HorizVert,   // -|
    VertHoriz,   // |-
}

struct TikzEdge {
    from: String,
    to: String,
    style: EdgeStyle,
    color: Color,
    line_width: f32,
    label: Option<String>,
    path_type: PathType,
    from_anchor: Option<String>,
    to_anchor: Option<String>,
    label_pos: f32, // 0.0..1.0, default 0.5
}

/// Dash pattern for TikZ lines
#[derive(Debug, Clone, Copy, PartialEq)]
enum DashPattern {
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DashDotDot,
}

/// Arrow tip style
#[derive(Debug, Clone, Copy, PartialEq)]
enum ArrowTip {
    Stealth,
    Latex,
    Triangle,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgeStyle {
    Line,
    Arrow,
    ReverseArrow,
    BiArrow,
    Dashed,
}

/// Global diagram settings
struct DiagramSettings {
    node_distance_x: f32,  // horizontal distance between nodes
    node_distance_y: f32,  // vertical distance between nodes
    styles: HashMap<String, TikzStyle>,
}

/// Parse and render a TikZ picture source
pub fn render_tikz(source: &str) -> TikzRenderResult {
    let mut nodes: Vec<TikzNode> = Vec::new();
    let mut edges: Vec<TikzEdge> = Vec::new();

    // Parse environment-level options (before first \node/\draw)
    let mut settings = parse_environment_options(source);

    // Skip past environment options block [...]
    // The source may start with "[" for tikzpicture options
    let body_source = skip_env_options(source);

    // Collect multi-line statements (end with ;)
    let mut statements: Vec<String> = Vec::new();
    let mut current_stmt = String::new();

    for line in body_source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('%') {
            continue;
        }
        current_stmt.push(' ');
        current_stmt.push_str(trimmed);
        if trimmed.ends_with(';') {
            statements.push(std::mem::take(&mut current_stmt));
        }
    }
    if !current_stmt.trim().is_empty() {
        statements.push(current_stmt);
    }

    for stmt in &statements {
        let s = stmt.trim();
        if s.starts_with("\\node") || s.starts_with("\\coordinate") {
            if let Some(node) = parse_node(s, &settings) {
                nodes.push(node);
            }
        } else if s.starts_with("\\draw") || s.starts_with("\\path") || s.starts_with("\\filldraw") {
            parse_draw_command(s, &mut edges, &settings);
        }
    }

    if nodes.is_empty() && edges.is_empty() {
        return TikzRenderResult {
            elements: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    }

    // Auto-size nodes based on text content
    for node in &mut nodes {
        auto_size_node(node);
    }

    // Resolve relative positioning (multi-pass for chains)
    resolve_positions(&mut nodes, &mut settings);

    // Handle fit nodes (compute bounding box around referenced nodes)
    resolve_fit_nodes(&mut nodes);

    // Compute bounding box
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    for node in &nodes {
        let hw = node.width / 2.0;
        let hh = node.height / 2.0;
        min_x = min_x.min(node.x - hw);
        max_x = max_x.max(node.x + hw);
        min_y = min_y.min(node.y - hh);
        max_y = max_y.max(node.y + hh);
        // Account for labels extending beyond node bounds
        if let Some((pos, text)) = &node.label {
            let label_fs = 8.0;
            let tw = font::measure_text(text, FontId::Helvetica, label_fs);
            match pos.as_str() {
                "left" => { min_x = min_x.min(node.x - hw - tw - 6.0); }
                "right" => { max_x = max_x.max(node.x + hw + tw + 6.0); }
                "above" => { min_y = min_y.min(node.y - hh - label_fs - 4.0); }
                "below" => { max_y = max_y.max(node.y + hh + label_fs + 4.0); }
                _ => {}
            }
        }
    }

    let padding = 15.0;
    min_x -= padding;
    min_y -= padding;
    max_x += padding;
    max_y += padding;

    let total_w = (max_x - min_x).max(50.0);
    let total_h = (max_y - min_y).max(30.0);

    let mut elements: Vec<TikzElement> = Vec::new();

    // Draw fit nodes first (background grouping boxes)
    for node in &nodes {
        if !node.fit_nodes.is_empty() {
            render_node(node, min_x, min_y, &mut elements);
        }
    }

    // Draw edges (behind regular nodes)
    for edge in &edges {
        let src = nodes.iter().find(|n| n.name == edge.from);
        let dst = nodes.iter().find(|n| n.name == edge.to);
        if let (Some(s), Some(d)) = (src, dst) {
            render_edge(edge, s, d, min_x, min_y, &mut elements);
        }
    }

    // Draw regular nodes (on top)
    for node in &nodes {
        if node.fit_nodes.is_empty() {
            render_node(node, min_x, min_y, &mut elements);
        }
    }

    TikzRenderResult {
        elements,
        width: total_w,
        height: total_h,
    }
}

/// Parse the tikzpicture environment options [...] for style definitions and settings
fn parse_environment_options(source: &str) -> DiagramSettings {
    let mut settings = DiagramSettings {
        node_distance_x: 2.0 * 28.35,  // 2cm default
        node_distance_y: 2.0 * 28.35,
        styles: HashMap::new(),
    };

    // Find the environment-level options block
    // The source may start with the options or they may be on the first few lines
    // Look for style definitions and node distance in the first chunk before \node/\draw
    let env_end = source.find("\\node")
        .or_else(|| source.find("\\draw"))
        .or_else(|| source.find("\\coordinate"))
        .unwrap_or(source.len());
    let env_header = &source[..env_end];

    // Parse node distance
    if let Some(nd_idx) = env_header.find("node distance=") {
        let rest = &env_header[nd_idx + 14..];
        // Could be "2cm" or "1.5cm and 2cm"
        let end = rest.find(',').or_else(|| rest.find(']')).unwrap_or(rest.len());
        let dist_str = rest[..end].trim();
        if let Some(and_idx) = dist_str.find(" and ") {
            // Separate y and x distances
            settings.node_distance_y = parse_dimension(&dist_str[..and_idx]);
            settings.node_distance_x = parse_dimension(&dist_str[and_idx + 5..]);
        } else {
            let d = parse_dimension(dist_str);
            settings.node_distance_x = d;
            settings.node_distance_y = d;
        }
    }

    // Parse style definitions: name/.style={...}
    let mut search_pos = 0;
    while search_pos < env_header.len() {
        if let Some(style_idx) = env_header[search_pos..].find("/.style=") {
            let abs_idx = search_pos + style_idx;
            // Find the style name (go backwards to find start)
            let name_start = env_header[..abs_idx].rfind(|c: char| c == ',' || c == '[' || c == '\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let style_name = env_header[name_start..abs_idx].trim().to_string();

            // Find the style body {...}
            let brace_start = abs_idx + 8; // skip "/.style="
            if let Some(brace_pos) = env_header[brace_start..].find('{') {
                let abs_brace = brace_start + brace_pos;
                if let Some(brace_end) = find_matching_brace(env_header, abs_brace) {
                    let body = &env_header[abs_brace + 1..brace_end];
                    let style = parse_style_body(body);
                    settings.styles.insert(style_name, style);
                    search_pos = brace_end + 1;
                    continue;
                }
            }
            search_pos = abs_idx + 8;
        } else {
            break;
        }
    }

    settings
}

/// Skip the environment options block [...] at the start of TikZ source.
/// Returns the remaining source after the closing ].
fn skip_env_options(source: &str) -> &str {
    let trimmed = source.trim_start();
    if !trimmed.starts_with('[') {
        return source;
    }
    // Find the matching ]
    if let Some(end) = find_matching_bracket(trimmed, 0) {
        let rest = &trimmed[end + 1..];
        rest
    } else {
        source
    }
}

/// Parse the body of a style definition into a TikzStyle
fn parse_style_body(body: &str) -> TikzStyle {
    let mut style = TikzStyle::default();

    for opt in split_options(body) {
        let opt = opt.trim();
        apply_option_to_style(opt, &mut style);
    }

    style
}

/// Apply a single TikZ option string to a style
fn apply_option_to_style(opt: &str, style: &mut TikzStyle) {
    let opt = opt.trim();
    match opt {
        "rectangle" => style.shape = Some(Shape::Rectangle),
        "circle" => style.shape = Some(Shape::Circle),
        "ellipse" => style.shape = Some(Shape::Ellipse),
        "diamond" => style.shape = Some(Shape::Diamond),
        "star" | "star, star points=5" => style.shape = Some(Shape::Star),
        "trapezium" => style.shape = Some(Shape::Trapezium),
        "cloud" => style.shape = Some(Shape::Cloud),
        "rounded corners" => style.rounded_corners = true,
        "draw" => style.draw_color = Some(Color::BLACK),
        "dashed" => style.dashed = true,
        "thick" => style.thick = true,
        "very thick" => style.thick = true,
        "text centered" | "align=center" => {} // default behavior
        _ => {
            if opt.starts_with("rounded corners=") {
                style.rounded_corners = true;
            } else if opt.starts_with("draw=") {
                style.draw_color = Some(parse_color(&opt[5..]));
            } else if opt.starts_with("fill=") {
                style.fill_color = Some(parse_color(&opt[5..]));
            } else if opt.starts_with("text=") {
                style.text_color = Some(parse_color(&opt[5..]));
            } else if opt.starts_with("minimum width=") {
                style.minimum_width = Some(parse_dimension(&opt[14..]));
            } else if opt.starts_with("minimum height=") {
                style.minimum_height = Some(parse_dimension(&opt[15..]));
            } else if opt.starts_with("minimum size=") {
                let s = parse_dimension(&opt[13..]);
                style.minimum_width = Some(s);
                style.minimum_height = Some(s);
            } else if opt.starts_with("inner sep=") {
                style.inner_sep = Some(parse_dimension(&opt[10..]));
            } else if opt.starts_with("text width=") {
                style.text_width = Some(parse_dimension(&opt[11..]));
            } else if opt.starts_with("font=") {
                if opt.contains("\\small") || opt.contains("\\footnotesize") {
                    style.font_size = Some(7.0);
                } else if opt.contains("\\large") || opt.contains("\\Large") {
                    style.font_size = Some(11.0);
                } else if opt.contains("\\tiny") {
                    style.font_size = Some(6.0);
                }
                if opt.contains("\\bfseries") {
                    style.bold_font = true;
                }
            } else if opt.contains("->") || opt.contains(">=") {
                style.arrow = true;
            }
        }
    }
}

/// Apply a style's properties to a node, only overriding defaults
fn apply_style_to_node(style: &TikzStyle, node: &mut TikzNode) {
    if let Some(shape) = style.shape {
        if style.rounded_corners {
            node.shape = Shape::RoundedRect;
        } else {
            node.shape = shape;
        }
    } else if style.rounded_corners {
        node.shape = Shape::RoundedRect;
    }
    if let Some(c) = style.fill_color { node.fill_color = Some(c); }
    if let Some(c) = style.draw_color { node.draw_color = Some(c); }
    if let Some(c) = style.text_color { node.text_color = c; }
    if let Some(w) = style.minimum_width { node.minimum_width = w; }
    if let Some(h) = style.minimum_height { node.minimum_height = h; }
    if let Some(s) = style.inner_sep { node.inner_sep = s; }
    if let Some(fs) = style.font_size { node.font_size = fs; }
    if let Some(tw) = style.text_width { node.minimum_width = tw; }
    if style.bold_font { node.bold = true; }
    if style.dashed { node.draw_color = node.draw_color.or(Some(Color::DARK_GRAY)); }
}

fn parse_node(stmt: &str, settings: &DiagramSettings) -> Option<TikzNode> {
    let is_coord = stmt.starts_with("\\coordinate");

    let mut node = TikzNode {
        name: String::new(),
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        text: Vec::new(),
        shape: if is_coord { Shape::None } else { Shape::Rectangle },
        fill_color: None,
        draw_color: None,
        text_color: Color::BLACK,
        minimum_width: 0.0,
        minimum_height: 0.0,
        inner_sep: 6.0,
        font_size: 9.0,
        bold: false,
        relative_to: None,
        xshift: 0.0,
        yshift: 0.0,
        fit_nodes: Vec::new(),
        label: None,
        positioned: false,
    };

    let mut has_at_coord = false;

    // Parse options [...]
    if let Some(opt_start) = stmt.find('[') {
        if let Some(opt_end) = find_matching_bracket(stmt, opt_start) {
            let opts_str = &stmt[opt_start + 1..opt_end];
            let options = split_options(opts_str);

            for opt in &options {
                let opt = opt.trim();

                // First check if it's a style reference
                if let Some(style) = settings.styles.get(opt) {
                    apply_style_to_node(style, &mut node);
                    continue;
                }

                // Parse relative positioning: right=of name, below=2cm of name
                if let Some(rel) = parse_relative_position(opt) {
                    node.relative_to = Some(rel);
                    continue;
                }

                // xshift/yshift
                if opt.starts_with("xshift=") {
                    node.xshift = parse_dimension(&opt[7..]);
                    continue;
                }
                if opt.starts_with("yshift=") {
                    node.yshift = -parse_dimension(&opt[7..]); // negate Y
                    continue;
                }

                // fit=(a)(b)(c)
                if opt.starts_with("fit=") {
                    let fit_str = &opt[4..];
                    let mut rest = fit_str;
                    while let Some(p) = rest.find('(') {
                        if let Some(pe) = rest[p + 1..].find(')') {
                            node.fit_nodes.push(rest[p + 1..p + 1 + pe].trim().to_string());
                            rest = &rest[p + 1 + pe + 1..];
                        } else {
                            break;
                        }
                    }
                    continue;
                }

                // label=above:Text or label={above:Text}
                if opt.starts_with("label=") {
                    let label_val = opt[6..].trim_start_matches('{').trim_end_matches('}');
                    if let Some(colon) = label_val.find(':') {
                        let pos = label_val[..colon].trim().to_string();
                        let text = label_val[colon + 1..].trim().to_string();
                        node.label = Some((pos, text));
                    } else {
                        node.label = Some(("above".to_string(), label_val.to_string()));
                    }
                    continue;
                }

                // Standard node options
                if opt == "circle" { node.shape = Shape::Circle; }
                else if opt == "diamond" { node.shape = Shape::Diamond; }
                else if opt == "ellipse" { node.shape = Shape::Ellipse; }
                else if opt == "rectangle" { node.shape = Shape::Rectangle; }
                else if opt == "star" { node.shape = Shape::Star; }
                else if opt == "trapezium" { node.shape = Shape::Trapezium; }
                else if opt == "cloud" { node.shape = Shape::Cloud; }
                else if opt.starts_with("rounded corners") || opt.starts_with("rounded rect") {
                    node.shape = Shape::RoundedRect;
                }
                else if opt == "coordinate" { node.shape = Shape::None; }
                else if opt == "draw" { node.draw_color = Some(Color::BLACK); }
                else if opt.starts_with("draw=") { node.draw_color = Some(parse_color(&opt[5..])); }
                else if opt.starts_with("fill=") { node.fill_color = Some(parse_color(&opt[5..])); }
                else if opt.starts_with("text=") { node.text_color = parse_color(&opt[5..]); }
                else if opt.starts_with("minimum width=") {
                    node.minimum_width = parse_dimension(&opt[14..]);
                }
                else if opt.starts_with("minimum height=") {
                    node.minimum_height = parse_dimension(&opt[15..]);
                }
                else if opt.starts_with("minimum size=") {
                    let s = parse_dimension(&opt[13..]);
                    node.minimum_width = s;
                    node.minimum_height = s;
                }
                else if opt.starts_with("inner sep=") {
                    node.inner_sep = parse_dimension(&opt[10..]);
                }
                else if opt.starts_with("font=") {
                    if opt.contains("\\small") || opt.contains("\\footnotesize") { node.font_size = 7.0; }
                    else if opt.contains("\\large") || opt.contains("\\Large") { node.font_size = 11.0; }
                    else if opt.contains("\\tiny") { node.font_size = 6.0; }
                    if opt.contains("\\bfseries") { node.bold = true; }
                }
                else if opt.starts_with("text width=") {
                    node.minimum_width = parse_dimension(&opt[11..]);
                }
                else if opt == "dashed" {
                    node.draw_color = node.draw_color.or(Some(Color::DARK_GRAY));
                }
                else if opt == "text centered" || opt == "align=center"
                    || opt == "thick" || opt == "very thick" {
                    // Already handled or cosmetic
                }
                else if !opt.contains('=') && !opt.contains(' ') && opt.len() < 20 {
                    // Try as color name
                    let test = parse_color(opt);
                    if test != Color::rgb(0.3, 0.3, 0.6) {
                        node.fill_color = Some(test);
                    }
                }
            }
        }
    }

    // Parse name (name)
    let after_opts = if let Some(bracket_end) = stmt.find(']') {
        &stmt[bracket_end + 1..]
    } else {
        let skip = if is_coord { 11 } else { 5 };
        &stmt[skip..]
    };

    // Find (name) — first parenthesized group that's not a coordinate
    let mut search = after_opts;
    while let Some(p_start) = search.find('(') {
        if let Some(p_end) = search[p_start + 1..].find(')') {
            let inner = search[p_start + 1..p_start + 1 + p_end].trim();
            if !inner.contains(',') && !inner.is_empty() {
                if node.name.is_empty() {
                    node.name = inner.to_string();
                }
            }
            search = &search[p_start + 1 + p_end + 1..];
        } else {
            break;
        }
    }

    // Parse "at (x,y)"
    if let Some(at_idx) = stmt.find(" at ") {
        let rest = &stmt[at_idx + 4..];
        if let Some(p_start) = rest.find('(') {
            if let Some(p_end) = rest[p_start + 1..].find(')') {
                let coords = &rest[p_start + 1..p_start + 1 + p_end];
                let parts: Vec<&str> = coords.split(',').collect();
                if parts.len() >= 2 {
                    node.x = parse_tikz_coord_value(parts[0].trim());
                    node.y = -parse_tikz_coord_value(parts[1].trim()); // Negate Y
                    has_at_coord = true;
                }
            }
        }
    }

    // Parse text {text with \\ line breaks}
    if !is_coord {
        if let Some(brace_start) = stmt.rfind('{') {
            let brace_depth_start = stmt[..brace_start].matches('{').count();
            let brace_depth_end = stmt[..brace_start].matches('}').count();
            if brace_depth_start <= brace_depth_end + 1 {
                if let Some(brace_end) = find_matching_brace(stmt, brace_start) {
                    let raw_text = &stmt[brace_start + 1..brace_end];
                    if !raw_text.is_empty() {
                        let cleaned = strip_latex_commands(raw_text);
                        node.text = cleaned.split("\\\\")
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                }
            }
        }
    }

    if node.name.is_empty() && node.text.is_empty() && node.fit_nodes.is_empty() {
        return None;
    }
    if node.name.is_empty() {
        node.name = node.text.join(" ").chars().take(20).collect();
    }

    // Mark as positioned if it has explicit coordinates or no relative ref
    node.positioned = has_at_coord;

    Some(node)
}

/// Parse relative positioning from an option string
/// Handles: right=of foo, below=2cm of foo, right=1.5cm of foo
fn parse_relative_position(opt: &str) -> Option<RelativePos> {
    // Match "direction=..." where direction is right/left/above/below
    let (direction, rest) = if let Some(r) = opt.strip_prefix("right=") {
        ("right", r)
    } else if let Some(r) = opt.strip_prefix("left=") {
        ("left", r)
    } else if let Some(r) = opt.strip_prefix("above=") {
        ("above", r)
    } else if let Some(r) = opt.strip_prefix("below=") {
        ("below", r)
    } else if opt.starts_with("above right=") {
        ("right", &opt[12..])
    } else if opt.starts_with("below right=") {
        ("right", &opt[12..])  // treat as right with below offset
    } else if opt.starts_with("above left=") {
        ("left", &opt[11..])
    } else if opt.starts_with("below left=") {
        ("left", &opt[11..])
    } else {
        return None;
    };

    // Parse "of name" or "2cm of name"
    if let Some(of_idx) = rest.find("of ") {
        let before_of = rest[..of_idx].trim();
        let name = rest[of_idx + 3..].trim().to_string();

        // Parse optional explicit distance
        let explicit_dist = if before_of.is_empty() {
            0.0 // use default node distance
        } else {
            parse_dimension(before_of)
        };

        return match direction {
            "right" => Some(RelativePos::RightOf(name, explicit_dist)),
            "left" => Some(RelativePos::LeftOf(name, explicit_dist)),
            "above" => Some(RelativePos::AboveOf(name, explicit_dist)),
            "below" => Some(RelativePos::BelowOf(name, explicit_dist)),
            _ => None,
        };
    }

    None
}

/// Resolve relative positions in multiple passes (for chains like A -> B -> C)
fn resolve_positions(nodes: &mut Vec<TikzNode>, settings: &DiagramSettings) {
    // First node without explicit position or relative ref gets (0,0)
    let has_any_positioned = nodes.iter().any(|n| n.positioned);
    if !has_any_positioned && !nodes.is_empty() {
        nodes[0].positioned = true; // anchor first node at (0,0)
    }

    // Multiple passes to resolve chains
    let max_passes = nodes.len() + 1;
    for _ in 0..max_passes {
        let mut all_resolved = true;

        for i in 0..nodes.len() {
            if nodes[i].positioned {
                continue;
            }

            let rel = match &nodes[i].relative_to {
                Some(r) => r.clone(),
                None => {
                    // No relative ref and no explicit coord — position at origin
                    nodes[i].positioned = true;
                    continue;
                }
            };

            // Find anchor node
            let anchor_name = match &rel {
                RelativePos::RightOf(n, _) | RelativePos::LeftOf(n, _)
                | RelativePos::AboveOf(n, _) | RelativePos::BelowOf(n, _) => n.clone(),
            };

            // Look up anchor position (must already be positioned)
            let anchor = nodes.iter()
                .find(|n| n.name == anchor_name && n.positioned)
                .map(|n| (n.x, n.y, n.width, n.height));

            if let Some((ax, ay, aw, ah)) = anchor {
                let cur_w = nodes[i].width;
                let cur_h = nodes[i].height;

                match &rel {
                    RelativePos::RightOf(_, dist) => {
                        let d = if *dist > 0.0 { *dist } else { settings.node_distance_x };
                        // Center-to-center distance = half widths + gap
                        nodes[i].x = ax + aw / 2.0 + d + cur_w / 2.0;
                        nodes[i].y = ay;
                    }
                    RelativePos::LeftOf(_, dist) => {
                        let d = if *dist > 0.0 { *dist } else { settings.node_distance_x };
                        nodes[i].x = ax - aw / 2.0 - d - cur_w / 2.0;
                        nodes[i].y = ay;
                    }
                    RelativePos::BelowOf(_, dist) => {
                        let d = if *dist > 0.0 { *dist } else { settings.node_distance_y };
                        nodes[i].x = ax;
                        nodes[i].y = ay + ah / 2.0 + d + cur_h / 2.0;
                    }
                    RelativePos::AboveOf(_, dist) => {
                        let d = if *dist > 0.0 { *dist } else { settings.node_distance_y };
                        nodes[i].x = ax;
                        nodes[i].y = ay - ah / 2.0 - d - cur_h / 2.0;
                    }
                }

                // Apply xshift/yshift
                nodes[i].x += nodes[i].xshift;
                nodes[i].y += nodes[i].yshift;
                nodes[i].positioned = true;
            } else {
                all_resolved = false;
            }
        }

        if all_resolved {
            break;
        }
    }
}

/// Resolve fit nodes — compute bounding box around referenced nodes
fn resolve_fit_nodes(nodes: &mut Vec<TikzNode>) {
    let fit_indices: Vec<usize> = nodes.iter().enumerate()
        .filter(|(_, n)| !n.fit_nodes.is_empty())
        .map(|(i, _)| i)
        .collect();

    for idx in fit_indices {
        let fit_names = nodes[idx].fit_nodes.clone();
        let inner_sep = nodes[idx].inner_sep;

        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for name in &fit_names {
            if let Some(ref_node) = nodes.iter().find(|n| &n.name == name) {
                let hw = ref_node.width / 2.0;
                let hh = ref_node.height / 2.0;
                min_x = min_x.min(ref_node.x - hw);
                max_x = max_x.max(ref_node.x + hw);
                min_y = min_y.min(ref_node.y - hh);
                max_y = max_y.max(ref_node.y + hh);
            }
        }

        if min_x < f32::MAX {
            let pad = inner_sep + 4.0;
            nodes[idx].x = (min_x + max_x) / 2.0;
            nodes[idx].y = (min_y + max_y) / 2.0;
            nodes[idx].width = (max_x - min_x) + pad * 2.0;
            nodes[idx].height = (max_y - min_y) + pad * 2.0;
            nodes[idx].positioned = true;
            // Fit nodes typically only have draw, no fill
            if nodes[idx].fill_color.is_none() && nodes[idx].draw_color.is_none() {
                nodes[idx].draw_color = Some(Color::DARK_GRAY);
            }
        }
    }
}

/// Render a single edge into elements
fn render_edge(edge: &TikzEdge, s: &TikzNode, d: &TikzNode, min_x: f32, min_y: f32, elements: &mut Vec<TikzElement>) {
    match edge.path_type {
        PathType::Straight => {
            let (sx, sy) = edge_point(s, d.x, d.y);
            let (dx, dy) = edge_point(d, s.x, s.y);
            let px1 = sx - min_x;
            let py1 = sy - min_y;
            let px2 = dx - min_x;
            let py2 = dy - min_y;
            emit_edge_element(edge, px1, py1, px2, py2, elements);
        }
        PathType::VertHoriz => {
            // |-- path: go vertical to target Y, then horizontal to target X
            let mid_y = d.y;
            // Segment 1: from source vertically to mid_y
            let (sx, sy) = edge_point(s, s.x, mid_y);
            let px1 = sx - min_x;
            let py1 = sy - min_y;
            let px_mid = sx - min_x;
            let py_mid = mid_y - min_y;
            // Segment 2: horizontally to destination
            let (dx, dy) = edge_point(d, sx, d.y);
            let px2 = dx - min_x;
            let py2 = dy - min_y;

            // Draw segment 1 as line
            elements.push(TikzElement::Line {
                x1: px1, y1: py1, x2: px_mid, y2: py_mid,
                width: edge.line_width, color: edge.color,
            });
            // Draw segment 2 with arrow
            emit_edge_element(edge, px_mid, py_mid, px2, py2, elements);
        }
        PathType::HorizVert => {
            // -| path: go horizontal to target X, then vertical to target Y
            let mid_x = d.x;
            let (sx, sy) = edge_point(s, mid_x, s.y);
            let px1 = sx - min_x;
            let py1 = sy - min_y;
            let px_mid = mid_x - min_x;
            let py_mid = sy - min_y;
            let (dx, dy) = edge_point(d, d.x, sy);
            let px2 = dx - min_x;
            let py2 = dy - min_y;

            elements.push(TikzElement::Line {
                x1: px1, y1: py1, x2: px_mid, y2: py_mid,
                width: edge.line_width, color: edge.color,
            });
            emit_edge_element(edge, px_mid, py_mid, px2, py2, elements);
        }
    }

    // Edge label
    if let Some(label) = &edge.label {
        let sx = s.x - min_x;
        let sy = s.y - min_y;
        let dx = d.x - min_x;
        let dy = d.y - min_y;
        let mx = (sx + dx) / 2.0;
        let my = (sy + dy) / 2.0 - 8.0;
        elements.push(TikzElement::Text {
            x: mx, y: my,
            text: label.clone(),
            font_size: 8.0,
            bold: false,
            color: Color::DARK_GRAY,
        });
    }
}

fn emit_edge_element(edge: &TikzEdge, x1: f32, y1: f32, x2: f32, y2: f32, elements: &mut Vec<TikzElement>) {
    match edge.style {
        EdgeStyle::Arrow | EdgeStyle::BiArrow => {
            elements.push(TikzElement::Arrow {
                x1, y1, x2, y2,
                width: edge.line_width,
                color: edge.color,
                bidirectional: edge.style == EdgeStyle::BiArrow,
            });
        }
        EdgeStyle::ReverseArrow => {
            elements.push(TikzElement::Arrow {
                x1: x2, y1: y2, x2: x1, y2: y1,
                width: edge.line_width,
                color: edge.color,
                bidirectional: false,
            });
        }
        EdgeStyle::Line | EdgeStyle::Dashed => {
            elements.push(TikzElement::Line {
                x1, y1, x2, y2,
                width: if edge.style == EdgeStyle::Dashed { edge.line_width * 0.7 } else { edge.line_width },
                color: edge.color,
            });
        }
    }
}

/// Render a node into elements
fn render_node(node: &TikzNode, min_x: f32, min_y: f32, elements: &mut Vec<TikzElement>) {
    let px = node.x - min_x;
    let py = node.y - min_y;

    if node.shape != Shape::None {
        let fill = node.fill_color.or_else(|| {
            if node.fit_nodes.is_empty() {
                // Only default fill for non-fit nodes
                match node.shape {
                    Shape::Rectangle | Shape::RoundedRect | Shape::Trapezium => Some(Color::rgb(0.92, 0.95, 1.0)),
                    Shape::Circle | Shape::Star => Some(Color::rgb(1.0, 0.92, 0.92)),
                    Shape::Diamond => Some(Color::rgb(0.95, 0.95, 0.88)),
                    Shape::Ellipse | Shape::Cloud => Some(Color::rgb(0.92, 1.0, 0.92)),
                    Shape::None => None,
                }
            } else {
                None  // Fit nodes have no fill by default
            }
        });
        let stroke = node.draw_color.or(Some(Color::rgb(0.3, 0.3, 0.5)));

        elements.push(TikzElement::Rect {
            x: px - node.width / 2.0,
            y: py - node.height / 2.0,
            width: node.width,
            height: node.height,
            fill,
            stroke,
            stroke_width: 0.8,
            corner_radius: match node.shape {
                    Shape::Circle | Shape::Ellipse => node.width.min(node.height) / 2.0,
                    Shape::RoundedRect | Shape::Cloud => 4.0,
                    Shape::Star => 2.0,
                    _ => 0.0,
                },
        });
    }

    // Node text (multi-line centered)
    let total_lines = node.text.len();
    if total_lines > 0 {
        let line_h = node.font_size * 1.3;
        let start_y = py - (total_lines as f32 * line_h) / 2.0 + node.font_size * 0.4;
        for (i, line) in node.text.iter().enumerate() {
            if line.is_empty() { continue; }
            let tw = font::measure_text(line, FontId::Helvetica, node.font_size);
            elements.push(TikzElement::Text {
                x: px - tw / 2.0,
                y: start_y + i as f32 * line_h,
                text: line.clone(),
                font_size: node.font_size,
                bold: node.bold,
                color: node.text_color,
            });
        }
    }

    // External label
    if let Some((pos, label_text)) = &node.label {
        let label_fs = 8.0;
        let tw = font::measure_text(label_text, FontId::Helvetica, label_fs);
        let (lx, ly) = match pos.as_str() {
            "above" => (px - tw / 2.0, py - node.height / 2.0 - label_fs - 2.0),
            "below" => (px - tw / 2.0, py + node.height / 2.0 + label_fs + 2.0),
            "left" => (px - node.width / 2.0 - tw - 6.0, py + label_fs * 0.3),
            "right" => (px + node.width / 2.0 + 6.0, py + label_fs * 0.3),
            _ => (px - tw / 2.0, py - node.height / 2.0 - label_fs - 2.0),
        };
        elements.push(TikzElement::Text {
            x: lx, y: ly,
            text: label_text.clone(),
            font_size: label_fs,
            bold: false,
            color: Color::DARK_GRAY,
        });
    }
}

/// Compute the point on a node's border closest to target (tx, ty)
fn edge_point(node: &TikzNode, tx: f32, ty: f32) -> (f32, f32) {
    if node.shape == Shape::None {
        return (node.x, node.y);
    }

    let dx = tx - node.x;
    let dy = ty - node.y;
    let angle = dy.atan2(dx);
    let hw = node.width / 2.0;
    let hh = node.height / 2.0;

    match node.shape {
        Shape::Circle | Shape::Ellipse => {
            let r = hw.max(hh);
            (node.x + r * angle.cos(), node.y + r * angle.sin())
        }
        _ => {
            let cos = angle.cos();
            let sin = angle.sin();
            if cos.abs() < 0.001 && sin.abs() < 0.001 {
                return (node.x, node.y);
            }
            let t = if cos.abs() * hh > sin.abs() * hw {
                hw / cos.abs()
            } else {
                hh / sin.abs()
            };
            (node.x + t * cos, node.y + t * sin)
        }
    }
}

fn auto_size_node(node: &mut TikzNode) {
    let font_size = node.font_size;
    let padding = node.inner_sep;

    if node.text.is_empty() && node.fit_nodes.is_empty() {
        node.width = node.minimum_width.max(10.0);
        node.height = node.minimum_height.max(10.0);
        return;
    }

    if !node.fit_nodes.is_empty() {
        // Fit nodes get sized later in resolve_fit_nodes
        return;
    }

    let mut max_tw = 0.0f32;
    for line in &node.text {
        let tw = font::measure_text(line, FontId::Helvetica, font_size);
        max_tw = max_tw.max(tw);
    }
    let text_h = node.text.len() as f32 * font_size * 1.3;

    node.width = (max_tw + padding * 2.0).max(node.minimum_width);
    node.height = (text_h + padding * 2.0).max(node.minimum_height);

    if node.shape == Shape::Circle {
        let size = node.width.max(node.height);
        node.width = size;
        node.height = size;
    }
}

fn parse_draw_command(stmt: &str, edges: &mut Vec<TikzEdge>, settings: &DiagramSettings) {
    let mut style = EdgeStyle::Line;
    let mut color = Color::DARK_GRAY;
    let mut line_width = 0.8f32;
    let mut label: Option<String> = None;

    if let Some(opt_start) = stmt.find('[') {
        if let Some(opt_end) = find_matching_bracket(stmt, opt_start) {
            let opts_str = &stmt[opt_start + 1..opt_end];

            // Check for named style references first
            for opt in split_options(opts_str) {
                let opt = opt.trim();
                if let Some(st) = settings.styles.get(opt) {
                    if st.arrow { style = EdgeStyle::Arrow; }
                    if st.thick { line_width = 1.2; }
                    if let Some(c) = st.draw_color { color = c; }
                    continue;
                }
            }

            // Then direct options
            if opts_str.contains("<->") { style = EdgeStyle::BiArrow; }
            else if opts_str.contains("->") { style = EdgeStyle::Arrow; }
            else if opts_str.contains("<-") { style = EdgeStyle::ReverseArrow; }
            if opts_str.contains("dashed") { style = EdgeStyle::Dashed; }
            if opts_str.contains("very thick") { line_width = 1.6; }
            else if opts_str.contains("thick") && line_width < 1.2 { line_width = 1.2; }

            for opt in split_options(opts_str) {
                let opt = opt.trim();
                if opt.starts_with("color=") { color = parse_color(&opt[6..]); }
                else if !opt.contains('=') && !opt.contains('-') && !opt.contains("thick")
                    && !opt.contains("dashed") && opt.len() < 20
                    && !settings.styles.contains_key(opt) {
                    let c = parse_color(opt);
                    if c != Color::rgb(0.3, 0.3, 0.6) { color = c; }
                }
            }
        }
    }

    let stmt_after_opts = if let Some(bracket_end) = stmt.find(']') {
        &stmt[bracket_end + 1..]
    } else {
        let skip = if stmt.starts_with("\\draw") { 5 }
            else if stmt.starts_with("\\path") { 5 }
            else if stmt.starts_with("\\filldraw") { 9 }
            else { 0 };
        &stmt[skip..]
    };

    // Extract label from "node {text}" in the path
    if let Some(node_idx) = stmt_after_opts.find("node") {
        let after_node = &stmt_after_opts[node_idx + 4..];
        let text_start = if let Some(b) = after_node.find('{') { b } else { after_node.len() };
        if text_start < after_node.len() {
            if let Some(brace_end) = find_matching_brace(after_node, text_start) {
                label = Some(after_node[text_start + 1..brace_end].trim().to_string());
            }
        }
    }

    // Detect path type: -- (straight), |- (vert-horiz), -| (horiz-vert)
    // and find node references (name)
    let mut names: Vec<String> = Vec::new();
    let mut path_types: Vec<PathType> = Vec::new();

    let mut rest = stmt_after_opts;
    let mut last_was_name = false;

    while !rest.is_empty() {
        let rest_trimmed = rest.trim_start();

        // Check for path connectors
        if rest_trimmed.starts_with("|-") {
            path_types.push(PathType::VertHoriz);
            rest = &rest_trimmed[2..];
            last_was_name = false;
            continue;
        } else if rest_trimmed.starts_with("-|") {
            path_types.push(PathType::HorizVert);
            rest = &rest_trimmed[2..];
            last_was_name = false;
            continue;
        } else if rest_trimmed.starts_with("--") {
            path_types.push(PathType::Straight);
            rest = &rest_trimmed[2..];
            last_was_name = false;
            continue;
        }

        // Check for (name)
        if let Some(p_start) = rest_trimmed.find('(') {
            // Skip anything before the paren
            if p_start > 0 {
                rest = &rest_trimmed[p_start..];
                continue;
            }
            if let Some(p_end) = rest_trimmed[1..].find(')') {
                let inner = rest_trimmed[1..1 + p_end].trim();
                if !inner.is_empty() && !inner.contains(',')
                    && !inner.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == ' ')
                {
                    // Check it's not inside "node {}"
                    let before = &rest_trimmed[..0]; // name starts at (
                    if !before.ends_with("node") && !before.ends_with("node ") {
                        if last_was_name && path_types.len() < names.len() {
                            // Default to straight if no explicit connector between two names
                            path_types.push(PathType::Straight);
                        }
                        names.push(inner.to_string());
                        last_was_name = true;
                    }
                }
                rest = &rest_trimmed[1 + p_end + 1..];
            } else {
                break;
            }
        } else {
            // Skip to next ( or end
            break;
        }
    }

    // Create edges between consecutive names
    for i in 0..names.len().saturating_sub(1) {
        let pt = path_types.get(i).copied().unwrap_or(PathType::Straight);
        edges.push(TikzEdge {
            from: names[i].clone(),
            to: names[i + 1].clone(),
            style,
            color,
            line_width,
            label: if i == 0 { label.clone() } else { None },
            path_type: pt,
            from_anchor: None,
            to_anchor: None,
            label_pos: 0.5,
        });
    }
}

/// Parse xcolor color names
fn parse_color(name: &str) -> Color {
    match name.trim().to_lowercase().as_str() {
        "red" => Color::rgb(0.8, 0.2, 0.2),
        "blue" => Color::rgb(0.2, 0.2, 0.8),
        "green" => Color::rgb(0.2, 0.6, 0.2),
        "black" => Color::BLACK,
        "white" => Color::rgb(1.0, 1.0, 1.0),
        "gray" | "grey" => Color::GRAY,
        "lightgray" | "lightgrey" | "light gray" => Color::LIGHT_GRAY,
        "darkgray" | "darkgrey" => Color::DARK_GRAY,
        "yellow" => Color::rgb(0.9, 0.9, 0.2),
        "orange" => Color::rgb(0.9, 0.6, 0.1),
        "purple" | "violet" => Color::rgb(0.6, 0.2, 0.8),
        "cyan" => Color::rgb(0.0, 0.7, 0.7),
        "magenta" | "pink" => Color::rgb(0.8, 0.2, 0.6),
        "brown" => Color::rgb(0.6, 0.4, 0.2),
        "olive" => Color::rgb(0.5, 0.5, 0.0),
        "teal" => Color::rgb(0.0, 0.5, 0.5),
        _ => {
            // Try "color!intensity" format like "blue!30" or "blue!20"
            if let Some(excl) = name.find('!') {
                let base_name = &name[..excl];
                let intensity: f32 = name[excl + 1..].trim_end_matches(|c: char| !c.is_ascii_digit())
                    .parse().unwrap_or(50.0) / 100.0;
                let base = parse_color(base_name);
                // Mix with white
                let br = base.r as f32 / 255.0;
                let bg = base.g as f32 / 255.0;
                let bb = base.b as f32 / 255.0;
                Color::rgb(
                    br + (1.0 - br) * (1.0 - intensity),
                    bg + (1.0 - bg) * (1.0 - intensity),
                    bb + (1.0 - bb) * (1.0 - intensity),
                )
            } else {
                Color::rgb(0.3, 0.3, 0.6) // Default blue-gray
            }
        }
    }
}

fn parse_tikz_coord_value(s: &str) -> f32 {
    let s = s.trim();
    if let Some(star_idx) = s.find('*') {
        let left: f32 = parse_tikz_coord_value(&s[..star_idx]);
        let right: f32 = parse_tikz_coord_value(&s[star_idx + 1..]);
        return left * right / 28.35; // undo double cm conversion
    }

    let cleaned: String = s.chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    let val: f32 = cleaned.parse().unwrap_or(0.0);

    if s.contains("cm") { val * 28.35 }
    else if s.contains("mm") { val * 2.835 }
    else if s.contains("pt") { val }
    else if s.contains("in") { val * 72.0 }
    else { val * 28.35 }
}

fn parse_dimension(s: &str) -> f32 {
    let s = s.trim();
    let cleaned: String = s.chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let val: f32 = cleaned.parse().unwrap_or(0.0);

    if s.contains("cm") { val * 28.35 }
    else if s.contains("mm") { val * 2.835 }
    else if s.contains("pt") { val }
    else if s.contains("em") { val * 10.0 }
    else if s.contains("ex") { val * 5.0 }
    else { val * 28.35 }
}

fn strip_latex_commands(text: &str) -> String {
    let mut result = text.to_string();
    let commands = [
        "\\textbf{", "\\textit{", "\\textrm{", "\\textsc{", "\\textsf{",
        "\\texttt{", "\\emph{", "\\mathbf{", "\\mathrm{", "\\bfseries",
        "\\itshape", "\\rmfamily", "\\sffamily",
    ];
    for cmd in &commands {
        result = result.replace(cmd, "");
    }
    let open_count = result.matches('{').count();
    let close_count = result.matches('}').count();
    if close_count > open_count {
        let excess = close_count - open_count;
        for _ in 0..excess {
            if let Some(pos) = result.rfind('}') {
                result.remove(pos);
            }
        }
    }
    result.replace("\\,", " ").replace("\\;", " ").replace("\\!", "")
        .replace("\\quad", " ").replace("\\qquad", "  ")
        .replace("~", " ").replace("\\&", "&")
}

fn find_matching_bracket(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    for i in start..bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 { return Some(i); }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_brace(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    for i in start..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 { return Some(i); }
            }
            _ => {}
        }
    }
    None
}

/// Split comma-separated options, respecting nested braces/brackets
fn split_options(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth_brace = 0;
    let mut depth_bracket = 0;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        match bytes[i] {
            b'{' => depth_brace += 1,
            b'}' => depth_brace -= 1,
            b'[' => depth_bracket += 1,
            b']' => depth_bracket -= 1,
            b',' if depth_brace == 0 && depth_bracket == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}
