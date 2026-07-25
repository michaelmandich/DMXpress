//! Minimal Art-Net (v4) codec.
//!
//! Spec references:
//! - ID: "Art-Net\0"
//! - OpPoll      = 0x2000
//! - OpPollReply = 0x2100
//! - OpDmx       = 0x5000
//! - Protocol version = 14
//! - Port = 6454

use std::net::Ipv4Addr;

pub const ARTNET_PORT: u16 = 6454;
pub const ARTNET_ID: &[u8; 8] = b"Art-Net\0";
pub const PROTOCOL_VERSION: u16 = 14;

pub const OP_POLL: u16 = 0x2000;
pub const OP_POLL_REPLY: u16 = 0x2100;
pub const OP_DMX: u16 = 0x5000;

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

/// Parsed ArtPollReply (only the fields we care about).
#[derive(Debug, Clone)]
pub struct PollReply {
    pub ip: Ipv4Addr,
    pub short_name: String,
    pub long_name: String,
}

/// Try to parse an ArtPollReply packet.
pub fn parse_poll_reply(buf: &[u8]) -> Option<PollReply> {
    if buf.len() < 207 {
        return None;
    }
    if &buf[0..8] != ARTNET_ID {
        return None;
    }
    let op = u16::from_le_bytes([buf[8], buf[9]]);
    if op != OP_POLL_REPLY {
        return None;
    }
    let ip = Ipv4Addr::new(buf[10], buf[11], buf[12], buf[13]);
    let short = cstr(&buf[26..26 + 18]);
    let long = cstr(&buf[44..44 + 64]);
    Some(PollReply {
        ip,
        short_name: short,
        long_name: long,
    })
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}
