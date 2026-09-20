//! Background networking thread: Art-Net and sACN output, node discovery,
//! and the live diagnostics the Network screen shows.
//!
//! One thread owns every socket. The UI never touches the network; it
//! pushes commands down `cmd_tx` (a new [`NetConfig`], a poll request, a
//! test pattern) and reads results off two channels:
//!
//! - `evt_rx` carries the original [`NetEvent`]s — discovered nodes and log
//!   lines — which the app drains into its log every frame.
//! - `diag_rx` carries [`DiagEvent`]s for the Network screen: per-universe
//!   send rates, full ArtPollReply records, sACN sources heard on the wire,
//!   and a summary of every packet in or out. It is a *bounded* channel and
//!   the thread drops on overflow, so a closed Network window can never
//!   make the thread block or the process grow.
//!
//! The 40 Hz sender pages the app's contiguous 1024-slot frame across two
//! universes and can emit each page over Art-Net, sACN, or both. Art-Net
//! goes out through one socket per interface bound to that interface's
//! address, because a limited broadcast otherwise leaves only by the
//! default route — usually the internet adapter, not the lighting one.
//! sACN is multicast (or unicast) from a socket whose multicast interface
//! is the chosen one. Every socket is non-blocking and the loop sleeps two
//! milliseconds between passes, so a stalled network cannot stall the UI.
//!
//! Test patterns are applied *here*, to the outgoing copy of the frame:
//! the mixer's shared buffer is never written, so when the pattern ends
//! the show resumes exactly where it was.

pub(crate) mod checklist;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol as SockProto, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::artnet::{self, PollReply};
use crate::sacn::{self, Cid};

/// Contiguous application universes. Address 1 starts at the selected base
/// universe and each further block of 512 slots goes out on the next one,
/// so the console's address space is one flat 1..=[`DMX_SLOTS`] range.
///
/// Everything downstream is written against this constant: the frame, the
/// mixer, the sender's paging, the per-universe counters and the addressing
/// maths. Widening the console is this line plus new hardware.
pub const DMX_UNIVERSES: usize = 4;

/// Highest Art-Net Port-Address (15 bits: net, sub-net and universe joined).
pub const ARTNET_MAX_PORT_ADDRESS: u16 = 0x7FFF;

/// Ceiling on sACN multicast group memberships for one socket. The OS limit
/// is 20 on Linux (`igmp_max_memberships`) and comparable on Windows; past
/// it every join fails and the failures read as socket errors.
const MAX_MULTICAST_JOINS: usize = 18;
pub const DMX_SLOTS: usize = 512 * DMX_UNIVERSES;

/// Where the network configuration lives, next to the other state files.
pub const NETWORK_FILE: &str = "network.json";

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

// ---- configuration ----

/// Which wire protocol(s) carry the show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Protocol {
    #[default]
    ArtNet,
    Sacn,
    Both,
}

impl Protocol {
    pub fn artnet(self) -> bool {
        matches!(self, Protocol::ArtNet | Protocol::Both)
    }
    pub fn sacn(self) -> bool {
        matches!(self, Protocol::Sacn | Protocol::Both)
    }
    pub fn label(self) -> &'static str {
        match self {
            Protocol::ArtNet => "Art-Net",
            Protocol::Sacn => "sACN (E1.31)",
            Protocol::Both => "Art-Net + sACN",
        }
    }
}

/// How ArtDmx packets are addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ArtNetMode {
    /// Unicast to the node picked in the Art-Net panel, else broadcast
    /// everywhere — what the app did before this screen existed.
    #[default]
    Auto,
    /// Limited broadcast, 255.255.255.255.
    Broadcast,
    /// The interface's own subnet broadcast (2.255.255.255 on a 2.x/8 net).
    DirectedBroadcast,
    /// One fixed node.
    Unicast,
    /// A unicast copy to every node that answered an ArtPoll.
    AllNodes,
}

impl ArtNetMode {
    pub const ALL: [ArtNetMode; 5] = [
        ArtNetMode::Auto,
        ArtNetMode::Broadcast,
        ArtNetMode::DirectedBroadcast,
        ArtNetMode::Unicast,
        ArtNetMode::AllNodes,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ArtNetMode::Auto => "Auto (selected node, else broadcast)",
            ArtNetMode::Broadcast => "Broadcast 255.255.255.255",
            ArtNetMode::DirectedBroadcast => "Directed broadcast (subnet)",
            ArtNetMode::Unicast => "Unicast to one node",
            ArtNetMode::AllNodes => "Unicast to every discovered node",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ArtNetConfig {
    pub mode: ArtNetMode,
    /// Target for [`ArtNetMode::Unicast`].
    pub target: Option<Ipv4Addr>,
    /// Explicit Port-Address per application universe; `None` means the
    /// base universe and the ones after it.
    ///
    /// A `Vec`, not `[u16; DMX_UNIVERSES]`: a fixed-length array refuses a
    /// saved file with a different count, and because the whole config is
    /// read with one `from_str`, that one field would throw away the
    /// interface, priority, unicast list and the sACN CID with it. Short
    /// lists are filled in from the base, long ones ignored.
    pub explicit: Option<Vec<u16>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SacnConfig {
    /// First sACN universe; the pages after it follow on from the base.
    pub base_universe: u16,
    /// Explicit universe per application universe, overriding the base.
    /// A `Vec` for the reason given on [`ArtNetConfig::explicit`].
    pub explicit: Option<Vec<u16>>,
    pub priority: u8,
    pub source_name: String,
    /// Multicast to 239.255.x.y — what nearly every receiver expects.
    pub multicast: bool,
    /// Extra unicast copies, for receivers that cannot join multicast.
    pub unicast: Vec<Ipv4Addr>,
    /// Our component identifier, generated once and kept for good so
    /// receivers keep recognising this console across restarts.
    pub cid: Cid,
}

impl Default for SacnConfig {
    fn default() -> Self {
        Self {
            base_universe: 1,
            explicit: None,
            priority: sacn::DEFAULT_PRIORITY,
            source_name: "DMXpress".into(),
            multicast: true,
            unicast: Vec::new(),
            cid: sacn::generate_cid(),
        }
    }
}

/// Everything the Network screen configures, persisted to `network.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    pub protocol: Protocol,
    /// Interface to send from; `None` = every interface / OS default.
    pub interface: Option<Ipv4Addr>,
    pub artnet: ArtNetConfig,
    pub sacn: SacnConfig,
    /// Seconds between automatic ArtPolls.
    pub poll_interval_s: f32,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            protocol: Protocol::ArtNet,
            interface: None,
            artnet: ArtNetConfig::default(),
            sacn: SacnConfig::default(),
            poll_interval_s: 5.0,
        }
    }
}

