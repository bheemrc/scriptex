/// Pre-scan AST to collect label→number mappings and TOC entries

use std::collections::HashMap;
use crate::document::*;
use super::text::node_to_text;

/// Pre-scan AST to collect label→display-number mappings for \ref resolution.
pub(super) struct LabelCollector<'a> {
    pub labels: HashMap<String, String>,
    pub citations: HashMap<String, u32>,
    fig_counter: u32,
    tbl_counter: u32,
    bib_counter: u32,
    eq_counter: u32,
    sec_counters: [u32; 7],
    theorem_counters: HashMap<String, u32>,
    pending_number: Option<String>,
    pending_is_section: bool,
    theorem_defs: &'a [TheoremDef],
}

pub(super) fn collect_labels(nodes: &[Node], doc: &Document) -> (HashMap<String, String>, HashMap<String, u32>) {
    let mut ctx = LabelCollector {
        labels: HashMap::new(),
        citations: HashMap::new(),
        fig_counter: 0,
        tbl_counter: 0,
        bib_counter: 0,
        eq_counter: 0,
        sec_counters: [0u32; 7],
        theorem_counters: HashMap::new(),
        pending_number: None,
        pending_is_section: false,
        theorem_defs: &doc.preamble.theorem_defs,
    };
    collect_labels_inner(nodes, &mut ctx);
    (ctx.labels, ctx.citations)
}

impl LabelCollector<'_> {
    fn current_section_str(&self) -> String {
        if self.sec_counters[4] > 0 {
            format!("{}.{}.{}", self.sec_counters[2], self.sec_counters[3], self.sec_counters[4])
        } else if self.sec_counters[3] > 0 {
            format!("{}.{}", self.sec_counters[2], self.sec_counters[3])
        } else if self.sec_counters[2] > 0 {
            format!("{}", self.sec_counters[2])
        } else {
            "??".to_string()
        }
    }
}

fn collect_labels_inner(nodes: &[Node], ctx: &mut LabelCollector) {
    for node in nodes {
        match node {
            Node::Section { level, numbered, title } => {
                if *numbered {
                    let idx = (level.depth() + 1).max(0) as usize;
                    if idx < ctx.sec_counters.len() {
                        ctx.sec_counters[idx] += 1;
                        for i in (idx + 1)..ctx.sec_counters.len() {
                            ctx.sec_counters[i] = 0;
                        }
                    }
                    if idx <= 2 {
                        ctx.theorem_counters.clear();
                    }
                    ctx.pending_number = None;
                    ctx.pending_is_section = true;
                }
                collect_labels_inner(title, ctx);
            }
            Node::Figure(fig) => {
                if fig.caption.is_some() {
                    ctx.fig_counter += 1;
                    if let Some(ref lbl) = fig.label {
                        ctx.labels.insert(lbl.clone(), ctx.fig_counter.to_string());
                    }
                }
                collect_labels_inner(&fig.content, ctx);
            }
            Node::Table(table) => {
                if table.caption.is_some() {
                    ctx.tbl_counter += 1;
                    if let Some(ref lbl) = table.label {
                        ctx.labels.insert(lbl.clone(), ctx.tbl_counter.to_string());
                    }
                }
            }
            Node::BibItem(key) => {
                ctx.bib_counter += 1;
                ctx.citations.insert(key.clone(), ctx.bib_counter);
            }
            Node::Theorem(thm) => {
                let counter_name = if let Some(def) = ctx.theorem_defs.iter()
                    .find(|d| d.env_name == thm.env_name)
                {
                    def.counter.clone().unwrap_or_else(|| thm.env_name.clone())
                } else {
                    thm.env_name.clone()
                };
                let count = ctx.theorem_counters.entry(counter_name).or_insert(0);
                *count += 1;
                let num = *count;
                let sec = ctx.sec_counters[2];
                let thm_label = if sec > 0 {
                    format!("{}.{}", sec, num)
                } else {
                    format!("{}", num)
                };
                ctx.pending_number = Some(thm_label);
                ctx.pending_is_section = false;
                collect_labels_inner(&thm.body, ctx);
            }
            Node::Proof(content) => {
                collect_labels_inner(content, ctx);
            }
            Node::DisplayMath(math_data) => {
                if math_data.numbered {
                    ctx.eq_counter += 1;
                    let eq_label = format!("{}", ctx.eq_counter);
                    ctx.pending_number = Some(eq_label);
                    ctx.pending_is_section = false;
                }
            }
            Node::Label(name) => {
                let num = if let Some(ref pending) = ctx.pending_number {
                    pending.clone()
                } else if ctx.pending_is_section {
                    ctx.current_section_str()
                } else {
                    ctx.current_section_str()
                };
                ctx.labels.insert(name.clone(), num);
            }
            Node::ItemizeList(items) | Node::EnumerateList(items) => {
                for item in items {
                    collect_labels_inner(&item.content, ctx);
                }
            }
            Node::Environment(env) => {
                collect_labels_inner(&env.content, ctx);
            }
            Node::Paragraph(c) | Node::Quote(c) | Node::Quotation(c) | Node::Abstract(c)
            | Node::Center(c) | Node::FlushLeft(c) | Node::FlushRight(c)
            | Node::Bold(c) | Node::Italic(c) | Node::Group(c) | Node::SmallCaps(c)
            | Node::Footnote(c) | Node::Colored { content: c, .. }
            | Node::Minipage { content: c, .. } => {
                collect_labels_inner(c, ctx);
            }
            _ => {}
        }
    }
}

