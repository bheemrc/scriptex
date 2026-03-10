/// Direct PDF generation - bypasses intermediate formats for maximum speed
/// Uses parallel page content generation via rayon (when available)

use anyhow::Result;
use std::io::Write;
#[cfg(feature = "rayon")]
use rayon::prelude::*;
use crate::document::Document;
use crate::layout::{LayoutResult, PageElement, ImageFormat};
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
    let encoding_id = alloc_obj(); // Custom encoding with ligatures
    let resource_id = alloc_obj();

    let mut page_ids = Vec::with_capacity(num_pages);
    let mut content_ids = Vec::with_capacity(num_pages);
    for _ in 0..num_pages {
        page_ids.push(alloc_obj());
        content_ids.push(alloc_obj());
    }

    // Allocate IDs for embedded images (and SMask objects for PNGs with alpha)
    let num_images = layout.images.len();
    let mut image_ids = Vec::with_capacity(num_images);
    let mut smask_ids: Vec<Option<u32>> = Vec::with_capacity(num_images);
    for i in 0..num_images {
        image_ids.push(alloc_obj());
        if layout.images[i].has_alpha && layout.images[i].format == ImageFormat::Png {
            smask_ids.push(Some(alloc_obj()));
        } else {
            smask_ids.push(None);
        }
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

    // Custom encoding: WinAnsiEncoding + ligature glyphs at 0x01-0x05
    begin_obj!(encoding_id);
    w.write_all(b"<< /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [1 /fi /fl] >>\nendobj\n")?;

    // Fonts with custom encoding (includes ligatures)
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
        w.write_all(b" /Encoding ")?;
        w.write_all(itoa_obj.format(encoding_id).as_bytes())?;
        w.write_all(b" 0 R >>\nendobj\n")?;
    }
    // Symbol font uses its own encoding (not WinAnsi)
    begin_obj!(font_symbol_id);
    w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>\nendobj\n")?;

    // Times fonts with custom encoding (includes ligatures)
    for (id, name) in [
        (font_times_roman_id, "Times-Roman"),
        (font_times_italic_id, "Times-Italic"),
        (font_times_bold_id, "Times-Bold"),
        (font_times_bolditalic_id, "Times-BoldItalic"),
    ] {
        begin_obj!(id);
        w.write_all(b"<< /Type /Font /Subtype /Type1 /BaseFont /")?;
        w.write_all(name.as_bytes())?;
        w.write_all(b" /Encoding ")?;
        w.write_all(itoa_obj.format(encoding_id).as_bytes())?;
        w.write_all(b" 0 R >>\nendobj\n")?;
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
        write_image_xobject(&mut w, img, i + 1, smask_ids[i])?;

        // Write SMask object for PNG images with alpha
        if let Some(smask_id) = smask_ids[i] {
            if smask_id as usize > offsets.len() { offsets.resize(smask_id as usize, 0); }
            begin_obj!(smask_id);
            write_smask_xobject(&mut w, img)?;
        }
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
        if link.url.is_empty() {
            // Internal cross-reference link
            if let Some(dest_page) = link.dest_page {
                let dest_page_idx = (dest_page as usize).min(page_ids.len().saturating_sub(1));
                let dest_y_pdf = layout.height - link.dest_y;
                write!(w, "<< /Type /Annot /Subtype /Link /Rect [{:.1} {:.1} {:.1} {:.1}] /Border [0 0 0] /Dest [{} 0 R /XYZ 0 {:.0} 0] >>\nendobj\n",
                    x1, y1, x2, y2, page_ids[dest_page_idx], dest_y_pdf)?;
            } else {
                write!(w, "<< /Type /Annot /Subtype /Link /Rect [{:.1} {:.1} {:.1} {:.1}] /Border [0 0 0] >>\nendobj\n",
                    x1, y1, x2, y2)?;
            }
        } else {
            // External URL link
            write!(w, "<< /Type /Annot /Subtype /Link /Rect [{:.1} {:.1} {:.1} {:.1}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({}) >> >>\nendobj\n",
                x1, y1, x2, y2, escape_pdf_string(&link.url))?;
        }
    }

    // Write outline objects (PDF bookmarks) with hierarchical structure
    if let Some(outline_root) = outline_root_id {
        if outline_root as usize > offsets.len() { offsets.resize(outline_root as usize, 0); }

        // Include all entries up to level 3 (section, subsection, subsubsection)
        let visible: Vec<usize> = (0..num_outlines)
            .filter(|&i| layout.outlines[i].level <= 3)
            .collect();

        // Find children of a given parent level at position range
        // For root: children are entries at the shallowest level
        let min_level = visible.iter().map(|&i| layout.outlines[i].level).min().unwrap_or(0);

        // Find top-level entries (those at min_level)
        let top_entries: Vec<usize> = visible.iter().copied()
            .filter(|&i| layout.outlines[i].level == min_level)
            .collect();

        begin_obj!(outline_root);
        if let Some(&first_idx) = top_entries.first() {
            let total_count: usize = visible.len();
            w.write_all(b"<< /Type /Outlines /First ")?;
            w.write_all(itoa_obj.format(outline_item_ids[first_idx]).as_bytes())?;
            w.write_all(b" 0 R /Last ")?;
            w.write_all(itoa_obj.format(outline_item_ids[*top_entries.last().unwrap()]).as_bytes())?;
            w.write_all(b" 0 R /Count ")?;
            w.write_all(itoa_obj.format(total_count).as_bytes())?;
            w.write_all(b" >>\nendobj\n")?;
        } else {
            w.write_all(b"<< /Type /Outlines /Count 0 >>\nendobj\n")?;
        }

        // For each visible entry, find its children (entries at next level between this and next same-level sibling)
        for &idx in &visible {
            let entry = &layout.outlines[idx];
            let item_id = outline_item_ids[idx];
            let my_level = entry.level;
            if item_id as usize > offsets.len() { offsets.resize(item_id as usize, 0); }
            begin_obj!(item_id);
            w.write_all(b"<< /Title (")?;
            w.write_all(escape_pdf_string(&entry.title).as_bytes())?;
            w.write_all(b")")?;

            // Parent: root if top-level, otherwise find parent entry
            let parent_id = if my_level == min_level {
                outline_root
            } else {
                // Find the nearest preceding entry at level-1
                let mut pid = outline_root;
                for &vi in &visible {
                    if vi >= idx { break; }
                    if layout.outlines[vi].level == my_level - 1 { pid = outline_item_ids[vi]; }
                }
                pid
            };
            w.write_all(b" /Parent ")?;
            w.write_all(itoa_obj.format(parent_id).as_bytes())?;
            w.write_all(b" 0 R")?;

            // Destination
            let page_idx = (entry.page as usize).min(page_ids.len().saturating_sub(1));
            let dest_y = layout.height - entry.y;
            w.write_all(b" /Dest [")?;
            w.write_all(itoa_obj.format(page_ids[page_idx]).as_bytes())?;
            write!(w, " 0 R /XYZ 0 {:.0} 0]", dest_y)?;

            // Find children of this entry (entries at my_level+1 before next same-level entry)
            let child_level = my_level + 1;
            let vis_pos = visible.iter().position(|&v| v == idx).unwrap_or(0);
            let mut children: Vec<usize> = Vec::new();
            for &vi in &visible[vis_pos + 1..] {
                let vl = layout.outlines[vi].level;
                if vl <= my_level { break; } // hit sibling or ancestor
                if vl == child_level { children.push(vi); }
            }
            if !children.is_empty() {
                w.write_all(b" /First ")?;
                w.write_all(itoa_obj.format(outline_item_ids[children[0]]).as_bytes())?;
                w.write_all(b" 0 R /Last ")?;
                w.write_all(itoa_obj.format(outline_item_ids[*children.last().unwrap()]).as_bytes())?;
                w.write_all(b" 0 R /Count ")?;
                // Negative count = collapsed by default
                let child_count = -(children.len() as i32);
                write!(w, "{}", child_count)?;
            }

            // Sibling links: find prev/next at same level under same parent
            let siblings: Vec<usize> = if my_level == min_level {
                top_entries.clone()
            } else {
                // Find siblings: entries at same level between parent bounds
                let mut sibs = Vec::new();
                // Walk backward to find parent position
                let parent_vis_pos = (0..vis_pos).rev().find(|&p| layout.outlines[visible[p]].level == my_level - 1);
                let start = parent_vis_pos.map_or(0, |p| p + 1);
                for &vi in &visible[start..] {
                    let vl = layout.outlines[vi].level;
                    if vl < my_level { break; }
                    if vl == my_level { sibs.push(vi); }
                }
                sibs
            };
            let sib_pos = siblings.iter().position(|&s| s == idx);
            if let Some(sp) = sib_pos {
                if sp > 0 {
                    w.write_all(b" /Prev ")?;
                    w.write_all(itoa_obj.format(outline_item_ids[siblings[sp - 1]]).as_bytes())?;
                    w.write_all(b" 0 R")?;
                }
                if sp + 1 < siblings.len() {
                    w.write_all(b" /Next ")?;
                    w.write_all(itoa_obj.format(outline_item_ids[siblings[sp + 1]]).as_bytes())?;
                    w.write_all(b" 0 R")?;
                }
            }

            w.write_all(b" >>\nendobj\n")?;
        }
    }

    // Metadata
    let info_id = next_obj;
    next_obj += 1;
    if info_id as usize > offsets.len() { offsets.resize(info_id as usize, 0); }
    begin_obj!(info_id);
    w.write_all(b"<< /Producer (ScripTeX 0.1)")?;
    if let Some(title) = &doc.preamble.title {
        let clean = clean_metadata_text(title);
        w.write_all(b" /Title (")?;
        w.write_all(escape_pdf_string(&clean).as_bytes())?;
        w.write_all(b")")?;
    }
    if let Some(author) = &doc.preamble.author {
        let clean = clean_metadata_text(author);
        w.write_all(b" /Author (")?;
        w.write_all(escape_pdf_string(&clean).as_bytes())?;
        w.write_all(b")")?;
    }
    if let Some(subject) = &doc.preamble.subject {
        let clean = clean_metadata_text(subject);
        w.write_all(b" /Subject (")?;
        w.write_all(escape_pdf_string(&clean).as_bytes())?;
        w.write_all(b")")?;
    }
    if let Some(keywords) = &doc.preamble.keywords {
        let clean = clean_metadata_text(keywords);
        w.write_all(b" /Keywords (")?;
        w.write_all(escape_pdf_string(&clean).as_bytes())?;
        w.write_all(b")")?;
    }
    w.write_all(b" /Creator (ScripTeX)")?;
    // Add creation date in PDF date format: D:YYYYMMDDHHmmSS
    {
        use std::time::SystemTime;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let secs = now.as_secs();
        // Simple UTC date formatting without chrono dependency
        let days = secs / 86400;
        let time_of_day = secs % 86400;
        let hours = time_of_day / 3600;
        let mins = (time_of_day % 3600) / 60;
        let seconds = time_of_day % 60;
        // Approximate date from epoch days (good enough for metadata)
        let (year, month, day) = epoch_days_to_date(days);
        write!(w, " /CreationDate (D:{:04}{:02}{:02}{:02}{:02}{:02}Z)", year, month, day, hours, mins, seconds)?;
    }
    w.write_all(b" >>\nendobj\n")?;

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
                    FontStyle::SansSerif => 1,          // Helvetica
                    FontStyle::SansSerifBold => 2,      // Helvetica-Bold
                    FontStyle::SansSerifItalic => 3,    // Helvetica-Oblique
                    FontStyle::SansSerifBoldItalic => 4, // Helvetica-BoldOblique
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
                c.extend_from_slice(b" Td\n");
                cur_tx = *x;
                cur_ty = pdf_y;

                // Text escaping + encoding conversion
                let text_bytes = text.as_bytes();
                let tlen = text_bytes.len();
                if font_id == 6 || font_id == 10 {
                    // Symbol/ZapfDingbats font: no kerning
                    c.push(b'(');
                    for ch in text.chars() {
                        let b = ch as u8;
                        match b {
                            b'(' => c.extend_from_slice(b"\\("),
                            b')' => c.extend_from_slice(b"\\)"),
                            b'\\' => c.extend_from_slice(b"\\\\"),
                            _ => c.push(b),
                        }
                    }
                    c.extend_from_slice(b") Tj\n");
                } else {
                // Single-pass encoding with inline kerning via TJ operator
                // Writes directly to output buffer, no temp buffers needed
                let is_mono = font_id == 5;
                let kern_font: Option<crate::font::FontId> = match font_id {
                    1 => Some(crate::font::FontId::Helvetica),
                    2 => Some(crate::font::FontId::HelveticaBold),
                    3 => Some(crate::font::FontId::HelveticaOblique),
                    4 => Some(crate::font::FontId::HelveticaBoldOblique),
                    7 => Some(crate::font::FontId::TimesRoman),
                    8 => Some(crate::font::FontId::TimesItalic),
                    9 => Some(crate::font::FontId::TimesBold),
                    11 => Some(crate::font::FontId::TimesBoldItalic),
                    _ => None,
                };

                // We start with [( and always use TJ format for kernable fonts,
                // or ( and Tj for non-kernable fonts (Courier)
                let start_pos = c.len();
                if kern_font.is_some() {
                    c.extend_from_slice(b"[(");
                } else {
                    c.push(b'(');
                }

                let mut prev_glyph: u8 = 0;
                let mut has_kerns = false;
                let mut tpos = 0;
                while tpos < tlen {
                    let b = text_bytes[tpos];
                    let glyph: u8; // logical glyph byte for kern lookup
                    if b < 0x80 {
                        match b {
                            b'\n' | b'\r' => { glyph = b' '; }
                            b'(' => { glyph = b'('; }
                            b')' => { glyph = b')'; }
                            b'\\' => { glyph = b'\\'; }
                            b'`' => {
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'`' {
                                    glyph = 0x93; tpos += 1;
                                } else {
                                    glyph = 0x91;
                                }
                            }
                            b'\'' => {
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'\'' {
                                    glyph = 0x94; tpos += 1;
                                } else {
                                    glyph = b;
                                }
                            }
                            b'-' => {
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'-' {
                                    if tpos + 2 < tlen && text_bytes[tpos + 2] == b'-' {
                                        glyph = 0x97; tpos += 2;
                                    } else {
                                        glyph = 0x96; tpos += 1;
                                    }
                                } else {
                                    glyph = b'-';
                                }
                            }
                            b'f' if !is_mono => {
                                // Only fi and fl ligatures are in Standard 14 fonts;
                                // ff/ffi/ffl are NOT standard and render as boxes in many viewers.
                                // For ffi/ffl: emit 'f' now, next iteration picks up fi/fl.
                                if tpos + 1 < tlen && text_bytes[tpos + 1] == b'i' {
                                    // Check it's not ffi (just fi)
                                    glyph = crate::font::LIG_FI; tpos += 1;
                                } else if tpos + 1 < tlen && text_bytes[tpos + 1] == b'l' {
                                    // Check it's not ffl (just fl)
                                    glyph = crate::font::LIG_FL; tpos += 1;
                                } else {
                                    glyph = b'f';
                                }
                            }
                            _ => { glyph = b; }
                        }
                        tpos += 1;
                    } else if b < 0xC0 {
                        tpos += 1;
                        continue; // stray continuation byte
                    } else {
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
                        glyph = match codepoint {
                            0x00A0..=0x00FF => codepoint as u8,
                            0x2022 => 0x95, 0x2013 => 0x96, 0x2014 => 0x97,
                            0x2018 => 0x91, 0x2019 => 0x92, 0x201C => 0x93, 0x201D => 0x94,
                            0x2026 => 0x85, 0x2020 => 0x86, 0x2021 => 0x87, 0x2030 => 0x89,
                            0x0152 => 0x8C, 0x0153 => 0x9C, 0x0160 => 0x8A, 0x0161 => 0x9A,
                            0x0178 => 0x9F, 0x017D => 0x8E, 0x017E => 0x9E, 0x0192 => 0x83,
                            0x02C6 => 0x88, 0x02DC => 0x98, 0x20AC => 0x80, 0x2122 => 0x99,
                            _ => b'?',
                        };
                        tpos += advance;
                    }

                    // Check kern pair before emitting glyph
                    if let Some(kf) = kern_font {
                        if prev_glyph != 0 {
                            let k = crate::font::kern_pair(kf, prev_glyph, glyph);
                            if k != 0 {
                                // Close current string, insert kern adjustment, open new string
                                c.push(b')');
                                let mut kbuf = itoa::Buffer::new();
                                c.extend_from_slice(kbuf.format(-k as i32).as_bytes());
                                c.push(b'(');
                                has_kerns = true;
                            }
                        }
                    }
                    prev_glyph = glyph;

                    // Encode glyph to PDF string
                    match glyph {
                        b'(' => c.extend_from_slice(b"\\("),
                        b')' => c.extend_from_slice(b"\\)"),
                        b'\\' => c.extend_from_slice(b"\\\\"),
                        _ => c.push(glyph),
                    }
                }

                // Close text operator
                if kern_font.is_some() {
                    if has_kerns {
                        c.extend_from_slice(b")] TJ\n");
                    } else {
                        // No kerns found — rewrite prefix from [( to just (
                        // and use Tj instead of ] TJ
                        c[start_pos] = b'(';
                        // Shift content left by 1 to remove the [
                        let content_start = start_pos + 2;
                        let content_end = c.len();
                        c.copy_within(content_start..content_end, content_start - 1);
                        c.truncate(content_end - 1);
                        c.extend_from_slice(b") Tj\n");
                    }
                } else {
                    c.extend_from_slice(b") Tj\n");
                }
                } // end WinAnsi branch
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

            PageElement::Image { x, y, width: img_w, height: img_h, image_idx, angle } => {
                if in_bt {
                    c.extend_from_slice(b"ET\n");
                    in_bt = false;
                }
                // PDF image drawing: save state, apply CTM, draw, restore
                let pdf_y = height - y - img_h;
                c.extend_from_slice(b"q\n");

                if *angle != 0.0 {
                    // Rotation: translate to center of bounding box, rotate, then scale image
                    let cx = *x + img_w / 2.0;
                    let cy = pdf_y + img_h / 2.0;
                    let rad = angle.to_radians();
                    let cos_a = rad.cos();
                    let sin_a = rad.sin();
                    // Translate to center
                    write_f32_fast(&mut c, 1.0); c.extend_from_slice(b" 0 0 ");
                    write_f32_fast(&mut c, 1.0); c.push(b' ');
                    write_f32_fast(&mut c, cx); c.push(b' ');
                    write_f32_fast(&mut c, cy); c.extend_from_slice(b" cm\n");
                    // Rotate
                    write_f32_fast(&mut c, cos_a); c.push(b' ');
                    write_f32_fast(&mut c, sin_a); c.push(b' ');
                    write_f32_fast(&mut c, -sin_a); c.push(b' ');
                    write_f32_fast(&mut c, cos_a);
                    c.extend_from_slice(b" 0 0 cm\n");
                    // Scale and offset from center
                    write_f32_fast(&mut c, *img_w); c.extend_from_slice(b" 0 0 ");
                    write_f32_fast(&mut c, *img_h); c.push(b' ');
                    write_f32_fast(&mut c, -img_w / 2.0); c.push(b' ');
                    write_f32_fast(&mut c, -img_h / 2.0);
                    c.extend_from_slice(b" cm\n");
                } else {
                    // CTM maps unit square [0,0]-[1,1] to [x,y]-[x+w,y+h] in PDF coords
                    write_f32_fast(&mut c, *img_w);
                    c.extend_from_slice(b" 0 0 ");
                    write_f32_fast(&mut c, *img_h);
                    c.push(b' ');
                    write_f32_fast(&mut c, *x);
                    c.push(b' ');
                    write_f32_fast(&mut c, pdf_y);
                    c.extend_from_slice(b" cm\n");
                }
                c.extend_from_slice(b"/Im");
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
fn write_image_xobject<W: Write>(w: &mut CountingWriter<W>, img: &crate::layout::EmbeddedImage, _index: usize, smask_id: Option<u32>) -> Result<()> {
    match &img.format {
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
            if let Some((rgb_data, _has_alpha, _alpha_data)) = decode_png_to_rgb(&img.data) {
                let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&rgb_data, 6);
                w.write_all(b"<< /Type /XObject /Subtype /Image")?;
                w.write_all(b" /Width ")?;
                w.write_all(itoa::Buffer::new().format(img.width_px).as_bytes())?;
                w.write_all(b" /Height ")?;
                w.write_all(itoa::Buffer::new().format(img.height_px).as_bytes())?;
                w.write_all(b" /ColorSpace /DeviceRGB /BitsPerComponent 8")?;
                w.write_all(b" /Filter /FlateDecode")?;
                // Reference SMask for transparency
                if let Some(sid) = smask_id {
                    w.write_all(b" /SMask ")?;
                    w.write_all(itoa::Buffer::new().format(sid).as_bytes())?;
                    w.write_all(b" 0 R")?;
                }
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
        ImageFormat::Pdf { bbox, resources } => {
            // PDF page embedded as Form XObject — preserves vector quality
            w.write_all(b"<< /Type /XObject /Subtype /Form")?;
            write!(w, " /BBox [{:.2} {:.2} {:.2} {:.2}]", bbox[0], bbox[1], bbox[2], bbox[3])?;
            w.write_all(b" /Resources ")?;
            w.write_all(resources)?;
            w.write_all(b" /Length ")?;
            w.write_all(itoa::Buffer::new().format(img.data.len()).as_bytes())?;
            w.write_all(b" >>\nstream\n")?;
            w.write_all(&img.data)?;
            w.write_all(b"\nendstream\nendobj\n")?;
        }
        ImageFormat::Svg => {
            // SVG: convert to PDF drawing commands and emit as Form XObject
            if let Ok(svg_text) = std::str::from_utf8(&img.data) {
                if let Some(doc) = crate::svg_render::parse_svg(svg_text) {
                    let svg_content = crate::svg_render::svg_to_pdf_content(&doc);
                    let mut content_stream = Vec::with_capacity(svg_content.len() + 64);
                    let sx = if doc.width > 0.0 { 1.0 / doc.width } else { 1.0 };
                    let sy = if doc.height > 0.0 { 1.0 / doc.height } else { 1.0 };
                    use std::io::Write as _;
                    write!(content_stream, "{:.6} 0 0 {:.6} 0 0 cm\n", sx, sy).unwrap();
                    content_stream.extend_from_slice(&svg_content);
                    w.write_all(b"<< /Type /XObject /Subtype /Form")?;
                    w.write_all(b" /BBox [0 0 1 1]")?;
                    w.write_all(b" /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>")?;
                    w.write_all(b" /Length ")?;
                    w.write_all(itoa::Buffer::new().format(content_stream.len()).as_bytes())?;
                    w.write_all(b" >>\nstream\n")?;
                    w.write_all(&content_stream)?;
                    w.write_all(b"\nendstream\nendobj\n")?;
                } else {
                    w.write_all(b"<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 0 >>\nstream\n\nendstream\nendobj\n")?;
                }
            } else {
                w.write_all(b"<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 0 >>\nstream\n\nendstream\nendobj\n")?;
            }
        }
    }
    Ok(())
}

/// Write an SMask (alpha channel) XObject for a PNG image with transparency
fn write_smask_xobject<W: Write>(w: &mut CountingWriter<W>, img: &crate::layout::EmbeddedImage) -> Result<()> {
    if let Some((_rgb, _has_alpha, alpha_data)) = decode_png_to_rgb(&img.data) {
        if !alpha_data.is_empty() {
            let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&alpha_data, 6);
            w.write_all(b"<< /Type /XObject /Subtype /Image")?;
            w.write_all(b" /Width ")?;
            w.write_all(itoa::Buffer::new().format(img.width_px).as_bytes())?;
            w.write_all(b" /Height ")?;
            w.write_all(itoa::Buffer::new().format(img.height_px).as_bytes())?;
            w.write_all(b" /ColorSpace /DeviceGray /BitsPerComponent 8")?;
            w.write_all(b" /Filter /FlateDecode")?;
            w.write_all(b" /Length ")?;
            w.write_all(itoa::Buffer::new().format(compressed.len()).as_bytes())?;
            w.write_all(b" >>\nstream\n")?;
            w.write_all(&compressed)?;
            w.write_all(b"\nendstream\nendobj\n")?;
        } else {
            w.write_all(b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\xff\nendstream\nendobj\n")?;
        }
    } else {
        w.write_all(b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\xff\nendstream\nendobj\n")?;
    }
    Ok(())
}

/// Decode PNG raw image data to RGB bytes and optional alpha channel
fn decode_png_to_rgb(png_data: &[u8]) -> Option<(Vec<u8>, bool, Vec<u8>)> {
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

    // Convert to RGB and extract alpha
    let has_alpha = channels == 4 || channels == 2;
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    let mut alpha = if has_alpha { Vec::with_capacity(width as usize * height as usize) } else { Vec::new() };
    for y in 0..height as usize {
        for x in 0..width as usize {
            let idx = (y * width as usize + x) * channels;
            match channels {
                1 => { let g = pixels[idx]; rgb.extend_from_slice(&[g, g, g]); }
                2 => { let g = pixels[idx]; rgb.extend_from_slice(&[g, g, g]); alpha.push(pixels[idx + 1]); }
                3 => { rgb.extend_from_slice(&pixels[idx..idx+3]); }
                4 => { rgb.extend_from_slice(&pixels[idx..idx+3]); alpha.push(pixels[idx + 3]); }
                _ => { rgb.extend_from_slice(&[0, 0, 0]); }
            }
        }
    }

    Some((rgb, has_alpha, alpha))
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

/// Convert epoch days to (year, month, day)
fn epoch_days_to_date(mut days: u64) -> (u32, u32, u32) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

/// Clean LaTeX markup from metadata text (title, author)
fn clean_metadata_text(s: &str) -> String {
    let mut result = s.replace("\\\\", " ")   // line breaks → space
        .replace("\\newline", " ")
        .replace('\n', " ")              // literal newlines → space
        .replace('\r', "")
        .replace("\\,", " ")
        .replace("\\;", " ")
        .replace("\\!", "")
        .replace("\\textbf{", "").replace("\\textit{", "")
        .replace("\\emph{", "").replace("\\textrm{", "")
        .replace("\\textsc{", "").replace("\\textsf{", "")
        .replace("\\texttt{", "");
    // Remove stray closing braces from stripped commands
    result = result.replace('}', "");
    // Collapse multiple spaces
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }
    result.trim().to_string()
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
