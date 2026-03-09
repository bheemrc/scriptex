/// High-performance LaTeX lexer using zero-copy approach
/// Operates directly on byte slices for maximum speed
///
/// Optimizations:
/// - Compact 8-byte token representation (was 24 bytes)
/// - 256-byte lookup table for O(1) byte classification
/// - 8-byte batch scanning for text runs
/// - memchr for fast comment scanning

use memchr::memchr;

/// Command IDs for fast parser dispatch (avoids string comparison)
pub mod cmd_id {
    pub const NONE: u8 = 0;
    pub const END: u8 = 1;
    pub const BEGIN: u8 = 2;
    pub const SECTION: u8 = 3;
    pub const SECTION_STAR: u8 = 4;
    pub const SUBSECTION: u8 = 5;
    pub const SUBSECTION_STAR: u8 = 6;
    pub const ITEM: u8 = 7;
    pub const TEXTBF: u8 = 8;
    pub const TEXTIT: u8 = 9;
    pub const EMPH: u8 = 10;
    pub const USEPACKAGE: u8 = 11;
    pub const DOCUMENTCLASS: u8 = 12;
    pub const MAKETITLE: u8 = 13;
    pub const TABLEOFCONTENTS: u8 = 14;
    pub const LABEL: u8 = 15;
    pub const REF: u8 = 16;
    pub const CITE: u8 = 17;
    pub const CHAPTER: u8 = 18;
    pub const SUBSUBSECTION: u8 = 19;
    pub const PARAGRAPH: u8 = 20;
    pub const TEXTTT: u8 = 21;
    pub const HLINE: u8 = 22;
    pub const INCLUDEGRAPHICS: u8 = 23;
    pub const CAPTION: u8 = 24;
    pub const FOOTNOTE: u8 = 25;
    pub const VSPACE: u8 = 26;
    pub const HSPACE: u8 = 27;
    pub const NEWPAGE: u8 = 28;
    pub const NOINDENT: u8 = 29;
    pub const CENTERING: u8 = 30;
    pub const TITLE: u8 = 31;
    pub const AUTHOR: u8 = 32;
    pub const DATE: u8 = 33;
    pub const HREF: u8 = 34;
    pub const URL: u8 = 35;
    pub const COLOR: u8 = 36;
    pub const TEXTCOLOR: u8 = 37;
    pub const BIBITEM: u8 = 38;
    pub const TWOCOLUMN: u8 = 39;
    pub const ONECOLUMN: u8 = 40;
    pub const ICMLTITLE: u8 = 41;
}

/// Compact token representation: 8 bytes total
/// - kind: u8 (token type)
/// - cmd: u8 (command ID for fast dispatch, 0 = unknown)
/// - len: u16 (length of token, max 65535)
/// - pos: u32 (position in source, max 4GB)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Token {
    pub kind: TokenKind,
    pub cmd: u8,
    pub len: u16,
    pub pos: u32,
}

impl Token {
    pub const EOF: Token = Token { kind: TokenKind::Eof, cmd: 0, len: 0, pos: u32::MAX };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TokenKind {
    Command = 0,
    OpenBrace = 1,
    CloseBrace = 2,
    OpenBracket = 3,
    CloseBracket = 4,
    Dollar = 5,
    DoubleDollar = 6,
    Ampersand = 7,
    Tilde = 8,
    Caret = 9,
    Underscore = 10,
    Hash = 11,
    Comment = 12,
    DoubleBackslash = 13,
    Text = 14,
    Space = 15,
    ParBreak = 16,
    Eof = 17,
}

impl Token {
    #[inline]
    fn new(kind: TokenKind, pos: usize, len: usize) -> Self {
        Token {
            kind,
            cmd: 0,
            len: len.min(65535) as u16,
            pos: pos as u32,
        }
    }

    #[inline]
    fn new_cmd(pos: usize, len: usize, cmd: u8) -> Self {
        Token {
            kind: TokenKind::Command,
            cmd,
            len: len.min(65535) as u16,
            pos: pos as u32,
        }
    }

