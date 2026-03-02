use anyhow::Result;
use clap::Parser as ClapParser;
use std::path::PathBuf;
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod lexer;
mod parser;
mod document;
mod layout;
mod typeset;
mod pdf;
mod color;
mod error;
mod font;
mod highlight;
mod math_layout;
mod macro_expand;
mod hyphenate;
#[allow(dead_code)]
mod tikz;
mod tikz_render;
mod pgfplots;
#[allow(dead_code)]
mod bibliography;
#[allow(dead_code)]
mod xref;
#[allow(dead_code)]
mod image_embed;
#[allow(dead_code)]
mod font_embed;

use crate::parser::Parser;

#[derive(ClapParser, Debug)]
#[command(name = "soniclatex", about = "Blazing fast LaTeX to PDF compiler")]
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
    let effective_source: &str = if macro_expand::MacroEngine::has_macros(source_after_includes) {
        expanded = macro_expand::expand(source_after_includes);
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
    let doc = parser.parse()?;
    let parse_time = parse_start.elapsed();
    eprintln!("[PARSE]   {:.3}ms", parse_time.as_secs_f64() * 1000.0);

    // Layout
    let layout_start = Instant::now();
    let layout_result = layout::layout_document(&doc, effective_source)?;
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
