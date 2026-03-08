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
pub use prescans::{collect_labels, collect_toc_entries, TocEntry};
use state::{LayoutState, PageStyle};
use text::{layout_paragraph, layout_text_content, layout_text_content_source, layout_text_line, node_to_text, resolve_citations};

use math::layout_display_math_data;
use images::{load_image_for_pdf, layout_tikz_diagram};
use sections::{layout_section, layout_table_of_contents};
use title::layout_title;
use lists::{layout_list, layout_description_list, layout_bibliography};
use tables::layout_table;
use environments::{layout_theorem, layout_proof, layout_algorithm, layout_verbatim, layout_code_block, layout_centered, layout_flush_right};

use anyhow::Result;
use crate::color::Color;
use crate::document::*;
use crate::typeset::{FontMetrics, FontStyle};
use crate::font::{self, FontId};

pub fn layout_document(doc: &Document, source: &str) -> Result<LayoutResult> {
    layout_document_inner(doc, source, std::collections::HashMap::new(), std::collections::HashMap::new(), String::new())
}

pub fn layout_document_with_images(doc: &Document, source: &str, project_images: std::collections::HashMap<String, Vec<u8>>) -> Result<LayoutResult> {
    layout_document_inner(doc, source, project_images, std::collections::HashMap::new(), String::new())
}

pub fn layout_document_with_author_years(doc: &Document, source: &str, author_year_map: std::collections::HashMap<String, (String, String)>) -> Result<LayoutResult> {
    layout_document_inner(doc, source, std::collections::HashMap::new(), author_year_map, String::new())
}

/// Full layout entry point with all options.
pub fn layout_document_full(
    doc: &Document, source: &str,
    project_images: std::collections::HashMap<String, Vec<u8>>,
    author_year_map: std::collections::HashMap<String, (String, String)>,
    base_dir: String,
) -> Result<LayoutResult> {
    layout_document_inner(doc, source, project_images, author_year_map, base_dir)
}

