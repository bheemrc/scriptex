/// Public types for the layout engine — used by pdf.rs and main.rs

use crate::color::Color;
use crate::typeset::FontStyle;

/// High bit flag indicating text_offset refers to source (mmap) rather than page text_buffer
pub const SOURCE_REF_FLAG: u32 = 0x80000000;

/// A clickable link annotation on a page
#[derive(Debug, Clone)]
pub struct LinkAnnotation {
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub url: String,           // external URL (empty for internal links)
    pub dest_page: Option<u32>, // internal destination page (0-indexed)
    pub dest_y: f32,           // internal destination y position
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
        angle: f32, // rotation angle in degrees (0 = no rotation)
    },
}

/// Embedded image data for PDF generation
#[derive(Debug, Clone)]
pub struct EmbeddedImage {
    pub data: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub format: ImageFormat,
    pub has_alpha: bool,
    pub alpha_data: Vec<u8>, // compressed alpha channel for SMask (PNG only)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    /// PDF page embedded as Form XObject
    Pdf {
        /// BBox: [x0 y0 x1 y1] in points
        bbox: [f32; 4],
        /// Raw PDF Resources dictionary bytes (to embed in Form XObject)
        resources: Vec<u8>,
    },
    Svg,
}
