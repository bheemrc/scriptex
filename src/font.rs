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
    // Widths from the official Adobe Symbol font AFM file
    let mut w = [500u16; 256]; // default
    let mut i = 0;
    while i < 32 { w[i] = 0; i += 1; }
    w[32] = 250;  // space
    w[33] = 333;  // exclam
    w[34] = 713;  // universal
    w[35] = 500;  // numbersign
    w[36] = 549;  // existential
    w[37] = 833;  // percent
    w[38] = 778;  // ampersand
    w[39] = 439;  // suchthat
    w[40] = 333;  // parenleft
    w[41] = 333;  // parenright
    w[42] = 500;  // asteriskmath
    w[43] = 549;  // plus
    w[44] = 250;  // comma
    w[45] = 549;  // minus
    w[46] = 250;  // period
    w[47] = 278;  // slash
    w[48] = 500; w[49] = 500; w[50] = 500; w[51] = 500; w[52] = 500; // 0-4
    w[53] = 500; w[54] = 500; w[55] = 500; w[56] = 500; w[57] = 500; // 5-9
    w[58] = 278;  // colon
    w[59] = 278;  // semicolon
    w[60] = 549;  // less
    w[61] = 549;  // equal
    w[62] = 549;  // greater
    w[63] = 444;  // question
    w[64] = 549;  // congruent
    // Uppercase Greek (from Adobe Symbol AFM)
    w[65] = 722;  // Alpha
    w[66] = 667;  // Beta
    w[67] = 722;  // Chi
    w[68] = 612;  // Delta
    w[69] = 611;  // Epsilon
    w[70] = 763;  // Phi
    w[71] = 603;  // Gamma
    w[72] = 722;  // Eta
    w[73] = 333;  // Iota
    w[74] = 631;  // theta1
    w[75] = 722;  // Kappa
    w[76] = 686;  // Lambda
    w[77] = 889;  // Mu
    w[78] = 722;  // Nu
    w[79] = 722;  // Omicron
    w[80] = 768;  // Pi
    w[81] = 741;  // Theta
    w[82] = 556;  // Rho
    w[83] = 592;  // Sigma
    w[84] = 611;  // Tau
    w[85] = 690;  // Upsilon
    w[86] = 439;  // sigma1
    w[87] = 768;  // Omega
    w[88] = 645;  // Xi
    w[89] = 795;  // Psi
    w[90] = 611;  // Zeta
    w[91] = 333;  // bracketleft
    w[92] = 863;  // therefore
    w[93] = 333;  // bracketright
    w[94] = 658;  // perpendicular
    w[95] = 500;  // underscore
    w[96] = 500;  // radicalex
    // Lowercase Greek (from Adobe Symbol AFM)
    w[97] = 631;  // alpha
    w[98] = 549;  // beta
    w[99] = 549;  // chi
    w[100] = 494; // delta
    w[101] = 439; // epsilon
    w[102] = 521; // phi
    w[103] = 411; // gamma
    w[104] = 603; // eta
    w[105] = 329; // iota
    w[106] = 603; // phi1
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
    w[118] = 713; // omega1
    w[119] = 686; // omega
    w[120] = 493; // xi
    w[121] = 686; // psi
    w[122] = 494; // zeta
    // Symbols and operators (from Adobe Symbol AFM)
    w[163] = 549; // lessequal
    w[164] = 167; // fraction
    w[165] = 713; // infinity
    w[166] = 500; // florin
    w[167] = 753; // club
    w[168] = 753; // diamond
    w[169] = 753; // heart
    w[170] = 753; // spade
    w[171] = 1042; // arrowboth
    w[172] = 987; // arrowleft
    w[173] = 603; // arrowup
    w[174] = 987; // arrowright
    w[175] = 603; // arrowdown
    w[176] = 400; // degree
    w[177] = 549; // plusminus
    w[178] = 411; // second
    w[179] = 549; // greaterequal
    w[180] = 549; // multiply
    w[181] = 713; // proportional
    w[182] = 494; // partialdiff
    w[183] = 460; // bullet
    w[184] = 549; // divide
    w[185] = 549; // notequal
    w[186] = 549; // equivalence
    w[187] = 549; // approxequal
    w[188] = 1000; // ellipsis
    w[192] = 823; // aleph
    w[193] = 686; // Ifraktur
    w[194] = 795; // Rfraktur
    w[195] = 987; // weierstrass
    w[196] = 768; // circlemultiply
    w[197] = 768; // circleplus
    w[198] = 823; // emptyset
    w[199] = 768; // intersection
    w[200] = 768; // union
    w[201] = 713; // propersuperset
    w[202] = 713; // reflexsuperset
    w[203] = 713; // notsubset
    w[204] = 713; // propersubset
    w[205] = 713; // reflexsubset
    w[206] = 713; // element
    w[207] = 713; // notelement
    w[208] = 768; // angle
    w[209] = 713; // gradient (nabla)
    w[213] = 823; // product
    w[214] = 549; // radical
    w[215] = 250; // dotmath
    w[216] = 713; // logicalnot
    w[217] = 603; // logicaland
    w[218] = 603; // logicalor
    w[219] = 1042; // arrowdblboth
    w[220] = 987; // arrowdblleft
    w[222] = 987; // arrowdblright
    w[224] = 494; // lozenge
    w[225] = 329; // angleleft
    w[229] = 713; // summation
    w[241] = 329; // angleright
    w[242] = 274; // integral
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

    // Check if text is pure ASCII (common fast path)
    if bytes.iter().all(|&b| b < 0x80) {
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
    } else {
        // Slow path: decode UTF-8 chars and map to WinAnsi byte positions
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b < 0x80 {
                total += widths[b as usize] as u32;
                i += 1;
            } else if b < 0xC0 {
                i += 1; // stray continuation byte
            } else {
                // Decode UTF-8 to Unicode codepoint
                let (cp, advance) = if b < 0xE0 && i + 1 < bytes.len() {
                    (((b as u32 & 0x1F) << 6) | (bytes[i+1] as u32 & 0x3F), 2)
                } else if b < 0xF0 && i + 2 < bytes.len() {
                    (((b as u32 & 0x0F) << 12) | ((bytes[i+1] as u32 & 0x3F) << 6)
                        | (bytes[i+2] as u32 & 0x3F), 3)
                } else if i + 3 < bytes.len() {
                    (((b as u32 & 0x07) << 18) | ((bytes[i+1] as u32 & 0x3F) << 12)
                        | ((bytes[i+2] as u32 & 0x3F) << 6) | (bytes[i+3] as u32 & 0x3F), 4)
                } else {
                    (0xFFFD, 1)
                };
                // Map Unicode codepoint to WinAnsi byte (same as pdf.rs encoding)
                let win_byte = unicode_to_winansi(cp);
                total += widths[win_byte as usize] as u32;
                i += advance;
            }
        }
    }

    total as f32 * scale
}

