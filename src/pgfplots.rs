/// PGFPlots renderer: parses \begin{axis}...\end{axis} and produces
/// positioned drawing primitives (lines, text, rectangles) for PDF output.
///
/// Supports:
/// - \addplot coordinates {(x,y) ...};
/// - \addplot {expression};  (basic polynomial evaluation)
/// - Line plots, bar charts (ybar)
/// - Axis labels, title, legend
/// - Grid (major, both)
/// - domain, samples
/// - symbolic x coords

use crate::color::Color;
use crate::font::{self, FontId};

/// A rendered plot element
#[derive(Debug)]
pub enum PlotElement {
    Line {
        x1: f32, y1: f32,
        x2: f32, y2: f32,
        width: f32,
        color: Color,
    },
    Rect {
        x: f32, y: f32,
        width: f32, height: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
    },
    Text {
        x: f32, y: f32,
        text: String,
        font_size: f32,
        color: Color,
        anchor: TextAnchor,
        rotation: f32,
    },
    Circle {
        cx: f32, cy: f32,
        radius: f32,
        fill: Color,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum TextAnchor {
    Center,
    West,
    East,
    North,
    South,
}

/// Axis configuration
#[derive(Debug)]
struct AxisConfig {
    title: Option<String>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    width: f32,
    height: f32,
    grid: GridStyle,
    ybar: bool,
    domain: (f64, f64),
    samples: usize,
    symbolic_x: Vec<String>,
    xtick_data: bool,
    legend: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GridStyle {
    None,
    Major,
    Both,
}

impl Default for AxisConfig {
    fn default() -> Self {
        AxisConfig {
            title: None,
            xlabel: None,
            ylabel: None,
            width: 280.0,  // ~10cm
            height: 168.0,  // ~6cm
            grid: GridStyle::None,
            ybar: false,
            domain: (-5.0, 5.0),
            samples: 25,
            symbolic_x: Vec::new(),
            xtick_data: false,
            legend: Vec::new(),
        }
    }
}

/// A data series
#[derive(Debug)]
struct PlotSeries {
    points: Vec<(f64, f64)>,
    style: SeriesStyle,
    color_idx: usize,
}

#[derive(Debug, Clone, Copy)]
enum SeriesStyle {
    Line,
    Bar,
}

/// Default color cycle (matching pgfplots default)
const PLOT_COLORS: &[(u8, u8, u8)] = &[
    (0, 114, 189),    // blue
    (217, 83, 25),    // red/orange
    (237, 177, 32),   // yellow
    (126, 47, 142),   // purple
    (119, 172, 48),   // green
    (77, 190, 238),   // cyan
    (162, 20, 47),    // dark red
];

fn plot_color(idx: usize) -> Color {
    let (r, g, b) = PLOT_COLORS[idx % PLOT_COLORS.len()];
    Color { r, g, b }
}

/// Render a tikzpicture containing an axis environment
pub fn render_pgfplot(source: &str) -> Option<(Vec<PlotElement>, f32, f32)> {
    // Find \begin{axis} ... \end{axis}
    let axis_start = source.find("\\begin{axis}")?;
    let axis_end = source.find("\\end{axis}")?;
    let axis_body = &source[axis_start + 12..axis_end];

    // Parse axis options
    let mut config = AxisConfig::default();
    let mut body_start = 0;
    if let Some(opts_start) = axis_body.find('[') {
        if let Some(opts_end) = find_matching_bracket(axis_body, opts_start) {
            let opts = &axis_body[opts_start + 1..opts_end];
            parse_axis_options(opts, &mut config);
            body_start = opts_end + 1;
        }
    }

    // Parse \addplot commands — use only the part after options to avoid multi-byte issues
    let plot_body = &axis_body[body_start..];
    let mut series_list: Vec<PlotSeries> = Vec::new();
    let mut pos = 0;
    let bytes = plot_body.as_bytes();
    while pos < bytes.len() {
        // Ensure we're at a char boundary
        if !plot_body.is_char_boundary(pos) { pos += 1; continue; }
        if plot_body[pos..].starts_with("\\addplot") {
            pos += 8;
            // Skip optional + or *
            while pos < bytes.len() && (bytes[pos] == b'+' || bytes[pos] == b'*') {
                pos += 1;
            }
            // Skip whitespace
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
            // Skip optional [options]
            if pos < bytes.len() && bytes[pos] == b'[' {
                if let Some(end) = find_matching_bracket(plot_body, pos) {
                    pos = end + 1;
                }
            }
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }

            let style = if config.ybar { SeriesStyle::Bar } else { SeriesStyle::Line };

            if pos < bytes.len() && plot_body.is_char_boundary(pos) && plot_body[pos..].starts_with("coordinates") {
                pos += 11;
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
                if pos < bytes.len() && bytes[pos] == b'{' {
                    if let Some(end_brace) = find_matching_brace(plot_body, pos) {
                        let coord_str = &plot_body[pos + 1..end_brace];
                        let points = parse_coordinates(coord_str, &config.symbolic_x);
                        series_list.push(PlotSeries {
                            points,
                            style,
                            color_idx: series_list.len(),
                        });
                        pos = end_brace + 1;
                    }
                }
            } else if pos < bytes.len() && plot_body.is_char_boundary(pos) && (plot_body[pos..].starts_with("{") || plot_body[pos..].starts_with("table")) {
                // Skip table plots for now
                if let Some(semi) = plot_body[pos..].find(';') {
                    pos += semi + 1;
                }
            } else {
                // Expression plot: \addplot {expr};
                let expr_start = pos;
                if pos < bytes.len() && plot_body.is_char_boundary(pos) {
                    if let Some(semi) = plot_body[pos..].find(';') {
                        let expr = plot_body[expr_start..pos + semi].trim();
                        // Remove surrounding braces if present
                        let expr = expr.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(expr);
                        let points = evaluate_expression(expr, config.domain, config.samples);
                        series_list.push(PlotSeries {
                            points,
                            style: SeriesStyle::Line,
                            color_idx: series_list.len(),
                        });
                        pos += semi + 1;
                    } else {
                        pos += 1;
                    }
                } else {
                    pos += 1;
                }
            }
        } else if pos < bytes.len() && plot_body.is_char_boundary(pos) && plot_body[pos..].starts_with("\\legend") {
            pos += 7;
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
            if pos < bytes.len() && bytes[pos] == b'{' {
                if let Some(end_brace) = find_matching_brace(plot_body, pos) {
                    let legend_str = &plot_body[pos + 1..end_brace];
                    config.legend = legend_str.split(',')
                        .map(|s| s.trim().replace("$", "").to_string())
                        .collect();
                    pos = end_brace + 1;
                } else { pos += 1; }
            } else { pos += 1; }
        } else {
            pos += 1;
        }
    }

    if series_list.is_empty() {
        return None;
    }

    // Generate plot elements
    let elements = generate_plot(&config, &series_list);
    let total_width = config.width + 80.0;  // margin for labels
    let total_height = config.height + 80.0;  // margin for title + labels
    Some((elements, total_width, total_height))
}

fn parse_axis_options(opts: &str, config: &mut AxisConfig) {
    // Parse key=value pairs (handling nested braces)
    let mut pos = 0;
    let bytes = opts.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b',') { pos += 1; }
        if pos >= bytes.len() { break; }

        // Read key
        let key_start = pos;
        while pos < bytes.len() && bytes[pos] != b'=' && bytes[pos] != b',' && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let key = opts[key_start..pos].trim();

        if pos < bytes.len() && bytes[pos] == b'=' {
            pos += 1; // skip =
            // Read value (may be braced)
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
            let value = if pos < bytes.len() && bytes[pos] == b'{' {
                if let Some(end) = find_matching_brace(opts, pos) {
                    let v = &opts[pos + 1..end];
                    pos = end + 1;
                    v
                } else {
                    pos += 1;
                    ""
                }
            } else {
                let val_start = pos;
                while pos < bytes.len() && bytes[pos] != b',' && bytes[pos] != b']' {
                    pos += 1;
                }
                opts[val_start..pos].trim()
            };

            match key {
                "title" => config.title = Some(strip_math(value)),
                "xlabel" => config.xlabel = Some(strip_math(value)),
                "ylabel" => config.ylabel = Some(strip_math(value)),
                "width" => config.width = parse_dimension(value),
                "height" => config.height = parse_dimension(value),
                "grid" => config.grid = match value {
                    "major" => GridStyle::Major,
                    "both" | "minor" => GridStyle::Both,
                    _ => GridStyle::None,
                },
                "domain" => {
                    let parts: Vec<&str> = value.split(':').collect();
                    if parts.len() == 2 {
                        config.domain = (
                            parts[0].trim().parse().unwrap_or(-5.0),
                            parts[1].trim().parse().unwrap_or(5.0),
                        );
                    }
                }
                "samples" => config.samples = value.parse().unwrap_or(25),
                "symbolic x coords" => {
                    config.symbolic_x = value.split(',').map(|s| s.trim().to_string()).collect();
                }
                "xtick" => {
                    if value == "data" { config.xtick_data = true; }
                }
                _ => {}
            }
        } else {
            // Bare key (no value)
            match key {
                "ybar" => config.ybar = true,
                _ => {}
            }
        }
    }
}

