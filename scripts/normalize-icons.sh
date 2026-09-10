#!/usr/bin/env bash
# Rewrites every icon under icons/cryptoicon/ and icons/fiaticon/ into the house
# style defined by scripts/svgo.config.mjs, so the two directories stay a single
# coherent icon library instead of a pile of exporter output.
#
#   scripts/normalize-icons.sh           normalize in place
#   scripts/normalize-icons.sh --check   fail if any icon is not already normalized
#
# icons/appicon/ is deliberately left alone: those are 1024-point sources for a
# PNG conversion, not members of the 64x64 library.
#
# The pass is idempotent, so --check is just "running it would change nothing".
# Run scripts/export-swift-icons.sh afterwards to carry changes into the catalog.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CONFIG="$SCRIPT_DIR/svgo.config.mjs"
DIRS=("$REPO_ROOT/icons/cryptoicon" "$REPO_ROOT/icons/fiaticon")

if ! command -v svgo &>/dev/null; then
  echo "error: svgo not found. Install with: brew install svgo" >&2
  exit 1
fi

check_only=false
case "${1-}" in
  --check) check_only=true ;;
  "") ;;
  *) echo "usage: $(basename "$0") [--check]" >&2; exit 2 ;;
esac

if [[ "$check_only" == true ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  drift=0
  for dir in "${DIRS[@]}"; do
    out="$tmp/$(basename "$dir")"
    mkdir -p "$out"
    svgo --config "$CONFIG" -f "$dir" -o "$out" -q
    for svg in "$dir"/*.svg; do
      if ! cmp -s "$svg" "$out/$(basename "$svg")"; then
        echo "  not normalized: ${svg#"$REPO_ROOT"/}"
        ((drift++))
      fi
    done
  done
  if ((drift > 0)); then
    echo "$drift icon(s) drifted from the house style. Run scripts/normalize-icons.sh." >&2
    exit 1
  fi
  echo "all icons normalized."
  exit 0
fi

for dir in "${DIRS[@]}"; do
  svgo --config "$CONFIG" -f "$dir" -o "$dir" -q
  echo "  [$(basename "$dir")] normalized $(ls "$dir"/*.svg | wc -l | tr -d ' ') icons."
done