/// Map a Unicode codepoint to its WinAnsi byte equivalent.
/// Returns '?' (0x3F) for unmappable characters.
#[inline]
fn unicode_to_winansi(cp: u32) -> u8 {
    match cp {
        0x00..=0x7F => cp as u8,
        0x00A0..=0x00FF => cp as u8,
        0x2022 => 0x95, 0x2013 => 0x96, 0x2014 => 0x97,
        0x2018 => 0x91, 0x2019 => 0x92, 0x201C => 0x93, 0x201D => 0x94,
        0x2026 => 0x85, 0x2020 => 0x86, 0x2021 => 0x87, 0x2030 => 0x89,
        0x0152 => 0x8C, 0x0153 => 0x9C, 0x0160 => 0x8A, 0x0161 => 0x9A,
        0x0178 => 0x9F, 0x017D => 0x8E, 0x017E => 0x9E, 0x0192 => 0x83,
        0x02C6 => 0x88, 0x02DC => 0x98, 0x20AC => 0x80, 0x2122 => 0x99,
        _ => b'?',
    }
}

/// Measure text width returning total in 1/1000 em (for precise integer math)
#[inline]
pub fn measure_text_1000(text: &str, font: FontId) -> u32 {
    let widths = font_widths(font);
    let bytes = text.as_bytes();
    let mut total: u32 = 0;
    if bytes.iter().all(|&b| b < 0x80) {
        for &b in bytes {
            total += widths[b as usize] as u32;
        }
    } else {
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b < 0x80 {
                total += widths[b as usize] as u32;
                i += 1;
            } else if b < 0xC0 {
                i += 1;
            } else {
                let (cp, advance) = if b < 0xE0 && i + 1 < bytes.len() {
                    (((b as u32 & 0x1F) << 6) | (bytes[i+1] as u32 & 0x3F), 2)
                } else if b < 0xF0 && i + 2 < bytes.len() {
                    (((b as u32 & 0x0F) << 12) | ((bytes[i+1] as u32 & 0x3F) << 6)
                        | (bytes[i+2] as u32 & 0x3F), 3)
                } else if i + 3 < bytes.len() {
                    (((b as u32 & 0x07) << 18) | ((bytes[i+1] as u32 & 0x3F) << 12)
                        | ((bytes[i+2] as u32 & 0x3F) << 6) | (bytes[i+3] as u32 & 0x3F), 4)
                } else {
                    (0xFFFD, 1)
                };
                let win_byte = unicode_to_winansi(cp);
                total += widths[win_byte as usize] as u32;
                i += advance;
            }
        }
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
        FontStyle::Symbol => FontId::Symbol,
    }
}