/// Table of contents entry
pub(super) struct TocEntry {
    pub level: SectionLevel,
    pub number: String,
    pub title: String,
    pub page: u32,
}

/// TOC fixup: position where a page number should be stamped after layout
pub(super) struct TocFixup {
    pub elem_idx: u32,
    pub text_offset: u32,
    pub toc_idx: u32,
}

/// Pre-scan AST to collect section entries for table of contents
pub(super) fn collect_toc_entries(nodes: &[Node], source: &str) -> Vec<TocEntry> {
    let mut entries = Vec::new();
    let mut counters = [0u32; 7];
    let mut appendix = false;
    collect_toc_inner(nodes, &mut entries, &mut counters, &mut appendix, source);
    entries
}

fn collect_toc_inner(nodes: &[Node], entries: &mut Vec<TocEntry>, counters: &mut [u32; 7], appendix: &mut bool, source: &str) {
    for node in nodes {
        match node {
            Node::Appendix => {
                *appendix = true;
                counters[2] = 0;
                counters[3] = 0;
                counters[4] = 0;
            }
            Node::Section { level, title, numbered } => {
                let mut number = String::new();
                if *numbered {
                    let idx = (level.depth() + 1).max(0) as usize;
                    if idx < counters.len() {
                        counters[idx] += 1;
                        for i in (idx + 1)..counters.len() {
                            counters[i] = 0;
                        }
                    }
                    let mut ibuf = itoa::Buffer::new();
                    match level {
                        SectionLevel::Part => {
                            number.push_str("Part ");
                            number.push_str(ibuf.format(counters[0]));
                        }
                        SectionLevel::Chapter => {
                            number.push_str(ibuf.format(counters[1]));
                        }
                        SectionLevel::Section => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                        }
                        SectionLevel::Subsection => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                            number.push('.');
                            number.push_str(ibuf.format(counters[3]));
                        }
                        SectionLevel::Subsubsection => {
                            if *appendix {
                                let letter = (b'A' + (counters[2] - 1).min(25) as u8) as char;
                                number.push(letter);
                            } else {
                                number.push_str(ibuf.format(counters[2]));
                            }
                            number.push('.');
                            number.push_str(ibuf.format(counters[3]));
                            number.push('.');
                            number.push_str(ibuf.format(counters[4]));
                        }
                        _ => {}
                    }
                }
                if level.depth() <= 3 {
                    let mut title_text = String::new();
                    for n in title {
                        node_to_text(n, &mut title_text, source);
                    }
                    entries.push(TocEntry {
                        level: *level,
                        number,
                        title: title_text,
                        page: 0,
                    });
                }
            }
            Node::Paragraph(c) | Node::Group(c) => {
                collect_toc_inner(c, entries, counters, appendix, source);
            }
            _ => {}
        }
    }
}
