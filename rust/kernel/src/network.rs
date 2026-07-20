//! Fixed-address QEMU user-network bring-up over legacy VirtIO-net.

use spin::Mutex;

use crate::net::{self, Ipv4Address, MacAddress};
use crate::virtio_net::{VirtioNet, VirtioNetError};

const LOCAL_IP: Ipv4Address = [10, 0, 2, 15];
const ARP_POLL_ATTEMPTS: usize = 2_000_000;

static NETWORK: Mutex<Option<NetworkState>> = Mutex::new(None);

struct NetworkState {
    device: VirtioNet,
    gateway_mac: Option<MacAddress>,
    gateway_echoed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInfo {
    pub mac: MacAddress,
    pub local_ip: Ipv4Address,
    pub gateway_mac: Option<MacAddress>,
    pub gateway_echoed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    Device(VirtioNetError),
    GatewayUnreachable,
    GatewayNoEcho,
}

impl From<VirtioNetError> for NetworkError {
    fn from(error: VirtioNetError) -> Self {
        Self::Device(error)
    }
}

pub fn initialize() -> Result<NetworkInfo, NetworkError> {
    let mut device = VirtioNet::probe()?;
    let mac = device.mac();
    let request = net::arp_request(mac, LOCAL_IP, net::QEMU_GATEWAY);
    device.transmit(&request)?;
    let mut gateway_mac = None;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if let Some(mac) = net::arp_reply_mac(&frame, LOCAL_IP, net::QEMU_GATEWAY) {
                gateway_mac = Some(mac);
                break;
            }
        }
        core::hint::spin_loop();
    }
    let gateway_mac = gateway_mac.ok_or(NetworkError::GatewayUnreachable)?;
    let request = net::icmp_echo_request(mac, gateway_mac, LOCAL_IP, net::QEMU_GATEWAY, 0x564e);
    device.transmit(&request)?;
    let mut gateway_echoed = false;
    for _ in 0..ARP_POLL_ATTEMPTS {
        if let Some(frame) = device.receive()? {
            if net::is_icmp_echo_reply(&frame, LOCAL_IP, net::QEMU_GATEWAY, 0x564e) {
                gateway_echoed = true;
                break;
            }
        }
        core::hint::spin_loop();
    }
    let info = NetworkInfo {
        mac,
        local_ip: LOCAL_IP,
        gateway_mac: Some(gateway_mac),
        gateway_echoed,
    };
    *NETWORK.lock() = Some(NetworkState {
        device,
        gateway_mac: Some(gateway_mac),
        gateway_echoed,
    });
    if info.gateway_echoed {
        Ok(info)
    } else {
        Err(NetworkError::GatewayNoEcho)
    }
}

pub fn status() -> Option<NetworkInfo> {
    let state = NETWORK.lock();
    state.as_ref().map(|state| NetworkInfo {
        mac: state.device.mac(),
        local_ip: LOCAL_IP,
        gateway_mac: state.gateway_mac,
        gateway_echoed: state.gateway_echoed,
    })
}
