//! Multi-Socket TCP/IP Network Subsystem for Vanta OS.
//!
//! Provides dynamic ARP resolution/replying, ICMP echo handling, UDP datagrams,
//! and a full TCP client/server state machine supporting multiple concurrent sockets,
//! streaming buffers, and POSIX/Linux socket syscall integration.

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use spin::Mutex;

use crate::net::{self, Ipv4Address, MacAddress};
use crate::virtio_net::{VirtioNet, VirtioNetError};

const ARP_POLL_ATTEMPTS: usize = 2_000_000;
const TCP_POLL_ATTEMPTS: usize = 2_000_000;
const DEFAULT_WINDOW_SIZE: u16 = 65535;
const MAX_TCP_CHUNK_SIZE: usize = 1400;
const CONFIG_PATH: &str = "/etc/network.conf";
const DEFAULT_CONFIGURATION: &[u8] =
    b"address=10.0.2.15\ngateway=10.0.2.2\ndns=10.0.2.3\ntcp_host=10.0.2.2\ntcp_port=18080\n";

static NETWORK: Mutex<Option<NetworkState>> = Mutex::new(None);
static NEXT_TCP_PORT: AtomicU16 = AtomicU16::new(49_152);
static NEXT_SOCKET_HANDLE: AtomicU32 = AtomicU32::new(1);
static ISN_COUNTER: AtomicU32 = AtomicU32::new(0x1000_0000);

// -----------------------------------------------------------------------------
// Core Network State & Sockets
// -----------------------------------------------------------------------------

struct NetworkState {
    device: VirtioNet,
    configuration: NetworkConfig,
    arp_table: BTreeMap<Ipv4Address, MacAddress>,
    gateway_mac: Option<MacAddress>,
    gateway_echoed: bool,
    dns_replied: bool,
    tcp_connected: bool,
    sockets: BTreeMap<u32, Socket>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkConfig {
    pub address: Ipv4Address,
    pub gateway: Ipv4Address,
    pub dns: Ipv4Address,
    pub tcp_host: Ipv4Address,
    pub tcp_port: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInfo {
    pub mac: MacAddress,
    pub local_ip: Ipv4Address,
    pub dns_server: Ipv4Address,
    pub tcp_host: Ipv4Address,
    pub tcp_port: u16,
    pub gateway_mac: Option<MacAddress>,
    pub gateway_echoed: bool,
    pub dns_replied: bool,
    pub tcp_connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpConnection {
    pub local_port: u16,
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
    pub next_sequence: u32,
    pub acknowledgement: u32,
    pub socket_handle: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    Device(VirtioNetError),
    GatewayUnreachable,
    GatewayNoEcho,
    DnsUnreachable,
    Configuration,
    Unavailable,
    InvalidTcpPort,
    TcpHandshakeTimeout,
    TcpReceiveTimeout,
    TcpPayloadTooLarge,
    SocketNotFound,
    InvalidSocketType,
    AlreadyBound,
    PortInUse,
    NotListening,
    NotConnected,
    AlreadyConnected,
    ConnectionReset,
    ConnectionClosed,
    WouldBlock,
    InvalidArgument,
}

impl From<VirtioNetError> for NetworkError {
    fn from(error: VirtioNetError) -> Self {
        Self::Device(error)
    }
}

// -----------------------------------------------------------------------------
// Sockets Data Structures
// -----------------------------------------------------------------------------

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

pub struct SocketOptions {
    pub reuse_addr: bool,
    pub rcvbuf: usize,
    pub sndbuf: usize,
    pub nonblocking: bool,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self {
            reuse_addr: false,
            rcvbuf: 65536,
            sndbuf: 65536,
            nonblocking: false,
        }
    }
}

pub struct UdpDatagram {
    pub src_ip: Ipv4Address,
    pub src_port: u16,
    pub data: Vec<u8>,
}

pub struct PendingSyn {
    pub remote_ip: Ipv4Address,
    pub remote_port: u16,
    pub remote_mac: MacAddress,
    pub our_isn: u32,
    pub peer_seq: u32,
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
// Initialization & Bring-up
// -----------------------------------------------------------------------------

pub fn initialize() -> Result<NetworkInfo, NetworkError> {
    crate::serial_println!("[net] probing VirtIO network device");
    let mut device = VirtioNet::probe()?;
    crate::serial_println!("[net] loading VFS configuration");
    let configuration = load_configuration()?;
    let mac = device.mac();

    let mut arp_table = BTreeMap::new();

    // 1. ARP gateway resolution
    let request = net::arp_request(mac, configuration.address, configuration.gateway);
    device.transmit(&request)?;
    let mut gateway_mac = None;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if let Some(resolved_mac) =
                net::arp_reply_mac(&frame, configuration.address, configuration.gateway)
            {
                gateway_mac = Some(resolved_mac);
                arp_table.insert(configuration.gateway, resolved_mac);
                break;
            }
        }
        core::hint::spin_loop();
    }
    let gateway_mac = gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;

    // 2. ICMP echo request to gateway
    let request = net::icmp_echo_request(
        mac,
        gateway_mac,
        configuration.address,
        configuration.gateway,
        0x564e,
    );
    device.transmit(&request)?;
    let mut gateway_echoed = false;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if net::is_icmp_echo_reply(&frame, configuration.address, configuration.gateway, 0x564e)
            {
                gateway_echoed = true;
                break;
            }
        }
        core::hint::spin_loop();
    }
    let info = NetworkInfo {
        mac,
        local_ip: configuration.address,
        dns_server: configuration.dns,
        tcp_host: configuration.tcp_host,
        tcp_port: configuration.tcp_port,
        gateway_mac: Some(gateway_mac),
        gateway_echoed,
        dns_replied: false,
        tcp_connected: false,
    };
    if !info.gateway_echoed {
        return Err(NetworkError::GatewayNoEcho);
    }

    // 3. UDP DNS query
    let request = net::udp_dns_query(mac, gateway_mac, configuration.address, configuration.dns);
    device.transmit(&request)?;
    let mut dns_replied = false;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if net::is_udp_dns_reply(&frame, configuration.address, configuration.dns) {
                dns_replied = true;
                break;
            }
        }
        core::hint::spin_loop();
    }

    *NETWORK.lock() = Some(NetworkState {
        device,
        configuration,
        arp_table,
        gateway_mac: Some(gateway_mac),
        gateway_echoed,
        dns_replied,
        tcp_connected: false,
        sockets: BTreeMap::new(),
    });

    if dns_replied {
        Ok(NetworkInfo {
            dns_replied,
            ..info
        })
    } else {
        Err(NetworkError::DnsUnreachable)
    }
}

