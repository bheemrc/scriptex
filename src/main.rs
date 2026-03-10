use anyhow::Result;
use clap::Parser as ClapParser;
use std::path::PathBuf;
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use scriptex::parser::Parser;
use scriptex::{lexer, layout, pdf, macro_expand, bibliography};

#[derive(ClapParser, Debug)]
#[command(name = "scriptex", about = "Blazing fast LaTeX to PDF compiler")]
struct Args {
    /// Input LaTeX file
    input: PathBuf,

    /// Output PDF file (defaults to input with .pdf extension)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Number of threads (0 = auto)
    #[arg(short = 'j', long, default_value = "0")]
    threads: usize,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    }

    if args.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(args.threads)
            .build_global()
            .ok();
    }

    let total_start = Instant::now();

    // Read input via mmap (avoids 97MB allocation + copy)
    let read_start = Instant::now();
    let file = std::fs::File::open(&args.input)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    // Pre-fault pages into memory for faster random access
    mmap.advise(memmap2::Advice::WillNeed)?;
    let source = std::str::from_utf8(&mmap)
        .map_err(|e| anyhow::anyhow!("Input file is not valid UTF-8: {}", e))?;
    let read_time = read_start.elapsed();
    eprintln!("[READ]    {:.3}ms - Read {} bytes", read_time.as_secs_f64() * 1000.0, source.len());

    // Resolve \input{} and \include{} commands (inline external files)
    let base_dir = args.input.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let included;
    let source_after_includes: &str = if source.contains("\\input") || source.contains("\\include{") {
        included = resolve_inputs(source, &base_dir, 0);
        &included
    } else {
        source
    };

    // Macro expansion (if needed)
    let expand_start = Instant::now();
    let expanded;
    let effective_source: &str = if let Some(exp) = macro_expand::expand_with_base_dir(source_after_includes, Some(&base_dir)) {
        expanded = exp;
        eprintln!("[MACROS]  {:.3}ms - expanded {} → {} bytes",
            expand_start.elapsed().as_secs_f64() * 1000.0, source_after_includes.len(), expanded.len());
        &expanded
    } else {
        eprintln!("[MACROS]  {:.3}ms - none found",
            expand_start.elapsed().as_secs_f64() * 1000.0);
        source_after_includes
    };

    // Lex (parallel for large files)
    let lex_start = Instant::now();
    let tokens = lexer::tokenize_parallel(effective_source);
    let lex_time = lex_start.elapsed();
    eprintln!("[LEX]     {:.3}ms - {} tokens", lex_time.as_secs_f64() * 1000.0, tokens.len());

    // Parse
    let parse_start = Instant::now();
    let mut parser = Parser::new(tokens, effective_source);
    let mut doc = parser.parse()?;
    let parse_time = parse_start.elapsed();
    eprintln!("[PARSE]   {:.3}ms", parse_time.as_secs_f64() * 1000.0);

    // Bibliography resolution: load .bib files from the same directory
    let bib_start = Instant::now();
    let (bib_loaded, author_year_map) = resolve_bibliography_from_dir(&mut doc, effective_source, &base_dir);
    if bib_loaded {
        eprintln!("[BIB]     {:.3}ms", bib_start.elapsed().as_secs_f64() * 1000.0);
    }

    // Layout
    let layout_start = Instant::now();
    let base_dir_str = base_dir.to_string_lossy().to_string();
    let layout_result = layout::layout_document_full(
        &doc, effective_source,
        std::collections::HashMap::new(),
        author_year_map,
        base_dir_str,
    )?;
    let layout_time = layout_start.elapsed();
    eprintln!("[LAYOUT]  {:.3}ms - {} pages", layout_time.as_secs_f64() * 1000.0, layout_result.num_pages());

    // Generate PDF
    let pdf_start = Instant::now();
    let output_path = args.output.unwrap_or_else(|| {
        let mut p = args.input.clone();
        p.set_extension("pdf");
        p
    });
    let bytes_written = pdf::generate_pdf(&layout_result, &doc, &output_path, effective_source)?;
    let pdf_time = pdf_start.elapsed();
    eprintln!("[PDF]     {:.3}ms - wrote {} bytes", pdf_time.as_secs_f64() * 1000.0, bytes_written);

    let total_time = total_start.elapsed();
    eprintln!("[TOTAL]   {:.3}ms", total_time.as_secs_f64() * 1000.0);
    eprintln!("Output: {}", output_path.display());

    Ok(())
}

