#!/bin/sh
# Le certificat HTTPS de Moli, obtenu par Tailscale (Let's Encrypt, 90 jours), pour
# `[server.tls]` de moli.toml. À lancer sur la machine qui héberge Moli, à la main ou
# par la minuterie de docs/systemd/ ; Moli relit les fichiers tout seul (pas de redémarrage).
#
#   scripts/tailscale-cert.sh <nom.tailnet.ts.net> [dossier]   dossier : ./data/tls par défaut
#
# Droits : `tailscale cert` veut root ou l'opérateur (`sudo tailscale set --operator=$USER`,
# une fois). Les fichiers sont donnés à l'utilisateur 1000 du conteneur (MOLI_UID) : la clé
# en 0600, le certificat en 0644. La clé n'est jamais affichée.
set -eu
name=${1:-}
dir=${2:-./data/tls}
uid=${MOLI_UID:-1000}
if [ -z "$name" ]; then
  echo "usage : $0 <nom.tailnet.ts.net> [dossier]" >&2
  echo "  le nom : « tailscale status --json | grep DNSName », sans le point final" >&2
  exit 2
fi
name=${name%.}
command -v tailscale >/dev/null 2>&1 || { echo "tailscale introuvable" >&2; exit 1; }

umask 077
mkdir -p "$dir"
tmp=$(mktemp -d "$dir/.renew.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
# Dans un dossier à part, puis déplacés d'un coup : Moli ne voit jamais un fichier à moitié écrit.
tailscale cert --cert-file "$tmp/moli.crt" --key-file "$tmp/moli.key" "$name" >/dev/null
chmod 0644 "$tmp/moli.crt"
chmod 0600 "$tmp/moli.key"
if [ "$(id -u)" = 0 ]; then
  chown "$uid:$uid" "$tmp/moli.crt" "$tmp/moli.key"
fi
mv -f "$tmp/moli.crt" "$dir/moli.crt"
mv -f "$tmp/moli.key" "$dir/moli.key"
chmod 0755 "$dir"
# L'échéance, sans rien d'autre (jamais la clé).
if command -v openssl >/dev/null 2>&1; then
  echo "certificat : $dir/moli.crt ($(openssl x509 -noout -enddate -in "$dir/moli.crt" | sed 's/notAfter=/valable jusqu au /'))"
else
  echo "certificat : $dir/moli.crt"
fi
echo "clé : $dir/moli.key (0600, utilisateur $uid) — Moli le relit dans la minute, ou tout de suite : docker kill -s HUP moli-os"
