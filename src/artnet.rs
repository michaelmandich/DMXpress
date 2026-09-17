//! Minimal Art-Net (v4) codec.
//!
//! Spec references:
//! - ID: "Art-Net\0"
//! - OpPoll      = 0x2000
//! - OpPollReply = 0x2100
//! - OpDmx       = 0x5000
//! - Protocol version = 14
//! - Port = 6454
//!
//! Besides the packet builders the sender needs, this parses the whole
//! ArtPollReply a node answers with — every field the Network screen shows
//! when an operator is working out why a node stays dark — and models the
//! 15-bit Port-Address (net / sub-net / universe) that Art-Net addresses
//! universes with, since the split is what nodes show on their displays
//! while the joined number is what goes on the wire.

use std::net::Ipv4Addr;

pub const ARTNET_PORT: u16 = 6454;
pub const ARTNET_ID: &[u8; 8] = b"Art-Net\0";
pub const PROTOCOL_VERSION: u16 = 14;

pub const OP_POLL: u16 = 0x2000;
pub const OP_POLL_REPLY: u16 = 0x2100;
pub const OP_DMX: u16 = 0x5000;
pub const OP_NZS: u16 = 0x5100;
pub const OP_SYNC: u16 = 0x5200;
pub const OP_ADDRESS: u16 = 0x6000;
pub const OP_INPUT: u16 = 0x7000;
pub const OP_IP_PROG: u16 = 0xF800;
pub const OP_IP_PROG_REPLY: u16 = 0xF900;

/// Build an ArtPoll packet (broadcast to 255.255.255.255:6454).
pub fn build_poll() -> Vec<u8> {
    let mut p = Vec::with_capacity(14);
    p.extend_from_slice(ARTNET_ID);
    p.extend_from_slice(&OP_POLL.to_le_bytes()); // OpCode is little-endian
    p.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes()); // ProtVer is big-endian
    p.push(0x02); // TalkToMe: send ArtPollReply on changes
    p.push(0x00); // Priority
    p
}

/// Build an ArtDmx packet for the given universe (15-bit) and DMX payload.
/// `sequence` may be 0 to disable sequencing.
pub fn build_dmx(sequence: u8, universe: u16, data: &[u8]) -> Vec<u8> {
    // DMX payload length must be even, 2..=512.
    let mut len = data.len();
    if len < 2 {
        len = 2;
    }
    if len % 2 != 0 {
        len += 1;
    }
    if len > 512 {
        len = 512;
    }

    let mut p = Vec::with_capacity(18 + len);
    p.extend_from_slice(ARTNET_ID);
    p.extend_from_slice(&OP_DMX.to_le_bytes());
    p.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    p.push(sequence);
    p.push(0x00); // Physical
    p.extend_from_slice(&universe.to_le_bytes()); // SubUni + Net
    p.extend_from_slice(&(len as u16).to_be_bytes()); // Length big-endian

    let mut payload = vec![0u8; len];
    let copy = data.len().min(len);
    payload[..copy].copy_from_slice(&data[..copy]);
    p.extend_from_slice(&payload);
    p
}

/// The OpCode of an Art-Net packet, if the buffer is one at all.
pub fn op_code(buf: &[u8]) -> Option<u16> {
    if buf.len() < 10 || &buf[0..8] != ARTNET_ID {
        return None;
    }
    Some(u16::from_le_bytes([buf[8], buf[9]]))
}

/// Human name for an OpCode, for the packet monitor.
pub fn op_name(op: u16) -> &'static str {
    match op {
        OP_POLL => "ArtPoll",
        OP_POLL_REPLY => "ArtPollReply",
        OP_DMX => "ArtDmx",
        OP_NZS => "ArtNzs",
        OP_SYNC => "ArtSync",
        OP_ADDRESS => "ArtAddress",
        OP_INPUT => "ArtInput",
        OP_IP_PROG => "ArtIpProg",
        OP_IP_PROG_REPLY => "ArtIpProgReply",
        _ => "Art-Net",
    }
}

/// The universe (15-bit Port-Address) and sequence carried by an ArtDmx
/// packet, for the packet monitor.
pub fn dmx_header(buf: &[u8]) -> Option<(u16, u8)> {
    if op_code(buf)? != OP_DMX || buf.len() < 18 {
        return None;
    }
    Some((u16::from_le_bytes([buf[14], buf[15]]) & 0x7FFF, buf[12]))
}

/// An Art-Net Port-Address: the 15-bit universe number split the way nodes
/// display it. Net is 7 bits (0..=127), sub-net and universe 4 bits each
/// (0..=15); the joined value is `net << 8 | subnet << 4 | universe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortAddress {
    pub net: u8,
    pub subnet: u8,
    pub universe: u8,
}

impl PortAddress {
    /// Split a 15-bit Port-Address into its three fields. Bit 15 is ignored.
    pub fn split(port_address: u16) -> Self {
        Self {
            net: ((port_address >> 8) & 0x7F) as u8,
            subnet: ((port_address >> 4) & 0x0F) as u8,
            universe: (port_address & 0x0F) as u8,
        }
    }

