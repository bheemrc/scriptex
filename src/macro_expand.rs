/// Macro expansion engine for \def, \newcommand, \renewcommand, \let, \DeclareMathOperator
/// Two-pass approach: collect definitions from preamble, then expand in full source.

use std::collections::HashMap;

/// A macro definition
#[derive(Debug, Clone)]
struct MacroDef {
    /// Number of parameters (0-9)
    param_count: u8,
    /// Replacement text with #1..#9 placeholders
    body: String,
    /// Optional default for first argument (from \newcommand[n][default]{...})
    default_first: Option<String>,
}

/// Dimension-setting commands extracted from .sty files to inject into the preamble.
/// These are lines like \setlength\textwidth{...}, \addtolength\topmargin{...}, etc.
#[derive(Debug, Clone)]
pub struct StyleDimensions {
    /// Raw lines to inject before \begin{document}
    pub lines: Vec<String>,
}

/// Environment definition (from \newenvironment)
#[derive(Debug, Clone)]
struct EnvDef {
    begin_code: String,
    end_code: String,
}

/// Macro expansion engine
pub struct MacroEngine {
    macros: HashMap<String, MacroDef>,
    environments: HashMap<String, EnvDef>,
}

impl MacroEngine {
    pub fn new() -> Self {
        MacroEngine {
            macros: HashMap::new(),
            environments: HashMap::new(),
        }
    }

    /// Quick scan: does the source contain any macro definitions or built-in macros?
    /// Only scans the preamble (before \begin{document}) for speed on large files.
    pub fn has_macros(source: &str) -> bool {
        // Macro definitions are in the preamble — find \begin{document} and only scan up to it
        let scan_end = source.find("\\begin{document}")
            .unwrap_or_else(|| source.len().min(50000)); // fallback: scan first 50KB
        let preamble = &source[..scan_end];
        preamble.contains("\\def") || preamble.contains("\\newcommand")
            || preamble.contains("\\renewcommand") || preamble.contains("\\let\\")
            || preamble.contains("\\DeclarePairedDelimiter")
            || preamble.contains("\\providecommand")
            || preamble.contains("\\DeclareRobustCommand")
            || preamble.contains("\\newenvironment")
            || preamble.contains("\\DeclareMathOperator")
            || preamble.contains("\\today") || source[scan_end..source.len().min(scan_end + 10000)].contains("\\today")
    }

    /// Collect macro definitions from local .sty files referenced by \usepackage.
    /// Only collects "safe" user-facing macros (no @ internals).
    /// Also extracts dimension-setting commands (\setlength, \addtolength, etc.)
    /// and returns them for injection into the preamble.
    pub fn collect_style_definitions(&mut self, source: &str, base_dir: &std::path::Path) -> Vec<String> {
        let mut dim_lines = Vec::new();
        // Find preamble (before \begin{document})
        let preamble_end = source.find("\\begin{document}")
            .unwrap_or(source.len().min(50000));
        let preamble = &source[..preamble_end];

        // Find all \usepackage references
        let mut pos = 0;
        while let Some(idx) = preamble[pos..].find("\\usepackage") {
            pos += idx + 11;
            // Skip optional arg
            let rest = &preamble[pos..];
            let mut p = 0;
            while p < rest.len() && rest.as_bytes()[p] == b' ' { p += 1; }
            if p < rest.len() && rest.as_bytes()[p] == b'[' {
                if let Some(close) = rest[p..].find(']') {
                    p += close + 1;
                }
            }
            // Read braced package name(s)
            while p < rest.len() && rest.as_bytes()[p] == b' ' { p += 1; }
            if p < rest.len() && rest.as_bytes()[p] == b'{' {
                if let Some(close) = rest[p..].find('}') {
                    let names = &rest[p + 1..p + close];
                    for name in names.split(',') {
                        let name = name.trim();
                        if name.is_empty() { continue; }
                        let sty_path = base_dir.join(format!("{}.sty", name));
                        if sty_path.exists() {
                            if let Ok(content) = std::fs::read_to_string(&sty_path) {
                                // Collect into a temporary engine, then filter safe macros
                                let mut temp = MacroEngine::new();
                                temp.collect_definitions(&content);
                                // Only keep macros with no @ in name or body
                                for (name, def) in temp.macros {
                                    if !name.contains('@') && !def.body.contains('@') {
                                        self.macros.insert(name, def);
                                    }
                                }
                                for (name, def) in temp.environments {
                                    if !name.contains('@') && !def.begin_code.contains('@')
                                        && !def.end_code.contains('@') {
                                        self.environments.insert(name, def);
                                    }
                                }
                                // Extract dimension-setting commands from style file
                                extract_dimension_commands(&content, &mut dim_lines);
                            }
                        }
                    }
                }
            }
        }
        dim_lines
    }

