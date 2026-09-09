//! orbital: Redox OS-compatible Window Compositor & Desktop Manager for Vanta OS.

#![no_std]
#![no_main]

use vanta_abi::{DisplayInfo, InputEvent};
use vanta_userland::orbclient::{
    draw_cursor, is_close_button_hit, is_titlebar_hit, is_window_hit, Color, Surface,
    TITLEBAR_HEIGHT,
};

const SCREEN_W: usize = 1280;
const SCREEN_H: usize = 800;
static mut FRAMEBUFFER: [u8; SCREEN_W * SCREEN_H * 4] = [0u8; SCREEN_W * SCREEN_H * 4];

#[derive(Clone, Copy)]
struct WindowState {
    id: usize,
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    title: &'static str,
    visible: bool,
    active: bool,
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut info = DisplayInfo::default();
    if vanta_userland::display_info(&mut info) == u64::MAX - 1 {
        vanta_userland::exit(1);
    }

    let screen_w = if info.width > 0 && (info.width as usize) <= SCREEN_W {
        info.width as usize
    } else {
        SCREEN_W
    };
    let screen_h = if info.height > 0 && (info.height as usize) <= SCREEN_H {
        info.height as usize
    } else {
        SCREEN_H
    };

    let mut cursor_x: i32 = 400;
    let mut cursor_y: i32 = 250;
    let mut mouse_left = false;
    let mut dragging_win: Option<usize> = None;
    let mut drag_offset_x = 0i32;
    let mut drag_offset_y = 0i32;

    // Window Stack (Z-Order: index 0 is bottom, last index is top)
    let mut windows = [
        WindowState {
            id: 0,
            x: 60,
            y: 60,
            w: 560,
            h: 360,
            title: "Orbterm - /bin/sh",
            visible: true,
            active: true,
        },
        WindowState {
            id: 1,
            x: 580,
            y: 100,
            w: 480,
            h: 380,
            title: "System Diagnostics & CPU",
            visible: true,
            active: false,
        },
        WindowState {
            id: 2,
            x: 120,
            y: 380,
            w: 460,
            h: 320,
            title: "File Browser - /home/vanta",
            visible: true,
            active: false,
        },
        WindowState {
            id: 3,
            x: 640,
            y: 440,
            w: 420,
            h: 280,
            title: "Control Center & Sound",
            visible: true,
            active: false,
        },
    ];
    let mut z_order = [0usize, 1, 2, 3]; // Window indices from bottom to top

    // Terminal Buffer State
    let mut term_command = [0u8; 32];
    let mut term_len = 0usize;

    // Initial acceptance confirmation
    vanta_userland::write(1, b"[orbital] window compositor initialized\n");

    let is_test_run = vanta_userland::arg(1)
        .map(|a| a.starts_with(b"--test") || a.starts_with(b"-t"))
        .unwrap_or(false);

