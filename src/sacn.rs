//! Streaming ACN / sACN (ANSI E1.31-2018) codec.
//!
//! An E1.31 packet is three nested PDUs, each led by a flags+length word
//! whose low 12 bits count the bytes from that word to the end of the
//! packet (so the three lengths shrink by the size of each header):
//!
//! - **Root layer** (38 bytes): preamble, the ACN packet identifier
//!   `"ASC-E1.17\0\0\0"`, a vector saying whether this is DMX data or an
//!   extended (discovery) packet, and the sender's CID — a UUID that lets a
//!   receiver tell sources apart even when they share a name.
//! - **Framing layer** (77 bytes for data): source name, priority (a receiver
//!   with two sources on one universe keeps the higher priority, or merges
//!   HTP on a tie), a per-universe sequence number, options, and the
//!   universe number.
//! - **DMP layer** (11 bytes + start code + slots): a "set property" with
//!   address/data type `0xA1`, first address 0, increment 1, and a count
//!   that includes the start code — 513 for a full universe.
//!
//! Data is normally multicast to `239.255.<hi>.<lo>` of the universe on UDP
//! 5568 so a receiver subscribes only to the universes it patches. Every ten
//! seconds a source also multicasts a *universe discovery* packet on universe
//! 64214 listing what it sends, which is how receivers show a source list —
//! and how the Network screen lists the other sources on the wire.

use std::net::Ipv4Addr;

pub const SACN_PORT: u16 = 5568;
pub const ACN_PACKET_ID: [u8; 12] = *b"ASC-E1.17\0\0\0";
pub const VECTOR_ROOT_DATA: u32 = 0x0000_0004;
pub const VECTOR_ROOT_EXTENDED: u32 = 0x0000_0008;
pub const VECTOR_FRAMING_DATA: u32 = 0x0000_0002;
pub const VECTOR_FRAMING_DISCOVERY: u32 = 0x0000_0002;
pub const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;
pub const VECTOR_UNIVERSE_DISCOVERY_LIST: u32 = 0x0000_0001;
/// Universe carrying discovery packets (multicast 239.255.250.214).
pub const DISCOVERY_UNIVERSE: u16 = 64214;
pub const DISCOVERY_INTERVAL_S: u64 = 10;
pub const DEFAULT_PRIORITY: u8 = 100;
pub const MAX_PRIORITY: u8 = 200;
pub const MIN_UNIVERSE: u16 = 1;
pub const MAX_UNIVERSE: u16 = 63999;
/// Framing options: preview data (not for live output).
pub const OPT_PREVIEW: u8 = 0x80;
/// Framing options: this source is stopping on this universe.
pub const OPT_TERMINATED: u8 = 0x40;
/// Bytes before the slots in a data packet (root 38 + framing 77 + DMP 11).
pub const DATA_HEADER_LEN: usize = 126;
const ROOT_LEN: usize = 38;
const FRAMING_AT: usize = 38;
const DMP_AT: usize = 115;
const DISCOVERY_FRAMING_LEN: usize = 74;
const DISCOVERY_LAYER_AT: usize = ROOT_LEN + DISCOVERY_FRAMING_LEN;

/// Component identifier: the 16-byte UUID that names one sender.
pub type Cid = [u8; 16];

/// Multicast group for a universe: 239.255.<universe hi>.<universe lo>.
pub fn multicast_addr(universe: u16) -> Ipv4Addr {
    let [hi, lo] = universe.to_be_bytes();
    Ipv4Addr::new(239, 255, hi, lo)
}

/// Flags (0x7 in the high nibble) plus a 12-bit length, big-endian.
fn flags_length(len: usize) -> [u8; 2] {
    (0x7000 | (len as u16 & 0x0FFF)).to_be_bytes()
}

/// The 64-byte, null-terminated UTF-8 source name field.
fn name_field(name: &str) -> [u8; 64] {
    let mut out = [0u8; 64];
    // Leave room for the terminator and never split a multi-byte character.
    let mut cut = name.len().min(63);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    out[..cut].copy_from_slice(&name.as_bytes()[..cut]);
    out
}

fn root_layer(p: &mut Vec<u8>, total: usize, vector: u32, cid: &Cid) {
    p.extend_from_slice(&[0x00, 0x10, 0x00, 0x00]); // preamble, post-amble sizes
    p.extend_from_slice(&ACN_PACKET_ID);
    p.extend_from_slice(&flags_length(total - 16));
    p.extend_from_slice(&vector.to_be_bytes());
    p.extend_from_slice(cid);
}

