#![no_std]
#![no_main]

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let Some((sender, receiver)) = vanta_userland::ipc_pair() else {
        vanta_userland::write(2, b"[procd] ipc pair failed\n");
        vanta_userland::exit(1)
    };
    if sender != SEND_FD || receiver != RECEIVE_FD {
        vanta_userland::write(2, b"[procd] descriptor layout failed\n");
        vanta_userland::exit(2)
    }

    let first = vanta_userland::spawn(b"/bin/service-test");
    if first == u64::MAX || vanta_userland::ipc_send(sender, b"crash") == u64::MAX - 1 {
        vanta_userland::write(2, b"[procd] first service launch failed\n");
        vanta_userland::exit(3)
    }
    let mut response = [0_u8; 64];
    let response_len = receive(receiver, &mut response);
    let first_status = vanta_userland::wait(first);
    if response_len != b"audit:service-crash".len() as u64
        || &response[..response_len as usize] != b"audit:service-crash"
        || first_status == 0
    {
        vanta_userland::write(2, b"[procd] crash containment failed\n");
        vanta_userland::exit(4)
    }
    vanta_userland::write(1, b"[procd] audit service crashed\n");

    let second = vanta_userland::spawn(b"/bin/service-test");
    if second == u64::MAX || vanta_userland::ipc_send(sender, b"healthy") == u64::MAX - 1 {
        vanta_userland::write(2, b"[procd] restart launch failed\n");
        vanta_userland::exit(5)
    }
    let response_len = receive(receiver, &mut response);
    let second_status = vanta_userland::wait(second);
    if response_len != b"audit:service-restarted".len() as u64
        || &response[..response_len as usize] != b"audit:service-restarted"
        || second_status != 0
    {
        vanta_userland::write(2, b"[procd] restart containment failed\n");
        vanta_userland::exit(6)
    }
    vanta_userland::write(1, b"[procd] audit service restarted\n");

    if vanta_userland::ipc_revoke(sender) == u64::MAX - 1
        || vanta_userland::ipc_send(sender, b"revoked") != u64::MAX - 1
    {
        vanta_userland::write(2, b"[procd] revocation failed\n");
        vanta_userland::exit(7)
    }
    vanta_userland::write(1, b"[procd] audit authority revoked\n");
    vanta_userland::close(sender);
    vanta_userland::close(receiver);
    vanta_userland::write(1, b"[procd] Gate B IPC supervisor passed\n");
    vanta_userland::exit(0)
}

fn receive(fd: u64, buffer: &mut [u8]) -> u64 {
    loop {
        let count = vanta_userland::ipc_recv(fd, buffer);
        if count != 0 {
            return count;
        }
        vanta_userland::yield_now();
    }
}
