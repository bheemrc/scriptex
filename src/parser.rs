use anyhow::{Result, bail};
use crate::lexer::{Token, TokenKind, cmd_id};
use crate::document::*;
use crate::color::Color;


pub struct Parser<'a> {
    tokens: Vec<Token>,
    source: &'a str,
    pos: usize,
    section_counters: [u32; 7],
}

impl<'a> Parser<'a> {
    pub fn new(mut tokens: Vec<Token>, source: &'a str) -> Self {
        // Add sentinel EOF token to avoid bounds checks in hot loops
        tokens.push(Token::EOF);
        Parser {
            tokens,
            source,
            pos: 0,
            section_counters: [0; 7],
        }
    }

    pub fn parse(&mut self) -> Result<Document> {
        self.skip_whitespace_and_comments();
        let class = self.parse_document_class()?;
        let preamble = self.parse_preamble()?;
        let body = self.parse_body()?;
        Ok(Document { class, preamble, body })
    }

    #[inline(always)]
    fn current(&self) -> Token {
        // SAFETY: sentinel EOF token guarantees self.pos is always valid
        unsafe { *self.tokens.get_unchecked(self.pos) }
    }

    fn peek(&self) -> Token {
        self.current()
    }

    #[inline(always)]
    fn advance(&mut self) -> Token {
        // SAFETY: sentinel EOF token guarantees self.pos is always valid
        let tok = unsafe { *self.tokens.get_unchecked(self.pos) };
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    fn token_text(&self, token: Token) -> &'a str {
        token.text(self.source)
    }

    /// Get text of current token without allocating
    fn current_text(&self) -> &'a str {
        self.current().text(self.source)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            let tok = self.current();
            match tok.kind {
                TokenKind::Space | TokenKind::Comment => { self.pos += 1; }
                TokenKind::Text => {
                    // Skip text tokens that are only whitespace (spaces merged into text)
                    // Fast check: if first byte is not space/tab, it's definitely not all whitespace
                    let first_byte = unsafe { *self.source.as_bytes().get_unchecked(tok.pos as usize) };
                    if first_byte != b' ' && first_byte != b'\t' {
                        break;
                    }
                    let text = &self.source.as_bytes()[tok.pos as usize..(tok.pos as usize + tok.len as usize)];
                    if text.iter().all(|&b| b == b' ' || b == b'\t') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    fn expect_open_brace(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        match self.current().kind {
            TokenKind::OpenBrace => { self.advance(); Ok(()) }
            _ => bail!("Expected '{{', got {:?}", self.current()),
        }
    }

    fn expect_close_brace(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        match self.current().kind {
            TokenKind::CloseBrace => { self.advance(); Ok(()) }
            _ => bail!("Expected '}}', got {:?}", self.current()),
        }
    }

    fn read_braced_text(&mut self) -> Result<String> {
        self.expect_open_brace()?;

        // Fast path: check for simple cases
        if self.pos + 1 < self.tokens.len() {
            if self.current().kind == TokenKind::CloseBrace {
                self.advance();
                return Ok(String::new());
            }
            // Single text token - use source slice
            if self.current().kind == TokenKind::Text
                && self.tokens[self.pos + 1].kind == TokenKind::CloseBrace
            {
                let start = self.current().pos as usize;
                let end = start + self.current().len as usize;
                self.pos += 2;
                return Ok(self.source[start..end].to_string());
            }
            // Multiple tokens before close brace with no nesting - use source range
            if !matches!(self.current().kind, TokenKind::OpenBrace | TokenKind::Eof) {
                let start = self.current().pos as usize;
                let mut end = start;
                let mut scan = self.pos;
                let mut simple = true;
                while scan < self.tokens.len() {
                    let tok = self.tokens[scan];
                    match tok.kind {
                        TokenKind::CloseBrace => break,
                        TokenKind::OpenBrace | TokenKind::Eof => { simple = false; break; }
                        _ => {
                            end = tok.pos as usize + tok.len as usize;
                            scan += 1;
                        }
                    }
                }
                if simple && scan < self.tokens.len() && self.tokens[scan].kind == TokenKind::CloseBrace {
                    self.pos = scan + 1;
                    return Ok(self.source[start..end].to_string());
                }
            }
        }

        // General case with nesting
        let mut depth = 1;
        let mut text = String::with_capacity(32);
        while depth > 0 {
            match self.current().kind {
                TokenKind::OpenBrace => { depth += 1; text.push('{'); self.advance(); }
                TokenKind::CloseBrace => {
                    depth -= 1;
                    if depth > 0 { text.push('}'); }
                    self.advance();
                }
                TokenKind::Eof => bail!("Unexpected end of input in braced group"),
                _ => {
                    text.push_str(self.current_text());
                    self.advance();
                }
            }
        }
        Ok(text)
    }

    fn read_braced_nodes(&mut self) -> Result<Vec<Node>> {
        self.expect_open_brace()?;
        self.parse_nodes_until_close_brace()
    }

    fn parse_nodes_until_close_brace(&mut self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        let mut depth = 1;

        loop {
            match self.current().kind {
                TokenKind::CloseBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        break;
                    }
                    nodes.push(Node::RightBrace);
                    self.advance();
                }
                TokenKind::OpenBrace => {
                    depth += 1;
                    self.advance();
                    let inner = self.parse_nodes_until_close_brace()?;
                    depth -= 1;
                    nodes.push(Node::Group(inner));
                }
                TokenKind::Eof => bail!("Unexpected end of input, expected '}}'"),
                _ => {
                    if let Some(node) = self.parse_node()? {
                        nodes.push(node);
                    }
                }
            }
        }

        Ok(nodes)
    }

    fn try_read_optional_arg(&mut self) -> Option<String> {
        self.skip_whitespace_and_comments();
        if self.current().kind == TokenKind::OpenBracket {
            self.advance();
            let mut text = String::new();
            let mut depth = 1;
            loop {
                match self.current().kind {
                    TokenKind::OpenBracket => { depth += 1; text.push('['); self.advance(); }
                    TokenKind::CloseBracket => {
                        depth -= 1;
                        if depth == 0 { self.advance(); break; }
                        text.push(']');
                        self.advance();
                    }
                    TokenKind::Eof => break,
                    _ => {
                        text.push_str(self.current().text(self.source));
                        self.advance();
                    }
                }
            }
            Some(text)
        } else {
            None
        }
    }

    fn parse_document_class(&mut self) -> Result<DocumentClass> {
        self.skip_whitespace_and_comments();

        // Check for \documentclass
        match self.current().kind {
            TokenKind::Command => {
                let cmd = self.current().text(self.source).to_string();
                if cmd != "\\documentclass" {
                    // No document class, use default
                    return Ok(DocumentClass {
                        class_type: ClassType::Article,
                        options: Vec::new(),
                    });
                }
                self.advance();
            }
            _ => {
                return Ok(DocumentClass {
                    class_type: ClassType::Article,
                    options: Vec::new(),
                });
            }
        }

        let options = self.try_read_optional_arg()
            .map(|s| s.split(',').map(|o| o.trim().to_string()).collect())
            .unwrap_or_default();

        let class_name = self.read_braced_text()?;

        let class_type = match class_name.as_str() {
            "article" => ClassType::Article,
            "report" => ClassType::Report,
            "book" => ClassType::Book,
            "letter" => ClassType::Letter,
            "beamer" => ClassType::Beamer,
            "memoir" => ClassType::Memoir,
            other => ClassType::Custom(other.to_string()),
        };

        Ok(DocumentClass { class_type, options })
    }