fn parse_coordinates(s: &str, symbolic_x: &[String]) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Find (
        if let Some(paren_start) = s[pos..].find('(') {
            let abs_start = pos + paren_start + 1;
            if let Some(paren_end) = s[abs_start..].find(')') {
                let pair = &s[abs_start..abs_start + paren_end];
                if let Some(comma) = pair.find(',') {
                    let x_str = pair[..comma].trim();
                    let y_str = pair[comma + 1..].trim();

                    let x = if let Ok(v) = x_str.parse::<f64>() {
                        v
                    } else if let Some(idx) = symbolic_x.iter().position(|sx| sx == x_str) {
                        idx as f64
                    } else {
                        pos = abs_start + paren_end + 1;
                        continue;
                    };

                    let y: f64 = y_str.parse().unwrap_or(0.0);
                    points.push((x, y));
                }
                pos = abs_start + paren_end + 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    points
}

fn evaluate_expression(expr: &str, domain: (f64, f64), samples: usize) -> Vec<(f64, f64)> {
    let mut points = Vec::with_capacity(samples);
    let step = (domain.1 - domain.0) / (samples.max(2) - 1) as f64;
    for i in 0..samples {
        let x = domain.0 + i as f64 * step;
        let y = eval_simple(expr, x);
        if y.is_finite() {
            points.push((x, y));
        }
    }
    points
}

/// Simple expression evaluator for polynomial-like expressions
fn eval_simple(expr: &str, x: f64) -> f64 {
    // Remove whitespace
    let expr = expr.replace(" ", "");
    // Handle addition/subtraction at the top level
    eval_add_sub(&expr, x)
}

fn eval_add_sub(expr: &str, x: f64) -> f64 {
    // Split on + or - at top level (not inside parens)
    let bytes = expr.as_bytes();
    let mut depth = 0;
    let mut last_op_pos = 0;
    let mut result = 0.0f64;
    let mut current_sign = 1.0f64;
    let mut i = 0;

    // Handle leading minus
    if !bytes.is_empty() && bytes[0] == b'-' {
        current_sign = -1.0;
        i = 1;
        last_op_pos = 1;
    }

    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
                let term = &expr[last_op_pos..i];
                result += current_sign * eval_mul_div(term, x);
                current_sign = if bytes[i] == b'+' { 1.0 } else { -1.0 };
                last_op_pos = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    // Last term
    if last_op_pos < bytes.len() {
        let term = &expr[last_op_pos..];
        result += current_sign * eval_mul_div(term, x);
    }
    result
}

fn eval_mul_div(expr: &str, x: f64) -> f64 {
    let bytes = expr.as_bytes();
    let mut depth = 0;
    let mut last_op_pos = 0;
    let mut result = 1.0f64;
    let mut current_op = b'*';

    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                let term = &expr[last_op_pos..i];
                let val = eval_power(term, x);
                if current_op == b'*' { result *= val; } else { result /= val; }
                current_op = bytes[i];
                last_op_pos = i + 1;
            }
            _ => {}
        }
    }
    let term = &expr[last_op_pos..];
    let val = eval_power(term, x);
    if current_op == b'*' { result *= val; } else { result /= val; }
    result
}

