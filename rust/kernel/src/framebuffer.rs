use core::fmt;
use limine::framebuffer::Framebuffer;
use noto_sans_mono_bitmap::{get_raster, get_raster_width, FontWeight, RasterHeight};
use spin::Mutex;

const FONT_WEIGHT: FontWeight = FontWeight::Regular;
const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;
const LINE_HEIGHT: usize = 18;
const PADDING: usize = 4;

const BACKDROP: [u8; 3] = [0x12, 0x12, 0x18];
const FOREGROUND: [u8; 3] = [0xe6, 0xe6, 0xe6];

pub struct Writer {
    addr: *mut u8,
    width: usize,
    height: usize,
    pitch: usize,
    bpp: usize,
    red_shift: u32,
    green_shift: u32,
    blue_shift: u32,
    x: usize,
    y: usize,
}

unsafe impl Send for Writer {}

impl Writer {
    fn new(fb: &'static Framebuffer) -> Self {
        let mut w = Self {
            addr: fb.address() as *mut u8,
            width: fb.width as usize,
            height: fb.height as usize,
            pitch: fb.pitch as usize,
            bpp: (fb.bpp / 8) as usize,
            red_shift: fb.red_mask_shift as u32,
            green_shift: fb.green_mask_shift as u32,
            blue_shift: fb.blue_mask_shift as u32,
            x: PADDING,
            y: PADDING,
        };
        w.clear();
        w
    }

    pub fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, BACKDROP);
            }
        }
        self.x = PADDING;
        self.y = PADDING;
    }

    fn encode(&self, rgb: [u8; 3]) -> u32 {
        ((rgb[0] as u32) << self.red_shift)
            | ((rgb[1] as u32) << self.green_shift)
            | ((rgb[2] as u32) << self.blue_shift)
    }

    fn put_pixel(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        if x >= self.width || y >= self.height { return; }
        let off = y * self.pitch + x * self.bpp;
        let val = self.encode(rgb);
        unsafe {
            match self.bpp {
                4 => core::ptr::write_volatile(self.addr.add(off) as *mut u32, val),
                3 => {
                    self.addr.add(off).write_volatile((val & 0xff) as u8);
                    self.addr.add(off + 1).write_volatile(((val >> 8) & 0xff) as u8);
                    self.addr.add(off + 2).write_volatile(((val >> 16) & 0xff) as u8);
                }
                2 => core::ptr::write_volatile(self.addr.add(off) as *mut u16, val as u16),
                _ => {}
            }
        }
    }

    fn newline(&mut self) {
        self.x = PADDING;
        self.y += LINE_HEIGHT;
        if self.y + FONT_HEIGHT.val() + PADDING >= self.height {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let shift_pix = LINE_HEIGHT;
        let shift_bytes = shift_pix * self.pitch;
        let total = self.height * self.pitch;
        if shift_bytes >= total {
            self.clear();
            return;
        }
        unsafe {
            core::ptr::copy(self.addr.add(shift_bytes), self.addr, total - shift_bytes);
        }
        // wipe last shift_pix rows with backdrop
        for y in (self.height - shift_pix)..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, BACKDROP);
            }
        }
        self.y -= shift_pix;
    }

    fn backspace(&mut self) {
        let w = get_raster_width(FONT_WEIGHT, FONT_HEIGHT);
        if self.x >= PADDING + w {
            self.x -= w;
            for dy in 0..FONT_HEIGHT.val() {
                for dx in 0..w {
                    self.put_pixel(self.x + dx, self.y + dy, BACKDROP);
                }
            }
        }
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.x = PADDING,
            '\x08' => self.backspace(),
            c => {
                let w = get_raster_width(FONT_WEIGHT, FONT_HEIGHT);
                if self.x + w >= self.width.saturating_sub(PADDING) {
                    self.newline();
                }
                if let Some(raster) = get_raster(c, FONT_WEIGHT, FONT_HEIGHT) {
                    for (dy, row) in raster.raster().iter().enumerate() {
                        for (dx, &intensity) in row.iter().enumerate() {
                            let color = blend(BACKDROP, FOREGROUND, intensity);
                            self.put_pixel(self.x + dx, self.y + dy, color);
                        }
                    }
                }
                self.x += w;
            }
        }
    }
}

fn blend(bg: [u8; 3], fg: [u8; 3], a: u8) -> [u8; 3] {
    let a = a as u16;
    let inv = 255 - a;
    [
        ((bg[0] as u16 * inv + fg[0] as u16 * a) / 255) as u8,
        ((bg[1] as u16 * inv + fg[1] as u16 * a) / 255) as u8,
        ((bg[2] as u16 * inv + fg[2] as u16 * a) / 255) as u8,
    ]
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() { self.write_char(c); }
        Ok(())
    }
}

pub static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

pub fn init(fb: &'static Framebuffer) {
    *WRITER.lock() = Some(Writer::new(fb));
}

pub fn with_writer<F: FnOnce(&mut Writer)>(f: F) {
    x86_64::instructions::interrupts::without_interrupts(|| {
        if let Some(w) = WRITER.lock().as_mut() {
            f(w);
        }
    });
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    with_writer(|w| { let _ = w.write_fmt(args); });
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::framebuffer::_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($fmt:expr) => ($crate::kprint!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::kprint!(concat!($fmt, "\n"), $($arg)*));
}
