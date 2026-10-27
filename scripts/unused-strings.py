#!/usr/bin/env python3
"""Shipped copy nothing reads.

Everything under `resources/` ships whether or not it is read. The
`RuntimeStrings` tables are the only localized copy, and most keys are the
English string itself, so a line deleted from a view leaves its translations
behind.

A key is reachable when its text, with `%@`/`%lld`/… treated as a wildcard,
appears anywhere that can produce it: Swift, Rust (core's error templates),
another resource file, or the chain catalog. Test fixtures do not count. A
dotted key is reachable when a screen copy struct spells it out or its
namespace is interpolated with an id (`"addressHint.\\(chain.id).empty"`).
The reverse holds too: a dotted key Swift spells out must be in the source
table.

Locales are checked against each other as well: a key set that drifts means
one language silently falls back to another.

`<key>#one` is the singular form `AppLocalization.format(_:count:)` reads
beside `<key>`: it is reachable through its key, and must not outlive it.
"""

import json
import pathlib
import re

from source_scan import hand_written, production_text, report, run, test_modules

STRINGS = pathlib.Path('resources/strings')
FORMAT = re.compile(r'%(?:@|%|lld|llu|ld|lu|d|u|f|s|\d*\.\d+f|\.\d+f)')
# Catalog IDs can contain hyphens, e.g. endpointCapability.token-history.
DOTTED = re.compile(r'^[a-zA-Z_]+[._][a-zA-Z0-9_.-]+$')
# A wrapped Rust string literal: `\` at a line's end continues it.
CONTINUATION = re.compile(r'\\\s*\n\s*')


def production():
    """(path, production text) for every file that can produce copy.

    Fixtures cannot keep shipped translations reachable, and `mod tests;` in
    the middle of a Rust file does not hide the production items after it.
    """
    paths = [path for suffix, roots in (
                 ('.swift', ('swift',)), ('.rs', ('core', 'cli')),
                 ('.json', ('resources',)), ('.toml', ('core/data',)))
             for root in roots for path in hand_written(root, suffix)
             if not path.name.startswith('RuntimeStrings.')]
    declared_tests = test_modules(path for path in paths if path.suffix == '.rs')
    return [(path, production_text(path, declared_tests)) for path in paths]


def reachable(key, haystack):
    """A key is reachable when its literal text, wildcarded at each format
    specifier, is somewhere that could produce it."""
    pieces = [re.escape(piece) for piece in FORMAT.split(re.sub(r'\s+', ' ', key).strip())
              if piece]
    return bool(pieces) and re.search('.{0,120}'.join(pieces), haystack)


def dotted_reachable(key, haystack):
    namespace = re.split(r'[._]', key, maxsplit=1)[0]
    return key in haystack or f'{namespace}.\\(' in haystack or f'{namespace}_\\(' in haystack


def locales(base):
    # `RuntimeStrings.manifest.json` sits beside the locales and names them;
    # it is not one of them.
    return sorted(path for path in STRINGS.glob(f'{base}.*.json')
                  if not path.name.endswith('.manifest.json'))


def scan():
    sources = production()
    # Production strings with wrapped literals and indentation flattened.
    haystack = re.sub(r'\s+', ' ', CONTINUATION.sub('', '\n'.join(text for _, text in sources)))
    failures = []

    bases = sorted({path.name.split('.')[0] for path in STRINGS.glob('*.*.json')
                    if not path.name.endswith('.manifest.json')})
    for base in bases:
        files = locales(base)
        if not files:
            continue
        tables = {path.name: json.loads(path.read_text()) for path in files}
        keysets = {name: set(table) for name, table in tables.items() if isinstance(table, dict)}
        if not keysets:
            continue
        reference = set().union(*keysets.values())
        for name, keys in keysets.items():
            for missing in sorted(reference - keys):
                failures.append(f"  {name:<36} missing key present in another locale: {missing!r}")

        source = tables[files[0].name]
        for key in sorted(source):
            if key.endswith('#one'):
                if key.removesuffix('#one') not in source:
                    failures.append(f"  {base:<36} singular form without its key {key!r}")
                continue
            found = (dotted_reachable(key, haystack) if DOTTED.match(key) and ' ' not in key
                     else reachable(key, haystack))
            if not found:
                failures.append(f"  {base:<36} no source produces {key!r}")

    # The reverse: a dotted key spelled out in Swift must be in the source
    # table, or the screen shows the key itself.
    source_keys = set(json.loads((STRINGS / 'RuntimeStrings.en.json').read_text()))
    swift = '\n'.join(text for path, text in sources if path.suffix == '.swift')
    spelled = r'AppLocalization\.(?:string|format)\("([A-Za-z_]+\.[A-Za-z0-9_.-]+)"'
    for key in sorted(set(re.findall(spelled, swift))):
        if key not in source_keys:
            failures.append(f"  {'RuntimeStrings.en.json':<36} Swift names a missing key {key!r}")

    # Core's sentences with values (`refused`, `failed`,
    # `LocalizableMessage::new`) are the ones a front end can only translate by
    # table: an untranslated one reads in English with its values in, which is
    # the gap this closes. Test modules are out of scope; they make up
    # sentences of their own.
    rust = '\n'.join(CONTINUATION.sub('', text) for path, text in sources
                     if path.suffix == '.rs' and path.parts[:2] == ('core', 'src'))
    sentence = r'(?:\brefused|\bfailed|LocalizableMessage::new)\(\s*"((?:[^"\\]|\\.)*)"'
    for key in sorted(set(re.findall(sentence, rust))):
        if key not in source_keys:
            failures.append(f"  {'RuntimeStrings.en.json':<36} core names an untranslated sentence {key!r}")

    return report(failures, f'{len(failures)} unused or inconsistent string(s)')


if __name__ == '__main__':
    run(scan)
