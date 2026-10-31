# WolfVoice : téléchargements et installation en français

**Version conseillée : v0.2.0-bloodmoon.2**, avec le correctif du son spatial.
Aucune compilation n'est nécessaire pour les fichiers ci-dessous.

## Vous installez votre propre serveur vocal pour votre grille ?

Commencez par le **[guide d'installation pas à pas](INSTALLATION_FR.md)**.
Un seul parcours : serveur Linux, deux clés, port 9443 avec authentification RPC,
puis DLL et deux fichiers de clés dans le dossier bin de chaque simulateur.
Il ne nécessite pas de liste d'IP autorisées.

| À télécharger | Usage |
| --- | --- |
| [Serveur WolfVoice Linux x86-64 compilé](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.2/wolfvoice-x86_64-linux-musl) | Sur la machine qui héberge la voix |
| [Somme de contrôle du serveur](https://github.com/Vlad-BloodMoon/wolfvoice/releases/download/v0.2.0-bloodmoon.2/wolfvoice-x86_64-linux-musl.sha256) | Vérifier le téléchargement |
| [Modules WebRTC OpenSim 0.9.3.1 : ZIP compilé](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip) | Sur chaque simulateur |
| [Somme de contrôle du ZIP](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip.sha256) | Vérifier le ZIP |
| [Modèle wolfvoice.ini pour une nouvelle grille](../contrib/wolfvoice-fresh-grid.ini.example) | À adapter avec votre nom de serveur |

Le ZIP des modules fournit les trois DLL et leurs PDB :
WebRtcVoice, WebRtcVoiceRegionModule et WebRtcVoiceServiceModule.
Il ne contient pas un simulateur complet ni de clé privée.
Les DLL sont compilées contre OpenSim 0.9.3.1 (.NET 8) ; pour une autre version
ou une distribution modifiée, vérifier la compatibilité avant remplacement.

Le serveur WolfVoice est un programme Linux x86-64 ; les DLL se placent dans
OpenSim. Ce sont deux parties différentes. Aucun composant vocal supplémentaire
n'est à installer sur l'ordinateur des résidents : utiliser un viewer compatible WebRTC.

## Vous raccordez seulement un simulateur à une grille équipée ?

Demandez à l'administrateur de cette grille son URL vocale, sa configuration,
les clés nécessaires et ses conditions d'accès au serveur.
Vous n'avez pas besoin d'installer votre propre serveur WolfVoice.
Les clés d'une autre grille ne doivent jamais être réutilisées pour votre serveur.

## Quelle version choisir ?

- **v0.2.0-bloodmoon.2 : version actuelle conseillée**, axes corrigés, orientation
  normalisée et binaire compatible avec les processeurs x86-64 sans AVX.
- v0.2.0-bloodmoon.1 : ancienne version, sans le correctif des axes.
- Les DLL restent en v0.9.3.1-webrtc.1 : le correctif des axes se trouve côté serveur.
- Les fichiers GitHub « Source code (zip/tar.gz) » sont les sources, pas les
  programmes compilés. Utiliser les liens du tableau pour les binaires.

[Release du serveur](https://github.com/Vlad-BloodMoon/wolfvoice/releases/tag/v0.2.0-bloodmoon.2)
· [Release des modules](https://github.com/Vlad-BloodMoon/opensim/releases/tag/v0.9.3.1-webrtc.1)
· [Explication du correctif spatial](SPATIAL_INVESTIGATION.md)

Le code source reste disponible dans les deux dépôts. Le moteur WolfVoice est
développé par Shadow Wolf, Lone Wolf et les contributeurs du projet.
