/// Text content layout and plain text extraction

use std::collections::HashMap;
use crate::document::*;

use super::state::LayoutState;
use super::types::*;

use anyhow::Result;

// ============================================================
// Text layout functions
// ============================================================

pub(super) fn layout_paragraph(children: &[Node], state: &mut LayoutState, _doc: &Document, source: &str) -> Result<()> {
    let with_indent = if state.suppress_next_indent { state.suppress_next_indent = false; false } else { true };
    super::spans::layout_rich_paragraph(children, state, source, with_indent)
}

/// Calculate word spacing for justified text.
#[inline]
pub(super) fn justify_line(line: &[u8], available_width: f32, avg_width: f32, font_size: f32, is_last_line: bool) -> i16 {
    justify_line_ext(line, available_width, avg_width, font_size, is_last_line, crate::font::FontId::Helvetica)
}

pub(super) fn justify_line_ext(line: &[u8], available_width: f32, avg_width: f32, font_size: f32, is_last_line: bool, font_id: crate::font::FontId) -> i16 {
    if is_last_line { return 0; }
    let num_spaces = memchr::memchr_iter(b' ', line).count();
    if num_spaces == 0 { return 0; }

    // Use per-glyph font metrics for accurate width — but only for
    // lines where char-count estimation would be noticeably wrong
    let natural_width = {
        let est_width = line.len() as f32 * avg_width;
        let est_slack = (available_width - est_width).abs();
        // Only use accurate metrics if the estimated slack is small
        // (i.e., the line is close to full width where justification matters)
        if est_slack < font_size * 2.0 && line.len() <= 200 {
            let widths = crate::font::font_widths(font_id);
            let scale = font_size / 1000.0;
            let mut w = 0.0f32;
            for &b in line {
                w += widths[b as usize] as f32 * scale;
            }
            w
        } else {
            est_width
        }
    };

    let extra = available_width - natural_width;
    // Skip justification if line is way too short (< 70% full) or way too long
    if extra > available_width * 0.3 { return 0; }
    if extra < -font_size * 1.0 { return 0; }
    let ws = extra / num_spaces as f32;
    // Clamp: allow slight compression (-0.5pt) and moderate stretch (+4pt per space)
    let ws_clamped = ws.max(-0.5).min(4.0);
    (ws_clamped * 50.0).min(i16::MAX as f32) as i16
}

