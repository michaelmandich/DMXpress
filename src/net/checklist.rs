//! The guided troubleshooting checklist on the Network screen.
//!
//! Getting a node to light up is mostly a matter of five things lining up:
//! the PC has an address on the node's network, packets leave by that
//! adapter, the firewall lets replies back in, the universe numbers match,
//! and nobody else is shouting over us. Each of those is something the app
//! can observe, so instead of a generic "check your network" this turns the
//! live state — interfaces, the config, what nodes answered, what sACN
//! sources are audible, the send rates — into plain findings that say what
//! is wrong and what to do about it.
//!
//! [`evaluate`] is a pure function of a [`Facts`] snapshot, so the UI feeds
//! it what it already holds and the tests feed it made-up rigs.

use std::net::Ipv4Addr;

use super::{ArtNetMode, IfaceInfo, NetConfig, NetStats, Proto};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

/// One line of the checklist: what was found and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub advice: String,
}

impl Finding {
    fn ok(title: impl Into<String>) -> Self {
        Self { severity: Severity::Ok, title: title.into(), advice: String::new() }
    }
    fn warn(title: impl Into<String>, advice: impl Into<String>) -> Self {
        Self { severity: Severity::Warn, title: title.into(), advice: advice.into() }
    }
    fn fail(title: impl Into<String>, advice: impl Into<String>) -> Self {
        Self { severity: Severity::Fail, title: title.into(), advice: advice.into() }
    }
}

/// What the checklist knows about one Art-Net node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFact {
    pub ip: Ipv4Addr,
    pub name: String,
    /// Port-Addresses the node outputs.
    pub outputs: Vec<u16>,
}

/// One sACN source heard sending data on a universe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFact {
    pub name: String,
    pub universe: u16,
    pub priority: u8,
}

/// Everything the checklist looks at.
pub struct Facts<'a> {
    pub cfg: &'a NetConfig,
    /// Base Port-Address from the toolbar.
    pub artnet_base: u16,
    /// Node picked in the Art-Net panel (used by [`ArtNetMode::Auto`]).
    pub legacy_target: Option<Ipv4Addr>,
    pub ifaces: &'a [IfaceInfo],
    pub nodes: &'a [NodeFact],
    pub sources: &'a [SourceFact],
    pub stats: &'a NetStats,
    /// Seconds since the last ArtPoll went out.
    pub poll_age_s: Option<f32>,
    pub replies_since_poll: usize,
}

const FIREWALL_ADVICE: &str = "Nodes answer an ArtPoll within a second. Either nothing on this \
    network speaks Art-Net, the poll left by the wrong adapter, or Windows Defender Firewall is \
    dropping the reply: open Windows Security → Firewall & network protection → Allow an app \
    through firewall, tick DMXpress for Private (and Public, if the adapter is marked public), \
    or in an admin prompt run: netsh advfirewall firewall add rule name=\"Art-Net in\" dir=in \
    action=allow protocol=UDP localport=6454";

