# wolfvoice

## Installation facile / ready-to-use downloads

**Français : [téléchargements compilés et choix des fichiers](docs/TELECHARGEMENTS_FR.md) · [installation pas à pas pour votre propre grille](docs/INSTALLATION_FR.md).**

| Ready-to-use component | Download |
| --- | --- |
| Voice server, Linux x86-64, spatial axes fixed | [WolfVoice v0.2.0-bloodmoon.2](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.2) |
| Region DLLs/PDBs, OpenSim 0.9.3.1 / .NET 8 | [WebRTC_Modules_0.9.3.1.zip](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip) |

No compilation is required for these downloads. The server and simulator modules are separate components. Other grids must configure their own hostname and keys; no BloodMoon key is included. Existing region modules do not need replacing just for the server-side axis fix.

## Current fork release: v0.2.0-bloodmoon.2

[Download the release](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.2).
The Linux x86_64 binary retains upstream 0.2.0 functionality and corrects the spatial listener axes and quaternion normalization. The operator reported successful in-world results on 28 October 2026. The published binary also passes a TLS/RPC startup test on an emulated CPU without AVX.
See [the spatial fix and upstream review notes](docs/SPATIAL_INVESTIGATION.md).
Local voice, participant dots, individual volume and private IM voice calls were confirmed by the operator. All hosted simulators were updated and restarted.
Group voice is implemented upstream but has not been confirmed in these operator tests. Ad-hoc conferences need a separate external service and are not enabled by this release.

Existing installations: replace only the binary, keep your systemd unit, TLS files, firewall rules and limits. See [deployment notes](docs/INTEGRATION.md).

**By Shadow Wolf and Lone Wolf, Wolf Territories Grid and IntelligentWolf Ltd.**

