/// Layout engine: converts document AST to positioned page elements
/// Direct layout without intermediate format for maximum speed

use anyhow::Result;
use std::collections::HashMap;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle, wrap_text};
use crate::math_layout;
use crate::font::{self, FontId};

/// Pre-scan AST to collect label→display-number mappings for \ref resolution.
/// This avoids a full two-pass layout while still resolving cross-references.
fn collect_labels(nodes: &[Node]) -> (HashMap<String, String>, HashMap<String, u32>) {
    let mut labels = HashMap::new();
    let mut citations = HashMap::new();
    let mut fig_counter = 0u32;
    let mut tbl_counter = 0u32;
    let mut bib_counter = 0u32;
    let mut sec_counters = [0u32; 7];
    collect_labels_inner(nodes, &mut labels, &mut citations, &mut fig_counter, &mut tbl_counter, &mut bib_counter, &mut sec_counters);
    (labels, citations)
}

fn collect_labels_inner(
    nodes: &[Node],
    labels: &mut HashMap<String, String>,
    citations: &mut HashMap<String, u32>,
    fig_counter: &mut u32,
    tbl_counter: &mut u32,
    bib_counter: &mut u32,
    sec_counters: &mut [u32; 7],
) {
    for node in nodes {
        match node {
            Node::Section { level, numbered, .. } => {
                if *numbered {
                    let idx = (level.depth() + 1).max(0) as usize;
                    if idx < sec_counters.len() {
                        sec_counters[idx] += 1;
                        for i in (idx + 1)..sec_counters.len() {
                            sec_counters[i] = 0;
                        }
                    }
                }
            }
            Node::Figure(fig) => {
                if fig.caption.is_some() {
                    *fig_counter += 1;
                    if let Some(ref lbl) = fig.label {
                        labels.insert(lbl.clone(), fig_counter.to_string());
                    }
                }
                collect_labels_inner(&fig.content, labels, citations, fig_counter, tbl_counter, bib_counter, sec_counters);
            }
            Node::Table(table) => {
                if table.caption.is_some() {
                    *tbl_counter += 1;
                    if let Some(ref lbl) = table.label {
                        labels.insert(lbl.clone(), tbl_counter.to_string());
                    }
                }
            }
            Node::BibItem(key) => {
                *bib_counter += 1;
                citations.insert(key.clone(), *bib_counter);
            }
            Node::Label(name) => {
                // Section label: use current section number
                let sec_num = if sec_counters[4] > 0 {
                    format!("{}.{}.{}", sec_counters[2], sec_counters[3], sec_counters[4])
                } else if sec_counters[3] > 0 {
                    format!("{}.{}", sec_counters[2], sec_counters[3])
                } else if sec_counters[2] > 0 {
                    format!("{}", sec_counters[2])
                } else {
                    "??".to_string()
                };
                labels.insert(name.clone(), sec_num);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) => {
                for item in items {
                    collect_labels_inner(&item.content, labels, citations, fig_counter, tbl_counter, bib_counter, sec_counters);
                }
            }
            Node::Environment(env) => {
                collect_labels_inner(&env.content, labels, citations, fig_counter, tbl_counter, bib_counter, sec_counters);
            }
            Node::Quote(c) | Node::Quotation(c) | Node::Abstract(c) | Node::Center(c)
            | Node::FlushLeft(c) | Node::FlushRight(c) | Node::Bold(c) | Node::Italic(c)
            | Node::Group(c) => {
                collect_labels_inner(c, labels, citations, fig_counter, tbl_counter, bib_counter, sec_counters);
            }
            _ => {}
        }
    }
}

/// Flat storage for all laid-out pages - eliminates per-page allocations
#[derive(Debug)]
pub struct LayoutResult {
    pub all_elements: Vec<PageElement>,
    pub all_text: String,
    pub rect_data: Vec<RectData>,
    pub page_bounds: Vec<PageBounds>,
    pub width: f32,
    pub height: f32,
}

/// Boundary indices for a single page within the flat storage
#[derive(Debug, Clone, Copy)]
pub struct PageBounds {
    pub elem_start: u32,
    pub elem_end: u32,
    pub text_start: u32,
    pub text_end: u32,
}

impl LayoutResult {
    #[inline]
    pub fn page_elements(&self, page: usize) -> &[PageElement] {
        let b = &self.page_bounds[page];
        &self.all_elements[b.elem_start as usize..b.elem_end as usize]
    }

    #[inline]
    pub fn page_text(&self, page: usize) -> &str {
        let b = &self.page_bounds[page];
        &self.all_text[b.text_start as usize..b.text_end as usize]
    }

    #[inline]
    pub fn num_pages(&self) -> usize {
        self.page_bounds.len()
    }

    /// Get text from a page's text buffer given per-page offset and length
    #[inline]
    pub fn get_page_text(&self, page: usize, offset: u32, len: u32) -> &str {
        let b = &self.page_bounds[page];
        let start = b.text_start as usize + offset as usize;
        &self.all_text[start..start + len as usize]
    }
}


/// A positioned element on a page
/// Rect data stored separately (boxed) since it's rare but large
#[derive(Debug, Clone)]
pub struct RectData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub stroke_width: f32,
    pub corner_radius: f32,
}

#[derive(Debug, Clone)]
pub enum PageElement {
    Text {
        x: f32,
        y: f32,
        text_offset: u32,
        text_len: u16,       // max 65535 chars per text span
        font_size_100: u16,  // font_size * 100, max 655.35pt
        font_style: FontStyle,
        color: Color,
        word_spacing_50: i16, // word spacing * 50, for justified text. 0 = default
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width_1000: u16,     // width * 1000, max 65.535pt
        color: Color,
    },
    Rect(u32), // index into LayoutResult.rect_data
}

struct LayoutState {
    page_setup: PageSetup,
    base_font_size: f32,
    current_x: f32,
    current_y: f32,
    cached_max_y: f32,    // cached: height - margin_bottom - footer_height
    cached_start_y: f32,  // cached: margin_top + header_height
    cached_text_width: f32, // cached: page_setup.text_width() - indent
    cached_text_left: f32,  // cached: margin_left + indent
    current_font_size: f32,
    current_font_style: FontStyle,
    current_color: Color,
    // Cached word-wrap parameters (invalidated when font changes)
    cached_avg_width: f32,
    cached_line_height: f32,
    cached_step: f32,
    cached_font_size_100: u16,
    cached_max_chars: usize,
    cached_font_key: u32,     // hash of font_size+style for invalidation
    // Flat storage - single allocation for all pages
    all_elements: Vec<PageElement>,
    all_text: String,
    rect_data: Vec<RectData>,
    page_bounds: Vec<PageBounds>,
    // Current page boundary tracking
    current_page_elem_start: u32,
    current_page_text_start: u32,
    page_number: u32,
    indent: f32,
    paragraph_indent: f32,
    line_spacing: f32,
    section_counters: [u32; 7],
    figure_counter: u32,
    table_counter: u32,
    footnotes: Vec<Vec<Node>>,
    footnote_counter: u32,
    suppress_next_indent: bool,
    text_buf: String,
    label_map: HashMap<String, String>,
    citation_map: HashMap<String, u32>,
}

impl LayoutState {
    fn new(page_setup: PageSetup, font_size: f32, line_spacing: f32) -> Self {
        let max_y = page_setup.height - page_setup.margin_bottom - page_setup.footer_height;
        let start_y = page_setup.margin_top + page_setup.header_height;
        let text_w = page_setup.text_width();
        let avg_w = font_size * 0.48; // Regular
        let lh = font_size * 1.2;
        let st = lh * line_spacing;
        let para_w = text_w - 20.0; // paragraph_indent = 20.0
        let max_chars = (para_w / avg_w) as usize;
        let font_key = (font_size.to_bits() & 0xFFFF0000) | (FontStyle::Regular as u32);
        LayoutState {
            page_setup,
            base_font_size: font_size,
            current_x: page_setup.margin_left,
            current_y: start_y,
            cached_max_y: max_y,
            cached_start_y: start_y,
            cached_text_width: text_w,
            cached_text_left: page_setup.margin_left,
            current_font_size: font_size,
            current_font_style: FontStyle::Regular,
            current_color: Color::BLACK,
            cached_avg_width: avg_w,
            cached_line_height: lh,
            cached_step: st,
            cached_font_size_100: (font_size * 100.0) as u16,
            cached_max_chars: max_chars,
            cached_font_key: font_key,
            // Pre-allocate flat storage for ~50K pages
            all_elements: Vec::with_capacity(1_600_000), // ~30 elements per page * 51K pages
            all_text: String::with_capacity(8 * 1024 * 1024), // ~8MB for text
            rect_data: Vec::with_capacity(64_000), // rects are rare
            page_bounds: Vec::with_capacity(51000),
            current_page_elem_start: 0,
            current_page_text_start: 0,
            page_number: 1,
            indent: 0.0,
            paragraph_indent: 20.0,
            line_spacing,
            section_counters: [0; 7],
            figure_counter: 0,
            table_counter: 0,
            footnotes: Vec::new(),
            footnote_counter: 0,
            suppress_next_indent: false,
            text_buf: String::with_capacity(4096),
            label_map: HashMap::new(),
            citation_map: HashMap::new(),
        }
    }

    #[inline(always)]
    fn text_width(&self) -> f32 {
        self.cached_text_width
    }

    #[inline(always)]
    fn text_left(&self) -> f32 {
        self.cached_text_left
    }

    /// Update cached word-wrap parameters if font changed. Returns (avg_width, line_height, step, font_size_100, max_chars)
    #[inline(always)]
    fn wrap_params(&mut self) -> (f32, f32, f32, u16, usize) {
        let key = (self.current_font_size.to_bits() & 0xFFFF0000) | (self.current_font_style as u32);
        if key != self.cached_font_key {
            let fs = self.current_font_size;
            self.cached_avg_width = fs * match self.current_font_style {
                FontStyle::Monospace => 0.6,
                FontStyle::Bold | FontStyle::BoldItalic => 0.52,
                _ => 0.48,
            };
            self.cached_line_height = fs * 1.2;
            self.cached_step = self.cached_line_height * self.line_spacing;
            self.cached_font_size_100 = (fs * 100.0) as u16;
            let para_width = self.cached_text_width - self.paragraph_indent;
            self.cached_max_chars = (para_width / self.cached_avg_width) as usize;
            self.cached_font_key = key;
        }
        (self.cached_avg_width, self.cached_line_height, self.cached_step, self.cached_font_size_100, self.cached_max_chars)
    }

    #[inline(always)]
    fn set_indent(&mut self, indent: f32) {
        self.indent = indent;
        self.cached_text_width = self.page_setup.text_width() - indent;
        self.cached_text_left = self.page_setup.margin_left + indent;
        // Invalidate wrap cache since max_chars depends on text_width
        self.cached_font_key = u32::MAX;
    }

    #[inline(always)]
    fn max_y(&self) -> f32 {
        self.cached_max_y
    }

    fn metrics(&self) -> FontMetrics {
        FontMetrics::new(self.current_font_size, self.current_font_style)
    }

