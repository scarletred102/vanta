use alloc::string::String;

use crate::{keyboard, kprint, kprintln, serial_println};
use pc_keyboard::{layouts::Us104Key, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
use spin::Mutex;

static KBD: Mutex<Option<Keyboard<Us104Key, ScancodeSet1>>> = Mutex::new(None);
static INPUT: Mutex<String> = Mutex::new(String::new());

fn banner() {
    kprintln!("vanta os | kernel terminal");
    kprintln!("type 'help' for available commands");
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
    INPUT.lock().clear();

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
                let command = core::mem::take(&mut *INPUT.lock());
                kprintln!();
                execute(command.as_str());
                prompt();
            }
            '\u{8}' | '\u{7f}' => {
                if INPUT.lock().pop().is_some() {
                    kprint!("\x08 \x08");
                }
            }
            c if (c as u32) >= 0x20 => {
                INPUT.lock().push(c);
                kprint!("{}", c);
            }
            _ => {}
        },
        DecodedKey::RawKey(_) => {}
    }
}

fn execute(command: &str) {
    match command {
        "" => {}
        "help" => {
            kprintln!("help  status  ls  cat /etc/config  cat /etc/persistent  clear");
        }
        "status" => {
            kprintln!("kernel: rust-native | userspace: ring 3 | storage: VantaFS");
        }
        "ls" => match crate::vfs::list_root() {
            Ok(paths) => {
                for path in paths {
                    kprintln!("{}", path);
                }
            }
            Err(_) => kprintln!("ls: VFS unavailable"),
        },
        command if command.starts_with("cat ") => {
            let path = command.strip_prefix("cat ").expect("cat command");
            match crate::vfs::read_root(path) {
                Ok(contents) => {
                    let needs_newline = contents.last().copied() != Some(b'\n');
                    for byte in contents {
                        kprint!("{}", byte as char);
                    }
                    if needs_newline {
                        kprintln!();
                    }
                }
                Err(_) => kprintln!("{}: not found", path),
            }
        }
        command if command.starts_with("write ") => {
            let Some((path, contents)) = command[6..].split_once(' ') else {
                kprintln!("usage: write <path> <text>");
                return;
            };
            match crate::vfs::write_root(path, contents.as_bytes()) {
                Ok(()) => kprintln!("wrote {} bytes to {}", contents.len(), path),
                Err(_) => kprintln!("write: failed for {}", path),
            }
        }
        "clear" => kprint!("\x1b[2J\x1b[H"),
        _ => kprintln!("{}: command not found", command),
    }
}
