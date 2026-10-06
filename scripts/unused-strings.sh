#!/usr/bin/env bash
# Shipped copy nothing reads.
#
# Everything under `resources/` ships whether or not it is read. The
# `RuntimeStrings` tables are the only localized copy, and most keys are the
# English string itself, so a line deleted from a view leaves its
# translations behind.
#
# A key is reachable when its text, with `%@`/`%lld`/… treated as a wildcard,
# appears anywhere that can produce it: Swift, Rust (core's error templates),
# another resource file, or the chain catalog. Test fixtures do not count.
# A dotted key is reachable when a screen copy struct spells it out or its
# namespace is interpolated with an id (`"addressHint.\(chain.id).empty"`).
# The reverse holds too: a
# dotted key Swift spells out must be in the source table.
#
# Locales are checked against each other as well: a key set that drifts means
# one language silently falls back to another.
#
# `<key>#one` is the singular form `AppLocalization.format(_:count:)` reads
# beside `<key>`: it is reachable through its key, and must not outlive it.
#
# Exits non-zero when any are found, so it can gate.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -B - <<'PY'
import json, pathlib, re, sys
from scripts.source_scan import hand_written, production_text, test_modules

STRINGS = pathlib.Path('resources/strings')
FORMAT = re.compile(r'%(?:@|%|lld|llu|ld|lu|d|u|f|s|\d*\.\d+f|\.\d+f)')
# Catalog IDs can contain hyphens, e.g. endpointCapability.token-history.
DOTTED = re.compile(r'^[a-zA-Z_]+[._][a-zA-Z0-9_.-]+$')

# Fixtures cannot keep shipped translations reachable. Use the same Rust item
# splitting as the public-function scan, so `mod tests;` in the middle of a
# source file does not hide the production items that follow it.
paths = [p for suffix, roots in (
             ('.swift', ('swift',)), ('.rs', ('core', 'cli', 'ffi')),
             ('.json', ('resources',)), ('.toml', ('core/data',)),
             ('.kt', ('kotlin',)), ('.xml', ('kotlin',)))
         for root in roots for p in hand_written(root, suffix)
         if not p.name.startswith('RuntimeStrings.')]
declared_tests = test_modules(p for p in paths if p.suffix == '.rs')
production = [(p, production_text(p, declared_tests)) for p in paths]

def corpus():
    """Production strings with wrapped literals and indentation flattened."""
    text = '\n'.join(text for _, text in production)
    return re.sub(r'\s+', ' ', re.sub(r'\\\s*\n\s*', '', text))

def reachable(key, haystack):
    """A key is reachable when its literal text, wildcarded at each format
    specifier, is somewhere that could produce it."""
    pieces = [re.escape(p) for p in FORMAT.split(re.sub(r'\s+', ' ', key).strip()) if p]
    return bool(pieces) and re.search('.{0,120}'.join(pieces), haystack)

def locales(base):
    # `RuntimeStrings.manifest.json` sits beside the locales and names them;
    # it is not one of them.
    return sorted(p for p in STRINGS.glob(f'{base}.*.json')
                  if not p.name.endswith('.manifest.json'))

failures = []
haystack = corpus()

def dotted_reachable(key):
    namespace = re.split(r'[._]', key, maxsplit=1)[0]
    return key in haystack or f'{namespace}.\\(' in haystack or f'{namespace}_\\(' in haystack

bases = sorted({p.name.split('.')[0] for p in STRINGS.glob('*.*.json')
                if not p.name.endswith('.manifest.json')})
for base in bases:
    files = locales(base)
    if not files:
        continue
    keysets = {p.name: set(json.loads(p.read_text())) for p in files
               if isinstance(json.loads(p.read_text()), dict)}
    if not keysets:
        continue
    reference = set().union(*keysets.values())
    for name, keys in keysets.items():
        for missing in sorted(reference - keys):
            failures.append(f"  {name:<36} missing key present in another locale: {missing!r}")

    source = json.loads(files[0].read_text())
    for key in sorted(source):
        if key.endswith('#one'):
            if key.removesuffix('#one') not in source:
                failures.append(f"  {base:<36} singular form without its key {key!r}")
            continue
        found = (dotted_reachable(key) if DOTTED.match(key) and ' ' not in key
                 else reachable(key, haystack))
        if not found:
            failures.append(f"  {base:<36} no source produces {key!r}")

# The reverse: a dotted key spelled out in Swift must be in the source table,
# or the screen shows the key itself.
source_keys = set(json.loads((STRINGS / 'RuntimeStrings.en.json').read_text()))
swift = '\n'.join(text for p, text in production if p.suffix == '.swift')
for key in sorted(set(re.findall(r'AppLocalization\.(?:string|format)\("([A-Za-z_]+\.[A-Za-z0-9_.-]+)"', swift))):
    if key not in source_keys:
        failures.append(f"  {'RuntimeStrings.en.json':<36} Swift names a missing key {key!r}")

# Core's sentences with values (`refused`, `failed`, `LocalizableMessage::new`) are
# the ones a front end can only translate by table: an untranslated one reads
# in English with its values in, which is the gap this closes. Test modules
# are out of scope; they make up sentences of their own.
rust = '\n'.join(re.sub(r'\\\s*\n\s*', '', text)
                 for p, text in production if p.suffix == '.rs' and p.parts[:2] == ('core', 'src'))
for key in sorted(set(re.findall(r'(?:\brefused|\bfailed|LocalizableMessage::new)\(\s*"((?:[^"\\]|\\.)*)"', rust))):
    if key not in source_keys:
        failures.append(f"  {'RuntimeStrings.en.json':<36} core names an untranslated sentence {key!r}")

for line in failures:
    print(line)
print(f"\n  {len(failures)} unused or inconsistent string(s)")
sys.exit(1 if failures else 0)
PY
