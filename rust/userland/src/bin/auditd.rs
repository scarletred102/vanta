#![no_std]
#![no_main]

use vanta_abi::CapabilityId;
use vanta_services::{ServiceId, ServiceOperation, ServiceRequest, ServiceResponse};

const SEND_FD: u64 = 5;
const RECEIVE_FD: u64 = 6;
const AUTHORITY: CapabilityId = CapabilityId::from_parts(5, 11);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let audit_fd = vanta_userland::open(
        b"/home/vanta/service-audit.log",
        vanta_userland::OPEN_CREATE | vanta_userland::OPEN_APPEND,
    );
    if audit_fd == u64::MAX || audit_fd == u64::MAX - 1 {
        vanta_userland::exit(1)
    }
    let mut bytes = [0_u8; vanta_services::MAX_IPC_PAYLOAD];
    loop {
        let length = vanta_userland::ipc_recv(RECEIVE_FD, &mut bytes);
        if length != vanta_services::MAX_IPC_PAYLOAD as u64 {
            vanta_userland::close(audit_fd);
            vanta_userland::exit(2)
        }
        let Ok(request) = ServiceRequest::decode(&bytes) else {
            vanta_userland::close(audit_fd);
            vanta_userland::exit(3)
        };
        if request.service != ServiceId::Security || request.authority != AUTHORITY {
            vanta_userland::close(audit_fd);
            vanta_userland::exit(4)
        }
        match request.operation {
            ServiceOperation::Audit => {
                if vanta_userland::write(audit_fd, request.payload()) == u64::MAX - 1 {
                    vanta_userland::close(audit_fd);
                    vanta_userland::exit(5)
                }
            }
            ServiceOperation::Shutdown => {
                let response =
                    ServiceResponse::success(request.request_id, ServiceId::Security, 1, AUTHORITY)
                        .with_payload(b"drained")
                        .unwrap()
                        .encode();
                if vanta_userland::ipc_send(SEND_FD, &response) == u64::MAX - 1 {
                    vanta_userland::close(audit_fd);
                    vanta_userland::exit(6)
                }
                vanta_userland::close(audit_fd);
                vanta_userland::exit(0)
            }
            _ => {
                vanta_userland::close(audit_fd);
                vanta_userland::exit(7)
            }
        }
    }
}
