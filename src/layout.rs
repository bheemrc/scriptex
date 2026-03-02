/// Layout engine: converts document AST to positioned page elements
/// Direct layout without intermediate format for maximum speed

use anyhow::Result;
use std::collections::HashMap;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle, wrap_text};
use crate::math_layout;
use crate::font::{self, FontId};

/// Compute baselineskip factor matching pdflatex defaults for standard font sizes.
#[inline]
fn baselineskip_factor(font_size: f32) -> f32 {
    // pdflatex: 10pt→12pt(1.2), 11pt→13.6pt(1.236), 12pt→14.5pt(1.208)
    let fs_int = (font_size + 0.5) as u32;
    match fs_int {
        11 => 1.236,
        12 => 1.208,
        _ => 1.2,
    }
}

/// Pre-scan AST to collect label→display-number mappings for \ref resolution.
/// This avoids a full two-pass layout while still resolving cross-references.
struct LabelCollector<'a> {
    labels: HashMap<String, String>,
    citations: HashMap<String, u32>,
    fig_counter: u32,
    tbl_counter: u32,
    bib_counter: u32,
    eq_counter: u32,
    sec_counters: [u32; 7],
    theorem_counters: HashMap<String, u32>,
    /// Current "pending" label number — set by theorem/equation context, consumed by Label node
    pending_number: Option<String>,
    /// If true, pending_number should default to current section number (lazy computation)
    pending_is_section: bool,
    theorem_defs: &'a [TheoremDef],
}

fn collect_labels(nodes: &[Node], doc: &Document) -> (HashMap<String, String>, HashMap<String, u32>) {
    let mut ctx = LabelCollector {
        labels: HashMap::new(),
        citations: HashMap::new(),
        fig_counter: 0,
        tbl_counter: 0,
        bib_counter: 0,
        eq_counter: 0,
        sec_counters: [0u32; 7],
        theorem_counters: HashMap::new(),
        pending_number: None,
        pending_is_section: false,
        theorem_defs: &doc.preamble.theorem_defs,
    };
    collect_labels_inner(nodes, &mut ctx);
    (ctx.labels, ctx.citations)
}

impl LabelCollector<'_> {
    fn current_section_str(&self) -> String {
        if self.sec_counters[4] > 0 {
            format!("{}.{}.{}", self.sec_counters[2], self.sec_counters[3], self.sec_counters[4])
        } else if self.sec_counters[3] > 0 {
            format!("{}.{}", self.sec_counters[2], self.sec_counters[3])
        } else if self.sec_counters[2] > 0 {
            format!("{}", self.sec_counters[2])
        } else {
            "??".to_string()
        }
    }
}

fn collect_labels_inner(nodes: &[Node], ctx: &mut LabelCollector) {
    for node in nodes {
        match node {
            Node::Section { level, numbered, title } => {
                if *numbered {
                    let idx = (level.depth() + 1).max(0) as usize;
                    if idx < ctx.sec_counters.len() {
                        ctx.sec_counters[idx] += 1;
                        for i in (idx + 1)..ctx.sec_counters.len() {
                            ctx.sec_counters[i] = 0;
                        }
                    }
                    // Reset theorem counters on new section
                    if idx <= 2 {
                        ctx.theorem_counters.clear();
                    }
                    ctx.pending_number = None;
                    ctx.pending_is_section = true;
                }
                // Also traverse title for any embedded labels
                collect_labels_inner(title, ctx);
            }
            Node::Figure(fig) => {
                if fig.caption.is_some() {
                    ctx.fig_counter += 1;
                    if let Some(ref lbl) = fig.label {
                        ctx.labels.insert(lbl.clone(), ctx.fig_counter.to_string());
                    }
                }
                collect_labels_inner(&fig.content, ctx);
            }
            Node::Table(table) => {
                if table.caption.is_some() {
                    ctx.tbl_counter += 1;
                    if let Some(ref lbl) = table.label {
                        ctx.labels.insert(lbl.clone(), ctx.tbl_counter.to_string());
                    }
                }
            }
            Node::BibItem(key) => {
                ctx.bib_counter += 1;
                ctx.citations.insert(key.clone(), ctx.bib_counter);
            }
            Node::Theorem(thm) => {
                // Compute theorem number the same way layout_theorem does
                let counter_name = if let Some(def) = ctx.theorem_defs.iter()
                    .find(|d| d.env_name == thm.env_name)
                {
                    def.counter.clone().unwrap_or_else(|| thm.env_name.clone())
                } else {
                    thm.env_name.clone()
                };
                let count = ctx.theorem_counters.entry(counter_name).or_insert(0);
                *count += 1;
                let num = *count;
                let sec = ctx.sec_counters[2];
                let thm_label = if sec > 0 {
                    format!("{}.{}", sec, num)
                } else {
                    format!("{}", num)
                };
                ctx.pending_number = Some(thm_label);
                ctx.pending_is_section = false;
                // Traverse theorem body for labels
                collect_labels_inner(&thm.body, ctx);
            }
            Node::Proof(content) => {
                collect_labels_inner(content, ctx);
            }
            Node::DisplayMath(math_data) => {
                if math_data.numbered {
                    ctx.eq_counter += 1;
                    let eq_label = format!("{}", ctx.eq_counter);
                    ctx.pending_number = Some(eq_label);
                    ctx.pending_is_section = false;
                }
                // Check for labels in math nodes
                collect_math_labels(&math_data.nodes, ctx);
            }
            Node::Label(name) => {
                let num = if let Some(ref pending) = ctx.pending_number {
                    pending.clone()
                } else if ctx.pending_is_section {
                    ctx.current_section_str()
                } else {
                    ctx.current_section_str()
                };
                ctx.labels.insert(name.clone(), num);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) => {
                for item in items {
                    collect_labels_inner(&item.content, ctx);
                }
            }
            Node::Environment(env) => {
                collect_labels_inner(&env.content, ctx);
            }
            Node::Paragraph(c) | Node::Quote(c) | Node::Quotation(c) | Node::Abstract(c)
            | Node::Center(c) | Node::FlushLeft(c) | Node::FlushRight(c)
            | Node::Bold(c) | Node::Italic(c) | Node::Group(c) | Node::SmallCaps(c)
            | Node::Footnote(c) | Node::Colored { content: c, .. }
            | Node::Minipage { content: c, .. } => {
                collect_labels_inner(c, ctx);
            }
            _ => {}
        }
    }
}

/// Check math nodes for embedded labels — math nodes don't contain labels directly,
/// but this exists for potential future use with \tag{} labels.
fn collect_math_labels(_nodes: &[MathNode], _ctx: &mut LabelCollector) {
    // Math nodes don't contain Node::Label — labels in display math are
    // parsed at the Node level, adjacent to the DisplayMath node.
}

/// Table of contents entry
struct TocEntry {
    level: SectionLevel,
    number: String,
    title: String,
    page: u32, // filled in during layout (0 = unknown)
}

/// TOC fixup: position where a page number should be stamped after layout
struct TocFixup {
    elem_idx: u32,     // index into all_elements
    text_offset: u32,  // offset into all_text where "   " placeholder was written
    toc_idx: u32,      // index into toc_entries
}

/// Pre-scan AST to collect section entries for table of contents
fn collect_toc_entries(nodes: &[Node], source: &str) -> Vec<TocEntry> {
    let mut entries = Vec::new();
    let mut counters = [0u32; 7];
    let mut appendix = false;
    collect_toc_inner(nodes, &mut entries, &mut counters, &mut appendix, source);
    entries
}

fn collect_toc_inner(nodes: &[Node], entries: &mut Vec<TocEntry>, counters: &mut [u32; 7], appendix: &mut bool, source: &str) {
    for node in nodes {
        match node {
            Node::Appendix => {
                *appendix = true;
                counters[2] = 0;
                counters[3] = 0;
                counters[4] = 0;
            }
            Node::Section { level, title, numbered } => {
                let mut number = String::new();
                if *numbered {
                    let idx = (level.depth() + 1).max(0) as usize;
                    if idx < counters.len() {
                        counters[idx] += 1;
                        for i in (idx + 1)..counters.len() {
                            counters[i] = 0;
                        }
                    }
                    let mut ibuf = itoa::Buffer::new();
                    match level {
                        SectionLevel::Part => {
                            number.push_str("Part ");
                            number.push_str(ibuf.format(counters[0]));
                        }
                        SectionLevel::Chapter => {
                            number.push_str(ibuf.format(counters[1]));
                        }
                        SectionLevel::Section => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                        }
                        SectionLevel::Subsection => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                            number.push('.');
                            number.push_str(ibuf.format(counters[3]));
                        }
                        SectionLevel::Subsubsection => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                            number.push('.');
                            number.push_str(ibuf.format(counters[3]));
                            number.push('.');
                            number.push_str(ibuf.format(counters[4]));
                        }
                        _ => {}
                    }
                }
                // Only include section/subsection/subsubsection in TOC (skip paragraph/subparagraph)
                if level.depth() <= 3 {
                    let mut title_text = String::new();
                    for n in title {
                        node_to_text(n, &mut title_text, source);
                    }
                    entries.push(TocEntry {
                        level: *level,
                        number,
                        title: title_text,
                        page: 0,
                    });
                }
            }
            Node::Paragraph(c) | Node::Group(c) => {
                collect_toc_inner(c, entries, counters, appendix, source);
            }
            _ => {}
        }
    }
}

/// A clickable link annotation on a page
#[derive(Debug, Clone)]
pub struct LinkAnnotation {
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub url: String,
}

/// A PDF bookmark/outline entry
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub title: String,
    pub page: u32,
    pub y: f32,
    pub level: i32, // section depth
}

/// Flat storage for all laid-out pages - eliminates per-page allocations
#[derive(Debug)]
pub struct LayoutResult {
    pub all_elements: Vec<PageElement>,
    pub all_text: String,
    pub rect_data: Vec<RectData>,
    pub images: Vec<EmbeddedImage>,
    pub links: Vec<LinkAnnotation>,
    pub outlines: Vec<OutlineEntry>,
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
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        image_idx: u32, // index into LayoutResult.images
    },
}

/// Embedded image data for PDF generation
#[derive(Debug, Clone)]
pub struct EmbeddedImage {
    pub data: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PageStyle {
    Plain,    // centered page number at bottom, no header
    Headings, // section title + page number in header, no footer
    Empty,    // no header or footer
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
    images: Vec<EmbeddedImage>,
    page_bounds: Vec<PageBounds>,
    // Current page boundary tracking
    current_page_elem_start: u32,
    current_page_text_start: u32,
    page_number: u32,
    indent: f32,
    right_indent: f32,
    paragraph_indent: f32,
    line_spacing: f32,
    section_counters: [u32; 7],
    figure_counter: u32,
    table_counter: u32,
    footnotes: Vec<Vec<Node>>,
    footnote_counter: u32,
    footnote_reserved: f32, // height reserved at bottom of page for footnotes
    suppress_next_indent: bool,
    list_depth: u32,
    text_buf: String,
    label_map: HashMap<String, String>,
    citation_map: HashMap<String, u32>,
    equation_counter: u32,
    theorem_counters: HashMap<String, u32>,
    current_section_num: u32,
    appendix_mode: bool,
    toc_entries: Vec<TocEntry>,
    links: Vec<LinkAnnotation>,
    outlines: Vec<OutlineEntry>,
    source_ptr: *const u8,
    source_len: usize,
    toc_fixups: Vec<TocFixup>,
    toc_section_idx: u32, // tracks which toc_entry we're on during layout_section
    page_style: PageStyle, // plain, headings, empty
    current_section_title: String, // for running headers
    first_page: bool, // first page always uses plain style
    is_amsart: bool, // amsart class: centered small-caps headings, different title style
    amsart_header_author: String, // uppercase author for amsart even-page headers
    amsart_header_title: String, // uppercase short title for amsart odd-page headers
    deferred_abstract_idx: Option<usize>, // index into body nodes for deferred abstract (amsart)
    amsart_pre_title: bool, // suppress content before \maketitle in amsart
}

impl LayoutState {
    fn new(page_setup: PageSetup, font_size: f32, line_spacing: f32) -> Self {
        let max_y = page_setup.height - page_setup.margin_bottom - page_setup.footer_height;
        let start_y = page_setup.margin_top + page_setup.header_height;
        let text_w = page_setup.text_width();
        let avg_w = font_size * 0.48; // Regular
        let lh = font_size * baselineskip_factor(font_size);
        let st = lh * line_spacing;
        let para_w = text_w - 17.0; // paragraph_indent = 17.0
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
            images: Vec::new(),
            page_bounds: Vec::with_capacity(51000),
            current_page_elem_start: 0,
            current_page_text_start: 0,
            page_number: 1,
            indent: 0.0,
            right_indent: 0.0,
            paragraph_indent: 17.0, // 1.5em for 11pt (pdflatex default)
            line_spacing,
            section_counters: [0; 7],
            figure_counter: 0,
            table_counter: 0,
            footnotes: Vec::new(),
            footnote_counter: 0,
            footnote_reserved: 0.0,
            suppress_next_indent: false,
            list_depth: 0,
            text_buf: String::with_capacity(4096),
            label_map: HashMap::new(),
            citation_map: HashMap::new(),
            equation_counter: 0,
            theorem_counters: HashMap::new(),
            current_section_num: 0,
            appendix_mode: false,
            toc_entries: Vec::new(),
            links: Vec::new(),
            outlines: Vec::new(),
            source_ptr: std::ptr::null(),
            source_len: 0,
            toc_fixups: Vec::new(),
            toc_section_idx: 0,
            page_style: PageStyle::Plain,
            current_section_title: String::new(),
            first_page: true,
            is_amsart: false,
            amsart_header_author: String::new(),
            amsart_header_title: String::new(),
            deferred_abstract_idx: None,
            amsart_pre_title: false,
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
            self.cached_line_height = fs * baselineskip_factor(self.base_font_size);
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
        self.cached_text_width = self.page_setup.text_width() - indent - self.right_indent;
        self.cached_text_left = self.page_setup.margin_left + indent;
        // Invalidate wrap cache since max_chars depends on text_width
        self.cached_font_key = u32::MAX;
    }

