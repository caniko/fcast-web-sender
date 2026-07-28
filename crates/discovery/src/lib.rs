//! Minimal mDNS-SD implementation for `_fcast._tcp.local`.
//!
//! The parser is intentionally independent from a network socket. Fixture
//! packets can therefore exercise SRV/TXT/A/AAAA handling deterministically,
//! while the small browser below supplies the local multicast transport.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};
use thiserror::Error;

pub const SERVICE_NAME: &str = "_fcast._tcp.local";
pub const MDNS_SOCKET: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5353);
pub const MDNS_MULTICAST: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);
pub const MDNS_MULTICAST_V6: SocketAddr = SocketAddr::new(
    IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xfb)),
    5353,
);

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS packet ended at byte {0}")]
    Truncated(usize),
    #[error("mDNS packet contains an invalid name")]
    InvalidName,
    #[error("mDNS packet contains an invalid record")]
    InvalidRecord,
    #[error("mDNS socket error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceRecord {
    pub id: String,
    pub instance: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
    pub txt: BTreeMap<String, String>,
    pub ttl_seconds: u32,
}

impl ServiceRecord {
    pub fn display_name(&self) -> &str {
        self.txt
            .get("fn")
            .map(String::as_str)
            .unwrap_or(&self.instance)
    }

    pub fn first_address(&self) -> Option<IpAddr> {
        self.addresses.first().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordType {
    Ptr,
    Txt,
    A,
    Aaaa,
    Srv,
    Other(u16),
}

impl RecordType {
    fn from_u16(value: u16) -> Self {
        match value {
            1 => Self::A,
            12 => Self::Ptr,
            16 => Self::Txt,
            28 => Self::Aaaa,
            33 => Self::Srv,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug)]
struct RawRecord {
    name: String,
    record_type: RecordType,
    data: Vec<u8>,
    target_name: Option<String>,
    ttl_seconds: u32,
}

pub fn build_query(service: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(64);
    packet.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in service.trim_end_matches('.').split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0, 12, 0, 1]);
    packet
}

pub fn parse_response(packet: &[u8], service: &str) -> Result<Vec<ServiceRecord>, DiscoveryError> {
    if packet.len() < 12 {
        return Err(DiscoveryError::Truncated(packet.len()));
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let authorities = u16::from_be_bytes([packet[8], packet[9]]) as usize;
    let additionals = u16::from_be_bytes([packet[10], packet[11]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        let (_, next) = read_name(packet, offset)?;
        offset = next.checked_add(4).ok_or(DiscoveryError::InvalidRecord)?;
        if offset > packet.len() {
            return Err(DiscoveryError::Truncated(offset));
        }
    }

    let mut records = Vec::with_capacity(answers + authorities + additionals);
    for _ in 0..answers + authorities + additionals {
        let (name, next) = read_name(packet, offset)?;
        offset = next;
        if offset + 10 > packet.len() {
            return Err(DiscoveryError::Truncated(offset));
        }
        let record_type =
            RecordType::from_u16(u16::from_be_bytes([packet[offset], packet[offset + 1]]));
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let ttl_seconds = u32::from_be_bytes([
            packet[offset + 4],
            packet[offset + 5],
            packet[offset + 6],
            packet[offset + 7],
        ]);
        let data_length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + data_length > packet.len() {
            return Err(DiscoveryError::Truncated(offset));
        }
        if class & 0x7fff != 1 {
            offset += data_length;
            continue;
        }
        let target_name = match record_type {
            RecordType::Ptr => Some(read_name(packet, offset)?.0),
            RecordType::Srv if data_length >= 6 => Some(read_name(packet, offset + 6)?.0),
            _ => None,
        };
        records.push(RawRecord {
            name,
            record_type,
            data: packet[offset..offset + data_length].to_vec(),
            target_name,
            ttl_seconds,
        });
        offset += data_length;
    }
    assemble_records(records, service)
}

fn assemble_records(
    raw: Vec<RawRecord>,
    service: &str,
) -> Result<Vec<ServiceRecord>, DiscoveryError> {
    let service = canonical_name(service);
    let mut instances = BTreeMap::<String, String>::new();
    let mut srv = BTreeMap::<String, (u16, String)>::new();
    let mut txt = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut addresses = BTreeMap::<String, Vec<IpAddr>>::new();
    let mut ttl_seconds = u32::MAX;

    for record in &raw {
        ttl_seconds = ttl_seconds.min(record.ttl_seconds);
        match record.record_type {
            RecordType::Ptr if canonical_name(&record.name) == service => {
                let Some(name) = record.target_name.as_deref() else {
                    return Err(DiscoveryError::InvalidRecord);
                };
                instances.insert(name.to_owned(), record.name.clone());
            }
            RecordType::Srv => {
                if record.data.len() < 6 {
                    return Err(DiscoveryError::InvalidRecord);
                }
                let port = u16::from_be_bytes([record.data[4], record.data[5]]);
                let Some(host) = record.target_name.as_deref() else {
                    return Err(DiscoveryError::InvalidRecord);
                };
                srv.insert(canonical_name(&record.name), (port, host.to_owned()));
            }
            RecordType::Txt => {
                let mut values = BTreeMap::new();
                let mut cursor = 0;
                while cursor < record.data.len() {
                    let length = record.data[cursor] as usize;
                    cursor += 1;
                    if cursor + length > record.data.len() {
                        return Err(DiscoveryError::Truncated(cursor));
                    }
                    let text = String::from_utf8_lossy(&record.data[cursor..cursor + length]);
                    if let Some((key, value)) = text.split_once('=') {
                        values.insert(key.to_owned(), value.to_owned());
                    } else if !text.is_empty() {
                        values.insert(text.into_owned(), String::new());
                    }
                    cursor += length;
                }
                txt.insert(canonical_name(&record.name), values);
            }
            RecordType::A if record.data.len() == 4 => {
                addresses
                    .entry(canonical_name(&record.name))
                    .or_default()
                    .push(IpAddr::V4(Ipv4Addr::new(
                        record.data[0],
                        record.data[1],
                        record.data[2],
                        record.data[3],
                    )));
            }
            RecordType::Aaaa if record.data.len() == 16 => {
                let mut octets = [0_u8; 16];
                octets.copy_from_slice(&record.data);
                addresses
                    .entry(canonical_name(&record.name))
                    .or_default()
                    .push(IpAddr::V6(Ipv6Addr::from(octets)));
            }
            RecordType::Ptr | RecordType::A | RecordType::Aaaa | RecordType::Other(_) => {}
        }
    }

    let mut result = Vec::new();
    for instance in instances.keys() {
        let instance_name = canonical_name(instance);
        let Some((port, host)) = srv.get(&instance_name) else {
            continue;
        };
        let host_name = canonical_name(host);
        let instance_txt = txt.get(&instance_name).cloned().unwrap_or_default();
        let instance_addresses = addresses.get(&host_name).cloned().unwrap_or_default();
        if instance_addresses.is_empty() {
            continue;
        }
        let id = format!("{}:{}", instance_name, port);
        result.push(ServiceRecord {
            id,
            instance: instance.trim_end_matches('.').to_owned(),
            host: host.trim_end_matches('.').to_owned(),
            port: *port,
            addresses: instance_addresses,
            txt: instance_txt,
            ttl_seconds: if ttl_seconds == u32::MAX {
                0
            } else {
                ttl_seconds
            },
        });
    }
    Ok(result)
}

fn canonical_name(name: &str) -> String {
    format!("{}.", name.trim_end_matches('.').to_ascii_lowercase())
}

fn read_name(packet: &[u8], start: usize) -> Result<(String, usize), DiscoveryError> {
    let mut cursor = start;
    let mut next = None;
    let mut labels = Vec::new();
    for _ in 0..128 {
        let length = *packet
            .get(cursor)
            .ok_or(DiscoveryError::Truncated(cursor))?;
        if length == 0 {
            let end = next.unwrap_or(cursor + 1);
            return Ok((format!("{}.", labels.join(".")), end));
        }
        if length & 0xc0 == 0xc0 {
            let second = *packet
                .get(cursor + 1)
                .ok_or(DiscoveryError::Truncated(cursor + 1))?;
            let target = ((length as usize & 0x3f) << 8) | second as usize;
            if next.is_none() {
                next = Some(cursor + 2);
            }
            cursor = target;
            continue;
        }
        if length & 0xc0 != 0 || length > 63 {
            return Err(DiscoveryError::InvalidName);
        }
        cursor += 1;
        let label = packet
            .get(cursor..cursor + length as usize)
            .ok_or(DiscoveryError::Truncated(cursor))?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        cursor += length as usize;
    }
    Err(DiscoveryError::InvalidName)
}

#[derive(Debug)]
pub struct MdnsBrowser {
    sockets: Vec<UdpSocket>,
    timeout: Duration,
}

impl MdnsBrowser {
    pub fn bind(timeout: Duration) -> Result<Self, DiscoveryError> {
        let socket = UdpSocket::bind(MDNS_SOCKET)?;
        socket.join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 251), &Ipv4Addr::UNSPECIFIED)?;
        let mut sockets = vec![socket];
        if let Ok(socket) =
            UdpSocket::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 5353))
            && socket
                .join_multicast_v6(&Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xfb), 0)
                .is_ok()
        {
            sockets.push(socket);
        }
        for socket in &sockets {
            socket.set_read_timeout(Some(timeout))?;
        }
        Ok(Self { sockets, timeout })
    }

    pub fn discover(&self, service: &str) -> Result<Vec<ServiceRecord>, DiscoveryError> {
        let query = build_query(service);
        for (index, socket) in self.sockets.iter().enumerate() {
            let destination = if index == 0 {
                MDNS_MULTICAST
            } else {
                MDNS_MULTICAST_V6
            };
            let _ = socket.send_to(&query, destination);
        }
        let deadline = Instant::now() + self.timeout;
        let mut buffer = [0_u8; 16 * 1024];
        let mut found = BTreeMap::new();
        for socket in &self.sockets {
            loop {
                match socket.recv(&mut buffer) {
                    Ok(length) => {
                        for record in parse_response(&buffer[..length], service)? {
                            found.insert(record.id.clone(), record);
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
        }
        Ok(found.into_values().collect())
    }
}

#[derive(Debug, Default)]
pub struct DiscoveryCache {
    entries: BTreeMap<String, (ServiceRecord, Instant)>,
}

impl DiscoveryCache {
    pub fn upsert_at(&mut self, record: ServiceRecord, now: Instant) {
        let expiry = now + Duration::from_secs(record.ttl_seconds as u64);
        self.entries.insert(record.id.clone(), (record, expiry));
    }

    pub fn expire_at(&mut self, now: Instant) -> Vec<String> {
        let expired = self
            .entries
            .iter()
            .filter(|(_, (_, expiry))| *expiry <= now)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &expired {
            self.entries.remove(id);
        }
        expired
    }

    pub fn values_at(&mut self, now: Instant) -> Vec<ServiceRecord> {
        self.expire_at(now);
        self.entries
            .values()
            .map(|(record, _)| record.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_has_the_expected_service_question() {
        let packet = build_query(SERVICE_NAME);
        assert_eq!(&packet[..6], &[0, 0, 0, 0, 0, 1]);
        assert_eq!(
            &packet[12..],
            &[
                6, b'_', b'f', b'c', b'a', b's', b't', 4, b'_', b't', b'c', b'p', 5, b'l', b'o',
                b'c', b'a', b'l', 0, 0, 12, 0, 1,
            ]
        );
    }

    #[test]
    fn txt_values_accept_flags_and_key_value_pairs() {
        let raw = vec![RawRecord {
            name: "receiver._fcast._tcp.local".into(),
            record_type: RecordType::Txt,
            data: vec![
                3, b'f', b'n', b'=', 5, b'v', b'a', b'l', b'u', b'e', 4, b'v', b'4', b'=', b'1',
            ],
            target_name: None,
            ttl_seconds: 60,
        }];
        let records = assemble_records(raw, SERVICE_NAME).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn compressed_names_are_decoded() {
        let packet = [
            0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0, 0,
            1, 0, 1, 0, 0, 0, 0, 0, 4, 192, 0, 2, 1, 2, 3, 4,
        ];
        let records = parse_response(&packet, SERVICE_NAME).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn discovery_cache_expires_records_using_advertised_ttl() {
        let record = ServiceRecord {
            id: "receiver._fcast._tcp.local.:1234".into(),
            instance: "receiver".into(),
            host: "receiver.local".into(),
            port: 1234,
            addresses: vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))],
            txt: BTreeMap::new(),
            ttl_seconds: 1,
        };
        let now = Instant::now();
        let mut cache = DiscoveryCache::default();
        cache.upsert_at(record, now);
        assert_eq!(cache.values_at(now).len(), 1);
        assert_eq!(cache.expire_at(now + Duration::from_secs(2)).len(), 1);
        assert!(cache.values_at(now + Duration::from_secs(2)).is_empty());
    }
}