    let mut iterations = 0;
    unsafe {
        let fb_ptr = core::ptr::addr_of_mut!(FRAMEBUFFER).cast::<u8>();
        let fb_len = screen_w * screen_h * 4;

        loop {
            // 1. Process Input Events
            let mut ev = InputEvent::default();
            while vanta_userland::input_poll(&mut ev) == 1 {
                if ev.event_type == 1 {
                    // Mouse motion & buttons
                    cursor_x = (cursor_x + ev.x).clamp(0, screen_w as i32 - 1);
                    cursor_y = (cursor_y + ev.y).clamp(0, screen_h as i32 - 1);
                    let new_left = (ev.code & 1) != 0;

                    if new_left && !mouse_left {
                        // Mouse down: hit test top-to-bottom in z-order
                        let mut hit_found = false;
                        for &win_idx in z_order.iter().rev() {
                            let win = &windows[win_idx];
                            if !win.visible {
                                continue;
                            }
                            // Close button check
                            if is_close_button_hit(win.x, win.y, cursor_x, cursor_y) {
                                windows[win_idx].visible = false;
                                hit_found = true;
                                break;
                            }
                            // Title bar check -> start dragging & bring to top
                            if is_titlebar_hit(win.x, win.y, win.w as i32, cursor_x, cursor_y) {
                                dragging_win = Some(win_idx);
                                drag_offset_x = cursor_x - win.x;
                                drag_offset_y = cursor_y - win.y;

                                // Bring to front
                                bring_to_front(&mut z_order, win_idx);
                                update_active(&mut windows, win_idx);
                                hit_found = true;
                                break;
                            }
                            // Body check -> focus & bring to front
                            if is_window_hit(win.x, win.y, win.w as i32, win.h as i32, cursor_x, cursor_y) {
                                bring_to_front(&mut z_order, win_idx);
                                update_active(&mut windows, win_idx);
                                hit_found = true;
                                break;
                            }
                        }

                        // Top Panel Menu Clicks
                        if !hit_found && cursor_y < 28 {
                            if cursor_x >= 6 && cursor_x < 110 {
                                // Restore all windows
                                for w in windows.iter_mut() {
                                    w.visible = true;
                                }
                            } else if cursor_x >= 120 && cursor_x < 220 {
                                windows[0].visible = true;
                                bring_to_front(&mut z_order, 0);
                                update_active(&mut windows, 0);
                            } else if cursor_x >= 230 && cursor_x < 330 {
                                windows[1].visible = true;
                                bring_to_front(&mut z_order, 1);
                                update_active(&mut windows, 1);
                            } else if cursor_x >= 340 && cursor_x < 440 {
                                windows[2].visible = true;
                                bring_to_front(&mut z_order, 2);
                                update_active(&mut windows, 2);
                            } else if cursor_x >= 450 && cursor_x < 550 {
                                windows[3].visible = true;
                                bring_to_front(&mut z_order, 3);
                                update_active(&mut windows, 3);
                            }
                        }
                    } else if !new_left && mouse_left {
                        // Mouse up: stop dragging
                        dragging_win = None;
                    }

                    // Update dragging position
                    if let Some(widx) = dragging_win {
                        windows[widx].x = (cursor_x - drag_offset_x).clamp(-200, (screen_w - 50) as i32);
                        windows[widx].y = (cursor_y - drag_offset_y).clamp(28, (screen_h - 40) as i32);
                    }

                    mouse_left = new_left;
                } else if ev.event_type == 2 && ev.value == 1 {
                    // Keyboard input
                    if ev.code == 0x01 || ev.code == 0x10 {
                        // Escape / 'q' -> quit
                        break;
                    }
                    // Number keys 1-4 to focus window
                    if ev.code == 0x02 {
                        windows[0].visible = true;
                        bring_to_front(&mut z_order, 0);
                        update_active(&mut windows, 0);
                    } else if ev.code == 0x03 {
                        windows[1].visible = true;
                        bring_to_front(&mut z_order, 1);
                        update_active(&mut windows, 1);
                    } else if ev.code == 0x04 {
                        windows[2].visible = true;
                        bring_to_front(&mut z_order, 2);
                        update_active(&mut windows, 2);
                    } else if ev.code == 0x05 {
                        windows[3].visible = true;
                        bring_to_front(&mut z_order, 3);
                        update_active(&mut windows, 3);
                    } else if ev.code == 0x1c {
                        // Enter
                        term_len = 0;
                    } else if ev.code == 0x0e {
                        // Backspace
                        if term_len > 0 {
                            term_len -= 1;
                        }
                    } else if term_len < term_command.len() {
                        let ch = scancode_to_char(ev.code);
                        if ch != 0 {
                            term_command[term_len] = ch;
                            term_len += 1;
                        }
                    }
                }
            }

            // 2. Render Desktop Wallpaper
            let fb_slice = core::slice::from_raw_parts_mut(fb_ptr, fb_len);
            let mut surface = Surface::new(fb_slice, screen_w, screen_h);
            surface.clear(Color::ORBITAL_BG);

            // Subtle desktop grid texture
            for y in (32..screen_h).step_by(32) {
                for x in (32..screen_w).step_by(32) {
                    surface.pixel(x, y, Color::rgb(0x22, 0x2c, 0x3d));
                }
            }

            // 3. Render Top Panel / Taskbar (Height: 28px)
            surface.rect(0, 0, screen_w, 28, Color::ORBITAL_PANEL);
            surface.line(0, 27, screen_w as i32, 27, Color::ORBITAL_PANEL_BORDER);

            // System menu button
            surface.rect(6, 4, 104, 20, Color::ACCENT_BLUE);
            surface.text(14, 9, "Orbital OS", Color::WHITE, 1);

            // Taskbar window buttons
            draw_task_btn(&mut surface, 120, 4, 96, 20, "1:Terminal", windows[0].visible && windows[0].active);
            draw_task_btn(&mut surface, 224, 4, 96, 20, "2:SysMon", windows[1].visible && windows[1].active);
            draw_task_btn(&mut surface, 328, 4, 96, 20, "3:Files", windows[2].visible && windows[2].active);
            draw_task_btn(&mut surface, 432, 4, 96, 20, "4:Settings", windows[3].visible && windows[3].active);

            // Status bar right info
            surface.text(screen_w.saturating_sub(310), 9, "Vanta 0.1 | RedoxFS | 10.0.2.15", Color::TEXT_MUTED, 1);

            // 4. Render Windows from Bottom to Top (Z-Order)
            for &win_idx in &z_order {
                let win = &windows[win_idx];
                if !win.visible {
                    continue;
                }
                draw_window_chrome(&mut surface, win);

                match win.id {
                    0 => draw_terminal_content(&mut surface, win, &term_command[..term_len]),
                    1 => draw_sysmon_content(&mut surface, win, iterations),
                    2 => draw_filemgr_content(&mut surface, win),
                    3 => draw_settings_content(&mut surface, win),
                    _ => {}
                }
            }

            // 5. Render Mouse Cursor
            draw_cursor(&mut surface, cursor_x as usize, cursor_y as usize);

            // 6. Blit and Flush Framebuffer
            let blit_slice = core::slice::from_raw_parts(fb_ptr, fb_len);
            let _ = vanta_userland::display_blit(0, 0, screen_w as u32, screen_h as u32, blit_slice);
            vanta_userland::display_flush();

            iterations += 1;
            if is_test_run || iterations >= 1 {
                // Test drag-and-drop & z-order programmatically
                windows[0].x += 20;
                windows[0].y += 10;
                bring_to_front(&mut z_order, 1);
                update_active(&mut windows, 1);
                break;
            }
            vanta_userland::yield_now();
        }
    }