/// Map a Unicode codepoint to the PDF Symbol font byte encoding.
/// Returns the Symbol encoding byte position for Greek letters, math operators, arrows, etc.
/// Based on the Adobe Symbol Encoding table.
#[inline]
pub fn unicode_to_symbol_byte(ch: char) -> Option<u8> {
    let cp = ch as u32;
    Some(match cp {
        // Lowercase Greek letters
        0x03B1 => 0x61, // α alpha
        0x03B2 => 0x62, // β beta
        0x03C7 => 0x63, // χ chi
        0x03B4 => 0x64, // δ delta
        0x03B5 => 0x65, // ε epsilon
        0x03C6 => 0x66, // φ phi
        0x03B3 => 0x67, // γ gamma
        0x03B7 => 0x68, // η eta
        0x03B9 => 0x69, // ι iota
        0x03BA => 0x6B, // κ kappa
        0x03BB => 0x6C, // λ lambda
        0x03BC => 0x6D, // μ mu
        0x03BD => 0x6E, // ν nu
        0x03BF => 0x6F, // ο omicron
        0x03C0 => 0x70, // π pi
        0x03B8 => 0x71, // θ theta
        0x03C1 => 0x72, // ρ rho
        0x03C3 => 0x73, // σ sigma
        0x03C4 => 0x74, // τ tau
        0x03C5 => 0x75, // υ upsilon
        0x03C9 => 0x77, // ω omega
        0x03BE => 0x78, // ξ xi
        0x03C8 => 0x79, // ψ psi
        0x03B6 => 0x7A, // ζ zeta
        // Uppercase Greek letters
        0x0391 => 0x41, // Α Alpha
        0x0392 => 0x42, // Β Beta
        0x0393 => 0x47, // Γ Gamma
        0x0394 => 0x44, // Δ Delta
        0x0395 => 0x45, // Ε Epsilon
        0x0396 => 0x5A, // Ζ Zeta
        0x0397 => 0x48, // Η Eta
        0x0398 => 0x51, // Θ Theta
        0x0399 => 0x49, // Ι Iota
        0x039A => 0x4B, // Κ Kappa
        0x039B => 0x4C, // Λ Lambda
        0x039C => 0x4D, // Μ Mu
        0x039D => 0x4E, // Ν Nu
        0x039E => 0x58, // Ξ Xi
        0x039F => 0x4F, // Ο Omicron
        0x03A0 => 0x50, // Π Pi
        0x03A1 => 0x52, // Ρ Rho
        0x03A3 => 0x53, // Σ Sigma
        0x03A4 => 0x54, // Τ Tau
        0x03A5 => 0x55, // Υ Upsilon
        0x03A6 => 0x46, // Φ Phi
        0x03A7 => 0x43, // Χ Chi
        0x03A8 => 0x59, // Ψ Psi
        0x03A9 => 0x57, // Ω Omega
        // Math operators and relations
        0x2264 => 0xA3, // ≤ lessequal
        0x2265 => 0xB3, // ≥ greaterequal
        0x2260 => 0xB9, // ≠ notequal
        0x2248 => 0xBB, // ≈ approxequal
        0x2261 => 0xBA, // ≡ equivalence
        0x00D7 => 0xB4, // × multiply
        0x00F7 => 0xB8, // ÷ divide
        0x00B1 => 0xB1, // ± plusminus
        0x2213 => 0xB1, // ∓ minusplus (approximate as ±)
        0x00B7 => 0xD7, // · periodcentered/bullet
        0x2022 => 0xB7, // • bullet
        0x2208 => 0xCE, // ∈ element
        0x2209 => 0xCF, // ∉ notelement
        0x2282 => 0xCC, // ⊂ propersubset
        0x2283 => 0xC9, // ⊃ propersuperset
        0x2286 => 0xCD, // ⊆ reflexsubset
        0x2287 => 0xCA, // ⊇ reflexsuperset
        0x222A => 0xC8, // ∪ union
        0x2229 => 0xC7, // ∩ intersection
        0x2200 => 0x22, // ∀ universal
        0x2203 => 0x24, // ∃ existential
        0x2207 => 0xD1, // ∇ nabla/gradient
        0x2202 => 0xB6, // ∂ partialdiff
        0x221E => 0xA5, // ∞ infinity
        0x221A => 0xD6, // √ radical
        0x2205 => 0xC6, // ∅ emptyset
        0x2220 => 0xD0, // ∠ angle
        // Arrows
        0x2190 => 0xAC, // ← arrowleft
        0x2191 => 0xAD, // ↑ arrowup
        0x2192 => 0xAE, // → arrowright
        0x2193 => 0xAF, // ↓ arrowdown
        0x2194 => 0xAB, // ↔ arrowboth
        0x21D0 => 0xDC, // ⇐ arrowdblleft
        0x21D1 => 0xDD, // ⇑ arrowdblup
        0x21D2 => 0xDE, // ⇒ arrowdblright
        0x21D3 => 0xDF, // ⇓ arrowdbldown
        0x21D4 => 0xDB, // ⇔ arrowdblboth
        // Large operators
        0x2211 => 0xE5, // ∑ summation
        0x220F => 0xD5, // ∏ product
        0x222B => 0xF2, // ∫ integral
        // Miscellaneous
        0x2032 => 0xA2, // ′ prime
        0x2026 => 0xBC, // … ellipsis
        0x00B0 => 0xB0, // ° degree
        // Delimiters
        0x27E8 => 0xE1, // ⟨ angleleft
        0x27E9 => 0xF1, // ⟩ angleright
        0x2329 => 0xE1, // 〈 angleleft (old)
        0x232A => 0xF1, // 〉 angleright (old)
        0x230A => 0xEB, // ⌊ lfloor (bracketleftbt)
        0x230B => 0xFB, // ⌋ rfloor (bracketrightbt)
        0x2308 => 0xE9, // ⌈ lceil (bracketlefttp)
        0x2309 => 0xF9, // ⌉ rceil (bracketrighttp)
        // Additional operators
        0x2295 => 0xC5, // ⊕ circleplus
        0x2297 => 0xC4, // ⊗ circlemultiply
        0x2227 => 0xD9, // ∧ logicaland
        0x2228 => 0xDA, // ∨ logicalor
        0x2245 => 0x40, // ≅ congruent
        0x221D => 0xB5, // ∝ proportional
        0x22A5 => 0x5E, // ⊥ perpendicular
        0x2225 => 0xBD, // ∥ parallel (use bar2)
        0x21A6 => 0xAE, // ↦ mapsto (approximate as →)
        0x21AA => 0xAE, // ↪ hookrightarrow (approximate as →)
        0x2243 => 0x40, // ≃ simeq (approximate as ≅)
        _ => return None,
    })
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