    fn new_page(&mut self) {
        // Add page number to current page before finalizing
        let digit_width = 9.0 * 0.5;
        let center_x = self.page_setup.width / 2.0;
        let y = self.page_setup.height - self.page_setup.margin_bottom + 10.0;
        let mut num_buf = [0u8; 8];
        let n = self.page_number;
        let mut pos = 8;
        let mut v = n;
        loop {
            pos -= 1;
            num_buf[pos] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 { break; }
        }
        let num_str = unsafe { std::str::from_utf8_unchecked(&num_buf[pos..8]) };
        let num_len = 8 - pos;
        let text_width = num_len as f32 * digit_width;
        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
        self.all_text.push_str(num_str);
        self.all_elements.push(PageElement::Text {
            x: center_x - text_width / 2.0,
            y,
            text_offset: offset,
            text_len: num_len as u16,
            font_size_100: 900, // 9.0 * 100
            font_style: FontStyle::Regular,
            color: Color::GRAY,
            word_spacing_50: 0,
        });

        // Record page boundary (NO ALLOCATION!)
        self.page_bounds.push(PageBounds {
            elem_start: self.current_page_elem_start,
            elem_end: self.all_elements.len() as u32,
            text_start: self.current_page_text_start,
            text_end: self.all_text.len() as u32,
        });
        // Start new page
        self.current_page_elem_start = self.all_elements.len() as u32;
        self.current_page_text_start = self.all_text.len() as u32;
        self.page_number += 1;
        self.current_x = self.text_left();
        self.current_y = self.cached_start_y;
    }

    #[inline(always)]
    fn ensure_space(&mut self, height: f32) {
        if self.current_y + height > self.cached_max_y {
            self.new_page();
        }
    }

    #[inline(always)]
    fn add_vertical_space(&mut self, space: f32) {
        self.current_y += space;
        if self.current_y > self.cached_max_y {
            self.new_page();
        }
    }

    #[inline(always)]
    fn emit_text(&mut self, text: &str, font_size: f32, style: FontStyle, color: Color) {
        if text.is_empty() {
            return;
        }
        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
        self.all_text.push_str(text);
        self.all_elements.push(PageElement::Text {
            x: self.current_x,
            y: self.current_y,
            text_offset: offset,
            text_len: text.len().min(65535) as u16,
            font_size_100: (font_size * 100.0) as u16,
            font_style: style,
            color,
            word_spacing_50: 0,
        });
    }

    #[inline]
    fn emit_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.all_elements.push(PageElement::Line {
            x1, y1, x2, y2, width_1000: (width * 1000.0) as u16, color,
        });
    }

    #[inline]
    fn emit_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<Color>) {
        let idx = self.rect_data.len() as u32;
        self.rect_data.push(RectData {
            x, y, width: w, height: h, fill, stroke, stroke_width: 0.5, corner_radius: 0.0,
        });
        self.all_elements.push(PageElement::Rect(idx));
    }

    fn emit_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<Color>, corner_radius: f32) {
        let idx = self.rect_data.len() as u32;
        self.rect_data.push(RectData {
            x, y, width: w, height: h, fill, stroke, stroke_width: 0.5, corner_radius,
        });
        self.all_elements.push(PageElement::Rect(idx));
    }
}

pub fn layout_document(doc: &Document, source: &str) -> Result<LayoutResult> {
    let mut state = LayoutState::new(
        doc.preamble.page_setup,
        doc.preamble.font_size,
        doc.preamble.line_spacing,
    );

    // Check for font size in class options
    for opt in &doc.class.options {
        match opt.as_str() {
            "10pt" => { state.base_font_size = 10.0; state.current_font_size = 10.0; }
            "11pt" => { state.base_font_size = 11.0; state.current_font_size = 11.0; }
            "12pt" => { state.base_font_size = 12.0; state.current_font_size = 12.0; }
            "twocolumn" => { state.page_setup.columns = 2; }
            "a4paper" => {
                state.page_setup.width = 595.276;
                state.page_setup.height = 841.890;
            }
            "letterpaper" => {
                state.page_setup.width = 612.0;
                state.page_setup.height = 792.0;
            }
            _ => {}
        }
    }

    // Pre-scan for label→number mappings (fast O(n) AST walk)
    let (labels, citations) = collect_labels(&doc.body);
    state.label_map = labels;
    state.citation_map = citations;

    // Layout body
    layout_nodes(&doc.body, &mut state, doc, source)?;

    // Finalize last page
    if state.all_elements.len() as u32 > state.current_page_elem_start {
        state.new_page();
    }

    // Ensure at least one page
    if state.page_bounds.is_empty() {
        state.page_bounds.push(PageBounds {
            elem_start: 0,
            elem_end: 0,
            text_start: 0,
            text_end: 0,
        });
    }

    Ok(LayoutResult {
        all_elements: state.all_elements,
        all_text: state.all_text,
        rect_data: state.rect_data,
        page_bounds: state.page_bounds,
        width: state.page_setup.width,
        height: state.page_setup.height,
    })
}

/// High bit flag indicating text_offset refers to source (mmap) rather than page text_buffer
pub const SOURCE_REF_FLAG: u32 = 0x80000000;

