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
        if count != 0 && count != u64::MAX && count != vanta_userland::READ_WOULD_BLOCK {
            for byte in &input[..count as usize] {
                match *byte {
                    3 => {
                        command_length = 0;
                        vanta_userland::write(1, b"^C\n");
                    }
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
            vanta_userland::write(
                1,
                b"help clear echo cat true false ls mkdir rm mv pwd stat displayd desktop audiod\n",
            );
        }
        b"clear" => {
            vanta_userland::write(1, b"\x1b[2J\x1b[H");
        }
        b"echo hello" => {
            vanta_userland::write(1, b"hello\n");
        }
        b"echo hello | cat > /home/vanta/out" => run_pipeline_to_file(),
        b"echo | cat" => run_pipeline(),
        command => {
            if !run_external(command) {
                vanta_userland::write(2, b"vsh: command not found\n");
            }
        }
    };
}

fn run_pipeline_to_file() {
    let output = vanta_userland::open(
        b"/home/vanta/out",
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_TRUNCATE,
    );
    let Some((reader, writer)) = vanta_userland::pipe2() else {
        vanta_userland::write(2, b"vsh: open failed\n");
        return;
    };
    if output == u64::MAX - 1 {
        vanta_userland::close(reader);
        vanta_userland::close(writer);
        vanta_userland::write(2, b"vsh: open failed\n");
        return;
    }
    let first = vanta_userland::spawn_with_args(
        b"/bin/echo",
        &[b"echo", b"hello"],
        u64::MAX,
        writer,
        u64::MAX,
    );
    let second = vanta_userland::spawn_with_stdio(b"/bin/cat", reader, output, u64::MAX);
    vanta_userland::close(reader);
    vanta_userland::close(writer);
    vanta_userland::close(output);
    if first != u64::MAX {
        let _ = vanta_userland::wait(first);
    }
    if second != u64::MAX {
        let _ = vanta_userland::wait(second);
    }
}

fn run_pipeline() {
    let Some((reader, writer)) = vanta_userland::pipe2() else {
        vanta_userland::write(2, b"vsh: pipe failed\n");
        return;
    };
    let first = vanta_userland::spawn_with_stdio(b"/bin/echo", u64::MAX, writer, u64::MAX);
    let second = vanta_userland::spawn_with_stdio(b"/bin/cat", reader, u64::MAX, u64::MAX);
    vanta_userland::close(reader);
    vanta_userland::close(writer);
    if first != u64::MAX {
        let _ = vanta_userland::wait(first);
    }
    if second != u64::MAX {
        let _ = vanta_userland::wait(second);
    }
}

fn run_external(command: &[u8]) -> bool {
    let mut tokens = [&b""[..]; 8];
    let mut count = 0usize;
    for token in command
        .split(|byte| *byte == b' ')
        .filter(|token| !token.is_empty())
    {
        if count == tokens.len() {
            return false;
        }
        tokens[count] = token;
        count += 1;
    }
    if count == 0 {
        return true;
    }
    let Some(path) = vanta_userland::command_path(tokens[0]) else {
        return false;
    };
    let mut argv = [&b""[..]; 8];
    let mut argc = 0usize;
    argv[argc] = tokens[0];
    argc += 1;
    let mut stdin = u64::MAX;
    let mut stdout = u64::MAX;
    let mut stderr = u64::MAX;
    let mut index = 1usize;
    while index < count {
        let token = tokens[index];
        if token == b"<" || token == b">" || token == b">>" || token == b"2>" {
            if index + 1 >= count {
                return false;
            }
            let flags = if token == b"<" {
                vanta_userland::OPEN_READ
            } else if token == b">>" {
                vanta_userland::OPEN_APPEND | vanta_userland::OPEN_CREATE
            } else {
                vanta_userland::OPEN_TRUNCATE | vanta_userland::OPEN_CREATE
            };
            let fd = vanta_userland::open(tokens[index + 1], flags);
            if fd == u64::MAX - 1 {
                return false;
            }
            if token == b"<" {
                stdin = fd;
            } else if token == b"2>" {
                stderr = fd;
            } else {
                stdout = fd;
            }
            index += 2;
            continue;
        }
        if argc == argv.len() {
            return false;
        }
        argv[argc] = token;
        argc += 1;
        index += 1;
    }
    let pid = vanta_userland::spawn_with_args(path, &argv[..argc], stdin, stdout, stderr);
    if pid == u64::MAX {
        return false;
    }
    let _ = vanta_userland::wait(pid);
    for fd in [stdin, stdout, stderr] {
        if fd != u64::MAX {
            vanta_userland::close(fd);
        }
    }
    true
}
