# Keys, credentials and hardening

> Applies to **wolfvoice 0.2.0** and the matching region module: version 1.2.0 of
> os-webrtc-janus, branch `chatsession-p2p-session-id-and-fast-fail` of
> [intelligentwolf/os-webrtc-janus](https://github.com/intelligentwolf/os-webrtc-janus).
> wolfvoice 0.1.x has no relay key: there, the firewall rule that limits TCP 9443 to
> your region hosts is the only thing protecting the endpoint.
>
> **Mixing versions.** wolfvoice 0.2.0 still serves a region module without keys (an
> older addon, or OpenSim master's built-in copy) for spatial voice, but only while
> wolfvoice itself has no relay key. The hardened module also works with wolfvoice 0.1.x:
> 0.1.x ignores the key it sends, and the position feed pauses itself when the server
> does not know `wolf.positions`.

wolfvoice uses two shared secrets. Keep them different.

| Secret | wolfvoice | Region (`[WebRtcVoice]`) | What it does |
|---|---|---|---|
| **Relay key** | `WOLFVOICE_RPC_KEY` or `/etc/wolfvoice/rpc_key` | `RpcKeyFile` (default `/etc/opensim/voice_rpc_key`) | Authenticates every JSON-RPC call from a region. Without it the region can claim any avatar and region. |
| **Credential key** | `WOLFVOICE_CRED_KEY` or `/etc/wolfvoice/cred_key` | `CredKeyFile` (default `/etc/opensim/voice_cred_key`) | Signs the ticket (`channel_credentials`) that admits one avatar to one call, group or conference room. |

Why two: anything that calls the JSON-RPC needs the relay key, so it tends to end up on
more machines than the regions. The credential key stays with the regions and the voice
server only, so holding the relay key is not enough to mint a ticket into someone's call.

This fork's `setup.sh` does not create keys. Follow [installation and key setup](INSTALLATION.md), which covers both fresh two-key deployments and credential-only rolling upgrades. Generate each new key with
`openssl rand -hex 32`. On the voice server: owner `root`, group
`wolfvoice`, mode `640`. On a region host: readable by the user OpenSim runs as only.

The region re-reads both key files when they change (at most once a minute), so a key can
be installed or replaced without restarting regions. wolfvoice reads its keys at start.

## Setting up from scratch

1. Install the matching modules and prepare the region configuration.
2. Create the server keys using the non-overwriting commands in [INSTALLATION.md](INSTALLATION.md).
3. Transfer the same key contents privately to every authorized region host.
4. Restart wolfvoice and then the regions. For a rolling upgrade with old modules, use credential-only mode until every region can send the relay key.

## Adding the credential key to a grid already running one key

Regions that have not restarted onto the new module sign tickets with the relay key, and
Firestorm keeps a call's ticket for the whole call (it sends it again on every reconnect),
so the change-over is in three steps:

1. Restart every region onto the new module (with only the relay key, nothing changes).
2. Create the credential key on the voice server and restart wolfvoice. While
   `WOLFVOICE_LEGACY_CREDENTIALS` is not `0`, tickets signed with the relay key are still
   accepted. Then copy the key to every region host; regions use it within a minute.
3. A day later, set `WOLFVOICE_LEGACY_CREDENTIALS=0` for wolfvoice and restart it. Regions
   keep accepting both signatures on call invitations from other regions unless
   `AcceptLegacyCallCredentials = false`.

## What the region enforces

- **Voice off is enforced.** A spatial connection is refused (HTTP 401) when the estate does
  not allow voice, or - unless the estate is tax-free - the avatar's parcel does not. These
  are the same rules `ParcelVoiceInfoRequest` uses to tell the viewer.
- **Neighbour connections get the estate room.** A connection from an avatar that is only a
  child agent in the region never picks a parcel's room.
- **Room rights are checked on every connect**, not just when the ticket is issued: group
  membership and the role's voice power, conference membership, Wolf Chat room access,
  and for a private call that the other party is a real avatar. Tickets do not expire on a
  timer, because Firestorm reuses them on reconnect.
- **The voice server's certificate is verified**, chain and host name, even when OpenSim's
  global `NoVerifyCertChain` / `NoVerifyCertHostname` are true (the default). The check
  applies to the wolfvoice URL only. Your `WebRtcVoiceServerURI` must therefore use the
  host name on the certificate, not an IP address.
- **Positions.** Once a second the region tells wolfvoice where its avatars in voice really
  are (`wolf.positions`; nothing is sent while nobody there is in voice). Set
  `PositionFeed = false` to turn it off.

## What wolfvoice enforces

- A session neither hears nor is heard until the viewer has joined on its data channel.
- The viewer's reported position is held to within a few metres (more for a fast-moving
  avatar) of the region's, and the listening position to 50 m of the region's.
- A private call's room is the pair of avatars the ticket was issued for, so a crafted
  avatar id cannot reach someone else's call. Rooms are capped: 2 people for a private
  call, 25 for a conference, 64 for group and Wolf Chat rooms ("voice room is full", which
  the viewer shows as channel full).
- ICE candidates are tried only for public addresses (add your own networks with
  `WOLFVOICE_ICE_ALLOW=10.0.0.0/8,...`), at most 32 per request and 128 per session.
- Session limits (service, per avatar, per room) are claimed atomically; a media port is
  never handed out while it is still in use.
- Viewer-supplied text is escaped before it is logged, and the relay key never is.
- The health endpoint (`GET /`) answers `ok`. With the relay key in an `X-Wolfvoice-Key`
  header it answers the session counts.