fn layout_nodes(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    let source_base = source.as_ptr() as usize;
    // Pre-compute wrap params once for the common case (same font across TextParagraphs)
    let (_, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let mut line_height = line_height;
    let mut step = step;
    let mut font_size_100 = font_size_100;
    let mut max_chars_single = max_chars_single;
    let mut font_style = state.current_font_style;
    let mut color = state.current_color;
    let mut font_key = state.cached_font_key;

    for node in nodes {
        match node {
            Node::TextParagraph(offset, len) => {
                let start = *offset as usize;
                let end = start + *len as usize;
                let raw = &source[start..end];
                let bytes = raw.as_bytes();
                let text = if !bytes.is_empty() && bytes[0] > b' ' && bytes[bytes.len()-1] > b' ' {
                    raw
                } else {
                    raw.trim()
                };
                if text.is_empty() { continue; }
                let src_off = (text.as_ptr() as usize - source_base) as u32;

                // Refresh cached values if font changed
                let cur_key = state.cached_font_key;
                if cur_key != font_key {
                    let params = state.wrap_params();
                    line_height = params.1;
                    step = params.2;
                    font_size_100 = params.3;
                    max_chars_single = params.4;
                    font_style = state.current_font_style;
                    color = state.current_color;
                    font_key = cur_key;
                }

                // Inline single-line fast path (most TextParagraphs)
                if text.len() <= max_chars_single {
                    state.ensure_space(line_height);
                    let pi = if state.suppress_next_indent { state.suppress_next_indent = false; 0.0 } else { state.paragraph_indent };
                    let x = state.text_left() + pi;
                    state.all_elements.push(PageElement::Text {
                        x,
                        y: state.current_y,
                        text_offset: src_off | SOURCE_REF_FLAG,
                        text_len: text.len() as u16,
                        font_size_100,
                        font_style,
                        color,
                        word_spacing_50: 0,
                    });
                    state.current_y += step;
                    state.current_x = state.text_left();
                    state.add_vertical_space(state.current_font_size * 0.2);
                } else {
                    layout_text_content_source(text, state, src_off)?;
                }
            }
            Node::Paragraph(children) => {
                layout_paragraph(children, state, doc, source)?;
            }
            Node::Section { level, title, numbered } => {
                layout_section(*level, title, *numbered, state, doc, source)?;
            }
            _ => layout_node(node, state, doc, source)?,
        }
    }
    Ok(())
}




fn layout_node(node: &Node, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    match node {
        Node::Paragraph(children) => {
            layout_paragraph(children, state, doc, source)?;
        }

        Node::TextParagraph(offset, len) => {
            // Fast path: extract text directly from source, no intermediate Node
            let start = *offset as usize;
            let end = start + *len as usize;
            let raw = &source[start..end];
            let bytes = raw.as_bytes();
            // Fast trim: check first/last bytes before full scan
            let text = if !bytes.is_empty() && bytes[0] > b' ' && bytes[bytes.len()-1] > b' ' {
                raw
            } else {
                raw.trim()
            };
            if !text.is_empty() {
                layout_text_content(text, state)?;
            }
        }

        Node::Section { level, title, numbered } => {
            layout_section(*level, title, *numbered, state, doc, source)?;
        }

        Node::MakeTitle => {
            layout_title(state, doc)?;
        }

        Node::TableOfContents => {
            // Simple "Contents" heading
            state.add_vertical_space(10.0);
            state.emit_text("Contents", state.base_font_size * 1.44, FontStyle::Bold, Color::BLACK);
            state.current_y += state.base_font_size * 1.44 * 1.2 + 10.0;
            state.emit_line(
                state.text_left(),
                state.current_y,
                state.text_left() + state.text_width(),
                state.current_y,
                0.5,
                Color::BLACK,
            );
            state.current_y += 10.0;
        }

        Node::ItemizeList(items) => {
            layout_list(items, state, doc, false, source)?;
        }

        Node::EnumerateList(items) => {
            layout_list(items, state, doc, true, source)?;
        }

        Node::DescriptionList(items) => {
            layout_description_list(items, state, doc, source)?;
        }

        Node::Table(table) => {
            layout_table(table, state, doc, source)?;
        }

        Node::Figure(fig) => {
            state.add_vertical_space(10.0);
            let saved_indent = state.indent;
            let saved_font_size = state.current_font_size;
            layout_nodes(&fig.content, state, doc, source)?;
            if let Some(cap) = &fig.caption {
                state.figure_counter += 1;
                let fig_num = state.figure_counter;
                state.current_y += 6.0;
                state.current_x = state.text_left();
                // "Figure N: caption text" as single string for correct spacing
                state.text_buf.clear();
                state.text_buf.push_str("Figure ");
                let mut ibuf = itoa::Buffer::new();
                state.text_buf.push_str(ibuf.format(fig_num));
                state.text_buf.push_str(": ");
                let prefix_len = state.text_buf.len();

                for node in cap {
                    node_to_text(node, &mut state.text_buf, source);
                }
                // Emit bold prefix using accurate AFM metrics
                let prefix: &str = unsafe { &*(state.text_buf[..prefix_len].as_ref() as *const str) };
                let prefix_width = font::measure_text(prefix, FontId::HelveticaBold, state.current_font_size);
                state.emit_text(prefix, state.current_font_size, FontStyle::Bold, Color::BLACK);
                state.current_x += prefix_width;

                let cap_text: &str = unsafe { &*(state.text_buf[prefix_len..].as_ref() as *const str) };
                if !cap_text.is_empty() {
                    layout_text_line(cap_text, state);
                }
                state.current_y += state.current_font_size * 1.2;
            }
            state.set_indent(saved_indent);
            state.current_font_size = saved_font_size;
            state.current_x = state.text_left();
            state.add_vertical_space(10.0);
        }

        Node::Image(img) => {
            // Use actual image dimensions if available, or specified dimensions
            let img_w = img.width.unwrap_or(200.0);
            let img_h = img.height.unwrap_or(150.0);

            // Apply scale if specified
            let (img_w, img_h) = if let Some(scale) = img.scale {
                (img_w * scale, img_h * scale)
            } else {
                (img_w, img_h)
            };

            // Cap width to text width
            let (img_w, img_h) = if img_w > state.text_width() {
                let scale = state.text_width() / img_w;
                (state.text_width(), img_h * scale)
            } else {
                (img_w, img_h)
            };

            state.ensure_space(img_h + 10.0);

            // Draw placeholder rectangle with image path
            let x = state.text_left() + (state.text_width() - img_w) / 2.0;
            state.emit_rect(x, state.current_y, img_w, img_h,
                Some(Color::rgb(0.95, 0.95, 0.95)), Some(Color::LIGHT_GRAY));

            // Show filename centered in the image area
            let label = format!("[Image: {}]", img.path);
            let tw = font::measure_text(&label, FontId::Helvetica, 8.0);
            let cx = x + (img_w - tw) / 2.0;
            state.current_x = cx;
            state.emit_text(&label, 8.0, FontStyle::Italic, Color::GRAY);
            state.current_y += img_h + 6.0;
            state.current_x = state.text_left();
        }

        Node::HRule => {
            state.add_vertical_space(6.0);
            state.emit_line(
                state.text_left(),
                state.current_y,
                state.text_left() + state.text_width(),
                state.current_y,
                0.5,
                Color::BLACK,
            );
            state.current_y += 6.0;
        }

        Node::VSpace(pts) => {
            state.add_vertical_space(*pts);
        }

        Node::PageBreak => {
            state.new_page();
        }

        Node::DisplayMath(math_nodes) => {
            layout_display_math(math_nodes, state)?;
        }

        Node::Quote(content) | Node::Quotation(content) => {
            let saved_indent = state.indent;
            state.set_indent(state.indent + 30.0);
            state.current_x = state.text_left();
            state.add_vertical_space(6.0);
            layout_nodes(content, state, doc, source)?;
            state.add_vertical_space(6.0);
            state.set_indent(saved_indent);
            state.current_x = state.text_left();
        }

        Node::Abstract(content) => {
            state.add_vertical_space(10.0);
            // Center "Abstract" title
            let title = "Abstract";
            let metrics = FontMetrics::new(state.base_font_size * 1.1, FontStyle::Bold);
            let tw = metrics.measure_text(title);
            let cx = state.text_left() + (state.text_width() - tw) / 2.0;
            state.current_x = cx;
            state.emit_text(title, state.base_font_size * 1.1, FontStyle::Bold, Color::BLACK);
            state.current_y += metrics.line_height() + 4.0;

            let saved_indent = state.indent;
            state.set_indent(state.indent + 30.0);
            state.current_x = state.text_left();
            let saved_size = state.current_font_size;
            state.current_font_size = state.base_font_size * 0.9;
            layout_nodes(content, state, doc, source)?;
            state.current_font_size = saved_size;
            state.set_indent(saved_indent);
            state.current_x = state.text_left();
            state.add_vertical_space(10.0);
        }

        Node::Center(content) => {
            // For centered content, we layout normally but center each text element
            layout_centered(content, state, doc, source)?;
        }

        Node::FlushLeft(content) => {
            layout_nodes(content, state, doc, source)?;
        }

        Node::FlushRight(content) => {
            // Simplified: just layout normally
            layout_nodes(content, state, doc, source)?;
        }

        Node::Verbatim(text) => {
            if text.starts_with("%%tikz:") {
                // TikZ diagram — render natively
                if let Some(end) = text.find("%%\n") {
                    let tikz_source = &text[end + 3..];
                    layout_tikz_diagram(tikz_source, state, doc)?;
                } else {
                    layout_verbatim(text, state)?;
                }
            } else if text.starts_with("%%lang:") {
                // Code block with language
                if let Some(end) = text.find("%%\n") {
                    let lang = &text[7..end];
                    let code = &text[end + 3..];
                    layout_code_block(code, Some(lang), state)?;
                } else {
                    layout_verbatim(text, state)?;
                }
            } else {
                layout_verbatim(text, state)?;
            }
        }

        Node::Environment(env) => {
            if env.name == "thebibliography" {
                layout_bibliography(&env.content, state, doc, source)?;
            } else {
                layout_nodes(&env.content, state, doc, source)?;
            }
        }

        Node::Minipage { width, content } => {
            let saved_indent = state.indent;
            layout_nodes(content, state, doc, source)?;
            state.set_indent(saved_indent);
        }

        // Font size declarations (e.g. \small, \large) with empty content
        // act as switches that change state for subsequent content
        Node::FontSize { size, content } if content.is_empty() => {
            state.current_font_size = size.to_points(doc.preamble.font_size);
        }

        // These are inline elements that shouldn't appear at top level,
        // but handle gracefully
        Node::Text(_) | Node::TextRef(_, _) | Node::Bold(_) | Node::Italic(_) | Node::Monospace(_)
        | Node::SmallCaps(_) | Node::Underline(_) | Node::Emph(_)
        | Node::InlineMath(_) | Node::Group(_) | Node::Colored { .. }
        | Node::FontSize { .. } | Node::Superscript(_) | Node::Subscript(_)
        | Node::NonBreakingSpace | Node::HSpace(_) | Node::LineBreak
        | Node::Footnote(_) | Node::Code(_) => {
            // Wrap in paragraph
            layout_paragraph(&[node.clone()], state, doc, source)?;
        }

        Node::Citation(key) => {
            // Resolve citation using pre-scanned citation map
            let cite_text = if let Some(&num) = state.citation_map.get(key) {
                format!("[{}]", num)
            } else {
                format!("[{}]", key)
            };
            state.emit_text(&cite_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&cite_text, FontId::Helvetica, state.current_font_size);
        }

        Node::Ref(label) => {
            // Resolve reference using pre-scanned label map
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                resolved.clone()
            } else {
                "??".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::Helvetica, state.current_font_size);
        }

        // Skip these (BibItem handled inside layout_bibliography)
        Node::Label(_) | Node::Raw(_) | Node::BibItem(_) => {}

        // Special characters at top level
        Node::EnDash | Node::EmDash | Node::Ellipsis
        | Node::Copyright | Node::Registered | Node::Trademark
        | Node::LeftQuote | Node::RightQuote
        | Node::LeftDoubleQuote | Node::RightDoubleQuote
        | Node::Ampersand | Node::Percent | Node::Dollar
        | Node::Hash | Node::Underscore | Node::Backslash
        | Node::Tilde | Node::Caret | Node::LeftBrace | Node::RightBrace
        | Node::Strikethrough(_) => {}
    }

    Ok(())
}

fn layout_title(state: &mut LayoutState, doc: &Document) -> Result<()> {
    state.add_vertical_space(40.0);

    if let Some(title) = &doc.preamble.title {
        let size = state.base_font_size * 1.728;
        let metrics = FontMetrics::new(size, FontStyle::Bold);

        // Split on \\ (literal double backslash) for line breaks in title
        let segments: Vec<&str> = title.split("\\\\").collect();
        for segment in &segments {
            let segment = segment.trim();
            if segment.is_empty() { continue; }
            let lines = wrap_text(segment, &metrics, state.text_width());
            for line in &lines {
                let tw = metrics.measure_text(line);
                let cx = state.text_left() + (state.text_width() - tw) / 2.0;
                state.ensure_space(metrics.line_height());
                state.current_x = cx;
                state.emit_text(line, size, FontStyle::Bold, Color::BLACK);
                state.current_y += metrics.line_height();
            }
        }

        state.add_vertical_space(12.0);
    }

    if let Some(author) = &doc.preamble.author {
        let size = state.base_font_size * 1.2;
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        let tw = metrics.measure_text(author);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.ensure_space(metrics.line_height());
        state.current_x = cx;
        state.emit_text(author, size, FontStyle::Regular, Color::BLACK);
        state.current_y += metrics.line_height();
        state.add_vertical_space(6.0);
    }

    if let Some(date) = &doc.preamble.date {
        let size = state.base_font_size;
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        let tw = metrics.measure_text(date);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.ensure_space(metrics.line_height());
        state.current_x = cx;
        state.emit_text(date, size, FontStyle::Regular, Color::DARK_GRAY);
        state.current_y += metrics.line_height();
    }

    state.add_vertical_space(24.0);

    // Horizontal rule after title
    state.emit_line(
        state.text_left() + state.text_width() * 0.2,
        state.current_y,
        state.text_left() + state.text_width() * 0.8,
        state.current_y,
        0.5,
        Color::GRAY,
    );
    state.add_vertical_space(20.0);

    Ok(())
}

fn layout_section(
    level: SectionLevel,
    title: &[Node],
    numbered: bool,
    state: &mut LayoutState,
    _doc: &Document,
    source: &str,
) -> Result<()> {
    // Add spacing before
    state.add_vertical_space(level.spacing_before());

    let font_size = level.font_size(state.base_font_size);
    let style = FontStyle::Bold;
    let line_height = font_size * 1.2;

    state.ensure_space(line_height);

    // Build full title text into reusable buffer (avoids 3 separate String allocations)
    state.text_buf.clear();

    if numbered {
        let idx = (level.depth() + 1).max(0) as usize;
        if idx < state.section_counters.len() {
            state.section_counters[idx] += 1;
            for i in (idx + 1)..state.section_counters.len() {
                state.section_counters[i] = 0;
            }
        }

        // Fast integer formatting (avoids write!/format! overhead for 50K sections)
        let mut ibuf = itoa::Buffer::new();
        match level {
            SectionLevel::Part => {
                state.text_buf.push_str("Part ");
                state.text_buf.push_str(ibuf.format(state.section_counters[0]));
                state.text_buf.push(' ');
            }
            SectionLevel::Chapter => {
                state.text_buf.push_str(ibuf.format(state.section_counters[1]));
                state.text_buf.push_str("  ");
            }
            SectionLevel::Section => {
                state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                state.text_buf.push_str("  ");
            }
            SectionLevel::Subsection => {
                state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                state.text_buf.push_str("  ");
            }
            SectionLevel::Subsubsection => {
                state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[4]));
                state.text_buf.push_str("  ");
            }
            _ => {}
        }
    }

    // Append title text directly to buffer
    for node in title {
        node_to_text(node, &mut state.text_buf, source);
    }

    state.current_x = state.text_left();

    // For short titles (most common), skip wrap_text and emit directly
    // SAFETY: text_buf not modified during emit_text (emit_text uses all_text)
    let full_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
    let avg_width = font_size * 0.52; // bold
    let estimated_width = full_text.len() as f32 * avg_width;

    if estimated_width <= state.text_width() {
        // Single line - emit directly without wrap_text allocation
        state.emit_text(full_text, font_size, style, Color::BLACK);
        state.current_y += line_height;
        state.current_x = state.text_left();
    } else {
        // Multi-line - use wrap_text
        let metrics = FontMetrics::new(font_size, style);
        let lines = wrap_text(full_text, &metrics, state.text_width());
        for line in &lines {
            state.emit_text(line, font_size, style, Color::BLACK);
            state.current_y += line_height;
            state.current_x = state.text_left();
        }
    }

    // Rule under section for top-level
    if level == SectionLevel::Section || level == SectionLevel::Chapter {
        state.current_y += 2.0;
        state.emit_line(
            state.text_left(),
            state.current_y,
            state.text_left() + state.text_width(),
            state.current_y,
            0.3,
            Color::LIGHT_GRAY,
        );
        state.current_y += 2.0;
    }

    state.add_vertical_space(level.spacing_after());
    state.current_x = state.text_left();
    state.suppress_next_indent = true;

    Ok(())
}

