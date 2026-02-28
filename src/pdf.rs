/// Direct PDF generation - bypasses intermediate formats for maximum speed
/// Uses parallel page content generation via rayon

use anyhow::Result;
use std::io::Write;
use std::path::Path;
use rayon::prelude::*;
use crate::document::Document;
use crate::layout::{LayoutResult, PageElement};
use crate::typeset::FontStyle;

/// Generate a PDF file from laid-out pages
/// Uses batched generation+write to keep peak memory low (~3MB vs 150MB)
pub fn generate_pdf(layout: &LayoutResult, doc: &Document, output: &Path, source: &str) -> Result<usize> {
    use std::time::Instant;
    use std::io::BufWriter;

    let write_start = Instant::now();
    let file = std::fs::File::create(output)?;
    let mut writer = BufWriter::with_capacity(16 * 1024 * 1024, file);
    let bytes_written = write_pdf_streaming(&mut writer, layout, doc, source)?;
    writer.flush()?;
    let write_time = write_start.elapsed();

    eprintln!("  [PDF-GEN+WRITE] {:.3}ms",
        write_time.as_secs_f64() * 1000.0);

    Ok(bytes_written)
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
    let resource_id = alloc_obj();

    let mut page_ids = Vec::with_capacity(num_pages);
    let mut content_ids = Vec::with_capacity(num_pages);
    for _ in 0..num_pages {
        page_ids.push(alloc_obj());
        content_ids.push(alloc_obj());
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
    w.write_all(b" 0 R >>\nendobj\n")?;

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

    // Fonts
    for (id, name) in [
        (font_regular_id, "Helvetica"),
        (font_bold_id, "Helvetica-Bold"),
        (font_italic_id, "Helvetica-Oblique"),
        (font_bolditalic_id, "Helvetica-BoldOblique"),
        (font_mono_id, "Courier"),
        (font_symbol_id, "Symbol"),
    ] {
        begin_obj!(id);
        w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /")?;
        w.write_all(name.as_bytes())?;
        w.write_all(b" /Encoding /WinAnsiEncoding >>\nendobj\n")?;
    }

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
    w.write_all(b" 0 R >> >>\nendobj\n")?;

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

    let mut page_suffix = Vec::with_capacity(64);
    page_suffix.extend_from_slice(b" 0 R /Resources ");
    page_suffix.extend_from_slice(itoa_buf.format(resource_id).as_bytes());
    page_suffix.extend_from_slice(b" 0 R >>\nendobj\n");

    // Generate and write content streams in batches to reduce peak memory
    // Each batch generates ~1000 page contents in parallel, then writes them
    const BATCH_SIZE: usize = 5000;
    for batch_start in (0..num_pages).step_by(BATCH_SIZE) {
        let batch_end = (batch_start + BATCH_SIZE).min(num_pages);

        // Generate batch in parallel
        let batch_contents: Vec<Vec<u8>> = (batch_start..batch_end).into_par_iter()
            .map(|i| generate_page_content(
                layout.page_elements(i),
                layout.page_text(i),
                &layout.rect_data,
                layout.height,
                source,
            ))
            .collect();

        // Write batch sequentially
        for (j, content) in batch_contents.iter().enumerate() {
            let i = batch_start + j;

            // Content stream object
            begin_obj!(content_ids[i]);
            w.write_all(b"<< /Length ")?;
            w.write_all(itoa_buf.format(content.len()).as_bytes())?;
            w.write_all(b" >>\nstream\n")?;
            w.write_all(content)?;
            w.write_all(b"\nendstream\nendobj\n")?;

            // Page object
            begin_obj!(page_ids[i]);
            w.write_all(&page_prefix)?;
            w.write_all(itoa_buf.format(content_ids[i]).as_bytes())?;
            w.write_all(&page_suffix)?;
        }
        // batch_contents dropped here, freeing ~6MB
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
    w.write_all(b" /Root 1 0 R >>\nstartxref\n")?;
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
    let mut cr: u8 = 0;
    let mut cg: u8 = 0;
    let mut cb: u8 = 0;
    let mut in_bt = false; // track whether we're inside a BT block
    let mut cur_tx: f32 = 0.0; // current text position (Td is relative!)
    let mut cur_ty: f32 = 0.0;

    for elem in elements {
        match elem {
            PageElement::Text { x, y, text_offset, text_len, font_size_100, font_style, color } => {
                if *text_len == 0 {
                    continue;
                }
                let tlen = *text_len as usize;
                // Decode text source: high bit = source reference, else page text_buffer
                let text = if *text_offset & SOURCE_REF_FLAG != 0 {
                    let off = (*text_offset & !SOURCE_REF_FLAG) as usize;
                    &source[off..off + tlen]
                } else {
                    &text_buffer[*text_offset as usize..*text_offset as usize + tlen]
                };

                let font_id = match font_style {
                    FontStyle::Regular | FontStyle::SmallCaps => 1u8,
                    FontStyle::Bold => 2,
                    FontStyle::Italic => 3,
                    FontStyle::BoldItalic => 4,
                    FontStyle::Monospace => 5,
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
                    c.push(b'0' + font_id);
                    c.push(b' ');
                    write_f32_fast(&mut c, font_size);
                    c.extend_from_slice(b" Tf\n");
                    current_font = font_id;
                    current_size = *font_size_100;
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

                // Text escaping + UTF-8 to WinAnsiEncoding conversion
                // Fast path: scan for runs of safe ASCII (no escaping or encoding needed)
                let text_bytes = text.as_bytes();
                let tlen = text_bytes.len();
                let mut tpos = 0;
                while tpos < tlen {
                    // Fast ASCII scan: find next byte needing special handling
                    // Special bytes: ( ) \ (need escaping) and >= 0x80 (non-ASCII)
                    let scan_start = tpos;
                    while tpos < tlen {
                        let b = text_bytes[tpos];
                        if b == b'(' || b == b')' || b == b'\\' || b >= 0x80 {
                            break;
                        }
                        tpos += 1;
                    }
                    // Emit safe ASCII run
                    if tpos > scan_start {
                        c.extend_from_slice(&text_bytes[scan_start..tpos]);
                    }
                    if tpos >= tlen { break; }

                    let b = text_bytes[tpos];
                    if b < 0x80 {
                        // ASCII special char
                        match b {
                            b'(' => c.extend_from_slice(b"\\("),
                            b')' => c.extend_from_slice(b"\\)"),
                            b'\\' => c.extend_from_slice(b"\\\\"),
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

                write_f32_fast(&mut c, rect.x);
                c.push(b' ');
                write_f32_fast(&mut c, pdf_y);
                c.push(b' ');
                write_f32_fast(&mut c, rect.width);
                c.push(b' ');
                write_f32_fast(&mut c, rect.height);
                c.extend_from_slice(b" re\n");

                match (rect.fill.is_some(), rect.stroke.is_some()) {
                    (true, true) => c.extend_from_slice(b"B\n"),
                    (true, false) => c.extend_from_slice(b"f\n"),
                    (false, true) => c.extend_from_slice(b"S\n"),
                    (false, false) => {}
                }

                c.extend_from_slice(b"Q\n");
            }
        }
    }

    if in_bt {
        c.extend_from_slice(b"ET\n");
    }

    c
}

/// Fast f32 formatting - avoids the overhead of write!() formatting
#[inline]
fn write_f32_fast(buf: &mut Vec<u8>, val: f32) {
    let negative = val < 0.0;
    let val = if negative { -val } else { val };
    let int_part = val as u32;
    let frac_100 = ((val - int_part as f32) * 100.0 + 0.5) as u32;

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

    // Write fractional part (2 decimal places)
    if frac_100 > 0 {
        buf.push(b'.');
        let f = frac_100.min(99);
        buf.push(b'0' + (f / 10) as u8);
        let last = (f % 10) as u8;
        if last > 0 {
            buf.push(b'0' + last);
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
