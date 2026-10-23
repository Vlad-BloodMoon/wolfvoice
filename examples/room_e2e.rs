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

//! End-to-end check of Wolf Chat voice ROOMS (MultiAgent "wchan-..." channels), driven
//! the way WolfViewer drives them, with every case answered by what a listener actually
//! DECODES - not by packet counts.
//!
//! Each synthetic viewer:
//!   - offers an Opus sendrecv track plus the "SLData" data channel (llwebrtc.cpp:1207-1231),
//!   - is provisioned through the same JSON-RPC body the region relays
//!     (WebRtcVoiceServiceConnector; the ad-hoc fields from llvoicewebrtc.cpp
//!     requestVoiceConnection: channel_type "multiagent", channel, credentials),
//!   - sends {"j":{"p":true}} once its data channel is open (sendJoin),
//!   - sends Opus 20 ms frames all the time, like a live microphone: a 440/660 Hz tone
//!     while "talking", digital silence otherwise,
//!   - decodes everything it receives and measures its RMS energy.
//!
//! Cases (each prints PASS/FAIL with received-audio stats):
//!   1 first join while a spatial session is up (make-before-break), A and B both ways
//!   2 join order: A first then B / B first then A / both at once
//!   3 rejoin (log the room session out, provision a new one)
//!   4 stop talking -> listener's "speaking" clears; leave -> listener told "l"
//!   5 a third client joining late hears both
//!
//! Run ON the voice host, preferably against a second (test) instance - WOLFVOICE_BIND,
//! WOLFVOICE_MEDIA_LO/HI let one run beside the live service, e.g. WOLFVOICE_BIND=127.0.0.1:9444 -
//! with that instance's relay key:
//!     export WOLFVOICE_RPC_KEY=...   # the test instance's relay key
//!     ./room_e2e https://127.0.0.1:9444
//! WOLFVOICE_RPC_KEY is read from the environment and never printed: it signs the call
//! credentials exactly as the region does (main.rs call_credentials, WebRtcVoiceRegionModule
//! CallCredentials) and is the relay key of the request.

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
/// A decoded stereo frame louder than this counts as heard. The tone is sent at 0.3 peak
/// (RMS ~0.21) and wolfvoice mixes non-spatial rooms at 1/sqrt(2) per side, so a heard
/// frame is ~0.15 RMS; Opus silence decodes to ~0.
const HEARD_RMS: f32 = 0.02;
/// How long a listener gets to hear a talker before the case fails.
const HEAR_WITHIN: Duration = Duration::from_secs(4);

fn rpc_key() -> String {
    std::env::var("WOLFVOICE_RPC_KEY").unwrap_or_default()
}

/// [HARDENING 2026-10-23] The key room credentials are signed with: WOLFVOICE_CRED_KEY when the
/// instance has a separate credential key (main.rs CRED_KEY_ENV), else the relay key as before.
fn cred_key() -> String {
    std::env::var("WOLFVOICE_CRED_KEY").ok().filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_string()).unwrap_or_else(rpc_key)
}

/// Source: wolfvoice main.rs call_credentials - HMAC-SHA256 over "wolfcall1:<channel>:<user>".
fn call_credentials(key: &str, channel: &str, user_id: &str) -> String {
    let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
    let tag = ring::hmac::sign(&k, format!("wolfcall1:{channel}:{user_id}").as_bytes());
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Default)]
struct Observed {
    connected: AtomicBool,
    dc_open: AtomicBool,
    gathered: AtomicBool,
    closed: AtomicBool,
    /// Decoded frames received, and how many of them were audible.
    frames: AtomicU64,
    heard: AtomicU64,
    /// Last "v" (speaking) the server reported per agent, and agents told "l".
    speaking: Mutex<HashMap<String, bool>>,
    left: Mutex<Vec<String>>,
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
        match state {
            RTCPeerConnectionState::Connected => self.obs.connected.store(true, Ordering::SeqCst),
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                self.obs.closed.store(true, Ordering::SeqCst)
            }
            _ => {}
        }
    }

    /// The server's mix for us. Decode every packet, exactly as a viewer would play it.
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

/// One synthetic viewer connection (one voice session).
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
    /// The "microphone": while set the sender loop sends tone, otherwise silence.
    talking: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
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

