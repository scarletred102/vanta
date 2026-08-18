#![allow(dead_code)]

use alloc::vec::Vec;

pub type MacAddress = [u8; 6];
pub type Ipv4Address = [u8; 4];

pub const ETHERNET_HEADER_SIZE: usize = 14;
pub const ARP_PACKET_SIZE: usize = 28;
pub const IPV4_HEADER_SIZE: usize = 20;
pub const ICMP_HEADER_SIZE: usize = 8;
pub const UDP_HEADER_SIZE: usize = 8;
pub const TCP_HEADER_SIZE: usize = 20;

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY: u16 = 2;

pub const IP_PROTOCOL_ICMP: u8 = 1;
pub const IP_PROTOCOL_TCP: u8 = 6;
pub const IP_PROTOCOL_UDP: u8 = 17;

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;
pub const TCP_URG: u8 = 0x20;

pub const MAX_TCP_PAYLOAD: usize = 1460;
pub const TCP_FRAME_CAPACITY: usize =
    ETHERNET_HEADER_SIZE + IPV4_HEADER_SIZE + TCP_HEADER_SIZE + MAX_TCP_PAYLOAD;

pub const BROADCAST_MAC: MacAddress = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetHeader {
    pub dest_mac: MacAddress,
    pub src_mac: MacAddress,
    pub ethertype: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArpPacket {
    pub opcode: u16,
    pub sender_mac: MacAddress,
    pub sender_ip: Ipv4Address,
    pub target_mac: MacAddress,
    pub target_ip: Ipv4Address,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub tos: u8,
    pub total_length: u16,
    pub id: u16,
    pub flags_fragment: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_ip: Ipv4Address,
    pub dest_ip: Ipv4Address,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IcmpHeader {
    pub msg_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub identifier: u16,
    pub sequence: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dest_port: u16,
    pub length: u16,
    pub checksum: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dest_port: u16,
    pub sequence: u32,
    pub acknowledgement: u32,
    pub data_offset: usize,
    pub flags: u8,
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_pointer: u16,
}

pub struct TcpFrame {
    bytes: [u8; TCP_FRAME_CAPACITY],
    length: usize,
}

impl TcpFrame {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

pub struct TcpSegment<'a> {
    pub sequence: u32,
    pub acknowledgement: u32,
    pub flags: u8,
    pub payload: &'a [u8],
}

// -----------------------------------------------------------------------------
// Ethernet Helpers
// -----------------------------------------------------------------------------

pub fn parse_ethernet(frame: &[u8]) -> Option<(EthernetHeader, &[u8])> {
    if frame.len() < ETHERNET_HEADER_SIZE {
        return None;
    }
    let mut dest_mac = [0u8; 6];
    let mut src_mac = [0u8; 6];
    dest_mac.copy_from_slice(&frame[..6]);
    src_mac.copy_from_slice(&frame[6..12]);
    let ethertype = read_u16(frame, 12)?;
    let header = EthernetHeader {
        dest_mac,
        src_mac,
        ethertype,
    };
    Some((header, &frame[ETHERNET_HEADER_SIZE..]))
}

pub fn build_ethernet(
    dest_mac: MacAddress,
    src_mac: MacAddress,
    ethertype: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ETHERNET_HEADER_SIZE + payload.len());
    frame.extend_from_slice(&dest_mac);
    frame.extend_from_slice(&src_mac);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

// -----------------------------------------------------------------------------
// ARP Helpers
// -----------------------------------------------------------------------------

pub fn parse_arp(payload: &[u8]) -> Option<ArpPacket> {
    if payload.len() < ARP_PACKET_SIZE {
        return None;
    }
    let htype = read_u16(payload, 0)?;
    let ptype = read_u16(payload, 2)?;
    let hlen = payload[4];
    let plen = payload[5];
    let opcode = read_u16(payload, 6)?;

    if htype != 1 || ptype != ETHERTYPE_IPV4 || hlen != 6 || plen != 4 {
        return None;
    }

    let mut sender_mac = [0u8; 6];
    let mut sender_ip = [0u8; 4];
    let mut target_mac = [0u8; 6];
    let mut target_ip = [0u8; 4];

    sender_mac.copy_from_slice(&payload[8..14]);
    sender_ip.copy_from_slice(&payload[14..18]);
    target_mac.copy_from_slice(&payload[18..24]);
    target_ip.copy_from_slice(&payload[24..28]);

    Some(ArpPacket {
        opcode,
        sender_mac,
        sender_ip,
        target_mac,
        target_ip,
    })
}

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

pub fn arp_reply(
    local_mac: MacAddress,
    local_ip: Ipv4Address,
    target_mac: MacAddress,
    target_ip: Ipv4Address,
) -> [u8; 42] {
    let mut frame = [0u8; ETHERNET_HEADER_SIZE + ARP_PACKET_SIZE];
    frame[..6].copy_from_slice(&target_mac);
    frame[6..12].copy_from_slice(&local_mac);
    write_u16(&mut frame, 12, ETHERTYPE_ARP);
    write_u16(&mut frame, ETHERNET_HEADER_SIZE, 1);
    write_u16(&mut frame, ETHERNET_HEADER_SIZE + 2, 0x0800);
    frame[ETHERNET_HEADER_SIZE + 4] = 6;
    frame[ETHERNET_HEADER_SIZE + 5] = 4;
    write_u16(&mut frame, ETHERNET_HEADER_SIZE + 6, ARP_REPLY);
    frame[ETHERNET_HEADER_SIZE + 8..ETHERNET_HEADER_SIZE + 14].copy_from_slice(&local_mac);
    frame[ETHERNET_HEADER_SIZE + 14..ETHERNET_HEADER_SIZE + 18].copy_from_slice(&local_ip);
    frame[ETHERNET_HEADER_SIZE + 18..ETHERNET_HEADER_SIZE + 24].copy_from_slice(&target_mac);
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

// -----------------------------------------------------------------------------
// IPv4 Helpers
// -----------------------------------------------------------------------------

pub fn parse_ipv4(payload: &[u8]) -> Option<(Ipv4Header, &[u8])> {
    if payload.len() < IPV4_HEADER_SIZE {
        return None;
    }
    let version_ihl = payload[0];
    let version = version_ihl >> 4;
    let ihl = version_ihl & 0x0f;
    let header_length = (ihl as usize) * 4;

    if version != 4 || header_length < IPV4_HEADER_SIZE || payload.len() < header_length {
        return None;
    }

    if !valid_checksum(&payload[..header_length]) {
        return None;
    }

    let tos = payload[1];
    let total_length = read_u16(payload, 2)?;
    let id = read_u16(payload, 4)?;
    let flags_fragment = read_u16(payload, 6)?;
    let ttl = payload[8];
    let protocol = payload[9];
    let checksum_val = read_u16(payload, 10)?;

    let mut src_ip = [0u8; 4];
    let mut dest_ip = [0u8; 4];
    src_ip.copy_from_slice(&payload[12..16]);
    dest_ip.copy_from_slice(&payload[16..20]);

    let packet_total = (total_length as usize).min(payload.len());
    if packet_total < header_length {
        return None;
    }

    let header = Ipv4Header {
        version,
        ihl,
        tos,
        total_length,
        id,
        flags_fragment,
        ttl,
        protocol,
        checksum: checksum_val,
        src_ip,
        dest_ip,
    };

    Some((header, &payload[header_length..packet_total]))
}

pub fn build_ipv4_packet(
    src_mac: MacAddress,
    dest_mac: MacAddress,
    src_ip: Ipv4Address,
    dest_ip: Ipv4Address,
    protocol: u8,
    id: u16,
    ttl: u8,
    payload: &[u8],
) -> Vec<u8> {
    let total_ip_len = IPV4_HEADER_SIZE + payload.len();
    let mut frame = Vec::with_capacity(ETHERNET_HEADER_SIZE + total_ip_len);

    frame.extend_from_slice(&dest_mac);
    frame.extend_from_slice(&src_mac);
    frame.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());

    let mut ip_hdr = [0u8; IPV4_HEADER_SIZE];
    ip_hdr[0] = 0x45; // Version 4, IHL 5 (20 bytes)
    ip_hdr[1] = 0;    // ToS
    write_u16(&mut ip_hdr, 2, total_ip_len as u16);
    write_u16(&mut ip_hdr, 4, id);
    write_u16(&mut ip_hdr, 6, 0x4000); // Don't fragment
    ip_hdr[8] = ttl;
    ip_hdr[9] = protocol;
    ip_hdr[12..16].copy_from_slice(&src_ip);
    ip_hdr[16..20].copy_from_slice(&dest_ip);

    let csum = checksum(&ip_hdr);
    write_u16(&mut ip_hdr, 10, csum);

    frame.extend_from_slice(&ip_hdr);
    frame.extend_from_slice(payload);
    frame
}

// -----------------------------------------------------------------------------
// ICMP Helpers
// -----------------------------------------------------------------------------

pub fn parse_icmp(payload: &[u8]) -> Option<(IcmpHeader, &[u8])> {
    if payload.len() < ICMP_HEADER_SIZE {
        return None;
    }
    if !valid_checksum(payload) {
        return None;
    }
    let msg_type = payload[0];
    let code = payload[1];
    let checksum_val = read_u16(payload, 2)?;
    let identifier = read_u16(payload, 4)?;
    let sequence = read_u16(payload, 6)?;

    let header = IcmpHeader {
        msg_type,
        code,
        checksum: checksum_val,
        identifier,
        sequence,
    };
    Some((header, &payload[ICMP_HEADER_SIZE..]))
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
    frame[icmp] = ICMP_ECHO_REQUEST;
    write_u16(&mut frame, icmp + 4, identifier);
    write_u16(&mut frame, icmp + 6, 1);
    let icmp_checksum = checksum(&frame[icmp..icmp + 8]);
    write_u16(&mut frame, icmp + 2, icmp_checksum);
    frame
}

pub fn build_icmp_echo_reply(
    local_mac: MacAddress,
    target_mac: MacAddress,
    local_ip: Ipv4Address,
    target_ip: Ipv4Address,
    identifier: u16,
    sequence: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut icmp_payload = Vec::with_capacity(ICMP_HEADER_SIZE + data.len());
    icmp_payload.push(ICMP_ECHO_REPLY);
    icmp_payload.push(0); // Code 0
    icmp_payload.extend_from_slice(&[0, 0]); // Checksum placeholder
    icmp_payload.extend_from_slice(&identifier.to_be_bytes());
    icmp_payload.extend_from_slice(&sequence.to_be_bytes());
    icmp_payload.extend_from_slice(data);

    let csum = checksum(&icmp_payload);
    write_u16(&mut icmp_payload, 2, csum);

    build_ipv4_packet(
        local_mac,
        target_mac,
        local_ip,
        target_ip,
        IP_PROTOCOL_ICMP,
        identifier,
        64,
        &icmp_payload,
    )
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

// -----------------------------------------------------------------------------
// UDP Helpers
// -----------------------------------------------------------------------------

pub fn parse_udp<'a>(
    payload: &'a [u8],
    src_ip: Ipv4Address,
    dest_ip: Ipv4Address,
) -> Option<(UdpHeader, &'a [u8])> {
    if payload.len() < UDP_HEADER_SIZE {
        return None;
    }
    let src_port = read_u16(payload, 0)?;
    let dest_port = read_u16(payload, 2)?;
    let length = read_u16(payload, 4)?;
    let checksum_val = read_u16(payload, 6)?;

    let len = (length as usize).min(payload.len());
    if len < UDP_HEADER_SIZE {
        return None;
    }

    if checksum_val != 0 && !valid_udp_checksum(src_ip, dest_ip, &payload[..len]) {
        return None;
    }

    let header = UdpHeader {
        src_port,
        dest_port,
        length,
        checksum: checksum_val,
    };
    Some((header, &payload[UDP_HEADER_SIZE..len]))
}

pub fn build_udp_frame(
    local_mac: MacAddress,
    remote_mac: MacAddress,
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = UDP_HEADER_SIZE + payload.len();
    let mut udp_buf = Vec::with_capacity(udp_len);
    udp_buf.extend_from_slice(&local_port.to_be_bytes());
    udp_buf.extend_from_slice(&remote_port.to_be_bytes());
    udp_buf.extend_from_slice(&(udp_len as u16).to_be_bytes());
    udp_buf.extend_from_slice(&[0, 0]); // Checksum placeholder
    udp_buf.extend_from_slice(payload);

    let csum = udp_checksum(local_ip, remote_ip, &udp_buf);
    let csum_to_write = if csum == 0 { 0xffff } else { csum };
    write_u16(&mut udp_buf, 6, csum_to_write);

    build_ipv4_packet(
        local_mac,
        remote_mac,
        local_ip,
        remote_ip,
        IP_PROTOCOL_UDP,
        local_port,
        64,
        &udp_buf,
    )
}

pub fn udp_dns_query(
    local_mac: MacAddress,
    gateway_mac: MacAddress,
    local_ip: Ipv4Address,
    dns_ip: Ipv4Address,
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
    frame[ip + 16..ip + 20].copy_from_slice(&dns_ip);
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

pub fn is_udp_dns_reply(frame: &[u8], local_ip: Ipv4Address, dns_ip: Ipv4Address) -> bool {
    let ip = ETHERNET_HEADER_SIZE;
    if frame.len() < ip + 20 + 8 + 12
        || read_u16(frame, 12) != Some(ETHERTYPE_IPV4)
        || frame[ip] >> 4 != 4
        || frame[ip + 9] != IP_PROTOCOL_UDP
        || frame[ip + 12..ip + 16] != dns_ip
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

// -----------------------------------------------------------------------------
// TCP Helpers
// -----------------------------------------------------------------------------

pub fn parse_tcp<'a>(
    payload: &'a [u8],
    src_ip: Ipv4Address,
    dest_ip: Ipv4Address,
) -> Option<(TcpHeader, &'a [u8])> {
    if payload.len() < TCP_HEADER_SIZE {
        return None;
    }
    let src_port = read_u16(payload, 0)?;
    let dest_port = read_u16(payload, 2)?;
    let sequence = read_u32(payload, 4)?;
    let acknowledgement = read_u32(payload, 8)?;
    let data_offset = ((payload[12] >> 4) as usize) * 4;
    let flags = payload[13];
    let window_size = read_u16(payload, 14)?;
    let checksum_val = read_u16(payload, 16)?;
    let urgent_pointer = read_u16(payload, 18)?;

    if data_offset < TCP_HEADER_SIZE || payload.len() < data_offset {
        return None;
    }

    if !valid_tcp_checksum(src_ip, dest_ip, payload) {
        return None;
    }

    let header = TcpHeader {
        src_port,
        dest_port,
        sequence,
        acknowledgement,
        data_offset,
        flags,
        window_size,
        checksum: checksum_val,
        urgent_pointer,
    };
    Some((header, &payload[data_offset..]))
}

pub fn build_tcp_frame(
    local_mac: MacAddress,
    remote_mac: MacAddress,
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    window_size: u16,
    payload: &[u8],
) -> Vec<u8> {
    let tcp_len = TCP_HEADER_SIZE + payload.len();
    let mut tcp_buf = Vec::with_capacity(tcp_len);
    tcp_buf.extend_from_slice(&local_port.to_be_bytes());
    tcp_buf.extend_from_slice(&remote_port.to_be_bytes());
    tcp_buf.extend_from_slice(&sequence.to_be_bytes());
    tcp_buf.extend_from_slice(&acknowledgement.to_be_bytes());
    tcp_buf.push((TCP_HEADER_SIZE as u8 / 4) << 4); // Data offset (5 words = 20 bytes)
    tcp_buf.push(flags);
    tcp_buf.extend_from_slice(&window_size.to_be_bytes());
    tcp_buf.extend_from_slice(&[0, 0]); // Checksum placeholder
    tcp_buf.extend_from_slice(&[0, 0]); // Urgent pointer
    tcp_buf.extend_from_slice(payload);

    let csum = tcp_checksum(local_ip, remote_ip, &tcp_buf);
    let csum_to_write = if csum == 0 { 0xffff } else { csum };
    write_u16(&mut tcp_buf, 16, csum_to_write);

    build_ipv4_packet(
        local_mac,
        remote_mac,
        local_ip,
        remote_ip,
        IP_PROTOCOL_TCP,
        sequence as u16,
        64,
        &tcp_buf,
    )
}

pub fn tcp_segment(
    local_mac: MacAddress,
    remote_mac: MacAddress,
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    payload: &[u8],
) -> Option<TcpFrame> {
    if payload.len() > MAX_TCP_PAYLOAD {
        return None;
    }

    let length = ETHERNET_HEADER_SIZE + IPV4_HEADER_SIZE + TCP_HEADER_SIZE + payload.len();
    let mut bytes = [0u8; TCP_FRAME_CAPACITY];
    bytes[..6].copy_from_slice(&remote_mac);
    bytes[6..12].copy_from_slice(&local_mac);
    write_u16(&mut bytes, 12, ETHERTYPE_IPV4);

    let ip = ETHERNET_HEADER_SIZE;
    bytes[ip] = 0x45;
    write_u16(
        &mut bytes,
        ip + 2,
        (IPV4_HEADER_SIZE + TCP_HEADER_SIZE + payload.len()) as u16,
    );
    write_u16(&mut bytes, ip + 4, sequence as u16);
    bytes[ip + 6] = 0x40; // Don't fragment
    bytes[ip + 8] = 64;
    bytes[ip + 9] = IP_PROTOCOL_TCP;
    bytes[ip + 12..ip + 16].copy_from_slice(&local_ip);
    bytes[ip + 16..ip + 20].copy_from_slice(&remote_ip);
    let ip_checksum = checksum(&bytes[ip..ip + IPV4_HEADER_SIZE]);
    write_u16(&mut bytes, ip + 10, ip_checksum);

    let tcp = ip + IPV4_HEADER_SIZE;
    write_u16(&mut bytes, tcp, local_port);
    write_u16(&mut bytes, tcp + 2, remote_port);
    write_u32(&mut bytes, tcp + 4, sequence);
    write_u32(&mut bytes, tcp + 8, acknowledgement);
    bytes[tcp + 12] = (TCP_HEADER_SIZE as u8 / 4) << 4;
    bytes[tcp + 13] = flags;
    write_u16(&mut bytes, tcp + 14, u16::MAX);
    bytes[tcp + TCP_HEADER_SIZE..length].copy_from_slice(payload);
    let checksum = tcp_checksum(local_ip, remote_ip, &bytes[tcp..length]);
    write_u16(
        &mut bytes,
        tcp + 16,
        if checksum == 0 { u16::MAX } else { checksum },
    );

    Some(TcpFrame { bytes, length })
}

pub fn tcp_reply<'a>(
    frame: &'a [u8],
    local_mac: MacAddress,
    remote_mac: MacAddress,
    local_ip: Ipv4Address,
    remote_ip: Ipv4Address,
    local_port: u16,
    remote_port: u16,
) -> Option<TcpSegment<'a>> {
    let ip = ETHERNET_HEADER_SIZE;
    if frame.len() < ETHERNET_HEADER_SIZE + IPV4_HEADER_SIZE + TCP_HEADER_SIZE
        || frame[..6] != local_mac
        || frame[6..12] != remote_mac
        || read_u16(frame, 12) != Some(ETHERTYPE_IPV4)
        || frame[ip] >> 4 != 4
        || frame[ip + 9] != IP_PROTOCOL_TCP
        || frame[ip + 12..ip + 16] != remote_ip
        || frame[ip + 16..ip + 20] != local_ip
    {
        return None;
    }

    let header_length = (frame[ip] & 0x0f) as usize * 4;
    let total_length = read_u16(frame, ip + 2).map(usize::from)?;
    if header_length < IPV4_HEADER_SIZE
        || total_length < header_length + TCP_HEADER_SIZE
        || frame.len() < ip + total_length
        || !valid_checksum(&frame[ip..ip + header_length])
    {
        return None;
    }

    let tcp = ip + header_length;
    let tcp_length = total_length - header_length;
    let tcp_header_length = (frame[tcp + 12] >> 4) as usize * 4;
    if tcp_header_length < TCP_HEADER_SIZE
        || tcp_header_length > tcp_length
        || read_u16(frame, tcp) != Some(remote_port)
        || read_u16(frame, tcp + 2) != Some(local_port)
        || !valid_tcp_checksum(remote_ip, local_ip, &frame[tcp..ip + total_length])
    {
        return None;
    }

    Some(TcpSegment {
        sequence: read_u32(frame, tcp + 4)?,
        acknowledgement: read_u32(frame, tcp + 8)?,
        flags: frame[tcp + 13],
        payload: &frame[tcp + tcp_header_length..ip + total_length],
    })
}

// -----------------------------------------------------------------------------
// Checksum Functions
// -----------------------------------------------------------------------------

pub fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

pub fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

pub fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

pub fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub fn checksum(bytes: &[u8]) -> u16 {
    !ones_complement_sum(bytes)
}

pub fn valid_checksum(bytes: &[u8]) -> bool {
    ones_complement_sum(bytes) == u16::MAX
}

pub fn udp_checksum(source: Ipv4Address, destination: Ipv4Address, datagram: &[u8]) -> u16 {
    !fold_ones_complement_sum(udp_sum(source, destination, datagram))
}

pub fn valid_udp_checksum(source: Ipv4Address, destination: Ipv4Address, datagram: &[u8]) -> bool {
    fold_ones_complement_sum(udp_sum(source, destination, datagram)) == u16::MAX
}

fn udp_sum(source: Ipv4Address, destination: Ipv4Address, datagram: &[u8]) -> u32 {
    let mut sum = 0;
    sum = add_ones_complement_sum(sum, &source);
    sum = add_ones_complement_sum(sum, &destination);
    sum = add_ones_complement_sum(sum, &[0, IP_PROTOCOL_UDP]);
    sum = add_ones_complement_sum(sum, &(datagram.len() as u16).to_be_bytes());
    add_ones_complement_sum(sum, datagram)
}

pub fn tcp_checksum(source: Ipv4Address, destination: Ipv4Address, segment: &[u8]) -> u16 {
    !fold_ones_complement_sum(tcp_sum(source, destination, segment))
}

pub fn valid_tcp_checksum(source: Ipv4Address, destination: Ipv4Address, segment: &[u8]) -> bool {
    fold_ones_complement_sum(tcp_sum(source, destination, segment)) == u16::MAX
}

fn tcp_sum(source: Ipv4Address, destination: Ipv4Address, segment: &[u8]) -> u32 {
    let mut sum = 0;
    sum = add_ones_complement_sum(sum, &source);
    sum = add_ones_complement_sum(sum, &destination);
    sum = add_ones_complement_sum(sum, &[0, IP_PROTOCOL_TCP]);
    sum = add_ones_complement_sum(sum, &(segment.len() as u16).to_be_bytes());
    add_ones_complement_sum(sum, segment)
}

fn ones_complement_sum(bytes: &[u8]) -> u16 {
    fold_ones_complement_sum(add_ones_complement_sum(0, bytes))
}

fn add_ones_complement_sum(mut sum: u32, bytes: &[u8]) -> u32 {
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    sum
}

fn fold_ones_complement_sum(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_MAC: MacAddress = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    const GATEWAY_MAC: MacAddress = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02];
    const LOCAL_IP: Ipv4Address = [10, 0, 2, 15];
    const HOST_IP: Ipv4Address = [10, 0, 2, 2];

    #[test]
    fn ethernet_round_trip() {
        let payload = b"hello ethernet";
        let frame = build_ethernet(GATEWAY_MAC, LOCAL_MAC, ETHERTYPE_IPV4, payload);
        let (hdr, data) = parse_ethernet(&frame).expect("valid ethernet frame");
        assert_eq!(hdr.dest_mac, GATEWAY_MAC);
        assert_eq!(hdr.src_mac, LOCAL_MAC);
        assert_eq!(hdr.ethertype, ETHERTYPE_IPV4);
        assert_eq!(data, payload);
    }

    #[test]
    fn arp_request_and_reply() {
        let req = arp_request(LOCAL_MAC, LOCAL_IP, HOST_IP);
        let (eth, arp_data) = parse_ethernet(&req).expect("ethernet");
        assert_eq!(eth.ethertype, ETHERTYPE_ARP);
        let arp = parse_arp(arp_data).expect("arp");
        assert_eq!(arp.opcode, ARP_REQUEST);
        assert_eq!(arp.sender_mac, LOCAL_MAC);
        assert_eq!(arp.sender_ip, LOCAL_IP);
        assert_eq!(arp.target_ip, HOST_IP);

        let rep = arp_reply(LOCAL_MAC, LOCAL_IP, GATEWAY_MAC, HOST_IP);
        let (eth, arp_data) = parse_ethernet(&rep).expect("ethernet");
        assert_eq!(eth.dest_mac, GATEWAY_MAC);
        let arp = parse_arp(arp_data).expect("arp");
        assert_eq!(arp.opcode, ARP_REPLY);
        assert_eq!(arp.sender_mac, LOCAL_MAC);
        assert_eq!(arp.target_mac, GATEWAY_MAC);
    }

    #[test]
    fn ipv4_and_udp_round_trip() {
        let payload = b"hello udp world";
        let frame = build_udp_frame(LOCAL_MAC, GATEWAY_MAC, LOCAL_IP, HOST_IP, 12345, 8080, payload);
        let (eth, ip_data) = parse_ethernet(&frame).expect("ethernet");
        assert_eq!(eth.ethertype, ETHERTYPE_IPV4);
        let (ip, udp_data) = parse_ipv4(ip_data).expect("ipv4");
        assert_eq!(ip.protocol, IP_PROTOCOL_UDP);
        assert_eq!(ip.src_ip, LOCAL_IP);
        assert_eq!(ip.dest_ip, HOST_IP);
        let (udp, data) = parse_udp(udp_data, ip.src_ip, ip.dest_ip).expect("udp");
        assert_eq!(udp.src_port, 12345);
        assert_eq!(udp.dest_port, 8080);
        assert_eq!(data, payload);
    }

    #[test]
    fn icmp_echo_request_and_reply() {
        let req = icmp_echo_request(LOCAL_MAC, GATEWAY_MAC, LOCAL_IP, HOST_IP, 0x1234);
        let (eth, ip_data) = parse_ethernet(&req).expect("ethernet");
        assert_eq!(eth.ethertype, ETHERTYPE_IPV4);
        let (ip, icmp_data) = parse_ipv4(ip_data).expect("ipv4");
        assert_eq!(ip.protocol, IP_PROTOCOL_ICMP);
        let (icmp, data) = parse_icmp(icmp_data).expect("icmp");
        assert_eq!(icmp.msg_type, ICMP_ECHO_REQUEST);
        assert_eq!(icmp.identifier, 0x1234);

        let rep = build_icmp_echo_reply(LOCAL_MAC, GATEWAY_MAC, LOCAL_IP, HOST_IP, 0x1234, 1, b"pingdata");
        let (_, ip_data) = parse_ethernet(&rep).expect("ethernet");
        let (ip, icmp_data) = parse_ipv4(ip_data).expect("ipv4");
        let (icmp, rep_data) = parse_icmp(icmp_data).expect("icmp");
        assert_eq!(icmp.msg_type, ICMP_ECHO_REPLY);
        assert_eq!(icmp.identifier, 0x1234);
        assert_eq!(icmp.sequence, 1);
        assert_eq!(rep_data, b"pingdata");
    }

    #[test]
    fn tcp_syn_has_valid_ipv4_and_tcp_checksums() {
        let frame = tcp_segment(
            LOCAL_MAC,
            GATEWAY_MAC,
            LOCAL_IP,
            HOST_IP,
            49_153,
            18_080,
            7,
            0,
            TCP_SYN,
            b"",
        )
        .expect("small TCP frame");
        let bytes = frame.as_bytes();
        let ip = ETHERNET_HEADER_SIZE;
        let tcp = ip + 20;

        assert!(valid_checksum(&bytes[ip..tcp]));
        assert!(valid_tcp_checksum(LOCAL_IP, HOST_IP, &bytes[tcp..]));
        assert_eq!(read_u16(bytes, tcp), Some(49_153));
        assert_eq!(read_u16(bytes, tcp + 2), Some(18_080));
    }

    #[test]
    fn tcp_reply_accepts_a_matching_syn_ack() {
        let reply = tcp_segment(
            GATEWAY_MAC,
            LOCAL_MAC,
            HOST_IP,
            LOCAL_IP,
            18_080,
            49_153,
            11,
            8,
            TCP_SYN | TCP_ACK,
            b"",
        )
        .expect("small TCP frame");

        let segment = tcp_reply(
            reply.as_bytes(),
            LOCAL_MAC,
            GATEWAY_MAC,
            LOCAL_IP,
            HOST_IP,
            49_153,
            18_080,
        )
        .expect("matching TCP reply");
        assert_eq!(segment.sequence, 11);
        assert_eq!(segment.acknowledgement, 8);
        assert_eq!(segment.flags, TCP_SYN | TCP_ACK);
        assert!(segment.payload.is_empty());
    }

    #[test]
    fn tcp_reply_rejects_a_corrupt_checksum() {
        let mut reply = tcp_segment(
            GATEWAY_MAC,
            LOCAL_MAC,
            HOST_IP,
            LOCAL_IP,
            18_080,
            49_153,
            11,
            8,
            TCP_SYN | TCP_ACK,
            b"",
        )
        .expect("small TCP frame");
        reply.bytes[ETHERNET_HEADER_SIZE + 20 + 16] ^= 0xff;

        assert!(tcp_reply(
            reply.as_bytes(),
            LOCAL_MAC,
            GATEWAY_MAC,
            LOCAL_IP,
            HOST_IP,
            49_153,
            18_080,
        )
        .is_none());
    }

    #[test]
    fn tcp_streaming_large_payload() {
        let mut large_data = [0u8; 1400];
        for (i, b) in large_data.iter_mut().enumerate() {
            *b = (i % 256) as u8;
        }
        let frame = build_tcp_frame(
            LOCAL_MAC,
            GATEWAY_MAC,
            LOCAL_IP,
            HOST_IP,
            50000,
            80,
            100,
            200,
            TCP_ACK | TCP_PSH,
            65535,
            &large_data,
        );
        let (eth, ip_data) = parse_ethernet(&frame).expect("ethernet");
        let (ip, tcp_data) = parse_ipv4(ip_data).expect("ipv4");
        let (tcp, payload) = parse_tcp(tcp_data, ip.src_ip, ip.dest_ip).expect("tcp");
        assert_eq!(tcp.src_port, 50000);
        assert_eq!(tcp.dest_port, 80);
        assert_eq!(tcp.sequence, 100);
        assert_eq!(tcp.acknowledgement, 200);
        assert_eq!(tcp.flags, TCP_ACK | TCP_PSH);
        assert_eq!(payload.len(), 1400);
        assert_eq!(payload, &large_data[..]);
    }
}