impl NetConfig {
    /// Read `network.json`, or start from defaults. A missing file is
    /// written straight away so the freshly generated CID is pinned.
    pub fn load() -> Self {
        match std::fs::read_to_string(NETWORK_FILE) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(mut cfg) => {
                    cfg.normalize();
                    // A file from before the CID existed gets one now, once.
                    if !text.contains("\"cid\"") {
                        cfg.save();
                    }
                    cfg
                }
                Err(_) => {
                    // Falling back to defaults silently would be the worst
                    // outcome here: `SacnConfig::default` mints a fresh CID,
                    // so every restart would look like a different console
                    // to every receiver, and the interface, priority and
                    // unicast list would be gone too. Re-save at once so the
                    // new identity is at least pinned rather than re-rolled
                    // on every launch, and leave the unreadable file's
                    // contents behind in a sibling so nothing is lost.
                    let _ = std::fs::write(format!("{NETWORK_FILE}.bad"), &text);
                    let cfg = Self::default();
                    cfg.save();
                    cfg
                }
            },
            Err(_) => {
                let cfg = Self::default();
                cfg.save();
                cfg
            }
        }
    }

    /// Bring a loaded config into line with the console's universe count,
    /// so everything downstream can assume an explicit list is exactly
    /// `DMX_UNIVERSES` long. A file written when the console had fewer
    /// universes is filled out from the base; one written when it had more
    /// is trimmed.
    fn normalize(&mut self) {
        let base = self.sacn.base_universe;
        if let Some(e) = &mut self.sacn.explicit {
            for i in e.len()..DMX_UNIVERSES {
                e.push(base.saturating_add(i as u16));
            }
            e.truncate(DMX_UNIVERSES);
        }
        if let Some(e) = &mut self.artnet.explicit {
            let from = e.last().copied().unwrap_or(0);
            for i in e.len()..DMX_UNIVERSES {
                e.push(from.saturating_add(1).saturating_add(i as u16 - 1));
            }
            e.truncate(DMX_UNIVERSES);
        }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(NETWORK_FILE, json);
        }
    }

    /// Art-Net Port-Address of each application universe given the base
    /// universe set in the toolbar.
    pub fn artnet_universes(&self, base: u16) -> [u16; DMX_UNIVERSES] {
        // Clamp the BASE so the whole block still fits, rather than clamping
        // each page: pinning every overflowing page to the top value would
        // send several different 512-slot pages to one Port-Address at 40 Hz
        // and quietly drop all but the last.
        let top = ARTNET_MAX_PORT_ADDRESS - (DMX_UNIVERSES as u16 - 1);
        let base = base.min(top);
        match &self.artnet.explicit {
            Some(e) => std::array::from_fn(|i| {
                e.get(i).copied().unwrap_or(base + i as u16) & ARTNET_MAX_PORT_ADDRESS
            }),
            None => std::array::from_fn(|i| base + i as u16),
        }
    }

    /// sACN universe of each application universe.
    pub fn sacn_universes(&self) -> [u16; DMX_UNIVERSES] {
        let top = sacn::MAX_UNIVERSE - (DMX_UNIVERSES as u16 - 1);
        let base = self
            .sacn
            .base_universe
            .clamp(sacn::MIN_UNIVERSE, top.max(sacn::MIN_UNIVERSE));
        let clamp = |u: u16| u.clamp(sacn::MIN_UNIVERSE, sacn::MAX_UNIVERSE);
        match &self.sacn.explicit {
            Some(e) => std::array::from_fn(|i| clamp(e.get(i).copied().unwrap_or(base + i as u16))),
            None => std::array::from_fn(|i| clamp(base + i as u16)),
        }
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs_f32(self.poll_interval_s.clamp(1.0, 60.0))
    }
}

// ---- interfaces ----

/// One IPv4 interface we can transmit on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfaceInfo {
    pub name: String,
    pub addr: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub prefix: u8,
    /// Directed (subnet) broadcast address.
    pub bcast: Ipv4Addr,
}

impl IfaceInfo {
    /// Whether `ip` is on this interface's subnet.
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let m = u32::from(self.mask);
        (u32::from(ip) & m) == (u32::from(self.addr) & m)
    }

    /// The subnet broadcast for an address and mask (2.0.0.1/8 → 2.255.255.255).
    pub fn directed_broadcast(addr: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(addr) | !u32::from(mask))
    }

    /// Lighting gear ships on 2.x.x.x (or 10.x.x.x) by convention.
    pub fn on_artnet_range(&self) -> bool {
        matches!(self.addr.octets()[0], 2 | 10)
    }
}

/// Every non-loopback IPv4 interface, in OS order.
pub fn interfaces() -> Vec<IfaceInfo> {
    let mut out = Vec::new();
    let Ok(addrs) = if_addrs::get_if_addrs() else {
        return out;
    };
    for iface in addrs {
        if iface.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            let bcast = v4
                .broadcast
                .unwrap_or_else(|| IfaceInfo::directed_broadcast(v4.ip, v4.netmask));
            out.push(IfaceInfo {
                name: iface.name,
                addr: v4.ip,
                mask: v4.netmask,
                prefix: v4.prefixlen,
                bcast,
            });
        }
    }
    out
}

// ---- commands and events ----

/// What a test pattern puts on one application universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// Every slot at 255.
    FullOn,
    /// One slot at 255 walking up the universe, 20 slots a second.
    Chase,
    /// A single 1-based channel at 255, everything else at 0.
    Single(u16),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TestPattern {
    /// Application universe index (0 or 1).
    pub page: usize,
    pub kind: TestKind,
    /// How long to hold it before the show comes back.
    pub seconds: f32,
}

/// Commands from UI to net thread.
pub enum NetCmd {
    /// Force-send an ArtPoll broadcast now.
    Poll,
    /// Set the target node IP. `None` = broadcast on every interface.
    SetTarget(Option<Ipv4Addr>),
    /// Set universe (0..=32767).
    SetUniverse(u16),
    /// Replace the whole network configuration.
    Configure(NetConfig),
    /// Start (`Some`) or stop (`None`) overriding the output with a pattern.
    TestPattern(Option<TestPattern>),
    /// Whether to emit a [`DiagEvent::Packet`] for every packet.
    Monitor(bool),
}

