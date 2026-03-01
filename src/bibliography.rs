/// Bibliography module: parse .bib files and resolve citations
///
/// Supports:
/// - BibTeX (.bib) parsing via the `biblatex` crate
/// - Citation styles: numeric [1], author-year (Smith, 2024), alpha [Smi24]
/// - \cite, \citep, \citet commands
/// - \bibliography{} and \printbibliography commands

use std::path::Path;
use std::collections::HashMap;

/// A bibliography entry parsed from .bib file
#[derive(Debug, Clone)]
pub struct BibEntry {
    pub key: String,
    pub entry_type: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: String,
    pub journal: Option<String>,
    pub booktitle: Option<String>,
    pub volume: Option<String>,
    pub number: Option<String>,
    pub pages: Option<String>,
    pub publisher: Option<String>,
    pub doi: Option<String>,
    pub url: Option<String>,
    /// Assigned citation number (1-based)
    pub cite_number: u32,
}

/// Citation style
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CitationStyle {
    /// [1], [2], [3]
    Numeric,
    /// (Smith, 2024)
    AuthorYear,
    /// [Smi24]
    Alpha,
}

/// Bibliography database
#[derive(Debug)]
pub struct Bibliography {
    pub entries: Vec<BibEntry>,
    pub key_map: HashMap<String, usize>,
    pub style: CitationStyle,
    /// Order in which citations appear in the document
    pub cite_order: Vec<String>,
}

impl Bibliography {
    pub fn new() -> Self {
        Bibliography {
            entries: Vec::new(),
            key_map: HashMap::new(),
            style: CitationStyle::Numeric,
            cite_order: Vec::new(),
        }
    }

    /// Load a .bib file and parse it
    pub fn load_bib_file(&mut self, path: &Path, base_dir: &Path) -> Result<(), String> {
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        };

        // Try with and without .bib extension
        let content = std::fs::read_to_string(&full_path)
            .or_else(|_| {
                let with_ext = full_path.with_extension("bib");
                std::fs::read_to_string(&with_ext)
            })
            .map_err(|e| format!("Failed to read bib file {:?}: {}", full_path, e))?;

