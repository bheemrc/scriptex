//! WASM API boundary — wasm-bindgen entry points for browser compilation.
//!
//! Provides two compilation modes:
//! - `compile_latex(source)` — single-file compilation
//! - `compile_latex_project(source, files_json)` — multi-file project compilation
//!
//! The multi-file mode accepts a JSON object mapping filenames to base64-encoded
//! content (for binary files) or plain string content (for text files).

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "wasm")]
#[wasm_bindgen(start)]
pub fn wasm_init() {
    console_error_panic_hook::set_once();
}

/// Compile a single LaTeX source file to PDF bytes.
/// Returns a Uint8Array containing the raw PDF.
#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn compile_latex(source: &str) -> Result<js_sys::Uint8Array, JsValue> {
    let pdf_bytes = crate::compile_latex_core(source)
        .map_err(|e| JsValue::from_str(&format!("Compilation error: {}", e)))?;

    Ok(js_sys::Uint8Array::from(pdf_bytes.as_slice()))
}

/// Compile a multi-file LaTeX project to PDF bytes.
///
/// `source` — the main .tex file content
/// `files_json` — a JSON string mapping filenames to their content:
///   {
///     "chapter1.tex": "\\section{Hello}...",
///     "refs.bib": "@article{key,...}",
///     "figure.png": "data:base64,iVBOR...",  // base64 with data: prefix for binary
///     "custom.sty": "\\newcommand{...}"
///   }
///
/// Text files (.tex, .sty, .cls, .bib) are stored as-is.
/// Binary files (.png, .jpg, .jpeg, .pdf) should be base64-encoded with "data:base64," prefix.
#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn compile_latex_project(source: &str, files_json: &str) -> Result<js_sys::Uint8Array, JsValue> {
    let mut project = crate::ProjectFiles::new();

    // Parse the JSON file map
    if !files_json.is_empty() {
        parse_project_files(files_json, &mut project)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse project files: {}", e)))?;
    }

    let pdf_bytes = crate::compile_latex_project(source, &project)
        .map_err(|e| JsValue::from_str(&format!("Compilation error: {}", e)))?;

    Ok(js_sys::Uint8Array::from(pdf_bytes.as_slice()))
}

/// Parse a simple JSON object of filename→content into ProjectFiles.
/// We do minimal JSON parsing to avoid pulling in serde_json for WASM size.
#[cfg(feature = "wasm")]
fn parse_project_files(json: &str, project: &mut crate::ProjectFiles) -> Result<(), String> {
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return Err("Expected JSON object".into());
    }
    let inner = &json[1..json.len()-1];

    // Simple state machine to parse "key": "value" pairs
    let mut chars = inner.chars().peekable();

    loop {
        // Skip whitespace
        while chars.peek().map_or(false, |c| c.is_whitespace() || *c == ',') {
            chars.next();
        }

        if chars.peek().is_none() { break; }

        // Parse key
        let key = parse_json_string(&mut chars)
            .map_err(|_| "Failed to parse filename key")?;

        // Skip : and whitespace
        while chars.peek().map_or(false, |c| c.is_whitespace() || *c == ':') {
            chars.next();
        }

        // Parse value
        let value = parse_json_string(&mut chars)
            .map_err(|_| format!("Failed to parse value for '{}'", key))?;

        // Determine file type by extension
        let lower = key.to_lowercase();
        if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".pdf") {
            // Binary file — decode from base64
            let data = if value.starts_with("data:base64,") {
                decode_base64(&value[12..])
                    .map_err(|_| format!("Failed to decode base64 for '{}'", key))?
            } else if value.starts_with("base64,") {
                decode_base64(&value[7..])
                    .map_err(|_| format!("Failed to decode base64 for '{}'", key))?
            } else {
                // Try raw base64
                decode_base64(&value)
                    .unwrap_or_else(|_| value.into_bytes())
            };
            project.add_binary_file(key, data);
        } else {
            // Text file (.tex, .sty, .cls, .bib, etc.)
            project.add_text_file(key, value);
        }
    }

    Ok(())
}

/// Parse a JSON string (handles escape sequences)
#[cfg(feature = "wasm")]
fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<String, ()> {
    if chars.next() != Some('"') { return Err(()); }
    let mut s = String::new();
    loop {
        match chars.next() {
            Some('"') => return Ok(s),
            Some('\\') => {
                match chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some(c) => { s.push('\\'); s.push(c); }
                    None => return Err(()),
                }
            }
            Some(c) => s.push(c),
            None => return Err(()),
        }
    }
}

/// Minimal base64 decoder (no external crate needed)
#[cfg(feature = "wasm")]
fn decode_base64(input: &str) -> Result<Vec<u8>, ()> {
    let input = input.trim();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for c in input.bytes() {
        let val = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(()),
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(output)
}
