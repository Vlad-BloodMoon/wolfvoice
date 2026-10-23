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

//! Optional: a Discord voice bridge's way into a Wolf Chat voice room.
//!
//! OFF unless a bridge key is installed (WOLFVOICE_BRIDGE_KEY or /etc/wolfvoice/bridge_key); without one the
//! bridge port is never opened. It needs a bridge client (on the Wolf Territories Grid, the wolfbridge service, which
//! is not part of this repository) and the Wolf Chat rooms of the region module. The bridge client sits in the Discord voice channel
//! and connects here as ONE participant of the linked Wolf Chat room ("wchan-<group>-<channel>", the room name the
//! region hands out in WolfConference.cs HandleWolfChannelCall). It is a session like a viewer's in every way the
//! mixer cares about (src/room.rs), with no WebRTC: its audio travels over this TLS connection instead.
//!
//!   * it HEARS the room: the mix for its session (every speaker but itself - room.rs mix_room_with never echoes a
//!     listener back to themselves) is sent to it as mono Opus, only while there is sound;
//!   * it SPEAKS in the room: the Discord channel's mix arrives as mono Opus, is decoded and pushed as its microphone;
//!   * viewers see it in the room's participant list (under the bridge's own agent id, from its HELLO)
//!     with a speaking dot that follows Discord's voices; Wolf Chat shows the Discord names beside it.
//!
//! Only Wolf Chat rooms can be joined this way - never local (spatial) voice and never a private call.
//!
//! Wire format, both directions: frames of [type u8][length u16 big-endian][payload]. The client's first frame is
//! HELLO, a JSON object {v, room, agent, time, nonce, sig} with
//!     sig = hex HMAC-SHA256(bridge key, "wolfvoice-bridge1\n" time "\n" nonce "\n" room "\n" agent)
//! signed with the key in /etc/wolfvoice/bridge_key (a separate secret from the region relay key, so neither opens
//! the other's door). Answered WELCOME {session} or ERROR {error} (then closed). After that: AUDIO (one 20 ms mono
//! Opus packet at 48 kHz), ROSTER (server -> client, {"people": [agent ids in the room besides the bridge]}, sent when
//! it changes) and PING (either way, every PING_EVERY; a side that hears nothing for IDLE_LIMIT hangs up).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::mixer::{FRAME_SAMPLES, SAMPLE_RATE};
use crate::room::{RoomKey, Session};

/// Where the bridge listener binds (env WOLFVOICE_BRIDGE_BIND overrides). 9443 is the relay's JSON-RPC. Firewall it
/// to the bridge client's host only.
pub const BRIDGE_BIND: &str = "0.0.0.0:9445";
pub const BRIDGE_BIND_ENV: &str = "WOLFVOICE_BRIDGE_BIND";
/// The bridge key (env first, then the file). With neither the listener does not start at all.
pub const BRIDGE_KEY_ENV: &str = "WOLFVOICE_BRIDGE_KEY";
pub const BRIDGE_KEY_FILE: &str = "/etc/wolfvoice/bridge_key";
/// A key shorter than this is refused (openssl rand -hex 32 makes 64).
pub const MIN_KEY_LEN: usize = 32;

pub const T_HELLO: u8 = 1;
pub const T_WELCOME: u8 = 2;
pub const T_AUDIO: u8 = 3;
pub const T_ROSTER: u8 = 4;
pub const T_PING: u8 = 5;
pub const T_ERROR: u8 = 15;

