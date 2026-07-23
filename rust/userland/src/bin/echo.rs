#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut wrote = false;
    for index in 1..8 {
        let Some(argument) = vanta_userland::arg(index) else {
            break;
        };
        if wrote {
            vanta_userland::write(1, b" ");
        }
        vanta_userland::write(1, argument);
        wrote = true;
    }
    vanta_userland::write(1, b"\n");
    vanta_userland::exit(0)
}
