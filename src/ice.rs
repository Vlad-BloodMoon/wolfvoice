/*
 * WolfVoice - spatial WebRTC voice for OpenSimulator
 * By Shadow Wolf and Lone Wolf, Wolf Territories Grid and IntelligentWolf Ltd
 * Copyright 2026 IntelligentWolf Ltd
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! [HARDENING 2026-10-23] Which remote ICE candidates we will try to reach.
//!
//! Every candidate a viewer names is an address this server then sends STUN connectivity
//! checks to (RFC 8445 §7.2). Left unfiltered, any grid user could make the voice box send
//! packets to loopback, link-local or private addresses - its own LAN and the hosts
//! behind it - and as many of them as they liked. Those candidates are useless to us in any
//! case: we are a public server, and a viewer's private host address is not reachable
//! from here (that is what its server-reflexive and peer-reflexive addresses are for).
//!
//! So a candidate is kept only when its connection address is a public unicast IP, or lies
//! in one of our own networks named in WOLFVOICE_ICE_ALLOW. Hostname candidates (the mDNS
//! ".local" names browsers use to hide host addresses) are dropped too: this server cannot
//! resolve them. Candidates arrive two ways and both are filtered: trickled through
//! VoiceSignalingRequest, and inside the SDP offer.
//!
//! Grammar (RFC 8839 §5.1): "candidate:" foundation SP component-id SP transport SP
//! priority SP connection-address SP port SP "typ" SP cand-type *(SP extension).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The most candidates taken from one VoiceSignalingRequest, from one SDP offer, and over
/// a session's whole life. libwebrtc gathers a handful per interface (host, srflx, relay,
/// UDP and TCP, IPv4 and IPv6); a viewer with several network adapters may send a couple
/// of dozen. Beyond that it is not a viewer.
pub const MAX_CANDIDATES_PER_REQUEST: usize = 32;
pub const MAX_SDP_CANDIDATES: usize = 32;
pub const MAX_CANDIDATES_PER_SESSION: u32 = 128;

/// The environment variable naming our own networks, as comma-separated CIDRs
/// ("10.99.0.0/24,fd00:1::/64"). Empty: only public addresses.
pub const ALLOW_ENV: &str = "WOLFVOICE_ICE_ALLOW";

#[derive(Clone, Debug, Default)]
pub struct IceFilter {
    allow: Vec<(IpAddr, u8)>,
}

/// The connection address of a candidate attribute, with or without the "a=" prefix.
pub fn candidate_address(candidate: &str) -> Option<&str> {
    let c = candidate.trim();
    let c = c.strip_prefix("a=").unwrap_or(c);
    let rest = c.strip_prefix("candidate:")?;
    rest.split_ascii_whitespace().nth(4)
}

fn in_net(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let bits = bits.min(32) as u32;
            if bits == 0 {
                return true;
            }
            let mask = u32::MAX << (32 - bits);
            (u32::from(a) & mask) == (u32::from(n) & mask)
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let bits = bits.min(128) as u32;
            if bits == 0 {
                return true;
            }
            let mask = u128::MAX << (128 - bits);
            (u128::from(a) & mask) == (u128::from(n) & mask)
        }
        _ => false,
    }
}

/// A public unicast address: something a server on the internet may legitimately send to.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                // 2001:db8::/32 documentation (RFC 3849)
                || in_net(IpAddr::V6(v6), IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0)), 32))
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let ip = IpAddr::V4(v4);
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        // 0.0.0.0/8 "this network", 100.64.0.0/10 shared (carrier-grade NAT, RFC 6598),
        // 240.0.0.0/4 reserved (RFC 1112).
        || in_net(ip, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 8)
        || in_net(ip, IpAddr::V4(Ipv4Addr::new(100, 64, 0, 0)), 10)
        || in_net(ip, IpAddr::V4(Ipv4Addr::new(240, 0, 0, 0)), 4))
}

impl IceFilter {
    /// Parse WOLFVOICE_ICE_ALLOW ("10.99.0.0/24, fd00::/8"; a bare address means /32 or /128).
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut allow = Vec::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (addr, bits) = match item.split_once('/') {
                Some((a, b)) => (a, Some(b)),
                None => (item, None),
            };
            let ip: IpAddr = addr.parse().map_err(|_| format!("{ALLOW_ENV}: {item:?} is not an address"))?;
            let max = if ip.is_ipv4() { 32 } else { 128 };
            let bits = match bits {
                Some(b) => b.parse::<u8>().ok().filter(|b| *b <= max).ok_or_else(|| format!("{ALLOW_ENV}: bad prefix in {item:?}"))?,
                None => max,
            };
            allow.push((ip, bits));
        }
        Ok(IceFilter { allow })
    }

    /// True when this server may send to `ip`.
    pub fn allows_ip(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        is_public(ip) || self.allow.iter().any(|(net, bits)| in_net(ip, *net, *bits))
    }

    /// True when the candidate names an address this server may send to.
    pub fn allows_candidate(&self, candidate: &str) -> bool {
        candidate_address(candidate)
            .and_then(|a| a.parse::<IpAddr>().ok())
            .is_some_and(|ip| self.allows_ip(ip))
    }

    /// The SDP with every candidate we may not reach removed, and at most
    /// MAX_SDP_CANDIDATES kept. Line endings are preserved. Returns the SDP and how many
    /// candidates were dropped.
    pub fn filter_sdp(&self, sdp: &str) -> (String, usize) {
        let mut out = String::with_capacity(sdp.len());
        let mut kept = 0usize;
        let mut dropped = 0usize;
        for line in sdp.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            if content.starts_with("a=candidate:") {
                if kept >= MAX_SDP_CANDIDATES || !self.allows_candidate(content) {
                    dropped += 1;
                    continue;
                }
                kept += 1;
            }
            out.push_str(line);
        }
        (out, dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(addr: &str) -> String {
        format!("candidate:1467250027 1 udp 2122260223 {addr} 46243 typ host generation 0")
    }

    #[test]
    fn reads_the_connection_address() {
        assert_eq!(candidate_address(&cand("203.0.113.9")), Some("203.0.113.9"));
        assert_eq!(candidate_address(&format!("a={}", cand("2a01:4f8::1"))), Some("2a01:4f8::1"));
        assert_eq!(candidate_address("candidate:1 1 udp"), None);
        assert_eq!(candidate_address("not a candidate"), None);
    }

    #[test]
    fn only_public_addresses_pass_by_default() {
        let f = IceFilter::default();
        for bad in [
            "127.0.0.1", "10.1.2.3", "172.16.5.5", "192.168.1.20", "169.254.10.10", "0.0.0.0",
            "100.64.1.1", "224.0.0.251", "255.255.255.255", "::1", "fe80::1", "fd12:3456::1",
            "::ffff:192.168.1.1", "abcd-1234.local", "198.51.100.7",
        ] {
            assert!(!f.allows_candidate(&cand(bad)), "{bad} must be dropped");
        }
        for good in ["1.1.1.1", "81.2.69.160", "2a01:4f8:c17:1::1", "::ffff:81.2.69.160"] {
            assert!(f.allows_candidate(&cand(good)), "{good} must pass");
        }
    }

    #[test]
    fn our_own_networks_can_be_allowed() {
        let f = IceFilter::parse("10.99.0.0/24, fd00:1::/64,192.168.7.7").unwrap();
        assert!(f.allows_candidate(&cand("10.99.0.142")));
        assert!(!f.allows_candidate(&cand("10.99.1.1")));
        assert!(f.allows_candidate(&cand("fd00:1::5")));
        assert!(f.allows_candidate(&cand("192.168.7.7")));
        assert!(!f.allows_candidate(&cand("192.168.7.8")));
        assert!(IceFilter::parse("10.0.0.0/33").is_err());
        assert!(IceFilter::parse("nonsense").is_err());
        assert!(IceFilter::parse("").unwrap().allow.is_empty());
    }

    #[test]
    fn sdp_loses_unreachable_candidates_and_is_capped() {
        let f = IceFilter::default();
        let sdp = format!(
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na={}\r\na={}\r\na=rtpmap:111 opus/48000/2\r\n",
            cand("192.168.1.2"),
            cand("81.2.69.160")
        );
        let (out, dropped) = f.filter_sdp(&sdp);
        assert_eq!(dropped, 1);
        assert!(!out.contains("192.168.1.2") && out.contains("81.2.69.160"));
        assert!(out.ends_with("a=rtpmap:111 opus/48000/2\r\n"), "other lines and CRLF kept: {out:?}");

        let mut many = String::from("v=0\r\n");
        for i in 0..(MAX_SDP_CANDIDATES + 10) {
            many.push_str(&format!("a={}\r\n", cand(&format!("81.2.69.{}", i % 250 + 1))));
        }
        let (out, dropped) = f.filter_sdp(&many);
        assert_eq!(out.matches("a=candidate:").count(), MAX_SDP_CANDIDATES);
        assert_eq!(dropped, 10);
    }
}
