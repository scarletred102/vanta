#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let forbidden = vanta_userland::open(
        b"/etc/vanta-gate-forbidden",
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_TRUNCATE,
    );
    if forbidden != u64::MAX - 1 {
        if forbidden != u64::MAX {
            vanta_userland::close(forbidden);
        }
        vanta_userland::write(2, b"[native] developer gate: forbidden write allowed\n");
        vanta_userland::exit(1);
    }

    let authorized_path = b"/home/vanta/native-gate-authorized";
    let authorized = vanta_userland::open(
        authorized_path,
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_TRUNCATE,
    );
    if authorized == u64::MAX - 1 || authorized == u64::MAX {
        vanta_userland::write(2, b"[native] developer gate: authorized write failed\n");
        vanta_userland::exit(2);
    }
    let write_ok = vanta_userland::write(authorized, b"developer\n") == 10;
    let _ = vanta_userland::close(authorized);
    let remove_ok = vanta_userland::unlink(authorized_path) == 0;
    if !write_ok || !remove_ok {
        vanta_userland::write(2, b"[native] developer gate: cleanup failed\n");
        vanta_userland::exit(3);
    }

    vanta_userland::write(1, b"[native] developer authorization passed\n");
    vanta_userland::exit(0)
}
