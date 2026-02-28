/// Layout engine: converts document AST to positioned page elements
/// Direct layout without intermediate format for maximum speed

use anyhow::Result;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle, wrap_text};

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
    footnotes: Vec<Vec<Node>>,
    footnote_counter: u32,
    text_buf: String,
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
            footnotes: Vec::new(),
            footnote_counter: 0,
            text_buf: String::with_capacity(4096),
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
            x, y, width: w, height: h, fill, stroke, stroke_width: 0.5,
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
                    let x = state.text_left() + state.paragraph_indent;
                    state.all_elements.push(PageElement::Text {
                        x,
                        y: state.current_y,
                        text_offset: src_off | SOURCE_REF_FLAG,
                        text_len: text.len() as u16,
                        font_size_100,
                        font_style,
                        color,
                    });
                    state.current_y += step;
                    state.current_x = state.text_left();
                    state.add_vertical_space(state.current_font_size * 0.4);
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
            layout_nodes(&fig.content, state, doc, source)?;
            if let Some(cap) = &fig.caption {
                state.current_y += 6.0;
                state.current_x = state.text_left();
                // "Figure N: " prefix
                state.text_buf.clear();
                state.text_buf.push_str("Figure ");
                let mut ibuf = itoa::Buffer::new();
                state.text_buf.push_str(ibuf.format(state.page_number));
                state.text_buf.push_str(": ");
                let prefix: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
                let prefix_width = state.metrics().measure_text(prefix);
                state.emit_text(prefix, state.current_font_size, FontStyle::Bold, Color::BLACK);
                state.current_x += prefix_width;

                // Use text_buf for caption text (reuse same buffer, append after prefix consumed)
                state.text_buf.clear();
                for node in cap {
                    node_to_text(node, &mut state.text_buf, source);
                }
                let cap_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
                layout_text_line(cap_text, state);
                state.current_y += state.current_font_size * 1.2;
            }
            state.set_indent(saved_indent);
            state.current_x = state.text_left();
            state.add_vertical_space(10.0);
        }

        Node::Image(img) => {
            // Placeholder for image
            let img_w = img.width.unwrap_or(200.0);
            let img_h = img.height.unwrap_or(150.0);
            state.ensure_space(img_h + 10.0);

            let x = state.text_left() + (state.text_width() - img_w) / 2.0;
            state.emit_rect(x, state.current_y, img_w, img_h, None, Some(Color::GRAY));
            // Show filename in center
            let metrics = FontMetrics::new(8.0, FontStyle::Italic);
            let text_w = metrics.measure_text(&img.path);
            state.emit_text(
                &img.path,
                8.0,
                FontStyle::Italic,
                Color::GRAY,
            );
            state.current_y += img_h + 6.0;
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
            layout_verbatim(text, state)?;
        }

        Node::Environment(env) => {
            // Generic environment: just layout content
            layout_nodes(&env.content, state, doc, source)?;
        }

        Node::Minipage { width, content } => {
            let saved_indent = state.indent;
            layout_nodes(content, state, doc, source)?;
            state.set_indent(saved_indent);
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

        // Skip these
        Node::Label(_) | Node::Ref(_) | Node::Citation(_) | Node::Raw(_) => {}

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
        let lines = wrap_text(title, &metrics, state.text_width());

        for line in &lines {
            let tw = metrics.measure_text(line);
            let cx = state.text_left() + (state.text_width() - tw) / 2.0;
            state.ensure_space(metrics.line_height());
            state.current_x = cx;
            state.emit_text(line, size, FontStyle::Bold, Color::BLACK);
            state.current_y += metrics.line_height();
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

    Ok(())
}

fn layout_paragraph(children: &[Node], state: &mut LayoutState, _doc: &Document, source: &str) -> Result<()> {
    // Extract paragraph text without allocating for the common case
    let text: &str = if children.len() == 1 {
        if let Node::Text(s) = &children[0] {
            s.trim()
        } else if let Node::TextRef(offset, len) = &children[0] {
            let raw = &source[*offset as usize..(*offset as usize + *len as usize)];
            let bytes = raw.as_bytes();
            if !bytes.is_empty() && bytes[0] > b' ' && bytes[bytes.len()-1] > b' ' {
                raw
            } else {
                raw.trim()
            }
        } else {
            state.text_buf.clear();
            node_to_text(&children[0], &mut state.text_buf, source);
            // SAFETY: text_buf is not modified during word-wrapping.
            unsafe { &*(state.text_buf.trim() as *const str) }
        }
    } else {
        state.text_buf.clear();
        for node in children {
            node_to_text(node, &mut state.text_buf, source);
        }
        unsafe { &*(state.text_buf.trim() as *const str) }
    };
    if !text.is_empty() {
        layout_text_content(text, state)?;
    }
    Ok(())
}

/// Core word-wrapping and text layout. Separate function to keep code in one place
/// and avoid icache bloat from inlining into the large layout_node match.
fn layout_text_content(text: &str, state: &mut LayoutState) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let para_width = state.text_width() - state.paragraph_indent;

    state.ensure_space(line_height);
    if text.len() <= max_chars_single {
        // Single line - emit directly
        state.current_x = state.text_left() + state.paragraph_indent;
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
        let mut buf_push_pos = (state.all_text.len() - state.current_page_text_start as usize);
        state.all_text.push_str(text);

        let x_first = state.text_left() + state.paragraph_indent;
        let x_rest = state.text_left();
        let max_chars_first = ((para_width - state.paragraph_indent) / avg_width) as usize;
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
                state.all_elements.push(PageElement::Text {
                    x: x_first,
                    y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
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
                state.all_elements.push(PageElement::Text {
                    x: x_rest,
                    y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }
    }

    state.current_x = state.text_left();
    state.add_vertical_space(font_size * 0.4);
    Ok(())
}

/// Zero-copy variant: stores source offsets (flagged with high bit) instead of copying text
/// to page text_buffer. Eliminates ~53MB of memcpy for TextParagraph nodes.
fn layout_text_content_source(text: &str, state: &mut LayoutState, src_off: u32) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let para_width = state.text_width() - state.paragraph_indent;

    state.ensure_space(line_height);
    if text.len() <= max_chars_single {
        // Single line - emit with source reference (no copy)
        state.current_x = state.text_left() + state.paragraph_indent;
        state.all_elements.push(PageElement::Text {
            x: state.current_x,
            y: state.current_y,
            text_offset: src_off | SOURCE_REF_FLAG,
            text_len: text.len().min(65535) as u16,
            font_size_100,
            font_style,
            color,
        });
        state.current_y += step;
    } else {
        // Multi-line: use source offsets directly - no buffer copy needed!
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;
        while pos < len && bytes[pos] <= b' ' { pos += 1; }

        let x_first = state.text_left() + state.paragraph_indent;
        let x_rest = state.text_left();
        let max_chars_first = ((para_width - state.paragraph_indent) / avg_width) as usize;
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
                state.all_elements.push(PageElement::Text {
                    x: x_first,
                    y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
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
                state.all_elements.push(PageElement::Text {
                    x: x_rest,
                    y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100,
                    font_style,
                    color,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }
    }

    state.current_x = state.text_left();
    state.add_vertical_space(font_size * 0.4);
    Ok(())
}

fn layout_inline_text(text: &str, _original_nodes: &[Node], state: &mut LayoutState) {
    // For now, emit as single text. TODO: proper inline formatting
    state.emit_text(
        text,
        state.current_font_size,
        state.current_font_style,
        state.current_color,
    );
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
    state.set_indent(state.indent + 20.0);
    state.add_vertical_space(4.0);

    for (i, item) in items.iter().enumerate() {
        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        // Draw bullet or number
        let marker_x = state.text_left() - 15.0;
        state.current_x = marker_x;
        if numbered {
            // Build "N." into text_buf, emit as single element
            state.text_buf.clear();
            let mut ibuf = itoa::Buffer::new();
            state.text_buf.push_str(ibuf.format(i + 1));
            state.text_buf.push('.');
            let marker: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(marker, state.current_font_size, FontStyle::Regular, Color::BLACK);
        } else {
            state.emit_text("\u{2022}", state.current_font_size, FontStyle::Regular, Color::BLACK);
        }
        state.current_x = state.text_left();

        // Layout item content using reusable buffer
        state.text_buf.clear();
        for node in &item.content {
            node_to_text(node, &mut state.text_buf, source);
        }
        // SAFETY: text_buf not modified during emit_text
        let item_text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
        if !item_text.is_empty() {
            let font_size = state.current_font_size;
            let font_style = state.current_font_style;
            let avg_width = font_size * 0.48;
            let space_width = font_size * 0.25;
            let text_width = state.text_width();

            // Inline word-wrap for list items (avoid wrap_text Vec<String> allocation)
            let bytes = item_text.as_bytes();
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
                if current_width > 0.0 && current_width + space_width + word_width > text_width {
                    let line = item_text[line_start..word_start].trim_end();
                    if !line.is_empty() {
                        state.ensure_space(line_h);
                        state.emit_text(line, font_size, font_style, Color::BLACK);
                        state.current_y += line_h * state.line_spacing;
                        state.current_x = state.text_left();
                    }
                    line_start = word_start;
                    current_width = word_width;
                } else {
                    if current_width > 0.0 { current_width += space_width; }
                    current_width += word_width;
                }
                if pos < len { pos += 1; while pos < len && bytes[pos] <= b' ' { pos += 1; } }
            }
            let remaining = item_text[line_start..].trim_end();
            if !remaining.is_empty() {
                state.ensure_space(line_h);
                state.emit_text(remaining, font_size, font_style, Color::BLACK);
                state.current_y += line_h * state.line_spacing;
                state.current_x = state.text_left();
            }
        }

        state.add_vertical_space(2.0);
    }

    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    state.add_vertical_space(4.0);

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

        // Use text_buf + inline word-wrap (avoids nodes_to_text + wrap_text allocations)
        state.text_buf.clear();
        for node in &item.content {
            node_to_text(node, &mut state.text_buf, source);
        }
        let item_text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
        if !item_text.is_empty() {
            let font_size = state.current_font_size;
            let font_style = state.current_font_style;
            let avg_width = font_size * 0.48;
            let space_width = font_size * 0.25;
            let text_width = state.text_width();
            let bytes = item_text.as_bytes();
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
                if current_width > 0.0 && current_width + space_width + word_width > text_width {
                    let line = item_text[line_start..word_start].trim_end();
                    if !line.is_empty() {
                        state.ensure_space(line_h);
                        state.emit_text(line, font_size, font_style, Color::BLACK);
                        state.current_y += line_h * state.line_spacing;
                        state.current_x = state.text_left();
                    }
                    line_start = word_start;
                    current_width = word_width;
                } else {
                    if current_width > 0.0 { current_width += space_width; }
                    current_width += word_width;
                }
                if pos < len { pos += 1; while pos < len && bytes[pos] <= b' ' { pos += 1; } }
            }
            let remaining = item_text[line_start..].trim_end();
            if !remaining.is_empty() {
                state.ensure_space(line_h);
                state.emit_text(remaining, font_size, font_style, Color::BLACK);
                state.current_y += line_h * state.line_spacing;
                state.current_x = state.text_left();
            }
        }

        state.set_indent(saved_indent);
        state.add_vertical_space(4.0);
    }

    state.current_x = state.text_left();

    Ok(())
}

fn layout_table(table: &Table, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    if table.rows.is_empty() {
        return Ok(());
    }

    state.add_vertical_space(8.0);

    // Calculate column widths
    let data_cols: Vec<&ColumnSpec> = table.columns.iter()
        .filter(|c| !matches!(c, ColumnSpec::Separator))
        .collect();
    let num_cols = data_cols.len().max(1);
    let available_width = state.text_width();
    let col_width = available_width / num_cols as f32;

    let cell_padding = 4.0;
    let row_height = state.metrics().line_height() + cell_padding * 2.0;

    let table_x = state.text_left();

    for (row_idx, row) in table.rows.iter().enumerate() {
        state.ensure_space(row_height);

        let y = state.current_y;

        for (col_idx, cell) in row.cells.iter().enumerate() {
            if col_idx >= num_cols {
                break;
            }

            let cx = table_x + col_idx as f32 * col_width + cell_padding;
            state.text_buf.clear();
            for node in &cell.content {
                node_to_text(node, &mut state.text_buf, source);
            }
            let cell_text: &str = unsafe { &*(state.text_buf.trim() as *const str) };

            // Determine alignment
            let align = if col_idx < data_cols.len() {
                data_cols[col_idx]
            } else {
                &ColumnSpec::Left
            };

            let metrics = state.metrics();
            let text_w = metrics.measure_text(cell_text);
            let cell_content_width = col_width * cell.colspan as f32 - cell_padding * 2.0;

            let text_x = match align {
                ColumnSpec::Center => cx + (cell_content_width - text_w) / 2.0,
                ColumnSpec::Right => cx + cell_content_width - text_w,
                _ => cx,
            };

            let style = if row_idx == 0 {
                FontStyle::Bold
            } else {
                state.current_font_style
            };

            state.emit_text(
                cell_text,
                state.current_font_size,
                style,
                Color::BLACK,
            );
            // Fix position
            if let Some(PageElement::Text { x, .. }) = state.all_elements.last_mut() {
                *x = text_x;
            }
        }

        // Draw horizontal line
        if row.hline_after || row_idx == 0 {
            let line_y = y + row_height - 2.0;
            state.emit_line(
                table_x,
                line_y,
                table_x + available_width,
                line_y,
                0.3,
                Color::GRAY,
            );
        }

        state.current_y += row_height;
    }

    // Caption
    if let Some(caption) = &table.caption {
        state.current_y += 4.0;
        state.text_buf.clear();
        state.text_buf.push_str("Table: ");
        for node in caption {
            node_to_text(node, &mut state.text_buf, source);
        }
        let full: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
        let font_size = state.current_font_size * 0.9;
        let metrics = FontMetrics::new(font_size, FontStyle::Regular);
        let tw = metrics.measure_text(full);
        let cx = state.text_left() + (state.text_width() - tw) / 2.0;
        state.current_x = cx;
        state.emit_text(full, font_size, FontStyle::Regular, Color::DARK_GRAY);
        state.current_y += metrics.line_height();
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();

    Ok(())
}

fn layout_display_math(math_nodes: &[MathNode], state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(8.0);

    state.text_buf.clear();
    math_to_text_buf(math_nodes, &mut state.text_buf);
    let math_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
    let metrics = FontMetrics::new(state.current_font_size, FontStyle::Italic);
    let tw = metrics.measure_text(math_text);

    state.ensure_space(metrics.line_height() + 16.0);

    // Center the math
    let cx = state.text_left() + (state.text_width() - tw) / 2.0;
    state.current_x = cx;
    state.emit_text(math_text, state.current_font_size, FontStyle::Italic, Color::BLACK);
    state.current_y += metrics.line_height();

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();

    Ok(())
}

fn layout_verbatim(text: &str, state: &mut LayoutState) -> Result<()> {
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

                state.add_vertical_space(font_size * 0.4);
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
    match node {
        Node::Text(s) => out.push_str(s),
        Node::TextRef(offset, len) => out.push_str(&source[*offset as usize..(*offset as usize + *len as usize)]),
        Node::Bold(children) | Node::Italic(children) | Node::Monospace(children)
        | Node::SmallCaps(children) | Node::Underline(children) | Node::Emph(children)
        | Node::Strikethrough(children) | Node::Superscript(children)
        | Node::Subscript(children) | Node::Group(children) => {
            for child in children {
                node_to_text(child, out, source);
            }
        }
        Node::Colored { content, .. } => {
            for child in content {
                node_to_text(child, out, source);
            }
        }
        Node::FontSize { content, .. } => {
            for child in content {
                node_to_text(child, out, source);
            }
        }
        Node::Paragraph(children) => {
            for child in children {
                node_to_text(child, out, source);
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
        Node::Footnote(content) => {
            // Inline footnote marker
            out.push_str("[*]");
        }
        Node::Ref(label) => {
            out.push_str("[ref]");
        }
        Node::Citation(key) => {
            out.push('[');
            out.push_str(key);
            out.push(']');
        }
        Node::Label(_) => {}
        Node::Code(s) => out.push_str(s),
        _ => {}
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
            out.push_str(s);
            out.push(' ');
        }
        MathNode::Text(s) => out.push_str(s),
        MathNode::Symbol(s) => out.push_str(s),
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
            out.push('^');
            if nodes.len() > 1 { out.push('{'); }
            math_to_text_buf(nodes, out);
            if nodes.len() > 1 { out.push('}'); }
        }
        MathNode::Sub(nodes) => {
            out.push('_');
            if nodes.len() > 1 { out.push('{'); }
            math_to_text_buf(nodes, out);
            if nodes.len() > 1 { out.push('}'); }
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
