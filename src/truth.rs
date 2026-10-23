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

//! [HARDENING 2026-10-23] Where each avatar REALLY is, as its region says.
//!
//! The viewer reports its own avatar and ear positions on the data channel
//! (llvoicewebrtc.cpp:1216-1255 sendPositionUpdate, "sp"/"lp"), and nothing stopped a
//! modified viewer from claiming to stand anywhere in its room and so listen in from
//! there. The viewer's own comment says the server must tether to "the actual position of
//! the avatar ... sent from the simulator to the voice server" (llvoicewebrtc.cpp:1185-1188,
//! 1209-1215). The region module now does that: about once a second each region sends the
//! positions of its ROOT agents that are in voice (method `wolf.positions`,
//! WebRtcVoiceRegionModule WolfPositionFeed). Here those positions are kept per agent and
//! the mixer clamps the viewer's claim to within a small tolerance of them.
//!
//! Positions are GLOBAL metres, the frame the viewer's "sp" uses (getPositionGlobal,
//! llvoicewebrtc.cpp:1093). The region sends RegionInfo.WorldLocX/Y + AbsolutePosition.
//!
//! Backward compatible by construction: an agent with no fresh region position (a region
//! still running the old module, or a feed that stopped) is mixed exactly as before.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A region position older than this is not used. The region sends every second
/// (WolfPositionFeed FEED_INTERVAL), so three missed sends in a row.
pub const FRESH: Duration = Duration::from_secs(3);

/// The viewer bumps its reported avatar position up to head height before sending it:
/// llvoicewebrtc.cpp:1096 `avatar_pos += LLVector3d(0.f, 0.f, 1.f);  // bump it up to head height`.
/// The region sends AbsolutePosition, so the same metre is added here.
pub const HEAD_BUMP: f64 = 1.0;

/// How far the viewer's claim may be from the region's position before it is pulled in,
/// for a standing avatar. Covers the viewer interpolating ahead of the simulator, a seated
/// avatar's offset and rounding (the viewer sends whole centimetres).
pub const BASE_TOLERANCE: f64 = 5.0;

/// Seconds of movement allowed on top of the sample's age: the time a region position
/// takes to reach us after it was read, and the viewer being ahead of it.
pub const LAG_ALLOW: f64 = 1.0;

/// The most slack ever given, however fast the avatar is going (an aircraft at 150 m/s for
/// one second). Past this a claim is pulled in whatever the speed.
pub const MAX_TOLERANCE: f64 = 150.0;

/// How long a sample is kept at all (only for its speed estimate), and how many agents.
const KEEP: Duration = Duration::from_secs(60);
const MAX_AGENTS: usize = 20_000;

/// Two samples closer together than this give no usable speed estimate.
const MIN_SPEED_INTERVAL: Duration = Duration::from_millis(200);

struct Sample {
    scene: String,
    pos: [f64; 3],
    /// Metres per second: the larger of what the region reported and what successive
    /// samples show (a keyframed train reports no physics velocity, but it still moves).
    speed: f64,
    at: Instant,
}

/// A usable region position: the point (head height, global metres) and how far from it
/// the viewer's claim may be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fresh {
    pub pos: [f64; 3],
    pub tolerance: f64,
}

#[derive(Default)]
pub struct Truths {
    map: RwLock<HashMap<String, Sample>>,
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `p` moved to the nearest point no further than `radius` from `centre`.
pub fn clamp_near(p: [f64; 3], centre: [f64; 3], radius: f64) -> [f64; 3] {
    let off = [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]];
    let d = norm(off);
    if d <= radius || d == 0.0 {
        return p;
    }
    let k = radius / d;
    [centre[0] + off[0] * k, centre[1] + off[1] * k, centre[2] + off[2] * k]
}

impl Truths {
    /// One position from a region: `agent` is a root agent of `scene`, at `pos` (global
    /// metres, feet/centre as the simulator holds it) moving at `vel` (m/s, may be zero).
    pub fn report(&self, agent: &str, scene: &str, pos: [f64; 3], vel: [f64; 3], now: Instant) {
        let pos = [pos[0], pos[1], pos[2] + HEAD_BUMP];
        let reported = norm(vel);
        let mut map = self.map.write();
        let speed = match map.get(agent) {
            Some(prev) => {
                let dt = now.saturating_duration_since(prev.at);
                if dt >= MIN_SPEED_INTERVAL {
                    reported.max(dist(prev.pos, pos) / dt.as_secs_f64())
                } else {
                    reported.max(prev.speed)
                }
            }
            None => reported,
        };
        if map.len() >= MAX_AGENTS && !map.contains_key(agent) {
            map.retain(|_, s| now.saturating_duration_since(s.at) < KEEP);
            if map.len() >= MAX_AGENTS {
                return;
            }
        }
        map.insert(agent.to_string(), Sample { scene: scene.to_string(), pos, speed, at: now });
    }

