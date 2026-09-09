//! Redox-compatible Orbclient 2D graphics engine and window primitives for Vanta OS.


use crate::font;

/// 32-bit RGBA / BGRA Color representation with alpha blending.
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

    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as u8,
            g: ((hex >> 8) & 0xff) as u8,
            b: (hex & 0xff) as u8,
            a: 0xff,
        }
    }

    /// Alpha-blend src over self (destination).
    #[inline]
    pub fn blend(self, src: Color) -> Self {
        if src.a == 0xff {
            src
        } else if src.a == 0 {
            self
        } else {
            let sa = src.a as u32;
            let da = (255 - sa) * (self.a as u32) / 255;
            let out_a = sa + da;
            if out_a == 0 {
                return Color::rgba(0, 0, 0, 0);
            }
            let r = ((src.r as u32 * sa + self.r as u32 * da) / out_a) as u8;
            let g = ((src.g as u32 * sa + self.g as u32 * da) / out_a) as u8;
            let b = ((src.b as u32 * sa + self.b as u32 * da) / out_a) as u8;
            Color::rgba(r, g, b, out_a as u8)
        }
    }

    // Redox Orbital Standard Palette
    pub const BLACK: Self = Self::rgb(0x00, 0x00, 0x00);
    pub const WHITE: Self = Self::rgb(0xff, 0xff, 0xff);
    pub const TRANSPARENT: Self = Self::rgba(0x00, 0x00, 0x00, 0x00);

    pub const ORBITAL_BG: Self = Self::rgb(0x1a, 0x22, 0x30);
    pub const ORBITAL_PANEL: Self = Self::rgb(0x12, 0x18, 0x24);
    pub const ORBITAL_PANEL_BORDER: Self = Self::rgb(0x2d, 0x3b, 0x52);

    pub const WIN_TITLEBAR_ACTIVE: Self = Self::rgb(0x2b, 0x38, 0x4e);
    pub const WIN_TITLEBAR_INACTIVE: Self = Self::rgb(0x1a, 0x20, 0x2c);
    pub const WIN_BODY_BG: Self = Self::rgb(0x0f, 0x14, 0x1d);
    pub const WIN_BORDER: Self = Self::rgb(0x3a, 0x4c, 0x68);

    pub const BTN_CLOSE: Self = Self::rgb(0xef, 0x44, 0x44);
    pub const BTN_MIN: Self = Self::rgb(0xf5, 0x9e, 0x0b);
    pub const BTN_MAX: Self = Self::rgb(0x10, 0xb9, 0x81);

    pub const TEXT_PRIMARY: Self = Self::rgb(0xf1, 0xf5, 0xf9);
    pub const TEXT_MUTED: Self = Self::rgb(0x94, 0xa3, 0xb8);
    pub const TEXT_DARK: Self = Self::rgb(0x33, 0x41, 0x55);

    pub const ACCENT_BLUE: Self = Self::rgb(0x3b, 0x82, 0xf6);
    pub const ACCENT_CYAN: Self = Self::rgb(0x06, 0xb6, 0xd4);
    pub const ACCENT_GREEN: Self = Self::rgb(0x22, 0xc5, 0x5e);
    pub const ACCENT_PURPLE: Self = Self::rgb(0x8b, 0x5c, 0xf6);

    pub const TERM_BG: Self = Self::rgb(0x08, 0x0c, 0x14);
    pub const TERM_FG: Self = Self::rgb(0x38, 0xbd, 0xf8);
}

/// Redox-compatible Mouse Event
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

/// Redox-compatible Keyboard Event
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyEvent {
    pub character: char,
    pub scancode: u8,
    pub pressed: bool,
}

/// Orbital System Event
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Mouse(MouseEvent),
    Key(KeyEvent),
    WindowMoved { x: i32, y: i32 },
    WindowClosed,
    None,
}

/// A 2D Surface/Canvas for rendering
pub struct Surface<'a> {
    pub data: &'a mut [u8],
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
}

impl<'a> Surface<'a> {
    pub fn new(data: &'a mut [u8], width: usize, height: usize) -> Self {
        let pitch = width * 4;
        Self {
            data,
            width,
            height,
            pitch,
        }
    }

