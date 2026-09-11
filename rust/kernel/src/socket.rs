//! BSD Socket Server Lifecycle and Data Structures for Vanta OS.
//!
//! Implements the 9-state TCP state machine (RFC 793 / RFC 1122),
//! syn_queue + accept_queue server connection management,
//! SYN Cookies (RFC 4987) under backlog saturation,
//! and TIME_WAIT reclamation (2*MSL) and port reuse.

#![allow(dead_code)]

use alloc::vec::Vec;
use crate::net::{Ipv4Address, MacAddress};

pub const SOMAXCONN: usize = 128;
pub const TCP_MSL_TICKS: u64 = 1000; // 1 second MSL (2*MSL = 2000 ticks = 2s)

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketType {
    Stream,
    Datagram,
    Raw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    Reset,
}

impl TcpState {
    pub fn is_active(&self) -> bool {
        !matches!(self, TcpState::Closed | TcpState::Reset)
    }

    pub fn can_send(&self) -> bool {
        matches!(self, TcpState::Established | TcpState::CloseWait)
    }

    pub fn can_recv(&self) -> bool {
        matches!(self, TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SocketOptions {
    pub reuse_addr: bool,
    pub reuse_port: bool,
    pub rcvbuf: usize,
    pub sndbuf: usize,
    pub nonblocking: bool,
    pub tcp_nodelay: bool,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self {
            reuse_addr: false,
            reuse_port: false,
            rcvbuf: 65536,
            sndbuf: 65536,
            nonblocking: false,
            tcp_nodelay: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UdpDatagram {
    pub src_ip: Ipv4Address,
    pub src_port: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct PendingSyn {
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
    pub remote_mac: MacAddress,
    pub our_isn: u32,
    pub peer_seq: u32,
    pub created_tick: u64,
}

pub struct TcpSocket {
    pub state: TcpState,
    pub local_ip: Ipv4Address,
    pub local_port: u16,
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
    pub remote_mac: Option<MacAddress>,
    pub seq_num: u32,
    pub ack_num: u32,
    pub snd_una: u32,
    pub snd_wnd: u16,
    pub rx_buffer: Vec<u8>,
    pub rx_closed: bool,
    pub tx_closed: bool,
    pub backlog: usize,
    pub accept_queue: Vec<u32>,
    pub pending_syns: Vec<PendingSyn>,
    pub time_wait_entered: Option<u64>,
    pub options: SocketOptions,
}

pub struct UdpSocket {
    pub local_ip: Ipv4Address,
    pub local_port: u16,
    pub bound: bool,
    pub connected_peer: Option<(Ipv4Address, u16)>,
    pub rx_queue: Vec<UdpDatagram>,
    pub options: SocketOptions,
}

pub enum Socket {
    Tcp(TcpSocket),
    Udp(UdpSocket),
    Raw,
}

// -----------------------------------------------------------------------------
// RFC 4987 SYN Cookie Generation & Verification
// -----------------------------------------------------------------------------
const SYN_COOKIE_SECRET: u32 = 0x5a17_c001;

fn syn_hash(local_ip: Ipv4Address, remote_ip: Ipv4Address, local_port: u16, remote_port: u16, t: u32) -> u32 {
    let mut h = SYN_COOKIE_SECRET ^ t;
    h = h.wrapping_mul(1664525).wrapping_add(1013904223);
    h ^= u32::from_ne_bytes(local_ip);
    h = h.wrapping_mul(1664525).wrapping_add(1013904223);
    h ^= u32::from_ne_bytes(remote_ip);
    h = h.wrapping_mul(1664525).wrapping_add(1013904223);
    h ^= ((local_port as u32) << 16) | (remote_port as u32);
    h = h.wrapping_mul(1664525).wrapping_add(1013904223);
    h
}

/// Generates an RFC 4987 SYN cookie when syn_queue is saturated.
pub fn generate_syn_cookie(
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    peer_seq: u32,
    time_min: u32,
) -> u32 {
    let t_bits = (time_min & 0x1f) << 27;
    let mss_bits = (1u32 & 0x07) << 24;
    let hash = syn_hash(local_ip, remote_ip, local_port, remote_port, time_min) ^ peer_seq;
    t_bits | mss_bits | (hash & 0x00ff_ffff)
}

/// Validates an incoming ACK acknowledgement as a valid SYN cookie.
pub fn validate_syn_cookie(
    cookie: u32,
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    peer_seq: u32,
    time_min: u32,
) -> bool {
    let cookie_t = (cookie >> 27) & 0x1f;
    let current_t = time_min & 0x1f;
    let diff = (current_t + 32 - cookie_t) % 32;
    if diff > 2 {
        return false;
    }
    let orig_t = time_min.wrapping_sub(diff);
    let expected_hash = (syn_hash(local_ip, remote_ip, local_port, remote_port, orig_t) ^ peer_seq) & 0x00ff_ffff;
    (cookie & 0x00ff_ffff) == expected_hash
}
