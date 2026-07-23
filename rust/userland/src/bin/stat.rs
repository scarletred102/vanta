#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let fd = vanta_userland::open(b"/etc/config", vanta_userland::OPEN_READ);
    if fd == u64::MAX - 1 {
        vanta_userland::exit(1);
    }
    let mut stat = [0_u8; 16];
    let result = vanta_userland::fstat(fd, &mut stat);
    vanta_userland::close(fd);
    if result == u64::MAX - 1 {
        vanta_userland::exit(1);
    }
    vanta_userland::write(1, b"stat: /etc/config\n");
    vanta_userland::exit(0)
}