    #[inline]
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        match self.kind {
            TokenKind::OpenBrace => "{",
            TokenKind::CloseBrace => "}",
            TokenKind::OpenBracket => "[",
            TokenKind::CloseBracket => "]",
            TokenKind::Dollar => "$",
            TokenKind::DoubleDollar => "$$",
            TokenKind::Ampersand => "&",
            TokenKind::Tilde => "~",
            TokenKind::Caret => "^",
            TokenKind::Underscore => "_",
            TokenKind::Hash => "#",
            TokenKind::DoubleBackslash => "\\\\",
            TokenKind::Space => " ",
            TokenKind::ParBreak => "\n\n",
            TokenKind::Eof => "",
            _ => {
                let start = self.pos as usize;
                let end = start + self.len as usize;
                &source[start..end.min(source.len())]
            }
        }
    }
}

// Byte classification
const CLASS_TEXT: u8 = 0;
const CLASS_BACKSLASH: u8 = 1;
const CLASS_OPEN_BRACE: u8 = 2;
const CLASS_CLOSE_BRACE: u8 = 3;
const CLASS_OPEN_BRACKET: u8 = 4;
const CLASS_CLOSE_BRACKET: u8 = 5;
const CLASS_DOLLAR: u8 = 6;
const CLASS_AMPERSAND: u8 = 7;
const CLASS_TILDE: u8 = 8;
const CLASS_CARET: u8 = 9;
const CLASS_UNDERSCORE: u8 = 10;
const CLASS_HASH: u8 = 11;
const CLASS_PERCENT: u8 = 12;
const CLASS_NEWLINE: u8 = 13;
const CLASS_SPACE: u8 = 14;

const fn build_class_table() -> [u8; 256] {
    let mut table = [CLASS_TEXT; 256];
    table[b'\\' as usize] = CLASS_BACKSLASH;
    table[b'{' as usize] = CLASS_OPEN_BRACE;
    table[b'}' as usize] = CLASS_CLOSE_BRACE;
    table[b'[' as usize] = CLASS_OPEN_BRACKET;
    table[b']' as usize] = CLASS_CLOSE_BRACKET;
    table[b'$' as usize] = CLASS_DOLLAR;
    table[b'&' as usize] = CLASS_AMPERSAND;
    table[b'~' as usize] = CLASS_TILDE;
    table[b'^' as usize] = CLASS_CARET;
    table[b'_' as usize] = CLASS_UNDERSCORE;
    table[b'#' as usize] = CLASS_HASH;
    table[b'%' as usize] = CLASS_PERCENT;
    table[b'\n' as usize] = CLASS_NEWLINE;
    // NOTE: spaces/tabs are CLASS_TEXT (merged into text runs) to reduce token count
    // Newlines remain separate for paragraph break detection
    table[b'\r' as usize] = CLASS_SPACE; // CR still treated as space (rare)
    table
}

static BYTE_CLASS: [u8; 256] = build_class_table();

const fn build_special_table() -> [bool; 256] {
    let mut table = [false; 256];
    table[b'\\' as usize] = true;
    table[b'{' as usize] = true;
    table[b'}' as usize] = true;
    table[b'[' as usize] = true;
    table[b']' as usize] = true;
    table[b'$' as usize] = true;
    table[b'&' as usize] = true;
    table[b'~' as usize] = true;
    table[b'^' as usize] = true;
    table[b'_' as usize] = true;
    table[b'#' as usize] = true;
    table[b'%' as usize] = true;
    table[b'\n' as usize] = true;
    // Spaces and tabs are NOT special - they're part of text runs
    table[b'\r' as usize] = true;
    table
}

static IS_SPECIAL: [bool; 256] = build_special_table();