fn eval_power(expr: &str, x: f64) -> f64 {
    if let Some(caret) = expr.find('^') {
        let base = eval_atom(&expr[..caret], x);
        let exp = eval_atom(&expr[caret + 1..], x);
        base.powf(exp)
    } else {
        eval_atom(expr, x)
    }
}

fn eval_atom(expr: &str, x: f64) -> f64 {
    let expr = expr.trim();
    if expr.is_empty() { return 0.0; }
    if expr == "x" { return x; }
    if let Ok(v) = expr.parse::<f64>() { return v; }

    // Handle parenthesized expression
    if expr.starts_with('(') && expr.ends_with(')') {
        return eval_add_sub(&expr[1..expr.len() - 1], x);
    }

    // Handle functions
    if expr.starts_with("sin(") { return eval_add_sub(&expr[4..expr.len() - 1], x).sin(); }
    if expr.starts_with("cos(") { return eval_add_sub(&expr[4..expr.len() - 1], x).cos(); }
    if expr.starts_with("exp(") { return eval_add_sub(&expr[4..expr.len() - 1], x).exp(); }
    if expr.starts_with("sqrt(") { return eval_add_sub(&expr[5..expr.len() - 1], x).sqrt(); }
    if expr.starts_with("abs(") { return eval_add_sub(&expr[4..expr.len() - 1], x).abs(); }
    if expr.starts_with("ln(") { return eval_add_sub(&expr[3..expr.len() - 1], x).ln(); }

    // Handle implicit multiplication like "3x"
    if let Some(pos) = expr.find('x') {
        if pos > 0 {
            let coeff: f64 = expr[..pos].parse().unwrap_or(1.0);
            return coeff * x;
        }
    }

    0.0
}

