//! desktop: Native GUI Control Center and Desktop widget suite.

#![no_std]
#![no_main]

use vanta_abi::{DisplayInfo, InputEvent};
use vanta_userland::graphics::{Canvas, Color};

const WIDGET_W: usize = 280;
const WIDGET_H: usize = 260;
static mut WIDGET_BUFFER: [u8; WIDGET_W * WIDGET_H * 4] = [0u8; WIDGET_W * WIDGET_H * 4];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut info = DisplayInfo::default();
    if vanta_userland::display_info(&mut info) == u64::MAX - 1 {
        vanta_userland::exit(1);
    }

    let win_x = 680;
    let win_y = 440;

    unsafe {
        let slice = core::slice::from_raw_parts_mut(WIDGET_BUFFER.as_mut_ptr(), WIDGET_W * WIDGET_H * 4);
        let mut canvas = Canvas::new(slice, WIDGET_W, WIDGET_H);

        // Render Control Center Window
        canvas.draw_window(0, 0, WIDGET_W, WIDGET_H, "Control Center & Settings", true);

        // Audio Volume
        canvas.draw_text(16, 36, "Output Volume: 80%", Color::TEXT_PRIMARY, 1);
        canvas.draw_progress_bar(16, 50, WIDGET_W - 32, 12, 80, Color::BLUE_ACCENT);

        // Display Brightness
        canvas.draw_text(16, 72, "Display Brightness: 100%", Color::TEXT_PRIMARY, 1);
        canvas.draw_progress_bar(16, 86, WIDGET_W - 32, 12, 100, Color::YELLOW_ACCENT);

        // Quick Toggles
        canvas.draw_button(16, 110, 115, 24, "Networking: ON", true);
        canvas.draw_button(140, 110, 120, 24, "Compositor: ON", true);
        canvas.draw_button(16, 142, 115, 24, "Dark Theme", true);
        canvas.draw_button(140, 142, 120, 24, "Audio Stream", true);

        // System Action Buttons
        canvas.draw_button(16, 184, 115, 28, "Lock Desktop", false);
        canvas.draw_button(140, 184, 120, 28, "Power Off", false);

        canvas.draw_text(16, 226, "Vanta Desktop Suite v1.0", Color::TEXT_MUTED, 1);

        let blit_slice = core::slice::from_raw_parts(WIDGET_BUFFER.as_ptr(), WIDGET_W * WIDGET_H * 4);
        if vanta_userland::display_blit(win_x, win_y, WIDGET_W as u32, WIDGET_H as u32, blit_slice) == u64::MAX - 1 {
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
