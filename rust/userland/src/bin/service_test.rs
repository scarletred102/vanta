#![no_std]
#![no_main]

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut request = [0_u8; 32];
    let length = loop {
        let count = vanta_userland::ipc_recv(RECEIVE_FD, &mut request);
        if count == u64::MAX - 1 {
            vanta_userland::write(2, b"[service-test] ipc receive failed\n");
            vanta_userland::exit(9)
        }
        if count != 0 {
            break count;
        }
        vanta_userland::yield_now();
    };
    if length == b"crash".len() as u64 && &request[..length as usize] == b"crash" {
        let _ = vanta_userland::ipc_send(SEND_FD, b"audit:service-crash");
        vanta_userland::exit(42)
    }
    if length == b"healthy".len() as u64 && &request[..length as usize] == b"healthy" {
        let _ = vanta_userland::ipc_send(SEND_FD, b"audit:service-restarted");
        vanta_userland::exit(0)
    }
    vanta_userland::exit(1)
}