#[derive(Clone)]
enum Kind {
    Spatial,
    Room(String),
}

async fn rpc(endpoint: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;
    Ok(http
        .post(endpoint)
        .header("Content-Type", "application/json-rpc")
        .json(&body)
        .send()
        .await?
        .json()
        .await?)
}

async fn connect(
    name: &str,
    agent: &str,
    scene: &str,
    kind: Kind,
    endpoint: &str,
    runtime: Arc<dyn Runtime>,
) -> anyhow::Result<Conn> {
    let codec = opus_params();
    let mut me = MediaEngine::default();
    me.register_codec(codec.clone(), RtpCodecKind::Audio)?;
    let registry = register_default_interceptors(InterceptorRegistry::new(), &mut me)?;
    let obs = Arc::new(Observed::default());
    let handler = Arc::new(Handler {
        name: name.to_string(),
        obs: obs.clone(),
        runtime: runtime.clone(),
    });
    let ssrc: u32 = (uuid::Uuid::new_v4().as_u128() as u32) | 1;
    let track = Arc::new(TrackLocalStaticSample::new(MediaStreamTrack::new(
        "SLStream".to_owned(),
        format!("SLAudio-{name}"),
        "SLAudio".to_owned(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
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
    // Source: llwebrtc.cpp:1210 CreateDataChannelOrError("SLData") before CreateOffer (:1266).
    let dc = pc.create_data_channel("SLData", None).await?;
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer).await?;
    for _ in 0..100 {
        if obs.gathered.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let sdp = pc
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local description"))?
        .sdp;

    let mut request = serde_json::json!({
        "jsep": { "type": "offer", "sdp": sdp },
        "voice_server_type": "webrtc",
        "parcel_local_id": 0,
    });
    match &kind {
        Kind::Spatial => request["channel_type"] = "local".into(),
        Kind::Room(ch) => {
            request["channel_type"] = "multiagent".into();
            request["channel"] = ch.clone().into();
            request["credentials"] = call_credentials(&cred_key(), ch, agent).into();
        }
    }
    let resp = rpc(
        endpoint,
        serde_json::json!({
            "jsonrpc": "2.0", "id": format!("e2e-{name}"),
            "method": "provision_voice_account_request",
            "params": { "request": request, "userID": agent, "scene": scene, "key": rpc_key() }
        }),
    )
    .await?;
    if let Some(err) = resp.get("error") {
        anyhow::bail!("[{name}] provision failed: {err}");
    }
    let result = resp.get("result").ok_or_else(|| anyhow::anyhow!("no result"))?;
    let session = result["viewer_session"].as_str().unwrap_or_default().to_string();
    let answer = result["jsep"]["sdp"].as_str().unwrap_or_default().to_string();
    let pt = opus_pt_from_sdp(&answer).unwrap_or(111);
    pc.set_remote_description(RTCSessionDescription::answer(answer)?).await?;

    // Data channel: join once open (sendJoin), record roster "v" and "l".
    {
        let obs = obs.clone();
        let dc2 = dc.clone();
        runtime.spawn(Box::pin(async move {
            while let Some(evt) = dc2.poll().await {
                match evt {
                    DataChannelEvent::OnOpen => {
                        obs.dc_open.store(true, Ordering::SeqCst);
                        let _ = dc2.send_text(r#"{"j":{"p":true}}"#).await;
                    }
                    DataChannelEvent::OnMessage(m) => {
                        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&m.data) else { continue };
                        let Some(map) = v.as_object() else { continue };
                        for (id, e) in map {
                            if e.get("l").and_then(|x| x.as_bool()) == Some(true) {
                                obs.left.lock().push(id.clone());
                                obs.speaking.lock().remove(id);
                            } else if let Some(sp) = e.get("v").and_then(|x| x.as_bool()) {
                                obs.speaking.lock().insert(id.clone(), sp);
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
        scene: scene.to_string(),
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
    /// A live microphone: one Opus frame every 20 ms, tone while talking, silence otherwise.
    fn start_mic(&self) {
        let track = self.track.clone();
        let (ssrc, pt) = (self.ssrc, self.pt);
        let talking = self.talking.clone();
        let stop = self.stop.clone();
        let freq = if self.name.starts_with('A') { 440.0 } else if self.name.starts_with('B') { 660.0 } else { 550.0 };
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
            serde_json::json!({
                "jsonrpc": "2.0", "id": format!("e2e-{}-logout", self.name),
                "method": "provision_voice_account_request",
                "params": {
                    "request": { "logout": true, "viewer_session": self.session, "voice_server_type": "webrtc" },
                    "userID": self.agent, "scene": self.scene, "key": rpc_key()
                }
            }),
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
/// Returns (ok, detail).
async fn check_hears(talker: &Conn, listeners: &[&Conn]) -> (bool, String) {
    let before: Vec<u64> = listeners.iter().map(|l| l.obs.heard.load(Ordering::SeqCst)).collect();
    let frames_before: Vec<u64> = listeners.iter().map(|l| l.obs.frames.load(Ordering::SeqCst)).collect();
    talker.talking.store(true, Ordering::SeqCst);
    let t = Instant::now();
    let mut first: Vec<Option<Duration>> = vec![None; listeners.len()];
    while t.elapsed() < HEAR_WITHIN {
        for (i, l) in listeners.iter().enumerate() {
            if first[i].is_none() && l.obs.heard.load(Ordering::SeqCst) > before[i] + 5 {
                first[i] = Some(t.elapsed());
            }
        }
        if first.iter().all(|f| f.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Keep talking a moment so the stats cover steady state.
    tokio::time::sleep(Duration::from_millis(500)).await;
    talker.talking.store(false, Ordering::SeqCst);
    let mut ok = true;
    let mut detail = Vec::new();
    for (i, l) in listeners.iter().enumerate() {
        let heard = l.obs.heard.load(Ordering::SeqCst) - before[i];
        let frames = l.obs.frames.load(Ordering::SeqCst) - frames_before[i];
        let speaking = l.obs.speaking.lock().get(&talker.agent).copied();
        ok &= first[i].is_some();
        detail.push(format!(
            "{}<-{}: {} (heard {heard}/{frames} frames, first at {}, roster v={:?})",
            l.name,
            talker.name,
            if first[i].is_some() { "HEARD" } else { "SILENT" },
            first[i].map(|d| format!("{} ms", d.as_millis())).unwrap_or_else(|| "-".into()),
            speaking
        ));
    }
    (ok, detail.join("; "))
}

struct Report {
    rows: Vec<(String, bool, String)>,
}

impl Report {
    fn add(&mut self, case: &str, ok: bool, detail: String) {
        println!("{} {case}: {detail}", if ok { "PASS" } else { "FAIL" });
        self.rows.push((case.to_string(), ok, detail));
    }
}

fn agent(n: u8, run: u32) -> String {
    format!("e2e0{n:x}{run:03x}-0000-4000-8000-0000000000{n:02x}")
}

/// Spatial session first (in its own scene, so it can never be what a listener hears),
/// then the room, then the spatial session goes - WolfViewer's order for a first join
/// (startAdHocSession: the ad-hoc session is made, the old one left).
async fn first_join(
    name: &str,
    agent: &str,
    channel: &str,
    endpoint: &str,
    rt: Arc<dyn Runtime>,
) -> anyhow::Result<Conn> {
    let spatial = connect(&format!("{name}-spatial"), agent, &format!("{agent}-scene"), Kind::Spatial, endpoint, rt.clone()).await?;
    wait_up(&[&spatial], Duration::from_secs(10)).await;
    let room = connect(name, agent, "00000000-0000-4000-8000-00000000e2e0", Kind::Room(channel.to_string()), endpoint, rt).await?;
    spatial.logout(endpoint).await;
    Ok(room)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let endpoint = std::env::args().nth(1).unwrap_or_else(|| "https://127.0.0.1:9444".to_string());
    if rpc_key().is_empty() {
        anyhow::bail!("WOLFVOICE_RPC_KEY is not set (source the instance's env file)");
    }
    let rt: Arc<dyn Runtime> = Arc::new(TokioRuntime);
    let run = std::process::id() & 0xfff;
    let mut rep = Report { rows: Vec::new() };
    let up = Duration::from_secs(10);

    // 1 + 2a: first join while spatial is up; A first, then B.
    {
        let ch = format!("wchan-e2e{run}-1");
        let a = first_join("A", &agent(1, run), &ch, &endpoint, rt.clone()).await?;
        wait_up(&[&a], up).await;
        let b = first_join("B", &agent(2, run), &ch, &endpoint, rt.clone()).await?;
        let ok_up = wait_up(&[&a, &b], up).await;
        rep.add("1 first join: both connected", ok_up, format!("A up={} B up={}", a.up(), b.up()));
        let (ok, d) = check_hears(&a, &[&b]).await;
        rep.add("1 first join (A first): B hears A", ok, d);
        let (ok, d) = check_hears(&b, &[&a]).await;
        rep.add("1 first join (A first): A hears B", ok, d);

        // 4: speaking clears after the talker stops; leave is announced.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let stuck = b.obs.speaking.lock().get(&a.agent).copied();
        rep.add("4 speaking clears when A stops", stuck != Some(true), format!("B's last v for A = {stuck:?}"));
        a.logout(&endpoint).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let told = b.obs.left.lock().contains(&a.agent);
        rep.add("4 B told A left", told, format!("left list {:?}", b.obs.left.lock()));
        b.logout(&endpoint).await;
    }

    // 2b: B first, then A.
    {
        let ch = format!("wchan-e2e{run}-2");
        let b = first_join("B", &agent(4, run), &ch, &endpoint, rt.clone()).await?;
        wait_up(&[&b], up).await;
        let a = first_join("A", &agent(3, run), &ch, &endpoint, rt.clone()).await?;
        wait_up(&[&a, &b], up).await;
        let (ok, d) = check_hears(&a, &[&b]).await;
        rep.add("2 join order (B first): B hears A", ok, d);
        let (ok, d) = check_hears(&b, &[&a]).await;
        rep.add("2 join order (B first): A hears B", ok, d);
        a.logout(&endpoint).await;
        b.logout(&endpoint).await;
    }

    // 2c: simultaneous joins.
    {
        let ch = format!("wchan-e2e{run}-3");
        let (ag5, ag6) = (agent(5, run), agent(6, run));
        let (a, b) = tokio::join!(
            first_join("A", &ag5, &ch, &endpoint, rt.clone()),
            first_join("B", &ag6, &ch, &endpoint, rt.clone())
        );
        let (a, b) = (a?, b?);
        wait_up(&[&a, &b], up).await;
        let (ok, d) = check_hears(&a, &[&b]).await;
        rep.add("2 simultaneous: B hears A", ok, d);
        let (ok, d) = check_hears(&b, &[&a]).await;
        rep.add("2 simultaneous: A hears B", ok, d);

        // 3: rejoin - A's room session logged out, a fresh one provisioned.
        a.logout(&endpoint).await;
        let a2 = connect("A2", &ag5, "00000000-0000-4000-8000-00000000e2e0", Kind::Room(ch.clone()), &endpoint, rt.clone()).await?;
        wait_up(&[&a2], up).await;
        let (ok, d) = check_hears(&a2, &[&b]).await;
        rep.add("3 rejoin: B hears A", ok, d);
        let (ok, d) = check_hears(&b, &[&a2]).await;
        rep.add("3 rejoin: A hears B", ok, d);

        // 5: a third client joins late and hears both.
        let c = first_join("C", &agent(7, run), &ch, &endpoint, rt.clone()).await?;
        wait_up(&[&c], up).await;
        let (ok, d) = check_hears(&a2, &[&b, &c]).await;
        rep.add("5 late joiner: C (and B) hear A", ok, d);
        let (ok, d) = check_hears(&b, &[&a2, &c]).await;
        rep.add("5 late joiner: C (and A) hear B", ok, d);
        let (ok, d) = check_hears(&c, &[&a2, &b]).await;
        rep.add("5 late joiner: A and B hear C", ok, d);
        a2.logout(&endpoint).await;
        b.logout(&endpoint).await;
        c.logout(&endpoint).await;
    }

    let failed = rep.rows.iter().filter(|r| !r.1).count();
    println!("\nSUMMARY: {} passed, {failed} failed", rep.rows.len() - failed);
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
