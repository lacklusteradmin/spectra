#!/usr/bin/env bash
# Fails when a Swift view restates a design value that SpectraLayout owns.
#
#   scripts/check-design-tokens.sh
#
# docs/IOS-UI.md defines the spacing scale, the corner-radius scale and the two
# Liquid Glass tints; swift/views/SpectraLayout.swift spells them in Swift. A
# screen that writes the number instead of the token drifts silently, and no
# review catches it by eye.
#
# The scale itself is not policed here. Changing a step is a design decision:
# edit docs/IOS-UI.md and SpectraLayout.swift together.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VIEWS="$REPO_ROOT/swift/views"
TOKENS="$VIEWS/SpectraLayout.swift"

fail=0
report() {
  local title="$1" remedy="$2" hits="$3"
  [[ -z "$hits" ]] && return 0
  printf '\n%s\n' "$title" >&2
  printf '%s\n' "$hits" | sed 's|^'"$REPO_ROOT"'/|  |' >&2
  printf '  -> %s\n' "$remedy" >&2
  fail=1
}

# A numeric radius at a call site. Component property declarations
# (`var cornerRadius: CGFloat = 6`) are a component's own geometry and do not
# match this pattern.
report "Numeric corner radius outside SpectraLayout:" \
  "use SpectraLayout.Radius (card/inner/control)" \
  "$(grep -rn 'cornerRadius: [0-9]' --include='*.swift' "$VIEWS" | grep -v "^$TOKENS:" || true)"

# A numeric padding, stack spacing or spacer minimum. `spacing: 0` is the
# absence of spacing, not a step, and stays literal. A component's own
# geometry — a frame, an offset, an icon size — is not spacing and is not
# matched.
report "Numeric spacing outside SpectraLayout:" \
  "use SpectraLayout.Space (xxs/xs/s/m/l/xl) or a named SpectraLayout value" \
  "$(grep -rnE 'padding\((\.[a-zA-Z]+, )?[0-9]|(^|[^A-Za-z])spacing: [1-9]|minLength: [1-9]' --include='*.swift' "$VIEWS" | grep -v "^$TOKENS:" || true)"

# A literal orange. The theme colour is the asset catalog's AccentColor
# (`.tint`, `Color.accentColor`) so that it can change in one place; a warning
# is `.spectraWarning`, which does not follow the theme. Decorative artwork
# with a fixed palette of its own ends the line with `design-tokens: artwork`.
report "Literal orange outside SpectraLayout:" \
  "use .tint / Color.accentColor for the theme colour, .spectraWarning for warnings and pending states" \
  "$(grep -rnw 'orange' --include='*.swift' "$VIEWS" | grep -v "^$TOKENS:" | grep -v 'design-tokens: artwork' | grep -vE '^[^:]+:[0-9]+: *//' || true)"

# A raw neutral glass tint. Accent-tinted glass (.orange/.red notices) carries
# its own colour and is deliberately not a token.
report "Raw white glass tint outside SpectraLayout:" \
  "use SpectraLayout.GlassTint.elevated / .content, or spectraElevatedFill / spectraCardFill" \
  "$(grep -rn 'tint(\.white\.opacity(' --include='*.swift' "$VIEWS" | grep -v "^$TOKENS:" || true)"

# Glass inside a card. Glass stops at the card, and every card has the card
# radius; a glass shape on a smaller step, a circle or a capsule is a surface
# nested inside one — an input, a chip, a tile, an icon backplate — which is
# glass on glass. The call is read whole, since its shape often sits on a later
# line than `glassEffect(`. Decorative artwork ends the call's first line with
# `design-tokens: artwork`.
report "Glass surface inside a card:" \
  "use spectraInsetFill / SpectraLayout.insetFill for a surface inside a card" \
  "$(find "$VIEWS" -name '*.swift' ! -path "$TOKENS" -print0 | xargs -0 perl -0777 -ne '
      while (/glassEffect(\((?:[^()]++|(?1))*\))/g) {
        my ($call, $pos) = ($&, $-[0]);
        my ($first) = substr($_, $pos) =~ /^([^\n]*)/;
        next if $first =~ /design-tokens: artwork/;
        next unless $call =~ /Radius\.(inner|control)|in: \.(circle|capsule)|\b(Circle|Capsule)\(/;
        my $line = 1 + (substr($_, 0, $pos) =~ tr/\n//);
        (my $flat = $call) =~ s/\s+/ /g;
        print "$ARGV:$line: $flat\n";
      }' || true)"

if (( fail )); then
  printf '\ndesign tokens: FAILED\n' >&2
  exit 1
fi
printf 'design tokens: ok\n'
