//! VFS-configured QEMU user-network bring-up over legacy VirtIO-net.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};
use spin::Mutex;

use crate::net::{self, Ipv4Address, MacAddress};
use crate::virtio_net::{VirtioNet, VirtioNetError};

const ARP_POLL_ATTEMPTS: usize = 2_000_000;
const TCP_POLL_ATTEMPTS: usize = 2_000_000;
const CONFIG_PATH: &str = "/etc/network.conf";
const DEFAULT_CONFIGURATION: &[u8] =
    b"address=10.0.2.15\ngateway=10.0.2.2\ndns=10.0.2.3\ntcp_host=10.0.2.2\ntcp_port=18080\n";

static NETWORK: Mutex<Option<NetworkState>> = Mutex::new(None);
static NEXT_TCP_PORT: AtomicU16 = AtomicU16::new(49_152);

struct NetworkState {
    device: VirtioNet,
    configuration: NetworkConfig,
    gateway_mac: Option<MacAddress>,
    gateway_echoed: bool,
    dns_replied: bool,
    tcp_connected: bool,
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
    local_port: u16,
    remote_ip: Ipv4Address,
    remote_port: u16,
    next_sequence: u32,
    acknowledgement: u32,
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
}

impl From<VirtioNetError> for NetworkError {
    fn from(error: VirtioNetError) -> Self {
        Self::Device(error)
    }
}

pub fn initialize() -> Result<NetworkInfo, NetworkError> {
    let configuration = load_configuration()?;
    let mut device = VirtioNet::probe()?;
    let mac = device.mac();
    let request = net::arp_request(mac, configuration.address, configuration.gateway);
    device.transmit(&request)?;
    let mut gateway_mac = None;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if let Some(mac) =
                net::arp_reply_mac(&frame, configuration.address, configuration.gateway)
            {
                gateway_mac = Some(mac);
                break;
            }
        }
        core::hint::spin_loop();
    }
    let gateway_mac = gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;
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
        gateway_mac: Some(gateway_mac),
        gateway_echoed,
        dns_replied,
        tcp_connected: false,
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

pub fn tcp_connect(
    remote_ip: Ipv4Address,
    remote_port: u16,
) -> Result<TcpConnection, NetworkError> {
    if remote_port == 0 {
        return Err(NetworkError::InvalidTcpPort);
    }

    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    let gateway_mac = state.gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;
    let local_ip = state.configuration.address;
    let local_port = NEXT_TCP_PORT.fetch_add(1, Ordering::Relaxed);
    let sequence = ((local_port as u32) << 16) | 1;
    let mac = state.device.mac();
    let syn = net::tcp_segment(
        mac,
        gateway_mac,
        local_ip,
        remote_ip,
        local_port,
        remote_port,
        sequence,
        0,
        net::TCP_SYN,
        b"",
    )
    .ok_or(NetworkError::TcpPayloadTooLarge)?;
    state.device.transmit(syn.as_bytes())?;

    for _ in 0..TCP_POLL_ATTEMPTS {
        if let Some(frame) = state.device.receive()? {
            let Some(reply) = net::tcp_reply(
                &frame,
                mac,
                gateway_mac,
                local_ip,
                remote_ip,
                local_port,
                remote_port,
            ) else {
                continue;
            };
            if reply.flags & (net::TCP_SYN | net::TCP_ACK) != (net::TCP_SYN | net::TCP_ACK)
                || reply.acknowledgement != sequence.wrapping_add(1)
            {
                continue;
            }

            let connection = TcpConnection {
                local_port,
                remote_ip,
                remote_port,
                next_sequence: sequence.wrapping_add(1),
                acknowledgement: reply.sequence.wrapping_add(1),
            };
            transmit_tcp(state, &connection, net::TCP_ACK, b"")?;
            state.tcp_connected = true;
            return Ok(connection);
        }
        core::hint::spin_loop();
    }

    Err(NetworkError::TcpHandshakeTimeout)
}

pub fn tcp_send(connection: &mut TcpConnection, bytes: &[u8]) -> Result<(), NetworkError> {
    if bytes.len() > net::MAX_TCP_PAYLOAD {
        return Err(NetworkError::TcpPayloadTooLarge);
    }
    if bytes.is_empty() {
        return Ok(());
    }

    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    transmit_tcp(state, connection, net::TCP_ACK | net::TCP_PSH, bytes)?;
    connection.next_sequence = connection.next_sequence.wrapping_add(bytes.len() as u32);
    Ok(())
}

pub fn tcp_receive(connection: &mut TcpConnection, limit: usize) -> Result<Vec<u8>, NetworkError> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    let gateway_mac = state.gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;
    let mac = state.device.mac();
    let local_ip = state.configuration.address;
    for _ in 0..TCP_POLL_ATTEMPTS {
        if let Some(frame) = state.device.receive()? {
            let Some(reply) = net::tcp_reply(
                &frame,
                mac,
                gateway_mac,
                local_ip,
                connection.remote_ip,
                connection.local_port,
                connection.remote_port,
            ) else {
                continue;
            };
            if reply.acknowledgement != connection.next_sequence || reply.payload.is_empty() {
                continue;
            }

            let length = reply.payload.len().min(limit);
            let bytes = reply.payload[..length].to_vec();
            connection.acknowledgement = reply.sequence.wrapping_add(reply.payload.len() as u32);
            transmit_tcp(state, connection, net::TCP_ACK, b"")?;
            return Ok(bytes);
        }
        core::hint::spin_loop();
    }

    Err(NetworkError::TcpReceiveTimeout)
}

pub fn tcp_close(connection: TcpConnection) -> Result<(), NetworkError> {
    let mut state = NETWORK.lock();
    let state = state.as_mut().ok_or(NetworkError::Unavailable)?;
    transmit_tcp(state, &connection, net::TCP_ACK | net::TCP_FIN, b"")
}

fn transmit_tcp(
    state: &mut NetworkState,
    connection: &TcpConnection,
    flags: u8,
    payload: &[u8],
) -> Result<(), NetworkError> {
    let gateway_mac = state.gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;
    let frame = net::tcp_segment(
        state.device.mac(),
        gateway_mac,
        state.configuration.address,
        connection.remote_ip,
        connection.local_port,
        connection.remote_port,
        connection.next_sequence,
        connection.acknowledgement,
        flags,
        payload,
    )
    .ok_or(NetworkError::TcpPayloadTooLarge)?;
    state.device.transmit(frame.as_bytes())?;
    Ok(())
}
