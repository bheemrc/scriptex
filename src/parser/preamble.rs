use anyhow::Result;
use crate::lexer::TokenKind;
use crate::document::*;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_document_class(&mut self) -> Result<DocumentClass> {
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

    pub(super) fn parse_preamble(&mut self) -> Result<Preamble> {
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
                                    "hyperref" => {
                                        for opt in &options {
                                            let opt = opt.trim();
                                            if opt == "colorlinks" {
                                                preamble.hyperref.color_links = true;
                                            } else if let Some(val) = opt.strip_prefix("linkcolor=") {
                                                preamble.hyperref.link_color = Some(val.trim().to_string());
                                            } else if let Some(val) = opt.strip_prefix("urlcolor=") {
                                                preamble.hyperref.url_color = Some(val.trim().to_string());
                                            } else if let Some(val) = opt.strip_prefix("citecolor=") {
                                                preamble.hyperref.cite_color = Some(val.trim().to_string());
                                            }
                                        }
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
                                "\\arraystretch" => {
                                    let val = self.read_braced_text().unwrap_or_default();
                                    if let Ok(f) = val.trim().parse::<f32>() {
                                        preamble.array_stretch = f;
                                    }
                                }
                                _ => { self.skip_command_args(); }
                            }
                        }
                        "\\setlength" => {
                            self.advance();
                            // Handle both \setlength{\textwidth}{...} and \setlength\textwidth{...}
                            let name_result = if self.current().kind == TokenKind::OpenBrace {
                                self.read_braced_text()
                            } else if self.current().kind == TokenKind::Command {
                                let cmd = self.current().text(self.source).to_string();
                                self.advance();
                                Ok(cmd)
                            } else {
                                Err(anyhow::anyhow!("expected name"))
                            };
                            if let (Ok(name), Ok(val)) = (name_result, self.read_braced_text()) {
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
                        "\\addtolength" => {
                            self.advance();
                            let name_result = if self.current().kind == TokenKind::OpenBrace {
                                self.read_braced_text()
                            } else if self.current().kind == TokenKind::Command {
                                let cmd = self.current().text(self.source).to_string();
                                self.advance();
                                Ok(cmd)
                            } else {
                                Err(anyhow::anyhow!("expected name"))
                            };
                            if let (Ok(name), Ok(val)) = (name_result, self.read_braced_text()) {
                                if let Some(pts) = self.parse_dimension(&val) {
                                    match name.trim_start_matches('\\') {
                                        "topmargin" => preamble.page_setup.margin_top += pts,
                                        "oddsidemargin" | "evensidemargin" => preamble.page_setup.margin_left += pts,
                                        "textheight" => {
                                            preamble.page_setup.margin_bottom -= pts;
                                        }
                                        "textwidth" => {
                                            preamble.page_setup.margin_right -= pts;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        "\\thispagestyle"
                        | "\\newcommand" | "\\def"
                        | "\\DeclareMathOperator"
                        | "\\bibliographystyle"
                        | "\\hypersetup" | "\\lstset" | "\\graphicspath"
                        | "\\numberwithin" => {
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
                    "hmargin" => {
                        setup.margin_left = points;
                        setup.margin_right = points;
                    }
                    "vmargin" => {
                        setup.margin_top = points;
                        setup.margin_bottom = points;
                    }
                    "textwidth" => {
                        let total = points + setup.margin_left + setup.margin_right;
                        // Adjust margins symmetrically to achieve desired text width
                        let margin = (setup.width - points) / 2.0;
                        setup.margin_left = margin;
                        setup.margin_right = margin;
                        let _ = total; // suppress warning
                    }
                    "textheight" => {
                        let margin = (setup.height - points) / 2.0;
                        setup.margin_top = margin;
                        setup.margin_bottom = margin;
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
