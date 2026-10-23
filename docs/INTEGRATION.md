# WolfVoice v0.2.0-bloodmoon.1

For exact installation, key creation, transfer and restart commands, see [INSTALLATION.md](INSTALLATION.md).

Based on intelligentwolf/wolfvoice d9ec3e9ececc6c1f3df8e264dc28032903e436ee.
Runtime sources are unchanged from that upstream version; this release integrates and documents it in this fork.

## Confirmed on the deployed grid
- Local voice, participant dots and per-user volume.
- Private voice calls from IM; invitation and audio confirmed by the operator.
- Hosted simulators updated and restarted.

Group voice is implemented upstream but not yet confirmed in these operator tests. Ad-hoc conferences need an additional external service and are not enabled here.

## Existing installation
Keep your current service, TLS files, user, firewall/IP restrictions and resource limits. Back up and replace only the voice binary, then restart the service during a planned interruption.

The matching three WebRTC region DLLs and PDBs are available in Vlad-BloodMoon/opensim release v0.9.3.1-webrtc.1. Do not use the earlier single-DLL prototype.

A shared server can retain local voice for old region modules while no relay key is configured. A separate credential key enables signed calls with updated modules: server /etc/wolfvoice/cred_key, region CredKeyFile pointing to the same secret contents. Enabling the relay key requires every client region to send it; otherwise its requests are refused.

The public release contains no grid key, private configuration or credentials.
Binary build source: d2d2670181b12354d51d83611ae5dbf36b822586 (GitHub Actions run 39933496255).
Previous release v0.1.1-bloodmoon.1 remains available for rollback.

The server binary uses the existing listener and media ports. On the deployed grid, only the credential key was enabled during rollout; IP allowlisting remains in place. Do not publish the shared credential key. Paths and server addresses in public examples must be adapted by each operator.