fn generate_plot(config: &AxisConfig, series: &[PlotSeries]) -> Vec<PlotElement> {
    let mut elements = Vec::new();

    // Margins
    let left_margin = 55.0;
    let bottom_margin = 40.0;
    let top_margin = if config.title.is_some() { 30.0 } else { 10.0 };
    let right_margin = 15.0;
    let legend_space = if !config.legend.is_empty() { 20.0 } else { 0.0 };

    let plot_w = config.width - left_margin - right_margin;
    let plot_h = config.height - top_margin - bottom_margin - legend_space;
    let plot_x = left_margin;
    let plot_y = top_margin;

    // Calculate data bounds
    let (x_min, x_max, y_min, y_max) = compute_bounds(series, config);

    let x_range = (x_max - x_min).max(1e-10);
    let y_range = (y_max - y_min).max(1e-10);

    // Helper closures for coordinate mapping
    let map_x = |v: f64| -> f32 { plot_x + ((v - x_min) / x_range) as f32 * plot_w };
    let map_y = |v: f64| -> f32 { plot_y + plot_h - ((v - y_min) / y_range) as f32 * plot_h };

    // Draw plot area background
    elements.push(PlotElement::Rect {
        x: plot_x, y: plot_y,
        width: plot_w, height: plot_h,
        fill: Some(Color { r: 255, g: 255, b: 255 }),
        stroke: Some(Color { r: 0, g: 0, b: 0 }),
    });

    // Grid lines
    if config.grid != GridStyle::None {
        let (x_ticks, _) = compute_ticks(x_min, x_max, 5, &config.symbolic_x);
        let (y_ticks, _) = compute_ticks(y_min, y_max, 5, &[]);
        let grid_color = Color { r: 200, g: 200, b: 200 };

        for &xt in &x_ticks {
            let px = map_x(xt);
            if px > plot_x && px < plot_x + plot_w {
                elements.push(PlotElement::Line {
                    x1: px, y1: plot_y,
                    x2: px, y2: plot_y + plot_h,
                    width: 0.3, color: grid_color,
                });
            }
        }
        for &yt in &y_ticks {
            let py = map_y(yt);
            if py > plot_y && py < plot_y + plot_h {
                elements.push(PlotElement::Line {
                    x1: plot_x, y1: py,
                    x2: plot_x + plot_w, y2: py,
                    width: 0.3, color: grid_color,
                });
            }
        }
    }

    // Draw data series
    for s in series {
        let color = plot_color(s.color_idx);
        match s.style {
            SeriesStyle::Line => {
                // Draw line segments
                for i in 1..s.points.len() {
                    let (x1, y1) = s.points[i - 1];
                    let (x2, y2) = s.points[i];
                    elements.push(PlotElement::Line {
                        x1: map_x(x1), y1: map_y(y1),
                        x2: map_x(x2), y2: map_y(y2),
                        width: 1.5, color,
                    });
                }
                // Draw data point markers
                for &(x, y) in &s.points {
                    elements.push(PlotElement::Circle {
                        cx: map_x(x), cy: map_y(y),
                        radius: 2.0, fill: color,
                    });
                }
            }
            SeriesStyle::Bar => {
                let n_series = series.iter().filter(|s| matches!(s.style, SeriesStyle::Bar)).count();
                let bar_group_width = if s.points.len() > 1 {
                    let dx = (s.points[1].0 - s.points[0].0).abs();
                    (dx / x_range) as f32 * plot_w * 0.8
                } else {
                    plot_w * 0.6 / s.points.len().max(1) as f32
                };
                let bar_width = bar_group_width / n_series as f32;
                let bar_offset = s.color_idx as f32 * bar_width - bar_group_width / 2.0 + bar_width / 2.0;

                for &(x, y) in &s.points {
                    let cx = map_x(x) + bar_offset;
                    let top = map_y(y);
                    let bottom = map_y(0.0f64.max(y_min));
                    let h = (bottom - top).abs();
                    elements.push(PlotElement::Rect {
                        x: cx - bar_width / 2.0 + 1.0,
                        y: top.min(bottom),
                        width: bar_width - 2.0,
                        height: h,
                        fill: Some(color),
                        stroke: Some(Color { r: 0, g: 0, b: 0 }),
                    });
                }
            }
        }
    }

    // Axis ticks and labels
    let (x_ticks, x_labels) = compute_ticks(x_min, x_max, 5, &config.symbolic_x);
    let (y_ticks, y_labels) = compute_ticks(y_min, y_max, 5, &[]);

    let tick_len = 4.0;
    let label_size = 8.0;

    // X-axis ticks
    for (i, &xt) in x_ticks.iter().enumerate() {
        let px = map_x(xt);
        // Tick mark
        elements.push(PlotElement::Line {
            x1: px, y1: plot_y + plot_h,
            x2: px, y2: plot_y + plot_h + tick_len,
            width: 0.5, color: Color::BLACK,
        });
        // Label
        if i < x_labels.len() {
            elements.push(PlotElement::Text {
                x: px, y: plot_y + plot_h + tick_len + 10.0,
                text: x_labels[i].clone(),
                font_size: label_size,
                color: Color::BLACK,
                anchor: TextAnchor::North,
                rotation: 0.0,
            });
        }
    }

    // Y-axis ticks
    for (i, &yt) in y_ticks.iter().enumerate() {
        let py = map_y(yt);
        elements.push(PlotElement::Line {
            x1: plot_x - tick_len, y1: py,
            x2: plot_x, y2: py,
            width: 0.5, color: Color::BLACK,
        });
        if i < y_labels.len() {
            elements.push(PlotElement::Text {
                x: plot_x - tick_len - 3.0, y: py,
                text: y_labels[i].clone(),
                font_size: label_size,
                color: Color::BLACK,
                anchor: TextAnchor::East,
                rotation: 0.0,
            });
        }
    }

    // Title
    if let Some(ref title) = config.title {
        elements.push(PlotElement::Text {
            x: plot_x + plot_w / 2.0,
            y: 5.0,
            text: title.clone(),
            font_size: 11.0,
            color: Color::BLACK,
            anchor: TextAnchor::North,
            rotation: 0.0,
        });
    }

    // X-axis label
    if let Some(ref xlabel) = config.xlabel {
        elements.push(PlotElement::Text {
            x: plot_x + plot_w / 2.0,
            y: config.height - 5.0,
            text: xlabel.clone(),
            font_size: 9.0,
            color: Color::BLACK,
            anchor: TextAnchor::North,
            rotation: 0.0,
        });
    }

    // Y-axis label (rotated)
    if let Some(ref ylabel) = config.ylabel {
        elements.push(PlotElement::Text {
            x: 8.0,
            y: plot_y + plot_h / 2.0,
            text: ylabel.clone(),
            font_size: 9.0,
            color: Color::BLACK,
            anchor: TextAnchor::Center,
            rotation: 90.0,
        });
    }

    // Legend
    if !config.legend.is_empty() {
        let legend_x = plot_x + plot_w - 10.0;
        let legend_y = plot_y + 10.0;
        let line_height = 12.0;
        let legend_w = config.legend.iter()
            .map(|l| font::measure_text(l, FontId::Helvetica, 8.0))
            .fold(0.0f32, f32::max) + 30.0;

        // Legend background
        elements.push(PlotElement::Rect {
            x: legend_x - legend_w,
            y: legend_y,
            width: legend_w,
            height: config.legend.len() as f32 * line_height + 6.0,
            fill: Some(Color { r: 255, g: 255, b: 255 }),
            stroke: Some(Color { r: 180, g: 180, b: 180 }),
        });

        for (i, label) in config.legend.iter().enumerate() {
            let ly = legend_y + 5.0 + i as f32 * line_height + 6.0;
            let color = plot_color(i);
            // Color line
            elements.push(PlotElement::Line {
                x1: legend_x - legend_w + 5.0,
                y1: ly,
                x2: legend_x - legend_w + 20.0,
                y2: ly,
                width: 2.0,
                color,
            });
            // Label text
            elements.push(PlotElement::Text {
                x: legend_x - legend_w + 25.0,
                y: ly,
                text: label.clone(),
                font_size: 8.0,
                color: Color::BLACK,
                anchor: TextAnchor::West,
                rotation: 0.0,
            });
        }
    }

    elements
}

