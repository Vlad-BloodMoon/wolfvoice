# Installation and signing keys

For this fork, setup.sh installs the server but does NOT create signing keys.
Create them manually as described below. Never paste a key into a public issue,
README, screenshot or download archive.

## 1. Fresh installation or existing installation?

Fresh Linux x86_64 server with systemd: download and review setup.sh from this
fork, then run it. It installs the pinned fork release and prepares the service
and TLS. It needs a hostname pointing at the server; standalone Certbot needs
port 80 available. On a machine already hosting Apache or another TLS setup,
prepare the certificates using your existing method rather than blindly running
the standalone certificate step.

```bash
curl -fsSLO https://raw.githubusercontent.com/Vlad-BloodMoon/wolfvoice/main/setup.sh
less setup.sh
sudo bash setup.sh
```

Existing working installation: do not rerun the installer. Verify the release
checksum, back up and replace only the binary, retaining your service unit, TLS
certificates, firewall/IP allowlist and resource limits. A service restart briefly
disconnects voice users.

## 2. Choose the key mode before creating files

- Fresh deployment with matching modules on ALL region hosts: create BOTH
  cred_key (room tickets) and rpc_key (relay authentication).
- Rolling upgrade with old region modules still using the shared server: create
  ONLY cred_key for now and retain the IP allowlist. This is the mode validated
  during this grid's rollout. Do not create rpc_key or set WOLFVOICE_RPC_KEY until
  every region using the service can send the relay key.
- If rpc_key already exists, do not remove or replace it as part of this guide.
  Distribute its existing contents to authorized, matching region modules.

The relay key is optional in the software. When configured on the server, it is
mandatory on every region request; old modules without it lose voice access.
Private and group voice need at least a credential/signing key. Firewall
restrictions remain necessary in either mode.

## 3. Create the server key files without overwriting existing keys

Run on the VOICE SERVER after the wolfvoice user/group exists.
This command creates only cred_key. For a fresh, fully updated deployment,
change the for line to: for key in cred_key rpc_key; do

```bash
sudo bash <<'KEYS'
set -euo pipefail
install -d -o root -g wolfvoice -m 750 /etc/wolfvoice
for key in cred_key; do
    path="/etc/wolfvoice/$key"
    if [ -e "$path" ]; then
        test -s "$path" || { echo "Existing empty key file: $path"; exit 1; }
        echo "Keeping existing key: $path"
    else
        umask 077
        openssl rand -hex 32 > "$path"
        echo "Created key file: $path"
    fi
    chown root:wolfvoice "$path"
    chmod 640 "$path"
done
KEYS
```

Do not rerun a raw openssl command with > on an existing key: that rotates it and
breaks signatures with the old secret. The command above preserves existing
nonempty files and prints only file paths, never their contents.
If WOLFVOICE_CRED_KEY or WOLFVOICE_RPC_KEY is set in your service environment,
it takes precedence over the corresponding file. Keep one intentional source.

## 4. Transfer the SAME key to each authorized region host

Transfer cred_key privately (SFTP/SCP or an equivalent secure channel). Do NOT
generate a different key on each region. If relay authentication is enabled,
transfer the existing rpc_key too.

On the REGION HOST, in the directory containing the transferred cred_key:
replace opensim below with the actual user and group that run your simulator.

```bash
sudo install -d -o opensim -g opensim -m 750 /etc/opensim
sudo install -o opensim -g opensim -m 600 cred_key /etc/opensim/voice_cred_key
# Only when the server uses a relay key:
# sudo install -o opensim -g opensim -m 600 rpc_key /etc/opensim/voice_rpc_key
```

After confirming the transfer, remove temporary copies of the secrets.

Alternative for an external host: put the administrator-supplied credential key
in the simulator's bin/voice_cred_key and use CredKeyFile = voice_cred_key.
The simulator must start with bin as its working directory, and its service
account must be able to read the file. No .txt extension.

## 5. Configure the existing region INI

Install the three matching WebRTC DLL/PDB pairs first. In the existing dedicated
bin/config/wolfvoice.ini, adapt this example:

```ini
[WebRtcVoice]
    Enabled = true
    SpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    NonSpatialVoiceService = WebRtcVoice.dll:WebRtcVoiceServiceConnector
    WebRtcVoiceServerURI = https://voice.example.org:9443
    CredKeyFile = /etc/opensim/voice_cred_key
    MessageDetails = false
    ; Add only if the VOICE SERVER has relay authentication enabled:
    ; RpcKeyFile = /etc/opensim/voice_rpc_key

[VivoxVoice]
    enabled = false
```

The default RpcKeyFile is /etc/opensim/voice_rpc_key. If it already exists, the
module reads it even when the line is omitted. Do not delete a deployed key
without planning the change.

## 6. Start and test

After the files are in place, restart the voice server to load its key:
```bash
sudo systemctl restart wolfvoice.service
sudo systemctl status wolfvoice.service --no-pager
sudo journalctl -u wolfvoice.service -n 40 --no-pager
```

Then restart the updated simulator. The server reads keys at startup; region
modules re-read files within about one minute, but DLL changes need a restart.
Test local voice, participant dots, per-user volume, a private invitation and
audio in both directions, then hangup and return to local voice.

## 7. Network and common errors

Keep TCP 9443 restricted to the public egress IPs of authorized region hosts.
Allow the configured UDP media range (default 40000-40999) for viewers.
Do not change existing grid firewall rules just to install this update.

- Local voice works but calls do not: check the matching DLLs, credential file
  path, file permissions and identical key contents. For cross-simulator calls,
  both simulators need matching updated modules.
- "wrong or missing relay key": the server requires rpc_key, but the region is
  sending none or a different one. Do not regenerate keys to fix this.
- TLS certificate refused: use the certificate's hostname in the voice URI,
  not the server's numerical IP.
- A warning that no relay key is configured is expected in credential-only mode;
  the IP allowlist is then the access restriction.
- No key value should appear in support messages.

Group voice is implemented upstream but was not separately validated in the
operator tests reported for this release. Ad-hoc conferences need a separate
external service.