/// The largest frame either side accepts. An Opus packet of 20 ms is at most 1275 bytes (RFC 6716 §3.2.1); a HELLO
/// or ROSTER is small JSON (a roster names at most MAX_SESSIONS agents of 36 characters - larger rooms are cut).
pub const MAX_FRAME: usize = 16 * 1024;
/// How far the HELLO's time may be from ours, and how long a nonce is remembered (covers the window both ways).
const SKEW_SECS: u64 = 60;
const NONCE_KEEP: Duration = Duration::from_secs(2 * SKEW_SECS + 10);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
pub const PING_EVERY: Duration = Duration::from_secs(10);
pub const IDLE_LIMIT: Duration = Duration::from_secs(30);
/// Bridges at once: one per linked Wolf Chat room with somebody on either side.
const MAX_BRIDGES: usize = 64;
/// [HARDENING 2026-10-23] Connections at once that have not yet passed HELLO. The port is
/// firewalled to the bridge client's host, but nothing else stopped anything that reached it from
/// holding a TLS handshake + HELLO_TIMEOUT per connection without limit. A bridge client
/// holds one connection per linked room, opened one at a time.
const MAX_UNAUTHENTICATED: usize = 16;
/// Frames queued towards the bridge before new ones are dropped (a stalled connection must never stall the mixer).
const OUT_QUEUE: usize = 50;
/// After the room's mix goes silent this many more frames are still sent, so the ends of words are not cut.
const HANGOVER_FRAMES: u32 = 10;
/// Below this peak the room's mix counts as silence (-60 dBFS).
const SILENCE_PEAK: f32 = 0.001;
/// The most room members named in one ROSTER.
const MAX_ROSTER: usize = 200;

/// The HELLO signature (see the module comment).
pub fn hello_sig(key: &str, time: u64, nonce: &str, room: &str, agent: &str) -> String {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
    let tag = ring::hmac::sign(&k, format!("wolfvoice-bridge1\n{time}\n{nonce}\n{room}\n{agent}").as_bytes());
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// A lowercase hyphenated uuid (OpenMetaverse UUID.ToString(), the form the region writes room names in).
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

/// Only a Wolf Chat voice room: "wchan-" + group uuid + "-" + channel uuid (WolfConference.cs HandleWolfChannelCall).
pub fn valid_room(room: &str) -> bool {
    let Some(rest) = room.strip_prefix("wchan-") else { return false };
    rest.len() == 73 && rest.as_bytes()[36] == b'-' && is_uuid(&rest[..36]) && is_uuid(&rest[37..])
}

/// One frame onto the wire.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    if payload.len() > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"));
    }
    let mut buf = Vec::with_capacity(3 + payload.len());
    buf.push(kind);
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf).await
}

/// One frame off the wire.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 3];
    r.read_exact(&mut head).await?;
    let len = u16::from_be_bytes([head[1], head[2]]) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok((head[0], payload))
}

/// The stereo room mix as one mono channel. Wolf Chat rooms are not positional, so every speaker sits in the centre
/// at FRAC_1_SQRT_2 on both sides (room.rs mix_room_with); (L + R) / sqrt 2 gives each voice back at its own level.
pub fn downmix(stereo: &[f32]) -> Vec<f32> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|lr| ((lr[0] + lr[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0))
        .collect()
}

/// One bridge in a room: its session (in the registry like a viewer's) and the way to its connection.
pub struct Link {
    pub session: Arc<Session>,
    tx: mpsc::Sender<(u8, Vec<u8>)>,
    encoder: Mutex<opus::Encoder>,
    /// Frames still to send after the mix went silent (HANGOVER_FRAMES).
    hangover: AtomicU32,
    /// Wakes the connection's writer: the room's people changed, or this bridge was dropped.
    changed: tokio::sync::Notify,
}

impl Link {
    /// The room's mix for this tick (minus the bridge itself): encoded and queued while there is sound. Never waits -
    /// a full queue drops the frame (the mixer must not be held up by one slow connection). [2026-10-22, scale] In
    /// silence this is one scan of the frame and nothing else: no allocation, no encode, nothing sent.
    pub fn send_mix(&self, stereo: &[f32]) {
        let loud = stereo.iter().any(|s| s.abs() >= SILENCE_PEAK);
        if loud {
            self.hangover.store(HANGOVER_FRAMES, Ordering::Relaxed);
        } else if self.hangover.load(Ordering::Relaxed) == 0 {
            return;
        } else {
            self.hangover.fetch_sub(1, Ordering::Relaxed);
        }
        let mono = downmix(stereo);
        let mut buf = vec![0u8; 1500];
        let n = match self.encoder.lock().encode_float(&mono, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                log::debug!("bridge {} opus encode: {e}", self.session.id);
                return;
            }
        };
        buf.truncate(n);
        let _ = self.tx.try_send((T_AUDIO, buf));
    }
}