    #[inline]
    pub fn pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let off = y * self.pitch + x * 4;
        if off + 3 < self.data.len() {
            if color.a == 0xff {
                self.data[off] = color.b;
                self.data[off + 1] = color.g;
                self.data[off + 2] = color.r;
                self.data[off + 3] = 0xff;
            } else if color.a > 0 {
                let existing = Color::rgba(
                    self.data[off + 2],
                    self.data[off + 1],
                    self.data[off],
                    self.data[off + 3],
                );
                let blended = existing.blend(color);
                self.data[off] = blended.b;
                self.data[off + 1] = blended.g;
                self.data[off + 2] = blended.r;
                self.data[off + 3] = blended.a;
            }
        }
    }

    pub fn clear(&mut self, color: Color) {
        self.rect(0, 0, self.width, self.height, color);
    }

    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        let max_x = (x + w).min(self.width);
        let max_y = (y + h).min(self.height);
        for row in y..max_y {
            for col in x..max_x {
                self.pixel(col, row, color);
            }
        }
    }

    pub fn outline_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        if w == 0 || h == 0 {
            return;
        }
        let max_x = (x + w).min(self.width);
        let max_y = (y + h).min(self.height);
        for col in x..max_x {
            self.pixel(col, y, color);
            if max_y > y {
                self.pixel(col, max_y - 1, color);
            }
        }
        for row in y..max_y {
            self.pixel(x, row, color);
            if max_x > x {
                self.pixel(max_x - 1, row, color);
            }
        }
    }

    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;

        loop {
            if cx >= 0 && cy >= 0 {
                self.pixel(cx as usize, cy as usize, color);
            }
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    pub fn char(&mut self, x: usize, y: usize, c: char, fg: Color, scale: usize) {
        let glyph = font::get_glyph(c);
        for row in 0..8 {
            let byte = glyph[row];
            for col in 0..8 {
                if byte & (1 << (7 - col)) != 0 {
                    if scale == 1 {
                        self.pixel(x + col, y + row, fg);
                    } else {
                        self.rect(x + col * scale, y + row * scale, scale, scale, fg);
                    }
                }
            }
        }
    }

    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: Color, scale: usize) {
        let mut cur_x = x;
        let mut cur_y = y;
        let char_w = 8 * scale;
        let line_h = 10 * scale;
        for c in s.chars() {
            if c == '\n' {
                cur_x = x;
                cur_y += line_h;
                continue;
            }
            if cur_x + char_w <= self.width {
                self.char(cur_x, cur_y, c, fg, scale);
            }
            cur_x += char_w;
        }
    }

    /// Blit an image/surface at (dst_x, dst_y)
    pub fn blit(&mut self, dst_x: i32, dst_y: i32, src: &[u8], src_w: usize, src_h: usize) {
        for row in 0..src_h {
            let target_y = dst_y + row as i32;
            if target_y < 0 || target_y >= self.height as i32 {
                continue;
            }
            for col in 0..src_w {
                let target_x = dst_x + col as i32;
                if target_x < 0 || target_x >= self.width as i32 {
                    continue;
                }
                let src_off = (row * src_w + col) * 4;
                if src_off + 3 < src.len() {
                    let color = Color::rgba(
                        src[src_off + 2],
                        src[src_off + 1],
                        src[src_off],
                        src[src_off + 3],
                    );
                    self.pixel(target_x as usize, target_y as usize, color);
                }
            }
        }
    }
}

/// Titlebar metrics
pub const TITLEBAR_HEIGHT: i32 = 24;
pub const CLOSE_BTN_RADIUS: i32 = 6;
pub const CLOSE_BTN_X: i32 = 12;
pub const CLOSE_BTN_Y: i32 = 12;

/// Hit test titlebar for dragging
pub fn is_titlebar_hit(win_x: i32, win_y: i32, win_w: i32, mouse_x: i32, mouse_y: i32) -> bool {
    mouse_x >= win_x && mouse_x < win_x + win_w && mouse_y >= win_y && mouse_y < win_y + TITLEBAR_HEIGHT
}

/// Hit test close button
pub fn is_close_button_hit(win_x: i32, win_y: i32, mouse_x: i32, mouse_y: i32) -> bool {
    let btn_x = win_x + CLOSE_BTN_X;
    let btn_y = win_y + CLOSE_BTN_Y;
    let dx = mouse_x - btn_x;
    let dy = mouse_y - btn_y;
    dx * dx + dy * dy <= CLOSE_BTN_RADIUS * CLOSE_BTN_RADIUS
}

/// Hit test whole window
pub fn is_window_hit(win_x: i32, win_y: i32, win_w: i32, win_h: i32, mouse_x: i32, mouse_y: i32) -> bool {
    mouse_x >= win_x && mouse_x < win_x + win_w && mouse_y >= win_y && mouse_y < win_y + win_h
}

/// Draw mouse cursor arrow with high-contrast shadow and white body
pub fn draw_cursor(surface: &mut Surface<'_>, x: usize, y: usize) {
    const CURSOR_BODY: [(i32, i32); 19] = [
        (0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (0, 8), (0, 9), (0, 10), (0, 11),
        (1, 1), (1, 2), (2, 2), (2, 3), (3, 3), (4, 4), (5, 5),
    ];
    const CURSOR_OUTLINE: [(i32, i32); 26] = [
        (-1, 0), (-1, 1), (-1, 2), (-1, 3), (-1, 4), (-1, 5), (-1, 6), (-1, 7), (-1, 8), (-1, 9), (-1, 10), (-1, 11), (-1, 12),
        (0, 12), (1, 11), (2, 10), (3, 9), (4, 8), (5, 7), (6, 6), (5, 5), (4, 4), (3, 3), (2, 2), (1, 1), (0, -1),
    ];

    // Shadow
    for &(dx, dy) in &CURSOR_OUTLINE {
        let px = (x as i32 + dx + 1) as usize;
        let py = (y as i32 + dy + 1) as usize;
        surface.pixel(px, py, Color::rgba(0, 0, 0, 180));
    }
    // Black border
    for &(dx, dy) in &CURSOR_OUTLINE {
        let px = (x as i32 + dx) as usize;
        let py = (y as i32 + dy) as usize;
        surface.pixel(px, py, Color::BLACK);
    }
    // White interior
    for &(dx, dy) in &CURSOR_BODY {
        let px = (x as i32 + dx) as usize;
        let py = (y as i32 + dy) as usize;
        surface.pixel(px, py, Color::WHITE);
    }
}