    #[inline(always)]
    fn set_right_indent(&mut self, right_indent: f32) {
        self.right_indent = right_indent;
        self.cached_text_width = self.page_setup.text_width() - self.indent - right_indent;
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

    fn source_str(&self) -> &str {
        if self.source_ptr.is_null() { return ""; }
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.source_ptr, self.source_len)) }
    }

    fn reserve_footnote_space(&mut self) {
        // Reserve space at bottom of page for each footnote
        let fn_size = self.base_font_size * 0.8;
        let fn_line_height = fn_size * 1.3;
        if self.footnote_reserved == 0.0 {
            // First footnote on this page — reserve separator space too
            self.footnote_reserved = 10.0 + fn_line_height;
        } else {
            self.footnote_reserved += fn_line_height;
        }
        self.cached_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height - self.footnote_reserved;
    }

    fn render_footnotes(&mut self) {
        if self.footnotes.is_empty() { return; }

        let footnotes = std::mem::take(&mut self.footnotes);
        let fn_start_num = self.footnote_counter - footnotes.len() as u32 + 1;
        let fn_size = self.base_font_size * 0.8;
        let fn_line_height = fn_size * 1.3;

        // Calculate total footnote height
        let total_height = footnotes.len() as f32 * fn_line_height + 10.0; // 10.0 for separator

        // Position footnotes at bottom of page (use original max_y before footnote reservation)
        let orig_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
        let fn_y_start = orig_max_y - total_height;

        // Only render if there's room (don't overlap with content)
        if fn_y_start < self.current_y + 20.0 {
            // Not enough room — skip for this page, re-accumulate
            self.footnotes = footnotes;
            return;
        }

        // Separator line
        self.emit_line(
            self.page_setup.margin_left,
            fn_y_start,
            self.page_setup.margin_left + self.page_setup.text_width() * 0.3,
            fn_y_start,
            0.4,
            Color::GRAY,
        );

        // Render each footnote
        let source = self.source_str() as *const str;
        let source_ref = unsafe { &*source };
        let mut y = fn_y_start + 6.0;
        for (i, fn_content) in footnotes.iter().enumerate() {
            let num = fn_start_num + i as u32;
            let num_str = format!("{}  ", num);
            let x = self.page_setup.margin_left;

            // Superscript number
            let sup_size = fn_size * 0.75;
            self.current_x = x;
            self.current_y = y;
            self.emit_text(&num_str, sup_size, FontStyle::Regular, Color::BLACK);

            // Footnote text — extract as plain text
            let mut fn_text = String::new();
            for node in fn_content {
                node_to_text(node, &mut fn_text, source_ref);
            }
            let fn_text = fn_text.trim().to_string();
            let text_x = x + font::measure_text(&num_str, FontId::Helvetica, sup_size);
            self.current_x = text_x;
            self.emit_text(&fn_text, fn_size, FontStyle::Regular, Color::BLACK);

            y += fn_line_height;
        }
    }

    fn new_page(&mut self) {
        // Render accumulated footnotes at bottom of page
        self.render_footnotes();

        // Determine effective style: first page always uses plain style
        let effective_style = if self.first_page { PageStyle::Plain } else { self.page_style };

        match effective_style {
            PageStyle::Plain => {
                // Centered page number at bottom
                self.emit_page_number_centered();
            }
            PageStyle::Headings => {
                let header_font_size = 9.0;
                let header_y = self.page_setup.margin_top - 14.0;
                let left_x = self.page_setup.margin_left;
                let right_x = self.page_setup.width - self.page_setup.margin_right;

                // Format page number string
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
                let digit_width = header_font_size * 0.5;
                let num_width = num_len as f32 * digit_width;

                if self.is_amsart {
                    // amsart: even pages = "page#  AUTHOR", odd pages = "SECTION TITLE  page#"
                    let is_even = self.page_number % 2 == 0;
                    if is_even {
                        // Even page: page number left, author right
                        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                        self.all_text.push_str(num_str);
                        self.all_elements.push(PageElement::Text {
                            x: left_x,
                            y: header_y,
                            text_offset: offset,
                            text_len: num_len as u16,
                            font_size_100: (header_font_size * 100.0) as u16,
                            font_style: FontStyle::Regular,
                            color: Color::BLACK,
                            word_spacing_50: 0,
                        });
                        // Author on right (small caps = uppercase at header size)
                        if !self.amsart_header_author.is_empty() {
                            let author: &str = unsafe { &*(self.amsart_header_author.as_str() as *const str) };
                            let author_len = author.len().min(u16::MAX as usize);
                            let author_w = font::measure_text(&author[..author_len], FontId::Helvetica, header_font_size);
                            let author_offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                            self.all_text.push_str(&author[..author_len]);
                            self.all_elements.push(PageElement::Text {
                                x: right_x - author_w,
                                y: header_y,
                                text_offset: author_offset,
                                text_len: author_len as u16,
                                font_size_100: (header_font_size * 100.0) as u16,
                                font_style: FontStyle::SmallCaps,
                                color: Color::BLACK,
                                word_spacing_50: 0,
                            });
                        }
                    } else {
                        // Odd page: paper title left, page number right
                        if !self.amsart_header_title.is_empty() {
                            let title: &str = unsafe { &*(self.amsart_header_title.as_str() as *const str) };
                            self.emit_header_text(title, left_x, header_y, header_font_size, FontStyle::SmallCaps);
                        } else if !self.current_section_title.is_empty() {
                            let title: &str = unsafe { &*(self.current_section_title.as_str() as *const str) };
                            self.emit_header_text(title, left_x, header_y, header_font_size, FontStyle::SmallCaps);
                        }
                        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                        self.all_text.push_str(num_str);
                        self.all_elements.push(PageElement::Text {
                            x: right_x - num_width,
                            y: header_y,
                            text_offset: offset,
                            text_len: num_len as u16,
                            font_size_100: (header_font_size * 100.0) as u16,
                            font_style: FontStyle::Regular,
                            color: Color::BLACK,
                            word_spacing_50: 0,
                        });
                    }
                } else {
                    // Standard article: page number right, section title left (italic)
                    let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                    self.all_text.push_str(num_str);
                    self.all_elements.push(PageElement::Text {
                        x: right_x - num_width,
                        y: header_y,
                        text_offset: offset,
                        text_len: num_len as u16,
                        font_size_100: (header_font_size * 100.0) as u16,
                        font_style: FontStyle::Regular,
                        color: Color::BLACK,
                        word_spacing_50: 0,
                    });
                    if !self.current_section_title.is_empty() {
                        let title: &str = unsafe { &*(self.current_section_title.as_str() as *const str) };
                        self.emit_header_text(title, left_x, header_y, header_font_size, FontStyle::Italic);
                    }
                }

                // Thin rule below header (amsart doesn't use this)
                if !self.is_amsart {
                    let rule_y = header_y + 4.0;
                    self.all_elements.push(PageElement::Line {
                        x1: left_x,
                        y1: rule_y,
                        x2: right_x,
                        y2: rule_y,
                        width_1000: 400, // 0.4pt
                        color: Color::BLACK,
                    });
                }
            }
            PageStyle::Empty => {
                // No header or footer
            }
        }

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
        self.first_page = false;
        self.current_x = self.text_left();
        self.current_y = self.cached_start_y;
        // Reset footnote reservation and restore max_y for new page
        self.footnote_reserved = 0.0;
        self.cached_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
    }

    /// Emit centered page number at bottom of page (for plain style)
    fn emit_page_number_centered(&mut self) {
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

    /// Emit header text with automatic Symbol font switching for Greek characters.
    fn emit_header_text(&mut self, text: &str, x: f32, y: f32, font_size: f32, base_style: FontStyle) {
        let font_size_100 = (font_size * 100.0) as u16;
        // Fast path: all ASCII — no Greek chars possible
        if text.bytes().all(|b| b < 0x80) {
            let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
            self.all_text.push_str(text);
            self.all_elements.push(PageElement::Text {
                x, y, text_offset: offset, text_len: text.len().min(65535) as u16,
                font_size_100, font_style: base_style, color: Color::BLACK, word_spacing_50: 0,
            });
            return;
        }
        // Split at Greek characters → emit in Symbol font
        let mut cur_x = x;
        let mut seg_start = 0;
        for (i, ch) in text.char_indices() {
            if (ch as u32) >= 0x0391 && (ch as u32) <= 0x03C9 {
                if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
                    // Emit preceding non-Greek segment
                    if i > seg_start {
                        let seg = &text[seg_start..i];
                        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                        self.all_text.push_str(seg);
                        let w = font::measure_text(seg, FontId::Helvetica, font_size);
                        self.all_elements.push(PageElement::Text {
                            x: cur_x, y, text_offset: offset, text_len: seg.len().min(65535) as u16,
                            font_size_100, font_style: base_style, color: Color::BLACK, word_spacing_50: 0,
                        });
                        cur_x += w;
                    }
                    // Emit Greek char in Symbol font
                    let sym_char = char::from(sym_byte);
                    let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                    self.all_text.push(sym_char);
                    let w = font::char_width_pt(FontId::Symbol, sym_byte, font_size);
                    self.all_elements.push(PageElement::Text {
                        x: cur_x, y, text_offset: offset, text_len: sym_char.len_utf8() as u16,
                        font_size_100, font_style: FontStyle::Symbol, color: Color::BLACK, word_spacing_50: 0,
                    });
                    cur_x += w;
                    seg_start = i + ch.len_utf8();
                }
            }
        }
        // Emit remaining text
        if seg_start < text.len() {
            let seg = &text[seg_start..];
            let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
            self.all_text.push_str(seg);
            self.all_elements.push(PageElement::Text {
                x: cur_x, y, text_offset: offset, text_len: seg.len().min(65535) as u16,
                font_size_100, font_style: base_style, color: Color::BLACK, word_spacing_50: 0,
            });
        }
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

    // Detect amsart class
    let is_ams = match &doc.class.class_type {
        ClassType::Custom(s) => s == "amsart" || s == "amsbook" || s == "amsproc",
        _ => false,
    };
    if is_ams {
        state.is_amsart = true;
        state.amsart_pre_title = true; // suppress content before \maketitle
        // amsart uses "headings" style by default
        state.page_style = PageStyle::Headings;
        state.page_setup.header_height = 20.0;
        // amsart default margins: ~1in left/right, ~1.25in top, ~1.25in bottom on a4
        state.page_setup.margin_left = 72.0;
        state.page_setup.margin_right = 72.0;
        state.page_setup.margin_top = 72.0;
        state.page_setup.margin_bottom = 72.0;
        state.page_setup.footer_height = 14.0;
        // Recalculate cached values
        state.cached_text_width = state.page_setup.text_width();
        state.cached_max_y = state.page_setup.height - state.page_setup.margin_bottom - state.page_setup.footer_height;
        state.cached_start_y = state.page_setup.margin_top + state.page_setup.header_height;
        state.current_y = state.cached_start_y;
        state.current_x = state.page_setup.margin_left;
        // Set amsart running header text: author (even pages), title (odd pages)
        if let Some(ref author) = doc.preamble.author {
            state.amsart_header_author = author.to_uppercase();
        }
        if let Some(ref title) = doc.preamble.title {
            state.amsart_header_title = title.to_uppercase();
        }
    }

    // Set page style from preamble
    match doc.preamble.page_style.as_str() {
        "headings" => {
            state.page_style = PageStyle::Headings;
            // Increase header_height to make room for running header
            state.page_setup.header_height = 20.0;
            // Recalculate cached start_y
            state.cached_start_y = state.page_setup.margin_top + state.page_setup.header_height;
            state.current_y = state.cached_start_y;
        }
        "empty" => {
            state.page_style = PageStyle::Empty;
        }
        _ => {} // "plain" or default
    }

    // Store source reference for footnote rendering
    state.source_ptr = source.as_ptr();
    state.source_len = source.len();

    // Pre-scan for label→number mappings (fast O(n) AST walk)
    let (labels, citations) = collect_labels(&doc.body, doc);
    state.label_map = labels;
    state.citation_map = citations;

    // Pre-scan sections for table of contents
    state.toc_entries = collect_toc_entries(&doc.body, source);

    // Layout body
    layout_nodes(&doc.body, &mut state, doc, source)?;

    // Render author addresses at end of document (amsart style)
    if !doc.preamble.addresses.is_empty() {
        let font_size = doc.preamble.font_size;
        let step = font_size * doc.preamble.line_spacing * 1.2;
        let small_size = font_size * 0.85;
        // Add vertical space before addresses (like amsart)
        state.current_y += step * 2.0;
        state.current_x = state.text_left();
        for addr_info in &doc.preamble.addresses {
            state.ensure_space(step * 3.0);
            // Address in small caps (amsart style)
            state.text_buf.clear();
            state.text_buf.push_str(&addr_info.address);
            let text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(text, small_size, FontStyle::SmallCaps, Color::BLACK);
            state.current_y += step;
            state.current_x = state.text_left();
            // Email in italic
            if let Some(email) = &addr_info.email {
                state.text_buf.clear();
                state.text_buf.push_str("Email address: ");
                state.text_buf.push_str(email);
                let text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
                state.emit_text(text, small_size, FontStyle::Italic, Color::BLACK);
                state.current_y += step * 1.5;
                state.current_x = state.text_left();
            }
        }
    }

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

    // Fixup TOC page numbers now that we know where each section landed
    for fixup in &state.toc_fixups {
        let toc_idx = fixup.toc_idx as usize;
        if toc_idx < state.toc_entries.len() {
            let page = state.toc_entries[toc_idx].page;
            if page > 0 {
                let offset = fixup.text_offset as usize;
                // Overwrite the 3-byte placeholder "   " with right-aligned page number
                let mut buf = [b' '; 3];
                let mut ibuf = itoa::Buffer::new();
                let s = ibuf.format(page);
                let s_bytes = s.as_bytes();
                // Right-align: put number at end of 3-char field
                let start = 3usize.saturating_sub(s_bytes.len());
                for (i, &b) in s_bytes.iter().enumerate() {
                    if start + i < 3 {
                        buf[start + i] = b;
                    }
                }
                // SAFETY: we're replacing ASCII spaces with ASCII digits
                let text_bytes = unsafe { state.all_text.as_bytes_mut() };
                if offset + 3 <= text_bytes.len() {
                    text_bytes[offset] = buf[0];
                    text_bytes[offset + 1] = buf[1];
                    text_bytes[offset + 2] = buf[2];
                }
            }
        }
    }

    Ok(LayoutResult {
        all_elements: state.all_elements,
        all_text: state.all_text,
        rect_data: state.rect_data,
        images: state.images,
        links: state.links,
        outlines: state.outlines,
        page_bounds: state.page_bounds,
        width: state.page_setup.width,
        height: state.page_setup.height,
    })
}

/// High bit flag indicating text_offset refers to source (mmap) rather than page text_buffer
pub const SOURCE_REF_FLAG: u32 = 0x80000000;

/// Check if a node is an inline element (should be inside a Paragraph)
fn is_inline_node(node: &Node) -> bool {
    matches!(node,
        Node::Text(_) | Node::TextRef(_, _) | Node::Bold(_) | Node::Italic(_)
        | Node::Monospace(_) | Node::SmallCaps(_) | Node::Underline(_) | Node::Emph(_)
        | Node::InlineMath(_) | Node::Group(_) | Node::Colored { .. }
        | Node::FontSize { .. } | Node::Superscript(_) | Node::Subscript(_)
        | Node::NonBreakingSpace | Node::HSpace(_) | Node::Code(_) | Node::Footnote(_)
        | Node::Citation(..) | Node::Ref(_) | Node::EqRef(_) | Node::Href { .. }
        | Node::FontStyleDecl(_) | Node::ColorDecl(_)
        | Node::EnDash | Node::EmDash | Node::Ellipsis
        | Node::LeftQuote | Node::RightQuote | Node::LeftDoubleQuote | Node::RightDoubleQuote
        | Node::Ampersand | Node::Percent | Node::Dollar | Node::Hash | Node::Underscore
        | Node::Tilde | Node::Caret | Node::LeftBrace | Node::RightBrace
    )
}

/// Group consecutive inline nodes into Paragraph nodes.
/// Block nodes pass through unchanged. This ensures text + inline math flow together.
fn group_inline_nodes(nodes: &[Node]) -> Vec<Node> {
    let mut result = Vec::with_capacity(nodes.len());
    let mut inline_buf: Vec<Node> = Vec::new();

    for node in nodes {
        if is_inline_node(node) {
            inline_buf.push(node.clone());
        } else {
            // Flush accumulated inline nodes as a Paragraph
            if !inline_buf.is_empty() {
                if inline_buf.len() == 1 {
                    // Single inline node — push as-is (let layout handle it)
                    result.push(inline_buf.remove(0));
                } else {
                    result.push(Node::Paragraph(std::mem::take(&mut inline_buf)));
                }
                inline_buf.clear();
            }
            result.push(node.clone());
        }
    }

    // Flush remaining
    if !inline_buf.is_empty() {
        if inline_buf.len() == 1 {
            result.push(inline_buf.remove(0));
        } else {
            result.push(Node::Paragraph(inline_buf));
        }
    }

    result
}

