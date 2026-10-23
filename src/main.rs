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

//! wolfvoice — the shared WebRTC voice service for WolfStorm and Firestorm.
//!
//! The region never talks WebRTC itself. OpenSim's os-webrtc-janus addon is used
//! purely as a relay: `WebRtcVoice.dll:WebRtcVoiceServiceConnector` forwards the
//! viewer's ProvisionVoiceAccountRequest / VoiceSignalingRequest capabilities to
//! this process as JSON-RPC, and we are the actual WebRTC peer for every viewer.
//!
//! Because both Firestorm and WolfStorm reach us through the same region
//! capability, they land in the same rooms with the same spatialisation.

mod admission;
mod bridge;
mod ice;
mod mixer;
mod npc;
mod proto;
mod room;
mod session;
mod truth;

use bytes::Bytes;
use hyper::body::HttpBody as _;
use hyper::service::service_fn;
use hyper::{Body, Method, Request, Response, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use room::{RoomKey, Session};
use session::{Endpoint, PortPool};

/// Where the JSON-RPC listener binds. TLS only, deliberately: the SDP carries this
/// server's DTLS fingerprint, so an attacker able to rewrite signalling in flight
/// could substitute their own and become the media endpoint. Restrict this port to
/// your region hosts at the firewall as well — it has no authentication of its own.
const RPC_BIND: &str = "0.0.0.0:9443";

/// Public address our ICE host candidates must advertise.
///
/// REQUIRED — there is deliberately no default. This must be an address that
/// viewers on the public internet can reach, because it is the only address they
/// ever learn: the Linden Lab voice protocol gives the server no way to trickle
/// candidates, so our candidate list has to be complete in the SDP answer.
///
/// If the machine has the address directly attached (the common case on a VPS),
/// use it as-is. Behind 1:1 NAT you must still advertise the PUBLIC address here,
/// and forward the media port range to this host.
const PUBLIC_IP_ENV: &str = "WOLFVOICE_PUBLIC_IP";

/// Media port range. Must match the UDP range opened in your firewall, and is also
/// the ceiling on concurrent sessions (one socket per peer connection).
const MEDIA_PORT_LO: u16 = 40000;
const MEDIA_PORT_HI: u16 = 40999;

const TLS_CERT: &str = "/etc/wolfvoice/tls/fullchain.pem";
const TLS_KEY: &str = "/etc/wolfvoice/tls/privkey.pem";

/// Mixer cadence. One Opus frame per tick per listener.
const TICK: Duration = Duration::from_millis(20);

/// Largest JSON-RPC body we will read. The biggest legitimate one is an SDP offer
/// plus a trickled ICE candidate list; a few hundred KiB is ample.
const MAX_RPC_BODY: usize = 256 * 1024;

/// Ceiling on concurrent sessions, checked before we allocate a media port.
/// Without it, anything able to reach the JSON-RPC port can exhaust the UDP port
/// range and the mixer's CPU budget just by asking for sessions. Kept below the
/// port range (MEDIA_PORT_HI - MEDIA_PORT_LO) so the pool always has spare ports.
pub(crate) const MAX_SESSIONS: usize = 900;

/// Sessions one agent may hold at once. Stock Firestorm opens one connection per nearby
/// WebRTC region (llvoicewebrtc.cpp estateSessionState neighbour set - up to four at a region
/// corner) plus one per group or IM call, and it must never be refused for that; 12 leaves
/// room while still stopping one resident taking the whole service (security scan M5).
const MAX_SESSIONS_PER_AGENT: usize = 12;

/// A session whose data channel never opened within this is a dead handshake: reaped.
const UNREADY_REAP: Duration = Duration::from_secs(30);
/// [2026-10-22] A viewer sends its join as soon as its data channel is ready (llvoicewebrtc.cpp VOICE_STATE_WAIT_FOR_DATA_CHANNEL
/// -> sendJoin); one still silent after this long is logged once.
const JOIN_OVERDUE: Duration = Duration::from_secs(10);

/// The JSON-RPC key the region relay must present (env, or this file). When neither is set the
/// service runs open, as before, and says so loudly at start-up.
const RPC_KEY_ENV: &str = "WOLFVOICE_RPC_KEY";
const RPC_KEY_FILE: &str = "/etc/wolfvoice/rpc_key";

/// [HARDENING 2026-10-23] The key room credentials are signed with (env, or this file), kept
/// apart from the relay key: the relay key is also held by every caller of the JSON-RPC (the
/// web hosts use it for wolf.npc_speak), and none of them should be able to mint a ticket into
/// someone's call. The region reads the same secret from [WebRtcVoice] CredKeyFile
/// (default /etc/opensim/voice_cred_key). Not configured: credentials are checked against the
/// relay key, exactly as before, so the order things are deployed in cannot break voice.
const CRED_KEY_ENV: &str = "WOLFVOICE_CRED_KEY";
const CRED_KEY_FILE: &str = "/etc/wolfvoice/cred_key";
/// While a separate credential key is configured, credentials signed with the RELAY key are
/// still accepted unless this is "0"/"false"/"off": regions not yet restarted onto the new
/// module sign with the relay key, and stock Firestorm keeps the credentials of a call for as
/// long as the call lasts (llvoicewebrtc.cpp:1483 startAdHocSession stores them; every
/// reconnect sends them again, :3400). Turn it off once every region runs the new module.
const LEGACY_CREDS_ENV: &str = "WOLFVOICE_LEGACY_CREDENTIALS";

/// [HARDENING 2026-10-23] The most different avatars one multiagent room may hold. A private
/// call is two people by definition. A conference: the region module's conference service
/// allows at most 25 members (WolfConference.cs CONFERENCE_MAX_INVITES + the inviter). Group voice and Wolf Chat
/// rooms: enough for a big meeting, while a room's mix stays well inside one tick (every
/// listener's mix sums every other speaker, room.rs mix_room_with).
const P2P_ROOM_MAX: usize = 2;
const CONF_ROOM_MAX: usize = 25;
const GROUP_ROOM_MAX: usize = 64;

/// [HARDENING 2026-10-23] The most entries one wolf.positions request may carry.
const MAX_POSITIONS: usize = 4096;
/// The JSON-RPC method the region module feeds avatar positions through (src/truth.rs).
const METHOD_POSITIONS: &str = "wolf.positions";

/// Connections at once, and how long one may take (TLS handshake + request): bounds a
/// slow-loris or connection flood (security scan M5).
const MAX_CONNECTIONS: usize = 512;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// A viewer-supplied id, made safe to write into a log line.
fn clean_id(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect()
}

/// [HARDENING 2026-10-23] Any outside text (a viewer's candidate or data-channel message, an
/// error that quotes one) made safe for one log line: control characters, newlines and quotes
/// escaped, at most 200 characters, so it cannot forge or split journal lines.
pub(crate) fn log_safe(s: &str) -> String {
    let mut out: String = s.chars().take(200).flat_map(char::escape_debug).collect();
    if s.chars().nth(200).is_some() {
        out.push('…');
    }
    out
}

/// Compare without leaking how much matched through the time taken.
fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// [WOLF CALLS 2026-10-19] The credential that admits one avatar to one private call room:
/// lowercase hex HMAC-SHA256(relay key, "wolfcall1:" + channel + ":" + avatar uuid). The region
/// computes the same string (WebRtcVoiceRegionModule.CallCredentials) and hands it to each of the
/// two people in the call; the avatar uuid is the one the REGION vouches for (params.user_id,
/// OpenMetaverse UUID.ToString(): lowercase, hyphenated), so a credential cannot be reused by
/// anyone else.
fn call_credentials(key: &str, channel: &str, user_id: &str) -> String {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
    let tag = ring::hmac::sign(&k, format!("wolfcall1:{channel}:{user_id}").as_bytes());
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// [HARDENING 2026-10-23] The two parties of a private call, from its channel and one party.
///
/// The region names a private call's room after the IM session, which for a P2P IM is the XOR
/// of the two avatars (llimview.cpp:2620; GridInstantMessage.cs:111), and before this every
/// credential for that channel opened the same room. But XOR does not pin a pair: anyone whose
/// id was (session XOR someone) could be issued a credential for the same channel - an HG
/// visitor's id is chosen by its home grid - and walk into the call. The room is therefore keyed
/// on the PAIR the region vouched for: this avatar (params.user_id) and channel XOR this avatar.
/// The two real parties derive the same pair; anyone else derives a pair with themselves in it.
/// None when the channel is not a bare uuid (group-, conf-, wchan- rooms) or names no other avatar.
fn p2p_pair(channel: &str, user_id: &str) -> Option<String> {
    if channel.len() != 36 || user_id.len() != 36 {
        return None;
    }
    let c = uuid::Uuid::parse_str(channel).ok()?;
    let u = uuid::Uuid::parse_str(user_id).ok()?;
    let mut other = [0u8; 16];
    for (i, b) in other.iter_mut().enumerate() {
        *b = c.as_bytes()[i] ^ u.as_bytes()[i];
    }
    let other = uuid::Uuid::from_bytes(other);
    if other.is_nil() || other == u {
        return None;
    }
    let (a, b) = (u.hyphenated().to_string(), other.hyphenated().to_string());
    Some(if a < b { format!("p2p:{a}:{b}") } else { format!("p2p:{b}:{a}") })
}

/// The room a multiagent channel opens for this avatar, and the most different avatars it
/// may hold.
fn multiagent_room(channel: &str, user_id: &str) -> Result<(RoomKey, usize), String> {
    if let Some(pair) = p2p_pair(channel, user_id) {
        return Ok((RoomKey::MultiAgent { channel: pair }, P2P_ROOM_MAX));
    }
    if uuid::Uuid::parse_str(channel).is_ok() {
        // A bare uuid that names no other avatar (it is this avatar's own id, or nil).
        return Err("not a call this avatar can be in".into());
    }
    let cap = if channel.starts_with("conf-") { CONF_ROOM_MAX } else { GROUP_ROOM_MAX };
    Ok((RoomKey::MultiAgent { channel: channel.to_string() }, cap))
}

struct App {
    rpc_key: Option<String>,
    /// [HARDENING 2026-10-23] The separate room-credential key, when one is configured.
    cred_key: Option<String>,
    /// Accept credentials signed with the relay key too (LEGACY_CREDS_ENV).
    legacy_creds: bool,
    /// [HARDENING 2026-10-23] Race-free session ceilings (src/admission.rs).
    admission: admission::Admission,
    /// [HARDENING 2026-10-23] Region-reported avatar positions (src/truth.rs).
    truths: truth::Truths,
    /// [HARDENING 2026-10-23] Which remote ICE candidates we will try (src/ice.rs).
    ice: ice::IceFilter,
    sessions: room::Registry,
    endpoints: RwLock<HashMap<String, Arc<Endpoint>>>,
    ports: PortPool,
    public_ip: String,
    runtime: Arc<dyn webrtc::runtime::Runtime>,
    /// Story characters speaking (optional, src/npc.rs).
    npcs: npc::NpcVoices,
    /// Hosts story clips may come from (npc::HOSTS_ENV); empty = story voices off.
    npc_hosts: Vec<String>,
    /// [DISCORD VOICE 2026-10-22] Discord voice channels in Wolf Chat rooms (src/bridge.rs).
    bridges: bridge::Bridges,
}

impl App {
    /// [HARDENING 2026-10-23] Is `given` the credential the region issued `user_id` for
    /// `channel`? Checked against the credential key, and, during the move to separate keys,
    /// against the relay key as well (LEGACY_CREDS_ENV). With no key at all nothing passes.
    fn credentials_ok(&self, channel: &str, user_id: &str, given: &str) -> bool {
        let signing = self.cred_key.as_deref().or(self.rpc_key.as_deref());
        let Some(key) = signing else { return false };
        if same_secret(&call_credentials(key, channel, user_id), given) {
            return true;
        }
        if self.legacy_creds && self.cred_key.is_some() {
            if let Some(rpc) = self.rpc_key.as_deref() {
                return same_secret(&call_credentials(rpc, channel, user_id), given);
            }
        }
        false
    }

    /// [HARDENING 2026-10-23] `wolf.positions`: where the region's root agents really are
    /// (src/truth.rs). Answers with the agents of the batch that hold a spatial session here,
    /// so the region keeps sending only those every second (WolfPositionFeed).
    fn positions(&self, raw: &Value) -> Result<Value, String> {
        let now = std::time::Instant::now();
        let vec3 = |v: Option<&Value>| -> Option<[f64; 3]> {
            let a = v?.as_array()?;
            if a.len() != 3 {
                return None;
            }
            let p = [a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?];
            // Global metres: the grid is at most a few million metres across.
            p.iter().all(|c| c.is_finite() && c.abs() < 1.0e8).then_some(p)
        };
        let is_uuid = |s: &str| s.len() == 36 && uuid::Uuid::parse_str(s).is_ok();
        let list = |k: &str| raw.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let positions = list("positions");
        let gone = list("gone");
        if positions.len() + gone.len() > MAX_POSITIONS {
            return Err(format!("more than {MAX_POSITIONS} positions in one request"));
        }
        let in_voice = self.sessions.spatial_agents();
        let mut want = Vec::new();
        for e in &positions {
            let (Some(agent), Some(scene)) = (e.get("a").and_then(Value::as_str), e.get("s").and_then(Value::as_str)) else { continue };
            if !is_uuid(agent) || !is_uuid(scene) {
                continue;
            }
            let Some(pos) = vec3(e.get("p")) else { continue };
            let vel = vec3(e.get("v")).unwrap_or([0.0; 3]);
            self.truths.report(agent, scene, pos, vel, now);
            if in_voice.contains(agent) {
                want.push(Value::from(agent));
            }
        }
        for e in &gone {
            if let (Some(agent), Some(scene)) = (e.get("a").and_then(Value::as_str), e.get("s").and_then(Value::as_str)) {
                self.truths.gone(agent, scene);
            }
        }
        Ok(json!({ "want": want }))
    }

    /// `wolf.npc_speak` (optional story voices, src/npc.rs): fetch the story service's
    /// clip, decode it, and queue it as a virtual speaker for the region `scene`. The reply comes
    /// once the clip is queued, not when it has finished playing.
    async fn npc_speak(&self, raw: &Value) -> Result<Value, String> {
        if self.npc_hosts.is_empty() {
            return Err(format!("story voices are off on this server ({} is not set)", npc::HOSTS_ENV));
        }
        let req = npc::parse_request(raw, &self.npc_hosts)?;
        let url = npc::check_url(&req.url, &self.npc_hosts)?;
        let bytes = npc::fetch(&url).await?;
        let (rate, samples) = npc::parse_wav(&bytes)?;
        let frames = npc::to_frames(&samples, rate);
        if frames.is_empty() {
            return Err("clip has no audio".into());
        }
        let ms = frames.len() as u64 * TICK.as_millis() as u64;
        self.npcs.add(&req, frames)?;
        log::info!("npc {} speaks {ms} ms in region {}", req.speaker, req.scene);
        Ok(json!({ "ok": true, "ms": ms }))
    }

    fn endpoint(&self, id: &str) -> Option<Arc<Endpoint>> {
        self.endpoints.read().get(id).cloned()
    }

    /// Tear a session down completely: transport, registry and room membership,
    /// then tell everyone still in the room that this agent has gone.
    async fn drop_session(&self, id: &str) {
        let ep = self.endpoints.write().remove(id);
        self.bridges.forget(id);
        let gone = self.sessions.remove(id);

        if let Some(ep) = ep {
            ep.close().await;
            // [HARDENING 2026-10-23] Only now is the media port really free again.
            self.ports.release(ep.port);
        }

        // Nothing else in the protocol expires a participant, so without this the
        // viewer keeps a departed avatar in its voice panel forever
        // (llvoicewebrtc.cpp:3212-3219 is the only removal path).
        if let Some(gone) = &gone {
            // [DISCORD VOICE 2026-10-22] A bridge in that room is told who is left (src/bridge.rs).
            self.bridges.room_changed(&gone.room());
            // A spatial session that never reported a position was never mixed
            // (see `room::mix_room`), which is indistinguishable from a broken
            // media path unless we say so. Record it on the way out.
            if gone.spatial && !gone.reported_position() {
                log::warn!(
                    "session {id} agent {} closed WITHOUT ever reporting a position; \
                     it was never audible to anyone and heard nobody",
                    gone.agent_id
                );
            }

            // Only announce a departure if this agent has no OTHER session still in
            // the room — an agent legitimately holds one session per region it can
            // hear, and losing a neighbour connection does not mean they left.
            let remaining = self.sessions.members(&gone.room());
            let still_present = remaining.iter().any(|m| m.agent_id == gone.agent_id);
            if !still_present {
                let notice = proto::leave_json(&gone.agent_id);
                for m in remaining {
                    if let Some(ep) = self.endpoint(&m.id) {
                        if ep.data_channel_open() {
                            ep.send_roster(&notice).await;
                        }
                    }
                }
            }
        }

        // [2026-10-22] One line per session end, with why it ended, whether the viewer ever joined and how long it lived,
        // so "viewer logged out" / "ICE failed" / "never joined" can be told apart without raising the log level.
        match &gone {
            Some(g) => log::info!(
                "session {id} removed ({} live): {}, {}, lived {} s, {}",
                self.sessions.session_count(),
                g.end_reason(),
                if g.spatial { "spatial" } else { "room" },
                g.created.elapsed().as_secs(),
                if g.joined.load(Ordering::Acquire) { "joined" } else { "NEVER JOINED" }
            ),
            None => log::info!("session {id} removed ({} live)", self.sessions.session_count()),
        }
    }

    /// ProvisionVoiceAccountRequest.
    ///
    /// Two shapes arrive here: a teardown (`logout`) and a connection request
    /// carrying an SDP offer. Source: llvoicewebrtc.cpp:2731-2734 and :2794-2802.
    async fn provision(&self, params: &proto::RpcParams) -> Result<Value, String> {
        let req = proto::ProvisionRequest::parse(&params.request)?;

        if req.logout {
            let Some(id) = req.viewer_session.clone() else {
                return Err("logout without viewer_session".into());
            };
            // Only the session's own agent may end it (security scan, Low).
            if let Some(s) = self.sessions.get(&id) {
                if s.agent_id != params.user_id {
                    return Err("that voice session is not yours".into());
                }
                s.set_end_reason("viewer logout");
            }
            self.drop_session(&id).await;
            // The viewer ignores this body (breakVoiceConnectionCoro discards the
            // result at llvoicewebrtc.cpp:2748), but the region-side connector
            // still expects a map.
            return Ok(json!({ "viewer_session": id }));
        }

        if req.wolf_move {
            return self.move_session(params, &req).await;
        }

        let offer = req
            .offer_sdp
            .as_deref()
            .ok_or_else(|| "provision without a jsep offer".to_string())?;

        // A renegotiation for an existing session: drop the old transport first so
        // we never leave two peer connections fighting over one agent.
        if let Some(existing) = req.viewer_session.clone() {
            if let Some(s) = self.sessions.get(&existing) {
                if s.agent_id != params.user_id {
                    return Err("that voice session is not yours".into());
                }
                log::info!("session {} re-provisioning; dropping old transport", clean_id(&existing));
                s.set_end_reason("re-provisioned by the viewer");
                self.drop_session(&existing).await;
            }
        }

        // Identity comes from the REGION (params.user_id), never from the viewer's
        // own request body. That is the whole security value of routing voice
        // through the capability: the region issued the cap per-agent and told us
        // who it belongs to.
        if params.user_id.is_empty() {
            return Err("region did not supply userID".into());
        }

        let channel_type = req.channel_type.unwrap_or(proto::ChannelType::Local);
        let spatial = channel_type == proto::ChannelType::Local;
        let (room, room_cap) = match channel_type {
            proto::ChannelType::Local => (
                RoomKey::Spatial {
                    region: params.scene.clone(),
                    parcel: req.parcel_local_id,
                },
                None,
            ),
            proto::ChannelType::MultiAgent => {
                let channel = req
                    .channel
                    .clone()
                    .ok_or_else(|| "multiagent request without a channel".to_string())?;
                // [WOLF CALLS 2026-10-19] A call room is private: only the avatars the region
                // invited may join (the region signs channel + avatar - WebRtcVoiceRegionModule
                // CallCredentials). Before that, anyone who named a channel was let in. With no key
                // there is nothing to check a signature against, so call rooms are refused rather
                // than left open. [HARDENING 2026-10-23] Signed with the credential key, or the
                // relay key during the change-over (credentials_ok).
                if self.cred_key.is_none() && self.rpc_key.is_none() {
                    return Err("private calls need the relay key".into());
                }
                let given = req.credentials.as_deref().unwrap_or_default();
                if !self.credentials_ok(&channel, &params.user_id, given) {
                    log::warn!(
                        "refused call room {} for agent {}: not invited",
                        clean_id(&channel),
                        clean_id(&params.user_id)
                    );
                    return Err("not invited to this call".into());
                }
                let (room, cap) = multiagent_room(&channel, &params.user_id)?;
                (room, Some(cap))
            }
        };

        // [HARDENING 2026-10-23] Every ceiling - the service's, this avatar's and the room's -
        // checked and claimed in one step, so parallel provisions cannot all pass the same
        // check (src/admission.rs). Refused politely rather than exhausting ports or CPU: the
        // viewer treats a failed provision as retryable, so this degrades to "voice
        // unavailable" instead of taking the service down for everyone already connected.
        let reservation = match self.admission.reserve(
            &self.sessions,
            &params.user_id,
            &room,
            room_cap,
            MAX_SESSIONS,
            MAX_SESSIONS_PER_AGENT,
        ) {
            Ok(r) => r,
            Err(why) => {
                log::warn!(
                    "refusing provision for agent {} ({}): {}",
                    clean_id(&params.user_id),
                    if spatial { "spatial" } else { "room" },
                    why.message()
                );
                return Err(why.message().into());
            }
        };

        // [HARDENING 2026-10-23] Candidates in the offer are addresses we will send STUN checks
        // to: only reachable public ones (and our own networks) are kept (src/ice.rs).
        let (offer, dropped) = self.ice.filter_sdp(offer);
        if dropped > 0 {
            log::debug!("agent {}: {dropped} unreachable candidate(s) removed from the offer", clean_id(&params.user_id));
        }

        // A PLAIN uuid, not "wv-<uuid>". The LL/Firestorm viewer keeps viewer_session in an
        // LLUUID (llvoicewebrtc.cpp mViewerSession), and LLUUID::set() turns anything that is
        // not 36 characters into the null uuid (lluuid.cpp:215-237) - so with the old prefix
        // every ICE trickle and every logout from stock Firestorm reached us as 00000000-...
        // (found 2026-10-09 while building wolf_move). A uuid round-trips through both.
        let id = uuid::Uuid::new_v4().to_string();
        let sess = Arc::new(Session::new(
            id.clone(),
            params.user_id.clone(),
            room,
            spatial,
        ));
        let in_offer = offer.matches("a=candidate:").count() as u32;
        sess.remote_candidates.store(in_offer, Ordering::Relaxed);

        // [HARDENING 2026-10-23] A port no live session holds (session.rs PortPool).
        let port = self
            .ports
            .take(&self.public_ip)
            .ok_or_else(|| "voice service has no free media port".to_string())?;
        let ep = match session::establish(sess.clone(), &offer, &self.public_ip, port, self.runtime.clone()).await {
            Ok(ep) => ep,
            Err(e) => {
                self.ports.release(port);
                return Err(e);
            }
        };

        let Some(sdp) = session::answer_sdp(&ep).await else {
            ep.close().await;
            self.ports.release(port);
            return Err("no answer sdp".to_string());
        };

        self.endpoints.write().insert(id.clone(), Arc::new(ep));
        let joined_room = sess.room();
        self.sessions.insert(sess);
        // Registered: the registry counts it from here, so the reservation is given back.
        drop(reservation);
        self.bridges.room_changed(&joined_room);

        // The PARCEL is logged deliberately: spatial rooms are keyed on region +
        // parcel, so "why can these two not hear each other" is almost always
        // answered by them being on different parcels — and that is unanswerable
        // from the log without this field.
        log::info!(
            "session {id} agent {} region {} parcel {} port {port} spatial={spatial} ({} live)",
            clean_id(&params.user_id),
            clean_id(&params.scene),
            req.parcel_local_id,
            self.sessions.session_count()
        );

        Ok(proto::provision_answer(&id, &sdp))
    }

    /// The Wolf "wolf_move": put an existing spatial session into the room of the
    /// region this request came through, keeping its peer connection, so a crossing or
    /// teleport is a change of room and not a new call.
    ///
    /// Security is the same as a new provision: the REGION names the agent
    /// (params.user_id, from the per-agent cap) and the scene, and we only move a
    /// session that already belongs to that agent. A viewer cannot move someone
    /// else's session, nor into a region it is not in (that region would not have
    /// issued it a cap).
    async fn move_session(&self, params: &proto::RpcParams, req: &proto::ProvisionRequest) -> Result<Value, String> {
        let id = req
            .viewer_session
            .clone()
            .ok_or_else(|| "wolf_move without viewer_session".to_string())?;
        if params.user_id.is_empty() || params.scene.is_empty() {
            return Err("region did not supply userID and scene".into());
        }
        let sess = self
            .sessions
            .get(&id)
            .ok_or_else(|| format!("unknown viewer_session {}", clean_id(&id)))?;
        if sess.agent_id != params.user_id {
            log::warn!("wolf_move refused: session {} belongs to {}, not {}", clean_id(&id), sess.agent_id, clean_id(&params.user_id));
            return Err("that voice session is not yours".into());
        }
        if !sess.spatial {
            return Err("only local (spatial) voice moves".into());
        }
        if self.endpoint(&id).is_none() {
            return Err(format!("viewer_session {id} has no transport"));
        }

        let to = RoomKey::Spatial {
            region: params.scene.clone(),
            parcel: req.parcel_local_id,
        };
        let Some(from) = self.sessions.relocate(&id, to.clone()) else {
            // Already there: nothing to do, and that is success.
            return Ok(json!({ "viewer_session": id, "wolf_moved": true }));
        };
        self.bridges.room_changed(&from);
        self.bridges.room_changed(&to);

        // The old room: this agent has gone, unless they still hold another session
        // there (a neighbour connection) - the rule drop_session uses.
        let old_members = self.sessions.members(&from);
        if !old_members.iter().any(|m| m.agent_id == sess.agent_id) {
            let notice = proto::leave_json(&sess.agent_id);
            for m in &old_members {
                if let Some(ep) = self.endpoint(&m.id) {
                    if ep.data_channel_open() {
                        ep.send_roster(&notice).await;
                    }
                }
            }
        }
        // The mover: everyone it could hear in the old room is gone from its list,
        // except people it will meet again in the new room (they are announced as
        // joins on the next tick anyway, and a leave + join would only flicker).
        if let Some(ep) = self.endpoint(&id) {
            if ep.data_channel_open() {
                let new_agents: std::collections::HashSet<String> =
                    self.sessions.members(&to).iter().map(|m| m.agent_id.clone()).collect();
                for m in &old_members {
                    if m.agent_id != sess.agent_id && !new_agents.contains(&m.agent_id) {
                        ep.send_roster(&proto::leave_json(&m.agent_id)).await;
                    }
                }
            }
        }

        log::info!(
            "session {id} agent {} MOVED {:?} -> region {} parcel {} (same transport)",
            sess.agent_id, from, params.scene, req.parcel_local_id
        );
        Ok(json!({ "viewer_session": id, "wolf_moved": true }))
    }

    /// VoiceSignalingRequest — trickled ICE candidates.
    /// Source: llvoicewebrtc.cpp:2496-2519.
    async fn signaling(&self, params: &proto::RpcParams) -> Result<Value, String> {
        let req = proto::SignalingRequest::parse(&params.request);
        let Some(id) = req.viewer_session.clone() else {
            return Err("signaling without viewer_session".into());
        };
        let Some(ep) = self.endpoint(&id) else {
            // Not fatal: the viewer trickles candidates concurrently with the
            // provision call and may reference a session we already tore down.
            return Err(format!("unknown viewer_session {}", clean_id(&id)));
        };
        if ep.session.agent_id != params.user_id {
            return Err("that voice session is not yours".into());
        }

        // add_ice_candidate logs both outcomes itself; a rejection here is not
        // fatal to the session, because the others may still pair.
        // [HARDENING 2026-10-23] At most MAX_CANDIDATES_PER_REQUEST from one request and
        // MAX_CANDIDATES_PER_SESSION over the session's life, and only addresses we may send to
        // (src/ice.rs); the rest are dropped without being tried.
        let mut skipped = 0usize;
        for c in req.candidates.iter().take(ice::MAX_CANDIDATES_PER_REQUEST) {
            if !self.ice.allows_candidate(&c.candidate) {
                skipped += 1;
                continue;
            }
            if ep.session.remote_candidates.fetch_add(1, Ordering::Relaxed) >= ice::MAX_CANDIDATES_PER_SESSION {
                skipped += 1;
                continue;
            }
            let _ = ep.add_ice_candidate(c).await;
        }
        skipped += req.candidates.len().saturating_sub(ice::MAX_CANDIDATES_PER_REQUEST);
        if skipped > 0 {
            log::debug!("session {} {skipped} remote candidate(s) not used", clean_id(&id));
        }
        if req.completed {
            log::debug!("session {id} end-of-candidates");
        }
        Ok(json!({ "viewer_session": id }))
    }
}

/// Mixer loop: one pass per room per 20 ms, spread across all available cores.
///
/// The work parallelises on two independent axes, and we use both:
///
///   * **rooms** never interact, so each room's tick is its own task;
///   * **listeners within a room** each need their own Opus encode, which is the
///     dominant cost, and those are independent too — so each listener is its own
///     task as well. That second axis matters because the interesting case is one
///     busy region, where per-room parallelism alone would still leave everything
///     on a single core.
///
/// Task churn is real but small against the work: an encode is hundreds of
/// microseconds, a tokio spawn is a couple.
///
/// Overload behaviour is deliberate. A tick is skipped entirely if the previous
/// tick's tasks have not all finished. That gives two things at once: outstanding
/// work for one endpoint can never overlap itself (which would let two frames race
/// and emit RTP timestamps out of order), and a server that cannot keep up degrades
/// by dropping whole frames — audible, but bounded — rather than growing a task
/// queue until it dies.
async fn mixer_loop(app: Arc<App>) {
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut skipped: u64 = 0;
    let mut last_warned = std::time::Instant::now();
    let mut last_pruned = std::time::Instant::now();

    loop {
        ticker.tick().await;

        if inflight.load(Ordering::Acquire) != 0 {
            skipped += 1;
            // Rate-limit the complaint; at 50 ticks a second an unthrottled log
            // would itself become the bottleneck.
            if last_warned.elapsed() >= Duration::from_secs(10) {
                log::warn!(
                    "mixer overloaded: {skipped} tick(s) skipped in the last 10s \
                     ({} sessions). Audio will break up; the host needs more CPU \
                     or fewer concurrent listeners.",
                    app.sessions.session_count()
                );
                skipped = 0;
                last_warned = std::time::Instant::now();
            }
            continue;
        }

        // Story characters: one frame each for this tick, shared by every room of their region.
        let npc_frames = app.npcs.tick();

        // [HARDENING 2026-10-23] Region positions nobody will use again.
        if last_pruned.elapsed() >= Duration::from_secs(10) {
            app.truths.prune(std::time::Instant::now());
            last_pruned = std::time::Instant::now();
        }

        // Reap anything the transport marked dead before mixing.
        let dead: Vec<String> = app
            .endpoints
            .read()
            .values()
            .filter(|ep| {
                ep.session.closed.load(Ordering::Relaxed)
                    || (!ep.session.ready.load(Ordering::Acquire) && ep.session.created.elapsed() > UNREADY_REAP)
            })
            .map(|ep| ep.session.id.clone())
            .collect();
        for id in dead {
            if let Some(ep) = app.endpoint(&id) {
                // Closed sessions recorded their own cause (session.rs); what is left is the unready reap.
                ep.session.set_end_reason("no data channel within 30 s");
            }
            app.drop_session(&id).await;
        }
        // [2026-10-22] A data channel open for JOIN_OVERDUE with no join from the viewer: said once per session.
        for ep in app.endpoints.read().values() {
            if ep.session.join_overdue(JOIN_OVERDUE) {
                log::warn!(
                    "session {} agent {} has had its data channel open {} s without the viewer's join; \
                     it sees nobody and is not announced to anyone",
                    ep.session.id,
                    ep.session.agent_id,
                    JOIN_OVERDUE.as_secs()
                );
            }
        }

        for key in app.sessions.rooms() {
            let members = app.sessions.members(&key);
            if members.is_empty() {
                continue;
            }

            // A room of ONE still needs its roster.
            //
            // Firestorm does not depend on this for its OWN entry — it adds itself
            // locally the moment the peer connection comes up
            // (llvoicewebrtc.cpp:1020, in OnConnectionEstablished). WolfStorm does
            // not: it builds its participant list purely from the keys we send
            // (js/voice/voice_llwebrtc.js), so a lone WolfStorm user previously had
            // no entry for themselves at all and no level meter.
            //
            // Skipping the room also left the per-listener announce state unbuilt
            // and the speaker level pinned at zero, so the behaviour changed shape
            // at exactly N=2 for no good reason. Whether a user is alone in their
            // region has nothing to do with whether their own voice UI works.
            //
            // No audio can exist in a room of one, so the mix is still not sent:
            // that keeps the Opus encode off the hot path for the common case of
            // a mostly-empty grid, exactly as before.
            let room_npcs: Vec<Arc<npc::NpcFrame>> = match &key {
                RoomKey::Spatial { region, .. } => npc_frames.iter().filter(|f| &f.scene == region).cloned().collect(),
                RoomKey::MultiAgent { .. } => Vec::new(),
            };
            // A lone listener still hears a story character speaking near them.
            let alone = members.len() < 2 && room_npcs.is_empty();

            let app = app.clone();
            let inflight = inflight.clone();
            inflight.fetch_add(1, Ordering::AcqRel);
            tokio::spawn(async move {
                // Summing is cheap next to the encodes and needs the whole room's
                // frames at once, so it stays inline in the room's own task.
                let outputs = room::mix_room_with(&members, &room_npcs, &app.truths);

                let mut handles = Vec::with_capacity(outputs.len());
                for out in outputs {
                    let app = app.clone();
                    handles.push(tokio::spawn(async move {
                        // [DISCORD VOICE 2026-10-22] A Discord bridge has no WebRTC endpoint: its mix goes down its own
                        // connection (src/bridge.rs), and its roster is not needed (it is sent the room's people itself).
                        if let Some(b) = app.bridges.get(&out.session.id) {
                            if !alone {
                                b.send_mix(&out.stereo);
                            }
                            return;
                        }
                        let Some(ep) = app.endpoint(&out.session.id) else {
                            return;
                        };
                        // send_roster owns the "was it delivered?" bookkeeping, so
                        // it must be called even when the channel has just gone —
                        // it un-records anything that did not reach the wire.
                        if let Some(roster) = out.roster {
                            ep.send_roster(&roster).await;
                        }
                        // Media is pointless before the data channel opens (the
                        // viewer is not in VOICE_STATE_SESSION_UP until it does —
                        // llvoicewebrtc.cpp:2976-2990), and a listener with nobody
                        // else in the room has nothing but silence coming.
                        // [HARDENING 2026-10-23] Nor is any media sent before the viewer has
                        // JOINED: a session that keeps its data channel open without ever
                        // saying "j" would otherwise be a hidden listener (room.rs mixes it
                        // silence; this saves the encode too).
                        if alone || !ep.data_channel_open() || !out.session.joined.load(Ordering::Acquire) {
                            return;
                        }
                        if let Err(e) = ep.send_mix(&out.stereo).await {
                            log::debug!("send_mix {}: {e}", out.session.id);
                        }
                    }));
                }
                for h in handles {
                    let _ = h.await;
                }
                inflight.fetch_sub(1, Ordering::AcqRel);
            });
        }
    }
}

// ─────────────────────────── HTTP / JSON-RPC ───────────────────────────

async fn handle(app: Arc<App>, req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    // A tiny health endpoint, reachable only from the allow-listed region hosts.
    // [HARDENING 2026-10-23] Without the relay key it says only "ok"; the counts (which tell
    // anyone how many people are in voice) need the key in an X-Wolfvoice-Key header.
    if req.method() == Method::GET {
        let keyed = match app.rpc_key.as_deref() {
            Some(k) => req
                .headers()
                .get("x-wolfvoice-key")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|given| same_secret(k, given.trim())),
            None => false,
        };
        if !keyed {
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain")
                .body(Body::from("ok"))
                .unwrap());
        }
        let body = json!({
            "service": "wolfvoice",
            "sessions": app.sessions.session_count(),
            "rooms": app.sessions.rooms().len(),
            "npcs": app.npcs.active(),
            "bridges": app.bridges.count(),
            "pending": app.admission.pending(),
        });
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap());
    }

    if req.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::empty())
            .unwrap());
    }

    // Bounded read. `to_bytes` would buffer the entire body with no limit, so a
    // single large POST could drive the process into its MemoryMax and get it
    // OOM-killed — a trivial denial of service for anyone who can reach the port.
    // The largest legitimate body is an SDP offer plus a trickled candidate list.
    let mut body = req.into_body();
    let mut whole = bytes::BytesMut::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        if whole.len() + chunk.len() > MAX_RPC_BODY {
            log::warn!(
                "rejecting oversize JSON-RPC body (>{} bytes) — possible abuse",
                MAX_RPC_BODY
            );
            return Ok(json_200(&proto::rpc_err(&Value::Null, "request body too large")));
        }
        whole.extend_from_slice(&chunk);
    }
    Ok(rpc_response(app, whole.freeze()).await)
}

