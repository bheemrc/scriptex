use anyhow::{Result, bail};
use crate::lexer::{TokenKind, cmd_id};
use crate::document::*;
use crate::color::Color;
use super::Parser;
use super::dimensions::parse_dimension_simple;

impl<'a> Parser<'a> {
    pub(super) fn parse_begin_environment(&mut self) -> Result<Option<Node>> {
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

    pub(super) fn parse_environment_body(&mut self, env_name: &str) -> Result<Vec<Node>> {
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

    pub(super) fn parse_float_environment(&mut self, env_name: &str, is_table: bool) -> Result<Option<Node>> {
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

    pub(super) fn parse_tcolorbox_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
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

    pub(super) fn parse_wrapfigure_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
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

    pub(super) fn parse_display_math_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
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
    pub(super) fn skip_environment_raw(&mut self, env_name: &str) -> Result<()> {
        self.capture_environment_raw(env_name)?;
        Ok(())
    }

    /// Capture the raw source text of an environment body, then skip past \end{env_name}
    pub(super) fn capture_environment_raw(&mut self, env_name: &str) -> Result<String> {
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

    pub(super) fn parse_verbatim_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
        let mut text = String::new();
        self.read_verbatim_content(env_name, &mut text)?;
        Ok(Some(Node::Verbatim(text)))
    }

    pub(super) fn read_verbatim_content(&mut self, env_name: &str, text: &mut String) -> Result<()> {
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
}
