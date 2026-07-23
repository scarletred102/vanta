#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let fd = vanta_userland::open(b"/", vanta_userland::OPEN_READ);
    if fd == u64::MAX - 1 {
        vanta_userland::exit(1);
    }
    let mut buffer = [0_u8; 256];
    let count = vanta_userland::getdents(fd, &mut buffer);
    if count != u64::MAX - 1 {
        vanta_userland::write(1, &buffer[..count as usize]);
    }
    vanta_userland::close(fd);
    vanta_userland::exit(0)
}
