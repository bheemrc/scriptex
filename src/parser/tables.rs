use anyhow::{Result, bail};
use crate::lexer::{TokenKind, cmd_id};
use crate::document::*;
use super::Parser;

impl<'a> Parser<'a> {
    pub(crate) fn parse_tabular_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
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
                                // Check if pending content is only whitespace (e.g. between \hline and \end)
                                let has_real_content = current_cell_content.iter().any(|n| match n {
                                    Node::Text(s) => !s.trim().is_empty(),
                                    Node::TextRef(off, len) => {
                                        let src = &self.source[*off as usize..(*off as usize + *len as usize)];
                                        !src.trim().is_empty()
                                    }
                                    _ => true,
                                });
                                // Finish current cell/row (skip whitespace-only trailing rows)
                                if has_real_content || !current_cells.is_empty() {
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

        // If a trailing \hline was pending (no row after it), apply to last row
        if hline_before_next {
            if let Some(last_row) = rows.last_mut() {
                last_row.hline_after = true;
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

    pub(crate) fn parse_column_spec(&self, spec: &str) -> Vec<ColumnSpec> {
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
