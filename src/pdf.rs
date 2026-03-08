/// Direct PDF generation - bypasses intermediate formats for maximum speed
/// Uses parallel page content generation via rayon (when available)

use anyhow::Result;
use std::io::Write;
#[cfg(feature = "rayon")]
use rayon::prelude::*;
use crate::document::Document;
use crate::layout::{LayoutResult, PageElement};
use crate::typeset::FontStyle;

/// Generate a PDF file from laid-out pages (filesystem version, CLI only)
/// Uses batched generation+write to keep peak memory low (~3MB vs 150MB)
pub fn generate_pdf(layout: &LayoutResult, doc: &Document, output: &std::path::Path, source: &str) -> Result<usize> {
    use std::io::BufWriter;

    let file = std::fs::File::create(output)?;
    let mut writer = BufWriter::with_capacity(16 * 1024 * 1024, file);
    let bytes_written = write_pdf_streaming(&mut writer, layout, doc, source)?;
    writer.flush()?;

    Ok(bytes_written)
}

/// Generate PDF to any writer (in-memory Vec<u8>, BufWriter, etc.)
/// Used by both CLI and WASM paths.
pub fn write_pdf_to_writer<W: Write>(writer: &mut W, layout: &LayoutResult, doc: &Document, source: &str) -> Result<usize> {
    write_pdf_streaming(writer, layout, doc, source)
}

/// Counting writer wrapper - tracks bytes written for xref offsets
struct CountingWriter<W: Write> {
    inner: W,
    count: usize,
}

