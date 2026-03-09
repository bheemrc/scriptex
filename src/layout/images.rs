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
pub(super) fn load_image_for_pdf(path: &str, state: &LayoutState, graphics_paths: &[String]) -> Option<(EmbeddedImage, u32, u32)> {
    // First: check in-memory project images
    if !state.project_images.is_empty() {
        if let Some(result) = load_from_project_images(path, &state.project_images) {
            return Some(result);
        }
    }

    // Second: try filesystem (native only)
    #[cfg(not(target_arch = "wasm32"))]
    {
        let extensions = ["", ".png", ".jpg", ".jpeg", ".pdf", ".svg"];
        let mut search_paths = Vec::new();

        for ext in &extensions {
            // Try absolute/CWD-relative path first
            search_paths.push(format!("{}{}", path, ext));
            // Try relative to base_dir
            if !state.base_dir.is_empty() {
                search_paths.push(format!("{}/{}{}", state.base_dir, path, ext));
            }
            // Try graphics_paths
            for gp in graphics_paths {
                search_paths.push(format!("{}{}{}", gp, path, ext));
                if !state.base_dir.is_empty() {
                    search_paths.push(format!("{}/{}{}{}", state.base_dir, gp, path, ext));
                }
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
            return Some((EmbeddedImage { data, width_px: w, height_px: h, format: ImageFormat::Jpeg, has_alpha: false, alpha_data: Vec::new() }, w, h));
        }
    } else if data[0..4] == [0x89, b'P', b'N', b'G'] {
        // PNG
        if data.len() >= 26 {
            let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
            let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
            let color_type = data[25];
            let has_alpha = color_type == 4 || color_type == 6;
            return Some((EmbeddedImage { data, width_px: w, height_px: h, format: ImageFormat::Png, has_alpha, alpha_data: Vec::new() }, w, h));
        }
    } else if data.len() >= 5 && &data[0..5] == b"%PDF-" {
        // PDF
        if let Some(result) = load_pdf_page(&data) {
            return Some(result);
        }
    } else if crate::svg_render::is_svg_data(&data) {
        // SVG
        if let Some((w, h)) = crate::svg_render::svg_dimensions(&data) {
            return Some((EmbeddedImage { data, width_px: w, height_px: h, format: ImageFormat::Svg, has_alpha: false, alpha_data: Vec::new() }, w, h));
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

/// Load a PDF file and extract the first page as a Form XObject.
/// Returns (EmbeddedImage with content stream, width_px, height_px).
/// width_px/height_px are MediaBox dimensions in points (treated as "pixels" at 72dpi).
fn load_pdf_page(data: &[u8]) -> Option<(EmbeddedImage, u32, u32)> {
    let pdf = PdfParser::new(data)?;
    let page = pdf.first_page()?;

    let w = (page.bbox[2] - page.bbox[0]).max(1.0) as u32;
    let h = (page.bbox[3] - page.bbox[1]).max(1.0) as u32;

    Some((EmbeddedImage {
        data: page.content_stream,
        width_px: w,
        height_px: h,
        format: ImageFormat::Pdf {
            bbox: page.bbox,
            resources: page.resources,
        },
        has_alpha: false,
        alpha_data: Vec::new(),
    }, w, h))
}

/// Extracted first-page data from a PDF
struct PdfPageData {
    bbox: [f32; 4],
    content_stream: Vec<u8>,
    resources: Vec<u8>,
}

/// Minimal PDF parser — just enough to extract the first page's content stream,
/// resources dictionary, and MediaBox for Form XObject embedding.
struct PdfParser<'a> {
    data: &'a [u8],
    /// Cross-reference table: object number -> byte offset
    xref: Vec<usize>,
}

impl<'a> PdfParser<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        let xref = Self::parse_xref(data)?;
        Some(PdfParser { data, xref })
    }

    /// Find startxref and parse cross-reference table
    fn parse_xref(data: &[u8]) -> Option<Vec<usize>> {
        let search_start = data.len().saturating_sub(1024);
        let tail = &data[search_start..];
        let startxref_pos = find_bytes(tail, b"startxref")
            .map(|p| p + search_start)?;

        let after = &data[startxref_pos + 9..];
        let xref_offset = parse_next_int(after)? as usize;

        if xref_offset >= data.len() { return None; }

        if data.len() > xref_offset + 4 && &data[xref_offset..xref_offset + 4] == b"xref" {
            Self::parse_xref_table(data, xref_offset)
        } else {
            Self::parse_xref_stream(data, xref_offset)
        }
    }

    fn parse_xref_table(data: &[u8], offset: usize) -> Option<Vec<usize>> {
        let mut xref = vec![0usize; 4096];
        let mut pos = offset + 4;

        while pos < data.len() && (data[pos] == b' ' || data[pos] == b'\n' || data[pos] == b'\r') {
            pos += 1;
        }

        while pos < data.len() {
            if pos + 7 <= data.len() && &data[pos..pos + 7] == b"trailer" {
                break;
            }

            let start_obj = pdf_parse_int_at(data, &mut pos)? as usize;
            pdf_skip_whitespace(data, &mut pos);
            let count = pdf_parse_int_at(data, &mut pos)? as usize;
            pdf_skip_whitespace(data, &mut pos);

            if start_obj + count > xref.len() {
                xref.resize(start_obj + count, 0);
            }

            for i in 0..count {
                if pos + 20 > data.len() { break; }
                let obj_offset = pdf_parse_int_at(data, &mut pos)? as usize;
                pdf_skip_whitespace(data, &mut pos);
                let _gen = pdf_parse_int_at(data, &mut pos)?;
                pdf_skip_whitespace(data, &mut pos);
                let status = data[pos];
                pos += 1;
                pdf_skip_whitespace(data, &mut pos);

                if status == b'n' {
                    xref[start_obj + i] = obj_offset;
                }
            }
        }

        Some(xref)
    }

    fn parse_xref_stream(data: &[u8], offset: usize) -> Option<Vec<usize>> {
        let obj_data = &data[offset..];

        let dict_start = find_bytes(obj_data, b"<<")?;
        let dict_end = find_matching_dict_end(obj_data, dict_start)?;
        let dict = &obj_data[dict_start..dict_end + 2];

        let size = extract_int_value(dict, b"/Size")? as usize;
        let w_array = extract_array(dict, b"/W")?;
        if w_array.len() != 3 { return None; }

        let w1 = w_array[0] as usize;
        let w2 = w_array[1] as usize;
        let w3 = w_array[2] as usize;
        let entry_size = w1 + w2 + w3;
        if entry_size == 0 { return None; }

        let index_ranges = if let Some(idx) = extract_array(dict, b"/Index") {
            idx
        } else {
            vec![0, size as i64]
        };

        let stream_data = extract_stream(obj_data, dict_end + 2, dict)?;

        let mut xref = vec![0usize; size];
        let mut stream_pos = 0;

        let mut range_idx = 0;
        while range_idx + 1 < index_ranges.len() {
            let start_obj = index_ranges[range_idx] as usize;
            let count = index_ranges[range_idx + 1] as usize;
            range_idx += 2;

            for i in 0..count {
                if stream_pos + entry_size > stream_data.len() { break; }

                let typ = if w1 > 0 {
                    read_be_uint(&stream_data[stream_pos..stream_pos + w1])
                } else {
                    1
                };
                let field2 = read_be_uint(&stream_data[stream_pos + w1..stream_pos + w1 + w2]);
                let _field3 = if w3 > 0 {
                    read_be_uint(&stream_data[stream_pos + w1 + w2..stream_pos + entry_size])
                } else {
                    0
                };

                let obj_num = start_obj + i;
                if obj_num < xref.len() && typ == 1 {
                    xref[obj_num] = field2 as usize;
                }

                stream_pos += entry_size;
            }
        }

        Some(xref)
    }

    fn resolve_object(&self, obj_num: usize) -> Option<&'a [u8]> {
        if obj_num >= self.xref.len() { return None; }
        let offset = self.xref[obj_num];
        if offset == 0 || offset >= self.data.len() { return None; }
        Some(&self.data[offset..])
    }

    fn first_page(&self) -> Option<PdfPageData> {
        let root_num = self.find_root()?;
        let root_obj = self.resolve_object(root_num)?;

        let pages_num = self.extract_ref(root_obj, b"/Pages")?;
        let pages_obj = self.resolve_object(pages_num)?;

        let page_obj = self.find_first_page_obj(pages_obj, pages_num)?;

        let bbox = self.extract_mediabox(page_obj)
            .or_else(|| self.extract_mediabox(pages_obj))
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);

        let resources = self.extract_resources_bytes(page_obj)
            .or_else(|| self.extract_resources_bytes(pages_obj))
            .unwrap_or_else(|| b"<< >>".to_vec());

        let content_stream = self.extract_content_stream(page_obj)?;

        Some(PdfPageData {
            bbox,
            content_stream,
            resources,
        })
    }

    fn find_root(&self) -> Option<usize> {
        if let Some(pos) = find_bytes(self.data, b"trailer") {
            let trailer = &self.data[pos..];
            if let Some(root) = self.extract_ref(trailer, b"/Root") {
                return Some(root);
            }
        }
        let search_start = self.data.len().saturating_sub(4096);
        let tail = &self.data[search_start..];
        if let Some(pos) = find_bytes(tail, b"/Root") {
            let from = &tail[pos..];
            if let Some(r) = parse_ref_after_key(from, b"/Root") {
                return Some(r);
            }
        }
        None
    }

    fn extract_ref(&self, obj_data: &[u8], key: &[u8]) -> Option<usize> {
        parse_ref_after_key(obj_data, key)
    }

    fn find_first_page_obj(&self, node: &'a [u8], _node_num: usize) -> Option<&'a [u8]> {
        if contains_token(node, b"/Type /Page\n") || contains_token(node, b"/Type /Page/")
            || contains_token(node, b"/Type /Page ") || contains_token(node, b"/Type/Page")
        {
            if !contains_token(node, b"/Type /Pages") && !contains_token(node, b"/Type/Pages") {
                return Some(node);
            }
        }

        let kids_start = find_bytes(node, b"/Kids")?;
        let after_kids = &node[kids_start + 5..];
        let arr_start = find_bytes(after_kids, b"[")?;
        let arr_data = &after_kids[arr_start + 1..];

        let first_ref = parse_next_ref(arr_data)?;
        let child_obj = self.resolve_object(first_ref)?;

        if contains_token(child_obj, b"/Type /Pages") || contains_token(child_obj, b"/Type/Pages") {
            self.find_first_page_obj(child_obj, first_ref)
        } else {
            Some(child_obj)
        }
    }

    fn extract_mediabox(&self, obj_data: &[u8]) -> Option<[f32; 4]> {
        let pos = find_bytes(obj_data, b"/MediaBox")?;
        let after = &obj_data[pos + 9..];
        let arr_start = find_bytes(after, b"[")?;
        let arr_data = &after[arr_start + 1..];
        let arr_end = find_bytes(arr_data, b"]")?;
        let arr_str = std::str::from_utf8(&arr_data[..arr_end]).ok()?;
        let nums: Vec<f32> = arr_str.split_whitespace()
            .filter_map(|s| s.parse::<f32>().ok())
            .collect();
        if nums.len() >= 4 {
            Some([nums[0], nums[1], nums[2], nums[3]])
        } else {
            None
        }
    }

    fn extract_resources_bytes(&self, obj_data: &[u8]) -> Option<Vec<u8>> {
        let pos = find_bytes(obj_data, b"/Resources")?;
        let after = &obj_data[pos + 10..];

        let mut i = 0;
        while i < after.len() && (after[i] == b' ' || after[i] == b'\n' || after[i] == b'\r') {
            i += 1;
        }

        if i >= after.len() { return None; }

        if after[i] == b'<' && i + 1 < after.len() && after[i + 1] == b'<' {
            let dict_end = find_matching_dict_end(after, i)?;
            Some(after[i..dict_end + 2].to_vec())
        } else if after[i].is_ascii_digit() {
            let ref_num = parse_next_ref(&after[i..])?;
            let ref_obj = self.resolve_object(ref_num)?;
            let dict_start = find_bytes(ref_obj, b"<<")?;
            let dict_end = find_matching_dict_end(ref_obj, dict_start)?;
            Some(ref_obj[dict_start..dict_end + 2].to_vec())
        } else {
            None
        }
    }

    fn extract_content_stream(&self, page_obj: &[u8]) -> Option<Vec<u8>> {
        let pos = find_bytes(page_obj, b"/Contents")?;
        let after = &page_obj[pos + 9..];

        let mut i = 0;
        while i < after.len() && (after[i] == b' ' || after[i] == b'\n' || after[i] == b'\r') {
            i += 1;
        }

        if i >= after.len() { return None; }

        if after[i] == b'[' {
            let arr_end = find_bytes(&after[i..], b"]")?;
            let arr_data = &after[i + 1..i + arr_end];
            let mut combined = Vec::new();
            let mut scan = 0;
            while scan < arr_data.len() {
                if arr_data[scan].is_ascii_digit() {
                    if let Some(ref_num) = parse_next_ref(&arr_data[scan..]) {
                        if let Some(stream) = self.extract_stream_from_object(ref_num) {
                            if !combined.is_empty() {
                                combined.push(b'\n');
                            }
                            combined.extend_from_slice(&stream);
                        }
                        while scan < arr_data.len() && arr_data[scan] != b'R' { scan += 1; }
                        if scan < arr_data.len() { scan += 1; }
                        continue;
                    }
                }
                scan += 1;
            }
            if combined.is_empty() { None } else { Some(combined) }
        } else if after[i].is_ascii_digit() {
            let ref_num = parse_next_ref(&after[i..])?;
            self.extract_stream_from_object(ref_num)
        } else {
            None
        }
    }

    fn extract_stream_from_object(&self, obj_num: usize) -> Option<Vec<u8>> {
        let obj_data = self.resolve_object(obj_num)?;
        let dict_start = find_bytes(obj_data, b"<<")?;
        let dict_end = find_matching_dict_end(obj_data, dict_start)?;
        let dict = &obj_data[dict_start..dict_end + 2];
        extract_stream(obj_data, dict_end + 2, dict)
    }
}

