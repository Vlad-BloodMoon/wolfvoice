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

//! Optional: story characters speaking through spatial voice.
//!
//! OFF unless WOLFVOICE_NPC_HOSTS names the hosts clips may come from. A story/NPC service of
//! your own (on the Wolf Territories Grid, the Wolf Adventure brain) calls the JSON-RPC method
//! `wolf.npc_speak` - authenticated with the relay key like every region call - with a short-lived
//! https URL to a WAV clip it made (for example with piper). We fetch it,
//! decode it, resample it onto the mixer's 48 kHz / 960-sample grid, and play it as a VIRTUAL
//! SPEAKER at the character's position in every spatial room of that region. The normal
//! per-listener distance mix (mixer::distance_gain, REF_DISTANCE 10 m, MAX_DISTANCE 60 m) and the
//! listener's own per-speaker gain/mute decide who hears it, exactly as for a real avatar.
//!
//! What it deliberately does NOT do: announce the character in the voice roster. A roster join
//! creates a participant in the viewer (llvoicewebrtc.cpp:3205-3208) and nothing in the protocol
//! ever expires one except an explicit leave (:3212-3219), so a virtual entry would need its own
//! join/leave bookkeeping per listener and is untested in either viewer, so the
//! roster is left to real participants.

use parking_lot::Mutex;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use crate::mixer::{FRAME_SAMPLES, SAMPLE_RATE};

/// The JSON-RPC method name a story service calls.
pub const METHOD_NPC_SPEAK: &str = "wolf.npc_speak";

/// Hosts a clip may come from: a comma-separated list of exact host names
/// ("stories.example.org,www.example.org"). Unset or empty: story voices are off and
/// `wolf.npc_speak` is refused.
pub const HOSTS_ENV: &str = "WOLFVOICE_NPC_HOSTS";
/// A clip is at most 2 MB, fetched within 5 s.
pub const MAX_CLIP_BYTES: usize = 2 * 1024 * 1024;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Characters speaking at once across the whole service, and the most audio one character may
/// have queued (60 s): bounds memory whatever a caller sends.
pub const MAX_ACTIVE: usize = 32;
pub const MAX_QUEUED_FRAMES: usize = 60 * (SAMPLE_RATE as usize) / FRAME_SAMPLES;
/// Most listeners a call may name (for example a class session).
pub const MAX_LISTENERS: usize = 200;

/// One 20 ms frame of one character, shared by every room of its region this tick.
pub struct NpcFrame {
    pub scene: String,
    pub speaker: String,
    pub pos: [f64; 3],
    pub gain: f32,
    pub listeners: Option<Arc<HashSet<String>>>,
    pub frame: Vec<f32>,
}

impl NpcFrame {
    /// May this listener hear it? `listeners` None = anyone in range.
    pub fn audible_to(&self, agent: &str) -> bool {
        match &self.listeners {
            None => true,
            Some(set) => set.contains(agent),
        }
    }
}

struct Voice {
    scene: String,
    speaker: String,
    pos: [f64; 3],
    gain: f32,
    listeners: Option<Arc<HashSet<String>>>,
    frames: VecDeque<Vec<f32>>,
}

/// Every character currently speaking.
#[derive(Default)]
pub struct NpcVoices {
    voices: Mutex<Vec<Voice>>,
}

impl NpcVoices {
    /// Queue a decoded clip. A character already speaking in the same region has the clip
    /// appended (its lines play in order); its position, gain and listeners take the newest.
    pub fn add(&self, req: &SpeakRequest, frames: Vec<Vec<f32>>) -> Result<(), String> {
        let mut v = self.voices.lock();
        if let Some(existing) = v.iter_mut().find(|x| x.speaker == req.speaker && x.scene == req.scene) {
            if existing.frames.len() + frames.len() > MAX_QUEUED_FRAMES {
                return Err("that character already has a minute of speech queued".into());
            }
            existing.pos = req.pos;
            existing.gain = req.gain;
            existing.listeners = req.listeners.clone();
            existing.frames.extend(frames);
            return Ok(());
        }
        if v.len() >= MAX_ACTIVE {
            return Err("too many characters speaking at once".into());
        }
        if frames.len() > MAX_QUEUED_FRAMES {
            return Err("clip too long".into());
        }
        v.push(Voice {
            scene: req.scene.clone(),
            speaker: req.speaker.clone(),
            pos: req.pos,
            gain: req.gain,
            listeners: req.listeners.clone(),
            frames: frames.into(),
        });
        Ok(())
    }

