/// Image embedding module: load PNG/JPEG images and create PDF image XObjects
///
/// JPEG: embed raw bytes directly (DCTDecode filter) - fastest path
/// PNG: decode and re-encode as raw RGB/RGBA with FlateDecode

use std::path::{Path, PathBuf};
use std::io::Write;

/// Information about an embedded image
#[derive(Debug, Clone)]
pub struct EmbeddedImage {
    /// Original image path
    pub path: String,
    /// Width in pixels
    pub pixel_width: u32,
    /// Height in pixels
    pub pixel_height: u32,
    /// Display width in points
    pub display_width: f32,
    /// Display height in points
    pub display_height: f32,
    /// Image format
    pub format: ImageFormat,
    /// Raw image data for PDF embedding
    pub data: Vec<u8>,
    /// Color space (RGB, Gray, CMYK)
    pub color_space: ColorSpace,
    /// Bits per component
    pub bits_per_component: u8,
    /// Whether the image has alpha channel (separate SMask)
    pub has_alpha: bool,
    /// Alpha channel data (if any)
    pub alpha_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorSpace {
    DeviceRGB,
    DeviceGray,
    DeviceCMYK,
}

impl EmbeddedImage {
    /// PDF filter name for this image's data
    pub fn pdf_filter(&self) -> &str {
        match self.format {
            ImageFormat::Jpeg => "/DCTDecode",
            ImageFormat::Png => "/FlateDecode",
        }
    }

    /// PDF color space name
    pub fn pdf_color_space(&self) -> &str {
        match self.color_space {
            ColorSpace::DeviceRGB => "/DeviceRGB",
            ColorSpace::DeviceGray => "/DeviceGray",
            ColorSpace::DeviceCMYK => "/DeviceCMYK",
        }
    }
}

/// Load an image from a file path, resolving relative to a base directory
pub fn load_image(path: &str, base_dir: &Path, target_width: Option<f32>, target_height: Option<f32>) -> Option<EmbeddedImage> {
    let full_path = resolve_image_path(path, base_dir)?;
    let ext = full_path.extension()?.to_str()?.to_lowercase();

    match ext.as_str() {
        "jpg" | "jpeg" => load_jpeg(&full_path, target_width, target_height),
        "png" => load_png(&full_path, target_width, target_height),
        _ => None,
    }
}

fn resolve_image_path(path: &str, base_dir: &Path) -> Option<PathBuf> {
    // Clean path
    let path = path.trim().trim_matches(|c| c == '"' || c == '\'');

    let p = Path::new(path);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }

    // Try relative to base dir
    let full = base_dir.join(path);
    if full.exists() {
        return Some(full);
    }

    // Try with common extensions
    for ext in &["png", "jpg", "jpeg", "pdf", "eps"] {
        let with_ext = full.with_extension(ext);
        if with_ext.exists() {
            return Some(with_ext);
        }
    }

    // Try in common subdirectories
    for subdir in &["images", "figures", "fig", "img"] {
        let in_subdir = base_dir.join(subdir).join(path);
        if in_subdir.exists() {
            return Some(in_subdir);
        }
        for ext in &["png", "jpg", "jpeg"] {
            let with_ext = in_subdir.with_extension(ext);
            if with_ext.exists() {
                return Some(with_ext);
            }
        }
    }

    None
}

/// Load a JPEG image (embed raw bytes - fastest path)
fn load_jpeg(path: &Path, target_width: Option<f32>, target_height: Option<f32>) -> Option<EmbeddedImage> {
    let data = std::fs::read(path).ok()?;

    // Parse JPEG header to get dimensions
    let (width, height) = parse_jpeg_dimensions(&data)?;

    let (dw, dh) = compute_display_size(width, height, target_width, target_height);

    Some(EmbeddedImage {
        path: path.to_string_lossy().to_string(),
        pixel_width: width,
        pixel_height: height,
        display_width: dw,
        display_height: dh,
        format: ImageFormat::Jpeg,
        data,
        color_space: ColorSpace::DeviceRGB,
        bits_per_component: 8,
        has_alpha: false,
        alpha_data: Vec::new(),
    })
}

