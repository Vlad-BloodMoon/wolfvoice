# Changelog

By Shadow Wolf and Lone Wolf, Wolf Territories Grid and IntelligentWolf Ltd.

## v0.2.0-bloodmoon.2 - 2026-10-28: spatial listener axes

- Use listener-local -Y for right instead of +X (forward).
- Normalize quantized viewer rotations; include elevation in pan direction length.
- Preserve distance attenuation, per-user gain/mute and private-call credentials.
- Publish the exact field-tested static Linux x86-64 artifact, compiled for baseline
  CPUs and tested with TLS/RPC under QEMU without AVX.
- Operator reported successful in-world results on BloodMoon.
- Full technical explanation: [spatial correction](docs/SPATIAL_INVESTIGATION.md).

For an existing server, replace only the binary and restart the voice service.
No new OpenSim DLLs, settings, keys or ports are required for this patch.

## 0.2.0 - 2026-10-23: security hardening

Written for operators of other OpenSimulator grids. Use 0.2.0 together with the
hardened region module, version 1.2.0 of os-webrtc-janus on branch
`chatsession-p2p-session-id-and-fast-fail` of
[intelligentwolf/os-webrtc-janus](https://github.com/intelligentwolf/os-webrtc-janus);
the README says how to build it into each kind of OpenSim tree.

### Upgrading from 0.1.x

1. Build the hardened region module into your regions (README step 2). It works with
   wolfvoice 0.1.x too, so regions can go first.
2. Upgrade wolfvoice by backing up and replacing only the binary; retain the existing
   service, certificates, keys and firewall configuration. See docs/INSTALLATION.md.
3. Create the two keys (README step 3), copy them to every region host, set
   `RpcKeyFile` / `CredKeyFile` in `bin/config/wolfvoice.ini` (or use the default
   paths), restart wolfvoice, then the regions. This fork's installer does not create
   keys; follow the manual, non-overwriting procedure in docs/INSTALLATION.md.

If some regions must stay on a module without keys (an older addon, or OpenSim master's
built-in copy), run wolfvoice **without** a relay key for now: those regions send none,
and a keyed wolfvoice answers them "not authorised". Spatial voice keeps working; calls
and group voice need the hardened module.

### Security

- **Relay key.** Every JSON-RPC request from a region must carry the shared relay key
  (`WOLFVOICE_RPC_KEY` or `/etc/wolfvoice/rpc_key`); anything else is answered
  "not authorised". Without a key the service still runs, warns at start that the port is
  open, and the firewall is the only protection, as in 0.1.x.
- **Credential key.** Tickets that admit one avatar to one call, group or conference room
  are an HMAC with a second key (`WOLFVOICE_CRED_KEY` or `/etc/wolfvoice/cred_key`), so
  holding the relay key is not enough to join someone's call. While regions change over,
  tickets signed with the relay key are still accepted unless
  `WOLFVOICE_LEGACY_CREDENTIALS=0`. Changing over is described in docs/SECURITY.md.
- **No audio before join.** A session neither hears nor is heard until the viewer has
  joined on its data channel.
- **Private calls keyed on the ticket's pair of avatars**, so a crafted session or avatar
  id cannot reach someone else's call.
- **Room caps**: 2 people in a private call, 25 in a conference, 64 in a group or Wolf Chat
  room. A full room answers "voice room is full", which the region turns into the
  viewer's own "channel full" message.
- **Positions held to the truth.** With the hardened module, each region reports where its
  avatars in voice really are, once a second (`wolf.positions`). A viewer's reported
  position is pulled to within a few metres of it (more for a fast-moving avatar) and the
  listening position to within 50 m, so a modified viewer cannot listen in from
  elsewhere. Avatars with no fresh region position are mixed exactly as before.
- **ICE candidate filter.** Only public addresses are tried, at most 32 candidates per
  request and 128 per session. Add your own networks with
  `WOLFVOICE_ICE_ALLOW=10.0.0.0/8,...` if viewers reach the server over a private network.
- **Race-free session limits** (whole service, per avatar, per room), claimed atomically.
- **Media ports**: a port still in use is never handed out again.
- **Logs**: text supplied by viewers is escaped before it is logged; the relay key is never
  logged.
- **Health endpoint**: `GET /` answers `ok`. The session counts need the relay key in an
  `X-Wolfvoice-Key` header.
- **Bridge port**: at most 16 connections may be waiting to authenticate.

### New

- **Private voice calls** (Firestorm's Call button on an IM), with the callee rung on
  whichever region they are on.
- **Group voice**: "Join call" in a group's chat window; members whose role allows voice.
- **Sessions that follow the avatar** across region crossings and teleports for viewers
  that ask for it (`wolf_move`); stock viewers reconnect as before.
- **Clearer logs**: why each session ended, and when a viewer never joins.
- **Optional, off by default**, needing no Wolf Territories service while off:
  - story/NPC voices played as spatial speakers (`wolf.npc_speak`), on only when
    `WOLFVOICE_NPC_HOSTS` names the hosts clips may come from;
  - a Discord voice bridge into Wolf Chat voice rooms, on only when a bridge key is
    installed (`WOLFVOICE_BRIDGE_KEY` or `/etc/wolfvoice/bridge_key`); it needs a bridge
    client that is not part of this repository.
- `WOLFVOICE_BIND`, `WOLFVOICE_MEDIA_LO` and `WOLFVOICE_MEDIA_HI` override the listener
  and the media port range, so a test instance can run beside the live one.
- Test harnesses: `examples/fs_compat.rs` (Firestorm's exact behaviour, 34 checks) and
  `examples/room_e2e.rs` (multi-party rooms, 14 checks). `cargo test` runs 75 tests.

### Region module (os-webrtc-janus 1.2.0)

- Sends the relay key and signs tickets with the credential key (`RpcKeyFile`,
  `CredKeyFile`, re-read when they change; `AcceptLegacyCallCredentials` for the
  change-over).
- Its own TLS client for wolfvoice that always verifies the certificate chain and host
  name, whatever OpenSim's `NoVerifyCertChain` / `NoVerifyCertHostname` say.
  `WebRtcVoiceServerURI` must therefore use the host name on the certificate.
- Enforces voice off: estate and parcel voice flags, parcel bans and access lists. The
  region, not the viewer, picks the parcel room from where the avatar stands, and
  neighbour (child-agent) connections always get the estate room.
- Re-checks room rights on every connect: group membership and voice power, conference
  membership, and that the other party of a call is a real avatar.
- The position feed (`PositionFeed`, default on; it pauses itself against a wolfvoice
  that does not know `wolf.positions`).
- The text-IM fix: `ChatSessionRequest` echoes the viewer's P2P session id and fails
  `start conference` fast. The same fix is in upstream OpenSimulator as
  [opensim/opensim#69](https://github.com/opensim/opensim/pull/69).
- Ad-hoc conferences and Wolf Chat voice rooms are optional Wolf Territories features,
  **off** unless `ConferenceApiUrl` is set; without it "start conference" fails at once
  with the viewer's own message, as before.

## 0.1.1 - 2026-08-23

- Hardened the untrusted-input paths; the parcel id is logged; the reference client
  recovers after a teleport.
- After 0.1.1 (no release): session ids are bare UUIDs so viewers can send their ICE
  candidates; documentation fixes.

## 0.1.0 - 2026-08-23

- First public release.
