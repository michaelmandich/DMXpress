//! Background networking thread: ArtPoll discovery + 40 Hz ArtDmx sender.

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::artnet;

/// Two contiguous application universes. Addresses 1..=512 use the selected
/// base universe; 513..=1024 use the next Art-Net universe.
pub const DMX_UNIVERSES: usize = 2;
pub const DMX_SLOTS: usize = 512 * DMX_UNIVERSES;

/// One universe frame. Keeping this as a distinct type prevents accidental
/// short payloads while retaining slice/index ergonomics throughout the mixer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame(pub [u8; DMX_SLOTS]);

impl Frame {
    pub const fn black() -> Self {
        Self([0; DMX_SLOTS])
    }

    pub fn blend_channel(&mut self, index: usize, to: u8, amount: f32) {
        if index >= DMX_SLOTS {
            return;
        }
        let from = self.0[index] as f32;
        self.0[index] = (from + (to as f32 - from) * amount.clamp(0.0, 1.0))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::black()
    }
}

impl std::ops::Deref for Frame {
    type Target = [u8; DMX_SLOTS];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Frame {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredNode {
    pub ip: Ipv4Addr,
    pub short_name: String,
    pub long_name: String,
}

/// Commands from UI to net thread.
pub enum NetCmd {
    /// Force-send an ArtPoll broadcast now.
    Poll,
    /// Set the target node IP. `None` = broadcast on every interface.
    SetTarget(Option<Ipv4Addr>),
    /// Set universe (0..=32767).
    SetUniverse(u16),
}

/// Events from net thread to UI.
pub enum NetEvent {
    Discovered(DiscoveredNode),
    Status(String),
}

/// Shared contiguous DMX buffer. Updated by UI, paged into universes by net.
pub type DmxBuffer = Arc<Mutex<Frame>>;

pub struct NetHandle {
    pub cmd_tx: Sender<NetCmd>,
    pub evt_rx: Receiver<NetEvent>,
    pub dmx: DmxBuffer,
}

pub fn spawn() -> Result<NetHandle> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<NetCmd>();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<NetEvent>();
    let dmx: DmxBuffer = Arc::new(Mutex::new(Frame::black()));
    let dmx_clone = dmx.clone();

    std::thread::Builder::new()
        .name("dmxpress-net".into())
        .spawn(move || {
            if let Err(e) = run(cmd_rx, evt_tx.clone(), dmx_clone) {
                let _ = evt_tx.send(NetEvent::Status(format!("net thread error: {e:#}")));
            }
        })?;

    Ok(NetHandle {
        cmd_tx,
        evt_rx,
        dmx,
    })
}

/// One IPv4 interface we can transmit on.
struct Iface {
    name: String,
    addr: Ipv4Addr,
    bcast: Ipv4Addr,
}

fn enumerate_ifaces() -> Vec<Iface> {
    let mut out = Vec::new();
    let addrs = match if_addrs::get_if_addrs() {
        Ok(a) => a,
        Err(_) => return out,
    };
    for iface in addrs {
        if iface.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            let bcast = v4.broadcast.unwrap_or(Ipv4Addr::BROADCAST);
            out.push(Iface {
                name: iface.name,
                addr: v4.ip,
                bcast,
            });
        }
    }
    out
}

fn bind_socket() -> Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        sock.set_reuse_port(true)?;
    }
    sock.set_broadcast(true)?;
    let addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, artnet::ARTNET_PORT).into();
    sock.bind(&addr.into())?;
    sock.set_read_timeout(Some(Duration::from_millis(5)))?;
    Ok(sock.into())
}