fn read_be_uint(bytes: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in bytes {
        val = (val << 8) | b as u64;
    }
    val
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn contains_token(data: &[u8], token: &[u8]) -> bool {
    let search_len = data.len().min(2048);
    data[..search_len].windows(token.len()).any(|w| w == token)
}

fn find_matching_dict_end(data: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i + 1 < data.len() {
        if data[i] == b'<' && data[i + 1] == b'<' {
            depth += 1;
            i += 2;
        } else if data[i] == b'>' && data[i + 1] == b'>' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

fn parse_ref_after_key(data: &[u8], key: &[u8]) -> Option<usize> {
    let pos = find_bytes(data, key)?;
    let after = &data[pos + key.len()..];
    parse_next_ref(after)
}

fn parse_next_ref(data: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < data.len() && (data[i] == b' ' || data[i] == b'\n' || data[i] == b'\r') {
        i += 1;
    }
    let start = i;
    while i < data.len() && data[i].is_ascii_digit() {
        i += 1;
    }
    if i == start { return None; }
    let num = std::str::from_utf8(&data[start..i]).ok()?.parse::<usize>().ok()?;
    while i < data.len() && (data[i] == b' ' || data[i] == b'\n' || data[i] == b'\r') {
        i += 1;
    }
    while i < data.len() && data[i].is_ascii_digit() { i += 1; }
    while i < data.len() && (data[i] == b' ' || data[i] == b'\n' || data[i] == b'\r') {
        i += 1;
    }
    if i < data.len() && data[i] == b'R' {
        Some(num)
    } else {
        None
    }
}

fn parse_next_int(data: &[u8]) -> Option<i64> {
    let mut i = 0;
    while i < data.len() && !data[i].is_ascii_digit() && data[i] != b'-' {
        i += 1;
    }
    let start = i;
    if i < data.len() && data[i] == b'-' { i += 1; }
    while i < data.len() && data[i].is_ascii_digit() {
        i += 1;
    }
    if i == start { return None; }
    std::str::from_utf8(&data[start..i]).ok()?.parse().ok()
}

fn pdf_parse_int_at(data: &[u8], pos: &mut usize) -> Option<i64> {
    while *pos < data.len() && !data[*pos].is_ascii_digit() && data[*pos] != b'-' {
        if data[*pos].is_ascii_alphabetic() { return None; }
        *pos += 1;
    }
    let start = *pos;
    if *pos < data.len() && data[*pos] == b'-' { *pos += 1; }
    while *pos < data.len() && data[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start { return None; }
    std::str::from_utf8(&data[start..*pos]).ok()?.parse().ok()
}

fn pdf_skip_whitespace(data: &[u8], pos: &mut usize) {
    while *pos < data.len() && (data[*pos] == b' ' || data[*pos] == b'\n' || data[*pos] == b'\r' || data[*pos] == b'\t') {
        *pos += 1;
    }
}

fn extract_int_value(dict: &[u8], key: &[u8]) -> Option<i64> {
    let pos = find_bytes(dict, key)?;
    parse_next_int(&dict[pos + key.len()..])
}

fn extract_array(dict: &[u8], key: &[u8]) -> Option<Vec<i64>> {
    let pos = find_bytes(dict, key)?;
    let after = &dict[pos + key.len()..];
    let arr_start = find_bytes(after, b"[")?;
    let arr_end = find_bytes(&after[arr_start..], b"]")?;
    let arr_str = std::str::from_utf8(&after[arr_start + 1..arr_start + arr_end]).ok()?;
    Some(arr_str.split_whitespace()
        .filter_map(|s| s.parse::<i64>().ok())
        .collect())
}

fn extract_stream(obj_data: &[u8], after_dict: usize, dict: &[u8]) -> Option<Vec<u8>> {
    let stream_start_pos = find_bytes(&obj_data[after_dict..], b"stream")?;
    let mut stream_begin = after_dict + stream_start_pos + 6;
    if stream_begin < obj_data.len() && obj_data[stream_begin] == b'\r' { stream_begin += 1; }
    if stream_begin < obj_data.len() && obj_data[stream_begin] == b'\n' { stream_begin += 1; }

    let length = extract_int_value(dict, b"/Length")? as usize;
    if stream_begin + length > obj_data.len() { return None; }

    let raw_stream = &obj_data[stream_begin..stream_begin + length];

    let is_flate = find_bytes(dict, b"/FlateDecode").is_some();

    if is_flate {
        miniz_oxide::inflate::decompress_to_vec_zlib(raw_stream).ok()
    } else {
        Some(raw_stream.to_vec())
    }
}

/// Render TikZ diagram using native Rust renderer
pub(super) fn layout_tikz_diagram(tikz_source: &str, state: &mut LayoutState, _doc: &crate::document::Document) -> Result<()> {
    use crate::tikz_render::{self, TikzElement};

    state.add_vertical_space(state.base_font_size * 1.0);

    if tikz_source.contains("\\begin{axis}") {
        if let Some((plot_elems, total_w, total_h)) = crate::pgfplots::render_pgfplot(tikz_source) {
            return layout_pgfplot_elements(&plot_elems, total_w, total_h, state);
        }
    }

    let result = tikz_render::render_tikz(tikz_source);

    if result.elements.is_empty() {
        let placeholder = "[TikZ diagram]";
        let base = state.base_font_size;
        let box_h = base * 6.0;
        let box_w = state.text_width() * 0.5;
        state.ensure_space(box_h + base * 2.0);
        let x = state.text_left() + (state.text_width() - box_w) / 2.0;
        state.emit_rect(x, state.current_y, box_w, box_h,
            Some(Color::rgb(0.95, 0.95, 0.98)), Some(Color::rgb(0.6, 0.6, 0.8)));
        let tw = font::measure_text(placeholder, FontId::TimesRoman, base);
        state.current_x = x + (box_w - tw) / 2.0;
        state.emit_text(placeholder, base, FontStyle::Italic, Color::GRAY);
        state.current_y += box_h + base * 1.0;
        state.current_x = state.text_left();
        return Ok(());
    }

    let available_w = state.text_width() * 0.9;
    let scale = (available_w / result.width).min(2.0);
    let scaled_h = result.height * scale;
    state.ensure_space(scaled_h + state.base_font_size * 2.0);

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
                let arr_len = *width * 3.5 + 3.5;
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

    state.current_y = base_y + scaled_h + state.base_font_size * 1.0;
    state.current_x = state.text_left();
    state.add_vertical_space(state.base_font_size * 1.0);
    Ok(())
}

/// Render pre-computed diagram elements (from diagrams.rs) into layout
pub(super) fn layout_diagram_elements(result: &crate::tikz_render::TikzRenderResult, state: &mut LayoutState) -> Result<()> {
    use crate::tikz_render::TikzElement;

    state.add_vertical_space(10.0);

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
    state.ensure_space(scaled_h + state.base_font_size * 2.0);

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
                let tw = font::measure_text(text, FontId::TimesRoman, fs);
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

    state.current_y = base_y + scaled_h + state.base_font_size * 1.0;
    state.current_x = state.text_left();
    state.add_vertical_space(state.base_font_size * 1.0);
    Ok(())
}
