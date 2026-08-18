//! 2D graphics primitive rendering engine for Vanta Desktop.

use crate::font;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xff }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const BLACK: Self = Self::rgb(0x00, 0x00, 0x00);
    pub const WHITE: Self = Self::rgb(0xff, 0xff, 0xff);
    pub const DESKTOP_BG: Self = Self::rgb(0x0f, 0x14, 0x1c);
    pub const TOPBAR_BG: Self = Self::rgb(0x1a, 0x20, 0x2c);
    pub const TOPBAR_BORDER: Self = Self::rgb(0x2d, 0x37, 0x48);
    pub const WIN_TITLE_BG: Self = Self::rgb(0x1e, 0x25, 0x33);
    pub const WIN_BODY_BG: Self = Self::rgb(0x13, 0x17, 0x22);
    pub const WIN_BORDER: Self = Self::rgb(0x3e, 0x4c, 0x63);
    pub const TEXT_PRIMARY: Self = Self::rgb(0xe2, 0xe8, 0xf0);
    pub const TEXT_MUTED: Self = Self::rgb(0x94, 0xa3, 0xb8);
    pub const BLUE_ACCENT: Self = Self::rgb(0x3b, 0x82, 0xf6);
    pub const GREEN_ACCENT: Self = Self::rgb(0x10, 0xb9, 0x81);
    pub const RED_ACCENT: Self = Self::rgb(0xef, 0x44, 0x44);
    pub const YELLOW_ACCENT: Self = Self::rgb(0xf5, 0x9e, 0x0b);
    pub const TERM_BG: Self = Self::rgb(0x0a, 0x0d, 0x14);
    pub const TERM_FG: Self = Self::rgb(0x4a, 0xde, 0x80);
}

pub struct Canvas<'a> {
    pub buffer: &'a mut [u8],
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
}

impl<'a> Canvas<'a> {
    pub fn new(buffer: &'a mut [u8], width: usize, height: usize) -> Self {
        let pitch = width * 4;
        Self {
            buffer,
            width,
            height,
            pitch,
        }
    }

    #[inline]
    pub fn put_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let off = y * self.pitch + x * 4;
        if off + 3 < self.buffer.len() {
            self.buffer[off] = color.b;
            self.buffer[off + 1] = color.g;
            self.buffer[off + 2] = color.r;
            self.buffer[off + 3] = color.a;
        }
    }

    pub fn clear(&mut self, color: Color) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        let max_x = (x + w).min(self.width);
        let max_y = (y + h).min(self.height);
        for row in y..max_y {
            for col in x..max_x {
                self.put_pixel(col, row, color);
            }
        }
    }

    pub fn draw_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        if w == 0 || h == 0 {
            return;
        }
        let max_x = (x + w).min(self.width);
        let max_y = (y + h).min(self.height);
        for col in x..max_x {
            self.put_pixel(col, y, color);
            if max_y > y {
                self.put_pixel(col, max_y - 1, color);
            }
        }
        for row in y..max_y {
            self.put_pixel(x, row, color);
            if max_x > x {
                self.put_pixel(max_x - 1, row, color);
            }
        }
    }

    pub fn draw_char(&mut self, x: usize, y: usize, c: char, fg: Color, scale: usize) {
        let glyph = font::get_glyph(c);
        for row in 0..8 {
            let byte = glyph[row];
            for col in 0..8 {
                if byte & (1 << (7 - col)) != 0 {
                    if scale == 1 {
                        self.put_pixel(x + col, y + row, fg);
                    } else {
                        self.fill_rect(x + col * scale, y + row * scale, scale, scale, fg);
                    }
                }
            }
        }
    }

    pub fn draw_text(&mut self, x: usize, y: usize, text: &str, fg: Color, scale: usize) {
        let mut cur_x = x;
        let mut cur_y = y;
        let char_w = 8 * scale;
        let line_h = 10 * scale;
        for c in text.chars() {
            if c == '\n' {
                cur_x = x;
                cur_y += line_h;
                continue;
            }
            if cur_x + char_w <= self.width {
                self.draw_char(cur_x, cur_y, c, fg, scale);
            }
            cur_x += char_w;
        }
    }

    pub fn draw_window(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        title: &str,
        active: bool,
    ) {
        let title_h = 24;
        // Outer Shadow
        self.fill_rect(x + 4, y + 4, w, h, Color::rgb(0x05, 0x07, 0x0a));
        // Window Body
        self.fill_rect(x, y, w, h, Color::WIN_BODY_BG);
        // Window Title Bar
        let title_bg = if active {
            Color::WIN_TITLE_BG
        } else {
            Color::rgb(0x16, 0x1a, 0x24)
        };
        self.fill_rect(x, y, w, title_h, title_bg);
        // Window Border
        let border_col = if active {
            Color::WIN_BORDER
        } else {
            Color::rgb(0x28, 0x32, 0x42)
        };
        self.draw_rect(x, y, w, h, border_col);
        self.draw_rect(x, y, w, title_h, border_col);

        // Mac/Modern Style Window Control Dots
        self.fill_rect(x + 8, y + 7, 10, 10, Color::RED_ACCENT); // Close [X]
        self.fill_rect(x + 22, y + 7, 10, 10, Color::YELLOW_ACCENT); // Min [-]
        self.fill_rect(x + 36, y + 7, 10, 10, Color::GREEN_ACCENT); // Max [+]

        // Window Title Text
        self.draw_text(x + 54, y + 8, title, Color::TEXT_PRIMARY, 1);
    }

    pub fn draw_button(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        label: &str,
        highlighted: bool,
    ) {
        let bg = if highlighted {
            Color::BLUE_ACCENT
        } else {
            Color::rgb(0x25, 0x2f, 0x40)
        };
        self.fill_rect(x, y, w, h, bg);
        self.draw_rect(x, y, w, h, Color::rgb(0x4a, 0x5d, 0x7c));
        let text_x = x + 8;
        let text_y = y + (h.saturating_sub(8)) / 2;
        self.draw_text(text_x, text_y, label, Color::WHITE, 1);
    }

    pub fn draw_progress_bar(&mut self, x: usize, y: usize, w: usize, h: usize, pct: usize, fill_color: Color) {
        self.fill_rect(x, y, w, h, Color::rgb(0x1a, 0x22, 0x30));
        self.draw_rect(x, y, w, h, Color::rgb(0x3b, 0x4a, 0x63));
        let filled_w = ((w.saturating_sub(4)) * pct.min(100)) / 100;
        if filled_w > 0 {
            self.fill_rect(x + 2, y + 2, filled_w, h.saturating_sub(4), fill_color);
        }
    }

    pub fn draw_cursor(&mut self, x: usize, y: usize) {
        let cursor_shape: [(i32, i32); 16] = [
            (0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7),
            (0, 8), (0, 9), (0, 10), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5),
        ];
        // Black outline
        for (dx, dy) in &cursor_shape {
            let px = (x as i32 + dx) as usize;
            let py = (y as i32 + dy) as usize;
            self.put_pixel(px + 1, py + 1, Color::BLACK);
        }
        // White cursor body
        for (dx, dy) in &cursor_shape {
            let px = (x as i32 + dx) as usize;
            let py = (y as i32 + dy) as usize;
            self.put_pixel(px, py, Color::WHITE);
        }
    }
}