fn layout_paragraph(children: &[Node], state: &mut LayoutState, _doc: &Document, source: &str) -> Result<()> {
    let with_indent = if state.suppress_next_indent { state.suppress_next_indent = false; false } else { true };
    layout_rich_paragraph(children, state, source, with_indent)
}

/// Calculate word spacing for justified text.
/// Returns word_spacing_50 (word_spacing * 50, as i16) for a line.
/// Returns 0 for the last line of a paragraph.
#[inline]
/// Compute word spacing for justified text using avg char width estimate.
#[inline]
fn justify_line(line: &[u8], available_width: f32, avg_width: f32, font_size: f32, is_last_line: bool) -> i16 {
    if is_last_line { return 0; }
    let num_spaces = memchr::memchr_iter(b' ', line).count();
    if num_spaces == 0 { return 0; }
    let natural_width = line.len() as f32 * avg_width;
    let extra = available_width - natural_width;
    if extra <= 0.0 || extra > font_size * 2.0 { return 0; }
    let ws = extra / num_spaces as f32;
    (ws * 50.0).min(i16::MAX as f32) as i16
}

/// Core word-wrapping and text layout. Separate function to keep code in one place
/// and avoid icache bloat from inlining into the large layout_node match.
fn layout_text_content(text: &str, state: &mut LayoutState) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let pi = if state.suppress_next_indent { state.suppress_next_indent = false; 0.0 } else { state.paragraph_indent };
    let para_width = state.text_width() - pi;
    let full_text_width = state.text_width();

    state.ensure_space(line_height);
    if text.len() <= max_chars_single {
        // Single line - emit directly
        state.current_x = state.text_left() + pi;
        state.emit_text(text, font_size, font_style, color);
        state.current_y += step;
    } else {
        // Multi-line: pre-push entire text, then create PageElements with offsets
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;
        while pos < len && bytes[pos] <= b' ' { pos += 1; }

        // Pre-push text to buffer (one memcpy instead of per-line copies)
        let mut push_start: usize = 0;
        let mut buf_push_pos = state.all_text.len() - state.current_page_text_start as usize;
        state.all_text.push_str(text);

        let x_first = state.text_left() + pi;
        let x_rest = state.text_left();
        let max_chars_first = ((para_width - pi) / avg_width) as usize;
        let max_chars_rest = max_chars_single;

        // Integer-based page tracking: avoids float comparison per line
        // Original check: current_y + line_height <= cached_max_y
        // Lines allowed: floor((max_y - current_y - line_height) / step) + 1
        let mut lines_until_break = ((state.cached_max_y - state.current_y - line_height) / step) as i32 + 1;

        // === Handle first line separately (eliminates branch in main loop) ===
        if pos < len {
            let line_start = pos;
            let target = (pos + max_chars_first).min(len);

            let (mut line_end, next_pos) = if target >= len {
                (len, len)
            } else {
                match memchr::memrchr2(b' ', b'\n', &bytes[line_start..target]) {
                    Some(offset) => (line_start + offset, line_start + offset + 1),
                    None => match memchr::memchr2(b' ', b'\n', &bytes[target..]) {
                        Some(offset) => (target + offset, target + offset + 1),
                        None => (len, len),
                    }
                }
            };

            while line_end > line_start && bytes[line_end - 1] <= b' ' { line_end -= 1; }

            if line_end > line_start {
                if lines_until_break <= 0 {
                    state.new_page();
                    push_start = line_start;
                    buf_push_pos = 0;
                    state.all_text.push_str(&text[line_start..]);
                    lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
                }
                let is_last = next_pos >= len;
                let ws = justify_line(&bytes[line_start..line_end], para_width, avg_width, font_size, is_last);
                state.all_elements.push(PageElement::Text {
                    x: x_first,
                    y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                    word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }

        // === Remaining lines (no first_line branch needed) ===
        while pos < len {
            let line_start = pos;
            let target = (pos + max_chars_rest).min(len);

            let (mut line_end, next_pos) = if target >= len {
                (len, len)
            } else {
                match memchr::memrchr2(b' ', b'\n', &bytes[line_start..target]) {
                    Some(offset) => (line_start + offset, line_start + offset + 1),
                    None => match memchr::memchr2(b' ', b'\n', &bytes[target..]) {
                        Some(offset) => (target + offset, target + offset + 1),
                        None => (len, len),
                    }
                }
            };

            while line_end > line_start && bytes[line_end - 1] <= b' ' { line_end -= 1; }

            if line_end > line_start {
                if lines_until_break <= 0 {
                    state.new_page();
                    push_start = line_start;
                    buf_push_pos = 0;
                    state.all_text.push_str(&text[line_start..]);
                    lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
                }
                let is_last = next_pos >= len;
                let ws = justify_line(&bytes[line_start..line_end], full_text_width, avg_width, font_size, is_last);
                state.all_elements.push(PageElement::Text {
                    x: x_rest,
                    y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                    word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }
    }

    state.current_x = state.text_left();
    state.add_vertical_space(font_size * 0.2);
    Ok(())
}

/// Zero-copy variant: stores source offsets (flagged with high bit) instead of copying text
/// to page text_buffer. Eliminates ~53MB of memcpy for TextParagraph nodes.
fn layout_text_content_source(text: &str, state: &mut LayoutState, src_off: u32) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let pi = if state.suppress_next_indent { state.suppress_next_indent = false; 0.0 } else { state.paragraph_indent };
    let para_width = state.text_width() - pi;
    let full_text_width = state.text_width();

    state.ensure_space(line_height);
    if text.len() <= max_chars_single {
        // Single line - emit with source reference (no copy)
        state.current_x = state.text_left() + pi;
        state.all_elements.push(PageElement::Text {
            x: state.current_x,
            y: state.current_y,
            text_offset: src_off | SOURCE_REF_FLAG,
            text_len: text.len().min(65535) as u16,
            font_size_100,
            font_style,
            color,
            word_spacing_50: 0,
        });
        state.current_y += step;
    } else {
        // Multi-line: use source offsets directly - no buffer copy needed!
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;
        while pos < len && bytes[pos] <= b' ' { pos += 1; }

        let x_first = state.text_left() + pi;
        let x_rest = state.text_left();
        let max_chars_first = ((para_width - pi) / avg_width) as usize;
        let max_chars_rest = max_chars_single;

        let mut lines_until_break = ((state.cached_max_y - state.current_y - line_height) / step) as i32 + 1;

        // === First line ===
        if pos < len {
            let line_start = pos;
            let target = (pos + max_chars_first).min(len);

            let (mut line_end, next_pos) = if target >= len {
                (len, len)
            } else {
                match memchr::memrchr2(b' ', b'\n', &bytes[line_start..target]) {
                    Some(offset) => (line_start + offset, line_start + offset + 1),
                    None => match memchr::memchr2(b' ', b'\n', &bytes[target..]) {
                        Some(offset) => (target + offset, target + offset + 1),
                        None => (len, len),
                    }
                }
            };

            while line_end > line_start && bytes[line_end - 1] <= b' ' { line_end -= 1; }

            if line_end > line_start {
                if lines_until_break <= 0 {
                    state.new_page();
                    lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
                }
                let is_last = next_pos >= len;
                let ws = justify_line(&bytes[line_start..line_end], para_width, avg_width, font_size, is_last);
                state.all_elements.push(PageElement::Text {
                    x: x_first,
                    y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                    word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }

        // === Remaining lines ===
        while pos < len {
            let line_start = pos;
            let target = (pos + max_chars_rest).min(len);

            let (mut line_end, next_pos) = if target >= len {
                (len, len)
            } else {
                match memchr::memrchr2(b' ', b'\n', &bytes[line_start..target]) {
                    Some(offset) => (line_start + offset, line_start + offset + 1),
                    None => match memchr::memchr2(b' ', b'\n', &bytes[target..]) {
                        Some(offset) => (target + offset, target + offset + 1),
                        None => (len, len),
                    }
                }
            };

            while line_end > line_start && bytes[line_end - 1] <= b' ' { line_end -= 1; }

            if line_end > line_start {
                if lines_until_break <= 0 {
                    state.new_page();
                    lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
                }
                let is_last = next_pos >= len;
                let ws = justify_line(&bytes[line_start..line_end], full_text_width, avg_width, font_size, is_last);
                state.all_elements.push(PageElement::Text {
                    x: x_rest,
                    y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                    word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }
    }

    state.current_x = state.text_left();
    state.add_vertical_space(font_size * 0.2);
    Ok(())
}

/// A styled text span for rich-text layout
struct StyledSpan {
    text: String,
    style: FontStyle,
    color: Color,
}

/// Flatten AST nodes into a sequence of styled spans, preserving bold/italic/etc.
fn nodes_to_spans(nodes: &[Node], style: FontStyle, color: Color, out: &mut Vec<StyledSpan>, source: &str, labels: &HashMap<String, String>, citations: &HashMap<String, u32>) {
    for node in nodes {
        match node {
            Node::Text(s) => {
                out.push(StyledSpan { text: s.clone(), style, color });
            }
            Node::TextRef(offset, len) => {
                let text = &source[*offset as usize..(*offset as usize + *len as usize)];
                out.push(StyledSpan { text: text.to_string(), style, color });
            }
            Node::Bold(children) => {
                let s = match style {
                    FontStyle::Italic => FontStyle::BoldItalic,
                    _ => FontStyle::Bold,
                };
                nodes_to_spans(children, s, color, out, source, labels, citations);
            }
            Node::Italic(children) | Node::Emph(children) => {
                let s = match style {
                    FontStyle::Bold => FontStyle::BoldItalic,
                    _ => FontStyle::Italic,
                };
                nodes_to_spans(children, s, color, out, source, labels, citations);
            }
            Node::Monospace(children) => {
                let mut t = String::new();
                for c in children { node_to_text_resolved(c, &mut t, source, labels); }
                out.push(StyledSpan { text: t, style: FontStyle::Monospace, color });
            }
            Node::Code(s) => {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Monospace, color });
            }
            Node::SmallCaps(children) | Node::Underline(children)
            | Node::Group(children) | Node::Superscript(children)
            | Node::Subscript(children) | Node::Strikethrough(children) => {
                nodes_to_spans(children, style, color, out, source, labels, citations);
            }
            Node::Colored { content, color: c } => {
                nodes_to_spans(content, style, *c, out, source, labels, citations);
            }
            Node::FontSize { content, .. } => {
                nodes_to_spans(content, style, color, out, source, labels, citations);
            }
            Node::Paragraph(children) => {
                nodes_to_spans(children, style, color, out, source, labels, citations);
            }
            Node::NonBreakingSpace | Node::HSpace(_) => {
                out.push(StyledSpan { text: " ".to_string(), style, color });
            }
            Node::LineBreak => {
                out.push(StyledSpan { text: "\n".to_string(), style, color });
            }
            Node::InlineMath(math) => {
                let mut t = String::new();
                math_to_text_buf(math, &mut t);
                out.push(StyledSpan { text: t, style: FontStyle::Italic, color });
            }
            Node::Citation(key) => {
                let cite_text = if let Some(&num) = citations.get(key) {
                    format!("[{}]", num)
                } else {
                    format!("[{}]", key)
                };
                out.push(StyledSpan { text: cite_text, style, color });
            }
            _ => {
                let mut t = String::new();
                node_to_text_resolved(node, &mut t, source, labels);
                if !t.is_empty() {
                    out.push(StyledSpan { text: t, style, color });
                }
            }
        }
    }
}

/// Layout a paragraph with rich inline formatting (bold, italic, etc.).
/// Falls back to plain text layout when children are simple text.
fn layout_rich_paragraph(children: &[Node], state: &mut LayoutState, source: &str, with_indent: bool) -> Result<()> {
    // Check if any children have formatting
    let has_formatting = children.iter().any(|n| matches!(n,
        Node::Bold(_) | Node::Italic(_) | Node::Emph(_) | Node::Monospace(_)
        | Node::Colored { .. } | Node::Code(_) | Node::SmallCaps(_)
        | Node::Underline(_) | Node::InlineMath(_)
    ));

    if !has_formatting {
        // Fast path: plain text paragraph (no formatting)
        state.text_buf.clear();
        // SAFETY: label_map is not modified during node_to_text_ext
        let labels: &HashMap<String, String> = unsafe { &*(&state.label_map as *const _) };
        for node in children {
            node_to_text_resolved(node, &mut state.text_buf, source, labels);
        }
        let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
        if !text.is_empty() {
            if with_indent {
                layout_text_content(text, state)?;
            } else {
                layout_text_content_no_indent(text, state)?;
            }
        }
        return Ok(());
    }

    // Rich text: collect styled spans
    let mut spans = Vec::new();
    nodes_to_spans(children, state.current_font_style, state.current_color, &mut spans, source, &state.label_map, &state.citation_map);

    // Merge adjacent spans with same style and join into "words"
    // Strategy: split spans into words, keeping style info per word
    struct StyledWord {
        text: String,
        style: FontStyle,
        color: Color,
        width: f32,
    }

    let font_size = state.current_font_size;
    let line_height = font_size * 1.2;
    let step = line_height * state.line_spacing;
    let space_width = font_size * 0.25;
    let text_width = state.text_width();
    let indent = if with_indent { state.paragraph_indent } else { 0.0 };

    // Build word list from spans
    let mut words: Vec<StyledWord> = Vec::new();
    for span in &spans {
        if span.text == "\n" {
            // Force line break
            words.push(StyledWord { text: "\n".to_string(), style: span.style, color: span.color, width: 0.0 });
            continue;
        }
        let font_id = match span.style {
            FontStyle::Bold | FontStyle::BoldItalic => FontId::HelveticaBold,
            FontStyle::Monospace => FontId::Courier,
            _ => FontId::Helvetica,
        };
        // Split on whitespace, preserving leading/trailing space info
        let parts: Vec<&str> = span.text.split_whitespace().collect();
        let starts_with_space = span.text.starts_with(char::is_whitespace);
        let ends_with_space = span.text.ends_with(char::is_whitespace);
        if starts_with_space && !words.is_empty() {
            // Insert a space word to separate from previous span
            if let Some(last) = words.last() {
                if last.text != " " && last.text != "\n" {
                    words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, width: space_width });
                }
            }
        }
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, width: space_width });
            }
            let w = font::measure_text(part, font_id, font_size);
            words.push(StyledWord { text: part.to_string(), style: span.style, color: span.color, width: w });
        }
        if ends_with_space && !parts.is_empty() {
            words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, width: space_width });
        }
    }

    // Now layout words with wrapping
    state.ensure_space(line_height);
    let mut line_x = state.text_left() + indent;
    let mut current_line_width = 0.0;
    let max_width = text_width - indent;
    let mut first_line = true;

    for word in &words {
        if word.text == "\n" {
            // Force line break
            state.current_y += step;
            state.ensure_space(line_height);
            line_x = state.text_left();
            current_line_width = 0.0;
            first_line = false;
            continue;
        }
        if word.text == " " {
            current_line_width += space_width;
            line_x += space_width;
            continue;
        }

        let effective_max = if first_line { max_width } else { text_width };

        // Check if word fits on current line
        if current_line_width > 0.0 && current_line_width + word.width > effective_max {
            // Wrap to next line
            state.current_y += step;
            state.ensure_space(line_height);
            line_x = state.text_left();
            current_line_width = 0.0;
            first_line = false;
        }

        // Emit word
        state.current_x = line_x;
        state.emit_text(&word.text, font_size, word.style, word.color);
        line_x += word.width;
        current_line_width += word.width;
    }

    // Advance past last line
    state.current_y += step;
    state.current_x = state.text_left();
    state.add_vertical_space(font_size * 0.2);
    Ok(())
}