        self.parse_bib_content(&content)
    }

    /// Parse raw .bib content
    pub fn parse_bib_content(&mut self, content: &str) -> Result<(), String> {
        // Use biblatex crate if available, otherwise fall back to manual parsing
        match biblatex::Bibliography::parse(content) {
            Ok(bib) => {
                for entry in bib.iter() {
                    let key = entry.key.to_string();
                    let entry_type = format!("{:?}", entry.entry_type);

                    // Helper: extract text from chunks (slice of Spanned<Chunk>)
                    let chunks_to_string = |chunks: &[biblatex::Spanned<biblatex::Chunk>]| -> String {
                        chunks.iter().map(|sc| sc.v.get()).collect::<Vec<_>>().join("")
                    };

                    let title = entry.title().ok()
                        .map(|f| chunks_to_string(f))
                        .unwrap_or_default();

                    let authors: Vec<String> = entry.author().ok()
                        .map(|persons| {
                            persons.iter().map(|p| {
                                let given = &p.given_name;
                                let family = &p.name;
                                if given.is_empty() {
                                    family.to_string()
                                } else {
                                    format!("{} {}", given, family)
                                }
                            }).collect()
                        }).unwrap_or_default();

                    let year = entry.date().ok()
                        .and_then(|d| {
                            use biblatex::DateValue;
                            match d {
                                biblatex::PermissiveType::Typed(date) => {
                                    match date.value {
                                        DateValue::At(dt) | DateValue::After(dt) | DateValue::Before(dt) => {
                                            Some(format!("{}", dt.year))
                                        }
                                        DateValue::Between(dt, _) => Some(format!("{}", dt.year)),
                                    }
                                }
                                biblatex::PermissiveType::Chunks(chunks) => {
                                    Some(chunks_to_string(&chunks))
                                }
                            }
                        })
                        .or_else(|| {
                            entry.get("year").map(|v| chunks_to_string(v))
                        })
                        .unwrap_or_else(|| "n.d.".to_string());

                    let get_field = |name: &str| -> Option<String> {
                        entry.get(name).map(|v| chunks_to_string(v))
                    };

                    let bib_entry = BibEntry {
                        key: key.clone(),
                        entry_type,
                        title,
                        authors,
                        year,
                        journal: get_field("journal").or_else(|| get_field("journaltitle")),
                        booktitle: get_field("booktitle"),
                        volume: get_field("volume"),
                        number: get_field("number"),
                        pages: get_field("pages"),
                        publisher: get_field("publisher"),
                        doi: get_field("doi"),
                        url: get_field("url"),
                        cite_number: 0,
                    };

                    let idx = self.entries.len();
                    self.key_map.insert(key, idx);
                    self.entries.push(bib_entry);
                }
                Ok(())
            }
            Err(_e) => {
                // Fallback: basic manual parsing
                self.parse_bib_manual(content)
            }
        }
    }

    /// Manual .bib parser as fallback
    fn parse_bib_manual(&mut self, content: &str) -> Result<(), String> {
        let mut pos = 0;
        let bytes = content.as_bytes();
        let len = bytes.len();

        while pos < len {
            // Find @type{key,
            if bytes[pos] == b'@' {
                pos += 1;
                // Read entry type
                let type_start = pos;
                while pos < len && bytes[pos] != b'{' && !bytes[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                let entry_type = content[type_start..pos].to_lowercase();

                // Skip comments and strings
                if entry_type == "comment" || entry_type == "string" || entry_type == "preamble" {
                    // Skip to matching }
                    let mut depth = 0;
                    while pos < len {
                        if bytes[pos] == b'{' { depth += 1; }
                        if bytes[pos] == b'}' { depth -= 1; if depth == 0 { pos += 1; break; } }
                        pos += 1;
                    }
                    continue;
                }

                // Read key
                while pos < len && bytes[pos] != b'{' { pos += 1; }
                pos += 1; // skip {
                let key_start = pos;
                while pos < len && bytes[pos] != b',' && bytes[pos] != b'}' { pos += 1; }
                let key = content[key_start..pos].trim().to_string();
                if pos < len && bytes[pos] == b',' { pos += 1; }

                // Read fields
                let mut fields: HashMap<String, String> = HashMap::new();
                let mut depth = 1;
                while pos < len && depth > 0 {
                    // Skip whitespace
                    while pos < len && bytes[pos].is_ascii_whitespace() { pos += 1; }

                    if pos >= len || bytes[pos] == b'}' {
                        depth -= 1;
                        pos += 1;
                        continue;
                    }

                    // Read field name
                    let field_start = pos;
                    while pos < len && bytes[pos] != b'=' && bytes[pos] != b'}' { pos += 1; }
                    if pos >= len || bytes[pos] == b'}' { depth -= 1; pos += 1; continue; }
                    let field_name = content[field_start..pos].trim().to_lowercase();
                    pos += 1; // skip =

                    // Skip whitespace
                    while pos < len && bytes[pos].is_ascii_whitespace() { pos += 1; }

                    // Read field value
                    let value = if pos < len && bytes[pos] == b'{' {
                        pos += 1;
                        let val_start = pos;
                        let mut bdepth = 1;
                        while pos < len && bdepth > 0 {
                            if bytes[pos] == b'{' { bdepth += 1; }
                            if bytes[pos] == b'}' { bdepth -= 1; }
                            if bdepth > 0 { pos += 1; }
                        }
                        let val = content[val_start..pos].to_string();
                        if pos < len { pos += 1; } // skip closing }
                        val
                    } else if pos < len && bytes[pos] == b'"' {
                        pos += 1;
                        let val_start = pos;
                        while pos < len && bytes[pos] != b'"' { pos += 1; }
                        let val = content[val_start..pos].to_string();
                        if pos < len { pos += 1; }
                        val
                    } else {
                        let val_start = pos;
                        while pos < len && bytes[pos] != b',' && bytes[pos] != b'}' { pos += 1; }
                        content[val_start..pos].trim().to_string()
                    };

                    // Skip comma
                    while pos < len && (bytes[pos] == b',' || bytes[pos].is_ascii_whitespace()) { pos += 1; }

                    fields.insert(field_name, value);
                }

                let authors_str = fields.get("author").cloned().unwrap_or_default();
                let authors: Vec<String> = authors_str
                    .split(" and ")
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();

                let entry = BibEntry {
                    key: key.clone(),
                    entry_type: entry_type.clone(),
                    title: fields.get("title").cloned().unwrap_or_default(),
                    authors,
                    year: fields.get("year").cloned().unwrap_or_else(|| "n.d.".to_string()),
                    journal: fields.get("journal").or(fields.get("journaltitle")).cloned(),
                    booktitle: fields.get("booktitle").cloned(),
                    volume: fields.get("volume").cloned(),
                    number: fields.get("number").cloned(),
                    pages: fields.get("pages").cloned(),
                    publisher: fields.get("publisher").cloned(),
                    doi: fields.get("doi").cloned(),
                    url: fields.get("url").cloned(),
                    cite_number: 0,
                };

                let idx = self.entries.len();
                self.key_map.insert(key, idx);
                self.entries.push(entry);
            } else {
                pos += 1;
            }
        }

        Ok(())
    }

    /// Register a citation (called during first pass)
    pub fn register_citation(&mut self, key: &str) {
        // Handle multiple keys separated by commas
        for k in key.split(',') {
            let k = k.trim();
            if !k.is_empty() && !self.cite_order.contains(&k.to_string()) {
                self.cite_order.push(k.to_string());
            }
        }
    }

    /// Assign citation numbers based on citation order
    pub fn assign_numbers(&mut self) {
        for (i, key) in self.cite_order.iter().enumerate() {
            if let Some(&idx) = self.key_map.get(key) {
                self.entries[idx].cite_number = (i + 1) as u32;
            }
        }
    }

    /// Format a citation reference (inline text like [1] or (Smith, 2024))
    pub fn format_citation(&self, key: &str) -> String {
        // Handle multiple keys
        let keys: Vec<&str> = key.split(',').map(|k| k.trim()).collect();
        let mut parts = Vec::new();

        for k in &keys {
            if let Some(&idx) = self.key_map.get(*k) {
                let entry = &self.entries[idx];
                match self.style {
                    CitationStyle::Numeric => {
                        if entry.cite_number > 0 {
                            parts.push(format!("{}", entry.cite_number));
                        } else {
                            parts.push("?".to_string());
                        }
                    }
                    CitationStyle::AuthorYear => {
                        let author = if !entry.authors.is_empty() {
                            // Last name of first author
                            let first = &entry.authors[0];
                            first.split_whitespace().last().unwrap_or(first).to_string()
                        } else {
                            "Unknown".to_string()
                        };
                        if entry.authors.len() > 2 {
                            parts.push(format!("{} et al., {}", author, entry.year));
                        } else if entry.authors.len() == 2 {
                            let second = entry.authors[1].split_whitespace().last()
                                .unwrap_or(&entry.authors[1]).to_string();
                            parts.push(format!("{} and {}, {}", author, second, entry.year));
                        } else {
                            parts.push(format!("{}, {}", author, entry.year));
                        }
                    }
                    CitationStyle::Alpha => {
                        let alpha = if !entry.authors.is_empty() {
                            let first = &entry.authors[0];
                            let surname = first.split_whitespace().last().unwrap_or(first);
                            let prefix: String = surname.chars().take(3).collect();
                            let year_suffix: String = entry.year.chars().rev().take(2).collect::<String>().chars().rev().collect();
                            format!("{}{}", prefix, year_suffix)
                        } else {
                            "?".to_string()
                        };
                        parts.push(alpha);
                    }
                }
            } else {
                parts.push(format!("{}?", k));
            }
        }

        match self.style {
            CitationStyle::Numeric => format!("[{}]", parts.join(", ")),
            CitationStyle::AuthorYear => format!("({})", parts.join("; ")),
            CitationStyle::Alpha => format!("[{}]", parts.join(", ")),
        }
    }

    /// Format a bibliography entry for the references section
    pub fn format_entry(&self, entry: &BibEntry) -> String {
        let mut result = String::with_capacity(256);

        // Authors
        if !entry.authors.is_empty() {
            result.push_str(&entry.authors.join(", "));
            result.push_str(". ");
        }

        // Title
        if !entry.title.is_empty() {
            result.push_str(&clean_bib_text(&entry.title));
            result.push_str(". ");
        }

        // Journal/booktitle
        if let Some(ref journal) = entry.journal {
            result.push_str(&clean_bib_text(journal));
            // Volume
            if let Some(ref vol) = entry.volume {
                result.push_str(", ");
                result.push_str(vol);
                if let Some(ref num) = entry.number {
                    result.push('(');
                    result.push_str(num);
                    result.push(')');
                }
            }
            // Pages
            if let Some(ref pages) = entry.pages {
                result.push_str(":");
                result.push_str(pages);
            }
            result.push_str(", ");
        } else if let Some(ref booktitle) = entry.booktitle {
            result.push_str("In ");
            result.push_str(&clean_bib_text(booktitle));
            if let Some(ref pages) = entry.pages {
                result.push_str(", pages ");
                result.push_str(pages);
            }
            result.push_str(". ");
        }

        // Publisher
        if let Some(ref publisher) = entry.publisher {
            result.push_str(publisher);
            result.push_str(", ");
        }

        // Year
        result.push_str(&entry.year);
        result.push('.');

        result
    }

    /// Get entries in citation order for the bibliography section
    pub fn entries_in_order(&self) -> Vec<&BibEntry> {
        let mut ordered: Vec<&BibEntry> = self.cite_order.iter()
            .filter_map(|key| self.key_map.get(key).map(|&idx| &self.entries[idx]))
            .collect();

        // If no citations recorded, return all entries alphabetically
        if ordered.is_empty() {
            ordered = self.entries.iter().collect();
            ordered.sort_by(|a, b| a.key.cmp(&b.key));
        }

        ordered
    }

    /// Get entry by key
    pub fn get_entry(&self, key: &str) -> Option<&BibEntry> {
        self.key_map.get(key).map(|&idx| &self.entries[idx])
    }
}

/// Clean LaTeX formatting from bibliography text
fn clean_bib_text(text: &str) -> String {
    text.replace('{', "")
        .replace('}', "")
        .replace('~', " ")
        .replace("\\&", "&")
        .replace("\\'", "'")
        .replace("\\\"", "\"")
        .replace("\\textit", "")
        .replace("\\emph", "")
        .replace("\\textbf", "")
}