/// Load a PNG image (decode, re-encode for PDF)
fn load_png(path: &Path, target_width: Option<f32>, target_height: Option<f32>) -> Option<EmbeddedImage> {
    use image::GenericImageView;

    let img = image::open(path).ok()?;
    let (width, height) = img.dimensions();
    let (dw, dh) = compute_display_size(width, height, target_width, target_height);

    let has_alpha = img.color().has_alpha();
    let (rgb_data, alpha_data) = if has_alpha {
        let rgba = img.to_rgba8();
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        let mut alpha = Vec::with_capacity((width * height) as usize);
        for pixel in rgba.pixels() {
            rgb.push(pixel[0]);
            rgb.push(pixel[1]);
            rgb.push(pixel[2]);
            alpha.push(pixel[3]);
        }
        (rgb, alpha)
    } else {
        let rgb = img.to_rgb8();
        (rgb.into_raw(), Vec::new())
    };

    // Compress with flate2
    let compressed = compress_flate(&rgb_data);
    let alpha_compressed = if has_alpha { compress_flate(&alpha_data) } else { Vec::new() };

    Some(EmbeddedImage {
        path: path.to_string_lossy().to_string(),
        pixel_width: width,
        pixel_height: height,
        display_width: dw,
        display_height: dh,
        format: ImageFormat::Png,
        data: compressed,
        color_space: ColorSpace::DeviceRGB,
        bits_per_component: 8,
        has_alpha,
        alpha_data: alpha_compressed,
    })
}

fn compress_flate(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    let mut encoder = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::fast());
    encoder.write_all(data).unwrap_or(());
    encoder.finish().unwrap_or_default()
}

/// Parse JPEG dimensions from header (SOF marker)
fn parse_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None; // Not a JPEG
    }

    let mut pos = 2;
    while pos + 4 < data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        let marker = data[pos + 1];
        pos += 2;

        // SOF markers (Start Of Frame)
        if (marker >= 0xC0 && marker <= 0xC3) || (marker >= 0xC5 && marker <= 0xC7)
            || (marker >= 0xC9 && marker <= 0xCB) || (marker >= 0xCD && marker <= 0xCF)
        {
            if pos + 7 <= data.len() {
                let height = ((data[pos + 3] as u32) << 8) | data[pos + 4] as u32;
                let width = ((data[pos + 5] as u32) << 8) | data[pos + 6] as u32;
                return Some((width, height));
            }
            return None;
        }

        // Skip marker segment
        if pos + 2 <= data.len() {
            let seg_len = ((data[pos] as usize) << 8) | data[pos + 1] as usize;
            pos += seg_len;
        } else {
            break;
        }
    }

    None
}

/// Compute display size maintaining aspect ratio
fn compute_display_size(pixel_w: u32, pixel_h: u32, target_w: Option<f32>, target_h: Option<f32>) -> (f32, f32) {
    // Default: 72 DPI (1 pixel = 1 point)
    let natural_w = pixel_w as f32;
    let natural_h = pixel_h as f32;

    match (target_w, target_h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let scale = w / natural_w;
            (w, natural_h * scale)
        }
        (None, Some(h)) => {
            let scale = h / natural_h;
            (natural_w * scale, h)
        }
        (None, None) => {
            // Cap at reasonable page size (468pt = text width of US Letter)
            if natural_w > 468.0 {
                let scale = 468.0 / natural_w;
                (468.0, natural_h * scale)
            } else {
                (natural_w, natural_h)
            }
        }
    }
}

/// Write image XObject to PDF
/// Returns the number of bytes written and a Vec of objects written
pub fn write_image_xobject(img: &EmbeddedImage, obj_id: u32, buf: &mut Vec<u8>) {
    // Image XObject
    buf.extend_from_slice(itoa::Buffer::new().format(obj_id).as_bytes());
    buf.extend_from_slice(b" 0 obj\n<< /Type /XObject /Subtype /Image");
    buf.extend_from_slice(b" /Width ");
    buf.extend_from_slice(itoa::Buffer::new().format(img.pixel_width).as_bytes());
    buf.extend_from_slice(b" /Height ");
    buf.extend_from_slice(itoa::Buffer::new().format(img.pixel_height).as_bytes());
    buf.extend_from_slice(b" /ColorSpace ");
    buf.extend_from_slice(img.pdf_color_space().as_bytes());
    buf.extend_from_slice(b" /BitsPerComponent ");
    buf.extend_from_slice(itoa::Buffer::new().format(img.bits_per_component).as_bytes());
    buf.extend_from_slice(b" /Filter ");
    buf.extend_from_slice(img.pdf_filter().as_bytes());
    buf.extend_from_slice(b" /Length ");
    buf.extend_from_slice(itoa::Buffer::new().format(img.data.len()).as_bytes());
    buf.extend_from_slice(b" >>\nstream\n");
    buf.extend_from_slice(&img.data);
    buf.extend_from_slice(b"\nendstream\nendobj\n");
}