/// Always answers 200 with a JSON object.
///
/// This is not sloppiness, it is required. OpenSim's connector calls
/// EnsureSuccessStatusCode (WebUtil.cs:428) which throws on any non-2xx, and then
/// WebRtcVoiceServiceConnector.cs:149-160 sets the task result inside the catch and
/// STILL falls through to dereference the now-null response — a NullReferenceException
/// on a detached task, with the viewer left waiting forever. A 200 carrying
/// {"error": ...} is the only way to report a failure that the region can see.
async fn rpc_response(app: Arc<App>, body: Bytes) -> Response<Body> {
    let parsed: Result<proto::RpcRequest, _> = serde_json::from_slice(&body);
    let (id, method, params) = match parsed {
        Ok(r) => (r.id, r.method, r.params),
        Err(e) => {
            return json_200(&proto::rpc_err(&Value::Null, &format!("bad json-rpc: {e}")));
        }
    };

    // The relay's key, when this service has one (security scan H5).
    if let Some(key) = app.rpc_key.as_deref() {
        if !same_secret(key, &params.key) {
            log::warn!("{} refused: wrong or missing relay key", log_safe(&method));
            return json_200(&proto::rpc_err(&id, "not authorised"));
        }
    }

    let result = match method.as_str() {
        // Story voices are never open: with no relay key installed this method is refused.
        npc::METHOD_NPC_SPEAK if app.rpc_key.is_none() => Err("story voices need the relay key".to_string()),
        npc::METHOD_NPC_SPEAK => {
            // Its params are not the relay's shape, so read them from the raw body (the key was
            // checked above from the same object).
            let raw: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            app.npc_speak(raw.get("params").unwrap_or(&Value::Null)).await
        }
        // [HARDENING 2026-10-23] Region positions; never open, like story voices.
        METHOD_POSITIONS if app.rpc_key.is_none() => Err("positions need the relay key".to_string()),
        METHOD_POSITIONS => {
            let raw: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            app.positions(raw.get("params").unwrap_or(&Value::Null))
        }
        proto::METHOD_PROVISION => app.provision(&params).await,
        proto::METHOD_SIGNALING => app.signaling(&params).await,
        other => Err(format!("unknown method {}", log_safe(other))),
    };

    match result {
        Ok(v) => json_200(&proto::rpc_ok(&id, v)),
        Err(e) => {
            log::warn!("{} failed: {}", log_safe(&method), log_safe(&e));
            json_200(&proto::rpc_err(&id, &e))
        }
    }
}

