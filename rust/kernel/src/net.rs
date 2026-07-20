//! Minimal Ethernet and ARP support for the QEMU user-network gateway.

pub type MacAddress = [u8; 6];
pub type Ipv4Address = [u8; 4];

const ETHERNET_HEADER_SIZE: usize = 14;
const IPV4_HEADER_SIZE: usize = 20;
const ARP_PACKET_SIZE: usize = 28;
const TCP_HEADER_SIZE: usize = 20;
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV4: u16 = 0x0800;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;
const IP_PROTOCOL_ICMP: u8 = 1;
const IP_PROTOCOL_TCP: u8 = 6;
const IP_PROTOCOL_UDP: u8 = 17;

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;
pub const MAX_TCP_PAYLOAD: usize = 64;
const TCP_FRAME_CAPACITY: usize =
    ETHERNET_HEADER_SIZE + IPV4_HEADER_SIZE + TCP_HEADER_SIZE + MAX_TCP_PAYLOAD;

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

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn checksum(bytes: &[u8]) -> u16 {
    !ones_complement_sum(bytes)
}

fn valid_checksum(bytes: &[u8]) -> bool {
    ones_complement_sum(bytes) == u16::MAX
}

fn tcp_checksum(source: Ipv4Address, destination: Ipv4Address, segment: &[u8]) -> u16 {
    !fold_ones_complement_sum(tcp_sum(source, destination, segment))
}

fn valid_tcp_checksum(source: Ipv4Address, destination: Ipv4Address, segment: &[u8]) -> bool {
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
}
