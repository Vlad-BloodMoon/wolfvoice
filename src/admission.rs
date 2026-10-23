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

//! [HARDENING 2026-10-23] Admitting a new session: the service-wide ceiling, the per-agent
//! ceiling and the per-room ceiling, checked and claimed in ONE step.
//!
//! A provision spends a second or two in ICE gathering (session.rs GATHER_TIMEOUT) between
//! being checked and being registered. Before this, the checks read the registry and the
//! claim happened only at registration, so parallel provisions for one agent (or one room)
//! could all pass the same check together. Now a reservation is taken under one lock and
//! held until the session is in the registry (or the provision fails), and every check
//! counts the reservations still in flight as well as the registered sessions.

use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};

use crate::room::{Registry, RoomKey};

/// Why a session was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The whole service is at its ceiling.
    Capacity,
    /// This agent holds as many sessions as it may.
    AgentLimit,
    /// The room has as many people in it as it may (this agent is not one of them).
    RoomFull,
}

impl Refusal {
    /// The text the region relay receives (and logs). "voice room is full" is matched by the
    /// region module to answer the viewer 409, which it shows as ERROR_CHANNEL_FULL
    /// (llvoicewebrtc.cpp:3422-3424).
    pub fn message(self) -> &'static str {
        match self {
            Refusal::Capacity => "voice service is at capacity",
            Refusal::AgentLimit => "too many voice connections for this avatar",
            Refusal::RoomFull => "voice room is full",
        }
    }
}

#[derive(Default)]
struct Pending {
    total: usize,
    by_agent: HashMap<String, usize>,
    by_room: HashMap<RoomKey, Vec<String>>,
}

#[derive(Default)]
pub struct Admission {
    pending: Mutex<Pending>,
}

/// A claimed place. Dropping it gives the place back, so drop it only AFTER the session is
/// in the registry (then the registry counts it) or when the provision has failed.
pub struct Reservation<'a> {
    adm: &'a Admission,
    agent: String,
    room: RoomKey,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut p = self.adm.pending.lock();
        p.total = p.total.saturating_sub(1);
        if let Some(n) = p.by_agent.get_mut(&self.agent) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                p.by_agent.remove(&self.agent);
            }
        }
        if let Some(v) = p.by_room.get_mut(&self.room) {
            if let Some(i) = v.iter().position(|a| *a == self.agent) {
                v.swap_remove(i);
            }
            if v.is_empty() {
                p.by_room.remove(&self.room);
            }
        }
    }
}

impl Admission {
    /// Provisions in flight (counted against the ceilings, not yet in the registry).
    pub fn pending(&self) -> usize {
        self.pending.lock().total
    }

    /// Check every ceiling and claim a place, atomically with respect to every other
    /// reservation. `room_cap` is the most DIFFERENT agents the room may hold (None = no
    /// limit, as for spatial rooms, whose size is the region's crowd); an agent already in
    /// the room may always add a session there (a reconnect overlapping its old one).
    pub fn reserve<'a>(
        &'a self,
        reg: &Registry,
        agent: &str,
        room: &RoomKey,
        room_cap: Option<usize>,
        total_cap: usize,
        agent_cap: usize,
    ) -> Result<Reservation<'a>, Refusal> {
        let mut p = self.pending.lock();
        if reg.session_count() + p.total >= total_cap {
            return Err(Refusal::Capacity);
        }
        if reg.sessions_of(agent) + p.by_agent.get(agent).copied().unwrap_or(0) >= agent_cap {
            return Err(Refusal::AgentLimit);
        }
        if let Some(cap) = room_cap {
            let mut agents: HashSet<String> = reg.members(room).into_iter().map(|m| m.agent_id.clone()).collect();
            if let Some(v) = p.by_room.get(room) {
                agents.extend(v.iter().cloned());
            }
            if !agents.contains(agent) && agents.len() >= cap {
                return Err(Refusal::RoomFull);
            }
        }
        p.total += 1;
        *p.by_agent.entry(agent.to_string()).or_default() += 1;
        p.by_room.entry(room.clone()).or_default().push(agent.to_string());
        Ok(Reservation { adm: self, agent: agent.to_string(), room: room.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::Session;
    use std::sync::Arc;

    fn room() -> RoomKey {
        RoomKey::MultiAgent { channel: "group-x".into() }
    }

    #[test]
    fn reservations_count_against_every_ceiling() {
        let reg = Registry::default();
        let adm = Admission::default();
        // Total: two places; the third is refused while both are only reserved (the race).
        let a = adm.reserve(&reg, "a", &room(), None, 2, 10).unwrap();
        let b = adm.reserve(&reg, "b", &room(), None, 2, 10).unwrap();
        assert_eq!(adm.reserve(&reg, "c", &room(), None, 2, 10).err(), Some(Refusal::Capacity));
        drop(a);
        assert!(adm.reserve(&reg, "c", &room(), None, 2, 10).is_ok(), "a place given back is free again");
        drop(b);
        assert_eq!(adm.pending(), 0);

        // Per agent: parallel provisions for one agent cannot pass the same check together.
        let r1 = adm.reserve(&reg, "a", &room(), None, 100, 2).unwrap();
        let r2 = adm.reserve(&reg, "a", &room(), None, 100, 2).unwrap();
        assert_eq!(adm.reserve(&reg, "a", &room(), None, 100, 2).err(), Some(Refusal::AgentLimit));
        // A registered session counts the same as a reservation.
        reg.insert(Arc::new(Session::new("s1".into(), "a".into(), room(), false)));
        drop(r1);
        assert_eq!(adm.reserve(&reg, "a", &room(), None, 100, 2).err(), Some(Refusal::AgentLimit));
        drop(r2);
    }

    #[test]
    fn a_room_holds_at_most_its_cap_of_different_agents() {
        let reg = Registry::default();
        let adm = Admission::default();
        let p2p = RoomKey::MultiAgent { channel: "p2p:a:b".into() };
        reg.insert(Arc::new(Session::new("s1".into(), "a".into(), p2p.clone(), false)));
        let b = adm.reserve(&reg, "b", &p2p, Some(2), 100, 10).unwrap();
        assert_eq!(adm.reserve(&reg, "x", &p2p, Some(2), 100, 10).err(), Some(Refusal::RoomFull), "a third person is refused");
        assert!(adm.reserve(&reg, "a", &p2p, Some(2), 100, 10).is_ok(), "someone already in it may reconnect");
        drop(b);
        assert!(adm.reserve(&reg, "x", &p2p, Some(2), 100, 10).is_ok(), "a place a pending joiner gave up is free");
    }
}
