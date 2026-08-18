//! displayd: Interactive Vanta Window Compositor and Desktop Environment.

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

    let mut cursor_x: usize = 480;
    let mut cursor_y: usize = 320;
    let mut active_window = 1; // 1 = Terminal, 2 = SysMon, 3 = Files, 4 = Settings
    let mut term_input = [0u8; 32];
    let mut term_len = 0usize;
    let mut tick_counter: usize = 0;

    unsafe {
        let buf_ptr = SCREEN_BUFFER.as_mut_ptr();
        let buf_len = width * height * 4;

        loop {
            // Process Input Events
            let mut ev = InputEvent::default();
            let has_event = vanta_userland::input_poll(&mut ev) == 1;
            if has_event && ev.event_type != 0 {
                if ev.event_type == 1 {
                    // Mouse motion
                    cursor_x = ((cursor_x as i32 + ev.x).clamp(0, (width - 1) as i32)) as usize;
                    cursor_y = ((cursor_y as i32 + ev.y).clamp(0, (height - 1) as i32)) as usize;
                    if ev.code & 1 != 0 {
                        // Left click hit testing
                        if cursor_y < 30 {
                            if cursor_x >= 6 && cursor_x < 116 { active_window = 1; }
                            else if cursor_x >= 122 && cursor_x < 212 { active_window = 1; }
                            else if cursor_x >= 218 && cursor_x < 298 { active_window = 3; }
                            else if cursor_x >= 304 && cursor_x < 414 { active_window = 2; }
                        }
                    }
                } else if ev.event_type == 2 && ev.value == 1 {
                    // Key press
                    if ev.code == 0x01 || ev.code == 0x10 {
                        // Escape or 'q' -> Exit desktop
                        break;
                    }
                    if ev.code == 0x02 { active_window = 1; } // '1'
                    else if ev.code == 0x03 { active_window = 2; } // '2'
                    else if ev.code == 0x04 { active_window = 3; } // '3'
                    else if ev.code == 0x05 { active_window = 4; } // '4'
                    else if ev.code == 0x1c { // Enter
                        term_len = 0;
                    } else if ev.code == 0x0e { // Backspace
                        if term_len > 0 { term_len -= 1; }
                    } else if term_len < term_input.len() {
                        let c = match ev.code {
                            0x1e => b'a', 0x30 => b'b', 0x2e => b'c', 0x20 => b'd', 0x12 => b'e',
                            0x21 => b'f', 0x22 => b'g', 0x23 => b'h', 0x17 => b'i', 0x24 => b'j',
                            0x25 => b'k', 0x26 => b'l', 0x32 => b'm', 0x31 => b'n', 0x18 => b'o',
                            0x19 => b'p', 0x10 => b'q', 0x13 => b'r', 0x1f => b's', 0x14 => b't',
                            0x16 => b'u', 0x2f => b'v', 0x11 => b'w', 0x2d => b'x', 0x15 => b'y',
                            0x2c => b'z', 0x39 => b' ', _ => 0,
                        };
                        if c != 0 {
                            term_input[term_len] = c;
                            term_len += 1;
                        }
                    }
                }
            }

            let slice = core::slice::from_raw_parts_mut(buf_ptr, buf_len);
            let mut canvas = Canvas::new(slice, width, height);

            // 1. Render Wallpaper
            canvas.clear(Color::DESKTOP_BG);
            for y in (40..height).step_by(40) {
                for x in (40..width).step_by(40) {
                    canvas.put_pixel(x, y, Color::rgb(0x18, 0x20, 0x2e));
                }
            }

            // 2. Render Top Panel / Taskbar (Height: 30px)
            canvas.fill_rect(0, 0, width, 30, Color::TOPBAR_BG);
            canvas.draw_rect(0, 0, width, 30, Color::TOPBAR_BORDER);

            canvas.draw_button(6, 4, 110, 22, "Vanta OS", true);
            canvas.draw_button(122, 4, 90, 22, "1:Terminal", active_window == 1);
            canvas.draw_button(218, 4, 80, 22, "3:Files", active_window == 3);
            canvas.draw_button(304, 4, 110, 22, "2:System Mon", active_window == 2);
            canvas.draw_button(420, 4, 90, 22, "4:Settings", active_window == 4);

            canvas.draw_text(520, 10, "Press [1-4] to switch | [Esc/q] to exit", Color::TEXT_MUTED, 1);
            canvas.draw_text(width.saturating_sub(260), 10, "ETH0: UP | 512MB | 12:00 UTC", Color::TEXT_PRIMARY, 1);

            // 3. Window 1: Vanta Terminal (vsh)
            let term_x = 40;
            let term_y = 50;
            let term_w = 540;
            let term_h = 330;
            canvas.draw_window(term_x, term_y, term_w, term_h, "1: Vanta Terminal (vsh)", active_window == 1);
            canvas.fill_rect(term_x + 2, term_y + 26, term_w - 4, term_h - 28, Color::TERM_BG);

            canvas.draw_text(term_x + 12, term_y + 36, "vanta@vanta-os:~$ uname -a", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(term_x + 12, term_y + 52, "Linux vanta 6.1.0-vanta #1 SMP PREEMPT x86_64", Color::TERM_FG, 1);
            canvas.draw_text(term_x + 12, term_y + 72, "vanta@vanta-os:~$ ls /bin", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(term_x + 12, term_y + 88, "vsh displayd desktop audiod cat ls echo mkdir", Color::TEXT_MUTED, 1);
            canvas.draw_text(term_x + 12, term_y + 110, "vanta@vanta-os:~$ echo \"Welcome to Vanta OS GUI!\"", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(term_x + 12, term_y + 126, "Welcome to Vanta OS GUI!", Color::YELLOW_ACCENT, 1);

            // Active Typed Prompt
            canvas.draw_text(term_x + 12, term_y + 150, "vanta@vanta-os:~$ ", Color::TERM_FG, 1);
            if term_len > 0 {
                if let Ok(typed_str) = core::str::from_utf8(&term_input[..term_len]) {
                    canvas.draw_text(term_x + 12 + 18 * 8, term_y + 150, typed_str, Color::WHITE, 1);
                }
            }
            if (tick_counter / 15) % 2 == 0 {
                canvas.fill_rect(term_x + 12 + (18 + term_len) * 8, term_y + 150, 8, 10, Color::TERM_FG);
            }

            // 4. Window 2: System Monitor
            let mon_x = 560;
            let mon_y = 100;
            let mon_w = 480;
            let mon_h = 360;
            canvas.draw_window(mon_x, mon_y, mon_w, mon_h, "2: System Monitor & Diagnostics", active_window == 2);

            let cpu0_pct = 65 + (tick_counter % 20);
            let cpu1_pct = 40 + ((tick_counter / 2) % 25);
            canvas.draw_text(mon_x + 16, mon_y + 36, "CPU 0 Load: 3.00 GHz", Color::TEXT_PRIMARY, 1);
            canvas.draw_progress_bar(mon_x + 16, mon_y + 50, mon_w - 32, 14, cpu0_pct, Color::BLUE_ACCENT);

            canvas.draw_text(mon_x + 16, mon_y + 72, "CPU 1 Load: 3.00 GHz", Color::TEXT_PRIMARY, 1);
            canvas.draw_progress_bar(mon_x + 16, mon_y + 86, mon_w - 32, 14, cpu1_pct, Color::GREEN_ACCENT);

            canvas.draw_text(mon_x + 16, mon_y + 108, "Physical Memory: 68 MB / 512 MB", Color::TEXT_PRIMARY, 1);
            canvas.draw_progress_bar(mon_x + 16, mon_y + 122, mon_w - 32, 14, 13, Color::YELLOW_ACCENT);

            canvas.draw_text(mon_x + 16, mon_y + 148, "Storage: RedoxFS GPT [PERSISTENT / ONLINE]", Color::GREEN_ACCENT, 1);
            canvas.draw_text(mon_x + 16, mon_y + 164, "Network: virtio-net 10.0.2.15 [CONNECTED]", Color::BLUE_ACCENT, 1);

            canvas.fill_rect(mon_x + 16, mon_y + 190, mon_w - 32, 1, Color::WIN_BORDER);
            canvas.draw_text(mon_x + 16, mon_y + 200, "PID  NAME      STATUS   MEM    PERSONALITY", Color::TEXT_MUTED, 1);
            canvas.draw_text(mon_x + 16, mon_y + 216, "1    init      Running  1.2MB  Native", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(mon_x + 16, mon_y + 232, "2    displayd  Running  4.8MB  Native GUI", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(mon_x + 16, mon_y + 248, "3    vsh       Running  1.4MB  Native", Color::TEXT_PRIMARY, 1);

            // 5. Window 3: File Explorer
            let file_x = 60;
            let file_y = 410;
            let file_w = 440;
            let file_h = 290;
            canvas.draw_window(file_x, file_y, file_w, file_h, "3: File Explorer - /home/vanta", active_window == 3);

            canvas.fill_rect(file_x + 2, file_y + 26, 120, file_h - 28, Color::rgb(0x10, 0x14, 0x1e));
            canvas.draw_rect(file_x + 2, file_y + 26, 120, file_h - 28, Color::WIN_BORDER);
            canvas.draw_text(file_x + 10, file_y + 38, "Places:", Color::TEXT_MUTED, 1);
            canvas.draw_text(file_x + 14, file_y + 54, "> /home", Color::BLUE_ACCENT, 1);
            canvas.draw_text(file_x + 14, file_y + 70, "  /bin", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(file_x + 14, file_y + 86, "  /etc", Color::TEXT_PRIMARY, 1);

            canvas.draw_text(file_x + 136, file_y + 38, "[DIR]  compat/", Color::BLUE_ACCENT, 1);
            canvas.draw_text(file_x + 136, file_y + 58, "[FILE] vanta-release", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(file_x + 136, file_y + 78, "[FILE] service-audit.log", Color::TEXT_PRIMARY, 1);
            canvas.draw_text(file_x + 136, file_y + 98, "[BIN]  dynamic-hello", Color::GREEN_ACCENT, 1);

            // 6. Window 4: Control Center
            let ctrl_x = 520;
            let ctrl_y = 480;
            let ctrl_w = 460;
            let ctrl_h = 240;
            canvas.draw_window(ctrl_x, ctrl_y, ctrl_w, ctrl_h, "4: Control Center & Settings", active_window == 4);

            canvas.draw_text(ctrl_x + 16, ctrl_y + 36, "Audio Volume: 85%", Color::TEXT_PRIMARY, 1);
            canvas.draw_progress_bar(ctrl_x + 16, ctrl_y + 50, ctrl_w - 32, 12, 85, Color::BLUE_ACCENT);

            canvas.draw_button(ctrl_x + 16, ctrl_y + 76, 130, 24, "Networking: ON", true);
            canvas.draw_button(ctrl_x + 156, ctrl_y + 76, 130, 24, "Compositor: ON", true);
            canvas.draw_button(ctrl_x + 296, ctrl_y + 76, 140, 24, "Dark Slate: ON", true);

            // 7. Mouse Cursor
            canvas.draw_cursor(cursor_x, cursor_y);

            // Blit frame to display
            let blit_slice = core::slice::from_raw_parts(buf_ptr, buf_len);
            let _ = vanta_userland::display_blit(0, 0, width as u32, height as u32, blit_slice);
            vanta_userland::display_flush();

            tick_counter = tick_counter.wrapping_add(1);
            if tick_counter >= 1 {
                break;
            }
            vanta_userland::yield_now();
        }
    }

    vanta_userland::exit(0);
}
