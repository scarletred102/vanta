#![no_std]

use vanta_abi::CapabilityId;

pub const MAX_IPC_PAYLOAD: usize = 256;
pub const MAX_SERVICES: usize = 16;
pub const MAX_AUDIT_EVENTS: usize = 64;
const REQUEST_PAYLOAD_OFFSET: usize = 23;
const RESPONSE_PAYLOAD_OFFSET: usize = 34;
pub const MAX_REQUEST_PAYLOAD: usize = MAX_IPC_PAYLOAD - REQUEST_PAYLOAD_OFFSET;
pub const MAX_RESPONSE_PAYLOAD: usize = MAX_IPC_PAYLOAD - RESPONSE_PAYLOAD_OFFSET;

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
    QueueFull = 7,
    Revoked = 8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceOperation {
    Register = 1,
    Discover = 2,
    Healthy = 3,
    Crash = 4,
    ReadFile = 5,
    Audit = 6,
    Shutdown = 7,
}

impl ServiceOperation {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Register),
            2 => Some(Self::Discover),
            3 => Some(Self::Healthy),
            4 => Some(Self::Crash),
            5 => Some(Self::ReadFile),
            6 => Some(Self::Audit),
            7 => Some(Self::Shutdown),
            _ => None,
        }
    }
}

impl ServiceId {
    fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Process),
            2 => Some(Self::Vfs),
            3 => Some(Self::Network),
            4 => Some(Self::Device),
            5 => Some(Self::Display),
            6 => Some(Self::Audio),
            7 => Some(Self::Input),
            8 => Some(Self::Package),
            9 => Some(Self::Security),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceRequest {
    pub operation: ServiceOperation,
    pub service: ServiceId,
    pub request_id: u64,
    pub authority: CapabilityId,
    pub payload: [u8; MAX_REQUEST_PAYLOAD],
    pub payload_len: u16,
}

impl ServiceRequest {
    pub const fn empty(
        operation: ServiceOperation,
        service: ServiceId,
        request_id: u64,
        authority: CapabilityId,
    ) -> Self {
        Self {
            operation,
            service,
            request_id,
            authority,
            payload: [0; MAX_REQUEST_PAYLOAD],
            payload_len: 0,
        }
    }

    pub fn with_payload(mut self, payload: &[u8]) -> Result<Self, ServiceError> {
        if payload.len() > MAX_REQUEST_PAYLOAD {
            return Err(ServiceError::InvalidRequest);
        }
        self.payload[..payload.len()].copy_from_slice(payload);
        self.payload_len = payload.len() as u16;
        Ok(self)
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }

