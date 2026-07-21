#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::write(1, b"\n");
    vanta_userland::exit(0)
}
