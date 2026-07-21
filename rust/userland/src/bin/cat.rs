#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut buffer = [0_u8; 128];
    loop {
        let count = vanta_userland::read(0, &mut buffer);
        if count == 0 || count == u64::MAX {
            vanta_userland::exit(0);
        }
        vanta_userland::write(1, &buffer[..count as usize]);
    }
}
