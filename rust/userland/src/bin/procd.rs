#![no_std]
#![no_main]

use vanta_abi::CapabilityId;
use vanta_services::{ServiceId, ServiceOperation, ServiceRequest, ServiceResponse};

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;
const AUDIT_SEND_FD: u64 = 5;
const AUDIT_RECEIVE_FD: u64 = 6;
const OLD_AUTHORITY: CapabilityId = CapabilityId::from_parts(3, 9);
const NEW_AUTHORITY: CapabilityId = CapabilityId::from_parts(4, 10);
const AUDIT_AUTHORITY: CapabilityId = CapabilityId::from_parts(5, 11);

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
    let Some((audit_sender, audit_receiver)) = vanta_userland::ipc_pair() else {
        vanta_userland::write(2, b"[procd] audit ipc pair failed\n");
        vanta_userland::exit(3)
    };
    if audit_sender != AUDIT_SEND_FD || audit_receiver != AUDIT_RECEIVE_FD {
        vanta_userland::write(2, b"[procd] audit descriptor layout failed\n");
        vanta_userland::exit(4)
    }
    let auditd = vanta_userland::spawn(b"/bin/auditd");
    if auditd == u64::MAX {
        vanta_userland::write(2, b"[procd] audit service launch failed\n");
        vanta_userland::exit(5)
    }

    let first = vanta_userland::spawn(b"/bin/service-test");
    if first == u64::MAX || send(sender, ServiceOperation::Register, 1, OLD_AUTHORITY) {
        fail(
            audit_sender,
            auditd,
            6,
            b"[procd] first service launch failed\n",
        )
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
        fail(audit_sender, auditd, 7, b"[procd] registration failed\n")
    }
    vanta_userland::write(1, b"[procd] service registered\n");
    audit(audit_sender, 101, b"registered\n");

    if send(sender, ServiceOperation::Crash, 2, OLD_AUTHORITY) {
        fail(audit_sender, auditd, 8, b"[procd] crash request failed\n")
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
        fail(
            audit_sender,
            auditd,
            9,
            b"[procd] crash containment failed\n",
        )
    }
    vanta_userland::write(1, b"[procd] service crashed\n");
    audit(audit_sender, 102, b"crashed\n");

    let second = vanta_userland::spawn(b"/bin/vfsd");
    if second == u64::MAX || send(sender, ServiceOperation::Register, 3, NEW_AUTHORITY) {
        fail(audit_sender, auditd, 10, b"[procd] upgrade launch failed\n")
    }
    if !expect(
        receiver,
        &mut response_bytes,
        3,
        NEW_AUTHORITY,
        2,
        b"registered",
    ) {
        fail(
            audit_sender,
            auditd,
            11,
            b"[procd] upgrade registration failed\n",
        )
    }
    vanta_userland::write(1, b"[procd] service upgraded\n");
    audit(audit_sender, 103, b"upgraded\n");

    if send(sender, ServiceOperation::Discover, 4, NEW_AUTHORITY)
        || !expect(receiver, &mut response_bytes, 4, NEW_AUTHORITY, 2, b"vfsd")
    {
        fail(
            audit_sender,
            auditd,
            12,
            b"[procd] service discovery failed\n",
        )
    }
    vanta_userland::write(1, b"[procd] service discovered\n");
    audit(audit_sender, 104, b"discovered\n");

    if send_payload(
        sender,
        ServiceOperation::ReadFile,
        5,
        OLD_AUTHORITY,
        b"/etc/config",
    ) || !expect_error(
        receiver,
        &mut response_bytes,
        5,
        vanta_services::ServiceError::Revoked,
    ) {
        fail(
            audit_sender,
            auditd,
            13,
            b"[procd] stale authority was accepted\n",
        )
    }
    vanta_userland::write(1, b"[procd] stale service authority revoked\n");
    audit(audit_sender, 105, b"stale-revoked\n");

    if send_payload(
        sender,
        ServiceOperation::ReadFile,
        6,
        NEW_AUTHORITY,
        b"/etc/config",
    ) || !expect(
        receiver,
        &mut response_bytes,
        6,
        NEW_AUTHORITY,
        2,
        b"vanta-vfs-syscall\n",
    ) {
        fail(audit_sender, auditd, 14, b"[procd] vfs backend failed\n")
    }
    vanta_userland::write(1, b"[procd] vfs backend passed\n");
    audit(audit_sender, 106, b"backend-read\n");

    if send(sender, ServiceOperation::Healthy, 7, NEW_AUTHORITY)
        || !expect(
            receiver,
            &mut response_bytes,
            7,
            NEW_AUTHORITY,
            2,
            b"upgraded",
        )
        || vanta_userland::wait(second) != 0
    {
        fail(
            audit_sender,
            auditd,
            15,
            b"[procd] upgrade containment failed\n",
        )
    }

    if vanta_userland::ipc_revoke(sender) == u64::MAX - 1
        || vanta_userland::ipc_send(sender, &[0; vanta_services::MAX_IPC_PAYLOAD]) != u64::MAX - 1
    {
        fail(audit_sender, auditd, 16, b"[procd] revocation failed\n")
    }
    vanta_userland::write(1, b"[procd] service authority revoked\n");
    audit(audit_sender, 107, b"revoked\n");
    let audit_shutdown_sent = send_to(
        audit_sender,
        ServiceId::Security,
        ServiceOperation::Shutdown,
        108,
        AUDIT_AUTHORITY,
        &[],
    );
    vanta_userland::yield_now();
    let audit_ack = expect_audit_ack(audit_receiver, 108, &mut response_bytes);
    if audit_shutdown_sent || !audit_ack {
        vanta_userland::write(2, b"[procd] audit service shutdown failed\n");
        vanta_userland::close(audit_sender);
        vanta_userland::close(audit_receiver);
        vanta_userland::exit(17)
    }
    vanta_userland::close(sender);
    vanta_userland::close(receiver);
    vanta_userland::close(audit_sender);
    vanta_userland::close(audit_receiver);
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
    send_to(
        fd,
        ServiceId::Vfs,
        operation,
        request_id,
        authority,
        payload,
    )
}