/// Core word-wrapping and text layout.
pub(super) fn layout_text_content(text: &str, state: &mut LayoutState) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let pi = if state.suppress_next_indent { state.suppress_next_indent = false; 0.0 } else { state.paragraph_indent };
    let _para_width = state.text_width() - pi;
    let full_text_width = state.text_width();

    state.ensure_space(line_height);
    let normal_first = state.text_left() + pi;
    let inline_offset = if state.current_x > normal_first + 1.0 {
        state.current_x - state.text_left()
    } else {
        pi
    };
    if text.len() <= max_chars_single && inline_offset <= pi + 0.1 {
        state.current_x = state.text_left() + pi;
        state.emit_text(text, font_size, font_style, color);
        state.current_y += step;
    } else {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;
        while pos < len && bytes[pos] <= b' ' { pos += 1; }

        let mut push_start: usize = 0;
        let mut buf_push_pos = state.all_text.len() - state.current_page_text_start as usize;
        state.all_text.push_str(text);

        let x_first = state.text_left() + inline_offset;
        let x_rest = state.text_left();
        let first_line_width = full_text_width - inline_offset;
        let max_chars_first = (first_line_width / avg_width) as usize;
        let max_chars_rest = max_chars_single;

        let mut lines_until_break = ((state.cached_max_y - state.current_y - line_height) / step) as i32 + 1;

        // Orphan prevention: if only 1 line would fit, move whole paragraph to next page
        // Also estimate total lines — if paragraph is 3+ lines and only 1 line fits, start fresh
        let est_total_lines = (len as f32 * avg_width / full_text_width).ceil() as i32;
        if lines_until_break <= 1 && est_total_lines > 1 {
            state.new_page();
            push_start = 0;
            buf_push_pos = 0;
            state.all_text.push_str(text);
            lines_until_break = ((state.cached_max_y - state.cached_start_y - line_height) / step) as i32 + 1;
        }

        // First line
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
                    x: x_first, y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100, font_style, color, word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }

        // Remaining lines
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

                // Hyphenation
                let line_chars = line_end - line_start;
                let slack_chars = max_chars_rest.saturating_sub(line_chars);
                if slack_chars >= 4 && next_pos < len {
                    let mut ws_skip = next_pos;
                    while ws_skip < len && bytes[ws_skip] <= b' ' { ws_skip += 1; }
                    if ws_skip < len {
                        let mut we = ws_skip;
                        while we < len && bytes[we] > b' ' { we += 1; }
                        let next_word = &bytes[ws_skip..we];
                        if next_word.len() >= 5 {
                            let max_prefix = slack_chars.saturating_sub(1);
                            if let Some(bp) = crate::hyphenate::best_break(next_word, max_prefix) {
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
                                    x: x_rest, y: state.current_y, text_offset: hyph_off,
                                    text_len: hyph_len as u16, font_size_100, font_style, color,
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
                    x: x_rest, y: state.current_y,
                    text_offset: (buf_push_pos + line_start - push_start) as u32,
                    text_len: (line_end - line_start) as u16,
                    font_size_100, font_style, color, word_spacing_50: ws,
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

/// Zero-copy variant: stores source offsets (flagged with high bit) instead of copying text.
pub(super) fn layout_text_content_source(text: &str, state: &mut LayoutState, src_off: u32) -> Result<()> {
    let (avg_width, line_height, step, font_size_100, max_chars_single) = state.wrap_params();
    let font_size = state.current_font_size;
    let font_style = state.current_font_style;
    let color = state.current_color;
    let pi = if state.suppress_next_indent { state.suppress_next_indent = false; 0.0 } else { state.paragraph_indent };
    let para_width = state.text_width() - pi;
    let full_text_width = state.text_width();

    state.ensure_space(line_height);
    if text.len() <= max_chars_single {
        state.current_x = state.text_left() + pi;
        state.all_elements.push(PageElement::Text {
            x: state.current_x, y: state.current_y,
            text_offset: src_off | SOURCE_REF_FLAG,
            text_len: text.len().min(65535) as u16,
            font_size_100, font_style, color, word_spacing_50: 0,
        });
        state.current_y += step;
    } else {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = 0;
        while pos < len && bytes[pos] <= b' ' { pos += 1; }

        let x_first = state.text_left() + pi;
        let x_rest = state.text_left();
        let max_chars_first = ((para_width - pi) / avg_width) as usize;
        let max_chars_rest = max_chars_single;

        let mut lines_until_break = ((state.cached_max_y - state.current_y - line_height) / step) as i32 + 1;

        // First line
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
                    x: x_first, y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100, font_style, color, word_spacing_50: ws,
                });
                state.current_y += step;
                lines_until_break -= 1;
            }

            pos = next_pos;
            while pos < len && bytes[pos] <= b' ' { pos += 1; }
        }

        // Remaining lines
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
                    x: x_rest, y: state.current_y,
                    text_offset: (src_off + line_start as u32) | SOURCE_REF_FLAG,
                    text_len: (line_end - line_start) as u16,
                    font_size_100, font_style, color, word_spacing_50: ws,
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

/// Text content layout without paragraph indent
pub(super) fn layout_text_content_no_indent(text: &str, state: &mut LayoutState) -> Result<()> {
    let saved_indent = state.paragraph_indent;
    state.paragraph_indent = 0.0;
    layout_text_content(text, state)?;
    state.paragraph_indent = saved_indent;
    Ok(())
}

pub(super) fn layout_text_line(text: &str, state: &mut LayoutState) {
    state.emit_text(text, state.current_font_size, state.current_font_style, state.current_color);
}

// ============================================================
// Plain text extraction (node_to_text, math_to_text, etc.)
// ============================================================

pub(super) fn nodes_to_text_buf(nodes: &[Node], buf: &mut String, source: &str) {
    buf.clear();
    for node in nodes {
        node_to_text(node, buf, source);
    }
}

pub fn nodes_to_text(nodes: &[Node], source: &str) -> String {
    if nodes.len() == 1 {
        if let Node::Text(s) = &nodes[0] {
            return s.clone();
        }
        if let Node::TextRef(offset, len) = &nodes[0] {
            return source[*offset as usize..(*offset as usize + *len as usize)].to_string();
        }
    }

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

pub(super) fn node_to_text(node: &Node, out: &mut String, source: &str) {
    node_to_text_ext(node, out, source, None);
}

pub(super) fn node_to_text_resolved(node: &Node, out: &mut String, source: &str, labels: &HashMap<String, String>) {
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
            for child in content { node_to_text_ext(child, out, source, labels); }
        }
        Node::FontSize { content, .. } => {
            for child in content { node_to_text_ext(child, out, source, labels); }
        }
        Node::Paragraph(children) => {
            for child in children { node_to_text_ext(child, out, source, labels); }
        }
        Node::InlineMath(math) => { math_to_text_buf(math, out); }
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
        Node::Footnote(_content) => { out.push('\u{2020}'); }
        Node::Ref(label) | Node::Cref(label, _) => {
            if let Some(map) = labels {
                if let Some(resolved) = map.get(label) { out.push_str(resolved); }
                else { out.push_str("??"); }
            } else { out.push_str("??"); }
        }
        Node::EqRef(label) => {
            out.push('(');
            if let Some(map) = labels {
                if let Some(resolved) = map.get(label) { out.push_str(resolved); }
                else { out.push_str("??"); }
            } else { out.push_str("??"); }
            out.push(')');
        }
        Node::Citation(key, opt, _style) => {
            out.push('[');
            out.push_str(key);
            if let Some(o) = opt { out.push_str(", "); out.push_str(o); }
            out.push(']');
        }
        Node::Dingbat(code) => out.push(char::from(*code)),
        Node::Label(_) | Node::BibItem(_) => {}
        Node::Code(s) => out.push_str(s),
        Node::Href { content, .. } => {
            for c in content { node_to_text_ext(c, out, source, labels); }
        }
        _ => {}
    }
}

pub(super) fn resolve_citations(
    key: &str,
    opt: Option<&str>,
    citation_map: &HashMap<String, u32>,
    style: crate::document::CitationStyle,
    author_year_map: &HashMap<String, (String, String)>,
) -> String {
    use crate::document::CitationStyle;

    let keys: Vec<&str> = key.split(',').map(|k| k.trim()).collect();

    // Check if we have author-year data for any key
    let has_author_year = keys.iter().any(|k| author_year_map.contains_key(*k));

    // When author-year data is available, use it for all styles including Numeric (\cite{})
    if has_author_year {
        // Determine effective style: \cite{} (Numeric) becomes Parenthetical when author-year data exists
        let eff_style = if style == CitationStyle::Numeric { CitationStyle::Parenthetical } else { style };
        let mut parts = Vec::new();
        for k in &keys {
            if let Some((author, year)) = author_year_map.get(*k) {
                match eff_style {
                    CitationStyle::Parenthetical => parts.push(format!("{}, {}", author, year)),
                    CitationStyle::Textual => parts.push(format!("{} ({})", author, year)),
                    CitationStyle::AuthorOnly => parts.push(author.clone()),
                    CitationStyle::YearOnly => parts.push(year.clone()),
                    CitationStyle::AltNoParen => parts.push(format!("{} {}", author, year)),
                    CitationStyle::Numeric => {
                        if let Some(&num) = citation_map.get(*k) {
                            parts.push(num.to_string());
                        }
                    }
                }
            } else if let Some(&num) = citation_map.get(*k) {
                parts.push(num.to_string());
            } else {
                parts.push((*k).to_string());
            }
        }
        let base = parts.join("; ");
        match eff_style {
            CitationStyle::Parenthetical => {
                if let Some(text) = opt {
                    format!("({}; {})", base, text.replace('~', " "))
                } else {
                    format!("({})", base)
                }
            }
            CitationStyle::Textual | CitationStyle::AuthorOnly
            | CitationStyle::YearOnly | CitationStyle::AltNoParen => base,
            _ => format!("[{}]", base),
        }
    } else {
        // Numeric formatting (fallback)
        let mut nums = Vec::new();
        for k in &keys {
            if let Some(&num) = citation_map.get(*k) {
                nums.push(num.to_string());
            } else {
                nums.push((*k).to_string());
            }
        }
        let base = nums.join(",");
        match opt {
            Some(text) => {
                let clean = text.replace('~', " ");
                format!("[{}, {}]", base, clean)
            }
            None => format!("[{}]", base),
        }
    }
}

// ============================================================
// Math-to-text conversion
// ============================================================

#[inline]
pub(super) fn math_symbol_to_text(s: &str, out: &mut String) {
    match s.as_bytes() {
        [b] if *b < 0x80 => out.push(*b as char),
        [0xC2, b] => out.push(char::from(*b | 0x80)),
        [0xC3, b] => out.push(char::from((*b & 0x3F) | 0xC0)),
        _ => {
            let ch = s.chars().next().unwrap_or('?');
            match ch {
                '\u{2264}' => out.push_str("<="),
                '\u{2265}' => out.push_str(">="),
                '\u{2260}' => out.push_str("!="),
                '\u{2248}' => out.push_str("~~"),
                '\u{2261}' => out.push_str("==="),
                '\u{2192}' => out.push_str("->"),
                '\u{2190}' => out.push_str("<-"),
                '\u{2194}' => out.push_str("<->"),
                '\u{21D2}' => out.push_str("=>"),
                '\u{21D0}' => out.push_str("<="),
                '\u{21D4}' => out.push_str("<=>"),
                '\u{2208}' => out.push_str("in"),
                '\u{2209}' => out.push_str("not in"),
                '\u{2282}' => out.push_str("c="),
                '\u{2283}' => out.push_str("=c"),
                '\u{222A}' => out.push_str("U"),
                '\u{2229}' => out.push_str("n"),
                '\u{2200}' => out.push_str("for all"),
                '\u{2203}' => out.push_str("exists"),
                '\u{221E}' => out.push_str("inf"),
                '\u{2202}' => out.push_str("d"),
                '\u{2207}' => out.push_str("V"),
                '\u{221A}' => out.push_str("sqrt"),
                '\u{2211}' => out.push_str("S"),
                '\u{220F}' => out.push_str("P"),
                '\u{222B}' => out.push_str("int"),
                '\u{2205}' => out.push_str("{}"),
                '\u{2220}' => out.push_str("L"),
                '\u{2026}' => out.push_str("..."),
                '\u{2032}' => out.push('\''),
                '\u{2213}' => out.push_str("-/+"),
                '\u{03B1}'..='\u{03C9}' => out.push(ch),
                '\u{0393}' | '\u{0394}' | '\u{0398}' | '\u{039B}' | '\u{039E}'
                | '\u{03A0}' | '\u{03A3}' | '\u{03A6}' | '\u{03A8}' | '\u{03A9}'
                    => out.push(ch),
                '\u{00B0}' => out.push('\u{00B0}'),
                _ => out.push('?'),
            }
        }
    }
}

pub(super) fn math_to_text_buf(nodes: &[MathNode], out: &mut String) {
    for node in nodes { math_node_to_text(node, out); }
}

pub(super) fn math_to_text(nodes: &[MathNode]) -> String {
    let mut result = String::new();
    math_to_text_buf(nodes, &mut result);
    result
}

pub(super) fn math_node_to_text(node: &MathNode, out: &mut String) {
    match node {
        MathNode::Number(s) => out.push_str(s),
        MathNode::Variable(c) => out.push(*c),
        MathNode::Operator(s) => { out.push(' '); math_symbol_to_text(s, out); out.push(' '); }
        MathNode::Text(s) => out.push_str(s),
        MathNode::Symbol(s) => math_symbol_to_text(s, out),
        MathNode::Function(name) => out.push_str(name),
        MathNode::Space(_) => out.push(' '),
        MathNode::Frac { numer, denom } => {
            out.push('('); math_to_text_buf(numer, out); out.push_str(")/(");
            math_to_text_buf(denom, out); out.push(')');
        }
        MathNode::Sqrt { index, radicand } => {
            out.push_str("\u{221A}");
            if let Some(idx) = index { out.push('['); math_to_text_buf(idx, out); out.push(']'); }
            out.push('('); math_to_text_buf(radicand, out); out.push(')');
        }
        MathNode::Super(nodes) | MathNode::Sub(nodes) | MathNode::Group(nodes) => {
            math_to_text_buf(nodes, out);
        }
        MathNode::Sum { lower, upper } => {
            out.push_str("\u{2211}");
            if let Some(l) = lower { out.push_str("_{"); math_to_text_buf(l, out); out.push('}'); }
            if let Some(u) = upper { out.push_str("^{"); math_to_text_buf(u, out); out.push('}'); }
        }
        MathNode::Integral { lower, upper } => {
            out.push_str("\u{222B}");
            if let Some(l) = lower { out.push_str("_{"); math_to_text_buf(l, out); out.push('}'); }
            if let Some(u) = upper { out.push_str("^{"); math_to_text_buf(u, out); out.push('}'); }
        }
        MathNode::Product { lower, upper } => {
            out.push_str("\u{220F}");
            if let Some(l) = lower { out.push_str("_{"); math_to_text_buf(l, out); out.push('}'); }
            if let Some(u) = upper { out.push_str("^{"); math_to_text_buf(u, out); out.push('}'); }
        }
        MathNode::Left(d) | MathNode::Right(d) => out.push_str(d),
        MathNode::DelimitedGroup { left, right, content } => {
            out.push_str(left);
            math_to_text_buf(content, out);
            out.push_str(right);
        }
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
                if let Some(c) = cond { out.push_str(" if "); math_to_text_buf(c, out); }
                if i < rows.len() - 1 { out.push_str(", "); }
            }
        }
        MathNode::Binom { top, bottom } => {
            out.push('('); math_to_text_buf(top, out);
            out.push_str(" choose "); math_to_text_buf(bottom, out); out.push(')');
        }
        MathNode::Overset { over: _, base } | MathNode::Underset { under: _, base } => {
            math_to_text_buf(base, out);
        }
        MathNode::OperatorName(name) => out.push_str(name),
        MathNode::MathFont { content, .. } => math_to_text_buf(content, out),
        MathNode::AlignmentMark => out.push_str("  "),
        MathNode::NewLine => out.push('\n'),
        MathNode::Phantom(_) => {}
        MathNode::StyleSwitch(_) | MathNode::Boxed(_) | MathNode::LimitOp { .. } | MathNode::NoTag | MathNode::Tag(_) | MathNode::Label(_) | MathNode::Substack(_) => {}
        MathNode::StyledText(text, _) => out.push_str(text),
        MathNode::BigDelim { delim, .. } => out.push_str(delim),
        MathNode::Intertext(text) => out.push_str(text),
        MathNode::VPhantom(_) | MathNode::HPhantom(_) => {}
        MathNode::Pmod(content) => { out.push_str(" (mod "); math_to_text_buf(content, out); out.push(')'); }
        MathNode::Pod(content) => { out.push_str(" ("); math_to_text_buf(content, out); out.push(')'); }
        MathNode::Bmod => out.push_str(" mod "),
        MathNode::MathRel(content) | MathNode::MathBin(content) => math_to_text_buf(content, out),
        MathNode::Rule { .. } => {}
        MathNode::Middle(d) => out.push_str(d),
    }
}
