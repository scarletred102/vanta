#![no_std]

use vanta_abi::CapabilityId;

pub const MAX_IPC_PAYLOAD: usize = 256;
pub const MAX_SERVICES: usize = 16;
pub const MAX_AUDIT_EVENTS: usize = 64;

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
}
