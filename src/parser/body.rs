use anyhow::Result;
use crate::lexer::{TokenKind, cmd_id};
use crate::document::*;
use crate::color::Color;
use super::Parser;

impl<'a> Parser<'a> {
    pub(crate) fn parse_body(&mut self) -> Result<Vec<Node>> {
        // Integrated parse + paragraph grouping in a single pass
        let mut result = Vec::with_capacity(256 * 1024);
        let mut current_paragraph: Vec<Node> = Vec::new();

        macro_rules! flush_para {
            () => {
                if !current_paragraph.is_empty() {
                    // Optimize: single TextRef paragraph → TextParagraph (avoids Vec allocation)
                    if current_paragraph.len() == 1 {
                        if let Node::TextRef(offset, len) = current_paragraph[0] {
                            current_paragraph.clear();
                            result.push(Node::TextParagraph(offset, len));
                        } else {
                            result.push(Node::Paragraph(std::mem::take(&mut current_paragraph)));
                        }
                    } else {
                        result.push(Node::Paragraph(std::mem::take(&mut current_paragraph)));
                    }
                }
            };
        }

        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                // Fast path: inline text batching for Text/Space tokens
                // (avoids parse_node function call + match for most common tokens)
                TokenKind::Text | TokenKind::Space => {
                    let first = self.current();
                    let start = first.pos as usize;
                    let mut end = start + first.len as usize;
                    self.pos += 1;
                    loop {
                        let tok = self.current();
                        match tok.kind {
                            TokenKind::Text | TokenKind::Space => {
                                end = tok.pos as usize + tok.len as usize;
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    current_paragraph.push(Node::TextRef(start as u32, (end - start) as u32));
                }
                TokenKind::Command => {
                    let cid = self.current().cmd;
                    // Fast path: \end{document} check
                    if cid == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let env = self.read_braced_text()?;
                            if env == "document" {
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    // Fast path: section/subsection (50K calls, avoid parse_node + parse_command overhead)
                    else if cid == cmd_id::SECTION || cid == cmd_id::SECTION_STAR {
                        let starred = cid == cmd_id::SECTION_STAR;
                        self.advance();
                        if let Some(node) = self.parse_section(SectionLevel::Section, !starred)? {
                            flush_para!();
                            result.push(node);
                        }
                        continue;
                    }
                    else if cid == cmd_id::SUBSECTION || cid == cmd_id::SUBSECTION_STAR {
                        let starred = cid == cmd_id::SUBSECTION_STAR;
                        self.advance();
                        if let Some(node) = self.parse_section(SectionLevel::Subsection, !starred)? {
                            flush_para!();
                            result.push(node);
                        }
                        continue;
                    }
                    // Fast path: \begin (always produces block node)
                    else if cid == cmd_id::BEGIN {
                        self.advance();
                        if let Some(node) = self.parse_begin_environment()? {
                            flush_para!();
                            result.push(node);
                        }
                        continue;
                    }
                    if let Some(node) = self.parse_node()? {
                        if Self::is_block_node(&node) {
                            flush_para!();
                            result.push(node);
                        } else {
                            current_paragraph.push(node);
                        }
                    }
                }
                TokenKind::ParBreak => {
                    self.advance();
                    flush_para!();
                }
                _ => {
                    if let Some(node) = self.parse_node()? {
                        if Self::is_block_node(&node) {
                            flush_para!();
                            result.push(node);
                        } else {
                            current_paragraph.push(node);
                        }
                    }
                }
            }
        }

        flush_para!();
        Ok(result)
    }

    pub(crate) fn parse_node(&mut self) -> Result<Option<Node>> {
        match self.current().kind {
            TokenKind::Eof => Ok(None),
            TokenKind::Space | TokenKind::Text => {
                // Text batching: merge contiguous text+space tokens via source range
                let first = self.current();
                let start = first.pos as usize;
                let mut end = start + first.len as usize;
                self.pos += 1;
                loop {
                    let tok = self.current();
                    match tok.kind {
                        TokenKind::Text | TokenKind::Space => {
                            end = tok.pos as usize + tok.len as usize;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                Ok(Some(Node::TextRef(start as u32, (end - start) as u32)))
            }
            TokenKind::ParBreak | TokenKind::Comment => {
                self.advance();
                Ok(None)
            }
            TokenKind::Tilde => {
                self.advance();
                Ok(Some(Node::NonBreakingSpace))
            }
            TokenKind::Ampersand => {
                self.advance();
                Ok(Some(Node::Ampersand))
            }
            TokenKind::Hash => {
                self.advance();
                Ok(Some(Node::Hash))
            }
            TokenKind::Underscore => {
                self.advance();
                Ok(Some(Node::Underscore))
            }
            TokenKind::Caret => {
                self.advance();
                Ok(Some(Node::Caret))
            }
            TokenKind::DoubleBackslash => {
                self.advance();
                // Consume optional [dim] argument (e.g., \\[2em])
                self.try_read_optional_arg();
                Ok(Some(Node::LineBreak))
            }
            TokenKind::Dollar => {
                self.advance();
                let math = self.parse_math_until_dollar()?;
                Ok(Some(Node::InlineMath(math)))
            }
            TokenKind::DoubleDollar => {
                self.advance();
                let math = self.parse_math_until_double_dollar()?;
                Ok(Some(Node::DisplayMath(Box::new(DisplayMathData {
                    nodes: math,
                    numbered: false,
                    env_type: MathEnvType::DollarDollar,
                }))))
            }
            TokenKind::OpenBrace => {
                self.advance();
                let inner = self.parse_nodes_until_close_brace()?;
                Ok(Some(Node::Group(inner)))
            }
            TokenKind::CloseBrace => {
                // Stray close brace, skip
                self.advance();
                Ok(None)
            }
            TokenKind::OpenBracket => {
                self.advance();
                Ok(Some(Node::Text("[".to_string())))
            }
            TokenKind::CloseBracket => {
                self.advance();
                Ok(Some(Node::Text("]".to_string())))
            }
            TokenKind::Command => {
                self.parse_command()
            }
        }
    }

    pub(crate) fn parse_command(&mut self) -> Result<Option<Node>> {
        // Fast path: dispatch common commands by integer ID (avoids string comparison)
        let cid = self.current().cmd;
        if cid != cmd_id::NONE {
            self.advance();
            return match cid {
                cmd_id::TEXTBF => { let n = self.read_braced_nodes()?; Ok(Some(Node::Bold(n))) }
                cmd_id::TEXTIT => { let n = self.read_braced_nodes()?; Ok(Some(Node::Italic(n))) }
                cmd_id::TEXTTT => { let n = self.read_braced_nodes()?; Ok(Some(Node::Monospace(n))) }
                cmd_id::EMPH => { let n = self.read_braced_nodes()?; Ok(Some(Node::Emph(n))) }
                cmd_id::MAKETITLE => Ok(Some(Node::MakeTitle)),
                cmd_id::TABLEOFCONTENTS => Ok(Some(Node::TableOfContents)),
                cmd_id::NOINDENT => Ok(Some(Node::NoIndent)),
                cmd_id::NEWPAGE => Ok(Some(Node::PageBreak)),
                cmd_id::TWOCOLUMN => {
                    // \twocolumn[spanning content]
                    // The optional arg contains spanning content (title, abstract, etc.)
                    // We parse it as nodes by reading the bracket-delimited content
                    let spanning = self.try_read_bracket_nodes()?;
                    Ok(Some(Node::TwoColumn(spanning)))
                }
                cmd_id::ONECOLUMN => Ok(Some(Node::OneColumn)),
                cmd_id::ICMLTITLE => {
                    // \icmltitle{...} — treat as title, emit MakeTitle node
                    let content = self.read_braced_nodes()?;
                    let mut title_text = String::new();
                    for node in &content {
                        node_to_plain_text(node, &mut title_text, self.source);
                    }
                    self.body_title = Some(title_text.trim().to_string());
                    Ok(Some(Node::MakeTitle))
                }
                cmd_id::CENTERING => Ok(Some(Node::AlignmentDecl(AlignmentMode::Center))),
                cmd_id::HLINE => { self.skip_command_args(); Ok(Some(Node::HRule)) }
                cmd_id::LABEL => { match self.try_read_braced_text() { Some(l) => Ok(Some(Node::Label(l))), None => Ok(None) } }
                cmd_id::REF => { match self.try_read_braced_text() { Some(l) => Ok(Some(Node::Ref(l))), None => Ok(None) } }
                cmd_id::CITE => {
                    let opt1 = self.try_read_optional_arg();
                    let opt2 = self.try_read_optional_arg();
                    let k = self.read_braced_text()?;
                    // \cite[opt]{key} or \cite[pre][post]{key} (natbib two-arg form)
                    let opt = match (opt1, opt2) {
                        (Some(pre), Some(post)) => Some(format!("{} {}", pre, post)),
                        (Some(single), None) => Some(single),
                        _ => None,
                    };
                    Ok(Some(Node::Citation(k, opt, CitationStyle::Numeric)))
                }
                cmd_id::BIBITEM => { let _opt = self.try_read_optional_arg(); let k = self.read_braced_text()?; Ok(Some(Node::BibItem(k))) }
                cmd_id::FOOTNOTE => { let n = self.read_braced_nodes()?; Ok(Some(Node::Footnote(n))) }
                cmd_id::VSPACE => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::VSpace(pts))) }
                cmd_id::HSPACE => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::HSpace(pts))) }
                cmd_id::HREF => { let url = self.read_braced_text()?; let content = self.read_braced_nodes()?; Ok(Some(Node::Href { url, content })) }
                cmd_id::URL => { let url = self.read_braced_text()?; Ok(Some(Node::Url { url, clickable: true })) }
                cmd_id::TEXTCOLOR => { let cn = self.read_braced_text()?; let c = self.read_braced_nodes()?; let color = self.resolve_color(&cn).unwrap_or(Color::BLACK); Ok(Some(Node::Colored { color, content: c })) }
                cmd_id::COLOR => { let cn = self.read_braced_text()?; let color = self.resolve_color(&cn).unwrap_or(Color::BLACK); Ok(Some(Node::ColorDecl(color))) }
                cmd_id::CAPTION => {
                    let _opt = self.try_read_optional_arg(); // short caption
                    let content = self.read_braced_nodes()?;
                    // Outside float: render as styled paragraph
                    Ok(Some(Node::Paragraph(content)))
                }
                cmd_id::INCLUDEGRAPHICS => {
                    let opts = self.try_read_optional_arg();
                    let path = self.read_braced_text()?;
                    let mut width = None;
                    let mut height = None;
                    let mut scale = None;
                    let mut angle = None;
                    let mut keepaspectratio = false;
                    let mut trim = None;
                    let mut clip = false;
                    let mut page = None;
                    let mut viewport = None;
                    if let Some(opt_str) = opts {
                        for opt in opt_str.split(',') {
                            let opt = opt.trim();
                            // Boolean flags (no =)
                            if opt == "keepaspectratio" { keepaspectratio = true; continue; }
                            if opt == "clip" { clip = true; continue; }
                            let parts: Vec<&str> = opt.splitn(2, '=').collect();
                            if parts.len() == 2 {
                                let key = parts[0].trim();
                                let val = parts[1].trim();
                                match key {
                                    "width" => {
                                        if val.contains("\\textwidth") || val.contains("\\linewidth") || val.contains("\\columnwidth") {
                                            let factor: f32 = val.replace("\\textwidth", "").replace("\\linewidth", "").replace("\\columnwidth", "").trim().parse().unwrap_or(1.0);
                                            width = Some(factor * 468.0);
                                        } else {
                                            width = self.parse_dimension(val);
                                        }
                                    }
                                    "height" => height = self.parse_dimension(val),
                                    "scale" => scale = val.parse().ok(),
                                    "angle" => angle = val.parse().ok(),
                                    "keepaspectratio" => keepaspectratio = val != "false",
                                    "clip" => clip = val != "false",
                                    "page" => page = val.parse().ok(),
                                    "trim" => {
                                        // trim=left bottom right top (space-separated dimensions)
                                        let dims: Vec<f32> = val.split_whitespace()
                                            .filter_map(|d| self.parse_dimension(d))
                                            .collect();
                                        if dims.len() == 4 {
                                            trim = Some((dims[0], dims[1], dims[2], dims[3]));
                                        }
                                    }
                                    "viewport" => {
                                        let dims: Vec<f32> = val.split_whitespace()
                                            .filter_map(|d| self.parse_dimension(d))
                                            .collect();
                                        if dims.len() == 4 {
                                            viewport = Some((dims[0], dims[1], dims[2], dims[3]));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Ok(Some(Node::Image(Box::new(ImageData {
                        path, width, height, scale, angle, keepaspectratio,
                        trim, clip, page, viewport,
                    }))))
                }
                cmd_id::SECTION => self.parse_section(SectionLevel::Section, true),
                cmd_id::SECTION_STAR => self.parse_section(SectionLevel::Section, false),
                cmd_id::SUBSECTION => self.parse_section(SectionLevel::Subsection, true),
                cmd_id::SUBSECTION_STAR => self.parse_section(SectionLevel::Subsection, false),
                cmd_id::SUBSUBSECTION => self.parse_section(SectionLevel::Subsubsection, true),
                cmd_id::CHAPTER => self.parse_section(SectionLevel::Chapter, true),
                cmd_id::BEGIN => self.parse_begin_environment(),
                cmd_id::END => {
                    // Stray \end without matching \begin - skip
                    self.skip_command_args();
                    Ok(None)
                }
                cmd_id::USEPACKAGE | cmd_id::DOCUMENTCLASS | cmd_id::ITEM => {
                    self.skip_command_args();
                    Ok(None)
                }
                cmd_id::TITLE => {
                    let t = self.read_braced_text()?;
                    self.body_title = Some(t);
                    Ok(None)
                }
                cmd_id::AUTHOR => {
                    let a = self.read_braced_text()?;
                    self.body_authors.push(a);
                    Ok(None)
                }
                cmd_id::DATE => {
                    let d = self.read_braced_text()?;
                    self.body_date = Some(d);
                    Ok(None)
                }
                _ => { self.skip_command_args(); Ok(None) },
            };
        }

        // Slow path: string comparison for commands without cmd_id
        let cmd = self.current_text();
        let starred = cmd.ends_with('*');
        self.advance();

        match cmd {
            // Sectioning (variants not in cmd_id)
            "\\part" | "\\part*" => self.parse_section(SectionLevel::Part, !starred),
            "\\chapter*" => self.parse_section(SectionLevel::Chapter, false),
            "\\subsubsection*" => self.parse_section(SectionLevel::Subsubsection, false),
            "\\paragraph" | "\\paragraph*" => self.parse_section(SectionLevel::Paragraph, !starred),
            "\\subparagraph" | "\\subparagraph*" => self.parse_section(SectionLevel::Subparagraph, !starred),

            // Font styles (rare variants)
            "\\textsc" => { let n = self.read_braced_nodes()?; Ok(Some(Node::SmallCaps(n))) }
            "\\underline" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Underline(n))) }
            "\\sout" | "\\st" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Strikethrough(n))) }
            "\\textrm" | "\\textnormal" | "\\textup" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Group(n))) }
            "\\textsf" => { let n = self.read_braced_nodes()?; Ok(Some(Node::SansSerif(n))) }
            "\\textsl" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Italic(n))) }

            // Style switches — change font for subsequent text in scope
            "\\bf" | "\\bfseries" => Ok(Some(Node::FontStyleDecl(FontDeclType::Bold))),
            "\\it" | "\\itshape" | "\\sl" | "\\slshape" => Ok(Some(Node::FontStyleDecl(FontDeclType::Italic))),
            "\\tt" | "\\ttfamily" => Ok(Some(Node::FontStyleDecl(FontDeclType::Monospace))),
            "\\rm" | "\\rmfamily" | "\\normalfont" | "\\upshape" => Ok(Some(Node::FontStyleDecl(FontDeclType::Regular))),
            "\\sf" | "\\sffamily" => Ok(Some(Node::FontStyleDecl(FontDeclType::SansSerif))),
            "\\sc" | "\\scshape" => Ok(Some(Node::FontStyleDecl(FontDeclType::SmallCaps))),

            // Font sizes
            "\\tiny" => Ok(Some(Node::FontSize { size: FontSizeSpec::Tiny, content: vec![] })),
            "\\scriptsize" => Ok(Some(Node::FontSize { size: FontSizeSpec::Scriptsize, content: vec![] })),
            "\\footnotesize" => Ok(Some(Node::FontSize { size: FontSizeSpec::Footnotesize, content: vec![] })),
            "\\small" => Ok(Some(Node::FontSize { size: FontSizeSpec::Small, content: vec![] })),
            "\\normalsize" => Ok(Some(Node::FontSize { size: FontSizeSpec::Normalsize, content: vec![] })),
            "\\large" => Ok(Some(Node::FontSize { size: FontSizeSpec::Large, content: vec![] })),
            "\\Large" => Ok(Some(Node::FontSize { size: FontSizeSpec::LargeX, content: vec![] })),
            "\\LARGE" => Ok(Some(Node::FontSize { size: FontSizeSpec::LargeXX, content: vec![] })),
            "\\huge" => Ok(Some(Node::FontSize { size: FontSizeSpec::Huge, content: vec![] })),
            "\\Huge" => Ok(Some(Node::FontSize { size: FontSizeSpec::HugeX, content: vec![] })),

            // siunitx package commands
            "\\SI" | "\\qty" => {
                // \SI{value}{unit} or \qty{value}{unit}
                let _ = self.try_read_optional_arg(); // optional [options]
                let value = self.read_braced_text()?;
                let unit = self.read_braced_text()?;
                let unit_text = self.expand_si_unit(&unit);
                Ok(Some(Node::InlineMath(vec![
                    MathNode::Text(value),
                    MathNode::Space(2.0),
                    MathNode::Text(unit_text),
                ])))
            }
            "\\si" | "\\unit" => {
                // \si{unit} or \unit{unit}
                let _ = self.try_read_optional_arg();
                let unit = self.read_braced_text()?;
                let unit_text = self.expand_si_unit(&unit);
                Ok(Some(Node::Text(unit_text)))
            }
            "\\num" => {
                // \num{number} — format number with thin-space digit grouping
                let _ = self.try_read_optional_arg();
                let num = self.read_braced_text()?;
                if let Some(exp_idx) = num.find('e').or_else(|| num.find('E')) {
                    let mantissa = &num[..exp_idx];
                    let exponent = &num[exp_idx+1..];
                    let formatted = Self::format_num_grouping(mantissa);
                    // Render as mantissa × 10^exponent
                    Ok(Some(Node::InlineMath(vec![
                        MathNode::Number(formatted),
                        MathNode::Space(2.0),
                        MathNode::Symbol("\u{00D7}".to_string()), // ×
                        MathNode::Space(2.0),
                        MathNode::Number("10".to_string()),
                        MathNode::Super(vec![MathNode::Number(exponent.to_string())]),
                    ])))
                } else {
                    let formatted = Self::format_num_grouping(&num);
                    Ok(Some(Node::Text(formatted)))
                }
            }
            "\\ang" => {
                // \ang{degrees;minutes;seconds} or \ang{degrees}
                let _ = self.try_read_optional_arg();
                let arg = self.read_braced_text()?;
                let parts: Vec<&str> = arg.split(';').collect();
                let mut result = String::new();
                if let Some(deg) = parts.first() {
                    let d = deg.trim();
                    if !d.is_empty() { result.push_str(d); }
                    result.push('\u{00B0}'); // °
                }
                if let Some(min) = parts.get(1) {
                    let m = min.trim();
                    if !m.is_empty() { result.push_str(m); }
                    result.push('\u{2032}'); // ′
                }
                if let Some(sec) = parts.get(2) {
                    let s = sec.trim();
                    if !s.is_empty() { result.push_str(s); }
                    result.push('\u{2033}'); // ″
                }
                Ok(Some(Node::Text(result)))
            }

            // Spacing (starred variants fall through here)
            "\\hspace*" => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::HSpace(pts))) }
            "\\vspace*" => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::VSpace(pts))) }
            "\\quad" => Ok(Some(Node::HSpace(self.base_font_size))),          // 1em
            "\\qquad" => Ok(Some(Node::HSpace(self.base_font_size * 2.0))),    // 2em
            "\\enspace" => Ok(Some(Node::HSpace(self.base_font_size * 0.5))),  // 0.5em
            "\\thinspace" | "\\," => Ok(Some(Node::HSpace(self.base_font_size / 6.0))),  // 1/6 em (3mu)
            "\\;" => Ok(Some(Node::HSpace(self.base_font_size * 5.0 / 18.0))), // 5/18 em (5mu)
            "\\:" => Ok(Some(Node::HSpace(self.base_font_size * 4.0 / 18.0))), // 4/18 em (4mu)
            "\\!" => Ok(Some(Node::HSpace(-self.base_font_size / 6.0))),       // -1/6 em (-3mu)
            "\\ " => Ok(Some(Node::Text(" ".to_string()))), // explicit inter-word space
            "\\hfill" | "\\dotfill" | "\\hrulefill" => Ok(Some(Node::HFill)),
            "\\vfill" => Ok(Some(Node::VFill)),
            "\\phantom" | "\\hphantom" => {
                // Invisible space measured from content text using font metrics
                let text = self.read_braced_text()?;
                let w = crate::font::measure_text(text.trim(), crate::font::FontId::TimesRoman, self.base_font_size);
                Ok(Some(Node::HSpace(w)))
            }
            "\\vphantom" => {
                let _content = self.read_braced_nodes()?;
                Ok(None) // vertical phantom — no horizontal space, just affects line height
            }
            "\\smash" => {
                let _opt = self.try_read_optional_arg(); // optional [t] or [b]
                let content = self.read_braced_nodes()?;
                // Render content with zero height contribution
                Ok(Some(Node::Group(content)))
            }
            "\\rlap" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content))) // zero-width overlay right
            }
            "\\llap" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content))) // zero-width overlay left
            }
            "\\fbox" | "\\framebox" => {
                let content = self.read_braced_nodes()?;
                // Render as a colored box with thin frame
                Ok(Some(Node::ColorBox(Box::new(ColorBoxData {
                    content,
                    title: None,
                    bg_color: Color::WHITE,
                    frame_color: Color::BLACK,
                    corner_radius: 0.0,
                    rule_width: 0.4,
                    padding: 3.0,
                }))))
            }
            "\\centerline" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Center(content)))
            }
            "\\mbox" | "\\hbox" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::MBox(content)))
            }
            "\\makebox" => {
                // \makebox[width][pos]{content} — skip optional args
                self.try_read_optional_arg();
                self.try_read_optional_arg();
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }
            "\\hyperref" => {
                // \hyperref[label]{text}
                let label = self.try_read_optional_arg().unwrap_or_default();
                let content = self.read_braced_nodes()?;
                // Render as href with internal link (# prefix)
                Ok(Some(Node::Href { url: format!("#{}", label), content }))
            }
            "\\smallskip" | "\\smallbreak" => Ok(Some(Node::VSpace(self.base_font_size * 0.3))),  // ~3pt at 10pt
            "\\medskip" | "\\medbreak" => Ok(Some(Node::VSpace(self.base_font_size * 0.6))),    // ~6pt at 10pt
            "\\bigskip" | "\\bigbreak" => Ok(Some(Node::VSpace(self.base_font_size * 1.2))),    // ~12pt at 10pt

            // Breaks
            "\\newline" | "\\linebreak" => Ok(Some(Node::LineBreak)),
            "\\clearpage" | "\\cleardoublepage" => Ok(Some(Node::ClearPage)),
            "\\pagebreak" | "\\newpage" => Ok(Some(Node::PageBreak)),
            "\\nopagebreak" | "\\nolinebreak" => { self.try_read_optional_arg(); Ok(None) }
            "\\iffalse" => {
                // Skip everything until \fi
                loop {
                    if self.current().kind == TokenKind::Eof { break; }
                    if self.current().kind == TokenKind::Command && self.current().text(self.source) == "\\fi" {
                        self.advance();
                        break;
                    }
                    self.advance();
                }
                Ok(None)
            }

            // TeX dimension primitives
            "\\vskip" => {
                // \vskip 10pt — read dimension without braces
                let dim_text = self.read_tex_dimension_text();
                let pts = self.parse_dimension(&dim_text).unwrap_or(10.0);
                Ok(Some(Node::VSpace(pts)))
            }
            "\\hskip" => {
                let dim_text = self.read_tex_dimension_text();
                let pts = self.parse_dimension(&dim_text).unwrap_or(10.0);
                Ok(Some(Node::HSpace(pts)))
            }
            "\\kern" | "\\mkern" => {
                let (pts, remainder) = self.read_dimension_from_source();
                if let Some(rem) = remainder {
                    // Remainder text after the dimension — emit kern + text as a group
                    Ok(Some(Node::Group(vec![
                        Node::HSpace(pts),
                        Node::Text(rem),
                    ])))
                } else {
                    Ok(Some(Node::HSpace(pts)))
                }
            }
            "\\appendix" => Ok(Some(Node::Appendix)),
            "\\indent" => Ok(Some(Node::HSpace(self.base_font_size * 1.5))), // ~1.5em paragraph indent
            "\\marginpar" | "\\marginnote" => {
                let _opt = self.try_read_optional_arg(); // optional left-margin text
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::MarginNote(content)))
            }

            // Rules
            "\\hrule" => {
                // \hrule may have TeX primitive keyword args: height, width, depth
                loop {
                    self.skip_whitespace_and_comments();
                    if self.current().kind == TokenKind::Text {
                        let t = self.current().text(self.source).trim_start();
                        if t.starts_with("height") || t.starts_with("width") || t.starts_with("depth") {
                            self.advance(); // skip keyword token
                            let _ = self.read_tex_dimension_text(); // consume dimension
                            continue;
                        }
                    }
                    break;
                }
                Ok(Some(Node::HRule))
            }
            "\\rule" => {
                let _raise = self.try_read_optional_arg(); // optional raise
                let width_str = self.read_braced_text().unwrap_or_default();
                let height_str = self.read_braced_text().unwrap_or_default();
                let w = self.parse_dimension(&width_str).unwrap_or(0.0);
                let h = self.parse_dimension(&height_str).unwrap_or(0.4);
                if w == 0.0 {
                    // \rule{0pt}{height} is a strut (invisible height spacer)
                    Ok(Some(Node::VSpace(h)))
                } else {
                    // Inline filled rectangle with specific dimensions
                    Ok(Some(Node::Rule { width: w, height: h }))
                }
            }

            // Special characters
            "\\LaTeX" => Ok(Some(Node::LaTeXLogo)),
            "\\TeX" => Ok(Some(Node::TeXLogo)),
            "\\ldots" | "\\dots" | "\\textellipsis" => Ok(Some(Node::Ellipsis)),
            "\\textendash" => Ok(Some(Node::EnDash)),
            "\\textemdash" => Ok(Some(Node::EmDash)),
            "\\textquoteleft" => Ok(Some(Node::LeftQuote)),
            "\\textquoteright" => Ok(Some(Node::RightQuote)),
            "\\textquotedblleft" => Ok(Some(Node::LeftDoubleQuote)),
            "\\textquotedblright" => Ok(Some(Node::RightDoubleQuote)),

            // csquotes package commands
            "\\enquote" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Enquote(content, false)))
            }
            "\\enquote*" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Enquote(content, true)))
            }
            "\\textquote" => {
                let _ = self.try_read_optional_arg(); // optional cite arg
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Enquote(content, false)))
            }
            "\\textquote*" => {
                let _ = self.try_read_optional_arg();
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Enquote(content, true)))
            }
            "\\blockquote" => {
                let _ = self.try_read_optional_arg(); // optional cite arg
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::BlockQuote(content)))
            }

            "\\copyright" | "\\textcopyright" => Ok(Some(Node::Copyright)),
            "\\textregistered" => Ok(Some(Node::Registered)),
            "\\texttrademark" => Ok(Some(Node::Trademark)),

            // textcomp symbols
            "\\textdegree" => Ok(Some(Node::Text("\u{00B0}".to_string()))),       // °
            "\\textcelsius" => Ok(Some(Node::Text("\u{2103}".to_string()))),      // ℃
            "\\textmu" => Ok(Some(Node::Text("\u{00B5}".to_string()))),           // µ
            "\\textohm" => Ok(Some(Node::Text("\u{2126}".to_string()))),          // Ω
            "\\texteuro" => Ok(Some(Node::Text("\u{20AC}".to_string()))),         // €
            "\\textyen" => Ok(Some(Node::Text("\u{00A5}".to_string()))),          // ¥
            "\\textsterling" | "\\pounds" => Ok(Some(Node::Text("\u{00A3}".to_string()))), // £
            "\\textcent" => Ok(Some(Node::Text("\u{00A2}".to_string()))),         // ¢
            "\\textbullet" => Ok(Some(Node::Text("\u{2022}".to_string()))),       // •
            "\\textperiodcentered" => Ok(Some(Node::Text("\u{00B7}".to_string()))), // ·
            "\\textlangle" => Ok(Some(Node::Text("\u{27E8}".to_string()))),       // ⟨
            "\\textrangle" => Ok(Some(Node::Text("\u{27E9}".to_string()))),       // ⟩
            "\\textsection" => Ok(Some(Node::Text("\u{00A7}".to_string()))),      // §
            "\\textparagraph" => Ok(Some(Node::Text("\u{00B6}".to_string()))),    // ¶
            "\\textdagger" => Ok(Some(Node::Text("\u{2020}".to_string()))),       // †
            "\\textdaggerdbl" => Ok(Some(Node::Text("\u{2021}".to_string()))),    // ‡
            "\\checkmark" => Ok(Some(Node::Text("\u{2713}".to_string()))),        // ✓
            "\\maltese" => Ok(Some(Node::Text("\u{2720}".to_string()))),          // ✠
            "\\textquotedbl" => Ok(Some(Node::Text("\"".to_string()))),

            // URL commands
            "\\nolinkurl" => { let url = self.read_braced_text()?; Ok(Some(Node::Url { url, clickable: false })) }

            // xspace — smart spacing after macros
            "\\xspace" => {
                self.skip_whitespace_and_comments();
                let next = self.current();
                let is_punct = if next.kind == TokenKind::Text {
                    let t = next.text(self.source);
                    matches!(t.as_bytes().first(), Some(b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'\'' | b'-'))
                } else {
                    false
                };
                if is_punct { Ok(None) } else { Ok(Some(Node::Text(" ".to_string()))) }
            }
            "\\&" | "\\amp" => Ok(Some(Node::Ampersand)),
            "\\%" => Ok(Some(Node::Percent)),
            "\\$" => Ok(Some(Node::Dollar)),
            "\\#" => Ok(Some(Node::Hash)),
            "\\_" => Ok(Some(Node::Underscore)),
            "\\{" => Ok(Some(Node::LeftBrace)),
            "\\}" => Ok(Some(Node::RightBrace)),
            "\\~" => {
                let c = self.read_accent_char();
                match c.as_str() {
                    "a" => Ok(Some(Node::Text("\u{00E3}".to_string()))), // ã
                    "o" => Ok(Some(Node::Text("\u{00F5}".to_string()))), // õ
                    "n" => Ok(Some(Node::Text("\u{00F1}".to_string()))), // ñ
                    "A" => Ok(Some(Node::Text("\u{00C3}".to_string()))), // Ã
                    "O" => Ok(Some(Node::Text("\u{00D5}".to_string()))), // Õ
                    "N" => Ok(Some(Node::Text("\u{00D1}".to_string()))), // Ñ
                    "" => Ok(Some(Node::Tilde)),
                    _ => Ok(Some(Node::Text(c))),
                }
            }
            "\\^" => {
                let c = self.read_accent_char();
                match c.as_str() {
                    "a" => Ok(Some(Node::Text("\u{00E2}".to_string()))), // â
                    "e" => Ok(Some(Node::Text("\u{00EA}".to_string()))), // ê
                    "i" => Ok(Some(Node::Text("\u{00EE}".to_string()))), // î
                    "o" => Ok(Some(Node::Text("\u{00F4}".to_string()))), // ô
                    "u" => Ok(Some(Node::Text("\u{00FB}".to_string()))), // û
                    "A" => Ok(Some(Node::Text("\u{00C2}".to_string()))), // Â
                    "E" => Ok(Some(Node::Text("\u{00CA}".to_string()))), // Ê
                    "I" => Ok(Some(Node::Text("\u{00CE}".to_string()))), // Î
                    "O" => Ok(Some(Node::Text("\u{00D4}".to_string()))), // Ô
                    "U" => Ok(Some(Node::Text("\u{00DB}".to_string()))), // Û
                    "" => Ok(Some(Node::Caret)),
                    _ => Ok(Some(Node::Text(c))),
                }
            }
            // \[...\] display math
            "\\[" => {
                let math = self.parse_math_until_close_bracket()?;
                return Ok(Some(Node::DisplayMath(Box::new(DisplayMathData {
                    nodes: math,
                    numbered: false,
                    env_type: MathEnvType::DollarDollar,
                }))));
            }
            "\\]" => {
                // Stray \] without matching \[, skip
                return Ok(None);
            }

            "\\\\" => {
                // \\ is a line break in body mode; optional [dim] for extra spacing
                self.try_read_optional_arg();
                Ok(Some(Node::LineBreak))
            }
            "\\textbackslash" => Ok(Some(Node::Backslash)),
            "\\S" => Ok(Some(Node::Text("\u{00A7}".to_string()))),
            "\\P" => Ok(Some(Node::Text("\u{00B6}".to_string()))),
            "\\dag" => Ok(Some(Node::Text("\u{2020}".to_string()))),
            "\\ddag" => Ok(Some(Node::Text("\u{2021}".to_string()))),
            "\\o" => Ok(Some(Node::Text("\u{00F8}".to_string()))),  // ø
            "\\O" => Ok(Some(Node::Text("\u{00D8}".to_string()))),  // Ø
            "\\i" => Ok(Some(Node::Text("\u{0131}".to_string()))),  // ı (dotless i)
            "\\j" => Ok(Some(Node::Text("\u{0237}".to_string()))),  // ȷ (dotless j)
            "\\aa" => Ok(Some(Node::Text("\u{00E5}".to_string()))), // å
            "\\AA" => Ok(Some(Node::Text("\u{00C5}".to_string()))), // Å
            "\\ae" => Ok(Some(Node::Text("\u{00E6}".to_string()))), // æ
            "\\AE" => Ok(Some(Node::Text("\u{00C6}".to_string()))), // Æ
            "\\oe" => Ok(Some(Node::Text("\u{0153}".to_string()))), // œ
            "\\OE" => Ok(Some(Node::Text("\u{0152}".to_string()))), // Œ
            "\\ss" => Ok(Some(Node::Text("\u{00DF}".to_string()))), // ß
            "\\l" => Ok(Some(Node::Text("\u{0142}".to_string()))),  // ł
            "\\L" => Ok(Some(Node::Text("\u{0141}".to_string()))),  // Ł

            // Accented characters
            "\\\'" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "a" => "\u{00E1}", "e" => "\u{00E9}", "i" => "\u{00ED}",
                    "o" => "\u{00F3}", "u" => "\u{00FA}",
                    "A" => "\u{00C1}", "E" => "\u{00C9}", "I" => "\u{00CD}",
                    "O" => "\u{00D3}", "U" => "\u{00DA}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\`" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "a" => "\u{00E0}", "e" => "\u{00E8}", "i" => "\u{00EC}",
                    "o" => "\u{00F2}", "u" => "\u{00F9}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\\"" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "a" => "\u{00E4}", "e" => "\u{00EB}", "i" => "\u{00EF}",
                    "o" => "\u{00F6}", "u" => "\u{00FC}",
                    "A" => "\u{00C4}", "O" => "\u{00D6}", "U" => "\u{00DC}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }

            "\\c" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "c" => "\u{00E7}", "C" => "\u{00C7}", "s" => "\u{015F}", "S" => "\u{015E}",
                    "t" => "\u{0163}", "T" => "\u{0162}", "n" => "\u{0146}", "N" => "\u{0145}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\v" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "c" => "\u{010D}", "C" => "\u{010C}", "s" => "\u{0161}", "S" => "\u{0160}",
                    "z" => "\u{017E}", "Z" => "\u{017D}", "r" => "\u{0159}", "R" => "\u{0158}",
                    "n" => "\u{0148}", "N" => "\u{0147}", "e" => "\u{011B}", "E" => "\u{011A}",
                    "d" => "\u{010F}", "D" => "\u{010E}", "t" => "\u{0165}", "T" => "\u{0164}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\H" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "o" => "\u{0151}", "O" => "\u{0150}", "u" => "\u{0171}", "U" => "\u{0170}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\k" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "a" => "\u{0105}", "A" => "\u{0104}", "e" => "\u{0119}", "E" => "\u{0118}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\u" => {
                let c = self.read_accent_char();
                let accented = match c.as_str() {
                    "a" => "\u{0103}", "A" => "\u{0102}", "g" => "\u{011F}", "G" => "\u{011E}",
                    "i" => "\u{012D}", "I" => "\u{012C}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }

            // Colors (rare variants)
            "\\colorbox" => {
                let bg_name = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                let bg_color = self.resolve_color(&bg_name).unwrap_or(Color::WHITE);
                Ok(Some(Node::ColorBox(Box::new(ColorBoxData {
                    content,
                    title: None,
                    bg_color,
                    frame_color: bg_color,
                    corner_radius: 0.0,
                    rule_width: 0.0,
                    padding: 3.0,
                }))))
            }

            "\\captionof" => {
                // \captionof{figure/table}{caption text}
                let _type_name = self.read_braced_text()?;
                let _opt = self.try_read_optional_arg(); // short caption
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Center(content)))
            }

            // Box/layout commands
            "\\resizebox" | "\\resizebox*" => {
                // \resizebox{width}{height}{content} — skip width/height, return content
                let _width = self.read_braced_text()?;
                let _height = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }
            "\\scalebox" => {
                let _scale = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }
            "\\rotatebox" => {
                self.try_read_optional_arg(); // optional [origin]
                let _angle = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                // Rotation not supported in text-mode PDF; render content normally
                Ok(Some(Node::Group(content)))
            }
            "\\shortstack" => {
                // \shortstack[align]{line1\\line2\\line3}
                let _align = self.try_read_optional_arg();
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }
            "\\vbox" => {
                self.try_read_optional_arg();
                self.try_read_optional_arg();
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }
            "\\parbox" => {
                self.try_read_optional_arg(); // [pos]
                let width_str = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                let width = self.parse_dimension_with_textwidth(&width_str, 468.0); // default text width
                Ok(Some(Node::Minipage { width, content }))
            }
            "\\raisebox" => {
                let _raise = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }

            // Cross-references (variants not in cmd_id)
            // Use try_read_braced_text so incomplete \ref{} during live editing doesn't crash
            "\\eqref" => {
                match self.try_read_braced_text() { Some(l) => Ok(Some(Node::EqRef(l))), None => Ok(None) }
            }
            "\\cref" => {
                match self.try_read_braced_text() { Some(l) => Ok(Some(Node::Cref(l, false))), None => Ok(None) }
            }
            "\\Cref" => {
                match self.try_read_braced_text() { Some(l) => Ok(Some(Node::Cref(l, true))), None => Ok(None) }
            }
            "\\crefrange" => {
                let label1 = self.read_braced_text().unwrap_or_default();
                let label2 = self.read_braced_text().unwrap_or_default();
                Ok(Some(Node::CrefRange(label1, label2, false)))
            }
            "\\Crefrange" => {
                let label1 = self.read_braced_text().unwrap_or_default();
                let label2 = self.read_braced_text().unwrap_or_default();
                Ok(Some(Node::CrefRange(label1, label2, true)))
            }
            "\\labelcref" => {
                match self.try_read_braced_text() { Some(l) => Ok(Some(Node::LabelCref(l))), None => Ok(None) }
            }
            "\\pageref" | "\\autoref" | "\\nameref" => {
                match self.try_read_braced_text() { Some(l) => Ok(Some(Node::Ref(l))), None => Ok(None) }
            }
            "\\citep" | "\\Citep" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key, opt, CitationStyle::Parenthetical)))
            }
            "\\citet" | "\\Citet" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key, opt, CitationStyle::Textual)))
            }
            "\\citeauthor" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key, opt, CitationStyle::AuthorOnly)))
            }
            "\\citeyear" | "\\citeyearpar" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key, opt, CitationStyle::YearOnly)))
            }
            "\\citealp" | "\\citealt" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key, opt, CitationStyle::AltNoParen)))
            }

            // BibLaTeX citation commands
            "\\textcite" | "\\Textcite" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::TextCite)))
            }
            "\\parencite" | "\\Parencite" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::ParenCite)))
            }
            "\\autocite" | "\\Autocite" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::AutoCite)))
            }
            "\\Citeauthor" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::CiteAuthor)))
            }
            "\\citetitle" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::CiteTitle)))
            }
            "\\fullcite" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::FullCite)))
            }
            "\\footcite" => {
                let opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::BiblatexCitation(key, opt, BiblatexCiteType::FootCite)))
            }

            // Letter class commands
            "\\opening" => {
                let text = self.read_braced_text()?;
                Ok(Some(Node::LetterOpening(text)))
            }
            "\\closing" => {
                let text = self.read_braced_text()?;
                Ok(Some(Node::LetterClosing(text)))
            }
            "\\cc" => {
                let text = self.read_braced_text()?;
                Ok(Some(Node::LetterCc(text)))
            }
            "\\encl" => {
                let text = self.read_braced_text()?;
                Ok(Some(Node::LetterEncl(text)))
            }
            "\\ps" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::LetterPs(content)))
            }
            "\\signature" => {
                let text = self.read_braced_text()?;
                self.body_signature = Some(text);
                Ok(None)
            }

            // Pifont \ding{n} — ZapfDingbats characters (byte code = ding number)
            "\\ding" => {
                let num_str = self.read_braced_text()?;
                if let Ok(code) = num_str.trim().parse::<u8>() {
                    Ok(Some(Node::Dingbat(code)))
                } else {
                    Ok(Some(Node::Text("?".to_string())))
                }
            }

            // Alignment declarations
            "\\raggedright" => Ok(Some(Node::AlignmentDecl(AlignmentMode::FlushLeft))),
            "\\raggedleft" => Ok(Some(Node::AlignmentDecl(AlignmentMode::FlushRight))),

            // \fontsize{size}{baselineskip}\selectfont — set font size
            "\\fontsize" => {
                let size_str = self.read_braced_text().unwrap_or_default();
                let _skip_str = self.read_braced_text().unwrap_or_default();
                if let Some(pts) = self.parse_dimension(&size_str) {
                    return Ok(Some(Node::FontSize { size: FontSizeSpec::Points(pts), content: vec![] }));
                }
                Ok(None)
            }

            // No-ops
            "\\nobreak" | "\\allowbreak" | "\\relax" | "\\protect"
            | "\\sloppy" | "\\fussy"
            | "\\selectfont" | "\\frenchspacing"
            | "\\nonfrenchspacing" | "\\newblock"
            | "\\/" | "\\ignorespaces" | "\\leavevmode"
            | "\\unskip" | "\\null" | "\\-" => Ok(None),
            "\\strut" => {
                // Zero-width box with full font height — ensures consistent line/row height
                Ok(Some(Node::Rule { width: 0.0, height: 0.0 }))
            }

            // \input and \include are resolved during pre-processing (main.rs)
            // If they reach the parser, just consume the argument
            "\\input" | "\\include" => { let _file = self.read_braced_text()?; Ok(None) }
            "\\pagestyle" | "\\thispagestyle" => { let _style = self.read_braced_text()?; Ok(None) }
            "\\setlength" | "\\addtolength" => {
                let name = self.read_braced_text().unwrap_or_default();
                let val = self.read_braced_text().unwrap_or_default();
                let len_name = name.trim_start_matches('\\');
                if let Some(pts) = self.parse_dimension(&val) {
                    match len_name {
                        "parindent" => return Ok(Some(Node::SetParIndent(pts))),
                        "parskip" => return Ok(Some(Node::SetParSkip(pts))),
                        "baselineskip" => return Ok(Some(Node::SetBaselineSkip(pts))),
                        _ => {}
                    }
                }
                Ok(None)
            }
            "\\setcounter" => {
                let name = self.read_braced_text()?;
                let val_str = self.read_braced_text()?;
                let val: i32 = val_str.trim().parse().unwrap_or(0);
                Ok(Some(Node::SetCounter(name, val)))
            }
            "\\addtocounter" => {
                let name = self.read_braced_text()?;
                let val_str = self.read_braced_text()?;
                let val: i32 = val_str.trim().parse().unwrap_or(0);
                // addtocounter uses negative value trick: -1 means relative add
                // We'll emit as SetCounter with a sentinel offset
                Ok(Some(Node::SetCounter(format!("add:{}", name), val)))
            }
            "\\stepcounter" => {
                let name = self.read_braced_text()?;
                Ok(Some(Node::StepCounter(name)))
            }
            "\\refstepcounter" => {
                let name = self.read_braced_text()?;
                Ok(Some(Node::RefStepCounter(name)))
            }
            "\\newcounter" => {
                let name = self.read_braced_text()?;
                let parent = self.try_read_optional_arg();
                Ok(Some(Node::NewCounter { name, parent }))
            }
            "\\numberwithin" => {
                let child = self.read_braced_text()?;
                let parent = self.read_braced_text()?;
                Ok(Some(Node::NumberWithin { child, parent }))
            }
            "\\definecolor" => {
                let name = self.read_braced_text()?;
                let model = self.read_braced_text()?;
                let spec = self.read_braced_text()?;
                if let Some(color) = parse_color_model(&model, &spec) {
                    self.custom_colors.insert(name.clone(), color);
                    Ok(Some(Node::DefineColor { name, color }))
                } else {
                    Ok(None)
                }
            }
            // Explicit paragraph break — acts like a blank line in LaTeX
            "\\par" => Ok(Some(Node::VSpace(0.0))),

            // Text special characters
            "\\textgreater" => Ok(Some(Node::Text(">".to_string()))),
            "\\textless" => Ok(Some(Node::Text("<".to_string()))),
            "\\textasciitilde" => Ok(Some(Node::Text("~".to_string()))),
            "\\textasciigrave" => Ok(Some(Node::Text("`".to_string()))),
            "\\textbar" => Ok(Some(Node::Text("|".to_string()))),
            "\\textbullet" => Ok(Some(Node::Text("\u{2022}".to_string()))),
            "\\textsection" => Ok(Some(Node::Text("\u{00A7}".to_string()))),
            "\\textdagger" => Ok(Some(Node::Text("\u{2020}".to_string()))),
            "\\textdaggerdbl" => Ok(Some(Node::Text("\u{2021}".to_string()))),
            "\\textparagraph" => Ok(Some(Node::Text("\u{00B6}".to_string()))),
            "\\textsterling" => Ok(Some(Node::Text("\u{00A3}".to_string()))),
            "\\checkmark" | "\\cmark" => Ok(Some(Node::Dingbat(0x33))), // ✓ ZapfDingbats checkmark
            "\\xmark" => Ok(Some(Node::Dingbat(0x37))), // ✗ ZapfDingbats ballot X

            // Text superscript/subscript
            "\\textsuperscript" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Superscript(content)))
            }
            "\\textsubscript" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Subscript(content)))
            }

            // \fcolorbox{frame_color}{bg_color}{content}
            "\\fcolorbox" => {
                let frame_name = self.read_braced_text()?;
                let bg_name = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                let frame_color = self.resolve_color(&frame_name).unwrap_or(Color::BLACK);
                let bg_color = self.resolve_color(&bg_name).unwrap_or(Color::WHITE);
                Ok(Some(Node::ColorBox(Box::new(ColorBoxData {
                    content,
                    title: None,
                    bg_color,
                    frame_color,
                    corner_radius: 0.0,
                    rule_width: 0.6,
                    padding: 3.0,
                }))))
            }

            // \adjustbox{options}{content} — render content, skip options
            "\\adjustbox" => {
                let _opts = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }

            // \pagenumbering{style} — stored for layout
            "\\pagenumbering" => {
                let style = self.read_braced_text()?;
                Ok(Some(Node::PageNumbering(style)))
            }

            // Index and glossary — silently skip arguments
            "\\index" | "\\glossary" | "\\nomenclature" => {
                self.skip_command_args();
                Ok(None)
            }

            // ICML/conference-specific commands — suppress output
            "\\icmlsetsymbol" | "\\icmlkeywords" | "\\icmltitlerunning"
            | "\\icmlaffiliation" | "\\icmlcorrespondingauthor" => {
                self.skip_command_args();
                Ok(None)
            }
            "\\icmlauthor" => {
                // \icmlauthor{Name}{affiliations} — skip in layout (handled by style)
                self.skip_command_args();
                Ok(None)
            }

            // Box save/restore commands
            "\\newsavebox" | "\\savebox" => {
                self.skip_command_args();
                Ok(None)
            }
            "\\usebox" => {
                // \usebox{\boxname} — we can't restore the saved content
                self.skip_command_args();
                Ok(None)
            }
            "\\sbox" => {
                self.skip_command_args();
                Ok(None)
            }

            // Scoping
            "\\begingroup" | "\\endgroup" | "\\bgroup" | "\\egroup" => Ok(None),

            // floatrow package
            "\\floatbox" => {
                // \floatbox[setup]{type}[width]{caption}{body}
                // Read optional setup, type, optional width, then caption and body as nodes
                let _ = self.try_read_optional_arg(); // [setup]
                let _ = self.read_braced_text(); // {type}
                let _ = self.try_read_optional_arg(); // [width]
                let caption_nodes = self.read_braced_nodes().unwrap_or_default();
                let body_nodes = self.read_braced_nodes().unwrap_or_default();
                // Combine body + caption as a group
                let mut combined = body_nodes;
                combined.extend(caption_nodes);
                Ok(Some(Node::Group(combined)))
            }
            "\\floatsetup" | "\\thisfloatsetup" | "\\capbeside" | "\\fcapside"
            | "\\ffigbox" | "\\ttabbox" => {
                self.skip_command_args();
                Ok(None)
            }

            // todonotes
            "\\todo" => {
                self.try_read_optional_arg(); // options
                let _text = self.read_braced_text()?;
                Ok(None)
            }
            "\\missingfigure" => {
                self.skip_command_args();
                Ok(None)
            }

            // Counter manipulation
            "\\stepcounter" | "\\refstepcounter" => {
                self.skip_command_args();
                Ok(None)
            }

            // Misc TeX primitives
            "\\global" | "\\long" | "\\protected" | "\\outer" => Ok(None),
            "\\newif" | "\\newtoks" | "\\newcount" | "\\newdimen" | "\\newskip" => {
                self.skip_command_args();
                Ok(None)
            }
            "\\ifdefined" | "\\ifx" | "\\ifnum" | "\\ifdim" | "\\ifcase" | "\\iftrue" => {
                // Skip to matching \fi — simple scanning
                self.skip_conditional();
                Ok(None)
            }

            // Print affiliations and notice (ICML)
            "\\printAffiliationsAndNotice" => {
                self.skip_command_args();
                Ok(None)
            }

            // List of figures/tables
            "\\listoffigures" => Ok(Some(Node::ListOfFigures)),
            "\\listoftables" => Ok(Some(Node::ListOfTables)),


            "\\allowdisplaybreaks" | "\\mathsurround" | "\\hfuzz" => { self.skip_command_args(); Ok(None) }
            "\\newcommand" | "\\newcommand*" | "\\renewcommand" | "\\renewcommand*"
            | "\\providecommand" | "\\providecommand*"
            | "\\DeclareRobustCommand" | "\\DeclareRobustCommand*"
            | "\\def" => { self.skip_command_args(); Ok(None) }
            "\\bibliography" | "\\addbibresource" => {
                let _bib_file = self.read_braced_text()?;
                // Bibliography loading happens outside the parser
                Ok(None)
            }
            "\\printbibliography" => {
                self.skip_command_args(); // skip optional [heading=...]
                Ok(Some(Node::PrintBibliography))
            }
            "\\nocite" => {
                self.skip_command_args();
                Ok(None)
            }
            "\\bibliographystyle" => { let _style = self.read_braced_text()?; Ok(None) }

            // AMS article class commands — store address/email for end-of-document rendering
            "\\address" => {
                let text = self.read_braced_text()?;
                self.body_addresses.push((expand_latex_accents(&text), None));
                // Also store as letter sender address (used by letter class layout)
                self.body_sender_address = Some(expand_latex_accents(&text));
                Ok(None)
            }
            "\\email" => {
                let text = self.read_braced_text()?;
                // Attach email to the most recent address
                if let Some(last) = self.body_addresses.last_mut() {
                    last.1 = Some(text);
                }
                Ok(None)
            }
            "\\thanks" => {
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Footnote(content)))
            }
            "\\footnotemark" => {
                let _opt = self.try_read_optional_arg(); // optional [n]
                // Render as superscript footnote number (simplified: just increment counter)
                Ok(Some(Node::Superscript(vec![Node::Text("*".to_string())])))
            }
            "\\footnotetext" => {
                let _opt = self.try_read_optional_arg(); // optional [n]
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Footnote(content)))
            }
            "\\dedicatory" | "\\urladdr" | "\\curraddr" => {
                self.skip_command_args(); Ok(None)
            }

            "\\keywords" => {
                let text = self.read_braced_text()?;
                self.body_keywords = Some(text);
                Ok(None)
            }
            "\\subjclass" => {
                let year = self.try_read_optional_arg().unwrap_or_else(|| "2020".to_string());
                let text = self.read_braced_text()?;
                self.body_subjclass = Some((year, text));
                Ok(None)
            }

            // \texorpdfstring{TeX text}{PDF string} — use first arg for display
            "\\texorpdfstring" => {
                let tex_content = self.read_braced_nodes()?;
                let _pdf_string = self.read_braced_text()?;
                // Return the TeX content for display
                if tex_content.len() == 1 {
                    Ok(Some(tex_content.into_iter().next().unwrap()))
                } else {
                    Ok(Some(Node::Group(tex_content)))
                }
            }

            // \ensuremath{...} — render as inline math
            "\\ensuremath" => {
                // Read the braced argument as math nodes
                if self.current().kind == TokenKind::OpenBrace {
                    self.advance(); // skip {
                    let mut nodes = Vec::new();
                    loop {
                        if self.current().kind == TokenKind::CloseBrace || self.current().kind == TokenKind::Eof {
                            break;
                        }
                        if let Some(node) = self.parse_math_node()? {
                            nodes.push(node);
                        }
                    }
                    if self.current().kind == TokenKind::CloseBrace {
                        self.advance();
                    }
                    Ok(Some(Node::InlineMath(nodes)))
                } else {
                    Ok(None)
                }
            }

            "\\mintinline" => {
                // \mintinline{lang}{code} or \mintinline[opts]{lang}{code}
                let _opt = self.try_read_optional_arg();
                let _lang = self.read_braced_text().ok();
                let code = self.read_braced_text().unwrap_or_default();
                Ok(Some(Node::Code(code)))
            }
            "\\inputminted" => {
                let _opt = self.try_read_optional_arg();
                let lang = self.read_braced_text().ok();
                let file = self.read_braced_text().unwrap_or_default();
                Ok(Some(Node::Listing(Box::new(ListingData {
                    code: format!("% inputminted: {}", file),
                    language: lang.map(|l| l.to_lowercase()),
                    caption: None, label: None,
                    numbers: ListingNumbers::None, frame: false,
                }))))
            }
            "\\lstset" => {
                let _ = self.read_braced_text(); // consume options, no effect for now
                Ok(None)
            }

            "\\verb" | "\\verb*" | "\\lstinline" => {
                // \verb|code| or \lstinline|code| or \lstinline{code}
                // Skip optional arg for \lstinline (e.g., \lstinline[style=foo])
                if cmd == "\\lstinline" {
                    let _ = self.try_read_optional_arg();
                }
                // Check if braces are used as delimiters (common for \lstinline)
                if self.current().kind == TokenKind::OpenBrace {
                    let content = self.read_braced_text()?;
                    return Ok(Some(Node::Code(content)));
                }
                // Otherwise use delimiter-based parsing like \verb
                let tok = self.tokens[self.pos.saturating_sub(1)]; // the command token
                let verb_end = tok.pos as usize + tok.len as usize;
                if verb_end < self.source.len() {
                    let delim = self.source.as_bytes()[verb_end];
                    // Find closing delimiter in source
                    if let Some(close_offset) = self.source[verb_end + 1..].find(delim as char) {
                        let content = &self.source[verb_end + 1..verb_end + 1 + close_offset];
                        let content_end = verb_end + 1 + close_offset + 1; // past closing delim
                        // Skip all tokens that fall within the verb content range
                        while self.current().kind != TokenKind::Eof {
                            let tp = self.current().pos as usize;
                            if tp >= content_end { break; }
                            self.advance();
                        }
                        Ok(Some(Node::Code(content.to_string())))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            // xy-pic: \xymatrix{A \ar[r] & B \\ C & D}
            "\\xymatrix" => {
                // Capture braced content as raw source for diagram rendering
                let raw = self.capture_braced_raw()?;
                Ok(Some(Node::Verbatim(format!("%%tikz:xymatrix%%\n\\xymatrix{{{}}}", raw))))
            }

            _ => {
                log::debug!("Unknown command: {}", cmd);
                // Skip optional and braced arguments to prevent arg text from leaking into body
                while self.current().kind == TokenKind::OpenBracket {
                    let _ = self.try_read_optional_arg();
                }
                while self.current().kind == TokenKind::OpenBrace {
                    let _ = self.read_braced_text();
                }
                Ok(None)
            }
        }
    }

    pub(crate) fn parse_section(&mut self, level: SectionLevel, numbered: bool) -> Result<Option<Node>> {
        let _opt = self.try_read_optional_arg(); // short title
        self.skip_whitespace_and_comments();
        let title = self.read_braced_nodes()?;

        if numbered {
            let idx = (level.depth() + 1) as usize;
            if idx < self.section_counters.len() {
                self.section_counters[idx] += 1;
                // Reset lower counters
                for i in (idx + 1)..self.section_counters.len() {
                    self.section_counters[i] = 0;
                }
            }
        }

        Ok(Some(Node::Section { level, title, numbered }))
    }

    /// Parse listing options from key=value string (language=Python,numbers=left,caption={...},...)
    pub(super) fn parse_listing_options(
        opts: &str,
        language: &mut Option<String>,
        caption: &mut Option<String>,
        label: &mut Option<String>,
        numbers: &mut ListingNumbers,
        frame: &mut bool,
    ) {
        for part in opts.split(',') {
            let part = part.trim();
            if let Some((key, val)) = part.split_once('=') {
                let key = key.trim().to_lowercase();
                let val = val.trim().trim_matches(|c| c == '{' || c == '}');
                match key.as_str() {
                    "language" => *language = Some(val.to_lowercase()),
                    "caption" => *caption = Some(val.to_string()),
                    "label" => *label = Some(val.to_string()),
                    "numbers" => {
                        *numbers = match val {
                            "left" => ListingNumbers::Left,
                            "right" => ListingNumbers::Right,
                            _ => ListingNumbers::None,
                        };
                    }
                    "frame" => {
                        *frame = val != "none";
                    }
                    _ => {}
                }
            } else if part == "linenos" {
                *numbers = ListingNumbers::Left;
            } else if part == "frame" {
                *frame = true;
            }
        }
    }

    /// Format a number string with thin-space digit grouping (siunitx \num convention).
    /// Groups digits in threes from the decimal point outward.
    /// "12345.6789" → "12\u{2009}345.678\u{2009}9"
    fn format_num_grouping(num: &str) -> String {
        let num = num.trim();
        // Split into integer and fractional parts
        let (int_part, frac_part) = if let Some(dot_idx) = num.find('.') {
            (&num[..dot_idx], Some(&num[dot_idx+1..]))
        } else {
            (num, None)
        };

        // Handle optional sign
        let (sign, digits) = if int_part.starts_with('-') || int_part.starts_with('+') {
            (&int_part[..1], &int_part[1..])
        } else {
            ("", int_part)
        };

        let mut result = String::with_capacity(num.len() + 4);
        result.push_str(sign);

        // Group integer part from right (only if > 4 digits, per SI convention)
        if digits.len() > 4 {
            let mut i = 0;
            let offset = digits.len() % 3;
            for ch in digits.chars() {
                if i > 0 && (i - offset) % 3 == 0 && i != digits.len() {
                    // For offset==0, first group at position 3, otherwise at offset
                    if offset == 0 || i >= offset {
                        result.push('\u{2009}'); // thin space
                    }
                }
                if offset > 0 && i == offset && i > 0 {
                    result.push('\u{2009}');
                }
                result.push(ch);
                i += 1;
            }
            // Simpler approach: rebuild
            result.truncate(sign.len());
            let d: Vec<char> = digits.chars().collect();
            let rem = d.len() % 3;
            for (i, &ch) in d.iter().enumerate() {
                if i > 0 && i % 3 == rem && rem > 0 {
                    result.push('\u{2009}');
                } else if i > 0 && rem == 0 && i % 3 == 0 {
                    result.push('\u{2009}');
                }
                result.push(ch);
            }
        } else {
            result.push_str(digits);
        }

        // Fractional part: group from left (only if > 4 digits)
        if let Some(frac) = frac_part {
            result.push('.');
            if frac.len() > 4 {
                for (i, ch) in frac.chars().enumerate() {
                    if i > 0 && i % 3 == 0 {
                        result.push('\u{2009}');
                    }
                    result.push(ch);
                }
            } else {
                result.push_str(frac);
            }
        }

        result
    }
}

