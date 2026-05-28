use crate::{keyboard, kprint, kprintln, serial_println};
use pc_keyboard::{layouts::Us104Key, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
use spin::Mutex;

static KBD: Mutex<Option<Keyboard<Us104Key, ScancodeSet1>>> = Mutex::new(None);

fn banner() {
    kprintln!("vanta os | kernel terminal");
    kprintln!("rust-rewrite session 1 — type to echo, Enter for newline, Backspace to delete");
    kprintln!();
    prompt();
}

fn prompt() {
    kprint!("vanta> ");
}

pub fn run() -> ! {
    *KBD.lock() = Some(Keyboard::new(
        ScancodeSet1::new(),
        Us104Key,
        HandleControl::Ignore,
    ));

    banner();
    serial_println!("[shell] entering main loop");

    loop {
        if let Some(sc) = keyboard::pop_scancode() {
            let mut guard = KBD.lock();
            let kbd = guard.as_mut().expect("keyboard not init");
            if let Ok(Some(ev)) = kbd.add_byte(sc) {
                if let Some(key) = kbd.process_keyevent(ev) {
                    drop(guard);
                    handle_key(key);
                }
            }
        } else {
            x86_64::instructions::hlt();
        }
    }
}

fn handle_key(key: DecodedKey) {
    match key {
        DecodedKey::Unicode(c) => match c {
            '\n' | '\r' => {
                kprintln!();
                prompt();
            }
            '\u{8}' | '\u{7f}' => {
                kprint!("\x08");
            }
            c if (c as u32) >= 0x20 => {
                kprint!("{}", c);
            }
            _ => {}
        },
        DecodedKey::RawKey(_) => {}
    }
}
