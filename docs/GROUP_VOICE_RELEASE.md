> **Archive / historical notes only.** For the latest BloodMoon-modified WolfVoice integration with **OpenSimulator group voice and AdHoc multi-avatar text/voice conferences**, use [WolfVoice `v0.2.0-bloodmoon.4`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.4) with [OpenSimulator modules `v0.9.3.1-webrtc.3`](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3). This older technical text describes a previous stage or generic upstream integration; do not use its earlier module download links for a new BloodMoon deployment. The BloodMoon API is restricted to approved hosts.

# OpenSim WebRTC — Group voice invitations

Release: v0.9.3.1-webrtc.2
Source commit: `07fca6e6e50d8998e61f3fb066f39013281216ba`

## What works

Voice invitations for existing OpenSim groups, including cross-simulator calls, accepting invitations, two-way audio, leaving and rejoining an active call. Rejoining does not ring members already connected to the same group call.

The operator has confirmed successful group audio. This release does not implement ad hoc conferences.

## Downloads and installation

[Download the verified module ZIP](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.2/webrtc-group-recall-fix.zip)

[ZIP checksum](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.2/SHA256SUMS)

ZIP SHA-256:
`e66b15d8cda19d76b894a1e033f5c5ba0b225d7ce7c6a481ed3f3269b207055c`

The ZIP contains three matching DLL/PDB pairs. They target OpenSim 0.9.3.1 / .NET 8.

**Existing compatible installation:** back up the current regional DLL/PDB, replace only `WebRtcVoiceRegionModule.dll` and its matching `.pdb` in `bin/`, then restart the simulator. Keep all other DLLs, configuration and keys.

**Fresh installation:** install all three matching DLL/PDB pairs, configure your own WolfVoice URL and keys, then restart. Do not replace unrelated OpenSim libraries or delete `bin/`.

Both regions must have the updated regional module for cross-simulator invitations. No WolfVoice server update is required.

[French installation guide](https://github.com/Vlad-BloodMoon/wolfvoice/blob/main/docs/INSTALLATION_FR.md)

The WolfVoice server remains **v0.2.0-bloodmoon.2**, with the spatial-axis correction and non-AVX x86-64 compatibility. This module release does not alter that binary.

## Known viewer limitations

- **French hang-up text:** Firestorm's French `nearby` translation says “Pas de réponse” on the normal non-P2P `STATE_HUNG_UP` path. Its English counterpart describes leaving the channel. This is a translation defect reported to Firestorm by the operator; it does not mean that all group members must answer. No viewer translation change is included in this package.
- **Disabled hang-up button:** some invited participants can have a grey hang-up button in their group IM window despite working audio. Use the red hang-up button in the voice participants panel, accessible via the headset icon. The exact cause is not yet confirmed.

The ZIP is the unchanged tested build and contains its original preview README. These updated release notes supersede that README for release status and the explanation of the French message.

## Français

Les appels des groupes OpenSim existants fonctionnent, avec invitations entre simulateurs et correction des rappels qui faisaient ressonner les participants déjà connectés.

Pour mettre à jour une installation compatible, remplacer uniquement `WebRtcVoiceRegionModule.dll` et son `.pdb`, puis redémarrer le simulateur. Pour une nouvelle installation, utiliser les trois paires DLL/PDB et configurer son propre serveur et ses clés.

Le faux message « Pas de réponse » au raccrochage vient de la traduction française de Firestorm. Le bouton Raccrocher parfois grisé chez les invités est un problème distinct encore non résolu ; utiliser le bouton rouge dans la liste des participants via le casque.

Le serveur WolfVoice et son correctif des axes restent inchangés. Les conférences ad hoc ne sont pas ajoutées par cette release.