pub fn layout_document_inner(
    doc: &Document,
    source: &str,
    project_images: std::collections::HashMap<String, Vec<u8>>,
    author_year_map: std::collections::HashMap<String, (String, String)>,
    base_dir: String,
) -> Result<LayoutResult> {
    let mut state = LayoutState::new(
        doc.preamble.page_setup,
        doc.preamble.font_size,
        doc.preamble.line_spacing,
    );
    state.project_images = project_images;
    state.author_year_map = author_year_map;
    state.base_dir = base_dir;

    // Apply preamble overrides
    if let Some(pi) = doc.preamble.paragraph_indent {
        state.paragraph_indent = pi;
    }
    // paragraph_skip is handled as extra vertical space between paragraphs
    // (stored in state for use in layout_text_content)

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
        "fancy" => {
            state.page_style = PageStyle::Fancy;
            let fh = &doc.preamble.fancy_header;
            state.fancy_head_left = fh.head_left.clone();
            state.fancy_head_center = fh.head_center.clone();
            state.fancy_head_right = fh.head_right.clone();
            state.fancy_foot_left = fh.foot_left.clone();
            state.fancy_foot_center = fh.foot_center.clone();
            state.fancy_foot_right = fh.foot_right.clone();
            state.fancy_head_rule = if fh.head_rule_width > 0.0 { fh.head_rule_width } else { 0.4 };
            state.fancy_foot_rule = fh.foot_rule_width;
            // Reserve space for header/footer
            let has_header = !fh.head_left.is_empty() || !fh.head_center.is_empty() || !fh.head_right.is_empty();
            let has_footer = !fh.foot_left.is_empty() || !fh.foot_center.is_empty() || !fh.foot_right.is_empty();
            if has_header {
                state.page_setup.header_height = 20.0;
                state.cached_start_y = state.page_setup.margin_top + state.page_setup.header_height;
                state.current_y = state.cached_start_y;
            }
            if has_footer {
                state.page_setup.footer_height = 20.0;
                state.cached_max_y = state.page_setup.height - state.page_setup.margin_bottom - state.page_setup.footer_height;
            }
        }
        _ => {}
    }

    state.source_ptr = source.as_ptr();
    state.source_len = source.len();

    // Activate two-column mode if requested
    let is_twocolumn = doc.class.options.iter().any(|o| o == "twocolumn") || state.page_setup.columns >= 2;
    if is_twocolumn {
        state.page_setup.columns = 2;
        // Don't enter twocolumn yet — title/abstract span both columns
        // We'll enter twocolumn after \maketitle
    }

    let (labels, citations, label_types) = collect_labels(&doc.body, doc);
    state.label_map = labels;
    state.citation_map = citations;
    state.label_types = label_types;

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
    match node {
        // Group is inline only if ALL children are inline
        Node::Group(children) => children.iter().all(is_inline_node),
        _ => matches!(node,
            Node::Text(_) | Node::TextRef(_, _) | Node::Bold(_) | Node::Italic(_)
            | Node::Monospace(_) | Node::SmallCaps(_) | Node::Underline(_) | Node::Emph(_)
            | Node::InlineMath(_) | Node::Colored { .. }
            | Node::FontSize { .. } | Node::Superscript(_) | Node::Subscript(_)
            | Node::NonBreakingSpace | Node::HSpace(_) | Node::HFill | Node::Code(_) | Node::Footnote(_)
            | Node::Strikethrough(_) | Node::Dingbat(_)
            | Node::Citation(..) | Node::Ref(_) | Node::EqRef(_) | Node::Cref(..) | Node::Href { .. }
            | Node::FontStyleDecl(_) | Node::ColorDecl(_)
            | Node::EnDash | Node::EmDash | Node::Ellipsis
            | Node::LeftQuote | Node::RightQuote | Node::LeftDoubleQuote | Node::RightDoubleQuote
            | Node::Ampersand | Node::Percent | Node::Dollar | Node::Hash | Node::Underscore
            | Node::Tilde | Node::Caret | Node::LeftBrace | Node::RightBrace
        ),
    }
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
            | Node::Citation(..) | Node::Ref(_) | Node::EqRef(_) | Node::Cref(..) | Node::Href { .. }
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

    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];
        if state.amsart_pre_title {
            match node {
                Node::MakeTitle => {}
                Node::Abstract(_) => {}
                _ => { i += 1; continue; }
            }
            layout_node(node, state, doc, source)?;
            i += 1;
            continue;
        }

        // Detect consecutive minipage groups for side-by-side layout
        if matches!(node, Node::Minipage { .. }) {
            let group_start = i;
            let mut group_end = i + 1;
            // Collect consecutive minipages, possibly separated by HSpace/HFill
            while group_end < nodes.len() {
                match &nodes[group_end] {
                    Node::HSpace(_) => {
                        // Check if next after HSpace is another minipage
                        if group_end + 1 < nodes.len() && matches!(&nodes[group_end + 1], Node::Minipage { .. }) {
                            group_end += 2; // skip HSpace + Minipage
                        } else {
                            break;
                        }
                    }
                    Node::Minipage { .. } => {
                        group_end += 1;
                    }
                    _ => break,
                }
            }
            let minipage_count = nodes[group_start..group_end].iter()
                .filter(|n| matches!(n, Node::Minipage { .. }))
                .count();
            if minipage_count > 1 {
                layout_minipage_group(&nodes[group_start..group_end], state, doc, source)?;
                i = group_end;
                continue;
            }
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
                if text.is_empty() { i += 1; continue; }
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
        i += 1;
    }
    Ok(())
}