/// Text content layout without paragraph indent
fn layout_text_content_no_indent(text: &str, state: &mut LayoutState) -> Result<()> {
    let saved_indent = state.paragraph_indent;
    state.paragraph_indent = 0.0;
    layout_text_content(text, state)?;
    state.paragraph_indent = saved_indent;
    Ok(())
}

fn layout_text_line(text: &str, state: &mut LayoutState) {
    state.emit_text(
        text,
        state.current_font_size,
        state.current_font_style,
        state.current_color,
    );
}

fn layout_list(
    items: &[ListItem],
    state: &mut LayoutState,
    _doc: &Document,
    numbered: bool,
    source: &str,
) -> Result<()> {
    let saved_indent = state.indent;
    let saved_para_indent = state.paragraph_indent;
    state.set_indent(state.indent + 20.0);
    state.paragraph_indent = 0.0; // No paragraph indent inside list items
    state.add_vertical_space(2.0);

    for (i, item) in items.iter().enumerate() {
        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        // Draw bullet or number
        let marker_x = state.text_left() - 15.0;
        state.current_x = marker_x;
        if numbered {
            state.text_buf.clear();
            let mut ibuf = itoa::Buffer::new();
            state.text_buf.push_str(ibuf.format(i + 1));
            state.text_buf.push('.');
            let marker: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(marker, state.current_font_size, FontStyle::Regular, Color::BLACK);
        } else {
            // Draw a filled bullet circle (larger than font's bullet glyph)
            let bullet_r = state.current_font_size * 0.15;
            let bx = marker_x + bullet_r + 2.0;
            let by = state.current_y + state.current_font_size * 0.35;
            state.emit_rounded_rect(bx - bullet_r, by - bullet_r, bullet_r * 2.0, bullet_r * 2.0,
                Some(Color::BLACK), None, bullet_r);
        }
        state.current_x = state.text_left();

        // Use rich paragraph layout to preserve bold/italic inline formatting
        layout_rich_paragraph(&item.content, state, source, false)?;
    }

    state.paragraph_indent = saved_para_indent;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    state.add_vertical_space(2.0);

    Ok(())
}

fn layout_description_list(
    items: &[ListItem],
    state: &mut LayoutState,
    _doc: &Document,
    source: &str,
) -> Result<()> {
    state.add_vertical_space(4.0);

    for item in items {
        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        if let Some(label) = &item.label {
            state.text_buf.clear();
            for node in label {
                node_to_text(node, &mut state.text_buf, source);
            }
            let label_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(label_text, state.current_font_size, FontStyle::Bold, Color::BLACK);
            state.current_y += line_h;
        }

        let saved_indent = state.indent;
        state.set_indent(state.indent + 20.0);
        state.current_x = state.text_left();

        // Use rich paragraph layout for description content
        layout_rich_paragraph(&item.content, state, source, false)?;

        state.set_indent(saved_indent);
        state.add_vertical_space(4.0);
    }

    state.current_x = state.text_left();

    Ok(())
}

/// Layout bibliography entries from \begin{thebibliography} environment
fn layout_bibliography(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Add "References" heading
    state.add_vertical_space(22.0);
    state.ensure_space(40.0);
    let heading = "References";
    let heading_size = state.current_font_size * 1.44;
    state.current_x = state.text_left();
    state.emit_text(heading, heading_size, FontStyle::Bold, Color::BLACK);
    state.current_y += heading_size * 1.2 + 10.0;

    // Walk nodes: each BibItem starts a new entry
    let mut bib_num = 0u32;
    let mut entry_nodes: Vec<&Node> = Vec::new();
    let indent = 24.0;

    for node in nodes {
        if let Node::BibItem(_key) = node {
            // Flush previous entry
            if bib_num > 0 && !entry_nodes.is_empty() {
                layout_bib_entry(bib_num, &entry_nodes, state, doc, source, indent)?;
                entry_nodes.clear();
            }
            bib_num += 1;
        } else {
            if bib_num > 0 {
                entry_nodes.push(node);
            }
        }
    }
    // Flush last entry
    if bib_num > 0 && !entry_nodes.is_empty() {
        layout_bib_entry(bib_num, &entry_nodes, state, doc, source, indent)?;
    }

    state.add_vertical_space(8.0);
    Ok(())
}

/// Layout a single bibliography entry: [N] followed by entry text
fn layout_bib_entry(num: u32, nodes: &[&Node], state: &mut LayoutState, doc: &Document, source: &str, indent: f32) -> Result<()> {
    state.ensure_space(state.current_font_size * 1.5);
    let font_size = state.current_font_size * 0.9;

    // Render [N] marker
    let mut ibuf = itoa::Buffer::new();
    let marker = format!("[{}]", ibuf.format(num));
    state.current_x = state.text_left();
    state.emit_text(&marker, font_size, FontStyle::Regular, Color::BLACK);

    // Collect entry text
    let mut text = String::new();
    for node in nodes {
        node_to_text(*node, &mut text, source);
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        state.current_y += font_size * 1.2;
        return Ok(());
    }

    // Layout entry text with hanging indent
    let saved_indent = state.indent;
    state.set_indent(state.text_left() + indent);
    state.current_x = state.text_left() + indent;
    let metrics = FontMetrics::new(font_size, FontStyle::Regular);
    let available = state.text_width() - indent;
    let line_h = metrics.line_height();

    // Simple word-wrap
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let space_w = metrics.measure_text(" ");
    let mut line_x = state.current_x;
    let mut line_w = 0.0f32;
    let mut first_word = true;

    for word in &words {
        let word_w = metrics.measure_text(word);
        if !first_word && line_w + space_w + word_w > available {
            // New line
            state.current_y += line_h;
            state.ensure_space(line_h);
            line_x = state.text_left() + indent;
            state.current_x = line_x;
            line_w = 0.0;
            first_word = true;
        }
        if !first_word {
            line_w += space_w;
            state.current_x = line_x + line_w;
        }
        state.emit_text(word, font_size, FontStyle::Regular, Color::BLACK);
        line_w += word_w;
        first_word = false;
    }

    state.current_y += line_h + 2.0;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    Ok(())
}