    /// Collect dimension commands from .sty file content and store them for injection.
    /// Handles \setlength, \addtolength, and raw TeX dimension assignments.
    #[allow(dead_code)]
    pub fn style_dimensions(&self) -> &[String] {
        &[]
    }

    /// Collect all macro definitions from source (preamble + body)
    pub fn collect_definitions(&mut self, source: &str) {
        let bytes = source.as_bytes();
        let len = bytes.len();
        let mut pos = 0;

        while pos < len {
            // Skip to next backslash
            match memchr::memchr(b'\\', &bytes[pos..]) {
                None => break,
                Some(offset) => pos += offset,
            }

            // Try each definition form
            if source[pos..].starts_with("\\newcommand") || source[pos..].starts_with("\\renewcommand") || source[pos..].starts_with("\\providecommand") || source[pos..].starts_with("\\DeclareRobustCommand") {
                if let Some(new_pos) = self.parse_newcommand(source, pos) {
                    pos = new_pos;
                    continue;
                }
            } else if source[pos..].starts_with("\\def") && pos + 4 < len
                && (bytes[pos + 4] == b'\\' || bytes[pos + 4] == b' ' || bytes[pos + 4] == b'\n')
            {
                if let Some(new_pos) = self.parse_def(source, pos) {
                    pos = new_pos;
                    continue;
                }
            } else if source[pos..].starts_with("\\let") && pos + 4 < len
                && !bytes[pos + 4].is_ascii_alphabetic()
            {
                if let Some(new_pos) = self.parse_let(source, pos) {
                    pos = new_pos;
                    continue;
                }
            } else if source[pos..].starts_with("\\DeclareMathOperator") {
                if let Some(new_pos) = self.parse_declare_math_operator(source, pos) {
                    pos = new_pos;
                    continue;
                }
            } else if source[pos..].starts_with("\\DeclarePairedDelimiter") {
                if let Some(new_pos) = self.parse_declare_paired_delimiter(source, pos) {
                    pos = new_pos;
                    continue;
                }
            } else if source[pos..].starts_with("\\newenvironment") || source[pos..].starts_with("\\renewenvironment") {
                if let Some(new_pos) = self.parse_newenvironment(source, pos) {
                    pos = new_pos;
                    continue;
                }
            }

            pos += 1;
        }
    }