/// Layout a group of consecutive minipages side-by-side.
/// Renders each minipage sequentially into the main element buffer,
/// then shifts elements horizontally to achieve side-by-side placement.
fn layout_minipage_group(nodes: &[Node], state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Collect minipage widths and content references
    let mut minipages: Vec<(f32, &Vec<Node>)> = Vec::new();
    for node in nodes {
        if let Node::Minipage { width, content } = node {
            minipages.push((*width, content));
        }
        // Skip HSpace nodes between minipages
    }
    if minipages.is_empty() { return Ok(()); }

    let text_width = state.page_setup.text_width();
    let text_left = state.text_left();

    // Calculate total minipage width and gap
    let total_mp_width: f32 = minipages.iter().map(|(w, _)| *w).sum();
    let gap = if minipages.len() > 1 && total_mp_width < text_width {
        (text_width - total_mp_width) / (minipages.len() - 1) as f32
    } else {
        0.0
    };

    // Save state before laying out minipages
    let saved_indent = state.indent;
    let saved_right_indent = state.right_indent;
    let saved_para_indent = state.paragraph_indent;
    let start_y = state.current_y;

    // Layout each minipage and record element ranges + heights
    struct MpResult {
        elem_start: usize,
        elem_end: usize,
        rect_start: usize,
        rect_end: usize,
        width: f32,
        height: f32,
    }
    let mut results: Vec<MpResult> = Vec::new();

    for (mp_width, content) in &minipages {
        let mp_w = if *mp_width > text_width { text_width } else { *mp_width };

        // Constrain layout to minipage width
        state.set_indent(0.0);
        state.set_right_indent(text_width - mp_w);
        state.paragraph_indent = saved_para_indent.min(mp_w * 0.05); // scale indent to width
        state.current_y = start_y;
        state.current_x = state.text_left();

        let elem_start = state.all_elements.len();
        let rect_start = state.rect_data.len();

        layout_nodes(content, state, doc, source)?;

        let elem_end = state.all_elements.len();
        let rect_end = state.rect_data.len();
        let height = state.current_y - start_y;

        results.push(MpResult {
            elem_start, elem_end, rect_start, rect_end,
            width: mp_w, height,
        });
    }

    // Now shift each minipage's elements to the correct X position
    let mut x_offset = text_left;
    let mut max_height: f32 = 0.0;

    for result in &results {
        let shift_x = x_offset - text_left; // how much to shift right from default left
        max_height = max_height.max(result.height);

        if shift_x.abs() > 0.1 {
            for elem in &mut state.all_elements[result.elem_start..result.elem_end] {
                match elem {
                    PageElement::Text { x, .. } => { *x += shift_x; }
                    PageElement::Line { x1, x2, .. } => { *x1 += shift_x; *x2 += shift_x; }
                    PageElement::Image { x, .. } => { *x += shift_x; }
                    PageElement::Rect(idx) => {
                        let ri = *idx as usize;
                        if ri < state.rect_data.len() {
                            state.rect_data[ri].x += shift_x;
                        }
                    }
                }
            }
        }

        x_offset += result.width + gap;
    }

    // Restore state, advance Y to the max height of all minipages
    state.set_indent(saved_indent);
    state.set_right_indent(saved_right_indent);
    state.paragraph_indent = saved_para_indent;
    state.current_y = start_y + max_height;
    state.current_x = state.text_left();
    state.suppress_next_indent = true;

    Ok(())
}

/// Render deferred top-of-page floats
fn render_top_floats(state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    let floats = state.take_top_floats();
    for float_data in &floats {
        let fig = FigureData {
            content: float_data.content.clone(),
            caption: float_data.caption.clone(),
            label: float_data.label.clone(),
            placement: "h".to_string(),
            starred: false,
        };
        layout_figure_inline(&fig, state, doc, source)?;
    }
    Ok(())
}