/// Events from net thread to UI.
pub enum NetEvent {
    Discovered(DiscoveredNode),
    Status(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    ArtNet,
    Sacn,
}

impl Proto {
    pub fn label(self) -> &'static str {
        match self {
            Proto::ArtNet => "Art-Net",
            Proto::Sacn => "sACN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
}

/// One packet, summarised for the monitor.
#[derive(Debug, Clone)]
pub struct PacketSummary {
    /// Seconds since the thread started.
    pub t: f64,
    pub dir: Direction,
    pub proto: Proto,
    pub kind: &'static str,
    /// Destination when sent, source when received.
    pub addr: SocketAddrV4,
    pub universe: Option<u16>,
    pub sequence: Option<u8>,
    pub len: usize,
    pub raw: Vec<u8>,
}

/// Send statistics for one application universe over one protocol.
#[derive(Debug, Clone)]
pub struct UniverseStat {
    pub proto: Proto,
    pub page: usize,
    pub universe: u16,
    /// Frames actually sent per second over the last window.
    pub fps: f32,
    pub bytes_per_s: f32,
    pub last_send: Option<Instant>,
    pub total_frames: u64,
    pub destinations: Vec<SocketAddrV4>,
}

/// A snapshot of the thread's health, published once a second.
#[derive(Debug, Clone, Default)]
pub struct NetStats {
    pub universes: Vec<UniverseStat>,
    /// One line per open socket: what it is bound to and does.
    pub sockets: Vec<String>,
    pub last_error: Option<(String, Instant)>,
    pub error_count: u64,
    pub artnet_listening: bool,
    pub sacn_ready: bool,
    pub sacn_listening: bool,
    /// The interface multicast was pinned to, or `None` for the OS default.
    pub multicast_if: Option<Ipv4Addr>,
    pub uptime_s: f32,
    /// Whether a test pattern is overriding output right now.
    pub test_active: bool,
}

/// Diagnostics for the Network screen.
pub enum DiagEvent {
    Stats(NetStats),
    Node { reply: PollReply, from: Ipv4Addr },
    SacnDiscovery { info: sacn::DiscoveryInfo, from: Ipv4Addr },
    SacnData { info: sacn::DataInfo, from: Ipv4Addr },
    Packet(PacketSummary),
    PollSent,
    TestEnded,
}

/// Shared contiguous DMX buffer. Updated by UI, paged into universes by net.
pub type DmxBuffer = Arc<Mutex<Frame>>;

pub struct NetHandle {
    pub cmd_tx: Sender<NetCmd>,
    pub evt_rx: Receiver<NetEvent>,
    /// Diagnostics for the Network screen; drained there, never by the app.
    pub diag_rx: Receiver<DiagEvent>,
    pub dmx: DmxBuffer,
    /// The configuration the thread started with.
    pub config: NetConfig,
}

pub fn spawn() -> Result<NetHandle> {
    let config = NetConfig::load();
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<NetCmd>();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<NetEvent>();
    // Bounded: the packet monitor can produce hundreds of events a second
    // and nobody drains them while the Network window is closed.
    let (diag_tx, diag_rx) = crossbeam_channel::bounded::<DiagEvent>(4096);
    let dmx: DmxBuffer = Arc::new(Mutex::new(Frame::black()));
    let dmx_clone = dmx.clone();
    let cfg = config.clone();

    std::thread::Builder::new()
        .name("dmxpress-net".into())
        .spawn(move || {
            let mut net = Net::new(cfg, evt_tx.clone(), diag_tx, dmx_clone);
            if let Err(e) = net.run(cmd_rx) {
                let _ = evt_tx.send(NetEvent::Status(format!("net thread error: {e:#}")));
            }
        })?;

    Ok(NetHandle {
        cmd_tx,
        evt_rx,
        diag_rx,
        dmx,
        config,
    })
}

// ---- the thread ----

const DMX_INTERVAL: Duration = Duration::from_millis(25); // 40 Hz
const IFACE_REFRESH: Duration = Duration::from_secs(5);
const SOCKET_RETRY: Duration = Duration::from_secs(3);
const STATS_WINDOW: Duration = Duration::from_secs(1);
const NODE_EXPIRY: Duration = Duration::from_secs(60);

fn new_udp(bind: SocketAddrV4, broadcast: bool) -> std::io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(SockProto::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        s.set_reuse_port(true)?;
    }
    if broadcast {
        s.set_broadcast(true)?;
    }
    s.bind(&SocketAddr::from(bind).into())?;
    s.set_nonblocking(true)?;
    Ok(s.into())
}

/// A send socket bound to one interface's address, so what leaves it
/// leaves by that interface.
struct Egress {
    iface: IfaceInfo,
    sock: UdpSocket,
}

#[derive(Default, Clone, Copy)]
struct Counter {
    frames: u32,
    bytes: u64,
    total: u64,
    last: Option<Instant>,
}

struct ActiveTest {
    pattern: TestPattern,
    started: Instant,
    ends: Instant,
}

struct Net {
    cfg: NetConfig,
    universe: u16,
    legacy_target: Option<Ipv4Addr>,
    ifaces: Vec<IfaceInfo>,
    ifaces_at: Instant,
    artnet_rx: Option<UdpSocket>,
    artnet_tx: Vec<Egress>,
    sacn_tx: Option<UdpSocket>,
    sacn_rx: Option<UdpSocket>,
    /// (universe, interface) groups the sACN listener has joined.
    sacn_joined: Vec<(u16, Ipv4Addr)>,
    sockets_at: Option<Instant>,
    seq_artnet: u8,
    seq_sacn: [u8; DMX_UNIVERSES],
    nodes: Vec<(PollReply, Instant)>,
    last_poll: Instant,
    last_discovery: Instant,
    last_dmx: Instant,
    test: Option<ActiveTest>,
    monitor: bool,
    counters: [[Counter; DMX_UNIVERSES]; 2],
    destinations: [[Vec<SocketAddrV4>; DMX_UNIVERSES]; 2],
    window_start: Instant,
    last_error: Option<(String, Instant)>,
    error_count: u64,
    last_logged_error: Option<(String, Instant)>,
    started: Instant,
    evt_tx: Sender<NetEvent>,
    diag_tx: Sender<DiagEvent>,
    dmx: DmxBuffer,
    rx_buf: [u8; 2048],
}

impl Net {
    fn new(cfg: NetConfig, evt_tx: Sender<NetEvent>, diag_tx: Sender<DiagEvent>, dmx: DmxBuffer) -> Self {
        let now = Instant::now();
        Self {
            cfg,
            universe: 0,
            legacy_target: None,
            ifaces: Vec::new(),
            ifaces_at: now,
            artnet_rx: None,
            artnet_tx: Vec::new(),
            sacn_tx: None,
            sacn_rx: None,
            sacn_joined: Vec::new(),
            sockets_at: None,
            seq_artnet: 1,
            seq_sacn: [0; DMX_UNIVERSES],
            nodes: Vec::new(),
            last_poll: now,
            last_discovery: now - Duration::from_secs(sacn::DISCOVERY_INTERVAL_S),
            last_dmx: now,
            test: None,
            monitor: false,
            counters: Default::default(),
            destinations: Default::default(),
            window_start: now,
            last_error: None,
            error_count: 0,
            last_logged_error: None,
            started: now,
            evt_tx,
            diag_tx,
            dmx,
            rx_buf: [0; 2048],
        }
    }

