/// Cross-reference module: label/ref resolution and table of contents generation
///
/// Implements two-pass compilation:
/// - Pass 1: Layout runs, collecting label positions (page numbers, section numbers)
/// - Pass 2: Layout runs again with resolved references
///
/// Also generates table of contents from section headings

use std::collections::HashMap;
use crate::document::SectionLevel;

/// A collected label with its resolved information
#[derive(Debug, Clone)]
pub struct LabelInfo {
    /// Page number where the label appears
    pub page_number: u32,
    /// Section number string (e.g., "3.2.1")
    pub section_number: String,
    /// Section title (for \nameref)
    pub section_title: String,
    /// Type of labeled item
    pub label_type: LabelType,
    /// For equation/figure/table numbering
    pub counter_value: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LabelType {
    Section,
    Equation,
    Figure,
    Table,
    Item,
    Footnote,
    Custom(String),
}

/// Table of contents entry
#[derive(Debug, Clone)]
pub struct TocEntry {
    pub level: SectionLevel,
    pub number: String,
    pub title: String,
    pub page_number: u32,
}

/// Cross-reference database, built during pass 1
#[derive(Debug)]
pub struct CrossRefDb {
    /// label_name -> LabelInfo
    pub labels: HashMap<String, LabelInfo>,
    /// Table of contents entries (ordered)
    pub toc_entries: Vec<TocEntry>,
    /// Equation counter
    pub equation_counter: u32,
    /// Figure counter
    pub figure_counter: u32,
    /// Table counter
    pub table_counter: u32,
    /// Current section number parts
    pub section_counters: [u32; 7],
}

impl CrossRefDb {
    pub fn new() -> Self {
        CrossRefDb {
            labels: HashMap::new(),
            toc_entries: Vec::new(),
            equation_counter: 0,
            figure_counter: 0,
            table_counter: 0,
            section_counters: [0; 7],
        }
    }

    /// Register a section heading (updates counters, creates TOC entry)
    pub fn register_section(&mut self, level: SectionLevel, title: &str, numbered: bool, page_number: u32) {
        if !numbered {
            // Unnumbered sections still go in TOC
            self.toc_entries.push(TocEntry {
                level,
                number: String::new(),
                title: title.to_string(),
                page_number,
            });
            return;
        }

        let idx = (level.depth() + 1).max(0) as usize;
        if idx < self.section_counters.len() {
            self.section_counters[idx] += 1;
            for i in (idx + 1)..self.section_counters.len() {
                self.section_counters[i] = 0;
            }
        }

        let number = self.format_section_number(level);

        self.toc_entries.push(TocEntry {
            level,
            number: number.clone(),
            title: title.to_string(),
            page_number,
        });
    }

    /// Format the current section number as a string
    pub fn format_section_number(&self, level: SectionLevel) -> String {
        match level {
            SectionLevel::Part => {
                format!("Part {}", self.section_counters[0])
            }
            SectionLevel::Chapter => {
                format!("{}", self.section_counters[1])
            }
            SectionLevel::Section => {
                format!("{}", self.section_counters[2])
            }
            SectionLevel::Subsection => {
                format!("{}.{}", self.section_counters[2], self.section_counters[3])
            }
            SectionLevel::Subsubsection => {
                format!("{}.{}.{}", self.section_counters[2], self.section_counters[3], self.section_counters[4])
            }
            SectionLevel::Paragraph => {
                format!("{}", self.section_counters[5])
            }
            SectionLevel::Subparagraph => {
                format!("{}", self.section_counters[6])
            }
        }
    }

    /// Register a label
    pub fn register_label(&mut self, name: &str, page_number: u32, label_type: LabelType) {
        let (section_number, counter_value) = match label_type {
            LabelType::Section => (self.current_section_string(), 0),
            LabelType::Equation => {
                self.equation_counter += 1;
                (format!("{}", self.equation_counter), self.equation_counter)
            }
            LabelType::Figure => {
                self.figure_counter += 1;
                (format!("{}", self.figure_counter), self.figure_counter)
            }
            LabelType::Table => {
                self.table_counter += 1;
                (format!("{}", self.table_counter), self.table_counter)
            }
            _ => (self.current_section_string(), 0),
        };

        let section_title = self.toc_entries.last()
            .map(|e| e.title.clone())
            .unwrap_or_default();

        self.labels.insert(name.to_string(), LabelInfo {
            page_number,
            section_number,
            section_title,
            label_type,
            counter_value,
        });
    }

    /// Get the current section number string
    fn current_section_string(&self) -> String {
        // Find the deepest non-zero counter
        if self.section_counters[4] > 0 {
            format!("{}.{}.{}", self.section_counters[2], self.section_counters[3], self.section_counters[4])
        } else if self.section_counters[3] > 0 {
            format!("{}.{}", self.section_counters[2], self.section_counters[3])
        } else if self.section_counters[2] > 0 {
            format!("{}", self.section_counters[2])
        } else if self.section_counters[1] > 0 {
            format!("{}", self.section_counters[1])
        } else {
            String::new()
        }
    }

    /// Resolve a \ref{label} to its display text
    pub fn resolve_ref(&self, label: &str) -> String {
        if let Some(info) = self.labels.get(label) {
            if info.counter_value > 0 {
                format!("{}", info.counter_value)
            } else {
                info.section_number.clone()
            }
        } else {
            "??".to_string()
        }
    }

    /// Resolve a \pageref{label} to its page number
    pub fn resolve_pageref(&self, label: &str) -> String {
        if let Some(info) = self.labels.get(label) {
            format!("{}", info.page_number)
        } else {
            "??".to_string()
        }
    }

    /// Resolve an \eqref{label} to its display text
    pub fn resolve_eqref(&self, label: &str) -> String {
        if let Some(info) = self.labels.get(label) {
            format!("({})", info.counter_value)
        } else {
            "(??)".to_string()
        }
    }

    /// Check if references have changed between passes
    /// (If all labels are the same, no need for additional passes)
    pub fn has_changed(&self, other: &CrossRefDb) -> bool {
        if self.labels.len() != other.labels.len() {
            return true;
        }
        for (key, info) in &self.labels {
            if let Some(other_info) = other.labels.get(key) {
                if info.page_number != other_info.page_number
                    || info.section_number != other_info.section_number
                    || info.counter_value != other_info.counter_value
                {
                    return true;
                }
            } else {
                return true;
            }
        }
        false
    }
}

/// Generate PDF bookmark entries (outlines) from TOC
pub fn generate_pdf_bookmarks(toc: &[TocEntry], page_ids: &[u32]) -> Vec<u8> {
    if toc.is_empty() {
        return Vec::new();
    }

    let mut buf = Vec::with_capacity(toc.len() * 128);

    // This would generate the /Outlines dictionary and child entries
    // For a first implementation, we embed the outline in the catalog
    // Each entry: << /Title (text) /Parent outlines_ref /Dest [page_ref /Fit] >>

    // Simplified: just the titles for now
    for entry in toc {
        if entry.level.depth() <= 2 { // Only include sections and above
            let title = if entry.number.is_empty() {
                entry.title.clone()
            } else {
                format!("{} {}", entry.number, entry.title)
            };
            // Will be used by PDF writer
            buf.extend_from_slice(title.as_bytes());
            buf.push(b'\n');
        }
    }

    buf
}