fn json_200(v: &Value) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(v.to_string()))
        .unwrap()
}

fn load_tls() -> Result<rustls::ServerConfig, String> {
    let certs = {
        let f = std::fs::File::open(TLS_CERT).map_err(|e| format!("{TLS_CERT}: {e}"))?;
        let mut r = std::io::BufReader::new(f);
        rustls_pemfile::certs(&mut r)
            .map_err(|e| format!("{TLS_CERT}: {e}"))?
            .into_iter()
            .map(rustls::Certificate)
            .collect::<Vec<_>>()
    };
    if certs.is_empty() {
        return Err(format!("{TLS_CERT} contained no certificates"));
    }

    let key = {
        let f = std::fs::File::open(TLS_KEY).map_err(|e| format!("{TLS_KEY}: {e}"))?;
        let mut r = std::io::BufReader::new(f);
        // certbot issues ECDSA keys by default in v4, which land in the PKCS#8
        // section, but accept an RSA key too so a key-type change cannot brick us.
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut r)
            .map_err(|e| format!("{TLS_KEY}: {e}"))?;
        if keys.is_empty() {
            let f = std::fs::File::open(TLS_KEY).map_err(|e| format!("{TLS_KEY}: {e}"))?;
            let mut r = std::io::BufReader::new(f);
            keys = rustls_pemfile::rsa_private_keys(&mut r)
                .map_err(|e| format!("{TLS_KEY}: {e}"))?;
        }
        if keys.is_empty() {
            let f = std::fs::File::open(TLS_KEY).map_err(|e| format!("{TLS_KEY}: {e}"))?;
            let mut r = std::io::BufReader::new(f);
            keys = rustls_pemfile::ec_private_keys(&mut r)
                .map_err(|e| format!("{TLS_KEY}: {e}"))?;
        }
        rustls::PrivateKey(
            keys.into_iter()
                .next()
                .ok_or_else(|| format!("{TLS_KEY} contained no usable private key"))?,
        )
    };

    rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("tls config: {e}"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Fail loudly rather than guessing: a wrong public address produces a service
    // that answers every request and then never connects, which is a miserable
    // thing to debug.
    let public_ip = match std::env::var(PUBLIC_IP_ENV) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!(
                "{PUBLIC_IP_ENV} is not set.\n\n\
                 Set it to the PUBLIC IP address viewers should send voice media to,\n\
                 e.g. {PUBLIC_IP_ENV}=203.0.113.10\n\n\
                 It cannot be guessed: it is the only address the viewer ever learns."
            );
            std::process::exit(2);
        }
    };

    // The relay's key: the environment wins, then the file. None = open (logged as a warning).
    let rpc_key = std::env::var(RPC_KEY_ENV)
        .ok()
        .or_else(|| std::fs::read_to_string(RPC_KEY_FILE).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    if rpc_key.is_none() {
        log::warn!(
            "no relay key ({RPC_KEY_ENV} or {RPC_KEY_FILE}): JSON-RPC is OPEN to anyone who can reach \
             the port - firewall it to the region hosts"
        );
    }

    // [HARDENING 2026-10-23] The room-credential key (CRED_KEY_ENV), separate from the relay key.
    let cred_key = std::env::var(CRED_KEY_ENV)
        .ok()
        .or_else(|| std::fs::read_to_string(CRED_KEY_FILE).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let legacy_creds = !matches!(
        std::env::var(LEGACY_CREDS_ENV).unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    );
    match (&cred_key, &rpc_key) {
        (Some(c), Some(r)) if c == r => {
            log::error!("{CRED_KEY_FILE} is the same secret as the relay key: credentials are NOT separated");
        }
        (Some(_), _) if legacy_creds => log::info!(
            "room credentials: separate credential key, and (change-over) relay-key credentials still \
             accepted - set {LEGACY_CREDS_ENV}=0 once every region runs the new module"
        ),
        (Some(_), _) => log::info!("room credentials: separate credential key only"),
        (None, Some(_)) => log::info!("room credentials: signed with the relay key (no {CRED_KEY_FILE})"),
        (None, None) => {}
    }

    let ice = match ice::IceFilter::parse(&std::env::var(ice::ALLOW_ENV).unwrap_or_default()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let env_port = |k: &str, d: u16| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok()).unwrap_or(d);
    let media_lo = env_port("WOLFVOICE_MEDIA_LO", MEDIA_PORT_LO);
    let media_hi = env_port("WOLFVOICE_MEDIA_HI", MEDIA_PORT_HI);
    if media_hi <= media_lo || ((media_hi - media_lo) as usize) < MAX_SESSIONS {
        eprintln!("WOLFVOICE_MEDIA_LO/HI must give at least {MAX_SESSIONS} ports");
        std::process::exit(2);
    }

    let npc_hosts = npc::parse_hosts(&std::env::var(npc::HOSTS_ENV).unwrap_or_default());
    if npc_hosts.is_empty() {
        log::info!("story voices off ({} not set)", npc::HOSTS_ENV);
    } else {
        log::info!("story voices on, clips from: {}", npc_hosts.join(", "));
    }

    let app = Arc::new(App {
        rpc_key,
        cred_key,
        legacy_creds,
        admission: admission::Admission::default(),
        truths: truth::Truths::default(),
        ice,
        sessions: room::Registry::default(),
        endpoints: RwLock::new(HashMap::new()),
        ports: PortPool::new(media_lo, media_hi),
        public_ip: public_ip.clone(),
        runtime: Arc::new(webrtc::runtime::TokioRuntime),
        npcs: npc::NpcVoices::default(),
        npc_hosts,
        bridges: bridge::Bridges::default(),
    });

    tokio::spawn(mixer_loop(app.clone()));

    let tls = Arc::new(load_tls()?);
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    // [2026-10-09] Overridable, so a second (test) instance can run beside the live one:
    // WOLFVOICE_BIND, WOLFVOICE_MEDIA_LO / WOLFVOICE_MEDIA_HI. Defaults are unchanged.
    let addr: SocketAddr = std::env::var("WOLFVOICE_BIND").unwrap_or_else(|_| RPC_BIND.to_string()).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // [DISCORD VOICE 2026-10-22] The Discord voice bridge's own port (src/bridge.rs), only with its key installed.
    let bridge_key = std::env::var(bridge::BRIDGE_KEY_ENV)
        .ok()
        .or_else(|| std::fs::read_to_string(bridge::BRIDGE_KEY_FILE).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    match bridge_key {
        Some(k) if k.len() >= bridge::MIN_KEY_LEN => {
            let baddr: SocketAddr = std::env::var(bridge::BRIDGE_BIND_ENV).unwrap_or_else(|_| bridge::BRIDGE_BIND.to_string()).parse()?;
            tokio::spawn(bridge::serve(app.clone(), acceptor.clone(), baddr, Arc::new(k)));
        }
        Some(_) => log::error!("{} is shorter than {} characters: Discord voice bridge NOT started", bridge::BRIDGE_KEY_FILE, bridge::MIN_KEY_LEN),
        None => log::info!("no Discord voice bridge key ({}): bridge port not opened", bridge::BRIDGE_KEY_FILE),
    }

    let connections = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));

    log::info!(
        "wolfvoice listening on {addr} (TLS), media {media_lo}-{media_hi}/udp on {public_ip}"
    );

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("accept: {e}");
                continue;
            }
        };
        let Ok(permit) = connections.clone().try_acquire_owned() else {
            log::warn!("connection from {peer} refused: {MAX_CONNECTIONS} already open");
            continue;
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let _permit = permit;
            // Handshake and request together within CONNECTION_TIMEOUT: a client that trickles
            // bytes cannot hold a connection open for ever.
            let work = async {
                let tls_stream = match acceptor.accept(stream).await {
                    Ok(s) => s,
                    Err(e) => {
                        log::debug!("tls handshake from {peer}: {e}");
                        return;
                    }
                };
                let svc = service_fn(move |req| handle(app.clone(), req));
                if let Err(e) = hyper::server::conn::Http::new()
                    .http1_only(true)
                    .http1_keep_alive(false)
                    .serve_connection(tls_stream, svc)
                    .await
                {
                    log::debug!("connection from {peer}: {e}");
                }
            };
            if tokio::time::timeout(CONNECTION_TIMEOUT, work).await.is_err() {
                log::debug!("connection from {peer} timed out");
            }
        });
    }
}