/// Every bridge connected now, by session id, and the HELLO nonces seen lately.
#[derive(Default)]
pub struct Bridges {
    links: RwLock<HashMap<String, Arc<Link>>>,
    nonces: Mutex<HashMap<String, Instant>>,
}

impl Bridges {
    pub fn get(&self, session_id: &str) -> Option<Arc<Link>> {
        self.links.read().get(session_id).cloned()
    }

    pub fn count(&self) -> usize {
        self.links.read().len()
    }

    /// Called from App::drop_session: the session is gone, so is its link (its connection is woken to end).
    pub fn forget(&self, session_id: &str) {
        if let Some(l) = self.links.write().remove(session_id) {
            l.changed.notify_one();
        }
    }

    /// Called from App wherever a room's people change (provision, drop_session, move_session): a bridge in that room
    /// sends its new ROSTER. Event-driven, so an idle bridge costs nothing; only Wolf Chat rooms can hold one.
    pub fn room_changed(&self, room: &RoomKey) {
        if !matches!(room, RoomKey::MultiAgent { .. }) {
            return;
        }
        for l in self.links.read().values() {
            if l.session.room() == *room {
                l.changed.notify_one();
            }
        }
    }

    /// The session ids of bridges already in `room`.
    fn in_room(&self, room: &RoomKey) -> Vec<String> {
        self.links.read().values().filter(|l| l.session.room() == *room).map(|l| l.session.id.clone()).collect()
    }

    /// A nonce is good once within NONCE_KEEP.
    fn nonce_once(&self, nonce: &str) -> bool {
        let mut seen = self.nonces.lock();
        seen.retain(|_, t| t.elapsed() < NONCE_KEEP);
        if seen.contains_key(nonce) {
            return false;
        }
        seen.insert(nonce.to_string(), Instant::now());
        true
    }
}

/// A HELLO checked: the room and the bridge's agent id.
#[derive(Debug, PartialEq)]
pub struct Hello {
    pub room: String,
    pub agent: String,
}

