/// Image loading, TikZ diagram, and pgfplots rendering

use crate::color::Color;
use crate::typeset::FontStyle;
use crate::font::{self, FontId};
use super::state::LayoutState;
use super::types::*;

use anyhow::Result;

/// Try to load an image file and return embedded data + native dimensions.
/// Checks in-memory project images first (for WASM / multi-file compilation),
/// then falls back to filesystem (native CLI only).
pub(super) fn load_image_for_pdf(path: &str, state: &LayoutState) -> Option<(EmbeddedImage, u32, u32)> {
    // First: check in-memory project images
    if !state.project_images.is_empty() {
        if let Some(result) = load_from_project_images(path, &state.project_images) {
            return Some(result);
        }
    }

    // Second: try filesystem (native only)
    #[cfg(not(target_arch = "wasm32"))]
    {
        let extensions = ["", ".png", ".jpg", ".jpeg"];
        let mut search_paths = Vec::new();

        for ext in &extensions {
            // Try absolute/CWD-relative path first
            search_paths.push(format!("{}{}", path, ext));
            // Try relative to base_dir
            if !state.base_dir.is_empty() {
                search_paths.push(format!("{}/{}{}", state.base_dir, path, ext));
            }
        }

        for candidate in &search_paths {
            let p = std::path::Path::new(candidate);
            if !p.exists() { continue; }
            let data = match std::fs::read(p) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if let Some(result) = detect_image_format(data) {
                return Some(result);
            }
        }
    }

    None
}

/// Look up an image in the project's in-memory image store.
/// Tries exact match, then basename, then with common extensions.
fn load_from_project_images(
    path: &str,
    project_images: &std::collections::HashMap<String, Vec<u8>>,
) -> Option<(EmbeddedImage, u32, u32)> {
    // Extract just the filename (strip directory paths)
    let basename = path.rsplit('/').next().unwrap_or(path);
    let basename_no_ext = basename.rsplit_once('.').map(|(b, _)| b).unwrap_or(basename);

    // Candidates: exact path, basename, basename with extensions
    let candidates = [
        path.to_string(),
        basename.to_string(),
        format!("{}.png", basename_no_ext),
        format!("{}.jpg", basename_no_ext),
        format!("{}.jpeg", basename_no_ext),
        format!("{}.pdf", basename_no_ext),
    ];

    for candidate in &candidates {
        if let Some(data) = project_images.get(candidate.as_str()) {
            if let Some(result) = detect_image_format(data.clone()) {
                return Some(result);
            }
        }
    }

    // Fuzzy match: find any key that ends with the basename
    if !basename.is_empty() {
        for (key, data) in project_images {
            if key.ends_with(basename) {
                if let Some(result) = detect_image_format(data.clone()) {
                    return Some(result);
                }
            }
        }
    }

    None
}

/// Detect image format from raw bytes and return embedded image with dimensions.
fn detect_image_format(data: Vec<u8>) -> Option<(EmbeddedImage, u32, u32)> {
    if data.len() < 8 { return None; }

    if data[0..2] == [0xFF, 0xD8] {
        // JPEG
        if let Some((w, h)) = jpeg_dimensions(&data) {
            return Some((EmbeddedImage { data, width_px: w, height_px: h, format: ImageFormat::Jpeg }, w, h));
        }
    } else if data[0..4] == [0x89, b'P', b'N', b'G'] {
        // PNG
        if data.len() >= 24 {
            let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
            let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
            return Some((EmbeddedImage { data, width_px: w, height_px: h, format: ImageFormat::Png }, w, h));
        }
    }

    None
}

fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 < data.len() {
        if data[i] != 0xFF { i += 1; continue; }
        let marker = data[i + 1];
        if marker == 0 || marker == 0xFF { i += 1; continue; }
        let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xCC {
            if i + 9 < data.len() {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
        }
        i += 2 + seg_len;
    }
    None
}

