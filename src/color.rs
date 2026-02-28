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
            _ => None,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}