    /// Expand all macros in the source, returning new source string.
    /// Uses iterative expansion with recursion depth limit.
    pub fn expand(&self, source: &str) -> String {
        if self.macros.is_empty() && self.environments.is_empty() {
            return source.to_string();
        }

        let mut result = String::with_capacity(source.len() + source.len() / 4);
        let bytes = source.as_bytes();
        let len = bytes.len();
        let mut pos = 0;

        while pos < len {
            // Fast scan for backslash
            match memchr::memchr(b'\\', &bytes[pos..]) {
                None => {
                    result.push_str(&source[pos..]);
                    break;
                }
                Some(offset) => {
                    result.push_str(&source[pos..pos + offset]);
                    pos += offset;
                }
            }

            // Skip definition commands themselves (don't expand them, they're consumed)
            if source[pos..].starts_with("\\newcommand") || source[pos..].starts_with("\\renewcommand")
                || source[pos..].starts_with("\\providecommand")
            {
                if let Some(end) = self.skip_newcommand_def(source, pos) {
                    pos = end;
                    continue;
                }
            }
            if source[pos..].starts_with("\\def") && pos + 4 < len
                && (bytes[pos + 4] == b'\\' || bytes[pos + 4] == b' ' || bytes[pos + 4] == b'\n')
            {
                if let Some(end) = self.skip_def_def(source, pos) {
                    pos = end;
                    continue;
                }
            }
            if source[pos..].starts_with("\\let") && pos + 4 < len
                && !bytes[pos + 4].is_ascii_alphabetic()
            {
                if let Some(end) = self.skip_let_def(source, pos) {
                    pos = end;
                    continue;
                }
            }
            if source[pos..].starts_with("\\DeclareMathOperator") {
                if let Some(end) = self.skip_declare_math_op(source, pos) {
                    pos = end;
                    continue;
                }
            }
            if source[pos..].starts_with("\\newenvironment") || source[pos..].starts_with("\\renewenvironment") {
                if let Some(end) = self.skip_newenvironment_def(source, pos) {
                    pos = end;
                    continue;
                }
            }

            // Check for \begin{env} or \end{env} environment expansion
            if !self.environments.is_empty() {
                if source[pos..].starts_with("\\begin{") {
                    let after_begin = pos + 7; // length of \begin{
                    if let Some(close) = source[after_begin..].find('}') {
                        let env_name = &source[after_begin..after_begin + close];
                        if let Some(env_def) = self.environments.get(env_name) {
                            result.push_str(&env_def.begin_code);
                            pos = after_begin + close + 1;
                            continue;
                        }
                    }
                } else if source[pos..].starts_with("\\end{") {
                    let after_end = pos + 5; // length of \end{
                    if let Some(close) = source[after_end..].find('}') {
                        let env_name = &source[after_end..after_end + close];
                        if let Some(env_def) = self.environments.get(env_name) {
                            result.push_str(&env_def.end_code);
                            pos = after_end + close + 1;
                            continue;
                        }
                    }
                }
            }

            // Extract command name
            let cmd_start = pos;
            pos += 1; // skip backslash
            if pos < len && bytes[pos].is_ascii_alphabetic() {
                while pos < len && bytes[pos].is_ascii_alphabetic() {
                    pos += 1;
                }
                // Include trailing * if present
                if pos < len && bytes[pos] == b'*' {
                    pos += 1;
                }
            } else if pos < len {
                pos += 1; // single non-alpha char like \, \; etc.
            }

            let cmd_name = &source[cmd_start..pos];

            // Built-in expansions
            if cmd_name == "\\today" {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                // Convert to Y-M-D using civil day calculation
                let days = (now / 86400) as i64 + 719468;
                let era = days.div_euclid(146097);
                let doe = days.rem_euclid(146097);
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                let y = yoe + era * 400;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = doy - (153 * mp + 2) / 5 + 1;
                let m = if mp < 10 { mp + 3 } else { mp - 9 };
                let y = if m <= 2 { y + 1 } else { y };
                let month_name = match m {
                    1 => "January", 2 => "February", 3 => "March",
                    4 => "April", 5 => "May", 6 => "June",
                    7 => "July", 8 => "August", 9 => "September",
                    10 => "October", 11 => "November", 12 => "December",
                    _ => "?",
                };
                result.push_str(&format!("{} {}, {}", month_name, d, y));
                continue;
            }

            // Look up macro
            if let Some(macro_def) = self.macros.get(cmd_name) {
                // Read arguments
                let args = self.read_macro_args(source, &mut pos, macro_def);
                // Substitute
                let expanded = self.substitute(macro_def, &args);
                result.push_str(&expanded);
            } else {
                result.push_str(cmd_name);
            }
        }

        result
    }

    // ---- Definition parsers ----

    /// Parse \newcommand{\name}[n][default]{body} or \newcommand*
    fn parse_newcommand(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        // Skip \newcommand or \renewcommand or \providecommand
        while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes().get(pos) == Some(&b'\\') {
            pos += 1;
        }
        // Skip optional *
        if source.as_bytes().get(pos) == Some(&b'*') {
            pos += 1;
        }

        // Skip whitespace
        pos = skip_ws(source, pos);

        // Read command name: either {name} or just \name
        let name = if source.as_bytes().get(pos) == Some(&b'{') {
            let (n, end) = read_braced(source, pos)?;
            pos = end;
            n
        } else if source.as_bytes().get(pos) == Some(&b'\\') {
            let cmd_end = pos + 1;
            let mut ce = cmd_end;
            while ce < source.len() && source.as_bytes()[ce].is_ascii_alphabetic() {
                ce += 1;
            }
            let n = source[pos..ce].to_string();
            pos = ce;
            n
        } else {
            return None;
        };

        pos = skip_ws(source, pos);

        // Optional [n] - number of parameters
        let mut param_count = 0u8;
        let mut default_first = None;
        if source.as_bytes().get(pos) == Some(&b'[') {
            let (n_str, end) = read_bracketed(source, pos)?;
            pos = end;
            param_count = n_str.trim().parse().unwrap_or(0);

            pos = skip_ws(source, pos);

            // Optional [default] for first argument
            if source.as_bytes().get(pos) == Some(&b'[') {
                let (def, end2) = read_bracketed(source, pos)?;
                pos = end2;
                default_first = Some(def);
            }
        }

        pos = skip_ws(source, pos);

        // Read body {..}
        let (body, end) = read_braced(source, pos)?;
        pos = end;

        self.macros.insert(name, MacroDef { param_count, body, default_first });
        Some(pos)
    }

