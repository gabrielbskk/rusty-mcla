#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${CODESPACE_NETBOOK_SSH_KEY:-}" || -z "${CODESPACE_NETBOOK_KNOWN_HOSTS:-}" ]]; then
  echo "Netbook SFTP secrets are unavailable for this repository."
  exit 0
fi

install -d -m 700 "$HOME/.ssh"
printf '%s\n' "$CODESPACE_NETBOOK_SSH_KEY" > "$HOME/.ssh/voidbook-cloud-ed25519"
printf '%s\n' "$CODESPACE_NETBOOK_KNOWN_HOSTS" > "$HOME/.ssh/known_hosts-voidbook-cloud"
chmod 600 "$HOME/.ssh/voidbook-cloud-ed25519" "$HOME/.ssh/known_hosts-voidbook-cloud"

ssh_config="$HOME/.ssh/config"
touch "$ssh_config"
chmod 600 "$ssh_config"
if ! grep -q '^Host voidbook-cloud$' "$ssh_config"; then
  cat >> "$ssh_config" <<'SSHCFG'
Host voidbook-cloud
    HostName 100.89.141.42
    User codespacecloud
    Port 22
    IdentityFile ~/.ssh/voidbook-cloud-ed25519
    IdentitiesOnly yes
    UserKnownHostsFile ~/.ssh/known_hosts-voidbook-cloud
    StrictHostKeyChecking yes
    ServerAliveInterval 30
SSHCFG
fi

echo "SFTP is configured. If this Codespace is not on Tailscale yet, run: sudo tailscale up --accept-routes"
echo "Then open the printed login link and connect with: sftp voidbook-cloud"
