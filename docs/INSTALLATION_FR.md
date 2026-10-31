# Installer WolfVoice sur sa propre grille — guide français

[Retour aux téléchargements](TELECHARGEMENTS_FR.md)

Ce parcours concerne **une nouvelle installation** sur un serveur Ubuntu 24.04
Linux x86-64 avec systemd, et des simulateurs OpenSim 0.9.3.1 / .NET 8.
Il installe **v0.2.0-bloodmoon.2**, avec le correctif des axes, sans compilation.

Si la voix fonctionne déjà, suivre plutôt [la mise à jour en fin de page](#mise-à-jour-dun-serveur-déjà-installé).
Ne pas lancer l'installateur pour mettre à jour un service personnalisé.

## Le fonctionnement choisi

Ce guide suit un seul mode : **deux clés partagées avec les simulateurs**, sans
liste d'IP autorisées. Le serveur reçoit les requêtes sur TCP 9443, sous TLS,
et refuse les requêtes vocales sans la bonne clé RPC. Le port est ouvert à
l'étape 5, après activation et vérification de cette authentification.

## 1. Préparer les deux informations

Noter :
- l'adresse IP publique du serveur vocal ;
- un nom DNS, par exemple `voice.votre-grille.fr`, pointant vers cette IP.

Le nom et les IP sont ceux de **votre grille**, pas ceux de BloodMoon.
Le parcours simple ci-dessous suppose le port TCP 80 disponible pour Certbot.
Si Apache/Nginx occupe déjà ce port, conserver votre méthode de certificats et
préparer les fichiers TLS avant l'installation, avec l'aide de votre administrateur.
Ne pas arrêter votre site web pour suivre ce guide.

## 2. Préparer le réseau

Sur le pare-feu du serveur, et celui de l'hébergeur s'il existe :
- **TCP 9443** : le garder fermé depuis Internet pour le moment ; il sera ouvert
  à tous les simulateurs à l'étape 5, après vérification de la clé RPC ;
- **UDP 40000 à 40999** : autoriser les connexions des viewers ;
- **TCP 80** : doit être accessible pour l'émission et le renouvellement du
  certificat Let's Encrypt dans ce parcours.

Si le serveur est derrière un routeur, rediriger ces ports vers lui.
Ne pas modifier les ports ROBUST ou OpenSim pour installer la voix.

Exemple pour un pare-feu UFW **déjà actif** :

```bash
sudo ufw allow 40000:40999/udp
sudo ufw allow 80/tcp
sudo ufw status
```

Vérifier que 9443 reste fermé à ce stade, y compris sur le pare-feu de
l'hébergeur. Si une ancienne règle l'ouvre déjà, la corriger avant de continuer.
Ne pas activer ou réinitialiser UFW à distance sans préserver votre accès SSH.

## 3. Installer le serveur vocal

**Sur la machine du serveur vocal**, télécharger le dossier d'installation
complet pour inclure les scripts de renouvellement TLS. On ne compile rien.

```bash
(
set -e
sudo apt-get update
sudo apt-get install -y curl ca-certificates openssl python3
mkdir -p "$HOME/wolfvoice-installation"
cd "$HOME/wolfvoice-installation"
curl -fL https://github.com/Vlad-BloodMoon/wolfvoice/archive/refs/tags/v0.2.0-bloodmoon.2.tar.gz   -o wolfvoice-installation.tar.gz
tar -xzf wolfvoice-installation.tar.gz
cd wolfvoice-0.2.0-bloodmoon.2
less setup.sh
sudo bash setup.sh
)
```

Dans `less`, appuyer sur **q** pour revenir à la commande suivante.
L'installateur demande :
1. l'IP publique à annoncer aux viewers ;
2. le nom DNS vocal, par exemple `voice.votre-grille.fr`.

Il télécharge et vérifie le binaire compilé, crée le compte système
`wolfvoice`, prépare le certificat et démarre `wolfvoice.service`.
Il ne crée pas les clés et ne configure pas le pare-feu initial.
Son récapitulatif contient des conseils génériques sur une liste d'IP : pour
le mode à deux clés retenu ici, appliquer les étapes 4 et 5 ci-dessous.

## 4. Créer les deux clés de votre nouvelle grille

Ce parcours utilise deux clés : l'une autorise les requêtes des simulateurs,
l'autre signe les accès aux appels. Tous les simulateurs utilisant ce nouveau
serveur devront recevoir les modules de l'étape 6 et ces mêmes clés.

**Sur le serveur vocal :**

```bash
sudo bash <<'KEYS'
set -euo pipefail
install -d -o root -g wolfvoice -m 750 /etc/wolfvoice
for key in cred_key rpc_key; do
    path="/etc/wolfvoice/$key"
    if [ -e "$path" ]; then
        test -s "$path" || { echo "Fichier existant vide : $path"; exit 1; }
        echo "Clé existante conservée : $path"
    else
        umask 077
        openssl rand -hex 32 > "$path"
        echo "Clé créée : $path"
    fi
    chown root:wolfvoice "$path"
    chmod 640 "$path"
done
KEYS
```

La commande conserve les clés existantes et n'affiche pas leur contenu.
Ne pas générer de nouvelles clés sur chaque simulateur.

Pour préparer leur transfert privé sous votre compte utilisateur :

```bash
(
set -e
mkdir -p "$HOME/wolfvoice-keys"
chmod 700 "$HOME/wolfvoice-keys"
sudo install -o "$(id -un)" -m 600 /etc/wolfvoice/cred_key   "$HOME/wolfvoice-keys/voice_cred_key"
sudo install -o "$(id -un)" -m 600 /etc/wolfvoice/rpc_key   "$HOME/wolfvoice-keys/voice_rpc_key"
)
```

Avec SFTP/SCP, transférer ces deux fichiers dans le dossier **bin** de chaque
simulateur autorisé. Garder exactement les noms `voice_cred_key` et
`voice_rpc_key`, sans extension `.txt`.
Seul le compte qui exécute OpenSim doit pouvoir les lire.
Ne pas les publier dans un ZIP public, GitHub ou une capture d'écran.
Supprimer les copies temporaires de transfert après vérification.

Chaque simulateur doit utiliser les DLL de l'étape 6 et les deux clés.
Ce guide ne modifie pas une installation existante fonctionnant avec une seule clé.

## 5. Activer l'authentification, puis ouvrir le port 9443

```bash
sudo systemctl restart wolfvoice.service
sleep 5
sudo systemctl status wolfvoice.service --no-pager -l
sudo journalctl -u wolfvoice.service -n 25 --no-pager
```

Attendre un service `active (running)` et une ligne
`wolfvoice listening on 0.0.0.0:9443 (TLS)`.
Le message indiquant que le pont Discord est désactivé est normal.
Si les logs indiquent `no relay key`, **ne pas ouvrir 9443** : vérifier que
`/etc/wolfvoice/rpc_key` existe, n'est pas vide et est lisible par le groupe
`wolfvoice`, puis redémarrer le service.

Vérifier ensuite qu'une requête sans clé est refusée. La commande demande votre
nom DNS vocal et contacte le serveur localement avec son certificat TLS normal :

```bash
(
set -euo pipefail
read -r -p "Nom DNS du serveur vocal : " voice_dns
curl --silent --show-error --fail \
  --resolve "$voice_dns:9443:127.0.0.1" \
  "https://$voice_dns:9443/" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"provision_voice_account_request","params":{}}' \
  | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r.get("error")=="not authorised", r; print("OK : requete sans cle RPC refusee")'
)
```

Le résultat doit être `OK : requete sans cle RPC refusee`. Ce refus confirme
l'authentification ; le démarrage du service seul ne suffit pas.

Après ce résultat, ouvrir **TCP 9443** sur le pare-feu du serveur et celui de
l'hébergeur, sans liste d'IP. Pour UFW déjà actif :

```bash
sudo ufw allow 9443/tcp
sudo ufw status
```

Ne pas ouvrir d'autres ports de signalisation. Les requêtes vocales restent
protégées par la clé RPC ; la connexion seule et la page de santé publique ne
constituent pas une autorisation d'utiliser le service.

## 6. Installer les DLL sur chaque simulateur

Télécharger :
- [WebRTC_Modules_0.9.3.1.zip](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip)
- [sa somme de contrôle](https://github.com/Vlad-BloodMoon/opensim/releases/download/v0.9.3.1-webrtc.1/WebRTC_Modules_0.9.3.1.zip.sha256)

Sous Linux, depuis le dossier contenant ces deux téléchargements :
`sha256sum -c WebRTC_Modules_0.9.3.1.zip.sha256`.
Le résultat doit être `OK`. Le ZIP contient aussi les sommes des modules.

1. Arrêter le simulateur et sauvegarder les fichiers à remplacer ainsi que sa
   configuration vocale actuelle.
2. Extraire le ZIP dans un dossier temporaire.
3. Copier les **six fichiers** ci-dessous directement dans le dossier **bin**
   du simulateur, en acceptant leur remplacement :

| DLL | Symboles correspondants |
| --- | --- |
| WebRtcVoice.dll | WebRtcVoice.pdb |
| WebRtcVoiceRegionModule.dll | WebRtcVoiceRegionModule.pdb |
| WebRtcVoiceServiceModule.dll | WebRtcVoiceServiceModule.pdb |

Ne pas supprimer le dossier bin, ne pas créer de dossier bin/bin.
Conserver `WebRtcJanusService.dll` s'il existe ; il n'est pas utilisé par
le connecteur WolfVoice. Aucun serveur Janus n'est nécessaire.

Le ZIP est un pack de modules, pas un OpenSim complet. Ces binaires ciblent
OpenSim 0.9.3.1 / .NET 8 ; ne pas remplacer les DLL d'une autre distribution
sans vérifier sa compatibilité.

## 7. Configurer le simulateur

Créer ou remplacer le fichier **bin/config/wolfvoice.ini** après sauvegarde
de l'ancien fichier. Copier ceci, puis remplacer le nom DNS :

```ini
[WebRtcVoice]
    Enabled = true
    SpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    NonSpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    WebRtcVoiceServerURI = https://voice.votre-grille.fr:9443
    RpcKeyFile = voice_rpc_key
    CredKeyFile = voice_cred_key
    MessageDetails = false
    StunServers = stun:stun.l.google.com:19302,stun:stun1.l.google.com:19302

[VivoxVoice]
    enabled = false
```

[Modèle à télécharger](../contrib/wolfvoice-fresh-grid.ini.example)
([fichier brut](https://raw.githubusercontent.com/Vlad-BloodMoon/wolfvoice/main/contrib/wolfvoice-fresh-grid.ini.example)).
Enregistrer sous `wolfvoice.ini`, pas `wolfvoice.ini.txt`.

Les fichiers doivent être ici :

| Dossier | Fichiers |
| --- | --- |
| bin/ | Les trois DLL et leurs trois PDB, voice_cred_key, voice_rpc_key |
| bin/config/ | wolfvoice.ini |

Démarrer OpenSim **depuis son dossier bin** : les chemins de clés ci-dessus
sont relatifs à ce dossier. Le certificat du serveur doit correspondre au nom
DNS indiqué dans l'URI ; ne pas remplacer ce nom par une IP.

Dans cette architecture, les simulateurs contactent WolfVoice directement.
Il n'y a rien à ajouter dans la liste de services de ROBUST pour la voix.

## 8. Redémarrer et tester

Redémarrer le simulateur. Avec un viewer compatible WebRTC :
1. Activer la voix au niveau du domaine/estate et de la parcelle
   (À propos du terrain → Son → Autoriser la voix).
2. Avec deux avatars, tester la voix locale dans les deux sens.
3. Tester les indicateurs de parole et le volume individuel.
4. Marcher en parlant et tourner l'avatar pour vérifier la spatialisation.
5. Tester un appel privé IM, raccrocher, puis vérifier le retour à la voix locale.
6. Pour tester entre deux simulateurs, équiper les deux des mêmes modules et clés.

Les visiteurs Hypergrid ont été testés sur BloodMoon avec succès.
La voix de groupe n'a pas été validée dans ces essais ; les conférences ad hoc
ne sont pas fournies par ce pack seul.
Ne pas confondre ces fonctionnalités avec un appel privé entre deux avatars.

## En cas de problème

| Symptôme | Vérification |
| --- | --- |
| Aucun accès au serveur | TCP 9443 ouvert après l'étape 5 sur les pare-feu serveur/hébergeur, DNS et TLS |
| Connexion vocale sans son | IP publique annoncée, UDP 40000–40999 et éventuelle redirection NAT |
| Appel refusé / mauvaise clé RPC | Mêmes fichiers de clés, chemins corrects et compte OpenSim autorisé à les lire |
| Voix locale mais pas d'appel privé | Les deux simulateurs à jour et même clé d'appels |
| Certificat refusé | Nom DNS de l'URI présent sur le certificat valide |
| Aucune demande vocale | Voix autorisée sur estate/parcelle, viewer compatible, conflit avec une autre configuration vocale |

Pour demander de l'aide, fournir les erreurs utiles, jamais le contenu des clés.

## Mise à jour d'un serveur déjà installé

Ne pas relancer setup.sh. Télécharger le [binaire actuel](TELECHARGEMENTS_FR.md)
et son fichier .sha256 dans un dossier temporaire, puis vérifier :
`sha256sum -c wolfvoice-x86_64-linux-musl.sha256`.

Sauvegarder le binaire actuel, remplacer uniquement
`/usr/local/bin/wolfvoice` avec les droits d'exécution, puis redémarrer
`wolfvoice.service` pendant une courte interruption annoncée.
Conserver le compte, l'unité systemd, les certificats, les clés et le pare-feu.

Le correctif des axes ne demande pas de nouvelles DLL si les modules
v0.9.3.1-webrtc.1 sont déjà installés.

Développé à partir de WolfVoice, par Shadow Wolf, Lone Wolf et les contributeurs.