impl<W: Write> CountingWriter<W> {
    fn new(inner: W) -> Self { Self { inner, count: 0 } }
    fn position(&self) -> usize { self.count }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n;
        Ok(n)
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.inner.write_all(buf)?;
        self.count += buf.len();
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

fn write_pdf_streaming<W: Write>(writer: W, layout: &LayoutResult, doc: &Document, source: &str) -> Result<usize> {
    let mut w = CountingWriter::new(writer);
    let num_pages = layout.num_pages();

    // Header
    w.write_all(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")?;

    let mut next_obj: u32 = 1;
    let mut offsets: Vec<usize> = Vec::with_capacity(num_pages * 2 + 20);

    let mut alloc_obj = || { let id = next_obj; next_obj += 1; id };

    // Pre-allocate object IDs
    let catalog_id = alloc_obj();
    let pages_id = alloc_obj();
    let font_regular_id = alloc_obj();
    let font_bold_id = alloc_obj();
    let font_italic_id = alloc_obj();
    let font_bolditalic_id = alloc_obj();
    let font_mono_id = alloc_obj();
    let font_symbol_id = alloc_obj();
    let font_times_roman_id = alloc_obj();
    let font_times_italic_id = alloc_obj();
    let font_times_bold_id = alloc_obj();
    let font_times_bolditalic_id = alloc_obj();
    let font_dingbats_id = alloc_obj();
    let resource_id = alloc_obj();

    let mut page_ids = Vec::with_capacity(num_pages);
    let mut content_ids = Vec::with_capacity(num_pages);
    for _ in 0..num_pages {
        page_ids.push(alloc_obj());
        content_ids.push(alloc_obj());
    }

    // Allocate IDs for embedded images
    let num_images = layout.images.len();
    let mut image_ids = Vec::with_capacity(num_images);
    for _ in 0..num_images {
        image_ids.push(alloc_obj());
    }

    // Allocate IDs for link annotations
    let num_links = layout.links.len();
    let mut link_ids = Vec::with_capacity(num_links);
    for _ in 0..num_links {
        link_ids.push(alloc_obj());
    }
    // Group links by page for efficient lookup
    let mut links_by_page: Vec<Vec<usize>> = vec![Vec::new(); num_pages];
    for (i, link) in layout.links.iter().enumerate() {
        let page = link.page as usize;
        if page < num_pages {
            links_by_page[page].push(i);
        }
    }

    // Allocate outline object IDs
    let num_outlines = layout.outlines.len();
    let outline_root_id = if num_outlines > 0 { Some(alloc_obj()) } else { None };
    let mut outline_item_ids = Vec::with_capacity(num_outlines);
    for _ in 0..num_outlines {
        outline_item_ids.push(alloc_obj());
    }

    // Pre-fill offsets vec
    let total_objs = next_obj as usize;
    offsets.resize(total_objs, 0);

    let mut itoa_obj = itoa::Buffer::new();
    macro_rules! begin_obj {
        ($id:expr) => {{
            let id_val = $id;
            offsets[id_val as usize - 1] = w.position();
            w.write_all(itoa_obj.format(id_val).as_bytes())?;
            w.write_all(b" 0 obj\n")?;
        }};
    }

    // Catalog
    begin_obj!(catalog_id);
    w.write_all(b"<< /Type /Catalog /Pages ")?;
    w.write_all(itoa_obj.format(pages_id).as_bytes())?;
    w.write_all(b" 0 R")?;
    if let Some(outline_id) = outline_root_id {
        w.write_all(b" /Outlines ")?;
        w.write_all(itoa_obj.format(outline_id).as_bytes())?;
        w.write_all(b" 0 R /PageMode /UseOutlines")?;
    }
    w.write_all(b" >>\nendobj\n")?;

    // Pages object - write kids directly
    begin_obj!(pages_id);
    w.write_all(b"<< /Type /Pages /Kids [")?;
    {
        let mut kids_buf = Vec::with_capacity(num_pages * 12);
        for id in &page_ids {
            kids_buf.extend_from_slice(itoa_obj.format(*id).as_bytes());
            kids_buf.extend_from_slice(b" 0 R ");
        }
        w.write_all(&kids_buf)?;
    }
    w.write_all(b"] /Count ")?;
    w.write_all(itoa_obj.format(num_pages).as_bytes())?;
    w.write_all(b" >>\nendobj\n")?;

    // Fonts (WinAnsi encoding for text fonts)
    for (id, name) in [
        (font_regular_id, "Helvetica"),
        (font_bold_id, "Helvetica-Bold"),
        (font_italic_id, "Helvetica-Oblique"),
        (font_bolditalic_id, "Helvetica-BoldOblique"),
        (font_mono_id, "Courier"),
    ] {
        begin_obj!(id);
        w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /")?;
        w.write_all(name.as_bytes())?;
        w.write_all(b" /Encoding /WinAnsiEncoding >>\nendobj\n")?;
    }
    // Symbol font uses its own encoding (not WinAnsi)
    begin_obj!(font_symbol_id);
    w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>\nendobj\n")?;

    // Times fonts (WinAnsi encoding) — for math rendering
    for (id, name) in [
        (font_times_roman_id, "Times-Roman"),
        (font_times_italic_id, "Times-Italic"),
        (font_times_bold_id, "Times-Bold"),
        (font_times_bolditalic_id, "Times-BoldItalic"),
    ] {
        begin_obj!(id);
        w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /")?;
        w.write_all(name.as_bytes())?;
        w.write_all(b" /Encoding /WinAnsiEncoding >>\nendobj\n")?;
    }

    // ZapfDingbats font uses its own encoding (not WinAnsi)
    begin_obj!(font_dingbats_id);
    w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>\nendobj\n")?;

    // Resources
    begin_obj!(resource_id);
    w.write_all(b"<< /Font << /F1 ")?;
    w.write_all(itoa_obj.format(font_regular_id).as_bytes())?;
    w.write_all(b" 0 R /F2 ")?;
    w.write_all(itoa_obj.format(font_bold_id).as_bytes())?;
    w.write_all(b" 0 R /F3 ")?;
    w.write_all(itoa_obj.format(font_italic_id).as_bytes())?;
    w.write_all(b" 0 R /F4 ")?;
    w.write_all(itoa_obj.format(font_bolditalic_id).as_bytes())?;
    w.write_all(b" 0 R /F5 ")?;
    w.write_all(itoa_obj.format(font_mono_id).as_bytes())?;
    w.write_all(b" 0 R /F6 ")?;
    w.write_all(itoa_obj.format(font_symbol_id).as_bytes())?;
    w.write_all(b" 0 R /F7 ")?;
    w.write_all(itoa_obj.format(font_times_roman_id).as_bytes())?;
    w.write_all(b" 0 R /F8 ")?;
    w.write_all(itoa_obj.format(font_times_italic_id).as_bytes())?;
    w.write_all(b" 0 R /F9 ")?;
    w.write_all(itoa_obj.format(font_times_bold_id).as_bytes())?;
    w.write_all(b" 0 R /F10 ")?;
    w.write_all(itoa_obj.format(font_dingbats_id).as_bytes())?;
    w.write_all(b" 0 R /F11 ")?;
    w.write_all(itoa_obj.format(font_times_bolditalic_id).as_bytes())?;
    w.write_all(b" 0 R >>")?;
    // XObject references for embedded images
    if !image_ids.is_empty() {
        w.write_all(b" /XObject << ")?;
        for (i, &img_id) in image_ids.iter().enumerate() {
            w.write_all(b"/Im")?;
            w.write_all(itoa_obj.format(i + 1).as_bytes())?;
            w.write_all(b" ")?;
            w.write_all(itoa_obj.format(img_id).as_bytes())?;
            w.write_all(b" 0 R ")?;
        }
        w.write_all(b">>")?;
    }
    w.write_all(b" >>\nendobj\n")?;

    // Pre-build page object constant parts (same for all pages)
    let mut itoa_buf = itoa::Buffer::new();
    let mut page_prefix = Vec::with_capacity(128);
    page_prefix.extend_from_slice(b"<< /Type /Page /Parent ");
    page_prefix.extend_from_slice(itoa_buf.format(pages_id).as_bytes());
    page_prefix.extend_from_slice(b" 0 R /MediaBox [0 0 ");
    if num_pages > 0 {
        page_prefix.extend_from_slice(itoa_buf.format(layout.width as u32).as_bytes());
        page_prefix.push(b' ');
        page_prefix.extend_from_slice(itoa_buf.format(layout.height as u32).as_bytes());
    }
    page_prefix.extend_from_slice(b"] /Contents ");

    // page_suffix replaced by inline writing to support per-page annotations

    // Generate and write content streams in batches to reduce peak memory
    // Each batch generates ~1000 page contents in parallel, then writes them
    const BATCH_SIZE: usize = 5000;
    for batch_start in (0..num_pages).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE).min(num_pages);

        // Generate batch (parallel with rayon, sequential in WASM)
        #[cfg(feature = "rayon")]
        let batch_contents: Vec<Vec<u8>> = (batch_start..batch_end).into_par_iter()
            .map(|i| generate_page_content(
                layout.page_elements(i),
                layout.page_text(i),
                &layout.rect_data,
                layout.height,
                source,
            ))
            .collect();

        #[cfg(not(feature = "rayon"))]
        let batch_contents: Vec<Vec<u8>> = (batch_start..batch_end)
            .map(|i| generate_page_content(
                layout.page_elements(i),
                layout.page_text(i),
                &layout.rect_data,
                layout.height,
                source,
            ))
            .collect();

        // Write batch sequentially (with FlateDecode compression for reasonable-sized documents)
        // Skip compression for very large documents (>10K pages) to avoid excessive CPU time
        let use_compression = num_pages <= 10000;

        for (j, content) in batch_contents.iter().enumerate() {
            let i = batch_start + j;

            begin_obj!(content_ids[i]);
            if use_compression {
                let compressed = miniz_oxide::deflate::compress_to_vec_zlib(content, 4);
                w.write_all(b"<< /Filter /FlateDecode /Length ")?;
                w.write_all(itoa_buf.format(compressed.len()).as_bytes())?;
                w.write_all(b" >>\nstream\n")?;
                w.write_all(&compressed)?;
            } else {
                w.write_all(b"<< /Length ")?;
                w.write_all(itoa_buf.format(content.len()).as_bytes())?;
                w.write_all(b" >>\nstream\n")?;
                w.write_all(content)?;
            }
            w.write_all(b"\nendstream\nendobj\n")?;

            // Page object
            begin_obj!(page_ids[i]);
            w.write_all(&page_prefix)?;
            w.write_all(itoa_buf.format(content_ids[i]).as_bytes())?;
            w.write_all(b" 0 R /Resources ")?;
            w.write_all(itoa_buf.format(resource_id).as_bytes())?;
            w.write_all(b" 0 R")?;
            // Add link annotations for this page
            if !links_by_page[i].is_empty() {
                w.write_all(b" /Annots [")?;
                for &link_idx in &links_by_page[i] {
                    w.write_all(itoa_buf.format(link_ids[link_idx]).as_bytes())?;
                    w.write_all(b" 0 R ")?;
                }
                w.write_all(b"]")?;
            }
            w.write_all(b" >>\nendobj\n")?;
        }
        // batch_contents dropped here, freeing ~6MB
    }