    /// One frame from every speaking character, once per mixer tick. Finished ones are dropped.
    pub fn tick(&self) -> Vec<Arc<NpcFrame>> {
        let mut v = self.voices.lock();
        let mut out = Vec::with_capacity(v.len());
        for voice in v.iter_mut() {
            if let Some(frame) = voice.frames.pop_front() {
                out.push(Arc::new(NpcFrame {
                    scene: voice.scene.clone(),
                    speaker: voice.speaker.clone(),
                    pos: voice.pos,
                    gain: voice.gain,
                    listeners: voice.listeners.clone(),
                    frame,
                }));
            }
        }
        v.retain(|x| !x.frames.is_empty());
        out
    }

    pub fn active(&self) -> usize {
        self.voices.lock().len()
    }
}

/// A validated `wolf.npc_speak` call.
#[derive(Debug, Clone)]
pub struct SpeakRequest {
    pub scene: String,
    pub speaker: String,
    pub pos: [f64; 3],
    pub url: String,
    pub gain: f32,
    pub listeners: Option<Arc<HashSet<String>>>,
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Check the params object of a `wolf.npc_speak` call (the key is checked by the caller,
/// main.rs rpc_response, exactly as for the relay's own methods). `allowed` is HOSTS_ENV.
pub fn parse_request(p: &serde_json::Value, allowed: &[String]) -> Result<SpeakRequest, String> {
    let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_ascii_lowercase();
    let scene = s("scene");
    let speaker = s("speaker");
    if !is_uuid(&scene) {
        return Err("scene must be a region uuid".into());
    }
    if !is_uuid(&speaker) {
        return Err("speaker must be a uuid".into());
    }
    let pos = match p.get("pos").and_then(|v| v.as_array()) {
        Some(a) if a.len() == 3 => {
            let mut out = [0.0f64; 3];
            for (i, c) in a.iter().enumerate() {
                out[i] = c.as_f64().filter(|x| x.is_finite()).ok_or("pos must be three numbers")?;
            }
            out
        }
        _ => return Err("pos must be [x,y,z]".into()),
    };
    let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    check_url(&url, allowed)?;
    let gain = p.get("gain").and_then(|v| v.as_f64()).unwrap_or(1.0);
    if !gain.is_finite() {
        return Err("gain must be a number".into());
    }
    let gain = gain.clamp(0.0, 2.0) as f32;
    let listeners = match p.get("listeners") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(a)) => {
            if a.len() > MAX_LISTENERS {
                return Err("too many listeners".into());
            }
            let mut set = HashSet::with_capacity(a.len());
            for v in a {
                let id = v.as_str().unwrap_or("").trim().to_ascii_lowercase();
                if !is_uuid(&id) {
                    return Err("listeners must be agent uuids".into());
                }
                set.insert(id);
            }
            Some(Arc::new(set))
        }
        Some(_) => return Err("listeners must be a list or null".into()),
    };
    Ok(SpeakRequest { scene, speaker, pos, url, gain, listeners })
}

/// Parse HOSTS_ENV: exact host names, lower-cased; blanks dropped.
pub fn parse_hosts(v: &str) -> Vec<String> {
    v.split(',').map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty()).collect()
}

/// https only, port 443, no user-info, host on the allow-list (`allowed`, from HOSTS_ENV).
pub fn check_url(url: &str, allowed: &[String]) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(url).map_err(|_| "bad url".to_string())?;
    if u.scheme() != "https" {
        return Err("url must be https".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("url must not carry credentials".into());
    }
    if u.port().is_some_and(|p| p != 443) {
        return Err("url must use port 443".into());
    }
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    if !allowed.contains(&host) {
        return Err("url host is not allowed".into());
    }
    Ok(u)
}