fn send_to(
    fd: u64,
    service: ServiceId,
    operation: ServiceOperation,
    request_id: u64,
    authority: CapabilityId,
    payload: &[u8],
) -> bool {
    let Ok(request) =
        ServiceRequest::empty(operation, service, request_id, authority).with_payload(payload)
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

fn expect_error(
    fd: u64,
    buffer: &mut [u8; vanta_services::MAX_IPC_PAYLOAD],
    request_id: u64,
    error: vanta_services::ServiceError,
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
        && response.result == -(error as i32)
        && response.authority == CapabilityId::INVALID
}

fn expect_audit_ack(
    fd: u64,
    request_id: u64,
    buffer: &mut [u8; vanta_services::MAX_IPC_PAYLOAD],
) -> bool {
    let length = receive(fd, buffer);
    if length != vanta_services::MAX_IPC_PAYLOAD as u64 {
        return false;
    }
    let Ok(response) = ServiceResponse::decode(buffer) else {
        return false;
    };
    response.request_id == request_id
        && response.service == ServiceId::Security
        && response.authority == AUDIT_AUTHORITY
        && response.result == 0
        && response.payload() == b"drained"
}

fn receive(fd: u64, buffer: &mut [u8]) -> u64 {
    vanta_userland::ipc_recv(fd, buffer)
}

fn fail(audit_sender: u64, auditd: u64, code: u64, message: &[u8]) -> ! {
    vanta_userland::write(2, message);
    let _ = send_to(
        audit_sender,
        ServiceId::Security,
        ServiceOperation::Shutdown,
        255,
        AUDIT_AUTHORITY,
        &[],
    );
    let _ = auditd;
    vanta_userland::close(audit_sender);
    vanta_userland::exit(code)
}

fn audit(fd: u64, request_id: u64, event: &[u8]) {
    let _ = send_to(
        fd,
        ServiceId::Security,
        ServiceOperation::Audit,
        request_id,
        AUDIT_AUTHORITY,
        event,
    );
}
