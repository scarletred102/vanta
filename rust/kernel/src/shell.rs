use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

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
            kprintln!("help  status  ls  cat <path>  write <path> <text>");
            kprintln!("mkdir <path>  stat <path>  mv <old> <new>  rm <path>");
            kprintln!("run <path>  net  clear");
        }
        "status" => {
            kprintln!(
                "kernel: rust-native | userspace: ring 3 spawn/wait/exec/socket | storage: VantaFS"
            );
        }
        "net" => match crate::network::status() {
            Some(info) => {
                let gateway = info.gateway_mac.unwrap_or([0; 6]);
                kprintln!(
                    "net: {}.{}.{}.{} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    info.local_ip[0],
                    info.local_ip[1],
                    info.local_ip[2],
                    info.local_ip[3],
                    info.mac[0],
                    info.mac[1],
                    info.mac[2],
                    info.mac[3],
                    info.mac[4],
                    info.mac[5]
                );
                kprintln!(
                    "gateway: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    gateway[0],
                    gateway[1],
                    gateway[2],
                    gateway[3],
                    gateway[4],
                    gateway[5]
                );
                kprintln!(
                    "dns: {}.{}.{}.{}",
                    info.dns_server[0],
                    info.dns_server[1],
                    info.dns_server[2],
                    info.dns_server[3]
                );
                kprintln!(
                    "tcp target: {}.{}.{}.{}:{}",
                    info.tcp_host[0],
                    info.tcp_host[1],
                    info.tcp_host[2],
                    info.tcp_host[3],
                    info.tcp_port
                );
                kprintln!(
                    "icmp: {}",
                    if info.gateway_echoed {
                        "ok"
                    } else {
                        "unavailable"
                    }
                );
                kprintln!(
                    "udp dns: {}",
                    if info.dns_replied {
                        "ok"
                    } else {
                        "unavailable"
                    }
                );
                kprintln!(
                    "tcp socket: {}",
                    if info.tcp_connected {
                        "last connection ok"
                    } else {
                        "not connected"
                    }
                );
            }
            None => kprintln!("net: unavailable"),
        },
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
        command if command.starts_with("stat ") => {
            let path = command.strip_prefix("stat ").expect("stat command");
            match crate::vfs::file_info_root(path) {
                Ok(info) => kprintln!(
                    "{}: {} {} ({} sector{})",
                    path,
                    if info.is_directory {
                        "directory"
                    } else {
                        "file"
                    },
                    if info.is_directory { 0 } else { info.length },
                    info.allocated_sectors,
                    if info.allocated_sectors == 1 { "" } else { "s" }
                ),
                Err(_) => kprintln!("{}: not found", path),
            }
        }
        command if command.starts_with("mv ") => {
            let Some((old_path, new_path)) = command[3..].split_once(' ') else {
                kprintln!("usage: mv <old> <new>");
                return;
            };
            match crate::vfs::rename_root(old_path, new_path) {
                Ok(()) => kprintln!("renamed {} to {}", old_path, new_path),
                Err(_) => kprintln!("mv: failed for {}", old_path),
            }
        }
        command if command.starts_with("mkdir ") => {
            let path = command.strip_prefix("mkdir ").expect("mkdir command");
            match crate::vfs::create_dir_root(path) {
                Ok(()) => kprintln!("created {}", path),
                Err(_) => kprintln!("mkdir: failed for {}", path),
            }
        }
        command if command.starts_with("rm ") => {
            let path = command.strip_prefix("rm ").expect("rm command");
            match crate::vfs::remove_root(path) {
                Ok(()) => kprintln!("removed {}", path),
                Err(_) => kprintln!("rm: failed for {}", path),
            }
        }
        command if command.starts_with("run ") => {
            let path = command.strip_prefix("run ").expect("run command");
            let image = match crate::vfs::read_root(path) {
                Ok(image) => image,
                Err(_) => {
                    kprintln!("run: {}: not found", path);
                    return;
                }
            };
            let process = match crate::process::load_elf(&image) {
                Ok(process) => process,
                Err(_) => {
                    kprintln!("run: {}: not an executable", path);
                    return;
                }
            };
            kprintln!("starting {}", path);
            let mut processes = Vec::with_capacity(1);
            processes.push(Box::new(process));
            unsafe { crate::scheduler::start(processes) }
        }
        "clear" => kprint!("\x1b[2J\x1b[H"),
        _ => kprintln!("{}: command not found", command),
    }
}