#[cfg(test)]
mod call_tests {
    use super::{call_credentials, same_secret};

    // Expected value from Python's hmac module (an independent HMAC-SHA256), 2026-10-19:
    // hmac.new(b'test-relay-key', b'wolfcall1:<channel>:<user>', hashlib.sha256).hexdigest()
    const KEY: &str = "test-relay-key";
    const CHANNEL: &str = "6f2b1c3e-0000-4000-8000-00000000abcd";
    const USER: &str = "e2afe467-ae22-44e0-be07-fb1e50ae7e6f";
    const EXPECTED: &str = "a75c55c9b07043cdea7668a3140ac1b1e8c4fb4eff326714554f65f5c8a29558";

    #[test]
    fn matches_independent_hmac() {
        assert_eq!(call_credentials(KEY, CHANNEL, USER), EXPECTED);
    }

    #[test]
    fn bound_to_avatar_channel_and_key() {
        let good = call_credentials(KEY, CHANNEL, USER);
        assert!(same_secret(&good, EXPECTED));
        assert!(!same_secret(&call_credentials(KEY, CHANNEL, "11111111-2222-3333-4444-555555555555"), &good));
        assert!(!same_secret(&call_credentials(KEY, "another-channel", USER), &good));
        assert!(!same_secret(&call_credentials("other-key", CHANNEL, USER), &good));
        assert!(!same_secret("", &good));
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn app(rpc: Option<&str>, cred: Option<&str>, legacy: bool) -> App {
        App {
            rpc_key: rpc.map(String::from),
            cred_key: cred.map(String::from),
            legacy_creds: legacy,
            admission: admission::Admission::default(),
            truths: truth::Truths::default(),
            ice: ice::IceFilter::default(),
            sessions: room::Registry::default(),
            endpoints: RwLock::new(HashMap::new()),
            ports: PortPool::new(46100, 46199),
            public_ip: "127.0.0.1".into(),
            runtime: Arc::new(webrtc::runtime::TokioRuntime),
            npcs: npc::NpcVoices::default(),
            npc_hosts: Vec::new(),
            bridges: bridge::Bridges::default(),
        }
    }

    fn xor(a: &str, b: &str) -> String {
        let (a, b) = (uuid::Uuid::parse_str(a).unwrap(), uuid::Uuid::parse_str(b).unwrap());
        let mut o = [0u8; 16];
        for (i, x) in o.iter_mut().enumerate() {
            *x = a.as_bytes()[i] ^ b.as_bytes()[i];
        }
        uuid::Uuid::from_bytes(o).hyphenated().to_string()
    }

    const A: &str = "e2afe467-ae22-44e0-be07-fb1e50ae7e6f";
    const B: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    const W: &str = "11111111-2222-3333-4444-555555555555";

    /// Gap 6: both parties of a private call reach the SAME room, and someone whose id was
    /// crafted so that (session XOR id) is a real avatar reaches a room of their own.
    #[test]
    fn a_private_call_room_is_the_pair_not_the_session() {
        let session = xor(A, B); // the P2P IM session id, llimview.cpp:2620
        let ra = p2p_pair(&session, A).unwrap();
        let rb = p2p_pair(&session, B).unwrap();
        assert_eq!(ra, rb, "caller and callee meet in one room");
        assert!(ra.contains(A) && ra.contains(B));
        // The attacker X = session XOR W (an HG id chosen by its home grid; W its own alt).
        let x = xor(&session, W);
        let rx = p2p_pair(&session, &x).unwrap();
        assert_ne!(rx, ra, "a crafted id never lands in A and B's call");
        assert!(rx.contains(W) && rx.contains(&x));
        // Not a P2P channel: named rooms keep their name; an avatar's own id is no call.
        assert_eq!(p2p_pair("group-0b5e7b4a-0000-4000-8000-000000000001", A), None);
        assert_eq!(p2p_pair(A, A), None, "channel == own id gives the nil uuid as the other party");
        assert!(multiagent_room(A, A).is_err());
        let (room, cap) = multiagent_room(&session, A).unwrap();
        assert_eq!((room, cap), (RoomKey::MultiAgent { channel: ra }, P2P_ROOM_MAX));
        assert_eq!(multiagent_room("conf-x", A).unwrap().1, CONF_ROOM_MAX);
        assert_eq!(multiagent_room("wchan-x", A).unwrap().1, GROUP_ROOM_MAX);
    }

    /// Gap 4: separate keys, with the change-over rule.
    #[test]
    fn credentials_follow_the_key_rules() {
        let ch = "group-0b5e7b4a-0000-4000-8000-000000000001";
        let by_rpc = call_credentials("relay-key", ch, A);
        let by_cred = call_credentials("cred-key", ch, A);
        // Only the relay key configured: as before.
        let old = app(Some("relay-key"), None, true);
        assert!(old.credentials_ok(ch, A, &by_rpc));
        assert!(!old.credentials_ok(ch, A, &by_cred));
        // Both keys, change-over on: either signature.
        let both = app(Some("relay-key"), Some("cred-key"), true);
        assert!(both.credentials_ok(ch, A, &by_cred));
        assert!(both.credentials_ok(ch, A, &by_rpc));
        assert!(!both.credentials_ok(ch, B, &by_cred), "bound to the avatar");
        // Change-over off: the relay key no longer mints credentials.
        let strict = app(Some("relay-key"), Some("cred-key"), false);
        assert!(strict.credentials_ok(ch, A, &by_cred));
        assert!(!strict.credentials_ok(ch, A, &by_rpc));
        // No key at all: nothing passes.
        assert!(!app(None, None, true).credentials_ok(ch, A, &by_rpc));
    }

    /// Gap 7: wolf.positions stores the region's positions, answers with the agents in voice,
    /// forgets on "gone", and ignores malformed entries.
    #[test]
    fn positions_are_stored_and_answered() {
        let app = app(Some("k"), None, true);
        let s = Arc::new(room::Session::new("s1".into(), A.into(), RoomKey::Spatial { region: "r".into(), parcel: 0 }, true));
        app.sessions.insert(s);
        let scene = "0b5e7b4a-0000-4000-8000-000000000001";
        let v = json!({ "positions": [
            { "a": A, "s": scene, "p": [1000.0, 2000.0, 25.0], "v": [1.0, 0.0, 0.0] },
            { "a": B, "s": scene, "p": [1010.0, 2000.0, 25.0] },
            { "a": "not-a-uuid", "s": scene, "p": [0.0, 0.0, 0.0] },
            { "a": W, "s": scene, "p": [f64::MAX, 0.0, 0.0] }
        ]});
        let r = app.positions(&v).unwrap();
        assert_eq!(r["want"], json!([A]), "only agents with a spatial session are wanted");
        let now = std::time::Instant::now();
        assert_eq!(app.truths.fresh(A, now).unwrap().pos, [1000.0, 2000.0, 26.0]);
        assert!(app.truths.fresh(B, now).is_some());
        assert!(app.truths.fresh(W, now).is_none(), "out-of-range numbers are refused");
        app.positions(&json!({ "gone": [{ "a": A, "s": scene }] })).unwrap();
        assert!(app.truths.fresh(A, now).is_none());
        let too_many: Vec<Value> = (0..=MAX_POSITIONS).map(|_| json!({})).collect();
        assert!(app.positions(&json!({ "positions": too_many })).is_err());
    }

    /// Gap 9: outside text cannot forge or split a log line.
    #[test]
    fn log_safe_escapes_and_truncates() {
        assert_eq!(log_safe("a\nb\r\"c\u{1b}"), "a\\nb\\r\\\"c\\u{1b}");
        let long = "x".repeat(500);
        let s = log_safe(&long);
        assert!(s.chars().count() <= 201 && s.ends_with('…'));
    }
}
