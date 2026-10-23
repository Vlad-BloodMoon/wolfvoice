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

//! [HARDENING 2026-10-23] Stock-Firestorm compatibility check for the hardened wolfvoice, plus the
//! end-to-end checks of the hardening itself. Every synthetic viewer here does exactly what stock
//! Firestorm 7.2 does (indra/newview/llvoicewebrtc.cpp, the absolute reference):
//!
//!   * offers an Opus sendrecv track with the "SLData" data channel created before the offer
//!     (llwebrtc.cpp:1210 CreateDataChannelOrError, :1266 CreateOffer);
//!   * provisions with the body the viewer sends, relayed as the region relays it
//!     (WebRtcVoiceServiceConnector: {request, userID, scene, key}). Spatial: jsep offer,
//!     channel_type "local", voice_server_type "webrtc", parcel_local_id only when valid
//!     (llvoicewebrtc.cpp:2790-2800). Ad-hoc: jsep offer, credentials, channel, channel_type
//!     "multiagent", voice_server_type (llvoicewebrtc.cpp:3393-3403);
//!   * once its data channel is ready sends its join (VOICE_STATE_WAIT_FOR_DATA_CHANNEL -> sendJoin,
//!     llvoicewebrtc.cpp:2983-2986): a spatial connection is built non-primary (:3322) so its first join
//!     is {"j":{}}, followed at SESSION_UP by its position (:2987-2991) and, its own region being the
//!     agent's region, a second join {"j":{"p":true}} (:3008-3019); an ad-hoc connection is primary from
//!     construction (mPrimary(true), :2389) and sends {"j":{"p":true}} and never a position (adhoc
//!     sessionState sendData is empty, llvoicewebrtc.h:384);
//!   * sends positions as "sp"/"sh"/"lp"/"lh", each component (int)(value * 100) of GLOBAL metres
//!     (llvoicewebrtc.cpp:1216-1249, getPositionGlobal :1093);
//!   * reconnects to a call with the credentials it stored when the call started (startAdHocSession
//!     :1483 keeps them; requestVoiceConnection :3400 sends them again every time);
//!   * logs out with {"logout":true,"viewer_session"} (:2731-2735).
//!
//! Each case answers with what a listener DECODES (Opus RMS), not packet counts.
//!
//! Run ON the voice host, preferably against a second (test) instance - WOLFVOICE_BIND,
//! WOLFVOICE_MEDIA_LO/HI let one run beside the live service, e.g. WOLFVOICE_BIND=127.0.0.1:9444 -
//! with that instance's relay key:
//!     export WOLFVOICE_RPC_KEY=...   # the test instance's relay key
//!     ./fs_compat https://127.0.0.1:9444
//! WOLFVOICE_RPC_KEY (and WOLFVOICE_CRED_KEY when the instance has a separate credential key) are read
//! from the environment and never printed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use rtc::interceptor::Registry as InterceptorRegistry;
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_OPUS};
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};

use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    RTCPeerConnectionState,
};
use webrtc::runtime::{Runtime, TokioRuntime};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 960;
/// A decoded frame louder than this counts as heard (the tone is 0.3 peak; Opus silence decodes ~0).
const HEARD_RMS: f32 = 0.02;
const HEAR_WITHIN: Duration = Duration::from_secs(4);
/// How long a listener must stay silent for a "must not hear" case.
const SILENT_FOR: Duration = Duration::from_secs(3);
/// A region at grid 1000,1000 (global metres), as RegionInfo.WorldLocX/Y.
const REGION_ORIGIN: [f64; 3] = [256_000.0, 256_000.0, 0.0];

fn rpc_key() -> String {
    std::env::var("WOLFVOICE_RPC_KEY").unwrap_or_default()
}
/// The key that signs room credentials on the instance under test (the relay key when none).
fn cred_key() -> String {
    std::env::var("WOLFVOICE_CRED_KEY").ok().filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_string()).unwrap_or_else(rpc_key)
}