/// Load .bib files and resolve citations for the CLI pipeline.
/// Scans for \bibliography{} commands, loads the referenced .bib files,
/// collects citations from the AST, and appends a reference section.
fn resolve_bibliography_from_dir(
    doc: &mut scriptex::document::Document,
    source: &str,
    base_dir: &std::path::Path,
) -> (bool, std::collections::HashMap<String, (String, String)>) {
    use scriptex::document::{Node, EnvironmentData};

    // Find \bibliography{filename} commands in source
    let mut bib_filenames = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let rest = &source[i..];
            let cmd_len = if rest.starts_with("\\bibliography{") {
                14
            } else if rest.starts_with("\\addbibresource{") {
                16
            } else {
                i += 1;
                continue;
            };
            let after = &rest[cmd_len..];
            if let Some(close) = after.find('}') {
                for name in after[..close].split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        if name.ends_with(".bib") {
                            bib_filenames.push(name.to_string());
                        } else {
                            bib_filenames.push(format!("{}.bib", name));
                        }
                    }
                }
                i += cmd_len + close + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    if bib_filenames.is_empty() {
        return (false, std::collections::HashMap::new());
    }

    // Load .bib files
    let mut bib = bibliography::Bibliography::new();
    let mut loaded = false;
    for filename in &bib_filenames {
        let path = base_dir.join(filename);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if bib.parse_bib_content(&content).is_ok() {
                    loaded = true;
                }
            }
        }
    }

    if !loaded || bib.entries.is_empty() {
        return (false, std::collections::HashMap::new());
    }

    // Collect citation keys from AST
    let mut citation_keys = Vec::new();
    collect_citation_keys_recursive(&doc.body, &mut citation_keys);

    // Register and assign numbers
    for key in &citation_keys {
        bib.register_citation(key);
    }
    bib.assign_numbers();

    // Check for existing thebibliography environment with content
    let has_manual_bib = doc.body.iter().any(|node| {
        matches!(node, Node::Environment(env) if env.name == "thebibliography" && !env.content.is_empty())
    });

    // Get author-year map for natbib citations
    let ay_map = bib.author_year_map();

    // Generate reference section nodes
    if !has_manual_bib && !bib.cite_order.is_empty() {
        let mut content: Vec<Node> = Vec::new();
        let entries = bib.entries_in_order();
        for entry in &entries {
            content.push(Node::BibItem(entry.key.clone()));
            let formatted = bib.format_entry(entry);
            content.push(Node::Text(formatted));
        }
        let bib_env = Node::Environment(Box::new(EnvironmentData {
            name: "thebibliography".to_string(),
            args: vec![],
            content,
        }));

        // Replace empty \printbibliography placeholder if present, otherwise append
        let mut replaced = false;
        for node in doc.body.iter_mut() {
            if let Node::Environment(env) = node {
                if env.name == "thebibliography" && env.content.is_empty() {
                    *node = bib_env.clone();
                    replaced = true;
                    break;
                }
            }
        }
        if !replaced {
            doc.body.push(bib_env);
        }
    }

    (true, ay_map)
}

