#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    pub const RED: Color = Color { r: 255, g: 0, b: 0 };
    pub const GREEN: Color = Color { r: 0, g: 128, b: 0 };
    pub const BLUE: Color = Color { r: 0, g: 0, b: 255 };
    pub const CYAN: Color = Color { r: 0, g: 255, b: 255 };
    pub const MAGENTA: Color = Color { r: 255, g: 0, b: 255 };
    pub const YELLOW: Color = Color { r: 255, g: 255, b: 0 };
    pub const GRAY: Color = Color { r: 128, g: 128, b: 128 };
    pub const LIGHT_GRAY: Color = Color { r: 212, g: 212, b: 212 };
    pub const DARK_GRAY: Color = Color { r: 84, g: 84, b: 84 };
    pub const ORANGE: Color = Color { r: 255, g: 166, b: 0 };
    pub const PURPLE: Color = Color { r: 128, g: 0, b: 128 };
    pub const BROWN: Color = Color { r: 166, g: 41, b: 41 };

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color {
            r: (r * 255.0 + 0.5) as u8,
            g: (g * 255.0 + 0.5) as u8,
            b: (b * 255.0 + 0.5) as u8,
        }
    }

    pub fn from_rgb_u8(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Convert to f32 for PDF output
    #[inline(always)]
    pub fn r_f32(self) -> f32 { self.r as f32 / 255.0 }
    #[inline(always)]
    pub fn g_f32(self) -> f32 { self.g as f32 / 255.0 }
    #[inline(always)]
    pub fn b_f32(self) -> f32 { self.b as f32 / 255.0 }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color { r, g, b })
        } else {
            None
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        // Handle xcolor !mix syntax (e.g., "blue!50", "red!30!white")
        if name.contains('!') {
            return Self::from_mix_spec(name);
        }
        match name.to_lowercase().as_str() {
            "black" => Some(Color::BLACK),
            "white" => Some(Color::WHITE),
            "red" => Some(Color::RED),
            "green" => Some(Color::GREEN),
            "blue" => Some(Color::BLUE),
            "cyan" => Some(Color::CYAN),
            "magenta" => Some(Color::MAGENTA),
            "yellow" => Some(Color::YELLOW),
            "gray" | "grey" => Some(Color::GRAY),
            "lightgray" | "lightgrey" => Some(Color::LIGHT_GRAY),
            "darkgray" | "darkgrey" => Some(Color::DARK_GRAY),
            "orange" => Some(Color::ORANGE),
            "purple" | "violet" => Some(Color::PURPLE),
            "brown" => Some(Color::BROWN),
            // dvipsnames
            "navyblue" => Some(Color::from_rgb_u8(0, 0, 128)),
            "royalblue" => Some(Color::from_rgb_u8(0, 35, 102)),
            "cornflowerblue" => Some(Color::from_rgb_u8(89, 112, 193)),
            "midnightblue" => Some(Color::from_rgb_u8(0, 34, 102)),
            "processblue" => Some(Color::from_rgb_u8(10, 173, 234)),
            "cerulean" => Some(Color::from_rgb_u8(0, 162, 227)),
            "teal" | "tealblue" => Some(Color::from_rgb_u8(0, 128, 128)),
            "aquamarine" => Some(Color::from_rgb_u8(46, 204, 186)),
            "forestgreen" => Some(Color::from_rgb_u8(0, 155, 85)),
            "olivegreen" => Some(Color::from_rgb_u8(0, 153, 0)),
            "limegreen" | "lime" => Some(Color::from_rgb_u8(128, 222, 0)),
            "yellowgreen" => Some(Color::from_rgb_u8(142, 176, 0)),
            "goldenrod" => Some(Color::from_rgb_u8(255, 223, 66)),
            "dandelion" => Some(Color::from_rgb_u8(255, 181, 41)),
            "apricot" => Some(Color::from_rgb_u8(255, 173, 122)),
            "peach" => Some(Color::from_rgb_u8(255, 127, 69)),
            "melon" => Some(Color::from_rgb_u8(255, 137, 127)),
            "yelloworange" => Some(Color::from_rgb_u8(255, 174, 0)),
            "burntorange" => Some(Color::from_rgb_u8(255, 125, 0)),
            "bittersweet" => Some(Color::from_rgb_u8(193, 1, 0)),
            "redorange" => Some(Color::from_rgb_u8(255, 59, 33)),
            "mahogany" => Some(Color::from_rgb_u8(166, 25, 22)),
            "maroon" => Some(Color::from_rgb_u8(173, 52, 30)),
            "brickred" => Some(Color::from_rgb_u8(182, 50, 28)),
            "orangered" => Some(Color::from_rgb_u8(255, 0, 127)),
            "rubinered" => Some(Color::from_rgb_u8(255, 0, 222)),
            "wildstrawberry" => Some(Color::from_rgb_u8(255, 10, 156)),
            "salmon" => Some(Color::from_rgb_u8(255, 120, 158)),
            "carnationpink" => Some(Color::from_rgb_u8(255, 94, 255)),
            "pink" => Some(Color::from_rgb_u8(255, 182, 193)),
            "plum" => Some(Color::from_rgb_u8(128, 0, 255)),
            "orchid" => Some(Color::from_rgb_u8(173, 91, 173)),
            "lavender" => Some(Color::from_rgb_u8(255, 133, 255)),
            "thistle" => Some(Color::from_rgb_u8(224, 146, 255)),
            "darkred" | "darkviolet" => Some(Color::from_rgb_u8(138, 0, 0)),
            "olive" | "olivegreen" => Some(Color::from_rgb_u8(0, 153, 0)),
            _ => None,
        }
    }

    /// Parse xcolor mix specification like "blue!50", "red!30!white"
    fn from_mix_spec(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split('!').collect();
        if parts.len() < 2 { return None; }

        let base = Color::from_name(parts[0])?;
        let pct: f32 = parts[1].parse().unwrap_or(50.0) / 100.0;
        let other = if parts.len() >= 3 {
            Color::from_name(parts[2]).unwrap_or(Color::WHITE)
        } else {
            Color::WHITE
        };

        Some(Color::from_rgb_u8(
            (base.r as f32 * pct + other.r as f32 * (1.0 - pct) + 0.5) as u8,
            (base.g as f32 * pct + other.g as f32 * (1.0 - pct) + 0.5) as u8,
            (base.b as f32 * pct + other.b as f32 * (1.0 - pct) + 0.5) as u8,
        ))
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}
