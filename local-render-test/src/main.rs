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

use digital_garden_visitor_counter::counter::render_separated_number;
use minifb::{Key, Window, WindowOptions};
use std::time::Duration;

fn main() {
    let render = render_separated_number(1, 10);
    let mut window = Window::new(
        "Test",
        render.width,
        render.height,
        WindowOptions::default(),
    )
    .unwrap();

    // Render out to a file just to test PNG output.
    {
        let render = render_separated_number(1_234_567_890, 10);
        std::fs::write("test-output.png", render.to_png_bytes().unwrap()).unwrap();
    }

    let mut num = 0;

    window.limit_update_rate(Some(Duration::from_millis(1000 / 60)));
    while window.is_open() && !window.is_key_down(Key::Escape) {
        let render = render_separated_number(num, 10);
        let pixels = &render.pixels;
        let mut buffer: Vec<u32> = vec![0; render.width * render.height];
        for (i, val) in buffer.iter_mut().enumerate() {
            *val = (pixels[i * 4 + 1] as u32) << 16
                | (pixels[i * 4 + 2] as u32) << 8
                | pixels[i * 4 + 3] as u32;
        }

        window
            .update_with_buffer(&buffer, render.width, render.height)
            .unwrap();
        num += 12321;
    }
}