pub fn status() -> Option<NetworkInfo> {
    let state = NETWORK.lock();
    state.as_ref().map(|state| NetworkInfo {
        mac: state.device.mac(),
        local_ip: state.configuration.address,
        dns_server: state.configuration.dns,
        tcp_host: state.configuration.tcp_host,
        tcp_port: state.configuration.tcp_port,
        gateway_mac: state.gateway_mac,
        gateway_echoed: state.gateway_echoed,
        dns_replied: state.dns_replied,
        tcp_connected: state.tcp_connected,
    })
}

fn load_configuration() -> Result<NetworkConfig, NetworkError> {
    let bytes = match crate::vfs::read_root(CONFIG_PATH) {
        Ok(bytes) => bytes,
        Err(_) => {
            crate::vfs::write_root(CONFIG_PATH, DEFAULT_CONFIGURATION)
                .map_err(|_| NetworkError::Configuration)?;
            DEFAULT_CONFIGURATION.to_vec()
        }
    };
    let configuration = parse_configuration(&bytes).ok_or(NetworkError::Configuration)?;
    crate::serial_println!(
        "[net] VFS configuration loaded address={}.{}.{}.{} gateway={}.{}.{}.{} dns={}.{}.{}.{} tcp={}.{}.{}.{}:{}",
        configuration.address[0],
        configuration.address[1],
        configuration.address[2],
        configuration.address[3],
        configuration.gateway[0],
        configuration.gateway[1],
        configuration.gateway[2],
        configuration.gateway[3],
        configuration.dns[0],
        configuration.dns[1],
        configuration.dns[2],
        configuration.dns[3],
        configuration.tcp_host[0],
        configuration.tcp_host[1],
        configuration.tcp_host[2],
        configuration.tcp_host[3],
        configuration.tcp_port,
    );
    Ok(configuration)
}

fn parse_configuration(bytes: &[u8]) -> Option<NetworkConfig> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut address = None;
    let mut gateway = None;
    let mut dns = None;
    let mut tcp_host = None;
    let mut tcp_port = None;
    for line in text.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "address" => address = parse_ipv4(value),
            "gateway" => gateway = parse_ipv4(value),
            "dns" => dns = parse_ipv4(value),
            "tcp_host" => tcp_host = parse_ipv4(value),
            "tcp_port" => tcp_port = parse_port(value),
            _ => {}
        }
    }
    let tcp_port = tcp_port?;
    if tcp_port == 0 {
        return None;
    }
    Some(NetworkConfig {
        address: address?,
        gateway: gateway?,
        dns: dns?,
        tcp_host: tcp_host?,
        tcp_port,
    })
}

fn parse_ipv4(value: &str) -> Option<Ipv4Address> {
    let bytes = value.as_bytes();
    let mut address = [0; 4];
    let mut component = 0;
    let mut number = 0u16;
    let mut digits = 0;
    let mut index = 0;
    while index <= bytes.len() {
        if index == bytes.len() || bytes[index] == b'.' {
            if digits == 0 || number > u8::MAX as u16 || component == address.len() {
                return None;
            }
            address[component] = number as u8;
            component += 1;
            number = 0;
            digits = 0;
        } else {
            let byte = bytes[index];
            if !byte.is_ascii_digit() {
                return None;
            }
            number = number.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
            digits += 1;
        }
        index += 1;
    }
    (component == address.len()).then_some(address)
}

fn parse_port(value: &str) -> Option<u16> {
    if value.is_empty() {
        return None;
    }
    let mut port = 0u32;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        port = port.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
        if port > u16::MAX as u32 {
            return None;
        }
    }
    Some(port as u16)
}

fn generate_isn(local_port: u16, remote_port: u16) -> u32 {
    let counter = ISN_COUNTER.fetch_add(0x1000, Ordering::Relaxed);
    counter ^ (((local_port as u32) << 16) | (remote_port as u32))
}

// -----------------------------------------------------------------------------
// Central Packet Dispatcher & Network Poller
// -----------------------------------------------------------------------------

fn resolve_destination_mac(state: &mut NetworkState, target_ip: Ipv4Address) -> Result<MacAddress, NetworkError> {
    let our_ip = state.configuration.address;
    let is_local_subnet = target_ip[0] == our_ip[0]
        && target_ip[1] == our_ip[1]
        && target_ip[2] == our_ip[2];

    if is_local_subnet {
        if let Some(&mac) = state.arp_table.get(&target_ip) {
            return Ok(mac);
        }
        let mac = state.device.mac();
        let request = net::arp_request(mac, our_ip, target_ip);
        state.device.transmit(&request)?;
        for _ in 0..10_000 {
            if let Some(frame) = state.device.receive()? {
                if let Some(reply_mac) = net::arp_reply_mac(&frame, our_ip, target_ip) {
                    state.arp_table.insert(target_ip, reply_mac);
                    return Ok(reply_mac);
                }
            }
            core::hint::spin_loop();
        }
        if let Some(gateway) = state.gateway_mac {
            return Ok(gateway);
        }
        Err(NetworkError::GatewayUnreachable)
    } else {
        state.gateway_mac.ok_or(NetworkError::GatewayUnreachable)
    }
}