    // Write image XObjects
    for (i, img) in layout.images.iter().enumerate() {
        let img_id = image_ids[i];
        if img_id as usize > offsets.len() { offsets.resize(img_id as usize, 0); }
        begin_obj!(img_id);
        write_image_xobject(&mut w, img, i + 1)?;
    }

    // Write link annotation objects
    for (i, link) in layout.links.iter().enumerate() {
        let link_id = link_ids[i];
        if link_id as usize > offsets.len() { offsets.resize(link_id as usize, 0); }
        begin_obj!(link_id);
        // PDF coordinates: y=0 is bottom of page
        let y1 = layout.height - link.y - link.height;
        let y2 = layout.height - link.y;
        let x1 = link.x;
        let x2 = link.x + link.width;
        write!(w, "<< /Type /Annot /Subtype /Link /Rect [{:.1} {:.1} {:.1} {:.1}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({}) >> >>\nendobj\n",
            x1, y1, x2, y2, escape_pdf_string(&link.url))?;
    }

    // Write outline objects (PDF bookmarks)
    if let Some(outline_root) = outline_root_id {
        if outline_root as usize > offsets.len() { offsets.resize(outline_root as usize, 0); }
        // Build outline tree — flat list with proper sibling links
        // Only include top-level items (section level ≤ 2) for cleaner bookmarks
        let top_entries: Vec<usize> = (0..num_outlines)
            .filter(|&i| layout.outlines[i].level <= 2)
            .collect();

        begin_obj!(outline_root);
        if let Some(&first_idx) = top_entries.first() {
            w.write_all(b"<< /Type /Outlines /First ")?;
            w.write_all(itoa_obj.format(outline_item_ids[first_idx]).as_bytes())?;
            w.write_all(b" 0 R /Last ")?;
            let last_idx = top_entries[top_entries.len() - 1];
            w.write_all(itoa_obj.format(outline_item_ids[last_idx]).as_bytes())?;
            w.write_all(b" 0 R /Count ")?;
            w.write_all(itoa_obj.format(top_entries.len()).as_bytes())?;
            w.write_all(b" >>\nendobj\n")?;
        } else {
            w.write_all(b"<< /Type /Outlines /Count 0 >>\nendobj\n")?;
        }

        // Write individual outline items
        for (pos, &idx) in top_entries.iter().enumerate() {
            let entry = &layout.outlines[idx];
            let item_id = outline_item_ids[idx];
            if item_id as usize > offsets.len() { offsets.resize(item_id as usize, 0); }
            begin_obj!(item_id);
            w.write_all(b"<< /Title (")?;
            w.write_all(escape_pdf_string(&entry.title).as_bytes())?;
            w.write_all(b") /Parent ")?;
            w.write_all(itoa_obj.format(outline_root).as_bytes())?;
            w.write_all(b" 0 R")?;

            // Destination: page + position
            let page_idx = (entry.page as usize).min(page_ids.len().saturating_sub(1));
            let dest_y = layout.height - entry.y;
            w.write_all(b" /Dest [")?;
            w.write_all(itoa_obj.format(page_ids[page_idx]).as_bytes())?;
            write!(w, " 0 R /XYZ 0 {:.0} 0]", dest_y)?;

            // Sibling links
            if pos > 0 {
                w.write_all(b" /Prev ")?;
                w.write_all(itoa_obj.format(outline_item_ids[top_entries[pos - 1]]).as_bytes())?;
                w.write_all(b" 0 R")?;
            }
            if pos + 1 < top_entries.len() {
                w.write_all(b" /Next ")?;
                w.write_all(itoa_obj.format(outline_item_ids[top_entries[pos + 1]]).as_bytes())?;
                w.write_all(b" 0 R")?;
            }
            w.write_all(b" >>\nendobj\n")?;
        }
    }