fn run(cmd_rx: Receiver<NetCmd>, evt_tx: Sender<NetEvent>, dmx: DmxBuffer) -> Result<()> {
    let sock = bind_socket()?;

    let ifaces = enumerate_ifaces();
    if ifaces.is_empty() {
        let _ = evt_tx.send(NetEvent::Status(
            "WARN: no non-loopback IPv4 interfaces found".into(),
        ));
    } else {
        for i in &ifaces {
            let _ = evt_tx.send(NetEvent::Status(format!(
                "iface {} {} -> bcast {}",
                i.name, i.addr, i.bcast
            )));
        }
    }

    let mut target: Option<Ipv4Addr> = None;
    let mut universe: u16 = 0;
    let mut sequence: u8 = 1;

    let poll = artnet::build_poll();

    send_broadcast(&sock, &poll, &ifaces, &evt_tx);
    let _ = evt_tx.send(NetEvent::Status(format!(
        "Listening on 0.0.0.0:{} — initial ArtPoll sent",
        artnet::ARTNET_PORT
    )));

    let mut last_dmx = Instant::now();
    let mut last_poll = Instant::now();
    let dmx_interval = Duration::from_millis(25); // 40 Hz
    let poll_interval = Duration::from_secs(5);

    let mut rx_buf = [0u8; 1500];

    loop {
        // Process UI commands.
        loop {
            match cmd_rx.try_recv() {
                Ok(NetCmd::Poll) => {
                    send_broadcast(&sock, &poll, &ifaces, &evt_tx);
                }
                Ok(NetCmd::SetTarget(t)) => {
                    target = t;
                    let _ = evt_tx.send(NetEvent::Status(match t {
                        Some(ip) => format!("Target set: {ip}"),
                        None => "Target cleared (broadcast)".into(),
                    }));
                }
                Ok(NetCmd::SetUniverse(u)) => {
                    universe = u;
                    let _ = evt_tx.send(NetEvent::Status(format!("Universe set: {u}")));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // Drain inbound packets (ArtPollReply etc).
        loop {
            match sock.recv_from(&mut rx_buf) {
                Ok((n, from)) => {
                    if let Some(reply) = artnet::parse_poll_reply(&rx_buf[..n]) {
                        let _ = evt_tx.send(NetEvent::Status(format!(
                            "ArtPollReply from {from}: {} ({})",
                            reply.short_name, reply.ip
                        )));
                        let _ = evt_tx.send(NetEvent::Discovered(DiscoveredNode {
                            ip: reply.ip,
                            short_name: reply.short_name,
                            long_name: reply.long_name,
                        }));
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(e) => {
                    let _ = evt_tx.send(NetEvent::Status(format!("recv error: {e}")));
                    break;
                }
            }
        }

        // Periodic ArtPoll.
        if last_poll.elapsed() >= poll_interval {
            send_broadcast(&sock, &poll, &ifaces, &evt_tx);
            last_poll = Instant::now();
        }

        // Periodic ArtDmx. Art-Net carries at most 512 slots per packet, so
        // page the contiguous application frame across consecutive universes.
        if last_dmx.elapsed() >= dmx_interval {
            let snapshot = *dmx.lock();
            for page in 0..DMX_UNIVERSES {
                let start = page * 512;
                let end = start + 512;
                let output_universe = universe.saturating_add(page as u16);
                let pkt = artnet::build_dmx(
                    sequence,
                    output_universe,
                    &snapshot.0[start..end],
                );

                match target {
                    Some(ip) => {
                        let dest = SocketAddrV4::new(ip, artnet::ARTNET_PORT);
                        if let Err(e) = sock.send_to(&pkt, dest) {
                            let _ = evt_tx.send(NetEvent::Status(format!(
                                "send error to {ip} (universe {output_universe}): {e}"
                            )));
                        }
                    }
                    None => {
                        send_broadcast(&sock, &pkt, &ifaces, &evt_tx);
                    }
                }
            }
            sequence = sequence.wrapping_add(1).max(1);
            last_dmx = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(2));
    }
}

fn send_broadcast(
    sock: &UdpSocket,
    pkt: &[u8],
    ifaces: &[Iface],
    evt_tx: &Sender<NetEvent>,
) {
    let limited = SocketAddrV4::new(Ipv4Addr::BROADCAST, artnet::ARTNET_PORT);
    if let Err(e) = sock.send_to(pkt, limited) {
        let _ = evt_tx.send(NetEvent::Status(format!(
            "send error to 255.255.255.255: {e}"
        )));
    }
    // For each interface, open a temp socket bound to its IP and send the broadcast.
    // This forces the packet out that specific interface (the kernel otherwise picks
    // one by routing table, usually the default-route iface only).
    for i in ifaces {
        if let Err(e) = send_from_iface(i, pkt) {
            let _ = evt_tx.send(NetEvent::Status(format!(
                "send error via {} ({}): {e}",
                i.name, i.addr
            )));
        }
    }
}

fn send_from_iface(i: &Iface, pkt: &[u8]) -> std::io::Result<()> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        s.set_reuse_port(true)?;
    }
    s.set_broadcast(true)?;
    // Bind to the iface's address with ephemeral port — forces egress via this iface.
    let bind: SocketAddr = SocketAddrV4::new(i.addr, 0).into();
    s.bind(&bind.into())?;
    // Send to the iface's directed broadcast AND to limited broadcast.
    let dir: SocketAddr = SocketAddrV4::new(i.bcast, artnet::ARTNET_PORT).into();
    s.send_to(pkt, &dir.into())?;
    let lim: SocketAddr = SocketAddrV4::new(Ipv4Addr::BROADCAST, artnet::ARTNET_PORT).into();
    let _ = s.send_to(pkt, &lim.into());
    Ok(())
}
