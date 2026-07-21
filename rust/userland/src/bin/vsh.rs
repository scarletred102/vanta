#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::write(1, b"vanta native shell\nvanta> ");
    let mut input = [0_u8; 64];
    let mut command = [0_u8; 64];
    let mut command_length = 0usize;
    loop {
        let count = vanta_userland::read(0, &mut input);
        if count != 0 && count != u64::MAX {
            for byte in &input[..count as usize] {
                match *byte {
                    8 => {
                        vanta_userland::write(1, b"\x08 \x08");
                    }
                    b'\n' => {
                        vanta_userland::write(1, b"\n");
                    }
                    byte => {
                        if command_length < command.len() {
                            command[command_length] = byte;
                            command_length += 1;
                            vanta_userland::write(1, &[byte]);
                        }
                    }
                };
                if *byte == 8 && command_length > 0 {
                    command_length -= 1;
                }
                if *byte == b'\n' {
                    run_command(&command[..command_length]);
                    command_length = 0;
                    vanta_userland::write(1, b"vanta> ");
                }
            }
        }
        vanta_userland::yield_now();
    }
}

fn run_command(command: &[u8]) {
    match command {
        b"" => {}
        b"help" => {
            vanta_userland::write(1, b"help clear echo cat true false\n");
        }
        b"clear" => {
            vanta_userland::write(1, b"\x1b[2J\x1b[H");
        }
        command => match vanta_userland::command_path(command) {
            Some(path) => {
                let pid = vanta_userland::spawn(path);
                if pid != u64::MAX {
                    let _ = vanta_userland::wait(pid);
                } else {
                    vanta_userland::write(2, b"vsh: spawn failed\n");
                }
            }
            None => {
                vanta_userland::write(2, b"vsh: command not found\n");
            }
        },
    };
}
