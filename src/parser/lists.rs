use anyhow::{Result, bail};
use crate::lexer::{TokenKind, cmd_id};
use crate::document::*;
use super::Parser;

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

impl<'a> Parser<'a> {
    pub(super) fn parse_list_environment(&mut self, env_name: &str) -> Result<Option<Node>> {
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
                                    // Only push final item if it has non-whitespace content
                                    let has_content = current_content.iter().any(|n| match n {
                                        Node::Text(s) => !s.trim().is_empty(),
                                        Node::TextRef(off, len) => !self.source[*off as usize..(*off as usize + *len as usize)].trim().is_empty(),
                                        Node::Paragraph(c) | Node::Group(c) => c.iter().any(|n2| match n2 {
                                            Node::Text(s) => !s.trim().is_empty(),
                                            Node::TextRef(o, l) => !self.source[*o as usize..(*o as usize + *l as usize)].trim().is_empty(),
                                            _ => true,
                                        }),
                                        Node::Label(_) | Node::HSpace(_) | Node::VSpace(_) | Node::NonBreakingSpace => false,
                                        _ => true,
                                    });
                                    if has_content || current_label.is_some() {
                                        items.push(ListItem {
                                            label: current_label.take(),
                                            content: std::mem::take(&mut current_content),
                                        });
                                    }
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
}