/// Render a tcolorbox / colored framed box
fn layout_colorbox(boxdata: &ColorBoxData, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    state.add_vertical_space(8.0);
    let padding = boxdata.padding;

    let start_y = state.current_y;
    let title_height = if boxdata.title.is_some() { 20.0 } else { 0.0 };

    // Narrow content area using indent
    let saved_indent = state.indent;
    let saved_right_indent = state.right_indent;
    state.set_indent(saved_indent + padding);
    state.set_right_indent(saved_right_indent + padding);

    let box_x = state.page_setup.margin_left + saved_indent;
    let box_w = state.page_setup.text_width() - saved_indent - saved_right_indent;

    if let Some(ref title) = boxdata.title {
        // Title bar
        state.current_y += 2.0;
        let title_bar_y = state.current_y;
        if boxdata.corner_radius > 0.0 {
            state.emit_rounded_rect(
                box_x, title_bar_y, box_w, title_height,
                Some(boxdata.frame_color), None, boxdata.corner_radius,
            );
        } else {
            state.emit_rect(
                box_x, title_bar_y, box_w, title_height,
                Some(boxdata.frame_color), None,
            );
        }
        // Render title text in white
        state.current_x = state.text_left();
        for tn in title {
            let saved_color = state.current_color;
            state.current_color = Color::WHITE;
            layout_node(tn, state, doc, source)?;
            state.current_color = saved_color;
        }
        state.current_y = title_bar_y + title_height + 4.0;
        state.current_x = state.text_left();
    } else {
        state.current_y += padding;
    }

    // Record element index before content — we'll insert background rect here
    let elem_start = state.all_elements.len();

    // Render box content
    layout_nodes(&boxdata.content, state, doc, source)?;
    state.current_y += padding;

    // Restore margins
    state.set_indent(saved_indent);
    state.set_right_indent(saved_right_indent);

    let total_height = state.current_y - start_y;

    // Insert background rect BEFORE content elements (PDF paints in order)
    let has_bg = boxdata.bg_color != Color::WHITE;
    let has_frame = boxdata.rule_width > 0.0;
    let fill = if has_bg { Some(boxdata.bg_color) } else { None };
    let stroke = if has_frame { Some(boxdata.frame_color) } else { None };
    if fill.is_some() || stroke.is_some() {
        let idx = state.rect_data.len() as u32;
        if boxdata.corner_radius > 0.0 {
            state.rect_data.push(RectData {
                x: box_x, y: start_y, width: box_w, height: total_height,
                fill, stroke, stroke_width: boxdata.rule_width.max(0.5), corner_radius: boxdata.corner_radius,
            });
        } else {
            state.rect_data.push(RectData {
                x: box_x, y: start_y, width: box_w, height: total_height,
                fill, stroke, stroke_width: boxdata.rule_width.max(0.5), corner_radius: 0.0,
            });
        }
        state.all_elements.insert(elem_start, PageElement::Rect(idx));
    }

    state.add_vertical_space(8.0);
    state.current_x = state.text_left();
    Ok(())
}

/// Render a figure/table float inline at the current position
fn layout_figure_inline(fig: &FigureData, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
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
        let prefix_width = font::measure_text(prefix, FontId::TimesBold, state.current_font_size);
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
    Ok(())
}

