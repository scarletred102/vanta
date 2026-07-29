#![no_std]

use vanta_abi::CapabilityId;

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceId {
    Process = 1,
    Vfs = 2,
    Network = 3,
    Device = 4,
    Display = 5,
    Audio = 6,
    Input = 7,
    Package = 8,
    Security = 9,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    InvalidRequest = 1,
    AccessDenied = 2,
    NotFound = 3,
    Io = 4,
    Unsupported = 5,
    ServiceUnavailable = 6,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceRequestHeader {
    pub service: ServiceId,
    pub operation: u16,
    pub request_id: u64,
    pub authority: CapabilityId,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceResponseHeader {
    pub request_id: u64,
    pub result: i32,
}

impl ServiceResponseHeader {
    pub const fn success(request_id: u64) -> Self {
        Self {
            request_id,
            result: 0,
        }
    }

    pub const fn error(request_id: u64, error: ServiceError) -> Self {
        Self {
            request_id,
            result: -(error as i32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_errors_are_stable_negative_results() {
        assert_eq!(ServiceResponseHeader::success(7).result, 0);
        assert_eq!(
            ServiceResponseHeader::error(7, ServiceError::ServiceUnavailable).result,
            -6
        );
    }

    #[test]
    fn requests_carry_authority_explicitly() {
        let request = ServiceRequestHeader {
            service: ServiceId::Vfs,
            operation: 1,
            request_id: 42,
            authority: CapabilityId::from_parts(3, 9),
        };
        assert_eq!(request.authority.slot(), 3);
        assert_eq!(request.authority.generation(), 9);
    }
}
