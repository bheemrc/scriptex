/// Font embedding module: load, subset, and embed TrueType/OpenType fonts in PDF
///
/// Uses ttf-parser for reading font files and subsetter for creating subsets
/// containing only the glyphs used in the document.

use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use owned_ttf_parser::AsFaceRef;

/// An embedded font ready for PDF inclusion
#[derive(Debug)]
pub struct EmbeddedFont {
    /// PostScript name of the font
    pub ps_name: String,
    /// Font family name
    pub family_name: String,
    /// Whether the font is bold
    pub is_bold: bool,
    /// Whether the font is italic
    pub is_italic: bool,
    /// Subsetted font data
    pub data: Vec<u8>,
    /// Character to glyph ID mapping (only used chars)
    pub cmap: HashMap<char, u16>,
    /// Glyph widths (glyph_id -> width in 1/1000 em)
    pub widths: HashMap<u16, u16>,
    /// Font metrics
    pub ascent: i16,
    pub descent: i16,
    pub cap_height: i16,
    pub units_per_em: u16,
    /// Font flags for PDF
    pub flags: u32,
    /// Font bounding box [llx, lly, urx, ury]
    pub bbox: [i16; 4],
    /// Italic angle
    pub italic_angle: f32,
}

/// Font resolver: finds font files on the system
pub struct FontResolver {
    /// Cached font file paths
    font_paths: HashMap<String, PathBuf>,
}

impl FontResolver {
    pub fn new() -> Self {
        let mut resolver = FontResolver {
            font_paths: HashMap::new(),
        };
        resolver.scan_system_fonts();
        resolver
    }

    fn scan_system_fonts(&mut self) {
        // Common font directories
        let font_dirs: Vec<PathBuf> = vec![
            // macOS
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
            dirs_home().join("Library/Fonts"),
            // Linux
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
            dirs_home().join(".fonts"),
            dirs_home().join(".local/share/fonts"),
        ];

        for dir in font_dirs {
            if dir.exists() {
                self.scan_directory(&dir);
            }
        }
    }

    fn scan_directory(&mut self, dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    self.scan_directory(&path);
                } else if let Some(ext) = path.extension() {
                    let ext = ext.to_string_lossy().to_lowercase();
                    if ext == "ttf" || ext == "otf" || ext == "ttc" {
                        if let Some(name) = path.file_stem() {
                            let name = name.to_string_lossy().to_lowercase();
                            self.font_paths.insert(name, path.clone());
                        }
                    }
                }
            }
        }
    }

    /// Find a font file by name
    pub fn find_font(&self, name: &str) -> Option<&PathBuf> {
        let lower = name.to_lowercase();
        self.font_paths.get(&lower)
            .or_else(|| {
                // Try without hyphens/spaces
                let normalized: String = lower.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
                self.font_paths.iter()
                    .find(|(k, _)| {
                        let kn: String = k.chars().filter(|c| c.is_alphanumeric()).collect();
                        kn == normalized
                    })
                    .map(|(_, v)| v)
            })
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Load and subset a TrueType font
pub fn load_and_subset(
    path: &Path,
    used_chars: &HashSet<char>,
) -> Option<EmbeddedFont> {
    let font_data = std::fs::read(path).ok()?;
    let face = owned_ttf_parser::OwnedFace::from_vec(font_data.clone(), 0).ok()?;
    let face_ref = face.as_face_ref();

    // Build cmap
    let mut cmap = HashMap::new();
    let mut used_gids = HashSet::new();

    for &ch in used_chars {
        if let Some(gid) = face_ref.glyph_index(ch) {
            cmap.insert(ch, gid.0);
            used_gids.insert(gid.0);
        }
    }

    // Glyph widths
    let units_per_em = face_ref.units_per_em();
    let mut widths = HashMap::new();
    for &gid in &used_gids {
        let gid = owned_ttf_parser::GlyphId(gid);
        let advance = face_ref.glyph_hor_advance(gid).unwrap_or(0);
        let width_1000 = (advance as u32 * 1000 / units_per_em as u32) as u16;
        widths.insert(gid.0, width_1000);
    }

    // Font metrics
    let ascent = face_ref.ascender();
    let descent = face_ref.descender();
    let cap_height = face_ref.capital_height().unwrap_or(ascent);

    // Font flags
    let mut flags = 0u32;
    if face_ref.is_monospaced() { flags |= 1; }  // FixedPitch
    flags |= 32; // Nonsymbolic (has standard encoding)

    // Bounding box
    let bbox = face_ref.global_bounding_box();
    let font_bbox = [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max];

    // Font names
    let family = face_ref.names()
        .into_iter()
        .find(|n| n.name_id == owned_ttf_parser::name_id::FAMILY)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let ps_name = family.replace(' ', "-");
    let is_bold = face_ref.is_bold();
    let is_italic = face_ref.is_italic();
    let italic_angle = if is_italic { -12.0 } else { 0.0 };

    // Subset the font
    let subset_data = subset_font(&font_data, &used_gids);

    Some(EmbeddedFont {
        ps_name,
        family_name: family,
        is_bold,
        is_italic,
        data: subset_data.unwrap_or(font_data),
        cmap,
        widths,
        ascent,
        descent,
        cap_height,
        units_per_em,
        flags,
        bbox: font_bbox,
        italic_angle,
    })
}

/// Subset a font to only include specific glyphs
fn subset_font(font_data: &[u8], glyph_ids: &HashSet<u16>) -> Option<Vec<u8>> {
    let mut sorted_gids: Vec<u16> = glyph_ids.iter().copied().collect();
    sorted_gids.sort();
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&sorted_gids);
    match subsetter::subset(font_data, 0, &remapper) {
        Ok(subset) => Some(subset),
        Err(_) => None,
    }
}

