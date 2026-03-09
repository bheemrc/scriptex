use anyhow::{Result, bail};
use crate::lexer::{TokenKind, cmd_id};
use crate::document::*;
use crate::font::FontId;
use super::Parser;

impl<'a> Parser<'a> {
    pub(crate) fn parse_math_until_dollar(&mut self) -> Result<Vec<MathNode>> {
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

    pub(crate) fn parse_math_until_double_dollar(&mut self) -> Result<Vec<MathNode>> {
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

    /// Parse math nodes until \] is encountered (for \[...\] display math)
    pub(crate) fn parse_math_until_close_bracket(&mut self) -> Result<Vec<MathNode>> {
        let mut nodes = Vec::new();
        loop {
            match self.current().kind {
                TokenKind::Eof => bail!("Unexpected end in \\[...\\] display math"),
                TokenKind::Command => {
                    let cmd = self.current_text();
                    if cmd == "\\]" {
                        self.advance();
                        break;
                    }
                    if let Some(node) = self.parse_math_node()? {
                        nodes.push(node);
                    }
                }
                _ => {
                    if let Some(node) = self.parse_math_node()? {
                        nodes.push(node);
                    }
                }
            }
        }
        Ok(nodes)
    }

    pub(crate) fn parse_math_node(&mut self) -> Result<Option<MathNode>> {
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

    pub(crate) fn parse_math_arg(&mut self) -> Result<Vec<MathNode>> {
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

    pub(crate) fn parse_math_command(&mut self, cmd: &str) -> Result<Option<MathNode>> {
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
            "\\bigcap" => Ok(Some(MathNode::Symbol("\u{22C2}".to_string()))),
            "\\bigcup" => Ok(Some(MathNode::Symbol("\u{22C3}".to_string()))),
            "\\bigwedge" => Ok(Some(MathNode::Symbol("\u{22C0}".to_string()))),
            "\\bigvee" => Ok(Some(MathNode::Symbol("\u{22C1}".to_string()))),
            "\\degree" => Ok(Some(MathNode::Symbol("\u{00B0}".to_string()))),
            "\\setminus" => Ok(Some(MathNode::Operator("\\".to_string()))),
            "\\oplus" => Ok(Some(MathNode::Operator("\u{2295}".to_string()))),
            "\\otimes" => Ok(Some(MathNode::Operator("\u{2297}".to_string()))),
            "\\wedge" | "\\land" => Ok(Some(MathNode::Operator("\u{2227}".to_string()))),
            "\\vee" | "\\lor" => Ok(Some(MathNode::Operator("\u{2228}".to_string()))),
            "\\mapsto" => Ok(Some(MathNode::Operator("\u{21A6}".to_string()))),
            "\\hookrightarrow" => Ok(Some(MathNode::Operator("\u{21AA}".to_string()))),
            "\\hookleftarrow" => Ok(Some(MathNode::Operator("\u{21A9}".to_string()))),
            "\\twoheadrightarrow" => Ok(Some(MathNode::Operator("\u{21A0}".to_string()))),
            "\\twoheadleftarrow" => Ok(Some(MathNode::Operator("\u{219E}".to_string()))),
            "\\longrightarrow" => Ok(Some(MathNode::Operator("\u{27F6}".to_string()))),
            "\\longleftarrow" => Ok(Some(MathNode::Operator("\u{27F5}".to_string()))),
            "\\longleftrightarrow" => Ok(Some(MathNode::Operator("\u{27F7}".to_string()))),
            "\\longhookrightarrow" => Ok(Some(MathNode::Operator("\u{27F6}".to_string()))), // approx as long→
            "\\longmapsto" => Ok(Some(MathNode::Operator("\u{27FC}".to_string()))),
            "\\Longrightarrow" => Ok(Some(MathNode::Operator("\u{27F9}".to_string()))),
            "\\Longleftarrow" => Ok(Some(MathNode::Operator("\u{27F8}".to_string()))),
            "\\Longleftrightarrow" => Ok(Some(MathNode::Operator("\u{27FA}".to_string()))),
            "\\cong" => Ok(Some(MathNode::Operator("\u{2245}".to_string()))),
            "\\simeq" => Ok(Some(MathNode::Operator("\u{2243}".to_string()))),
            "\\propto" => Ok(Some(MathNode::Operator("\u{221D}".to_string()))),
            "\\perp" => Ok(Some(MathNode::Operator("\u{22A5}".to_string()))),
            "\\parallel" => Ok(Some(MathNode::Operator("\u{2225}".to_string()))),
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
            | "\\dim" | "\\codim" | "\\ker" | "\\coker"
            | "\\deg" | "\\hom" | "\\arg"
            | "\\var" | "\\cov" | "\\sgn"
            | "\\tr" | "\\diag" | "\\rank" | "\\lcm"
            | "\\Hom" | "\\End" | "\\Aut"
            | "\\Spec" | "\\Proj" | "\\GL" | "\\SL"
            | "\\Ann" | "\\Tor" | "\\Ext" | "\\Mor"
            | "\\im" | "\\coim" | "\\id" | "\\ord" | "\\card" | "\\supp" => {
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
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Acute }))
            }
            "\\grave" => {
                let base = self.parse_math_arg()?;
                Ok(Some(MathNode::Accent { base, accent_type: AccentType::Grave }))
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
    pub(crate) fn expand_si_unit(&self, unit: &str) -> String {
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
    pub(crate) fn read_math_delimiter(&mut self) -> String {
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

    pub(crate) fn parse_limits(&mut self) -> Result<(Option<Vec<MathNode>>, Option<Vec<MathNode>>)> {
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
    pub(crate) fn parse_math_matrix_body(&mut self, env_name: &str) -> Result<Vec<Vec<Vec<MathNode>>>> {
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
    pub(crate) fn parse_math_cases_body(&mut self, env_name: &str) -> Result<Vec<(Vec<MathNode>, Option<Vec<MathNode>>)>> {
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
    pub(crate) fn skip_math_env_body(&mut self, env_name: &str) -> Result<()> {
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