fn layout_nodes(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Pre-process: group consecutive loose inline nodes into Paragraph nodes.
    // This handles the case where the parser produces [Text, InlineMath, Text, ...]
    // at top level without wrapping them in a Paragraph.
    // Fast-path: skip scan if first node is a block element (covers doc.body with 50K+ sections)
    let needs_scan = nodes.first().map_or(false, |n| !matches!(n,
        Node::Section { .. } | Node::TextParagraph(_, _) | Node::Paragraph(_)
        | Node::DisplayMath(_) | Node::Table(_) | Node::Figure(_)
        | Node::ItemizeList(_) | Node::EnumerateList(_) | Node::DescriptionList(_)
        | Node::Environment(_) | Node::MakeTitle | Node::TableOfContents
    ));
    if needs_scan {
        let has_loose_inlines = nodes.iter().any(|n| matches!(n,
            Node::InlineMath(_) | Node::Bold(_) | Node::Italic(_) | Node::Emph(_)
            | Node::Colored { .. } | Node::Code(_) | Node::SmallCaps(_)
            | Node::Underline(_) | Node::Footnote(_) | Node::FontStyleDecl(_) | Node::ColorDecl(_)
            | Node::Citation(..) | Node::Ref(_) | Node::EqRef(_) | Node::Href { .. }
            | Node::NonBreakingSpace
        ));
        if has_loose_inlines {
            let grouped = group_inline_nodes(nodes);
            return layout_nodes_inner(&grouped, state, doc, source);
        }
    }

    layout_nodes_inner(nodes, state, doc, source)
}

