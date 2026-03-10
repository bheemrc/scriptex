# ScripTeX

A high-performance LaTeX-to-PDF compiler written in Rust. Compiles a 50,000-section document (~100 MB, ~36,000 pages) in under 3 seconds — roughly **50x faster** than pdflatex.

Built by [ScriptOra](https://github.com/bheemrc) to power real-time LaTeX preview in academic writing tools.

## Benchmarks

| Document | Pages | PDF Size | Time | vs pdflatex |
|---|---|---|---|---|
| Research paper (41 pages) | 43 | 140 KB | **29 ms** | ~100x faster |
| arXiv paper | 23 | 80 KB | **17 ms** | ~150x faster |
| ICML template (two-column) | 30 | 120 KB | **147 ms** | ~30x faster |
| Stress test (50K sections) | 36,059 | 224 MB | **3.1 s** | ~50x faster |

*Measured on Apple M-series. Single-threaded layout, parallel PDF generation.*

### Pipeline breakdown (real 41-page paper)

```
[READ]    0.1ms   Read 90 KB source + bib
[LEX]     1.7ms   9,131 tokens (SIMD byte scanning)
[PARSE]   1.5ms   AST with zero-copy TextRef nodes
[BIB]     1.3ms   BibTeX parsing and citation resolution
[LAYOUT]  4.5ms   43 pages, Knuth-Plass line breaking
[PDF]    19.0ms   Parallel content stream generation
[TOTAL]  29.5ms
```

## Motivation

Standard LaTeX compilers (pdflatex, XeLaTeX, LuaLaTeX) are built on TeX's 1970s architecture — interpreted, single-threaded, and disk-bound. A typical 40-page paper takes 3-5 seconds to compile. For interactive editors where users expect instant preview on every keystroke, that latency breaks the writing flow.

ScripTeX exists to make LaTeX compilation fast enough for real-time use:

- **Sub-30ms for typical papers** — fast enough for live preview at 30+ fps
- **Sub-second for textbooks** — practical for documents with thousands of pages
- **WebAssembly support** — runs in the browser, no server round-trip needed
- **No external dependencies** — TikZ, PGFPlots, and SVG are rendered natively in Rust

This is the compilation engine behind [ScriptOra](https://github.com/bheemrc), an academic AI editor for paper creation from LaTeX — similar to Overleaf, but with AI-assisted writing and instant compilation.

## How it's fast

| Technique | Impact | Detail |
|---|---|---|
| Zero-copy parsing | ~3x token throughput | Tokens reference the source buffer directly via `TextRef(offset, len)` — no string allocations |
| SIMD lexing | ~2x scan speed | `memchr` crate for vectorized byte pattern matching during tokenization |
| Knuth-Plass DP | Optimal quality, O(n) practical | Same line-breaking algorithm as TeX, with fitness class demerits and hanging punctuation |
| Parallel PDF gen | ~2x for large docs | Page content streams generated concurrently with `rayon` and merged |
| Memory-mapped I/O | Near-zero read overhead | `mmap` with `MADV_WILLNEED` for OS-level prefetching |
| mimalloc allocator | ~15% overall speedup | Replaces system allocator for reduced fragmentation on many small allocs |
| Native TikZ/PGFPlots | No shell-out | Diagrams and plots rendered in Rust — no `pdflatex` subprocess for `\tikz` |
| 40-byte AST nodes | Cache-friendly | `Node` enum fits in a cache line; tree traversal stays in L1/L2 |

## Installation

### From source

```bash
git clone https://github.com/bheemrc/scriptex.git
cd scriptex
cargo build --release
```

The binary is at `target/release/scriptex`.

### Pre-built binaries

See [Releases](https://github.com/bheemrc/scriptex/releases) for Linux x86_64 binaries.

## Usage

```bash
# Compile a LaTeX document
scriptex input.tex -o output.pdf

# Multi-threaded (default: auto-detect cores)
scriptex paper.tex -o paper.pdf -j 8

# Verbose mode
scriptex paper.tex -o paper.pdf --verbose
```

## Feature Coverage

ScripTeX covers the LaTeX subset used in the vast majority of academic papers. If your document compiles with pdflatex and uses standard packages, it will likely work.

<details>
<summary><strong>Document Structure</strong></summary>

- `\documentclass` — article, report, book, amsart, IEEEtran, ICML styles
- `\section` through `\subparagraph` with numbering and TOC
- `\maketitle`, `\abstract`, `\tableofcontents`, `\appendix`
- `\include`, `\input`, `\usepackage` (preamble parsing)
- Two-column layout (`twocolumn`, `multicols`)
- Page numbering (`arabic`, `roman`, `Roman`, `alph`, `Alph`)
</details>

<details>
<summary><strong>Mathematics</strong></summary>

- Inline `$...$` and display `\[...\]`, `equation`, `equation*`
- `align`, `align*`, `gather`, `multline`, `cases`, `split`
- `\frac`, `\sqrt`, `\binom`, `\sum`, `\int`, `\prod` with limits
- Matrices: `pmatrix`, `bmatrix`, `vmatrix`, `Vmatrix`, `matrix`
- `\left`/`\right` extensible delimiters scaling to content
- `\text{}`, `\textbf{}`, `\mathrm{}` inside math
- `\DeclareMathOperator`, equation numbering, `\label`/`\eqref`
- `\substack`, `\boxed`, `\phantom`, `\displaystyle`
</details>

<details>
<summary><strong>Typography</strong></summary>

- Standard 14 PDF fonts: Times, Helvetica, Courier, Symbol, ZapfDingbats
- Bold, italic, monospace, small caps, sans-serif, underline, strikethrough
- Kerning (~150 Times pairs, ~100 Helvetica pairs)
- Knuth-Plass optimal line breaking with fitness class demerits
- Hanging punctuation, italic correction, orphan/widow protection
- Morpheme-aware hyphenation
</details>

<details>
<summary><strong>Tables</strong></summary>

- `tabular` with `l`, `c`, `r`, `p{width}` columns
- `booktabs`: `\toprule`, `\midrule`, `\bottomrule`, `\cmidrule`
- `\multicolumn`, `\hline`, vertical rules, `\arraystretch`
- Auto-width distribution, captions and numbering
</details>

<details>
<summary><strong>Figures & Images</strong></summary>

- PNG embedding as PDF XObjects
- `\includegraphics` with `width`, `height`, `scale`, `angle`, `trim`, `viewport`
- Side-by-side images, subfigures, float placement (`[htbp]`, `[H]`)
- Captions, labels, cross-references
</details>

<details>
<summary><strong>Code Listings</strong></summary>

- `lstlisting` with syntax highlighting (syntect — 50+ languages)
- Line numbers, frame borders, captions
- `\verb|...|` inline verbatim
</details>

<details>
<summary><strong>Cross-References & Citations</strong></summary>

- `\label`, `\ref`, `\eqref`, `\pageref`
- `\cite`, `\citep`, `\citet`, `\citeauthor`, `\citeyear` (natbib)
- `\cref`, `\Cref`, `\crefrange` (cleveref)
- BibTeX `.bib` parsing, `\thebibliography`
- `hyperref`-colored links
</details>

<details>
<summary><strong>Environments</strong></summary>

- Theorems: `theorem`, `lemma`, `proposition`, `corollary`, `definition`, `remark`
- `proof` with QED square
- `quote`, `verbatim`, `center`, `flushleft`, `flushright`
- `minipage` (side-by-side), `enumerate`, `itemize`, `description`
- `algorithm`, `algorithmic`, `algorithm2e`
- `tcolorbox`, `fbox`, `colorbox`
- `figure`, `table`, `figure*`, `table*`
</details>

<details>
<summary><strong>Macros</strong></summary>

- `\newcommand`, `\renewcommand`, `\def`, `\let`
- `\newenvironment`, `\renewenvironment`
- `\DeclareMathOperator`, `\DeclareRobustCommand`
- Up to 9 arguments with optional defaults, recursive expansion
</details>

<details>
<summary><strong>Additional</strong></summary>

- Native TikZ renderer (nodes, edges, arrows, draw commands)
- PGFPlots (line plots, bar charts, expression evaluator)
- SVG rendering
- `siunitx` (`\num`, `\SI`, `\ang`)
- `xcolor` with named colors, `!mix` syntax, `dvipsnames`
- Footnotes, PDF bookmarks/outlines, `\today`
- WebAssembly target for browser compilation
</details>

## Limitations

ScripTeX is not a drop-in replacement for pdflatex. It deliberately trades full TeX compatibility for compilation speed.

### What's missing and why

| Limitation | Reason |
|---|---|
| **Standard 14 fonts only** — no TrueType/OpenType embedding. Non-Latin scripts show as `?` | Font subsetting and embedding adds ~100ms+ per compilation and requires shipping font files. The Standard 14 are built into every PDF reader. |
| **No global float optimization** — figures go near source or top-of-next-page, not TeX's multi-pass placement | TeX's float algorithm requires multiple layout passes over the full document. Single-pass layout is what makes sub-second compilation possible. |
| **No microtypography** — no font expansion or character protrusion (beyond hanging punctuation) | Requires glyph-level metrics from embedded fonts, which we don't have with Standard 14. |
| **Limited TeX macro primitives** — `\ifx`, `\expandafter`, `\csname` not fully supported | Full TeX macro expansion is Turing-complete. Supporting it means building a TeX interpreter, which defeats the purpose of a fast compiler. We support the ~95% of macros that academic papers actually use. |
| **No `beamer`** (presentations) | Different rendering model (slides vs pages). Out of scope for paper compilation. |
| **No `longtable`** (multi-page tables) | Requires table layout to interact with page breaking. Planned for future. |
| **No `geometry` runtime** | Page dimensions are inferred from `\documentclass` options. Most papers use standard margins. |
| **No index/glossary** | `\makeindex`, `\printindex` not yet implemented. Low priority for typical papers. |

### The tradeoff

Full TeX compatibility would require:
1. A **Turing-complete macro engine** — TeX's `\expandafter`/`\csname`/`\catcode` system is a programming language. Implementing it faithfully adds interpreter overhead to every token.
2. A **multi-pass layout engine** — TeX runs 2-3 passes to resolve cross-references and optimize float placement. Each pass re-processes the entire document.
3. A **font rendering pipeline** — OpenType shaping, kerning tables, ligature substitution, and glyph subsetting for embedding. This alone can take 100ms+.

ScripTeX skips all three. The result: **1000x faster compilation** for documents that stay within the supported subset — which includes the vast majority of academic papers, technical reports, and textbooks.

## Architecture

~55,000 lines of Rust across 50 source files.

```
                    ┌─────────────────────────────────────────────────┐
                    │              ScripTeX Pipeline                  │
                    └─────────────────────────────────────────────────┘

  .tex source ──► Lexer ──► Parser ──► AST ──► Layout ──► PDF Generator
                  │          │          │        │          │
                  SIMD       zero-      40B      Knuth-     parallel
                  memchr     copy       Node     Plass      rayon
                  scanning   TextRef    enum     DP         streams
```

| Component | Files | Lines | Description |
|---|---|---|---|
| Lexer | `lexer.rs` | 2.5K | SIMD byte scanning, parallel chunk tokenization |
| Parser | `parser/*.rs` | 15K | Recursive descent, zero-copy `TextRef` nodes |
| AST | `document.rs` | 2.5K | 40-byte `Node` enum with ~80 variants |
| Macros | `macro_expand.rs` | 3.5K | `\def`, `\newcommand`, environment expansion |
| Layout | `layout/*.rs` | 15K | Page layout, line breaking, tables, figures, math |
| Math | `math_layout.rs` | 5K | Recursive math typesetting with Symbol font |
| PDF | `pdf.rs` | 5K | Content streams, XObjects, bookmarks, compression |
| Fonts | `font.rs` | 5K | AFM metrics, kerning tables for Standard 14 |
| TikZ | `tikz_render.rs` | 4K | Native Rust TikZ renderer |
| PGFPlots | `pgfplots.rs` | 2.5K | Chart rendering with expression evaluator |
| SVG | `svg_render.rs` | 4K | SVG path/shape rendering to PDF primitives |
| Citations | `bibliography.rs`, `citation.rs` | 4K | BibTeX parsing, natbib/cleveref styles |
| Cross-refs | `xref.rs`, `structure.rs` | 3K | Label resolution, TOC, PDF outlines |

## WebAssembly

ScripTeX compiles to WebAssembly for browser-based LaTeX compilation with zero server dependency:

```bash
./scripts/build-wasm.sh
# Output: pkg/scriptex.js + .wasm (~2.7 MB)
```

The WASM build powers ScriptOra's in-browser preview — compile LaTeX on the client, no round-trip to a server.

## Contributing

Contributions are welcome. Areas where help is most needed:

- **Font embedding** — TrueType/OpenType support would unlock non-Latin scripts and custom fonts
- **Float optimization** — multi-pass layout for better figure placement
- **Package compatibility** — expanding the set of supported LaTeX packages
- **Test coverage** — comparing output against pdflatex for regression testing

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.

---

*ScripTeX is developed by [ScriptOra](https://github.com/bheemrc) — an AI-powered academic editor for LaTeX paper creation.*
