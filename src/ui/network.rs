//! The Network screen: protocol setup, live diagnostics, a troubleshooting
//! checklist, test patterns and a packet monitor — everything needed to get
//! Art-Net or sACN out of the console and into a node, and to see why it
//! isn't. Opened from Settings → Network….
//!
//! The screen owns a [`NetworkState`] on the app: the editable copy of the
//! [`NetConfig`], the interface list, the last stats snapshot, every node
//! and sACN source heard, and the packet ring. It never talks to a socket.
//! Edits are saved to `network.json` and pushed to the net thread as one
//! `NetCmd::Configure`; everything shown comes back as [`DiagEvent`]s on
//! the thread's diagnostics channel, drained at the top of every frame
//! whether or not the window is open so the tables are current the moment
//! it opens and the bounded channel never fills.
//!
//! A status strip stays above the tabs so the numbers that matter — is it
//! sending, how fast, to where, any errors — are visible whichever tab is
//! being worked on. The checklist tab is [`checklist::evaluate`] over the
//! same state, rendered one finding per row.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use eframe::egui;

use super::theme;
use crate::app::App;
use crate::artnet::{PollReply, PortAddress};
use crate::net::{
    self,
    checklist::{self, Finding, NodeFact, Severity, SourceFact},
    ArtNetMode, DiagEvent, Direction, IfaceInfo, NetCmd, NetConfig, NetStats, PacketSummary,
    Proto, Protocol, TestKind, TestPattern,
};
use crate::sacn;

/// How many packets the monitor keeps.
const MONITOR_LEN: usize = 50;
/// A node that has not answered for this long is dropped from the table.
const NODE_EXPIRY: Duration = Duration::from_secs(300);
/// An sACN source silent for this long is dropped.
const SOURCE_EXPIRY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Setup,
    Nodes,
    Checklist,
    Test,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Setup, Tab::Nodes, Tab::Checklist, Tab::Test];
    fn label(self) -> &'static str {
        match self {
            Tab::Setup => "Setup",
            Tab::Nodes => "Nodes & sources",
            Tab::Checklist => "Checklist",
            Tab::Test => "Test & monitor",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Tab::Setup => "Protocol, interface, universe numbers and addressing",
            Tab::Nodes => "Every Art-Net node and sACN source heard on the wire",
            Tab::Checklist => "Live checks that say what is wrong and what to do",
            Tab::Test => "Send a test pattern and watch packets go by",
        }
    }
}

/// One Art-Net node, from its most recent ArtPollReply.
pub(crate) struct NodeEntry {
    pub reply: PollReply,
    /// Address the reply actually came from (differs from `reply.ip` when a
    /// node is misconfigured — worth showing).
    pub from: Ipv4Addr,
    pub first_seen: Instant,
    pub last_seen: Instant,
    pub replies: u32,
}

/// Another sACN source on the network, from discovery and/or its data.
pub(crate) struct SourceEntry {
    pub cid: sacn::Cid,
    pub name: String,
    pub from: Ipv4Addr,
    /// What its discovery packets list.
    pub universes: Vec<u16>,
    /// (universe, priority, sequence, slots, preview, last seen) from data
    /// packets on the universes we listen to.
    pub data: Vec<(u16, u8, u8, u16, bool, Instant)>,
    pub last_seen: Instant,
}

pub(crate) struct NetworkState {
    pub open: bool,
    pub zoom: f32,
    pub tab: Tab,
    pub cfg: NetConfig,
    pub ifaces: Vec<IfaceInfo>,
    ifaces_at: Option<Instant>,
    pub stats: NetStats,
    stats_at: Option<Instant>,
    pub nodes: Vec<NodeEntry>,
    pub sources: Vec<SourceEntry>,
    pub packets: VecDeque<PacketSummary>,
    pub paused: bool,
    /// What we last told the thread about packet monitoring.
    monitoring: bool,
    pub last_poll: Option<Instant>,
    pub replies_since_poll: usize,
    pub test_kind: TestKind,
    pub test_page: usize,
    pub test_channel: u16,
    pub test_seconds: f32,
    pub test_until: Option<Instant>,
    pub artnet_target_entry: String,
    pub sacn_unicast_entry: String,
    pub started: Instant,
    copied_at: Option<Instant>,
}

impl NetworkState {
    pub fn new(cfg: NetConfig) -> Self {
        Self {
            open: false,
            zoom: 1.0,
            tab: Tab::Setup,
            artnet_target_entry: cfg.artnet.target.map_or(String::new(), |ip| ip.to_string()),
            cfg,
            ifaces: Vec::new(),
            ifaces_at: None,
            stats: NetStats::default(),
            stats_at: None,
            nodes: Vec::new(),
            sources: Vec::new(),
            packets: VecDeque::with_capacity(MONITOR_LEN + 1),
            paused: false,
            monitoring: false,
            last_poll: None,
            replies_since_poll: 0,
            test_kind: TestKind::FullOn,
            test_page: 0,
            test_channel: 1,
            test_seconds: 5.0,
            test_until: None,
            sacn_unicast_entry: String::new(),
            started: Instant::now(),
            copied_at: None,
        }
    }
}