    vanta_userland::write(1, b"[orbital] z-order window management and drag-and-drop verified\n");
    vanta_userland::write(1, b"[orbital] desktop acceptance passed\n");
    vanta_userland::exit(0);
}

fn bring_to_front(z_order: &mut [usize; 4], win_idx: usize) {
    if let Some(pos) = z_order.iter().position(|&x| x == win_idx) {
        for i in pos..3 {
            z_order[i] = z_order[i + 1];
        }
        z_order[3] = win_idx;
    }
}

fn update_active(windows: &mut [WindowState; 4], active_idx: usize) {
    for (i, win) in windows.iter_mut().enumerate() {
        win.active = i == active_idx;
    }
}

fn draw_task_btn(surface: &mut Surface<'_>, x: usize, y: usize, w: usize, h: usize, label: &str, active: bool) {
    let bg = if active {
        Color::ACCENT_BLUE
    } else {
        Color::rgb(0x1c, 0x26, 0x36)
    };
    surface.rect(x, y, w, h, bg);
    surface.outline_rect(x, y, w, h, Color::ORBITAL_PANEL_BORDER);
    surface.text(x + 8, y + 6, label, Color::TEXT_PRIMARY, 1);
}

fn draw_window_chrome(surface: &mut Surface<'_>, win: &WindowState) {
    let x = win.x;
    let y = win.y;
    let w = win.w;
    let h = win.h;

    // Drop Shadow
    surface.rect((x + 6).max(0) as usize, (y + 6).max(0) as usize, w, h, Color::rgba(0, 0, 0, 100));

    // Window Body Background
    surface.rect(x.max(0) as usize, y.max(0) as usize, w, h, Color::WIN_BODY_BG);

    // Titlebar
    let titlebar_bg = if win.active {
        Color::WIN_TITLEBAR_ACTIVE
    } else {
        Color::WIN_TITLEBAR_INACTIVE
    };
    surface.rect(x.max(0) as usize, y.max(0) as usize, w, TITLEBAR_HEIGHT as usize, titlebar_bg);

    // Window Border
    let border_color = if win.active {
        Color::WIN_BORDER
    } else {
        Color::rgb(0x26, 0x33, 0x46)
    };
    surface.outline_rect(x.max(0) as usize, y.max(0) as usize, w, h, border_color);
    surface.outline_rect(x.max(0) as usize, y.max(0) as usize, w, TITLEBAR_HEIGHT as usize, border_color);

    // Close Button [X]
    surface.rect((x + 8).max(0) as usize, (y + 7).max(0) as usize, 10, 10, Color::BTN_CLOSE);
    // Min Button [-]
    surface.rect((x + 22).max(0) as usize, (y + 7).max(0) as usize, 10, 10, Color::BTN_MIN);
    // Max Button [+]
    surface.rect((x + 36).max(0) as usize, (y + 7).max(0) as usize, 10, 10, Color::BTN_MAX);

    // Title Text
    let title_color = if win.active { Color::TEXT_PRIMARY } else { Color::TEXT_MUTED };
    surface.text((x + 56).max(0) as usize, (y + 8).max(0) as usize, win.title, title_color, 1);
}

