# WolfVoice v0.2.0-bloodmoon.2 — spatial voice correction

Released 28 October 2026. Based on upstream WolfVoice 0.2.0.

## Fixed

- Correct listener stereo axes: local +X is forward, +Y is left, and -Y is right.
  A speaker directly ahead is now centred instead of hard right.
- Normalize the viewer's quantized orientation quaternion before inverse rotation.
- Account for elevation in direction length, avoiding extreme pan near overhead.
- Restore baseline Linux x86-64 compatibility, including KVM CPUs without AVX.

The BloodMoon operator reported successful in-world results after installing this
build. The previous candidate's SIGILL startup failure is also resolved.
Local voice, participant dots, per-user volume, private calls and credential handling
are retained from the preceding 0.2.0 release. Group voice remains implemented but
is not covered by the operator's reported tests; external ad-hoc conference services
are not enabled by this release.

## Download and upgrade

Download `wolfvoice-x86_64-linux-musl` and its `.sha256` file from this release.
This release publishes only the validated Linux x86-64 static binary.

Verify with `sha256sum -c wolfvoice-x86_64-linux-musl.sha256`.
Back up the currently running binary, install the new binary with executable
permissions, then restart your WolfVoice service during a planned voice interruption.

Keep your existing systemd unit, account, TLS certificates, credential/relay keys,
ports, firewall allowlist and limits. Existing installations do not need the
installer, new OpenSim DLLs, INI edits or new keys for this spatial correction.

The public assets contain no grid credentials.

## Evidence and provenance

- Geometry regression reproduces the original front-source failure; all 16 corrected
  geometry/mixer tests pass.
- Full server CI: 76 tests, clippy and example builds passed.
- The exact published binary passed TLS startup and JSON-RPC under QEMU qemu64 without AVX,
  then started successfully on the operator's KVM VM.
- Binary build commit: `aa14b9c7bd37f67382b8d59945bda26d86e5975d`.
- Validated build: https://github.com/Vlad-BloodMoon/wolfvoice/actions/runs/40700492483
- This release promotes the tested artifact unchanged; the release tag also includes
  the final documentation and publication metadata.

[Technical explanation for upstream developers](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/SPATIAL_INVESTIGATION.md).
The attached `SPATIAL_FIX.md` contains the same explanation.
Distance attenuation is unchanged. Correct stereo direction is established by tests;
the field report does not prove a cause for every possible camera or audio dropout.