fn age(t: Instant) -> String {
    let s = t.elapsed().as_secs_f32();
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 120.0 {
        format!("{s:.0} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

fn kb(bytes_per_s: f32) -> String {
    if bytes_per_s >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_s / 1024.0)
    } else {
        format!("{bytes_per_s:.0} B/s")
    }
}

/// Classic offset / hex / ASCII dump, 16 bytes a row.
fn hex_dump(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len() * 4);
    for (i, row) in raw.chunks(16).enumerate() {
        out.push_str(&format!("{:04x}  ", i * 16));
        for j in 0..16 {
            match row.get(j) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push(' ');
        for b in row {
            out.push(if (0x20..0x7F).contains(b) { *b as char } else { '.' });
        }
        out.push('\n');
    }
    out
}

fn severity_pill(ui: &mut egui::Ui, s: Severity) {
    // In a narrow grid column the label would wrap to "WAR / N".
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    match s {
        Severity::Ok => theme::pill(ui, "OK", theme::OK),
        Severity::Warn => theme::pill(ui, "WARN", theme::WARN),
        Severity::Fail => theme::pill(ui, "FAIL", theme::DANGER),
    };
}

impl App {
    /// Pull the net thread's diagnostics into the screen's state. Called
    /// every frame, window open or not.
    pub(crate) fn drain_network_events(&mut self) {
        let now = Instant::now();
        let st = &mut self.network;
        while let Ok(e) = self.net.diag_rx.try_recv() {
            match e {
                DiagEvent::Stats(s) => {
                    st.stats = s;
                    st.stats_at = Some(now);
                }
                DiagEvent::Node { reply, from } => {
                    st.replies_since_poll += 1;
                    match st
                        .nodes
                        .iter_mut()
                        .find(|n| n.reply.ip == reply.ip && n.reply.bind_index == reply.bind_index)
                    {
                        Some(n) => {
                            n.reply = reply;
                            n.from = from;
                            n.last_seen = now;
                            n.replies += 1;
                        }
                        None => st.nodes.push(NodeEntry {
                            reply,
                            from,
                            first_seen: now,
                            last_seen: now,
                            replies: 1,
                        }),
                    }
                }
                DiagEvent::SacnDiscovery { info, from } => {
                    let entry = match st.sources.iter_mut().find(|s| s.cid == info.cid) {
                        Some(s) => s,
                        None => {
                            st.sources.push(SourceEntry {
                                cid: info.cid,
                                name: String::new(),
                                from,
                                universes: Vec::new(),
                                data: Vec::new(),
                                last_seen: now,
                            });
                            st.sources.last_mut().expect("just pushed")
                        }
                    };
                    entry.name = info.source_name;
                    entry.from = from;
                    entry.last_seen = now;
                    // Page 0 restarts the list; later pages extend it.
                    if info.page == 0 {
                        entry.universes = info.universes;
                    } else {
                        entry.universes.extend(info.universes);
                    }
                    if info.page == info.last_page {
                        entry.universes.sort_unstable();
                        entry.universes.dedup();
                    }
                }
                DiagEvent::SacnData { info, from } => {
                    let (preview, terminated) = (info.preview(), info.terminated());
                    let entry = match st.sources.iter_mut().find(|s| s.cid == info.cid) {
                        Some(s) => s,
                        None => {
                            st.sources.push(SourceEntry {
                                cid: info.cid,
                                name: String::new(),
                                from,
                                universes: Vec::new(),
                                data: Vec::new(),
                                last_seen: now,
                            });
                            st.sources.last_mut().expect("just pushed")
                        }
                    };
                    entry.name = info.source_name;
                    entry.from = from;
                    entry.last_seen = now;
                    let row = (
                        info.universe,
                        info.priority,
                        info.sequence,
                        info.slot_count,
                        preview,
                        now,
                    );
                    if terminated {
                        entry.data.retain(|d| d.0 != info.universe);
                    } else {
                        match entry.data.iter_mut().find(|d| d.0 == info.universe) {
                            Some(d) => *d = row,
                            None => entry.data.push(row),
                        }
                    }
                }
                DiagEvent::Packet(p) => {
                    if !st.paused {
                        st.packets.push_front(p);
                        st.packets.truncate(MONITOR_LEN);
                    }
                }
                DiagEvent::PollSent => {
                    st.last_poll = Some(now);
                    st.replies_since_poll = 0;
                }
                DiagEvent::TestEnded => st.test_until = None,
            }
        }
        st.nodes.retain(|n| now.duration_since(n.last_seen) < NODE_EXPIRY);
        st.sources.retain(|s| now.duration_since(s.last_seen) < SOURCE_EXPIRY);
        for s in &mut st.sources {
            s.data.retain(|d| now.duration_since(d.5) < Duration::from_secs(5));
        }
    }

    /// Persist the edited configuration and hand it to the thread.
    fn network_apply(&mut self) {
        self.network.cfg.save();
        let _ = self.net.cmd_tx.send(NetCmd::Configure(self.network.cfg.clone()));
    }

    pub(crate) fn network_window(&mut self, ctx: &egui::Context) {
        self.drain_network_events();
        let want_monitor = self.network.open;
        if want_monitor != self.network.monitoring {
            self.network.monitoring = want_monitor;
            let _ = self.net.cmd_tx.send(NetCmd::Monitor(want_monitor));
        }
        if !self.network.open {
            return;
        }
        if self
            .network
            .ifaces_at
            .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
        {
            self.network.ifaces = net::interfaces();
            self.network.ifaces_at = Some(Instant::now());
        }
        // Counters and ages move on their own; keep them moving on screen.
        ctx.request_repaint_after(Duration::from_millis(250));

        let mut open = true;
        let mut popped = self.popped_out.contains("network");
        let mut zoom_level = self.network.zoom;
        super::floating_panel(
            ctx,
            "network",
            "Network",
            &mut open,
            &mut popped,
            Some(&mut zoom_level),
            [860.0, 660.0],
            [100.0, 60.0],
            |ui| {
                self.network_status_strip(ui);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for tab in Tab::ALL {
                        if ui
                            .selectable_label(self.network.tab == tab, tab.label())
                            .on_hover_text(tab.hint())
                            .clicked()
                        {
                            self.network.tab = tab;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("network_body")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.network.tab {
                        Tab::Setup => self.network_setup(ui),
                        Tab::Nodes => self.network_nodes(ui),
                        Tab::Checklist => self.network_checklist(ui),
                        Tab::Test => self.network_test(ui),
                    });
            },
        );
        self.network.open = open;
        self.network.zoom = zoom_level;
        if popped {
            self.popped_out.insert("network");
        } else {
            self.popped_out.remove("network");
        }
    }

    // ---- status strip ----

    fn network_status_strip(&mut self, ui: &mut egui::Ui) {
        let st = &self.network;
        let stats = &st.stats;
        ui.horizontal_wrapped(|ui| {
            for proto in [Proto::ArtNet, Proto::Sacn] {
                let on = match proto {
                    Proto::ArtNet => st.cfg.protocol.artnet(),
                    Proto::Sacn => st.cfg.protocol.sacn(),
                };
                if !on {
                    continue;
                }
                let sending = stats.universes.iter().any(|u| u.proto == proto && u.fps > 0.0);
                let colour = if sending { theme::OK } else { theme::WARN };
                theme::pill(ui, proto.label(), colour).on_hover_text(if sending {
                    "This protocol is enabled and frames are leaving"
                } else {
                    "Enabled, but no frame has left in the last second"
                });
            }
            let via = match st.cfg.interface {
                Some(ip) => match st.ifaces.iter().find(|i| i.addr == ip) {
                    Some(i) => format!("via {} ({})", ip, i.name),
                    None => format!("via {ip} (missing!)"),
                },
                None => format!("via any interface ({})", st.ifaces.len()),
            };
            theme::pill(ui, &via, theme::ACCENT_SOFT)
                .on_hover_text("The adapter packets leave by. Change it under Setup.");
            if stats.test_active || st.test_until.is_some() {
                theme::pill(ui, "TEST PATTERN OVERRIDING OUTPUT", theme::WARN)
                    .on_hover_text("The show is not on the wire until the pattern ends");
            }
            if let Some((msg, at)) = &stats.last_error {
                if at.elapsed() < Duration::from_secs(10) {
                    theme::pill(ui, "socket error", theme::DANGER).on_hover_text(msg);
                }
            }
        });
        if stats.universes.is_empty() {
            theme::hint(
                ui,
                if st.stats_at.is_none() {
                    "Waiting for the first statistics from the network thread…"
                } else {
                    "Nothing is being sent."
                },
            );
        } else {
            // One row per (protocol, universe): the numbers line up and a
            // stalled universe reads as a red row rather than a wrapped mess.
            egui::Grid::new("net_status_universes")
                .spacing([14.0, 2.0])
                .show(ui, |ui| {
                    for u in &stats.universes {
                        let colour = if u.fps >= 30.0 {
                            theme::TEXT
                        } else if u.fps > 0.0 {
                            theme::WARN
                        } else {
                            theme::DANGER
                        };
                        let mono = |s: String| egui::RichText::new(s).monospace().color(colour);
                        ui.label(mono(format!("U{}", u.page + 1))).on_hover_text(format!(
                            "Console universe {}: channels {}–{}",
                            u.page + 1,
                            u.page * 512 + 1,
                            (u.page + 1) * 512
                        ));
                        let target = if u.proto == Proto::ArtNet {
                            format!("Art-Net {} ({})", u.universe, PortAddress::split(u.universe))
                        } else {
                            format!("sACN {}", u.universe)
                        };
                        ui.label(mono(target)).on_hover_text("Protocol universe this console universe is sent as");
                        ui.label(mono(format!("{:5.1} fps", u.fps)))
                            .on_hover_text("Frames actually sent in the last second (target 40)");
                        ui.label(mono(format!("{:>10}", kb(u.bytes_per_s))))
                            .on_hover_text("Bytes sent per second, all destinations together");
                        let last = u.last_send.map_or("never".to_string(), |t| format!("{} ago", age(t)));
                        ui.label(mono(format!("last {last}")))
                            .on_hover_text(format!("{} frames sent since start", u.total_frames));
                        let dests: Vec<String> = u.destinations.iter().map(|d| d.to_string()).collect();
                        // Broadcast-everywhere can be four addresses; keep the row short.
                        let shown = match dests.len() {
                            0 => "nowhere yet".to_string(),
                            1 | 2 => dests.join(", "),
                            n => format!("{}, {} +{} more", dests[0], dests[1], n - 2),
                        };
                        ui.label(
                            egui::RichText::new(format!("→ {shown}"))
                                .monospace()
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        )
                        .on_hover_text(format!(
                            "Every address this universe has been sent to since the last change:\n{}",
                            dests.join("\n")
                        ));
                        ui.end_row();
                    }
                });
        }
        ui.horizontal(|ui| {
            if ui
                .button("Poll now")
                .on_hover_text("Broadcast an ArtPoll; every Art-Net node answers with an ArtPollReply")
                .clicked()
            {
                let _ = self.net.cmd_tx.send(NetCmd::Poll);
            }
            if ui
                .button("Copy diagnostics")
                .on_hover_text("Put a plain-text report of this screen on the clipboard to paste into a support message")
                .clicked()
            {
                let report = self.network_report();
                ui.output_mut(|o| o.copied_text = report);
                self.network.copied_at = Some(Instant::now());
            }
            if self
                .network
                .copied_at
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            {
                theme::pill(ui, "copied", theme::OK);
            }
            let errs = self.network.stats.error_count;
            if errs > 0 {
                ui.label(
                    egui::RichText::new(format!("{errs} socket error(s) since start"))
                        .color(theme::TEXT_DIM)
                        .size(11.0),
                );
            }
        });
    }

    // ---- setup tab ----

    fn network_setup(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        theme::section(ui, "Protocol");
        ui.horizontal(|ui| {
            ui.label("Output protocol");
            let cfg = &mut self.network.cfg;
            egui::ComboBox::from_id_salt("net_protocol")
                .selected_text(cfg.protocol.label())
                .show_ui(ui, |ui| {
                    for p in [Protocol::ArtNet, Protocol::Sacn, Protocol::Both] {
                        changed |= ui.selectable_value(&mut cfg.protocol, p, p.label()).changed();
                    }
                })
                .response
                .on_hover_text(
                    "Art-Net (UDP 6454, broadcast or unicast) is what most nodes speak by default; \
                     sACN / E1.31 (UDP 5568, multicast) is the ANSI standard with priorities and \
                     source discovery. Both sends every frame twice — only for mixed rigs.",
                );
        });
        theme::hint(ui, "Pick per show. A node normally listens to one protocol at a time.");

        theme::section(ui, "Interface");
        ui.horizontal(|ui| {
            ui.label("Send from");
            let cfg = &mut self.network.cfg;
            let current = match cfg.interface {
                None => "Any / all interfaces".to_string(),
                Some(ip) => match self.network.ifaces.iter().find(|i| i.addr == ip) {
                    Some(i) => format!("{} — {}/{}", i.name, i.addr, i.prefix),
                    None => format!("{ip} (not present)"),
                },
            };
            egui::ComboBox::from_id_salt("net_iface")
                .width(320.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut cfg.interface, None, "Any / all interfaces")
                        .changed();
                    for i in &self.network.ifaces {
                        changed |= ui
                            .selectable_value(
                                &mut cfg.interface,
                                Some(i.addr),
                                format!("{} — {}/{}", i.name, i.addr, i.prefix),
                            )
                            .changed();
                    }
                })
                .response
                .on_hover_text(
                    "The adapter plugged into the lighting network. Any broadcasts Art-Net out of \
                     every adapter but leaves sACN multicast to the OS default route.",
                );
        });
        if self.network.ifaces.is_empty() {
            theme::pill(ui, "no IPv4 interfaces found", theme::DANGER);
        } else {
            egui::Grid::new("net_ifaces")
                .striped(true)
                .spacing([14.0, 3.0])
                .show(ui, |ui| {
                    for h in ["Interface", "Address", "Mask", "Broadcast", ""] {
                        ui.label(egui::RichText::new(h).color(theme::TEXT_DIM).size(11.0));
                    }
                    ui.end_row();
                    for i in &self.network.ifaces {
                        ui.label(&i.name);
                        ui.monospace(format!("{}/{}", i.addr, i.prefix));
                        ui.monospace(i.mask.to_string());
                        ui.monospace(i.bcast.to_string());
                        if i.on_artnet_range() {
                            theme::pill(ui, "Art-Net range", theme::OK)
                                .on_hover_text("2.x.x.x / 10.x.x.x: where Art-Net nodes live by default");
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
        }

        theme::section(ui, "Universe mapping");
        theme::hint(
            ui,
            "The console has two universes of 512 channels. Each goes to one protocol universe.",
        );
        let art = self.network.cfg.artnet_universes(self.universe);
        let sacn_u = self.network.cfg.sacn_universes();
        egui::Grid::new("net_map").spacing([14.0, 4.0]).show(ui, |ui| {
            for h in ["Console", "Art-Net Port-Address", "sACN universe"] {
                ui.label(egui::RichText::new(h).color(theme::TEXT_DIM).size(11.0));
            }
            ui.end_row();
            for page in 0..net::DMX_UNIVERSES {
                ui.label(format!("Universe {} (ch {}–{})", page + 1, page * 512 + 1, (page + 1) * 512));
                if self.network.cfg.protocol.artnet() {
                    ui.monospace(format!("{} = {}", art[page], PortAddress::split(art[page])))
                        .on_hover_text("15-bit value = net.sub-net.universe");
                } else {
                    ui.label(egui::RichText::new("off").color(theme::TEXT_DIM));
                }
                if self.network.cfg.protocol.sacn() {
                    ui.monospace(format!("{} → {}", sacn_u[page], sacn::multicast_addr(sacn_u[page])))
                        .on_hover_text("Universe → multicast group");
                } else {
                    ui.label(egui::RichText::new("off").color(theme::TEXT_DIM));
                }
                ui.end_row();
            }
        });

        if self.network.cfg.protocol.artnet() {
            theme::section(ui, "Art-Net addressing");
            let mut pa = PortAddress::split(self.universe);
            let mut base_changed = false;
            ui.horizontal(|ui| {
                ui.label("Base Port-Address");
                ui.label("net");
                base_changed |= ui
                    .add(egui::DragValue::new(&mut pa.net).range(0..=127))
                    .on_hover_text("Bits 14–8: the node's Net switch (0–127)")
                    .changed();
                ui.label("sub-net");
                base_changed |= ui
                    .add(egui::DragValue::new(&mut pa.subnet).range(0..=15))
                    .on_hover_text("Bits 7–4: the node's Sub-Net switch (0–15)")
                    .changed();
                ui.label("universe");
                base_changed |= ui
                    .add(egui::DragValue::new(&mut pa.universe).range(0..=15))
                    .on_hover_text("Bits 3–0: the port's universe switch (0–15)")
                    .changed();
                let block = self.network.cfg.artnet_universes(pa.join());
                ui.monospace(format!(
                    "= {}",
                    block.iter().map(|u| format!("0x{u:04X}")).collect::<Vec<_>>().join(" ")
                ))
                .on_hover_text(format!(
                    "The {} Port-Addresses as sent in ArtDmx. Near the top of the range the                      base is pulled down so the whole block still fits.",
                    net::DMX_UNIVERSES
                ));
            });
            if base_changed {
                self.universe = pa.join();
                let _ = self.net.cmd_tx.send(NetCmd::SetUniverse(self.universe));
            }
            let mut explicit = self.network.cfg.artnet.explicit.is_some();
            if ui
                .checkbox(&mut explicit, "Set each universe's Port-Address explicitly")
                .on_hover_text("Instead of base and base + 1, choose any Port-Address per console universe")
                .changed()
            {
                self.network.cfg.artnet.explicit = explicit.then(|| art.to_vec());
                changed = true;
            }
            if let Some(e) = &mut self.network.cfg.artnet.explicit {
                ui.horizontal(|ui| {
                    for (page, u) in e.iter_mut().enumerate() {
                        ui.label(format!("U{}", page + 1));
                        changed |= ui
                            .add(egui::DragValue::new(u).range(0..=net::ARTNET_MAX_PORT_ADDRESS))
                            .on_hover_text("15-bit Port-Address for this console universe")
                            .changed();
                        ui.monospace(PortAddress::split(*u).to_string());
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Send ArtDmx");
                let cfg = &mut self.network.cfg;
                egui::ComboBox::from_id_salt("net_artmode")
                    .width(280.0)
                    .selected_text(cfg.artnet.mode.label())
                    .show_ui(ui, |ui| {
                        for m in ArtNetMode::ALL {
                            changed |= ui.selectable_value(&mut cfg.artnet.mode, m, m.label()).changed();
                        }
                    })
                    .response
                    .on_hover_text(
                        "Broadcast reaches every node on the subnet without knowing addresses; \
                         directed broadcast is the same but only on the interface's own subnet \
                         (e.g. 2.255.255.255); unicast sends to one node only and works across \
                         routers; Auto follows the node picked in the Art-Net panel.",
                    );
            });
            if self.network.cfg.artnet.mode == ArtNetMode::Auto {
                theme::hint(
                    ui,
                    match self.selected {
                        Some(ip) => format!("Auto: unicasting to {ip}, the node selected in the Art-Net panel."),
                        None => "Auto: no node selected in the Art-Net panel, so broadcasting on every interface.".to_string(),
                    },
                );
            }
            if self.network.cfg.artnet.mode == ArtNetMode::Unicast {
                ui.horizontal(|ui| {
                    ui.label("Target node");
                    let resp = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.network.artnet_target_entry)
                                .hint_text("e.g. 2.0.0.10")
                                .font(egui::TextStyle::Monospace)
                                .desired_width(140.0),
                        )
                        .on_hover_text("The node's IP address, from its display or the table under Nodes");
                    let parsed = self.network.artnet_target_entry.trim().parse::<Ipv4Addr>().ok();
                    if resp.changed() && parsed != self.network.cfg.artnet.target && parsed.is_some() {
                        self.network.cfg.artnet.target = parsed;
                        changed = true;
                    }
                    if parsed.is_none() && !self.network.artnet_target_entry.trim().is_empty() {
                        theme::pill(ui, "not an IPv4 address", theme::WARN);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Poll every");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.network.cfg.poll_interval_s)
                            .range(1.0..=60.0)
                            .speed(0.5)
                            .suffix(" s"),
                    )
                    .on_hover_text("How often an ArtPoll goes out; nodes that stop answering age out of the table")
                    .changed();
            });
        }

        if self.network.cfg.protocol.sacn() {
            theme::section(ui, "sACN (E1.31)");
            let cfg = &mut self.network.cfg;
            egui::Grid::new("net_sacn").spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("Base universe");
                changed |= ui
                    .add(egui::DragValue::new(&mut cfg.sacn.base_universe).range(
                        sacn::MIN_UNIVERSE
                            ..=sacn::MAX_UNIVERSE - (net::DMX_UNIVERSES as u16 - 1),
                    ))
                    .on_hover_text(format!(
                        "First of this console's {} sACN universes; they run on from here.                          Nodes usually map Art-Net universe 0 to sACN 1",
                        net::DMX_UNIVERSES
                    ))
                    .changed();
                ui.end_row();
                ui.label("Priority");
                changed |= ui
                    .add(egui::DragValue::new(&mut cfg.sacn.priority).range(1..=200))
                    .on_hover_text("1–200, default 100. When two sources send one universe the receiver keeps the higher priority")
                    .changed();
                ui.end_row();
                ui.label("Source name");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut cfg.sacn.source_name).desired_width(200.0))
                    .on_hover_text("Shown in receivers' source lists (up to 63 bytes)")
                    .changed();
                ui.end_row();
                ui.label("Multicast");
                changed |= ui
                    .checkbox(&mut cfg.sacn.multicast, "send to 239.255.x.y")
                    .on_hover_text("Standard sACN delivery: receivers subscribe to their universes' groups. Turn off only for pure unicast rigs")
                    .changed();
                ui.end_row();
                ui.label("CID");
                ui.monospace(sacn::cid_string(&cfg.sacn.cid))
                    .on_hover_text("This console's fixed component identifier, generated once and stored in network.json");
                ui.end_row();
            });
            let mut explicit = cfg.sacn.explicit.is_some();
            if ui
                .checkbox(&mut explicit, "Set each universe's sACN number explicitly")
                .on_hover_text("Instead of base and base + 1, choose any universe per console universe")
                .changed()
            {
                cfg.sacn.explicit = explicit.then(|| sacn_u.to_vec());
                changed = true;
            }
            if let Some(e) = &mut cfg.sacn.explicit {
                ui.horizontal(|ui| {
                    for (page, u) in e.iter_mut().enumerate() {
                        ui.label(format!("U{}", page + 1));
                        changed |= ui
                            .add(egui::DragValue::new(u).range(sacn::MIN_UNIVERSE..=sacn::MAX_UNIVERSE))
                            .on_hover_text("sACN universe for this console universe")
                            .changed();
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Unicast copies");
                ui.add(
                    egui::TextEdit::singleline(&mut self.network.sacn_unicast_entry)
                        .hint_text("receiver IP")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(140.0),
                )
                .on_hover_text("Also send every universe straight to this receiver, for gear that cannot join multicast");
                if ui.button("Add").clicked() {
                    if let Ok(ip) = self.network.sacn_unicast_entry.trim().parse::<Ipv4Addr>() {
                        if !self.network.cfg.sacn.unicast.contains(&ip) {
                            self.network.cfg.sacn.unicast.push(ip);
                            changed = true;
                        }
                        self.network.sacn_unicast_entry.clear();
                    }
                }
            });
            let mut remove: Option<usize> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, ip) in self.network.cfg.sacn.unicast.iter().enumerate() {
                    ui.monospace(ip.to_string());
                    if ui.small_button("×").on_hover_text("Stop sending to this receiver").clicked() {
                        remove = Some(i);
                    }
                }
            });
            if let Some(i) = remove {
                self.network.cfg.sacn.unicast.remove(i);
                changed = true;
            }
            theme::hint(
                ui,
                "Universe discovery is announced every 10 s on 239.255.250.214 so receivers list this console.",
            );
        }

        if changed {
            self.network_apply();
        }
    }

    // ---- nodes tab ----

    fn network_nodes(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Art-Net nodes");
        let ours = self.network.cfg.artnet_universes(self.universe);
        if self.network.nodes.is_empty() {
            theme::hint(
                ui,
                match self.network.last_poll {
                    Some(t) if t.elapsed() > Duration::from_secs(3) => {
                        "No ArtPollReply yet. Press Poll now; if nothing answers, see the Checklist."
                    }
                    _ => "Waiting for ArtPollReply…",
                },
            );
        }
        let mut make_target: Option<Ipv4Addr> = None;
        // Two lines per node rather than a twelve-column table: the name,
        // address and verdict on the first, the reply's details on the
        // second, so nothing needs a horizontal scroll to read.
        for n in &self.network.nodes {
            let r = &n.reply;
            let ports = (r.num_ports as usize).min(4);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&r.short_name).family(theme::medium()))
                    .on_hover_text(format!(
                        "{}\nFirst seen {} ago, {} replies. OEM 0x{:04X}, ESTA 0x{:04X}, style {}, status1 0x{:02X}{}",
                        r.long_name,
                        age(n.first_seen),
                        n.replies,
                        r.oem,
                        r.esta_manufacturer,
                        r.style,
                        r.status1,
                        r.status2.map_or(String::new(), |s| format!(", status2 0x{s:02X}"))
                    ));
                let ip = ui.monospace(r.ip.to_string());
                if n.from != r.ip {
                    ip.on_hover_text(format!(
                        "Reply came from {} — the node reports a different address",
                        n.from
                    ));
                } else if let Some(b) = r.bind_ip.filter(|b| *b != r.ip && !b.is_unspecified()) {
                    ip.on_hover_text(format!("Bound to root device {b}, index {}", r.bind_index.unwrap_or(0)));
                }
                let hits: Vec<String> = ours
                    .iter()
                    .filter(|u| r.outputs(**u))
                    .map(|u| u.to_string())
                    .collect();
                if hits.is_empty() {
                    let mine = ours.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
                    theme::pill(ui, &format!("not listening to universe {mine}"), theme::WARN)
                        .on_hover_text("None of this node's output ports is set to a universe we send");
                } else {
                    theme::pill(ui, &format!("✓ universe {}", hits.join(", ")), theme::OK)
                        .on_hover_text("This node outputs a universe we send");
                }
                if ui
                    .small_button("Unicast")
                    .on_hover_text("Send ArtDmx to this node only")
                    .clicked()
                {
                    make_target = Some(r.ip);
                }
            });
            ui.horizontal_wrapped(|ui| {
                let dim = |s: String| egui::RichText::new(s).monospace().size(11.5).color(theme::TEXT_DIM);
                let outs: Vec<String> = r
                    .output_addresses()
                    .iter()
                    .map(|u| format!("{} ({u})", PortAddress::split(*u)))
                    .collect();
                ui.label(dim(format!(
                    "{} port{} · out {}",
                    r.num_ports,
                    if r.num_ports == 1 { "" } else { "s" },
                    if outs.is_empty() { "—".into() } else { outs.join(" ") }
                )))
                .on_hover_text(format!(
                    "Output Port-Addresses as net.sub-net.universe (15-bit value). GoodOutput {:02X?}, PortTypes {:02X?}",
                    &r.good_output[..ports],
                    &r.port_types[..ports]
                ));
                let ins: Vec<String> = r.input_addresses().iter().map(|u| u.to_string()).collect();
                ui.label(dim(format!("· in {}", if ins.is_empty() { "—".into() } else { ins.join(" ") })))
                    .on_hover_text(format!("Input ports' Port-Addresses. GoodInput {:02X?}", &r.good_input[..ports]));
                ui.label(dim(format!("· fw {}.{}", r.firmware.0, r.firmware.1)))
                    .on_hover_text(format!("Firmware version; UBEA {}", r.ubea_version));
                ui.label(dim(format!("· mac {}", r.mac_string().unwrap_or_else(|| "—".into()))));
                ui.label(dim(format!("· seen {} ago", age(n.last_seen))))
                    .on_hover_text("Time since its last ArtPollReply");
                if !r.node_report.is_empty() {
                    ui.label(dim(format!("· {}", r.node_report)))
                        .on_hover_text("The node's own status report");
                }
            });
            ui.separator();
        }
        if let Some(ip) = make_target {
            self.network.cfg.artnet.mode = ArtNetMode::Unicast;
            self.network.cfg.artnet.target = Some(ip);
            self.network.artnet_target_entry = ip.to_string();
            self.network_apply();
        }

        theme::section(ui, "sACN sources");
        if !self.network.cfg.protocol.sacn() {
            theme::hint(ui, "Enable sACN under Setup to listen for other sources.");
        } else if self.network.sources.is_empty() {
            theme::hint(
                ui,
                "No other sACN source heard. Sources announce themselves every 10 s; data on our own universes is also watched.",
            );
        }
        let ours = self.network.cfg.sacn_universes();
        let our_prio = self.network.cfg.sacn.priority;
        egui::ScrollArea::horizontal().id_salt("net_sources_h").show(ui, |ui| {
            egui::Grid::new("net_sources")
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for h in ["Source", "From", "CID", "Announces", "On our universes", "Seen"] {
                        ui.label(egui::RichText::new(h).color(theme::TEXT_DIM).size(11.0));
                    }
                    ui.end_row();
                    for s in &self.network.sources {
                        ui.label(if s.name.is_empty() { "(unnamed)" } else { &s.name });
                        ui.monospace(s.from.to_string());
                        ui.monospace(sacn::cid_string(&s.cid));
                        let list: Vec<String> = s.universes.iter().map(|u| u.to_string()).collect();
                        ui.monospace(if list.is_empty() { "—".into() } else { list.join(", ") })
                            .on_hover_text("Universes listed in its discovery packets");
                        ui.horizontal(|ui| {
                            let mut any = false;
                            for (u, prio, seq, slots, preview, _) in &s.data {
                                if !ours.contains(u) {
                                    continue;
                                }
                                any = true;
                                let text = format!("U{u} @ {prio}");
                                let colour = if *preview {
                                    theme::ACCENT_SOFT
                                } else if *prio > our_prio {
                                    theme::DANGER
                                } else if *prio == our_prio {
                                    theme::WARN
                                } else {
                                    theme::OK
                                };
                                theme::pill(ui, &text, colour).on_hover_text(format!(
                                    "Priority {prio} vs ours {our_prio}: {}. Sequence {seq}, {slots} slots{}.",
                                    if *prio > our_prio {
                                        "receivers ignore us"
                                    } else if *prio == our_prio {
                                        "receivers HTP-merge both"
                                    } else {
                                        "receivers ignore it"
                                    },
                                    if *preview { ", preview data" } else { "" }
                                ));
                            }
                            if !any {
                                ui.label(egui::RichText::new("—").color(theme::TEXT_DIM));
                            }
                        });
                        ui.monospace(age(s.last_seen));
                        ui.end_row();
                    }
                });
        });
    }

    // ---- checklist tab ----

    fn network_findings(&self) -> Vec<Finding> {
        let st = &self.network;
        let nodes: Vec<NodeFact> = st
            .nodes
            .iter()
            .map(|n| NodeFact {
                ip: n.reply.ip,
                name: n.reply.short_name.clone(),
                outputs: n.reply.output_addresses(),
            })
            .collect();
        let sources: Vec<SourceFact> = st
            .sources
            .iter()
            .flat_map(|s| {
                s.data.iter().map(move |d| SourceFact {
                    name: if s.name.is_empty() { s.from.to_string() } else { s.name.clone() },
                    universe: d.0,
                    priority: d.1,
                })
            })
            .collect();
        let facts = checklist::Facts {
            cfg: &st.cfg,
            artnet_base: self.universe,
            legacy_target: self.selected,
            ifaces: &st.ifaces,
            nodes: &nodes,
            sources: &sources,
            stats: &st.stats,
            poll_age_s: st.last_poll.map(|t| t.elapsed().as_secs_f32()),
            replies_since_poll: st.replies_since_poll,
        };
        checklist::evaluate(&facts)
    }

    fn network_checklist(&mut self, ui: &mut egui::Ui) {
        let findings = self.network_findings();
        let fails = findings.iter().filter(|f| f.severity == Severity::Fail).count();
        let warns = findings.iter().filter(|f| f.severity == Severity::Warn).count();
        ui.horizontal(|ui| {
            if fails == 0 && warns == 0 {
                theme::pill(ui, "everything checks out", theme::OK);
            }
            if fails > 0 {
                theme::pill(ui, &format!("{fails} problem{}", if fails == 1 { "" } else { "s" }), theme::DANGER);
            }
            if warns > 0 {
                theme::pill(ui, &format!("{warns} warning{}", if warns == 1 { "" } else { "s" }), theme::WARN);
            }
            theme::hint(ui, "Re-evaluated live. Work from the top down.");
        });
        ui.add_space(4.0);
        egui::Grid::new("net_checklist")
            .striped(true)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                for f in &findings {
                    ui.vertical(|ui| {
                        ui.add_space(1.0);
                        severity_pill(ui, f.severity);
                    });
                    ui.vertical(|ui| {
                        ui.set_max_width(700.0);
                        ui.label(egui::RichText::new(&f.title).family(theme::medium()));
                        if !f.advice.is_empty() {
                            ui.label(egui::RichText::new(&f.advice).color(theme::TEXT_DIM).size(11.5));
                        }
                    });
                    ui.end_row();
                }
            });
    }

    // ---- test & monitor tab ----

    fn network_test(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Test pattern");
        theme::hint(
            ui,
            "Replaces the show on one universe while it runs — the wire, not the programmer — and the show returns when it ends.",
        );
        let active = self.network.test_until;
        ui.horizontal(|ui| {
            ui.label("Universe");
            egui::ComboBox::from_id_salt("net_test_page")
                .selected_text(format!("Universe {}", self.network.test_page + 1))
                .show_ui(ui, |ui| {
                    for p in 0..net::DMX_UNIVERSES {
                        ui.selectable_value(&mut self.network.test_page, p, format!("Universe {}", p + 1));
                    }
                })
                .response
                .on_hover_text("Which console universe the pattern replaces");
            ui.label("Pattern");
            let st = &mut self.network;
            let is_single = matches!(st.test_kind, TestKind::Single(_));
            if ui
                .selectable_label(st.test_kind == TestKind::FullOn, "Full on")
                .on_hover_text("Every channel at 255: does anything light at all?")
                .clicked()
            {
                st.test_kind = TestKind::FullOn;
            }
            if ui
                .selectable_label(st.test_kind == TestKind::Chase, "Chase")
                .on_hover_text("One channel at 255 walking through all 512, 20 a second: watch which fixture flashes when")
                .clicked()
            {
                st.test_kind = TestKind::Chase;
            }
            if ui
                .selectable_label(is_single, "Single channel")
                .on_hover_text("One channel at 255, every other channel at 0: is this address patched where you think?")
                .clicked()
            {
                st.test_kind = TestKind::Single(st.test_channel);
            }
            if is_single {
                if ui
                    .add(egui::DragValue::new(&mut st.test_channel).range(1..=512).prefix("ch "))
                    .on_hover_text("Channel within the universe (1–512)")
                    .changed()
                {
                    st.test_kind = TestKind::Single(st.test_channel);
                }
            }
            ui.label("for");
            ui.add(egui::DragValue::new(&mut st.test_seconds).range(1.0..=120.0).speed(0.5).suffix(" s"))
                .on_hover_text("How long to hold the pattern before the show comes back");
        });
        ui.horizontal(|ui| {
            match active {
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now()).as_secs_f32();
                    theme::pill(ui, &format!("OVERRIDING OUTPUT — {left:.1} s left"), theme::WARN);
                    if ui.button("Stop now").on_hover_text("End the pattern and restore the show at once").clicked() {
                        let _ = self.net.cmd_tx.send(NetCmd::TestPattern(None));
                        self.network.test_until = None;
                    }
                }
                None => {
                    if ui
                        .button("Send test pattern")
                        .on_hover_text("Start overriding the chosen universe on the wire")
                        .clicked()
                    {
                        let p = TestPattern {
                            page: self.network.test_page,
                            kind: self.network.test_kind,
                            seconds: self.network.test_seconds,
                        };
                        let _ = self.net.cmd_tx.send(NetCmd::TestPattern(Some(p)));
                        self.network.test_until =
                            Some(Instant::now() + Duration::from_secs_f32(self.network.test_seconds));
                    }
                }
            }
        });

        theme::section(ui, "Packet monitor");
        ui.horizontal(|ui| {
            let st = &mut self.network;
            let label = if st.paused { "Resume" } else { "Pause" };
            if ui
                .button(label)
                .on_hover_text("Freeze the list to read it; packets keep flowing underneath")
                .clicked()
            {
                st.paused = !st.paused;
            }
            if ui.button("Clear").clicked() {
                st.packets.clear();
            }
            if st.paused {
                theme::pill(ui, "paused", theme::WARN);
            }
            theme::hint(
                ui,
                format!("Last {} of {} packets, newest first. Sent and received, both protocols.", st.packets.len(), MONITOR_LEN),
            );
        });
        // Height-limited so the hex view below stays within reach.
        egui::ScrollArea::both().id_salt("net_monitor_h").max_height(260.0).show(ui, |ui| {
            egui::Grid::new("net_monitor")
                .striped(true)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for h in ["Time", "Dir", "Proto", "Kind", "Address", "Universe", "Seq", "Bytes"] {
                        ui.label(egui::RichText::new(h).color(theme::TEXT_DIM).size(11.0));
                    }
                    ui.end_row();
                    for p in &self.network.packets {
                        ui.monospace(format!("{:9.3}", p.t));
                        let (arrow, colour) = match p.dir {
                            Direction::Sent => ("→ out", theme::ACCENT_SOFT),
                            Direction::Received => ("← in", theme::OK),
                        };
                        ui.label(egui::RichText::new(arrow).monospace().color(colour));
                        ui.monospace(p.proto.label());
                        ui.monospace(p.kind);
                        ui.monospace(p.addr.to_string());
                        ui.monospace(p.universe.map_or("—".to_string(), |u| u.to_string()));
                        ui.monospace(p.sequence.map_or("—".to_string(), |s| s.to_string()));
                        ui.monospace(p.len.to_string());
                        ui.end_row();
                    }
                });
        });
        if let Some(p) = self.network.packets.front() {
            theme::section(ui, "Most recent packet");
            theme::hint(
                ui,
                format!(
                    "{} {} {} {} — {} bytes",
                    match p.dir { Direction::Sent => "sent to", Direction::Received => "received from" },
                    p.addr,
                    p.proto.label(),
                    p.kind,
                    p.len
                ),
            );
            egui::ScrollArea::vertical()
                .id_salt("net_hex")
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(hex_dump(&p.raw)).monospace().size(11.0))
                            .selectable(true),
                    );
                });
        }
    }

    // ---- diagnostics report ----

    /// Everything on the screen as plain text, for a support message.
    fn network_report(&self) -> String {
        use std::fmt::Write;
        let st = &self.network;
        let cfg = &st.cfg;
        let mut r = String::new();
        let _ = writeln!(r, "DMXpress v{} network diagnostics", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(r, "Uptime {:.0} s, thread uptime {:.0} s", st.started.elapsed().as_secs_f32(), st.stats.uptime_s);
        let _ = writeln!(r);
        let _ = writeln!(r, "[Config]");
        let _ = writeln!(r, "protocol: {}", cfg.protocol.label());
        let _ = writeln!(r, "interface: {}", cfg.interface.map_or("any".to_string(), |ip| ip.to_string()));
        let art = cfg.artnet_universes(self.universe);
        let _ = writeln!(
            r,
            "art-net: mode {:?}, target {}, base {} ({}), universes {}, explicit {}, poll every {:.0} s",
            cfg.artnet.mode,
            cfg.artnet.target.map_or("none".to_string(), |ip| ip.to_string()),
            self.universe,
            PortAddress::split(self.universe),
            art.iter()
                .map(|u| format!("{u} ({})", PortAddress::split(*u)))
                .collect::<Vec<_>>()
                .join(" / "),
            cfg.artnet.explicit.is_some(),
            cfg.poll_interval_s
        );
        let _ = writeln!(r, "art-net panel selection: {}", self.selected.map_or("none".to_string(), |ip| ip.to_string()));
        let su = cfg.sacn_universes();
        let _ = writeln!(
            r,
            "sacn: universes {}, priority {}, name \"{}\", multicast {}, unicast [{}], cid {}",
            su.iter().map(u16::to_string).collect::<Vec<_>>().join(" / "),
            cfg.sacn.priority,
            cfg.sacn.source_name,
            cfg.sacn.multicast,
            cfg.sacn.unicast.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join(", "),
            sacn::cid_string(&cfg.sacn.cid)
        );
        let _ = writeln!(r);
        let _ = writeln!(r, "[Interfaces]");
        for i in &st.ifaces {
            let _ = writeln!(
                r,
                "{}: {}/{} mask {} bcast {}{}",
                i.name,
                i.addr,
                i.prefix,
                i.mask,
                i.bcast,
                if i.on_artnet_range() { " (Art-Net range)" } else { "" }
            );
        }
        let _ = writeln!(r);
        let _ = writeln!(r, "[Sockets]");
        for s in &st.stats.sockets {
            let _ = writeln!(r, "{s}");
        }
        let _ = writeln!(
            r,
            "art-net listener {}, sacn sender {}, sacn listener {}, multicast interface {}",
            st.stats.artnet_listening,
            st.stats.sacn_ready,
            st.stats.sacn_listening,
            st.stats.multicast_if.map_or("OS default".to_string(), |ip| ip.to_string())
        );
        let _ = writeln!(r);
        let _ = writeln!(r, "[Output]");
        for u in &st.stats.universes {
            let _ = writeln!(
                r,
                "U{} {} universe {}: {:.1} fps, {}, {} frames total, last {}, to {}",
                u.page + 1,
                u.proto.label(),
                u.universe,
                u.fps,
                kb(u.bytes_per_s),
                u.total_frames,
                u.last_send.map_or("never".to_string(), |t| format!("{} ago", age(t))),
                u.destinations.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ")
            );
        }
        let _ = writeln!(r, "test pattern active: {}", st.stats.test_active);
        let _ = writeln!(r, "socket errors: {}", st.stats.error_count);
        if let Some((m, at)) = &st.stats.last_error {
            let _ = writeln!(r, "last error ({} ago): {m}", age(*at));
        }
        let _ = writeln!(r);
        let _ = writeln!(r, "[Art-Net nodes] ({})", st.nodes.len());
        for n in &st.nodes {
            let x = &n.reply;
            let _ = writeln!(
                r,
                "{} \"{}\" ({}) from {}: ports {}, out [{}], in [{}], fw {}.{}, mac {}, report \"{}\", seen {} ago, {} replies, listening to ours: {}",
                x.ip,
                x.short_name,
                x.long_name,
                n.from,
                x.num_ports,
                x.output_addresses().iter().map(|u| format!("{u} ({})", PortAddress::split(*u))).collect::<Vec<_>>().join(", "),
                x.input_addresses().iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "),
                x.firmware.0,
                x.firmware.1,
                x.mac_string().unwrap_or_else(|| "-".into()),
                x.node_report,
                age(n.last_seen),
                n.replies,
                art.iter().any(|u| x.outputs(*u))
            );
        }
        let _ = writeln!(r);
        let _ = writeln!(r, "[sACN sources] ({})", st.sources.len());
        for s in &st.sources {
            let _ = writeln!(
                r,
                "\"{}\" {} from {}: announces [{}], data [{}], seen {} ago",
                s.name,
                sacn::cid_string(&s.cid),
                s.from,
                s.universes.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "),
                s.data
                    .iter()
                    .map(|d| format!("U{} prio {} seq {} {} slots{}", d.0, d.1, d.2, d.3, if d.4 { " preview" } else { "" }))
                    .collect::<Vec<_>>()
                    .join("; "),
                age(s.last_seen)
            );
        }
        let _ = writeln!(r);
        let _ = writeln!(r, "[Checklist]");
        for f in self.network_findings() {
            let tag = match f.severity {
                Severity::Ok => "OK  ",
                Severity::Warn => "WARN",
                Severity::Fail => "FAIL",
            };
            let _ = writeln!(r, "[{tag}] {}", f.title);
            if !f.advice.is_empty() {
                let _ = writeln!(r, "       {}", f.advice);
            }
        }
        let _ = writeln!(r);
        let _ = writeln!(r, "[Last packets]");
        for p in st.packets.iter().take(20) {
            let _ = writeln!(
                r,
                "{:9.3} {} {} {} {} universe {} seq {} {} bytes",
                p.t,
                match p.dir { Direction::Sent => "out", Direction::Received => "in " },
                p.proto.label(),
                p.kind,
                p.addr,
                p.universe.map_or("-".to_string(), |u| u.to_string()),
                p.sequence.map_or("-".to_string(), |s| s.to_string()),
                p.len
            );
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Frame, UniverseStat, DMX_SLOTS};
    use crate::stage::headless::{render_frames, save};
    use std::net::SocketAddrV4;

    /// A rig for the screenshots: one node on the wrong universe, one on
    /// the right one, a rival sACN source, live-looking stats and a few
    /// packets — everything the tables and the checklist can show.
    fn populate(app: &mut App) {
        let now = Instant::now();
        let mut reply = crate::artnet::parse_poll_reply(&crate::artnet::sample_reply()).unwrap();
        app.network.nodes.push(NodeEntry { reply: reply.clone(), from: reply.ip, first_seen: now, last_seen: now, replies: 3 });
        reply.ip = Ipv4Addr::new(2, 0, 0, 11);
        reply.short_name = "Node2".into();
        reply.long_name = "Second test node with a long name".into();
        reply.sub_switch = 0;
        reply.sw_out = [0, 1, 0, 0];
        reply.node_report = "#0001 [0042] Power On Tests successful".into();
        app.network.nodes.push(NodeEntry { reply, from: Ipv4Addr::new(2, 0, 0, 11), first_seen: now, last_seen: now, replies: 1 });
        app.network.sources.push(SourceEntry {
            cid: [7; 16],
            name: "Other desk".into(),
            from: Ipv4Addr::new(2, 0, 0, 50),
            universes: vec![1, 2, 3],
            data: vec![(1, 150, 9, 512, false, now)],
            last_seen: now,
        });
        app.network.cfg.protocol = Protocol::Both;
        app.network.stats = NetStats {
            // One row per universe per protocol, generated: a hardcoded
            // pair keeps passing while never drawing the 3rd or 4th row the
            // grid has to grow.
            universes: (0..crate::net::DMX_UNIVERSES)
                .flat_map(|page| {
                    [
                        (Proto::ArtNet, page as u16, 40.0, 21200.0, 4000u64,
                         SocketAddrV4::new(Ipv4Addr::new(2, 255, 255, 255), 6454)),
                        (Proto::Sacn, page as u16 + 1, if page == 0 { 22.0 } else { 40.0 },
                         if page == 0 { 14000.0 } else { 25500.0 }, 2200 + page as u64 * 1800,
                         SocketAddrV4::new(Ipv4Addr::new(239, 255, 0, page as u8 + 1), 5568)),
                    ]
                    .map(|(proto, universe, fps, bytes_per_s, total_frames, dest)| UniverseStat {
                        proto,
                        page,
                        universe,
                        fps,
                        bytes_per_s,
                        last_send: Some(now),
                        total_frames,
                        destinations: vec![dest],
                    })
                })
                .collect(),
            sockets: vec!["Art-Net listener 0.0.0.0:6454".into()],
            last_error: Some(("send ArtDmx to 10.255.255.255:6454: A socket operation was attempted to an unreachable network. (os error 10051)".into(), now)),
            error_count: 12,
            artnet_listening: true,
            sacn_ready: true,
            sacn_listening: true,
            multicast_if: None,
            uptime_s: 100.0,
            test_active: false,
        };
        app.network.last_poll = Some(now - Duration::from_secs(5));
        let dmx = crate::artnet::build_dmx(3, 0, &[170; 512]);
        for i in 0..6 {
            app.network.packets.push_front(PacketSummary {
                t: 12.0 + i as f64 * 0.025,
                dir: if i % 3 == 2 { Direction::Received } else { Direction::Sent },
                proto: if i % 2 == 0 { Proto::ArtNet } else { Proto::Sacn },
                kind: if i % 3 == 2 { "ArtPollReply" } else if i % 2 == 0 { "ArtDmx" } else { "E1.31 Data" },
                addr: SocketAddrV4::new(Ipv4Addr::new(2, 255, 255, 255), 6454),
                universe: Some(i as u16 % 2),
                sequence: Some(i as u8 + 3),
                len: dmx.len(),
                raw: dmx.clone(),
            });
        }
        app.network.test_until = Some(now + Duration::from_secs(4));
    }

    /// The Network window on every tab, rendered through egui's renderer
    /// with the theme installed, written to `target/network_headless*.png`.
    #[test]
    fn network_window_renders_headless() {
        let mut app = App::new();
        *app.net.dmx.lock() = Frame([170u8; DMX_SLOTS]);
        populate(&mut app);
        app.network.open = true;
        let size = [1500, 950];
        let scenes = [
            ("network_headless", Tab::Setup),
            ("network_headless_nodes", Tab::Nodes),
            ("network_headless_checklist", Tab::Checklist),
            ("network_headless_test", Tab::Test),
        ];
        for (name, tab) in scenes {
            app.network.tab = tab;
            let Some(pixels) = render_frames(5, size, |ctx, frame| {
                if frame == 0 {
                    theme::install(ctx);
                } else {
                    app.draw_ui(ctx);
                }
            }) else {
                eprintln!("no GPU adapter — skipping");
                return;
            };
            save(&pixels, size, name);
            let lit = pixels
                .chunks(4)
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120)
                .count();
            assert!(lit > 1000, "{name} came out black");
        }
        // The report mentions the things the tables show.
        let report = app.network_report();
        assert!(report.contains("Node1"));
        assert!(report.contains("Other desk"));
        assert!(report.contains("[Checklist]"));
        assert!(report.contains("isn't listening"));
    }

    #[test]
    fn hex_dump_has_offset_hex_and_ascii_columns() {
        let d = hex_dump(b"Art-Net\0\x00\x50\x00\x0e\x01\x00\x00\x00\x02\x00AB");
        let lines: Vec<&str> = d.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("0000  41 72 74 2d 4e 65 74 00  00 50 00 0e 01 00 00 00  Art-Net.."));
        assert!(lines[1].starts_with("0010  02 00 41 42"));
        assert!(lines[1].ends_with(".AB"));
    }
}