    fn status(&self, s: String) {
        let _ = self.evt_tx.send(NetEvent::Status(s));
    }

    fn diag(&self, e: DiagEvent) {
        match self.diag_tx.try_send(e) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Record a socket failure for the status strip and, rate-limited so a
    /// dead link does not flood the log at 80 packets a second, the log.
    fn fail(&mut self, what: &str, e: &std::io::Error) {
        let msg = format!("{what}: {e}");
        let now = Instant::now();
        self.error_count += 1;
        let repeat = self
            .last_logged_error
            .as_ref()
            .is_some_and(|(m, at)| *m == msg && now.duration_since(*at) < Duration::from_secs(5));
        if !repeat {
            self.status(msg.clone());
            self.last_logged_error = Some((msg.clone(), now));
        }
        self.last_error = Some((msg, now));
    }

    fn refresh_ifaces(&mut self, force: bool) {
        self.ifaces_at = Instant::now();
        let fresh = interfaces();
        if fresh != self.ifaces || force {
            self.ifaces = fresh;
            self.rebuild_sockets();
        }
    }

    /// The interface(s) we send from: the selected one, or all of them.
    fn send_ifaces(&self) -> Vec<IfaceInfo> {
        match self.cfg.interface {
            Some(ip) => self.ifaces.iter().filter(|i| i.addr == ip).cloned().collect(),
            None => self.ifaces.clone(),
        }
    }

    fn rebuild_sockets(&mut self) {
        self.sockets_at = Some(Instant::now());
        // Art-Net listener: any address, so replies arrive whichever
        // interface the node is on and whichever way it addresses them.
        if self.artnet_rx.is_none() {
            match new_udp(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, artnet::ARTNET_PORT), true) {
                Ok(s) => self.artnet_rx = Some(s),
                Err(e) => self.fail("bind 0.0.0.0:6454 for Art-Net", &e),
            }
        }
        // Art-Net egress: one socket per interface we send from.
        self.artnet_tx.clear();
        for iface in self.send_ifaces() {
            match new_udp(SocketAddrV4::new(iface.addr, 0), true) {
                Ok(sock) => self.artnet_tx.push(Egress { iface, sock }),
                Err(e) => self.fail(&format!("bind {} for Art-Net", iface.addr), &e),
            }
        }
        if self.cfg.interface.is_some() && self.artnet_tx.is_empty() {
            let ip = self.cfg.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);
            self.status(format!("WARN: selected interface {ip} not present — sending via OS default"));
        }
        // sACN sender: multicast pinned to the chosen interface.
        self.sacn_tx = None;
        let bind_ip = self
            .cfg
            .interface
            .filter(|ip| self.ifaces.iter().any(|i| i.addr == *ip))
            .unwrap_or(Ipv4Addr::UNSPECIFIED);
        match new_udp(SocketAddrV4::new(bind_ip, 0), false) {
            Ok(s) => {
                let s2 = Socket::from(s);
                if bind_ip != Ipv4Addr::UNSPECIFIED {
                    if let Err(e) = s2.set_multicast_if_v4(&bind_ip) {
                        self.fail(&format!("set multicast interface {bind_ip}"), &e);
                    }
                }
                let _ = s2.set_multicast_ttl_v4(1);
                self.sacn_tx = Some(s2.into());
            }
            Err(e) => self.fail("open sACN send socket", &e),
        }
        // sACN listener: rejoin every group on the new interface set.
        self.sacn_rx = None;
        self.sacn_joined.clear();
        self.ensure_sacn_rx();
    }

