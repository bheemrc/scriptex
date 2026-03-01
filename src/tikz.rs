/// TikZ rendering module: shell-out to pdflatex for TikZ pictures
/// Results are cached by content hash to avoid re-rendering
///
/// Approach:
/// 1. Parser captures raw tikzpicture source text
/// 2. At layout time, check cache (~/.cache/sonicspeedlatex/{sha256}.pdf)
/// 3. If not cached: write minimal .tex wrapper -> shell-out pdflatex -> get PDF
/// 4. Parse single-page PDF -> extract content -> embed as Form XObject
/// 5. Fallback: render placeholder box if pdflatex unavailable

use sha2::{Sha256, Digest};
use std::path::PathBuf;

/// Result of rendering a TikZ picture
pub struct TikzResult {
    /// Width in points
    pub width: f32,
    /// Height in points
    pub height: f32,
    /// PDF content of the rendered diagram (raw bytes from the cropped PDF)
    pub pdf_bytes: Option<Vec<u8>>,
    /// Whether rendering succeeded
    pub success: bool,
}

/// Cache directory for rendered TikZ pictures
fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cache").join("sonicspeedlatex")
}

/// Compute SHA-256 hash of TikZ source for caching
fn content_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Check if a cached PDF exists for this TikZ source
fn get_cached(source: &str) -> Option<Vec<u8>> {
    let hash = content_hash(source);
    let cache_path = cache_dir().join(format!("{}.pdf", hash));
    std::fs::read(&cache_path).ok()
}

/// Save rendered PDF to cache
fn save_to_cache(source: &str, pdf_bytes: &[u8]) {
    let hash = content_hash(source);
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let cache_path = dir.join(format!("{}.pdf", hash));
    let _ = std::fs::write(&cache_path, pdf_bytes);
}

/// Check if pdflatex is available on the system
pub fn has_pdflatex() -> bool {
    std::process::Command::new("pdflatex")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Render a TikZ picture using pdflatex
///
/// Returns TikzResult with the rendered PDF bytes or a placeholder
pub fn render_tikz(tikz_source: &str, preamble_packages: &[String]) -> TikzResult {
    // Check cache first
    if let Some(cached) = get_cached(tikz_source) {
        // Try to extract dimensions from cached PDF
        let (w, h) = extract_pdf_dimensions(&cached).unwrap_or((300.0, 200.0));
        return TikzResult {
            width: w,
            height: h,
            pdf_bytes: Some(cached),
            success: true,
        };
    }

    // Try pdflatex
    if !has_pdflatex() {
        return TikzResult {
            width: 300.0,
            height: 80.0,
            pdf_bytes: None,
            success: false,
        };
    }

    // Create temporary directory
    let tmp_dir = std::env::temp_dir().join(format!("soniclatex_tikz_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Write .tex wrapper
    let tex_path = tmp_dir.join("tikz.tex");
    let mut tex_content = String::with_capacity(tikz_source.len() + 512);
    tex_content.push_str("\\documentclass[border=2pt]{standalone}\n");
    tex_content.push_str("\\usepackage{tikz}\n");
    tex_content.push_str("\\usepackage{pgfplots}\n");
    tex_content.push_str("\\pgfplotsset{compat=1.18}\n");

    // Add any relevant packages from the original document
    for pkg in preamble_packages {
        if pkg.starts_with("tikz") || pkg.starts_with("pgf") || pkg == "amsmath" || pkg == "amssymb" {
            tex_content.push_str(&format!("\\usepackage{{{}}}\n", pkg));
        }
    }

    tex_content.push_str("\\begin{document}\n");
    tex_content.push_str("\\begin{tikzpicture}\n");
    tex_content.push_str(tikz_source);
    tex_content.push_str("\n\\end{tikzpicture}\n");
    tex_content.push_str("\\end{document}\n");

    if std::fs::write(&tex_path, &tex_content).is_err() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return TikzResult { width: 300.0, height: 80.0, pdf_bytes: None, success: false };
    }

    // Run pdflatex
    let output = std::process::Command::new("pdflatex")
        .arg("-interaction=nonstopmode")
        .arg("-halt-on-error")
        .arg("-output-directory")
        .arg(tmp_dir.to_str().unwrap_or("/tmp"))
        .arg(tex_path.to_str().unwrap_or("tikz.tex"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();

    let result = match output {
        Ok(out) if out.status.success() => {
            let pdf_path = tmp_dir.join("tikz.pdf");
            if let Ok(pdf_bytes) = std::fs::read(&pdf_path) {
                let (w, h) = extract_pdf_dimensions(&pdf_bytes).unwrap_or((300.0, 200.0));
                save_to_cache(tikz_source, &pdf_bytes);
                TikzResult {
                    width: w,
                    height: h,
                    pdf_bytes: Some(pdf_bytes),
                    success: true,
                }
            } else {
                TikzResult { width: 300.0, height: 80.0, pdf_bytes: None, success: false }
            }
        }
        _ => TikzResult { width: 300.0, height: 80.0, pdf_bytes: None, success: false },
    };

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
    result
}

/// Extract page dimensions from a PDF file (MediaBox)
fn extract_pdf_dimensions(pdf_bytes: &[u8]) -> Option<(f32, f32)> {
    // Quick and dirty: find /MediaBox in the PDF
    let content = std::str::from_utf8(pdf_bytes).ok()?;

    // Look for /MediaBox [x1 y1 x2 y2]
    let idx = content.find("/MediaBox")?;
    let rest = &content[idx..];
    let bracket_start = rest.find('[')?;
    let bracket_end = rest.find(']')?;
    let coords_str = &rest[bracket_start + 1..bracket_end];
    let nums: Vec<f32> = coords_str.split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    if nums.len() >= 4 {
        let width = nums[2] - nums[0];
        let height = nums[3] - nums[1];
        Some((width.abs(), height.abs()))
    } else {
        None
    }
}

/// Generate a Form XObject reference for embedding a TikZ PDF
/// Returns (xobject_dict_bytes, content_stream_bytes)
pub fn tikz_to_xobject(pdf_bytes: &[u8], width: f32, height: f32) -> Option<(Vec<u8>, Vec<u8>)> {
    // For now, we embed the first page's content stream directly
    // A full implementation would parse the PDF structure and extract the page content
    // For simplicity, we create a simple Form XObject

    let content = std::str::from_utf8(pdf_bytes).ok()?;

    // Find stream content
    let stream_start = content.find("stream\n")? + 7;
    let stream_end = content[stream_start..].find("\nendstream")? + stream_start;
    let stream_data = &pdf_bytes[stream_start..stream_end];

    // Create XObject dictionary
    let dict = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Length {} >>",
        width as u32, height as u32, stream_data.len()
    );

    Some((dict.into_bytes(), stream_data.to_vec()))
}
