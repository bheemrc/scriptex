/// Layout state: mutable context used by all layout functions

use std::collections::HashMap;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};
use super::types::*;
use super::prescans::{TocEntry, TocFixup};

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
    pub line_spacing: f32,
    pub section_counters: [u32; 7],
    pub figure_counter: u32,
    pub table_counter: u32,
    pub footnotes: Vec<Vec<Node>>,
    pub footnote_counter: u32,
    pub footnote_reserved: f32,
    pub suppress_next_indent: bool,
    pub list_depth: u32,
    pub text_buf: String,
    pub label_map: HashMap<String, String>,
    pub citation_map: HashMap<String, u32>,
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
    pub current_section_title: String,
    pub first_page: bool,
    pub is_amsart: bool,
    pub amsart_header_author: String,
    pub amsart_header_title: String,
    pub deferred_abstract_idx: Option<usize>,
    pub amsart_pre_title: bool,
}

impl LayoutState {
    pub fn new(page_setup: PageSetup, font_size: f32, line_spacing: f32) -> Self {
        let max_y = page_setup.height - page_setup.margin_bottom - page_setup.footer_height;
        let start_y = page_setup.margin_top + page_setup.header_height;
        let text_w = page_setup.text_width();
        let avg_w = font_size * 0.48;
        let lh = font_size * baselineskip_factor(font_size);
        let st = lh * line_spacing;
        let para_w = text_w - 17.0;
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
            paragraph_indent: 17.0,
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
    pub fn set_indent(&mut self, indent: f32) {
        self.indent = indent;
        self.cached_text_width = self.page_setup.text_width() - indent - self.right_indent;
        self.cached_text_left = self.page_setup.margin_left + indent;
        self.cached_font_key = u32::MAX;
    }

    #[inline(always)]
    pub fn set_right_indent(&mut self, right_indent: f32) {
        self.right_indent = right_indent;
        self.cached_text_width = self.page_setup.text_width() - self.indent - right_indent;
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
        let fn_line_height = fn_size * 1.3;
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
        let fn_line_height = fn_size * 1.3;

        let total_height = footnotes.len() as f32 * fn_line_height + 10.0;
        let orig_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
        let fn_y_start = orig_max_y - total_height;

        if fn_y_start < self.current_y + 20.0 {
            self.footnotes = footnotes;
            return;
        }

        self.emit_line(
            self.page_setup.margin_left,
            fn_y_start,
            self.page_setup.margin_left + self.page_setup.text_width() * 0.3,
            fn_y_start,
            0.4,
            Color::GRAY,
        );

        let source = self.source_str() as *const str;
        let source_ref = unsafe { &*source };
        let mut y = fn_y_start + 6.0;
        for (i, fn_content) in footnotes.iter().enumerate() {
            let num = fn_start_num + i as u32;
            let num_str = format!("{}  ", num);
            let x = self.page_setup.margin_left;

            let sup_size = fn_size * 0.75;
            self.current_x = x;
            self.current_y = y;
            self.emit_text(&num_str, sup_size, FontStyle::Regular, Color::BLACK);

            let mut fn_text = String::new();
            for node in fn_content {
                super::text::node_to_text(node, &mut fn_text, source_ref);
            }
            let fn_text = fn_text.trim().to_string();
            let text_x = x + font::measure_text(&num_str, FontId::Helvetica, sup_size);
            self.current_x = text_x;
            self.emit_text(&fn_text, fn_size, FontStyle::Regular, Color::BLACK);

            y += fn_line_height;
        }
    }

    pub fn new_page(&mut self) {
        self.render_footnotes();

        let effective_style = if self.first_page { PageStyle::Plain } else { self.page_style };

        match effective_style {
            PageStyle::Plain => {
                self.emit_page_number_centered();
            }
            PageStyle::Headings => {
                let header_font_size = 9.0;
                let header_y = self.page_setup.margin_top - 14.0;
                let left_x = self.page_setup.margin_left;
                let right_x = self.page_setup.width - self.page_setup.margin_right;

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
                            let author_w = font::measure_text(&author[..author_len], FontId::Helvetica, header_font_size);
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
                    let rule_y = header_y + 4.0;
                    self.all_elements.push(PageElement::Line {
                        x1: left_x, y1: rule_y, x2: right_x, y2: rule_y,
                        width_1000: 400, color: Color::BLACK,
                    });
                }
            }
            PageStyle::Empty => {}
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
        self.current_x = self.text_left();
        self.current_y = self.cached_start_y;
        self.footnote_reserved = 0.0;
        self.cached_max_y = self.page_setup.height - self.page_setup.margin_bottom
            - self.page_setup.footer_height;
    }

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
            x: center_x - text_width / 2.0, y, text_offset: offset,
            text_len: num_len as u16, font_size_100: 900,
            font_style: FontStyle::Regular, color: Color::GRAY, word_spacing_50: 0,
        });
    }

    #[inline(always)]
    pub fn ensure_space(&mut self, height: f32) {
        if self.current_y + height > self.cached_max_y {
            self.new_page();
        }
    }

    #[inline(always)]
    pub fn add_vertical_space(&mut self, space: f32) {
        self.current_y += space;
        if self.current_y > self.cached_max_y {
            self.new_page();
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
                        let w = font::measure_text(seg, FontId::Helvetica, font_size);
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