fn compute_bounds(series: &[PlotSeries], config: &AxisConfig) -> (f64, f64, f64, f64) {
    let mut x_min = f64::MAX;
    let mut x_max = f64::MIN;
    let mut y_min = 0.0f64; // Usually include zero for bar charts
    let mut y_max = f64::MIN;

    for s in series {
        for &(x, y) in &s.points {
            if x < x_min { x_min = x; }
            if x > x_max { x_max = x; }
            if y < y_min { y_min = y; }
            if y > y_max { y_max = y; }
        }
    }

    // Add padding
    let y_pad = (y_max - y_min) * 0.1;
    y_max += y_pad;
    if !config.ybar {
        y_min -= y_pad;
    }

    // For symbolic x coords, use index-based bounds
    if !config.symbolic_x.is_empty() {
        x_min = -0.5;
        x_max = (config.symbolic_x.len() as f64) - 0.5;
    }

    (x_min, x_max, y_min, y_max)
}

fn compute_ticks(min: f64, max: f64, target_count: usize, symbolic: &[String]) -> (Vec<f64>, Vec<String>) {
    if !symbolic.is_empty() {
        let ticks: Vec<f64> = (0..symbolic.len()).map(|i| i as f64).collect();
        let labels = symbolic.to_vec();
        return (ticks, labels);
    }

    let range = max - min;
    if range <= 0.0 {
        return (vec![min], vec![format_number(min)]);
    }

    // Nice tick spacing
    let raw_step = range / target_count as f64;
    let magnitude = 10.0f64.powf(raw_step.log10().floor());
    let nice_step = if raw_step / magnitude < 1.5 {
        magnitude
    } else if raw_step / magnitude < 3.5 {
        2.0 * magnitude
    } else if raw_step / magnitude < 7.5 {
        5.0 * magnitude
    } else {
        10.0 * magnitude
    };

    let start = (min / nice_step).ceil() * nice_step;
    let mut ticks = Vec::new();
    let mut labels = Vec::new();
    let mut v = start;
    while v <= max + nice_step * 0.01 {
        ticks.push(v);
        labels.push(format_number(v));
        v += nice_step;
    }
    (ticks, labels)
}

