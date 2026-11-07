# BloodMoon WolfVoice — deployment and integration

**Current validated pairing (7 November 2026):** [WolfVoice server `v0.2.0-bloodmoon.4`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.4) and [OpenSimulator WebRTC modules `v0.9.3.1-webrtc.3`](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3) for OpenSimulator **0.9.3.1 / .NET 8**.

**Compatible server:** **BloodMoon's modified WolfVoice server**. Stock upstream WolfVoice does not yet include the complete BloodMoon spatial-axis/group/conference integration. This backend does **not require a Janus gateway** but **does** require matching regional modules.

## Existing BloodMoon server

The `.4` executable is **byte-identical** to the field-tested `.2/.3` version. If your deployed voice service already uses that validated binary, **do not replace or restart it merely to add ad-hoc conferences**. Preserve the existing TLS, ports, key files, firewall/allowlist, service identity, resource limits and unit.

**Do not rerun the fresh-install `setup.sh` on a production server.** It is intended for a fresh server and does not install regional modules or the conference PHP/MySQL registry.

## Simulator upgrade

Use the complete [WebRTC module ZIP `v0.9.3.1-webrtc.3`](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.3/WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip) (also attached to WolfVoice `.4`). It contains matching `WebRtcVoice`, `WebRtcVoiceRegionModule` and `WebRtcVoiceServiceModule` DLL/PDB pairs, plus documentation, checksum and provenance files.

1. Stop the selected simulator, back up its module DLL/PDB files and dedicated voice INI.
2. For a fresh installation install **all three matching DLL/PDB pairs** into OpenSimulator `bin/`. For an existing installation on the same compatible baseline, replacing only `WebRtcVoiceRegionModule.dll` and its `.pdb` may suffice; retain the other matching modules.
3. Preserve `WebRtcVoiceServerURI`, `RpcKeyFile`, `CredKeyFile`, old keys, and other voice settings.
4. For **BloodMoon-approved** simulators, add in `[WebRtcVoice]`:

   ```ini
   ConferenceApiUrl = https://bloodmoonpack.com/adhocconf/wolf_api.php
   ```

5. Restart this simulator. **Repeat on every simulator** whose residents use AdHoc conferences; having the correct DLL but **omitting `ConferenceApiUrl`** on another simulator prevents invitations and IM text from arriving there.
6. Test local spatial voice, private calls, OpenSim group voice, ad-hoc multi-avatar text IMs, optional conference voice, individual hang-up/rejoin, and inter-simulator routing.

The hosted BloodMoon conference API is **restricted to explicitly trusted/allowed simulators**. The credential key is shared privately and **not included** in any public downloads. Operators of independent grids must deploy/adapt their own [conference API/MySQL backend](https://github.com/Vlad-BloodMoon/OpenSimulator-AdHocConferences) and evaluate shared-secret security before inviting outside operators.

## Provenance and rollback

- [WolfVoice binary provenance](RELEASE-v0.2.0-bloodmoon.4.md): same tested axes-fixed Linux x86-64 binary as `.2/.3`.
- [OpenSim `v0.9.3.1-webrtc.3` source and binary](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3): complete modules, including the **exact AdHoc regional DLL tested in Firestorm**.
- [AdHoc source, secure API and installation docs](https://github.com/Vlad-BloodMoon/OpenSimulator-AdHocConferences): private signing key must never be copied to public repositories.
- To roll back a simulator, stop it and restore the backed-up compatible DLL/PDB files and `wolfvoice.ini`; restart that simulator. No server rollback should be needed for a module-only upgrade.

Further installation steps: [Français](INSTALLATION_FR.md) · [English](INSTALLATION.md).
