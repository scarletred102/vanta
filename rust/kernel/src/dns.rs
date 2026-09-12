//! RFC 1035 In-Kernel & Daemon DNS Resolver
//!
//! Provides a standards-compliant DNS stub resolver querying QEMU virtual gateway DNS (10.0.2.3:53):
//! - Transaction ID tracking
//! - QNAME label encoding and pointer compression decoding
//! - Standard A record parsing (extracting IPv4 address and TTL)
//! - NXDOMAIN detection (RCODE = 3)
//! - Local TTL caching with timestamp-based expiry
//! - Static mapping lookup (/etc/hosts, localhost -> 127.0.0.1)
//! - Query counting and telemetry via /proc/net/dns

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use spin::Mutex;

use crate::net::Ipv4Address;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    InvalidHostname,
    FormatError,
    ServerFailure,
    NxDomain,
    NoRecords,
    Timeout,
    NetworkError,
}

#[derive(Clone, Debug)]
pub struct DnsCacheEntry {
    pub ip: Ipv4Address,
    pub ttl: u32,
    pub expire_tick: u64,
}

static TX_ID_COUNTER: AtomicU16 = AtomicU16::new(0x2000);
static OUTBOUND_QUERIES: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);

static CACHE: Mutex<Option<BTreeMap<String, DnsCacheEntry>>> = Mutex::new(None);

fn with_cache<F, R>(f: F) -> R
where
    F: FnOnce(&mut BTreeMap<String, DnsCacheEntry>) -> R,
{
    let mut guard = CACHE.lock();
    if guard.is_none() {
        *guard = Some(BTreeMap::new());
    }
    f(guard.as_mut().unwrap())
}

pub fn get_dns_stats() -> (u64, u64, usize) {
    let queries = OUTBOUND_QUERIES.load(Ordering::Relaxed);
    let hits = CACHE_HITS.load(Ordering::Relaxed);
    let entries = with_cache(|c| c.len());
    (queries, hits, entries)
}

pub fn clear_cache() {
    with_cache(|c| c.clear());
}

