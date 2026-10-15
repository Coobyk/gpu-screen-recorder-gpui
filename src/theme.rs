//! A small, dark, neutral theme shared by every widget.

use gpui::Rgba;

/// `gpui::rgb` is not a `const fn`, so the palette is built with a const
/// helper instead of calling it from each constant.
const fn color(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

pub const BG: Rgba = color(0x1e1f22);
pub const PANEL: Rgba = color(0x2b2d31);
pub const PANEL_ALT: Rgba = color(0x313338);
pub const FIELD: Rgba = color(0x1a1b1e);
pub const BORDER: Rgba = color(0x3f4147);
pub const TEXT: Rgba = color(0xe6e6e6);
pub const TEXT_MUTED: Rgba = color(0x9aa0a6);
pub const ACCENT: Rgba = color(0x5865f2);
pub const ACCENT_HOVER: Rgba = color(0x6b76f5);
pub const DANGER: Rgba = color(0xd83c3c);
pub const DANGER_HOVER: Rgba = color(0xe35252);