    // Metadata
    let info_id = next_obj;
    next_obj += 1;
    if info_id as usize > offsets.len() { offsets.resize(info_id as usize, 0); }
    begin_obj!(info_id);
    w.write_all(b"<< /Producer (SonicSpeedLaTeX 0.1)")?;
    if let Some(title) = &doc.preamble.title {
        w.write_all(b" /Title (")?;
        w.write_all(escape_pdf_string(title).as_bytes())?;
        w.write_all(b")")?;
    }
    if let Some(author) = &doc.preamble.author {
        w.write_all(b" /Author (")?;
        w.write_all(escape_pdf_string(author).as_bytes())?;
        w.write_all(b")")?;
    }
    w.write_all(b" /Creator (SonicSpeedLaTeX) >>\nendobj\n")?;

    // Cross-reference table
    let xref_offset = w.position();
    w.write_all(b"xref\n0 ")?;
    w.write_all(itoa_obj.format(next_obj).as_bytes())?;
    w.write_all(b"\n0000000000 65535 f \n")?;

    // Batch xref entries
    {
        let xref_count = next_obj as usize - 1;
        let mut xref_buf = Vec::with_capacity(xref_count * 20);
        let mut entry = *b"0000000000 00000 n \n";
        for i in 0..xref_count {
            let mut offset = offsets[i];
            for j in (0..10).rev() {
                entry[j] = b'0' + (offset % 10) as u8;
                offset /= 10;
            }
            xref_buf.extend_from_slice(&entry);
        }
        w.write_all(&xref_buf)?;
    }

    // Trailer
    w.write_all(b"trailer\n<< /Size ")?;
    w.write_all(itoa_obj.format(next_obj).as_bytes())?;
    w.write_all(b" /Root 1 0 R /Info ")?;
    w.write_all(itoa_obj.format(info_id).as_bytes())?;
    w.write_all(b" 0 R >>\nstartxref\n")?;
    w.write_all(itoa_obj.format(xref_offset).as_bytes())?;
    w.write_all(b"\n%%EOF\n")?;

    Ok(w.position())
}