    /// Parse \def\name#1#2{body}
    fn parse_def(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start + 4; // skip \def
        pos = skip_ws(source, pos);

        // Read macro name
        if source.as_bytes().get(pos) != Some(&b'\\') {
            return None;
        }
        let name_start = pos;
        pos += 1;
        while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let name = source[name_start..pos].to_string();

        // Count #n parameter markers before the body
        let mut param_count = 0u8;
        loop {
            let pp = skip_ws(source, pos);
            if source.as_bytes().get(pp) == Some(&b'#') && pp + 1 < source.len()
                && source.as_bytes()[pp + 1].is_ascii_digit()
            {
                param_count += 1;
                pos = pp + 2;
            } else {
                pos = pp;
                break;
            }
        }

        // Read body {..}
        let (body, end) = read_braced(source, pos)?;

        self.macros.insert(name, MacroDef { param_count, body, default_first: None });
        Some(end)
    }

    /// Parse \let\foo\bar or \let\foo=\bar
    fn parse_let(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start + 4; // skip \let
        pos = skip_ws(source, pos);

        // Read target name
        if source.as_bytes().get(pos) != Some(&b'\\') {
            return None;
        }
        let name_start = pos;
        pos += 1;
        while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let name = source[name_start..pos].to_string();

        pos = skip_ws(source, pos);
        // Skip optional =
        if source.as_bytes().get(pos) == Some(&b'=') {
            pos += 1;
        }
        pos = skip_ws(source, pos);

        // Read source command
        if source.as_bytes().get(pos) == Some(&b'\\') {
            let src_start = pos;
            pos += 1;
            while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() {
                pos += 1;
            }
            let src_name = &source[src_start..pos];

            // If source is a known macro, copy it; otherwise create a passthrough
            if let Some(existing) = self.macros.get(src_name).cloned() {
                self.macros.insert(name, existing);
            } else {
                // Create alias that expands to the source command
                self.macros.insert(name, MacroDef {
                    param_count: 0,
                    body: src_name.to_string(),
                    default_first: None,
                });
            }
            Some(pos)
        } else {
            Some(pos)
        }
    }

    /// Parse \DeclareMathOperator{\Hom}{Hom}
    fn parse_declare_math_operator(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        // Skip \DeclareMathOperator or \DeclareMathOperator*
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'*') {
            pos += 1;
        }
        pos = skip_ws(source, pos);

        // Read command name {name}
        let (name, end1) = read_braced(source, pos)?;
        pos = skip_ws(source, end1);

        // Read operator text {text}
        let (text, end2) = read_braced(source, pos)?;