fn layout_node(node: &Node, state: &mut LayoutState, doc: &Document, source: &str) -> Result<()> {
    // Render any deferred top-of-page floats
    if state.should_render_top_floats() {
        render_top_floats(state, doc, source)?;
    }

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
            // After title, activate two-column mode if document uses it
            if state.page_setup.columns >= 2 && !state.twocolumn_active {
                state.enter_twocolumn();
            }
        }

        Node::TableOfContents => {
            layout_table_of_contents(state)?;
        }

        Node::ListOfFigures | Node::ListOfTables => {
            // Stub — emit heading, content not yet tracked
            let title = if matches!(node, Node::ListOfFigures) { "List of Figures" } else { "List of Tables" };
            state.emit_section_heading(title, 14.0);
        }

        Node::PageNumbering(style) => {
            state.set_page_numbering(style);
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

        // Group containing block elements (e.g. from \resizebox wrapping a table)
        Node::Group(children) if !is_inline_node(node) => {
            layout_nodes(children, state, doc, source)?;
        }

        Node::Figure(fig) => {
            // In two-column mode, starred figures span both columns
            let was_spanning = state.spanning_mode;
            if fig.starred && state.twocolumn_active && !state.spanning_mode {
                state.enter_spanning();
            }

            // Check float placement hint
            let placement = &fig.placement;
            let has_h = placement.contains('h') || placement.contains('H') || placement.contains('!');
            let has_t = placement.contains('t');

            // If placement is purely [t] (no [h]), defer to top of next page
            if has_t && !has_h && !state.deferred_top_floats.is_empty() == false {
                // Defer: store content for top-of-next-page placement
                use state::DeferredFloat;
                state.deferred_top_floats.push(DeferredFloat {
                    content: fig.content.clone(),
                    caption: fig.caption.clone(),
                    label: fig.label.clone(),
                    is_table: false,
                });
            } else {
                // Inline placement (default, [h], [H], [htbp], etc.)
                layout_figure_inline(fig, state, doc, source)?;
            }

            // Restore column mode after starred figure
            if fig.starred && state.twocolumn_active && !was_spanning {
                state.exit_spanning();
            }
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
                let tw = font::measure_text(&label, FontId::TimesRoman, 8.0);
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

        Node::SetParIndent(pts) => {
            state.paragraph_indent = *pts;
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

        Node::Proof { header, content } => {
            layout_proof(header.as_deref(), content, state, doc, source)?;
        }

        Node::Algorithm { caption, label, content, line_numbered } => {
            layout_algorithm(caption, label, content, *line_numbered, state, doc)?;
        }

        Node::Minipage { width, content } => {
            let saved_indent = state.indent;
            let saved_right_indent = state.right_indent;
            // Constrain width: set right_indent so available width = min(width, text_width)
            let text_width = state.page_setup.text_width();
            let mp_width = if *width > text_width { text_width } else { *width };
            let extra_right = text_width - mp_width;
            state.set_right_indent(saved_right_indent + extra_right);
            layout_nodes(content, state, doc, source)?;
            state.set_indent(saved_indent);
            state.set_right_indent(saved_right_indent);
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
            state.current_x += font::measure_text(&num_str, FontId::TimesRoman, sup_size);
            state.footnotes.push(content.clone());
            state.reserve_footnote_space();
        }

        Node::ColorBox(boxdata) => {
            layout_colorbox(boxdata, state, doc, source)?;
        }

        Node::WrapFigure { content, caption, label, .. } => {
            // Simplified: render as inline figure (full wrapfigure layout would need
            // text flow tracking which is very complex)
            state.add_vertical_space(6.0);
            layout_nodes(content, state, doc, source)?;
            if let Some(cap) = caption {
                let saved_style = state.current_font_style;
                state.current_font_style = FontStyle::Regular;
                state.figure_counter += 1;
                let fig_num = state.figure_counter;
                let mut cap_text = format!("Figure {}: ", fig_num);
                if let Some(ref l) = label {
                    state.label_map.insert(l.clone(), fig_num.to_string());
                }
                state.current_x = state.text_left();
                state.emit_text(&cap_text, state.current_font_size * 0.9, FontStyle::Bold, Color::BLACK);
                state.current_x += font::measure_text(&cap_text, FontId::TimesBold, state.current_font_size * 0.9);
                for cn in cap { layout_node(cn, state, doc, source)?; }
                state.current_y += state.current_font_size * 1.2;
                state.current_font_style = saved_style;
            }
            state.add_vertical_space(6.0);
            state.current_x = state.text_left();
        }

        Node::SubFigure { width, content, caption } => {
            // width is either fraction of textwidth (0..1) or absolute points
            let text_w = state.text_width();
            let sub_w = if *width <= 1.0 && *width > 0.0 { *width * text_w } else if *width > 1.0 { *width } else { text_w * 0.45 };

            // Save state for side-by-side layout
            let saved_x = state.current_x;
            let saved_y = state.current_y;
            let saved_indent = state.indent;
            let saved_right_indent = state.right_indent;

            // Constrain width for this subfigure
            let sub_left = state.current_x;
            state.indent = sub_left - state.page_setup.margin_left;
            state.right_indent = (state.page_setup.width - state.page_setup.margin_right) - (sub_left + sub_w);
            if state.right_indent < 0.0 { state.right_indent = 0.0; }
            state.cached_text_width = state.page_setup.text_width() - state.indent - state.right_indent;
            state.cached_text_left = state.page_setup.margin_left + state.indent;

            layout_nodes(content, state, doc, source)?;

            if let Some(cap) = caption {
                let fs = state.current_font_size * 0.8;
                state.current_x = state.cached_text_left;
                for cn in cap { layout_node(cn, state, doc, source)?; }
                state.current_y += fs * 1.2;
            }

            let bottom_y = state.current_y;

            // Restore and advance horizontally
            state.indent = saved_indent;
            state.right_indent = saved_right_indent;
            state.cached_text_width = state.page_setup.text_width() - state.indent - state.right_indent;
            state.cached_text_left = state.page_setup.margin_left + state.indent;

            // If next sibling is also a SubFigure, stay on same row
            state.current_x = sub_left + sub_w + 4.0;
            // If we overflowed the line, wrap to next line
            if state.current_x + sub_w * 0.3 > state.page_setup.width - state.page_setup.margin_right {
                state.current_x = state.text_left();
                state.current_y = bottom_y;
            } else {
                // Stay at saved_y for next subfigure (side-by-side)
                state.current_y = saved_y;
            }
        }

        Node::Text(_) | Node::TextRef(_, _) | Node::Bold(_) | Node::Italic(_) | Node::Monospace(_)
        | Node::SmallCaps(_) | Node::Underline(_) | Node::Emph(_)
        | Node::InlineMath(_) | Node::Group(_) | Node::Colored { .. }
        | Node::FontSize { .. } | Node::Superscript(_) | Node::Subscript(_)
        | Node::NonBreakingSpace | Node::HSpace(_) | Node::LineBreak
        | Node::Code(_) | Node::Strikethrough(_) => {
            layout_paragraph(&[node.clone()], state, doc, source)?;
        }

        Node::Citation(key, opt, style) => {
            let cite_text = resolve_citations(key, opt.as_deref(), &state.citation_map, *style, &state.author_year_map);
            state.emit_text(&cite_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&cite_text, FontId::TimesRoman, state.current_font_size);
        }

        Node::Ref(label) => {
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                resolved.clone()
            } else {
                "??".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::TimesRoman, state.current_font_size);
        }

        Node::EqRef(label) => {
            let ref_text = if let Some(resolved) = state.label_map.get(label) {
                format!("({})", resolved)
            } else {
                "(??)".to_string()
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::TimesRoman, state.current_font_size);
        }

        Node::Cref(label, capitalize) => {
            let num = state.label_map.get(label).cloned().unwrap_or_else(|| "??".to_string());
            let type_name = state.label_types.get(label).map(|s| s.as_str()).unwrap_or("section");
            let prefix = match type_name {
                "section" => if *capitalize { "Section" } else { "section" },
                "subsection" => if *capitalize { "Section" } else { "section" },
                "subsubsection" => if *capitalize { "Section" } else { "section" },
                "chapter" => if *capitalize { "Chapter" } else { "chapter" },
                "part" => if *capitalize { "Part" } else { "part" },
                "figure" => if *capitalize { "Figure" } else { "figure" },
                "table" => if *capitalize { "Table" } else { "table" },
                "equation" => if *capitalize { "Equation" } else { "eq." },
                "theorem" => if *capitalize { "Theorem" } else { "theorem" },
                "lemma" => if *capitalize { "Lemma" } else { "lemma" },
                "proposition" => if *capitalize { "Proposition" } else { "proposition" },
                "corollary" => if *capitalize { "Corollary" } else { "corollary" },
                "definition" => if *capitalize { "Definition" } else { "definition" },
                "remark" => if *capitalize { "Remark" } else { "remark" },
                "example" => if *capitalize { "Example" } else { "example" },
                _ => if *capitalize { "Section" } else { "section" },
            };
            let ref_text = if type_name == "equation" && !*capitalize {
                format!("{}~({})", prefix, num)
            } else {
                format!("{}~{}", prefix, num)
            };
            state.emit_text(&ref_text, state.current_font_size, FontStyle::Regular, Color::BLACK);
            state.current_x += font::measure_text(&ref_text, FontId::TimesRoman, state.current_font_size);
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

        Node::TwoColumn(content) => {
            if content.is_empty() {
                // Empty TwoColumn = just activate two-column mode (document class option)
                if !state.twocolumn_active {
                    state.enter_twocolumn();
                }
            } else {
                // \twocolumn[spanning content] — LaTeX semantics:
                // 1. Layout spanning content at full page width (not in columns)
                // 2. Then enter two-column mode for all subsequent body content
                if state.twocolumn_active {
                    // Already in twocolumn — enter spanning mode for the bracket content
                    state.enter_spanning();
                    layout_nodes(content, state, doc, source)?;
                    state.exit_spanning();
                } else {
                    // Layout spanning content at full width first
                    layout_nodes(content, state, doc, source)?;
                    // Then enter two-column mode for everything after
                    state.enter_twocolumn();
                }
            }
        }

        Node::OneColumn => {
            // Switch back to single-column mode
            if state.twocolumn_active {
                state.twocolumn_active = false;
                state.spanning_mode = false;
                state.current_column = 0;
                state.cached_text_width = state.page_setup.text_width() - state.indent - state.right_indent;
                state.cached_text_left = state.page_setup.margin_left + state.indent;
                state.current_x = state.cached_text_left;
                state.cached_font_key = u32::MAX;
                state.new_page(); // force page break on column mode switch
            }
        }

        Node::Label(_) | Node::Raw(_) | Node::BibItem(_) | Node::DefineColor { .. } => {}

        Node::SetCounter(name, val) => {
            let (counter_name, is_add) = if let Some(n) = name.strip_prefix("add:") {
                (n, true)
            } else {
                (name.as_str(), false)
            };
            match counter_name {
                "equation" => {
                    if is_add { state.equation_counter = (state.equation_counter as i32 + val) as u32; }
                    else { state.equation_counter = *val as u32; }
                }
                "figure" => {
                    if is_add { state.figure_counter = (state.figure_counter as i32 + val) as u32; }
                    else { state.figure_counter = *val as u32; }
                }
                "table" => {
                    if is_add { state.table_counter = (state.table_counter as i32 + val) as u32; }
                    else { state.table_counter = *val as u32; }
                }
                "footnote" => {
                    if is_add { state.footnote_counter = (state.footnote_counter as i32 + val) as u32; }
                    else { state.footnote_counter = *val as u32; }
                }
                _ => {}
            }
        }

        Node::EnDash | Node::EmDash | Node::Ellipsis
        | Node::Copyright | Node::Registered | Node::Trademark
        | Node::LeftQuote | Node::RightQuote
        | Node::LeftDoubleQuote | Node::RightDoubleQuote
        | Node::Ampersand | Node::Percent | Node::Dollar
        | Node::Hash | Node::Underscore | Node::Backslash
        | Node::Tilde | Node::Caret | Node::LeftBrace | Node::RightBrace
        | Node::HFill | Node::Dingbat(_) => {}
    }

    Ok(())
}