/// Check a HELLO against the key, the clock and the nonces seen. The error is what the client is told.
pub fn check_hello(key: &str, payload: &[u8], now: u64, bridges: &Bridges) -> Result<Hello, String> {
    let v: Value = serde_json::from_slice(payload).map_err(|_| "hello is not JSON".to_string())?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    if v.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("unknown bridge protocol version".into());
    }
    let (room, agent, nonce, sig) = (s("room"), s("agent"), s("nonce"), s("sig"));
    let time = v.get("time").and_then(Value::as_u64).unwrap_or(0);
    if nonce.len() != 32 || !nonce.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("bad nonce".into());
    }
    if !crate::same_secret(&hello_sig(key, time, &nonce, &room, &agent), &sig.to_ascii_lowercase()) {
        return Err("not authorised".into());
    }
    if time.abs_diff(now) > SKEW_SECS {
        return Err("the bridge clock is too far out".into());
    }
    if !valid_room(&room) {
        return Err("only Wolf Chat voice rooms can be bridged".into());
    }
    if !is_uuid(&agent) {
        return Err("bad agent id".into());
    }
    if !bridges.nonce_once(&nonce.to_ascii_lowercase()) {
        return Err("not authorised (hello replayed)".into());
    }
    Ok(Hello { room, agent })
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The agents in the room besides the bridge itself, sorted (what a ROSTER says).
fn people(app: &crate::App, room: &RoomKey, me: &str) -> Vec<String> {
    let mut v: Vec<String> = app
        .sessions
        .members(room)
        .iter()
        .filter(|m| m.id != me && app.bridges.get(&m.id).is_none())
        .map(|m| m.agent_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    v.sort();
    v.truncate(MAX_ROSTER);
    v
}

/// Accept bridge connections for ever (TLS, the same certificate as the JSON-RPC port).
pub async fn serve(app: Arc<crate::App>, acceptor: tokio_rustls::TlsAcceptor, addr: std::net::SocketAddr, key: Arc<String>) {
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("Discord voice bridge listener not started on {addr}: {e}");
            return;
        }
    };
    log::info!("Discord voice bridge listening on {addr} (TLS)");
    let unauthenticated = Arc::new(tokio::sync::Semaphore::new(MAX_UNAUTHENTICATED));
    let mut refused: u64 = 0;
    let mut last_refusal_log = Instant::now() - Duration::from_secs(60);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("bridge accept: {e}");
                continue;
            }
        };
        // [HARDENING 2026-10-23] A place among the not-yet-authenticated, or the connection is
        // closed at once (logged at most once a minute, with a count).
        let Ok(permit) = unauthenticated.clone().try_acquire_owned() else {
            refused += 1;
            if last_refusal_log.elapsed() >= Duration::from_secs(60) {
                log::warn!("bridge port: {refused} connection(s) refused, {MAX_UNAUTHENTICATED} already waiting for HELLO");
                refused = 0;
                last_refusal_log = Instant::now();
            }
            continue;
        };
        let _ = stream.set_nodelay(true);
        let (app, acceptor, key) = (app.clone(), acceptor.clone(), key.clone());
        tokio::spawn(async move {
            let tls = match tokio::time::timeout(HELLO_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    log::debug!("bridge tls handshake from {peer}: {e}");
                    return;
                }
                Err(_) => return,
            };
            handle(app, tls, &key, &peer.to_string(), Some(permit)).await;
        });
    }
}

