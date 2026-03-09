use anyhow::{Result, bail};
use crate::lexer::{Token, TokenKind, cmd_id};
use crate::document::*;
use crate::color::Color;

mod preamble;
mod dimensions;
mod body;
mod environments;
mod lists;
mod tables;
mod algorithms;
mod math;


pub struct Parser<'a> {
    pub(super) tokens: Vec<Token>,
    pub(super) source: &'a str,
    pub(super) pos: usize,
    pub(super) section_counters: [u32; 7],
    // Body-time title/author (for amsart where these appear after \begin{document})
    pub(super) body_title: Option<String>,
    pub(super) body_authors: Vec<String>,
    pub(super) body_addresses: Vec<(String, Option<String>)>, // (address, email)
    pub(super) body_date: Option<String>,
    pub(super) body_keywords: Option<String>,
    pub(super) body_subjclass: Option<(String, String)>, // (year, text)
    pub(super) custom_colors: std::collections::HashMap<String, Color>,
    pub(super) base_font_size: f32, // for em/ex unit conversion
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
            body_title: None,
            body_authors: Vec::new(),
            body_addresses: Vec::new(),
            body_date: None,
            body_keywords: None,
            body_subjclass: None,
            custom_colors: std::collections::HashMap::new(),
            base_font_size: 10.0, // default, updated from preamble
        }
    }

    /// Resolve a color name, checking custom_colors first, then built-in names.
    pub(super) fn resolve_color(&self, name: &str) -> Option<Color> {
        self.custom_colors.get(name).copied().or_else(|| Color::from_name(name))
    }

    pub fn parse(&mut self) -> Result<Document> {
        self.skip_whitespace_and_comments();
        let class = self.parse_document_class()?;
        // Set base_font_size from document class options for em/ex unit conversion
        for opt in &class.options {
            match opt.as_str() {
                "10pt" => self.base_font_size = 10.0,
                "11pt" => self.base_font_size = 11.0,
                "12pt" => self.base_font_size = 12.0,
                _ => {}
            }
        }
        let mut preamble = self.parse_preamble()?;
        let body = self.parse_body()?;
        // Apply body-time title/author (amsart places these after \begin{document})
        if preamble.title.is_none() {
            if let Some(t) = self.body_title.take() {
                preamble.title = Some(t);
            }
        }
        if preamble.author.is_none() && !self.body_authors.is_empty() {
            preamble.author = Some(self.body_authors.join(" and "));
        }
        // Transfer addresses
        for (addr, email) in std::mem::take(&mut self.body_addresses) {
            preamble.addresses.push(crate::document::AuthorAddress { address: addr, email });
        }
        // Transfer body-time date (amsart places \date after \begin{document})
        if preamble.date.is_none() {
            if let Some(d) = self.body_date.take() {
                preamble.date = Some(d);
            }
        }
        // Transfer keywords/subjclass
        if let Some(kw) = self.body_keywords.take() {
            preamble.keywords = Some(kw);
        }
        if let Some(sc) = self.body_subjclass.take() {
            preamble.subjclass = Some(sc);
        }
        Ok(Document { class, preamble, body })
    }

    #[inline(always)]
    pub(super) fn current(&self) -> Token {
        // SAFETY: sentinel EOF token guarantees self.pos is always valid
        unsafe { *self.tokens.get_unchecked(self.pos) }
    }

    pub(super) fn peek(&self) -> Token {
        self.current()
    }

    #[inline(always)]
    pub(super) fn advance(&mut self) -> Token {
        // SAFETY: sentinel EOF token guarantees self.pos is always valid
        let tok = unsafe { *self.tokens.get_unchecked(self.pos) };
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    pub(super) fn token_text(&self, token: Token) -> &'a str {
        token.text(self.source)
    }

    /// Get text of current token without allocating
    pub(super) fn current_text(&self) -> &'a str {
        self.current().text(self.source)
    }

    pub(super) fn skip_whitespace_and_comments(&mut self) {
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

    pub(super) fn expect_open_brace(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        match self.current().kind {
            TokenKind::OpenBrace => { self.advance(); Ok(()) }
            _ => bail!("Expected '{{', got {:?}", self.current()),
        }
    }

    pub(super) fn expect_close_brace(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        match self.current().kind {
            TokenKind::CloseBrace => { self.advance(); Ok(()) }
            _ => bail!("Expected '}}', got {:?}", self.current()),
        }
    }

    /// Read accent target: braced group or single unbraced letter, or empty
    pub(super) fn read_accent_char(&mut self) -> String {
        self.skip_whitespace_and_comments();
        if self.current().kind == TokenKind::OpenBrace {
            self.read_braced_text().unwrap_or_default()
        } else if self.current().kind == TokenKind::Text {
            let text = self.current_text();
            let first_char = text.chars().next().unwrap();
            let char_len = first_char.len_utf8();
            if text.len() as u16 <= char_len as u16 {
                // Single char token: consume entirely
                self.advance();
            } else {
                // Multi-char text: split off first char, leave rest
                let tok = self.current();
                self.tokens[self.pos] = Token {
                    kind: TokenKind::Text,
                    cmd: 0,
                    pos: tok.pos + char_len as u32,
                    len: tok.len - char_len as u16,
                };
            }
            first_char.to_string()
        } else {
            String::new()
        }
    }

    /// Like read_braced_text but returns None if no opening brace is found
    /// (instead of erroring). Used for commands like \ref, \label, \cite
    /// that should degrade gracefully during live editing.
    pub(super) fn try_read_braced_text(&mut self) -> Option<String> {
        self.skip_whitespace_and_comments();
        if self.current().kind != TokenKind::OpenBrace {
            return None;
        }
        self.read_braced_text().ok()
    }

    pub(super) fn read_braced_text(&mut self) -> Result<String> {
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

    pub(super) fn read_braced_nodes(&mut self) -> Result<Vec<Node>> {
        self.expect_open_brace()?;
        self.parse_nodes_until_close_brace()
    }

    pub(super) fn parse_nodes_until_close_brace(&mut self) -> Result<Vec<Node>> {
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

    pub(super) fn try_read_optional_arg(&mut self) -> Option<String> {
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

    /// Read optional bracket-delimited content as math nodes (for \xrightarrow[below]{above})
    pub(super) fn try_read_optional_math_arg(&mut self) -> Option<Vec<MathNode>> {
        self.skip_whitespace_and_comments();
        if self.current().kind == TokenKind::OpenBracket {
            self.advance();
            let mut nodes = Vec::new();
            let mut depth = 1;
            loop {
                match self.current().kind {
                    TokenKind::OpenBracket => { depth += 1; self.advance(); }
                    TokenKind::CloseBracket => {
                        depth -= 1;
                        if depth == 0 { self.advance(); break; }
                        self.advance();
                    }
                    TokenKind::Eof => break,
                    _ => {
                        if let Ok(Some(node)) = self.parse_math_node() {
                            nodes.push(node);
                        } else {
                            self.advance();
                        }
                    }
                }
            }
            if nodes.is_empty() { None } else { Some(nodes) }
        } else {
            None
        }
    }

    /// Read optional bracket-delimited content as parsed nodes.
    /// Used for \twocolumn[...spanning content...]
    pub(super) fn try_read_bracket_nodes(&mut self) -> Result<Vec<Node>> {
        self.skip_whitespace_and_comments();
        if self.current().kind != TokenKind::OpenBracket {
            return Ok(Vec::new());
        }
        self.advance(); // skip '['

        // Read all content until matching ']'
        let mut nodes = Vec::new();
        let mut depth = 1;
        while self.current().kind != TokenKind::Eof && depth > 0 {
            match self.current().kind {
                TokenKind::OpenBracket => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::CloseBracket => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance(); // skip ']'
                        break;
                    }
                    self.advance();
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

    pub(super) fn skip_command_args(&mut self) {
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

    /// Skip to matching \end{env_name}, consuming everything
    pub(super) fn skip_environment_body(&mut self, env_name: &str) -> Result<()> {
        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command if self.current().cmd == cmd_id::END => {
                    let _save = self.pos;
                    self.advance();
                    self.skip_whitespace_and_comments();
                    if self.current().kind == TokenKind::OpenBrace {
                        let name = self.read_braced_text()?;
                        if name == env_name {
                            return Ok(());
                        }
                    }
                    // Not the right \end, keep going (pos already advanced)
                }
                _ => { self.advance(); }
            }
        }
        Ok(())
    }

    /// Skip TeX conditional to matching \fi
    pub(super) fn skip_conditional(&mut self) {
        let mut depth = 1;
        while depth > 0 && self.pos < self.tokens.len() {
            let tok = self.current();
            if tok.kind == TokenKind::Command {
                let txt = tok.text(self.source);
                if txt == "\\fi" {
                    depth -= 1;
                    self.advance();
                    if depth == 0 { return; }
                } else if txt.starts_with("\\if") {
                    depth += 1;
                    self.advance();
                } else if txt == "\\else" {
                    // At depth 1, skip to \fi
                    self.advance();
                } else {
                    self.advance();
                }
            } else if tok.kind == TokenKind::Eof {
                break;
            } else {
                self.advance();
            }
        }
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
            | Node::FlushRight(_) | Node::Environment(_) | Node::Appendix
            | Node::TwoColumn(_) | Node::OneColumn
        )
    }
}