/// Generate PDF content stream for a single page - designed to be called in parallel
fn generate_page_content(elements: &[PageElement], text_buffer: &str, rect_data: &[crate::layout::RectData], height: f32, source: &str) -> Vec<u8> {
    use crate::layout::SOURCE_REF_FLAG;
    // Estimate capacity: each element ~80 bytes (text + formatting + coordinates)
    let estimated_size = elements.len() * 80 + text_buffer.len() + 512;
    let mut c = Vec::with_capacity(estimated_size);

    let mut current_font: u8 = 0; // 0 = none set
    let mut current_size: u16 = 0; // size * 100 for comparison
    let mut current_tw50: i16 = 0; // word spacing * 50 for justified text
    let mut cr: u8 = 0;
    let mut cg: u8 = 0;
    let mut cb: u8 = 0;
    let mut in_bt = false; // track whether we're inside a BT block
    let mut cur_tx: f32 = 0.0; // current text position (Td is relative!)
    let mut cur_ty: f32 = 0.0;

    for elem in elements {
        match elem {
            PageElement::Text { x, y, text_offset, text_len, font_size_100, font_style, color, word_spacing_50 } => {
                if *text_len == 0 {
                    continue;
                }
                let tlen = *text_len as usize;
                // Decode text source: high bit = source reference, else page text_buffer
                let text = if *text_offset & SOURCE_REF_FLAG != 0 {
                    let off = (*text_offset & !SOURCE_REF_FLAG) as usize;
                    if off + tlen > source.len() { continue; }
                    &source[off..off + tlen]
                } else {
                    let off = *text_offset as usize;
                    if off + tlen > text_buffer.len() { continue; }
                    &text_buffer[off..off + tlen]
                };

                let font_id = match font_style {
                    FontStyle::Regular | FontStyle::SmallCaps => 7u8, // Times-Roman (serif body)
                    FontStyle::Bold => 9,                              // Times-Bold
                    FontStyle::Italic => 8,                            // Times-Italic
                    FontStyle::BoldItalic => 11,                       // Times-BoldItalic
                    FontStyle::Monospace => 5,
                    FontStyle::Symbol => 6,
                    FontStyle::TimesRoman => 7,
                    FontStyle::TimesItalic => 8,
                    FontStyle::TimesBold => 9,
                    FontStyle::ZapfDingbats => 10,
                };

                let pdf_y = height - y;
                let font_size = *font_size_100 as f32 * 0.01;

                if !in_bt {
                    c.extend_from_slice(b"BT\n");
                    in_bt = true;
                    cur_tx = 0.0;
                    cur_ty = 0.0;
                }

                if color.r != cr || color.g != cg || color.b != cb {
                    write_f32_3(&mut c, color.r_f32(), color.g_f32(), color.b_f32());
                    c.extend_from_slice(b" rg\n");
                    cr = color.r; cg = color.g; cb = color.b;
                }

                if font_id != current_font || *font_size_100 != current_size {
                    c.push(b'/');
                    c.push(b'F');
                    if font_id >= 10 {
                        c.push(b'0' + font_id / 10);
                        c.push(b'0' + font_id % 10);
                    } else {
                        c.push(b'0' + font_id);
                    }
                    c.push(b' ');
                    write_f32_fast(&mut c, font_size);
                    c.extend_from_slice(b" Tf\n");
                    current_font = font_id;
                    current_size = *font_size_100;
                }

                // Word spacing for justified text (Tw operator)
                if *word_spacing_50 != current_tw50 {
                    if *word_spacing_50 != 0 {
                        let ws = *word_spacing_50 as f32 * 0.02;
                        write_f32_fast(&mut c, ws);
                        c.extend_from_slice(b" Tw\n");
                    } else {
                        c.extend_from_slice(b"0 Tw\n");
                    }
                    current_tw50 = *word_spacing_50;
                }

                // Position (Td is relative to current text position)
                let dx = *x - cur_tx;
                let dy = pdf_y - cur_ty;
                write_f32_fast(&mut c, dx);
                c.push(b' ');
                write_f32_fast(&mut c, dy);
                c.extend_from_slice(b" Td\n(");
                cur_tx = *x;
                cur_ty = pdf_y;

                // Text escaping + encoding conversion
                let text_bytes = text.as_bytes();
                let tlen = text_bytes.len();
                if font_id == 6 || font_id == 10 {
                    // Symbol/ZapfDingbats font: each char is a Unicode codepoint U+00XX where XX
                    // is the font encoding byte. We must extract the byte value from the char,
                    // NOT iterate over UTF-8 bytes (which would corrupt bytes > 127).
                    for ch in text.chars() {
                        let b = ch as u8;
                        match b {
                            b'(' => c.extend_from_slice(b"\\("),
                            b')' => c.extend_from_slice(b"\\)"),
                            b'\\' => c.extend_from_slice(b"\\\\"),
                            _ => c.push(b),
                        }
                    }
                } else {
                // WinAnsiEncoding conversion
                // Also handles LaTeX quote ligatures: `` → left dquote, '' → right dquote
                let mut tpos = 0;
                while tpos < tlen {
                    // Fast ASCII scan: find next byte needing special handling
                    // Special bytes: ( ) \ ` (escaping), \n \r (→space), >= 0x80 (non-ASCII)
                    let scan_start = tpos;
                    while tpos < tlen {
                        let b = text_bytes[tpos];
                        if b == b'(' || b == b')' || b == b'\\' || b == b'`' || b == b'\n' || b == b'\r' || b >= 0x80 {
                            break;
                        }
                        tpos += 1;
                    }
                    // Emit safe ASCII run, converting '' to right double quote and --/--- to en/em-dash
                    if tpos > scan_start {
                        let run = &text_bytes[scan_start..tpos];
                        // Check if run contains ligature chars (' or -)
                        let has_quote = memchr::memchr(b'\'', run).is_some();
                        let has_dash = memchr::memchr(b'-', run).is_some();
                        if has_quote || has_dash {
                            let mut rpos = 0;
                            while rpos < run.len() {
                                if run[rpos] == b'\'' && rpos + 1 < run.len() && run[rpos + 1] == b'\'' {
                                    c.push(0x94); // WinAnsi right double quote
                                    rpos += 2;
                                } else if run[rpos] == b'-' && rpos + 1 < run.len() && run[rpos + 1] == b'-' {
                                    if rpos + 2 < run.len() && run[rpos + 2] == b'-' {
                                        c.push(0x97); // WinAnsi em-dash (---)
                                        rpos += 3;
                                    } else {
                                        c.push(0x96); // WinAnsi en-dash (--)
                                        rpos += 2;
                                    }
                                } else {
                                    c.push(run[rpos]);
                                    rpos += 1;
                                }
                            }
                        } else {
                            c.extend_from_slice(run);
                        }
                    }
                    if tpos >= tlen { break; }

                    let b = text_bytes[tpos];
                    if b < 0x80 {
                        // ASCII special char + LaTeX quote ligatures
                        match b {
                            b'\n' | b'\r' => c.push(b' '), // newlines → space
                            b'(' => c.extend_from_slice(b"\\("),
                            b')' => c.extend_from_slice(b"\\)"),
                            b'\\' => c.extend_from_slice(b"\\\\"),
                            b'`' => {
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'`' {
                                    c.push(0x93); // WinAnsi left double quote
                                    tpos += 1;
                                } else {
                                    c.push(0x91); // WinAnsi left single quote
                                }
                            }
                            b'\'' => {
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'\'' {
                                    c.push(0x94); // WinAnsi right double quote
                                    tpos += 1;
                                } else {
                                    c.push(b);
                                }
                            }
                            _ => c.push(b),
                        }
                        tpos += 1;
                    } else if b < 0xC0 {
                        tpos += 1; // stray continuation byte
                    } else {
                        // UTF-8 multi-byte: decode and convert to WinAnsi
                        let (codepoint, advance) = if b < 0xE0 && tpos + 1 < tlen {
                            (((b as u32 & 0x1F) << 6) | (text_bytes[tpos+1] as u32 & 0x3F), 2)
                        } else if b < 0xF0 && tpos + 2 < tlen {
                            (((b as u32 & 0x0F) << 12) | ((text_bytes[tpos+1] as u32 & 0x3F) << 6)
                                | (text_bytes[tpos+2] as u32 & 0x3F), 3)
                        } else if tpos + 3 < tlen {
                            (((b as u32 & 0x07) << 18) | ((text_bytes[tpos+1] as u32 & 0x3F) << 12)
                                | ((text_bytes[tpos+2] as u32 & 0x3F) << 6) | (text_bytes[tpos+3] as u32 & 0x3F), 4)
                        } else {
                            (0xFFFD, 1)
                        };
                        c.push(match codepoint {
                            0x00A0..=0x00FF => codepoint as u8,
                            0x2022 => 0x95, 0x2013 => 0x96, 0x2014 => 0x97,
                            0x2018 => 0x91, 0x2019 => 0x92, 0x201C => 0x93, 0x201D => 0x94,
                            0x2026 => 0x85, 0x2020 => 0x86, 0x2021 => 0x87, 0x2030 => 0x89,
                            0x0152 => 0x8C, 0x0153 => 0x9C, 0x0160 => 0x8A, 0x0161 => 0x9A,
                            0x0178 => 0x9F, 0x017D => 0x8E, 0x017E => 0x9E, 0x0192 => 0x83,
                            0x02C6 => 0x88, 0x02DC => 0x98, 0x20AC => 0x80, 0x2122 => 0x99,
                            _ => b'?',
                        });
                        tpos += advance;
                    }
                }
                } // end WinAnsi branch

                c.extend_from_slice(b") Tj\n");
            }

            PageElement::Line { x1, y1, x2, y2, width_1000, color } => {
                if in_bt {
                    c.extend_from_slice(b"ET\n");
                    in_bt = false;
                }
                let pdf_y1 = height - y1;
                let pdf_y2 = height - y2;

                c.extend_from_slice(b"q\n");
                write_f32_3(&mut c, color.r_f32(), color.g_f32(), color.b_f32());
                c.extend_from_slice(b" RG\n");
                write_f32_fast(&mut c, *width_1000 as f32 * 0.001);
                c.extend_from_slice(b" w\n");
                write_f32_fast(&mut c, *x1);
                c.push(b' ');
                write_f32_fast(&mut c, pdf_y1);
                c.extend_from_slice(b" m ");
                write_f32_fast(&mut c, *x2);
                c.push(b' ');
                write_f32_fast(&mut c, pdf_y2);
                c.extend_from_slice(b" l S\nQ\n");
            }

            PageElement::Rect(rect_idx) => {
                if in_bt {
                    c.extend_from_slice(b"ET\n");
                    in_bt = false;
                }
                let rect = &rect_data[*rect_idx as usize];
                let pdf_y = height - rect.y - rect.height;

                c.extend_from_slice(b"q\n");

                if let Some(fc) = &rect.fill {
                    write_f32_3(&mut c, fc.r_f32(), fc.g_f32(), fc.b_f32());
                    c.extend_from_slice(b" rg\n");
                }
                if let Some(sc) = &rect.stroke {
                    write_f32_3(&mut c, sc.r_f32(), sc.g_f32(), sc.b_f32());
                    c.extend_from_slice(b" RG\n");
                    write_f32_fast(&mut c, rect.stroke_width);
                    c.extend_from_slice(b" w\n");
                }

                if rect.corner_radius > 0.0 {
                    // Rounded rect / circle via Bézier curves
                    let r = rect.corner_radius.min(rect.width / 2.0).min(rect.height / 2.0);
                    let x = rect.x;
                    let y = pdf_y;
                    let w = rect.width;
                    let h = rect.height;
                    // Kappa for quarter-circle Bézier approximation
                    let k = r * 0.5523;

                    // Move to start (top-left + r)
                    write_f32_fast(&mut c, x + r); c.push(b' ');
                    write_f32_fast(&mut c, y + h); c.extend_from_slice(b" m\n");
                    // Top edge → top-right corner
                    write_f32_fast(&mut c, x + w - r); c.push(b' ');
                    write_f32_fast(&mut c, y + h); c.extend_from_slice(b" l\n");
                    // Top-right arc
                    write_f32_fast(&mut c, x + w - r + k); c.push(b' ');
                    write_f32_fast(&mut c, y + h); c.push(b' ');
                    write_f32_fast(&mut c, x + w); c.push(b' ');
                    write_f32_fast(&mut c, y + h - r + k); c.push(b' ');
                    write_f32_fast(&mut c, x + w); c.push(b' ');
                    write_f32_fast(&mut c, y + h - r); c.extend_from_slice(b" c\n");
                    // Right edge → bottom-right corner
                    write_f32_fast(&mut c, x + w); c.push(b' ');
                    write_f32_fast(&mut c, y + r); c.extend_from_slice(b" l\n");
                    // Bottom-right arc
                    write_f32_fast(&mut c, x + w); c.push(b' ');
                    write_f32_fast(&mut c, y + r - k); c.push(b' ');
                    write_f32_fast(&mut c, x + w - r + k); c.push(b' ');
                    write_f32_fast(&mut c, y); c.push(b' ');
                    write_f32_fast(&mut c, x + w - r); c.push(b' ');
                    write_f32_fast(&mut c, y); c.extend_from_slice(b" c\n");
                    // Bottom edge → bottom-left corner
                    write_f32_fast(&mut c, x + r); c.push(b' ');
                    write_f32_fast(&mut c, y); c.extend_from_slice(b" l\n");
                    // Bottom-left arc
                    write_f32_fast(&mut c, x + r - k); c.push(b' ');
                    write_f32_fast(&mut c, y); c.push(b' ');
                    write_f32_fast(&mut c, x); c.push(b' ');
                    write_f32_fast(&mut c, y + r - k); c.push(b' ');
                    write_f32_fast(&mut c, x); c.push(b' ');
                    write_f32_fast(&mut c, y + r); c.extend_from_slice(b" c\n");
                    // Left edge → top-left corner
                    write_f32_fast(&mut c, x); c.push(b' ');
                    write_f32_fast(&mut c, y + h - r); c.extend_from_slice(b" l\n");
                    // Top-left arc
                    write_f32_fast(&mut c, x); c.push(b' ');
                    write_f32_fast(&mut c, y + h - r + k); c.push(b' ');
                    write_f32_fast(&mut c, x + r - k); c.push(b' ');
                    write_f32_fast(&mut c, y + h); c.push(b' ');
                    write_f32_fast(&mut c, x + r); c.push(b' ');
                    write_f32_fast(&mut c, y + h); c.extend_from_slice(b" c\n");
                    c.extend_from_slice(b"h\n"); // close path
                } else {
                    write_f32_fast(&mut c, rect.x);
                    c.push(b' ');
                    write_f32_fast(&mut c, pdf_y);
                    c.push(b' ');
                    write_f32_fast(&mut c, rect.width);
                    c.push(b' ');
                    write_f32_fast(&mut c, rect.height);
                    c.extend_from_slice(b" re\n");
                }

                match (rect.fill.is_some(), rect.stroke.is_some()) {
                    (true, true) => c.extend_from_slice(b"B\n"),
                    (true, false) => c.extend_from_slice(b"f\n"),
                    (false, true) => c.extend_from_slice(b"S\n"),
                    (false, false) => {}
                }

                c.extend_from_slice(b"Q\n");
            }

            PageElement::Image { x, y, width: img_w, height: img_h, image_idx } => {
                if in_bt {
                    c.extend_from_slice(b"ET\n");
                    in_bt = false;
                }
                // PDF image drawing: save state, apply CTM, draw, restore
                // CTM maps unit square [0,0]-[1,1] to [x,y]-[x+w,y+h] in PDF coords
                let pdf_y = height - y - img_h;
                c.extend_from_slice(b"q\n");
                write_f32_fast(&mut c, *img_w);
                c.extend_from_slice(b" 0 0 ");
                write_f32_fast(&mut c, *img_h);
                c.push(b' ');
                write_f32_fast(&mut c, *x);
                c.push(b' ');
                write_f32_fast(&mut c, pdf_y);
                c.extend_from_slice(b" cm\n/Im");
                c.extend_from_slice(itoa::Buffer::new().format(*image_idx + 1).as_bytes());
                c.extend_from_slice(b" Do\nQ\n");
            }
        }
    }

    if in_bt {
        c.extend_from_slice(b"ET\n");
    }

    c
}