/// Fetch a clip: no redirects, FETCH_TIMEOUT for the whole request, at most MAX_CLIP_BYTES.
pub async fn fetch(url: &reqwest::Url) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut resp = client.get(url.clone()).send().await.map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fetch answered {}", resp.status().as_u16()));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_CLIP_BYTES) {
        return Err("clip too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("fetch failed: {e}"))? {
        if body.len() + chunk.len() > MAX_CLIP_BYTES {
            return Err("clip too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Decode a RIFF/WAVE file of 16-bit PCM mono. Returns (sample rate, samples).
///
/// Chunks are walked properly (each is id, little-endian u32 size, data, padded to an even
/// length), so a LIST or other chunk before `data` is skipped rather than played as noise. A
/// `data` size larger than what is present (a streamed header) is cut to what is present.
pub fn parse_wav(b: &[u8]) -> Result<(u32, Vec<i16>), String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let le16 = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let le32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut at = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = le32(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " {
            if size < 16 || body + 16 > b.len() {
                return Err("short fmt chunk".into());
            }
            fmt = Some((le16(body), le16(body + 2), le32(body + 4), le16(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("data before fmt")?;
            if format != 1 {
                return Err("only PCM WAV is supported".into());
            }
            if channels != 1 {
                return Err("only mono WAV is supported".into());
            }
            if bits != 16 {
                return Err("only 16-bit WAV is supported".into());
            }
            if rate == 0 || rate > 192_000 {
                return Err("bad sample rate".into());
            }
            let end = body.saturating_add(size).min(b.len());
            let samples = b[body..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            return Ok((rate, samples));
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    Err("no data chunk".into())
}

/// Resample to SAMPLE_RATE by linear interpolation and cut into FRAME_SAMPLES frames (the last
/// one padded with silence). Output length is ceil(n * 48000 / rate) samples.
pub fn to_frames(samples: &[i16], rate: u32) -> Vec<Vec<f32>> {
    if samples.is_empty() || rate == 0 {
        return Vec::new();
    }
    let n = samples.len();
    let out_len = (n as u64 * SAMPLE_RATE as u64).div_ceil(rate as u64) as usize;
    let step = rate as f64 / SAMPLE_RATE as f64;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let x = i as f64 * step;
        let i0 = (x.floor() as usize).min(n - 1);
        let i1 = (i0 + 1).min(n - 1);
        let t = (x - i0 as f64) as f32;
        let a = samples[i0] as f32 / 32768.0;
        let b = samples[i1] as f32 / 32768.0;
        out.push(a + (b - a) * t);
    }
    out.chunks(FRAME_SAMPLES)
        .map(|c| {
            let mut f = c.to_vec();
            f.resize(FRAME_SAMPLES, 0.0);
            f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, bits: u16, samples: &[i16], extra_chunk: bool) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        v.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        if extra_chunk {
            v.extend_from_slice(b"LIST");
            v.extend_from_slice(&3u32.to_le_bytes());
            v.extend_from_slice(&[1, 2, 3, 0]); // odd size, padded
        }
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&data);
        v
    }

    #[test]
    fn parses_piper_style_wav_and_skips_other_chunks() {
        let s: Vec<i16> = (0..100).map(|i| i as i16 * 10).collect();
        let (rate, out) = parse_wav(&wav(22050, 1, 16, &s, true)).unwrap();
        assert_eq!(rate, 22050);
        assert_eq!(out, s);
    }

    #[test]
    fn refuses_what_it_cannot_play() {
        assert!(parse_wav(b"not a wav at all").is_err());
        assert!(parse_wav(&wav(22050, 2, 16, &[0, 0], false)).is_err(), "stereo");
        assert!(parse_wav(&wav(22050, 1, 8, &[0, 0], false)).is_err(), "8-bit");
        let mut no_data = wav(22050, 1, 16, &[], false);
        no_data.truncate(36);
        assert!(parse_wav(&no_data).is_err());
    }

    #[test]
    fn a_streamed_data_size_is_cut_to_what_is_present() {
        let mut w = wav(22050, 1, 16, &[1, 2, 3], false);
        let n = w.len();
        w[n - 10..n - 6].copy_from_slice(&u32::MAX.to_le_bytes());
        let (_, out) = parse_wav(&w).unwrap();
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn resampling_gives_the_right_length_in_whole_frames() {
        // One second at 22050 Hz -> 48000 samples -> exactly 50 frames of 960.
        let s = vec![1000i16; 22050];
        let f = to_frames(&s, 22050);
        assert_eq!(f.len(), 50);
        assert!(f.iter().all(|x| x.len() == FRAME_SAMPLES));
        // 100 samples at 22050 -> ceil(100*48000/22050) = 218 samples -> one padded frame.
        let f = to_frames(&[1000i16; 100], 22050);
        assert_eq!(f.len(), 1);
        assert!((f[0][0] - 1000.0 / 32768.0).abs() < 1e-6);
        assert_eq!(f[0][217 + 1], 0.0, "padding after the clip is silence");
        assert!(to_frames(&[], 22050).is_empty());
    }

    #[test]
    fn url_must_be_https_on_an_allowed_host() {
        let hosts = parse_hosts(" Stories.Example.org, www.example.org ,,");
        assert_eq!(hosts, vec!["stories.example.org".to_string(), "www.example.org".to_string()]);
        assert!(check_url("https://stories.example.org/media.php?c=a&e=1&s=b", &hosts).is_ok());
        assert!(check_url("https://www.example.org/x", &hosts).is_ok());
        assert!(check_url("http://stories.example.org/x", &hosts).is_err());
        assert!(check_url("https://evil.example/x", &hosts).is_err());
        assert!(check_url("https://stories.example.org.evil.example/x", &hosts).is_err());
        assert!(check_url("https://user:pw@stories.example.org/x", &hosts).is_err());
        assert!(check_url("https://stories.example.org:8443/x", &hosts).is_err());
        assert!(check_url("https://127.0.0.1/x", &hosts).is_err());
        assert!(check_url("https://stories.example.org/x", &[]).is_err(), "no hosts: off");
        assert!(parse_hosts("").is_empty());
    }

    fn test_hosts() -> Vec<String> {
        parse_hosts("stories.example.org")
    }

    fn params(listeners: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"key":"k","scene":"1402e3d3-a959-42f8-b423-201807a3e2f1",
            "speaker":"AAAAAAAA-a959-42f8-b423-201807a3e2f1","pos":[1.0,2.0,3.0],
            "url":"https://stories.example.org/x","gain":5.0,"listeners":listeners})
    }

    #[test]
    fn request_is_validated_and_listeners_filter() {
        let r = parse_request(&params(serde_json::Value::Null), &test_hosts()).unwrap();
        assert_eq!(r.speaker, "aaaaaaaa-a959-42f8-b423-201807a3e2f1", "uuids are lower-cased");
        assert_eq!(r.gain, 2.0, "gain is clamped to 2");
        assert!(r.listeners.is_none());
        let a = "11111111-1111-4111-8111-111111111111";
        let r = parse_request(&params(serde_json::json!([a])), &test_hosts()).unwrap();
        let frame = NpcFrame { scene: r.scene.clone(), speaker: r.speaker.clone(), pos: r.pos, gain: r.gain,
                               listeners: r.listeners.clone(), frame: vec![0.0; FRAME_SAMPLES] };
        assert!(frame.audible_to(a));
        assert!(!frame.audible_to("22222222-2222-4222-8222-222222222222"));
        assert!(parse_request(&params(serde_json::json!(["not-a-uuid"])), &test_hosts()).is_err());
        let mut bad = params(serde_json::Value::Null);
        bad["pos"] = serde_json::json!([1.0, 2.0]);
        assert!(parse_request(&bad, &test_hosts()).is_err());
        bad["pos"] = serde_json::json!([1.0, 2.0, 3.0]);
        bad["scene"] = serde_json::json!("x");
        assert!(parse_request(&bad, &test_hosts()).is_err());
    }

    #[test]
    fn clips_queue_per_character_and_finish() {
        let reg = NpcVoices::default();
        let r = parse_request(&params(serde_json::Value::Null), &test_hosts()).unwrap();
        reg.add(&r, vec![vec![0.1; FRAME_SAMPLES]; 2]).unwrap();
        reg.add(&r, vec![vec![0.2; FRAME_SAMPLES]; 1]).unwrap();
        assert_eq!(reg.active(), 1, "a second line from the same character queues behind the first");
        assert_eq!(reg.tick().len(), 1);
        assert_eq!(reg.tick().len(), 1);
        let last = reg.tick();
        assert!((last[0].frame[0] - 0.2).abs() < 1e-6, "lines play in order");
        assert_eq!(reg.active(), 0, "a finished character is dropped");
        assert!(reg.tick().is_empty());
        assert!(reg.add(&r, vec![vec![0.0; FRAME_SAMPLES]; MAX_QUEUED_FRAMES + 1]).is_err());
    }
}