pub fn next_tx_id() -> u16 {
    TX_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Encodes an RFC 1035 DNS A-record query for the given hostname.
pub fn encode_query(hostname: &str, tx_id: u16) -> Result<Vec<u8>, DnsError> {
    if hostname.is_empty() || hostname.len() > 255 {
        return Err(DnsError::InvalidHostname);
    }
    let mut packet = Vec::with_capacity(64 + hostname.len());
    // Header (12 bytes)
    packet.extend_from_slice(&tx_id.to_be_bytes());
    packet.extend_from_slice(&0x0100u16.to_be_bytes()); // QR=0, Opcode=0, RD=1
    packet.extend_from_slice(&1u16.to_be_bytes());      // QDCOUNT = 1
    packet.extend_from_slice(&0u16.to_be_bytes());      // ANCOUNT = 0
    packet.extend_from_slice(&0u16.to_be_bytes());      // NSCOUNT = 0
    packet.extend_from_slice(&0u16.to_be_bytes());      // ARCOUNT = 0

    // Question Section: QNAME
    for label in hostname.split('.') {
        if label.is_empty() {
            continue;
        }
        if label.len() > 63 {
            return Err(DnsError::InvalidHostname);
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0); // Zero length octet terminates QNAME

    packet.extend_from_slice(&1u16.to_be_bytes()); // QTYPE = 1 (A)
    packet.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = 1 (IN)
    Ok(packet)
}

/// Decodes the QNAME, QTYPE, QCLASS from an incoming DNS query packet.
pub fn decode_query_name(packet: &[u8]) -> Result<(u16, String, u16, u16), DnsError> {
    if packet.len() < 12 {
        return Err(DnsError::FormatError);
    }
    let tx_id = u16::from_be_bytes([packet[0], packet[1]]);
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount < 1 {
        return Err(DnsError::FormatError);
    }

    let mut offset = 12;
    let mut hostname = String::new();
    let mut first = true;

    while offset < packet.len() {
        let len = packet[offset] as usize;
        offset += 1;
        if len == 0 {
            break;
        }
        if (len & 0xc0) != 0 {
            return Err(DnsError::FormatError); // Compression not expected in query QNAME
        }
        if offset + len > packet.len() {
            return Err(DnsError::FormatError);
        }
        if !first {
            hostname.push('.');
        }
        first = false;
        let label = core::str::from_utf8(&packet[offset..offset + len])
            .map_err(|_| DnsError::InvalidHostname)?;
        hostname.push_str(label);
        offset += len;
    }

    if offset + 4 > packet.len() {
        return Err(DnsError::FormatError);
    }
    let qtype = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
    let qclass = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);

    Ok((tx_id, hostname, qtype, qclass))
}

fn skip_dns_name(packet: &[u8], mut offset: usize) -> Result<usize, DnsError> {
    let mut jumped = false;
    let mut final_offset = 0;
    let mut steps = 0;

    while offset < packet.len() && steps < 128 {
        steps += 1;
        let len = packet[offset];
        if len == 0 {
            return if jumped {
                Ok(final_offset)
            } else {
                Ok(offset + 1)
            };
        }
        if (len & 0xc0) == 0xc0 {
            if offset + 2 > packet.len() {
                return Err(DnsError::FormatError);
            }
            if !jumped {
                final_offset = offset + 2;
                jumped = true;
            }
            let ptr = (((len & 0x3f) as usize) << 8) | (packet[offset + 1] as usize);
            offset = ptr;
        } else {
            let label_len = len as usize;
            offset += 1 + label_len;
        }
    }
    if jumped {
        Ok(final_offset)
    } else {
        Err(DnsError::FormatError)
    }
}

/// Decodes an RFC 1035 response packet. Returns (IPv4 Address, TTL).
pub fn decode_response(packet: &[u8], expected_tx_id: Option<u16>) -> Result<(Ipv4Address, u32), DnsError> {
    if packet.len() < 12 {
        return Err(DnsError::FormatError);
    }
    let tx_id = u16::from_be_bytes([packet[0], packet[1]]);
    if let Some(expected) = expected_tx_id {
        if tx_id != expected {
            return Err(DnsError::FormatError);
        }
    }

    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    let qr = (flags >> 15) & 1;
    if qr == 0 {
        return Err(DnsError::FormatError);
    }
    let rcode = flags & 0x0f;
    if rcode == 3 {
        return Err(DnsError::NxDomain);
    } else if rcode != 0 {
        return Err(DnsError::ServerFailure);
    }

    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    let ancount = u16::from_be_bytes([packet[6], packet[7]]);
    if ancount == 0 {
        return Err(DnsError::NoRecords);
    }

    let mut offset = 12;
    for _ in 0..qdcount {
        offset = skip_dns_name(packet, offset)?;
        if offset + 4 > packet.len() {
            return Err(DnsError::FormatError);
        }
        offset += 4; // QTYPE (2) + QCLASS (2)
    }

    for _ in 0..ancount {
        offset = skip_dns_name(packet, offset)?;
        if offset + 10 > packet.len() {
            return Err(DnsError::FormatError);
        }
        let rtype = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let rclass = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let ttl = u32::from_be_bytes([
            packet[offset + 4],
            packet[offset + 5],
            packet[offset + 6],
            packet[offset + 7],
        ]);
        let rdlength = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + rdlength > packet.len() {
            return Err(DnsError::FormatError);
        }

        if rtype == 1 && rclass == 1 && rdlength == 4 {
            // Type A, Class IN
            let ip = [
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ];
            return Ok((ip, ttl));
        }
        offset += rdlength;
    }

    Err(DnsError::NoRecords)
}

/// Builds an RFC 1035 standard response to a query using cached IP and TTL.
pub fn build_response(query_packet: &[u8], ip: Ipv4Address, ttl: u32) -> Vec<u8> {
    let mut resp = Vec::with_capacity(query_packet.len() + 16);
    if query_packet.len() < 12 {
        return resp;
    }
    // Transaction ID from query
    resp.extend_from_slice(&query_packet[0..2]);
    // Flags: Response (0x8000) | Recursion Desired (0x0100) | Recursion Available (0x0080)
    resp.extend_from_slice(&0x8180u16.to_be_bytes());
    // QDCOUNT = 1
    resp.extend_from_slice(&1u16.to_be_bytes());
    // ANCOUNT = 1
    resp.extend_from_slice(&1u16.to_be_bytes());
    // NSCOUNT = 0
    resp.extend_from_slice(&0u16.to_be_bytes());
    // ARCOUNT = 0
    resp.extend_from_slice(&0u16.to_be_bytes());

    // Copy Question section from query
    let mut q_offset = 12;
    while q_offset < query_packet.len() {
        let len = query_packet[q_offset];
        q_offset += 1;
        if len == 0 {
            break;
        }
        q_offset += len as usize;
    }
    q_offset += 4; // QTYPE + QCLASS
    if q_offset <= query_packet.len() {
        resp.extend_from_slice(&query_packet[12..q_offset]);
    }

    // Answer RR
    // Pointer to QNAME at byte 12
    resp.push(0xc0);
    resp.push(0x0c);
    // TYPE = 1 (A)
    resp.extend_from_slice(&1u16.to_be_bytes());
    // CLASS = 1 (IN)
    resp.extend_from_slice(&1u16.to_be_bytes());
    // TTL
    resp.extend_from_slice(&ttl.to_be_bytes());
    // RDLENGTH = 4
    resp.extend_from_slice(&4u16.to_be_bytes());
    // RDATA = IPv4 address
    resp.extend_from_slice(&ip);

    resp
}

/// Intercepts DNS queries destined to 10.0.2.3:53. Returns Some(response_payload) if handled.
pub(crate) fn handle_dns_datagram(
    query_bytes: &[u8],
    state: &mut crate::network::NetworkState,
) -> Option<Vec<u8>> {
    let (_tx_id, hostname, qtype, qclass) = decode_query_name(query_bytes).ok()?;
    if qtype != 1 || qclass != 1 {
        return None;
    }

    let mut qname_lower = hostname;
    qname_lower.make_ascii_lowercase();

    // 1. Static host checks
    if qname_lower == "localhost" {
        return Some(build_response(query_bytes, [127, 0, 0, 1], 86400));
    }
    if qname_lower == "vanta" {
        return Some(build_response(query_bytes, state.configuration.address, 86400));
    }

    // 2. TTL Cache check
    let current_ticks = crate::timer::current_tick();
    let cached = with_cache(|cache| {
        if let Some(entry) = cache.get(&qname_lower) {
            if current_ticks < entry.expire_tick {
                let remaining = ((entry.expire_tick - current_ticks) / 1000) as u32;
                return Some((entry.ip, remaining.max(1)));
            }
        }
        None
    });

    if let Some((ip, ttl)) = cached {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!(
            "[dns] cache hit for {}: {}.{}.{}.{} (ttl={}s, 0 outbound packets)",
            qname_lower, ip[0], ip[1], ip[2], ip[3], ttl
        );
        return Some(build_response(query_bytes, ip, ttl));
    }

    // 3. Cache Miss: Dispatch real network query to 10.0.2.3:53
    OUTBOUND_QUERIES.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "[dns] cache miss for {}: querying 10.0.2.3:53 (outbound query #{})",
        qname_lower, OUTBOUND_QUERIES.load(Ordering::Relaxed)
    );

    None
}

/// Records an incoming DNS response into the local TTL cache.
pub fn record_dns_response(packet: &[u8]) {
    if packet.len() < 12 {
        return;
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    let qr = (flags >> 15) & 1;
    if qr != 1 {
        return; // Not a response
    }
    let rcode = flags & 0x0f;
    if rcode != 0 {
        return; // Non-zero rcode (e.g. NXDOMAIN), do not cache as positive A record
    }
    if let Ok((_tx_id, hostname, _qtype, _qclass)) = decode_query_name(packet) {
        if let Ok((resolved_ip, ttl)) = decode_response(packet, None) {
            let mut qname_lower = hostname;
            qname_lower.make_ascii_lowercase();
            let current_ticks = crate::timer::current_tick();
            crate::serial_println!(
                "[dns] resolved {}: {}.{}.{}.{} (ttl={}s)",
                qname_lower, resolved_ip[0], resolved_ip[1], resolved_ip[2], resolved_ip[3], ttl
            );
            with_cache(|cache| {
                cache.insert(qname_lower, DnsCacheEntry {
                    ip: resolved_ip,
                    ttl,
                    expire_tick: current_ticks + (ttl.max(5) as u64) * 1000,
                });
            });
        }
    }
}