        // Expand to \operatorname{text}
        self.macros.insert(name, MacroDef {
            param_count: 0,
            body: format!("\\operatorname{{{}}}", text),
            default_first: None,
        });
        Some(end2)
    }

    /// Parse \DeclarePairedDelimiter{\norm}{\lVert}{\rVert}
    fn parse_declare_paired_delimiter(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        // Skip \DeclarePairedDelimiter or \DeclarePairedDelimiterX etc.
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'X') || source.as_bytes().get(pos) == Some(&b'*') {
            pos += 1;
        }
        pos = skip_ws(source, pos);

        // Read command name {\norm}
        let (name, end1) = read_braced(source, pos)?;
        pos = skip_ws(source, end1);

        // Read left delimiter {\lVert}
        let (left_delim, end2) = read_braced(source, pos)?;
        pos = skip_ws(source, end2);

        // Read right delimiter {\rVert}
        let (right_delim, end3) = read_braced(source, pos)?;

        // Expand \norm{x} to \left<left>x\right<right>
        self.macros.insert(name, MacroDef {
            param_count: 1,
            body: format!("\\left{}#1\\right{}", left_delim, right_delim),
            default_first: None,
        });
        Some(end3)
    }

    /// Parse \newenvironment{name}{begin_code}{end_code}
    fn parse_newenvironment(&mut self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        // Skip \newenvironment or \renewenvironment
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'*') { pos += 1; }
        pos = skip_ws(source, pos);

        // Read environment name {name}
        let (name, end1) = read_braced(source, pos)?;
        pos = skip_ws(source, end1);

        // Skip optional [n] parameter count
        if source.as_bytes().get(pos) == Some(&b'[') {
            let (_, end) = read_bracketed(source, pos)?;
            pos = end;
            pos = skip_ws(source, pos);
            // Skip optional [default]
            if source.as_bytes().get(pos) == Some(&b'[') {
                let (_, end2) = read_bracketed(source, pos)?;
                pos = end2;
                pos = skip_ws(source, pos);
            }
        }

        // Read begin code {begin_code}
        let (begin_code, end2) = read_braced(source, pos)?;
        pos = skip_ws(source, end2);

        // Read end code {end_code} (optional — some defs omit it)
        let end_code = if source.as_bytes().get(pos) == Some(&b'{') {
            let (ec, end3) = read_braced(source, pos)?;
            pos = end3;
            ec
        } else {
            String::new()
        };

        self.environments.insert(name, EnvDef { begin_code, end_code });
        Some(pos)
    }

    // ---- Skip definition commands during expansion ----

    fn skip_newcommand_def(&self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'*') { pos += 1; }
        pos = skip_ws(source, pos);

        // Skip name
        if source.as_bytes().get(pos) == Some(&b'{') {
            let (_, end) = read_braced(source, pos)?;
            pos = end;
        } else if source.as_bytes().get(pos) == Some(&b'\\') {
            pos += 1;
            while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() { pos += 1; }
        } else {
            return None;
        }
        pos = skip_ws(source, pos);

        // Skip [n]
        if source.as_bytes().get(pos) == Some(&b'[') {
            let (_, end) = read_bracketed(source, pos)?;
            pos = end;
            pos = skip_ws(source, pos);
            // Skip [default]
            if source.as_bytes().get(pos) == Some(&b'[') {
                let (_, end2) = read_bracketed(source, pos)?;
                pos = end2;
            }
        }
        pos = skip_ws(source, pos);
        // Skip body
        let (_, end) = read_braced(source, pos)?;
        Some(end)
    }

    fn skip_def_def(&self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start + 4;
        pos = skip_ws(source, pos);
        if source.as_bytes().get(pos) != Some(&b'\\') { return None; }
        pos += 1;
        while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() { pos += 1; }
        // Skip #n markers
        loop {
            let pp = skip_ws(source, pos);
            if source.as_bytes().get(pp) == Some(&b'#') && pp + 1 < source.len()
                && source.as_bytes()[pp + 1].is_ascii_digit()
            {
                pos = pp + 2;
            } else {
                pos = pp;
                break;
            }
        }
        let (_, end) = read_braced(source, pos)?;
        Some(end)
    }

    fn skip_let_def(&self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start + 4;
        pos = skip_ws(source, pos);
        if source.as_bytes().get(pos) != Some(&b'\\') { return None; }
        pos += 1;
        while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() { pos += 1; }
        pos = skip_ws(source, pos);
        if source.as_bytes().get(pos) == Some(&b'=') { pos += 1; }
        pos = skip_ws(source, pos);
        if source.as_bytes().get(pos) == Some(&b'\\') {
            pos += 1;
            while pos < source.len() && source.as_bytes()[pos].is_ascii_alphabetic() { pos += 1; }
        }
        Some(pos)
    }

    fn skip_declare_math_op(&self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'*') { pos += 1; }
        pos = skip_ws(source, pos);
        let (_, end1) = read_braced(source, pos)?;
        pos = skip_ws(source, end1);
        let (_, end2) = read_braced(source, pos)?;
        Some(end2)
    }

    fn skip_newenvironment_def(&self, source: &str, start: usize) -> Option<usize> {
        let mut pos = start;
        // Skip \newenvironment or \renewenvironment
        while pos < source.len() && (source.as_bytes()[pos].is_ascii_alphabetic() || source.as_bytes()[pos] == b'\\') {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'*') { pos += 1; }
        pos = skip_ws(source, pos);
        // Skip {name}
        let (_, end1) = read_braced(source, pos)?;
        pos = skip_ws(source, end1);
        // Skip optional [n]
        if source.as_bytes().get(pos) == Some(&b'[') {
            let (_, end) = read_bracketed(source, pos)?;
            pos = end;
            pos = skip_ws(source, pos);
            if source.as_bytes().get(pos) == Some(&b'[') {
                let (_, end2) = read_bracketed(source, pos)?;
                pos = end2;
                pos = skip_ws(source, pos);
            }
        }
        // Skip {begin_code}
        let (_, end2) = read_braced(source, pos)?;
        pos = skip_ws(source, end2);
        // Skip {end_code} if present
        if source.as_bytes().get(pos) == Some(&b'{') {
            let (_, end3) = read_braced(source, pos)?;
            pos = end3;
        }
        Some(pos)
    }

    // ---- Expansion helpers ----

    /// Read arguments for a macro invocation
    fn read_macro_args(&self, source: &str, pos: &mut usize, def: &MacroDef) -> Vec<String> {
        let mut args = Vec::with_capacity(def.param_count as usize);
        *pos = skip_ws(source, *pos);

        for i in 0..def.param_count {
            *pos = skip_ws(source, *pos);
            if *pos >= source.len() {
                // Use default for first arg if available
                if i == 0 {
                    if let Some(default) = &def.default_first {
                        args.push(default.clone());
                        continue;
                    }
                }
                args.push(String::new());
                continue;
            }

            // Handle optional first argument [default]
            if i == 0 && def.default_first.is_some() {
                if source.as_bytes().get(*pos) == Some(&b'[') {
                    if let Some((arg, end)) = read_bracketed(source, *pos) {
                        *pos = end;
                        args.push(arg);
                        continue;
                    }
                }
                // Use default
                args.push(def.default_first.as_ref().unwrap().clone());
                continue;
            }

            if source.as_bytes().get(*pos) == Some(&b'{') {
                if let Some((arg, end)) = read_braced(source, *pos) {
                    *pos = end;
                    args.push(arg);
                } else {
                    args.push(String::new());
                }
            } else {
                // Single token argument
                let start = *pos;
                if source.as_bytes().get(*pos) == Some(&b'\\') {
                    *pos += 1;
                    while *pos < source.len() && source.as_bytes()[*pos].is_ascii_alphabetic() {
                        *pos += 1;
                    }
                } else {
                    *pos += 1;
                }
                args.push(source[start..*pos].to_string());
            }
        }

        args
    }

    /// Substitute #1..#9 in body with arguments
    fn substitute(&self, def: &MacroDef, args: &[String]) -> String {
        let body = &def.body;
        let bytes = body.as_bytes();
        let len = bytes.len();
        let mut result = String::with_capacity(len + 32);
        let mut i = 0;

        while i < len {
            if bytes[i] == b'#' && i + 1 < len && bytes[i + 1].is_ascii_digit() {
                let idx = (bytes[i + 1] - b'0') as usize;
                if idx >= 1 && idx <= args.len() {
                    result.push_str(&args[idx - 1]);
                }
                i += 2;
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }

        result
    }
}