    /// Open the sACN listener if needed and join the discovery group plus
    /// our own universes' groups, so other sources on them show up.
    fn ensure_sacn_rx(&mut self) {
        if !self.cfg.protocol.sacn() {
            self.sacn_rx = None;
            self.sacn_joined.clear();
            return;
        }
        if self.sacn_rx.is_none() {
            match new_udp(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, sacn::SACN_PORT), false) {
                Ok(s) => self.sacn_rx = Some(s),
                Err(e) => {
                    self.fail("bind 0.0.0.0:5568 for sACN", &e);
                    return;
                }
            }
        }
        let mut wanted: Vec<(u16, Ipv4Addr)> = Vec::new();
        let ifaces = self.send_ifaces();
        let universes = self.cfg.sacn_universes();
        // Discovery goes on every interface — it is one group and it is how
        // other sources are found at all. Our own universes do not: joining
        // N universes on every interface runs into the per-socket membership
        // cap (20 on Linux, comparable on Windows), and a normal Windows box
        // with Ethernet, Wi-Fi, a Hyper-V vEthernet and a VPN is already at
        // four interfaces. Past the cap each join fails, gets reported as a
        // socket error, and the console quietly stops hearing other sources
        // on those universes — which is exactly when the priority-conflict
        // checks are needed.
        for iface in &ifaces {
            wanted.push((sacn::DISCOVERY_UNIVERSE, iface.addr));
        }
        let listen_on: Vec<Ipv4Addr> = match self.cfg.interface {
            // An explicitly chosen interface is the lighting NIC; listen there.
            Some(ip) if ifaces.iter().any(|i| i.addr == ip) => vec![ip],
            _ => ifaces.iter().map(|i| i.addr).collect(),
        };
        let mut dropped = 0usize;
        for ip in &listen_on {
            for u in universes {
                if wanted.len() >= MAX_MULTICAST_JOINS {
                    dropped += 1;
                    continue;
                }
                wanted.push((u, *ip));
            }
        }
        if dropped > 0 {
            self.status(format!(
                "WARN: {dropped} sACN group join(s) skipped — too many network interfaces.                  Pick the lighting interface to listen on all {} universes.",
                DMX_UNIVERSES
            ));
        }
        if ifaces.is_empty() {
            wanted.push((sacn::DISCOVERY_UNIVERSE, Ipv4Addr::UNSPECIFIED));
        }
        let Some(rx) = self.sacn_rx.take() else { return };
        let sock = Socket::from(rx);
        let mut failures = Vec::new();
        for (u, ip) in &self.sacn_joined {
            if !wanted.contains(&(*u, *ip)) {
                let _ = sock.leave_multicast_v4(&sacn::multicast_addr(*u), ip);
            }
        }
        for (u, ip) in &wanted {
            if !self.sacn_joined.contains(&(*u, *ip)) {
                if let Err(e) = sock.join_multicast_v4(&sacn::multicast_addr(*u), ip) {
                    failures.push((format!("join {} on {ip}", sacn::multicast_addr(*u)), e));
                }
            }
        }
        self.sacn_joined = wanted;
        self.sacn_rx = Some(sock.into());
        for (what, e) in failures {
            self.fail(&what, &e);
        }
    }

    fn artnet_universes(&self) -> [u16; DMX_UNIVERSES] {
        self.cfg.artnet_universes(self.universe)
    }

    /// Where each ArtDmx packet goes: (destination, egress socket index or
    /// `None` for the listener socket, which the OS routes).
    fn artnet_targets(&self) -> Vec<(SocketAddrV4, Option<usize>)> {
        let port = artnet::ARTNET_PORT;
        let unicast = |ip: Ipv4Addr| (SocketAddrV4::new(ip, port), self.default_egress());
        let mut out = Vec::new();
        match self.cfg.artnet.mode {
            ArtNetMode::Auto => match self.legacy_target {
                Some(ip) => out.push(unicast(ip)),
                None => out.extend(self.broadcast_targets(true)),
            },
            ArtNetMode::Broadcast => {
                if self.artnet_tx.is_empty() {
                    out.push((SocketAddrV4::new(Ipv4Addr::BROADCAST, port), None));
                }
                for i in 0..self.artnet_tx.len() {
                    out.push((SocketAddrV4::new(Ipv4Addr::BROADCAST, port), Some(i)));
                }
            }
            ArtNetMode::DirectedBroadcast => out.extend(self.broadcast_targets(false)),
            ArtNetMode::Unicast => {
                if let Some(ip) = self.cfg.artnet.target {
                    out.push(unicast(ip));
                }
            }
            ArtNetMode::AllNodes => {
                for (n, _) in &self.nodes {
                    out.push(unicast(n.ip));
                }
            }
        }
        out
    }

    /// Directed broadcast on every egress interface, plus the limited
    /// broadcast when `limited` (what the app always did in Auto mode).
    fn broadcast_targets(&self, limited: bool) -> Vec<(SocketAddrV4, Option<usize>)> {
        let port = artnet::ARTNET_PORT;
        let mut out = Vec::new();
        for (i, e) in self.artnet_tx.iter().enumerate() {
            out.push((SocketAddrV4::new(e.iface.bcast, port), Some(i)));
            if limited {
                out.push((SocketAddrV4::new(Ipv4Addr::BROADCAST, port), Some(i)));
            }
        }
        if self.artnet_tx.is_empty() {
            out.push((SocketAddrV4::new(Ipv4Addr::BROADCAST, port), None));
        }
        out
    }

    /// Unicast leaves by the selected interface when there is one, else by
    /// the listener socket so the OS routes it.
    fn default_egress(&self) -> Option<usize> {
        (self.cfg.interface.is_some() && !self.artnet_tx.is_empty()).then_some(0)
    }

    fn send_artnet(&mut self, pkt: &[u8], kind: &'static str, page: Option<usize>) -> usize {
        let mut sent = 0;
        let targets = self.artnet_targets();
        let header = artnet::dmx_header(pkt);
        for (dest, egress) in targets {
            let res = match egress {
                Some(i) => self.artnet_tx[i].sock.send_to(pkt, dest),
                None => match &self.artnet_rx {
                    Some(s) => s.send_to(pkt, dest),
                    None => continue,
                },
            };
            match res {
                Ok(n) => {
                    sent += 1;
                    if let Some(p) = page {
                        let c = &mut self.counters[0][p];
                        c.bytes += n as u64;
                        if !self.destinations[0][p].contains(&dest) {
                            self.destinations[0][p].push(dest);
                        }
                    }
                    self.monitor_packet(Direction::Sent, Proto::ArtNet, kind, dest, header.map(|h| h.0), header.map(|h| h.1), pkt);
                }
                Err(e) => self.fail(&format!("send {kind} to {dest}"), &e),
            }
        }
        sent
    }

    fn sacn_targets(&self, universe: u16) -> Vec<SocketAddrV4> {
        let mut out = Vec::new();
        if self.cfg.sacn.multicast {
            out.push(SocketAddrV4::new(sacn::multicast_addr(universe), sacn::SACN_PORT));
        }
        for ip in &self.cfg.sacn.unicast {
            out.push(SocketAddrV4::new(*ip, sacn::SACN_PORT));
        }
        out
    }

    /// Send one E1.31 packet to every destination for `universe` (data and
    /// terminate packets) or to the discovery group alone (`targets`).
    fn send_sacn(
        &mut self,
        pkt: &[u8],
        kind: &'static str,
        universe: u16,
        seq: Option<u8>,
        page: Option<usize>,
        targets: Vec<SocketAddrV4>,
    ) {
        let Some(sock) = self.sacn_tx.take() else { return };
        for dest in targets {
            match sock.send_to(pkt, dest) {
                Ok(n) => {
                    if let Some(p) = page {
                        let c = &mut self.counters[1][p];
                        c.bytes += n as u64;
                        if !self.destinations[1][p].contains(&dest) {
                            self.destinations[1][p].push(dest);
                        }
                    }
                    self.monitor_packet(Direction::Sent, Proto::Sacn, kind, dest, Some(universe), seq, pkt);
                }
                Err(e) => self.fail(&format!("send {kind} to {dest}"), &e),
            }
        }
        self.sacn_tx = Some(sock);
    }

    #[allow(clippy::too_many_arguments)]
    fn monitor_packet(
        &self,
        dir: Direction,
        proto: Proto,
        kind: &'static str,
        addr: SocketAddrV4,
        universe: Option<u16>,
        sequence: Option<u8>,
        raw: &[u8],
    ) {
        if !self.monitor {
            return;
        }
        self.diag(DiagEvent::Packet(PacketSummary {
            t: self.started.elapsed().as_secs_f64(),
            dir,
            proto,
            kind,
            addr,
            universe,
            sequence,
            len: raw.len(),
            raw: raw.to_vec(),
        }));
    }

    fn poll(&mut self) {
        let poll = artnet::build_poll();
        // ArtPoll is always broadcast, whatever the DMX addressing mode, and
        // a unicast target gets its own copy since Art-Net 4 nodes may sit
        // behind a switch that drops broadcasts.
        let mut targets = self.broadcast_targets(true);
        if let Some(ip) = self.cfg.artnet.target.filter(|_| self.cfg.artnet.mode == ArtNetMode::Unicast) {
            targets.push((SocketAddrV4::new(ip, artnet::ARTNET_PORT), self.default_egress()));
        }
        for (dest, egress) in targets {
            let res = match egress {
                Some(i) => self.artnet_tx[i].sock.send_to(&poll, dest),
                None => match &self.artnet_rx {
                    Some(s) => s.send_to(&poll, dest),
                    None => continue,
                },
            };
            match res {
                Ok(_) => self.monitor_packet(Direction::Sent, Proto::ArtNet, "ArtPoll", dest, None, None, &poll),
                Err(e) => self.fail(&format!("send ArtPoll to {dest}"), &e),
            }
        }
        self.last_poll = Instant::now();
        self.diag(DiagEvent::PollSent);
    }

    fn discovery(&mut self) {
        let universes = self.cfg.sacn_universes().to_vec();
        let pkt = sacn::build_discovery(&self.cfg.sacn.cid, &self.cfg.sacn.source_name, 0, 0, &universes);
        let group = vec![SocketAddrV4::new(sacn::multicast_addr(sacn::DISCOVERY_UNIVERSE), sacn::SACN_PORT)];
        self.send_sacn(&pkt, "E1.31 Discovery", sacn::DISCOVERY_UNIVERSE, None, None, group);
        self.last_discovery = Instant::now();
    }

    /// Tell receivers we have stopped on `universe` (three packets with the
    /// terminated bit, as E1.31 asks) so they drop us at once instead of
    /// holding the last look for their timeout.
    fn terminate(&mut self, universe: u16, page: usize) {
        for _ in 0..3 {
            self.seq_sacn[page] = self.seq_sacn[page].wrapping_add(1);
            let pkt = sacn::build_data(
                &self.cfg.sacn.cid,
                &self.cfg.sacn.source_name,
                self.cfg.sacn.priority,
                self.seq_sacn[page],
                sacn::OPT_TERMINATED,
                universe,
                &[],
            );
            let targets = self.sacn_targets(universe);
            self.send_sacn(&pkt, "E1.31 Terminate", universe, Some(self.seq_sacn[page]), None, targets);
        }
    }

    fn configure(&mut self, new: NetConfig) {
        let iface_changed = new.interface != self.cfg.interface;
        let old_sacn: Vec<u16> = if self.cfg.protocol.sacn() { self.cfg.sacn_universes().to_vec() } else { Vec::new() };
        let new_sacn: Vec<u16> = if new.protocol.sacn() { new.sacn_universes().to_vec() } else { Vec::new() };
        // Say goodbye on universes we are leaving before the config flips.
        for (page, u) in old_sacn.iter().enumerate() {
            if !new_sacn.contains(u) {
                self.terminate(*u, page);
            }
        }
        self.cfg = new;
        if iface_changed {
            self.refresh_ifaces(true);
        } else {
            self.ensure_sacn_rx();
        }
        self.destinations = Default::default();
        // Announce the new universe set straight away.
        self.last_discovery = Instant::now() - Duration::from_secs(sacn::DISCOVERY_INTERVAL_S);
        self.status(format!(
            "Network: {} via {}",
            self.cfg.protocol.label(),
            self.cfg.interface.map_or("any interface".to_string(), |ip| ip.to_string())
        ));
    }

    fn apply_test(&self, page: usize, slots: &mut [u8]) {
        let Some(t) = &self.test else { return };
        if t.pattern.page != page {
            return;
        }
        match t.pattern.kind {
            TestKind::FullOn => slots.fill(255),
            TestKind::Single(ch) => {
                slots.fill(0);
                if (1..=512).contains(&ch) {
                    slots[ch as usize - 1] = 255;
                }
            }
            TestKind::Chase => {
                slots.fill(0);
                let step = (t.started.elapsed().as_millis() / 50) as usize % 512;
                slots[step] = 255;
                if step > 0 {
                    slots[step - 1] = 64;
                }
            }
        }
    }

    fn send_frame(&mut self) {
        let snapshot = *self.dmx.lock();
        let artnet_on = self.cfg.protocol.artnet();
        let sacn_on = self.cfg.protocol.sacn();
        let art_u = self.artnet_universes();
        let sacn_u = self.cfg.sacn_universes();
        for page in 0..DMX_UNIVERSES {
            let mut slots = [0u8; 512];
            slots.copy_from_slice(&snapshot.0[page * 512..(page + 1) * 512]);
            self.apply_test(page, &mut slots);
            if artnet_on {
                let pkt = artnet::build_dmx(self.seq_artnet, art_u[page], &slots);
                if self.send_artnet(&pkt, "ArtDmx", Some(page)) > 0 {
                    let c = &mut self.counters[0][page];
                    c.frames += 1;
                    c.total += 1;
                    c.last = Some(Instant::now());
                }
            }
            if sacn_on {
                self.seq_sacn[page] = self.seq_sacn[page].wrapping_add(1);
                let pkt = sacn::build_data(
                    &self.cfg.sacn.cid,
                    &self.cfg.sacn.source_name,
                    self.cfg.sacn.priority,
                    self.seq_sacn[page],
                    0,
                    sacn_u[page],
                    &slots,
                );
                let before = self.error_count;
                let targets = self.sacn_targets(sacn_u[page]);
                self.send_sacn(&pkt, "E1.31 Data", sacn_u[page], Some(self.seq_sacn[page]), Some(page), targets);
                if self.error_count == before && self.sacn_tx.is_some() {
                    let c = &mut self.counters[1][page];
                    c.frames += 1;
                    c.total += 1;
                    c.last = Some(Instant::now());
                }
            }
        }
        self.seq_artnet = self.seq_artnet.wrapping_add(1).max(1);
        // Advance by a whole interval rather than restarting the clock
        // here: this runs AFTER the send, so `= Instant::now()` makes the
        // real period 25 ms plus the send plus up to one poll granule, and
        // the error accumulates instead of being corrected. More universes
        // means a longer send, so the measured rate would sag below 40 Hz.
        // A clock that has fallen badly behind is reset rather than
        // sprinting to catch up.
        self.last_dmx += DMX_INTERVAL;
        if self.last_dmx.elapsed() > DMX_INTERVAL * 4 {
            self.last_dmx = Instant::now();
        }
    }

    fn publish_stats(&mut self) {
        let elapsed = self.window_start.elapsed().as_secs_f32().max(0.001);
        let art_u = self.artnet_universes();
        let sacn_u = self.cfg.sacn_universes();
        let mut universes = Vec::new();
        for (pi, proto) in [Proto::ArtNet, Proto::Sacn].into_iter().enumerate() {
            let on = match proto {
                Proto::ArtNet => self.cfg.protocol.artnet(),
                Proto::Sacn => self.cfg.protocol.sacn(),
            };
            if !on {
                continue;
            }
            for page in 0..DMX_UNIVERSES {
                let c = self.counters[pi][page];
                universes.push(UniverseStat {
                    proto,
                    page,
                    universe: if proto == Proto::ArtNet { art_u[page] } else { sacn_u[page] },
                    fps: c.frames as f32 / elapsed,
                    bytes_per_s: c.bytes as f32 / elapsed,
                    last_send: c.last,
                    total_frames: c.total,
                    destinations: self.destinations[pi][page].clone(),
                });
            }
        }
        for row in &mut self.counters {
            for c in row.iter_mut() {
                c.frames = 0;
                c.bytes = 0;
            }
        }
        self.window_start = Instant::now();

        let mut sockets = Vec::new();
        if self.artnet_rx.is_some() {
            sockets.push(format!("Art-Net listener 0.0.0.0:{}", artnet::ARTNET_PORT));
        }
        for e in &self.artnet_tx {
            sockets.push(format!("Art-Net egress {} ({})", e.iface.addr, e.iface.name));
        }
        if self.sacn_tx.is_some() {
            sockets.push(format!(
                "sACN sender via {}",
                self.cfg.interface.map_or("OS default route".to_string(), |ip| ip.to_string())
            ));
        }
        if self.sacn_rx.is_some() {
            let groups: Vec<String> = self
                .sacn_joined
                .iter()
                .map(|(u, ip)| format!("{}@{ip}", sacn::multicast_addr(*u)))
                .collect();
            sockets.push(format!("sACN listener 0.0.0.0:{} joined {}", sacn::SACN_PORT, groups.join(", ")));
        }
        self.diag(DiagEvent::Stats(NetStats {
            universes,
            sockets,
            last_error: self.last_error.clone(),
            error_count: self.error_count,
            artnet_listening: self.artnet_rx.is_some(),
            sacn_ready: self.sacn_tx.is_some(),
            sacn_listening: self.sacn_rx.is_some(),
            multicast_if: self.cfg.interface.filter(|ip| self.ifaces.iter().any(|i| i.addr == *ip)),
            uptime_s: self.started.elapsed().as_secs_f32(),
            test_active: self.test.is_some(),
        }));
    }

    fn on_artnet_packet(&mut self, n: usize, from: SocketAddrV4) {
        let buf = &self.rx_buf[..n];
        let Some(op) = artnet::op_code(buf) else { return };
        // Our own broadcasts loop back; only a node's reply from this host
        // (a software node) is worth showing.
        let from_self = self.ifaces.iter().any(|i| i.addr == *from.ip());
        if from_self && op != artnet::OP_POLL_REPLY {
            return;
        }
        let header = artnet::dmx_header(buf);
        let raw = buf.to_vec();
        self.monitor_packet(Direction::Received, Proto::ArtNet, artnet::op_name(op), from, header.map(|h| h.0), header.map(|h| h.1), &raw);
        if op == artnet::OP_POLL_REPLY {
            if let Some(reply) = artnet::parse_poll_reply(&raw) {
                self.status(format!("ArtPollReply from {from}: {} ({})", reply.short_name, reply.ip));
                let _ = self.evt_tx.send(NetEvent::Discovered(DiscoveredNode {
                    ip: reply.ip,
                    short_name: reply.short_name.clone(),
                    long_name: reply.long_name.clone(),
                }));
                let now = Instant::now();
                match self.nodes.iter_mut().find(|(r, _)| r.ip == reply.ip && r.bind_index == reply.bind_index) {
                    Some(slot) => *slot = (reply.clone(), now),
                    None => self.nodes.push((reply.clone(), now)),
                }
                self.diag(DiagEvent::Node { reply, from: *from.ip() });
            }
        }
    }

    fn on_sacn_packet(&mut self, n: usize, from: SocketAddrV4) {
        let buf = &self.rx_buf[..n];
        let Some(pkt) = sacn::parse(buf) else { return };
        let raw = buf.to_vec();
        match pkt {
            sacn::Packet::Data(info) => {
                if info.cid == self.cfg.sacn.cid {
                    return;
                }
                let kind = if info.terminated() { "E1.31 Terminate" } else { "E1.31 Data" };
                self.monitor_packet(Direction::Received, Proto::Sacn, kind, from, Some(info.universe), Some(info.sequence), &raw);
                self.diag(DiagEvent::SacnData { info, from: *from.ip() });
            }
            sacn::Packet::Discovery(info) => {
                if info.cid == self.cfg.sacn.cid {
                    return;
                }
                self.monitor_packet(Direction::Received, Proto::Sacn, "E1.31 Discovery", from, Some(sacn::DISCOVERY_UNIVERSE), None, &raw);
                self.diag(DiagEvent::SacnDiscovery { info, from: *from.ip() });
            }
        }
    }

    /// Pull everything waiting on one listener socket.
    fn drain_socket(&mut self, which: Proto) {
        loop {
            let sock = match which {
                Proto::ArtNet => self.artnet_rx.as_ref(),
                Proto::Sacn => self.sacn_rx.as_ref(),
            };
            let Some(sock) = sock else { return };
            match sock.recv_from(&mut self.rx_buf) {
                Ok((n, SocketAddr::V4(from))) => match which {
                    Proto::ArtNet => self.on_artnet_packet(n, from),
                    Proto::Sacn => self.on_sacn_packet(n, from),
                },
                Ok(_) => {}
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return;
                }
                // Windows reports an ICMP "port unreachable" from an earlier
                // send as a receive error on the same socket; it is not
                // fatal and says nothing about what is waiting to be read.
                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    self.fail("receive (a node refused our packet)", e);
                    return;
                }
                Err(e) => {
                    self.fail("receive", &e);
                    return;
                }
            }
        }
    }

    fn run(&mut self, cmd_rx: Receiver<NetCmd>) -> Result<()> {
        self.refresh_ifaces(true);
        if self.ifaces.is_empty() {
            self.status("WARN: no non-loopback IPv4 interfaces found".into());
        } else {
            for i in &self.ifaces {
                self.status(format!("iface {} {}/{} -> bcast {}", i.name, i.addr, i.prefix, i.bcast));
            }
        }
        self.poll();
        self.status(format!(
            "Listening on 0.0.0.0:{} — initial ArtPoll sent ({})",
            artnet::ARTNET_PORT,
            self.cfg.protocol.label()
        ));

        loop {
            // Process UI commands.
            loop {
                match cmd_rx.try_recv() {
                    Ok(NetCmd::Poll) => self.poll(),
                    Ok(NetCmd::SetTarget(t)) => {
                        self.legacy_target = t;
                        self.destinations = Default::default();
                        self.status(match t {
                            Some(ip) => format!("Target set: {ip}"),
                            None => "Target cleared (broadcast)".into(),
                        });
                    }
                    Ok(NetCmd::SetUniverse(u)) => {
                        self.universe = u;
                        self.destinations = Default::default();
                        self.status(format!("Universe set: {u}"));
                    }
                    Ok(NetCmd::Configure(cfg)) => self.configure(cfg),
                    Ok(NetCmd::TestPattern(Some(p))) => {
                        let now = Instant::now();
                        let secs = p.seconds.clamp(0.5, 600.0);
                        self.test = Some(ActiveTest {
                            pattern: p,
                            started: now,
                            ends: now + Duration::from_secs_f32(secs),
                        });
                        self.status(format!("Test pattern on universe {} for {secs:.0} s", p.page + 1));
                    }
                    Ok(NetCmd::TestPattern(None)) => {
                        if self.test.take().is_some() {
                            self.status("Test pattern stopped".into());
                            self.diag(DiagEvent::TestEnded);
                        }
                    }
                    Ok(NetCmd::Monitor(on)) => self.monitor = on,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }

            self.drain_socket(Proto::ArtNet);
            self.drain_socket(Proto::Sacn);

            if self.ifaces_at.elapsed() >= IFACE_REFRESH {
                self.refresh_ifaces(false);
            }
            let missing = self.artnet_rx.is_none()
                || (self.cfg.protocol.sacn() && (self.sacn_tx.is_none() || self.sacn_rx.is_none()));
            if missing && self.sockets_at.is_none_or(|t| t.elapsed() >= SOCKET_RETRY) {
                self.rebuild_sockets();
            }

            if self.last_poll.elapsed() >= self.cfg.poll_interval() {
                self.poll();
            }
            if self.cfg.protocol.sacn()
                && self.last_discovery.elapsed() >= Duration::from_secs(sacn::DISCOVERY_INTERVAL_S)
            {
                self.discovery();
            }
            if self.test.as_ref().is_some_and(|t| Instant::now() >= t.ends) {
                self.test = None;
                self.status("Test pattern finished — show output restored".into());
                self.diag(DiagEvent::TestEnded);
            }
            let now = Instant::now();
            self.nodes.retain(|(_, seen)| now.duration_since(*seen) < NODE_EXPIRY);

            if self.last_dmx.elapsed() >= DMX_INTERVAL {
                self.send_frame();
            }
            if self.window_start.elapsed() >= STATS_WINDOW {
                self.publish_stats();
            }

            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of universes is consecutive from the base, however many the
    /// console has.
    #[test]
    fn universe_mapping_follows_base_or_explicit() {
        let mut cfg = NetConfig::default();
        let run = |from: u16| -> Vec<u16> { (0..DMX_UNIVERSES as u16).map(|i| from + i).collect() };
        assert_eq!(cfg.artnet_universes(0x0359).to_vec(), run(0x0359));
        assert_eq!(cfg.sacn_universes().to_vec(), run(1));

        let explicit: Vec<u16> = (0..DMX_UNIVERSES as u16).map(|i| 5 + i * 4).collect();
        cfg.artnet.explicit = Some(explicit.clone());
        assert_eq!(cfg.artnet_universes(0).to_vec(), explicit);
        cfg.artnet.explicit = None;

        // A short list left over from a console with fewer universes is
        // filled out from the base rather than rejected — rejecting it would
        // throw away the whole config file, CID and all.
        cfg.sacn.explicit = Some(vec![40]);
        cfg.normalize();
        let mut want = vec![40];
        want.extend(2..=DMX_UNIVERSES as u16);
        assert_eq!(cfg.sacn_universes().to_vec(), want);
        cfg.sacn.explicit = None;
    }

    /// At the very top of either range the BASE is pulled down so the whole
    /// block still fits. Clamping each page instead would aim several
    /// different 512-slot pages at one universe and silently drop all but
    /// the last.
    #[test]
    fn a_base_at_the_ceiling_keeps_the_block_distinct() {
        let mut cfg = NetConfig::default();
        let art = cfg.artnet_universes(ARTNET_MAX_PORT_ADDRESS);
        assert_eq!(art[DMX_UNIVERSES - 1], ARTNET_MAX_PORT_ADDRESS);
        assert!(art.windows(2).all(|w| w[1] == w[0] + 1), "{art:?}");

        cfg.sacn.base_universe = sacn::MAX_UNIVERSE;
        let su = cfg.sacn_universes();
        assert_eq!(su[DMX_UNIVERSES - 1], sacn::MAX_UNIVERSE);
        assert!(su.windows(2).all(|w| w[1] == w[0] + 1), "{su:?}");
    }

    /// A config file the current build cannot parse must not cost the
    /// operator their sACN identity: `SacnConfig::default` mints a new CID,
    /// so silently falling back would make the console a different source
    /// to every receiver on every launch.
    #[test]
    fn an_unparsable_config_is_not_silently_accepted() {
        assert!(serde_json::from_str::<NetConfig>("{ this is not json }").is_err());
    }

    #[test]
    fn interface_subnet_helpers() {
        let i = IfaceInfo {
            name: "eth".into(),
            addr: Ipv4Addr::new(2, 0, 0, 1),
            mask: Ipv4Addr::new(255, 0, 0, 0),
            prefix: 8,
            bcast: IfaceInfo::directed_broadcast(Ipv4Addr::new(2, 0, 0, 1), Ipv4Addr::new(255, 0, 0, 0)),
        };
        assert_eq!(i.bcast, Ipv4Addr::new(2, 255, 255, 255));
        assert!(i.contains(Ipv4Addr::new(2, 100, 3, 4)));
        assert!(!i.contains(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(i.on_artnet_range());
        assert_eq!(
            IfaceInfo::directed_broadcast(Ipv4Addr::new(192, 168, 1, 20), Ipv4Addr::new(255, 255, 255, 0)),
            Ipv4Addr::new(192, 168, 1, 255)
        );
    }

    #[test]
    fn config_survives_a_json_round_trip_with_defaults() {
        let cfg = NetConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: NetConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
        // An old, sparse file still loads and gets every default.
        let old: NetConfig = serde_json::from_str(r#"{"protocol":"Sacn"}"#).unwrap();
        assert_eq!(old.protocol, Protocol::Sacn);
        assert_eq!(old.sacn.priority, 100);
        assert_eq!(old.sacn.source_name, "DMXpress");
        assert!(old.poll_interval_s > 0.0);
    }
}
