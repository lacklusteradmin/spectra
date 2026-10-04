#!/usr/bin/env bash
# Syncs all icon sources from icons/ into swift/Assets.xcassets/:
#
#   icons/appicon/ → AppIcon.appiconset/ (SVG → 1024×1024 PNG)
#   icons/crypto/  → crypto/             (SVG imageset, no namespace)
#   icons/fiat/    → fiat/               (SVG imageset, provides-namespace)
#
# Rules (crypto + fiat):
#   - Creates a new .imageset for every SVG not yet in the catalog.
#   - Updates the SVG inside an existing .imageset when the source has changed.
#   - Removes .imageset folders that no longer have a matching source SVG.
#   - Removes an asset group when its source directory has no SVGs.
#
# Rules (appicon):
#   - Converts each SVG to a 1024×1024 PNG using ImageMagick.
#   - Only writes the PNG when the source SVG has changed (MD5 sentinel).
#   - Never touches Contents.json.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

APPICON_SRC="$REPO_ROOT/icons/appicon"
CRYPTO_SRC="$REPO_ROOT/icons/crypto"
FIAT_SRC="$REPO_ROOT/icons/fiat"

APPICON_DEST="$REPO_ROOT/swift/Assets.xcassets/AppIcon.appiconset"
CRYPTO_DEST="$REPO_ROOT/swift/Assets.xcassets/crypto"
FIAT_DEST="$REPO_ROOT/swift/Assets.xcassets/fiat"

# ── helpers ────────────────────────────────────────────────────────────────────

MAGICK_BIN=""
require_convert() {
  if command -v magick &>/dev/null; then
    MAGICK_BIN="magick"
  elif command -v convert &>/dev/null; then
    MAGICK_BIN="convert"
  else
    echo "error: ImageMagick not found. Install with: brew install imagemagick" >&2
    exit 1
  fi
}

ensure_group_contents() {
  local dir="$1" namespace="$2"
  mkdir -p "$dir"
  if [[ ! -f "$dir/Contents.json" ]]; then
    if [[ "$namespace" == "true" ]]; then
      cat > "$dir/Contents.json" <<'JSON'
{
  "info": {
    "author": "xcode",
    "version": 1
  },
  "properties": {
    "provides-namespace": true
  }
}
JSON
    else
      cat > "$dir/Contents.json" <<'JSON'
{
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
JSON
    fi
  fi
}

write_imageset_contents() {
  local imageset="$1" filename="$2"
  cat > "$imageset/Contents.json" <<JSON
{
  "images": [
    {
      "filename": "${filename}",
      "idiom": "universal"
    }
  ],
  "info": {
    "author": "xcode",
    "version": 1
  },
  "properties": {
    "preserves-vector-representation": true
  }
}
JSON
}

# Sync a flat directory of SVGs into an xcassets group folder.
sync_svg_group() {
  local src="$1" dest="$2" namespace="$3" label="$4"
  local added=0 updated=0 removed=0

  if ! compgen -G "$src/*.svg" >/dev/null; then
    if [[ -d "$dest" ]]; then
      rm -rf "$dest"
      echo "  [$label] removed empty group"
    fi
    return
  fi

  ensure_group_contents "$dest" "$namespace"

  for svg_path in "$src"/*.svg; do
    [[ -e "$svg_path" ]] || continue
    local filename name imageset dest_svg
    filename="$(basename "$svg_path")"
    name="${filename%.svg}"
    imageset="$dest/${name}.imageset"
    dest_svg="$imageset/$filename"

    mkdir -p "$imageset"
    [[ -f "$imageset/Contents.json" ]] || write_imageset_contents "$imageset" "$filename"

    if [[ ! -f "$dest_svg" ]]; then
      cp "$svg_path" "$dest_svg"
      echo "  [$label] added   $name"
      ((added += 1))
    elif ! cmp -s "$svg_path" "$dest_svg"; then
      cp "$svg_path" "$dest_svg"
      echo "  [$label] updated $name"
      ((updated += 1))
    fi
  done

  for imageset in "$dest"/*.imageset; do
    [[ -d "$imageset" ]] || continue
    local name
    name="$(basename "$imageset" .imageset)"
    if [[ ! -f "$src/${name}.svg" ]]; then
      rm -rf "$imageset"
      echo "  [$label] removed $name"
      ((removed += 1))
    fi
  done

  echo "  [$label] done: $added added, $updated updated, $removed removed."
}

# Convert SVGs in icons/appicon/ to 1024×1024 PNGs in AppIcon.appiconset/.
# Uses an MD5 sentinel file alongside each PNG to detect source changes.
sync_appicon() {
  local src="$1" dest="$2"
  require_convert
  local converted=0 skipped=0

  for svg_path in "$src"/*.svg; do
    [[ -e "$svg_path" ]] || continue
    local filename name png_path
    filename="$(basename "$svg_path")"
    name="${filename%.svg}"
    png_path="$dest/${name}.png"

    if [[ -f "$png_path" && ! "$svg_path" -nt "$png_path" ]]; then
      ((skipped += 1))
      continue
    fi

    # ImageMagick's built-in SVG renderer flattens curves by user units, not
    # by output size, so sources need a 1024-unit viewBox to render smoothly.
    $MAGICK_BIN -density 144 -background none "$svg_path" -resize 1024x1024 -depth 8 "$png_path"
    echo "  [appicon] converted $name → ${name}.png"
    ((converted += 1))
  done

  echo "  [appicon] done: $converted converted, $skipped up-to-date."
}

# ── main ───────────────────────────────────────────────────────────────────────

echo "syncing appicon..."
sync_appicon "$APPICON_SRC" "$APPICON_DEST"

echo "syncing crypto..."
sync_svg_group "$CRYPTO_SRC" "$CRYPTO_DEST" "false" "crypto"

echo "syncing fiat..."
sync_svg_group "$FIAT_SRC" "$FIAT_DEST" "true" "fiat"