/// Extract plain text from a node tree (for title extraction, etc.)
fn node_to_plain_text(node: &Node, out: &mut String, source: &str) {
    match node {
        Node::Text(t) => out.push_str(t),
        Node::TextRef(offset, len) => {
            let s = &source[*offset as usize..*offset as usize + *len as usize];
            out.push_str(s.trim());
        }
        Node::Bold(children) | Node::Italic(children) | Node::Emph(children)
        | Node::Monospace(children) | Node::SansSerif(children) | Node::SmallCaps(children)
        | Node::Underline(children) | Node::Group(children) | Node::MBox(children)
        | Node::Paragraph(children) => {
            for child in children {
                node_to_plain_text(child, out, source);
            }
        }
        Node::Colored { content, .. } => {
            for child in content {
                node_to_plain_text(child, out, source);
            }
        }
        Node::NonBreakingSpace => out.push(' '),
        Node::HSpace(_) => out.push(' '),
        _ => {}
    }
}

/// Parse a \definecolor model+spec into a Color.
/// Supports rgb (0-1 floats), RGB (0-255 ints), HTML (hex), gray, cmyk, named.
fn parse_color_model(model: &str, spec: &str) -> Option<Color> {
    match model {
        "rgb" => {
            let parts: Vec<f32> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if parts.len() == 3 {
                Some(Color::from_rgb_u8((parts[0] * 255.0) as u8, (parts[1] * 255.0) as u8, (parts[2] * 255.0) as u8))
            } else { None }
        }
        "RGB" => {
            let parts: Vec<u8> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if parts.len() == 3 { Some(Color::from_rgb_u8(parts[0], parts[1], parts[2])) } else { None }
        }
        "HTML" | "html" => {
            let hex = spec.trim().trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Color::from_rgb_u8(r, g, b))
            } else { None }
        }
        "gray" => {
            let v: f32 = spec.trim().parse().ok()?;
            let b = (v * 255.0) as u8;
            Some(Color::from_rgb_u8(b, b, b))
        }
        "cmyk" => {
            let parts: Vec<f32> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if parts.len() == 4 {
                let (c, m, y, k) = (parts[0], parts[1], parts[2], parts[3]);
                let r = ((1.0 - c) * (1.0 - k) * 255.0) as u8;
                let g = ((1.0 - m) * (1.0 - k) * 255.0) as u8;
                let b = ((1.0 - y) * (1.0 - k) * 255.0) as u8;
                Some(Color::from_rgb_u8(r, g, b))
            } else { None }
        }
        "named" => Color::from_name(spec.trim()),
        _ => None,
    }
}