/// Recursively collect citation keys from AST nodes
fn collect_citation_keys_recursive(nodes: &[scriptex::document::Node], keys: &mut Vec<String>) {
    use scriptex::document::Node;
    for node in nodes {
        match node {
            Node::Citation(key, _, _) => {
                for k in key.split(',') {
                    let k = k.trim();
                    if !k.is_empty() && !keys.contains(&k.to_string()) {
                        keys.push(k.to_string());
                    }
                }
            }
            Node::Paragraph(c) | Node::Bold(c) | Node::Italic(c)
            | Node::Monospace(c) | Node::SansSerif(c) | Node::SmallCaps(c) | Node::Underline(c)
            | Node::Strikethrough(c) | Node::Superscript(c) | Node::Subscript(c)
            | Node::Emph(c) | Node::Quote(c) | Node::Quotation(c)
            | Node::Abstract(c) | Node::Center(c) | Node::FlushLeft(c)
            | Node::FlushRight(c) | Node::Group(c) | Node::MBox(c) | Node::Footnote(c)
            | Node::Proof { content: c, .. } | Node::TwoColumn(c) => {
                collect_citation_keys_recursive(c, keys);
            }
            Node::Section { title, .. } => {
                collect_citation_keys_recursive(title, keys);
            }
            Node::Colored { content, .. } | Node::FontSize { content, .. }
            | Node::Minipage { content, .. } => {
                collect_citation_keys_recursive(content, keys);
            }
            Node::Environment(env) => {
                collect_citation_keys_recursive(&env.content, keys);
            }
            Node::Figure(fig) => {
                collect_citation_keys_recursive(&fig.content, keys);
                if let Some(cap) = &fig.caption { collect_citation_keys_recursive(cap, keys); }
            }
            Node::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells { collect_citation_keys_recursive(&cell.content, keys); }
                }
                if let Some(cap) = &table.caption { collect_citation_keys_recursive(cap, keys); }
            }
            Node::Theorem(thm) => { collect_citation_keys_recursive(&thm.body, keys); }
            Node::ItemizeList(items) | Node::EnumerateList(items) | Node::DescriptionList(items) => {
                for item in items {
                    collect_citation_keys_recursive(&item.content, keys);
                    if let Some(label) = &item.label { collect_citation_keys_recursive(label, keys); }
                }
            }
            Node::Href { content, .. } => { collect_citation_keys_recursive(content, keys); }
            _ => {}
        }
    }
}

/// Recursively resolve \input{file} and \include{file} commands by inlining file contents.
fn resolve_inputs(source: &str, base_dir: &std::path::Path, depth: u32) -> String {
    if depth > 10 { return source.to_string(); } // prevent infinite recursion

    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for \input or \include
        if bytes[i] == b'\\' && i + 6 < bytes.len() {
            let rest = &source[i..];
            let (cmd, cmd_len) = if rest.starts_with("\\input") && !rest[6..7.min(rest.len())].starts_with(|c: char| c.is_ascii_alphabetic()) {
                ("\\input", 6)
            } else if rest.starts_with("\\include{") {
                ("\\include", 8)
            } else {
                result.push(source.as_bytes()[i] as char);
                i += 1;
                continue;
            };

            // Skip whitespace after command name
            let mut j = cmd_len;
            while j < rest.len() && rest.as_bytes()[j] == b' ' { j += 1; }

            if j < rest.len() && rest.as_bytes()[j] == b'{' {
                // Read braced filename
                j += 1;
                let start = j;
                while j < rest.len() && rest.as_bytes()[j] != b'}' { j += 1; }
                if j < rest.len() {
                    let filename = rest[start..j].trim();
                    j += 1; // skip '}'

                    // Resolve file path
                    let mut path = base_dir.join(filename);
                    if !path.exists() && path.extension().is_none() {
                        path.set_extension("tex");
                    }

                    if path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            // For \include, add \clearpage before and after
                            if cmd == "\\include" {
                                result.push_str("\\clearpage\n");
                            }
                            // Recursively resolve nested inputs
                            let file_dir = path.parent().unwrap_or(base_dir);
                            let resolved = resolve_inputs(&content, file_dir, depth + 1);
                            result.push_str(&resolved);
                            if cmd == "\\include" {
                                result.push_str("\n\\clearpage\n");
                            }
                        }
                    }
                    i += j;
                    continue;
                }
            }
        }

        result.push(bytes[i] as char);
        i += 1;
    }

    result
}