/// Evaluate the checklist. Findings come out in the order an operator
/// should work through them: interface, then addressing, then replies,
/// then universes, then the rest.
pub fn evaluate(f: &Facts) -> Vec<Finding> {
    let mut out = Vec::new();
    let cfg = f.cfg;
    let artnet_on = cfg.protocol.artnet();
    let sacn_on = cfg.protocol.sacn();

    // 1. An interface with an address.
    let send_ifaces: Vec<&IfaceInfo> = match cfg.interface {
        Some(ip) => f.ifaces.iter().filter(|i| i.addr == ip).collect(),
        None => f.ifaces.iter().collect(),
    };
    if f.ifaces.is_empty() {
        out.push(Finding::fail(
            "No network interface has an IPv4 address",
            "Plug the lighting network in (or enable the adapter) and give it an address — for \
             Art-Net gear usually a static one such as 2.0.0.1 with mask 255.0.0.0. Nothing can \
             leave this PC until then.",
        ));
    } else if send_ifaces.is_empty() {
        out.push(Finding::fail(
            format!(
                "Selected interface {} is not present",
                cfg.interface.map_or(String::new(), |ip| ip.to_string())
            ),
            format!(
                "Its address changed or the adapter is down. Pick one of: {}, or Any.",
                describe_ifaces(f.ifaces)
            ),
        ));
    } else if cfg.interface.is_some() {
        let i = send_ifaces[0];
        out.push(Finding::ok(format!("Sending from {} — {}/{}", i.name, i.addr, i.prefix)));
    } else {
        out.push(Finding::ok(format!(
            "Sending from every interface: {}",
            describe_ifaces(f.ifaces)
        )));
    }

    // 2. Art-Net addressing.
    if artnet_on {
        let broadcasting = match cfg.artnet.mode {
            ArtNetMode::Auto => f.legacy_target.is_none(),
            ArtNetMode::Broadcast | ArtNetMode::DirectedBroadcast => true,
            ArtNetMode::Unicast | ArtNetMode::AllNodes => false,
        };
        if broadcasting && !send_ifaces.is_empty() {
            if send_ifaces.iter().any(|i| i.on_artnet_range()) {
                let b: Vec<String> = send_ifaces
                    .iter()
                    .filter(|i| i.on_artnet_range())
                    .map(|i| i.bcast.to_string())
                    .collect();
                out.push(Finding::ok(format!("Art-Net broadcast reaches {}", b.join(", "))));
            } else {
                out.push(Finding::warn(
                    "Art-Net broadcast, but no interface is on a 2.x.x.x or 10.x.x.x network",
                    format!(
                        "Art-Net nodes ship on 2.0.0.0/8 (some on 10.0.0.0/8) and only hear \
                         broadcasts on their own subnet. This PC is on {}. Either give the \
                         lighting adapter a static address such as 2.0.0.1 / 255.0.0.0, set the \
                         node to your subnet, or find the node's IP and use Unicast.",
                        describe_ifaces(&send_ifaces.iter().map(|i| (*i).clone()).collect::<Vec<_>>())
                    ),
                ));
            }
        }
        let unicast = match cfg.artnet.mode {
            ArtNetMode::Auto => f.legacy_target,
            ArtNetMode::Unicast => cfg.artnet.target,
            _ => None,
        };
        if cfg.artnet.mode == ArtNetMode::Unicast && unicast.is_none() {
            out.push(Finding::fail(
                "Unicast selected but no target address is set",
                "Type the node's IP under Art-Net addressing, or pick a node from the table.",
            ));
        }
        if let Some(ip) = unicast {
            match f.ifaces.iter().find(|i| i.contains(ip)) {
                Some(i) => out.push(Finding::ok(format!(
                    "Unicast target {ip} is on {}'s subnet ({}/{})",
                    i.name, i.addr, i.prefix
                ))),
                None => out.push(Finding::warn(
                    format!("Unicast target {ip} is not on any local subnet"),
                    format!(
                        "Packets to it go via the default gateway, and a node is normally on the \
                         same subnet as the console. Check the node's IP and mask against this \
                         PC's ({}); if they really are on different networks, a router must sit \
                         between them.",
                        describe_ifaces(f.ifaces)
                    ),
                )),
            }
        }
        if cfg.artnet.mode == ArtNetMode::AllNodes && f.nodes.is_empty() {
            out.push(Finding::warn(
                "Sending to every discovered node, but none has been discovered",
                "Nothing is leaving until a node answers an ArtPoll. See the reply check below.",
            ));
        }

        // 3. Replies.
        if !f.nodes.is_empty() {
            out.push(Finding::ok(format!(
                "{} node{} answered ArtPoll",
                f.nodes.len(),
                if f.nodes.len() == 1 { "" } else { "s" }
            )));
        } else {
            match f.poll_age_s {
                Some(age) if age >= 3.0 && f.replies_since_poll == 0 => out.push(Finding::fail(
                    format!("No ArtPollReply within 3 s of the last poll ({age:.0} s ago)"),
                    FIREWALL_ADVICE,
                )),
                Some(age) => out.push(Finding::ok(format!(
                    "ArtPoll sent {age:.1} s ago — waiting for replies"
                ))),
                None => out.push(Finding::warn(
                    "No ArtPoll has been sent yet",
                    "Press Poll now.",
                )),
            }
        }

        // 4. Universe overlap per node.
        let ours = cfg.artnet_universes(f.artnet_base);
        for n in f.nodes {
            let hit: Vec<u16> = ours.iter().copied().filter(|u| n.outputs.contains(u)).collect();
            if hit.is_empty() {
                let theirs: Vec<String> = n
                    .outputs
                    .iter()
                    .map(|u| format!("{u} ({})", crate::artnet::PortAddress::split(*u)))
                    .collect();
                // Every universe we send, not the first two: the
                // membership test above already checks them all, so naming
                // a subset would point the operator at the wrong one.
                let mine = ours
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(Finding::warn(
                    format!("{} ({}) isn't listening to universe {mine}", n.name, n.ip),
                    if theirs.is_empty() {
                        "It reports no output ports at all; check the node's port configuration.".to_string()
                    } else {
                        format!(
                            "It outputs Port-Address {} (net.sub-net.universe). Set the node's \
                             port to {} — or set the base universe here to {} so the console \
                             matches the node.",
                            theirs.join(", "),
                            mine,
                            n.outputs[0]
                        )
                    },
                ));
            } else {
                let list: Vec<String> = hit.iter().map(|u| u.to_string()).collect();
                out.push(Finding::ok(format!(
                    "{} ({}) listens to universe {}",
                    n.name,
                    n.ip,
                    list.join(" and ")
                )));
            }
        }
    }

    // 5. sACN.
    if sacn_on {
        if f.stats.uptime_s > 1.0 && !f.stats.sacn_ready {
            out.push(Finding::fail(
                "sACN send socket could not be opened",
                f.stats
                    .last_error
                    .as_ref()
                    .map_or("See the socket error below.".to_string(), |(m, _)| m.clone()),
            ));
        } else if cfg.interface.is_none() && f.ifaces.len() > 1 {
            out.push(Finding::warn(
                "sACN multicast leaves by the OS default route",
                format!(
                    "With the interface set to Any, Windows sends multicast on its default-route \
                     adapter — usually the one with internet, not the lighting one. Pick the \
                     lighting interface explicitly ({}).",
                    describe_ifaces(f.ifaces)
                ),
            ));
        } else if !send_ifaces.is_empty() {
            let u = cfg.sacn_universes();
            let pairs = u
                .iter()
                .map(|n| format!("{n} → {}", crate::sacn::multicast_addr(*n)))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(Finding::ok(format!("sACN universes multicast to {pairs}")));
        }
        if f.stats.uptime_s > 1.0 && !f.stats.sacn_listening {
            out.push(Finding::warn(
                "Not listening for other sACN sources",
                "Port 5568 could not be bound, so priority conflicts cannot be seen. Another sACN \
                 program may hold the port exclusively.",
            ));
        }
        let ours = cfg.sacn_universes();
        for s in f.sources {
            if !ours.contains(&s.universe) {
                continue;
            }
            if s.priority > cfg.sacn.priority {
                out.push(Finding::warn(
                    format!(
                        "\"{}\" also sends universe {} at priority {} — above ours ({})",
                        s.name, s.universe, s.priority, cfg.sacn.priority
                    ),
                    "Receivers keep the highest-priority source and ignore ours. Raise our \
                     priority above theirs (Setup → sACN), or stop the other console.",
                ));
            } else if s.priority == cfg.sacn.priority {
                out.push(Finding::warn(
                    format!(
                        "\"{}\" also sends universe {} at the same priority ({})",
                        s.name, s.universe, s.priority
                    ),
                    "Receivers merge equal-priority sources highest-takes-precedence, so the \
                     brighter console wins each channel. Raise ours if this console should own \
                     the universe.",
                ));
            }
        }
    }

    // 6. Both protocols to the same node.
    if artnet_on && sacn_on {
        let art_u = cfg.artnet_universes(f.artnet_base);
        let sacn_u = cfg.sacn_universes();
        for n in f.nodes {
            if cfg.sacn.unicast.contains(&n.ip) {
                out.push(Finding::warn(
                    format!("{} ({}) gets both Art-Net and sACN from this console", n.name, n.ip),
                    "A node that listens to both protocols will flicker between the two streams. \
                     Pick one protocol per node: remove it from the sACN unicast list or switch \
                     the node's protocol.",
                ));
            } else if cfg.sacn.multicast
                && n.outputs.iter().any(|u| {
                    art_u.contains(u) && sacn_u.iter().any(|s| *s == *u || *s == u + 1)
                })
            {
                out.push(Finding::warn(
                    format!("{} ({}) may receive the same universe over both protocols", n.name, n.ip),
                    "Most nodes map Art-Net universe N to sACN universe N+1. If this node also \
                     listens to sACN multicast it sees both streams and flickers — choose a \
                     single protocol for the show, or give sACN different universe numbers.",
                ));
            }
        }
    }

    // 7. Frame rate.
    if f.stats.uptime_s >= 2.0 {
        let active: Vec<_> = f.stats.universes.iter().collect();
        if active.is_empty() {
            out.push(Finding::warn("No protocol is sending", "Choose Art-Net, sACN or both."));
        } else if active.iter().all(|u| u.fps <= 0.0) {
            out.push(Finding::fail(
                "No frames are leaving",
                "The sockets are open but every send fails or has no destination. Check the \
                 socket error below and the addressing mode above.",
            ));
        } else {
            let mut slow = false;
            for u in &active {
                if u.fps > 0.0 && u.fps < 30.0 {
                    slow = true;
                    out.push(Finding::warn(
                        format!(
                            "{} universe {} is going out at {:.0} Hz (target 40)",
                            u.proto.label(),
                            u.universe,
                            u.fps
                        ),
                        "The machine is busy or the socket is stalling. Close other heavy programs, \
                         and look for send errors below — each failed send costs time.",
                    ));
                }
            }
            if !slow {
                let art = active.iter().find(|u| u.proto == Proto::ArtNet).map(|u| u.fps);
                let sacn = active.iter().find(|u| u.proto == Proto::Sacn).map(|u| u.fps);
                let rate = art.or(sacn).unwrap_or(0.0);
                out.push(Finding::ok(format!("Sending at {rate:.0} frames a second per universe")));
            }
        }
    }

    // 8. Socket errors.
    if let Some((msg, at)) = &f.stats.last_error {
        if at.elapsed().as_secs() < 10 {
            out.push(Finding::fail(format!("Socket error: {msg}"), error_advice(msg)));
        }
    }

    out
}

