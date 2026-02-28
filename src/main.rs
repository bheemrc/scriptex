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

    // Lex (parallel for large files)
    let lex_start = Instant::now();
    let tokens = lexer::tokenize_parallel(&source);
    let lex_time = lex_start.elapsed();
    eprintln!("[LEX]     {:.3}ms - {} tokens", lex_time.as_secs_f64() * 1000.0, tokens.len());

    // Parse
    let parse_start = Instant::now();
    let mut parser = Parser::new(tokens, &source);
    let doc = parser.parse()?;
    let parse_time = parse_start.elapsed();
    eprintln!("[PARSE]   {:.3}ms", parse_time.as_secs_f64() * 1000.0);

    // Layout
    let layout_start = Instant::now();
    let layout_result = layout::layout_document(&doc, &source)?;
    let layout_time = layout_start.elapsed();
    eprintln!("[LAYOUT]  {:.3}ms - {} pages", layout_time.as_secs_f64() * 1000.0, layout_result.num_pages());

    // Generate PDF
    let pdf_start = Instant::now();
    let output_path = args.output.unwrap_or_else(|| {
        let mut p = args.input.clone();
        p.set_extension("pdf");
        p
    });
    let bytes_written = pdf::generate_pdf(&layout_result, &doc, &output_path, &source)?;
    let pdf_time = pdf_start.elapsed();
    eprintln!("[PDF]     {:.3}ms - wrote {} bytes", pdf_time.as_secs_f64() * 1000.0, bytes_written);

    let total_time = total_start.elapsed();
    eprintln!("[TOTAL]   {:.3}ms", total_time.as_secs_f64() * 1000.0);
    eprintln!("Output: {}", output_path.display());

    Ok(())
}
