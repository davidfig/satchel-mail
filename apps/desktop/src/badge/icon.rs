//! Draws the count for Windows, whose taskbar overlay takes an image rather
//! than a number: white digits on a red circle, as RGBA pixels.

/// Width and height in pixels. Windows shows it at about 16 points, so digits
/// are blocky and large to stay readable.
pub const SIZE: u32 = 32;

const RED: [u8; 3] = [0xD3, 0x2F, 0x2F];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// A 3x5 pixel character, one row per byte, leftmost pixel in the highest bit.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0b000, 0b010, 0b111, 0b010, 0b000],
    }
}

/// What the icon says: the count, up to what fits.
fn label(count: i64) -> String {
    if count > 99 { "99+".to_string() } else { count.to_string() }
}

/// RGBA pixels, `SIZE` by `SIZE`, for a count above zero.
pub fn render(count: i64) -> Vec<u8> {
    let mut rgba = vec![0; (SIZE * SIZE * 4) as usize];
    let center = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = (x as f32 + 0.5 - center).hypot(y as f32 + 0.5 - center);
            // Fades over the last pixel, so the edge isn't jagged.
            let alpha = (center - distance + 0.5).clamp(0.0, 1.0);
            put(&mut rgba, x, y, RED, (alpha * 255.0) as u8);
        }
    }
    let text = label(count);
    let chars = text.chars().count() as u32;
    let scale = match chars {
        1 => 4,
        2 => 3,
        _ => 2,
    };
    let width = (chars * 3 + chars - 1) * scale;
    let (left, top) = ((SIZE - width) / 2, (SIZE - 5 * scale) / 2);
    for (i, c) in text.chars().enumerate() {
        let start = left + i as u32 * 4 * scale;
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for y in 0..scale {
                        for x in 0..scale {
                            put(&mut rgba, start + col * scale + x, top + row as u32 * scale + y, WHITE, 255);
                        }
                    }
                }
            }
        }
    }
    rgba
}

fn put(rgba: &mut [u8], x: u32, y: u32, [r, g, b]: [u8; 3], alpha: u8) {
    let at = ((y * SIZE + x) * 4) as usize;
    rgba[at..at + 4].copy_from_slice(&[r, g, b, alpha]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], x: u32, y: u32) -> &[u8] {
        let at = ((y * SIZE + x) * 4) as usize;
        &rgba[at..at + 4]
    }

    fn white_bounds(rgba: &[u8]) -> (u32, u32, u32, u32) {
        let (mut left, mut top, mut right, mut bottom) = (SIZE, SIZE, 0, 0);
        for y in 0..SIZE {
            for x in 0..SIZE {
                if pixel(rgba, x, y) == [0xFF, 0xFF, 0xFF, 0xFF] {
                    (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x), bottom.max(y));
                }
            }
        }
        (left, top, right, bottom)
    }

    #[test]
    fn caps_the_label_at_what_fits() {
        assert_eq!(label(7), "7");
        assert_eq!(label(99), "99");
        assert_eq!(label(100), "99+");
        assert_eq!(label(250_000), "99+");
    }

    #[test]
    fn is_an_opaque_circle_with_clear_corners() {
        let rgba = render(1);
        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(pixel(&rgba, x, y)[3], 0, "corner {x},{y}");
        }
        assert_eq!(pixel(&rgba, 3, SIZE / 2), [RED[0], RED[1], RED[2], 255]);
    }

    #[test]
    fn digits_stay_inside_the_circle_and_centered() {
        for count in [1, 8, 10, 88, 99, 100] {
            let rgba = render(count);
            let (left, top, right, bottom) = white_bounds(&rgba);
            assert!(left <= right, "{count} drew no digits");
            let half = SIZE as f32 / 2.0;
            for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
                let outside = (x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half) >= half;
                assert!(!outside, "{count} leaves the circle at {x},{y}");
            }
            assert!((left as i32 - (SIZE - 1 - right) as i32).abs() <= 1, "{count} is off center");
        }
    }

    #[test]
    fn every_count_looks_different() {
        let images: Vec<_> = (1..=100).map(render).collect();
        for (i, a) in images.iter().enumerate() {
            assert!(images[i + 1..].iter().all(|b| a != b), "{} repeats", i + 1);
        }
    }
}