    /// The region says `agent` is no longer a root agent there (crossed, teleported, left):
    /// forget its position, unless another region has reported it since.
    pub fn gone(&self, agent: &str, scene: &str) {
        let mut map = self.map.write();
        if map.get(agent).is_some_and(|s| s.scene == scene) {
            map.remove(agent);
        }
    }

    /// The region position to hold `agent` to now, or None when there is no fresh one.
    pub fn fresh(&self, agent: &str, now: Instant) -> Option<Fresh> {
        let map = self.map.read();
        let s = map.get(agent)?;
        let age = now.saturating_duration_since(s.at);
        if age > FRESH {
            return None;
        }
        let tolerance = (BASE_TOLERANCE + s.speed * (age.as_secs_f64() + LAG_ALLOW)).min(MAX_TOLERANCE);
        Some(Fresh { pos: s.pos, tolerance })
    }

    /// Drop samples nobody will use again.
    pub fn prune(&self, now: Instant) {
        self.map.write().retain(|_, s| now.saturating_duration_since(s.at) < KEEP);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_avatar_is_held_within_the_base_tolerance() {
        let t = Truths::default();
        let now = Instant::now();
        t.report("a", "r1", [1000.0, 2000.0, 20.0], [0.0; 3], now);
        let f = t.fresh("a", now).unwrap();
        assert_eq!(f.pos, [1000.0, 2000.0, 21.0], "head height is added as the viewer does");
        assert!((f.tolerance - (BASE_TOLERANCE + 0.0)).abs() < 1e-9);
        // A claim 200 m away comes back to the edge of the tolerance.
        let c = clamp_near([1200.0, 2000.0, 21.0], f.pos, f.tolerance);
        assert!((dist(c, f.pos) - BASE_TOLERANCE).abs() < 1e-9);
        // An honest claim inside it is untouched.
        assert_eq!(clamp_near([1002.0, 2000.0, 21.0], f.pos, f.tolerance), [1002.0, 2000.0, 21.0]);
    }

    #[test]
    fn a_moving_avatar_gets_slack_for_its_speed() {
        let t = Truths::default();
        let now = Instant::now();
        t.report("a", "r1", [0.0; 3], [20.0, 0.0, 0.0], now);
        let f = t.fresh("a", now).unwrap();
        assert!((f.tolerance - (BASE_TOLERANCE + 20.0 * LAG_ALLOW)).abs() < 1e-9);
        // Measured from successive samples when the region reports no velocity (keyframed train).
        t.report("b", "r1", [0.0; 3], [0.0; 3], now);
        t.report("b", "r1", [30.0, 0.0, 0.0], [0.0; 3], now + Duration::from_secs(1));
        let f = t.fresh("b", now + Duration::from_secs(1)).unwrap();
        assert!((f.tolerance - (BASE_TOLERANCE + 30.0 * LAG_ALLOW)).abs() < 1e-6, "{f:?}");
        // Never more than the ceiling.
        t.report("c", "r1", [0.0; 3], [10_000.0, 0.0, 0.0], now);
        assert_eq!(t.fresh("c", now).unwrap().tolerance, MAX_TOLERANCE);
    }

    #[test]
    fn stale_and_gone_positions_are_not_used() {
        let t = Truths::default();
        let now = Instant::now();
        t.report("a", "r1", [0.0; 3], [0.0; 3], now);
        assert!(t.fresh("a", now + FRESH + Duration::from_millis(1)).is_none(), "stale");
        t.report("a", "r1", [0.0; 3], [0.0; 3], now);
        t.gone("a", "r2");
        assert!(t.fresh("a", now).is_some(), "another region's 'gone' does not remove r1's sample");
        t.gone("a", "r1");
        assert!(t.fresh("a", now).is_none());
        t.report("z", "r1", [0.0; 3], [0.0; 3], now);
        t.prune(now + KEEP + Duration::from_secs(1));
        assert_eq!(t.len(), 0);
    }
}
