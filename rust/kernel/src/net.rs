//! Minimal Ethernet and ARP support for the QEMU user-network gateway.

pub type MacAddress = [u8; 6];
pub type Ipv4Address = [u8; 4];

pub const QEMU_GATEWAY: Ipv4Address = [10, 0, 2, 2];
const ETHERNET_HEADER_SIZE: usize = 14;
const ARP_PACKET_SIZE: usize = 28;
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV4: u16 = 0x0800;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;
const IP_PROTOCOL_ICMP: u8 = 1;
const IP_PROTOCOL_UDP: u8 = 17;
const QEMU_DNS: Ipv4Address = [10, 0, 2, 3];

pub fn arp_request(
    local_mac: MacAddress,
    local_ip: Ipv4Address,
    target_ip: Ipv4Address,
) -> [u8; 42] {
    let mut frame = [0u8; ETHERNET_HEADER_SIZE + ARP_PACKET_SIZE];
    frame[..6].fill(0xff);
    frame[6..12].copy_from_slice(&local_mac);
    write_u16(&mut frame, 12, ETHERTYPE_ARP);
    write_u16(&mut frame, ETHERNET_HEADER_SIZE, 1);
    write_u16(&mut frame, ETHERNET_HEADER_SIZE + 2, 0x0800);
    frame[ETHERNET_HEADER_SIZE + 4] = 6;
    frame[ETHERNET_HEADER_SIZE + 5] = 4;
    write_u16(&mut frame, ETHERNET_HEADER_SIZE + 6, ARP_REQUEST);
    frame[ETHERNET_HEADER_SIZE + 8..ETHERNET_HEADER_SIZE + 14].copy_from_slice(&local_mac);
    frame[ETHERNET_HEADER_SIZE + 14..ETHERNET_HEADER_SIZE + 18].copy_from_slice(&local_ip);
    frame[ETHERNET_HEADER_SIZE + 24..ETHERNET_HEADER_SIZE + 28].copy_from_slice(&target_ip);
    frame
}

pub fn arp_reply_mac(
    frame: &[u8],
    local_ip: Ipv4Address,
    expected_sender: Ipv4Address,
) -> Option<MacAddress> {
    if frame.len() < ETHERNET_HEADER_SIZE + ARP_PACKET_SIZE
        || read_u16(frame, 12)? != ETHERTYPE_ARP
        || read_u16(frame, ETHERNET_HEADER_SIZE)? != 1
        || read_u16(frame, ETHERNET_HEADER_SIZE + 2)? != 0x0800
        || frame[ETHERNET_HEADER_SIZE + 4] != 6
        || frame[ETHERNET_HEADER_SIZE + 5] != 4
        || read_u16(frame, ETHERNET_HEADER_SIZE + 6)? != ARP_REPLY
        || frame[ETHERNET_HEADER_SIZE + 14..ETHERNET_HEADER_SIZE + 18] != expected_sender
        || frame[ETHERNET_HEADER_SIZE + 24..ETHERNET_HEADER_SIZE + 28] != local_ip
    {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&frame[ETHERNET_HEADER_SIZE + 8..ETHERNET_HEADER_SIZE + 14]);
    Some(mac)
}

pub fn icmp_echo_request(
    local_mac: MacAddress,
    gateway_mac: MacAddress,
    local_ip: Ipv4Address,
    target_ip: Ipv4Address,
    identifier: u16,
) -> [u8; 42] {
    let mut frame = [0u8; 42];
    frame[..6].copy_from_slice(&gateway_mac);
    frame[6..12].copy_from_slice(&local_mac);
    write_u16(&mut frame, 12, ETHERTYPE_IPV4);
    let ip = ETHERNET_HEADER_SIZE;
    frame[ip] = 0x45;
    write_u16(&mut frame, ip + 2, 28);
    write_u16(&mut frame, ip + 4, identifier);
    frame[ip + 8] = 64;
    frame[ip + 9] = IP_PROTOCOL_ICMP;
    frame[ip + 12..ip + 16].copy_from_slice(&local_ip);
    frame[ip + 16..ip + 20].copy_from_slice(&target_ip);
    let header_checksum = checksum(&frame[ip..ip + 20]);
    write_u16(&mut frame, ip + 10, header_checksum);
    let icmp = ip + 20;
    frame[icmp] = 8;
    write_u16(&mut frame, icmp + 4, identifier);
    write_u16(&mut frame, icmp + 6, 1);
    let icmp_checksum = checksum(&frame[icmp..icmp + 8]);
    write_u16(&mut frame, icmp + 2, icmp_checksum);
    frame
}