/// Build an E1.31 data packet. `slots` are DMX slots 1..=512 (a start code
/// of 0 is prepended); more than 512 are dropped.
pub fn build_data(
    cid: &Cid,
    source_name: &str,
    priority: u8,
    sequence: u8,
    options: u8,
    universe: u16,
    slots: &[u8],
) -> Vec<u8> {
    let n = slots.len().min(512);
    let total = DATA_HEADER_LEN + n;
    let mut p = Vec::with_capacity(total);
    root_layer(&mut p, total, VECTOR_ROOT_DATA, cid);
    // Framing layer.
    p.extend_from_slice(&flags_length(total - FRAMING_AT));
    p.extend_from_slice(&VECTOR_FRAMING_DATA.to_be_bytes());
    p.extend_from_slice(&name_field(source_name));
    p.push(priority.min(MAX_PRIORITY));
    p.extend_from_slice(&[0, 0]); // synchronization address: none
    p.push(sequence);
    p.push(options);
    p.extend_from_slice(&universe.to_be_bytes());
    // DMP layer.
    p.extend_from_slice(&flags_length(total - DMP_AT));
    p.push(VECTOR_DMP_SET_PROPERTY);
    p.push(0xA1); // address type & data type
    p.extend_from_slice(&[0, 0]); // first property address
    p.extend_from_slice(&[0, 1]); // address increment
    p.extend_from_slice(&((n + 1) as u16).to_be_bytes()); // count incl. start code
    p.push(0); // start code: dimmer data
    p.extend_from_slice(&slots[..n]);
    debug_assert_eq!(p.len(), total);
    p
}

/// Build a universe discovery packet (one page of up to 512 universes).
pub fn build_discovery(
    cid: &Cid,
    source_name: &str,
    page: u8,
    last_page: u8,
    universes: &[u16],
) -> Vec<u8> {
    let n = universes.len().min(512);
    let total = DISCOVERY_LAYER_AT + 8 + 2 * n;
    let mut p = Vec::with_capacity(total);
    root_layer(&mut p, total, VECTOR_ROOT_EXTENDED, cid);
    // Framing layer (extended discovery flavour: name plus reserved bytes).
    p.extend_from_slice(&flags_length(total - FRAMING_AT));
    p.extend_from_slice(&VECTOR_FRAMING_DISCOVERY.to_be_bytes());
    p.extend_from_slice(&name_field(source_name));
    p.extend_from_slice(&[0, 0, 0, 0]); // reserved
    // Universe discovery layer.
    p.extend_from_slice(&flags_length(total - DISCOVERY_LAYER_AT));
    p.extend_from_slice(&VECTOR_UNIVERSE_DISCOVERY_LIST.to_be_bytes());
    p.push(page);
    p.push(last_page);
    for u in &universes[..n] {
        p.extend_from_slice(&u.to_be_bytes());
    }
    debug_assert_eq!(p.len(), total);
    p
}

/// The interesting fields of a received data packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataInfo {
    pub cid: Cid,
    pub source_name: String,
    pub priority: u8,
    pub sequence: u8,
    pub options: u8,
    pub universe: u16,
    pub start_code: u8,
    /// Slots carried, not counting the start code.
    pub slot_count: u16,
}

impl DataInfo {
    pub fn terminated(&self) -> bool {
        self.options & OPT_TERMINATED != 0
    }
    pub fn preview(&self) -> bool {
        self.options & OPT_PREVIEW != 0
    }
}