fn layout_nodes_inner(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
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
        // amsart: suppress everything before \maketitle (keywords, MSC, etc.)
        if state.amsart_pre_title {
            match node {
                Node::MakeTitle => { /* fall through to layout_node */ }
                Node::Abstract(_) => { /* fall through — sets deferred flag */ }
                _ => continue,
            }
            layout_node(node, state, doc, source)?;
            continue;
        }

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
            state.amsart_pre_title = false; // allow content after \maketitle
            layout_title(state, doc, source)?;
        }

        Node::TableOfContents => {
            layout_table_of_contents(state)?;
        }

        Node::Appendix => {
            // Switch to appendix mode: reset section counter, use A, B, C numbering
            state.appendix_mode = true;
            state.section_counters[2] = 0; // Reset section counter
            state.section_counters[3] = 0;
            state.section_counters[4] = 0;
        }

        Node::NoIndent => {
            state.suppress_next_indent = true;
        }

        Node::ItemizeList(items) => {
            layout_list(items, state, doc, false, source)?;
            state.suppress_next_indent = true;
        }

        Node::EnumerateList(items) => {
            layout_list(items, state, doc, true, source)?;
            state.suppress_next_indent = true;
        }

        Node::DescriptionList(items) => {
            layout_description_list(items, state, doc, source)?;
            state.suppress_next_indent = true;
        }

        Node::Table(table) => {
            layout_table(table, state, doc, source)?;
            state.suppress_next_indent = true;
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
            state.suppress_next_indent = true;
        }

        Node::Image(img) => {
            // Try to load the actual image file
            let img_loaded = load_image_for_pdf(&img.path, state);

            if let Some((embedded, native_w, native_h)) = img_loaded {
                // Determine display dimensions
                let (img_w, img_h) = if let Some(w) = img.width {
                    if let Some(h) = img.height {
                        (w, h)
                    } else {
                        let ratio = native_h as f32 / native_w as f32;
                        (w, w * ratio)
                    }
                } else if let Some(h) = img.height {
                    let ratio = native_w as f32 / native_h as f32;
                    (h * ratio, h)
                } else if let Some(scale) = img.scale {
                    (native_w as f32 * scale, native_h as f32 * scale)
                } else {
                    // Default: use native size in points (assume 72 DPI)
                    (native_w as f32, native_h as f32)
                };

                // Cap width to text width
                let (img_w, img_h) = if img_w > state.text_width() {
                    let scale = state.text_width() / img_w;
                    (state.text_width(), img_h * scale)
                } else {
                    (img_w, img_h)
                };

                state.ensure_space(img_h + 10.0);

                let image_idx = state.images.len() as u32;
                state.images.push(embedded);

                let x = state.text_left() + (state.text_width() - img_w) / 2.0;
                state.all_elements.push(PageElement::Image {
                    x, y: state.current_y, width: img_w, height: img_h, image_idx,
                });
                state.current_y += img_h + 6.0;
                state.current_x = state.text_left();
            } else {
                // Fallback: placeholder rectangle
                let img_w = img.width.unwrap_or(200.0).min(state.text_width());
                let img_h = img.height.unwrap_or(150.0);
                let (img_w, img_h) = if let Some(scale) = img.scale {
                    (img_w * scale, img_h * scale)
                } else { (img_w, img_h) };
                let (img_w, img_h) = if img_w > state.text_width() {
                    let s = state.text_width() / img_w;
                    (state.text_width(), img_h * s)
                } else { (img_w, img_h) };

                state.ensure_space(img_h + 10.0);
                let x = state.text_left() + (state.text_width() - img_w) / 2.0;
                state.emit_rect(x, state.current_y, img_w, img_h,
                    Some(Color::rgb(0.95, 0.95, 0.95)), Some(Color::LIGHT_GRAY));
                let label = format!("[Image: {}]", img.path);
                let tw = font::measure_text(&label, FontId::Helvetica, 8.0);
                let cx = x + (img_w - tw) / 2.0;
                state.current_x = cx;
                state.emit_text(&label, 8.0, FontStyle::Italic, Color::GRAY);
                state.current_y += img_h + 6.0;
                state.current_x = state.text_left();
            }
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

        Node::DisplayMath(math_data) => {
            layout_display_math_data(math_data, state)?;
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
            state.suppress_next_indent = true;
        }

        Node::Abstract(content) => {
            if state.is_amsart {
                // amsart: skip abstract here — it will be rendered by \maketitle handler
                // Just mark that we have one
                state.deferred_abstract_idx = Some(1);
            } else {
                // Standard article: centered "Abstract" heading
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
            }
            state.add_vertical_space(10.0);
        }

        Node::Center(content) => {
            // For centered content, we layout normally but center each text element
            layout_centered(content, state, doc, source)?;
            state.suppress_next_indent = true;
        }

        Node::FlushLeft(content) => {
            layout_nodes(content, state, doc, source)?;
            state.suppress_next_indent = true;
        }

        Node::FlushRight(content) => {
            layout_flush_right(content, state, doc, source)?;
            state.suppress_next_indent = true;
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
            state.suppress_next_indent = true;
        }

        Node::Environment(env) => {
            if env.name == "thebibliography" {
                layout_bibliography(&env.content, state, doc, source)?;
            } else {
                layout_nodes(&env.content, state, doc, source)?;
            }
        }

        Node::Theorem(thm_data) => {
            layout_theorem(thm_data, state, doc, source)?;
        }

        Node::Proof(content) => {
            layout_proof(content, state, doc, source)?;
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

        // Font style declarations (e.g. \bfseries, \itshape) change state for subsequent content
        Node::FontStyleDecl(decl) => {
            state.current_font_style = match decl {
                FontDeclType::Bold => FontStyle::Bold,
                FontDeclType::Italic => FontStyle::Italic,
                FontDeclType::Monospace => FontStyle::Monospace,
                FontDeclType::Regular => FontStyle::Regular,
                FontDeclType::SmallCaps => FontStyle::Regular, // approximate with regular
            };
        }

        Node::ColorDecl(c) => {
            state.current_color = *c;
        }

        // Inline elements at top level — should not appear here but handle gracefully.
        // Accumulated into groups by layout_nodes_grouped() wrapper.
        Node::Footnote(content) => {
            // Emit superscript number and store footnote for bottom-of-page rendering
            state.footnote_counter += 1;
            let num = state.footnote_counter;
            let num_str = format!("{}", num);
            let sup_size = state.current_font_size * 0.65;
            let saved_y = state.current_y;
            state.current_y -= state.current_font_size * 0.35;
            state.emit_text(&num_str, sup_size, FontStyle::Regular, Color::BLACK);
            state.current_y = saved_y;
            state.current_x += font::measure_text(&num_str, FontId::Helvetica, sup_size);
            state.footnotes.push(content.clone());
            state.reserve_footnote_space();
        }

        Node::Text(_) | Node::TextRef(_, _) | Node::Bold(_) | Node::Italic(_) | Node::Monospace(_)
        | Node::SmallCaps(_) | Node::Underline(_) | Node::Emph(_)
        | Node::InlineMath(_) | Node::Group(_) | Node::Colored { .. }
        | Node::FontSize { .. } | Node::Superscript(_) | Node::Subscript(_)
        | Node::NonBreakingSpace | Node::HSpace(_) | Node::LineBreak
        | Node::Code(_) => {
            // This should be handled by the grouping wrapper, but as fallback:
            layout_paragraph(&[node.clone()], state, doc, source)?;
        }

        Node::Citation(key, opt) => {
            // Resolve citation(s) — key may contain comma-separated keys
            let cite_text = resolve_citations(key, opt.as_deref(), &state.citation_map);
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

        Node::EqRef(label) => {
            // Equation reference — renders as (N)
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                format!("({})", resolved)
            } else {
                "(??)".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::Helvetica, state.current_font_size);
        }

        Node::Href { url, content } => {
            // Render link content in blue with underline
            let link_color = Color::from_rgb_u8(0, 0, 180);
            let saved_color = state.current_color;
            state.current_color = link_color;
            let start_x = state.current_x;
            let start_y = state.current_y;
            for child in content {
                layout_node(child, state, doc, source)?;
            }
            let end_x = state.current_x;
            // Underline
            let underline_y = start_y + state.current_font_size * 0.15;
            state.emit_line(start_x, underline_y, end_x, underline_y, 0.3, link_color);
            // Register link annotation
            let page = state.page_bounds.len() as u32;
            state.links.push(LinkAnnotation {
                page,
                x: start_x,
                y: start_y - state.current_font_size * 0.8,
                width: end_x - start_x,
                height: state.current_font_size * 1.2,
                url: url.clone(),
            });
            state.current_color = saved_color;
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

fn layout_title(state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    if state.is_amsart {
        return layout_title_amsart(state, doc, source);
    }

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

fn layout_title_amsart(state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(60.0);

    // amsart title: uppercase bold centered
    if let Some(title) = &doc.preamble.title {
        let size = state.base_font_size * 1.2;
        let upper_title = title.to_uppercase();
        let metrics = FontMetrics::new(size, FontStyle::Bold);

        let segments: Vec<&str> = upper_title.split("\\\\").collect();
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

        state.add_vertical_space(16.0);
    }

    // amsart author: small caps centered (we approximate with uppercase at smaller size)
    if let Some(author) = &doc.preamble.author {
        let size = state.base_font_size;
        let upper_author = author.to_uppercase();
        let metrics = FontMetrics::new(size, FontStyle::Regular);
        // Split on \and or \\
        let parts: Vec<&str> = upper_author.split("\\AND").collect();
        for part in &parts {
            let part = part.trim();
            if part.is_empty() { continue; }
            let tw = metrics.measure_text(part);
            let cx = state.text_left() + (state.text_width() - tw) / 2.0;
            state.ensure_space(metrics.line_height());
            state.current_x = cx;
            state.emit_text(part, size, FontStyle::Regular, Color::BLACK);
            state.current_y += metrics.line_height();
        }
        state.add_vertical_space(10.0);
    }

    // Render deferred abstract if present (from body, before \maketitle)
    if state.deferred_abstract_idx.is_some() {
        // Find the Abstract node in the body
        for node in &doc.body {
            if let Node::Abstract(content) = node {
                state.add_vertical_space(6.0);
                let saved_indent = state.indent;
                let saved_right = state.right_indent;
                state.set_right_indent(36.0);
                state.set_indent(state.indent + 36.0);
                state.current_x = state.text_left();
                let saved_size = state.current_font_size;
                let abs_size = state.base_font_size * 0.9;
                state.current_font_size = abs_size;
                // Emit "Abstract." prefix in small caps
                let prefix = "Abstract. ";
                let prefix_w = font::measure_text(prefix, FontId::HelveticaBold, abs_size);
                state.emit_text(prefix, abs_size, FontStyle::SmallCaps, Color::BLACK);
                state.current_x += prefix_w;
                layout_nodes(content, state, doc, source)?;
                state.current_font_size = saved_size;
                state.set_right_indent(saved_right);
                state.set_indent(saved_indent);
                state.current_x = state.text_left();
                state.add_vertical_space(6.0);
                break;
            }
        }
        state.deferred_abstract_idx = None;
    }

    // Render first-page footer items (date, MSC, keywords) at bottom of page 1
    // In amsart, these appear as footnote-like items at the bottom of the first page
    {
        let fn_size = state.base_font_size * 0.7;
        let fn_lh = fn_size * 1.4;
        let mut footer_lines: Vec<(String, FontStyle)> = Vec::new();

        if let Some(date) = &doc.preamble.date {
            footer_lines.push((format!("Date: {}.", date.trim_end_matches('.')), FontStyle::Italic));
        }
        if let Some((year, text)) = &doc.preamble.subjclass {
            footer_lines.push((
                format!("{} Mathematics Subject Classification. {}.", year, text.trim_end_matches('.')),
                FontStyle::Italic,
            ));
        }
        if let Some(kw) = &doc.preamble.keywords {
            footer_lines.push((
                format!("Key words and phrases. {}.", kw.trim_end_matches('.')),
                FontStyle::Italic,
            ));
        }

        if !footer_lines.is_empty() {
            // Reserve space at page bottom and render
            let total_h = footer_lines.len() as f32 * fn_lh + 12.0;
            let orig_max_y = state.page_setup.height - state.page_setup.margin_bottom
                - state.page_setup.footer_height;
            let footer_y = orig_max_y - total_h;

            // Reduce available space on this page
            state.cached_max_y = footer_y - 10.0;

            // Separator line
            state.emit_line(
                state.page_setup.margin_left,
                footer_y,
                state.page_setup.margin_left + state.page_setup.text_width() * 0.3,
                footer_y,
                0.4,
                Color::GRAY,
            );

            let text_w = state.page_setup.text_width();
            let mut y = footer_y + 8.0;
            for (text, style) in &footer_lines {
                // Word-wrap the footer line
                let metrics = FontMetrics::new(fn_size, *style);
                let lines = wrap_text(text, &metrics, text_w);
                for line in &lines {
                    state.current_x = state.page_setup.margin_left;
                    state.current_y = y;
                    state.emit_text(line, fn_size, *style, Color::BLACK);
                    y += fn_lh;
                }
            }
        }
    }

    Ok(())
}

fn layout_table_of_contents(state: &mut LayoutState) -> Result<()> {
    let base = state.base_font_size;

    // "Contents" heading
    state.add_vertical_space(10.0);
    let heading_size = base * 1.44;
    state.ensure_space(heading_size * 1.2);
    state.current_x = state.text_left();
    state.emit_text("Contents", heading_size, FontStyle::Bold, Color::BLACK);
    state.current_y += heading_size * 1.2 + 6.0;
    state.emit_line(
        state.text_left(),
        state.current_y,
        state.text_left() + state.text_width(),
        state.current_y,
        0.5,
        Color::BLACK,
    );
    state.current_y += 8.0;

    // Take entries to avoid borrow issues
    let entries = std::mem::take(&mut state.toc_entries);
    let dot_char = ". ";
    let dot_width = font::measure_text(dot_char, FontId::Helvetica, base);
    let page_num_width = font::measure_text("000", FontId::Helvetica, base); // reserve space for up to 3 digits

    // Pre-allocate max dot leader string (reused for all entries)
    let max_dots = 200; // enough for widest line
    let mut dot_leader = String::with_capacity(max_dots * 2);
    for _ in 0..max_dots {
        dot_leader.push('.');
        dot_leader.push(' ');
    }

    // Pre-compute avg char widths for fast estimation
    let avg_width_bold = base * 0.52;
    let avg_width_reg = base * 0.48;

    for (toc_idx, entry) in entries.iter().enumerate() {
        let depth = entry.level.depth();
        // Indent: 0 for section, 15 for subsection, 30 for subsubsection
        let indent = match depth {
            d if d <= 1 => 0.0,
            2 => 15.0,
            3 => 30.0,
            _ => 45.0,
        };
        let font_size = match depth {
            d if d <= 1 => base,
            2 => base * 0.95,
            _ => base * 0.9,
        };
        let style = if depth <= 1 { FontStyle::Bold } else { FontStyle::Regular };
        let line_height = font_size * 1.4;

        state.ensure_space(line_height);
        let x = state.text_left() + indent;
        let right_edge = state.text_left() + state.text_width();

        // Build entry text: "1.2  Title"
        state.text_buf.clear();
        if !entry.number.is_empty() {
            state.text_buf.push_str(&entry.number);
            state.text_buf.push_str("  ");
        }
        state.text_buf.push_str(&entry.title);

        // SAFETY: text_buf not modified during emit_text
        let text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };

        // Fast width estimation instead of per-glyph measurement
        let avg_w = if depth <= 1 { avg_width_bold } else { avg_width_reg };
        let text_w = text.len() as f32 * avg_w * (font_size / base);
        let available = state.text_width() - indent;

        state.current_x = x;
        if text_w <= available - page_num_width - 10.0 {
            state.emit_text(text, font_size, style, Color::BLACK);
            let after_text_x = x + text_w + 4.0;

            // Dot leaders - slice from pre-allocated string
            let dot_start = after_text_x;
            let dot_end = right_edge - page_num_width - 4.0;
            if dot_end > dot_start + dot_width * 2.0 {
                let num_dots = ((dot_end - dot_start) / dot_width) as usize;
                let num_dots = num_dots.min(max_dots);
                let slice_end = (num_dots * 2).min(dot_leader.len());
                state.current_x = dot_start;
                state.emit_text(&dot_leader[..slice_end], base * 0.9, FontStyle::Regular, Color::GRAY);
            }

            // Page number placeholder (will be fixed up after layout)
            let page_x = right_edge - page_num_width;
            state.current_x = page_x;
            // Record fixup position
            let text_offset = state.all_text.len() as u32;
            let elem_idx = state.all_elements.len() as u32;
            state.emit_text("   ", font_size, FontStyle::Regular, Color::BLACK);
            state.toc_fixups.push(TocFixup {
                elem_idx,
                text_offset,
                toc_idx: toc_idx as u32,
            });
        } else {
            // Truncate long titles
            let metrics = FontMetrics::new(font_size, style);
            let truncated_avail = available - page_num_width - 10.0;
            if truncated_avail > 0.0 {
                let lines = wrap_text(text, &metrics, truncated_avail);
                if let Some(first) = lines.first() {
                    state.emit_text(first, font_size, style, Color::BLACK);
                }
            }
        }

        state.current_y += line_height;
        state.current_x = state.text_left();

        // Extra space after section-level entries
        if depth <= 1 {
            state.current_y += 2.0;
        }
    }

    // Restore entries
    state.toc_entries = entries;

    state.add_vertical_space(16.0);
    state.emit_line(
        state.text_left(),
        state.current_y,
        state.text_left() + state.text_width(),
        state.current_y,
        0.3,
        Color::LIGHT_GRAY,
    );
    state.add_vertical_space(12.0);

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

    // amsart uses different font sizes and styles for headings
    let (font_size, style) = if state.is_amsart {
        match level {
            SectionLevel::Section => (state.base_font_size, FontStyle::SmallCaps),
            SectionLevel::Subsection => (state.base_font_size, FontStyle::Bold),
            SectionLevel::Subsubsection => (state.base_font_size, FontStyle::Italic),
            _ => (level.font_size(state.base_font_size), FontStyle::Bold),
        }
    } else {
        (level.font_size(state.base_font_size), FontStyle::Bold)
    };
    let line_height = font_size * 1.2;

    // Ensure room for heading + spacing + at least one line of following content
    // This prevents orphaned headings at page bottom
    state.ensure_space(line_height + level.spacing_after() + state.cached_line_height);

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
                state.current_section_num = state.section_counters[2];
                state.theorem_counters.clear(); // Reset theorem counters for new section
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                if state.is_amsart {
                    state.text_buf.push_str(". ");
                } else {
                    state.text_buf.push_str("  ");
                }
            }
            SectionLevel::Subsection => {
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                if state.is_amsart {
                    state.text_buf.push_str(". ");
                } else {
                    state.text_buf.push_str("  ");
                }
            }
            SectionLevel::Subsubsection => {
                if state.appendix_mode {
                    let letter = (b'A' + (state.section_counters[2] - 1).min(25) as u8) as char;
                    state.text_buf.push(letter);
                } else {
                    state.text_buf.push_str(ibuf.format(state.section_counters[2]));
                }
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[3]));
                state.text_buf.push('.');
                state.text_buf.push_str(ibuf.format(state.section_counters[4]));
                if state.is_amsart {
                    state.text_buf.push_str(". ");
                } else {
                    state.text_buf.push_str("  ");
                }
            }
            _ => {}
        }
    }

    // Check if title contains inline math (needs mixed font rendering for Greek etc.)
    let has_inline_math = title.iter().any(|n| matches!(n, Node::InlineMath(_)));

    // Append title text directly to buffer (Latin approximations for outlines/headers)
    let title_start = state.text_buf.len();
    for node in title {
        node_to_text(node, &mut state.text_buf, source);
    }
    // amsart section headings use small caps (we approximate with uppercase)
    if state.is_amsart && matches!(level, SectionLevel::Section) {
        let title_text = state.text_buf[title_start..].to_uppercase();
        state.text_buf.truncate(title_start);
        state.text_buf.push_str(&title_text);
    }

    // Record outline/bookmark entry (cap at 5000 to avoid excessive allocations)
    if level.depth() <= 3 && state.outlines.len() < 5000 {
        state.outlines.push(OutlineEntry {
            title: state.text_buf.clone(),
            page: state.page_bounds.len() as u32,
            y: state.current_y,
            level: level.depth(),
        });
    }

    // Update running header title for headings page style
    if matches!(level, SectionLevel::Section) {
        state.current_section_title.clear();
        state.current_section_title.push_str(&state.text_buf);
    }

    // Record page number for TOC entry
    if numbered && (state.toc_section_idx as usize) < state.toc_entries.len() {
        // page_number is 1-based, page_bounds.len() is the 0-based page index
        state.toc_entries[state.toc_section_idx as usize].page = state.page_number;
        state.toc_section_idx += 1;
    }

    state.current_x = state.text_left();

    // \paragraph and \subparagraph: run-in headings (bold title inline with text)
    let run_in = matches!(level, SectionLevel::Paragraph | SectionLevel::Subparagraph);
    // amsart: center section headings
    let centered = state.is_amsart && matches!(level, SectionLevel::Section);

    if has_inline_math && !run_in {
        // Mixed font rendering: emit text segments with header font, math symbols with Symbol font
        let base_font_id = match style {
            FontStyle::SmallCaps | FontStyle::Bold => FontId::HelveticaBold,
            FontStyle::Italic => FontId::HelveticaOblique,
            _ => FontId::Helvetica,
        };

        // Build display segments: (text, font_style) pairs
        struct Seg { text: String, sym: bool }
        let mut segs: Vec<Seg> = Vec::new();

        // First add the number prefix
        let prefix = state.text_buf[..title_start].to_string();
        if !prefix.is_empty() {
            segs.push(Seg { text: prefix, sym: false });
        }

        // Then process title nodes
        for node in title {
            match node {
                Node::InlineMath(math_nodes) => {
                    for mn in math_nodes.iter() {
                        match mn {
                            MathNode::Symbol(s) => {
                                if let Some(first_char) = s.chars().next() {
                                    if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                                        segs.push(Seg { text: String::from(byte as char), sym: true });
                                    }
                                }
                            }
                            MathNode::Variable(ch) => {
                                // Variables in math: use italic
                                let mut t = String::new();
                                t.push(*ch);
                                segs.push(Seg { text: t, sym: false });
                            }
                            _ => {
                                let mut t = String::new();
                                math_to_text_buf(std::slice::from_ref(mn), &mut t);
                                if !t.is_empty() {
                                    segs.push(Seg { text: t, sym: false });
                                }
                            }
                        }
                    }
                }
                _ => {
                    let mut t = String::new();
                    node_to_text(node, &mut t, source);
                    if !t.is_empty() {
                        segs.push(Seg { text: t, sym: false });
                    }
                }
            }
        }

        // Measure total width
        let total_w: f32 = segs.iter().map(|s| {
            if s.sym {
                font::measure_text(&s.text, FontId::Symbol, font_size)
            } else {
                font::measure_text(&s.text, base_font_id, font_size)
            }
        }).sum();

        // Position (centered for amsart, left-aligned otherwise)
        if centered {
            state.current_x = state.text_left() + (state.text_width() - total_w) / 2.0;
        }

        // Emit each segment
        for seg in &segs {
            let (seg_style, seg_font) = if seg.sym {
                (FontStyle::Symbol, FontId::Symbol)
            } else {
                (style, base_font_id)
            };
            state.emit_text(&seg.text, font_size, seg_style, Color::BLACK);
            state.current_x += font::measure_text(&seg.text, seg_font, font_size);
        }

        state.current_y += line_height;
        state.current_x = state.text_left();
    } else {
    // For short titles (most common), skip wrap_text and emit directly
    // SAFETY: text_buf not modified during emit_text (emit_text uses all_text)
    let full_text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
    let avg_width = font_size * 0.52; // bold
    let estimated_width = full_text.len() as f32 * avg_width;

    if run_in {
        // Run-in heading: emit bold title and leave cursor after it for inline continuation
        let text_w = font::measure_text(full_text, FontId::HelveticaBold, font_size);
        state.emit_text(full_text, font_size, style, Color::BLACK);
        state.current_x += text_w + font_size * 0.5; // en-space after heading
        // Don't advance Y or add spacing — next paragraph continues on same line
    } else if centered {
        // Centered heading (amsart sections)
        let font_id = match style {
            FontStyle::SmallCaps | FontStyle::Bold => FontId::HelveticaBold,
            FontStyle::Italic => FontId::HelveticaOblique,
            _ => FontId::Helvetica,
        };
        let text_w = font::measure_text(full_text, font_id, font_size);
        let cx = state.text_left() + (state.text_width() - text_w) / 2.0;
        state.current_x = cx;
        state.emit_text(full_text, font_size, style, Color::BLACK);
        state.current_y += line_height;
        state.current_x = state.text_left();
    } else if estimated_width <= state.text_width() {
        // Single line - emit directly without wrap_text allocation
        state.emit_text(full_text, font_size, style, Color::BLACK);
        state.current_y += line_height;
        state.current_x = state.text_left();
    } else {
        // Multi-line - use wrap_text
        let metrics = FontMetrics::new(font_size, style);
        let lines = wrap_text(full_text, &metrics, state.text_width());
        for line in &lines {
            if centered {
                let font_id = match style {
                    FontStyle::SmallCaps | FontStyle::Bold => FontId::HelveticaBold,
                    _ => FontId::Helvetica,
                };
                let tw = font::measure_text(line, font_id, font_size);
                state.current_x = state.text_left() + (state.text_width() - tw) / 2.0;
            }
            state.emit_text(line, font_size, style, Color::BLACK);
            state.current_y += line_height;
            state.current_x = state.text_left();
        }
    }
    } // close has_inline_math else

    if !run_in {
        state.add_vertical_space(level.spacing_after());
        state.current_x = state.text_left();
    }
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
    // If current_x is already advanced (e.g., after a description list label), use it
    let normal_first = state.text_left() + pi;
    let inline_offset = if state.current_x > normal_first + 1.0 {
        state.current_x - state.text_left()
    } else {
        pi
    };
    if text.len() <= max_chars_single && inline_offset <= pi + 0.1 {
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

        let x_first = state.text_left() + inline_offset;
        let x_rest = state.text_left();
        let first_line_width = full_text_width - inline_offset;
        let max_chars_first = (first_line_width / avg_width) as usize;
        let max_chars_rest = max_chars_single;

        // Integer-based page tracking: avoids float comparison per line
        // Original check: current_y + line_height <= cached_max_y
        // Lines allowed: floor((max_y - current_y - line_height) / step) + 1
        let mut lines_until_break = ((state.cached_max_y - state.current_y - line_height) / step) as i32 + 1;

        // Orphan prevention: if only 1 line fits on this page and paragraph is multi-line,
        // break to next page so at least 2 lines appear together
        if lines_until_break == 1 {
            state.new_page();
            push_start = 0;
            buf_push_pos = 0;
            state.all_text.push_str(text);
            lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
        }

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
                let ws = justify_line(&bytes[line_start..line_end], first_line_width, avg_width, font_size, is_last);
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

                // Try hyphenation when line has significant slack
                let line_chars = line_end - line_start;
                let slack_chars = max_chars_rest.saturating_sub(line_chars);
                if slack_chars >= 4 && next_pos < len {
                    // Find next word
                    let mut ws_skip = next_pos;
                    while ws_skip < len && bytes[ws_skip] <= b' ' { ws_skip += 1; }
                    if ws_skip < len {
                        let mut we = ws_skip;
                        while we < len && bytes[we] > b' ' { we += 1; }
                        let next_word = &bytes[ws_skip..we];
                        if next_word.len() >= 5 {
                            let max_prefix = slack_chars.saturating_sub(1); // -1 for hyphen
                            if let Some(bp) = crate::hyphenate::best_break(next_word, max_prefix) {
                                // Build hyphenated line in text buffer
                                let hyph_off = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                                state.all_text.push_str(&text[line_start..line_end]);
                                state.all_text.push(' ');
                                state.all_text.push_str(&text[ws_skip..ws_skip + bp]);
                                state.all_text.push('-');
                                let hyph_len = (line_end - line_start) + 1 + bp + 1;

                                let hyph_bytes_start = state.current_page_text_start as usize + hyph_off as usize;
                                let ws = justify_line(
                                    &state.all_text.as_bytes()[hyph_bytes_start..hyph_bytes_start + hyph_len],
                                    full_text_width, avg_width, font_size, false,
                                );
                                state.all_elements.push(PageElement::Text {
                                    x: x_rest,
                                    y: state.current_y,
                                    text_offset: hyph_off,
                                    text_len: hyph_len as u16,
                                    font_size_100,
                                    font_style,
                                    color,
                                    word_spacing_50: ws,
                                });
                                state.current_y += step;
                                lines_until_break -= 1;

                                pos = ws_skip + bp;
                                while pos < len && bytes[pos] <= b' ' { pos += 1; }
                                continue;
                            }
                        }
                    }
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
    Ok(())
}

/// A styled text span for rich-text layout
struct StyledSpan {
    text: String,
    style: FontStyle,
    color: Color,
    font_size: f32,
    underline: bool,
    strikethrough: bool,
}

/// Flatten AST nodes into a sequence of styled spans, preserving bold/italic/etc.
fn nodes_to_spans(nodes: &[Node], style: FontStyle, color: Color, font_size: f32, base_size: f32, out: &mut Vec<StyledSpan>, source: &str, labels: &HashMap<String, String>, citations: &HashMap<String, u32>) {
    nodes_to_spans_sc(nodes, style, color, font_size, base_size, false, out, source, labels, citations);
}

fn nodes_to_spans_sc(nodes: &[Node], style: FontStyle, color: Color, font_size: f32, base_size: f32, smallcaps: bool, out: &mut Vec<StyledSpan>, source: &str, labels: &HashMap<String, String>, citations: &HashMap<String, u32>) {
    let mut style = style;
    let mut color = color;
    let mut font_size = font_size;
    let mut smallcaps = smallcaps;
    for node in nodes {
        match node {
            Node::FontStyleDecl(decl) => {
                match decl {
                    FontDeclType::SmallCaps => { smallcaps = true; }
                    _ => {
                        smallcaps = false;
                        style = match decl {
                            FontDeclType::Bold => FontStyle::Bold,
                            FontDeclType::Italic => FontStyle::Italic,
                            FontDeclType::Monospace => FontStyle::Monospace,
                            FontDeclType::Regular => FontStyle::Regular,
                            FontDeclType::SmallCaps => unreachable!(),
                        };
                    }
                }
            }
            Node::ColorDecl(c) => {
                color = *c;
            }
            Node::FontSize { size, content } if content.is_empty() => {
                // Declaration form: \large, \small etc. — changes font_size for subsequent siblings
                font_size = size.to_points(base_size);
            }
            Node::Text(s) => {
                // In LaTeX, single newlines in source are spaces (not line breaks)
                let normalized = if s.contains('\n') { s.replace('\n', " ") } else { s.clone() };
                if smallcaps {
                    let sc_size = font_size * 0.82;
                    out.push(StyledSpan { text: normalized.to_ascii_uppercase(), style, color, font_size: sc_size, underline: false, strikethrough: false });
                } else {
                    out.push(StyledSpan { text: normalized, style, color, font_size, underline: false, strikethrough: false });
                }
            }
            Node::TextRef(offset, len) => {
                let raw = &source[*offset as usize..(*offset as usize + *len as usize)];
                let text = if raw.contains('\n') { raw.replace('\n', " ") } else { raw.to_string() };
                if smallcaps {
                    let sc_size = font_size * 0.82;
                    out.push(StyledSpan { text: text.to_ascii_uppercase(), style, color, font_size: sc_size, underline: false, strikethrough: false });
                } else {
                    out.push(StyledSpan { text, style, color, font_size, underline: false, strikethrough: false });
                }
            }
            Node::Bold(children) => {
                let s = match style {
                    FontStyle::Italic => FontStyle::BoldItalic,
                    _ => FontStyle::Bold,
                };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Italic(children) | Node::Emph(children) => {
                let s = match style {
                    FontStyle::Bold => FontStyle::BoldItalic,
                    _ => FontStyle::Italic,
                };
                nodes_to_spans_sc(children, s, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Monospace(children) => {
                let mut t = String::new();
                for c in children { node_to_text_resolved(c, &mut t, source, labels); }
                out.push(StyledSpan { text: t, style: FontStyle::Monospace, color, font_size, underline: false, strikethrough: false });
            }
            Node::Code(s) => {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Monospace, color, font_size, underline: false, strikethrough: false });
            }
            Node::SmallCaps(children) => {
                // Explicit \textsc{} — enable smallcaps for children
                nodes_to_spans_sc(children, style, color, font_size, base_size, true, out, source, labels, citations);
            }
            Node::Underline(children) => {
                let start_idx = out.len();
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
                for span in &mut out[start_idx..] {
                    span.underline = true;
                }
            }
            Node::Strikethrough(children) => {
                let start_idx = out.len();
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
                for span in &mut out[start_idx..] {
                    span.strikethrough = true;
                }
            }
            Node::Group(children) | Node::Superscript(children)
            | Node::Subscript(children) => {
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Colored { content, color: c } => {
                nodes_to_spans_sc(content, style, *c, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::FontSize { size, content } => {
                let new_size = size.to_points(base_size);
                nodes_to_spans_sc(content, style, color, new_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::Paragraph(children) => {
                nodes_to_spans_sc(children, style, color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            Node::NonBreakingSpace | Node::HSpace(_) => {
                out.push(StyledSpan { text: " ".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::LineBreak => {
                out.push(StyledSpan { text: "\n".to_string(), style, color, font_size, underline: false, strikethrough: false });
            }
            Node::InlineMath(math) => {
                inline_math_to_spans(math, color, font_size, out);
            }
            Node::Citation(key, opt) => {
                let cite_text = resolve_citations(key, opt.as_deref(), citations);
                out.push(StyledSpan { text: cite_text, style, color, font_size, underline: false, strikethrough: false });
            }
            Node::Href { content, .. } => {
                let link_color = Color::from_rgb_u8(0, 0, 180);
                nodes_to_spans_sc(content, style, link_color, font_size, base_size, smallcaps, out, source, labels, citations);
            }
            _ => {
                let mut t = String::new();
                node_to_text_resolved(node, &mut t, source, labels);
                if !t.is_empty() {
                    out.push(StyledSpan { text: t, style, color, font_size, underline: false, strikethrough: false });
                }
            }
        }
    }
}

/// Convert inline math nodes to styled spans for paragraph rendering.
/// Uses Symbol font for Greek/math symbols, italic for variables, regular for text.
fn inline_math_to_spans(nodes: &[MathNode], color: Color, font_size: f32, out: &mut Vec<StyledSpan>) {
    for node in nodes {
        inline_math_node_to_spans(node, color, font_size, out);
    }
}

fn inline_math_node_to_spans(node: &MathNode, color: Color, font_size: f32, out: &mut Vec<StyledSpan>) {
    match node {
        MathNode::Number(s) => {
            out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Variable(c) => {
            out.push(StyledSpan { text: c.to_string(), style: FontStyle::Italic, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Operator(s) => {
            // Try Symbol font for math operators
            let first_char = s.chars().next().unwrap_or('?');
            if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            } else if s.len() == 1 && first_char.is_ascii() {
                // Simple ASCII operators: +, -, =, <, >, etc.
                out.push(StyledSpan { text: format!(" {} ", s), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            } else {
                out.push(StyledSpan { text: format!(" {} ", s), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Symbol(s) => {
            let first_char = s.chars().next().unwrap_or('?');
            if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            } else if first_char.is_ascii() {
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Italic, color, font_size, underline: false, strikethrough: false });
            } else {
                // Fallback: try WinAnsi
                out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Text(s) => {
            out.push(StyledSpan { text: s.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Function(name) => {
            out.push(StyledSpan { text: name.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::OperatorName(name) => {
            out.push(StyledSpan { text: name.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Space(w) => {
            if *w > 0.0 {
                out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Group(nodes) => {
            inline_math_to_spans(nodes, color, font_size, out);
        }
        MathNode::Phantom(_) => {
            // Invisible spacer — skip in inline mode
        }
        MathNode::Super(nodes) => {
            inline_math_to_spans(nodes, color, font_size, out);
        }
        MathNode::Sub(nodes) => {
            inline_math_to_spans(nodes, color, font_size, out);
        }
        MathNode::Frac { numer, denom } => {
            inline_math_to_spans(numer, color, font_size, out);
            out.push(StyledSpan { text: "/".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(denom, color, font_size, out);
        }
        MathNode::Sqrt { radicand, .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{221A}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
            out.push(StyledSpan { text: "(".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(radicand, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Left(d) | MathNode::Right(d) => {
            let first_char = d.chars().next().unwrap_or('.');
            if first_char != '.' {
                if let Some(byte) = font::unicode_to_symbol_byte(first_char) {
                    out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
                } else {
                    out.push(StyledSpan { text: d.clone(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                }
            }
        }
        MathNode::Sum { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{2211}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Integral { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{222B}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Product { .. } => {
            if let Some(byte) = font::unicode_to_symbol_byte('\u{220F}') {
                out.push(StyledSpan { text: String::from(byte as char), style: FontStyle::Symbol, color, font_size, underline: false, strikethrough: false });
            }
        }
        MathNode::Accent { base, .. } => {
            inline_math_to_spans(base, color, font_size, out);
        }
        MathNode::Over { content, .. } | MathNode::Under { content, .. } => {
            inline_math_to_spans(content, color, font_size, out);
        }
        MathNode::Binom { top, bottom } => {
            out.push(StyledSpan { text: "(".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(top, color, font_size, out);
            out.push(StyledSpan { text: ", ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
            inline_math_to_spans(bottom, color, font_size, out);
            out.push(StyledSpan { text: ")".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::Overset { base, .. } | MathNode::Underset { base, .. } => {
            inline_math_to_spans(base, color, font_size, out);
        }
        MathNode::MathFont { content, .. } => {
            inline_math_to_spans(content, color, font_size, out);
        }
        MathNode::Matrix { rows, .. } => {
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    out.push(StyledSpan { text: "; ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                }
                for (j, cell) in row.iter().enumerate() {
                    if j > 0 {
                        out.push(StyledSpan { text: ", ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                    }
                    inline_math_to_spans(cell, color, font_size, out);
                }
            }
        }
        MathNode::Cases { rows } => {
            for (i, (val, cond)) in rows.iter().enumerate() {
                if i > 0 {
                    out.push(StyledSpan { text: "; ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                }
                inline_math_to_spans(val, color, font_size, out);
                if let Some(c) = cond {
                    out.push(StyledSpan { text: " if ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
                    inline_math_to_spans(c, color, font_size, out);
                }
            }
        }
        MathNode::AlignmentMark => {
            out.push(StyledSpan { text: " ".to_string(), style: FontStyle::Regular, color, font_size, underline: false, strikethrough: false });
        }
        MathNode::NewLine | MathNode::StyleSwitch(_) | MathNode::BigDelim { .. } => {
            // Skip
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
        | Node::Underline(_) | Node::InlineMath(_) | Node::Href { .. }
        | Node::Footnote(_) | Node::FontStyleDecl(_) | Node::ColorDecl(_) | Node::Group(_)
        | Node::FontSize { .. } | Node::Citation(..)
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

    // Rich text: build words directly, using math layout for inline math
    // Merge adjacent spans with same style and join into "words"
    // Strategy: split spans into words, keeping style info per word
    struct StyledWord {
        text: String,
        style: FontStyle,
        color: Color,
        font_size: f32,
        width: f32,
        math: Option<math_layout::MathBox>, // For inline math: rendered math elements
        superscript: bool, // If true, render smaller and raised
        underline: bool,
        strikethrough: bool,
    }

    let font_size = state.current_font_size;
    let base_font_size = state.base_font_size;
    let line_height = font_size * baselineskip_factor(base_font_size);
    let step = line_height * state.line_spacing;
    let space_width = font_size * 0.25;
    let text_width = state.text_width();
    let indent = if with_indent { state.paragraph_indent } else { 0.0 };

    // Build word list from spans, with special handling for inline math
    let mut words: Vec<StyledWord> = Vec::new();

    // First pass: collect spans, but handle InlineMath nodes specially
    // by using the math layout engine instead of text conversion
    let fn_count_before = state.footnotes.len();
    let labels_ref: &HashMap<String, String> = &state.label_map;
    let citations_ref: &HashMap<String, u32> = &state.citation_map;

    for child in children {
        match child {
            Node::Footnote(content) => {
                // Emit superscript footnote number and store content for bottom-of-page
                state.footnote_counter += 1;
                let num = state.footnote_counter;
                let num_str = format!("{}", num);
                let sup_size = font_size * 0.65;
                let w = font::measure_text(&num_str, FontId::Helvetica, sup_size);
                words.push(StyledWord {
                    text: num_str,
                    style: FontStyle::Regular,
                    color: state.current_color,
                    font_size,
                    width: w,
                    math: None,
                    superscript: true,
                    underline: false,
                    strikethrough: false,
                });
                state.footnotes.push(content.clone());
            }
            Node::InlineMath(math) => {
                // Use the math layout engine for proper sub/superscript rendering
                let math_box = math_layout::layout_math(math, font_size);
                if math_box.width > 0.0 {
                    // Add a thin space before math if no space precedes it
                    // (handles `text$x$` case — adds ~1pt gap)
                    if let Some(last) = words.last() {
                        if last.text != " " && last.text != "\n" {
                            words.push(StyledWord { text: " ".to_string(), style: FontStyle::Regular, color: state.current_color, width: space_width * 0.5, math: None, superscript: false, font_size, underline: false, strikethrough: false });
                        }
                    }
                    let w = math_box.width;
                    words.push(StyledWord {
                        text: String::new(),
                        style: FontStyle::Regular,
                        color: state.current_color,
                        font_size,
                        width: w,
                        math: Some(math_box),
                        superscript: false,
                        underline: false,
                        strikethrough: false,
                    });
                    // No unconditional thin space after math — let natural source
                    // whitespace in following Text nodes provide proper word spacing.
                    // This avoids: thin space before punctuation ($x$,) and
                    // thin space replacing normal word space ($x$ more).
                }
            }
            _ => {
                // Regular text/formatting: collect as styled spans then split into words
                let mut node_spans = Vec::new();
                let node_style = state.current_font_style;
                let node_color = state.current_color;
                nodes_to_spans(&[child.clone()], node_style, node_color, font_size, base_font_size, &mut node_spans, source, labels_ref, citations_ref);

                for span in &node_spans {
                    let sf = span.font_size;
                    let sw = sf * 0.25; // space width at this font size
                    if span.text == "\n" {
                        words.push(StyledWord { text: "\n".to_string(), style: span.style, color: span.color, font_size: sf, width: 0.0, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                        continue;
                    }
                    let font_id = match span.style {
                        FontStyle::Bold | FontStyle::BoldItalic => FontId::HelveticaBold,
                        FontStyle::Monospace => FontId::Courier,
                        FontStyle::Symbol => FontId::Symbol,
                        _ => FontId::Helvetica,
                    };
                    let parts: Vec<&str> = span.text.split_whitespace().collect();
                    let starts_with_space = span.text.starts_with(char::is_whitespace);
                    let ends_with_space = span.text.ends_with(char::is_whitespace);
                    if starts_with_space && !words.is_empty() {
                        if let Some(last) = words.last() {
                            if last.text != " " && last.text != "\n" {
                                words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                            }
                        }
                    }
                    for (i, part) in parts.iter().enumerate() {
                        if i > 0 {
                            words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                        }
                        let w = font::measure_text(part, font_id, sf);
                        words.push(StyledWord { text: part.to_string(), style: span.style, color: span.color, font_size: sf, width: w, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                    }
                    if ends_with_space && !parts.is_empty() {
                        words.push(StyledWord { text: " ".to_string(), style: span.style, color: span.color, font_size: sf, width: sw, math: None, superscript: false, underline: span.underline, strikethrough: span.strikethrough });
                    }
                }
            }
        }
    }

    // Reserve footnote space for any footnotes collected during word building
    let fn_count_after = state.footnotes.len();
    for _ in fn_count_before..fn_count_after {
        state.reserve_footnote_space();
    }

    // Two-pass line layout: first determine line breaks, then emit with correct baselines.
    let text_ascent = font_size * 0.75;
    let text_descent = font_size * 0.25;
    state.ensure_space(line_height);
    let normal_start = state.text_left() + indent;
    let initial_line_x = if state.current_x > normal_start + 1.0 {
        state.current_x
    } else {
        normal_start
    };
    let first_line_used = initial_line_x - state.text_left();

    // === Pass 1: Determine line breaks ===
    // Each line is represented by a (start_word_idx, end_word_idx, max_above, max_below, hyphen_break)
    struct LineInfo {
        start: usize,
        end: usize, // exclusive
        max_above: f32,
        max_below: f32,
        // If set, the last word on this line should be hyphenated at this byte position
        hyphen: Option<(usize, usize)>, // (word_idx, byte_pos)
    }
    let mut lines: Vec<LineInfo> = Vec::new();
    let mut line_start = 0usize;
    let mut current_line_width = 0.0f32;
    let mut first_line = true;
    let mut line_max_above = text_ascent;
    let mut line_max_below = text_descent;

    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if word.text == "\n" {
            // Forced line break
            lines.push(LineInfo { start: line_start, end: i, max_above: line_max_above, max_below: line_max_below, hyphen: None });
            line_start = i + 1;
            current_line_width = 0.0;
            first_line = false;
            line_max_above = text_ascent;
            line_max_below = text_descent;
            i += 1;
            continue;
        }
        if word.text == " " {
            current_line_width += word.width;
            i += 1;
            continue;
        }

        let effective_max = if first_line { text_width - first_line_used } else { text_width };

        if current_line_width > 0.0 && current_line_width + word.width > effective_max {
            // Try hyphenation
            let mut hyphenated = false;
            if word.math.is_none() && !word.superscript && word.text.len() >= 5 {
                let remaining = effective_max - current_line_width;
                let fid = font::style_to_font_id(word.style);
                let hyphen_w = font::measure_text("-", fid, word.font_size);
                let avg_char_w = word.width / word.text.len() as f32;
                let max_prefix = ((remaining - hyphen_w) / avg_char_w) as usize;
                if max_prefix >= 2 {
                    if let Some(bp) = crate::hyphenate::best_break(word.text.as_bytes(), max_prefix) {
                        let prefix_w = font::measure_text(&word.text[..bp], fid, word.font_size);
                        if prefix_w + hyphen_w <= remaining {
                            // Include this word's prefix on current line
                            lines.push(LineInfo { start: line_start, end: i + 1, max_above: line_max_above, max_below: line_max_below, hyphen: Some((i, bp)) });
                            line_start = i; // suffix starts a new line (same word index, handled in pass 2)
                            current_line_width = 0.0;
                            first_line = false;
                            line_max_above = text_ascent;
                            line_max_below = text_descent;
                            // Track suffix extent
                            if word.font_size > font_size + 0.5 {
                                line_max_above = line_max_above.max(word.font_size * 0.75);
                                line_max_below = line_max_below.max(word.font_size * 0.25);
                            }
                            let suffix_w = font::measure_text(&word.text[bp..], fid, word.font_size);
                            current_line_width = suffix_w;
                            i += 1;
                            hyphenated = true;
                        }
                    }
                }
            }
            if !hyphenated {
                // Wrap: current line ends before this word
                lines.push(LineInfo { start: line_start, end: i, max_above: line_max_above, max_below: line_max_below, hyphen: None });
                line_start = i;
                current_line_width = 0.0;
                first_line = false;
                line_max_above = text_ascent;
                line_max_below = text_descent;
                // Don't increment i — re-process this word on the new line
                continue;
            } else {
                continue;
            }
        }

        // Track line extent
        if let Some(ref math_box) = word.math {
            line_max_above = line_max_above.max(math_box.height);
            line_max_below = line_max_below.max(math_box.depth);
        } else if word.font_size > font_size + 0.5 {
            line_max_above = line_max_above.max(word.font_size * 0.75);
            line_max_below = line_max_below.max(word.font_size * 0.25);
        }

        current_line_width += word.width;
        i += 1;
    }
    // Final line
    if line_start < words.len() {
        lines.push(LineInfo { start: line_start, end: words.len(), max_above: line_max_above, max_below: line_max_below, hyphen: None });
    }

    // === Pass 2: Emit lines with correct baselines ===
    let mut first_line = true;
    let mut prev_max_below = text_descent;
    for line in &lines {
        if !first_line {
            // Step = max of (prev_below + this_above, baselineskip)
            let effective_step = (prev_max_below + line.max_above).max(step);
            state.current_y += effective_step;
            state.ensure_space(line_height);
        } else if line.max_above > text_ascent {
            // First line: push baseline down if content is taller than default
            state.current_y += line.max_above - text_ascent;
        }

        let mut line_x = if first_line { initial_line_x } else { state.text_left() };

        for wi in line.start..line.end {
            let word = &words[wi];
            if word.text == " " {
                line_x += word.width;
                continue;
            }

            // Handle hyphenation: emit only prefix for last word if hyphenated
            if let Some((hyph_wi, bp)) = line.hyphen {
                if wi == hyph_wi {
                    // Emit prefix + "-"
                    state.current_x = line_x;
                    state.text_buf.clear();
                    state.text_buf.push_str(&word.text[..bp]);
                    state.text_buf.push('-');
                    let hyph: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
                    state.emit_text(hyph, word.font_size, word.style, word.color);
                    continue;
                }
            }

            // Emit word
            state.current_x = line_x;
            if let Some(ref math_box) = word.math {
                let saved_x = state.current_x;
                let saved_y = state.current_y;
                emit_math_elements(math_box, line_x, state.current_y, state);
                state.current_x = saved_x;
                state.current_y = saved_y;
            } else if word.superscript {
                let sup_size = word.font_size * 0.65;
                let saved_y = state.current_y;
                state.current_y -= word.font_size * 0.35;
                state.emit_text(&word.text, sup_size, word.style, word.color);
                state.current_y = saved_y;
            } else {
                state.emit_text(&word.text, word.font_size, word.style, word.color);
            }
            // Draw underline if needed
            if word.underline && word.text != " " && !word.text.is_empty() {
                let ul_y = state.current_y + word.font_size * 0.15;
                let ul_thickness = (word.font_size * 0.05).max(0.4);
                state.emit_line(line_x, ul_y, line_x + word.width, ul_y, ul_thickness, word.color);
            }
            // Draw strikethrough if needed
            if word.strikethrough && word.text != " " && !word.text.is_empty() {
                let st_y = state.current_y - word.font_size * 0.25;
                let st_thickness = (word.font_size * 0.05).max(0.4);
                state.emit_line(line_x, st_y, line_x + word.width, st_y, st_thickness, word.color);
            }
            line_x += word.width;
        }

        // Handle hyphenated suffix on next line
        if let Some((hyph_wi, bp)) = line.hyphen {
            // The suffix will be the first content of the next line, handled there
            // But we need to track that the next line starts with the suffix
            // Actually, line_start for the next line is set to the same word index
            // and the next line's loop will encounter the word and emit it fully
            // We need to mark that it should emit the suffix only
            let _ = (hyph_wi, bp); // used in next line
        }

        prev_max_below = line.max_below;
        first_line = false;
    }

    // Advance past last line (use step as minimum baseline advance)
    state.current_y += step;
    state.current_x = state.text_left();
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
    doc: &Document,
    numbered: bool,
    source: &str,
) -> Result<()> {
    let saved_indent = state.indent;
    let saved_para_indent = state.paragraph_indent;
    let depth = state.list_depth;
    state.list_depth += 1;
    state.set_indent(state.indent + 20.0);
    state.paragraph_indent = 0.0; // No paragraph indent inside list items
    state.add_vertical_space(2.0);

    for (i, item) in items.iter().enumerate() {
        // Skip empty items (e.g., trailing \item with no content)
        if item.label.is_none() && item.content.iter().all(|n| match n {
            Node::Text(s) => s.trim().is_empty(),
            Node::TextRef(off, len) => source[*off as usize..(*off as usize + *len as usize)].trim().is_empty(),
            _ => false,
        }) {
            continue;
        }
        state.current_x = state.text_left();
        let line_h = state.current_font_size * 1.2;
        state.ensure_space(line_h);

        // Draw bullet or number/letter marker
        let marker_x = state.text_left() - 15.0;
        state.current_x = marker_x;
        if numbered {
            state.text_buf.clear();
            match depth {
                0 => {
                    // Level 1: 1. 2. 3.
                    let mut ibuf = itoa::Buffer::new();
                    state.text_buf.push_str(ibuf.format(i + 1));
                    state.text_buf.push('.');
                }
                1 => {
                    // Level 2: (a) (b) (c)
                    state.text_buf.push('(');
                    state.text_buf.push((b'a' + (i as u8).min(25)) as char);
                    state.text_buf.push(')');
                }
                2 => {
                    // Level 3: i. ii. iii. iv.
                    let roman = to_roman_lower(i + 1);
                    state.text_buf.push_str(&roman);
                    state.text_buf.push('.');
                }
                _ => {
                    // Level 4+: A. B. C.
                    state.text_buf.push((b'A' + (i as u8).min(25)) as char);
                    state.text_buf.push('.');
                }
            }
            let marker: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(marker, state.current_font_size, FontStyle::Regular, Color::BLACK);
        } else {
            // Bullet style varies by depth
            let bullet_r = state.current_font_size * match depth {
                0 => 0.15,      // Filled circle (large)
                1 => 0.12,      // Smaller
                _ => 0.08,      // Smallest
            };
            let bx = marker_x + bullet_r + 2.0;
            let by = state.current_y + state.current_font_size * 0.35;
            if depth == 0 || depth >= 2 {
                // Filled bullet
                state.emit_rounded_rect(bx - bullet_r, by - bullet_r, bullet_r * 2.0, bullet_r * 2.0,
                    Some(Color::BLACK), None, bullet_r);
            } else {
                // Open bullet (level 2) — ring only
                state.emit_rounded_rect(bx - bullet_r, by - bullet_r, bullet_r * 2.0, bullet_r * 2.0,
                    None, Some(Color::BLACK), bullet_r);
            }
        }
        state.current_x = state.text_left();

        // Split item content into inline prefix (rendered as paragraph) and block children (nested lists, etc.)
        let mut inline_end = item.content.len();
        for (j, node) in item.content.iter().enumerate() {
            if !is_inline_node(node) {
                inline_end = j;
                break;
            }
        }

        // Render leading inline content as rich paragraph
        if inline_end > 0 {
            layout_rich_paragraph(&item.content[..inline_end], state, source, false)?;
        }

        // Render remaining content — use layout_nodes which properly groups
        // consecutive inline nodes into Paragraph nodes for flowing text.
        if inline_end < item.content.len() {
            layout_nodes(&item.content[inline_end..], state, doc, source)?;
        }
    }

    state.list_depth = depth;
    state.paragraph_indent = saved_para_indent;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    state.add_vertical_space(2.0);

    Ok(())
}

/// Convert number to lowercase Roman numerals (for list labels)
fn to_roman_lower(mut n: usize) -> String {
    let mut s = String::new();
    for &(val, sym) in &[(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"),
        (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"),
        (5, "v"), (4, "iv"), (1, "i")] {
        while n >= val {
            s.push_str(sym);
            n -= val;
        }
    }
    s
}

fn layout_description_list(
    items: &[ListItem],
    state: &mut LayoutState,
    doc: &Document,
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
            let label_w = font::measure_text(label_text, FontId::HelveticaBold, state.current_font_size);
            state.emit_text(label_text, state.current_font_size, FontStyle::Bold, Color::BLACK);
            state.current_x += label_w + state.current_font_size * 0.5; // space after label
        }

        // Render content inline (continuation on same line as label)
        let saved_indent = state.indent;

        // Split into inline-prefix vs block children
        let mut inline_end = item.content.len();
        for (j, node) in item.content.iter().enumerate() {
            if !is_inline_node(node) {
                inline_end = j;
                break;
            }
        }

        if inline_end > 0 {
            layout_rich_paragraph(&item.content[..inline_end], state, source, false)?;
        }

        // Render remaining block content indented
        if inline_end < item.content.len() {
            state.set_indent(state.indent + 20.0);
            state.current_x = state.text_left();
            for node in &item.content[inline_end..] {
                if is_inline_node(node) {
                    layout_rich_paragraph(std::slice::from_ref(node), state, source, false)?;
                } else {
                    layout_node(node, state, doc, source)?;
                }
            }
            state.set_indent(saved_indent);
        }

        state.add_vertical_space(4.0);
    }

    state.current_x = state.text_left();

    Ok(())
}

/// Layout bibliography entries from \begin{thebibliography} environment
fn layout_bibliography(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Add heading - amsart uses small caps "References", article uses bold
    state.add_vertical_space(22.0);
    state.ensure_space(40.0);
    if state.is_amsart {
        let heading = "References";
        let heading_size = state.current_font_size * 1.2;
        let heading_w = crate::font::measure_text(heading, crate::font::FontId::Helvetica, heading_size);
        state.current_x = state.text_left() + (state.text_width() - heading_w) * 0.5;
        state.emit_text(heading, heading_size, FontStyle::SmallCaps, Color::BLACK);
        state.current_y += heading_size * 1.2 + 8.0;
    } else {
        let heading = "References";
        let heading_size = state.current_font_size * 1.44;
        state.current_x = state.text_left();
        state.emit_text(heading, heading_size, FontStyle::Bold, Color::BLACK);
        state.current_y += heading_size * 1.2 + 10.0;
    }

    // Walk nodes: each BibItem starts a new entry
    let mut bib_num = 0u32;
    let mut entry_nodes: Vec<&Node> = Vec::new();
    let indent = if state.is_amsart { 20.0 } else { 24.0 };

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

/// Layout a single bibliography entry: [N] followed by entry text with rich formatting
fn layout_bib_entry(num: u32, nodes: &[&Node], state: &mut LayoutState, doc: &Document, source: &str, indent: f32) -> Result<()> {
    state.ensure_space(state.current_font_size * 1.2);
    let font_size = if state.is_amsart {
        state.current_font_size * 0.85
    } else {
        state.current_font_size * 0.9
    };

    // Render [N] marker
    let mut ibuf = itoa::Buffer::new();
    let marker = format!("[{}]", ibuf.format(num));
    let marker_w = crate::font::measure_text(&marker, crate::font::FontId::Helvetica, font_size);
    // Right-align marker in the indent area
    let marker_x = state.text_left() + indent - marker_w - 4.0;
    state.current_x = marker_x.max(state.text_left());
    state.emit_text(&marker, font_size, FontStyle::Regular, Color::BLACK);

    // Layout entry text with hanging indent
    // Wrap nodes in a paragraph so layout_paragraph handles word-wrapping with rich text
    let saved_indent = state.indent;
    let saved_font_size = state.current_font_size;
    state.set_indent(state.text_left() + indent);
    state.current_x = state.text_left() + indent;
    state.current_font_size = font_size;

    // Build a paragraph node, merging adjacent text nodes to fix accent splitting
    // e.g. TextRef("N") + Group([Text("ø")]) + TextRef("dland") → Text("Nødland")
    let para_nodes = merge_adjacent_text(nodes, source);
    let para = Node::Paragraph(para_nodes);
    layout_node(&para, state, doc, source)?;

    state.current_y += font_size * 0.3; // small gap after entry
    state.current_font_size = saved_font_size;
    state.set_indent(saved_indent);
    state.current_x = state.text_left();
    Ok(())
}

/// Merge adjacent text-like nodes to prevent spurious spaces from accent groups.
/// e.g. TextRef("N") + Group([Text("ø")]) + TextRef("dland") → Text("Nødland")
fn merge_adjacent_text(nodes: &[&Node], source: &str) -> Vec<Node> {
    let mut result: Vec<Node> = Vec::with_capacity(nodes.len());
    let mut text_buf = String::new();

    for node in nodes {
        match node {
            Node::Text(s) => text_buf.push_str(s),
            Node::TextRef(offset, len) => {
                text_buf.push_str(&source[*offset as usize..(*offset as usize + *len as usize)]);
            }
            Node::Group(children) if children.len() == 1 => {
                // Single-element groups (from {accent} like {\o}) — extract text
                if let Some(text) = extract_simple_text(&children[0], source) {
                    text_buf.push_str(&text);
                } else {
                    flush_text_buf(&mut text_buf, &mut result);
                    result.push((*node).clone());
                }
            }
            Node::NonBreakingSpace => text_buf.push(' '),
            Node::EnDash => text_buf.push('\u{2013}'),
            Node::EmDash => text_buf.push('\u{2014}'),
            Node::Ellipsis => text_buf.push_str("\u{2026}"),
            Node::Ampersand => text_buf.push('&'),
            Node::Percent => text_buf.push('%'),
            Node::Dollar => text_buf.push('$'),
            Node::Hash => text_buf.push('#'),
            Node::Underscore => text_buf.push('_'),
            Node::Tilde => text_buf.push('~'),
            Node::LeftQuote => text_buf.push('\u{2018}'),
            Node::RightQuote => text_buf.push('\u{2019}'),
            Node::LeftDoubleQuote => text_buf.push('\u{201C}'),
            Node::RightDoubleQuote => text_buf.push('\u{201D}'),
            _ => {
                flush_text_buf(&mut text_buf, &mut result);
                result.push((*node).clone());
            }
        }
    }
    flush_text_buf(&mut text_buf, &mut result);
    result
}

fn flush_text_buf(buf: &mut String, result: &mut Vec<Node>) {
    if !buf.is_empty() {
        result.push(Node::Text(std::mem::take(buf)));
    }
}

fn extract_simple_text(node: &Node, source: &str) -> Option<String> {
    match node {
        Node::Text(s) => Some(s.clone()),
        Node::TextRef(offset, len) => {
            Some(source[*offset as usize..(*offset as usize + *len as usize)].to_string())
        }
        _ => None,
    }
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
            Node::FontStyleDecl(FontDeclType::Bold) => has_bold = true,
            Node::FontStyleDecl(FontDeclType::Italic) => return FontStyle::Italic,
            Node::Italic(_) => return FontStyle::Italic,
            Node::Text(t) if t.trim().is_empty() => {} // skip whitespace
            Node::TextRef(_, _) => has_non_bold = true,
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

    // Track logical column for each cell (handles multicolumn)
    // (logical_col, colspan, optional alignment override from \multicolumn)
    let mut cell_logical_cols: Vec<Vec<(u32, u32, Option<ColumnSpec>)>> = Vec::with_capacity(table.rows.len());

    for row in &table.rows {
        let mut row_texts = Vec::with_capacity(num_cols);
        let mut row_styles = Vec::with_capacity(num_cols);
        let mut row_cols = Vec::with_capacity(num_cols);
        let mut logical_col: u32 = 0;
        for cell in &row.cells {
            if logical_col as usize >= num_cols { break; }
            let span = cell.colspan.max(1);
            let mut text = String::new();
            for node in &cell.content {
                node_to_text_resolved(node, &mut text, source, &state.label_map);
            }
            let trimmed = text.trim().to_string();
            let style = detect_cell_style(&cell.content);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };
            let w = font::measure_text(&trimmed, fid, font_size);
            // Only count single-column cells for individual column width measurement
            if span == 1 && (logical_col as usize) < num_cols {
                if w > col_max_widths[logical_col as usize] {
                    col_max_widths[logical_col as usize] = w;
                }
            }
            row_texts.push(trimmed);
            row_styles.push(style);
            row_cols.push((logical_col, span, cell.alignment.clone()));
            logical_col += span;
        }
        while row_texts.len() < num_cols {
            row_texts.push(String::new());
            row_styles.push(FontStyle::Regular);
            row_cols.push((logical_col, 1, None));
            logical_col += 1;
        }
        cell_texts.push(row_texts);
        cell_styles.push(row_styles);
        cell_logical_cols.push(row_cols);
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
            // For multicolumn cells, use the combined width of spanned columns
            let (logical_col, span, _align_override) = cell_logical_cols.get(row_idx)
                .and_then(|r| r.get(col_idx))
                .cloned()
                .unwrap_or((col_idx as u32, 1, None));
            let col_w = if span > 1 {
                (logical_col..logical_col + span)
                    .map(|c| col_widths.get(c as usize).copied().unwrap_or(0.0))
                    .sum::<f32>()
            } else {
                col_widths.get(logical_col as usize).copied().unwrap_or(100.0)
            };
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
        let rule_sep = if table.rows[row_idx].hline_before { font_size * 0.9 } else { 0.0 };
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
        for (cell_idx, cell_lines) in wrapped_cells[row_idx].iter().enumerate() {
            // Get logical column and colspan for this cell
            let (logical_col, span, align_override) = cell_logical_cols.get(row_idx)
                .and_then(|r| r.get(cell_idx))
                .cloned()
                .unwrap_or((cell_idx as u32, 1, None));
            if logical_col as usize >= num_cols { break; }

            // Position at the logical column
            col_x = table_x + col_widths.iter().take(logical_col as usize).sum::<f32>();

            // Combined width for multicolumn cells
            let col_w = if span > 1 {
                (logical_col..logical_col + span)
                    .map(|c| col_widths.get(c as usize).copied().unwrap_or(0.0))
                    .sum::<f32>()
            } else {
                col_widths.get(logical_col as usize).copied().unwrap_or(100.0)
            };
            let cx = col_x + cell_padding;
            let cell_content_width = col_w - cell_padding * 2.0;

            // Use alignment: cell override (from \multicolumn) > column spec > left default
            let default_center = ColumnSpec::Center;
            let align = if let Some(ref ov) = align_override {
                ov
            } else if span > 1 {
                &default_center
            } else if (logical_col as usize) < data_cols.len() {
                data_cols[logical_col as usize]
            } else {
                &ColumnSpec::Left
            };

            // Use style detected from cell content (bold from \textbf{}, etc.)
            let style = cell_styles[row_idx].get(cell_idx).copied().unwrap_or(FontStyle::Regular);
            let fid = if style == FontStyle::Bold { FontId::HelveticaBold } else { FontId::Helvetica };

            for (line_idx, line_text) in cell_lines.iter().enumerate() {
                let display_w = font::measure_text(line_text, fid, font_size);
                let text_x = match align {
                    ColumnSpec::Center => cx + (cell_content_width - display_w) / 2.0,
                    ColumnSpec::Right => cx + cell_content_width - display_w,
                    _ => cx,
                };
                // Push text down below hline_before rules so ascenders don't overlap
                let rule_sep = if row.hline_before { font_size * 0.9 } else { 0.0 };
                let text_y = y + cell_padding + rule_sep + line_idx as f32 * line_h;
                state.current_x = text_x;
                state.current_y = text_y;
                state.emit_text(line_text, state.current_font_size, style, Color::BLACK);
            }
        }

        // Draw horizontal lines (booktabs style)
        if row.hline_before {
            let rule_width = if row_idx == 0 { 1.2 } else { 0.8 }; // toprule=1.2, midrule=0.8
            state.emit_line(table_x, y, table_x + actual_table_width, y, rule_width, Color::BLACK);
        }
        if row.hline_after {
            let line_y = y + row_height;
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

    // Check for pgfplots axis environment first
    if tikz_source.contains("\\begin{axis}") {
        if let Some((plot_elems, total_w, total_h)) = crate::pgfplots::render_pgfplot(tikz_source) {
            return layout_pgfplot_elements(&plot_elems, total_w, total_h, state);
        }
    }

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

fn layout_pgfplot_elements(elems: &[crate::pgfplots::PlotElement], total_w: f32, total_h: f32, state: &mut LayoutState) -> Result<()> {
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
                state.emit_line(
                    base_x + x1 * scale, base_y + y1 * scale,
                    base_x + x2 * scale, base_y + y2 * scale,
                    width * scale, *color,
                );
            }
            PlotElement::Rect { x, y, width, height, fill, stroke } => {
                state.emit_rect(
                    base_x + x * scale, base_y + y * scale,
                    width * scale, height * scale,
                    *fill, *stroke,
                );
            }
            PlotElement::Text { x, y, text, font_size, color, anchor, rotation } => {
                let fs = font_size * scale;
                let tw = font::measure_text(text, FontId::Helvetica, fs);
                let abs_x = base_x + x * scale;
                let abs_y = base_y + y * scale;
                // Adjust position based on anchor
                let (tx, ty) = match anchor {
                    TextAnchor::Center => (abs_x - tw / 2.0, abs_y),
                    TextAnchor::West => (abs_x, abs_y),
                    TextAnchor::East => (abs_x - tw, abs_y),
                    TextAnchor::North => (abs_x - tw / 2.0, abs_y),
                    TextAnchor::South => (abs_x - tw / 2.0, abs_y - fs),
                };
                // For rotated text (Y-axis label), we'll render it horizontally as a workaround
                // PDF rotation of text requires transform matrices which we handle specially
                if *rotation > 0.0 {
                    // Y-axis label — render vertically by emitting chars stacked
                    // For now, render horizontally at the position
                    state.emit_text(text, fs, FontStyle::Regular, *color);
                    state.current_x = tx;
                } else {
                    let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                    state.all_text.push_str(text);
                    state.all_elements.push(PageElement::Text {
                        x: tx,
                        y: ty,
                        text_offset: offset,
                        text_len: text.len().min(65535) as u16,
                        font_size_100: (fs * 100.0) as u16,
                        font_style: FontStyle::Regular,
                        color: *color,
                        word_spacing_50: 0,
                    });
                }
            }
            PlotElement::Circle { cx, cy, radius, fill } => {
                // Approximate circle with a small filled rect
                let r = radius * scale;
                let abs_cx = base_x + cx * scale;
                let abs_cy = base_y + cy * scale;
                state.emit_rect(
                    abs_cx - r, abs_cy - r,
                    r * 2.0, r * 2.0,
                    Some(*fill), None,
                );
            }
        }
    }

    state.current_y = base_y + scaled_h + 10.0;
    state.current_x = state.text_left();
    state.add_vertical_space(10.0);
    Ok(())
}

fn layout_display_math_data(math_data: &DisplayMathData, state: &mut LayoutState) -> Result<()> {
    let has_alignment = math_data.nodes.iter().any(|n| matches!(n, MathNode::AlignmentMark | MathNode::NewLine));

    if has_alignment && matches!(math_data.env_type, MathEnvType::Align | MathEnvType::Gather) {
        layout_aligned_math(&math_data.nodes, math_data.numbered, state)
    } else {
        layout_display_math_simple(&math_data.nodes, math_data.numbered, state)
    }
}

/// Check if a single node is a breakable operator
fn is_breakable_op(node: &MathNode) -> bool {
    match node {
        MathNode::Operator(op) => matches!(op.as_str(),
            "+" | "-" | "=" | "<" | ">" | "≤" | "≥" | "≠" | "≈"
            | "∈" | "∉" | "⊂" | "⊃" | "⊆" | "⊇" | "∼" | "≅" | "≃"
            | "→" | "←" | "↦" | "⟶" | "⟵"
            | "∧" | "∨" | "⊕" | "⊗" | "×"
        ),
        MathNode::Symbol(s) => matches!(s.as_str(),
            "+" | "-" | "=" | "<" | ">" | "≤" | "≥" | "≠" | "≈"
            | "∈" | "∉" | "⊂" | "⊃" | "⊆" | "⊇" | "∼" | "≅" | "≃"
            | "→" | "←" | "↦" | "⟶" | "⟵"
        ),
        _ => false,
    }
}

/// Check if a MathNode is a breakable binary/relational operator (including operators inside Groups)
fn is_math_break_point(node: &MathNode) -> bool {
    match node {
        MathNode::Group(children) => {
            // Groups like [Space, Operator("+"), Space] are breakable
            children.iter().any(|c| is_breakable_op(c))
        }
        _ => is_breakable_op(node),
    }
}

fn layout_display_math_simple(math_nodes: &[MathNode], numbered: bool, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(8.0);

    // Filter out alignment marks and newlines for simple display
    let filtered: Vec<&MathNode> = math_nodes.iter()
        .filter(|n| !matches!(n, MathNode::AlignmentMark | MathNode::NewLine))
        .collect();
    let owned: Vec<MathNode> = filtered.into_iter().cloned().collect();

    let math_box = math_layout::layout_math(&owned, state.current_font_size);

    // Reserve space for equation number if needed
    let eq_num_width = if numbered { 40.0 } else { 0.0 };
    let avail_width = state.text_width() - eq_num_width;

    // If equation fits, render normally
    if math_box.width <= avail_width {
        let total_height = math_box.height + math_box.depth;
        state.ensure_space(total_height + 16.0);
        let cx = state.text_left() + (avail_width - math_box.width) / 2.0;
        let baseline_y = state.current_y + math_box.height;
        emit_math_elements(&math_box, cx, baseline_y, state);

        if numbered {
            state.equation_counter += 1;
            let eq_text = format!("({})", state.equation_counter);
            let num_x = state.text_left() + state.text_width() - 30.0;
            let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
            state.all_text.push_str(&eq_text);
            state.all_elements.push(PageElement::Text {
                x: num_x,
                y: baseline_y,
                text_offset: offset,
                text_len: eq_text.len().min(65535) as u16,
                font_size_100: (state.current_font_size * 100.0) as u16,
                font_style: FontStyle::Regular,
                color: Color::BLACK,
                word_spacing_50: 0,
            });
        }

        state.current_y = baseline_y + math_box.depth;
    } else {
        // Equation too wide — auto line-break at operator positions
        let font_size = state.current_font_size;
        let indent = font_size * 2.0; // continuation line indent

        // Find break points: indices of binary/relational operators
        let mut break_indices: Vec<usize> = Vec::new();
        for (i, node) in owned.iter().enumerate() {
            if is_math_break_point(node) {
                break_indices.push(i);
            }
        }

        if break_indices.is_empty() {
            // No break points found — render as single overflowing line (left-aligned)
            let total_height = math_box.height + math_box.depth;
            state.ensure_space(total_height + 16.0);
            let baseline_y = state.current_y + math_box.height;
            emit_math_elements(&math_box, state.text_left(), baseline_y, state);
            state.current_y = baseline_y + math_box.depth;
        } else {
            // Greedy line-breaking: accumulate nodes until width exceeds avail_width,
            // then break at the last operator that still fits
            let mut lines: Vec<(usize, usize)> = Vec::new(); // (start, end) ranges
            let mut line_start = 0;
            let mut last_valid_break = 0;
            let mut _last_valid_break_idx = 0;

            for (bi, &break_pos) in break_indices.iter().enumerate() {
                // Try layout from line_start to break_pos (exclusive — operator goes to next line)
                let segment = &owned[line_start..break_pos];
                let seg_box = math_layout::layout_math(segment, font_size);
                let line_avail = if lines.is_empty() { avail_width } else { avail_width - indent };

                if seg_box.width > line_avail && last_valid_break > line_start {
                    // Previous break was the last valid one
                    lines.push((line_start, last_valid_break));
                    line_start = last_valid_break;
                }
                last_valid_break = break_pos;
                _last_valid_break_idx = bi;
            }
            // Final segment
            lines.push((line_start, owned.len()));

            // Render each line
            let mut first_line_baseline_y = 0.0f32;
            for (li, &(start, end)) in lines.iter().enumerate() {
                let segment = &owned[start..end];
                let seg_box = math_layout::layout_math(segment, font_size);
                let total_h = seg_box.height + seg_box.depth;
                let row_spacing = font_size * 0.4;
                state.ensure_space(total_h + row_spacing);

                let line_avail = if li == 0 { avail_width } else { avail_width - indent };
                let cx = if seg_box.width <= line_avail {
                    if li == 0 {
                        // Center first line
                        state.text_left() + (avail_width - seg_box.width) / 2.0
                    } else {
                        // Right-align continuation lines (standard LaTeX behavior)
                        state.text_left() + avail_width - seg_box.width
                    }
                } else {
                    state.text_left() + if li > 0 { indent } else { 0.0 }
                };

                let baseline_y = state.current_y + seg_box.height;
                if li == 0 { first_line_baseline_y = baseline_y; }
                emit_math_elements(&seg_box, cx, baseline_y, state);
                state.current_y = baseline_y + seg_box.depth + row_spacing;
            }

            // Equation number on first line
            if numbered {
                state.equation_counter += 1;
                let eq_text = format!("({})", state.equation_counter);
                let num_x = state.text_left() + state.text_width() - 30.0;
                let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
                state.all_text.push_str(&eq_text);
                state.all_elements.push(PageElement::Text {
                    x: num_x,
                    y: first_line_baseline_y,
                    text_offset: offset,
                    text_len: eq_text.len().min(65535) as u16,
                    font_size_100: (font_size * 100.0) as u16,
                    font_style: FontStyle::Regular,
                    color: Color::BLACK,
                    word_spacing_50: 0,
                });
            }
        }
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();
    // In LaTeX, text after display math without a blank line continues the paragraph (no indent)
    state.suppress_next_indent = true;

    Ok(())
}

fn layout_aligned_math(math_nodes: &[MathNode], numbered: bool, state: &mut LayoutState) -> Result<()> {
    state.add_vertical_space(8.0);

    // Split nodes into rows at NewLine, then each row into columns at AlignmentMark
    let mut rows: Vec<Vec<Vec<MathNode>>> = Vec::new();
    let mut current_row: Vec<Vec<MathNode>> = Vec::new();
    let mut current_col: Vec<MathNode> = Vec::new();

    for node in math_nodes {
        match node {
            MathNode::NewLine => {
                current_row.push(std::mem::take(&mut current_col));
                rows.push(std::mem::take(&mut current_row));
            }
            MathNode::AlignmentMark => {
                current_row.push(std::mem::take(&mut current_col));
            }
            _ => {
                current_col.push(node.clone());
            }
        }
    }
    if !current_col.is_empty() || !current_row.is_empty() {
        current_row.push(current_col);
        rows.push(current_row);
    }

    if rows.is_empty() {
        return Ok(());
    }

    let font_size = state.current_font_size;

    // Pass 1: layout all cells, measure column widths
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut cell_boxes: Vec<Vec<math_layout::MathBox>> = Vec::new();
    let mut col_widths = vec![0.0f32; num_cols];

    for row in &rows {
        let mut row_boxes = Vec::new();
        for (j, cell) in row.iter().enumerate() {
            let mb = math_layout::layout_math(cell, font_size);
            if j < num_cols {
                col_widths[j] = col_widths[j].max(mb.width);
            }
            row_boxes.push(mb);
        }
        cell_boxes.push(row_boxes);
    }

    let col_gap = font_size * 0.5;
    let row_spacing = font_size * 1.6;
    let total_content_width: f32 = col_widths.iter().sum::<f32>() + col_gap * (num_cols.max(1) - 1) as f32;
    let total_height = row_spacing * rows.len() as f32;
    let eq_num_width = if numbered { 40.0 } else { 0.0 };
    let avail_width = state.text_width() - eq_num_width;

    state.ensure_space(total_height + 16.0);

    // Pass 2: position with alignment
    // Odd columns (0, 2, 4...) right-align, even columns (1, 3, 5...) left-align
    let base_x = if total_content_width > avail_width {
        state.text_left() // Left-align if too wide
    } else {
        state.text_left() + (avail_width - total_content_width) / 2.0
    };

    for (row_idx, row_boxes) in cell_boxes.iter().enumerate() {
        let baseline_y = state.current_y;
        let mut col_x = base_x;

        for (j, cell_box) in row_boxes.iter().enumerate() {
            let col_w = if j < col_widths.len() { col_widths[j] } else { cell_box.width };

            // Alignment: odd columns right-align to align point, even columns left-align
            let cx = if j % 2 == 0 {
                // Right-align (before alignment mark)
                col_x + col_w - cell_box.width
            } else {
                // Left-align (after alignment mark)
                col_x
            };

            emit_math_elements(cell_box, cx, baseline_y + cell_box.height, state);
            col_x += col_w + col_gap;
        }

        // Equation number on each line (or just last line)
        if numbered {
            state.equation_counter += 1;
            let eq_text = format!("({})", state.equation_counter);
            let num_x = state.text_left() + state.text_width() - 30.0;
            let offset = (state.all_text.len() - state.current_page_text_start as usize) as u32;
            state.all_text.push_str(&eq_text);
            let max_h = row_boxes.iter().map(|b| b.height).fold(0.0f32, f32::max);
            state.all_elements.push(PageElement::Text {
                x: num_x,
                y: baseline_y + max_h,
                text_offset: offset,
                text_len: eq_text.len().min(65535) as u16,
                font_size_100: (font_size * 100.0) as u16,
                font_style: FontStyle::Regular,
                color: Color::BLACK,
                word_spacing_50: 0,
            });
        }

        state.current_y += row_spacing;
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();
    // In LaTeX, text after display math without a blank line continues the paragraph (no indent)
    state.suppress_next_indent = true;

    Ok(())
}

fn emit_math_elements(math_box: &math_layout::MathBox, cx: f32, baseline_y: f32, state: &mut LayoutState) {
    for elem in &math_box.elements {
        match elem {
            math_layout::MathElement::Text { x, y, text, font_size, font_id, color } => {
                let style = match font_id {
                    FontId::HelveticaOblique => FontStyle::Italic,
                    FontId::HelveticaBold => FontStyle::Bold,
                    FontId::HelveticaBoldOblique => FontStyle::BoldItalic,
                    FontId::Courier => FontStyle::Monospace,
                    FontId::Symbol => FontStyle::Symbol,
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
}

fn layout_theorem(thm: &TheoremData, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(10.0);

    // Look up theorem definition to get proper title, numbering, and style
    let (display_title, is_numbered, thm_style) = if let Some(def) = doc.preamble.theorem_defs.iter()
        .find(|d| d.env_name == thm.env_name)
    {
        (def.display_title.clone(), def.numbered, def.style)
    } else {
        // Not a \newtheorem env — use the env_name as-is, capitalize first letter
        let mut title = thm.env_name.clone();
        if let Some(first) = title.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        // Guess style from env name
        let style = match thm.env_name.as_str() {
            "definition" | "example" | "notation" | "convention" | "assumption"
                => TheoremStyle::Definition,
            "remark" | "note" | "observation"
                => TheoremStyle::Remark,
            _ => TheoremStyle::Plain,
        };
        (title, false, style)
    };

    // Build header: "Theorem N" or "Theorem N (Name)"
    let mut header = display_title.clone();
    if is_numbered {
        // Use the theorem's counter (shared counter or own counter from \newtheorem)
        let counter_name = if let Some(def) = doc.preamble.theorem_defs.iter()
            .find(|d| d.env_name == thm.env_name)
        {
            def.counter.clone().unwrap_or_else(|| thm.env_name.clone())
        } else {
            thm.env_name.clone()
        };
        let count = state.theorem_counters.entry(counter_name).or_insert(0);
        *count += 1;
        let num = *count;
        if state.current_section_num > 0 {
            header.push_str(&format!(" {}.{}", state.current_section_num, num));
        } else {
            header.push_str(&format!(" {}", num));
        }
    }
    if let Some(ref name) = thm.optional_name {
        header.push_str(&format!(" ({})", name));
    }
    header.push('.');

    // Determine label font style based on theorem style:
    // plain → Bold, definition → Bold, remark → Italic
    let label_style = match thm_style {
        TheoremStyle::Plain | TheoremStyle::Definition => FontStyle::Bold,
        TheoremStyle::Remark => FontStyle::Italic,
    };

    // Emit header on its own line
    let font_size = state.current_font_size;
    state.ensure_space(font_size * 1.2);
    state.current_x = state.text_left();
    state.emit_text(&header, font_size, label_style, Color::BLACK);
    state.current_y += font_size * 1.2;
    state.current_x = state.text_left();
    state.suppress_next_indent = true;

    // Layout body: plain → italic, definition/remark → upright
    let saved_style = state.current_font_style;
    if thm_style == TheoremStyle::Plain {
        state.current_font_style = FontStyle::Italic;
    }
    layout_nodes(&thm.body, state, doc, source)?;
    state.current_font_style = saved_style;

    state.add_vertical_space(10.0);
    Ok(())
}

fn layout_proof(content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(8.0);

    // "Proof." header in italic on its own line
    let font_size = state.current_font_size;
    let header = "Proof.";
    state.ensure_space(font_size * 1.2);
    state.current_x = state.text_left();
    state.emit_text(header, font_size, FontStyle::Italic, Color::BLACK);
    state.current_y += font_size * 1.2;
    state.current_x = state.text_left();
    state.suppress_next_indent = true;

    // Layout body in regular style
    layout_nodes(content, state, doc, source)?;

    // QED square at end of proof — draw as a filled rectangle
    let sq = font_size * 0.5;
    let qed_x = state.text_left() + state.text_width() - sq;
    let qed_y = state.current_y - sq * 0.7;
    state.emit_line(qed_x, qed_y, qed_x + sq, qed_y, sq, Color::BLACK);

    state.add_vertical_space(8.0);
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
    for node in content {
        match node {
            Node::Paragraph(children) => {
                state.text_buf.clear();
                for child in children {
                    node_to_text(child, &mut state.text_buf, source);
                }
                let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            Node::TextParagraph(offset, len) | Node::TextRef(offset, len) => {
                let text = source[*offset as usize..(*offset as usize + *len as usize)].trim();
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            Node::Text(s) => {
                let text = s.trim();
                if text.is_empty() { continue; }
                layout_centered_text(text, state)?;
            }
            _ => {
                layout_node(node, state, doc, source)?;
            }
        }
    }
    Ok(())
}

fn layout_centered_text(text: &str, state: &mut LayoutState) -> Result<()> {
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let line_h = font_size * 1.2;
    let fid = font::style_to_font_id(font_style);
    let space_width = font::measure_text(" ", fid, font_size);
    let para_width = state.text_width();

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
        let word_width = font::measure_text(&text[word_start..pos], fid, font_size);
        if current_width > 0.0 && current_width + space_width + word_width > para_width {
            let line = text[line_start..word_start].trim_end();
            if !line.is_empty() {
                state.ensure_space(line_h);
                let tw = font::measure_text(line, fid, font_size);
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
        let tw = font::measure_text(remaining, fid, font_size);
        state.current_x = state.text_left() + (para_width - tw) / 2.0;
        state.emit_text(remaining, font_size, font_style, color);
        state.current_y += line_h * state.line_spacing;
    }

    state.add_vertical_space(font_size * 0.2);
    Ok(())
}

fn layout_flush_right(content: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    for node in content {
        match node {
            Node::Paragraph(children) => {
                state.text_buf.clear();
                for child in children {
                    node_to_text(child, &mut state.text_buf, source);
                }
                let text: &str = unsafe { &*(state.text_buf.trim() as *const str) };
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            Node::TextParagraph(offset, len) | Node::TextRef(offset, len) => {
                let text = source[*offset as usize..(*offset as usize + *len as usize)].trim();
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            Node::Text(s) => {
                let text = s.trim();
                if text.is_empty() { continue; }
                layout_right_aligned_text(text, state)?;
            }
            _ => {
                layout_node(node, state, doc, source)?;
            }
        }
    }
    Ok(())
}

fn layout_right_aligned_text(text: &str, state: &mut LayoutState) -> Result<()> {
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let line_h = font_size * 1.2;
    let fid = font::style_to_font_id(font_style);
    let space_width = font::measure_text(" ", fid, font_size);
    let para_width = state.text_width();

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
        let word_width = font::measure_text(&text[word_start..pos], fid, font_size);
        if current_width > 0.0 && current_width + space_width + word_width > para_width {
            let line = text[line_start..word_start].trim_end();
            if !line.is_empty() {
                state.ensure_space(line_h);
                let tw = font::measure_text(line, fid, font_size);
                state.current_x = state.text_left() + para_width - tw;
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
        let tw = font::measure_text(remaining, fid, font_size);
        state.current_x = state.text_left() + para_width - tw;
        state.emit_text(remaining, font_size, font_style, color);
        state.current_y += line_h * state.line_spacing;
    }

    state.add_vertical_space(font_size * 0.2);
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
        Node::SmallCaps(children) => {
            let start = out.len();
            for child in children {
                node_to_text_ext(child, out, source, labels);
            }
            // Uppercase the collected text for small-caps approximation
            let collected = out[start..].to_ascii_uppercase();
            out.truncate(start);
            out.push_str(&collected);
        }
        Node::Bold(children) | Node::Italic(children) | Node::Monospace(children)
        | Node::Underline(children) | Node::Emph(children)
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
        Node::Footnote(_content) => {
            // Footnote mark — use dagger symbol as a generic marker in text extraction
            out.push('\u{2020}'); // †
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
        Node::EqRef(label) => {
            out.push('(');
            if let Some(map) = labels {
                if let Some(resolved) = map.get(label) {
                    out.push_str(resolved);
                } else {
                    out.push_str("??");
                }
            } else {
                out.push_str("??");
            }
            out.push(')');
        }
        Node::Citation(key, opt) => {
            out.push('[');
            out.push_str(key);
            if let Some(o) = opt {
                out.push_str(", ");
                out.push_str(o);
            }
            out.push(']');
        }
        Node::Label(_) | Node::BibItem(_) => {}
        Node::Code(s) => out.push_str(s),
        Node::Href { content, .. } => {
            for c in content { node_to_text_ext(c, out, source, labels); }
        }
        _ => {}
    }
}

/// Map Unicode math/Greek symbols to WinAnsi-safe text representations.
/// Symbols in the Latin-1 range (U+00A0..U+00FF) pass through directly as they're in WinAnsi.
/// Resolve citation key(s) to display text like "[1,3,5]" or "[key]"
fn resolve_citations(key: &str, opt: Option<&str>, citation_map: &HashMap<String, u32>) -> String {
    let keys: Vec<&str> = key.split(',').map(|k| k.trim()).collect();
    let mut nums = Vec::new();
    let mut any_resolved = false;
    for k in &keys {
        if let Some(&num) = citation_map.get(*k) {
            nums.push(num.to_string());
            any_resolved = true;
        } else {
            nums.push((*k).to_string());
        }
    }
    let base = nums.join(",");
    match opt {
        Some(text) => {
            // Expand ~ to space in optional citation text
            let clean = text.replace('~', " ");
            format!("[{}, {}]", base, clean)
        }
        None => format!("[{}]", base),
    }
}

/// Try to load an image file and return embedded data + native dimensions
fn load_image_for_pdf(path: &str, _state: &LayoutState) -> Option<(EmbeddedImage, u32, u32)> {
    // Resolve path relative to the tex file directory
    // Try the path as-is first, then with common extensions
    let candidates = [
        path.to_string(),
        format!("{}.png", path),
        format!("{}.jpg", path),
        format!("{}.jpeg", path),
        format!("{}.pdf", path),
    ];

    for candidate in &candidates {
        let p = std::path::Path::new(candidate);
        if !p.exists() { continue; }

        let data = match std::fs::read(p) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Detect format from magic bytes
        if data.len() < 8 { continue; }

        if data[0..2] == [0xFF, 0xD8] {
            // JPEG
            if let Some((w, h)) = jpeg_dimensions(&data) {
                return Some((EmbeddedImage {
                    data, width_px: w, height_px: h, format: ImageFormat::Jpeg,
                }, w, h));
            }
        } else if data[0..4] == [0x89, b'P', b'N', b'G'] {
            // PNG — extract dimensions from IHDR chunk
            if data.len() >= 24 {
                let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                return Some((EmbeddedImage {
                    data, width_px: w, height_px: h, format: ImageFormat::Png,
                }, w, h));
            }
        }
        // PDF images not supported — skip
    }

    None
}

/// Extract width/height from JPEG SOF marker
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // skip FFD8
    while i + 4 < data.len() {
        if data[i] != 0xFF { i += 1; continue; }
        let marker = data[i + 1];
        if marker == 0 || marker == 0xFF { i += 1; continue; }
        let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        // SOF markers: 0xC0..0xCF (except 0xC4 DHT and 0xCC DAC)
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
                // Greek letters → preserve Unicode (rendered via Symbol font)
                '\u{03B1}'..='\u{03C9}' => out.push(ch),  // α-ω
                '\u{0393}' | '\u{0394}' | '\u{0398}' | '\u{039B}' | '\u{039E}'
                | '\u{03A0}' | '\u{03A3}' | '\u{03A6}' | '\u{03A8}' | '\u{03A9}'
                    => out.push(ch),  // Γ Δ Θ Λ Ξ Π Σ Φ Ψ Ω
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
        MathNode::Cases { rows } => {
            for (i, (value, cond)) in rows.iter().enumerate() {
                math_to_text_buf(value, out);
                if let Some(c) = cond {
                    out.push_str(" if ");
                    math_to_text_buf(c, out);
                }
                if i < rows.len() - 1 { out.push_str(", "); }
            }
        }
        MathNode::Binom { top, bottom } => {
            out.push('(');
            math_to_text_buf(top, out);
            out.push_str(" choose ");
            math_to_text_buf(bottom, out);
            out.push(')');
        }
        MathNode::Overset { over, base } | MathNode::Underset { under: over, base } => {
            math_to_text_buf(base, out);
        }
        MathNode::OperatorName(name) => out.push_str(name),
        MathNode::MathFont { content, .. } => math_to_text_buf(content, out),
        MathNode::AlignmentMark => out.push_str("  "),
        MathNode::NewLine => out.push('\n'),
        MathNode::Phantom(content) => {
            // Phantom: invisible but takes space — emit nothing in text mode
        }
        MathNode::StyleSwitch(_) => {}
        MathNode::BigDelim { delim, .. } => out.push_str(delim),
    }
}
