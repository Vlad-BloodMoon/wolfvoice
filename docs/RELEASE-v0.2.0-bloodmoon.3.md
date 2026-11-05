# WolfVoice v0.2.0-bloodmoon.3 — group voice deployment package

This release updates the documentation and pairs the existing validated WolfVoice server with the OpenSim group voice invitation modules. **There is no server audio or executable change compared with v0.2.0-bloodmoon.2.** If that server is already installed, keep it running; update only the simulator modules where needed.

## Downloads

- **Voice server:** `wolfvoice-x86_64-linux-musl` and its `.sha256` file attached here. This is the same tested non-AVX-compatible x86-64 binary as v0.2.0-bloodmoon.2.
- **Simulator modules:** [OpenSim WebRTC v0.9.3.1-webrtc.2](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.2).
- **Direct module ZIP:** [webrtc-group-recall-fix.zip](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.2/webrtc-group-recall-fix.zip).
- **ZIP verification:** [SHA256SUMS](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.2/SHA256SUMS).
- **[Complete French installation guide](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/INSTALLATION_FR.md)** and **[English installation/key guide](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/INSTALLATION.md)**.

Server and modules are separate components. No private key is included.

## Validated functionality

The operator has confirmed proximity voice, private IM calls, speaking indicators, per-user volume, and existing OpenSim group calls with invitations, accepting calls and audible participants across simulators.
The regional module also avoids ringing members already connected when another member rejoins the same group.

Ad hoc conferences are outside the scope of this package.

## Upgrade an existing compatible simulator

Back up the current module, replace **only `WebRtcVoiceRegionModule.dll` and its matching `.pdb`** from the module ZIP in `bin/`, then restart the simulator.
Keep your other DLLs, voice INI and existing keys. Both simulators need the updated regional module for cross-simulator invitations.

For a fresh compatible OpenSim 0.9.3.1 / .NET 8 installation, install the three matching DLL/PDB pairs from that ZIP and follow the installation guide with your own hostname and keys. Do not delete `bin/` or replace unrelated libraries.

## Spatial correction retained

The server retains the listener-axis correction, quaternion normalization and non-AVX compatibility validated for v0.2.0-bloodmoon.2. Distance attenuation is unchanged.
See [technical explanation](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/SPATIAL_INVESTIGATION.md).

## Known viewer limitations

- **French group hang-up text:** Firestorm's French `nearby` string incorrectly says “Pas de réponse” even on the normal non-P2P `STATE_HUNG_UP` path. This translation defect has been reported by the operator. It does not mean all group members must answer. No viewer translation patch is included in these modules.
- **Grey hang-up button:** some participants who accept an invitation can have a disabled group IM hang-up button despite working audio. Use the red hang-up button in the voice participants panel via the headset icon. The exact cause is not confirmed.

The module ZIP is the exact tested artifact and still contains its original preview README; the current release notes and guides supersede that embedded README.

## Provenance

- Server binary build: `aa14b9c7bd37f67382b8d59945bda26d86e5975d`.
- Validated server build: https://github.com/Vlad-BloodMoon/wolfvoice/actions/runs/40700492483.
- OpenSim module source: `07fca6e6e50d8998e61f3fb066f39013281216ba`.
- Module ZIP SHA-256: `e66b15d8cda19d76b894a1e033f5c5ba0b225d7ce7c6a481ed3f3269b207055c`.
- The server executable is republished unchanged; the new release tag includes updated documentation.

## Français

Cette release rassemble la documentation à jour et les liens vers les modules d’invitations vocales de groupe. **Le binaire serveur est identique à celui de v0.2.0-bloodmoon.2**, avec les axes corrigés : aucun remplacement ni redémarrage du serveur vocal n’est nécessaire si cette version fonctionne déjà.

Mettre à jour les simulateurs compatibles avec `WebRtcVoiceRegionModule.dll` et son `.pdb`, puis redémarrer les simulateurs. Conserver la configuration, les clés et les autres DLL.
Pour une nouvelle installation, suivre le guide français et installer les trois paires DLL/PDB. Les clés privées ne sont jamais fournies dans les téléchargements publics.
