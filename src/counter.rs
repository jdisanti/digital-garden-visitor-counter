// Digital garden visitor counter
// A simple visitor counter for digital gardens that runs as an AWS Lambda function.
// Copyright (C) 2023 John DiSanti.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use std::mem::size_of;

pub struct Render {
    pub width: usize,
    pub height: usize,
    // In 32-bit RGBA format
    pub pixels: Vec<u8>,
}

impl Render {
    /// Convert this render to an in-memory PNG image.
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, png::EncodingError> {
        // Guestimate the size of the PNG and pre-allocate a buffer.
        let mut png: Vec<u8> = Vec::with_capacity(self.pixels.len());

        let mut encoder = png::Encoder::new(&mut png, self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut encoder = encoder.write_header()?;
        encoder.write_image_data(&self.pixels)?;
        encoder.finish()?;
        Ok(png)
    }
}

/// Render a number with a space between every 3 digits.
///
/// The `reserve_width` is a minimum width of the image in number of digits.
/// This is useful if you want the image to always be the same width.
pub fn render_separated_number(number: usize, reserve_width: usize) -> Render {
    let number = number.to_string();

    // Spacing between groups of digits in pixels.
    let group_spacing = 3;

    // Split the number into groups of 3 digits, starting from the right.
    let groups: Vec<_> = number
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|b| std::str::from_utf8(b).unwrap())
        .collect();

    // Calculate the image size and allocate memory.
    let (width, height) = font::text_size(number.len().max(reserve_width));
    let width = 2 + width + group_spacing * (reserve_width / 3).max(number.len() / 3);
    let height = 2 + height; // 2px padding total
    let mut pixels = vec![0; width * height * size_of::<u32>()];

    // Calculate the very first X offset such that the number ends up right-aligned.
    let mut x = 1 // 1px padding on the left
        // First, calculate offset in character widths
        + Some(reserve_width as isize - number.len() as isize)
            .filter(|&n| n > 0)
            .map(|n| font::text_size(n as usize).0)
            .unwrap_or(0)
        // Then refine the offset in number of group spaces skipped
        + Some((reserve_width / 3) as isize - (number.len() / 3) as isize)
            .filter(|&n| n > 0)
            .map(|n| n as usize * group_spacing)
            .unwrap_or(0);
    for group in groups {
        font::blit_into(&mut pixels, width, group, x, 1);
        x += font::text_size(group.len()).0 + group_spacing;
    }

    Render {
        width,
        height,
        pixels,
    }
}

mod font {
    use std::mem::size_of;

    pub const GLYPH_WIDTH: usize = 8;
    pub const GLYPH_HEIGHT: usize = 16;
    pub const GLYPH_SIZE: usize = GLYPH_WIDTH * GLYPH_HEIGHT;
    pub const GLYPH_KERN: usize = 1;
    pub const GLYPH_COUNT: usize = 10;

    /// Blit a number into the given buffer.
    pub fn blit_into(
        buffer: &mut [u8],
        buffer_width: usize,
        text: &str,
        offset_x: usize,
        offset_y: usize,
    ) {
        let mut x = offset_x;
        for c in text.chars() {
            if let Some(glyph) = glyph_for_char(c) {
                blit_glyph_into(buffer, buffer_width, glyph, x, offset_y);
                x += GLYPH_WIDTH + GLYPH_KERN;
            }
        }
    }

    fn blit_glyph_into(
        buffer: &mut [u8],
        buffer_width: usize,
        glyph: &[u8; GLYPH_SIZE],
        x: usize,
        y: usize,
    ) {
        for row in 0..GLYPH_HEIGHT {
            for col in 0..GLYPH_WIDTH {
                let dest_index = ((y + row) * buffer_width + (x + col)) * size_of::<u32>();
                match glyph[row * GLYPH_WIDTH + col] {
                    0 => {
                        buffer[dest_index] = 0x00;
                        buffer[dest_index + 1] = 0x00;
                        buffer[dest_index + 2] = 0x00;
                        buffer[dest_index + 3] = 0x00;
                    }
                    _ => {
                        buffer[dest_index] = 0xFF;
                        buffer[dest_index + 1] = 0xFF;
                        buffer[dest_index + 2] = 0xFF;
                        buffer[dest_index + 3] = 0xFF;
                    }
                }
            }
        }
    }