fn format_number(v: f64) -> String {
    if v.abs() < 1e-10 { return "0".to_string(); }
    if v.fract().abs() < 1e-10 {
        format!("{}", v as i64)
    } else if v.abs() < 0.01 || v.abs() >= 10000.0 {
        format!("{:.1e}", v)
    } else {
        let s = format!("{:.2}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn strip_math(s: &str) -> String {
    s.replace("$", "")
     .replace("\\(", "")
     .replace("\\)", "")
     .replace("\\$", "$")
}

fn parse_dimension(s: &str) -> f32 {
    let s = s.trim();
    if s.ends_with("cm") {
        let v: f32 = s[..s.len() - 2].trim().parse().unwrap_or(10.0);
        v * 28.35  // cm to points
    } else if s.ends_with("pt") {
        s[..s.len() - 2].trim().parse().unwrap_or(280.0)
    } else if s.ends_with("in") {
        let v: f32 = s[..s.len() - 2].trim().parse().unwrap_or(4.0);
        v * 72.0
    } else {
        s.parse().unwrap_or(280.0)
    }
}

fn find_matching_bracket(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&b'[') { return None; }
    let mut depth = 1;
    let mut pos = start + 1;
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'[' => depth += 1,
            b']' => { depth -= 1; if depth == 0 { return Some(pos); } }
            _ => {}
        }
        pos += 1;
    }
    None
}

fn find_matching_brace(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&b'{') { return None; }
    let mut depth = 1;
    let mut pos = start + 1;
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { return Some(pos); } }
            b'\\' => { pos += 1; } // skip escaped char
            _ => {}
        }
        pos += 1;
    }
    None
}