/// One bridge connection, start to end (any byte stream: TLS in service, an in-memory pipe in the tests).
/// `unauthenticated` is the connection's place among those waiting for HELLO; it is given up
/// as soon as the HELLO has been read and checked.
pub async fn handle<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    app: Arc<crate::App>,
    stream: S,
    key: &str,
    peer: &str,
    unauthenticated: Option<tokio::sync::OwnedSemaphorePermit>,
) {
    let (mut rd, mut wr) = tokio::io::split(stream);
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut rd)).await {
        Ok(Ok((T_HELLO, p))) => check_hello(key, &p, now_secs(), &app.bridges),
        Ok(Ok(_)) => Err("expected hello".to_string()),
        Ok(Err(_)) | Err(_) => return,
    };
    drop(unauthenticated);
    let hello = match hello {
        Ok(h) => h,
        Err(e) => {
            log::warn!("bridge from {peer} refused: {e}");
            let _ = write_frame(&mut wr, T_ERROR, json!({ "error": e }).to_string().as_bytes()).await;
            return;
        }
    };
    let room = RoomKey::MultiAgent { channel: hello.room.clone() };
    // One bridge per room: a reconnect replaces the connection it lost (that one may not have noticed yet).
    for old in app.bridges.in_room(&room) {
        log::info!("bridge session {old} in {} replaced by a new connection", hello.room);
        app.drop_session(&old).await;
    }
    if app.bridges.count() >= MAX_BRIDGES {
        log::warn!("bridge from {peer} refused: {MAX_BRIDGES} bridges already");
        let _ = write_frame(&mut wr, T_ERROR, json!({ "error": "too many bridges" }).to_string().as_bytes()).await;
        return;
    }
    // 32 kbit/s mono speech (the bridge client's bitrate): about 4 KB/s while the room talks, nothing in silence.
    let encoder = match opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
        .and_then(|mut e| e.set_bitrate(opus::Bitrate::Bits(32_000)).map(|_| e))
    {
        Ok(e) => e,
        Err(e) => {
            log::error!("bridge opus encoder: {e}");
            return;
        }
    };
    // [HARDENING 2026-10-23] A place under the service-wide session ceiling, claimed race-free
    // (src/admission.rs). Every bridge has the same agent id, so the per-agent ceiling does not
    // apply (MAX_BRIDGES is its limit), nor a room's (one bridge per room).
    let reservation = match app.admission.reserve(&app.sessions, &hello.agent, &room, None, crate::MAX_SESSIONS, usize::MAX) {
        Ok(r) => r,
        Err(why) => {
            log::warn!("bridge from {peer} refused: {}", why.message());
            let _ = write_frame(&mut wr, T_ERROR, json!({ "error": why.message() }).to_string().as_bytes()).await;
            return;
        }
    };
    let id = uuid::Uuid::new_v4().to_string();
    let session = Arc::new(Session::new(id.clone(), hello.agent.clone(), room.clone(), false));
    // No data channel and no handshake: it is listening and present from the start. A join with "p":true, as a
    // viewer's own region connection announces itself (room.rs build_roster announces only joined speakers).
    session.set_ready(true);
    session.primary.store(true, Ordering::Relaxed);
    session.joined.store(true, Ordering::Release);
    let (tx, mut rx) = mpsc::channel::<(u8, Vec<u8>)>(OUT_QUEUE);
    let link = Arc::new(Link {
        session: session.clone(),
        tx: tx.clone(),
        encoder: Mutex::new(encoder),
        hangover: AtomicU32::new(0),
        changed: tokio::sync::Notify::new(),
    });
    app.bridges.links.write().insert(id.clone(), link.clone());
    app.sessions.insert(session.clone());
    drop(reservation);
    app.bridges.room_changed(&room);
    log::info!("bridge session {id} agent {} joined {} from {peer} ({} bridges)", hello.agent, hello.room, app.bridges.count());
    if write_frame(&mut wr, T_WELCOME, json!({ "session": id }).to_string().as_bytes()).await.is_err() {
        app.drop_session(&id).await;
        return;
    }

    // Reading: the Discord mix, decoded into the session's microphone frames (as session.rs on_track does for a
    // viewer). Its own task, because read_frame is not safe to cancel half-way through a frame.
    let last_rx = Arc::new(Mutex::new(Instant::now()));
    let reader = {
        let (session, last_rx) = (session.clone(), last_rx.clone());
        tokio::spawn(async move {
            let mut decoder = match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("bridge opus decoder: {e}");
                    return;
                }
            };
            // 120 ms at 48 kHz, the longest Opus packet (session.rs MAX_DECODE_SAMPLES).
            let mut pcm = vec![0f32; 5760];
            while let Ok((kind, payload)) = read_frame(&mut rd).await {
                *last_rx.lock() = Instant::now();
                if kind != T_AUDIO || payload.is_empty() {
                    continue;
                }
                match decoder.decode_float(&payload, &mut pcm, false) {
                    Ok(n) => {
                        let mut off = 0;
                        while off < n {
                            let end = (off + FRAME_SAMPLES).min(n);
                            let mut f = pcm[off..end].to_vec();
                            f.resize(FRAME_SAMPLES, 0.0);
                            session.push_frame(f);
                            off = end;
                        }
                    }
                    Err(e) => log::debug!("bridge {} opus decode: {e}", session.id),
                }
            }
        })
    };

    // Writing: the room's mix (queued by the mixer through Link::send_mix), the roster when the room's people change
    // (Link::changed, woken by App - no polling), and a ping every PING_EVERY, which is also when a dead peer is noticed.
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut told: Option<Vec<String>> = None;
    let reader_done = reader;
    tokio::pin!(reader_done);
    let mut send_roster = true;
    let why = loop {
        if app.bridges.get(&id).is_none() {
            break "replaced";
        }
        if send_roster {
            send_roster = false;
            let now = people(&app, &room, &id);
            if told.as_ref() != Some(&now) {
                if write_frame(&mut wr, T_ROSTER, json!({ "people": now }).to_string().as_bytes()).await.is_err() {
                    break "write failed";
                }
                told = Some(now);
            }
        }
        tokio::select! {
            m = rx.recv() => {
                let Some((kind, payload)) = m else { break "queue closed" };
                if write_frame(&mut wr, kind, &payload).await.is_err() {
                    break "write failed";
                }
            }
            _ = link.changed.notified() => send_roster = true,
            _ = &mut reader_done => break "closed by the bridge",
            _ = ping.tick() => {
                if last_rx.lock().elapsed() > IDLE_LIMIT {
                    break "silent for too long";
                }
                if write_frame(&mut wr, T_PING, &[]).await.is_err() {
                    break "write failed";
                }
            }
        }
    };
    reader_done.abort();
    let _ = wr.shutdown().await;
    if app.bridges.get(&id).is_some() {
        app.drop_session(&id).await;
    }
    log::info!("bridge session {id} in {} ended: {why}", hello.room);
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef-test";
    const ROOM: &str = "wchan-6f2b1c3e-0000-4000-8000-00000000abcd-11111111-2222-3333-4444-555555555555";
    const AGENT: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

    fn hello_json(key: &str, time: u64, nonce: &str, room: &str) -> Vec<u8> {
        json!({"v": 1, "room": room, "agent": AGENT, "time": time, "nonce": nonce,
               "sig": hello_sig(key, time, nonce, room, AGENT)})
        .to_string()
        .into_bytes()
    }

    // Expected value from Python's hmac module (an independent HMAC-SHA256), 2026-10-22:
    // hmac.new(KEY, b"wolfvoice-bridge1\n1761100000\n" + b"a"*32 + b"\n" + ROOM + b"\n" + AGENT, sha256).hexdigest()
    #[test]
    fn signature_matches_independent_hmac() {
        assert_eq!(
            hello_sig(KEY, 1761100000, &"a".repeat(32), ROOM, AGENT),
            "424823f1491ef623d3c1580c3550aea6f69beadd4fe0434c05a590983fde9fcf"
        );
    }

    #[test]
    fn only_wolf_chat_rooms() {
        assert!(valid_room(ROOM));
        assert!(!valid_room("wchan-6F2B1C3E-0000-4000-8000-00000000ABCD-11111111-2222-3333-4444-555555555555"));
        assert!(!valid_room("6f2b1c3e-0000-4000-8000-00000000abcd"), "a private call room");
        assert!(!valid_room("wchan-6f2b1c3e-0000-4000-8000-00000000abcd"));
        assert!(!valid_room(&format!("{ROOM}x")));
    }

    #[test]
    fn hello_is_checked() {
        let b = Bridges::default();
        let now = 1_761_100_000;
        let n1 = "0123456789abcdef0123456789abcdef";
        assert_eq!(check_hello(KEY, &hello_json(KEY, now, n1, ROOM), now, &b), Ok(Hello { room: ROOM.into(), agent: AGENT.into() }));
        assert!(check_hello(KEY, &hello_json(KEY, now, n1, ROOM), now, &b).is_err(), "replayed");
        let n2 = "fedcba9876543210fedcba9876543210";
        assert!(check_hello(KEY, &hello_json("another-key-another-key-another-key", now, n2, ROOM), now, &b).is_err(), "wrong key");
        assert!(check_hello(KEY, &hello_json(KEY, now - 120, n2, ROOM), now, &b).is_err(), "too old");
        let call = "6f2b1c3e-0000-4000-8000-00000000abcd";
        assert!(check_hello(KEY, &hello_json(KEY, now, n2, call), now, &b).is_err(), "not a Wolf Chat room");
        assert!(check_hello(KEY, b"{}", now, &b).is_err());
        assert!(check_hello(KEY, b"not json", now, &b).is_err());
    }

    #[test]
    fn centred_voices_keep_their_level_in_mono() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let m = downmix(&[0.5 * s, 0.5 * s, -0.25 * s, -0.25 * s]);
        assert!((m[0] - 0.5).abs() < 1e-6 && (m[1] + 0.25).abs() < 1e-6);
    }

    fn test_app() -> Arc<crate::App> {
        Arc::new(crate::App {
            rpc_key: None,
            cred_key: None,
            legacy_creds: true,
            admission: crate::admission::Admission::default(),
            truths: crate::truth::Truths::default(),
            ice: crate::ice::IceFilter::default(),
            sessions: crate::room::Registry::default(),
            endpoints: RwLock::new(HashMap::new()),
            ports: crate::session::PortPool::new(41000, 41999),
            public_ip: "127.0.0.1".into(),
            runtime: Arc::new(webrtc::runtime::TokioRuntime),
            npcs: crate::npc::NpcVoices::default(),
            npc_hosts: Vec::new(),
            bridges: Bridges::default(),
        })
    }

    fn rms(f: &[f32]) -> f32 {
        (f.iter().map(|s| s * s).sum::<f32>() / f.len().max(1) as f32).sqrt()
    }

    fn tone(phase: usize) -> Vec<f32> {
        (0..FRAME_SAMPLES).map(|i| 0.3 * ((phase * FRAME_SAMPLES + i) as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin()).collect()
    }

    /// The next frame of `kind`, skipping the others (rosters and pings arrive whenever they are due).
    async fn next_of<R: AsyncRead + Unpin>(r: &mut R, kind: u8) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (k, p) = read_frame(r).await.unwrap();
                if k == kind {
                    return p;
                }
            }
        })
        .await
        .expect("frame in time")
    }

    async fn connect(app: &Arc<crate::App>, nonce: &str) -> (tokio::io::DuplexStream, String) {
        let (mut client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(handle(app.clone(), server, KEY, "test", None));
        write_frame(&mut client, T_HELLO, &hello_json(KEY, now_secs(), nonce, ROOM)).await.unwrap();
        let w: Value = serde_json::from_slice(&next_of(&mut client, T_WELCOME).await).unwrap();
        (client, w["session"].as_str().unwrap().to_string())
    }

    /// The loopback test: a bridge joins a room, is announced in its roster, speaks into it (Opus in, frames on its
    /// session), hears a viewer's voice (the mixer's output for it, Opus out) but never itself, and is gone when its
    /// connection closes; a second connection for the same room replaces the first.
    #[tokio::test]
    async fn bridge_loopback() {
        let app = test_app();
        let (mut client, id) = connect(&app, "00000000000000000000000000000001").await;
        let room = RoomKey::MultiAgent { channel: ROOM.into() };
        let bridge = app.sessions.get(&id).expect("bridge session registered");
        assert_eq!(bridge.agent_id, AGENT);
        assert!(bridge.ready.load(Ordering::Acquire) && bridge.joined.load(Ordering::Acquire));
        let r: Value = serde_json::from_slice(&next_of(&mut client, T_ROSTER).await).unwrap();
        assert_eq!(r["people"], json!([]));

        // A viewer in the room.
        let viewer = Arc::new(Session::new("v1".into(), "e2afe467-ae22-44e0-be07-fb1e50ae7e6f".into(), room.clone(), false));
        viewer.set_ready(true);
        viewer.joined.store(true, Ordering::Release);
        viewer.primary.store(true, Ordering::Relaxed);
        app.sessions.insert(viewer.clone());
        app.bridges.room_changed(&room); // as App::provision does after inserting
        let r: Value = serde_json::from_slice(&next_of(&mut client, T_ROSTER).await).unwrap();
        assert_eq!(r["people"], json!(["e2afe467-ae22-44e0-be07-fb1e50ae7e6f"]));

        // Discord speaks: Opus frames in become the bridge's microphone frames.
        let mut enc = opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip).unwrap();
        for i in 0..5 {
            let mut buf = vec![0u8; 1500];
            let n = enc.encode_float(&tone(i), &mut buf).unwrap();
            write_frame(&mut client, T_AUDIO, &buf[..n]).await.unwrap();
        }
        let mut heard = Vec::new();
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            while let Some(f) = bridge.take_frame() {
                heard.push(f);
            }
            if heard.len() >= 5 {
                break;
            }
        }
        assert_eq!(heard.len(), 5, "one 20 ms frame per packet");
        assert!(heard.iter().any(|f| rms(f) > 0.05), "the tone comes through");

        // The viewer is in the bridge's mix; the bridge never hears itself.
        for f in &heard {
            bridge.push_frame(f.clone());
        }
        let members = app.sessions.members(&room);
        let outs = crate::room::mix_room_with(&members, &[], &app.truths);
        let for_bridge = outs.iter().find(|o| o.session.id == id).unwrap();
        assert!(for_bridge.stereo.iter().all(|s| *s == 0.0), "the bridge's own voice is not sent back to Discord");
        let for_viewer = outs.iter().find(|o| o.session.id == "v1").unwrap();
        assert!(rms(&for_viewer.stereo) > 0.0, "the viewer hears Discord");
        let link = app.bridges.get(&id).unwrap();
        link.send_mix(&for_bridge.stereo);   // silence: nothing is sent
        let mut dec = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono).unwrap();
        let mut pcm = vec![0f32; 5760];
        let mut loudest = 0f32;
        for i in 0..5 {
            viewer.push_frame(tone(i));
            let outs = crate::room::mix_room_with(&app.sessions.members(&room), &[], &app.truths);
            link.send_mix(&outs.iter().find(|o| o.session.id == id).unwrap().stereo);
            let p = next_of(&mut client, T_AUDIO).await;
            let n = dec.decode_float(&p, &mut pcm, false).unwrap();
            assert_eq!(n, FRAME_SAMPLES);
            loudest = loudest.max(rms(&pcm[..n]));
        }
        assert!(loudest > 0.05, "Discord hears the viewer (rms {loudest})");

        // A reconnect replaces the old connection; closing the new one removes the bridge from the room.
        let (client2, id2) = connect(&app, "00000000000000000000000000000002").await;
        assert_ne!(id, id2);
        assert!(app.sessions.get(&id).is_none() && app.bridges.get(&id).is_none(), "the old bridge session is gone");
        assert!(app.sessions.get(&id2).is_some());
        drop(client);
        drop(client2);
        for _ in 0..100 {
            if app.sessions.get(&id2).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(app.sessions.get(&id2).is_none() && app.bridges.count() == 0, "a closed bridge leaves the room");
        assert!(app.sessions.get("v1").is_some(), "the viewer stays");
    }

    #[tokio::test]
    async fn refused_hello_is_answered_and_joins_nothing() {
        let app = test_app();
        let (mut client, server) = tokio::io::duplex(65536);
        tokio::spawn(handle(app.clone(), server, KEY, "test", None));
        let bad = hello_json("not-the-key-not-the-key-not-the-key", now_secs(), "00000000000000000000000000000003", ROOM);
        write_frame(&mut client, T_HELLO, &bad).await.unwrap();
        let e: Value = serde_json::from_slice(&next_of(&mut client, T_ERROR).await).unwrap();
        assert_eq!(e["error"], "not authorised");
        assert_eq!(app.sessions.session_count(), 0);
    }

    #[tokio::test]
    async fn frames_round_trip_and_oversize_is_refused() {
        let (mut a, mut b) = tokio::io::duplex(65536);
        write_frame(&mut a, T_AUDIO, &[1, 2, 3]).await.unwrap();
        write_frame(&mut a, T_PING, &[]).await.unwrap();
        assert_eq!(read_frame(&mut b).await.unwrap(), (T_AUDIO, vec![1, 2, 3]));
        assert_eq!(read_frame(&mut b).await.unwrap(), (T_PING, vec![]));
        assert!(write_frame(&mut a, T_AUDIO, &vec![0u8; MAX_FRAME + 1]).await.is_err());
        // A header claiming more than MAX_FRAME is refused before anything is allocated for it.
        let len = (MAX_FRAME as u16 + 1).to_be_bytes();
        a.write_all(&[T_AUDIO, len[0], len[1]]).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
    }
}
