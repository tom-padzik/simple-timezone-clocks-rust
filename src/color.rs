/// sRGB relative luminance, 0 for black and 1 for white.
pub fn relative_luminance(rgb: [u8; 3]) -> f32 {
    fn channel(value: u8) -> f32 {
        let s = value as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

/// Dark ink stays readable on this background.
pub fn prefers_dark_text(rgb: [u8; 3]) -> bool {
    relative_luminance(rgb) > 0.179
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_picks_readable_ink() {
        assert!(prefers_dark_text([255, 255, 255]));
        assert!(prefers_dark_text([255, 220, 0]));
        assert!(!prefers_dark_text([0, 0, 0]));
        assert!(!prefers_dark_text([10, 20, 80]));
    }
}
