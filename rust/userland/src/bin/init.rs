#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::write(1, b"vanta init: starting /bin/vsh\n");
    let acceptance_ok = native_acceptance();
    let procd = vanta_userland::spawn(b"/bin/procd");
    let procd_ok = procd != u64::MAX && vanta_userland::wait(procd) == 0;
    vanta_userland::write(
        2,
        if procd_ok {
            b"[native] acceptance: procd-gate ok\n"
        } else {
            b"[native] acceptance: procd-gate failed\n"
        },
    );
    let gate = vanta_userland::spawn(b"/bin/native-gate");
    let gate_ok = gate != u64::MAX && vanta_userland::wait(gate) == 0;
    vanta_userland::write(
        2,
        if gate_ok {
            b"[native] acceptance: developer-gate ok\n"
        } else {
            b"[native] acceptance: developer-gate failed\n"
        },
    );
    if acceptance_ok && gate_ok && procd_ok {
        vanta_userland::write(1, b"[native] terminal/filesystem acceptance passed\n");
        vanta_userland::write(1, b"[native] Gate B IPC acceptance passed\n");
    } else {
        vanta_userland::write(2, b"[native] terminal/filesystem acceptance failed\n");
    }
    let shell = vanta_userland::spawn(b"/bin/vsh");
    if shell == u64::MAX {
        vanta_userland::write(2, b"vanta init: could not spawn /bin/vsh\n");
        vanta_userland::exit(1)
    }
    vanta_userland::exit(vanta_userland::wait(shell))
}

fn native_acceptance() -> bool {
    let path = b"/home/vanta/init-acceptance";
    let renamed = b"/home/vanta/init-acceptance-renamed";
    let directory = b"/home/vanta/init-acceptance-dir";
    let _ = vanta_userland::mkdir(b"/home/vanta");
    let fd = vanta_userland::open(
        path,
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_TRUNCATE,
    );
    if fd == u64::MAX - 1 {
        vanta_userland::write(2, b"[native] acceptance: create failed\n");
        return false;
    }
    let _write_result = vanta_userland::write(fd, b"vanta-native\n");
    let mut stat = [0_u8; 16];
    let stat_ok = vanta_userland::fstat(fd, &mut stat) == 0
        && u64::from_ne_bytes(stat[..8].try_into().unwrap()) == 13;
    vanta_userland::close(fd);
    let read_fd = vanta_userland::open(path, vanta_userland::OPEN_READ);
    if read_fd == u64::MAX - 1 {
        vanta_userland::write(2, b"[native] acceptance: read-open failed\n");
        return false;
    }
    let mut bytes = [0_u8; 32];
    let count = vanta_userland::read(read_fd, &mut bytes);
    vanta_userland::close(read_fd);
    let read_ok = count == 13 && &bytes[..13] == b"vanta-native\n";
    let _mkdir_result = vanta_userland::mkdir(directory);
    let directory_fd = vanta_userland::open(directory, vanta_userland::OPEN_READ);
    let dir_ok = directory_fd != u64::MAX - 1;
    if dir_ok {
        vanta_userland::close(directory_fd);
    }
    let rename_ok = vanta_userland::rename(path, renamed) == 0;
    let remove_ok = vanta_userland::unlink(renamed) == 0;
    let remove_dir_ok = vanta_userland::unlink(directory) == 0;
    let c_hello = vanta_userland::spawn(b"/bin/c-hello");
    let c_hello_ok = c_hello != u64::MAX && vanta_userland::wait(c_hello) == 0;
    vanta_userland::write(
        2,
        if c_hello_ok {
            b"[native] acceptance: c-hello ok\n"
        } else {
            b"[native] acceptance: c-hello failed\n"
        },
    );
    let c_sdk_smoke = vanta_userland::spawn(b"/bin/c-sdk-smoke");
    let c_sdk_smoke_ok = c_sdk_smoke != u64::MAX && vanta_userland::wait(c_sdk_smoke) == 0;
    vanta_userland::write(
        2,
        if c_sdk_smoke_ok {
            b"[native] acceptance: c-sdk-smoke ok\n"
        } else {
            b"[native] acceptance: c-sdk-smoke failed\n"
        },
    );
    let c_stdio_smoke = vanta_userland::spawn(b"/bin/c-stdio-smoke");
    let c_stdio_smoke_ok = c_stdio_smoke != u64::MAX && vanta_userland::wait(c_stdio_smoke) == 0;
    vanta_userland::write(
        2,
        if c_stdio_smoke_ok {
            b"[native] acceptance: c-stdio-smoke ok\n"
        } else {
            b"[native] acceptance: c-stdio-smoke failed\n"
        },
    );
    let c_dir_smoke = vanta_userland::spawn(b"/bin/c-dir-smoke");
    let c_dir_smoke_ok = c_dir_smoke != u64::MAX && vanta_userland::wait(c_dir_smoke) == 0;
    vanta_userland::write(
        2,
        if c_dir_smoke_ok {
            b"[native] acceptance: c-dir-smoke ok\n"
        } else {
            b"[native] acceptance: c-dir-smoke failed\n"
        },
    );
    let c_env_smoke = vanta_userland::spawn(b"/bin/c-env-smoke");
    let c_env_smoke_ok = c_env_smoke != u64::MAX && vanta_userland::wait(c_env_smoke) == 0;
    vanta_userland::write(
        2,
        if c_env_smoke_ok {
            b"[native] acceptance: c-env-smoke ok\n"
        } else {
            b"[native] acceptance: c-env-smoke failed\n"
        },
    );
    let c_process_smoke = vanta_userland::spawn(b"/bin/c-process-smoke");
    let c_process_smoke_ok =
        c_process_smoke != u64::MAX && vanta_userland::wait(c_process_smoke) == 0;
    vanta_userland::write(
        2,
        if c_process_smoke_ok {
            b"[native] acceptance: c-process-smoke ok\n"
        } else {
            b"[native] acceptance: c-process-smoke failed\n"
        },
    );
    let c_exec_smoke = vanta_userland::spawn(b"/bin/c-exec-smoke");
    let c_exec_smoke_ok = c_exec_smoke != u64::MAX && vanta_userland::wait(c_exec_smoke) == 0;
    vanta_userland::write(
        2,
        if c_exec_smoke_ok {
            b"[native] acceptance: c-exec-smoke ok\n"
        } else {
            b"[native] acceptance: c-exec-smoke failed\n"
        },
    );
    vanta_userland::write(2, b"w1 ");
    vanta_userland::write(2, if stat_ok { b"s1 " } else { b"s0 " });
    vanta_userland::write(2, if read_ok { b"r1 " } else { b"r0 " });
    vanta_userland::write(2, if dir_ok { b"d1 " } else { b"d0 " });
    vanta_userland::write(2, if rename_ok { b"n1 " } else { b"n0 " });
    vanta_userland::write(2, if remove_ok { b"x1 " } else { b"x0 " });
    vanta_userland::write(2, if remove_dir_ok { b"D1\n" } else { b"D0\n" });
    vanta_userland::write(2, if c_hello_ok { b"c1\n" } else { b"c0\n" });
    stat_ok
        && read_ok
        && dir_ok
        && rename_ok
        && remove_ok
        && remove_dir_ok
        && c_hello_ok
        && c_sdk_smoke_ok
        && c_stdio_smoke_ok
        && c_dir_smoke_ok
        && c_env_smoke_ok
        && c_process_smoke_ok
        && c_exec_smoke_ok
}