    pub fn encode(&self) -> [u8; MAX_IPC_PAYLOAD] {
        let mut bytes = [0; MAX_IPC_PAYLOAD];
        bytes[0] = b'V';
        bytes[1] = b'S';
        bytes[2] = self.operation as u8;
        bytes[3..5].copy_from_slice(&(self.service as u16).to_le_bytes());
        bytes[5..13].copy_from_slice(&self.request_id.to_le_bytes());
        bytes[13..21].copy_from_slice(&self.authority.raw().to_le_bytes());
        bytes[21..23].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[REQUEST_PAYLOAD_OFFSET..REQUEST_PAYLOAD_OFFSET + self.payload_len as usize]
            .copy_from_slice(self.payload());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ServiceError> {
        if bytes.len() != MAX_IPC_PAYLOAD || bytes[0..2] != [b'V', b'S'] {
            return Err(ServiceError::InvalidRequest);
        }
        let payload_len = u16::from_le_bytes([bytes[21], bytes[22]]) as usize;
        if payload_len > MAX_REQUEST_PAYLOAD {
            return Err(ServiceError::InvalidRequest);
        }
        let mut payload = [0; MAX_REQUEST_PAYLOAD];
        payload[..payload_len]
            .copy_from_slice(&bytes[REQUEST_PAYLOAD_OFFSET..REQUEST_PAYLOAD_OFFSET + payload_len]);
        Ok(Self {
            operation: ServiceOperation::from_raw(bytes[2]).ok_or(ServiceError::InvalidRequest)?,
            service: ServiceId::from_raw(u16::from_le_bytes([bytes[3], bytes[4]]))
                .ok_or(ServiceError::InvalidRequest)?,
            request_id: u64::from_le_bytes(bytes[5..13].try_into().unwrap()),
            authority: CapabilityId::from_raw(u64::from_le_bytes(
                bytes[13..21].try_into().unwrap(),
            )),
            payload,
            payload_len: payload_len as u16,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceResponse {
    pub request_id: u64,
    pub service: ServiceId,
    pub generation: u64,
    pub authority: CapabilityId,
    pub result: i32,
    pub payload: [u8; MAX_RESPONSE_PAYLOAD],
    pub payload_len: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceEndpoint {
    pub service: ServiceId,
    pub generation: u64,
    pub authority: CapabilityId,
}

impl ServiceResponse {
    pub const fn success(
        request_id: u64,
        service: ServiceId,
        generation: u64,
        authority: CapabilityId,
    ) -> Self {
        Self {
            request_id,
            service,
            generation,
            authority,
            result: 0,
            payload: [0; MAX_RESPONSE_PAYLOAD],
            payload_len: 0,
        }
    }

    pub const fn error(request_id: u64, service: ServiceId, error: ServiceError) -> Self {
        Self {
            request_id,
            service,
            generation: 0,
            authority: CapabilityId::INVALID,
            result: -(error as i32),
            payload: [0; MAX_RESPONSE_PAYLOAD],
            payload_len: 0,
        }
    }

    pub fn with_payload(mut self, payload: &[u8]) -> Result<Self, ServiceError> {
        if payload.len() > MAX_RESPONSE_PAYLOAD {
            return Err(ServiceError::InvalidRequest);
        }
        self.payload[..payload.len()].copy_from_slice(payload);
        self.payload_len = payload.len() as u16;
        Ok(self)
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }

    pub fn encode(&self) -> [u8; MAX_IPC_PAYLOAD] {
        let mut bytes = [0; MAX_IPC_PAYLOAD];
        bytes[0] = b'V';
        bytes[1] = b'R';
        bytes[2..6].copy_from_slice(&self.result.to_le_bytes());
        bytes[6..14].copy_from_slice(&self.request_id.to_le_bytes());
        bytes[14..16].copy_from_slice(&(self.service as u16).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.generation.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.authority.raw().to_le_bytes());
        bytes[32..34].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[RESPONSE_PAYLOAD_OFFSET..RESPONSE_PAYLOAD_OFFSET + self.payload_len as usize]
            .copy_from_slice(self.payload());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ServiceError> {
        if bytes.len() != MAX_IPC_PAYLOAD || bytes[0..2] != [b'V', b'R'] {
            return Err(ServiceError::InvalidRequest);
        }
        let payload_len = u16::from_le_bytes([bytes[32], bytes[33]]) as usize;
        if payload_len > MAX_RESPONSE_PAYLOAD {
            return Err(ServiceError::InvalidRequest);
        }
        let mut payload = [0; MAX_RESPONSE_PAYLOAD];
        payload[..payload_len].copy_from_slice(
            &bytes[RESPONSE_PAYLOAD_OFFSET..RESPONSE_PAYLOAD_OFFSET + payload_len],
        );
        Ok(Self {
            request_id: u64::from_le_bytes(bytes[6..14].try_into().unwrap()),
            service: ServiceId::from_raw(u16::from_le_bytes([bytes[14], bytes[15]]))
                .ok_or(ServiceError::InvalidRequest)?,
            generation: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            authority: CapabilityId::from_raw(u64::from_le_bytes(
                bytes[24..32].try_into().unwrap(),
            )),
            result: i32::from_le_bytes(bytes[2..6].try_into().unwrap()),
            payload,
            payload_len: payload_len as u16,
        })
    }
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcFrame {
    pub header: ServiceRequestHeader,
    pub payload_len: u16,
    pub payload: [u8; MAX_IPC_PAYLOAD],
}

impl IpcFrame {
    pub const fn empty(header: ServiceRequestHeader) -> Self {
        Self {
            header,
            payload_len: 0,
            payload: [0; MAX_IPC_PAYLOAD],
        }
    }

    pub fn with_payload(mut self, payload: &[u8]) -> Result<Self, ServiceError> {
        if payload.len() > MAX_IPC_PAYLOAD {
            return Err(ServiceError::InvalidRequest);
        }
        self.payload[..payload.len()].copy_from_slice(payload);
        self.payload_len = payload.len() as u16;
        Ok(self)
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Registered,
    Running,
    Crashed,
    Restarting,
    Revoked,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditKind {
    Registered,
    Started,
    Request,
    Completed,
    Crashed,
    Restarted,
    Revoked,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub sequence: u64,
    pub service: ServiceId,
    pub kind: AuditKind,
    pub request_id: u64,
    pub authority: CapabilityId,
    pub result: i32,
}

#[derive(Clone, Copy)]
struct ServiceSlot {
    id: ServiceId,
    authority: CapabilityId,
    generation: u64,
    state: ServiceState,
    restart_count: u32,
}

pub struct ServiceSupervisor {
    slots: [Option<ServiceSlot>; MAX_SERVICES],
    audit: [Option<AuditEvent>; MAX_AUDIT_EVENTS],
    audit_next: usize,
    sequence: u64,
}

impl ServiceSupervisor {
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_SERVICES],
            audit: [None; MAX_AUDIT_EVENTS],
            audit_next: 0,
            sequence: 0,
        }
    }

    pub fn register(&mut self, id: ServiceId, authority: CapabilityId) -> Result<(), ServiceError> {
        if self.find(id).is_some() {
            return Err(ServiceError::InvalidRequest);
        }
        let index = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(ServiceError::QueueFull)?;
        self.slots[index] = Some(ServiceSlot {
            id,
            authority,
            generation: 1,
            state: ServiceState::Registered,
            restart_count: 0,
        });
        self.record(id, AuditKind::Registered, 0, authority, 0);
        Ok(())
    }

    pub fn start(&mut self, id: ServiceId) -> Result<(), ServiceError> {
        let slot = self.slot_mut(id)?;
        if slot.state == ServiceState::Revoked {
            return Err(ServiceError::Revoked);
        }
        slot.state = ServiceState::Running;
        let authority = slot.authority;
        self.record(id, AuditKind::Started, 0, authority, 0);
        Ok(())
    }

    pub fn dispatch(&mut self, frame: &IpcFrame) -> ServiceResponseHeader {
        let Ok(slot) = self.slot_mut(frame.header.service) else {
            return ServiceResponseHeader::error(frame.header.request_id, ServiceError::NotFound);
        };
        if slot.state == ServiceState::Revoked || slot.authority != frame.header.authority {
            return ServiceResponseHeader::error(frame.header.request_id, ServiceError::Revoked);
        }
        if slot.state != ServiceState::Running {
            return ServiceResponseHeader::error(
                frame.header.request_id,
                ServiceError::ServiceUnavailable,
            );
        }
        self.record(
            frame.header.service,
            AuditKind::Request,
            frame.header.request_id,
            frame.header.authority,
            0,
        );
        let response = ServiceResponseHeader::success(frame.header.request_id);
        self.record(
            frame.header.service,
            AuditKind::Completed,
            frame.header.request_id,
            frame.header.authority,
            response.result,
        );
        response
    }

    pub fn crash_and_restart(&mut self, id: ServiceId) -> Result<(), ServiceError> {
        let authority = {
            let slot = self.slot_mut(id)?;
            if slot.state == ServiceState::Revoked {
                return Err(ServiceError::Revoked);
            }
            slot.state = ServiceState::Crashed;
            slot.authority
        };
        self.record(id, AuditKind::Crashed, 0, authority, -1);
        let slot = self.slot_mut(id)?;
        slot.state = ServiceState::Restarting;
        slot.restart_count = slot.restart_count.saturating_add(1);
        slot.state = ServiceState::Running;
        self.record(id, AuditKind::Restarted, 0, authority, 0);
        Ok(())
    }

    pub fn revoke(&mut self, id: ServiceId) -> Result<CapabilityId, ServiceError> {
        let slot = self.slot_mut(id)?;
        let old = slot.authority;
        slot.state = ServiceState::Revoked;
        self.record(id, AuditKind::Revoked, 0, old, 0);
        Ok(old)
    }

    pub fn state(&self, id: ServiceId) -> Option<ServiceState> {
        self.find(id).map(|slot| slot.state)
    }

    pub fn restart_count(&self, id: ServiceId) -> Option<u32> {
        self.find(id).map(|slot| slot.restart_count)
    }

    pub fn discover(&self, id: ServiceId) -> Result<ServiceEndpoint, ServiceError> {
        let slot = self.find(id).ok_or(ServiceError::NotFound)?;
        if slot.state == ServiceState::Revoked {
            return Err(ServiceError::Revoked);
        }
        Ok(ServiceEndpoint {
            service: slot.id,
            generation: slot.generation,
            authority: slot.authority,
        })
    }

    pub fn upgrade(&mut self, id: ServiceId, authority: CapabilityId) -> Result<(), ServiceError> {
        let slot = self.slot_mut(id)?;
        if slot.state == ServiceState::Revoked {
            return Err(ServiceError::Revoked);
        }
        slot.authority = authority;
        slot.generation = slot.generation.saturating_add(1);
        slot.state = ServiceState::Running;
        Ok(())
    }

    pub fn audit_events(&self) -> impl Iterator<Item = AuditEvent> + '_ {
        self.audit.iter().filter_map(|event| *event)
    }

    fn find(&self, id: ServiceId) -> Option<&ServiceSlot> {
        self.slots.iter().flatten().find(|slot| slot.id == id)
    }

    fn slot_mut(&mut self, id: ServiceId) -> Result<&mut ServiceSlot, ServiceError> {
        self.slots
            .iter_mut()
            .flatten()
            .find(|slot| slot.id == id)
            .ok_or(ServiceError::NotFound)
    }

    fn record(
        &mut self,
        service: ServiceId,
        kind: AuditKind,
        request_id: u64,
        authority: CapabilityId,
        result: i32,
    ) {
        self.sequence = self.sequence.wrapping_add(1);
        self.audit[self.audit_next] = Some(AuditEvent {
            sequence: self.sequence,
            service,
            kind,
            request_id,
            authority,
            result,
        });
        self.audit_next = (self.audit_next + 1) % MAX_AUDIT_EVENTS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> CapabilityId {
        CapabilityId::from_parts(3, 9)
    }

    #[test]
    fn bounded_frames_reject_oversized_payloads() {
        let header = ServiceRequestHeader {
            service: ServiceId::Vfs,
            operation: 1,
            request_id: 42,
            authority: authority(),
        };
        assert!(IpcFrame::empty(header)
            .with_payload(&[0; MAX_IPC_PAYLOAD])
            .is_ok());
        assert_eq!(
            IpcFrame::empty(header).with_payload(&[0; MAX_IPC_PAYLOAD + 1]),
            Err(ServiceError::InvalidRequest)
        );
    }

    #[test]
    fn crashes_restart_without_losing_service_authority() {
        let mut supervisor = ServiceSupervisor::new();
        supervisor.register(ServiceId::Vfs, authority()).unwrap();
        supervisor.start(ServiceId::Vfs).unwrap();
        supervisor.crash_and_restart(ServiceId::Vfs).unwrap();
        assert_eq!(
            supervisor.state(ServiceId::Vfs),
            Some(ServiceState::Running)
        );
        assert_eq!(supervisor.restart_count(ServiceId::Vfs), Some(1));
        assert!(supervisor
            .audit_events()
            .any(|event| event.kind == AuditKind::Crashed));
        let frame = IpcFrame::empty(ServiceRequestHeader {
            service: ServiceId::Vfs,
            operation: 1,
            request_id: 7,
            authority: authority(),
        });
        assert_eq!(supervisor.dispatch(&frame).result, 0);
    }

    #[test]
    fn revocation_rejects_stale_requests() {
        let mut supervisor = ServiceSupervisor::new();
        supervisor
            .register(ServiceId::Security, authority())
            .unwrap();
        supervisor.start(ServiceId::Security).unwrap();
        supervisor.revoke(ServiceId::Security).unwrap();
        let frame = IpcFrame::empty(ServiceRequestHeader {
            service: ServiceId::Security,
            operation: 1,
            request_id: 8,
            authority: authority(),
        });
        assert_eq!(
            supervisor.dispatch(&frame).result,
            -(ServiceError::Revoked as i32)
        );
    }

    #[test]
    fn wire_frames_round_trip_identity_and_payload() {
        let authority = authority();
        let request =
            ServiceRequest::empty(ServiceOperation::Register, ServiceId::Vfs, 41, authority)
                .with_payload(b"vfsd")
                .unwrap();
        assert_eq!(ServiceRequest::decode(&request.encode()), Ok(request));

        let response = ServiceResponse::success(41, ServiceId::Vfs, 2, authority)
            .with_payload(b"discovered")
            .unwrap();
        assert_eq!(ServiceResponse::decode(&response.encode()), Ok(response));
    }

    #[test]
    fn discovery_changes_generation_on_upgrade_and_rejects_revoke() {
        let old = authority();
        let new = CapabilityId::from_parts(4, 10);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.register(ServiceId::Vfs, old).unwrap();
        assert_eq!(supervisor.discover(ServiceId::Vfs).unwrap().generation, 1);
        supervisor.start(ServiceId::Vfs).unwrap();
        supervisor.upgrade(ServiceId::Vfs, new).unwrap();
        let endpoint = supervisor.discover(ServiceId::Vfs).unwrap();
        assert_eq!(endpoint.generation, 2);
        assert_eq!(endpoint.authority, new);
        supervisor.revoke(ServiceId::Vfs).unwrap();
        assert_eq!(
            supervisor.discover(ServiceId::Vfs),
            Err(ServiceError::Revoked)
        );
    }
}
