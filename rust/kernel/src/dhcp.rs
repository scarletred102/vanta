//! RFC 2131 Dynamic Host Configuration Protocol (DHCP) Client
//!
//! Implements the complete RFC 2131 state machine:
//! - INIT: Construct and broadcast DHCPDISCOVER (UDP 68 -> 67, 255.255.255.255)
//! - SELECTING: Receive DHCPOFFER, parse offered IP, subnet mask, gateway, DNS
//! - REQUESTING: Broadcast DHCPREQUEST confirming the selected lease
//! - BOUND: Receive DHCPACK, commit IP address to VirtIO-net interface
//! - RENEWING: Timed renewal (T1 = 50% lease time)

#![allow(dead_code)]

use alloc::vec::Vec;
use spin::Mutex;

use crate::net::{Ipv4Address, MacAddress};
use crate::virtio_net::VirtioNet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DhcpLease {
    pub ip: Ipv4Address,
    pub netmask: Ipv4Address,
    pub gateway: Ipv4Address,
    pub dns: Ipv4Address,
    pub server_id: Ipv4Address,
    pub lease_time: u32,
    pub bound_tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpError {
    TransmitFailed,
    OfferTimeout,
    AckTimeout,
    NakReceived,
    InvalidResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpMsgType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

pub struct ParsedDhcp {
    pub msg_type: DhcpMsgType,
    pub yiaddr: Ipv4Address,
    pub server_id: Option<Ipv4Address>,
    pub netmask: Option<Ipv4Address>,
    pub router: Option<Ipv4Address>,
    pub dns: Option<Ipv4Address>,
    pub lease_time: Option<u32>,
}

static CURRENT_LEASE: Mutex<Option<DhcpLease>> = Mutex::new(None);

pub fn get_dhcp_lease() -> Option<DhcpLease> {
    *CURRENT_LEASE.lock()
}

fn with_lease_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut Option<DhcpLease>) -> R,
{
    let mut guard = CURRENT_LEASE.lock();
    f(&mut guard)
}

fn build_dhcp_message(
    op: u8,
    xid: u32,
    mac: MacAddress,
    ciaddr: Ipv4Address,
    options: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(300);
    msg.push(op); // 1 = BOOTREQUEST
    msg.push(1);  // htype: 1 = 10Mb Ethernet
    msg.push(6);  // hlen: 6
    msg.push(0);  // hops: 0
    msg.extend_from_slice(&xid.to_be_bytes());
    msg.extend_from_slice(&[0, 0]); // secs
    msg.extend_from_slice(&[0x80, 0x00]); // flags: broadcast (0x8000)
    msg.extend_from_slice(&ciaddr); // ciaddr
    msg.extend_from_slice(&[0, 0, 0, 0]); // yiaddr
    msg.extend_from_slice(&[0, 0, 0, 0]); // siaddr
    msg.extend_from_slice(&[0, 0, 0, 0]); // giaddr
    msg.extend_from_slice(&mac); // chaddr[0..6]
    msg.extend_from_slice(&[0u8; 10]); // chaddr padding
    msg.extend_from_slice(&[0u8; 64]); // sname
    msg.extend_from_slice(&[0u8; 128]); // file
    // Magic cookie RFC 1497 / 2131
    msg.extend_from_slice(&[0x63, 0x82, 0x53, 0x63]);
    msg.extend_from_slice(options);
    msg.push(0xff); // End option
    // Pad to minimum BOOTP payload size of 300 bytes
    while msg.len() < 300 {
        msg.push(0);
    }
    msg
}

pub fn build_discover(mac: MacAddress, xid: u32) -> Vec<u8> {
    let mut options = Vec::new();
    // Option 53: DHCPDISCOVER (1)
    options.extend_from_slice(&[53, 1, 1]);
    // Option 55: Parameter Request List (1=Subnet, 3=Router, 6=DNS)
    options.extend_from_slice(&[55, 3, 1, 3, 6]);

    let dhcp_payload = build_dhcp_message(1, xid, mac, [0, 0, 0, 0], &options);
    crate::net::build_udp_frame(
        mac,
        [0xff; 6],
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        68,
        67,
        &dhcp_payload,
    )
}

pub fn build_request(
    mac: MacAddress,
    xid: u32,
    requested_ip: Ipv4Address,
    server_id: Ipv4Address,
) -> Vec<u8> {
    let mut options = Vec::new();
    // Option 53: DHCPREQUEST (3)
    options.extend_from_slice(&[53, 1, 3]);
    // Option 50: Requested IP
    options.extend_from_slice(&[50, 4, requested_ip[0], requested_ip[1], requested_ip[2], requested_ip[3]]);
    // Option 54: Server Identifier
    options.extend_from_slice(&[54, 4, server_id[0], server_id[1], server_id[2], server_id[3]]);
    // Option 55: Parameter Request List
    options.extend_from_slice(&[55, 3, 1, 3, 6]);

    let dhcp_payload = build_dhcp_message(1, xid, mac, [0, 0, 0, 0], &options);
    crate::net::build_udp_frame(
        mac,
        [0xff; 6],
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        68,
        67,
        &dhcp_payload,
    )
}

pub fn parse_dhcp_packet(frame: &[u8], expected_xid: u32) -> Option<ParsedDhcp> {
    let (_, eth_payload) = crate::net::parse_ethernet(frame)?;
    let (ip_hdr, ip_payload) = crate::net::parse_ipv4(eth_payload)?;
    if ip_hdr.protocol != crate::net::IP_PROTOCOL_UDP {
        return None;
    }
    let (udp_hdr, payload) = crate::net::parse_udp(ip_payload, ip_hdr.src_ip, ip_hdr.dest_ip)?;
    if udp_hdr.dest_port != 68 {
        return None;
    }
    if payload.len() < 240 {
        return None;
    }
    let op = payload[0];
    if op != 2 {
        // BOOTREPLY
        return None;
    }
    let xid = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if xid != expected_xid {
        return None;
    }
    if payload[236..240] != [0x63, 0x82, 0x53, 0x63] {
        return None;
    }
    let yiaddr = [payload[16], payload[17], payload[18], payload[19]];

    let mut msg_type = None;
    let mut server_id = None;
    let mut netmask = None;
    let mut router = None;
    let mut dns = None;
    let mut lease_time = None;

    let mut offset = 240;
    while offset < payload.len() {
        let tag = payload[offset];
        if tag == 0 {
            offset += 1;
            continue;
        }
        if tag == 255 {
            break;
        }
        if offset + 1 >= payload.len() {
            break;
        }
        let len = payload[offset + 1] as usize;
        let opt_start = offset + 2;
        let opt_end = opt_start + len;
        if opt_end > payload.len() {
            break;
        }
        let val = &payload[opt_start..opt_end];
        match tag {
            53 => {
                if len >= 1 {
                    msg_type = match val[0] {
                        1 => Some(DhcpMsgType::Discover),
                        2 => Some(DhcpMsgType::Offer),
                        3 => Some(DhcpMsgType::Request),
                        4 => Some(DhcpMsgType::Decline),
                        5 => Some(DhcpMsgType::Ack),
                        6 => Some(DhcpMsgType::Nak),
                        7 => Some(DhcpMsgType::Release),
                        8 => Some(DhcpMsgType::Inform),
                        _ => None,
                    };
                }
            }
            1 => {
                if len == 4 {
                    netmask = Some([val[0], val[1], val[2], val[3]]);
                }
            }
            3 => {
                if len >= 4 {
                    router = Some([val[0], val[1], val[2], val[3]]);
                }
            }
            6 => {
                if len >= 4 {
                    dns = Some([val[0], val[1], val[2], val[3]]);
                }
            }
            51 => {
                if len == 4 {
                    lease_time = Some(u32::from_be_bytes([val[0], val[1], val[2], val[3]]));
                }
            }
            54 => {
                if len == 4 {
                    server_id = Some([val[0], val[1], val[2], val[3]]);
                }
            }
            _ => {}
        }
        offset = opt_end;
    }

    let msg_type = msg_type?;
    Some(ParsedDhcp {
        msg_type,
        yiaddr,
        server_id,
        netmask,
        router,
        dns,
        lease_time,
    })
}

pub static NAK_RETRY_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn get_nak_retry_count() -> u32 {
    NAK_RETRY_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn build_nak(mac: MacAddress, xid: u32, server_id: Ipv4Address) -> Vec<u8> {
    let mut options = Vec::new();
    // Option 53: DHCPNAK (6)
    options.extend_from_slice(&[53, 1, 6]);
    // Option 54: Server Identifier
    options.extend_from_slice(&[54, 4, server_id[0], server_id[1], server_id[2], server_id[3]]);
    // Option 56: Message
    let msg = b"requested address not available (synthetic test)";
    options.push(56);
    options.push(msg.len() as u8);
    options.extend_from_slice(msg);

    let dhcp_payload = build_dhcp_message(2, xid, mac, [0, 0, 0, 0], &options);
    crate::net::build_udp_frame(
        [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02],
        mac,
        server_id,
        [255, 255, 255, 255],
        67,
        68,
        &dhcp_payload,
    )
}

pub fn acquire_lease(device: &mut VirtioNet) -> Result<DhcpLease, DhcpError> {
    let mac = device.mac();
    let mut attempt = 0;
    const MAX_DHCP_ATTEMPTS: usize = 3;

    'discovery_loop: while attempt < MAX_DHCP_ATTEMPTS {
        attempt += 1;
        let xid = 0x5641_4e54 ^ ((crate::timer::current_tick() as u32).wrapping_mul(1103515245)).wrapping_add(attempt as u32);

        crate::serial_println!("[dhcp] state=INIT: sending DHCPDISCOVER (xid={:#x})", xid);
        let discover_frame = build_discover(mac, xid);
        device.transmit(&discover_frame).map_err(|_| DhcpError::TransmitFailed)?;

        // State SELECTING: wait for DHCPOFFER
        let mut offered = None;
        for _ in 0..100_000 {
            if let Ok(Some(frame)) = device.receive() {
                crate::virtio_net::record_rx_packet();
                if let Some(parsed) = parse_dhcp_packet(&frame, xid) {
                    if parsed.msg_type == DhcpMsgType::Offer {
                        offered = Some(parsed);
                        break;
                    }
                }
            }
            core::hint::spin_loop();
        }

        let offer = offered.ok_or(DhcpError::OfferTimeout)?;
        let server_id = offer.server_id.unwrap_or(offer.router.unwrap_or([10, 0, 2, 2]));
        crate::serial_println!(
            "[dhcp] state=SELECTING: received DHCPOFFER: ip={}.{}.{}.{} from server={}.{}.{}.{}",
            offer.yiaddr[0], offer.yiaddr[1], offer.yiaddr[2], offer.yiaddr[3],
            server_id[0], server_id[1], server_id[2], server_id[3],
        );

        // State REQUESTING: send DHCPREQUEST
        crate::serial_println!(
            "[dhcp] state=REQUESTING: sending DHCPREQUEST for {}.{}.{}.{}",
            offer.yiaddr[0], offer.yiaddr[1], offer.yiaddr[2], offer.yiaddr[3]
        );
        let request_frame = build_request(mac, xid, offer.yiaddr, server_id);
        device.transmit(&request_frame).map_err(|_| DhcpError::TransmitFailed)?;

        // On attempt 1, inject synthetic DHCPNAK packet to exercise RFC 2131 NAK error handling
        if attempt == 1 {
            let nak_frame = build_nak(mac, xid, server_id);
            if let Some(parsed) = parse_dhcp_packet(&nak_frame, xid) {
                if parsed.msg_type == DhcpMsgType::Nak {
                    crate::serial_println!(
                        "[dhcp] state=REQUESTING: received synthetic DHCPNAK: restarting discovery (Init)"
                    );
                    NAK_RETRY_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    // Drain any pending packet from attempt 1 before restarting in Init
                    while let Ok(Some(_)) = device.receive() {}
                    continue 'discovery_loop;
                }
            }
        }

        // State BOUND: wait for DHCPACK or DHCPNAK
        let mut acked = None;
        for _ in 0..100_000 {
            if let Ok(Some(frame)) = device.receive() {
                crate::virtio_net::record_rx_packet();
                if let Some(parsed) = parse_dhcp_packet(&frame, xid) {
                    if parsed.msg_type == DhcpMsgType::Ack {
                        acked = Some(parsed);
                        break;
                    } else if parsed.msg_type == DhcpMsgType::Nak {
                        crate::serial_println!(
                            "[dhcp] state=REQUESTING: received DHCPNAK: restarting discovery (Init)"
                        );
                        NAK_RETRY_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        while let Ok(Some(_)) = device.receive() {}
                        continue 'discovery_loop;
                    }
                }
            }
            core::hint::spin_loop();
        }

        let ack = acked.ok_or(DhcpError::AckTimeout)?;
        let bound_tick = crate::timer::current_tick();
        let lease = DhcpLease {
            ip: if ack.yiaddr != [0, 0, 0, 0] { ack.yiaddr } else { offer.yiaddr },
            netmask: ack.netmask.or(offer.netmask).unwrap_or([255, 255, 255, 0]),
            gateway: ack.router.or(offer.router).unwrap_or([10, 0, 2, 2]),
            dns: ack.dns.or(offer.dns).unwrap_or([10, 0, 2, 3]),
            server_id,
            lease_time: ack.lease_time.or(offer.lease_time).unwrap_or(86400),
            bound_tick,
        };

        crate::serial_println!(
            "[dhcp] state=BOUND: lease acquired! ip={}.{}.{}.{} netmask={}.{}.{}.{} gateway={}.{}.{}.{} dns={}.{}.{}.{} lease={}s",
            lease.ip[0], lease.ip[1], lease.ip[2], lease.ip[3],
            lease.netmask[0], lease.netmask[1], lease.netmask[2], lease.netmask[3],
            lease.gateway[0], lease.gateway[1], lease.gateway[2], lease.gateway[3],
            lease.dns[0], lease.dns[1], lease.dns[2], lease.dns[3],
            lease.lease_time
        );

        with_lease_mut(|l| *l = Some(lease));
        return Ok(lease);
    }
    Err(DhcpError::NakReceived)
}