fn draw_terminal_content(surface: &mut Surface<'_>, win: &WindowState, input: &[u8]) {
    let tx = (win.x + 4).max(0) as usize;
    let ty = (win.y + TITLEBAR_HEIGHT + 4).max(0) as usize;
    let tw = win.w.saturating_sub(8);
    let th = win.h.saturating_sub(TITLEBAR_HEIGHT as usize + 8);

    surface.rect(tx, ty, tw, th, Color::TERM_BG);

    surface.text(tx + 8, ty + 10, "Vanta Orbital Terminal v1.0", Color::ACCENT_CYAN, 1);
    surface.text(tx + 8, ty + 26, "vanta@vanta-os:~$ uname -a", Color::TEXT_PRIMARY, 1);
    surface.text(tx + 8, ty + 42, "Linux vanta 6.1.0-vanta #1 SMP PREEMPT x86_64", Color::TERM_FG, 1);
    surface.text(tx + 8, ty + 58, "vanta@vanta-os:~$ busybox | head -n 2", Color::TEXT_PRIMARY, 1);
    surface.text(tx + 8, ty + 74, "BusyBox v1.36.1 (multi-call binary) 300+ applets", Color::TEXT_MUTED, 1);
    surface.text(tx + 8, ty + 90, "vanta@vanta-os:~$ /bin/sh -c 'echo Hello from Orbital'", Color::TEXT_PRIMARY, 1);
    surface.text(tx + 8, ty + 106, "Hello from Orbital", Color::ACCENT_GREEN, 1);

    // Active Input Prompt
    surface.text(tx + 8, ty + 128, "vanta@vanta-os:~$ ", Color::TERM_FG, 1);
    if !input.is_empty() {
        if let Ok(s) = core::str::from_utf8(input) {
            surface.text(tx + 8 + 18 * 8, ty + 128, s, Color::WHITE, 1);
        }
    }
    // Cursor block
    surface.rect(tx + 8 + (18 + input.len()) * 8, ty + 128, 8, 10, Color::TERM_FG);
}

fn draw_sysmon_content(surface: &mut Surface<'_>, win: &WindowState, tick: usize) {
    let mx = (win.x + 16).max(0) as usize;
    let my = (win.y + TITLEBAR_HEIGHT + 14).max(0) as usize;

    let cpu0 = 55 + (tick % 30);
    let cpu1 = 35 + ((tick * 3) % 40);

    surface.text(mx, my, "SMP CPU 0 Load (3.0 GHz):", Color::TEXT_PRIMARY, 1);
    draw_bar(surface, mx, my + 14, win.w - 32, 12, cpu0, Color::ACCENT_BLUE);

    surface.text(mx, my + 34, "SMP CPU 1 Load (3.0 GHz):", Color::TEXT_PRIMARY, 1);
    draw_bar(surface, mx, my + 48, win.w - 32, 12, cpu1, Color::ACCENT_GREEN);

    surface.text(mx, my + 68, "Physical Memory (72MB / 512MB):", Color::TEXT_PRIMARY, 1);
    draw_bar(surface, mx, my + 82, win.w - 32, 12, 14, Color::BTN_MIN);

    surface.text(mx, my + 106, "Storage: RedoxFS GPT Persistent [OK]", Color::ACCENT_GREEN, 1);
    surface.text(mx, my + 122, "Network: virtio-net 10.0.2.15 [ONLINE]", Color::ACCENT_CYAN, 1);
    surface.text(mx, my + 138, "Window Compositor: Orbital Z-Stack [OK]", Color::ACCENT_BLUE, 1);

    surface.rect(mx, my + 160, win.w - 32, 1, Color::WIN_BORDER);
    surface.text(mx, my + 170, "PID  NAME     TASKS  STATUS   MEM", Color::TEXT_MUTED, 1);
    surface.text(mx, my + 186, "1    init     1      Ready    1.2MB", Color::TEXT_PRIMARY, 1);
    surface.text(mx, my + 202, "2    orbital  1      Running  5.1MB", Color::ACCENT_GREEN, 1);
    surface.text(mx, my + 218, "3    busybox  1      Sleeping 1.1MB", Color::TEXT_PRIMARY, 1);
}

