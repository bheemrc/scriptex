/// Font metrics module: per-glyph width tables for PDF Standard 14 fonts
/// Widths from Adobe AFM files, stored as u16 in 1/1000 em units
/// Provides O(1) character width lookup for accurate text measurement

/// Font identifiers matching PDF font numbering (F1-F6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FontId {
    Helvetica = 1,
    HelveticaBold = 2,
    HelveticaOblique = 3,
    HelveticaBoldOblique = 4,
    Courier = 5,
    Symbol = 6,
}

/// Helvetica character widths (WinAnsi encoding, indices 0-255)
/// Source: Helvetica AFM from Adobe
static HELVETICA_WIDTHS: [u16; 256] = [
    // 0x00-0x1F: control chars (use 0)
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    // 0x20 space through 0x7E tilde
    278, // space
    278, // !
    355, // "
    556, // #
    556, // $
    889, // %
    667, // &
    191, // '
    333, // (
    333, // )
    389, // *
    584, // +
    278, // ,
    333, // -
    278, // .
    278, // /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, // :
    278, // ;
    584, // <
    584, // =
    584, // >
    556, // ?
    1015,// @
    667, 667, 722, 722, 611, 556, 778, 722, 278, 500, // A-J
    667, 556, 833, 722, 778, 667, 778, 722, 667, 611, // K-T
    722, 667, 944, 667, 667, 611, // U-Z
    278, // [
    278, // backslash
    278, // ]
    469, // ^
    556, // _
    333, // `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, // a-j
    500, 222, 833, 556, 556, 556, 556, 333, 500, 278, // k-t
    556, 500, 722, 500, 500, 500, // u-z
    334, // {
    260, // |
    334, // }
    584, // ~
    0,   // DEL
    // 0x80-0xFF: extended WinAnsi
    556, // 0x80 Euro
    0,   // 0x81 undefined
    222, // 0x82 quotesinglbase
    556, // 0x83 florin
    333, // 0x84 quotedblbase
    1000,// 0x85 ellipsis
    556, // 0x86 dagger
    556, // 0x87 daggerdbl
    333, // 0x88 circumflex
    1000,// 0x89 perthousand
    667, // 0x8A Scaron
    333, // 0x8B guilsinglleft
    1000,// 0x8C OE
    0,   // 0x8D undefined
    611, // 0x8E Zcaron
    0,   // 0x8F undefined
    0,   // 0x90 undefined
    222, // 0x91 quoteleft
    222, // 0x92 quoteright
    333, // 0x93 quotedblleft
    333, // 0x94 quotedblright
    350, // 0x95 bullet
    556, // 0x96 endash
    1000,// 0x97 emdash
    333, // 0x98 tilde
    1000,// 0x99 trademark
    500, // 0x9A scaron
    333, // 0x9B guilsinglright
    944, // 0x9C oe
    0,   // 0x9D undefined
    500, // 0x9E zcaron
    667, // 0x9F Ydieresis
    278, // 0xA0 nbspace
    333, // 0xA1 exclamdown
    556, // 0xA2 cent
    556, // 0xA3 sterling
    556, // 0xA4 currency
    556, // 0xA5 yen
    260, // 0xA6 brokenbar
    556, // 0xA7 section
    333, // 0xA8 dieresis
    737, // 0xA9 copyright
    370, // 0xAA ordfeminine
    556, // 0xAB guillemotleft
    584, // 0xAC logicalnot
    333, // 0xAD softhyphen
    737, // 0xAE registered
    333, // 0xAF macron
    400, // 0xB0 degree
    584, // 0xB1 plusminus
    333, // 0xB2 twosuperior
    333, // 0xB3 threesuperior
    333, // 0xB4 acute
    556, // 0xB5 mu
    537, // 0xB6 paragraph
    278, // 0xB7 periodcentered
    333, // 0xB8 cedilla
    333, // 0xB9 onesuperior
    365, // 0xBA ordmasculine
    556, // 0xBB guillemotright
    834, // 0xBC onequarter
    834, // 0xBD onehalf
    834, // 0xBE threequarters
    611, // 0xBF questiondown
    667, // 0xC0 Agrave
    667, // 0xC1 Aacute
    667, // 0xC2 Acircumflex
    667, // 0xC3 Atilde
    667, // 0xC4 Adieresis
    667, // 0xC5 Aring
    1000,// 0xC6 AE
    722, // 0xC7 Ccedilla
    611, // 0xC8 Egrave
    611, // 0xC9 Eacute
    611, // 0xCA Ecircumflex
    611, // 0xCB Edieresis
    278, // 0xCC Igrave
    278, // 0xCD Iacute
    278, // 0xCE Icircumflex
    278, // 0xCF Idieresis
    722, // 0xD0 Eth
    722, // 0xD1 Ntilde
    778, // 0xD2 Ograve
    778, // 0xD3 Oacute
    778, // 0xD4 Ocircumflex
    778, // 0xD5 Otilde
    778, // 0xD6 Odieresis
    584, // 0xD7 multiply
    778, // 0xD8 Oslash
    722, // 0xD9 Ugrave
    722, // 0xDA Uacute
    722, // 0xDB Ucircumflex
    722, // 0xDC Udieresis
    667, // 0xDD Yacute
    667, // 0xDE Thorn
    611, // 0xDF germandbls
    556, // 0xE0 agrave
    556, // 0xE1 aacute
    556, // 0xE2 acircumflex
    556, // 0xE3 atilde
    556, // 0xE4 adieresis
    556, // 0xE5 aring
    889, // 0xE6 ae
    500, // 0xE7 ccedilla
    556, // 0xE8 egrave
    556, // 0xE9 eacute
    556, // 0xEA ecircumflex
    556, // 0xEB edieresis
    278, // 0xEC igrave
    278, // 0xED iacute
    278, // 0xEE icircumflex
    278, // 0xEF idieresis
    556, // 0xF0 eth
    556, // 0xF1 ntilde
    556, // 0xF2 ograve
    556, // 0xF3 oacute
    556, // 0xF4 ocircumflex
    556, // 0xF5 otilde
    556, // 0xF6 odieresis
    584, // 0xF7 divide
    611, // 0xF8 oslash
    556, // 0xF9 ugrave
    556, // 0xFA uacute
    556, // 0xFB ucircumflex
    556, // 0xFC udieresis
    500, // 0xFD yacute
    556, // 0xFE thorn
    500, // 0xFF ydieresis
];