/// The fields of a received universe discovery packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryInfo {
    pub cid: Cid,
    pub source_name: String,
    pub page: u8,
    pub last_page: u8,
    pub universes: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Data(DataInfo),
    Discovery(DiscoveryInfo),
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn name_from(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// Parse an E1.31 data or discovery packet, or `None` for anything else.
///
/// Lengths are checked against the buffer rather than trusted, so a
/// truncated or hostile packet on the wire is dropped instead of panicking.
pub fn parse(buf: &[u8]) -> Option<Packet> {
    if buf.len() < ROOT_LEN || buf[0..2] != [0x00, 0x10] || buf[4..16] != ACN_PACKET_ID {
        return None;
    }
    let mut cid = [0u8; 16];
    cid.copy_from_slice(&buf[22..38]);
    match be32(&buf[18..22]) {
        VECTOR_ROOT_DATA => {
            if buf.len() < DATA_HEADER_LEN || be32(&buf[40..44]) != VECTOR_FRAMING_DATA {
                return None;
            }
            if buf[117] != VECTOR_DMP_SET_PROPERTY || buf[118] != 0xA1 {
                return None;
            }
            let count = u16::from_be_bytes([buf[123], buf[124]]);
            let available = (buf.len() - 125) as u16;
            if count == 0 || count > available {
                return None;
            }
            Some(Packet::Data(DataInfo {
                cid,
                source_name: name_from(&buf[44..108]),
                priority: buf[108],
                sequence: buf[111],
                options: buf[112],
                universe: u16::from_be_bytes([buf[113], buf[114]]),
                start_code: buf[125],
                slot_count: count - 1,
            }))
        }
        VECTOR_ROOT_EXTENDED => {
            if buf.len() < DISCOVERY_LAYER_AT + 8
                || be32(&buf[40..44]) != VECTOR_FRAMING_DISCOVERY
                || be32(&buf[DISCOVERY_LAYER_AT + 2..DISCOVERY_LAYER_AT + 6])
                    != VECTOR_UNIVERSE_DISCOVERY_LIST
            {
                return None;
            }
            let list = &buf[DISCOVERY_LAYER_AT + 8..];
            let universes = list
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(Packet::Discovery(DiscoveryInfo {
                cid,
                source_name: name_from(&buf[44..108]),
                page: buf[DISCOVERY_LAYER_AT + 6],
                last_page: buf[DISCOVERY_LAYER_AT + 7],
                universes,
            }))
        }
        _ => None,
    }
}

/// A CID in the usual 8-4-4-4-12 UUID spelling.
pub fn cid_string(cid: &Cid) -> String {
    let h = |r: std::ops::Range<usize>| {
        cid[r].iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    format!("{}-{}-{}-{}-{}", h(0..4), h(4..6), h(6..8), h(8..10), h(10..16))
}

/// Make a fresh random CID (RFC 4122 version 4 layout).
///
/// There is no `rand` in the tree, so this draws on the standard library's
/// per-process random hash keys mixed with the clock and PID — plenty for an
/// identifier that only has to differ from the other consoles on a lighting
/// network, and generated once per install since the config persists it.
pub fn generate_cid() -> Cid {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = [0u8; 16];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    out[6] = (out[6] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: Cid = [
        0xEF, 0x07, 0xC8, 0xDD, 0x00, 0x64, 0x44, 0x01, 0xA3, 0xA2, 0x45, 0x9A, 0xF8, 0xE6, 0x14,
        0x22,
    ];

    #[test]
    fn multicast_address_is_239_255_hi_lo() {
        assert_eq!(multicast_addr(1), Ipv4Addr::new(239, 255, 0, 1));
        assert_eq!(multicast_addr(256), Ipv4Addr::new(239, 255, 1, 0));
        assert_eq!(multicast_addr(63999), Ipv4Addr::new(239, 255, 249, 255));
        assert_eq!(multicast_addr(DISCOVERY_UNIVERSE), Ipv4Addr::new(239, 255, 250, 214));
    }

    #[test]
    fn data_packet_is_byte_exact_against_the_spec() {
        let slots = [0x55u8; 512];
        let p = build_data(&CID, "DMXpress", 100, 7, 0, 3, &slots);
        assert_eq!(p.len(), 638);
        // Root layer.
        assert_eq!(&p[0..4], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(&p[16..18], &[0x72, 0x6E], "root flags+length 0x7000 | 622");
        assert_eq!(&p[18..22], &[0, 0, 0, 4]);
        assert_eq!(&p[22..38], &CID);
        // Framing layer.
        assert_eq!(&p[38..40], &[0x72, 0x58], "framing flags+length 0x7000 | 600");
        assert_eq!(&p[40..44], &[0, 0, 0, 2]);
        assert_eq!(&p[44..52], b"DMXpress");
        assert!(p[52..108].iter().all(|&b| b == 0));
        assert_eq!(p[108], 100, "priority");
        assert_eq!(&p[109..111], &[0, 0], "sync address");
        assert_eq!(p[111], 7, "sequence");
        assert_eq!(p[112], 0, "options");
        assert_eq!(&p[113..115], &[0, 3], "universe");
        // DMP layer.
        assert_eq!(&p[115..117], &[0x72, 0x0B], "DMP flags+length 0x7000 | 523");
        assert_eq!(p[117], 0x02, "DMP vector");
        assert_eq!(p[118], 0xA1, "address & data type");
        assert_eq!(&p[119..121], &[0, 0], "first property address");
        assert_eq!(&p[121..123], &[0, 1], "address increment");
        assert_eq!(&p[123..125], &[0x02, 0x01], "property value count 513");
        assert_eq!(p[125], 0, "start code");
        assert_eq!(&p[126..], &slots[..]);
    }

    #[test]
    fn short_payloads_shrink_every_length() {
        let p = build_data(&CID, "x", 100, 0, OPT_TERMINATED, 1, &[1, 2, 3]);
        assert_eq!(p.len(), 129);
        assert_eq!(&p[16..18], &flags_length(113));
        assert_eq!(&p[38..40], &flags_length(91));
        assert_eq!(&p[115..117], &flags_length(14));
        assert_eq!(&p[123..125], &[0, 4]);
        // More than a universe is clipped, never over-long.
        let p = build_data(&CID, "x", 100, 0, 0, 1, &[0; 600]);
        assert_eq!(p.len(), 638);
        // Priority above 200 is clamped, the name is cut at 63 bytes.
        let p = build_data(&CID, &"n".repeat(80), 255, 0, 0, 1, &[]);
        assert_eq!(p[108], 200);
        assert_eq!(p[44 + 63], 0);
        assert_eq!(p[44 + 62], b'n');
    }

    #[test]
    fn discovery_packet_layout() {
        let p = build_discovery(&CID, "DMXpress", 0, 0, &[1, 2]);
        assert_eq!(p.len(), 124);
        assert_eq!(&p[16..18], &flags_length(108));
        assert_eq!(&p[18..22], &[0, 0, 0, 8], "root vector: extended");
        assert_eq!(&p[38..40], &flags_length(86));
        assert_eq!(&p[40..44], &[0, 0, 0, 2], "framing vector: discovery");
        assert_eq!(&p[108..112], &[0, 0, 0, 0], "reserved");
        assert_eq!(&p[112..114], &flags_length(12));
        assert_eq!(&p[114..118], &[0, 0, 0, 1], "universe list vector");
        assert_eq!(&p[118..120], &[0, 0], "page / last page");
        assert_eq!(&p[120..124], &[0, 1, 0, 2]);
    }

    #[test]
    fn parse_round_trips_both_packet_kinds() {
        let p = build_data(&CID, "Desk A", 120, 9, OPT_PREVIEW, 42, &[7; 100]);
        match parse(&p) {
            Some(Packet::Data(d)) => {
                assert_eq!(d.cid, CID);
                assert_eq!(d.source_name, "Desk A");
                assert_eq!(d.priority, 120);
                assert_eq!(d.sequence, 9);
                assert!(d.preview() && !d.terminated());
                assert_eq!(d.universe, 42);
                assert_eq!(d.slot_count, 100);
            }
            other => panic!("unexpected {other:?}"),
        }
        let p = build_discovery(&CID, "Desk A", 0, 0, &[5, 6, 700]);
        match parse(&p) {
            Some(Packet::Discovery(d)) => {
                assert_eq!(d.universes, vec![5, 6, 700]);
                assert_eq!(d.source_name, "Desk A");
            }
            other => panic!("unexpected {other:?}"),
        }
        // Not sACN, truncated, or lying about its slot count: rejected.
        assert_eq!(parse(&crate::artnet::build_poll()), None);
        assert_eq!(parse(&p[..50]), None);
        let mut lie = build_data(&CID, "x", 100, 0, 0, 1, &[0; 10]);
        lie[123..125].copy_from_slice(&[0x02, 0x01]);
        assert_eq!(parse(&lie), None);
    }

    #[test]
    fn cid_spelling_and_generation() {
        assert_eq!(cid_string(&CID), "ef07c8dd-0064-4401-a3a2-459af8e61422");
        let a = generate_cid();
        let b = generate_cid();
        assert_ne!(a, b);
        assert_eq!(a[6] >> 4, 4, "version nibble");
        assert_eq!(a[8] & 0xC0, 0x80, "variant bits");
    }
}
