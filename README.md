# SonicSpeedLaTeX

A Rust-based LaTeX-to-PDF compiler built for speed. Compiles a 50,000-section document (~100MB, ~36,000 pages) in under 3 seconds — roughly **50x faster** than pdflatex.

## Benchmarks

| Document | Pages | PDF Size | Time | vs pdflatex |
|---|---|---|---|---|
| Research paper (41 pages) | 43 | 140 KB | **29 ms** | ~100x faster |
| arXiv paper | 23 | 80 KB | **17 ms** | ~150x faster |
| ICML template (two-column) | 30 | 120 KB | **147 ms** | ~30x faster |
| Stress test (50K sections) | 36,059 | 224 MB | **3.1 s** | ~50x faster |

*Measured on Apple M-series. Single-threaded layout, parallel PDF generation.*

## Why?

Standard LaTeX compilers (pdflatex, XeLaTeX, LuaLaTeX) are built on TeX's 1970s architecture — interpreted, single-threaded, and disk-bound. For large documents or rapid iteration workflows, compilation time becomes a bottleneck.

SonicSpeedLaTeX takes a different approach:

- **Zero-copy parsing** — tokens reference the source buffer directly; no intermediate string allocations
- **SIMD-accelerated lexing** — `memchr` for fast byte scanning during tokenization
- **Knuth-Plass line breaking** — optimal paragraph layout via dynamic programming, same algorithm as TeX
- **Parallel PDF generation** — page content streams generated concurrently with `rayon`
- **Memory-mapped I/O** — `mmap` for reading source files with OS-level caching
- **Custom allocator** — `mimalloc` for reduced allocation overhead
- **Native renderers** — TikZ, PGFPlots, and SVG rendered in Rust (no shell-out to external tools)

## Installation

### From source

```bash
git clone https://github.com/bheemrc/sonicspeedlatex.git
cd sonicspeedlatex
cargo build --release
```

The binary is at `target/release/soniclatex`.

### Pre-built binaries