pub fn is_icmp_echo_reply(
    frame: &[u8],
    local_ip: Ipv4Address,
    expected_sender: Ipv4Address,
    identifier: u16,
) -> bool {
    if frame.len() < 42 || read_u16(frame, 12) != Some(ETHERTYPE_IPV4) {
        return false;
    }
    let ip = ETHERNET_HEADER_SIZE;
    let header_length = (frame[ip] & 0x0f) as usize * 4;
    let total_length = read_u16(frame, ip + 2).map(usize::from).unwrap_or(0);
    if frame[ip] >> 4 != 4
        || header_length < 20
        || total_length < header_length + 8
        || frame.len() < ip + total_length
        || frame[ip + 9] != IP_PROTOCOL_ICMP
        || frame[ip + 12..ip + 16] != expected_sender
        || frame[ip + 16..ip + 20] != local_ip
        || !valid_checksum(&frame[ip..ip + header_length])
    {
        return false;
    }
    let icmp = ip + header_length;
    frame[icmp] == 0
        && frame[icmp + 1] == 0
        && read_u16(frame, icmp + 4) == Some(identifier)
        && valid_checksum(&frame[icmp..ip + total_length])
}

pub fn udp_dns_query(
    local_mac: MacAddress,
    gateway_mac: MacAddress,
    local_ip: Ipv4Address,
) -> [u8; 71] {
    let mut frame = [0u8; 71];
    frame[..6].copy_from_slice(&gateway_mac);
    frame[6..12].copy_from_slice(&local_mac);
    write_u16(&mut frame, 12, ETHERTYPE_IPV4);
    let ip = ETHERNET_HEADER_SIZE;
    frame[ip] = 0x45;
    write_u16(&mut frame, ip + 2, 57);
    write_u16(&mut frame, ip + 4, 0x5650);
    frame[ip + 8] = 64;
    frame[ip + 9] = IP_PROTOCOL_UDP;
    frame[ip + 12..ip + 16].copy_from_slice(&local_ip);
    frame[ip + 16..ip + 20].copy_from_slice(&QEMU_DNS);
    let header_checksum = checksum(&frame[ip..ip + 20]);
    write_u16(&mut frame, ip + 10, header_checksum);
    let udp = ip + 20;
    write_u16(&mut frame, udp, 49_152);
    write_u16(&mut frame, udp + 2, 53);
    write_u16(&mut frame, udp + 4, 37);
    let dns = udp + 8;
    write_u16(&mut frame, dns, 0x564e);
    write_u16(&mut frame, dns + 2, 0x0100);
    write_u16(&mut frame, dns + 4, 1);
    frame[dns + 12..dns + 25].copy_from_slice(b"\x07example\x03com\0");
    write_u16(&mut frame, dns + 25, 1);
    write_u16(&mut frame, dns + 27, 1);
    frame
}

pub fn is_udp_dns_reply(frame: &[u8], local_ip: Ipv4Address) -> bool {
    let ip = ETHERNET_HEADER_SIZE;
    if frame.len() < ip + 20 + 8 + 12
        || read_u16(frame, 12) != Some(ETHERTYPE_IPV4)
        || frame[ip] >> 4 != 4
        || frame[ip + 9] != IP_PROTOCOL_UDP
        || frame[ip + 12..ip + 16] != QEMU_DNS
        || frame[ip + 16..ip + 20] != local_ip
        || !valid_checksum(&frame[ip..ip + 20])
    {
        return false;
    }
    let udp = ip + 20;
    let udp_length = read_u16(frame, udp + 4).map(usize::from).unwrap_or(0);
    if udp_length < 20 || frame.len() < udp + udp_length {
        return false;
    }
    let dns = udp + 8;
    read_u16(frame, udp) == Some(53)
        && read_u16(frame, udp + 2) == Some(49_152)
        && read_u16(frame, dns) == Some(0x564e)
        && read_u16(frame, dns + 2).is_some_and(|flags| flags & 0x8000 != 0)
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn checksum(bytes: &[u8]) -> u16 {
    !ones_complement_sum(bytes)
}

fn valid_checksum(bytes: &[u8]) -> bool {
    ones_complement_sum(bytes) == u16::MAX
}

fn ones_complement_sum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        sum = (sum & 0xffff) + (sum >> 16);
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum as u16
}