pub struct Lexer<'a> {
    source: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source: source.as_bytes(),
            pos: 0,
        }
    }

    /// Pull-based tokenization: returns the next token and advances the lexer position.
    /// Used by the streaming parser to avoid materializing the entire token vec.
    #[inline]
    pub fn next_token(&mut self) -> Token {
        let len = self.source.len();
        if self.pos >= len {
            return Token::new(TokenKind::Eof, len, 0);
        }

        let b = unsafe { *self.source.get_unchecked(self.pos) };
        let class = unsafe { *BYTE_CLASS.get_unchecked(b as usize) };

        match class {
            CLASS_TEXT => {
                let start = self.pos;
                self.pos = self.scan_text_fast(self.pos + 1);
                Token::new(TokenKind::Text, start, self.pos - start)
            }
            CLASS_BACKSLASH => {
                if self.pos + 1 < len {
                    let next = unsafe { *self.source.get_unchecked(self.pos + 1) };
                    if next == b'\\' {
                        let tok = Token::new(TokenKind::DoubleBackslash, self.pos, 2);
                        self.pos += 2;
                        tok
                    } else {
                        self.lex_command()
                    }
                } else {
                    self.lex_command()
                }
            }
            CLASS_OPEN_BRACE => {
                let tok = Token::new(TokenKind::OpenBrace, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_CLOSE_BRACE => {
                let tok = Token::new(TokenKind::CloseBrace, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_OPEN_BRACKET => {
                let tok = Token::new(TokenKind::OpenBracket, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_CLOSE_BRACKET => {
                let tok = Token::new(TokenKind::CloseBracket, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_DOLLAR => {
                if self.pos + 1 < len && unsafe { *self.source.get_unchecked(self.pos + 1) } == b'$' {
                    let tok = Token::new(TokenKind::DoubleDollar, self.pos, 2);
                    self.pos += 2;
                    tok
                } else {
                    let tok = Token::new(TokenKind::Dollar, self.pos, 1);
                    self.pos += 1;
                    tok
                }
            }
            CLASS_AMPERSAND => {
                let tok = Token::new(TokenKind::Ampersand, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_TILDE => {
                let tok = Token::new(TokenKind::Tilde, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_CARET => {
                let tok = Token::new(TokenKind::Caret, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_UNDERSCORE => {
                let tok = Token::new(TokenKind::Underscore, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_HASH => {
                let tok = Token::new(TokenKind::Hash, self.pos, 1);
                self.pos += 1;
                tok
            }
            CLASS_PERCENT => {
                let start = self.pos;
                let rest = unsafe { self.source.get_unchecked(self.pos + 1..) };
                match memchr(b'\n', rest) {
                    Some(offset) => {
                        let end = self.pos + 1 + offset;
                        self.pos = end + 1;
                        Token::new(TokenKind::Comment, start, end - start)
                    }
                    None => {
                        self.pos = len;
                        Token::new(TokenKind::Comment, start, len - start)
                    }
                }
            }
            CLASS_NEWLINE => {
                let start = self.pos;
                let (newline_count, end) = self.scan_whitespace_block(self.pos);
                self.pos = end;
                if newline_count >= 2 {
                    Token::new(TokenKind::ParBreak, start, end - start)
                } else {
                    Token::new(TokenKind::Space, start, end - start)
                }
            }
            CLASS_SPACE => {
                let start = self.pos;
                self.pos = self.skip_horizontal_whitespace(self.pos + 1);
                Token::new(TokenKind::Space, start, self.pos - start)
            }
            _ => {
                let tok = Token::new(TokenKind::Text, self.pos, 1);
                self.pos += 1;
                tok
            }
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let len = self.source.len();
        let mut tokens = Vec::with_capacity(len / 4 + 64);
        loop {
            let tok = self.next_token();
            tokens.push(tok);
            if tok.kind == TokenKind::Eof { break; }
        }
        tokens
    }

    #[inline]
    fn scan_text_fast(&self, start: usize) -> usize {
        let src = self.source;
        let len = src.len();
        let mut pos = start;

        while pos + 8 <= len {
            unsafe {
                let p = src.as_ptr().add(pos);
                if *IS_SPECIAL.get_unchecked(*p as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(1) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(2) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(3) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(4) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(5) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(6) as usize)
                    || *IS_SPECIAL.get_unchecked(*p.add(7) as usize)
                {
                    if *IS_SPECIAL.get_unchecked(*p as usize) { return pos; }
                    if *IS_SPECIAL.get_unchecked(*p.add(1) as usize) { return pos + 1; }
                    if *IS_SPECIAL.get_unchecked(*p.add(2) as usize) { return pos + 2; }
                    if *IS_SPECIAL.get_unchecked(*p.add(3) as usize) { return pos + 3; }
                    if *IS_SPECIAL.get_unchecked(*p.add(4) as usize) { return pos + 4; }
                    if *IS_SPECIAL.get_unchecked(*p.add(5) as usize) { return pos + 5; }
                    if *IS_SPECIAL.get_unchecked(*p.add(6) as usize) { return pos + 6; }
                    return pos + 7;
                }
            }
            pos += 8;
        }

        while pos < len {
            let b = unsafe { *src.get_unchecked(pos) };
            if unsafe { *IS_SPECIAL.get_unchecked(b as usize) } {
                return pos;
            }
            pos += 1;
        }

        pos
    }

    #[inline]
    fn scan_whitespace_block(&self, start: usize) -> (usize, usize) {
        let src = self.source;
        let len = src.len();
        let mut pos = start;
        let mut newline_count = 0u32;

        while pos < len {
            let b = unsafe { *src.get_unchecked(pos) };
            match b {
                b'\n' => { newline_count += 1; pos += 1; }
                b' ' | b'\t' | b'\r' => { pos += 1; }
                _ => break,
            }
        }

        (newline_count as usize, pos)
    }

    #[inline]
    fn skip_horizontal_whitespace(&self, start: usize) -> usize {
        let src = self.source;
        let len = src.len();
        let mut pos = start;

        while pos < len {
            let b = unsafe { *src.get_unchecked(pos) };
            if b != b' ' && b != b'\t' && b != b'\r' {
                return pos;
            }
            pos += 1;
        }

        pos
    }

    #[inline]
    fn lex_command(&mut self) -> Token {
        let start = self.pos;
        self.pos += 1;

        let len = self.source.len();
        if self.pos >= len {
            return Token::new(TokenKind::Command, start, self.pos - start);
        }

        let b = unsafe { *self.source.get_unchecked(self.pos) };
        if b.is_ascii_alphabetic() || b == b'@' {
            self.pos += 1;
            while self.pos < len {
                let c = unsafe { *self.source.get_unchecked(self.pos) };
                if c.is_ascii_alphabetic() || c == b'@' || c == b'*' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        } else {
            self.pos += 1;
        }

        let cmd_len = self.pos - start;
        let cmd_id = self.identify_command(start, cmd_len);
        Token::new_cmd(start, cmd_len, cmd_id)
    }

    #[inline]
    fn identify_command(&self, start: usize, len: usize) -> u8 {
        // Fast dispatch on command length + first char after backslash
        if len < 2 { return cmd_id::NONE; }
        let src = self.source;
        let first = unsafe { *src.get_unchecked(start + 1) };
        match (len, first) {
            (4, b'e') => {
                // \end
                if unsafe { *src.get_unchecked(start + 2) == b'n' && *src.get_unchecked(start + 3) == b'd' } {
                    return cmd_id::END;
                }
            }
            (4, b'r') => {
                if unsafe { *src.get_unchecked(start + 2) == b'e' && *src.get_unchecked(start + 3) == b'f' } {
                    return cmd_id::REF;
                }
            }
            (4, b'u') => {
                if unsafe { *src.get_unchecked(start + 2) == b'r' && *src.get_unchecked(start + 3) == b'l' } {
                    return cmd_id::URL;
                }
            }
            (5, b'e') => {
                if &src[start+1..start+5] == b"emph" { return cmd_id::EMPH; }
            }
            (5, b'c') => {
                if &src[start+1..start+5] == b"cite" { return cmd_id::CITE; }
            }
            (5, b'h') => {
                if &src[start+1..start+5] == b"href" { return cmd_id::HREF; }
            }
            (5, b'i') => {
                if &src[start+1..start+5] == b"item" { return cmd_id::ITEM; }
            }
            (5, b'd') => {
                if &src[start+1..start+5] == b"date" { return cmd_id::DATE; }
            }
            (6, b'b') => {
                if &src[start+1..start+6] == b"begin" { return cmd_id::BEGIN; }
            }
            (6, b'l') => {
                if &src[start+1..start+6] == b"label" { return cmd_id::LABEL; }
            }
            (6, b'c') => {
                if &src[start+1..start+6] == b"color" { return cmd_id::COLOR; }
            }
            (6, b't') => {
                if &src[start+1..start+6] == b"title" { return cmd_id::TITLE; }
            }
            (6, b'h') => {
                if &src[start+1..start+6] == b"hline" { return cmd_id::HLINE; }
            }
            (7, b'a') => {
                if &src[start+1..start+7] == b"author" { return cmd_id::AUTHOR; }
            }
            (7, b't') => {
                if &src[start+1..start+7] == b"textbf" { return cmd_id::TEXTBF; }
                if &src[start+1..start+7] == b"textit" { return cmd_id::TEXTIT; }
                if &src[start+1..start+7] == b"texttt" { return cmd_id::TEXTTT; }
            }
            (7, b'v') => {
                if &src[start+1..start+7] == b"vspace" { return cmd_id::VSPACE; }
            }
            (7, b'h') => {
                if &src[start+1..start+7] == b"hspace" { return cmd_id::HSPACE; }
            }
            (8, b's') => {
                if &src[start+1..start+8] == b"section" { return cmd_id::SECTION; }
            }
            (8, b'c') => {
                if &src[start+1..start+8] == b"caption" { return cmd_id::CAPTION; }
                if &src[start+1..start+8] == b"chapter" { return cmd_id::CHAPTER; }
            }
            (8, b'n') => {
                if &src[start+1..start+8] == b"newpage" { return cmd_id::NEWPAGE; }
            }
            (8, b'b') => {
                if &src[start+1..start+8] == b"bibitem" { return cmd_id::BIBITEM; }
            }
            (9, b's') => {
                if &src[start+1..start+9] == b"section*" { return cmd_id::SECTION_STAR; }
            }
            (9, b'f') => {
                if &src[start+1..start+9] == b"footnote" { return cmd_id::FOOTNOTE; }
            }
            (9, b'n') => {
                if &src[start+1..start+9] == b"noindent" { return cmd_id::NOINDENT; }
            }
            (10, b'm') => {
                if &src[start+1..start+10] == b"maketitle" { return cmd_id::MAKETITLE; }
            }
            (10, b'i') => {
                if &src[start+1..start+10] == b"icmltitle" { return cmd_id::ICMLTITLE; }
            }
            (10, b't') => {
                if &src[start+1..start+10] == b"textcolor" { return cmd_id::TEXTCOLOR; }
                if &src[start+1..start+10] == b"twocolumn" { return cmd_id::TWOCOLUMN; }
            }
            (10, b'o') => {
                if &src[start+1..start+10] == b"onecolumn" { return cmd_id::ONECOLUMN; }
            }
            (10, b'c') => {
                if &src[start+1..start+10] == b"centering" { return cmd_id::CENTERING; }
            }
            (11, b's') => {
                if &src[start+1..start+11] == b"subsection" { return cmd_id::SUBSECTION; }
            }
            (11, b'u') => {
                if &src[start+1..start+11] == b"usepackage" { return cmd_id::USEPACKAGE; }
            }
            (12, b's') => {
                if &src[start+1..start+12] == b"subsection*" { return cmd_id::SUBSECTION_STAR; }
            }
            (14, b'd') => {
                if &src[start+1..start+14] == b"documentclass" { return cmd_id::DOCUMENTCLASS; }
            }
            (14, b's') => {
                if &src[start+1..start+14] == b"subsubsection" { return cmd_id::SUBSUBSECTION; }
            }
            (16, b'i') => {
                if &src[start+1..start+16] == b"includegraphics" { return cmd_id::INCLUDEGRAPHICS; }
            }
            (16, b't') => {
                if &src[start+1..start+16] == b"tableofcontents" { return cmd_id::TABLEOFCONTENTS; }
            }
            _ => {}
        }
        cmd_id::NONE
    }
}

/// Parallel tokenization: splits source at paragraph breaks and lexes chunks in parallel.
/// Falls back to single-threaded when rayon is not available (WASM).
pub fn tokenize_parallel(source: &str) -> Vec<Token> {
    #[cfg(not(feature = "rayon"))]
    {
        return Lexer::new(source).tokenize();
    }

    #[cfg(feature = "rayon")]
    {
        return _tokenize_parallel_rayon(source);
    }
}

#[cfg(feature = "rayon")]
fn _tokenize_parallel_rayon(source: &str) -> Vec<Token> {
    use rayon::prelude::*;

    let bytes = source.as_bytes();
    let len = bytes.len();
    let num_threads = rayon::current_num_threads().max(1);

    // Only parallelize for large inputs
    if len < 500_000 || num_threads <= 1 {
        return Lexer::new(source).tokenize();
    }

    let chunk_size = len / num_threads;
    let mut split_points = Vec::with_capacity(num_threads + 1);
    split_points.push(0usize);

    for i in 1..num_threads {
        let target = i * chunk_size;
        // Find nearest paragraph break (\n\n) near target
        let search_start = target.saturating_sub(2000);
        let search_end = (target + 2000).min(len);
        let mut best = target;
        let mut j = search_start;
        while j + 1 < search_end {
            if bytes[j] == b'\n' && bytes[j + 1] == b'\n' {
                best = j + 2;
                if best >= target { break; }
            }
            j += 1;
        }
        // Avoid duplicate or out-of-order split points
        if best > *split_points.last().unwrap() && best < len {
            split_points.push(best);
        }
    }
    split_points.push(len);

    // Lex each chunk in parallel
    let chunk_tokens: Vec<Vec<Token>> = split_points.windows(2)
        .collect::<Vec<_>>()
        .par_iter()
        .map(|w| {
            let start = w[0];
            let end = w[1];
            let chunk = &source[start..end];
            let mut lexer = Lexer::new(chunk);
            let mut tokens = lexer.tokenize();
            // Remove the EOF token from each chunk (we'll add one at the end)
            if let Some(last) = tokens.last() {
                if last.kind == TokenKind::Eof {
                    tokens.pop();
                }
            }
            // Adjust positions to be relative to full source
            for tok in &mut tokens {
                tok.pos += start as u32;
            }
            tokens
        })
        .collect();

    // Merge token streams
    let total_tokens: usize = chunk_tokens.iter().map(|v| v.len()).sum();
    let mut merged = Vec::with_capacity(total_tokens + 1);
    for chunk in chunk_tokens {
        merged.extend_from_slice(&chunk);
    }
    merged.push(Token::new(TokenKind::Eof, len, 0));
    merged
}

#[allow(unreachable_code)]
fn _ensure_tokenize_parallel_compiles() {
    // This function exists to suppress "unreachable code" warnings from the
    // cfg-gated returns in tokenize_parallel. It is never called.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_text() {
        // Spaces are merged into text runs (optimization: reduces token count)
        let mut lexer = Lexer::new("hello world");
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Text);
        assert_eq!(tokens[0].text("hello world"), "hello world");
        assert_eq!(tokens[1].kind, TokenKind::Eof);
    }

    #[test]
    fn test_command() {
        let source = "\\textbf{bold}";
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Command);
        assert_eq!(tokens[0].text(source), "\\textbf");
        assert_eq!(tokens[1].kind, TokenKind::OpenBrace);
        assert_eq!(tokens[2].kind, TokenKind::Text);
        assert_eq!(tokens[2].text(source), "bold");
        assert_eq!(tokens[3].kind, TokenKind::CloseBrace);
        assert_eq!(tokens[4].kind, TokenKind::Eof);
    }

    #[test]
    fn test_long_text_run() {
        let input = "abcdefghijklmnopqrstuvwxyz0123456789";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Text);
        assert_eq!(tokens[0].text(input), input);
        assert_eq!(tokens[1].kind, TokenKind::Eof);
    }

    #[test]
    fn test_special_chars() {
        let mut lexer = Lexer::new("a~b&c");
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Text);
        assert_eq!(tokens[1].kind, TokenKind::Tilde);
        assert_eq!(tokens[2].kind, TokenKind::Text);
        assert_eq!(tokens[3].kind, TokenKind::Ampersand);
        assert_eq!(tokens[4].kind, TokenKind::Text);
    }

    #[test]
    fn test_par_break() {
        let mut lexer = Lexer::new("first\n\nsecond");
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenKind::Text);
        assert_eq!(tokens[1].kind, TokenKind::ParBreak);
        assert_eq!(tokens[2].kind, TokenKind::Text);
    }

    #[test]
    fn test_token_size() {
        assert_eq!(std::mem::size_of::<Token>(), 8, "Token should be exactly 8 bytes");
    }
}