/// Fast f32 formatting with 3 decimal places for sub-point accuracy
#[inline]
fn write_f32_fast(buf: &mut Vec<u8>, val: f32) {
    let negative = val < 0.0;
    let val = if negative { -val } else { val };
    let mut int_part = val as u32;
    let mut frac_1000 = ((val - int_part as f32) * 1000.0 + 0.5) as u32;
    // Handle rounding carry (e.g., 1.9995 → frac rounds to 1000)
    if frac_1000 >= 1000 {
        int_part += 1;
        frac_1000 = 0;
    }

    if negative { buf.push(b'-'); }

    // Write integer part
    if int_part == 0 {
        buf.push(b'0');
    } else {
        let mut tmp = [0u8; 10];
        let mut pos = 10;
        let mut n = int_part;
        while n > 0 {
            pos -= 1;
            tmp[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        buf.extend_from_slice(&tmp[pos..10]);
    }

    // Write fractional part (3 decimal places, trailing zeros stripped)
    if frac_1000 > 0 {
        buf.push(b'.');
        let f = frac_1000;
        buf.push(b'0' + (f / 100) as u8);
        let tens = ((f / 10) % 10) as u8;
        let ones = (f % 10) as u8;
        if ones > 0 {
            buf.push(b'0' + tens);
            buf.push(b'0' + ones);
        } else if tens > 0 {
            buf.push(b'0' + tens);
        }
    }
}

#[inline]
fn write_f32_3(buf: &mut Vec<u8>, a: f32, b: f32, c_val: f32) {
    write_f32_fast(buf, a);
    buf.push(b' ');
    write_f32_fast(buf, b);
    buf.push(b' ');
    write_f32_fast(buf, c_val);
}

/// Write an image XObject to the PDF stream
fn write_image_xobject<W: Write>(w: &mut CountingWriter<W>, img: &crate::layout::EmbeddedImage, _index: usize) -> Result<()> {
    use crate::layout::ImageFormat;
    match img.format {
        ImageFormat::Jpeg => {
            // JPEG: embed directly with DCTDecode
            w.write_all(b"<< /Type /XObject /Subtype /Image")?;
            w.write_all(b" /Width ")?;
            w.write_all(itoa::Buffer::new().format(img.width_px).as_bytes())?;
            w.write_all(b" /Height ")?;
            w.write_all(itoa::Buffer::new().format(img.height_px).as_bytes())?;
            w.write_all(b" /ColorSpace /DeviceRGB /BitsPerComponent 8")?;
            w.write_all(b" /Filter /DCTDecode")?;
            w.write_all(b" /Length ")?;
            w.write_all(itoa::Buffer::new().format(img.data.len()).as_bytes())?;
            w.write_all(b" >>\nstream\n")?;
            w.write_all(&img.data)?;
            w.write_all(b"\nendstream\nendobj\n")?;
        }
        ImageFormat::Png => {
            // PNG: decode to raw pixels and embed with FlateDecode
            // Simple approach: extract IDAT chunks and re-deflate raw RGB data
            if let Some((rgb_data, has_alpha)) = decode_png_to_rgb(&img.data) {
                let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&rgb_data, 6);
                let cs = "/DeviceRGB";
                w.write_all(b"<< /Type /XObject /Subtype /Image")?;
                w.write_all(b" /Width ")?;
                w.write_all(itoa::Buffer::new().format(img.width_px).as_bytes())?;
                w.write_all(b" /Height ")?;
                w.write_all(itoa::Buffer::new().format(img.height_px).as_bytes())?;
                w.write_all(b" /ColorSpace ")?;
                w.write_all(cs.as_bytes())?;
                w.write_all(b" /BitsPerComponent 8")?;
                w.write_all(b" /Filter /FlateDecode")?;
                w.write_all(b" /Length ")?;
                w.write_all(itoa::Buffer::new().format(compressed.len()).as_bytes())?;
                w.write_all(b" >>\nstream\n")?;
                w.write_all(&compressed)?;
                w.write_all(b"\nendstream\nendobj\n")?;
            } else {
                // Fallback: empty image
                w.write_all(b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n\xff\xff\xff\nendstream\nendobj\n")?;
            }
        }
    }
    Ok(())
}

/// Decode PNG raw image data to RGB bytes
fn decode_png_to_rgb(png_data: &[u8]) -> Option<(Vec<u8>, bool)> {
    // Parse PNG chunks to extract IHDR and IDAT data
    if png_data.len() < 33 { return None; }
    // IHDR is at offset 8 (after signature)
    let width = u32::from_be_bytes([png_data[16], png_data[17], png_data[18], png_data[19]]);
    let height = u32::from_be_bytes([png_data[20], png_data[21], png_data[22], png_data[23]]);
    let bit_depth = png_data[24];
    let color_type = png_data[25];

    if bit_depth != 8 { return None; } // Only support 8-bit

    let channels: usize = match color_type {
        0 => 1, // Grayscale
        2 => 3, // RGB
        4 => 2, // Grayscale + Alpha
        6 => 4, // RGBA
        _ => return None,
    };

    // Collect all IDAT chunks
    let mut idat_data = Vec::new();
    let mut pos = 8; // after PNG signature
    while pos + 12 <= png_data.len() {
        let chunk_len = u32::from_be_bytes([png_data[pos], png_data[pos+1], png_data[pos+2], png_data[pos+3]]) as usize;
        let chunk_type = &png_data[pos+4..pos+8];
        if chunk_type == b"IDAT" {
            if pos + 8 + chunk_len <= png_data.len() {
                idat_data.extend_from_slice(&png_data[pos+8..pos+8+chunk_len]);
            }
        }
        pos += 12 + chunk_len; // 4 len + 4 type + data + 4 CRC
    }

    // Decompress IDAT data
    let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&idat_data).ok()?;

    // Reconstruct raw pixels (undo PNG filtering)
    let stride = 1 + width as usize * channels; // 1 filter byte + pixel data per row
    if decompressed.len() < stride * height as usize { return None; }

    let mut pixels = vec![0u8; width as usize * height as usize * channels];
    let mut prev_row = vec![0u8; width as usize * channels];

    for y in 0..height as usize {
        let row_start = y * stride;
        let filter = decompressed[row_start];
        let row_data = &decompressed[row_start + 1..row_start + stride];
        let px_start = y * width as usize * channels;

        for x in 0..width as usize * channels {
            let raw = row_data[x] as i32;
            let a = if x >= channels { pixels[px_start + x - channels] as i32 } else { 0 };
            let b = prev_row[x] as i32;
            let c = if x >= channels { prev_row[x - channels] as i32 } else { 0 };

            let val = match filter {
                0 => raw,               // None
                1 => raw + a,            // Sub
                2 => raw + b,            // Up
                3 => raw + (a + b) / 2,  // Average
                4 => raw + paeth(a, b, c), // Paeth
                _ => raw,
            };
            pixels[px_start + x] = (val & 0xFF) as u8;
        }
        prev_row.copy_from_slice(&pixels[px_start..px_start + width as usize * channels]);
    }

    // Convert to RGB
    let has_alpha = channels == 4 || channels == 2;
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let idx = (y * width as usize + x) * channels;
            match channels {
                1 => { let g = pixels[idx]; rgb.extend_from_slice(&[g, g, g]); }
                2 => { let g = pixels[idx]; rgb.extend_from_slice(&[g, g, g]); } // ignore alpha
                3 => { rgb.extend_from_slice(&pixels[idx..idx+3]); }
                4 => { rgb.extend_from_slice(&pixels[idx..idx+3]); } // ignore alpha
                _ => { rgb.extend_from_slice(&[0, 0, 0]); }
            }
        }
    }

    Some((rgb, has_alpha))
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a }
    else if pb <= pc { b }
    else { c }
}

fn escape_pdf_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '(' => result.push_str("\\("),
            ')' => result.push_str("\\)"),
            '\\' => result.push_str("\\\\"),
            _ if ch.is_ascii() => result.push(ch),
            _ => result.push('?'),
        }
    }
    result
}