/// Detect the font style of a table cell from its content nodes.
/// Returns Bold if all non-whitespace content is wrapped in \textbf, etc.
fn detect_cell_style(content: &[Node]) -> FontStyle {
    if content.is_empty() {
        return FontStyle::Regular;
    }
    // Check if all significant content nodes are Bold
    let mut has_bold = false;
    let mut has_non_bold = false;
    for node in content {
        match node {
            Node::Bold(_) => has_bold = true,
            Node::Text(t) if t.trim().is_empty() => {} // skip whitespace
            Node::TextRef(_, _) => has_non_bold = true,
            Node::Italic(_) => return FontStyle::Italic,
            _ => has_non_bold = true,
        }
    }
    if has_bold && !has_non_bold {
        FontStyle::Bold
    } else {
        FontStyle::Regular
    }
}

fn layout_table(table: &Table, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    if table.rows.is_empty() {
        return Ok(());
    }

    // Filter out spurious empty trailing rows (from \bottomrule before \end{tabular})
    let num_data_rows = {
        let num_cols = table.columns.iter().filter(|c| !matches!(c, ColumnSpec::Separator)).count().max(1);
        let mut n = table.rows.len();
        while n > 0 && table.rows[n - 1].cells.len() < num_cols
            && table.rows[n - 1].cells.iter().all(|c| c.content.is_empty()) {
            n -= 1;
        }
        n
    };

    // Increment table counter if this table has a caption (it's a float table)
    if table.caption.is_some() {
        state.table_counter += 1;
    }
    let tbl_num = state.table_counter;

    state.add_vertical_space(8.0);

    // Calculate column widths based on column spec and content
    let data_cols: Vec<&ColumnSpec> = table.columns.iter()
        .filter(|c| !matches!(c, ColumnSpec::Separator))
        .collect();
    let num_cols = data_cols.len().max(1);
    let available_width = state.text_width();
    let cell_padding = 4.0;

    // Check if any columns have explicit widths (p{width})
    let has_explicit_widths = data_cols.iter().any(|c| matches!(c, ColumnSpec::Paragraph(_)));

    // Measure max content width per column across all rows
    // Also detect per-cell font style from content nodes
    // Use accurate AFM metrics for measuring (not approximate FontMetrics)
    let base_metrics = state.metrics();
    let font_size = state.current_font_size;
    let mut col_max_widths = vec![0.0f32; num_cols];
    let mut cell_texts: Vec<Vec<String>> = Vec::with_capacity(table.rows.len());
    let mut cell_styles: Vec<Vec<FontStyle>> = Vec::with_capacity(table.rows.len());

    for row in &table.rows {
        let mut row_texts = Vec::with_capacity(num_cols);
        let mut row_styles = Vec::with_capacity(num_cols);
        for (col_idx, cell) in row.cells.iter().enumerate() {
            if col_idx >= num_cols { break; }
            let mut text = String::new();
            for node in &cell.content {
                node_to_text_resolved(node, &mut text, source, &state.label_map);
            }
            let trimmed = text.trim().to_string();
            // Detect cell style from content nodes
            let style = detect_cell_style(&cell.content);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };
            let w = font::measure_text(&trimmed, fid, font_size);
            if w > col_max_widths[col_idx] {
                col_max_widths[col_idx] = w;
            }
            row_texts.push(trimmed);
            row_styles.push(style);
        }
        while row_texts.len() < num_cols {
            row_texts.push(String::new());
            row_styles.push(FontStyle::Regular);
        }
        cell_texts.push(row_texts);
        cell_styles.push(row_styles);
    }

    // Compute column widths
    let col_widths: Vec<f32> = if has_explicit_widths {
        // Use explicit p{width} values, scale to fit available_width
        let mut widths: Vec<f32> = Vec::with_capacity(num_cols);
        let mut total_specified = 0.0f32;
        let mut num_auto = 0u32;
        for col in &data_cols {
            match col {
                ColumnSpec::Paragraph(w) => {
                    widths.push(*w);
                    total_specified += *w;
                }
                _ => {
                    widths.push(0.0); // will be computed
                    num_auto += 1;
                }
            }
        }
        while widths.len() < num_cols {
            widths.push(0.0);
            num_auto += 1;
        }
        // Auto columns get remaining space
        if num_auto > 0 {
            let remaining = (available_width - total_specified).max(0.0);
            let auto_w = remaining / num_auto as f32;
            for (i, w) in widths.iter_mut().enumerate() {
                if *w == 0.0 {
                    // Use content width or auto share
                    *w = col_max_widths.get(i).copied().unwrap_or(auto_w)
                        .min(auto_w).max(cell_padding * 3.0);
                }
            }
        }
        // Scale if total exceeds available
        let total: f32 = widths.iter().sum();
        if total > available_width && total > 0.0 {
            let scale = available_width / total;
            widths.iter_mut().for_each(|w| *w *= scale);
        }
        widths
    } else {
        // Content-based widths
        let total_content = col_max_widths.iter().sum::<f32>() + (num_cols as f32 * cell_padding * 2.0);
        if total_content <= available_width {
            let remaining = available_width - total_content;
            let extra_per_col = remaining / num_cols as f32;
            col_max_widths.iter().map(|&w| w + cell_padding * 2.0 + extra_per_col).collect()
        } else {
            col_max_widths.iter().map(|&w| {
                let ratio = if total_content > 0.0 { w / total_content } else { 1.0 / num_cols as f32 };
                (ratio * available_width).max(cell_padding * 3.0)
            }).collect()
        }
    };

    let line_h = base_metrics.line_height();
    let actual_table_width: f32 = col_widths.iter().sum();
    // Center the table if it's narrower than the text width
    let table_x = if actual_table_width < available_width {
        state.text_left() + (available_width - actual_table_width) / 2.0
    } else {
        state.text_left()
    };

    // Check if table has any hline (border style)
    let has_separators = table.columns.iter().any(|c| matches!(c, ColumnSpec::Separator));

    // Pre-compute wrapped lines for each cell and row heights
    let mut wrapped_cells: Vec<Vec<Vec<String>>> = Vec::with_capacity(table.rows.len());
    let mut row_heights: Vec<f32> = Vec::with_capacity(table.rows.len());

    for (row_idx, row_texts) in cell_texts.iter().enumerate() {
        let mut row_wrapped: Vec<Vec<String>> = Vec::with_capacity(num_cols);
        let mut max_lines = 1u32;
        for (col_idx, text) in row_texts.iter().enumerate() {
            let col_w = col_widths.get(col_idx).copied().unwrap_or(100.0);
            let content_w = col_w - cell_padding * 2.0;
            let fid = if cell_styles[row_idx][col_idx] == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };
            let text_w = font::measure_text(text, fid, font_size);
            if text_w <= content_w + 1.0 || content_w < 20.0 {
                row_wrapped.push(vec![text.clone()]);
            } else {
                // Word-wrap within cell using accurate AFM metrics
                let words: Vec<&str> = text.split_whitespace().collect();
                let mut lines: Vec<String> = Vec::new();
                let mut current_line = String::new();
                let mut current_w = 0.0f32;
                let space_w = font::measure_text(" ", fid, font_size);
                for word in &words {
                    let word_w = font::measure_text(word, fid, font_size);
                    if current_line.is_empty() {
                        current_line.push_str(word);
                        current_w = word_w;
                    } else if current_w + space_w + word_w <= content_w {
                        current_line.push(' ');
                        current_line.push_str(word);
                        current_w += space_w + word_w;
                    } else {
                        lines.push(std::mem::take(&mut current_line));
                        current_line.push_str(word);
                        current_w = word_w;
                    }
                }
                if !current_line.is_empty() {
                    lines.push(current_line);
                }
                if lines.is_empty() { lines.push(String::new()); }
                max_lines = max_lines.max(lines.len() as u32);
                row_wrapped.push(lines);
            }
        }
        let extra = table.rows[row_idx].extra_space_before;
        // Add space below rules (booktabs \belowrulesep) so ascenders don't overlap
        let rule_sep = if table.rows[row_idx].hline_before { font_size * 0.4 } else { 0.0 };
        let rh = max_lines as f32 * line_h + cell_padding * 2.0 + extra + rule_sep;
        row_heights.push(rh);
        wrapped_cells.push(row_wrapped);
    }

    // Compute total table height (rows only, caption will be added)
    let total_row_height: f32 = row_heights.iter().take(num_data_rows).sum();
    let caption_height = if table.caption.is_some() {
        state.current_font_size * 0.9 * 1.2 + 4.0 // approximate caption line height
    } else {
        0.0
    };
    let total_table_height = total_row_height + caption_height + 8.0; // +8 for spacing

    // Try to keep the entire table (caption + rows) together on one page
    let remaining_space = state.cached_max_y - state.current_y;
    let full_page_height = state.cached_max_y - state.cached_start_y;
    if total_table_height > remaining_space && total_table_height <= full_page_height {
        state.new_page();
    }

    // NOW render caption (after potential page break)
    if let Some(caption) = &table.caption {
        state.text_buf.clear();
        state.text_buf.push_str("Table ");
        let mut ibuf = itoa::Buffer::new();
        state.text_buf.push_str(ibuf.format(tbl_num));
        state.text_buf.push_str(": ");
        for node in caption {
            node_to_text(node, &mut state.text_buf, source);
        }
        let full: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
        let cap_font_size = state.current_font_size * 0.9;
        let cap_metrics = FontMetrics::new(cap_font_size, FontStyle::Regular);
        let tw = cap_metrics.measure_text(full);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.current_x = cx;
        state.emit_text(full, cap_font_size, FontStyle::Regular, Color::DARK_GRAY);
        state.current_y += cap_metrics.line_height() + 4.0;
    }

    // Render table rows (only data rows, skip spurious trailing rows)
    for row_idx in 0..num_data_rows {
        let row = &table.rows[row_idx];
        let row_height = row_heights[row_idx];
        let extra = row.extra_space_before;
        state.ensure_space(row_height);

        // Apply extra_space_before (from \addlinespace)
        if extra > 0.0 {
            state.current_y += extra;
        }

        let y = state.current_y;

        let mut col_x = table_x;
        for (col_idx, cell_lines) in wrapped_cells[row_idx].iter().enumerate() {
            if col_idx >= num_cols { break; }
            let col_w = col_widths[col_idx];
            let cx = col_x + cell_padding;
            let cell_content_width = col_w - cell_padding * 2.0;

            let align = if col_idx < data_cols.len() {
                data_cols[col_idx]
            } else {
                &ColumnSpec::Left
            };

            // Use style detected from cell content (bold from \textbf{}, etc.)
            let style = cell_styles[row_idx].get(col_idx).copied().unwrap_or(FontStyle::Regular);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };

            for (line_idx, line_text) in cell_lines.iter().enumerate() {
                let display_w = font::measure_text(line_text, fid, font_size);
                let text_x = match align {
                    ColumnSpec::Center => cx + (cell_content_width - display_w) / 2.0,
                    ColumnSpec::Right => cx + cell_content_width - display_w,
                    _ => cx,
                };
                // Push text down below hline_before rules so ascenders don't overlap
                let rule_sep = if row.hline_before { font_size * 0.4 } else { 0.0 };
                let text_y = y + cell_padding + rule_sep + line_idx as f32 * line_h;
                state.current_x = text_x;
                state.current_y = text_y;
                state.emit_text(line_text, state.current_font_size, style, Color::BLACK);
            }

            col_x += col_w;
        }

        // Draw horizontal lines (booktabs style)
        if row.hline_before {
            let rule_width = if row_idx == 0 { 1.2 } else { 0.8 }; // toprule=1.2, midrule=0.8
            state.emit_line(table_x, y, table_x + actual_table_width, y, rule_width, Color::BLACK);
        }
        if row.hline_after {
            let line_y = y + row_height - extra + 1.0;
            let rule_width = if row_idx == num_data_rows - 1 { 1.2 } else { 0.8 }; // bottomrule=1.2
            state.emit_line(table_x, line_y, table_x + actual_table_width, line_y, rule_width, Color::BLACK);
        }

        state.current_y = y + row_height - extra;
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();

    Ok(())
}