pub fn poll_network() -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    poll_network_locked(state)
}

fn poll_network_locked(state: &mut NetworkState) -> Result<(), NetworkError> {
    while let Some(frame) = state.device.receive()? {
        process_incoming_frame(state, &frame)?;
    }
    Ok(())
}

fn process_incoming_frame(state: &mut NetworkState, frame: &[u8]) -> Result<(), NetworkError> {
    let Some((eth, eth_payload)) = net::parse_ethernet(frame) else {
        return Ok(());
    };
    let our_mac = state.device.mac();
    let our_ip = state.configuration.address;

    match eth.ethertype {
        net::ETHERTYPE_ARP => {
            if let Some(arp) = net::parse_arp(eth_payload) {
                state.arp_table.insert(arp.sender_ip, arp.sender_mac);
                if arp.opcode == net::ARP_REQUEST && arp.target_ip == our_ip {
                    let reply = net::arp_reply(our_mac, our_ip, arp.sender_mac, arp.sender_ip);
                    let _ = state.device.transmit(&reply);
                }
            }
        }
        net::ETHERTYPE_IPV4 => {
            let Some((ip, ip_payload)) = net::parse_ipv4(eth_payload) else {
                return Ok(());
            };
            if ip.dest_ip != our_ip && ip.dest_ip != [255, 255, 255, 255] {
                return Ok(());
            }

            match ip.protocol {
                net::IP_PROTOCOL_ICMP => {
                    if let Some((icmp, icmp_data)) = net::parse_icmp(ip_payload) {
                        if icmp.msg_type == net::ICMP_ECHO_REQUEST {
                            let reply = net::build_icmp_echo_reply(
                                our_mac,
                                eth.src_mac,
                                our_ip,
                                ip.src_ip,
                                icmp.identifier,
                                icmp.sequence,
                                icmp_data,
                            );
                            let _ = state.device.transmit(&reply);
                        }
                    }
                }
                net::IP_PROTOCOL_UDP => {
                    if let Some((udp, udp_data)) = net::parse_udp(ip_payload, ip.src_ip, ip.dest_ip) {
                        for socket in state.sockets.values_mut() {
                            if let Socket::Udp(udp_sock) = socket {
                                if udp_sock.bound && (udp_sock.local_port == udp.dest_port || udp_sock.local_port == 0) {
                                    udp_sock.rx_queue.push(UdpDatagram {
                                        src_ip: ip.src_ip,
                                        src_port: udp.src_port,
                                        data: udp_data.to_vec(),
                                    });
                                }
                            }
                        }
                    }
                }
                net::IP_PROTOCOL_TCP => {
                    if let Some((tcp, tcp_payload)) = net::parse_tcp(ip_payload, ip.src_ip, ip.dest_ip) {
                        handle_incoming_tcp(state, eth.src_mac, ip.src_ip, &tcp, tcp_payload)?;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_incoming_tcp(
    state: &mut NetworkState,
    src_mac: MacAddress,
    src_ip: Ipv4Address,
    tcp: &net::TcpHeader,
    payload: &[u8],
) -> Result<(), NetworkError> {
    let our_mac = state.device.mac();
    let our_ip = state.configuration.address;

    // 1. First, look for matching established or active TCP socket
    let mut matched_socket_handle = None;
    for (&handle, socket) in state.sockets.iter() {
        if let Socket::Tcp(s) = socket {
            if s.local_port == tcp.dest_port && s.remote_ip == src_ip && s.remote_port == tcp.src_port {
                matched_socket_handle = Some(handle);
                break;
            }
        }
    }

    if let Some(handle) = matched_socket_handle {
        let Socket::Tcp(s) = state.sockets.get_mut(&handle).unwrap() else {
            return Ok(());
        };

        if tcp.flags & net::TCP_RST != 0 {
            s.state = TcpState::Reset;
            return Ok(());
        }

        match s.state {
            TcpState::SynSent => {
                if (tcp.flags & (net::TCP_SYN | net::TCP_ACK)) == (net::TCP_SYN | net::TCP_ACK)
                    && tcp.acknowledgement == s.seq_num.wrapping_add(1)
                {
                    s.seq_num = s.seq_num.wrapping_add(1);
                    s.ack_num = tcp.sequence.wrapping_add(1);
                    s.snd_wnd = tcp.window_size;
                    s.remote_mac = Some(src_mac);
                    s.state = TcpState::Established;
                    let ack = net::build_tcp_frame(
                        our_mac,
                        src_mac,
                        our_ip,
                        src_ip,
                        s.local_port,
                        s.remote_port,
                        s.seq_num,
                        s.ack_num,
                        net::TCP_ACK,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&ack);
                }
            }
            TcpState::Established | TcpState::CloseWait => {
                if !payload.is_empty() {
                    if tcp.sequence == s.ack_num {
                        s.rx_buffer.extend_from_slice(payload);
                        s.ack_num = s.ack_num.wrapping_add(payload.len() as u32);
                        let ack = net::build_tcp_frame(
                            our_mac,
                            src_mac,
                            our_ip,
                            src_ip,
                            s.local_port,
                            s.remote_port,
                            s.seq_num,
                            s.ack_num,
                            net::TCP_ACK,
                            DEFAULT_WINDOW_SIZE,
                            b"",
                        );
                        let _ = state.device.transmit(&ack);
                    } else if tcp.sequence < s.ack_num {
                        let ack = net::build_tcp_frame(
                            our_mac,
                            src_mac,
                            our_ip,
                            src_ip,
                            s.local_port,
                            s.remote_port,
                            s.seq_num,
                            s.ack_num,
                            net::TCP_ACK,
                            DEFAULT_WINDOW_SIZE,
                            b"",
                        );
                        let _ = state.device.transmit(&ack);
                    }
                }
                if tcp.flags & net::TCP_ACK != 0 {
                    s.snd_una = tcp.acknowledgement;
                }
                if tcp.flags & net::TCP_FIN != 0 && s.state == TcpState::Established {
                    s.ack_num = s.ack_num.wrapping_add(1);
                    s.rx_closed = true;
                    s.state = TcpState::CloseWait;
                    let ack = net::build_tcp_frame(
                        our_mac,
                        src_mac,
                        our_ip,
                        src_ip,
                        s.local_port,
                        s.remote_port,
                        s.seq_num,
                        s.ack_num,
                        net::TCP_ACK,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&ack);
                }
            }
            TcpState::FinWait1 => {
                if tcp.flags & net::TCP_ACK != 0 {
                    s.state = TcpState::FinWait2;
                }
                if tcp.flags & net::TCP_FIN != 0 {
                    s.ack_num = s.ack_num.wrapping_add(1);
                    s.state = if s.state == TcpState::FinWait2 {
                        TcpState::TimeWait
                    } else {
                        TcpState::Closing
                    };
                    let ack = net::build_tcp_frame(
                        our_mac,
                        src_mac,
                        our_ip,
                        src_ip,
                        s.local_port,
                        s.remote_port,
                        s.seq_num,
                        s.ack_num,
                        net::TCP_ACK,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&ack);
                }
            }
            TcpState::FinWait2 => {
                if tcp.flags & net::TCP_FIN != 0 {
                    s.ack_num = s.ack_num.wrapping_add(1);
                    s.state = TcpState::TimeWait;
                    let ack = net::build_tcp_frame(
                        our_mac,
                        src_mac,
                        our_ip,
                        src_ip,
                        s.local_port,
                        s.remote_port,
                        s.seq_num,
                        s.ack_num,
                        net::TCP_ACK,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&ack);
                }
            }
            TcpState::LastAck => {
                if tcp.flags & net::TCP_ACK != 0 {
                    s.state = TcpState::Closed;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // 2. Check for TCP listener socket on the destination port
    let mut listener_handle = None;
    for (&handle, socket) in state.sockets.iter() {
        if let Socket::Tcp(s) = socket {
            if s.local_port == tcp.dest_port && s.state == TcpState::Listen {
                listener_handle = Some(handle);
                break;
            }
        }
    }

    if let Some(l_handle) = listener_handle {
        let Socket::Tcp(listener) = state.sockets.get_mut(&l_handle).unwrap() else {
            return Ok(());
        };

        if tcp.flags & net::TCP_SYN != 0 && (tcp.flags & net::TCP_ACK == 0) {
            let our_isn = generate_isn(tcp.dest_port, tcp.src_port);
            let syn_ack = net::build_tcp_frame(
                our_mac,
                src_mac,
                our_ip,
                src_ip,
                tcp.dest_port,
                tcp.src_port,
                our_isn,
                tcp.sequence.wrapping_add(1),
                net::TCP_SYN | net::TCP_ACK,
                DEFAULT_WINDOW_SIZE,
                b"",
            );
            let _ = state.device.transmit(&syn_ack);
            listener.pending_syns.retain(|p| !(p.remote_ip == src_ip && p.remote_port == tcp.src_port));
            listener.pending_syns.push(PendingSyn {
                remote_ip: src_ip,
                remote_port: tcp.src_port,
                remote_mac: src_mac,
                our_isn,
                peer_seq: tcp.sequence,
            });
        } else if tcp.flags & net::TCP_ACK != 0 {
            if let Some(pos) = listener
                .pending_syns
                .iter()
                .position(|p| p.remote_ip == src_ip && p.remote_port == tcp.src_port)
            {
                let pending = listener.pending_syns.remove(pos);
                if tcp.acknowledgement == pending.our_isn.wrapping_add(1) {
                    let new_handle = NEXT_SOCKET_HANDLE.fetch_add(1, Ordering::Relaxed);
                    let mut new_sock = TcpSocket {
                        state: TcpState::Established,
                        local_ip: our_ip,
                        local_port: tcp.dest_port,
                        remote_ip: src_ip,
                        remote_port: tcp.src_port,
                        remote_mac: Some(src_mac),
                        seq_num: pending.our_isn.wrapping_add(1),
                        ack_num: tcp.sequence,
                        snd_una: pending.our_isn.wrapping_add(1),
                        snd_wnd: tcp.window_size,
                        rx_buffer: Vec::new(),
                        rx_closed: false,
                        tx_closed: false,
                        backlog: 0,
                        accept_queue: Vec::new(),
                        pending_syns: Vec::new(),
                        options: SocketOptions::default(),
                    };
                    if !payload.is_empty() {
                        new_sock.rx_buffer.extend_from_slice(payload);
                        new_sock.ack_num = new_sock.ack_num.wrapping_add(payload.len() as u32);
                        let ack = net::build_tcp_frame(
                            our_mac,
                            src_mac,
                            our_ip,
                            src_ip,
                            new_sock.local_port,
                            new_sock.remote_port,
                            new_sock.seq_num,
                            new_sock.ack_num,
                            net::TCP_ACK,
                            DEFAULT_WINDOW_SIZE,
                            b"",
                        );
                        let _ = state.device.transmit(&ack);
                    }
                    state.sockets.insert(new_handle, Socket::Tcp(new_sock));
                    let Socket::Tcp(listener) = state.sockets.get_mut(&l_handle).unwrap() else {
                        return Ok(());
                    };
                    listener.accept_queue.push(new_handle);
                }
            }
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Socket Operations backing Kernel & Syscalls
// -----------------------------------------------------------------------------

pub fn socket_create(domain: u32, socket_type: u32, _protocol: u32) -> Result<u32, NetworkError> {
    if domain != 2 { // AF_INET
        return Err(NetworkError::InvalidArgument);
    }
    let handle = NEXT_SOCKET_HANDLE.fetch_add(1, Ordering::Relaxed);
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    let our_ip = state.configuration.address;
    let socket = match socket_type {
        1 => { // SOCK_STREAM (TCP)
            Socket::Tcp(TcpSocket {
                state: TcpState::Closed,
                local_ip: our_ip,
                local_port: 0,
                remote_ip: [0, 0, 0, 0],
                remote_port: 0,
                remote_mac: None,
                seq_num: 0,
                ack_num: 0,
                snd_una: 0,
                snd_wnd: DEFAULT_WINDOW_SIZE,
                rx_buffer: Vec::new(),
                rx_closed: false,
                tx_closed: false,
                backlog: 0,
                accept_queue: Vec::new(),
                pending_syns: Vec::new(),
                options: SocketOptions::default(),
            })
        }
        2 => { // SOCK_DGRAM (UDP)
            Socket::Udp(UdpSocket {
                local_ip: our_ip,
                local_port: 0,
                bound: false,
                connected_peer: None,
                rx_queue: Vec::new(),
                options: SocketOptions::default(),
            })
        }
        3 => Socket::Raw,
        _ => return Err(NetworkError::InvalidSocketType),
    };

    state.sockets.insert(handle, socket);
    Ok(handle)
}

pub fn socket_bind(handle: u32, ip: Ipv4Address, port: u16) -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    let bind_port = if port == 0 {
        NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed)
    } else {
        port
    };

    let bind_ip = if ip == [0, 0, 0, 0] {
        state.configuration.address
    } else {
        ip
    };

    let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
    match socket {
        Socket::Tcp(tcp) => {
            if tcp.local_port != 0 {
                return Err(NetworkError::AlreadyBound);
            }
            tcp.local_ip = bind_ip;
            tcp.local_port = bind_port;
            Ok(())
        }
        Socket::Udp(udp) => {
            if udp.bound {
                return Err(NetworkError::AlreadyBound);
            }
            udp.local_ip = bind_ip;
            udp.local_port = bind_port;
            udp.bound = true;
            Ok(())
        }
        Socket::Raw => Ok(()),
    }
}

pub fn socket_listen(handle: u32, backlog: usize) -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
    match socket {
        Socket::Tcp(tcp) => {
            if tcp.local_port == 0 {
                tcp.local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
            }
            tcp.state = TcpState::Listen;
            tcp.backlog = if backlog == 0 { 128 } else { backlog };
            Ok(())
        }
        _ => Err(NetworkError::InvalidSocketType),
    }
}

pub fn socket_accept(handle: u32, nonblocking: bool) -> Result<(u32, Ipv4Address, u16), NetworkError> {
    for _ in 0..TCP_POLL_ATTEMPTS {
        {
            let mut state = NETWORK.lock();
            let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
            poll_network_locked(state)?;

            let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
            let Socket::Tcp(tcp) = socket else {
                return Err(NetworkError::InvalidSocketType);
            };
            if tcp.state != TcpState::Listen {
                return Err(NetworkError::NotListening);
            }

            if !tcp.accept_queue.is_empty() {
                let accepted_handle = tcp.accept_queue.remove(0);
                if let Some(Socket::Tcp(accepted_sock)) = state.sockets.get(&accepted_handle) {
                    return Ok((accepted_handle, accepted_sock.remote_ip, accepted_sock.remote_port));
                }
            }
        }
        if nonblocking {
            return Err(NetworkError::WouldBlock);
        }
        core::hint::spin_loop();
    }
    Err(NetworkError::TcpReceiveTimeout)
}

pub fn socket_connect(
    handle: u32,
    remote_ip: Ipv4Address,
    remote_port: u16,
    nonblocking: bool,
) -> Result<(), NetworkError> {
    if remote_port == 0 {
        return Err(NetworkError::InvalidTcpPort);
    }

    {
        let mut state = NETWORK.lock();
        let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

        let dest_mac = resolve_destination_mac(state, remote_ip)?;
        let our_mac = state.device.mac();
        let our_ip = state.configuration.address;

        let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
        match socket {
            Socket::Tcp(tcp) => {
                if tcp.state == TcpState::Established {
                    return Err(NetworkError::AlreadyConnected);
                }
                if tcp.local_port == 0 {
                    tcp.local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
                }
                tcp.remote_ip = remote_ip;
                tcp.remote_port = remote_port;
                tcp.remote_mac = Some(dest_mac);

                let sequence = generate_isn(tcp.local_port, remote_port);
                tcp.seq_num = sequence;
                tcp.state = TcpState::SynSent;

                let syn = net::build_tcp_frame(
                    our_mac,
                    dest_mac,
                    our_ip,
                    remote_ip,
                    tcp.local_port,
                    remote_port,
                    sequence,
                    0,
                    net::TCP_SYN,
                    DEFAULT_WINDOW_SIZE,
                    b"",
                );
                state.device.transmit(&syn)?;
            }
            Socket::Udp(udp) => {
                udp.connected_peer = Some((remote_ip, remote_port));
                if udp.local_port == 0 {
                    udp.local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
                    udp.bound = true;
                }
                return Ok(());
            }
            Socket::Raw => return Ok(()),
        }
    }

    if nonblocking {
        return Ok(());
    }

    for _ in 0..TCP_POLL_ATTEMPTS {
        {
            let mut state = NETWORK.lock();
            let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
            poll_network_locked(state)?;

            let socket = state.sockets.get(&handle).ok_or(NetworkError::SocketNotFound)?;
            if let Socket::Tcp(tcp) = socket {
                if tcp.state == TcpState::Established {
                    state.tcp_connected = true;
                    return Ok(());
                } else if tcp.state == TcpState::Reset || tcp.state == TcpState::Closed {
                    return Err(NetworkError::ConnectionReset);
                }
            }
        }
        core::hint::spin_loop();
    }

    Err(NetworkError::TcpHandshakeTimeout)
}

pub fn socket_send(handle: u32, bytes: &[u8]) -> Result<usize, NetworkError> {
    if bytes.is_empty() {
        return Ok(0);
    }

    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    enum SendAction {
        Tcp {
            remote_ip: Ipv4Address,
            remote_port: u16,
            local_port: u16,
            dest_mac: MacAddress,
            seq_num: u32,
            ack_num: u32,
        },
        Udp {
            dest_ip: Ipv4Address,
            dest_port: u16,
            local_port: u16,
        },
        Raw,
    }

    let action = {
        let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
        match socket {
            Socket::Tcp(tcp) => {
                if tcp.state != TcpState::Established && tcp.state != TcpState::CloseWait {
                    return Err(NetworkError::NotConnected);
                }
                let dest_mac = tcp.remote_mac.ok_or(NetworkError::GatewayUnreachable)?;
                SendAction::Tcp {
                    remote_ip: tcp.remote_ip,
                    remote_port: tcp.remote_port,
                    local_port: tcp.local_port,
                    dest_mac,
                    seq_num: tcp.seq_num,
                    ack_num: tcp.ack_num,
                }
            }
            Socket::Udp(udp) => {
                let (dest_ip, dest_port) = udp.connected_peer.ok_or(NetworkError::NotConnected)?;
                if udp.local_port == 0 {
                    udp.local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
                    udp.bound = true;
                }
                SendAction::Udp {
                    dest_ip,
                    dest_port,
                    local_port: udp.local_port,
                }
            }
            Socket::Raw => SendAction::Raw,
        }
    };

    match action {
        SendAction::Tcp { remote_ip, remote_port, local_port, dest_mac, mut seq_num, ack_num } => {
            let our_mac = state.device.mac();
            let our_ip = state.configuration.address;

            let mut offset = 0;
            while offset < bytes.len() {
                let chunk_len = (bytes.len() - offset).min(MAX_TCP_CHUNK_SIZE);
                let chunk = &bytes[offset..offset + chunk_len];

                let frame = net::build_tcp_frame(
                    our_mac,
                    dest_mac,
                    our_ip,
                    remote_ip,
                    local_port,
                    remote_port,
                    seq_num,
                    ack_num,
                    net::TCP_ACK | net::TCP_PSH,
                    DEFAULT_WINDOW_SIZE,
                    chunk,
                );
                state.device.transmit(&frame)?;
                seq_num = seq_num.wrapping_add(chunk_len as u32);
                offset += chunk_len;
            }

            if let Some(Socket::Tcp(tcp)) = state.sockets.get_mut(&handle) {
                tcp.seq_num = seq_num;
            }
            let _ = poll_network_locked(state);
            Ok(bytes.len())
        }
        SendAction::Udp { dest_ip, dest_port, local_port } => {
            let dest_mac = resolve_destination_mac(state, dest_ip)?;
            let our_mac = state.device.mac();
            let our_ip = state.configuration.address;
            let frame = net::build_udp_frame(
                our_mac,
                dest_mac,
                our_ip,
                dest_ip,
                local_port,
                dest_port,
                bytes,
            );
            state.device.transmit(&frame)?;
            Ok(bytes.len())
        }
        SendAction::Raw => Ok(bytes.len()),
    }
}

pub fn socket_sendto(
    handle: u32,
    bytes: &[u8],
    dest_ip: Ipv4Address,
    dest_port: u16,
) -> Result<usize, NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    enum SendToAction {
        Udp { local_port: u16 },
        Tcp {
            remote_ip: Ipv4Address,
            remote_port: u16,
            local_port: u16,
            dest_mac: MacAddress,
            seq_num: u32,
            ack_num: u32,
        },
        Raw,
    }

    let action = {
        let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
        match socket {
            Socket::Udp(udp) => {
                if udp.local_port == 0 {
                    udp.local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
                    udp.bound = true;
                }
                SendToAction::Udp { local_port: udp.local_port }
            }
            Socket::Tcp(tcp) => {
                if tcp.state != TcpState::Established && tcp.state != TcpState::CloseWait {
                    return Err(NetworkError::NotConnected);
                }
                let dest_mac = tcp.remote_mac.ok_or(NetworkError::GatewayUnreachable)?;
                SendToAction::Tcp {
                    remote_ip: tcp.remote_ip,
                    remote_port: tcp.remote_port,
                    local_port: tcp.local_port,
                    dest_mac,
                    seq_num: tcp.seq_num,
                    ack_num: tcp.ack_num,
                }
            }
            Socket::Raw => SendToAction::Raw,
        }
    };

    match action {
        SendToAction::Udp { local_port } => {
            let dest_mac = resolve_destination_mac(state, dest_ip)?;
            let our_mac = state.device.mac();
            let our_ip = state.configuration.address;
            let frame = net::build_udp_frame(
                our_mac,
                dest_mac,
                our_ip,
                dest_ip,
                local_port,
                dest_port,
                bytes,
            );
            state.device.transmit(&frame)?;
            Ok(bytes.len())
        }
        SendToAction::Tcp { remote_ip, remote_port, local_port, dest_mac, mut seq_num, ack_num } => {
            let our_mac = state.device.mac();
            let our_ip = state.configuration.address;

            let mut offset = 0;
            while offset < bytes.len() {
                let chunk_len = (bytes.len() - offset).min(MAX_TCP_CHUNK_SIZE);
                let chunk = &bytes[offset..offset + chunk_len];

                let frame = net::build_tcp_frame(
                    our_mac,
                    dest_mac,
                    our_ip,
                    remote_ip,
                    local_port,
                    remote_port,
                    seq_num,
                    ack_num,
                    net::TCP_ACK | net::TCP_PSH,
                    DEFAULT_WINDOW_SIZE,
                    chunk,
                );
                state.device.transmit(&frame)?;
                seq_num = seq_num.wrapping_add(chunk_len as u32);
                offset += chunk_len;
            }

            if let Some(Socket::Tcp(tcp)) = state.sockets.get_mut(&handle) {
                tcp.seq_num = seq_num;
            }
            let _ = poll_network_locked(state);
            Ok(bytes.len())
        }
        SendToAction::Raw => Ok(bytes.len()),
    }
}

pub fn socket_recv(handle: u32, limit: usize, nonblocking: bool) -> Result<Vec<u8>, NetworkError> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    for _ in 0..TCP_POLL_ATTEMPTS {
        {
            let mut state = NETWORK.lock();
            let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
            poll_network_locked(state)?;

            let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
            match socket {
                Socket::Tcp(tcp) => {
                    if !tcp.rx_buffer.is_empty() {
                        let take_len = tcp.rx_buffer.len().min(limit);
                        let result = tcp.rx_buffer.drain(..take_len).collect();
                        return Ok(result);
                    }
                    if tcp.rx_closed {
                        return Ok(Vec::new()); // EOF
                    }
                    if tcp.state == TcpState::Reset || tcp.state == TcpState::Closed {
                        return Err(NetworkError::ConnectionReset);
                    }
                }
                Socket::Udp(udp) => {
                    if !udp.rx_queue.is_empty() {
                        let dg = udp.rx_queue.remove(0);
                        let take_len = dg.data.len().min(limit);
                        return Ok(dg.data[..take_len].to_vec());
                    }
                }
                Socket::Raw => return Ok(Vec::new()),
            }
        }
        if nonblocking {
            return Err(NetworkError::WouldBlock);
        }
        core::hint::spin_loop();
    }

    Err(NetworkError::TcpReceiveTimeout)
}

pub fn socket_recvfrom(
    handle: u32,
    limit: usize,
    nonblocking: bool,
) -> Result<(Vec<u8>, Ipv4Address, u16), NetworkError> {
    for _ in 0..TCP_POLL_ATTEMPTS {
        {
            let mut state = NETWORK.lock();
            let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
            poll_network_locked(state)?;

            let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
            match socket {
                Socket::Udp(udp) => {
                    if !udp.rx_queue.is_empty() {
                        let dg = udp.rx_queue.remove(0);
                        let take_len = dg.data.len().min(limit);
                        return Ok((dg.data[..take_len].to_vec(), dg.src_ip, dg.src_port));
                    }
                }
                Socket::Tcp(tcp) => {
                    let remote_ip = tcp.remote_ip;
                    let remote_port = tcp.remote_port;
                    if !tcp.rx_buffer.is_empty() {
                        let take_len = tcp.rx_buffer.len().min(limit);
                        let result: Vec<u8> = tcp.rx_buffer.drain(..take_len).collect();
                        return Ok((result, remote_ip, remote_port));
                    }
                    if tcp.rx_closed {
                        return Ok((Vec::new(), remote_ip, remote_port));
                    }
                    if tcp.state == TcpState::Reset || tcp.state == TcpState::Closed {
                        return Err(NetworkError::ConnectionReset);
                    }
                }
                Socket::Raw => return Ok((Vec::new(), [0, 0, 0, 0], 0)),
            }
        }
        if nonblocking {
            return Err(NetworkError::WouldBlock);
        }
        core::hint::spin_loop();
    }
    Err(NetworkError::TcpReceiveTimeout)
}

pub fn socket_getsockname(handle: u32) -> Result<(Ipv4Address, u16), NetworkError> {
    let state = NETWORK.lock();
    let state = state.as_ref().ok_or(NetworkError::Unavailable)?;
    let socket = state.sockets.get(&handle).ok_or(NetworkError::SocketNotFound)?;
    match socket {
        Socket::Tcp(tcp) => Ok((tcp.local_ip, tcp.local_port)),
        Socket::Udp(udp) => Ok((udp.local_ip, udp.local_port)),
        Socket::Raw => Ok((state.configuration.address, 0)),
    }
}

pub fn socket_getpeername(handle: u32) -> Result<(Ipv4Address, u16), NetworkError> {
    let state = NETWORK.lock();
    let state = state.as_ref().ok_or(NetworkError::Unavailable)?;
    let socket = state.sockets.get(&handle).ok_or(NetworkError::SocketNotFound)?;
    match socket {
        Socket::Tcp(tcp) => {
            if tcp.state == TcpState::Closed || tcp.state == TcpState::Listen {
                Err(NetworkError::NotConnected)
            } else {
                Ok((tcp.remote_ip, tcp.remote_port))
            }
        }
        Socket::Udp(udp) => udp.connected_peer.ok_or(NetworkError::NotConnected),
        Socket::Raw => Err(NetworkError::NotConnected),
    }
}

pub fn socket_setsockopt(handle: u32, _level: u32, optname: u32, optval: u32) -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    let socket = state.sockets.get_mut(&handle).ok_or(NetworkError::SocketNotFound)?;
    let options = match socket {
        Socket::Tcp(tcp) => &mut tcp.options,
        Socket::Udp(udp) => &mut udp.options,
        Socket::Raw => return Ok(()),
    };
    match optname {
        2 => options.reuse_addr = optval != 0, // SO_REUSEADDR
        8 => options.rcvbuf = optval as usize, // SO_RCVBUF
        7 => options.sndbuf = optval as usize, // SO_SNDBUF
        _ => {}
    }
    Ok(())
}

pub fn socket_getsockopt(handle: u32, _level: u32, optname: u32) -> Result<u32, NetworkError> {
    let state = NETWORK.lock();
    let state = state.as_ref().ok_or(NetworkError::Unavailable)?;
    let socket = state.sockets.get(&handle).ok_or(NetworkError::SocketNotFound)?;
    let (sock_type, options) = match socket {
        Socket::Tcp(tcp) => (1, &tcp.options),
        Socket::Udp(udp) => (2, &udp.options),
        Socket::Raw => (3, &SocketOptions { reuse_addr: false, rcvbuf: 0, sndbuf: 0, nonblocking: false }),
    };
    match optname {
        2 => Ok(if options.reuse_addr { 1 } else { 0 }), // SO_REUSEADDR
        3 => Ok(sock_type),                              // SO_TYPE
        4 => Ok(0),                                      // SO_ERROR
        8 => Ok(options.rcvbuf as u32),                  // SO_RCVBUF
        7 => Ok(options.sndbuf as u32),                  // SO_SNDBUF
        _ => Ok(0),
    }
}

pub fn socket_close(handle: u32) -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;

    if let Some(socket) = state.sockets.get_mut(&handle) {
        if let Socket::Tcp(tcp) = socket {
            if tcp.state == TcpState::Established {
                if let Some(dest_mac) = tcp.remote_mac {
                    let our_mac = state.device.mac();
                    let our_ip = state.configuration.address;
                    let fin = net::build_tcp_frame(
                        our_mac,
                        dest_mac,
                        our_ip,
                        tcp.remote_ip,
                        tcp.local_port,
                        tcp.remote_port,
                        tcp.seq_num,
                        tcp.ack_num,
                        net::TCP_ACK | net::TCP_FIN,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&fin);
                    tcp.seq_num = tcp.seq_num.wrapping_add(1);
                    tcp.state = TcpState::FinWait1;
                }
            } else if tcp.state == TcpState::CloseWait {
                if let Some(dest_mac) = tcp.remote_mac {
                    let our_mac = state.device.mac();
                    let our_ip = state.configuration.address;
                    let fin = net::build_tcp_frame(
                        our_mac,
                        dest_mac,
                        our_ip,
                        tcp.remote_ip,
                        tcp.local_port,
                        tcp.remote_port,
                        tcp.seq_num,
                        tcp.ack_num,
                        net::TCP_ACK | net::TCP_FIN,
                        DEFAULT_WINDOW_SIZE,
                        b"",
                    );
                    let _ = state.device.transmit(&fin);
                    tcp.seq_num = tcp.seq_num.wrapping_add(1);
                    tcp.state = TcpState::LastAck;
                }
            }
        }
    }
    state.sockets.remove(&handle);
    Ok(())
}

pub fn socket_has_data(handle: u32) -> bool {
    let mut state = NETWORK.lock();
    let Some(state) = state.as_mut() else {
        return false;
    };
    let _ = poll_network_locked(state);
    let Some(socket) = state.sockets.get(&handle) else {
        return false;
    };
    match socket {
        Socket::Tcp(tcp) => !tcp.rx_buffer.is_empty() || !tcp.accept_queue.is_empty() || tcp.rx_closed,
        Socket::Udp(udp) => !udp.rx_queue.is_empty(),
        Socket::Raw => false,
    }
}

// -----------------------------------------------------------------------------
// Legacy TCP Connection Functions (Zero Regression Compatibility)
// -----------------------------------------------------------------------------

pub fn tcp_connect(
    remote_ip: Ipv4Address,
    remote_port: u16,
) -> Result<TcpConnection, NetworkError> {
    let socket_handle = socket_create(2, 1, 0)?;
    socket_connect(socket_handle, remote_ip, remote_port, false)?;

    let state = NETWORK.lock();
    let state = state.as_ref().ok_or(NetworkError::Unavailable)?;
    let Socket::Tcp(tcp) = state.sockets.get(&socket_handle).ok_or(NetworkError::SocketNotFound)? else {
        return Err(NetworkError::InvalidSocketType);
    };

    Ok(TcpConnection {
        local_port: tcp.local_port,
        remote_ip,
        remote_port,
        next_sequence: tcp.seq_num,
        acknowledgement: tcp.ack_num,
        socket_handle,
    })
}

pub fn tcp_send(connection: &mut TcpConnection, bytes: &[u8]) -> Result<(), NetworkError> {
    socket_send(connection.socket_handle, bytes)?;
    let state = NETWORK.lock();
    if let Some(state) = state.as_ref() {
        if let Some(Socket::Tcp(tcp)) = state.sockets.get(&connection.socket_handle) {
            connection.next_sequence = tcp.seq_num;
            connection.acknowledgement = tcp.ack_num;
        }
    }
    Ok(())
}

pub fn tcp_receive(connection: &mut TcpConnection, limit: usize) -> Result<Vec<u8>, NetworkError> {
    let bytes = socket_recv(connection.socket_handle, limit, false)?;
    let state = NETWORK.lock();
    if let Some(state) = state.as_ref() {
        if let Some(Socket::Tcp(tcp)) = state.sockets.get(&connection.socket_handle) {
            connection.next_sequence = tcp.seq_num;
            connection.acknowledgement = tcp.ack_num;
        }
    }
    Ok(bytes)
}

pub fn tcp_close(connection: TcpConnection) -> Result<(), NetworkError> {
    socket_close(connection.socket_handle)
}
