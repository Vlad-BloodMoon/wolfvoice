# 0.2.0 integration test

This branch merges upstream d9ec3e9ececc6c1f3df8e264dc28032903e436ee into
this fork without changing the deployed grid. Runtime sources use upstream's
implementation of join readiness, primary transitions, failed roster sends,
per-user gain and mute. The previous implementation remains in git history and
the grid-tested v0.1.1-bloodmoon.1 release remains available for rollback.

## Scope

Keep the existing installation, systemd unit, TLS certificates, account and
firewall rules. Do not run either installer as part of this upgrade. The retained
setup.sh still installs the last stable fork release, not this test branch.
Upstream example configuration and service files are reference material only.

## Validation sequence

1. Run cargo test --locked, clippy and build examples in GitHub Actions.
2. Build the three matching WebRTC modules against the OpenSim version used by
   the test simulator. The earlier one-DLL P2P prototype must not be combined
   with these modules.
3. Prefer a separate voice test instance with its own TCP listener and disjoint
   UDP media range. Set WOLFVOICE_BIND, WOLFVOICE_MEDIA_LO and WOLFVOICE_MEDIA_HI;
   this must not change or restart the existing shared voice service.
4. Install identical relay and credential key contents on the test voice host
   and test region host. Use RpcKeyFile and CredKeyFile in the EXISTING dedicated
   WebRTC INI and point only the test simulator at the test endpoint.
5. Validate local audio, participant dots, individual volume/mute, delayed joins,
   Reset Voice, two-way private invitation/accept/decline/hangup, and return to
   local voice. Then validate group voice and calls between two updated sims.
6. Only after validation prepare the shared-service rollout and rollback. A relay
   key on the shared service rejects requests from old modules that send no key:
   do not enable it while unchanged simulators still depend on that service.

Private calls and ordinary group voice use signed credentials. Ad-hoc conferences
and Wolf Chat rooms additionally need ConferenceApiUrl and its external service;
they are not enabled by this integration.

No binary has been deployed by preparing this branch. Automated tests do not
replace the Firestorm tests listed above.