    /// Join the three fields back into the 15-bit value that goes on the
    /// wire. Out-of-range fields are masked rather than rejected, so a
    /// DragValue that briefly overshoots cannot produce a bogus address.
    pub fn join(&self) -> u16 {
        ((self.net as u16 & 0x7F) << 8)
            | ((self.subnet as u16 & 0x0F) << 4)
            | (self.universe as u16 & 0x0F)
    }
}

impl std::fmt::Display for PortAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.net, self.subnet, self.universe)
    }
}

/// Parsed ArtPollReply: everything a node tells us about itself. Field
/// names follow the spec's table so they can be looked up directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollReply {
    pub ip: Ipv4Addr,
    /// UDP port the node listens on (always 6454 in practice).
    pub port: u16,
    /// Firmware version, high and low byte.
    pub firmware: (u8, u8),
    /// Bits 14-8 of every Port-Address this node uses.
    pub net_switch: u8,
    /// Bits 7-4 of every Port-Address this node uses.
    pub sub_switch: u8,
    pub oem: u16,
    pub ubea_version: u8,
    pub status1: u8,
    pub esta_manufacturer: u16,
    pub short_name: String,
    pub long_name: String,
    /// Free-text status such as "#0001 [0005] Power On Tests successful".
    pub node_report: String,
    /// Number of ports in this reply (0..=4).
    pub num_ports: u16,
    /// Per-port capability byte: bit 7 = can output DMX, bit 6 = can input.
    pub port_types: [u8; 4],
    pub good_input: [u8; 4],
    pub good_output: [u8; 4],
    /// Low nibble is the universe (bits 3-0) of each input port.
    pub sw_in: [u8; 4],
    /// Low nibble is the universe (bits 3-0) of each output port.
    pub sw_out: [u8; 4],
    pub style: u8,
    /// Hardware address, if the node filled it in (all-zero means unknown).
    pub mac: Option<[u8; 6]>,
    /// For multi-reply nodes: the root device's IP and this reply's index.
    pub bind_ip: Option<Ipv4Addr>,
    pub bind_index: Option<u8>,
    pub status2: Option<u8>,
}

impl PollReply {
    /// Full 15-bit Port-Address of each output port this node drives.
    ///
    /// A port counts as an output when its PortTypes byte says so. Some
    /// nodes leave PortTypes at zero yet still report ports, so when no port
    /// claims either direction every reported port is taken as an output —
    /// the common case for a plain DMX-out node.
    pub fn output_addresses(&self) -> Vec<u16> {
        let n = (self.num_ports as usize).min(4);
        let any_flagged = self.port_types[..n].iter().any(|t| t & 0xC0 != 0);
        (0..n)
            .filter(|&i| !any_flagged || self.port_types[i] & 0x80 != 0)
            .map(|i| self.port_address(self.sw_out[i]))
            .collect()
    }

    /// Full 15-bit Port-Address of each input port.
    pub fn input_addresses(&self) -> Vec<u16> {
        let n = (self.num_ports as usize).min(4);
        (0..n)
            .filter(|&i| self.port_types[i] & 0x40 != 0)
            .map(|i| self.port_address(self.sw_in[i]))
            .collect()
    }

    fn port_address(&self, sw: u8) -> u16 {
        PortAddress {
            net: self.net_switch,
            subnet: self.sub_switch,
            universe: sw & 0x0F,
        }
        .join()
    }

    /// Whether this node outputs the given Port-Address on any port.
    pub fn outputs(&self, port_address: u16) -> bool {
        self.output_addresses().contains(&port_address)
    }

    pub fn mac_string(&self) -> Option<String> {
        self.mac.map(|m| {
            m.iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(":")
        })
    }
}

