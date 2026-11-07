# WolfVoice BloodMoon — téléchargements vérifiés

**Version complète recommandée : `v0.2.0-bloodmoon.4` (serveur) + `v0.9.3.1-webrtc.3` (modules OpenSimulator 0.9.3.1 / .NET 8).**

**Attention :** ces fonctionnalités ont été testées **sur le serveur WolfVoice modifié de BloodMoon**. Le serveur WolfVoice d'origine n'a pas encore tous nos correctifs des axes et fonctionnalités de groupe/conférence. Les conférences IM à plusieurs avatars nécessitent en plus l'API centrale PHP/MySQL BloodMoon. **Notre API hébergée n'est accessible qu'aux simulateurs autorisés** ; les clés ne sont jamais distribuées publiquement.

| Téléchargement | Où l'utiliser |
| --- | --- |
| [Serveur WolfVoice BloodMoon, Linux x86-64](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/wolfvoice-x86_64-linux-musl) | Serveur vocal |
| [SHA-256 du binaire serveur](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/wolfvoice-x86_64-linux-musl.sha256) | Contrôle avant installation |
| [**Pack complet WebRTC OpenSim 0.9.3.1 — 3 DLL/PDB**](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip) | **Chaque simulateur** |
| [SHA-256 du ZIP de modules](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/WebRTC_Modules_BloodMoon_0.9.3.1-webrtc.3.zip.sha256) | Contrôle du paquet |
| [Release source des modules OpenSimulator `.3`](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.3) | Sources et historique |
| [API AdHoc et documentation](https://github.com/Vlad-BloodMoon/OpenSimulator-AdHocConferences/releases/tag/v0.1.0) | Service central de conférences ; installation distincte |
| [Fichier exemple WolfVoice](../contrib/wolfvoice-fresh-grid.ini.example) | Exemple à adapter, sans clé privée |
| [Installateur complet avec scripts TLS — `.tar.gz`](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.4/wolfvoice-installation-bloodmoon.4.tar.gz) | **Nouvelle installation serveur uniquement** |

## Le paquet complet des modules

Il inclut les paires `WebRtcVoice.dll/.pdb`, `WebRtcVoiceRegionModule.dll/.pdb` et `WebRtcVoiceServiceModule.dll/.pdb`, plus un modèle `wolfvoice.ini.example`, une notice d'installation, un journal de provenance et des sommes SHA-256. La DLL régionale est **la version exacte validée dans Firestorm** pour les conférences AdHoc sur deux simulateurs.

Pour une nouvelle installation, sauvegarder les fichiers existants et installer **les trois paires** dans `bin/`. Pour une installation déjà parfaitement compatible avec la même base, seule la paire `WebRtcVoiceRegionModule.dll/.pdb` peut être remplacée. Ne pas toucher aux autres DLL OpenSimulator.

**Sur chaque simulateur** autorisé, ajouter dans `[WebRtcVoice]` (sans retirer les anciennes options) :

```ini
ConferenceApiUrl = https://bloodmoonpack.com/adhocconf/wolf_api.php
```

Le service central BloodMoon vérifie les simulateurs autorisés et utilise des clés existantes partagées. Une installation indépendante doit disposer de sa **propre API**, de son propre serveur WolfVoice compatible et de son propre modèle de confiance.

## Fonctionnalités validées

Voix locale avec axes corrigés, indicateurs de parole, volume individuel, appels privés IM, appels vocaux des groupes OpenSimulator, conférences écrites **à plusieurs avatars**, voix de conférence facultative, invitations entre simulateurs et raccrochage/reconnexion individuel sans expulser les autres.

## Si ton serveur fonctionne déjà

Le binaire `.4` est **identique** à celui des releases BloodMoon `.2` et `.3`. **Ne redémarre pas et ne remplace pas le serveur** uniquement pour ces nouvelles fonctions : mets à jour les DLL et le fichier INI de **chaque simulateur**. Ne relance pas `setup.sh` sur un serveur existant.

## Installation et documentation

[Guide pas à pas français](INSTALLATION_FR.md) · [Guide anglais](INSTALLATION.md) · [Notes de la release `.4`](RELEASE-v0.2.0-bloodmoon.4.md) · [Détail du correctif spatial](SPATIAL_INVESTIGATION.md).

Merci à **Lone Wolf de WolfGrid** pour les fondations de WolfVoice. Cette édition est une intégration spécifique à BloodMoon.