/// Source: wolfvoice main.rs call_credentials / WebRtcVoiceRegionModule CallCredentials.
fn call_credentials(key: &str, channel: &str, user_id: &str) -> String {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
    let tag = ring::hmac::sign(&k, format!("wolfcall1:{channel}:{user_id}").as_bytes());
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// UUID XOR, as LLUUID's operator^ and GridInstantMessage do for a P2P session (llimview.cpp:2620).
fn xor(a: &str, b: &str) -> String {
    let (a, b) = (uuid::Uuid::parse_str(a).unwrap(), uuid::Uuid::parse_str(b).unwrap());
    let mut o = [0u8; 16];
    for (i, x) in o.iter_mut().enumerate() {
        *x = a.as_bytes()[i] ^ b.as_bytes()[i];
    }
    uuid::Uuid::from_bytes(o).hyphenated().to_string()
}

#[derive(Default)]
struct Observed {
    connected: AtomicBool,
    dc_open: AtomicBool,
    gathered: AtomicBool,
    frames: AtomicU64,
    heard: AtomicU64,
    /// The "p" of the last "j" the server announced per agent (what makes Firestorm create a
    /// participant, llvoicewebrtc.cpp:3181-3208).
    joins: Mutex<HashMap<String, bool>>,
}

struct Handler {
    name: String,
    obs: Arc<Observed>,
    runtime: Arc<dyn Runtime>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.obs.gathered.store(true, Ordering::SeqCst);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if state == RTCPeerConnectionState::Connected {
            self.obs.connected.store(true, Ordering::SeqCst);
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let obs = self.obs.clone();
        let name = self.name.clone();
        self.runtime.spawn(Box::pin(async move {
            let mut dec = match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[{name}] opus decoder: {e}");
                    return;
                }
            };
            let mut pcm = vec![0f32; FRAME_SAMPLES * 2 * 6];
            while let Some(evt) = track.poll().await {
                let TrackRemoteEvent::OnRtpPacket(p) = evt else { continue };
                if p.payload.is_empty() {
                    continue;
                }
                if let Ok(n) = dec.decode_float(&p.payload, &mut pcm, false) {
                    let s = &pcm[..n * 2];
                    let rms = (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt();
                    obs.frames.fetch_add(1, Ordering::SeqCst);
                    if rms > HEARD_RMS {
                        obs.heard.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        }));
    }

    async fn on_data_channel(&self, _dc: Arc<dyn DataChannel>) {}
}

#[derive(Clone)]
enum Kind {
    /// Spatial voice in `scene`; the viewer's own region (so it turns primary).
    Spatial { scene: String },
    /// An ad-hoc room with the credentials the viewer was handed.
    Room { channel: String, credentials: String },
}

#[derive(Clone, Copy, PartialEq)]
enum Joins {
    /// Stock Firestorm.
    Firestorm,
    /// Data channel open, never a join: a hidden listener.
    Never,
}

struct Conn {
    name: String,
    agent: String,
    scene: String,
    pc: Box<dyn PeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    pt: u8,
    obs: Arc<Observed>,
    session: String,
    talking: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

async fn rpc(endpoint: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    // The region checks the certificate (WolfVoiceHttp); this loopback client cannot (127.0.0.1 is
    // not on it), and it is not what is under test.
    let http = reqwest::Client::builder().danger_accept_invalid_certs(true).build()?;
    Ok(http
        .post(endpoint)
        .header("Content-Type", "application/json-rpc")
        .json(&body)
        .send()
        .await?
        .json()
        .await?)
}

/// The region's position feed (WebRtcVoiceRegionModule WolfPositionFeed): global metres.
async fn feed(endpoint: &str, scene: &str, who: &[(String, [f64; 3])]) -> anyhow::Result<serde_json::Value> {
    let positions: Vec<serde_json::Value> = who
        .iter()
        .map(|(a, p)| serde_json::json!({ "a": a, "s": scene, "p": [p[0], p[1], p[2]], "v": [0.0, 0.0, 0.0] }))
        .collect();
    rpc(
        endpoint,
        serde_json::json!({ "jsonrpc": "2.0", "id": "feed", "method": "wolf.positions",
            "params": { "key": rpc_key(), "positions": positions, "gone": [] } }),
    )
    .await
}

/// Feed every 800 ms until stopped, as the region does every second.
fn keep_feeding(endpoint: String, scene: String, who: Vec<(String, [f64; 3])>) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    tokio::spawn(async move {
        while !s.load(Ordering::SeqCst) {
            if let Err(e) = feed(&endpoint, &scene, &who).await {
                eprintln!("feed: {e}");
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    });
    stop
}

fn opus_params() -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: SAMPLE_RATE,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 111,
    }
}

fn opus_pt_from_sdp(sdp: &str) -> Option<u8> {
    sdp.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("a=rtpmap:")?;
        let (pt, codec) = rest.split_once(' ')?;
        codec.to_ascii_lowercase().starts_with("opus/").then(|| pt.parse().ok())?
    })
}

/// The FS position message (llvoicewebrtc.cpp:1224-1251).
fn position_json(avatar: [f64; 3], ear: [f64; 3]) -> String {
    let c = |v: f64| (v * 100.0) as i64;
    serde_json::json!({
        "sp": { "x": c(avatar[0]), "y": c(avatar[1]), "z": c(avatar[2]) },
        "sh": { "x": 0, "y": 0, "z": 0, "w": 100 },
        "lp": { "x": c(ear[0]), "y": c(ear[1]), "z": c(ear[2]) },
        "lh": { "x": 0, "y": 0, "z": 0, "w": 100 }
    })
    .to_string()
}

/// Provision one connection; `pos` is where a spatial viewer says it is (avatar and ear).
async fn connect(
    name: &str,
    agent: &str,
    kind: Kind,
    joins: Joins,
    pos: [f64; 3],
    endpoint: &str,
    runtime: Arc<dyn Runtime>,
) -> anyhow::Result<Conn> {
    let codec = opus_params();
    let mut me = MediaEngine::default();
    me.register_codec(codec.clone(), RtpCodecKind::Audio)?;
    let registry = register_default_interceptors(InterceptorRegistry::new(), &mut me)?;
    let obs = Arc::new(Observed::default());
    let handler = Arc::new(Handler { name: name.to_string(), obs: obs.clone(), runtime: runtime.clone() });
    let ssrc: u32 = (uuid::Uuid::new_v4().as_u128() as u32) | 1;
    let track = Arc::new(TrackLocalStaticSample::new(MediaStreamTrack::new(
        "SLStream".to_owned(),
        format!("SLAudio-{name}"),
        "SLAudio".to_owned(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
            codec: codec.rtp_codec.clone(),
            ..Default::default()
        }],
    ))?);
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_media_engine(me)
        .with_interceptor_registry(registry)
        .with_handler(handler)
        .with_runtime(runtime.clone())
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .build()
        .await?;
    let local: Arc<dyn TrackLocal> = track.clone();
    pc.add_track(local).await?;
    // llwebrtc.cpp:1210 - the data channel exists before the offer is made.
    let dc = pc.create_data_channel("SLData", None).await?;
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer).await?;
    for _ in 0..100 {
        if obs.gathered.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let sdp = pc.local_description().await.ok_or_else(|| anyhow::anyhow!("no local description"))?.sdp;

    let (scene, request) = match &kind {
        Kind::Spatial { scene } => (
            scene.clone(),
            // Estate voice: no parcel_local_id (INVALID_PARCEL_ID is not sent, :2795-2798).
            serde_json::json!({ "jsep": { "type": "offer", "sdp": sdp }, "channel_type": "local", "voice_server_type": "webrtc" }),
        ),
        Kind::Room { channel, credentials } => (
            "00000000-0000-4000-8000-00000000f5c0".to_string(),
            serde_json::json!({ "jsep": { "type": "offer", "sdp": sdp }, "credentials": credentials,
                                "channel": channel, "channel_type": "multiagent", "voice_server_type": "webrtc" }),
        ),
    };
    let resp = rpc(
        endpoint,
        serde_json::json!({ "jsonrpc": "2.0", "id": format!("fs-{name}"), "method": "provision_voice_account_request",
            "params": { "request": request, "userID": agent, "scene": scene, "key": rpc_key() } }),
    )
    .await?;
    if let Some(err) = resp.get("error") {
        anyhow::bail!("[{name}] provision refused: {err}");
    }
    let result = resp.get("result").ok_or_else(|| anyhow::anyhow!("no result"))?;
    // OnVoiceConnectionRequestSuccess (:2847-2855) needs viewer_session and an answer jsep.
    let session = result["viewer_session"].as_str().unwrap_or_default().to_string();
    let answer = result["jsep"]["sdp"].as_str().unwrap_or_default().to_string();
    if session.len() != 36 || result["jsep"]["type"] != "answer" || answer.is_empty() {
        anyhow::bail!("[{name}] answer Firestorm would reject: {result}");
    }
    let pt = opus_pt_from_sdp(&answer).unwrap_or(111);
    pc.set_remote_description(RTCSessionDescription::answer(answer)?).await?;

    // Data channel: the Firestorm join sequence, then record the roster's joins.
    {
        let obs = obs.clone();
        let dc2 = dc.clone();
        let spatial = matches!(kind, Kind::Spatial { .. });
        runtime.spawn(Box::pin(async move {
            while let Some(evt) = dc2.poll().await {
                match evt {
                    DataChannelEvent::OnOpen => {
                        obs.dc_open.store(true, Ordering::SeqCst);
                        if spatial {
                            if joins == Joins::Firestorm {
                                let _ = dc2.send_text(r#"{"j":{}}"#).await;
                            }
                            let _ = dc2.send_text(&position_json(pos, pos)).await;
                            if joins == Joins::Firestorm {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                let _ = dc2.send_text(r#"{"j":{"p":true}}"#).await;
                            }
                        } else if joins == Joins::Firestorm {
                            let _ = dc2.send_text(r#"{"j":{"p":true}}"#).await;
                        }
                    }
                    DataChannelEvent::OnMessage(m) => {
                        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&m.data) else { continue };
                        let Some(map) = v.as_object() else { continue };
                        for (id, e) in map {
                            if let Some(j) = e.get("j").filter(|j| j.is_object()) {
                                let p = j.get("p").and_then(|p| p.as_bool()).unwrap_or(false);
                                obs.joins.lock().insert(id.clone(), p);
                            }
                        }
                    }
                    DataChannelEvent::OnClose => break,
                    _ => {}
                }
            }
        }));
    }

    let conn = Conn {
        name: name.to_string(),
        agent: agent.to_string(),
        scene,
        pc: Box::new(pc),
        track,
        ssrc,
        pt,
        obs,
        session,
        talking: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
    };
    conn.start_mic();
    Ok(conn)
}

impl Conn {
    fn start_mic(&self) {
        let track = self.track.clone();
        let (ssrc, pt) = (self.ssrc, self.pt);
        let talking = self.talking.clone();
        let stop = self.stop.clone();
        let freq = 440.0 + (self.name.len() as f32) * 55.0;
        tokio::spawn(async move {
            let Ok(mut enc) = opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip) else { return };
            let step = 2.0 * std::f32::consts::PI * freq / SAMPLE_RATE as f32;
            let mut phase = 0f32;
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            while !stop.load(Ordering::SeqCst) {
                tick.tick().await;
                let on = talking.load(Ordering::SeqCst);
                let mut pcm = vec![0f32; FRAME_SAMPLES];
                for s in pcm.iter_mut() {
                    if on {
                        *s = phase.sin() * 0.3;
                    }
                    phase += step;
                }
                let mut out = vec![0u8; 4000];
                let Ok(n) = enc.encode_float(&pcm, &mut out) else { continue };
                out.truncate(n);
                let sample = Sample {
                    data: bytes::Bytes::from(out),
                    timestamp: rtc::shared::time::SystemInstant::now(),
                    duration: Duration::from_millis(20),
                    ..Default::default()
                };
                let _ = track.sample_writer(ssrc, pt).write_sample(&sample).await;
            }
        });
    }

    fn up(&self) -> bool {
        self.obs.connected.load(Ordering::SeqCst) && self.obs.dc_open.load(Ordering::SeqCst)
    }

    async fn logout(&self, endpoint: &str) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = rpc(
            endpoint,
            serde_json::json!({ "jsonrpc": "2.0", "id": format!("fs-{}-logout", self.name), "method": "provision_voice_account_request",
                "params": { "request": { "logout": true, "viewer_session": self.session, "voice_server_type": "webrtc" },
                            "userID": self.agent, "scene": self.scene, "key": rpc_key() } }),
        )
        .await;
        let _ = self.pc.close().await;
    }
}

