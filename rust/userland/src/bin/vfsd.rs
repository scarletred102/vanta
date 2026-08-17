#![no_std]
#![no_main]

use vanta_abi::CapabilityId;
use vanta_services::{ServiceId, ServiceOperation, ServiceRequest, ServiceResponse};

const SEND_FD: u64 = 3;
const RECEIVE_FD: u64 = 4;
const AUTHORITY: CapabilityId = CapabilityId::from_parts(4, 10);

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
            ServiceOperation::Register => response(request.request_id, 2, b"registered"),
            ServiceOperation::Discover => response(request.request_id, 2, b"vfsd"),
            ServiceOperation::ReadFile => read_file(&request),
            ServiceOperation::Healthy => response(request.request_id, 2, b"upgraded"),
            _ => vanta_userland::exit(4),
        };
        let encoded = response.encode();
        if vanta_userland::ipc_send(SEND_FD, &encoded) == u64::MAX - 1 {
            vanta_userland::exit(5)
        }
        if request.operation == ServiceOperation::Healthy {
            vanta_userland::exit(0)
        }
    }
}

fn read_file(request: &ServiceRequest) -> ServiceResponse {
    if request.payload() != b"/etc/config" {
        return ServiceResponse::error(
            request.request_id,
            ServiceId::Vfs,
            vanta_services::ServiceError::NotFound,
        );
    }
    let fd = vanta_userland::open(b"/etc/config", vanta_userland::OPEN_READ);
    if fd == u64::MAX - 1 {
        return ServiceResponse::error(
            request.request_id,
            ServiceId::Vfs,
            vanta_services::ServiceError::Io,
        );
    }
    let mut contents = [0_u8; vanta_services::MAX_RESPONSE_PAYLOAD];
    let length = vanta_userland::read(fd, &mut contents);
    vanta_userland::close(fd);
    if length > vanta_services::MAX_RESPONSE_PAYLOAD as u64 {
        return ServiceResponse::error(
            request.request_id,
            ServiceId::Vfs,
            vanta_services::ServiceError::Io,
        );
    }
    response(request.request_id, 2, &contents[..length as usize])
}

fn response(request_id: u64, generation: u64, payload: &[u8]) -> ServiceResponse {
    ServiceResponse::success(request_id, ServiceId::Vfs, generation, AUTHORITY)
        .with_payload(payload)
        .unwrap()
}