    pub fn glyph_for_char(c: char) -> Option<&'static [u8; GLYPH_SIZE]> {
        if c.is_ascii_digit() {
            let index = c as usize - '0' as usize;
            Some(&GLYPH_BITMAPS[index])
        } else {
            None
        }
    }

    /// Return the bitmap size for the given string length
    pub fn text_size(char_count: usize) -> (usize, usize) {
        (char_count * (GLYPH_WIDTH + GLYPH_KERN), GLYPH_HEIGHT)
    }

    /// Foreground
    const F: u8 = 1;

    #[rustfmt::skip]
    const GLYPH_BITMAPS: [[u8; GLYPH_SIZE]; GLYPH_COUNT] = [
        [ // 0
            0,0,0,F,F,0,0,0,
            0,0,F,F,F,F,0,0,
            0,F,F,0,0,F,F,0,
            0,F,0,0,0,0,F,0,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,F,0,F,F,
            F,F,0,F,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,F,0,0,0,0,F,0,
            0,F,F,0,0,F,F,0,
            0,0,F,F,F,F,0,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 1
            0,0,0,F,F,0,0,0,
            0,0,F,F,F,0,0,0,
            0,F,F,F,F,0,0,0,
            F,F,0,F,F,0,0,0,
            F,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            F,F,F,F,F,F,F,F,
            F,F,F,F,F,F,F,F,
        ],
        [ // 2
            0,0,0,F,F,0,0,0,
            0,F,F,F,F,F,0,0,
            0,F,F,0,0,F,F,0,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,F,F,0,0,
            0,0,0,0,F,F,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,F,F,0,0,0,0,
            0,F,F,F,0,0,0,0,
            F,F,F,F,F,F,F,F,
            F,F,F,F,F,F,F,F,
        ],
        [ // 3
            0,0,0,F,F,0,0,0,
            0,F,F,F,F,F,F,0,
            F,F,F,0,0,F,F,0,
            F,F,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,F,F,0,
            0,0,0,F,F,F,0,0,
            0,0,0,F,F,F,0,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,F,0,0,F,F,0,
            0,F,F,F,F,F,F,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 4
            0,0,0,0,0,F,F,0,
            0,0,0,0,F,F,F,0,
            0,0,0,F,F,F,F,0,
            0,0,0,F,0,F,F,0,
            0,0,F,F,0,F,F,0,
            0,F,F,0,0,F,F,0,
            0,F,F,0,0,F,F,0,
            F,F,0,0,0,F,F,0,
            F,F,F,F,F,F,F,F,
            F,F,F,F,F,F,F,F,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,F,F,F,F,
            0,0,0,0,F,F,F,F,
        ],
        [ // 5
            F,F,F,F,F,F,F,0,
            F,F,F,F,F,F,F,0,
            F,F,0,0,0,0,0,0,
            F,F,0,0,0,0,0,0,
            F,F,0,0,0,0,0,0,
            F,F,0,0,0,0,0,0,
            F,F,0,F,F,F,0,0,
            F,F,F,F,F,F,F,0,
            0,F,0,0,0,F,F,0,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,F,0,0,F,F,0,
            0,F,F,F,F,F,0,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 6
            0,0,0,F,F,0,0,0,
            0,F,F,F,F,F,F,0,
            0,F,F,0,0,F,F,0,
            F,F,0,0,0,0,0,0,
            F,F,0,0,0,0,0,0,
            F,F,0,0,0,0,0,0,
            F,F,0,F,F,0,0,0,
            F,F,F,F,F,F,F,0,
            F,F,F,0,0,F,F,0,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,F,0,0,0,0,F,0,
            0,F,F,0,0,F,F,0,
            0,F,F,F,F,F,F,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 7
            F,F,F,F,F,F,F,F,
            F,F,F,F,F,F,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,F,F,0,
            0,0,0,0,0,F,F,0,
            0,0,0,0,F,F,0,0,
            0,0,0,0,F,F,0,0,
            0,0,F,F,F,F,F,0,
            0,0,F,F,F,F,F,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 8
            0,0,0,F,F,0,0,0,
            0,F,F,F,F,F,F,0,
            0,F,F,0,0,F,F,0,
            0,F,0,0,0,0,F,0,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,F,F,0,0,F,F,0,
            0,F,F,F,F,F,F,0,
            0,0,F,F,F,F,0,0,
            0,F,F,0,0,F,F,0,
            F,F,F,0,0,F,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,F,F,0,0,F,F,0,
            0,F,F,F,F,F,F,0,
            0,0,0,F,F,0,0,0,
        ],
        [ // 9
            0,0,0,F,F,0,0,0,
            0,F,F,F,F,F,F,0,
            0,F,F,0,0,F,F,0,
            0,F,0,0,0,0,F,0,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            F,F,0,0,0,0,F,F,
            0,F,F,0,0,F,F,F,
            0,F,F,F,F,F,F,F,
            0,0,0,F,F,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,0,0,0,0,0,F,F,
            0,F,F,0,0,F,F,0,
            0,F,F,F,F,F,F,0,
            0,0,0,F,F,0,0,0,
        ],
    ];
}