/// Helvetica-Bold character widths
static HELVETICA_BOLD_WIDTHS: [u16; 256] = [
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    278, // space
    333, // !
    474, // "
    556, // #
    556, // $
    889, // %
    722, // &
    238, // '
    333, // (
    333, // )
    389, // *
    584, // +
    278, // ,
    333, // -
    278, // .
    278, // /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    333, // :
    333, // ;
    584, // <
    584, // =
    584, // >
    611, // ?
    975, // @
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, // A-J
    722, 611, 833, 722, 778, 667, 778, 722, 667, 611, // K-T
    722, 667, 944, 667, 667, 611, // U-Z
    333, // [
    278, // backslash
    333, // ]
    584, // ^
    556, // _
    333, // `
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, // a-j
    556, 278, 889, 611, 611, 611, 611, 389, 556, 333, // k-t
    611, 556, 778, 556, 556, 500, // u-z
    389, // {
    280, // |
    389, // }
    584, // ~
    0,   // DEL
    // 0x80-0xFF (abbreviated, using Helvetica values as close approximation)
    556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000,
    667, 333, 1000, 0, 611, 0,
    0, 278, 278, 500, 500, 350, 556, 1000, 333, 1000,
    556, 333, 944, 0, 500, 667,
    278, 333, 556, 556, 556, 556, 280, 556, 333, 737,
    370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 611, 556, 278, 333, 333,
    365, 556, 834, 834, 834, 611,
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667,
    667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722,
    722, 722, 722, 667, 667, 611,
    556, 556, 556, 556, 556, 556, 889, 556, 556, 556,
    556, 556, 278, 278, 278, 278,
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611,
    611, 611, 611, 556, 611, 556,
];

/// Helvetica-Oblique widths (same as Helvetica - oblique doesn't change widths)
static HELVETICA_OBLIQUE_WIDTHS: [u16; 256] = HELVETICA_WIDTHS;

/// Helvetica-BoldOblique widths (same as Helvetica-Bold)
static HELVETICA_BOLDOBLIQUE_WIDTHS: [u16; 256] = HELVETICA_BOLD_WIDTHS;

/// Courier widths (all 600 - monospace)
static COURIER_WIDTHS: [u16; 256] = {
    let mut w = [600u16; 256];
    // Control chars
    let mut i = 0;
    while i < 32 {
        w[i] = 0;
        i += 1;
    }
    w[127] = 0;
    w
};