    fn parse_preamble(&mut self) -> Result<Preamble> {
        let mut preamble = Preamble::default();

        loop {
            self.skip_whitespace_and_comments();
            match self.current().kind {
                TokenKind::Command => {
                    let cmd = self.current_text();
                    match cmd {
                        "\\begin" => {
                            // Check if it's \begin{document}
                            let save = self.pos;
                            self.advance();
                            let env = self.read_braced_text()?;
                            if env == "document" {
                                break;
                            }
                            // Not document, rewind
                            self.pos = save;
                            self.advance();
                        }
                        "\\usepackage" => {
                            self.advance();
                            let options: Vec<String> = self.try_read_optional_arg()
                                .map(|s| s.split(',').map(|o| o.trim().to_string()).collect())
                                .unwrap_or_default();
                            let name = self.read_braced_text()?;

                            // Handle geometry package
                            if name == "geometry" {
                                for opt in &options {
                                    self.apply_geometry_option(opt, &mut preamble.page_setup);
                                }
                            }
                            // Handle font size from package options
                            if name == "fontenc" || name == "inputenc" {
                                // standard packages, just record
                            }

                            preamble.packages.push(Package { name, options });
                        }
                        "\\title" => {
                            self.advance();
                            let title = self.read_braced_text()?;
                            preamble.title = Some(title);
                        }
                        "\\author" => {
                            self.advance();
                            let author = self.read_braced_text()?;
                            preamble.author = Some(author);
                        }
                        "\\date" => {
                            self.advance();
                            let date = self.read_braced_text()?;
                            preamble.date = Some(date);
                        }
                        "\\geometry" => {
                            self.advance();
                            let geom_text = self.read_braced_text()?;
                            for opt in geom_text.split(',') {
                                self.apply_geometry_option(opt.trim(), &mut preamble.page_setup);
                            }
                        }
                        "\\linespread" => {
                            self.advance();
                            let val = self.read_braced_text()?;
                            if let Ok(v) = val.parse::<f32>() {
                                preamble.line_spacing = v;
                            }
                        }
                        "\\pagestyle" | "\\thispagestyle" | "\\setlength"
                        | "\\newcommand" | "\\renewcommand" | "\\def"
                        | "\\DeclareMathOperator" | "\\theoremstyle"
                        | "\\newtheorem" | "\\bibliographystyle"
                        | "\\hypersetup" | "\\lstset" | "\\graphicspath"
                        | "\\setcounter" | "\\numberwithin" => {
                            // Skip these preamble commands - consume their arguments
                            self.advance();
                            self.skip_command_args();
                        }
                        _ => {
                            self.advance();
                            self.skip_command_args();
                        }
                    }
                }
                TokenKind::ParBreak | TokenKind::Space => { self.advance(); }
                TokenKind::Eof => break,
                _ => { self.advance(); }
            }
        }

        // Apply font size from document class options
        // (already parsed in class options)

        Ok(preamble)
    }

