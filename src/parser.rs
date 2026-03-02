use anyhow::{Result, bail};
use crate::lexer::{Token, TokenKind, cmd_id};
use crate::document::*;
use crate::color::Color;
use crate::font::FontId;

/// Expand enumitem label pattern (e.g., "(\arabic*)", "(\alph*)", "\roman*.")
fn expand_enumitem_label(pattern: &str, counter: u32) -> String {
    let mut result = pattern.to_string();
    // Replace counter format commands with the actual value
    if result.contains("\\arabic*") {
        result = result.replace("\\arabic*", &counter.to_string());
    } else if result.contains("\\alph*") {
        let ch = (b'a' + (counter - 1).min(25) as u8) as char;
        result = result.replace("\\alph*", &ch.to_string());
    } else if result.contains("\\Alph*") {
        let ch = (b'A' + (counter - 1).min(25) as u8) as char;
        result = result.replace("\\Alph*", &ch.to_string());
    } else if result.contains("\\roman*") {
        let roman = to_roman_lower(counter as usize);
        result = result.replace("\\roman*", &roman);
    } else if result.contains("\\Roman*") {
        let roman = to_roman_lower(counter as usize).to_uppercase();
        result = result.replace("\\Roman*", &roman);
    }
    // Strip any remaining backslashes from label formatting
    result = result.replace("\\textbf{", "").replace("\\textit{", "")
        .replace("\\textrm{", "").replace("}", "");
    result
}