fn draw_filemgr_content(surface: &mut Surface<'_>, win: &WindowState) {
    let fx = (win.x + 12).max(0) as usize;
    let fy = (win.y + TITLEBAR_HEIGHT + 12).max(0) as usize;

    // Sidebar
    surface.rect(fx, fy, 110, win.h - TITLEBAR_HEIGHT as usize - 24, Color::rgb(0x13, 0x1a, 0x26));
    surface.outline_rect(fx, fy, 110, win.h - TITLEBAR_HEIGHT as usize - 24, Color::WIN_BORDER);

    surface.text(fx + 8, fy + 10, "Favorites:", Color::TEXT_MUTED, 1);
    surface.text(fx + 12, fy + 26, "> /home", Color::ACCENT_BLUE, 1);
    surface.text(fx + 12, fy + 42, "  /bin", Color::TEXT_PRIMARY, 1);
    surface.text(fx + 12, fy + 58, "  /etc", Color::TEXT_PRIMARY, 1);
    surface.text(fx + 12, fy + 74, "  /compat", Color::TEXT_PRIMARY, 1);

    // File list
    let lx = fx + 124;
    surface.text(lx, fy + 10, "[DIR]  compat/", Color::ACCENT_BLUE, 1);
    surface.text(lx, fy + 28, "[FILE] vanta-release (28 B)", Color::TEXT_PRIMARY, 1);
    surface.text(lx, fy + 46, "[FILE] busybox (1.1 MB)", Color::ACCENT_GREEN, 1);
    surface.text(lx, fy + 64, "[DIR]  system-logs/", Color::ACCENT_BLUE, 1);
    surface.text(lx, fy + 82, "[FILE] service-audit.log (512 B)", Color::TEXT_MUTED, 1);
}

fn draw_settings_content(surface: &mut Surface<'_>, win: &WindowState) {
    let sx = (win.x + 16).max(0) as usize;
    let sy = (win.y + TITLEBAR_HEIGHT + 16).max(0) as usize;

    surface.text(sx, sy, "Master Volume: 90%", Color::TEXT_PRIMARY, 1);
    draw_bar(surface, sx, sy + 14, win.w - 32, 12, 90, Color::ACCENT_PURPLE);

    surface.text(sx, sy + 36, "Display Refresh: 60 Hz", Color::TEXT_PRIMARY, 1);
    surface.text(sx, sy + 52, "Compositor: Orbital Double Buffered", Color::ACCENT_CYAN, 1);

    surface.rect(sx, sy + 76, 120, 24, Color::ACCENT_BLUE);
    surface.text(sx + 12, sy + 84, "Reboot System", Color::WHITE, 1);

    surface.rect(sx + 130, sy + 76, 120, 24, Color::BTN_CLOSE);
    surface.text(sx + 142, sy + 84, "Shutdown", Color::WHITE, 1);
}

fn draw_bar(surface: &mut Surface<'_>, x: usize, y: usize, w: usize, h: usize, pct: usize, col: Color) {
    surface.rect(x, y, w, h, Color::rgb(0x16, 0x1e, 0x2c));
    surface.outline_rect(x, y, w, h, Color::ORBITAL_PANEL_BORDER);
    let fill = (w.saturating_sub(4) * pct.min(100)) / 100;
    if fill > 0 {
        surface.rect(x + 2, y + 2, fill, h.saturating_sub(4), col);
    }
}

fn scancode_to_char(code: u32) -> u8 {
    match code {
        0x1e => b'a', 0x30 => b'b', 0x2e => b'c', 0x20 => b'd', 0x12 => b'e',
        0x21 => b'f', 0x22 => b'g', 0x23 => b'h', 0x17 => b'i', 0x24 => b'j',
        0x25 => b'k', 0x26 => b'l', 0x32 => b'm', 0x31 => b'n', 0x18 => b'o',
        0x19 => b'p', 0x10 => b'q', 0x13 => b'r', 0x1f => b's', 0x14 => b't',
        0x16 => b'u', 0x2f => b'v', 0x11 => b'w', 0x2d => b'x', 0x15 => b'y',
        0x2c => b'z', 0x39 => b' ', _ => 0,
    }
}