See [Releases](https://github.com/bheemrc/sonicspeedlatex/releases) for Linux x86_64 binaries.

## Usage

```bash
# Basic compilation
soniclatex input.tex -o output.pdf

# With timing output (default)
soniclatex paper.tex -o paper.pdf
# [READ]    0.1ms
# [LEX]     1.2ms
# [PARSE]   0.8ms
# [LAYOUT]  4.5ms - 43 pages
# [PDF]     19.0ms
# [TOTAL]   29.5ms
```

## Supported Features

### Document Structure
- `\documentclass` (article, report, book, amsart, IEEEtran, ICML styles)
- `\section` through `\subparagraph` with numbering and TOC generation
- `\maketitle`, `\abstract`, `\tableofcontents`, `\appendix`
- `\include`, `\input`, `\usepackage` (preamble parsing)
- Two-column layout (`twocolumn`, `multicols`)
- Page numbering styles (`arabic`, `roman`, `Roman`, `alph`, `Alph`)

### Mathematics
- Inline (`$...$`) and display (`\[...\]`, `equation`, `equation*`) math
- `align`, `align*`, `gather`, `multline`, `cases`, `split`
- `\frac`, `\sqrt`, `\binom`, `\sum`, `\int`, `\prod` with limits
- Matrices: `pmatrix`, `bmatrix`, `vmatrix`, `Vmatrix`, `matrix`
- `\left`/`\right` extensible delimiters that scale to content height
- Subscripts, superscripts, `\overset`, `\underset`, `\stackrel`
- `\text{}`, `\textbf{}`, `\mathrm{}` inside math
- `\DeclareMathOperator`, `\newcommand` in math contexts
- Equation numbering with `\label`/`\eqref`/`\ref` cross-references
- `\substack`, `\boxed`, `\phantom`, `\displaystyle`

### Typography
- **Standard 14 PDF fonts**: Times, Helvetica, Courier, Symbol, ZapfDingbats
- Bold, italic, monospace, small caps, sans-serif, underline, strikethrough
- Kerning tables (~150 Times-Roman pairs, ~100 Helvetica pairs)
- Hanging punctuation (trailing periods/commas protrude into margin)
- Knuth-Plass optimal line breaking with fitness class demerits
- Hyphenation with morpheme-aware rules
- Italic correction at italic-to-upright transitions
- Orphan/widow protection

### Tables
- `tabular` with `l`, `c`, `r`, `p{width}` column specs
- `booktabs` rules: `\toprule`, `\midrule`, `\bottomrule`, `\cmidrule`
- `\multicolumn`, `\hline`, vertical rules
- `\arraystretch` for row height scaling
- Auto-width column distribution
- Table captions and numbering

### Figures & Images
- PNG image embedding (decoded and stored as PDF XObjects)
- `\includegraphics` with `width`, `height`, `scale`, `angle`, `trim`, `viewport`
- Side-by-side images with `\hfill` spacing
- Figure captions, labels, and cross-references
- Float placement hints (`[htbp]`, `[H]`)
- `\subfigure` side-by-side layout

### Code Listings
- `lstlisting` with syntax highlighting via `syntect`
- Language support: Rust, Python, C, C++, Java, JavaScript, Go, and more
- Line numbers, frame borders, captions
- `\verb|...|` inline verbatim

### Cross-References & Citations
- `\label`, `\ref`, `\eqref`, `\pageref`
- `\cite`, `\citep`, `\citet`, `\citeauthor`, `\citeyear` (natbib)
- `\cref`, `\Cref`, `\crefrange` (cleveref)
- BibTeX `.bib` file parsing
- `\thebibliography` with hanging indent
- `hyperref`-style colored links (maroon for refs, dark green for citations)

### Environments
- `theorem`, `lemma`, `proposition`, `corollary`, `definition`, `remark`, `example`
- `proof` with QED square and optional argument
- `quote`, `quotation`, `verbatim`, `center`, `flushleft`, `flushright`
- `minipage` with side-by-side layout
- `enumerate`, `itemize`, `description` (nested, custom labels)
- `algorithm`, `algorithmic`, `algorithm2e`
- `tcolorbox`, `fbox`, `colorbox`
- `figure`, `table`, `figure*`, `table*` (spanning in two-column)
- `abstract`, `titlepage`, `appendix`

### Macro System
- `\newcommand`, `\renewcommand`, `\def`, `\let`
- `\newenvironment`, `\renewenvironment`
- `\DeclareMathOperator`, `\DeclareRobustCommand`
- Up to 9 arguments with optional defaults
- Recursive expansion

### Additional Features
- Native TikZ renderer (nodes, edges, arrows, draw commands)
- PGFPlots renderer (line plots, bar charts, expression evaluation)
- SVG rendering
- `siunitx` (`\num`, `\SI`, `\ang`)
- `xcolor` with named colors, `!mix` syntax, `dvipsnames`
- `\textcolor`, `\colorbox`, `\definecolor`
- Footnotes with separator rule and hanging indent
- PDF bookmarks/outlines (hierarchical, collapsed)
- `\today` date expansion
- `\LaTeX` and `\TeX` logos with proper kerning
- WebAssembly target for browser-based compilation

## Limitations

This is not a drop-in replacement for pdflatex. Key limitations:

### Fonts
- **Standard 14 PDF fonts only** — no TrueType/OpenType font embedding. All text renders in Times Roman, Helvetica, or Courier. Characters outside WinAnsi encoding (most non-Latin scripts, many Unicode symbols) display as `?`.
- No font loading (`\usepackage{fontspec}` is ignored)

### Layout
- **No page-level float optimization** — figures are placed near their source position or deferred to the next page top, but TeX's global float placement algorithm is not implemented
- **No microtypography** — no font expansion, no character protrusion (beyond hanging punctuation)
- **No paragraph shaping** — no `\parshape`, `\hangindent`, or shaped text wrapping around figures (`wrapfig` is parsed but not rendered)

### Math
- **Symbol font limitations** — uses PDF Symbol font which lacks some mathematical symbols. Symbols not in the font render as approximations or blanks
- **No AMSmath full compatibility** — `\DeclareMathOperator` works but many advanced constructs (`\xrightarrow`, `\overset` with complex arguments) have limited support

### Missing Features
- No index generation (`\makeindex`, `\printindex`)
- No glossary support
- No `beamer` (presentations)
- No `tikz-cd` (commutative diagrams), `forest` (tree diagrams)
- No `longtable` (tables spanning multiple pages)
- No conditional compilation (`\ifthenelse`, `\ifx` beyond basic `\iffalse`)
- No `geometry` package runtime support (page dimensions are parsed from `\documentclass` options)
- No `biblatex` backend — only `bibtex`-style `.bib` files and `\thebibliography`

### Why These Limitations Exist

Full TeX compatibility requires implementing a Turing-complete macro expansion engine, a global float placement solver, and a font rendering pipeline — each of which adds significant complexity and runtime overhead. SonicSpeedLaTeX deliberately trades completeness for speed: it covers the subset of LaTeX used in ~90% of academic papers and technical documents while keeping compilation under a second.

If your document compiles with pdflatex and uses standard packages (amsmath, graphicx, hyperref, booktabs, listings, natbib, cleveref), it will likely render correctly. Documents relying on exotic packages or TeX primitives will need pdflatex.

## Architecture

~55,000 lines of Rust across 50 source files.

```
Source (.tex) ──► Lexer ──► Parser ──► AST ──► Layout Engine ──► PDF Generator
                  (SIMD)   (zero-copy)  (40B    (Knuth-Plass)    (parallel)
                                        nodes)
```

| Component | File(s) | Description |
|---|---|---|
| Lexer | `lexer.rs` | SIMD byte scanning, parallel chunk tokenization |
| Parser | `parser/*.rs` (9 files) | Recursive descent, zero-copy TextRef nodes |
| AST | `document.rs` | 40-byte Node enum, ~80 variants |
| Macros | `macro_expand.rs` | `\def`, `\newcommand`, environment expansion |
| Layout | `layout/*.rs` (13 files) | Page layout, Knuth-Plass line breaking, tables, figures |
| Math | `math_layout.rs` | Recursive math typesetting with Symbol font |
| PDF | `pdf.rs` | Parallel content streams, image XObjects, bookmarks |
| Fonts | `font.rs` | AFM metrics, kerning tables for Standard 14 fonts |
| TikZ | `tikz_render.rs` | Native Rust TikZ renderer (no external process) |
| PGFPlots | `pgfplots.rs` | Chart rendering with expression evaluator |
| SVG | `svg_render.rs` | SVG path rendering to PDF |
| Bibliography | `bibliography.rs`, `citation.rs` | BibTeX parsing, natbib citation styles |
| Cross-refs | `xref.rs`, `structure.rs` | Label resolution, TOC generation, PDF outlines |
| Highlighting | `highlight.rs` | `syntect`-based code coloring |

## WebAssembly

SonicSpeedLaTeX compiles to WebAssembly for browser-based LaTeX compilation:

```bash
./scripts/build-wasm.sh
# Output in pkg/
```

See `pkg/test.html` for a working browser demo.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