/// Write a CIDFont Type2 with Identity-H CMap to PDF
pub fn write_font_objects(
    font: &EmbeddedFont,
    font_obj_id: u32,
    cidfont_obj_id: u32,
    descriptor_obj_id: u32,
    fontfile_obj_id: u32,
    tounicode_obj_id: u32,
    buf: &mut Vec<u8>,
) {
    let mut itoa_buf = itoa::Buffer::new();

    // Type0 font dictionary
    buf.extend_from_slice(itoa_buf.format(font_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /");
    buf.extend_from_slice(font.ps_name.as_bytes());
    buf.extend_from_slice(b" /Encoding /Identity-H /DescendantFonts [");
    buf.extend_from_slice(itoa_buf.format(cidfont_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 R] /ToUnicode ");
    buf.extend_from_slice(itoa_buf.format(tounicode_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 R >>\nendobj\n");

    // CIDFont dictionary
    buf.extend_from_slice(itoa_buf.format(cidfont_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /");
    buf.extend_from_slice(font.ps_name.as_bytes());
    buf.extend_from_slice(b" /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >>");
    buf.extend_from_slice(b" /FontDescriptor ");
    buf.extend_from_slice(itoa_buf.format(descriptor_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 R");

    // Widths (DW + W array)
    buf.extend_from_slice(b" /DW 1000");
    if !font.widths.is_empty() {
        buf.extend_from_slice(b" /W [");
        let mut sorted_widths: Vec<_> = font.widths.iter().collect();
        sorted_widths.sort_by_key(|&(gid, _)| *gid);
        for (&gid, &width) in &sorted_widths {
            buf.extend_from_slice(itoa_buf.format(gid).as_bytes());
            buf.extend_from_slice(b" [");
            buf.extend_from_slice(itoa_buf.format(width).as_bytes());
            buf.extend_from_slice(b"] ");
        }
        buf.extend_from_slice(b"]");
    }

    buf.extend_from_slice(b" >>\nendobj\n");

    // Font descriptor
    buf.extend_from_slice(itoa_buf.format(descriptor_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 obj\n<< /Type /FontDescriptor /FontName /");
    buf.extend_from_slice(font.ps_name.as_bytes());
    buf.extend_from_slice(b" /Flags ");
    buf.extend_from_slice(itoa_buf.format(font.flags).as_bytes());
    buf.extend_from_slice(b" /FontBBox [");
    for (i, &val) in font.bbox.iter().enumerate() {
        if i > 0 { buf.push(b' '); }
        buf.extend_from_slice(itoa_buf.format(val).as_bytes());
    }
    buf.extend_from_slice(b"] /Ascent ");
    buf.extend_from_slice(itoa_buf.format(font.ascent).as_bytes());
    buf.extend_from_slice(b" /Descent ");
    buf.extend_from_slice(itoa_buf.format(font.descent).as_bytes());
    buf.extend_from_slice(b" /CapHeight ");
    buf.extend_from_slice(itoa_buf.format(font.cap_height).as_bytes());
    buf.extend_from_slice(b" /StemV 80");
    buf.extend_from_slice(b" /FontFile2 ");
    buf.extend_from_slice(itoa_buf.format(fontfile_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 R >>\nendobj\n");

    // Font file stream (compressed)
    let compressed = compress_font_data(&font.data);
    buf.extend_from_slice(itoa_buf.format(fontfile_obj_id).as_bytes());
    buf.extend_from_slice(b" 0 obj\n<< /Length ");
    buf.extend_from_slice(itoa_buf.format(compressed.len()).as_bytes());
    buf.extend_from_slice(b" /Length1 ");
    buf.extend_from_slice(itoa_buf.format(font.data.len()).as_bytes());
    buf.extend_from_slice(b" /Filter /FlateDecode >>\nstream\n");
    buf.extend_from_slice(&compressed);
    buf.extend_from_slice(b"\nendstream\nendobj\n");
}

fn compress_font_data(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::fast());
    encoder.write_all(data).unwrap_or(());
    encoder.finish().unwrap_or_default()
}

/// Generate ToUnicode CMap stream
pub fn generate_tounicode_cmap(cmap: &HashMap<char, u16>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);

    buf.extend_from_slice(b"/CIDInit /ProcSet findresource begin\n");
    buf.extend_from_slice(b"12 dict begin\n");
    buf.extend_from_slice(b"begincmap\n");
    buf.extend_from_slice(b"/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    buf.extend_from_slice(b"/CMapName /Adobe-Identity-UCS def\n");
    buf.extend_from_slice(b"/CMapType 2 def\n");
    buf.extend_from_slice(b"1 begincodespacerange\n");
    buf.extend_from_slice(b"<0000> <FFFF>\n");
    buf.extend_from_slice(b"endcodespacerange\n");

    // Generate mappings
    let mut sorted: Vec<_> = cmap.iter().collect();
    sorted.sort_by_key(|&(_, gid)| *gid);

    let chunks: Vec<_> = sorted.chunks(100).collect();
    for chunk in chunks {
        buf.extend_from_slice(itoa::Buffer::new().format(chunk.len()).as_bytes());
        buf.extend_from_slice(b" beginbfchar\n");
        for &(&ch, &gid) in chunk {
            write!(buf, "<{:04X}> <{:04X}>\n", gid, ch as u32).unwrap_or(());
        }
        buf.extend_from_slice(b"endbfchar\n");
    }

    buf.extend_from_slice(b"endcmap\n");
    buf.extend_from_slice(b"CMapName currentdict /CMap defineresource pop\n");
    buf.extend_from_slice(b"end\nend\n");

    buf
}
