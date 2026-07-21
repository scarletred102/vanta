#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::exit(0)
}
