/// Layout state: mutable context used by all layout functions

use std::collections::HashMap;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};
use super::types::*;
use super::prescans::{TocEntry, TocFixup};
use super::spans::StyledSpan;

fn to_roman(mut n: u32) -> String {
    let table = [
        (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"),
        (100, "c"), (90, "xc"), (50, "l"), (40, "xl"),
        (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
    ];
    let mut result = String::new();
    for &(val, sym) in &table {
        while n >= val {
            result.push_str(sym);
            n -= val;
        }
    }
    result
}

/// Compute baselineskip factor matching pdflatex defaults for standard font sizes.
#[inline]
pub(super) fn baselineskip_factor(font_size: f32) -> f32 {
    // pdflatex: 10pt→12pt(1.2), 11pt→13.6pt(1.236), 12pt→14.5pt(1.208)
    let fs_int = (font_size + 0.5) as u32;
    match fs_int {
        11 => 1.236,
        12 => 1.208,
        _ => 1.2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PageStyle {
    Plain,    // centered page number at bottom, no header
    Headings, // section title + page number in header, no footer
    Empty,    // no header or footer
    Fancy,    // fancyhdr custom headers/footers
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PageNumberingStyle {
    Arabic,
    Roman,      // lowercase roman (i, ii, iii, iv...)
    UpperRoman, // uppercase roman (I, II, III, IV...)
    Alph,       // lowercase letter (a, b, c...)
    UpperAlph,  // uppercase letter (A, B, C...)
}

pub(super) struct LayoutState {
    pub page_setup: PageSetup,
    pub base_font_size: f32,
    pub current_x: f32,
    pub current_y: f32,
    pub cached_max_y: f32,
    pub cached_start_y: f32,
    pub cached_text_width: f32,
    pub cached_text_left: f32,
    pub current_font_size: f32,
    pub current_font_style: FontStyle,
    pub current_color: Color,
    pub cached_avg_width: f32,
    pub cached_line_height: f32,
    pub cached_step: f32,
    pub cached_font_size_100: u16,
    pub cached_max_chars: usize,
    pub cached_font_key: u32,
    pub all_elements: Vec<PageElement>,
    pub all_text: String,
    pub rect_data: Vec<RectData>,
    pub images: Vec<EmbeddedImage>,
    pub page_bounds: Vec<PageBounds>,
    pub current_page_elem_start: u32,
    pub current_page_text_start: u32,
    pub page_number: u32,
    pub indent: f32,
    pub right_indent: f32,
    pub paragraph_indent: f32,
    pub paragraph_skip: f32,
    pub line_spacing: f32,
    pub baseline_skip_override: Option<f32>,
    pub alignment_mode: AlignmentMode,
    pub section_counters: [u32; 7],
    pub figure_counter: u32,
    pub table_counter: u32,
    pub algorithm_counter: u32,
    pub footnotes: Vec<Vec<Node>>,
    pub footnote_counter: u32,
    pub footnote_reserved: f32,
    pub suppress_next_indent: bool,
    pub list_depth: u32,
    pub array_stretch: f32,
    pub text_buf: String,
    pub label_map: HashMap<String, String>,
    pub label_types: HashMap<String, String>,
    /// Maps label names to (page_index, y_position) for clickable cross-references
    pub label_positions: HashMap<String, (u32, f32)>,
    pub citation_map: HashMap<String, u32>,
    /// Map from citation key to (author_short, year) for natbib-style citations
    pub author_year_map: HashMap<String, (String, String)>,
    pub equation_counter: u32,
    pub theorem_counters: HashMap<String, u32>,
    pub current_section_num: u32,
    pub appendix_mode: bool,
    pub toc_entries: Vec<TocEntry>,
    pub links: Vec<LinkAnnotation>,
    pub outlines: Vec<OutlineEntry>,
    pub source_ptr: *const u8,
    pub source_len: usize,
    pub toc_fixups: Vec<TocFixup>,
    pub toc_section_idx: u32,
    pub page_style: PageStyle,
    pub page_numbering: PageNumberingStyle,
    pub current_section_title: String,
    pub first_page: bool,
    pub is_amsart: bool,
    pub amsart_header_author: String,
    pub amsart_header_title: String,
    // fancyhdr configuration
    pub fancy_head_left: String,
    pub fancy_head_center: String,
    pub fancy_head_right: String,
    pub fancy_foot_left: String,
    pub fancy_foot_center: String,
    pub fancy_foot_right: String,
    pub fancy_head_rule: f32,
    pub fancy_foot_rule: f32,
    pub deferred_abstract_idx: Option<usize>,
    pub amsart_pre_title: bool,
    // Two-column layout support
    pub current_column: u32,          // 0 = left column (or single), 1 = right column
    pub twocolumn_active: bool,       // whether two-column mode is currently active
    pub spanning_mode: bool,          // true = spanning both columns (e.g. title)
    pub column1_max_y: f32,           // highest y reached in left column (for balancing)
    // In-memory project images (for WASM or multi-file compilation)
    pub project_images: HashMap<String, Vec<u8>>,
    // Base directory for resolving relative paths (images, includes)
    pub base_dir: String,
    // Deferred floats waiting for placement
    pub deferred_top_floats: Vec<DeferredFloat>,
    pub deferred_bottom_floats: Vec<DeferredFloat>,
    pub has_pending_top_floats: bool,
    // Pending vertical space for collapsing (LaTeX: consecutive \vspace takes max, not sum)
    pub pending_vspace: f32,
    // hyperref colors
    pub link_color: Color,  // internal cross-references
    pub url_color: Color,   // external URLs
    pub cite_color: Color,  // citations
}

/// A figure/table deferred for float placement
#[derive(Clone)]
pub struct DeferredFloat {
    pub content: Vec<Node>,
    pub caption: Option<Vec<Node>>,
    pub label: Option<String>,
    pub is_table: bool,
}

impl LayoutState {
    pub fn new(page_setup: PageSetup, font_size: f32, line_spacing: f32) -> Self {
        let max_y = page_setup.height - page_setup.margin_bottom - page_setup.footer_height;
        let start_y = page_setup.margin_top + page_setup.header_height;
        let text_w = page_setup.text_width();
        let avg_w = font_size * 0.47;
        let lh = font_size * baselineskip_factor(font_size);
        let st = lh * line_spacing;
        let para_w = text_w - font_size * 1.5;  // 1.5em paragraph indent
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
            all_elements: Vec::with_capacity(1_600_000),
            all_text: String::with_capacity(8 * 1024 * 1024),
            rect_data: Vec::with_capacity(64_000),
            images: Vec::new(),
            page_bounds: Vec::with_capacity(51000),
            current_page_elem_start: 0,
            current_page_text_start: 0,
            page_number: 1,
            indent: 0.0,
            right_indent: 0.0,
            paragraph_indent: font_size * 1.5,  // 1.5em (LaTeX default)
            paragraph_skip: 0.0,     // \parskip default is 0pt (set by parskip package)
            line_spacing,
            baseline_skip_override: None,
            alignment_mode: AlignmentMode::Justify,
            section_counters: [0; 7],
            figure_counter: 0,
            table_counter: 0,
            algorithm_counter: 0,
            footnotes: Vec::new(),
            footnote_counter: 0,
            footnote_reserved: 0.0,
            suppress_next_indent: true,  // LaTeX: first paragraph of document is not indented
            list_depth: 0,
            array_stretch: 1.0,
            text_buf: String::with_capacity(4096),
            label_map: HashMap::new(),
            label_types: HashMap::new(),
            label_positions: HashMap::new(),
            citation_map: HashMap::new(),
            author_year_map: HashMap::new(),
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
            page_numbering: PageNumberingStyle::Arabic,
            current_section_title: String::new(),
            first_page: true,
            is_amsart: false,
            amsart_header_author: String::new(),
            amsart_header_title: String::new(),
            fancy_head_left: String::new(),
            fancy_head_center: String::new(),
            fancy_head_right: String::new(),
            fancy_foot_left: String::new(),
            fancy_foot_center: String::new(),
            fancy_foot_right: String::new(),
            fancy_head_rule: 0.4,
            fancy_foot_rule: 0.0,
            deferred_abstract_idx: None,
            amsart_pre_title: false,
            current_column: 0,
            twocolumn_active: false,
            spanning_mode: false,
            column1_max_y: 0.0,
            project_images: HashMap::new(),
            base_dir: String::new(),
            deferred_top_floats: Vec::new(),
            deferred_bottom_floats: Vec::new(),
            has_pending_top_floats: false,
            pending_vspace: 0.0,
            link_color: Color::from_rgb_u8(140, 0, 0),   // dark red (hyperref default)
            url_color: Color::from_rgb_u8(0, 0, 180),    // blue
            cite_color: Color::from_rgb_u8(0, 100, 0),   // dark green
        }
    }

    #[inline(always)]
    pub fn text_width(&self) -> f32 {
        self.cached_text_width
    }

    #[inline(always)]
    pub fn text_left(&self) -> f32 {
        self.cached_text_left
    }

    #[inline(always)]
    pub fn wrap_params(&mut self) -> (f32, f32, f32, u16, usize) {
        let key = (self.current_font_size.to_bits() & 0xFFFF0000) | (self.current_font_style as u32);
        if key != self.cached_font_key {
            let fs = self.current_font_size;
            self.cached_avg_width = fs * match self.current_font_style {
                FontStyle::Monospace => 0.6,
                FontStyle::Bold | FontStyle::BoldItalic => 0.50,
                _ => 0.47,
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
    pub fn set_indent(&mut self, indent: f32) {
        self.indent = indent;
        let base_width = if self.twocolumn_active && !self.spanning_mode {
            self.page_setup.column_width()
        } else {
            self.page_setup.text_width()
        };
        self.cached_text_width = base_width - indent - self.right_indent;
        if self.twocolumn_active && !self.spanning_mode && self.current_column == 1 {
            let col_width = self.page_setup.column_width();
            self.cached_text_left = self.page_setup.margin_left + col_width + self.page_setup.column_sep + indent;
        } else {
            self.cached_text_left = self.page_setup.margin_left + indent;
        }
        self.cached_font_key = u32::MAX;
    }

    #[inline(always)]
    pub fn set_right_indent(&mut self, right_indent: f32) {
        self.right_indent = right_indent;
        let base_width = if self.twocolumn_active && !self.spanning_mode {
            self.page_setup.column_width()
        } else {
            self.page_setup.text_width()
        };
        self.cached_text_width = base_width - self.indent - right_indent;
        self.cached_font_key = u32::MAX;
    }

    #[inline(always)]
    pub fn max_y(&self) -> f32 {
        self.cached_max_y
    }

    pub fn metrics(&self) -> FontMetrics {
        FontMetrics::new(self.current_font_size, self.current_font_style)
    }

    pub fn source_str(&self) -> &str {
        if self.source_ptr.is_null() { return ""; }
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.source_ptr, self.source_len)) }
    }

    pub fn reserve_footnote_space(&mut self) {
        let fn_size = self.base_font_size * 0.8;
        let fn_line_height = fn_size * baselineskip_factor(fn_size);
        if self.footnote_reserved == 0.0 {
            self.footnote_reserved = 10.0 + fn_line_height;
        } else {
            self.footnote_reserved += fn_line_height;
        }
        self.cached_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height - self.footnote_reserved;
    }

    pub fn render_footnotes(&mut self) {
        if self.footnotes.is_empty() { return; }

        let footnotes = std::mem::take(&mut self.footnotes);
        let fn_start_num = self.footnote_counter - footnotes.len() as u32 + 1;
        let fn_size = self.base_font_size * 0.8;
        let fn_line_height = fn_size * baselineskip_factor(fn_size);

        // Column-aware footnote area
        let fn_left = if self.twocolumn_active && !self.spanning_mode {
            if self.current_column == 0 {
                self.page_setup.margin_left
            } else {
                self.page_setup.margin_left + self.page_setup.column_width() + self.page_setup.column_sep
            }
        } else {
            self.page_setup.margin_left
        };
        let fn_width = if self.twocolumn_active && !self.spanning_mode {
            self.page_setup.column_width()
        } else {
            self.page_setup.text_width()
        };

        // Convert footnote nodes to styled spans for rich rendering
        let source = self.source_str() as *const str;
        let source_ref = unsafe { &*source };
        // Hanging indent: measure widest footnote number for proper alignment
        let max_num = fn_start_num + footnotes.len() as u32;
        let mut ibuf_pre = itoa::Buffer::new();
        let num_w = font::measure_text(ibuf_pre.format(max_num), FontId::TimesRoman, fn_size * 0.75);
        let text_indent = num_w + fn_size * 0.4; // number width + small gap
        let usable_width = fn_width - text_indent;

        // Build span lists and estimate heights
        let labels = self.label_map.clone();
        let citations = self.citation_map.clone();
        let mut fn_span_lists: Vec<Vec<StyledSpan>> = Vec::with_capacity(footnotes.len());
        let mut total_lines: f32 = 0.0;

        for fn_content in &footnotes {
            let mut spans = Vec::new();
            super::spans::nodes_to_spans(
                fn_content, FontStyle::Regular, Color::BLACK,
                fn_size, fn_size, &mut spans, source_ref,
                &labels, &citations,
            );
            // Estimate number of lines from total text width
            let text_w: f32 = spans.iter().map(|s| {
                font::measure_text(&s.text, font::style_to_font_id(s.style), s.font_size)
            }).sum();
            let lines = if text_w <= usable_width {
                1.0
            } else {
                1.0 + ((text_w - usable_width) / usable_width).ceil()
            };
            total_lines += lines;
            fn_span_lists.push(spans);
        }

        let total_height = total_lines * fn_line_height + self.base_font_size * 1.0;
        let orig_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
        let fn_y_start = orig_max_y - total_height;

        if fn_y_start < self.current_y + 20.0 {
            // Not enough space — defer to next page
            self.footnotes = footnotes;
            return;
        }

        // Separator rule (LaTeX default: 0.4pt thick, ~1/3 text width)
        // LaTeX \footnoterule: \kern -3pt, rule 0.4pt thick, \kern 2.6pt
        // Total: rule sits 3pt above the footnote text area with 2.6pt gap below it
        let rule_y = fn_y_start;
        self.emit_line(
            fn_left,
            rule_y,
            fn_left + fn_width * 0.33,
            rule_y,
            0.4,
            Color::BLACK,
        );

        // Gap below rule: enough for ascenders not to touch the line
        let mut y = rule_y + fn_size * 1.1;
        for (i, spans) in fn_span_lists.iter().enumerate() {
            let num = fn_start_num + i as u32;
            let mut ibuf = itoa::Buffer::new();
            let num_str = ibuf.format(num);

            // Render superscript number
            let sup_size = fn_size * 0.75;
            self.current_x = fn_left;
            self.current_y = y - fn_size * 0.15;
            self.emit_text(num_str, sup_size, FontStyle::Regular, Color::BLACK);
            self.current_y = y;

            // Render styled spans with word wrapping
            let text_start_x = fn_left + text_indent;
            let mut line_x = text_start_x;

            for span in spans {
                let font_id = font::style_to_font_id(span.style);
                // Split span into words for wrapping
                let words: Vec<&str> = span.text.split_whitespace().collect();
                let space_w = font::measure_text(" ", font_id, span.font_size);

                for (wi, word) in words.iter().enumerate() {
                    let word_w = font::measure_text(word, font_id, span.font_size);

                    // Check if word fits on current line
                    if line_x > text_start_x && line_x + word_w > fn_left + fn_width {
                        y += fn_line_height;
                        line_x = text_start_x;
                    }

                    // Add space before word (except at line start)
                    if wi > 0 || line_x > text_start_x {
                        if line_x > text_start_x {
                            line_x += space_w;
                        }
                    }

                    self.current_x = line_x;
                    self.current_y = y;
                    self.emit_text(word, span.font_size, span.style, span.color);
                    line_x += word_w;
                }
            }

            y += fn_line_height;
        }
    }

    pub fn new_page(&mut self) {
        // In two-column mode: if we're in column 1, switch to column 2 first
        if self.twocolumn_active && !self.spanning_mode && self.current_column == 0 {
            self.switch_to_column(1);
            return;
        }

        self.render_footnotes();

        let effective_style = if self.first_page { PageStyle::Plain } else { self.page_style };

        match effective_style {
            PageStyle::Plain => {
                self.emit_page_number_centered();
            }
            PageStyle::Headings => {
                // Scale header font with base font: LaTeX uses \small for headers
                let header_font_size = (self.base_font_size * 0.9).max(8.0);
                let header_y = self.page_setup.margin_top - header_font_size * 1.6;
                let left_x = self.page_setup.margin_left;
                let right_x = self.page_setup.width - self.page_setup.margin_right;

                let mut num_string = String::new();
                self.format_page_number(&mut num_string);
                let num_str: &str = unsafe { &*(&*num_string as *const str) };
                let num_len = num_str.len();
                let num_width = font::measure_text(num_str, FontId::TimesRoman, header_font_size);

                if self.is_amsart {
                    let is_even = self.page_number % 2 == 0;
                    if is_even {
                        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                        self.all_text.push_str(num_str);
                        self.all_elements.push(PageElement::Text {
                            x: left_x, y: header_y, text_offset: offset,
                            text_len: num_len as u16,
                            font_size_100: (header_font_size * 100.0) as u16,
                            font_style: FontStyle::Regular, color: Color::BLACK, word_spacing_50: 0,
                        });
                        if !self.amsart_header_author.is_empty() {
                            let author: &str = unsafe { &*(self.amsart_header_author.as_str() as *const str) };
                            let author_len = author.len().min(u16::MAX as usize);
                            let author_w = font::measure_text(&author[..author_len], FontId::TimesRoman, header_font_size);
                            let author_offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                            self.all_text.push_str(&author[..author_len]);
                            self.all_elements.push(PageElement::Text {
                                x: right_x - author_w, y: header_y, text_offset: author_offset,
                                text_len: author_len as u16,
                                font_size_100: (header_font_size * 100.0) as u16,
                                font_style: FontStyle::SmallCaps, color: Color::BLACK, word_spacing_50: 0,
                            });
                        }
                    } else {
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
                            x: right_x - num_width, y: header_y, text_offset: offset,
                            text_len: num_len as u16,
                            font_size_100: (header_font_size * 100.0) as u16,
                            font_style: FontStyle::Regular, color: Color::BLACK, word_spacing_50: 0,
                        });
                    }
                } else {
                    let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                    self.all_text.push_str(num_str);
                    self.all_elements.push(PageElement::Text {
                        x: right_x - num_width, y: header_y, text_offset: offset,
                        text_len: num_len as u16,
                        font_size_100: (header_font_size * 100.0) as u16,
                        font_style: FontStyle::Regular, color: Color::BLACK, word_spacing_50: 0,
                    });
                    if !self.current_section_title.is_empty() {
                        let title: &str = unsafe { &*(self.current_section_title.as_str() as *const str) };
                        self.emit_header_text(title, left_x, header_y, header_font_size, FontStyle::Italic);
                    }
                }

                if !self.is_amsart {
                    // Rule sits below header text: gap = ~0.3x header font size
                    let rule_y = header_y + header_font_size * 0.4;
                    self.all_elements.push(PageElement::Line {
                        x1: left_x, y1: rule_y, x2: right_x, y2: rule_y,
                        width_1000: 400, color: Color::BLACK,
                    });
                }
            }
            PageStyle::Empty => {}
            PageStyle::Fancy => {
                let header_font_size = (self.base_font_size * 0.9).max(8.0);
                let left_x = self.page_setup.margin_left;
                let right_x = self.page_setup.width - self.page_setup.margin_right;

                // Render header
                let header_y = self.page_setup.margin_top - header_font_size * 1.6;
                let resolve = |tmpl: &str, state: &mut LayoutState| -> String {
                    let mut s = tmpl.to_string();
                    if s.contains("\x01PAGE\x01") {
                        let mut ibuf = itoa::Buffer::new();
                        s = s.replace("\x01PAGE\x01", ibuf.format(state.page_number));
                    }
                    if s.contains("\x01LEFTMARK\x01") {
                        let title: String = state.current_section_title.clone();
                        s = s.replace("\x01LEFTMARK\x01", &title);
                    }
                    if s.contains("\x01RIGHTMARK\x01") {
                        let title: String = state.current_section_title.clone();
                        s = s.replace("\x01RIGHTMARK\x01", &title);
                    }
                    if s.contains("\x01SECTION\x01") {
                        let mut ibuf = itoa::Buffer::new();
                        s = s.replace("\x01SECTION\x01", ibuf.format(state.current_section_num));
                    }
                    s
                };

                // Header left
                if !self.fancy_head_left.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_head_left.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    self.emit_header_text(&text, left_x, header_y, header_font_size, FontStyle::Regular);
                }
                // Header center
                if !self.fancy_head_center.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_head_center.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    let w = font::measure_text(&text, FontId::TimesRoman, header_font_size);
                    let cx = (left_x + right_x - w) / 2.0;
                    self.emit_header_text(&text, cx, header_y, header_font_size, FontStyle::Regular);
                }
                // Header right
                if !self.fancy_head_right.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_head_right.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    let w = font::measure_text(&text, FontId::TimesRoman, header_font_size);
                    self.emit_header_text(&text, right_x - w, header_y, header_font_size, FontStyle::Regular);
                }
                // Header rule
                if self.fancy_head_rule > 0.0 {
                    let rule_y = header_y + header_font_size * 0.4;
                    self.all_elements.push(PageElement::Line {
                        x1: left_x, y1: rule_y, x2: right_x, y2: rule_y,
                        width_1000: (self.fancy_head_rule * 1000.0) as u16, color: Color::BLACK,
                    });
                }

                // Footer
                let footer_y = self.page_setup.height - self.page_setup.margin_bottom + self.base_font_size * 1.0;
                // Footer rule
                if self.fancy_foot_rule > 0.0 {
                    let rule_y = footer_y - header_font_size * 0.6;
                    self.all_elements.push(PageElement::Line {
                        x1: left_x, y1: rule_y, x2: right_x, y2: rule_y,
                        width_1000: (self.fancy_foot_rule * 1000.0) as u16, color: Color::BLACK,
                    });
                }
                // Footer left
                if !self.fancy_foot_left.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_foot_left.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    self.emit_header_text(&text, left_x, footer_y, header_font_size, FontStyle::Regular);
                }
                // Footer center
                if !self.fancy_foot_center.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_foot_center.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    let w = font::measure_text(&text, FontId::TimesRoman, header_font_size);
                    let cx = (left_x + right_x - w) / 2.0;
                    self.emit_header_text(&text, cx, footer_y, header_font_size, FontStyle::Regular);
                }
                // Footer right
                if !self.fancy_foot_right.is_empty() {
                    let tmpl: String = unsafe { &*(self.fancy_foot_right.as_str() as *const str) }.to_string();
                    let text = resolve(&tmpl, self);
                    let w = font::measure_text(&text, FontId::TimesRoman, header_font_size);
                    self.emit_header_text(&text, right_x - w, footer_y, header_font_size, FontStyle::Regular);
                }
            }
        }

        self.page_bounds.push(PageBounds {
            elem_start: self.current_page_elem_start,
            elem_end: self.all_elements.len() as u32,
            text_start: self.current_page_text_start,
            text_end: self.all_text.len() as u32,
        });
        self.current_page_elem_start = self.all_elements.len() as u32;
        self.current_page_text_start = self.all_text.len() as u32;
        self.page_number += 1;
        self.first_page = false;
        self.footnote_reserved = 0.0;
        self.cached_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
        // Reset column start Y to page top for new pages
        self.cached_start_y = self.page_setup.margin_top + self.page_setup.header_height;

        // Reset to column 0 in two-column mode
        if self.twocolumn_active && !self.spanning_mode {
            self.current_column = 0;
            let col_width = self.page_setup.column_width();
            self.cached_text_left = self.page_setup.margin_left + self.indent;
            self.cached_text_width = col_width - self.indent - self.right_indent;
            self.column1_max_y = 0.0;
        }
        self.current_x = self.text_left();
        self.current_y = self.cached_start_y;

        // Mark that top floats should be rendered at start of this new page
        self.has_pending_top_floats = !self.deferred_top_floats.is_empty();
    }

    /// Check if there are deferred floats pending and whether this is the right
    /// time to render them. Called at the start of layout_nodes.
    pub fn should_render_top_floats(&self) -> bool {
        self.has_pending_top_floats && !self.deferred_top_floats.is_empty()
    }

    /// Take all pending top floats (consumer takes ownership)
    pub fn take_top_floats(&mut self) -> Vec<DeferredFloat> {
        self.has_pending_top_floats = false;
        std::mem::take(&mut self.deferred_top_floats)
    }

    pub fn set_page_numbering(&mut self, style: &str) {
        self.page_numbering = match style {
            "roman" => PageNumberingStyle::Roman,
            "Roman" => PageNumberingStyle::UpperRoman,
            "alph" => PageNumberingStyle::Alph,
            "Alph" => PageNumberingStyle::UpperAlph,
            _ => PageNumberingStyle::Arabic,
        };
        self.page_number = 1; // Reset counter on numbering change
    }

    fn format_page_number(&self, buf: &mut String) {
        let n = self.page_number;
        match self.page_numbering {
            PageNumberingStyle::Arabic => {
                use std::fmt::Write;
                let _ = write!(buf, "{}", n);
            }
            PageNumberingStyle::Roman | PageNumberingStyle::UpperRoman => {
                let lower = to_roman(n);
                if self.page_numbering == PageNumberingStyle::UpperRoman {
                    buf.push_str(&lower.to_uppercase());
                } else {
                    buf.push_str(&lower);
                }
            }
            PageNumberingStyle::Alph | PageNumberingStyle::UpperAlph => {
                if n >= 1 && n <= 26 {
                    let base = if self.page_numbering == PageNumberingStyle::UpperAlph { b'A' } else { b'a' };
                    buf.push((base + (n - 1) as u8) as char);
                } else {
                    use std::fmt::Write;
                    let _ = write!(buf, "{}", n);
                }
            }
        }
    }

    pub fn emit_section_heading(&mut self, title: &str, font_size: f32) {
        self.ensure_space(font_size * 2.0);
        self.current_y += font_size * 0.5;
        self.emit_text(title, font_size, FontStyle::Bold, Color::BLACK);
        self.current_y += font_size * 1.5;
    }

    fn emit_page_number_centered(&mut self) {
        let center_x = self.page_setup.width / 2.0;
        // Center page number in the bottom margin area
        let y = self.page_setup.height - self.page_setup.margin_bottom / 2.0;
        let mut num_str = String::new();
        self.format_page_number(&mut num_str);
        let page_num_size = self.base_font_size; // LaTeX default: same as body text
        let text_width = font::measure_text(&num_str, FontId::TimesRoman, page_num_size);
        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
        let num_len = num_str.len();
        self.all_text.push_str(&num_str);
        self.all_elements.push(PageElement::Text {
            x: center_x - text_width / 2.0, y, text_offset: offset,
            text_len: num_len as u16, font_size_100: (page_num_size * 100.0) as u16,
            font_style: FontStyle::Regular, color: Color::BLACK, word_spacing_50: 0,
        });
    }

    #[inline(always)]
    pub fn ensure_space(&mut self, height: f32) {
        if self.current_y + height > self.cached_max_y {
            if self.twocolumn_active && !self.spanning_mode && self.current_column == 0 {
                // Switch to column 2 instead of new page
                self.switch_to_column(1);
            } else {
                self.new_page();
            }
        }
    }

    #[inline(always)]
    pub fn add_vertical_space(&mut self, space: f32) {
        self.current_y += space;
        if self.current_y > self.cached_max_y {
            if self.twocolumn_active && !self.spanning_mode && self.current_column == 0 {
                self.switch_to_column(1);
            } else {
                self.new_page();
            }
        }
    }

    /// Enter two-column mode. Sets up column widths.
    /// If called after spanning content (e.g. \twocolumn[...]), columns start
    /// at the current Y position, not the top of the page.
    pub fn enter_twocolumn(&mut self) {
        self.twocolumn_active = true;
        self.current_column = 0;
        self.spanning_mode = false;
        // Ensure page_setup knows we're in 2-column mode (for column_width() calculation)
        if self.page_setup.columns < 2 {
            self.page_setup.columns = 2;
        }
        // Set column start Y to current position (so right column starts at same height)
        self.cached_start_y = self.current_y;
        // Reset indent/right_indent when entering twocolumn to avoid carrying over
        self.indent = 0.0;
        self.right_indent = 0.0;
        // Update text width to column width
        let col_width = self.page_setup.column_width();
        self.cached_text_width = col_width;
        self.cached_text_left = self.page_setup.margin_left;
        self.current_x = self.cached_text_left;
        self.cached_font_key = u32::MAX; // invalidate cached metrics
    }

    /// Enter spanning mode (both columns, e.g. for title/abstract or figure*/table*)
    pub fn enter_spanning(&mut self) {
        // If entering spanning from column 1, advance Y to below both columns
        // so spanning content doesn't overlap column 0's content
        if self.twocolumn_active && self.current_column == 1 {
            if self.column1_max_y > self.current_y {
                self.current_y = self.column1_max_y;
            }
        }
        self.spanning_mode = true;
        self.cached_text_width = self.page_setup.text_width() - self.indent - self.right_indent;
        self.cached_text_left = self.page_setup.margin_left + self.indent;
        self.current_x = self.cached_text_left;
        self.cached_font_key = u32::MAX;
    }

    /// Exit spanning mode, return to columnar layout
    pub fn exit_spanning(&mut self) {
        self.spanning_mode = false;
        if self.twocolumn_active {
            self.current_column = 0;
            // Update cached_start_y so right column starts below spanning content
            self.cached_start_y = self.current_y;
            let col_width = self.page_setup.column_width();
            self.cached_text_width = col_width - self.indent - self.right_indent;
            self.cached_text_left = self.page_setup.margin_left + self.indent;
            self.current_x = self.cached_text_left;
            self.cached_font_key = u32::MAX;
        }
    }

    /// Switch to specified column (0=left, 1=right)
    pub fn switch_to_column(&mut self, col: u32) {
        if col == 1 && self.current_column == 0 {
            // Save column 1 height
            self.column1_max_y = self.current_y;
            // Move to column 2
            self.current_column = 1;
            let col_width = self.page_setup.column_width();
            let col2_left = self.page_setup.margin_left + col_width + self.page_setup.column_sep;
            self.cached_text_left = col2_left + self.indent;
            self.cached_text_width = col_width - self.indent - self.right_indent;
            self.current_x = self.cached_text_left;
            self.current_y = self.cached_start_y; // start from top of page
            self.cached_font_key = u32::MAX;
        } else if col == 0 {
            self.current_column = 0;
            let col_width = self.page_setup.column_width();
            self.cached_text_left = self.page_setup.margin_left + self.indent;
            self.cached_text_width = col_width - self.indent - self.right_indent;
            self.current_x = self.cached_text_left;
            self.cached_font_key = u32::MAX;
        }
    }

    #[inline(always)]
    pub fn emit_text(&mut self, text: &str, font_size: f32, style: FontStyle, color: Color) {
        if text.is_empty() { return; }
        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
        self.all_text.push_str(text);
        self.all_elements.push(PageElement::Text {
            x: self.current_x, y: self.current_y, text_offset: offset,
            text_len: text.len().min(65535) as u16,
            font_size_100: (font_size * 100.0) as u16,
            font_style: style, color, word_spacing_50: 0,
        });
    }

    /// Emit text with simulated small caps: lowercase → uppercase at 75% size
    pub fn emit_text_smallcaps(&mut self, text: &str, font_size: f32, color: Color) {
        if text.is_empty() { return; }
        let small_size = font_size * 0.75;
        let mut seg_start = 0;
        let mut seg_is_upper = !text.as_bytes().first().map_or(true, |b| b.is_ascii_lowercase());

        for (i, ch) in text.char_indices() {
            let is_lower = ch.is_ascii_lowercase();
            let cur_upper = !is_lower;
            if cur_upper != seg_is_upper && i > seg_start {
                let seg = &text[seg_start..i];
                if seg_is_upper {
                    // Uppercase/spaces/digits: emit at normal size
                    let w = font::measure_text(seg, FontId::TimesRoman, font_size);
                    self.emit_text(seg, font_size, FontStyle::Regular, color);
                    self.current_x += w;
                } else {
                    // Lowercase: convert to uppercase, emit at small size
                    let upper: String = seg.to_uppercase();
                    let w = font::measure_text(&upper, FontId::TimesRoman, small_size);
                    self.emit_text(&upper, small_size, FontStyle::Regular, color);
                    self.current_x += w;
                }
                seg_start = i;
                seg_is_upper = cur_upper;
            }
        }
        // Final segment
        if seg_start < text.len() {
            let seg = &text[seg_start..];
            if seg_is_upper {
                let w = font::measure_text(seg, FontId::TimesRoman, font_size);
                self.emit_text(seg, font_size, FontStyle::Regular, color);
                self.current_x += w;
            } else {
                let upper: String = seg.to_uppercase();
                let w = font::measure_text(&upper, FontId::TimesRoman, small_size);
                self.emit_text(&upper, small_size, FontStyle::Regular, color);
                self.current_x += w;
            }
        }
    }

    /// Emit the TeX logo with proper kerning and baseline shifts
    /// Returns the total width consumed
    pub fn emit_tex_logo(&mut self, font_size: f32, style: FontStyle, color: Color) -> f32 {
        let fid = font::style_to_font_id(style);
        let x0 = self.current_x;
        let y0 = self.current_y;
        // T
        let t_w = font::measure_text("T", fid, font_size);
        self.emit_text("T", font_size, style, color);
        self.current_x = x0 + t_w - font_size * 0.17;
        // E (lowered by ~0.5ex)
        self.current_y = y0 + font_size * 0.22;
        let e_w = font::measure_text("E", fid, font_size);
        self.emit_text("E", font_size, style, color);
        self.current_y = y0;
        self.current_x = x0 + t_w - font_size * 0.17 + e_w - font_size * 0.12;
        // X
        let x_w = font::measure_text("X", fid, font_size);
        self.emit_text("X", font_size, style, color);
        let total = t_w - font_size * 0.17 + e_w - font_size * 0.12 + x_w;
        self.current_x = x0 + total;
        total
    }

    /// Emit the LaTeX logo with proper kerning and baseline shifts
    /// Returns the total width consumed
    pub fn emit_latex_logo(&mut self, font_size: f32, style: FontStyle, color: Color) -> f32 {
        let fid = font::style_to_font_id(style);
        let x0 = self.current_x;
        let y0 = self.current_y;
        // L
        let l_w = font::measure_text("L", fid, font_size);
        self.emit_text("L", font_size, style, color);
        self.current_x = x0 + l_w - font_size * 0.04;
        // A (raised and smaller: ~70% size, shifted up ~0.25ex)
        let a_size = font_size * 0.7;
        self.current_y = y0 - font_size * 0.25;
        let a_w = font::measure_text("A", fid, a_size);
        self.emit_text("A", a_size, style, color);
        self.current_y = y0;
        self.current_x = x0 + l_w - font_size * 0.04 + a_w - font_size * 0.02;
        // TeX
        let tex_w = self.emit_tex_logo(font_size, style, color);
        let total = l_w - font_size * 0.04 + a_w - font_size * 0.02 + tex_w;
        self.current_x = x0 + total;
        total
    }

    pub fn emit_header_text(&mut self, text: &str, x: f32, y: f32, font_size: f32, base_style: FontStyle) {
        let font_size_100 = (font_size * 100.0) as u16;
        if text.bytes().all(|b| b < 0x80) {
            let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
            self.all_text.push_str(text);
            self.all_elements.push(PageElement::Text {
                x, y, text_offset: offset, text_len: text.len().min(65535) as u16,
                font_size_100, font_style: base_style, color: Color::BLACK, word_spacing_50: 0,
            });
            return;
        }
        let mut cur_x = x;
        let mut seg_start = 0;
        for (i, ch) in text.char_indices() {
            if (ch as u32) >= 0x0391 && (ch as u32) <= 0x03C9 {
                if let Some(sym_byte) = font::unicode_to_symbol_byte(ch) {
                    if i > seg_start {
                        let seg = &text[seg_start..i];
                        let offset = (self.all_text.len() - self.current_page_text_start as usize) as u32;
                        self.all_text.push_str(seg);
                        let w = font::measure_text(seg, FontId::TimesRoman, font_size);
                        self.all_elements.push(PageElement::Text {
                            x: cur_x, y, text_offset: offset, text_len: seg.len().min(65535) as u16,
                            font_size_100, font_style: base_style, color: Color::BLACK, word_spacing_50: 0,
                        });
                        cur_x += w;
                    }
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
    pub fn emit_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.all_elements.push(PageElement::Line {
            x1, y1, x2, y2, width_1000: (width * 1000.0) as u16, color,
        });
    }

    #[inline]
    pub fn emit_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<Color>) {
        let idx = self.rect_data.len() as u32;
        self.rect_data.push(RectData {
            x, y, width: w, height: h, fill, stroke, stroke_width: 0.5, corner_radius: 0.0,
        });
        self.all_elements.push(PageElement::Rect(idx));
    }

    pub fn emit_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<Color>, corner_radius: f32) {
        let idx = self.rect_data.len() as u32;
        self.rect_data.push(RectData {
            x, y, width: w, height: h, fill, stroke, stroke_width: 0.5, corner_radius,
        });
        self.all_elements.push(PageElement::Rect(idx));
    }
}