// ---- Free helper functions ----

fn skip_ws(source: &str, mut pos: usize) -> usize {
    let bytes = source.as_bytes();
    while pos < bytes.len() {
        match bytes[pos] {
            b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
            b'%' => {
                // Skip comment to end of line
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                if pos < bytes.len() { pos += 1; }
            }
            _ => break,
        }
    }
    pos
}

/// Read content between matching braces, returning (content, pos_after_close_brace)
fn read_braced(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut pos = start + 1;
    let mut depth = 1;
    let content_start = pos;

    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let content = source[content_start..pos].to_string();
                    return Some((content, pos + 1));
                }
            }
            b'\\' => {
                pos += 1; // skip escaped char
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

/// Read content between brackets [], returning (content, pos_after_close_bracket)
fn read_bracketed(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut pos = start + 1;
    let mut depth = 1;
    let content_start = pos;

    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((source[content_start..pos].to_string(), pos + 1));
                }
            }
            b'\\' => { pos += 1; }
            _ => {}
        }
        pos += 1;
    }
    None
}

/// Top-level expansion function: if source has macros, collect and expand them
pub fn expand(source: &str) -> String {
    expand_with_base_dir(source, None).unwrap_or_else(|| source.to_string())
}

/// Expand macros, also collecting definitions from local .sty files.
/// Returns None if no expansion was needed (caller can use original source).
pub fn expand_with_base_dir(source: &str, base_dir: Option<&std::path::Path>) -> Option<String> {
    let mut engine = MacroEngine::new();
    let mut dim_lines = Vec::new();

    // Collect macros from local .sty files referenced by \usepackage
    if let Some(dir) = base_dir {
        dim_lines = engine.collect_style_definitions(source, dir);
    }

    let has_source_macros = MacroEngine::has_macros(source);
    let has_dims = !dim_lines.is_empty();
    if !has_source_macros && engine.macros.is_empty() && engine.environments.is_empty() && !has_dims {
        return None;
    }

    if has_source_macros {
        engine.collect_definitions(source);
    }

    let has_builtins = source.contains("\\today");
    if engine.macros.is_empty() && engine.environments.is_empty() && !has_builtins && !has_dims {
        return None;
    }

    // Single-pass expansion is sufficient for most documents.
    let pass1 = engine.expand(source);

    // Quick check: does the expanded text still contain user macros?
    let result = if engine.macros.keys().any(|name| pass1.contains(name.as_str())) {
        engine.expand(&pass1)
    } else {
        pass1
    };

    // Inject dimension-setting commands from .sty files before \begin{document}
    if has_dims {
        if let Some(bd_pos) = result.find("\\begin{document}") {
            let mut out = String::with_capacity(result.len() + dim_lines.iter().map(|l| l.len() + 1).sum::<usize>());
            out.push_str(&result[..bd_pos]);
            out.push('\n');
            for line in &dim_lines {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&result[bd_pos..]);
            return Some(out);
        }
    }

    Some(result)
}

