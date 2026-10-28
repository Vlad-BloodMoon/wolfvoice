# Spatial voice correction: listener axes and quaternion normalization

Released in **v0.2.0-bloodmoon.2**, 28 October 2026.
For upstream review by Lone Wolf / the WolfVoice developers.

## Report and field validation

On BloodMoon, the operator reported that following a speaking avatar could make
local voice very quiet or apparently inaudible. Audibility also depended on which
side of the speaker the listener occupied; moving the camera could improve it
even with avatar listening selected.

Source inspection identified the listener-axis error below. The operator installed
the compatible build from commit `aa14b9c7bd37f67382b8d59945bda26d86e5975d`
and reported a successful in-world result on 28 October 2026, then requested release.
This is an operator field report, not a controlled measurement of every viewer mode.

## The coordinate convention

Do not confuse region/world coordinates with the listener's local basis.

| Frame | +X | +Y | +Z |
| --- | --- | --- | --- |
| Region/world | East | North | Up |
| Avatar/camera local | Forward | Left | Up |

Local **right is -Y**, not +X. This convention applies to the OpenSim / Firestorm
path investigated here; there is no special OpenSim-versus-Second-Life axis swap
to apply to these viewer listener messages.

Primary source references:

- [OpenSim ScenePresence direction vectors, pinned OpenSim base](https://github.com/opensim/opensim/blob/eb01e50a678ce69c67bf7f9cd8865a28f7d2224a/OpenSim/Region/Framework/Scenes/ScenePresence.cs#L397-L404):
  forward (+1,0,0), left (0,+1,0), up (0,0,+1).
- [Firestorm LLCoordFrame](https://github.com/FirestormViewer/phoenix-firestorm/blob/master/indra/llmath/llcoordframe.h):
  getAtAxis() is X, getLeftAxis() is Y, getUpAxis() is Z.
- [Firestorm llvoicewebrtc.cpp](https://github.com/FirestormViewer/phoenix-firestorm/blob/master/indra/newview/llvoicewebrtc.cpp):
  updatePosition() selects the listener position/orientation; sendPositionUpdate()
  sends `sp`, `sh`, `lp`, `lh`, quantizing components with integer(value * 100).
- [Firestorm llvoicewebrtc.h](https://github.com/FirestormViewer/phoenix-firestorm/blob/master/indra/newview/llvoicewebrtc.h):
  earLocCamera / earLocAvatar / earLocMixed.

Avatar mode should use avatar position and rotation; mixed mode intentionally uses
avatar position and camera rotation. This patch does not change viewer settings.

## Calculation and the defect

Runtime changes are confined to `src/mixer.rs`; `src/room.rs` changes are tests.

1. Compute the relative world vector: `r = speaker_world - listener_pos`.
2. Normalize the listener quaternion `q=(x,y,z,w)`.
3. Rotate into listener space: `local = conjugate(q) * r * q`.
4. Use its rightward component for stereo pan.

The existing inverse-rotation formula was appropriate for a unit quaternion.
The old pan formula then used:

```text
lateral_old = local.x / sqrt(local.x^2 + local.y^2)
```

It treated the FORWARD component as listener-right. With identity rotation, a
speaker directly ahead was hard right; a speaker behind was hard left. Speakers
directly to either side were centred.

The corrected formula is:

```text
length = hypot(hypot(local.x, local.y), local.z)
lateral = -local.y / length
angle = (clamp(lateral, -1, 1) + 1) * pi/4
left = cos(angle)
right = sin(angle)
```

Coincident positions use lateral=0. For identity rotation:

| Source position relative to listener | Previous L / R | Corrected L / R |
| --- | --- | --- |
| Front (+X) | 0 / 1 | 0.707 / 0.707 |
| Back (-X) | 1 / 0 | 0.707 / 0.707 |
| Left (+Y) | 0.707 / 0.707 | 1 / 0 |
| Right (-Y) | 0.707 / 0.707 | 0 / 1 |

Values are rounded; floating-point cos(pi/2) is approximately zero.

Including Z prevents tiny horizontal offsets near the vertical axis from
producing extreme panning. This remains a stereo panner, not an HRTF renderer:
front/back and elevation are not perceptually distinguished by this model alone.

## Quaternion handling

The viewer truncates quaternion components to hundredths on the wire, so the
received quaternion can have a norm different from one. The vector rotation
formula requires unit length.

The patch divides finite components by their largest absolute value, then
normalizes the scaled quaternion. Scaling avoids overflow for very large finite
values. Zero or non-finite quaternions fall back to identity rotation.

## Scope and limits

- Distance attenuation is unchanged: full gain through 10 m, linear fade to zero at 60 m.
- Per-user gain, mute, participant dots, private calls, credentials and protocol are unchanged.
- No speaker-facing directivity is introduced. Turning only the speaker's orientation
  should not change these pan weights at fixed positions.
- Both the previous and corrected constant-power laws satisfy L^2 + R^2 = 1.
  The axis error proves incorrect stereo direction; it alone does not mathematically
  explain total loss in both channels. The positive field result does not establish
  the mechanism behind every earlier camera/dropout symptom.
- No new OpenSim DLL, INI setting, key, port or firewall rule is needed for this patch.
- Region-fed positions remain an independent feature requiring relay authentication.
  This release does not enable a relay key on existing installations.

## Regression evidence

`.github/workflows/spatial-geometry.yml` first reproduces the front-source failure
against the previous mixer, then passes all 16 corrected geometry/mixer tests.
Coverage includes front/back, left/right, yaw, pitch, hundredth quantization,
elevation, constant stereo power, translation, and invalid/large quaternions.

The full server suite passes 76 tests, clippy with warnings denied, and example
builds. A room-level regression follows a speaking avatar at a constant 3 m
through changing global coordinates, checking centred audio and stable stereo
energy. Speaker rotation changes independently.

## Separate binary compatibility correction

The first candidate binary failed on a KVM VM without AVX, before spatial mixing:
SIGILL at `ring_core_0_17_14__OPENSSL_cpuid_setup+0x96`, instruction
`vmovd %eax,%xmm0`. This was a binary compatibility problem, not a spatial formula
failure. The VM did not need a CPU-model or Proxmox configuration change.

The released build explicitly targets baseline x86-64 for Rust and native C/C++:
`-C target-cpu=x86-64` and
`-march=x86-64 -mtune=generic -mno-avx -mno-avx2 -mno-sse2avx`.
Runtime-dispatched optimized routines may still exist; the requirement is that
unsupported instructions are not executed on baseline CPUs.

Before packaging, CI starts the exact release binary under `qemu-x86_64 -cpu qemu64`
(without AVX), performs a TLS handshake, sends a JSON-RPC probe and verifies its
response. The operator also confirmed this binary starts on the actual VM.

[Validated build and no-AVX smoke test](https://github.com/Vlad-BloodMoon/wolfvoice/actions/runs/40700492483).
Binary build commit: `aa14b9c7bd37f67382b8d59945bda26d86e5975d`.
The release promotes that artifact unchanged.

## Reviewing or porting upstream

For the functional fix, review `src/mixer.rs`, the updated mixer unit tests,
`tests/spatial_geometry.rs`, and the two room-level regression changes in
`src/room.rs`. Build workflow changes address CI dependencies and baseline CPU
compatibility separately.

[PR with the complete diff](https://github.com/Vlad-BloodMoon/wolfvoice/pull/3).
The fix is independent of BloodMoon branding and needs no grid-specific constants.