/// Render TikZ diagram using native Rust renderer (no pdflatex shell-out).
/// Delegates to tikz_render module for parsing and layout, then emits page elements.
fn layout_tikz_diagram(tikz_source: &str, state: &mut LayoutState, _doc: &Document) -> Result<()> {
    use crate::tikz_render::{self, TikzElement};

    state.add_vertical_space(10.0);

    let result = tikz_render::render_tikz(tikz_source);

    if result.elements.is_empty() {
        // Fallback placeholder
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

    // Scale to fit available width
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
                    state.emit_rounded_rect(
                        base_x + x * scale, base_y + y * scale,
                        width * scale, height * scale,
                        *fill, *stroke, *corner_radius * scale,
                    );
                } else {
                    state.emit_rect(
                        base_x + x * scale, base_y + y * scale,
                        width * scale, height * scale,
                        *fill, *stroke,
                    );
                }
            }
            TikzElement::Line { x1, y1, x2, y2, width, color } => {
                state.emit_line(
                    base_x + x1 * scale, base_y + y1 * scale,
                    base_x + x2 * scale, base_y + y2 * scale,
                    *width, *color,
                );
            }
            TikzElement::Arrow { x1, y1, x2, y2, width, color, .. } => {
                let px1 = base_x + x1 * scale;
                let py1 = base_y + y1 * scale;
                let px2 = base_x + x2 * scale;
                let py2 = base_y + y2 * scale;
                // Main line
                state.emit_line(px1, py1, px2, py2, *width, *color);
                // Arrow head
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

fn layout_display_math(math_nodes: &[MathNode], state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(8.0);

    // Use the math layout engine for proper rendering
    let math_box = math_layout::layout_math(math_nodes, state.current_font_size);

    let total_height = math_box.height + math_box.depth;
    state.ensure_space(total_height + 16.0);

    // Center the math horizontally
    let cx = state.text_left() + (state.text_width() - math_box.width) / 2.0;
    let baseline_y = state.current_y + math_box.height;

    // Emit all math elements
    for elem in &math_box.elements {
        match elem {
            math_layout::MathElement::Text { x, y, text, font_size, font_id, color } => {
                let style = match font_id {
                    FontId::HelveticaOblique => FontStyle::Italic,
                    FontId::HelveticaBold => FontStyle::Bold,
                    FontId::Courier => FontStyle::Monospace,
                    FontId::Symbol => FontStyle::Regular, // Symbol font handled in PDF
                    _ => FontStyle::Regular,
                };
                let abs_x = cx + x;
                let abs_y = baseline_y + y;
                let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                state.all_text.push_str(text);
                state.all_elements.push(PageElement::Text {
                    x: abs_x,
                    y: abs_y,
                    text_offset: offset,
                    text_len: text.len().min(65535) as u16,
                    font_size_100: (*font_size * 100.0) as u16,
                    font_style: style,
                    color: *color,
                    word_spacing_50: 0,
                });
            }
            math_layout::MathElement::Line { x1, y1, x2, y2, width, color } => {
                state.emit_line(
                    cx + x1, baseline_y + y1,
                    cx + x2, baseline_y + y2,
                    *width, *color,
                );
            }
        }
    }

    state.current_y = baseline_y + math_box.depth;
    state.add_vertical_space(8.0);
    state.current_x = state.text_left();

    Ok(())
}

fn layout_verbatim(text: &str, state: &mut LayoutState) -> Result<()> {
    layout_code_block(text, None, state)
}

fn layout_code_block(text: &str, language: Option<&str>, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(6.0);

    let font_size = state.base_font_size * 0.85;
    let metrics = FontMetrics::new(font_size, FontStyle::Monospace);

    // Background rect
    let text_lines: Vec<&str> = text.lines().collect();
    let total_height = text_lines.len() as f32 * metrics.line_height() + 12.0;

    state.ensure_space(total_height);

    state.emit_rect(
        state.text_left() - 4.0,
        state.current_y - 4.0,
        state.text_width() + 8.0,
        total_height,
        Some(Color::rgb(0.96, 0.96, 0.96)),
        Some(Color::LIGHT_GRAY),
    );

    // Try syntax highlighting if language is specified
    if let Some(lang) = language {
        let highlighted = crate::highlight::get_highlighter().highlight(text, lang);
        if !highlighted.is_empty() {
            for line_spans in &highlighted {
                state.current_x = state.text_left() + 4.0;
                for span in line_spans {
                    let style = if span.bold {
                        FontStyle::Bold
                    } else {
                        FontStyle::Monospace
                    };
                    let color = span.color;
                    let w = font::measure_text(&span.text, FontId::Courier, font_size);
                    let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                    state.all_text.push_str(&span.text);
                    state.all_elements.push(PageElement::Text {
                        x: state.current_x,
                        y: state.current_y,
                        text_offset: offset,
                        text_len: span.text.len().min(65535) as u16,
                        font_size_100: (font_size * 100.0) as u16,
                        font_style: style,
                        color,
                        word_spacing_50: 0,
                    });
                    state.current_x += w;
                }
                state.current_y += metrics.line_height();
            }
            state.add_vertical_space(10.0);
            state.current_x = state.text_left();
            return Ok(());
        }
    }

    // Fallback: plain monospace
    for line in text_lines {
        state.current_x = state.text_left() + 4.0;
        state.emit_text(line, font_size, FontStyle::Monospace, Color::DARK_GRAY);
        state.current_y += metrics.line_height();
    }

    state.add_vertical_space(10.0);
    state.current_x = state.text_left();

    Ok(())
}

fn layout_centered(content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Simple centering: layout each child and center
    for node in content {
        match node {
            Node::Paragraph(children) => {
                state.text_buf.clear();
                for child in children {
                    node_to_text(child, &mut state.text_buf, source);
                }
                let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
                if text.is_empty() { continue; }

                let font_size = state.current_font_size;
                let font_style = state.current_font_style;
                let color = state.current_color;
                let line_h = font_size * 1.2;
                let avg_width = font_size * 0.48;
                let space_width = font_size * 0.25;
                let para_width = state.text_width();

                // Inline word-wrap with centering
                let bytes = text.as_bytes();
                let len = bytes.len();
                let mut pos = 0;
                let mut line_start = 0;
                let mut current_width: f32 = 0.0;
                while pos < len && bytes[pos] <= b' ' { pos += 1; }
                line_start = pos;

                while pos < len {
                    let word_start = pos;
                    pos = match memchr::memchr2(b' ', b'\n', &bytes[pos..]) {
                        Some(o) => pos + o,
                        None => len,
                    };
                    let word_width = (pos - word_start) as f32 * avg_width;
                    if current_width > 0.0 && current_width + space_width + word_width > para_width {
                        let line = text[line_start..word_start].trim_end();
                        if !line.is_empty() {
                            state.ensure_space(line_h);
                            let tw = line.len() as f32 * avg_width;
                            state.current_x = state.text_left() + (para_width - tw) / 2.0;
                            state.emit_text(line, font_size, font_style, color);
                            state.current_y += line_h * state.line_spacing;
                        }
                        line_start = word_start;
                        current_width = word_width;
                    } else {
                        if current_width > 0.0 { current_width += space_width; }
                        current_width += word_width;
                    }
                    if pos < len { pos += 1; while pos < len && bytes[pos] <= b' ' { pos += 1; } }
                }
                let remaining = text[line_start..].trim_end();
                if !remaining.is_empty() {
                    state.ensure_space(line_h);
                    let tw = remaining.len() as f32 * avg_width;
                    state.current_x = state.text_left() + (para_width - tw) / 2.0;
                    state.emit_text(remaining, font_size, font_style, color);
                    state.current_y += line_h * state.line_spacing;
                }

                state.add_vertical_space(font_size * 0.2);
            }
            _ => {
                layout_node(node, state, doc, source)?;
            }
        }
    }
    Ok(())
}

/// Write nodes as text into a buffer (avoids allocation)
fn nodes_to_text_buf(nodes: &[Node], buf: &mut String, source: &str) {
    buf.clear();
    for node in nodes {
        node_to_text(node, buf, source);
    }
}

/// Convert AST nodes to plain text (for measurement and simple rendering)
pub fn nodes_to_text(nodes: &[Node], source: &str) -> String {
    // Fast path: if there's only one Text node, return it directly
    if nodes.len() == 1 {
        if let Node::Text(s) = &nodes[0] {
            return s.clone();
        }
        if let Node::TextRef(offset, len) = &nodes[0] {
            return source[*offset as usize..(*offset as usize + *len as usize)].to_string();
        }
    }

    // Estimate capacity
    let cap: usize = nodes.iter().map(|n| {
        match n {
            Node::Text(s) => s.len(),
            Node::TextRef(_, len) => *len as usize,
            _ => 10,
        }
    }).sum();

    let mut result = String::with_capacity(cap);
    for node in nodes {
        node_to_text(node, &mut result, source);
    }
    result
}

fn node_to_text(node: &Node, out: &mut String, source: &str) {
    node_to_text_ext(node, out, source, None);
}

fn node_to_text_resolved(node: &Node, out: &mut String, source: &str, labels: &HashMap<String, String>) {
    node_to_text_ext(node, out, source, Some(labels));
}

fn node_to_text_ext(node: &Node, out: &mut String, source: &str, labels: Option<&HashMap<String, String>>) {
    match node {
        Node::Text(s) => out.push_str(s),
        Node::TextRef(offset, len) => out.push_str(&source[*offset as usize..(*offset as usize + *len as usize)]),
        Node::Bold(children) | Node::Italic(children) | Node::Monospace(children)
        | Node::SmallCaps(children) | Node::Underline(children) | Node::Emph(children)
        | Node::Strikethrough(children) | Node::Superscript(children)
        | Node::Subscript(children) | Node::Group(children) => {
            for child in children {
                node_to_text_ext(child, out, source, labels);
            }
        }
        Node::Colored { content, .. } => {
            for child in content {
                node_to_text_ext(child, out, source, labels);
            }
        }
        Node::FontSize { content, .. } => {
            for child in content {
                node_to_text_ext(child, out, source, labels);
            }
        }
        Node::Paragraph(children) => {
            for child in children {
                node_to_text_ext(child, out, source, labels);
            }
        }
        Node::InlineMath(math) => {
            math_to_text_buf(math, out);
        }
        Node::NonBreakingSpace => out.push(' '),
        Node::HSpace(_) => out.push(' '),
        Node::LineBreak => out.push('\n'),
        Node::EnDash => out.push('\u{2013}'),
        Node::EmDash => out.push('\u{2014}'),
        Node::Ellipsis => out.push_str("\u{2026}"),
        Node::LeftQuote => out.push('\u{2018}'),
        Node::RightQuote => out.push('\u{2019}'),
        Node::LeftDoubleQuote => out.push('\u{201C}'),
        Node::RightDoubleQuote => out.push('\u{201D}'),
        Node::Copyright => out.push('\u{00A9}'),
        Node::Registered => out.push('\u{00AE}'),
        Node::Trademark => out.push('\u{2122}'),
        Node::Ampersand => out.push('&'),
        Node::Percent => out.push('%'),
        Node::Dollar => out.push('$'),
        Node::Hash => out.push('#'),
        Node::Underscore => out.push('_'),
        Node::Backslash => out.push('\\'),
        Node::Tilde => out.push('~'),
        Node::Caret => out.push('^'),
        Node::LeftBrace => out.push('{'),
        Node::RightBrace => out.push('}'),
        Node::Footnote(_) => {
            out.push_str("[*]");
        }
        Node::Ref(label) => {
            if let Some(map) = labels {
                if let Some(resolved) = map.get(label) {
                    out.push_str(resolved);
                } else {
                    out.push_str("??");
                }
            } else {
                out.push_str("??");
            }
        }
        Node::Citation(key) => {
            out.push('[');
            out.push_str(key);
            out.push(']');
        }
        Node::Label(_) | Node::BibItem(_) => {}
        Node::Code(s) => out.push_str(s),
        _ => {}
    }
}

/// Map Unicode math/Greek symbols to WinAnsi-safe text representations.
/// Symbols in the Latin-1 range (U+00A0..U+00FF) pass through directly as they're in WinAnsi.
/// Others get ASCII approximations since Standard 14 fonts can't encode them.
#[inline]
fn math_symbol_to_text(s: &str, out: &mut String) {
    match s.as_bytes() {
        // Fast path: ASCII or Latin-1 chars pass through (includes ±, ×, ÷, ·)
        [b] if *b < 0x80 => out.push(*b as char),
        [0xC2, b] => out.push(char::from(*b | 0x80)),  // U+0080..U+00FF (2-byte UTF-8 in Latin-1)
        [0xC3, b] => out.push(char::from((*b & 0x3F) | 0xC0)),  // U+00C0..U+00FF
        _ => {
            // Multi-byte Unicode: map to ASCII approximation
            let ch = s.chars().next().unwrap_or('?');
            match ch {
                '\u{2264}' => out.push_str("<="),   // ≤
                '\u{2265}' => out.push_str(">="),   // ≥
                '\u{2260}' => out.push_str("!="),   // ≠
                '\u{2248}' => out.push_str("~~"),   // ≈
                '\u{2261}' => out.push_str("==="),  // ≡
                '\u{2192}' => out.push_str("->"),   // →
                '\u{2190}' => out.push_str("<-"),   // ←
                '\u{2194}' => out.push_str("<->"),  // ↔
                '\u{21D2}' => out.push_str("=>"),   // ⇒
                '\u{21D0}' => out.push_str("<="),   // ⇐
                '\u{21D4}' => out.push_str("<=>"),  // ⇔
                '\u{2208}' => out.push_str("in"),   // ∈
                '\u{2209}' => out.push_str("not in"), // ∉
                '\u{2282}' => out.push_str("c="),   // ⊂
                '\u{2283}' => out.push_str("=c"),   // ⊃
                '\u{222A}' => out.push_str("U"),    // ∪
                '\u{2229}' => out.push_str("n"),    // ∩
                '\u{2200}' => out.push_str("for all"), // ∀
                '\u{2203}' => out.push_str("exists"),  // ∃
                '\u{221E}' => out.push_str("inf"),  // ∞
                '\u{2202}' => out.push_str("d"),    // ∂
                '\u{2207}' => out.push_str("V"),    // ∇ (nabla)
                '\u{221A}' => out.push_str("sqrt"), // √
                '\u{2211}' => out.push_str("S"),    // Σ (sum)
                '\u{220F}' => out.push_str("P"),    // Π (product)
                '\u{222B}' => out.push_str("int"),  // ∫
                '\u{2205}' => out.push_str("{}"),   // ∅
                '\u{2220}' => out.push_str("L"),    // ∠
                '\u{2026}' => out.push_str("..."),  // …
                '\u{2032}' => out.push('\''),       // ′
                '\u{2213}' => out.push_str("-/+"),  // ∓
                // Greek letters → Latin approximations
                '\u{03B1}' => out.push('a'),  // α
                '\u{03B2}' => out.push('b'),  // β
                '\u{03B3}' => out.push('g'),  // γ
                '\u{03B4}' => out.push('d'),  // δ
                '\u{03B5}' => out.push('e'),  // ε
                '\u{03B6}' => out.push('z'),  // ζ
                '\u{03B7}' => out.push('h'),  // η
                '\u{03B8}' => out.push('q'),  // θ
                '\u{03B9}' => out.push('i'),  // ι
                '\u{03BA}' => out.push('k'),  // κ
                '\u{03BB}' => out.push('l'),  // λ
                '\u{03BC}' => out.push('u'),  // μ
                '\u{03BD}' => out.push('v'),  // ν
                '\u{03BE}' => out.push('x'),  // ξ
                '\u{03C0}' => out.push('p'),  // π
                '\u{03C1}' => out.push('r'),  // ρ
                '\u{03C3}' => out.push('s'),  // σ
                '\u{03C4}' => out.push('t'),  // τ
                '\u{03C5}' => out.push('u'),  // υ
                '\u{03C6}' => out.push('f'),  // φ
                '\u{03C7}' => out.push('c'),  // χ
                '\u{03C8}' => out.push('y'),  // ψ
                '\u{03C9}' => out.push('w'),  // ω
                // Uppercase Greek
                '\u{0393}' => out.push('G'),  // Γ
                '\u{0394}' => out.push('D'),  // Δ
                '\u{0398}' => out.push('Q'),  // Θ
                '\u{039B}' => out.push('L'),  // Λ
                '\u{039E}' => out.push('X'),  // Ξ
                '\u{03A0}' => out.push('P'),  // Π
                '\u{03A3}' => out.push('S'),  // Σ
                '\u{03A6}' => out.push('F'),  // Φ
                '\u{03A8}' => out.push('Y'),  // Ψ
                '\u{03A9}' => out.push('W'),  // Ω
                // Degree symbol (common in $^\circ$)
                '\u{00B0}' => out.push('\u{00B0}'), // ° (in WinAnsi)
                _ => out.push('?'),
            }
        }
    }
}

fn math_to_text_buf(nodes: &[MathNode], out: &mut String) {
    for node in nodes {
        math_node_to_text(node, out);
    }
}

fn math_to_text(nodes: &[MathNode]) -> String {
    let mut result = String::new();
    math_to_text_buf(nodes, &mut result);
    result
}

fn math_node_to_text(node: &MathNode, out: &mut String) {
    match node {
        MathNode::Number(s) => out.push_str(s),
        MathNode::Variable(c) => out.push(*c),
        MathNode::Operator(s) => {
            out.push(' ');
            math_symbol_to_text(s, out);
            out.push(' ');
        }
        MathNode::Text(s) => out.push_str(s),
        MathNode::Symbol(s) => math_symbol_to_text(s, out),
        MathNode::Function(name) => out.push_str(name),
        MathNode::Space(_) => out.push(' '),
        MathNode::Frac { numer, denom } => {
            out.push('(');
            math_to_text_buf(numer, out);
            out.push_str(")/(");
            math_to_text_buf(denom, out);
            out.push(')');
        }
        MathNode::Sqrt { index, radicand } => {
            out.push_str("\u{221A}");
            if let Some(idx) = index {
                out.push('[');
                math_to_text_buf(idx, out);
                out.push(']');
            }
            out.push('(');
            math_to_text_buf(radicand, out);
            out.push(')');
        }
        MathNode::Super(nodes) => {
            // Render superscript content inline (no real superscript in flat text mode)
            math_to_text_buf(nodes, out);
        }
        MathNode::Sub(nodes) => {
            // Render subscript content inline
            math_to_text_buf(nodes, out);
        }
        MathNode::Group(nodes) => {
            math_to_text_buf(nodes, out);
        }
        MathNode::Sum { lower, upper } => {
            out.push_str("\u{2211}");
            if let Some(l) = lower {
                out.push_str("_{");
                math_to_text_buf(l, out);
                out.push('}');
            }
            if let Some(u) = upper {
                out.push_str("^{");
                math_to_text_buf(u, out);
                out.push('}');
            }
        }
        MathNode::Integral { lower, upper } => {
            out.push_str("\u{222B}");
            if let Some(l) = lower {
                out.push_str("_{");
                math_to_text_buf(l, out);
                out.push('}');
            }
            if let Some(u) = upper {
                out.push_str("^{");
                math_to_text_buf(u, out);
                out.push('}');
            }
        }
        MathNode::Product { lower, upper } => {
            out.push_str("\u{220F}");
            if let Some(l) = lower {
                out.push_str("_{");
                math_to_text_buf(l, out);
                out.push('}');
            }
            if let Some(u) = upper {
                out.push_str("^{");
                math_to_text_buf(u, out);
                out.push('}');
            }
        }
        MathNode::Left(d) | MathNode::Right(d) => out.push_str(d),
        MathNode::Matrix { rows, .. } => {
            for (i, row) in rows.iter().enumerate() {
                for (j, cell) in row.iter().enumerate() {
                    math_to_text_buf(cell, out);
                    if j < row.len() - 1 { out.push_str(" & "); }
                }
                if i < rows.len() - 1 { out.push_str(" \\\\ "); }
            }
        }
        MathNode::Accent { base, accent_type } => {
            math_to_text_buf(base, out);
            match accent_type {
                AccentType::Hat => out.push('\u{0302}'),
                AccentType::Tilde => out.push('\u{0303}'),
                AccentType::Bar => out.push('\u{0304}'),
                AccentType::Dot => out.push('\u{0307}'),
                AccentType::DDot => out.push_str("\u{0308}"),
                AccentType::Vec => out.push('\u{20D7}'),
                _ => {}
            }
        }
        MathNode::Over { content, .. } | MathNode::Under { content, .. } => {
            math_to_text_buf(content, out);
        }
    }
}