fn describe_ifaces(ifaces: &[IfaceInfo]) -> String {
    if ifaces.is_empty() {
        return "no interfaces".into();
    }
    ifaces
        .iter()
        .map(|i| format!("{} {}/{}", i.name, i.addr, i.prefix))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Turn the OS's wording into the thing to actually do.
fn error_advice(msg: &str) -> String {
    let m = msg.to_ascii_lowercase();
    if m.contains("10051") || m.contains("unreachable") {
        "No route to that address: the interface has no address on that network. Check the \
         interface's IP and mask, or pick another interface."
            .into()
    } else if m.contains("10048") || m.contains("in use") {
        "UDP port 6454 (or 5568) is held by another program — another Art-Net/sACN tool such as \
         QLC+, sACNView or a visualiser. Close it, or start DMXpress first."
            .into()
    } else if m.contains("10013") || m.contains("denied") {
        "The OS refused the socket: a firewall or security product is blocking it. Allow \
         DMXpress through Windows Defender Firewall."
            .into()
    } else if m.contains("10049") || m.contains("not available") {
        "That address is not one of this PC's — the selected interface changed its IP. Pick \
         it again under Setup."
            .into()
    } else {
        "See the OS message; if it keeps happening, copy the diagnostics and ask for help.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Protocol;
    use std::time::Instant;

    fn iface(name: &str, addr: [u8; 4], mask: [u8; 4]) -> IfaceInfo {
        let addr = Ipv4Addr::from(addr);
        let mask = Ipv4Addr::from(mask);
        let prefix = u32::from(mask).count_ones() as u8;
        IfaceInfo { name: name.into(), addr, mask, prefix, bcast: IfaceInfo::directed_broadcast(addr, mask) }
    }

    fn stats(fps: f32) -> NetStats {
        NetStats {
            universes: vec![super::super::UniverseStat {
                proto: Proto::ArtNet,
                page: 0,
                universe: 0,
                fps,
                bytes_per_s: fps * 530.0,
                last_send: Some(Instant::now()),
                total_frames: 100,
                destinations: Vec::new(),
            }],
            uptime_s: 10.0,
            artnet_listening: true,
            sacn_ready: true,
            sacn_listening: true,
            ..Default::default()
        }
    }

    fn base<'a>(cfg: &'a NetConfig, ifaces: &'a [IfaceInfo], nodes: &'a [NodeFact], st: &'a NetStats) -> Facts<'a> {
        Facts {
            cfg,
            artnet_base: 0,
            legacy_target: None,
            ifaces,
            nodes,
            sources: &[],
            stats: st,
            poll_age_s: Some(1.0),
            replies_since_poll: 0,
        }
    }

    fn titles(v: &[Finding]) -> Vec<String> {
        v.iter().map(|f| f.title.clone()).collect()
    }

    #[test]
    fn broadcast_without_an_artnet_subnet_is_flagged() {
        let cfg = NetConfig::default();
        let home = [iface("Wi-Fi", [192, 168, 1, 20], [255, 255, 255, 0])];
        let st = stats(40.0);
        let v = evaluate(&base(&cfg, &home, &[], &st));
        let w = v.iter().find(|f| f.title.contains("2.x.x.x")).expect("warns about subnet");
        assert_eq!(w.severity, Severity::Warn);
        assert!(w.advice.contains("2.0.0.1"));
        // Give it a 2.x adapter and the warning turns into a tick.
        let rig = [iface("Wi-Fi", [192, 168, 1, 20], [255, 255, 255, 0]), iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let v = evaluate(&base(&cfg, &rig, &[], &st));
        assert!(!titles(&v).iter().any(|t| t.contains("2.x.x.x")));
        assert!(v.iter().any(|f| f.severity == Severity::Ok && f.title.contains("2.255.255.255")));
    }

    #[test]
    fn missing_interfaces_fail_first() {
        let cfg = NetConfig::default();
        let st = stats(40.0);
        let v = evaluate(&base(&cfg, &[], &[], &st));
        assert_eq!(v[0].severity, Severity::Fail);
        assert!(v[0].title.contains("No network interface"));
        let mut sel = NetConfig::default();
        sel.interface = Some(Ipv4Addr::new(2, 0, 0, 9));
        let home = [iface("Wi-Fi", [192, 168, 1, 20], [255, 255, 255, 0])];
        let v = evaluate(&base(&sel, &home, &[], &st));
        assert!(v[0].title.contains("2.0.0.9 is not present"));
    }

    #[test]
    fn unicast_off_subnet_needs_a_gateway() {
        let mut cfg = NetConfig::default();
        cfg.artnet.mode = ArtNetMode::Unicast;
        cfg.artnet.target = Some(Ipv4Addr::new(10, 7, 7, 7));
        let home = [iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let st = stats(40.0);
        let v = evaluate(&base(&cfg, &home, &[], &st));
        let w = v.iter().find(|f| f.title.contains("not on any local subnet")).unwrap();
        assert!(w.advice.contains("gateway"));
        cfg.artnet.target = None;
        let v = evaluate(&base(&cfg, &home, &[], &st));
        assert!(v.iter().any(|f| f.severity == Severity::Fail && f.title.contains("no target")));
    }

    #[test]
    fn silence_after_a_poll_points_at_the_firewall() {
        let cfg = NetConfig::default();
        let home = [iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let st = stats(40.0);
        let mut f = base(&cfg, &home, &[], &st);
        f.poll_age_s = Some(4.0);
        let v = evaluate(&f);
        let fail = v.iter().find(|x| x.title.contains("No ArtPollReply")).unwrap();
        assert_eq!(fail.severity, Severity::Fail);
        assert!(fail.advice.contains("Windows Defender Firewall"));
        assert!(fail.advice.contains("6454"));
        // Not yet 3 s: still waiting, no failure.
        f.poll_age_s = Some(1.0);
        assert!(!evaluate(&f).iter().any(|x| x.severity == Severity::Fail));
    }

    #[test]
    fn node_on_the_wrong_universe_is_named() {
        let cfg = NetConfig::default();
        let home = [iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let st = stats(40.0);
        let nodes = [
            NodeFact { ip: Ipv4Addr::new(2, 0, 0, 10), name: "Node1".into(), outputs: vec![4, 5] },
            NodeFact { ip: Ipv4Addr::new(2, 0, 0, 11), name: "Node2".into(), outputs: vec![1] },
        ];
        let v = evaluate(&base(&cfg, &home, &nodes, &st));
        let w = v.iter().find(|f| f.title.starts_with("Node1")).unwrap();
        assert_eq!(w.severity, Severity::Warn);
        assert!(w.title.contains("isn't listening to universe 0"), "{}", w.title);
        assert!(w.advice.contains("Port-Address 4 (0.0.4)"));
        let ok = v.iter().find(|f| f.title.starts_with("Node2")).unwrap();
        assert_eq!(ok.severity, Severity::Ok);
        assert!(ok.title.contains("listens to universe 1"));
    }

    #[test]
    fn sacn_priority_conflicts_and_default_route() {
        let mut cfg = NetConfig::default();
        cfg.protocol = Protocol::Sacn;
        let two = [iface("Wi-Fi", [192, 168, 1, 20], [255, 255, 255, 0]), iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let st = stats(40.0);
        let sources = [
            SourceFact { name: "Desk B".into(), universe: 1, priority: 150 },
            SourceFact { name: "Desk C".into(), universe: 2, priority: 100 },
            SourceFact { name: "Desk D".into(), universe: 9, priority: 200 },
        ];
        let mut f = base(&cfg, &two, &[], &st);
        f.sources = &sources;
        let v = evaluate(&f);
        assert!(v.iter().any(|x| x.title.contains("default route")));
        assert!(v.iter().any(|x| x.title.contains("Desk B") && x.title.contains("above ours")));
        assert!(v.iter().any(|x| x.title.contains("Desk C") && x.title.contains("same priority")));
        assert!(!v.iter().any(|x| x.title.contains("Desk D")));
        // No Art-Net checks when Art-Net is off.
        assert!(!v.iter().any(|x| x.title.contains("ArtPoll")));
    }

    #[test]
    fn both_protocols_to_one_node_and_slow_frames() {
        let mut cfg = NetConfig::default();
        cfg.protocol = Protocol::Both;
        cfg.sacn.unicast = vec![Ipv4Addr::new(2, 0, 0, 10)];
        let home = [iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let st = stats(22.0);
        let nodes = [NodeFact { ip: Ipv4Addr::new(2, 0, 0, 10), name: "Node1".into(), outputs: vec![0] }];
        let v = evaluate(&base(&cfg, &home, &nodes, &st));
        assert!(v.iter().any(|x| x.title.contains("both Art-Net and sACN")));
        let slow = v.iter().find(|x| x.title.contains("22 Hz")).unwrap();
        assert_eq!(slow.severity, Severity::Warn);
        assert!(slow.advice.contains("busy"));
    }

    #[test]
    fn socket_errors_get_specific_advice() {
        let cfg = NetConfig::default();
        let home = [iface("Ethernet", [2, 0, 0, 1], [255, 0, 0, 0])];
        let mut st = stats(40.0);
        st.last_error = Some(("send ArtDmx to 2.255.255.255:6454: A socket operation was attempted to an unreachable network. (os error 10051)".into(), Instant::now()));
        let v = evaluate(&base(&cfg, &home, &[], &st));
        let e = v.iter().find(|x| x.title.starts_with("Socket error")).unwrap();
        assert_eq!(e.severity, Severity::Fail);
        assert!(e.advice.contains("No route"));
        assert!(error_advice("os error 10048").contains("another program"));
    }
}