fn to_roman_lower(mut n: usize) -> String {
    let mut s = String::new();
    for &(val, sym) in &[(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"),
        (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"),
        (5, "v"), (4, "iv"), (1, "i")] {
        while n >= val { s.push_str(sym); n -= val; }
    }
    s
}


pub struct Parser<'a> {
    tokens: Vec<Token>,
    source: &'a str,
    pos: usize,
    section_counters: [u32; 7],
    // Body-time title/author (for amsart where these appear after \begin{document})
    body_title: Option<String>,
    body_authors: Vec<String>,
    body_addresses: Vec<(String, Option<String>)>, // (address, email)
    body_date: Option<String>,
    body_keywords: Option<String>,
    body_subjclass: Option<(String, String)>, // (year, text)
    custom_colors: std::collections::HashMap<String, Color>,
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
        }
    }

    /// Resolve a color name, checking custom_colors first, then built-in names.
    fn resolve_color(&self, name: &str) -> Option<Color> {
        self.custom_colors.get(name).copied().or_else(|| Color::from_name(name))
    }

    pub fn parse(&mut self) -> Result<Document> {
        self.skip_whitespace_and_comments();
        let class = self.parse_document_class()?;
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

    /// Read accent target: braced group or single unbraced letter, or empty
    fn read_accent_char(&mut self) -> String {
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

    /// Read optional bracket-delimited content as math nodes (for \xrightarrow[below]{above})
    fn try_read_optional_math_arg(&mut self) -> Option<Vec<MathNode>> {
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
    fn try_read_bracket_nodes(&mut self) -> Result<Vec<Node>> {
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
        let mut current_theorem_style = TheoremStyle::Plain;

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
                        "\\usepackage" | "\\RequirePackage" => {
                            self.advance();
                            let options: Vec<String> = self.try_read_optional_arg()
                                .map(|s| s.split(',').map(|o| o.trim().to_string()).collect())
                                .unwrap_or_default();
                            let names_str = self.read_braced_text()?;
                            // Handle comma-separated package names: \usepackage{amsmath,amssymb}
                            for name_raw in names_str.split(',') {
                                let name = name_raw.trim().to_string();
                                if name.is_empty() { continue; }

                                // Package-specific processing
                                match name.as_str() {
                                    "geometry" => {
                                        for opt in &options {
                                            self.apply_geometry_option(opt, &mut preamble.page_setup);
                                        }
                                    }
                                    "natbib" => {
                                        // natbib: author-year by default, numbers with [numbers] option
                                        // This info will be used by bibliography resolution
                                    }
                                    "setspace" => {
                                        // setspace package options
                                        for opt in &options {
                                            match opt.as_str() {
                                                "singlespacing" => preamble.line_spacing = 1.0,
                                                "onehalfspacing" => preamble.line_spacing = 1.5,
                                                "doublespacing" => preamble.line_spacing = 2.0,
                                                _ => {}
                                            }
                                        }
                                    }
                                    "multicol" | "twocolumn" => {
                                        // These enable two-column support (already built-in)
                                    }
                                    "parskip" => {
                                        // parskip package: no paragraph indent, add vertical space between paragraphs
                                        preamble.paragraph_indent = Some(0.0);
                                        preamble.paragraph_skip = Some(6.0); // ~0.5\baselineskip
                                    }
                                    "indentfirst" => {
                                        // indent first paragraph after section headings (default in some classes)
                                        // Our implementation already indents first paragraphs, so nothing to do
                                    }
                                    "enumitem" | "enumerate" | "mdwlist" => {
                                        // List customization packages — our lists work, nothing extra needed
                                    }
                                    "babel" | "polyglossia" => {
                                        // Language support — skip gracefully
                                        // Could set hyphenation language in the future
                                    }
                                    "inputenc" | "fontenc" | "fontspec" | "unicode-math" => {
                                        // Font encoding — we use UTF-8 natively, nothing needed
                                    }
                                    "microtype" | "lmodern" | "newtxtext" | "newtxmath" | "mathptmx"
                                    | "times" | "helvet" | "palatino" | "charter" | "utopia"
                                    | "libertine" | "libertinus" | "stix" | "stix2" => {
                                        // Font/typography packages — we use Standard 14 fonts
                                    }
                                    "xcolor" => {
                                        // Color support — already built in
                                    }
                                    "caption" | "subcaption" | "floatrow" | "float" => {
                                        // Float/caption customization — our floats work
                                    }
                                    "algorithm" | "algorithm2e" | "algorithmic" | "algpseudocode" => {
                                        // Algorithm environments — render as verbatim-like
                                    }
                                    "cleveref" => {
                                        // Smart cross-references — our \ref already works
                                    }
                                    "siunitx" => {
                                        // SI units — basic support through macro expansion
                                    }
                                    _ => {}
                                }

                                preamble.packages.push(Package { name, options: options.clone() });
                            }
                        }
                        "\\title" => {
                            self.advance();
                            let title = self.read_braced_text()?;
                            preamble.title = Some(strip_thanks(&title));
                        }
                        "\\author" => {
                            self.advance();
                            let author = self.read_braced_text()?;
                            preamble.author = Some(strip_thanks(&author));
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
                        "\\newtheorem" => {
                            self.advance();
                            // \newtheorem{name}{Title} or \newtheorem{name}[counter]{Title}
                            // or \newtheorem*{name}{Title}
                            let starred = if self.current().kind == TokenKind::Text && self.current_text() == "*" {
                                self.advance();
                                true
                            } else {
                                false
                            };
                            if let Ok(env_name) = self.read_braced_text() {
                                let counter = if self.current().kind == TokenKind::OpenBracket {
                                    self.try_read_optional_arg()
                                } else {
                                    None
                                };
                                if let Ok(title) = self.read_braced_text() {
                                    // Skip optional [within] argument
                                    if self.current().kind == TokenKind::OpenBracket {
                                        let _ = self.try_read_optional_arg();
                                    }
                                    preamble.theorem_defs.push(TheoremDef {
                                        env_name,
                                        display_title: title,
                                        numbered: !starred,
                                        counter,
                                        style: current_theorem_style,
                                    });
                                }
                            }
                        }
                        "\\theoremstyle" => {
                            self.advance();
                            if let Ok(style_name) = self.read_braced_text() {
                                current_theorem_style = match style_name.as_str() {
                                    "definition" => TheoremStyle::Definition,
                                    "remark" => TheoremStyle::Remark,
                                    _ => TheoremStyle::Plain,
                                };
                            }
                        }
                        "\\pagestyle" => {
                            self.advance();
                            if let Ok(style) = self.read_braced_text() {
                                preamble.page_style = style;
                            }
                        }
                        "\\fancyhead" => {
                            self.advance();
                            let pos = self.try_read_optional_arg().unwrap_or_default();
                            let text = self.read_braced_text().unwrap_or_default();
                            let text = self.expand_fancy_placeholders(&text, &preamble);
                            match pos.to_uppercase().as_str() {
                                "L" | "LO" | "LE" | "LO,LE" | "LE,LO" => preamble.fancy_header.head_left = text,
                                "R" | "RO" | "RE" | "RO,RE" | "RE,RO" => preamble.fancy_header.head_right = text,
                                "C" | "CO" | "CE" | "CO,CE" | "CE,CO" => preamble.fancy_header.head_center = text,
                                _ => preamble.fancy_header.head_center = text,
                            }
                            if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); }
                        }
                        "\\fancyfoot" => {
                            self.advance();
                            let pos = self.try_read_optional_arg().unwrap_or_default();
                            let text = self.read_braced_text().unwrap_or_default();
                            let text = self.expand_fancy_placeholders(&text, &preamble);
                            match pos.to_uppercase().as_str() {
                                "L" | "LO" | "LE" | "LO,LE" | "LE,LO" => preamble.fancy_header.foot_left = text,
                                "R" | "RO" | "RE" | "RO,RE" | "RE,RO" => preamble.fancy_header.foot_right = text,
                                "C" | "CO" | "CE" | "CO,CE" | "CE,CO" => preamble.fancy_header.foot_center = text,
                                _ => preamble.fancy_header.foot_center = text,
                            }
                            if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); }
                        }
                        "\\lhead" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.head_left = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\chead" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.head_center = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\rhead" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.head_right = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\lfoot" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.foot_left = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\cfoot" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.foot_center = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\rfoot" => { self.advance(); let t = self.read_braced_text().unwrap_or_default(); preamble.fancy_header.foot_right = self.expand_fancy_placeholders(&t, &preamble); if preamble.page_style.is_empty() { preamble.page_style = "fancy".to_string(); } }
                        "\\renewcommand" => {
                            self.advance();
                            let cmd_name = self.read_braced_text().unwrap_or_default();
                            match cmd_name.as_str() {
                                "\\headrulewidth" => {
                                    let val = self.read_braced_text().unwrap_or_default();
                                    preamble.fancy_header.head_rule_width = self.parse_dimension(&val).unwrap_or(0.4);
                                }
                                "\\footrulewidth" => {
                                    let val = self.read_braced_text().unwrap_or_default();
                                    preamble.fancy_header.foot_rule_width = self.parse_dimension(&val).unwrap_or(0.0);
                                }
                                _ => { self.skip_command_args(); }
                            }
                        }
                        "\\setlength" => {
                            self.advance();
                            if let (Ok(name), Ok(val)) = (self.read_braced_text(), self.read_braced_text()) {
                                if let Some(pts) = self.parse_dimension(&val) {
                                    match name.trim_start_matches('\\') {
                                        "parindent" => preamble.paragraph_indent = Some(pts),
                                        "parskip" => preamble.paragraph_skip = Some(pts),
                                        "topmargin" => preamble.page_setup.margin_top = pts + 72.0, // LaTeX topmargin is offset from 1in
                                        "oddsidemargin" | "evensidemargin" => preamble.page_setup.margin_left = pts + 72.0,
                                        "textwidth" => {
                                            preamble.page_setup.margin_right = preamble.page_setup.width - preamble.page_setup.margin_left - pts;
                                        }
                                        "textheight" => {
                                            preamble.page_setup.margin_bottom = preamble.page_setup.height - preamble.page_setup.margin_top - pts;
                                        }
                                        "columnsep" => preamble.page_setup.column_sep = pts,
                                        "baselineskip" | "baselinestretch" => preamble.line_spacing = (pts / (preamble.font_size * 1.2)).max(0.8).min(3.0),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        "\\setcounter" => {
                            self.advance();
                            self.skip_command_args();
                        }
                        "\\thispagestyle"
                        | "\\newcommand" | "\\def"
                        | "\\DeclareMathOperator"
                        | "\\bibliographystyle"
                        | "\\hypersetup" | "\\lstset" | "\\graphicspath"
                        | "\\numberwithin" | "\\addtolength" => {
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

    /// Expand fancyhdr placeholders like \thepage, \leftmark, \rightmark
    fn expand_fancy_placeholders(&self, text: &str, _preamble: &Preamble) -> String {
        // These are dynamic — store as placeholder tokens that get resolved at render time
        // For now, return the text with known placeholders marked
        text.replace("\\thepage", "\x01PAGE\x01")
            .replace("\\leftmark", "\x01LEFTMARK\x01")
            .replace("\\rightmark", "\x01RIGHTMARK\x01")
            .replace("\\thesection", "\x01SECTION\x01")
            .replace("\\thesubsection", "\x01SUBSECTION\x01")
    }

    /// Read a TeX dimension without braces (e.g., after \vskip: "10pt", "-2.5mm plus 1fil")
    fn read_tex_dimension_text(&mut self) -> String {
        self.skip_whitespace_and_comments();
        let mut text = String::new();
        // Read optional sign
        if self.current().kind == TokenKind::Text || self.current().kind == TokenKind::Command {
            let t = self.current().text(self.source);
            if t == "-" || t == "+" { text.push_str(t); self.advance(); }
        }
        // Read number + unit tokens until non-dimension token
        loop {
            match self.current().kind {
                TokenKind::Text => {
                    let t = self.current().text(self.source);
                    // Part of a dimension: digits, dots, units
                    if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+')
                        || ["pt", "mm", "cm", "in", "em", "ex", "sp", "bp", "dd", "pc", "mu", "fil", "fill"].contains(&t)
                    {
                        text.push_str(t);
                        self.advance();
                    } else if t == "plus" || t == "minus" {
                        // Glue component — stop at main dimension
                        break;
                    } else {
                        break;
                    }
                }
                TokenKind::Space => {
                    // Space might separate number from unit
                    if text.is_empty() || text.chars().last().map_or(false, |c| c.is_ascii_digit() || c == '.') {
                        self.advance();
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        text
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

    /// Parse a dimension that may include \textwidth, \linewidth, \columnwidth factors.
    /// E.g. "0.48\textwidth" → 0.48 * default_textwidth, "5cm" → normal dimension.
    fn parse_dimension_with_textwidth(&self, text: &str, default_textwidth: f32) -> f32 {
        let text = text.trim();
        // Check for factor * \textwidth pattern
        for keyword in &["\\textwidth", "\\linewidth", "\\columnwidth", "\\hsize"] {
            if let Some(idx) = text.find(keyword) {
                let factor_str = text[..idx].trim();
                let factor: f32 = if factor_str.is_empty() {
                    1.0
                } else {
                    factor_str.parse().unwrap_or(1.0)
                };
                return factor * default_textwidth;
            }
        }
        self.parse_dimension(text).unwrap_or(300.0)
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

    /// Skip to matching \end{env_name}, consuming everything
    fn skip_environment_body(&mut self, env_name: &str) -> Result<()> {
        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command if self.current().cmd == cmd_id::END => {
                    let save = self.pos;
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
    fn skip_conditional(&mut self) {
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
            | Node::FlushRight(_) | Node::Environment(_) | Node::Appendix
            | Node::TwoColumn(_) | Node::OneColumn
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
                cmd_id::CENTERING => Ok(None),
                cmd_id::HLINE => { self.skip_command_args(); Ok(Some(Node::HRule)) }
                cmd_id::LABEL => { let l = self.read_braced_text()?; Ok(Some(Node::Label(l))) }
                cmd_id::REF => { let l = self.read_braced_text()?; Ok(Some(Node::Ref(l))) }
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
                cmd_id::URL => { let url = self.read_braced_text()?; Ok(Some(Node::Href { url: url.clone(), content: vec![Node::Monospace(vec![Node::Text(url)])] })) }
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
            "\\textrm" | "\\textnormal" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Group(n))) }
            "\\textsf" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Group(n))) }
            "\\textsl" => { let n = self.read_braced_nodes()?; Ok(Some(Node::Italic(n))) }

            // Style switches — change font for subsequent text in scope
            "\\bf" | "\\bfseries" => Ok(Some(Node::FontStyleDecl(FontDeclType::Bold))),
            "\\it" | "\\itshape" | "\\sl" | "\\slshape" => Ok(Some(Node::FontStyleDecl(FontDeclType::Italic))),
            "\\tt" | "\\ttfamily" => Ok(Some(Node::FontStyleDecl(FontDeclType::Monospace))),
            "\\rm" | "\\rmfamily" | "\\sf" | "\\sffamily" | "\\normalfont" => Ok(Some(Node::FontStyleDecl(FontDeclType::Regular))),
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
                // \num{number} — format number with proper separators
                let _ = self.try_read_optional_arg();
                let num = self.read_braced_text()?;
                if let Some(exp_idx) = num.find('e').or_else(|| num.find('E')) {
                    let mantissa = &num[..exp_idx];
                    let exponent = &num[exp_idx+1..];
                    // Render as mantissa × 10^exponent
                    Ok(Some(Node::InlineMath(vec![
                        MathNode::Number(mantissa.to_string()),
                        MathNode::Space(2.0),
                        MathNode::Symbol("\u{00D7}".to_string()), // ×
                        MathNode::Space(2.0),
                        MathNode::Number("10".to_string()),
                        MathNode::Super(vec![MathNode::Number(exponent.to_string())]),
                    ])))
                } else {
                    Ok(Some(Node::Text(num)))
                }
            }

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
            "\\ " => Ok(Some(Node::Text(" ".to_string()))), // explicit inter-word space
            "\\hfill" | "\\dotfill" | "\\hrulefill" => Ok(Some(Node::HFill)),
            "\\vfill" => Ok(Some(Node::VSpace(200.0))),
            "\\phantom" | "\\hphantom" => {
                // Invisible space estimated from content text
                let text = self.read_braced_text()?;
                // Rough estimate: ~5pt per character at 10pt font size
                let w = text.trim().len() as f32 * 5.0;
                Ok(Some(Node::HSpace(w)))
            }
            "\\vphantom" => {
                let _content = self.read_braced_nodes()?;
                Ok(None) // vertical phantom — no horizontal space, just affects line height
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
            "\\mbox" | "\\makebox" => {
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
            "\\smallskip" | "\\smallbreak" => Ok(Some(Node::VSpace(3.0))),
            "\\medskip" | "\\medbreak" => Ok(Some(Node::VSpace(6.0))),
            "\\bigskip" | "\\bigbreak" => Ok(Some(Node::VSpace(12.0))),

            // Breaks
            "\\newline" | "\\linebreak" => Ok(Some(Node::LineBreak)),
            "\\clearpage" | "\\cleardoublepage" | "\\pagebreak" | "\\newpage" => Ok(Some(Node::PageBreak)),
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
            "\\kern" => {
                let dim_text = self.read_tex_dimension_text();
                let pts = self.parse_dimension(&dim_text).unwrap_or(0.0);
                Ok(Some(Node::HSpace(pts)))
            }
            "\\appendix" => Ok(Some(Node::Appendix)),
            "\\indent" => Ok(Some(Node::HSpace(20.0))),
            "\\marginpar" => {
                // Skip margin notes — they need margin space that may not exist
                self.skip_command_args();
                Ok(None)
            }

            // Rules
            "\\hrule" => { self.skip_command_args(); Ok(Some(Node::HRule)) }
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
                    // Inline filled rectangle
                    Ok(Some(Node::HRule))
                }
            }

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
            "\\\\" => Ok(Some(Node::Backslash)),
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
            "\\mbox" | "\\makebox" | "\\hbox" | "\\vbox" => {
                self.try_read_optional_arg();
                self.try_read_optional_arg(); // makebox has [width][align]{content}
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
            "\\eqref" => {
                let label = self.read_braced_text()?;
                Ok(Some(Node::EqRef(label)))
            }
            "\\cref" => {
                let label = self.read_braced_text()?;
                Ok(Some(Node::Cref(label, false)))
            }
            "\\Cref" => {
                let label = self.read_braced_text()?;
                Ok(Some(Node::Cref(label, true)))
            }
            "\\pageref" | "\\autoref" | "\\nameref" => {
                let label = self.read_braced_text()?;
                Ok(Some(Node::Ref(label)))
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

            // Pifont \ding{n} — ZapfDingbats characters (byte code = ding number)
            "\\ding" => {
                let num_str = self.read_braced_text()?;
                if let Ok(code) = num_str.trim().parse::<u8>() {
                    Ok(Some(Node::Dingbat(code)))
                } else {
                    Ok(Some(Node::Text("?".to_string())))
                }
            }

            // No-ops
            "\\nobreak" | "\\allowbreak" | "\\relax" | "\\protect"
            | "\\sloppy" | "\\fussy" | "\\raggedright" | "\\raggedleft"
            | "\\selectfont" | "\\frenchspacing"
            | "\\nonfrenchspacing" | "\\newblock" => Ok(None),

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
                        "parskip" => return Ok(Some(Node::VSpace(pts))),
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
            "\\textbackslash" => Ok(Some(Node::Text("\\".to_string()))),
            "\\textbar" => Ok(Some(Node::Text("|".to_string()))),
            "\\textbullet" => Ok(Some(Node::Text("\u{2022}".to_string()))),
            "\\textsection" => Ok(Some(Node::Text("\u{00A7}".to_string()))),
            "\\textdagger" => Ok(Some(Node::Text("\u{2020}".to_string()))),
            "\\textdaggerdbl" => Ok(Some(Node::Text("\u{2021}".to_string()))),
            "\\textparagraph" | "\\P" => Ok(Some(Node::Text("\u{00B6}".to_string()))),
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
                // Complex — just read what we can and pass through
                self.skip_command_args();
                Ok(None)
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
            "\\ifdefined" | "\\ifx" | "\\ifnum" | "\\ifdim" | "\\ifcase" | "\\iftrue" | "\\iffalse" => {
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

            // Phantoms in text mode
            "\\phantom" | "\\vphantom" | "\\hphantom" => {
                let _content = self.read_braced_nodes()?;
                // Invisible in text — just skip
                Ok(None)
            }

            "\\allowdisplaybreaks" | "\\mathsurround" | "\\hfuzz" => { self.skip_command_args(); Ok(None) }
            "\\newcommand" | "\\renewcommand" | "\\providecommand" | "\\def" => { self.skip_command_args(); Ok(None) }
            "\\bibliography" | "\\addbibresource" => {
                let _bib_file = self.read_braced_text()?;
                // Bibliography loading happens outside the parser
                Ok(None)
            }
            "\\printbibliography" => {
                self.skip_command_args(); // skip optional [heading=...]
                // Emit a thebibliography environment node so layout renders the bibliography
                Ok(Some(Node::Environment(Box::new(EnvironmentData {
                    name: "thebibliography".to_string(),
                    args: vec![],
                    content: vec![],
                }))))
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

            "\\verb" | "\\verb*" => {
                // \verb|code| — read delimiter char, then content until matching delimiter
                let tok = self.tokens[self.pos.saturating_sub(1)]; // the \verb token
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
            "tabular" | "tabular*" | "array" | "longtable" | "longtable*"
            | "tabularx" | "tabulary" | "supertabular" => {
                // longtable/tabularx/tabulary/supertabular — treat like tabular
                // tabular*, tabularx, tabulary have an extra {width} argument before {colspec}
                if env_name == "tabular*" || env_name.starts_with("tabularx") || env_name.starts_with("tabulary") {
                    let _ = self.read_braced_text(); // skip width arg
                }
                self.parse_tabular_environment(&env_name)
            }
            "table" | "table*" => self.parse_float_environment(&env_name, true),
            "figure" | "figure*" => self.parse_float_environment(&env_name, false),
            "algorithm" | "algorithm*" => self.parse_algorithm_float(&env_name),
            "algorithmic" | "algpseudocode" => {
                let (lines, line_numbered) = self.parse_algorithmic_body(&env_name)?;
                Ok(Some(Node::Algorithm { caption: None, label: None, content: lines, line_numbered }))
            }
            "equation" | "equation*" | "align" | "align*"
            | "gather" | "gather*" | "multline" | "multline*"
            | "flalign" | "flalign*" | "alignat" | "alignat*"
            | "eqnarray" | "eqnarray*" | "split" => {
                // alignat has an extra mandatory arg {num_columns} — skip it
                if env_name.starts_with("alignat") {
                    let _ = self.read_braced_text();
                }
                self.parse_display_math_environment(&env_name)
            }
            "subequations" => {
                // subequations wraps other math environments — parse contents normally
                let content = self.parse_environment_body("subequations")?;
                Ok(Some(Node::Group(content)))
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
                let width = self.parse_dimension_with_textwidth(&width_str, 345.0);
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
            "comment" => {
                // Skip all content — comment environment discards everything
                self.skip_environment_raw(&env_name)?;
                Ok(None)
            }
            "proof" => {
                let header = self.try_read_optional_arg(); // optional [Proof of ...]
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::Proof { header, content }))
            }
            "tcolorbox" | "mdframed" | "framed" | "shaded" | "shaded*" => {
                self.parse_tcolorbox_environment(&env_name)
            }
            "wrapfigure" | "wraptable" => {
                self.parse_wrapfigure_environment(&env_name)
            }
            "subfigure" | "subfloat" | "subcaptionbox" => {
                let opt = self.try_read_optional_arg();
                let width_str = self.read_braced_text().unwrap_or_default();
                let width = self.parse_dimension(&width_str).unwrap_or(0.45);
                let content = self.parse_environment_body(&env_name)?;
                let caption = opt.map(|s| vec![Node::Text(s)]);
                Ok(Some(Node::SubFigure { width, content, caption }))
            }
            "multicols" | "multicols*" => {
                let _ncols = self.read_braced_text().ok();
                let content = self.parse_environment_body(&env_name)?;
                Ok(Some(Node::TwoColumn(content)))
            }
            // Conference-specific environments to suppress
            "icmlauthorlist" => {
                // Skip all content within — handled by style file
                self.skip_environment_body(&env_name)?;
                Ok(None)
            }
            "split" => {
                // split inside equation — parse as math alignment
                self.parse_display_math_environment(&env_name)
            }
            _ => {
                // Check if this is a theorem-like environment (defined via \newtheorem)
                // We store the env_name and the body; the layout stage matches it
                // against theorem definitions.
                let opt_name = self.try_read_optional_arg();
                let content = self.parse_environment_body(&env_name)?;

                // Create a TheoremData node for possible theorem envs
                // Layout will check against preamble defs
                Ok(Some(Node::Theorem(Box::new(TheoremData {
                    env_name: env_name.clone(),
                    title: env_name.clone(), // will be overridden by layout if it matches a def
                    number: None,
                    optional_name: opt_name,
                    body: content,
                    italic_body: true,
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
        let opt = self.try_read_optional_arg();
        // Parse enumitem options
        let mut custom_label: Option<String> = None;
        let mut start_num: u32 = 1;
        if let Some(ref opt_str) = opt {
            for part in opt_str.split(',') {
                let part = part.trim();
                if let Some(label) = part.strip_prefix("label=") {
                    custom_label = Some(label.trim().to_string());
                } else if let Some(start) = part.strip_prefix("start=") {
                    start_num = start.trim().parse().unwrap_or(1);
                }
                // noitemsep/nosep just reduce spacing — handled visually
            }
        }
        let mut items = Vec::new();
        let mut current_content: Vec<Node> = Vec::new();
        let mut current_label: Option<Vec<Node>> = None;
        let mut in_item = false;
        let mut item_counter: u32 = start_num;

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
                        } else if let Some(ref pattern) = custom_label {
                            // Apply enumitem label pattern
                            let label = expand_enumitem_label(pattern, item_counter);
                            current_label = Some(vec![Node::Text(label)]);
                        }
                        item_counter += 1;
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
        let mut current_cell_rowspan: u32 = 1;
        let mut hline_before_next = false;
        let mut hline_after = false;
        let mut extra_space_next: f32 = 0.0;
        let mut cmidrule_pending: Vec<(u32, u32)> = Vec::new();

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
                                        rowspan: std::mem::replace(&mut current_cell_rowspan, 1),
                                        alignment: None,
                                    });
                                    rows.push(TableRow {
                                        cells: std::mem::take(&mut current_cells),
                                        hline_before: hline_before_next,
                                        hline_after,
                                        extra_space_before: extra_space_next,
                                        cmidrules: std::mem::take(&mut cmidrule_pending),
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
                        // hline between rows → draw before the next row AND after the previous
                        hline_before_next = true;
                        if !rows.is_empty() {
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
                        if cmd == "\\cline" || cmd == "\\cmidrule" {
                            self.advance();
                            let _trim = self.try_read_optional_arg(); // optional (lr) trim
                            if cmd == "\\cmidrule" {
                                // Also skip parenthesized trim arg like (lr)
                                if self.current().kind == TokenKind::Text {
                                    let t = self.current_text();
                                    if t.starts_with('(') { self.advance(); }
                                }
                            }
                            let range = self.read_braced_text().unwrap_or_default();
                            if let Some((start, end)) = parse_col_range(&range) {
                                cmidrule_pending.push((start, end));
                            } else {
                                hline_after = true;
                            }
                            continue;
                        }
                        if cmd == "\\multicolumn" {
                            self.advance();
                            let colspan_str = self.read_braced_text()?;
                            let align_str = self.read_braced_text()?;
                            let content = self.read_braced_nodes()?;
                            let alignment = match align_str.trim() {
                                "c" => Some(ColumnSpec::Center),
                                "r" => Some(ColumnSpec::Right),
                                "l" => Some(ColumnSpec::Left),
                                _ => None,
                            };
                            // Discard any content accumulated before \multicolumn
                            current_cell_content.clear();
                            current_cells.push(TableCell {
                                content,
                                colspan: colspan_str.parse().unwrap_or(1),
                                rowspan: std::mem::replace(&mut current_cell_rowspan, 1),
                                alignment,
                            });
                            // Skip whitespace after multicolumn, then consume the
                            // next & separator (it belongs to this cell boundary,
                            // not the next cell)
                            self.skip_whitespace_and_comments();
                            if self.current().kind == TokenKind::Ampersand {
                                self.advance();
                            }
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
                            let nrows_str = self.read_braced_text()?; // {nrows}
                            let _width = self.read_braced_text()?; // {width} or {*}
                            // Optional fixup arg [] may be present
                            let _ = self.try_read_optional_arg();
                            let content = self.read_braced_nodes()?; // {content}
                            current_cell_rowspan = nrows_str.parse::<u32>().unwrap_or(1);
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
                        rowspan: std::mem::replace(&mut current_cell_rowspan, 1),
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
                        rowspan: std::mem::replace(&mut current_cell_rowspan, 1),
                        alignment: None,
                    });
                    rows.push(TableRow {
                        cells: std::mem::take(&mut current_cells),
                        hline_before: hline_before_next,
                        hline_after,
                        extra_space_before: extra_space_next,
                        cmidrules: std::mem::take(&mut cmidrule_pending),
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
                'X' => { cols.push(ColumnSpec::Left); i += 1; } // tabularx X column → Left (auto-width)
                'm' | 'b' => {
                    // m{width} and b{width} — like p{width}
                    i += 1;
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
            // Must search recursively — tabular may be wrapped in \resizebox/Group nodes
            fn set_table_caption_label(nodes: &mut [Node], caption: &Option<Vec<Node>>, label: &Option<String>) -> bool {
                for node in nodes.iter_mut() {
                    match node {
                        Node::Table(ref mut tbl) => {
                            tbl.caption = caption.clone();
                            tbl.label = label.clone();
                            return true;
                        }
                        Node::Group(children) => {
                            if set_table_caption_label(children, caption, label) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
                false
            }
            set_table_caption_label(&mut content, &caption, &label);
            // Wrap in figure-like structure for layout but WITHOUT caption
            // (the Table node already has its own caption — avoid double caption)
            Ok(Some(Node::Figure(Box::new(FigureData {
                content,
                caption: None,
                label,
                placement,
                starred: env_name.ends_with('*'),
            }))))
        } else {
            Ok(Some(Node::Figure(Box::new(FigureData {
                content,
                caption,
                label,
                placement,
                starred: env_name.ends_with('*'),
            }))))
        }
    }

    fn parse_algorithm_float(&mut self, env_name: &str) -> Result<Option<Node>> {
        let _placement = self.try_read_optional_arg();
        let mut caption = None;
        let mut label = None;
        let mut algo_lines = Vec::new();
        let mut algo_line_numbered = false;

        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command => {
                    let cmd = self.current().text(self.source);
                    if cmd == "\\end" {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name { break; }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    let cmd = self.current().text(self.source);
                    match cmd {
                        "\\caption" => {
                            self.advance();
                            caption = Some(self.read_braced_text()?);
                        }
                        "\\label" => {
                            self.advance();
                            label = Some(self.read_braced_text()?);
                        }
                        "\\begin" => {
                            self.advance();
                            let inner_env = self.read_braced_text()?;
                            match inner_env.as_str() {
                                "algorithmic" | "algpseudocode" => {
                                    let (lines, numbered) = self.parse_algorithmic_body(&inner_env)?;
                                    algo_lines = lines;
                                    algo_line_numbered = numbered;
                                }
                                _ => {
                                    // Skip unknown inner environments
                                    let _ = self.parse_environment_body(&inner_env);
                                }
                            }
                        }
                        _ => { self.advance(); self.skip_command_args(); }
                    }
                }
                _ => { self.advance(); }
            }
        }
        Ok(Some(Node::Algorithm { caption, label, content: algo_lines, line_numbered: algo_line_numbered }))
    }

    fn parse_algorithmic_body(&mut self, env_name: &str) -> Result<(Vec<AlgoLine>, bool)> {
        let opt = self.try_read_optional_arg(); // e.g. [1] for line numbering
        let line_numbered = opt.as_ref().map_or(false, |o| o.trim() == "1");
        let mut lines: Vec<AlgoLine> = Vec::new();
        let mut indent: u32 = 0;

        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command => {
                    let cmd = self.current().text(self.source);
                    if cmd == "\\end" {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name { break; }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    let cmd = self.current().text(self.source);
                    match cmd {
                        "\\State" | "\\STATE" => {
                            self.advance();
                            let tokens = self.read_algo_line_content();
                            lines.push(AlgoLine { indent, content: tokens });
                        }
                        "\\If" | "\\IF" => {
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("if".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {} ", cond)));
                            tokens.push(AlgoToken::Keyword("then".to_string()));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\ElsIf" | "\\ELSIF" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("else if".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {} ", cond)));
                            tokens.push(AlgoToken::Keyword("then".to_string()));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\Else" | "\\ELSE" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword("else".to_string())] });
                            indent += 1;
                        }
                        "\\EndIf" | "\\ENDIF" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword("end if".to_string())] });
                        }
                        "\\For" | "\\FOR" => {
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("for".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {} ", cond)));
                            tokens.push(AlgoToken::Keyword("do".to_string()));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\ForAll" | "\\FORALL" => {
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("for all".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {} ", cond)));
                            tokens.push(AlgoToken::Keyword("do".to_string()));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\EndFor" | "\\ENDFOR" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword("end for".to_string())] });
                        }
                        "\\While" | "\\WHILE" => {
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("while".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {} ", cond)));
                            tokens.push(AlgoToken::Keyword("do".to_string()));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\EndWhile" | "\\ENDWHILE" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword("end while".to_string())] });
                        }
                        "\\Repeat" | "\\REPEAT" => {
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword("repeat".to_string())] });
                            indent += 1;
                        }
                        "\\Until" | "\\UNTIL" => {
                            if indent > 0 { indent -= 1; }
                            self.advance();
                            let cond = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword("until".to_string())];
                            tokens.push(AlgoToken::Text(format!(" {}", cond)));
                            lines.push(AlgoLine { indent, content: tokens });
                        }
                        "\\Return" | "\\RETURN" => {
                            self.advance();
                            let mut tokens = vec![AlgoToken::Keyword("return".to_string())];
                            let rest = self.read_algo_line_content();
                            if !rest.is_empty() {
                                tokens.push(AlgoToken::Text(" ".to_string()));
                                tokens.extend(rest);
                            }
                            lines.push(AlgoLine { indent, content: tokens });
                        }
                        "\\Require" | "\\REQUIRE" => {
                            self.advance();
                            let mut tokens = vec![AlgoToken::Keyword("Require:".to_string())];
                            let rest = self.read_algo_line_content();
                            if !rest.is_empty() {
                                tokens.push(AlgoToken::Text(" ".to_string()));
                                tokens.extend(rest);
                            }
                            lines.push(AlgoLine { indent: 0, content: tokens });
                        }
                        "\\Ensure" | "\\ENSURE" => {
                            self.advance();
                            let mut tokens = vec![AlgoToken::Keyword("Ensure:".to_string())];
                            let rest = self.read_algo_line_content();
                            if !rest.is_empty() {
                                tokens.push(AlgoToken::Text(" ".to_string()));
                                tokens.extend(rest);
                            }
                            lines.push(AlgoLine { indent: 0, content: tokens });
                        }
                        "\\Procedure" | "\\Function" => {
                            let kw = if cmd == "\\Procedure" { "procedure" } else { "function" };
                            self.advance();
                            let name = self.read_braced_text().unwrap_or_default();
                            let params = self.read_braced_text().unwrap_or_default();
                            let mut tokens = vec![AlgoToken::Keyword(kw.to_string())];
                            tokens.push(AlgoToken::Text(format!(" {}({})", name, params)));
                            lines.push(AlgoLine { indent, content: tokens });
                            indent += 1;
                        }
                        "\\EndProcedure" | "\\EndFunction" => {
                            if indent > 0 { indent -= 1; }
                            let kw = if cmd == "\\EndProcedure" { "end procedure" } else { "end function" };
                            self.advance();
                            lines.push(AlgoLine { indent, content: vec![AlgoToken::Keyword(kw.to_string())] });
                        }
                        "\\Comment" | "\\COMMENT" => {
                            self.advance();
                            let text = self.read_braced_text().unwrap_or_default();
                            // Add comment as part of previous line or new line
                            if let Some(last) = lines.last_mut() {
                                last.content.push(AlgoToken::Text(format!("  // {}", text)));
                            }
                        }
                        "\\Call" => {
                            self.advance();
                            let name = self.read_braced_text().unwrap_or_default();
                            let args = self.read_braced_text().unwrap_or_default();
                            if let Some(last) = lines.last_mut() {
                                last.content.push(AlgoToken::Text(format!("{}({})", name, args)));
                            }
                        }
                        _ => {
                            self.advance();
                            self.skip_command_args();
                        }
                    }
                }
                _ => { self.advance(); }
            }
        }
        Ok((lines, line_numbered))
    }

    /// Read algorithm line content (text + math) until the next algorithmic command or \end
    fn read_algo_line_content(&mut self) -> Vec<AlgoToken> {
        let mut tokens = Vec::new();
        let mut text_buf = String::new();

        loop {
            match self.current().kind {
                TokenKind::Eof => break,
                TokenKind::Command => {
                    let cmd = self.current().text(self.source);
                    // Stop at algorithmic control commands
                    match cmd {
                        "\\State" | "\\STATE" | "\\If" | "\\IF" | "\\Else" | "\\ELSE"
                        | "\\ElsIf" | "\\ELSIF" | "\\EndIf" | "\\ENDIF"
                        | "\\For" | "\\FOR" | "\\ForAll" | "\\FORALL" | "\\EndFor" | "\\ENDFOR"
                        | "\\While" | "\\WHILE" | "\\EndWhile" | "\\ENDWHILE"
                        | "\\Repeat" | "\\REPEAT" | "\\Until" | "\\UNTIL"
                        | "\\Return" | "\\RETURN" | "\\Require" | "\\REQUIRE"
                        | "\\Ensure" | "\\ENSURE" | "\\Procedure" | "\\Function"
                        | "\\EndProcedure" | "\\EndFunction" | "\\Comment" | "\\COMMENT"
                        | "\\end" => break,
                        "\\Call" => {
                            self.advance();
                            let name = self.read_braced_text().unwrap_or_default();
                            let args = self.read_braced_text().unwrap_or_default();
                            text_buf.push_str(&format!("{}({})", name, args));
                        }
                        "\\gets" | "\\leftarrow" => {
                            self.advance();
                            text_buf.push_str(" \u{2190} ");
                        }
                        "\\textbf" | "\\textit" | "\\texttt" => {
                            self.advance();
                            let t = self.read_braced_text().unwrap_or_default();
                            text_buf.push_str(&t);
                        }
                        "\\TRUE" | "\\true" => { self.advance(); text_buf.push_str("true"); }
                        "\\FALSE" | "\\false" => { self.advance(); text_buf.push_str("false"); }
                        "\\AND" | "\\and" => { self.advance(); text_buf.push_str(" and "); }
                        "\\OR" | "\\or" => { self.advance(); text_buf.push_str(" or "); }
                        "\\NOT" | "\\not" => { self.advance(); text_buf.push_str("not "); }
                        "\\TO" | "\\to" | "\\KwTo" => { self.advance(); text_buf.push_str(" to "); }
                        "\\DOWNTO" | "\\downto" => { self.advance(); text_buf.push_str(" downto "); }
                        _ => {
                            self.advance();
                            self.skip_command_args();
                        }
                    }
                }
                TokenKind::Dollar => {
                    // Flush text before math
                    if !text_buf.is_empty() {
                        tokens.push(AlgoToken::Text(std::mem::take(&mut text_buf)));
                    }
                    self.advance();
                    if let Ok(math) = self.parse_math_until_dollar() {
                        tokens.push(AlgoToken::Math(math));
                    }
                }
                TokenKind::Text | TokenKind::Space => {
                    text_buf.push_str(self.current().text(self.source));
                    self.advance();
                }
                TokenKind::OpenBrace => {
                    self.advance();
                    // Read until close brace
                    let mut depth = 1u32;
                    while depth > 0 && self.current().kind != TokenKind::Eof {
                        match self.current().kind {
                            TokenKind::OpenBrace => depth += 1,
                            TokenKind::CloseBrace => { depth -= 1; if depth == 0 { self.advance(); break; } }
                            _ => {}
                        }
                        text_buf.push_str(self.current().text(self.source));
                        self.advance();
                    }
                }
                TokenKind::CloseBrace => { self.advance(); }
                _ => { self.advance(); }
            }
        }
        if !text_buf.is_empty() {
            tokens.push(AlgoToken::Text(text_buf));
        }
        tokens
    }

    fn parse_tcolorbox_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        use crate::color::Color;

        let opt = self.try_read_optional_arg().unwrap_or_default();

        // Parse tcolorbox options
        let mut bg_color = Color::rgb(0.96, 0.96, 0.96);
        let mut frame_color = Color::rgb(0.5, 0.5, 0.5);
        let mut corner_radius: f32 = 4.0;
        let mut rule_width: f32 = 0.5;
        let mut padding: f32 = 8.0;
        let mut title_text: Option<String> = None;

        for part in opt.split(',') {
            let part = part.trim();
            if let Some((key, val)) = part.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "colback" => { bg_color = Color::from_spec(val).unwrap_or(bg_color); }
                    "colframe" => { frame_color = Color::from_spec(val).unwrap_or(frame_color); }
                    "arc" => {
                        if let Some(v) = parse_dimension_simple(val) { corner_radius = v; }
                    }
                    "boxrule" => {
                        if let Some(v) = parse_dimension_simple(val) { rule_width = v; }
                    }
                    "left" | "right" | "top" | "bottom" => {
                        if let Some(v) = parse_dimension_simple(val) { padding = v; }
                    }
                    "title" => {
                        // Strip braces: {text} -> text
                        let t = val.trim_start_matches('{').trim_end_matches('}');
                        title_text = Some(t.to_string());
                    }
                    _ => {} // ignore unknown options
                }
            }
        }

        let content = self.parse_environment_body(env_name)?;

        let title = title_text.map(|t| {
            // Parse title for formatting like \textbf{...}
            let cleaned = t.replace("\\textbf{", "").replace("\\textit{", "").replace('}', "");
            vec![Node::Bold(vec![Node::Text(cleaned)])]
        });

        Ok(Some(Node::ColorBox(Box::new(ColorBoxData {
            content,
            title,
            bg_color,
            frame_color,
            corner_radius,
            rule_width,
            padding,
        }))))
    }

    fn parse_wrapfigure_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        // \begin{wrapfigure}{r/l}{width}
        let placement_str = self.read_braced_text().unwrap_or_else(|_| "r".to_string());
        let placement = placement_str.chars().next().unwrap_or('r');
        let width_str = self.read_braced_text().unwrap_or_default();
        let width = self.parse_dimension(&width_str).unwrap_or(0.4);

        let mut content = Vec::new();
        let mut caption = None;
        let mut label = None;

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in wrapfigure"),
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
                        caption = Some(self.read_braced_nodes()?);
                        continue;
                    }
                    if cid == cmd_id::LABEL {
                        self.advance();
                        label = Some(self.read_braced_text()?);
                        continue;
                    }
                    if cid == cmd_id::CENTERING { self.advance(); continue; }
                    if let Some(node) = self.parse_node()? { content.push(node); }
                }
                _ => {
                    if let Some(node) = self.parse_node()? { content.push(node); }
                }
            }
        }

        Ok(Some(Node::WrapFigure {
            placement,
            width,
            content,
            caption,
            label,
        }))
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

        let numbered = matches!(env_name, "equation" | "align" | "gather" | "multline"
            | "flalign" | "alignat" | "eqnarray");
        let env_type = match env_name {
            "equation" | "equation*" => MathEnvType::Equation,
            "align" | "align*" | "flalign" | "flalign*"
            | "alignat" | "alignat*" | "eqnarray" | "eqnarray*"
            | "split" => MathEnvType::Align,
            "gather" | "gather*" => MathEnvType::Gather,
            "multline" | "multline*" => MathEnvType::Multline,
            _ => MathEnvType::DollarDollar,
        };
        Ok(Some(Node::DisplayMath(Box::new(DisplayMathData {
            nodes: math_nodes,
            numbered,
            env_type,
        }))))
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
                // In TeX math mode, whitespace is ignored — spacing comes from atom types
                self.advance();
                Ok(None)
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
                // Check for {n \choose k} pattern
                if let Some(pos) = nodes.iter().position(|n| matches!(n, MathNode::Text(t) if t == "\x01CHOOSE\x01")) {
                    let top: Vec<MathNode> = nodes[..pos].to_vec();
                    let bottom: Vec<MathNode> = nodes[pos + 1..].to_vec();
                    Ok(Some(MathNode::Binom { top, bottom }))
                } else {
                    Ok(Some(MathNode::Group(nodes)))
                }
            }
            TokenKind::CloseBrace => {
                self.advance();
                Ok(None)
            }
            TokenKind::Ampersand => {
                self.advance();
                Ok(Some(MathNode::AlignmentMark))
            }
            TokenKind::DoubleBackslash => {
                self.advance();
                // Skip optional [spacing] after \\
                if self.current().kind == TokenKind::OpenBracket {
                    let _ = self.try_read_optional_arg();
                }
                Ok(Some(MathNode::NewLine))
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
                    // Parse optional index as math nodes for expressions like \sqrt[n+1]{x}
                    self.advance(); // skip [
                    let mut nodes = Vec::new();
                    while self.current().kind != TokenKind::CloseBracket && self.current().kind != TokenKind::Eof {
                        if let Some(node) = self.parse_math_node()? {
                            nodes.push(node);
                        }
                    }
                    if self.current().kind == TokenKind::CloseBracket {
                        self.advance(); // skip ]
                    }
                    if nodes.is_empty() { None } else { Some(nodes) }
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
                let left_delim = self.read_math_delimiter();
                // Collect content until matching \right (recursive for nested \left...\right)
                let mut content = Vec::new();
                loop {
                    if self.current().kind == TokenKind::Eof {
                        break;
                    }
                    // Check for \right (ends this group)
                    if self.current().kind == TokenKind::Command {
                        let cmd_text = self.current().text(self.source);
                        if cmd_text == "\\right" {
                            self.advance();
                            let right_delim = self.read_math_delimiter();
                            return Ok(Some(MathNode::DelimitedGroup {
                                left: left_delim,
                                right: right_delim,
                                content,
                            }));
                        }
                    }
                    // parse_math_node handles nested \left recursively (→ DelimitedGroup)
                    if let Some(node) = self.parse_math_node()? {
                        content.push(node);
                    }
                }
                // Unmatched \left — fall back to old behavior
                Ok(Some(MathNode::Left(left_delim)))
            }
            "\\right" => {
                // Unmatched \right (shouldn't happen if \left...\right are balanced)
                let delim = self.read_math_delimiter();
                Ok(Some(MathNode::Right(delim)))
            }
            "\\boxed" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Boxed(content)))
            }
            "\\mathrlap" | "\\mathllap" | "\\mathclap" => {
                // Zero-width overlap — render content but take no width
                // Approximate as Phantom (render nothing, no width) — content still produces output in layout
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Group(content)))
            }
            "\\smash" => {
                // Remove vertical extent — render content visually but set zero height/depth
                let _opt = self.try_read_optional_arg(); // optional [t] or [b]
                let content = self.parse_math_arg()?;
                // Wrap in a group — layout will handle normally but we need to suppress height
                // For now emit as Group (visual rendering preserved, height contribution for layout)
                Ok(Some(MathNode::Group(content)))
            }
            "\\text" | "\\textrm" | "\\mathrm" | "\\textit" | "\\mathit"
            | "\\textbf" | "\\mathbf" | "\\texttt" | "\\mathtt"
            | "\\textsf" | "\\mathsf" | "\\mbox" | "\\hbox" => {
                let mut text = self.read_braced_text()?;
                // Strip TeX font declaration prefixes (e.g. \hbox{\rm text} → text)
                for prefix in &["\\rm ", "\\bf ", "\\it ", "\\sf ", "\\tt ",
                                "\\rm\n", "\\bf\n", "\\it\n", "\\sf\n", "\\tt\n"] {
                    if text.starts_with(prefix) {
                        text = text[prefix.len()..].to_string();
                        break;
                    }
                }
                let font_id = match cmd {
                    "\\textbf" | "\\mathbf" => FontId::TimesBold,
                    "\\textit" | "\\mathit" => FontId::TimesItalic,
                    "\\texttt" | "\\mathtt" => FontId::Courier,
                    "\\textsf" | "\\mathsf" => FontId::Helvetica,
                    _ => FontId::TimesRoman,
                };
                if font_id == FontId::TimesRoman {
                    Ok(Some(MathNode::Text(text)))
                } else {
                    Ok(Some(MathNode::StyledText(text, font_id)))
                }
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
            // Capital Greek variants (same glyphs as standard capitals)
            "\\varGamma" => Ok(Some(MathNode::Symbol("\u{0393}".to_string()))),
            "\\varDelta" => Ok(Some(MathNode::Symbol("\u{0394}".to_string()))),
            "\\varTheta" => Ok(Some(MathNode::Symbol("\u{0398}".to_string()))),
            "\\varLambda" => Ok(Some(MathNode::Symbol("\u{039B}".to_string()))),
            "\\varXi" => Ok(Some(MathNode::Symbol("\u{039E}".to_string()))),
            "\\varPi" => Ok(Some(MathNode::Symbol("\u{03A0}".to_string()))),
            "\\varSigma" => Ok(Some(MathNode::Symbol("\u{03A3}".to_string()))),
            "\\varPhi" => Ok(Some(MathNode::Symbol("\u{03A6}".to_string()))),
            "\\varPsi" => Ok(Some(MathNode::Symbol("\u{03A8}".to_string()))),
            "\\varOmega" => Ok(Some(MathNode::Symbol("\u{03A9}".to_string()))),

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
            "\\subsetneq" | "\\subsetneqq" => Ok(Some(MathNode::Operator("\u{2282}".to_string()))), // ⊂ (approx)
            "\\supsetneq" | "\\supsetneqq" => Ok(Some(MathNode::Operator("\u{2283}".to_string()))), // ⊃ (approx)
            "\\nsubseteq" => Ok(Some(MathNode::Operator("\u{2282}".to_string()))), // ⊂ (approx)
            "\\nsupseteq" => Ok(Some(MathNode::Operator("\u{2283}".to_string()))), // ⊃ (approx)
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
            "\\vdots" => Ok(Some(MathNode::Symbol("\u{22EE}".to_string()))),
            "\\ddots" => Ok(Some(MathNode::Symbol("\u{22F1}".to_string()))),
            "\\iddots" => Ok(Some(MathNode::Symbol("\u{22F0}".to_string()))),
            "\\prime" => Ok(Some(MathNode::Symbol("\u{2032}".to_string()))),
            "\\emptyset" | "\\varnothing" => Ok(Some(MathNode::Symbol("\u{2205}".to_string()))),
            "\\angle" => Ok(Some(MathNode::Symbol("\u{2220}".to_string()))),
            "\\ell" => Ok(Some(MathNode::Variable('l'))), // script l, approximated as italic l
            "\\wp" => Ok(Some(MathNode::Symbol("\u{2118}".to_string()))),
            "\\Re" => Ok(Some(MathNode::Symbol("\u{211C}".to_string()))),
            "\\Im" => Ok(Some(MathNode::Symbol("\u{2111}".to_string()))),
            "\\aleph" => Ok(Some(MathNode::Symbol("\u{2135}".to_string()))),
            "\\hbar" => Ok(Some(MathNode::Symbol("\u{210F}".to_string()))),
            "\\imath" => Ok(Some(MathNode::Variable('i'))),
            "\\jmath" => Ok(Some(MathNode::Variable('j'))),
            // Additional symbols
            "\\therefore" => Ok(Some(MathNode::Symbol("\u{2234}".to_string()))),
            "\\because" => Ok(Some(MathNode::Symbol("\u{2235}".to_string()))),
            "\\ll" => Ok(Some(MathNode::Operator("\u{226A}".to_string()))),
            "\\gg" => Ok(Some(MathNode::Operator("\u{226B}".to_string()))),
            "\\doteq" => Ok(Some(MathNode::Operator("\u{2250}".to_string()))),
            "\\mid" | "\\divides" => Ok(Some(MathNode::Operator("\u{2223}".to_string()))),
            "\\nmid" => Ok(Some(MathNode::Operator("\u{2224}".to_string()))),
            "\\leqq" => Ok(Some(MathNode::Operator("\u{2266}".to_string()))),
            "\\geqq" => Ok(Some(MathNode::Operator("\u{2267}".to_string()))),
            "\\clubsuit" => Ok(Some(MathNode::Symbol("\u{2663}".to_string()))),
            "\\heartsuit" => Ok(Some(MathNode::Symbol("\u{2661}".to_string()))),
            "\\spadesuit" => Ok(Some(MathNode::Symbol("\u{2660}".to_string()))),
            "\\diamondsuit" => Ok(Some(MathNode::Symbol("\u{2662}".to_string()))),
            "\\circ" => Ok(Some(MathNode::Operator("\u{2218}".to_string()))),
            "\\ast" => Ok(Some(MathNode::Operator("\u{2217}".to_string()))),
            "\\cdot" => Ok(Some(MathNode::Operator("\u{22C5}".to_string()))),
            "\\bigcap" => Ok(Some(MathNode::Symbol("\u{22C2}".to_string()))),
            "\\bigcup" => Ok(Some(MathNode::Symbol("\u{22C3}".to_string()))),
            "\\bigwedge" => Ok(Some(MathNode::Symbol("\u{22C0}".to_string()))),
            "\\bigvee" => Ok(Some(MathNode::Symbol("\u{22C1}".to_string()))),
            "\\circ" => Ok(Some(MathNode::Symbol("\u{00B0}".to_string()))),  // degree symbol
            "\\degree" => Ok(Some(MathNode::Symbol("\u{00B0}".to_string()))),
            "\\setminus" => Ok(Some(MathNode::Operator("\\".to_string()))),
            "\\oplus" => Ok(Some(MathNode::Operator("\u{2295}".to_string()))),
            "\\otimes" => Ok(Some(MathNode::Operator("\u{2297}".to_string()))),
            "\\wedge" | "\\land" => Ok(Some(MathNode::Operator("\u{2227}".to_string()))),
            "\\vee" | "\\lor" => Ok(Some(MathNode::Operator("\u{2228}".to_string()))),
            "\\mapsto" => Ok(Some(MathNode::Operator("\u{21A6}".to_string()))),
            "\\hookrightarrow" => Ok(Some(MathNode::Operator("\u{21AA}".to_string()))),
            "\\twoheadrightarrow" => Ok(Some(MathNode::Operator("\u{2192}".to_string()))), // approx as →
            "\\longrightarrow" => Ok(Some(MathNode::Operator("\u{2192}".to_string()))),
            "\\longleftarrow" => Ok(Some(MathNode::Operator("\u{2190}".to_string()))),
            "\\Longrightarrow" => Ok(Some(MathNode::Operator("\u{21D2}".to_string()))),
            "\\Longleftarrow" => Ok(Some(MathNode::Operator("\u{21D0}".to_string()))),
            "\\mid" => Ok(Some(MathNode::Operator("|".to_string()))),
            "\\nmid" => Ok(Some(MathNode::Operator("|/".to_string()))),
            "\\cong" => Ok(Some(MathNode::Operator("\u{2245}".to_string()))),
            "\\simeq" => Ok(Some(MathNode::Operator("\u{2243}".to_string()))),
            "\\propto" => Ok(Some(MathNode::Operator("\u{221D}".to_string()))),
            "\\perp" => Ok(Some(MathNode::Operator("\u{22A5}".to_string()))),
            "\\parallel" => Ok(Some(MathNode::Operator("\u{2225}".to_string()))),
            "\\bigcup" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Sum { lower, upper })) // render like large op
            }
            "\\bigcap" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Sum { lower, upper }))
            }
            "\\bigoplus" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Sum { lower, upper }))
            }
            "\\bigotimes" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Sum { lower, upper }))
            }
            "\\coprod" => {
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::Product { lower, upper }))
            }

            // Limit-style operators: subscript goes below in display math
            "\\lim" | "\\limsup" | "\\liminf"
            | "\\min" | "\\max" | "\\sup" | "\\inf"
            | "\\argmin" | "\\argmax"
            | "\\det" | "\\gcd" | "\\Pr" => {
                let name = cmd[1..].to_string();
                let (lower, upper) = self.parse_limits()?;
                Ok(Some(MathNode::LimitOp { name, lower, upper }))
            }
            // Regular math functions (no limit placement)
            "\\sin" | "\\cos" | "\\tan" | "\\cot" | "\\sec" | "\\csc"
            | "\\arcsin" | "\\arccos" | "\\arctan"
            | "\\sinh" | "\\cosh" | "\\tanh" | "\\coth"
            | "\\log" | "\\ln" | "\\lg" | "\\exp"
            | "\\dim" | "\\ker"
            | "\\deg" | "\\hom" | "\\arg"
            | "\\var" | "\\cov" | "\\sgn"
            | "\\tr" | "\\diag" | "\\rank" | "\\lcm"
            | "\\Hom" | "\\End" | "\\Aut" | "\\Im" | "\\Re" => {
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

            "\\label" => {
                let l = self.read_braced_text()?;
                Ok(Some(MathNode::Label(l)))
            }
            "\\notag" | "\\nonumber" => {
                Ok(Some(MathNode::NoTag))
            }
            "\\tag" | "\\tag*" => {
                let text = self.read_braced_text()?;
                Ok(Some(MathNode::Tag(text)))
            }
            "\\intertext" | "\\shortintertext" => {
                let text = self.read_braced_text()?;
                Ok(Some(MathNode::Intertext(text)))
            }

            // Binom
            "\\binom" | "\\dbinom" | "\\tbinom" => {
                let top = self.parse_math_arg()?;
                let bottom = self.parse_math_arg()?;
                Ok(Some(MathNode::Binom { top, bottom }))
            }
            "\\choose" | "\\over" => {
                // Sentinel — handled in the OpenBrace group loop which splits on this
                Ok(Some(MathNode::Text("\x01CHOOSE\x01".to_string())))
            }

            // Overset/Underset/Stackrel
            "\\overset" => {
                let over = self.parse_math_arg()?;
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Overset { over, base }))
            }
            "\\underset" => {
                let under = self.parse_math_arg()?;
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Underset { under, base }))
            }
            "\\stackrel" => {
                let over = self.parse_math_arg()?;
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Overset { over, base }))
            }
            "\\substack" => {
                // \substack{a \\ b \\ c} — stacked lines under sum/prod
                let inner = self.read_braced_text()?;
                let mut rows: Vec<Vec<MathNode>> = Vec::new();
                for line in inner.split("\\\\") {
                    let line = line.trim();
                    if !line.is_empty() {
                        rows.push(vec![MathNode::Text(line.to_string())]);
                    }
                }
                Ok(Some(MathNode::Substack(rows)))
            }

            // Extended arrows: \xrightarrow[below]{above}
            "\\xrightarrow" => {
                let below = self.try_read_optional_math_arg();
                let above = self.parse_math_arg()?;
                let arrow = vec![MathNode::Symbol("\u{2192}".to_string())];
                if let Some(under) = below {
                    // Has both above and below
                    Ok(Some(MathNode::Overset { over: above, base: vec![MathNode::Underset { under, base: arrow }] }))
                } else {
                    Ok(Some(MathNode::Overset { over: above, base: arrow }))
                }
            }
            "\\xleftarrow" => {
                let below = self.try_read_optional_math_arg();
                let above = self.parse_math_arg()?;
                let arrow = vec![MathNode::Symbol("\u{2190}".to_string())];
                if let Some(under) = below {
                    Ok(Some(MathNode::Overset { over: above, base: vec![MathNode::Underset { under, base: arrow }] }))
                } else {
                    Ok(Some(MathNode::Overset { over: above, base: arrow }))
                }
            }

            // Math font commands
            "\\mathbb" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::Blackboard, content }))
            }
            "\\mathcal" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::Calligraphic, content }))
            }
            "\\mathfrak" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::Fraktur, content }))
            }
            "\\mathscr" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::Script, content }))
            }
            "\\mathsf" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::SansSerif, content }))
            }
            "\\boldsymbol" | "\\bm" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathFont { font: MathFontType::BoldMath, content }))
            }

            // Operator name
            "\\operatorname" => {
                let name = self.read_braced_text()?;
                Ok(Some(MathNode::OperatorName(name)))
            }

            // Common paired delimiters (mathtools-style)
            "\\norm" | "\\lVert" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "\\|".to_string(), right: "\\|".to_string(), content,
                }))
            }
            "\\abs" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "|".to_string(), right: "|".to_string(), content,
                }))
            }
            "\\ceil" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "\\lceil".to_string(), right: "\\rceil".to_string(), content,
                }))
            }
            "\\floor" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "\\lfloor".to_string(), right: "\\rfloor".to_string(), content,
                }))
            }
            "\\inner" | "\\braket" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "\\langle".to_string(), right: "\\rangle".to_string(), content,
                }))
            }
            "\\set" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::DelimitedGroup {
                    left: "\\{".to_string(), right: "\\}".to_string(), content,
                }))
            }

            // Phantom
            "\\phantom" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Phantom(content)))
            }
            "\\vphantom" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::VPhantom(content)))
            }
            "\\hphantom" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::HPhantom(content)))
            }

            // Style switches
            "\\displaystyle" => Ok(Some(MathNode::StyleSwitch(MathStyleType::Display))),
            "\\textstyle" => Ok(Some(MathNode::StyleSwitch(MathStyleType::Text))),
            "\\scriptstyle" => Ok(Some(MathNode::StyleSwitch(MathStyleType::Script))),
            "\\scriptscriptstyle" => Ok(Some(MathNode::StyleSwitch(MathStyleType::ScriptScript))),

            // Big delimiters
            "\\big" | "\\bigl" | "\\bigr" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::BigDelim { delim: d, size: 1.2 }))
            }
            "\\Big" | "\\Bigl" | "\\Bigr" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::BigDelim { delim: d, size: 1.5 }))
            }
            "\\bigg" | "\\biggl" | "\\biggr" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::BigDelim { delim: d, size: 1.8 }))
            }
            "\\Bigg" | "\\Biggl" | "\\Biggr" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::BigDelim { delim: d, size: 2.2 }))
            }
            "\\bigm" | "\\Bigm" | "\\biggm" | "\\Biggm" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::BigDelim { delim: d, size: 1.5 }))
            }

            // Accents
            "\\breve" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Breve }))
            }
            "\\check" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Check }))
            }
            "\\acute" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Hat })) // approx
            }
            "\\grave" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Hat })) // approx
            }
            "\\widehat" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Hat }))
            }
            "\\widetilde" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Tilde }))
            }

            // Overbrace/underbrace
            "\\overbrace" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Over { content, over_type: OverType::Brace }))
            }
            "\\underbrace" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Under { content, under_type: UnderType::Brace }))
            }
            "\\overrightarrow" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Over { content, over_type: OverType::Arrow }))
            }
            "\\overleftarrow" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Over { content, over_type: OverType::Arrow }))
            }
            "\\underline" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Under { content, under_type: UnderType::Line }))
            }

            // \not — negation modifier
            "\\not" => {
                // Read next token and map to negated unicode
                if let Some(next) = self.parse_math_node()? {
                    let negated = match &next {
                        MathNode::Operator(op) => match op.as_str() {
                            "=" => Some("\u{2260}"),  // ≠
                            "<" => Some("\u{226E}"),  // ≮
                            ">" => Some("\u{226F}"),  // ≯
                            _ => None,
                        },
                        MathNode::Symbol(s) => match s.as_str() {
                            "\u{2208}" => Some("\u{2209}"),  // ∈ → ∉
                            "\u{2282}" => Some("\u{2284}"),  // ⊂ → ⊄
                            "\u{2283}" => Some("\u{2285}"),  // ⊃ → ⊅
                            "\u{2286}" => Some("\u{2288}"),  // ⊆ → ⊈
                            "\u{2287}" => Some("\u{2289}"),  // ⊇ → ⊉
                            "\u{2264}" => Some("\u{2270}"),  // ≤ → ≰
                            "\u{2265}" => Some("\u{2271}"),  // ≥ → ≱
                            "\u{2261}" => Some("\u{2262}"),  // ≡ → ≢
                            "\u{223C}" => Some("\u{2241}"),  // ∼ → ≁
                            "\u{2248}" => Some("\u{2249}"),  // ≈ → ≉
                            "\u{2203}" => Some("\u{2204}"),  // ∃ → ∄
                            "\u{2223}" => Some("\u{2224}"),  // ∣ → ∤
                            "\u{2225}" => Some("\u{2226}"),  // ∥ → ∦
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(neg) = negated {
                        Ok(Some(MathNode::Symbol(neg.to_string())))
                    } else {
                        Ok(Some(next))
                    }
                } else {
                    Ok(None)
                }
            }

            // \mathop — force operator spacing
            "\\mathop" => {
                let content = self.parse_math_arg()?;
                // Consume optional \nolimits / \limits after \mathop
                self.skip_whitespace_and_comments();
                if self.current().kind == TokenKind::Command {
                    let next_cmd = self.current().text(self.source);
                    if next_cmd == "\\nolimits" || next_cmd == "\\limits" {
                        self.advance();
                    }
                }
                // Convert group to OperatorName if it's just text
                let text = math_group_to_text(&content);
                if !text.is_empty() {
                    Ok(Some(MathNode::OperatorName(text)))
                } else {
                    Ok(Some(MathNode::Group(content)))
                }
            }

            // TeX-style font switches (declarations without braces)
            "\\rm" => {
                // Read remaining text tokens as upright text
                let mut text = String::new();
                loop {
                    match self.current().kind {
                        TokenKind::Text => {
                            text.push_str(self.current().text(self.source));
                            self.advance();
                        }
                        TokenKind::Space => {
                            if !text.is_empty() { text.push(' '); }
                            self.advance();
                        }
                        _ => break,
                    }
                }
                if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(MathNode::Text(text.trim_end().to_string())))
                }
            }
            "\\bf" => {
                let mut text = String::new();
                loop {
                    match self.current().kind {
                        TokenKind::Text => {
                            text.push_str(self.current().text(self.source));
                            self.advance();
                        }
                        TokenKind::Space => {
                            if !text.is_empty() { text.push(' '); }
                            self.advance();
                        }
                        _ => break,
                    }
                }
                if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(MathNode::Text(text.trim_end().to_string())))
                }
            }
            "\\it" | "\\sl" => {
                // Italic switch — math is already italic, just skip
                Ok(None)
            }
            "\\cal" => {
                // Calligraphic font — read as MathFont if braced arg, otherwise read text
                if self.current().kind == TokenKind::OpenBrace {
                    let content = self.parse_math_arg()?;
                    Ok(Some(MathNode::MathFont { font: MathFontType::Calligraphic, content }))
                } else {
                    let mut text = String::new();
                    loop {
                        match self.current().kind {
                            TokenKind::Text => {
                                text.push_str(self.current().text(self.source));
                                self.advance();
                            }
                            TokenKind::Space => {
                                if !text.is_empty() { text.push(' '); }
                                self.advance();
                            }
                            _ => break,
                        }
                    }
                    if text.is_empty() {
                        Ok(None)
                    } else {
                        let content = text.trim_end().chars().map(|c| MathNode::Variable(c)).collect();
                        Ok(Some(MathNode::MathFont { font: MathFontType::Calligraphic, content }))
                    }
                }
            }

            // \nolimits / \limits as standalone (when not consumed by \sum, \mathop, etc.)
            "\\nolimits" | "\\limits" => Ok(None),

            // \cr — row separator (equivalent to \\ in matrices/arrays)
            "\\cr" => Ok(Some(MathNode::NewLine)),

            // \mathstrut — invisible strut for vertical spacing, skip
            "\\mathstrut" | "\\strut" => Ok(Some(MathNode::Phantom(vec![MathNode::Variable('(')]))),

            // \noindent, \centering — layout hints, skip in math mode
            "\\noindent" | "\\centering" => Ok(None),

            // \joinrel — negative space to join relation arrows (e.g. \lhook\joinrel\longrightarrow)
            "\\joinrel" => Ok(Some(MathNode::Space(-3.0))),

            // \mkern, \kern — horizontal kerning in math mode
            "\\mkern" | "\\kern" => {
                // Read dimension like -70mu or 5pt
                let mut dim_str = String::new();
                while self.pos < self.tokens.len() {
                    let tk = &self.tokens[self.pos];
                    let t = tk.text(self.source);
                    if t.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == 'm' || c == 'u' || c == 'p' || c == 't' || c == 'e') {
                        dim_str.push_str(t);
                        self.advance();
                        if t.ends_with("mu") || t.ends_with("pt") || t.ends_with("em") {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                // Parse the value (mu = math unit ≈ 1/18 em)
                let val: f32 = dim_str.replace("mu", "").replace("pt", "").replace("em", "").trim().parse().unwrap_or(0.0);
                let pts = if dim_str.contains("mu") { val * 0.5 } else if dim_str.contains("em") { val * 10.0 } else { val };
                Ok(Some(MathNode::Space(pts)))
            }

            // \pmod, \bmod, \pod, \mod — modular arithmetic
            "\\pmod" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Pmod(content)))
            }
            "\\pod" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Pod(content)))
            }
            "\\bmod" => Ok(Some(MathNode::Bmod)),
            "\\mod" => {
                let content = self.parse_math_arg()?;
                // \mod{p} renders as "mod p" with wide space, no parens
                Ok(Some(MathNode::Group(vec![
                    MathNode::Space(18.0),
                    MathNode::OperatorName("mod".to_string()),
                    MathNode::Space(3.33),
                    MathNode::Group(content),
                ])))
            }

            // \cfrac — continued fraction (like \frac but with \displaystyle in numer/denom)
            "\\cfrac" => {
                // Optional alignment: \cfrac[l]{n}{d}
                if self.current().kind == TokenKind::OpenBracket {
                    let _ = self.try_read_optional_arg();
                }
                let numer = self.parse_math_arg()?;
                let denom = self.parse_math_arg()?;
                // Wrap content in displaystyle groups for larger rendering
                let styled_numer = vec![MathNode::StyleSwitch(MathStyleType::Display), MathNode::Group(numer)];
                let styled_denom = vec![MathNode::StyleSwitch(MathStyleType::Display), MathNode::Group(denom)];
                Ok(Some(MathNode::Frac { numer: styled_numer, denom: styled_denom }))
            }

            // \mathrel, \mathbin, \mathord — spacing wrappers
            "\\mathrel" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathRel(content)))
            }
            "\\mathbin" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::MathBin(content)))
            }
            "\\mathord" | "\\mathinner" | "\\mathnormal" | "\\mathopen" | "\\mathclose" | "\\mathpunct" => {
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Group(content)))
            }

            // \rule{width}{height} in math mode
            "\\rule" => {
                let _raise = self.try_read_optional_arg(); // optional raise
                let width_str = self.read_braced_text().unwrap_or_default();
                let height_str = self.read_braced_text().unwrap_or_default();
                let w = self.parse_dimension(&width_str).unwrap_or(0.0);
                let h = self.parse_dimension(&height_str).unwrap_or(0.4);
                Ok(Some(MathNode::Rule { width: w, height: h }))
            }

            // \middle delimiter
            "\\middle" => {
                let d = self.current().text(self.source).to_string();
                self.advance();
                Ok(Some(MathNode::Middle(d)))
            }

            // \raisebox in math — just render content
            "\\raisebox" => {
                let _raise = self.read_braced_text()?;
                let _ = self.try_read_optional_arg(); // optional height
                let _ = self.try_read_optional_arg(); // optional depth
                let content = self.parse_math_arg()?;
                Ok(Some(MathNode::Group(content)))
            }

            // \hfill, \hspace — spacing
            "\\hfill" => Ok(Some(MathNode::Space(20.0))),
            "\\hspace" | "\\hspace*" => {
                let dim = self.read_braced_text()?;
                let pts = self.parse_dimension(&dim).unwrap_or(10.0);
                Ok(Some(MathNode::Space(pts)))
            }

            // \begin in math mode — matrix/cases environments
            "\\begin" => {
                let env_name = self.read_braced_text()?;
                match env_name.as_str() {
                    "pmatrix" | "pmatrix*" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::Parenthesized }))
                    }
                    "bmatrix" | "bmatrix*" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::Bracketed }))
                    }
                    "Bmatrix" | "Bmatrix*" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::Braced }))
                    }
                    "vmatrix" | "vmatrix*" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::VerticalBar }))
                    }
                    "Vmatrix" | "Vmatrix*" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::DoubleBar }))
                    }
                    "matrix" | "smallmatrix" => {
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::Plain }))
                    }
                    "cases" | "dcases" | "rcases" => {
                        let rows = self.parse_math_cases_body(&env_name)?;
                        Ok(Some(MathNode::Cases { rows }))
                    }
                    "array" => {
                        // Skip column spec
                        if self.current().kind == TokenKind::OpenBrace {
                            let _ = self.read_braced_text()?;
                        }
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        Ok(Some(MathNode::Matrix { rows, style: MatrixStyle::Plain }))
                    }
                    "aligned" | "gathered" | "split" => {
                        // Parse as matrix-like alignment body
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        // Flatten into sequence with alignment marks and newlines
                        let mut nodes = Vec::new();
                        for (i, row) in rows.iter().enumerate() {
                            if i > 0 {
                                nodes.push(MathNode::NewLine);
                            }
                            for (j, cell) in row.iter().enumerate() {
                                if j > 0 {
                                    nodes.push(MathNode::AlignmentMark);
                                }
                                nodes.extend(cell.iter().cloned());
                            }
                        }
                        Ok(Some(MathNode::Group(nodes)))
                    }
                    "subarray" => {
                        // Skip column spec and parse as matrix
                        if self.current().kind == TokenKind::OpenBrace {
                            let _ = self.read_braced_text()?;
                        }
                        let rows = self.parse_math_matrix_body(&env_name)?;
                        let mut nodes = Vec::new();
                        for (i, row) in rows.iter().enumerate() {
                            if i > 0 {
                                nodes.push(MathNode::NewLine);
                            }
                            for cell in row {
                                nodes.extend(cell.iter().cloned());
                            }
                        }
                        Ok(Some(MathNode::Group(nodes)))
                    }
                    _ => {
                        // Unknown math environment — skip to \end
                        self.skip_math_env_body(&env_name)?;
                        Ok(Some(MathNode::Text(format!("[{}]", env_name))))
                    }
                }
            }

            _ => {
                // Unknown math command, render as text
                Ok(Some(MathNode::Text(cmd[1..].to_string())))
            }
        }
    }

    /// Expand siunitx unit macros to plain text
    fn expand_si_unit(&self, unit: &str) -> String {
        let mut result = unit.to_string();
        // Common SI unit macros
        let replacements = [
            ("\\meter", "m"), ("\\metre", "m"), ("\\kilogram", "kg"), ("\\gram", "g"),
            ("\\second", "s"), ("\\ampere", "A"), ("\\kelvin", "K"), ("\\mole", "mol"),
            ("\\candela", "cd"), ("\\hertz", "Hz"), ("\\newton", "N"), ("\\pascal", "Pa"),
            ("\\joule", "J"), ("\\watt", "W"), ("\\coulomb", "C"), ("\\volt", "V"),
            ("\\farad", "F"), ("\\ohm", "\u{03A9}"), ("\\siemens", "S"), ("\\weber", "Wb"),
            ("\\tesla", "T"), ("\\henry", "H"), ("\\lumen", "lm"), ("\\lux", "lx"),
            ("\\becquerel", "Bq"), ("\\gray", "Gy"), ("\\sievert", "Sv"),
            ("\\kilo", "k"), ("\\mega", "M"), ("\\giga", "G"), ("\\tera", "T"),
            ("\\milli", "m"), ("\\micro", "\u{03BC}"), ("\\nano", "n"), ("\\pico", "p"),
            ("\\centi", "c"), ("\\deci", "d"),
            ("\\per", "/"), ("\\of", " "), ("\\squared", "\u{00B2}"), ("\\cubed", "\u{00B3}"),
            ("\\tothe", "^"),
            ("\\degree", "\u{00B0}"), ("\\celsius", "\u{00B0}C"), ("\\percent", "%"),
            ("\\litre", "L"), ("\\liter", "L"),
            ("\\angstrom", "\u{00C5}"),
            ("\\electronvolt", "eV"), ("\\eV", "eV"),
            ("\\bar", "bar"), ("\\barn", "b"),
        ];
        for (macro_name, replacement) in replacements {
            result = result.replace(macro_name, replacement);
        }
        // Clean up any remaining backslash commands by removing them
        while let Some(idx) = result.find('\\') {
            let end = result[idx+1..].find(|c: char| !c.is_ascii_alphabetic())
                .map(|i| idx + 1 + i)
                .unwrap_or(result.len());
            result.replace_range(idx..end, "");
        }
        result.trim().to_string()
    }

    /// Read a math delimiter token after \left or \right.
    /// Handles: ( ) [ ] | . \{ \} \langle \rangle \lfloor \rfloor \lceil \rceil \| etc.
    fn read_math_delimiter(&mut self) -> String {
        self.skip_whitespace_and_comments();
        let tok = self.current();
        let text = tok.text(self.source);
        match tok.kind {
            TokenKind::Command => {
                // Delimiter commands like \{ \} \langle \rangle \| etc.
                let delim = text.to_string();
                self.advance();
                delim
            }
            _ => {
                let delim = text.to_string();
                self.advance();
                delim
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

    /// Parse matrix body: rows separated by \\, cells separated by &
    /// Until \end{env_name}
    fn parse_math_matrix_body(&mut self, env_name: &str) -> Result<Vec<Vec<Vec<MathNode>>>> {
        let mut rows: Vec<Vec<Vec<MathNode>>> = Vec::new();
        let mut current_row: Vec<Vec<MathNode>> = Vec::new();
        let mut current_cell: Vec<MathNode> = Vec::new();

        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in math matrix environment"),
                TokenKind::Ampersand => {
                    self.advance();
                    current_row.push(std::mem::take(&mut current_cell));
                }
                TokenKind::DoubleBackslash => {
                    self.advance();
                    current_row.push(std::mem::take(&mut current_cell));
                    rows.push(std::mem::take(&mut current_row));
                    // Skip optional [spacing] after \\
                    if self.current().kind == TokenKind::OpenBracket {
                        let _ = self.try_read_optional_arg();
                    }
                }
                TokenKind::Command => {
                    let cmd_id = self.current().cmd;
                    if cmd_id == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                // Push remaining cell/row
                                if !current_cell.is_empty() || !current_row.is_empty() {
                                    current_row.push(current_cell);
                                    rows.push(current_row);
                                }
                                return Ok(rows);
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    if let Some(mn) = self.parse_math_node()? {
                        current_cell.push(mn);
                    }
                }
                _ => {
                    if let Some(mn) = self.parse_math_node()? {
                        current_cell.push(mn);
                    }
                }
            }
        }
    }

    /// Parse cases body: rows with value & condition
    fn parse_math_cases_body(&mut self, env_name: &str) -> Result<Vec<(Vec<MathNode>, Option<Vec<MathNode>>)>> {
        let rows_raw = self.parse_math_matrix_body(env_name)?;
        let mut result = Vec::new();
        for row in rows_raw {
            let value = if !row.is_empty() { row[0].clone() } else { Vec::new() };
            let condition = if row.len() > 1 { Some(row[1].clone()) } else { None };
            result.push((value, condition));
        }
        Ok(result)
    }

    /// Skip until \end{env_name} in math mode
    fn skip_math_env_body(&mut self, env_name: &str) -> Result<()> {
        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in math environment {}", env_name),
                TokenKind::Command => {
                    if self.current().cmd == cmd_id::END {
                        let save = self.pos;
                        self.advance();
                        self.skip_whitespace_and_comments();
                        if self.current().kind == TokenKind::OpenBrace {
                            let name = self.read_braced_text()?;
                            if name == env_name {
                                return Ok(());
                            }
                            self.pos = save;
                        } else {
                            self.pos = save;
                        }
                    }
                    self.advance();
                }
                _ => { self.advance(); }
            }
        }
    }
}

