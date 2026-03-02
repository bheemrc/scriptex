use anyhow::Result;
use crate::lexer::TokenKind;
use crate::document::*;
use super::Parser;

impl<'a> Parser<'a> {
    pub(crate) fn parse_algorithm_float(&mut self, env_name: &str) -> Result<Option<Node>> {
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

    pub(crate) fn parse_algorithmic_body(&mut self, env_name: &str) -> Result<(Vec<AlgoLine>, bool)> {
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
    pub(crate) fn read_algo_line_content(&mut self) -> Vec<AlgoToken> {
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
}