/// Expand LaTeX accent commands in raw text (e.g. `\"a` → `ä`, `\'e` → `é`).
fn expand_latex_accents(input: &str) -> String {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            let cmd = bytes[i + 1];
            // Two-char accent commands: \"x, \'x, \`x, \~x, \^x, \=x, \.x
            let accent_map: Option<&[(char, char)]> = match cmd {
                b'"' => Some(&[
                    ('a', 'ä'), ('e', 'ë'), ('i', 'ï'), ('o', 'ö'), ('u', 'ü'),
                    ('A', 'Ä'), ('E', 'Ë'), ('I', 'Ï'), ('O', 'Ö'), ('U', 'Ü'), ('y', 'ÿ'),
                ]),
                b'\'' => Some(&[
                    ('a', 'á'), ('e', 'é'), ('i', 'í'), ('o', 'ó'), ('u', 'ú'),
                    ('A', 'Á'), ('E', 'É'), ('I', 'Í'), ('O', 'Ó'), ('U', 'Ú'),
                    ('y', 'ý'), ('Y', 'Ý'),
                ]),
                b'`' => Some(&[
                    ('a', 'à'), ('e', 'è'), ('i', 'ì'), ('o', 'ò'), ('u', 'ù'),
                    ('A', 'À'), ('E', 'È'), ('I', 'Ì'), ('O', 'Ò'), ('U', 'Ù'),
                ]),
                b'~' => Some(&[
                    ('a', 'ã'), ('n', 'ñ'), ('o', 'õ'),
                    ('A', 'Ã'), ('N', 'Ñ'), ('O', 'Õ'),
                ]),
                b'^' => Some(&[
                    ('a', 'â'), ('e', 'ê'), ('i', 'î'), ('o', 'ô'), ('u', 'û'),
                    ('A', 'Â'), ('E', 'Ê'), ('I', 'Î'), ('O', 'Ô'), ('U', 'Û'),
                ]),
                b'=' => Some(&[
                    ('a', 'ā'), ('e', 'ē'), ('i', 'ī'), ('o', 'ō'), ('u', 'ū'),
                ]),
                _ => None,
            };
            if let Some(map) = accent_map {
                // Read the target character: either \"{x} or \"x
                let mut j = i + 2;
                let braced = j < len && bytes[j] == b'{';
                if braced { j += 1; }
                if j < len {
                    let target = bytes[j] as char;
                    if let Some(&(_, accented)) = map.iter().find(|&&(c, _)| c == target) {
                        out.push(accented);
                        j += 1;
                        if braced && j < len && bytes[j] == b'}' { j += 1; }
                        i = j;
                        continue;
                    }
                }
            }
            // \ss → ß
            if cmd == b's' && i + 2 < len && bytes[i + 2] == b's' {
                let after = if i + 3 < len { bytes[i + 3] } else { b' ' };
                if !after.is_ascii_alphabetic() {
                    out.push('ß');
                    i += 3;
                    continue;
                }
            }
            // \c{c} → ç
            if cmd == b'c' && i + 2 < len && bytes[i + 2] == b'{' {
                if let Some(end) = input[i+3..].find('}') {
                    let ch = bytes[i + 3] as char;
                    let cedilla = match ch {
                        'c' => Some('ç'), 'C' => Some('Ç'),
                        's' => Some('ş'), 'S' => Some('Ş'),
                        _ => None,
                    };
                    if let Some(c) = cedilla {
                        out.push(c);
                        i = i + 4 + end;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
