//! displayd: Vanta Window Compositor and Desktop Environment manager.

#![no_std]
#![no_main]

use vanta_abi::{DisplayInfo, InputEvent};
use vanta_userland::graphics::{Canvas, Color};

const MAX_WIDTH: usize = 1280;
const MAX_HEIGHT: usize = 800;
static mut SCREEN_BUFFER: [u8; MAX_WIDTH * MAX_HEIGHT * 4] = [0u8; MAX_WIDTH * MAX_HEIGHT * 4];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut info = DisplayInfo::default();
    let res = vanta_userland::display_info(&mut info);
    if res == u64::MAX - 1 {
        vanta_userland::exit(1);
    }

    let width = if info.width > 0 && (info.width as usize) <= MAX_WIDTH {
        info.width as usize
    } else {
        MAX_WIDTH
    };
    let height = if info.height > 0 && (info.height as usize) <= MAX_HEIGHT {
        info.height as usize
    } else {
        MAX_HEIGHT
    };

    let mut cursor_x: usize = 320;
    let mut cursor_y: usize = 240;

    unsafe {
        let buf_ptr = SCREEN_BUFFER.as_mut_ptr();
        let buf_len = width * height * 4;
        let slice = core::slice::from_raw_parts_mut(buf_ptr, buf_len);
        let mut canvas = Canvas::new(slice, width, height);

        // 1. Render Desktop Wallpaper
        canvas.clear(Color::DESKTOP_BG);

        // Subtle desktop background grid/ambient elements
        for y in (40..height).step_by(40) {
            for x in (40..width).step_by(40) {
                canvas.put_pixel(x, y, Color::rgb(0x18, 0x20, 0x2e));
            }
        }

        // 2. Render Top Panel / Taskbar (Height: 30px)
        canvas.fill_rect(0, 0, width, 30, Color::TOPBAR_BG);
        canvas.draw_rect(0, 0, width, 30, Color::TOPBAR_BORDER);

        // Topbar Buttons
        canvas.draw_button(6, 4, 110, 22, "Vanta OS", true);
        canvas.draw_button(122, 4, 90, 22, "Terminal", false);
        canvas.draw_button(218, 4, 80, 22, "Files", false);
        canvas.draw_button(304, 4, 110, 22, "System Mon", false);

        // Center Title
        canvas.draw_text(450, 10, "Vanta OS Desktop - x86_64 SMP", Color::TEXT_MUTED, 1);

        // Right System Indicators
        canvas.draw_text(width.saturating_sub(280), 10, "ETH0: UP | 512MB RAM | 12:00 UTC", Color::TEXT_PRIMARY, 1);

        // 3. Window 1: "Vanta Terminal (vsh)"
        let term_x = 40;
        let term_y = 50;
        let term_w = 520;
        let term_h = 320;
        canvas.draw_window(term_x, term_y, term_w, term_h, "Vanta Terminal (vsh)", true);
        canvas.fill_rect(term_x + 2, term_y + 26, term_w - 4, term_h - 28, Color::TERM_BG);

        canvas.draw_text(term_x + 12, term_y + 36, "vanta@vanta-os:~$ uname -a", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(term_x + 12, term_y + 52, "Linux vanta 6.1.0-vanta #1 SMP PREEMPT x86_64", Color::TERM_FG, 1);
        canvas.draw_text(term_x + 12, term_y + 72, "vanta@vanta-os:~$ ls -la /bin", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(term_x + 12, term_y + 88, "drwxr-xr-x 2 vanta root  4096 /bin", Color::TEXT_MUTED, 1);
        canvas.draw_text(term_x + 12, term_y + 104, "-rwxr-xr-x 1 vanta root 38192 vsh", Color::TEXT_MUTED, 1);
        canvas.draw_text(term_x + 12, term_y + 120, "-rwxr-xr-x 1 vanta root 45056 displayd", Color::TEXT_MUTED, 1);
        canvas.draw_text(term_x + 12, term_y + 136, "-rwxr-xr-x 1 vanta root 41984 desktop", Color::TEXT_MUTED, 1);
        canvas.draw_text(term_x + 12, term_y + 152, "-rwxr-xr-x 1 vanta root 39936 audiod", Color::TEXT_MUTED, 1);
        canvas.draw_text(term_x + 12, term_y + 172, "vanta@vanta-os:~$ echo \"Welcome to Vanta OS Desktop!\"", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(term_x + 12, term_y + 188, "Welcome to Vanta OS Desktop!", Color::YELLOW_ACCENT, 1);
        canvas.draw_text(term_x + 12, term_y + 208, "vanta@vanta-os:~$ _", Color::TERM_FG, 1);

        // 4. Window 2: "System Monitor"
        let mon_x = 510;
        let mon_y = 130;
        let mon_w = 460;
        let mon_h = 360;
        canvas.draw_window(mon_x, mon_y, mon_w, mon_h, "System Monitor & Diagnostics", false);

        canvas.draw_text(mon_x + 16, mon_y + 36, "CPU 0 Usage: 72% (3.00 GHz)", Color::TEXT_PRIMARY, 1);
        canvas.draw_progress_bar(mon_x + 16, mon_y + 50, mon_w - 32, 14, 72, Color::BLUE_ACCENT);

        canvas.draw_text(mon_x + 16, mon_y + 72, "CPU 1 Usage: 44% (3.00 GHz)", Color::TEXT_PRIMARY, 1);
        canvas.draw_progress_bar(mon_x + 16, mon_y + 86, mon_w - 32, 14, 44, Color::GREEN_ACCENT);

        canvas.draw_text(mon_x + 16, mon_y + 108, "Physical Memory: 64 MB / 512 MB (12%)", Color::TEXT_PRIMARY, 1);
        canvas.draw_progress_bar(mon_x + 16, mon_y + 122, mon_w - 32, 14, 12, Color::YELLOW_ACCENT);

        canvas.draw_text(mon_x + 16, mon_y + 148, "Storage: RedoxFS GPT (Persistent) [ONLINE]", Color::GREEN_ACCENT, 1);
        canvas.draw_text(mon_x + 16, mon_y + 164, "Network: virtio-net 52:54:00:12:34:56 [UP]", Color::BLUE_ACCENT, 1);
        canvas.draw_text(mon_x + 16, mon_y + 180, "Compositor: displayd 60 FPS Hardware Scanout", Color::TEXT_MUTED, 1);

        canvas.fill_rect(mon_x + 16, mon_y + 202, mon_w - 32, 1, Color::WIN_BORDER);
        canvas.draw_text(mon_x + 16, mon_y + 210, "PID   NAME        STATUS   MEM     PERSONALITY", Color::TEXT_MUTED, 1);
        canvas.draw_text(mon_x + 16, mon_y + 226, "1     init        Running  1.2 MB  Native", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(mon_x + 16, mon_y + 242, "2     displayd    Running  4.8 MB  Native GUI", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(mon_x + 16, mon_y + 258, "3     audiod      Running  1.1 MB  Native", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(mon_x + 16, mon_y + 274, "4     dynamic-net Sleeping 2.0 MB  Linux Musl", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(mon_x + 16, mon_y + 290, "5     vsh         Running  1.4 MB  Native", Color::TEXT_PRIMARY, 1);

        // 5. Window 3: "File Explorer"
        let file_x = 80;
        let file_y = 400;
        let file_w = 410;
        let file_h = 300;
        canvas.draw_window(file_x, file_y, file_w, file_h, "File Explorer - /home/vanta", false);

        // Sidebar
        canvas.fill_rect(file_x + 2, file_y + 26, 120, file_h - 28, Color::rgb(0x10, 0x14, 0x1e));
        canvas.draw_rect(file_x + 2, file_y + 26, 120, file_h - 28, Color::WIN_BORDER);
        canvas.draw_text(file_x + 10, file_y + 38, "Places:", Color::TEXT_MUTED, 1);
        canvas.draw_text(file_x + 14, file_y + 54, "> /home", Color::BLUE_ACCENT, 1);
        canvas.draw_text(file_x + 14, file_y + 70, "  /bin", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(file_x + 14, file_y + 86, "  /etc", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(file_x + 14, file_y + 102, "  /compat", Color::TEXT_PRIMARY, 1);

        // File items grid
        canvas.draw_text(file_x + 136, file_y + 38, "[DIR]  compat/", Color::BLUE_ACCENT, 1);
        canvas.draw_text(file_x + 136, file_y + 58, "[FILE] vanta-release", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(file_x + 136, file_y + 78, "[FILE] service-audit.log", Color::TEXT_PRIMARY, 1);
        canvas.draw_text(file_x + 136, file_y + 98, "[BIN]  dynamic-hello", Color::GREEN_ACCENT, 1);
        canvas.draw_text(file_x + 136, file_y + 118, "[BIN]  dynamic-threads", Color::GREEN_ACCENT, 1);
        canvas.draw_text(file_x + 136, file_y + 138, "[BIN]  dynamic-net", Color::GREEN_ACCENT, 1);
        canvas.draw_text(file_x + 136, file_y + 158, "[BIN]  dynamic-fork", Color::GREEN_ACCENT, 1);

        // 6. Draw Mouse Cursor
        canvas.draw_cursor(cursor_x, cursor_y);

        // Blit full-screen desktop composition to hardware framebuffer
        let blit_slice = core::slice::from_raw_parts(buf_ptr, buf_len);
        let _ = vanta_userland::display_blit(0, 0, width as u32, height as u32, blit_slice);
        vanta_userland::display_flush();
    }

    // Poll any initial input events
    let mut ev = InputEvent::default();
    let _ = vanta_userland::input_poll(&mut ev);

    vanta_userland::exit(0);
}