/// Strip \thanks{...} from title/author strings (preamble raw text)
fn strip_thanks(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if i + 8 <= bytes.len() && &s[i..i+8] == "\\thanks{" {
            // Skip \thanks{...}
            i += 8;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'{' { depth += 1; }
                else if bytes[i] == b'}' { depth -= 1; }
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Parse column range "2-5" into (2, 5). Returns 1-based indices.
fn parse_col_range(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let start: u32 = parts[0].trim().parse().ok()?;
        let end: u32 = parts[1].trim().parse().ok()?;
        Some((start, end))
    } else {
        None
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

/// Parse a simple dimension string like "3mm", "0.5pt", "4mm" to points.
fn parse_dimension_simple(s: &str) -> Option<f32> {
    let s = s.trim();
    let (num, unit) = if s.ends_with("mm") {
        (s[..s.len()-2].trim(), "mm")
    } else if s.ends_with("cm") {
        (s[..s.len()-2].trim(), "cm")
    } else if s.ends_with("pt") {
        (s[..s.len()-2].trim(), "pt")
    } else if s.ends_with("in") {
        (s[..s.len()-2].trim(), "in")
    } else if s.ends_with("em") {
        (s[..s.len()-2].trim(), "em")
    } else if s.ends_with("ex") {
        (s[..s.len()-2].trim(), "ex")
    } else {
        (s, "pt")
    };
    let val: f32 = num.parse().ok()?;
    let pts = match unit {
        "mm" => val * 2.8346,
        "cm" => val * 28.346,
        "in" => val * 72.0,
        "em" => val * 10.0,
        "ex" => val * 4.3,
        _ => val,
    };
    Some(pts)
}

/// Extract text content from a group of MathNodes (for \mathop{\rm Text} pattern)
fn math_group_to_text(nodes: &[MathNode]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            MathNode::Variable(c) => text.push(*c),
            MathNode::Number(s) | MathNode::Text(s) | MathNode::OperatorName(s) => text.push_str(s),
            MathNode::Space(_) => { if !text.is_empty() { text.push(' '); } }
            MathNode::Group(inner) => text.push_str(&math_group_to_text(inner)),
            MathNode::Function(name) => text.push_str(name),
            _ => {}
        }
    }
    text.trim().to_string()
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

/// Extract plain text from a node tree (for title extraction, etc.)
fn node_to_plain_text(node: &Node, out: &mut String, source: &str) {
    match node {
        Node::Text(t) => out.push_str(t),
        Node::TextRef(offset, len) => {
            let s = &source[*offset as usize..*offset as usize + *len as usize];
            out.push_str(s.trim());
        }
        Node::Bold(children) | Node::Italic(children) | Node::Emph(children)
        | Node::Monospace(children) | Node::SmallCaps(children)
        | Node::Underline(children) | Node::Group(children)
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