async fn wait_up(conns: &[&Conn], within: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < within {
        if conns.iter().all(|c| c.up()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// `talker` talks; every listener must decode audible audio within HEAR_WITHIN.
async fn hears(talker: &Conn, listeners: &[&Conn]) -> (bool, String) {
    let before: Vec<u64> = listeners.iter().map(|l| l.obs.heard.load(Ordering::SeqCst)).collect();
    talker.talking.store(true, Ordering::SeqCst);
    let t = Instant::now();
    let mut first: Vec<Option<Duration>> = vec![None; listeners.len()];
    while t.elapsed() < HEAR_WITHIN && first.iter().any(|f| f.is_none()) {
        for (i, l) in listeners.iter().enumerate() {
            if first[i].is_none() && l.obs.heard.load(Ordering::SeqCst) > before[i] + 5 {
                first[i] = Some(t.elapsed());
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    talker.talking.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let detail = listeners
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{}<-{}: {}", l.name, talker.name, first[i].map(|d| format!("heard at {} ms", d.as_millis())).unwrap_or_else(|| "SILENT".into())))
        .collect::<Vec<_>>()
        .join("; ");
    (first.iter().all(|f| f.is_some()), detail)
}

/// `talker` talks for SILENT_FOR; no listener may decode anything audible.
async fn silent(talker: &Conn, listeners: &[&Conn]) -> (bool, String) {
    let before: Vec<(u64, u64)> = listeners.iter().map(|l| (l.obs.heard.load(Ordering::SeqCst), l.obs.frames.load(Ordering::SeqCst))).collect();
    talker.talking.store(true, Ordering::SeqCst);
    tokio::time::sleep(SILENT_FOR).await;
    talker.talking.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut ok = true;
    let mut detail = Vec::new();
    for (i, l) in listeners.iter().enumerate() {
        let heard = l.obs.heard.load(Ordering::SeqCst) - before[i].0;
        let frames = l.obs.frames.load(Ordering::SeqCst) - before[i].1;
        ok &= heard == 0;
        detail.push(format!("{}<-{}: {heard} audible of {frames} frames", l.name, talker.name));
    }
    (ok, detail.join("; "))
}

struct Report {
    rows: Vec<bool>,
}

impl Report {
    fn add(&mut self, case: &str, ok: bool, detail: String) {
        println!("{} {case}: {detail}", if ok { "PASS" } else { "FAIL" });
        self.rows.push(ok);
    }
}

fn agent(n: u8, run: u32) -> String {
    format!("f5c0{n:02x}{run:02x}-0000-4000-8000-0000000000{n:02x}")
}

fn at(dx: f64, dy: f64) -> [f64; 3] {
    [REGION_ORIGIN[0] + 128.0 + dx, REGION_ORIGIN[1] + 128.0 + dy, 26.0]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let endpoint = std::env::args().nth(1).unwrap_or_else(|| "https://127.0.0.1:9444".to_string());
    if rpc_key().is_empty() {
        anyhow::bail!("WOLFVOICE_RPC_KEY is not set (source the instance's env file)");
    }
    let rt: Arc<dyn Runtime> = Arc::new(TokioRuntime);
    let run = std::process::id() & 0xff;
    let mut rep = Report { rows: Vec::new() };
    let up = Duration::from_secs(10);
    let scene = format!("f5c0{run:04x}-0000-4000-8000-000000000001");

    // ---------------------------------------------------------------- spatial (estate voice)
    {
        let (aa, ab, ah, al) = (agent(1, run), agent(2, run), agent(3, run), agent(4, run));
        let sp = |s: &str| Kind::Spatial { scene: s.to_string() };
        let a = connect("A", &aa, sp(&scene), Joins::Firestorm, at(0.0, 0.0), &endpoint, rt.clone()).await?;
        let b = connect("B", &ab, sp(&scene), Joins::Firestorm, at(5.0, 0.0), &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&a, &b], up).await;
        rep.add("spatial: both Firestorm viewers connect (offer/answer, ICE, DTLS, SLData)", ok, format!("A up={} B up={}", a.up(), b.up()));
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (ok, d) = hears(&a, &[&b]).await;
        rep.add("spatial: B hears A", ok, d);
        let (ok, d) = hears(&b, &[&a]).await;
        rep.add("spatial: A hears B", ok, d);
        let pj = b.obs.joins.lock().get(&aa).copied();
        rep.add("spatial: B was told A joined as primary (Firestorm creates the participant)", pj == Some(true), format!("{pj:?}"));

        // Gap 1: a hidden listener (data channel open, never joins).
        let h = connect("Hidden", &ah, sp(&scene), Joins::Never, at(2.0, 0.0), &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&h], up).await;
        rep.add("gap 1: the hidden viewer's connection is up", ok, format!("up={}", h.up()));
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (ok, d) = silent(&a, &[&h]).await;
        rep.add("gap 1: a viewer that never sends its join hears nothing", ok, d);
        let (ok, d) = silent(&h, &[&a, &b]).await;
        rep.add("gap 1: and is heard by nobody", ok, d);
        let told = a.obs.joins.lock().contains_key(&ah);
        rep.add("gap 1: and is in nobody's roster", !told, format!("A told of Hidden: {told}"));
        h.logout(&endpoint).await;

        // Gap 7: the region feeds true positions; a viewer lying about where it stands is held to them.
        let l = connect("Liar", &al, sp(&scene), Joins::Firestorm, at(1.0, 1.0), &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&l], up).await;
        rep.add("gap 7: the liar's connection is up (claims to stand 1 m from A)", ok, format!("up={}", l.up()));
        // Region positions are feet/centre; the viewer bumps its own by 1 m (llvoicewebrtc.cpp:1096).
        let feet = |p: [f64; 3]| [p[0], p[1], p[2] - 1.0];
        let stop = keep_feeding(
            endpoint.clone(),
            scene.clone(),
            vec![(aa.clone(), feet(at(0.0, 0.0))), (ab.clone(), feet(at(5.0, 0.0))), (al.clone(), feet(at(500.0, 0.0)))],
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let want = feed(&endpoint, &scene, &[(aa.clone(), feet(at(0.0, 0.0)))]).await?;
        rep.add("gap 7: wolf.positions answers with the agents in spatial voice", want["result"]["want"] == serde_json::json!([aa]), format!("{}", want["result"]));
        let (ok, d) = silent(&a, &[&l]).await;
        rep.add("gap 7: the liar (really 500 m away) does not hear A", ok, d);
        let (ok, d) = silent(&l, &[&a]).await;
        rep.add("gap 7: nor is it heard by A as if beside them", ok, d);
        let (ok, d) = hears(&a, &[&b]).await;
        rep.add("gap 7: honest B (fed where it says it is) still hears A", ok, d);
        stop.store(true, Ordering::SeqCst);
        l.logout(&endpoint).await;

        // Gap 9: trickled candidates - private/loopback and too many are dropped, the session goes on.
        let mut cands: Vec<serde_json::Value> = (0..40)
            .map(|i| serde_json::json!({ "candidate": format!("candidate:{i} 1 udp 2122260223 192.168.{}.{} 5000{} typ host", i, i + 1, i % 10), "sdpMid": "0", "sdpMLineIndex": 0 }))
            .collect();
        cands.push(serde_json::json!({ "candidate": "candidate:99 1 udp 1 127.0.0.1 9 typ host", "sdpMid": "0", "sdpMLineIndex": 0 }));
        let r = rpc(
            &endpoint,
            serde_json::json!({ "jsonrpc": "2.0", "id": "ice", "method": "voice_signaling_request",
                "params": { "request": { "viewer_session": a.session, "candidates": cands, "voice_server_type": "webrtc" },
                            "userID": aa, "scene": scene, "key": rpc_key() } }),
        )
        .await?;
        rep.add("gap 9: a trickle of 41 private candidates is answered (and not tried)", r.get("result").is_some(), format!("{r}"));
        let (ok, d) = hears(&b, &[&a]).await;
        rep.add("gap 9: the session still works afterwards", ok, d);
        a.logout(&endpoint).await;
        b.logout(&endpoint).await;
    }

    // ---------------------------------------------------------------- private call
    {
        let (aa, ab, w) = (agent(0x11, run), agent(0x12, run), agent(0x13, run));
        let session = xor(&aa, &ab); // llimview.cpp:2620
        let room = |who: &str| Kind::Room { channel: session.clone(), credentials: call_credentials(&cred_key(), &session, who) };
        let a = connect("CallA", &aa, room(&aa), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let b = connect("CallB", &ab, room(&ab), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&a, &b], up).await;
        rep.add("p2p call: caller and callee connect with the region's credentials", ok, format!("A up={} B up={}", a.up(), b.up()));
        let (ok, d) = hears(&a, &[&b]).await;
        rep.add("p2p call: callee hears caller", ok, d);
        let (ok, d) = hears(&b, &[&a]).await;
        rep.add("p2p call: caller hears callee", ok, d);

        // Gap 6: X's id is crafted as session XOR W (an HG id chosen by its home grid); a region would
        // hand X a valid credential for the SAME channel. X must land in a room of its own.
        let x = xor(&session, &w);
        let xc = connect("Hijacker", &x, room(&x), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&xc], up).await;
        rep.add("gap 6: the crafted id's connection is accepted (its credential is genuine)", ok, format!("up={}", xc.up()));
        let (ok, d) = silent(&a, &[&xc]).await;
        rep.add("gap 6: but it does not hear the call", ok, d);
        let (ok, d) = silent(&xc, &[&a, &b]).await;
        rep.add("gap 6: and the call does not hear it", ok, d);
        xc.logout(&endpoint).await;

        // Reconnect with the stored credentials (llvoicewebrtc.cpp:1483, :3400).
        a.logout(&endpoint).await;
        let a2 = connect("CallA2", &aa, room(&aa), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&a2], up).await;
        rep.add("p2p call: reconnect with the stored credentials", ok, format!("up={}", a2.up()));
        let (ok, d) = hears(&b, &[&a2]).await;
        rep.add("p2p call: after the reconnect A hears B", ok, d);
        let (ok, d) = hears(&a2, &[&b]).await;
        rep.add("p2p call: and B hears A", ok, d);
        a2.logout(&endpoint).await;
        b.logout(&endpoint).await;

        // A credential for someone else, or made up, is refused.
        let forged = connect("Forged", &w, Kind::Room { channel: session.clone(), credentials: call_credentials(&cred_key(), &session, &aa) }, Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await;
        rep.add("p2p call: someone else's credential is refused", forged.is_err(), forged.err().map(|e| e.to_string()).unwrap_or_default());
    }

    // ---------------------------------------------------------------- group voice and Wolf Chat rooms
    for (label, channel) in [
        ("group voice", format!("group-f5c0{run:04x}-0000-4000-8000-0000000000a1")),
        ("Wolf Chat room", format!("wchan-f5c0{run:04x}-0000-4000-8000-0000000000a1-f5c0{run:04x}-0000-4000-8000-0000000000b2")),
    ] {
        let base = if label == "group voice" { 0x21 } else { 0x31 };
        let (aa, ab) = (agent(base, run), agent(base + 1, run));
        let room = |who: &str| Kind::Room { channel: channel.clone(), credentials: call_credentials(&cred_key(), &channel, who) };
        let a = connect(&format!("{label} A"), &aa, room(&aa), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let b = connect(&format!("{label} B"), &ab, room(&ab), Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await?;
        let ok = wait_up(&[&a, &b], up).await;
        rep.add(&format!("{label}: both connect"), ok, format!("A up={} B up={}", a.up(), b.up()));
        let (ok, d) = hears(&a, &[&b]).await;
        rep.add(&format!("{label}: B hears A"), ok, d);
        let (ok, d) = hears(&b, &[&a]).await;
        rep.add(&format!("{label}: A hears B"), ok, d);
        // Gap 4 change-over: a credential signed with the RELAY key (a region not yet restarted)
        // still opens the room while the instance accepts legacy credentials.
        if label == "group voice" && cred_key() != rpc_key() {
            let ac = agent(base + 2, run);
            let legacy = connect("Legacy", &ac, Kind::Room { channel: channel.clone(), credentials: call_credentials(&rpc_key(), &channel, &ac) }, Joins::Firestorm, [0.0; 3], &endpoint, rt.clone()).await;
            let ok = legacy.is_ok();
            rep.add("gap 4: change-over - a relay-key credential is still accepted", ok, legacy.as_ref().err().map(|e| e.to_string()).unwrap_or_default());
            if let Ok(c) = legacy {
                c.logout(&endpoint).await;
            }
        }
        a.logout(&endpoint).await;
        b.logout(&endpoint).await;
    }

    // ---------------------------------------------------------------- health endpoint (gap 9)
    {
        let http = reqwest::Client::builder().danger_accept_invalid_certs(true).build()?;
        let plain = http.get(&endpoint).send().await?.text().await?;
        rep.add("gap 9: health GET without the key says only ok", plain == "ok", format!("{plain:?}"));
        let keyed = http.get(&endpoint).header("X-Wolfvoice-Key", rpc_key()).send().await?.text().await?;
        rep.add("gap 9: with the key it gives the counts", keyed.contains("\"sessions\""), keyed);
    }

    let failed = rep.rows.iter().filter(|ok| !**ok).count();
    println!("\nSUMMARY: {} passed, {failed} failed", rep.rows.len() - failed);
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