Spatial WebRTC voice for [OpenSimulator](http://opensimulator.org) — one service that
Firestorm and browser-based viewers both reach through the region's own capabilities,
so users of different viewers hear each other, positionally.

wolfvoice is a **voice service backend for
[os-webrtc-janus](https://github.com/intelligentwolf/os-webrtc-janus)**. That addon does the
OpenSimulator half — capabilities, provider advertisement, session bookkeeping — and
wolfvoice is an alternative to point it at when you want the mixing done differently.

```
Firestorm 7.1.10+  /  browser viewer
        │  ProvisionVoiceAccountRequest + VoiceSignalingRequest   (LLSD caps)
        ▼
OpenSim region  +  os-webrtc-janus addon   ← REQUIRED
        │          forwards the caps onward as JSON-RPC
        ▼
   wolfvoice  ── answers the SDP, terminates DTLS/SRTP/SCTP,
        │         mixes a SEPARATE stream per listener
        ▼
   viewers   (UDP media)
```

## What you need

**os-webrtc-janus is a hard requirement.** It supplies `WebRtcVoice.dll` and the
region-side capability handlers that everything here depends on; without it there is
nothing for a viewer to talk to. See [step 2](#2-install-the-region-modules-precompiled-or-from-source) for building it.

**New in 0.2.0** (see [CHANGELOG.md](CHANGELOG.md)): a relay key and a separate credential
key, private calls and group voice with signed tickets, positions held to where the
region says the avatar is, room caps, an ICE candidate filter, and a health endpoint that
says only `ok` without the key. Use it with the hardened region module from step 2.

What you can skip is running a **Janus gateway server** alongside it. The addon ships
two backends: `WebRtcJanusService` (which drives Janus) and
`WebRtcVoiceServiceConnector` (a JSON-RPC client that will post to any URL). wolfvoice
plugs into the second, so you point the addon at wolfvoice instead of at Janus.

**For the voice server**

- A Linux host with a **public IP address** and a **DNS name** pointing at it. TLS is
  not optional: the SDP carries the server's DTLS fingerprint, so anyone able to
  rewrite plaintext signalling in flight could become the media endpoint.
- **UDP 40000–40999** open to the world. This range is also the ceiling on concurrent
  sessions — one socket per peer connection.
- **TCP 9443** reachable *from your region hosts only*. When a relay key is configured, every region request must carry it. Signed room
  tickets use the credential key. Credential-only mode retains the IP allowlist
  during rolling upgrades - see [installation](docs/INSTALLATION.md).
- Modest CPU: **~17 ms per listener per second**, spread across all cores. See [Capacity](#capacity).

**For viewers**

- **Firestorm 7.1.10 or newer** works with no configuration at all — the region tells
  it which voice system to use. Nothing to install, nothing to tick.
- Browser viewers need a client speaking the same contract.
  [`client/voice_llwebrtc.js`](client/voice_llwebrtc.js) is a working reference
  implementation, extracted from WolfStorm.

## Why a per-listener mix

Linden Lab's WebRTC voice puts spatialisation, per-user gain and mute on the
**server**: the viewer sends its position, gain and mute requests *up* a data channel
and receives one already-mixed audio stream plus a participant roster *down* it.

wolfvoice therefore computes **a distinct mix for every listener**. For example, if both listeners face east and A is north of B,
A hears B on their right while B simultaneously hears A on their left — which one
shared room mix cannot express, however it is panned.

Janus's AudioBridge plugin is a conference mixer: it produces a single mix for a room,
and it does not carry WebRTC data channels (no `incoming_data` callback, and Janus's
SDP helper declines an `m=application` line no plugin has claimed). Those are perfectly
reasonable choices for what AudioBridge is for, but they mean the parts of LL's
protocol that ride the data channel — position, per-user gain, mute, the speaker
roster — have nowhere to travel, and the viewer will not consider voice ready until
that channel opens. wolfvoice accepts the data channel and does the mixing itself,
which is the whole reason it exists.

If you want a straightforward conference-style voice service, os-webrtc-janus with
Janus is a simpler deployment and the sensible default. Choose wolfvoice when you want
positional audio.

## Quick start

### 1. Update the voice server

Download `wolfvoice-x86_64-linux-musl` and its `.sha256` file from [this fork's release](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.2).
Verify the checksum, back up the current binary and replace it during a planned voice interruption. Keep the existing systemd unit, certificates, account, IP restrictions and configuration. No installer is needed for an existing installation.

The retained `setup.sh` now points to this fork's v0.2.0-bloodmoon.2 binary.
It is a fresh-install helper, not the rolling-upgrade procedure. It does not create keys. For a new server, follow [installation and keys](docs/INSTALLATION.md); existing deployments should follow [deployment notes](docs/INTEGRATION.md).

### 2. Install the region modules (precompiled or from source)

**OpenSim 0.9.3.1 / .NET 8:** the matching three DLL/PDB pairs are already compiled in [WebRTC_Modules_0.9.3.1.zip](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip). Stop the simulator, back up the matching files and dedicated voice INI, replace the six files in `bin/`, configure your server URL and keys, then restart from `bin/`. Keep `WebRtcJanusService.dll` if present. See [the file locations and French steps](docs/INSTALLATION_FR.md#6-installer-les-dll-sur-chaque-simulateur). For other versions/distributions, check compatibility and use the source-build instructions below where appropriate.

wolfvoice needs the region half of WebRTC voice: the **os-webrtc-janus** region module.
Use the hardened version on branch `chatsession-p2p-session-id-and-fast-fail` of
[intelligentwolf/os-webrtc-janus](https://github.com/intelligentwolf/os-webrtc-janus) -
it is the one wolfvoice 0.2.0 is tested against, and the only one that sends the relay
key, signs call tickets and feeds avatar positions. It needs a .NET 8 OpenSimulator
(0.9.3.0 or newer); it does not build on 0.9.2.x.

First find out which tree you have, from the OpenSim root:

```bash
ls OpenSim/Addons/os-webrtc-janus 2>/dev/null && echo "B: upstream master, module built in"
ls Addons/os-webrtc-janus         2>/dev/null && echo "C: OpenSim-NGC"
ls prebuild.xml                   2>/dev/null && echo "prebuild tree (A or B)"
```

**A. OpenSimulator 0.9.3.x release, or a git tree from before 26 Feb 2026**
(no `OpenSim/Addons/os-webrtc-janus`). Tested with the `opensim-0.9.3.0-source.tar.gz`
release.

1. Add the module as an addon-module:

   ```bash
   cd opensim-0.9.3.0-source          # your OpenSim root
   git clone -b chatsession-p2p-session-id-and-fast-fail \
       https://github.com/intelligentwolf/os-webrtc-janus.git addon-modules/os-webrtc-janus
   ```

2. Generate the solution and build. The 0.9.3.0 source archive ships `runprebuild.sh`
   with Windows line endings and without the execute bit, so `./runprebuild.sh` fails
   there; these are the commands it runs:

   ```bash
   cp bin/System.Drawing.Common.dll.linux bin/System.Drawing.Common.dll
   dotnet bin/prebuild.dll /target vs2022 /targetframework net8_0 /excludedir = "obj | bin" /file prebuild.xml
   dotnet build -c Release OpenSim.sln
   ```

   (In a git checkout `./runprebuild.sh && ./compile.sh` does the same.) The module's
   DLLs land in `bin/`: `WebRtcVoice.dll`, `WebRtcVoiceServiceModule.dll`,
   `WebRtcVoiceRegionModule.dll` (and `WebRtcJanusService.dll`, unused with wolfvoice).

**B. Upstream OpenSimulator master since 26 Feb 2026** (`OpenSim/Addons/os-webrtc-janus`
exists). The module is built in. Do **not** clone a second copy into `addon-modules/` -
two projects with the same names break the build. Replace the built-in sources with the
hardened ones instead, then build as usual:

```bash
git clone -b chatsession-p2p-session-id-and-fast-fail \
    https://github.com/intelligentwolf/os-webrtc-janus.git ../os-webrtc-janus-wolf
for d in Janus WebRtcVoice WebRtcVoiceRegionModule WebRtcVoiceServiceModule; do
    rm -f OpenSim/Addons/os-webrtc-janus/$d/*.cs
    cp ../os-webrtc-janus-wolf/$d/*.cs OpenSim/Addons/os-webrtc-janus/$d/
done
./runprebuild.sh && ./compile.sh
```

If you keep the built-in module unchanged instead, wolfvoice still gives spatial voice,
but only while wolfvoice has **no relay key** (the built-in module sends none, so a keyed
wolfvoice answers every request "not authorised"), there are no private calls or group
voice, and TCP 9443 is protected by your firewall alone.

**C. OpenSim-NGC** (`Addons/os-webrtc-janus`). NGC carries its own, much-extended
version of the module. Do not copy this one into it; we have not tested NGC's module
with wolfvoice.

**D. A binary-only distribution** (just `bin/`, no sources): build in a source tree of
the **exact same** OpenSim version with A or B, then copy the three `WebRtc*.dll` files
listed above into the distribution's `bin/`, replacing any that are there.

The modules are loaded when OpenSim starts, so restart the region after installing them.
Nothing is needed on the Robust (grid) server: regions talk to wolfvoice directly.

### 3. Create and distribute the keys

**This fork's installer does not create signing keys automatically.**
Follow the [complete installation and key guide](docs/INSTALLATION.md) for the
commands, file ownership, secure transfer and restart sequence. Existing keys
are preserved by those commands.

Choose the mode first:
- **Fresh deployment, all modules updated:** create relay and credential keys.
- **Rolling upgrade with old modules:** create only the credential key and retain
  the IP allowlist. Enabling a relay key immediately rejects old modules that do
  not send it.

Server credential file: `/etc/wolfvoice/cred_key`.
Copy the SAME secret to each authorized region host; do not generate different
keys for different simulators. Region setting: `CredKeyFile`.
The optional relay key is `/etc/wolfvoice/rpc_key` on the server and
`RpcKeyFile` on regions. Never include these secrets in public downloads.

### 4. Configure each region

Create **`bin/config/wolfvoice.ini`** in each region's OpenSim directory (a copy of
[`contrib/wolfvoice.ini`](contrib/wolfvoice.ini)). OpenSim reads every `.ini` in
`bin/config/` after `OpenSim.ini`, as overrides, so a shared `OpenSim.ini` stays
untouched.

```ini
[WebRtcVoice]
    Enabled = true
    SpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    NonSpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    ; The host name on wolfvoice's certificate - not an IP address. The certificate is
    ; always verified (chain and name), whatever NoVerifyCertChain says.
    WebRtcVoiceServerURI = https://voice.example.org:9443
    ; Enable only when the server requires relay authentication:
    ; RpcKeyFile = /etc/opensim/voice_rpc_key
    CredKeyFile = /etc/opensim/voice_cred_key
    MessageDetails = false

[VivoxVoice]
    enabled = false
```

`RpcKeyFile` defaults to `/etc/opensim/voice_rpc_key`; if that file exists it is read
even with the line commented out. `CredKeyFile` defaults to `/etc/opensim/voice_cred_key`. Other settings, all optional: `PositionFeed` (default `true`),
`AcceptLegacyCallCredentials` (default `true`, see [docs/SECURITY.md](docs/SECURITY.md)),
`StunServers` (STUN servers given to the viewer; default Google's public ones).
In a simulator that runs several regions, `[WebRtcVoice]` applies to all of them.

Restart the region. When it works, the region log shows:

```
[WEBRTC VOICE SERVICE CONNECTOR]: WebRtcVoiceServiceConnector enabled
[WEBRTC VOICE SERVICE MODULE]: WebRtcVoiceService enabled
[REGION WEBRTC VOICE]: avatar positions go to the voice server once a second while anyone here is in voice
[REGION WEBRTC VOICE]: no ConferenceApiUrl - conference chats and Wolf Chat voice rooms are off (group voice is not affected)
[REGION WEBRTC VOICE]: enabled
[WEBRTC VOICE SERVICE MODULE]: Adding WebRtcVoiceService to region <your region>
```

The `ConferenceApiUrl` line is normal: ad-hoc conferences and Wolf Chat rooms are
optional Wolf Territories features that need a service of their own. If you also see
a warning that private calls are off, check that the region can read the configured
credential key (or relay key as fallback).

### 5. Allow voice on the estate and the parcel

Both are required, and this is the most common reason for "nothing happens":

- Region/Estate → Estate → **Allow Voice Chat**
- About Land → Sound → **Allow Voice Chat**

With either unset the viewer decides locally that voice is unavailable and never asks
the region for anything, so **nothing appears in any log**. The region also refuses
voice itself on such a parcel (and to avatars banned from it).

### 6. Check it

```bash
# from a region host
curl https://voice.example.org:9443/
# ok
curl -H "X-Wolfvoice-Key: $(sudo cat /etc/opensim/voice_rpc_key)" https://voice.example.org:9443/
# {"bridges":0,"npcs":0,"pending":0,"rooms":0,"service":"wolfvoice","sessions":0}
```

Then talk in-world: wolfvoice logs each session it opens (`journalctl -u wolfvoice -f`).

### If it does not work

| You see | Cause and fix |
|---|---|
| Nothing in either log when someone opens voice | Voice is off on the estate or parcel (step 5), or the viewer is older than Firestorm 7.1.10. |
| Region log: `[WOLFVOICE HTTP] refused the voice server's TLS certificate` | `WebRtcVoiceServerURI` uses an IP or a name not on the certificate, or the certificate is self-signed or expired. Use the certificate's host name. |
| wolfvoice log: `refused: wrong or missing relay key` | The region's `RpcKeyFile` differs from wolfvoice's `rpc_key`, is not readable by OpenSim, or the region runs a module without keys (built-in master module, older addon). |
| Private calls disabled | The region cannot read a credential key or fallback relay key. Check CredKeyFile and permissions. Local voice does not require a credential key. |
| Text IMs time out ("session initialization is timed out") | The region runs an addon without the `ChatSessionRequest` fix. Install the branch above (the fix is also in upstream OpenSimulator as [opensim/opensim#69](https://github.com/opensim/opensim/pull/69)). |
| Build: duplicate project names | Two copies of the module: one in `OpenSim/Addons/` and one in `addon-modules/`. Use route B. |
| `./runprebuild.sh: Permission denied` or `$'\r': command not found` | The 0.9.3.0 source archive; run the prebuild command directly (route A, step 2). |
| "voice room is full" / channel full | A private call holds 2, a conference 25, a group or Wolf Chat room 64. |

More in [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md).

## Documentation

| | |
|---|---|
| [docs/SERVER.md](docs/SERVER.md) | Installing, running and operating the service; TLS; firewall; capacity |
| [docs/OPENSIM.md](docs/OPENSIM.md) | The addon, region configuration, estate/parcel flags, rolling out to many regions |
| [docs/SECURITY.md](docs/SECURITY.md) | The relay and credential keys, changing over to two keys, and what the region and wolfvoice enforce |
| [docs/CLIENT.md](docs/CLIENT.md) | Viewer support, and writing your own client |
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | The complete wire contract, with citations into the Firestorm and OpenSim sources |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Every failure we hit, and how to tell them apart |

## Capacity

Measured with `cargo run --release --example load_test` on a 12-core VM:

| Clients | Speaking | Service CPU | Per listener | Frames delivered |
|---|---|---|---|---|
| 10 | 5 | 12 % of one core | 12.3 ms/s | all |
| 40 | 20 | 57 % of one core | 14.2 ms/s | all |
| 120 | 60 | **203 % of one core** | 16.9 ms/s | all, no ticks dropped |

Cost is roughly linear in listeners — the per-listener Opus encode dominates and the
O(N²) frame summing is comparatively free.

The mixer parallelises on both available axes: each room's tick is its own task, and
each *listener within* a room is its own task too. That second axis is the one that
matters, because the interesting case is a single busy region. The 120-listener run
above is the proof: 203 % of one core means the work genuinely spans cores, and every
frame still arrived.

So the first limit you will now meet is the **media port range — 1000 concurrent
sessions**, not CPU (12 cores at ~17 ms per listener is on the order of 700
listeners before saturation). Widen `MEDIA_PORT_LO`/`HI` and your firewall together
if you need more.

If the host ever cannot keep up, a whole 20 ms tick is skipped rather than queued —
audible, but bounded — and the service logs `mixer overloaded` with a count.

## Building from source

Needs a Rust toolchain and `cmake` (the `opus` crate compiles libopus from source).

```bash
cargo build --release
cargo test              # 75 tests, no network required
```

Release binaries for Linux (x86_64 and aarch64, static musl and glibc) are built by
GitHub Actions - see [.github/workflows/release.yml](.github/workflows/release.yml),
which also explains why macOS and Windows are build-from-source only.

## Testing it without a viewer

```bash
# The harnesses present the relay key and sign tickets the way the region does.
export WOLFVOICE_RPC_KEY="$(sudo cat /etc/wolfvoice/rpc_key)"
export WOLFVOICE_CRED_KEY="$(sudo cat /etc/wolfvoice/cred_key)"

# Correctness: two synthetic viewers, real SDP, real Opus, real mixing.
cargo run --release --example two_clients -- https://127.0.0.1:9443
# -> AUDIO_MIXED=true ROSTER_DELIVERED=true LEAVE_NOTIFIED=true / PASS

# Firestorm's exact behaviour (spatial, calls, group rooms, hardening): 34 checks.
cargo run --release --example fs_compat -- https://127.0.0.1:9443
# Multi-party rooms: join order, rejoin, late joiner: 14 checks.
cargo run --release --example room_e2e -- https://127.0.0.1:9443

# Capacity: N clients, M speaking, reports service CPU.
cargo run --release --example load_test -- https://127.0.0.1:9443 40 20
```

Run these on the voice host (the JSON-RPC port is firewalled to region hosts, but
loopback is permitted). Both harnesses share the server's own WebRTC crate, so they
cannot catch a shared misreading of the wire format — a real viewer remains the final
test.

## Known limitations

- **No TURN, and no way to add one.** Firestorm hardcodes its STUN servers to
  `stun:stunN.<grid>.secondlife.io`, which do not resolve outside Second Life. Media
  still connects, because the server advertises a routable host candidate and learns
  the viewer's address from the connectivity check — but a user whose network blocks
  outbound UDP cannot use voice at all. The client is told, rather than failing
  silently.
- **Single instance.** If wolfvoice is down, voice is down everywhere it is
  configured. `Restart=always` covers a crash, not a host failure.
- **Private calls and group voice** (`channel_type: multiagent`) need the hardened
  region module, and have had far less exercise than spatial voice.

## Credits

**By Shadow Wolf and Lone Wolf, Wolf Territories Grid and IntelligentWolf Ltd**
(formerly Wolf Software Systems Ltd), for the
[Wolf Territories Grid](https://wolfterritories.org), and released for the wider
OpenSimulator community.

Optional Wolf Territories extras are in the code but **off** unless you configure them,
and need no Wolf service to stay off: story/NPC voices (`WOLFVOICE_NPC_HOSTS`), a
Discord voice bridge (a bridge key; needs a bridge client that is not part of this
repository) and, in the region module, ad-hoc conferences and Wolf Chat voice rooms
(`ConferenceApiUrl`).

wolfvoice stands on **[os-webrtc-janus](https://github.com/Misterblue/os-webrtc-janus)
by Robert Adams**, which brought WebRTC voice to OpenSimulator and supplies the
region-side capability handlers, provider advertisement and session plumbing this
service depends on. wolfvoice is only a backend for it; the hard integration work with
OpenSimulator is his.

The wire protocol is Linden Lab's, as implemented in
[Firestorm](https://www.firestormviewer.org/).

## Licence

Apache License 2.0 — see [LICENSE](LICENSE).