/// Render TikZ diagram using native Rust renderer
pub(super) fn layout_tikz_diagram(tikz_source: &str, state: &mut LayoutState, _doc: &crate::document::Document) -> Result<()> {
    use crate::tikz_render::{self, TikzElement};

    state.add_vertical_space(10.0);

    if tikz_source.contains("\\begin{axis}") {
        if let Some((plot_elems, total_w, total_h)) = crate::pgfplots::render_pgfplot(tikz_source) {
            return layout_pgfplot_elements(&plot_elems, total_w, total_h, state);
        }
    }

    let result = tikz_render::render_tikz(tikz_source);

    if result.elements.is_empty() {
        let placeholder = "[TikZ diagram]";
        let box_h = 60.0;
        state.ensure_space(box_h + 20.0);
        let x = state.text_left() + (state.text_width() - 300.0) / 2.0;
        state.emit_rect(x, state.current_y, 300.0, box_h,
            Some(Color::rgb(0.95, 0.95, 0.98)), Some(Color::rgb(0.6, 0.6, 0.8)));
        let tw = font::measure_text(placeholder, FontId::Helvetica, 10.0);
        state.current_x = x + (300.0 - tw) / 2.0;
        state.emit_text(placeholder, 10.0, FontStyle::Italic, Color::GRAY);
        state.current_y += box_h + 10.0;
        state.current_x = state.text_left();
        return Ok(());
    }

    let available_w = state.text_width() * 0.9;
    let scale = (available_w / result.width).min(2.0);
    let scaled_h = result.height * scale;
    state.ensure_space(scaled_h + 20.0);

    let base_x = state.text_left() + (state.text_width() - result.width * scale) / 2.0;
    let base_y = state.current_y;

    for elem in &result.elements {
        match elem {
            TikzElement::Rect { x, y, width, height, fill, stroke, corner_radius, .. } => {
                if *corner_radius > 0.0 {
                    state.emit_rounded_rect(base_x + x * scale, base_y + y * scale, width * scale, height * scale, *fill, *stroke, *corner_radius * scale);
                } else {
                    state.emit_rect(base_x + x * scale, base_y + y * scale, width * scale, height * scale, *fill, *stroke);
                }
            }
            TikzElement::Line { x1, y1, x2, y2, width, color } => {
                state.emit_line(base_x + x1 * scale, base_y + y1 * scale, base_x + x2 * scale, base_y + y2 * scale, *width, *color);
            }
            TikzElement::Arrow { x1, y1, x2, y2, width, color, .. } => {
                let px1 = base_x + x1 * scale; let py1 = base_y + y1 * scale;
                let px2 = base_x + x2 * scale; let py2 = base_y + y2 * scale;
                state.emit_line(px1, py1, px2, py2, *width, *color);
                let angle = (py2 - py1).atan2(px2 - px1);
                let arr_len = 7.0;
                let a1x = px2 - arr_len * (angle - 0.35).cos();
                let a1y = py2 - arr_len * (angle - 0.35).sin();
                let a2x = px2 - arr_len * (angle + 0.35).cos();
                let a2y = py2 - arr_len * (angle + 0.35).sin();
                state.emit_line(px2, py2, a1x, a1y, *width, *color);
                state.emit_line(px2, py2, a2x, a2y, *width, *color);
            }
            TikzElement::Text { x, y, text, font_size, bold, color } => {
                let style = if *bold { FontStyle::Bold } else { FontStyle::Regular };
                let saved_y = state.current_y;
                state.current_x = base_x + x * scale;
                state.current_y = base_y + y * scale;
                state.emit_text(text, *font_size, style, *color);
                state.current_y = saved_y;
            }
        }
    }

    state.current_y = base_y + scaled_h + 10.0;
    state.current_x = state.text_left();
    state.add_vertical_space(10.0);
    Ok(())
}

pub(super) fn layout_pgfplot_elements(elems: &[crate::pgfplots::PlotElement], total_w: f32, total_h: f32, state: &mut LayoutState) -> Result<()> {
    use crate::pgfplots::{PlotElement, TextAnchor};

    let available_w = state.text_width();
    let scale = (available_w / total_w).min(1.5);
    let scaled_w = total_w * scale;
    let scaled_h = total_h * scale;
    state.ensure_space(scaled_h + 20.0);

    let base_x = state.text_left() + (available_w - scaled_w) / 2.0;
    let base_y = state.current_y;

    for elem in elems {
        match elem {
            PlotElement::Line { x1, y1, x2, y2, width, color } => {
                state.emit_line(base_x + x1 * scale, base_y + y1 * scale, base_x + x2 * scale, base_y + y2 * scale, width * scale, *color);
            }
            PlotElement::Rect { x, y, width, height, fill, stroke } => {
                state.emit_rect(base_x + x * scale, base_y + y * scale, width * scale, height * scale, *fill, *stroke);
            }
            PlotElement::Text { x, y, text, font_size, color, anchor, rotation } => {
                let fs = font_size * scale;
                let tw = font::measure_text(text, FontId::Helvetica, fs);
                let abs_x = base_x + x * scale;
                let abs_y = base_y + y * scale;
                let (tx, ty) = match anchor {
                    TextAnchor::Center => (abs_x - tw / 2.0, abs_y),
                    TextAnchor::West => (abs_x, abs_y),
                    TextAnchor::East => (abs_x - tw, abs_y),
                    TextAnchor::North => (abs_x - tw / 2.0, abs_y),
                    TextAnchor::South => (abs_x - tw / 2.0, abs_y - fs),
                };
                if *rotation > 0.0 {
                    state.emit_text(text, fs, FontStyle::Regular, *color);
                    state.current_x = tx;
                } else {
                    let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                    state.all_text.push_str(text);
                    state.all_elements.push(PageElement::Text {
                        x: tx, y: ty, text_offset: offset,
                        text_len: text.len().min(65535) as u16,
                        font_size_100: (fs * 100.0) as u16,
                        font_style: FontStyle::Regular, color: *color, word_spacing_50: 0,
                    });
                }
            }
            PlotElement::Circle { cx, cy, radius, fill } => {
                let r = radius * scale;
                let abs_cx = base_x + cx * scale;
                let abs_cy = base_y + cy * scale;
                state.emit_rect(abs_cx - r, abs_cy - r, r * 2.0, r * 2.0, Some(*fill), None);
            }
        }
    }

    state.current_y = base_y + scaled_h + 10.0;
    state.current_x = state.text_left();
    state.add_vertical_space(10.0);
    Ok(())
}