/// Symbol font widths (selected entries)
static SYMBOL_WIDTHS: [u16; 256] = {
    let mut w = [500u16; 256]; // default
    let mut i = 0;
    while i < 32 { w[i] = 0; i += 1; }
    w[32] = 250;  // space
    w[40] = 333;  // (
    w[41] = 333;  // )
    w[43] = 549;  // +
    w[44] = 250;  // ,
    w[45] = 549;  // minus
    w[46] = 250;  // .
    w[47] = 278;  // /
    w[48] = 500; w[49] = 500; w[50] = 500; w[51] = 500; w[52] = 500; // 0-4
    w[53] = 500; w[54] = 500; w[55] = 500; w[56] = 500; w[57] = 500; // 5-9
    w[61] = 549;  // =
    w[65] = 631;  // Alpha
    w[66] = 549;  // Beta
    w[67] = 603;  // Chi
    w[68] = 494;  // Delta
    w[69] = 439;  // Epsilon
    w[70] = 521;  // Phi
    w[71] = 411;  // Gamma
    w[72] = 603;  // Eta
    w[73] = 329;  // Iota
    w[75] = 549;  // Kappa
    w[76] = 686;  // Lambda
    w[77] = 713;  // Mu
    w[78] = 494;  // Nu
    w[79] = 768;  // Omicron
    w[80] = 603;  // Pi
    w[81] = 521;  // Theta
    w[82] = 549;  // Rho
    w[83] = 603;  // Sigma
    w[84] = 439;  // Tau
    w[85] = 576;  // Upsilon
    w[87] = 768;  // Omega
    w[88] = 603;  // Xi
    w[89] = 549;  // Psi
    w[90] = 494;  // Zeta
    w[97] = 631;  // alpha
    w[98] = 549;  // beta
    w[99] = 549;  // chi
    w[100] = 494; // delta
    w[101] = 439; // epsilon
    w[102] = 521; // phi
    w[103] = 411; // gamma
    w[104] = 603; // eta
    w[105] = 329; // iota
    w[107] = 549; // kappa
    w[108] = 549; // lambda
    w[109] = 576; // mu
    w[110] = 521; // nu
    w[111] = 549; // omicron
    w[112] = 549; // pi
    w[113] = 521; // theta
    w[114] = 549; // rho
    w[115] = 603; // sigma
    w[116] = 439; // tau
    w[117] = 576; // upsilon
    w[119] = 713; // omega
    w[120] = 493; // xi
    w[121] = 686; // psi
    w[122] = 494; // zeta
    // Large operators
    w[229] = 713; // summation (Sigma)
    w[242] = 713; // integral
    w[213] = 713; // product
    w
};

/// Get the width table for a given font
#[inline]
pub fn font_widths(font: FontId) -> &'static [u16; 256] {
    match font {
        FontId::Helvetica => &HELVETICA_WIDTHS,
        FontId::HelveticaBold => &HELVETICA_BOLD_WIDTHS,
        FontId::HelveticaOblique => &HELVETICA_OBLIQUE_WIDTHS,
        FontId::HelveticaBoldOblique => &HELVETICA_BOLDOBLIQUE_WIDTHS,
        FontId::Courier => &COURIER_WIDTHS,
        FontId::Symbol => &SYMBOL_WIDTHS,
    }
}

/// Get width of a single character in 1/1000 em units
#[inline(always)]
pub fn char_width_1000(font: FontId, byte: u8) -> u16 {
    let widths = font_widths(font);
    widths[byte as usize]
}

/// Get width of a character in points for a given font size
#[inline(always)]
pub fn char_width_pt(font: FontId, byte: u8, font_size: f32) -> f32 {
    char_width_1000(font, byte) as f32 * font_size * 0.001
}

/// Measure text width in points using per-character widths
/// For ASCII text, this is very fast (one table lookup per byte)
pub fn measure_text(text: &str, font: FontId, font_size: f32) -> f32 {
    let widths = font_widths(font);
    let scale = font_size * 0.001;
    let bytes = text.as_bytes();
    let mut total: u32 = 0;

    // Process 4 bytes at a time for better throughput
    let chunks = bytes.len() / 4;
    let mut i = 0;
    for _ in 0..chunks {
        unsafe {
            total += *widths.get_unchecked(*bytes.get_unchecked(i) as usize) as u32;
            total += *widths.get_unchecked(*bytes.get_unchecked(i + 1) as usize) as u32;
            total += *widths.get_unchecked(*bytes.get_unchecked(i + 2) as usize) as u32;
            total += *widths.get_unchecked(*bytes.get_unchecked(i + 3) as usize) as u32;
        }
        i += 4;
    }
    while i < bytes.len() {
        total += widths[bytes[i] as usize] as u32;
        i += 1;
    }

    total as f32 * scale
}

