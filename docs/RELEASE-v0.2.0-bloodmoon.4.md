# WolfVoice v0.2.0-bloodmoon.4 — BloodMoon complete WebRTC voice + ad-hoc conferences

**BloodMoon-specific release for OpenSimulator 0.9.3.1 / .NET 8.** Includes the complete, tested simulator module package `v0.9.3.1-webrtc.3` and updated installer/documentation.

> **Compatibility:** This fork's voice backend is **BloodMoon's modified WolfVoice server** with the corrected spatial listening axes. The complete group-invitation and multi-avatar IM/voice conference features depend on **BloodMoon's compatible OpenSimulator modules and the separate BloodMoon conference PHP/MySQL API**. Upstream/stock WolfVoice does **not** yet include the complete BloodMoon changes. Do not present this as a generic upstream WolfVoice or Janus release.

## Downloads

| Component | Release file |
| --- | --- |
| **Voice server**, Linux x86-64 static | [`wolfvoice-x86_64-linux-musl`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/wolfvoice-x86_64-linux-musl) and [SHA-256](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/wolfvoice-x86_64-linux-musl.sha256) |
| **Complete OpenSimulator 0.9.3.1 modules**, three DLL/PDB pairs | [`WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip) and [SHA-256](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip.sha256) |
| Source module release | [Vlad-BloodMoon/opensim `v0.9.3.1-webrtc.3`](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3) |
| **Ad-hoc conference API** source and dedicated documentation | [OpenSimulator-AdHocConferences `v0.1.0`](https://github.com/Vlad-BloodMoon/OpenSimulator-AdHocConferences/releases/tag/v0.1.0) |
| New installations | [French installation](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/INSTALLATION_FR.md) · [English installation](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/INSTALLATION.md) · [downloads FR](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/TELECHARGEMENTS_FR.md) |

**The server executable is intentionally byte-identical to the successful `v0.2.0-bloodmoon.3` / `.2` build.** No new media-server binary is required for the ad-hoc feature; **simulator module and central conference API** provide the new behavior. If your BloodMoon server is already functioning with that executable, **do not replace/restart it just to upgrade region modules**.

## Maintenance refresh — same tag and download links (7 November 2026)

The simulator module archives are refreshed **without changing the release name or ZIP filename** to carry a small conference notification update:

- The sender of conference system notices is now **OpenSimulator**, not Wolf Territories.
- The separate PHP conference registry permits re-inviting recipients still marked `invited` after a disconnection/relogin, without ringing members still marked `joined`.
- **Important**: this is a focused maintenance build, not a new end-to-end field validation. If the first reconnect does not reset the avatar's state from `joined`, the reinvitation issue can still occur. Testing with real simulator sessions is required.
- The PHP fix must be **deployed separately** to the BloodMoon server as `wolf_api.php`; replacing DLLs alone does not update the PHP backend.
- The main WolfVoice server executable is **unchanged**. The updated module ZIP and SHA-256 values will be different, though release URLs stay the same.

## Confirmed functionality on BloodMoon (7 November 2026)

- Local spatial/proximity audio with corrected listener axes, participant dots and per-user volume.
- Private voice calls from one-to-one IM.
- **OpenSimulator group voice**: cross-simulator invitations and member rejoin behavior.
- **Ad-hoc multi-avatar IM conferences**: invitations and text both on one sim and across separate simulators, no sender-message duplicates.
- **Optional ad-hoc voice conference**: participants can remain in the text conversation, join an audio call, hang up independently and join again without disconnecting everyone else or generating false leave/rejoin notifications.

The above has been validated on BloodMoon. These features have **not** been independently tested against arbitrary OpenSimulator forks/viewers or the unmodified upstream WolfVoice server.

## Installation on a BloodMoon-authorized simulator

1. Stop the simulator; back up its existing voice DLL/PDB files and `wolfvoice.ini`.
2. Install the **three matching DLL/PDB pairs** from `WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip` into the simulator's `bin/`. Keep unrelated modules and the current server URI/key files.
3. Add to the simulator's existing `[WebRtcVoice]` configuration:

   ```ini
   ConferenceApiUrl = https://bloodmoonpack.com/adhocconf/wolf_api.php
   ```

4. Restart the simulator and verify proximity voice, private voice, OpenSim group voice, a multi-avatar **text** conference and optional conference call.
5. Repeat the DLL + INI update on **every participating simulator**. Missing `ConferenceApiUrl` on a destination simulator prevents cross-simulator conference invitations.

**Do not redistribute BloodMoon's private keys.** Its hosted conference API is restricted to authorized trusted simulators and shared credential-key material. It is not an unauthenticated public service. Self-hosted deployments need a compatible PHP/MySQL registry and trust model. See the [AdHoc installation and security guides](https://github.com/Vlad-BloodMoon/OpenSimulator-AdHocConferences).

## Installer and deployment

- **New server installation:** download and review `setup.sh` from the current `main` branch; the pinned release is `v0.2.0-bloodmoon.4`. This helper installs the **server**, not the simulator DLLs or conference PHP API. TLS and signing keys require the documented setup steps.
- **Existing functioning server:** **do not run `setup.sh` again**; if its binary matches the preceding validated release, just update simulator modules and `ConferenceApiUrl`.
- No live keys, credentials, production INI, database password or private service configuration are attached to this release.

## Provenance and verification

- Server binary reused, unchanged, from [`v0.2.0-bloodmoon.3`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.3); original server source build `aa14b9c7bd37f67382b8d59945bda26d86e5975d`, tested under QEMU without AVX and validated on BloodMoon.
- Complete module ZIP reused **unchanged** from [OpenSimulator `v0.9.3.1-webrtc.3`](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3), validated GitHub Actions [run](https://github.com/Vlad-BloodMoon/opensim/actions/runs/42399259271).
- Regional `WebRtcVoiceRegionModule.dll` inside that ZIP is the exact BloodMoon-tested AdHoc `v0.1.0` binary, derived from its successful CI run #30.
- Both files are accompanied by published SHA-256 checksums; the release workflow verifies source checksums and recalculates public checksums.

## Credit

Thanks to **Lone Wolf of WolfGrid / WolfVoice** and the original WolfVoice contributors, OpenSimulator and Firestorm communities, and the BloodMoon testers. This is a BloodMoon-specific integration, not an upstream WolfVoice release.
