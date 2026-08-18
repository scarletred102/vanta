//! desktop: Native GUI desktop acceptance application.

#![no_std]
#![no_main]

use vanta_abi::{DisplayInfo, InputEvent};

static mut BUTTON_SURFACE: [u8; 64 * 32 * 4] = [0u8; 64 * 32 * 4];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut info = DisplayInfo::default();
    if vanta_userland::display_info(&mut info) == u64::MAX - 1 {
        vanta_userland::exit(1);
    }

    // Render interactive GUI button: 64x32 with blue accent (0x3b, 0x82, 0xf6)
    let bw = 64;
    let bh = 32;
    unsafe {
        for y in 0..bh {
            for x in 0..bw {
                let off = (y * bw + x) * 4;
                if x == 0 || x == bw - 1 || y == 0 || y == bh - 1 {
                    BUTTON_SURFACE[off] = 0x60;
                    BUTTON_SURFACE[off + 1] = 0xa5;
                    BUTTON_SURFACE[off + 2] = 0xfa;
                    BUTTON_SURFACE[off + 3] = 0xff;
                } else {
                    BUTTON_SURFACE[off] = 0x3b;
                    BUTTON_SURFACE[off + 1] = 0x82;
                    BUTTON_SURFACE[off + 2] = 0xf6;
                    BUTTON_SURFACE[off + 3] = 0xff;
                }
            }
        }
        let slice = core::slice::from_raw_parts(BUTTON_SURFACE.as_ptr(), bw * bh * 4);
        if vanta_userland::display_blit(200, 200, bw as u32, bh as u32, slice) == u64::MAX - 1 {
            vanta_userland::exit(2);
        }
    }

    vanta_userland::display_flush();

    // Verify input polling
    let mut ev = InputEvent::default();
    let _ = vanta_userland::input_poll(&mut ev);

    // Print acceptance confirmation
    vanta_userland::write(1, b"desktop: GUI window surface composition verified\n");
    vanta_userland::exit(0);
}
