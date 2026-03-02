/// Layout engine: converts document AST to positioned page elements
/// Direct layout without intermediate format for maximum speed

mod types;
mod state;
mod prescans;
mod text;
mod spans;
mod math;
mod images;
mod sections;
mod title;
mod lists;
mod tables;
mod environments;

pub use types::*;
use state::{LayoutState, PageStyle};
use prescans::{collect_labels, collect_toc_entries};
use text::{layout_paragraph, layout_text_content, layout_text_content_source, layout_text_line, node_to_text, resolve_citations};

use math::layout_display_math_data;
use images::{load_image_for_pdf, layout_tikz_diagram};
use sections::{layout_section, layout_table_of_contents};
use title::layout_title;
use lists::{layout_list, layout_description_list, layout_bibliography};
use tables::layout_table;
use environments::{layout_theorem, layout_proof, layout_verbatim, layout_code_block, layout_centered, layout_flush_right};

use anyhow::Result;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};

pub fn layout_document(doc: &Document, source: &str) -> Result<LayoutResult> {
    let mut state = LayoutState::new(
        doc.preamble.page_setup,
        doc.preamble.font_size,
        doc.preamble.line_spacing,
    );

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

    let is_ams = match &doc.class.class_type {
        ClassType::Custom(s) => s == "amsart" || s == "amsbook" || s == "amsproc",
        _ => false,
    };
    if is_ams {
        state.is_amsart = true;
        state.amsart_pre_title = true;
        state.page_style = PageStyle::Headings;
        state.page_setup.header_height = 20.0;
        state.page_setup.margin_left = 72.0;
        state.page_setup.margin_right = 72.0;
        state.page_setup.margin_top = 72.0;
        state.page_setup.margin_bottom = 72.0;
        state.page_setup.footer_height = 14.0;
        state.cached_text_width = state.page_setup.text_width();
        state.cached_max_y = state.page_setup.height - state.page_setup.margin_bottom - state.page_setup.footer_height;
        state.cached_start_y = state.page_setup.margin_top + state.page_setup.header_height;
        state.current_y = state.cached_start_y;
        state.current_x = state.page_setup.margin_left;
        if let Some(ref author) = doc.preamble.author {
            state.amsart_header_author = author.to_uppercase();
        }
        if let Some(ref title) = doc.preamble.title {
            state.amsart_header_title = title.to_uppercase();
        }
    }

    match doc.preamble.page_style.as_str() {
        "headings" => {
            state.page_style = PageStyle::Headings;
            state.page_setup.header_height = 20.0;
            state.cached_start_y = state.page_setup.margin_top + state.page_setup.header_height;
            state.current_y = state.cached_start_y;
        }
        "empty" => {
            state.page_style = PageStyle::Empty;
        }
        _ => {}
    }

    state.source_ptr = source.as_ptr();
    state.source_len = source.len();

    let (labels, citations) = collect_labels(&doc.body, doc);
    state.label_map = labels;
    state.citation_map = citations;

    state.toc_entries = collect_toc_entries(&doc.body, source);

    layout_nodes(&doc.body, &mut state, doc, source)?;

    // Render author addresses at end of document (amsart style)
    if !doc.preamble.addresses.is_empty() {
        let font_size = doc.preamble.font_size;
        let step = font_size * doc.preamble.line_spacing * 1.2;
        let small_size = font_size * 0.85;
        state.current_y += step * 2.0;
        state.current_x = state.text_left();
        for addr_info in &doc.preamble.addresses {
            state.ensure_space(step * 3.0);
            state.text_buf.clear();
            state.text_buf.push_str(&addr_info.address);
            let text: &str = unsafe { &*(state.text_buf.as_str() as *const str) };
            state.emit_text(text, small_size, FontStyle::SmallCaps, Color::BLACK);
            state.current_y += step;
            state.current_x = state.text_left();
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

    if state.page_bounds.is_empty() {
        state.page_bounds.push(PageBounds {
            elem_start: 0, elem_end: 0, text_start: 0, text_end: 0,
        });
    }

    // Fixup TOC page numbers
    for fixup in &state.toc_fixups {
        let toc_idx = fixup.toc_idx as usize;
        if toc_idx < state.toc_entries.len() {
            let page = state.toc_entries[toc_idx].page;
            if page > 0 {
                let offset = fixup.text_offset as usize;
                let mut buf = [b' '; 3];
                let mut ibuf = itoa::Buffer::new();
                let s = ibuf.format(page);
                let s_bytes = s.as_bytes();
                let start = 3usize.saturating_sub(s_bytes.len());
                for (i, &b) in s_bytes.iter().enumerate() {
                    if start + i < 3 { buf[start + i] = b; }
                }
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

fn group_inline_nodes(nodes: &[Node]) -> Vec<Node> {
    let mut result = Vec::with_capacity(nodes.len());
    let mut inline_buf: Vec<Node> = Vec::new();

    for node in nodes {
        if is_inline_node(node) {
            inline_buf.push(node.clone());
        } else {
            if !inline_buf.is_empty() {
                if inline_buf.len() == 1 {
                    result.push(inline_buf.remove(0));
                } else {
                    result.push(Node::Paragraph(std::mem::take(&mut inline_buf)));
                }
                inline_buf.clear();
            }
            result.push(node.clone());
        }
    }

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
    let (_, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let mut line_height = line_height;
    let mut step = step;
    let mut font_size_100 = font_size_100;
    let mut max_chars_single = max_chars_single;
    let mut font_style = state.current_font_style;
    let mut color = state.current_color;
    let mut font_key = state.cached_font_key;

    for node in nodes {
        if state.amsart_pre_title {
            match node {
                Node::MakeTitle => {}
                Node::Abstract(_) => {}
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
            let start = *offset as usize;
            let end = start + *len as usize;
            let raw = &source[start..end];
            let bytes = raw.as_bytes();
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
            state.amsart_pre_title = false;
            layout_title(state, doc, source)?;
        }

        Node::TableOfContents => {
            layout_table_of_contents(state)?;
        }

        Node::Appendix => {
            state.appendix_mode = true;
            state.section_counters[2] = 0;
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
                state.text_buf.clear();
                state.text_buf.push_str("Figure ");
                let mut ibuf = itoa::Buffer::new();
                state.text_buf.push_str(ibuf.format(fig_num));
                state.text_buf.push_str(": ");
                let prefix_len = state.text_buf.len();

                for node in cap {
                    node_to_text(node, &mut state.text_buf, source);
                }
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
            let img_loaded = load_image_for_pdf(&img.path, state);

            if let Some((embedded, native_w, native_h)) = img_loaded {
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
                    (native_w as f32, native_h as f32)
                };

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
                state.text_left(), state.current_y,
                state.text_left() + state.text_width(), state.current_y,
                0.5, Color::BLACK,
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
                state.deferred_abstract_idx = Some(1);
            } else {
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
                if let Some(end) = text.find("%%\n") {
                    let tikz_source = &text[end + 3..];
                    layout_tikz_diagram(tikz_source, state, doc)?;
                } else {
                    layout_verbatim(text, state)?;
                }
            } else if text.starts_with("%%lang:") {
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

        Node::Minipage { width: _, content } => {
            let saved_indent = state.indent;
            layout_nodes(content, state, doc, source)?;
            state.set_indent(saved_indent);
        }

        Node::FontSize { size, content } if content.is_empty() => {
            state.current_font_size = size.to_points(doc.preamble.font_size);
        }

        Node::FontStyleDecl(decl) => {
            state.current_font_style = match decl {
                FontDeclType::Bold => FontStyle::Bold,
                FontDeclType::Italic => FontStyle::Italic,
                FontDeclType::Monospace => FontStyle::Monospace,
                FontDeclType::Regular => FontStyle::Regular,
                FontDeclType::SmallCaps => FontStyle::Regular,
            };
        }

        Node::ColorDecl(c) => {
            state.current_color = *c;
        }

        Node::Footnote(content) => {
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
            layout_paragraph(&[node.clone()], state, doc, source)?;
        }

        Node::Citation(key, opt) => {
            let cite_text = resolve_citations(key, opt.as_deref(), &state.citation_map);
            state.emit_text(&cite_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&cite_text, FontId::Helvetica, state.current_font_size);
        }

        Node::Ref(label) => {
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                resolved.clone()
            } else {
                "??".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::Helvetica, state.current_font_size);
        }

        Node::EqRef(label) => {
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                format!("({})", resolved)
            } else {
                "(??)".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::Helvetica, state.current_font_size);
        }

        Node::Href { url, content } => {
            let link_color = Color::from_rgb_u8(0, 0, 180);
            let saved_color = state.current_color;
            state.current_color = link_color;
            let start_x = state.current_x;
            let start_y = state.current_y;
            for child in content {
                layout_node(child, state, doc, source)?;
            }
            let end_x = state.current_x;
            let underline_y = start_y + state.current_font_size * 0.15;
            state.emit_line(start_x, underline_y, end_x, underline_y, 0.3, link_color);
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

        Node::Label(_) | Node::Raw(_) | Node::BibItem(_) => {}

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
