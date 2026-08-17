#![no_std]
#![no_main]

use vanta_abi::CapabilityId;
use vanta_services::{ServiceId, ServiceOperation, ServiceRequest, ServiceResponse};

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;
const OLD_AUTHORITY: CapabilityId = CapabilityId::from_parts(3, 9);
const NEW_AUTHORITY: CapabilityId = CapabilityId::from_parts(4, 10);

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
    let audit_fd = vanta_userland::open(
        b"/home/vanta/service-audit.log",
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_APPEND,
    );
    if audit_fd == u64::MAX || audit_fd == u64::MAX - 1 {
        vanta_userland::write(2, b"[procd] audit log open failed\n");
        vanta_userland::exit(3)
    }

    let first = vanta_userland::spawn(b"/bin/service-test");
    if first == u64::MAX || send(sender, ServiceOperation::Register, 1, OLD_AUTHORITY) {
        fail(audit_fd, 4, b"[procd] first service launch failed\n")
    }
    let mut response_bytes = [0_u8; vanta_services::MAX_IPC_PAYLOAD];
    if !expect(
        receiver,
        &mut response_bytes,
        1,
        OLD_AUTHORITY,
        1,
        b"registered",
    ) {
        fail(audit_fd, 5, b"[procd] registration failed\n")
    }
    vanta_userland::write(1, b"[procd] service registered\n");
    audit(audit_fd, b"registered\n");

    if send(sender, ServiceOperation::Crash, 2, OLD_AUTHORITY) {
        fail(audit_fd, 6, b"[procd] crash request failed\n")
    }
    if !expect(
        receiver,
        &mut response_bytes,
        2,
        OLD_AUTHORITY,
        1,
        b"crashed",
    ) || vanta_userland::wait(first) == 0
    {
        fail(audit_fd, 7, b"[procd] crash containment failed\n")
    }
    vanta_userland::write(1, b"[procd] service crashed\n");
    audit(audit_fd, b"crashed\n");

    let second = vanta_userland::spawn(b"/bin/vfsd");
    if second == u64::MAX || send(sender, ServiceOperation::Register, 3, NEW_AUTHORITY) {
        fail(audit_fd, 8, b"[procd] upgrade launch failed\n")
    }
    if !expect(
        receiver,
        &mut response_bytes,
        3,
        NEW_AUTHORITY,
        2,
        b"registered",
    ) {
        fail(audit_fd, 9, b"[procd] upgrade registration failed\n")
    }
    vanta_userland::write(1, b"[procd] service upgraded\n");
    audit(audit_fd, b"upgraded\n");

    if send(sender, ServiceOperation::Discover, 4, NEW_AUTHORITY)
        || !expect(receiver, &mut response_bytes, 4, NEW_AUTHORITY, 2, b"vfsd")
    {
        fail(audit_fd, 10, b"[procd] service discovery failed\n")
    }
    vanta_userland::write(1, b"[procd] service discovered\n");
    audit(audit_fd, b"discovered\n");

    if send_payload(
        sender,
        ServiceOperation::ReadFile,
        5,
        NEW_AUTHORITY,
        b"/etc/config",
    ) || !expect(
        receiver,
        &mut response_bytes,
        5,
        NEW_AUTHORITY,
        2,
        b"vanta-vfs-syscall\n",
    ) {
        fail(audit_fd, 11, b"[procd] vfs backend failed\n")
    }
    vanta_userland::write(1, b"[procd] vfs backend passed\n");
    audit(audit_fd, b"backend-read\n");

    if send(sender, ServiceOperation::Healthy, 6, NEW_AUTHORITY)
        || !expect(
            receiver,
            &mut response_bytes,
            6,
            NEW_AUTHORITY,
            2,
            b"upgraded",
        )
        || vanta_userland::wait(second) != 0
    {
        fail(audit_fd, 12, b"[procd] upgrade containment failed\n")
    }

    if vanta_userland::ipc_revoke(sender) == u64::MAX - 1
        || vanta_userland::ipc_send(sender, &[0; vanta_services::MAX_IPC_PAYLOAD]) != u64::MAX - 1
    {
        fail(audit_fd, 13, b"[procd] revocation failed\n")
    }
    vanta_userland::write(1, b"[procd] service authority revoked\n");
    audit(audit_fd, b"revoked\n");
    vanta_userland::close(sender);
    vanta_userland::close(receiver);
    vanta_userland::close(audit_fd);
    vanta_userland::write(1, b"[procd] Gate B IPC supervisor passed\n");
    vanta_userland::exit(0)
}

fn send(fd: u64, operation: ServiceOperation, request_id: u64, authority: CapabilityId) -> bool {
    send_payload(fd, operation, request_id, authority, &[])
}

fn send_payload(
    fd: u64,
    operation: ServiceOperation,
    request_id: u64,
    authority: CapabilityId,
    payload: &[u8],
) -> bool {
    let Ok(request) = ServiceRequest::empty(operation, ServiceId::Vfs, request_id, authority)
        .with_payload(payload)
    else {
        return true;
    };
    let bytes = request.encode();
    vanta_userland::ipc_send(fd, &bytes) == u64::MAX - 1
}

fn expect(
    fd: u64,
    buffer: &mut [u8; vanta_services::MAX_IPC_PAYLOAD],
    request_id: u64,
    authority: CapabilityId,
    generation: u64,
    payload: &[u8],
) -> bool {
    let length = receive(fd, buffer);
    if length != vanta_services::MAX_IPC_PAYLOAD as u64 {
        return false;
    }
    let Ok(response) = ServiceResponse::decode(buffer) else {
        return false;
    };
    response.request_id == request_id
        && response.service == ServiceId::Vfs
        && response.generation == generation
        && response.authority == authority
        && response.result == 0
        && response.payload() == payload
}

fn receive(fd: u64, buffer: &mut [u8]) -> u64 {
    vanta_userland::ipc_recv(fd, buffer)
}

fn fail(audit_fd: u64, code: u64, message: &[u8]) -> ! {
    vanta_userland::write(2, message);
    vanta_userland::close(audit_fd);
    vanta_userland::exit(code)
}

fn audit(fd: u64, event: &[u8]) {
    let _ = vanta_userland::write(fd, event);
}