    fn apply_geometry_option(&self, opt: &str, setup: &mut PageSetup) {
        let parts: Vec<&str> = opt.split('=').collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let val = parts[1].trim();
            if let Some(points) = self.parse_dimension(val) {
                match key {
                    "top" | "tmargin" => setup.margin_top = points,
                    "bottom" | "bmargin" => setup.margin_bottom = points,
                    "left" | "lmargin" | "inner" => setup.margin_left = points,
                    "right" | "rmargin" | "outer" => setup.margin_right = points,
                    "margin" => {
                        setup.margin_top = points;
                        setup.margin_bottom = points;
                        setup.margin_left = points;
                        setup.margin_right = points;
                    }
                    "paperwidth" => setup.width = points,
                    "paperheight" => setup.height = points,
                    "headheight" | "headsep" => setup.header_height = points,
                    "footskip" => setup.footer_height = points,
                    "columnsep" => setup.column_sep = points,
                    _ => {}
                }
            }
            // Paper size options without =
        } else {
            match opt.trim() {
                "a4paper" => { setup.width = 595.276; setup.height = 841.890; }
                "letterpaper" => { setup.width = 612.0; setup.height = 792.0; }
                "a5paper" => { setup.width = 419.528; setup.height = 595.276; }
                "b5paper" => { setup.width = 498.898; setup.height = 708.661; }
                "legalpaper" => { setup.width = 612.0; setup.height = 1008.0; }
                "landscape" => { std::mem::swap(&mut setup.width, &mut setup.height); }
                "twocolumn" => { setup.columns = 2; }
                _ => {}
            }
        }
    }

    fn parse_dimension(&self, text: &str) -> Option<f32> {
        let text = text.trim();
        // Try to parse dimension with unit
        let (num_str, unit) = if text.ends_with("pt") {
            (&text[..text.len()-2], "pt")
        } else if text.ends_with("mm") {
            (&text[..text.len()-2], "mm")
        } else if text.ends_with("cm") {
            (&text[..text.len()-2], "cm")
        } else if text.ends_with("in") {
            (&text[..text.len()-2], "in")
        } else if text.ends_with("em") {
            (&text[..text.len()-2], "em")
        } else if text.ends_with("ex") {
            (&text[..text.len()-2], "ex")
        } else if text.ends_with("bp") {
            (&text[..text.len()-2], "bp")
        } else if text.ends_with("pc") {
            (&text[..text.len()-2], "pc")
        } else {
            (text, "pt")
        };

        let value: f32 = num_str.trim().parse().ok()?;
        let points = match unit {
            "pt" => value,
            "mm" => value * 2.83465,
            "cm" => value * 28.3465,
            "in" => value * 72.0,
            "em" => value * 10.0,
            "ex" => value * 5.0,
            "bp" => value,
            "pc" => value * 12.0,
            _ => value,
        };
        Some(points)
    }

    fn skip_command_args(&mut self) {
        loop {
            self.skip_whitespace_and_comments();
            match self.current().kind {
                TokenKind::OpenBrace => {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.current().kind {
                            TokenKind::OpenBrace => { depth += 1; self.advance(); }
                            TokenKind::CloseBrace => { depth -= 1; self.advance(); }
                            TokenKind::Eof => break,
                            _ => { self.advance(); }
                        }
                    }
                }
                TokenKind::OpenBracket => {
                    self.advance();
                    let mut depth = 1;
                    while depth > 0 {
                        match self.current().kind {
                            TokenKind::OpenBracket => { depth += 1; self.advance(); }
                            TokenKind::CloseBracket => { depth -= 1; self.advance(); }
                            TokenKind::Eof => break,
                            _ => { self.advance(); }
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_body(&mut self) -> Result<Vec<Node>> {
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

    #[inline]
    fn is_block_node(node: &Node) -> bool {
        matches!(node,
            Node::PageBreak | Node::HRule | Node::VSpace(_)
            | Node::Section { .. } | Node::Table(_) | Node::Figure(_)
            | Node::ItemizeList(_) | Node::EnumerateList(_) | Node::DescriptionList(_)
            | Node::DisplayMath(_) | Node::MakeTitle | Node::TableOfContents
            | Node::Quote(_) | Node::Quotation(_) | Node::Verbatim(_)
            | Node::Abstract(_) | Node::Center(_) | Node::FlushLeft(_)
            | Node::FlushRight(_) | Node::Environment(_)
        )
    }

    fn parse_node(&mut self) -> Result<Option<Node>> {
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
                Ok(Some(Node::DisplayMath(math)))
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

    fn parse_command(&mut self) -> Result<Option<Node>> {
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
                cmd_id::NOINDENT => Ok(None),
                cmd_id::NEWPAGE => Ok(Some(Node::PageBreak)),
                cmd_id::CENTERING => Ok(None),
                cmd_id::HLINE => { self.skip_command_args(); Ok(Some(Node::HRule)) }
                cmd_id::LABEL => { let l = self.read_braced_text()?; Ok(Some(Node::Label(l))) }
                cmd_id::REF => { let l = self.read_braced_text()?; Ok(Some(Node::Ref(l))) }
                cmd_id::CITE => { let _opt = self.try_read_optional_arg(); let k = self.read_braced_text()?; Ok(Some(Node::Citation(k))) }
                cmd_id::BIBITEM => { let _opt = self.try_read_optional_arg(); let k = self.read_braced_text()?; Ok(Some(Node::BibItem(k))) }
                cmd_id::FOOTNOTE => { let n = self.read_braced_nodes()?; Ok(Some(Node::Footnote(n))) }
                cmd_id::VSPACE => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::VSpace(pts))) }
                cmd_id::HSPACE => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::HSpace(pts))) }
                cmd_id::HREF => { let _url = self.read_braced_text()?; let content = self.read_braced_nodes()?; Ok(Some(Node::Group(content))) }
                cmd_id::URL => { let url = self.read_braced_text()?; Ok(Some(Node::Text(url))) }
                cmd_id::TEXTCOLOR => { let cn = self.read_braced_text()?; let c = self.read_braced_nodes()?; let color = Color::from_name(&cn).unwrap_or(Color::BLACK); Ok(Some(Node::Colored { color, content: c })) }
                cmd_id::COLOR => { let _cn = self.read_braced_text()?; Ok(Some(Node::Text(String::new()))) }
                cmd_id::CAPTION => { let _n = self.read_braced_nodes()?; Ok(None) }
                cmd_id::INCLUDEGRAPHICS => {
                    let opts = self.try_read_optional_arg();
                    let path = self.read_braced_text()?;
                    let mut width = None;
                    let mut height = None;
                    let mut scale = None;
                    if let Some(opt_str) = opts {
                        for opt in opt_str.split(',') {
                            let parts: Vec<&str> = opt.split('=').collect();
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
                                    _ => {}
                                }
                            }
                        }
                    }
                    Ok(Some(Node::Image(Box::new(ImageData { path, width, height, scale }))))
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
                _ => Ok(None),
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
            "\\textrm" | "\\textnormal" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Group(n))) }
            "\\textsf" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Group(n))) }
            "\\textsl" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Italic(n))) }

            // Style switches
            "\\bf" | "\\bfseries" | "\\it" | "\\itshape"
            | "\\tt" | "\\ttfamily" | "\\rm" | "\\rmfamily"
            | "\\sf" | "\\sffamily" | "\\sc" | "\\scshape"
            | "\\sl" | "\\slshape" | "\\normalfont" => Ok(None),

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

            // Spacing (starred variants fall through here)
            "\\hspace*" => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::HSpace(pts))) }
            "\\vspace*" => { let dim = self.read_braced_text()?; let pts = self.parse_dimension(&dim).unwrap_or(10.0); Ok(Some(Node::VSpace(pts))) }
            "\\quad" => Ok(Some(Node::HSpace(18.0))),
            "\\qquad" => Ok(Some(Node::HSpace(36.0))),
            "\\enspace" => Ok(Some(Node::HSpace(9.0))),
            "\\thinspace" | "\\," => Ok(Some(Node::HSpace(3.0))),
            "\\;" => Ok(Some(Node::HSpace(5.0))),
            "\\:" => Ok(Some(Node::HSpace(4.0))),
            "\\!" => Ok(Some(Node::HSpace(-3.0))),
            "\\hfill" => Ok(Some(Node::HSpace(0.0))),
            "\\smallskip" => Ok(Some(Node::VSpace(3.0))),
            "\\medskip" => Ok(Some(Node::VSpace(6.0))),
            "\\bigskip" => Ok(Some(Node::VSpace(12.0))),

            // Breaks
            "\\newline" | "\\linebreak" => Ok(Some(Node::LineBreak)),
            "\\clearpage" | "\\cleardoublepage" | "\\pagebreak" => Ok(Some(Node::PageBreak)),
            "\\indent" => Ok(Some(Node::HSpace(20.0))),

            // Rules
            "\\hrule" | "\\rule" => { self.skip_command_args(); Ok(Some(Node::HRule)) }

            // Special characters
            "\\LaTeX" | "\\TeX" => Ok(Some(Node::Text(cmd.trim_start_matches('\\').to_string()))),
            "\\ldots" | "\\dots" | "\\textellipsis" => Ok(Some(Node::Ellipsis)),
            "\\textendash" => Ok(Some(Node::EnDash)),
            "\\textemdash" => Ok(Some(Node::EmDash)),
            "\\textquoteleft" => Ok(Some(Node::LeftQuote)),
            "\\textquoteright" => Ok(Some(Node::RightQuote)),
            "\\textquotedblleft" => Ok(Some(Node::LeftDoubleQuote)),
            "\\textquotedblright" => Ok(Some(Node::RightDoubleQuote)),
            "\\copyright" => Ok(Some(Node::Copyright)),
            "\\textregistered" => Ok(Some(Node::Registered)),
            "\\texttrademark" => Ok(Some(Node::Trademark)),
            "\\&" | "\\amp" => Ok(Some(Node::Ampersand)),
            "\\%" => Ok(Some(Node::Percent)),
            "\\$" => Ok(Some(Node::Dollar)),
            "\\#" => Ok(Some(Node::Hash)),
            "\\_" => Ok(Some(Node::Underscore)),
            "\\{" => Ok(Some(Node::LeftBrace)),
            "\\}" => Ok(Some(Node::RightBrace)),
            "\\~" => Ok(Some(Node::Tilde)),
            "\\^" => Ok(Some(Node::Caret)),
            "\\\\" => Ok(Some(Node::Backslash)),
            "\\textbackslash" => Ok(Some(Node::Backslash)),
            "\\S" => Ok(Some(Node::Text("\u{00A7}".to_string()))),
            "\\P" => Ok(Some(Node::Text("\u{00B6}".to_string()))),
            "\\dag" => Ok(Some(Node::Text("\u{2020}".to_string()))),
            "\\ddag" => Ok(Some(Node::Text("\u{2021}".to_string()))),

            // Accented characters
            "\\\'" => {
                let c = self.read_braced_text().unwrap_or_default();
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
                let c = self.read_braced_text().unwrap_or_default();
                let accented = match c.as_str() {
                    "a" => "\u{00E0}", "e" => "\u{00E8}", "i" => "\u{00EC}",
                    "o" => "\u{00F2}", "u" => "\u{00F9}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }
            "\\\"" => {
                let c = self.read_braced_text().unwrap_or_default();
                let accented = match c.as_str() {
                    "a" => "\u{00E4}", "e" => "\u{00EB}", "i" => "\u{00EF}",
                    "o" => "\u{00F6}", "u" => "\u{00FC}",
                    "A" => "\u{00C4}", "O" => "\u{00D6}", "U" => "\u{00DC}",
                    _ => &c,
                };
                Ok(Some(Node::Text(accented.to_string())))
            }

            // Colors (rare variants)
            "\\colorbox" => {
                let _bg = self.read_braced_text()?;
                let content = self.read_braced_nodes()?;
                Ok(Some(Node::Group(content)))
            }

            // Cross-references (variants not in cmd_id)
            "\\eqref" | "\\pageref" | "\\autoref" | "\\cref" => {
                let label = self.read_braced_text()?;
                Ok(Some(Node::Ref(label)))
            }
            "\\citep" | "\\citet" | "\\citealp" => {
                let _opt = self.try_read_optional_arg();
                let key = self.read_braced_text()?;
                Ok(Some(Node::Citation(key)))
            }

            // No-ops
            "\\nobreak" | "\\allowbreak" | "\\relax" | "\\protect"
            | "\\sloppy" | "\\fussy" | "\\raggedright" | "\\raggedleft"
            | "\\selectfont" | "\\frenchspacing"
            | "\\nonfrenchspacing" => Ok(None),

            "\\input" | "\\include" => { let _file = self.read_braced_text()?; Ok(None) }
            "\\pagestyle" | "\\thispagestyle" => { let _style = self.read_braced_text()?; Ok(None) }
            "\\setlength" | "\\addtolength" => { self.skip_command_args(); Ok(None) }
            "\\newcommand" | "\\renewcommand" | "\\providecommand" | "\\def" => { self.skip_command_args(); Ok(None) }
            "\\bibliography" | "\\addbibresource" => {
                let _bib_file = self.read_braced_text()?;
                // Bibliography loading happens outside the parser
                Ok(None)
            }
            "\\printbibliography" | "\\nocite" => {
                self.skip_command_args();
                Ok(None)
            }
            "\\bibliographystyle" => { let _style = self.read_braced_text()?; Ok(None) }

            _ => {
                log::debug!("Unknown command: {}", cmd);
                Ok(None)
            }
        }
    }

    fn parse_section(&mut self, level: SectionLevel, numbered: bool) -> Result<Option<Node>> {
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

    fn parse_begin_environment(&mut self) -> Result<Option<Node>> {
        let env_name = self.read_braced_text()?;

        match env_name.as_str() {
            "itemize" => self.parse_list_environment("itemize"),
            "enumerate" => self.parse_list_environment("enumerate"),
            "description" => self.parse_list_environment("description"),
            "tabular" | "tabular*" | "array" => self.parse_tabular_environment(&env_name),
            "table" | "table*" => self.parse_float_environment(&env_name, true),
            "figure" | "figure*" => self.parse_float_environment(&env_name, false),
            "equation" | "equation*" | "align" | "align*"
            | "gather" | "gather*" | "multline" | "multline*" => {
                self.parse_display_math_environment(&env_name)
            }
            "verbatim" => self.parse_verbatim_environment(&env_name),
            "lstlisting" => {
                // Extract language from optional args: \begin{lstlisting}[language=Python]
                let opt = self.try_read_optional_arg();
                let lang = opt.as_ref().and_then(|o| {
                    for part in o.split(',') {
                        let parts: Vec<&str> = part.split('=').collect();
                        if parts.len() == 2 && parts[0].trim().eq_ignore_ascii_case("language") {
                            return Some(parts[1].trim().to_lowercase());
                        }
                    }
                    None
                });
                let mut text = String::new();
                self.read_verbatim_content(&env_name, &mut text)?;
                // Store language in the verbatim text as a prefix marker
                if let Some(l) = lang {
                    Ok(Some(Node::Verbatim(format!("%%lang:{}%%\n{}", l, text))))
                } else {
                    Ok(Some(Node::Verbatim(text)))
                }
            }
            "minted" => {
                // \begin{minted}{python}
                let lang = self.read_braced_text().ok();
                let mut text = String::new();
                self.read_verbatim_content(&env_name, &mut text)?;
                if let Some(l) = lang {
                    Ok(Some(Node::Verbatim(format!("%%lang:{}%%\n{}", l.to_lowercase(), text))))
                } else {
                    Ok(Some(Node::Verbatim(text)))
                }
            }
            "tikzpicture" | "pgfplots" | "pgfonlayer" | "scope"
            | "axis" | "semilogxaxis" | "semilogyaxis" | "loglogaxis" => {
                // Capture TikZ source for rendering via pdflatex shell-out
                let raw_source = self.capture_environment_raw(&env_name)?;
                Ok(Some(Node::Verbatim(format!("%%tikz:{}%%\n{}", env_name, raw_source))))
            }
            "quote" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Quote(content)))
            }
            "quotation" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Quotation(content)))
            }
            "abstract" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Abstract(content)))
            }
            "center" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Center(content)))
            }
            "flushleft" | "raggedright" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::FlushLeft(content)))
            }
            "flushright" | "raggedleft" => {
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::FlushRight(content)))
            }
            "minipage" => {
                let _opt = self.try_read_optional_arg();
                let width_str = self.read_braced_text()?;
                let width = self.parse_dimension(&width_str).unwrap_or(300.0);
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Minipage { width, content }))
            }
            "thebibliography" => {
                let _widest = self.try_read_optional_arg().or_else(|| {
                    self.read_braced_text().ok()
                });
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Environment(Box::new(EnvironmentData {
                    name: env_name,
                    args: vec![],
                    content,
                }))))
            }
            _ => {
                // Generic environment
                let _opt = self.try_read_optional_arg();
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Environment(Box::new(EnvironmentData {
                    name: env_name,
                    args: vec![],
                    content,
                }))))
            }
        }
    }

    fn parse_environment_body(&mut self, env_name: &str) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end of input, expected \\end{{{}}}", env_name),
                TokenKind::Command => {
                    if self.current().cmd == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    if let Some(node) = self.parse_node()? {
                        nodes.push(node);
                    }
                }
                _ => {
                    if let Some(node) = self.parse_node()? {
                        nodes.push(node);
                    }
                }
            }
        }

        Ok(nodes)
    }

    fn parse_list_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        let _opt = self.try_read_optional_arg();
        let mut items = Vec::new();
        let mut current_content: Vec<Node> = Vec::new();
        let mut current_label: Option<Vec<Node>> = None;
        let mut in_item = false;

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in list environment"),
                TokenKind::Command => {
                    let cid = self.current().cmd;
                    if cid == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                if in_item {
                                    items.push(ListItem {
                                        label: current_label.take(),
                                        content: std::mem::take(&mut current_content),
                                    });
                                }
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    if cid == cmd_id::ITEM {
                        self.advance();
                        if in_item {
                            items.push(ListItem {
                                label: current_label.take(),
                                content: std::mem::take(&mut current_content),
                            });
                        }
                        in_item = true;
                        current_label = None;

                        // Check for optional label [...]
                        if let Some(label_text) = self.try_read_optional_arg() {
                            current_label = Some(vec![Node::Text(label_text)]);
                        }
                    } else {
                        if let Some(node) = self.parse_node()? {
                            current_content.push(node);
                        }
                    }
                }
                _ => {
                    if let Some(node) = self.parse_node()? {
                        if in_item {
                            current_content.push(node);
                        }
                    }
                }
            }
        }

        match env_name {
            "itemize" => Ok(Some(Node::ItemizeList(items))),
            "enumerate" => Ok(Some(Node::EnumerateList(items))),
            "description" => Ok(Some(Node::DescriptionList(items))),
            _ => Ok(Some(Node::ItemizeList(items))),
        }
    }

    fn parse_tabular_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        // Parse column spec
        let col_spec_str = self.read_braced_text()?;
        let columns = self.parse_column_spec(&col_spec_str);

        let mut rows: Vec<TableRow> = Vec::new();
        let mut current_cells: Vec<TableCell> = Vec::new();
        let mut current_cell_content: Vec<Node> = Vec::new();
        let mut hline_before_next = false;
        let mut hline_after = false;
        let mut extra_space_next: f32 = 0.0;

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in tabular"),
                TokenKind::Command => {
                    let cid = self.current().cmd;
                    if cid == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                // Finish current cell/row
                                if !current_cell_content.is_empty() || !current_cells.is_empty() {
                                    current_cells.push(TableCell {
                                        content: std::mem::take(&mut current_cell_content),
                                        colspan: 1,
                                        alignment: None,
                                    });
                                    rows.push(TableRow {
                                        cells: std::mem::take(&mut current_cells),
                                        hline_before: hline_before_next,
                                        hline_after,
                                        extra_space_before: extra_space_next,
                                    });
                                    hline_before_next = false;
                                    extra_space_next = 0.0;
                                }
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }

                    if cid == cmd_id::HLINE {
                        self.advance();
                        // hline between rows → draw before the next row
                        if rows.is_empty() {
                            hline_before_next = true;
                        } else {
                            // Also set on previous row as hline_after for compat
                            rows.last_mut().unwrap().hline_after = true;
                        }
                        continue;
                    }
                    {
                        let cmd = self.current_text();
                        if cmd == "\\toprule" || cmd == "\\midrule" || cmd == "\\bottomrule" {
                            self.advance();
                            if cmd == "\\bottomrule" {
                                // bottomrule: set hline_after on the previous row
                                if let Some(last_row) = rows.last_mut() {
                                    last_row.hline_after = true;
                                }
                            } else {
                                // toprule/midrule: draw line before the next row
                                hline_before_next = true;
                            }
                            continue;
                        }
                        if cmd == "\\cline" {
                            self.advance();
                            self.skip_command_args();
                            hline_after = true;
                            continue;
                        }
                        if cmd == "\\multicolumn" {
                            self.advance();
                            let colspan_str = self.read_braced_text()?;
                            let _align = self.read_braced_text()?;
                            let content = self.read_braced_nodes()?;
                            current_cells.push(TableCell {
                                content,
                                colspan: colspan_str.parse().unwrap_or(1),
                                alignment: None,
                            });
                            continue;
                        }
                        if cmd == "\\addlinespace" {
                            self.advance();
                            // Parse optional arg like [5pt], default 3pt for booktabs
                            let space = if let Some(arg) = self.try_read_optional_arg() {
                                self.parse_dimension(&arg).unwrap_or(3.0)
                            } else {
                                3.0
                            };
                            extra_space_next += space;
                            continue;
                        }
                        if cmd == "\\noalign" {
                            self.advance();
                            self.try_read_optional_arg();
                            if self.current().kind == TokenKind::OpenBrace {
                                let _ = self.read_braced_text();
                            }
                            continue;
                        }
                        if cmd == "\\multirow" {
                            self.advance();
                            let _nrows = self.read_braced_text()?; // {nrows}
                            let _width = self.read_braced_text()?; // {width} or {*}
                            let content = self.read_braced_nodes()?; // {content}
                            current_cell_content.extend(content);
                            continue;
                        }
                    }

                    if let Some(node) = self.parse_node()? {
                        current_cell_content.push(node);
                    }
                }
                TokenKind::Ampersand => {
                    self.advance();
                    current_cells.push(TableCell {
                        content: std::mem::take(&mut current_cell_content),
                        colspan: 1,
                        alignment: None,
                    });
                }
                TokenKind::DoubleBackslash => {
                    self.advance();
                    // Optional [dimension] after \\
                    self.try_read_optional_arg();
                    current_cells.push(TableCell {
                        content: std::mem::take(&mut current_cell_content),
                        colspan: 1,
                        alignment: None,
                    });
                    rows.push(TableRow {
                        cells: std::mem::take(&mut current_cells),
                        hline_before: hline_before_next,
                        hline_after,
                        extra_space_before: extra_space_next,
                    });
                    hline_before_next = false;
                    hline_after = false;
                    extra_space_next = 0.0;
                }
                _ => {
                    if let Some(node) = self.parse_node()? {
                        current_cell_content.push(node);
                    }
                }
            }
        }

        Ok(Some(Node::Table(Box::new(Table {
            columns,
            rows,
            caption: None,
            label: None,
            centering: true,
        }))))
    }

    fn parse_column_spec(&self, spec: &str) -> Vec<ColumnSpec> {
        let mut cols = Vec::new();
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                'l' => { cols.push(ColumnSpec::Left); i += 1; }
                'c' => { cols.push(ColumnSpec::Center); i += 1; }
                'r' => { cols.push(ColumnSpec::Right); i += 1; }
                'p' => {
                    i += 1;
                    // Skip to { and read dimension
                    while i < chars.len() && chars[i] != '{' { i += 1; }
                    if i < chars.len() { i += 1; }
                    let start = i;
                    while i < chars.len() && chars[i] != '}' { i += 1; }
                    let dim_str: String = chars[start..i].iter().collect();
                    let width = self.parse_dimension(&dim_str).unwrap_or(100.0);
                    cols.push(ColumnSpec::Paragraph(width));
                    if i < chars.len() { i += 1; }
                }
                '|' => { cols.push(ColumnSpec::Separator); i += 1; }
                '@' => {
                    // Skip @{...}
                    i += 1;
                    if i < chars.len() && chars[i] == '{' {
                        let mut depth = 1;
                        i += 1;
                        while i < chars.len() && depth > 0 {
                            if chars[i] == '{' { depth += 1; }
                            if chars[i] == '}' { depth -= 1; }
                            i += 1;
                        }
                    }
                }
                _ => { i += 1; }
            }
        }
        cols
    }

    fn parse_float_environment(&mut self, env_name: &str, is_table: bool) -> Result<Option<Node>> {
        let placement = self.try_read_optional_arg().unwrap_or_else(|| "htbp".to_string());
        let mut content = Vec::new();
        let mut caption = None;
        let mut label = None;

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in float environment"),
                TokenKind::Command => {
                    let cid = self.current().cmd;
                    if cid == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    if cid == cmd_id::CAPTION {
                        self.advance();
                        let cap = self.read_braced_nodes()?;
                        caption = Some(cap);
                        continue;
                    }
                    if cid == cmd_id::LABEL {
                        self.advance();
                        let lab = self.read_braced_text()?;
                        label = Some(lab);
                        continue;
                    }
                    if cid == cmd_id::CENTERING {
                        self.advance();
                        continue;
                    }
                    if let Some(node) = self.parse_node()? {
                        content.push(node);
                    }
                }
                _ => {
                    if let Some(node) = self.parse_node()? {
                        content.push(node);
                    }
                }
            }
        }

        if is_table {
            // Find the Table node inside content and set caption/label
            for node in &mut content {
                if let Node::Table(ref mut tbl) = node {
                    tbl.caption = caption.clone();
                    tbl.label = label.clone();
                }
            }
            // Wrap in figure-like structure for layout but WITHOUT caption
            // (the Table node already has its own caption — avoid double caption)
            Ok(Some(Node::Figure(Box::new(FigureData {
                content,
                caption: None,
                label,
                placement,
            }))))
        } else {
            Ok(Some(Node::Figure(Box::new(FigureData {
                content,
                caption,
                label,
                placement,
            }))))
        }
    }

    fn parse_display_math_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        let mut math_nodes = Vec::new();

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in math environment"),
                TokenKind::Command => {
                    if self.current().cmd == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                break;
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    if let Some(mn) = self.parse_math_node()? {
                        math_nodes.push(mn);
                    }
                }
                _ => {
                    if let Some(mn) = self.parse_math_node()? {
                        math_nodes.push(mn);
                    }
                }
            }
        }

        Ok(Some(Node::DisplayMath(math_nodes)))
    }

    /// Skip an environment entirely, consuming tokens until \end{env_name}
    /// Used for environments we can't render (TikZ, pgfplots, etc.)
    fn skip_environment_raw(&mut self, env_name: &str) -> Result<()> {
        self.capture_environment_raw(env_name)?;
        Ok(())
    }

    /// Capture the raw source text of an environment body, then skip past \end{env_name}
    fn capture_environment_raw(&mut self, env_name: &str) -> Result<String> {
        let start_pos = self.current().pos as usize;
        let mut end_pos = start_pos;
        let mut depth = 1;
        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command => {
                    let cid = self.current().cmd;
                    if cid == cmd_id::END {
                        end_pos = self.current().pos as usize;
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                depth -= 1;
                                if depth == 0 {
                                    let raw = self.source[start_pos..end_pos].trim().to_string();
                                    return Ok(raw);
                                }
                            }
                        } else {
                            self.pos = save;
                            self.advance();
                        }
                    } else if cid == cmd_id::BEGIN {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                depth += 1;
                            }
                        } else {
                            self.pos = save;
                            self.advance();
                        }
                    } else {
                        self.advance();
                    }
                }
                _ => { self.advance(); }
            }
        }
        Ok(String::new())
    }

    fn parse_verbatim_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        let mut text = String::new();
        self.read_verbatim_content(env_name, &mut text)?;
        Ok(Some(Node::Verbatim(text)))
    }

    fn read_verbatim_content(&mut self, env_name: &str, text: &mut String) -> Result<()> {
        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command => {
                    if self.current().cmd == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                break;
                            }
                            self.pos = save;
                            text.push_str(self.current_text());
                        } else {
                            self.pos = save;
                            text.push_str(self.current_text());
                        }
                    } else {
                        text.push_str(self.current_text());
                        self.advance();
                    }
                }
                _ => {
                    // Use raw source text to preserve newlines in verbatim content
                    // (Token::text() returns " " for Space tokens, losing newlines)
                    let tok = self.current();
                    let start = tok.pos as usize;
                    let end = (start + tok.len as usize).min(self.source.len());
                    text.push_str(&self.source[start..end]);
                    self.advance();
                }
            }
        }
        Ok(())
    }

    fn parse_math_until_dollar(&mut self) -> Result<Vec<MathNode>> {
        let mut nodes = Vec::new();
        loop {
            match self.current().kind {
                TokenKind::Dollar => { self.advance(); break; }
                TokenKind::Eof => bail!("Unexpected end in inline math"),
                _ => {
                    if let Some(node) = self.parse_math_node()? {
                        nodes.push(node);
                    }
                }
            }
        }
        Ok(nodes)
    }

    fn parse_math_until_double_dollar(&mut self) -> Result<Vec<MathNode>> {
        let mut nodes = Vec::new();
        loop {
            match self.current().kind {
                TokenKind::DoubleDollar => { self.advance(); break; }
                TokenKind::Eof => bail!("Unexpected end in display math"),
                _ => {
                    if let Some(node) = self.parse_math_node()? {
                        nodes.push(node);
                    }
                }
            }
        }
        Ok(nodes)
    }

    fn parse_math_node(&mut self) -> Result<Option<MathNode>> {
        match self.current().kind {
            TokenKind::Eof => Ok(None),
            TokenKind::Space | TokenKind::ParBreak => {
                self.advance();
                Ok(Some(MathNode::Space(3.0)))
            }
            TokenKind::Comment => { self.advance(); Ok(None) }
            TokenKind::Text => {
                let text = self.current().text(self.source).to_string();
                self.advance();
                // Parse individual characters
                let mut nodes = Vec::new();
                for ch in text.chars() {
                    if ch.is_ascii_digit() || ch == '.' {
                        // Accumulate number
                        nodes.push(MathNode::Number(ch.to_string()));
                    } else if ch.is_ascii_alphabetic() {
                        nodes.push(MathNode::Variable(ch));
                    } else {
                        match ch {
                            '+' | '-' | '*' | '/' | '=' | '<' | '>' | '!' | ',' | ';' | ':' | '(' | ')' => {
                                nodes.push(MathNode::Operator(ch.to_string()));
                            }
                            _ => {
                                nodes.push(MathNode::Text(ch.to_string()));
                            }
                        }
                    }
                }
                if nodes.len() == 1 {
                    Ok(Some(nodes.remove(0)))
                } else {
                    Ok(Some(MathNode::Group(nodes)))
                }
            }
            TokenKind::Caret => {
                self.advance();
                let sup = self.parse_math_arg()?;
                Ok(Some(MathNode::Super(sup)))
            }
            TokenKind::Underscore => {
                self.advance();
                let sub = self.parse_math_arg()?;
                Ok(Some(MathNode::Sub(sub)))
            }
            TokenKind::OpenBrace => {
                self.advance();
                let mut nodes = Vec::new();
                let mut depth = 1;
                while depth > 0 {
                    match self.current().kind {
                        TokenKind::OpenBrace => { depth += 1; self.advance(); }
                        TokenKind::CloseBrace => {
                            depth -= 1;
                            self.advance();
                        }
                        TokenKind::Eof => break,
                        _ => {
                            if let Some(n) = self.parse_math_node()? {
                                nodes.push(n);
                            }
                        }
                    }
                }
                Ok(Some(MathNode::Group(nodes)))
            }
            TokenKind::CloseBrace => {
                self.advance();
                Ok(None)
            }
            TokenKind::Ampersand => {
                self.advance();
                Ok(Some(MathNode::Space(10.0)))
            }
            TokenKind::DoubleBackslash => {
                self.advance();
                Ok(Some(MathNode::Space(0.0))) // line break in math
            }
            TokenKind::Command => {
                let cmd = self.current().text(self.source).to_string();
                self.advance();
                self.parse_math_command(&cmd)
            }
            _ => {
                let text = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::Text(text)))
            }
        }
    }

    fn parse_math_arg(&mut self) -> Result<Vec<MathNode>> {
        self.skip_whitespace_and_comments();
        match self.current().kind {
            TokenKind::OpenBrace => {
                self.advance();
                let mut nodes = Vec::new();
                let mut depth = 1;
                while depth > 0 {
                    match self.current().kind {
                        TokenKind::OpenBrace => { depth += 1; self.advance(); }
                        TokenKind::CloseBrace => { depth -= 1; self.advance(); }
                        TokenKind::Eof => break,
                        _ => {
                            if let Some(n) = self.parse_math_node()? {
                                nodes.push(n);
                            }
                        }
                    }
                }
                Ok(nodes)
            }
            _ => {
                // Single token
                if let Some(n) = self.parse_math_node()? {
                    Ok(vec![n])
                } else {
                    Ok(vec![])
                }
            }
        }
    }

    fn parse_math_command(&mut self, cmd: &str) -> Result<Option<MathNode>> {
        match cmd {
            "\\frac" | "\\dfrac" | "\\tfrac" => {
                let numer = self.parse_math_arg()?;
                let denom = self.parse_math_arg()?;
                Ok(Some(MathNode::Frac { numer, denom }))
            }
            "\\sqrt" => {
                let index = if self.current().kind == TokenKind::OpenBracket {
                    let opt = self.try_read_optional_arg();
                    opt.map(|s| vec![MathNode::Text(s)])
                } else {
                    None
                };
                let radicand = self.parse_math_arg()?;
                Ok(Some(MathNode::Sqrt { index, radicand }))
            }
            "\\sum" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Sum { lower, upper }))
            }
            "\\int" | "\\oint" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Integral { lower, upper }))
            }
            "\\prod" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Product { lower, upper }))
            }
            "\\left" => {
                let delim = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::Left(delim)))
            }
            "\\right" => {
                let delim = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::Right(delim)))
            }
            "\\text" | "\\textrm" | "\\mathrm" | "\\textit" | "\\mathit"
            | "\\textbf" | "\\mathbf" | "\\texttt" | "\\mathtt"
            | "\\textsf" | "\\mathsf" | "\\mbox" | "\\hbox" => {
                let text = self.read_braced_text()?;
                Ok(Some(MathNode::Text(text)))
            }
            "\\hat" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Hat }))
            }
            "\\tilde" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Tilde }))
            }
            "\\bar" | "\\overline" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Bar }))
            }
            "\\vec" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Vec }))
            }
            "\\dot" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Dot }))
            }
            "\\ddot" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::DDot }))
            }

            // Greek letters
            "\\alpha" => Ok(Some(MathNode::Symbol("\u{03B1}".to_string()))),
            "\\beta" => Ok(Some(MathNode::Symbol("\u{03B2}".to_string()))),
            "\\gamma" => Ok(Some(MathNode::Symbol("\u{03B3}".to_string()))),
            "\\delta" => Ok(Some(MathNode::Symbol("\u{03B4}".to_string()))),
            "\\epsilon" | "\\varepsilon" => Ok(Some(MathNode::Symbol("\u{03B5}".to_string()))),
            "\\zeta" => Ok(Some(MathNode::Symbol("\u{03B6}".to_string()))),
            "\\eta" => Ok(Some(MathNode::Symbol("\u{03B7}".to_string()))),
            "\\theta" | "\\vartheta" => Ok(Some(MathNode::Symbol("\u{03B8}".to_string()))),
            "\\iota" => Ok(Some(MathNode::Symbol("\u{03B9}".to_string()))),
            "\\kappa" => Ok(Some(MathNode::Symbol("\u{03BA}".to_string()))),
            "\\lambda" => Ok(Some(MathNode::Symbol("\u{03BB}".to_string()))),
            "\\mu" => Ok(Some(MathNode::Symbol("\u{03BC}".to_string()))),
            "\\nu" => Ok(Some(MathNode::Symbol("\u{03BD}".to_string()))),
            "\\xi" => Ok(Some(MathNode::Symbol("\u{03BE}".to_string()))),
            "\\pi" | "\\varpi" => Ok(Some(MathNode::Symbol("\u{03C0}".to_string()))),
            "\\rho" | "\\varrho" => Ok(Some(MathNode::Symbol("\u{03C1}".to_string()))),
            "\\sigma" | "\\varsigma" => Ok(Some(MathNode::Symbol("\u{03C3}".to_string()))),
            "\\tau" => Ok(Some(MathNode::Symbol("\u{03C4}".to_string()))),
            "\\upsilon" => Ok(Some(MathNode::Symbol("\u{03C5}".to_string()))),
            "\\phi" | "\\varphi" => Ok(Some(MathNode::Symbol("\u{03C6}".to_string()))),
            "\\chi" => Ok(Some(MathNode::Symbol("\u{03C7}".to_string()))),
            "\\psi" => Ok(Some(MathNode::Symbol("\u{03C8}".to_string()))),
            "\\omega" => Ok(Some(MathNode::Symbol("\u{03C9}".to_string()))),
            "\\Gamma" => Ok(Some(MathNode::Symbol("\u{0393}".to_string()))),
            "\\Delta" => Ok(Some(MathNode::Symbol("\u{0394}".to_string()))),
            "\\Theta" => Ok(Some(MathNode::Symbol("\u{0398}".to_string()))),
            "\\Lambda" => Ok(Some(MathNode::Symbol("\u{039B}".to_string()))),
            "\\Xi" => Ok(Some(MathNode::Symbol("\u{039E}".to_string()))),
            "\\Pi" => Ok(Some(MathNode::Symbol("\u{03A0}".to_string()))),
            "\\Sigma" => Ok(Some(MathNode::Symbol("\u{03A3}".to_string()))),
            "\\Phi" => Ok(Some(MathNode::Symbol("\u{03A6}".to_string()))),
            "\\Psi" => Ok(Some(MathNode::Symbol("\u{03A8}".to_string()))),
            "\\Omega" => Ok(Some(MathNode::Symbol("\u{03A9}".to_string()))),

            // Math operators/symbols
            "\\times" => Ok(Some(MathNode::Operator("\u{00D7}".to_string()))),
            "\\div" => Ok(Some(MathNode::Operator("\u{00F7}".to_string()))),
            "\\pm" => Ok(Some(MathNode::Operator("\u{00B1}".to_string()))),
            "\\mp" => Ok(Some(MathNode::Operator("\u{2213}".to_string()))),
            "\\cdot" => Ok(Some(MathNode::Operator("\u{00B7}".to_string()))),
            "\\leq" | "\\le" => Ok(Some(MathNode::Operator("\u{2264}".to_string()))),
            "\\geq" | "\\ge" => Ok(Some(MathNode::Operator("\u{2265}".to_string()))),
            "\\neq" | "\\ne" => Ok(Some(MathNode::Operator("\u{2260}".to_string()))),
            "\\approx" => Ok(Some(MathNode::Operator("\u{2248}".to_string()))),
            "\\equiv" => Ok(Some(MathNode::Operator("\u{2261}".to_string()))),
            "\\sim" => Ok(Some(MathNode::Operator("~".to_string()))),
            "\\in" => Ok(Some(MathNode::Operator("\u{2208}".to_string()))),
            "\\notin" => Ok(Some(MathNode::Operator("\u{2209}".to_string()))),
            "\\subset" => Ok(Some(MathNode::Operator("\u{2282}".to_string()))),
            "\\supset" => Ok(Some(MathNode::Operator("\u{2283}".to_string()))),
            "\\subseteq" => Ok(Some(MathNode::Operator("\u{2286}".to_string()))),
            "\\supseteq" => Ok(Some(MathNode::Operator("\u{2287}".to_string()))),
            "\\cup" => Ok(Some(MathNode::Operator("\u{222A}".to_string()))),
            "\\cap" => Ok(Some(MathNode::Operator("\u{2229}".to_string()))),
            "\\forall" => Ok(Some(MathNode::Operator("\u{2200}".to_string()))),
            "\\exists" => Ok(Some(MathNode::Operator("\u{2203}".to_string()))),
            "\\nabla" => Ok(Some(MathNode::Symbol("\u{2207}".to_string()))),
            "\\partial" => Ok(Some(MathNode::Symbol("\u{2202}".to_string()))),
            "\\infty" => Ok(Some(MathNode::Symbol("\u{221E}".to_string()))),
            "\\to" | "\\rightarrow" => Ok(Some(MathNode::Operator("\u{2192}".to_string()))),
            "\\leftarrow" => Ok(Some(MathNode::Operator("\u{2190}".to_string()))),
            "\\Rightarrow" => Ok(Some(MathNode::Operator("\u{21D2}".to_string()))),
            "\\Leftarrow" => Ok(Some(MathNode::Operator("\u{21D0}".to_string()))),
            "\\leftrightarrow" => Ok(Some(MathNode::Operator("\u{2194}".to_string()))),
            "\\Leftrightarrow" | "\\iff" => Ok(Some(MathNode::Operator("\u{21D4}".to_string()))),
            "\\ldots" | "\\dots" | "\\cdots" => Ok(Some(MathNode::Symbol("\u{2026}".to_string()))),
            "\\prime" => Ok(Some(MathNode::Symbol("\u{2032}".to_string()))),
            "\\emptyset" | "\\varnothing" => Ok(Some(MathNode::Symbol("\u{2205}".to_string()))),
            "\\angle" => Ok(Some(MathNode::Symbol("\u{2220}".to_string()))),
            "\\circ" => Ok(Some(MathNode::Symbol("\u{00B0}".to_string()))),  // degree symbol
            "\\degree" => Ok(Some(MathNode::Symbol("\u{00B0}".to_string()))),

            // Math functions
            "\\sin" | "\\cos" | "\\tan" | "\\cot" | "\\sec" | "\\csc"
            | "\\arcsin" | "\\arccos" | "\\arctan"
            | "\\sinh" | "\\cosh" | "\\tanh" | "\\coth"
            | "\\log" | "\\ln" | "\\lg" | "\\exp"
            | "\\min" | "\\max" | "\\sup" | "\\inf"
            | "\\lim" | "\\limsup" | "\\liminf"
            | "\\det" | "\\dim" | "\\ker" | "\\gcd"
            | "\\deg" | "\\hom" | "\\arg" => {
                Ok(Some(MathNode::Function(cmd[1..].to_string())))
            }

            // Spacing in math
            "\\quad" => Ok(Some(MathNode::Space(18.0))),
            "\\qquad" => Ok(Some(MathNode::Space(36.0))),
            "\\," => Ok(Some(MathNode::Space(3.0))),
            "\\;" => Ok(Some(MathNode::Space(5.0))),
            "\\:" => Ok(Some(MathNode::Space(4.0))),
            "\\!" => Ok(Some(MathNode::Space(-3.0))),

            // Delimiters
            "\\langle" => Ok(Some(MathNode::Operator("\u{27E8}".to_string()))),
            "\\rangle" => Ok(Some(MathNode::Operator("\u{27E9}".to_string()))),
            "\\lfloor" => Ok(Some(MathNode::Operator("\u{230A}".to_string()))),
            "\\rfloor" => Ok(Some(MathNode::Operator("\u{230B}".to_string()))),
            "\\lceil" => Ok(Some(MathNode::Operator("\u{2308}".to_string()))),
            "\\rceil" => Ok(Some(MathNode::Operator("\u{2309}".to_string()))),

            "\\label" | "\\tag" | "\\notag" | "\\nonumber" => {
                self.skip_command_args();
                Ok(None)
            }

            _ => {
                // Unknown math command, render as text
                Ok(Some(MathNode::Text(cmd[1..].to_string())))
            }
        }
    }

    fn parse_limits(&mut self) -> Result<(Option<Vec<MathNode>>, Option<Vec<MathNode>>)> {
        let mut lower = None;
        let mut upper = None;

        // Look for _ and ^
        loop {
            self.skip_whitespace_and_comments();
            match self.current().kind {
                TokenKind::Underscore => {
                    self.advance();
                    lower = Some(self.parse_math_arg()?);
                }
                TokenKind::Caret => {
                    self.advance();
                    upper = Some(self.parse_math_arg()?);
                }
                TokenKind::Command => {
                    let cmd = self.current().text(self.source).to_string();
                    if cmd == "\\limits" || cmd == "\\nolimits" {
                        self.advance();
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }

        Ok((lower, upper))
    }
}
