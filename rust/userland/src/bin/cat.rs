#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    if let Some(path) = vanta_userland::arg(1) {
        let fd = vanta_userland::open(path, vanta_userland::OPEN_READ);
        if fd == u64::MAX - 1 {
            vanta_userland::exit(1);
        }
        copy_fd(fd);
        vanta_userland::close(fd);
        vanta_userland::exit(0);
    }
    copy_fd(0);
    vanta_userland::exit(0);
}

fn copy_fd(fd: u64) {
    let mut buffer = [0_u8; 128];
    loop {
        let count = vanta_userland::read(fd, &mut buffer);
        if count == vanta_userland::READ_WOULD_BLOCK {
            continue;
        }
        if count == 0 || count == u64::MAX {
            return;
        }
        vanta_userland::write(1, &buffer[..count as usize]);
    }
}
