#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    vanta_userland::write(1, b"vanta init: starting /bin/vsh\n");
    let result = vanta_userland::exec(b"/bin/vsh");
    vanta_userland::write(2, b"vanta init: could not exec /bin/vsh\n");
    vanta_userland::exit(result)
}