/// Try to parse an ArtPollReply packet.
pub fn parse_poll_reply(buf: &[u8]) -> Option<PollReply> {
    // 207 bytes reaches the end of the MAC; everything after is optional.
    if buf.len() < 207 {
        return None;
    }
    if op_code(buf)? != OP_POLL_REPLY {
        return None;
    }
    let ip = Ipv4Addr::new(buf[10], buf[11], buf[12], buf[13]);
    let arr4 = |at: usize| [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
    let mac = [buf[201], buf[202], buf[203], buf[204], buf[205], buf[206]];
    Some(PollReply {
        ip,
        port: u16::from_le_bytes([buf[14], buf[15]]),
        firmware: (buf[16], buf[17]),
        net_switch: buf[18] & 0x7F,
        sub_switch: buf[19] & 0x0F,
        oem: u16::from_be_bytes([buf[20], buf[21]]),
        ubea_version: buf[22],
        status1: buf[23],
        esta_manufacturer: u16::from_le_bytes([buf[24], buf[25]]),
        short_name: cstr(&buf[26..26 + 18]),
        long_name: cstr(&buf[44..44 + 64]),
        node_report: cstr(&buf[108..108 + 64]),
        num_ports: u16::from_be_bytes([buf[172], buf[173]]),
        port_types: arr4(174),
        good_input: arr4(178),
        good_output: arr4(182),
        sw_in: arr4(186),
        sw_out: arr4(190),
        style: buf[200],
        mac: if mac.iter().any(|&b| b != 0) { Some(mac) } else { None },
        bind_ip: (buf.len() >= 211)
            .then(|| Ipv4Addr::new(buf[207], buf[208], buf[209], buf[210])),
        bind_index: (buf.len() >= 212).then(|| buf[211]),
        status2: (buf.len() >= 213).then(|| buf[212]),
    })
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// A synthetic ArtPollReply the way a two-port node on net 0, sub-net 1
/// answers, with a MAC and the optional tail. Shared by the codec tests and
/// the Network screen's headless render.
#[cfg(test)]
pub(crate) fn sample_reply() -> Vec<u8> {
    let mut p = vec![0u8; 214];
    p[0..8].copy_from_slice(ARTNET_ID);
    p[8..10].copy_from_slice(&OP_POLL_REPLY.to_le_bytes());
    p[10..14].copy_from_slice(&[2, 0, 0, 10]);
    p[14..16].copy_from_slice(&ARTNET_PORT.to_le_bytes());
    p[16] = 1;
    p[17] = 4;
    p[18] = 0; // net
    p[19] = 1; // sub-net
    p[26..26 + 5].copy_from_slice(b"Node1");
    p[44..44 + 9].copy_from_slice(b"Test node");
    p[108..108 + 12].copy_from_slice(b"#0001 [0003]");
    p[173] = 2; // two ports
    p[174] = 0x80; // out
    p[175] = 0x80; // out
    p[190] = 2; // sw_out universes 2 and 3
    p[191] = 3;
    p[200] = 0;
    p[201..207].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03]);
    p[207..211].copy_from_slice(&[2, 0, 0, 10]);
    p[211] = 1;
    p[212] = 0x0E;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_address_split_and_join_round_trip() {
        // net 3, sub-net 5, universe 9 → 0x0359.
        let pa = PortAddress { net: 3, subnet: 5, universe: 9 };
        assert_eq!(pa.join(), 0x0359);
        assert_eq!(PortAddress::split(0x0359), pa);
        assert_eq!(PortAddress::split(0).join(), 0);
        assert_eq!(
            PortAddress::split(0x7FFF),
            PortAddress { net: 127, subnet: 15, universe: 15 }
        );
        // Bit 15 is not part of a Port-Address.
        assert_eq!(PortAddress::split(0xFFFF).join(), 0x7FFF);
        // Out-of-range fields are masked, never shifted into a neighbour.
        assert_eq!(
            PortAddress { net: 200, subnet: 20, universe: 17 }.join(),
            (72 << 8) | (4 << 4) | 1
        );
        assert_eq!(PortAddress::split(17).to_string(), "0.1.1");
    }

    #[test]
    fn poll_reply_parses_every_field() {
        let r = parse_poll_reply(&sample_reply()).expect("parses");
        assert_eq!(r.ip, Ipv4Addr::new(2, 0, 0, 10));
        assert_eq!(r.port, 6454);
        assert_eq!(r.firmware, (1, 4));
        assert_eq!(r.short_name, "Node1");
        assert_eq!(r.long_name, "Test node");
        assert_eq!(r.node_report, "#0001 [0003]");
        assert_eq!(r.num_ports, 2);
        assert_eq!(r.output_addresses(), vec![0x12, 0x13]);
        assert!(r.outputs(0x13));
        assert!(!r.outputs(0x11));
        assert_eq!(r.mac_string().as_deref(), Some("AA:BB:CC:01:02:03"));
        assert_eq!(r.bind_index, Some(1));
        assert_eq!(r.status2, Some(0x0E));
        assert_eq!(op_name(op_code(&sample_reply()).unwrap()), "ArtPollReply");
    }

    #[test]
    fn poll_reply_without_port_flags_treats_ports_as_outputs() {
        let mut p = sample_reply();
        p[174] = 0;
        p[175] = 0;
        let r = parse_poll_reply(&p).unwrap();
        assert_eq!(r.output_addresses(), vec![0x12, 0x13]);
        // Truncated tail: optional fields absent, core still parses.
        let r = parse_poll_reply(&p[..207]).unwrap();
        assert_eq!(r.bind_ip, None);
        assert!(parse_poll_reply(&p[..100]).is_none());
    }

    #[test]
    fn dmx_header_reads_universe_and_sequence() {
        let pkt = build_dmx(7, 0x0359, &[1, 2, 3]);
        assert_eq!(dmx_header(&pkt), Some((0x0359, 7)));
        assert_eq!(dmx_header(&build_poll()), None);
    }
}