/// Extract dimension-setting commands from a .sty file.
/// Looks for \setlength, \addtolength, and raw TeX dimension assignments.
fn extract_dimension_commands(content: &str, out: &mut Vec<String>) {
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip comments and empty lines
        if trimmed.starts_with('%') || trimmed.is_empty() { continue; }
        // \setlength\textheight{9.0in} or \setlength{\textwidth}{6.75in}
        if trimmed.starts_with("\\setlength") {
            // Only keep lines setting known layout dimensions
            if trimmed.contains("textwidth") || trimmed.contains("textheight")
                || trimmed.contains("topmargin") || trimmed.contains("oddsidemargin")
                || trimmed.contains("evensidemargin") || trimmed.contains("columnsep")
                || trimmed.contains("headheight") || trimmed.contains("headsep")
            {
                out.push(trimmed.to_string());
            }
        }
        // \addtolength{\topmargin}{-0.29in}
        else if trimmed.starts_with("\\addtolength") {
            if trimmed.contains("textwidth") || trimmed.contains("textheight")
                || trimmed.contains("topmargin") || trimmed.contains("oddsidemargin")
                || trimmed.contains("evensidemargin") || trimmed.contains("columnsep")
            {
                out.push(trimmed.to_string());
            }
        }
        // Raw TeX assignments: \evensidemargin -0.23in
        else if trimmed.starts_with("\\evensidemargin") || trimmed.starts_with("\\oddsidemargin") {
            // Convert to \setlength form: \setlength\evensidemargin{-0.23in}
            let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
            if parts.len() == 2 {
                let val = parts[1].trim();
                if !val.is_empty() && !val.starts_with('\\') {
                    out.push(format!("\\setlength{{{}}}{{{}}}", parts[0], val));
                }
            }
        }
        // \twocolumn command anywhere on the line
        if trimmed.contains("\\twocolumn") {
            out.push("\\twocolumn".to_string());
        }
    }
}
