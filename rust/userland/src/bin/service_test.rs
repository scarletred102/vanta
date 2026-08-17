#![no_std]
#![no_main]

use vanta_abi::CapabilityId;
use vanta_services::{ServiceId, ServiceOperation, ServiceRequest, ServiceResponse};

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;
const AUTHORITY: CapabilityId = CapabilityId::from_parts(3, 9);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut bytes = [0_u8; vanta_services::MAX_IPC_PAYLOAD];
    loop {
        let length = vanta_userland::ipc_recv(RECEIVE_FD, &mut bytes);
        if length != vanta_services::MAX_IPC_PAYLOAD as u64 {
            vanta_userland::exit(1)
        }
        let Ok(request) = ServiceRequest::decode(&bytes) else {
            vanta_userland::exit(2)
        };
        if request.service != ServiceId::Vfs || request.authority != AUTHORITY {
            vanta_userland::exit(3)
        }
        let response = match request.operation {
            ServiceOperation::Register => response(request.request_id, 1, b"registered"),
            ServiceOperation::Crash => response(request.request_id, 1, b"crashed"),
            _ => vanta_userland::exit(4),
        };
        let encoded = response.encode();
        if vanta_userland::ipc_send(SEND_FD, &encoded) == u64::MAX - 1 {
            vanta_userland::exit(5)
        }
        if request.operation == ServiceOperation::Crash {
            vanta_userland::exit(42)
        }
    }
}

fn response(request_id: u64, generation: u64, payload: &[u8]) -> ServiceResponse {
    ServiceResponse::success(request_id, ServiceId::Vfs, generation, AUTHORITY)
        .with_payload(payload)
        .unwrap()
}
