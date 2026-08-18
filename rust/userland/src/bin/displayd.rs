//! displayd: Vanta Window Compositor and Display Manager daemon.

#![no_std]
#![no_main]

use vanta_abi::{DisplayInfo, InputEvent};

const WIDTH: usize = 1024;
const HEIGHT: usize = 768;
const BPP: usize = 4;
const PITCH: usize = WIDTH * BPP;

// Static buffer for desktop composition
static mut FRAME_BUFFER: [u8; 128 * 1024] = [0u8; 128 * 1024];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut info = DisplayInfo::default();
    let res = vanta_userland::display_info(&mut info);
    if res == u64::MAX - 1 {
        vanta_userland::exit(1);
    }

    // Render desktop top bar: 1024 x 32 in modern dark slate (0x1e, 0x22, 0x2d)
    let w = 256;
    let h = 64;
    unsafe {
        for y in 0..h {
            for x in 0..w {
                let off = (y * w + x) * BPP;
                if y < 16 {
                    // Top bar
                    FRAME_BUFFER[off] = 0x2d;
                    FRAME_BUFFER[off + 1] = 0x22;
                    FRAME_BUFFER[off + 2] = 0x1e;
                    FRAME_BUFFER[off + 3] = 0xff;
                } else if x == 0 || x == w - 1 || y == 16 || y == h - 1 {
                    // Window border
                    FRAME_BUFFER[off] = 0x5a;
                    FRAME_BUFFER[off + 1] = 0x50;
                    FRAME_BUFFER[off + 2] = 0x48;
                    FRAME_BUFFER[off + 3] = 0xff;
                } else {
                    // Window client area
                    FRAME_BUFFER[off] = 0x14;
                    FRAME_BUFFER[off + 1] = 0x14;
                    FRAME_BUFFER[off + 2] = 0x18;
                    FRAME_BUFFER[off + 3] = 0xff;
                }
            }
        }
        let buf_slice = core::slice::from_raw_parts(FRAME_BUFFER.as_ptr(), w * h * BPP);
        let blit_res = vanta_userland::display_blit(100, 100, w as u32, h as u32, buf_slice);
        if blit_res == u64::MAX - 1 {
            vanta_userland::exit(2);
        }
    }
    vanta_userland::display_flush();

    // Poll any pending input events
    let mut ev = InputEvent::default();
    let _ = vanta_userland::input_poll(&mut ev);

    vanta_userland::exit(0);
}
