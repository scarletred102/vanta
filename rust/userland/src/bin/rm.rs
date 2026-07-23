#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let result = vanta_userland::unlink(b"/home/vanta/out");
    vanta_userland::exit(if result == u64::MAX - 1 { 1 } else { 0 })
}
