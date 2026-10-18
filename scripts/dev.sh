#!/bin/sh
# Lance une commande dans l'image de dev (aucune toolchain Rust locale requise).
# Usage : sh scripts/dev.sh cargo test        |  sh scripts/dev.sh sh scripts/check.sh
# Un seul volume target (moli-target) pour tous les clones et worktrees : les dépendances
# compilées sont partagées. Deux builds en même temps attendent leur tour (verrou Cargo).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
set -- moli-os-dev "$@"
# The house's private lists (scripts/leak-check.sh), when a house folder sits
# next to the repository: read-only, never copied into it.
house=$(dirname "$root")/moli-maison
for pair in leak-denylist.txt:MOLI_DENYLIST leak-allow.txt:MOLI_ALLOWLIST; do
  list=${pair%%:*}
  if [ -f "$house/$list" ]; then
    set -- -v "$house/$list:/house/$list:ro" -e "${pair#*:}=/house/$list" "$@"
  fi
done
exec docker run --rm \
  -v "$root:/src" \
  -v moli-cargo:/usr/local/cargo/registry \
  -v moli-target:/target \
  -w /src "$@"