/// Measure text width returning total in 1/1000 em (for precise integer math)
#[inline]
pub fn measure_text_1000(text: &str, font: FontId) -> u32 {
    let widths = font_widths(font);
    let bytes = text.as_bytes();
    let mut total: u32 = 0;
    for &b in bytes {
        total += widths[b as usize] as u32;
    }
    total
}

/// Average character width in 1/1000 em units for a font
/// Used for fast line-length estimation in bulk text layout
#[inline]
pub fn avg_char_width_1000(font: FontId) -> u16 {
    match font {
        FontId::Helvetica | FontId::HelveticaOblique => 513,     // weighted avg of lowercase
        FontId::HelveticaBold | FontId::HelveticaBoldOblique => 547,
        FontId::Courier => 600,
        FontId::Symbol => 500,
    }
}

/// Space width in 1/1000 em
#[inline(always)]
pub fn space_width_1000(font: FontId) -> u16 {
    match font {
        FontId::Helvetica | FontId::HelveticaOblique => 278,
        FontId::HelveticaBold | FontId::HelveticaBoldOblique => 278,
        FontId::Courier => 600,
        FontId::Symbol => 250,
    }
}

/// Convert FontStyle to FontId
#[inline]
pub fn style_to_font_id(style: crate::typeset::FontStyle) -> FontId {
    use crate::typeset::FontStyle;
    match style {
        FontStyle::Regular | FontStyle::SmallCaps => FontId::Helvetica,
        FontStyle::Bold => FontId::HelveticaBold,
        FontStyle::Italic => FontId::HelveticaOblique,
        FontStyle::BoldItalic => FontId::HelveticaBoldOblique,
        FontStyle::Monospace => FontId::Courier,
    }
}

/// Font metrics: ascent, descent, cap height, x-height (in 1/1000 em)
pub struct FontInfo {
    pub ascent: u16,
    pub descent: i16,   // negative value
    pub cap_height: u16,
    pub x_height: u16,
    pub line_gap: u16,
}

pub fn font_info(font: FontId) -> FontInfo {
    match font {
        FontId::Helvetica | FontId::HelveticaOblique => FontInfo {
            ascent: 718, descent: -207, cap_height: 718, x_height: 523, line_gap: 200,
        },
        FontId::HelveticaBold | FontId::HelveticaBoldOblique => FontInfo {
            ascent: 718, descent: -207, cap_height: 718, x_height: 532, line_gap: 200,
        },
        FontId::Courier => FontInfo {
            ascent: 629, descent: -157, cap_height: 562, x_height: 426, line_gap: 200,
        },
        FontId::Symbol => FontInfo {
            ascent: 1010, descent: -293, cap_height: 1010, x_height: 500, line_gap: 200,
        },
    }
}

/// Compute justified word spacing: extra space to add per word gap
/// to make a line fill the target width
#[inline]
pub fn justified_word_spacing(line_width: f32, target_width: f32, num_spaces: u32) -> f32 {
    if num_spaces == 0 {
        return 0.0;
    }
    let slack = target_width - line_width;
    if slack <= 0.0 || slack > target_width * 0.4 {
        // Too much stretch - don't justify (last line or very short)
        return 0.0;
    }
    slack / num_spaces as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helvetica_space() {
        assert_eq!(HELVETICA_WIDTHS[32], 278);
    }

    #[test]
    fn test_helvetica_uppercase_a() {
        assert_eq!(HELVETICA_WIDTHS[b'A' as usize], 667);
    }

    #[test]
    fn test_courier_monospace() {
        for i in 32..127u8 {
            assert_eq!(COURIER_WIDTHS[i as usize], 600, "Courier char {} should be 600", i as char);
        }
    }

    #[test]
    fn test_measure_text() {
        let w = measure_text("Hello", FontId::Helvetica, 10.0);
        // H=722 + e=556 + l=222 + l=222 + o=556 = 2278 / 1000 * 10 = 22.78
        assert!((w - 22.78).abs() < 0.01, "Expected ~22.78, got {}", w);
    }
}
