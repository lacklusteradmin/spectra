#!/usr/bin/env bash
# Public core functions no caller reaches.
#
# `unreachable-exports.sh` checks the FFI surface; this checks below it. A
# `pub fn` in a lib crate is API to rustc, so `dead_code` never fires on one,
# and `#[uniffi::export]` never named it, so the bindings never mentioned it
# either. Between the two gates a function can lose its last caller and keep
# compiling for as long as the file does.
#
# A function is reachable when something other than its own definition, a
# `use` line, or a comment names it — in Rust anywhere in the workspace, in
# hand-written Swift or Kotlin under its camelCase name. Tests do not count:
# a function kept alive only by the test that covers it is a test fixture,
# and belongs behind `#[cfg(test)]`, where `dead_code` can see it again.
#
# This is a name-based source scan, not a resolved call graph: another function
# with the same name, or an unreachable caller, can hide dead code. Review
# candidates and module ownership rather than treating a clean result as proof.
#
# Exits non-zero when any are found, so it can gate.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -B - <<'PY'
import re, pathlib, sys
from scripts.source_scan import frontend_test, hand_written, split_test_code, test_modules

DEFINITION = re.compile(r'\s*pub(?:\([^)]*\))? (?:async )?fn (\w+)')

def camel(name):
    head, *rest = name.split('_')
    return head + ''.join(p[:1].upper() + p[1:] for p in rest)

def strip_noise(text, keyword):
    """Comments, `use` lines and declarations are not calls."""
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    text = re.sub(r'(?m)^\s*(?://|///|//!).*$', '', text)
    text = re.sub(r'(?m)^\s*(?:pub )?use .*$', '', text)
    return re.sub(r'\b' + keyword + r'\s+\w+', keyword + ' __declaration__', text)

sources = sorted(p for d in ('core/src', 'ffi/src', 'cli/src')
                 for p in pathlib.Path(d).rglob('*.rs'))
declared_tests = test_modules(sources)

definitions, production = [], []
for path in sources:
    prod_lines, _ = split_test_code(path, path.read_text(), declared_tests)
    production.append(strip_noise(''.join(t for _, t in prod_lines), 'fn'))
    # Only core's surface is checked: the CLI is a binary, where `dead_code`
    # already fires on an uncalled function.
    if path.parts[0] != 'core':
        continue
    for lineno, line in prod_lines:
        m = DEFINITION.match(line)
        if m:
            definitions.append((m.group(1), f"{path.relative_to('core/src')}:{lineno}"))
# Every word in production Rust, and every word called in hand-written Swift
# or Kotlin: a name is reached when it is one of them. Collected once, so the
# check is a lookup per definition rather than a scan of the sources each.
rust_words = set(re.findall(r'\w+', '\n'.join(production)))

swift = strip_noise('\n'.join(p.read_text() for p in hand_written('swift', '.swift')
                             if not frontend_test(p)), 'func')
kotlin = strip_noise('\n'.join(p.read_text() for p in hand_written('kotlin', '.kt')
                              if not frontend_test(p)), 'fun')
called_words = set(re.findall(r'(\w+)\s*\(', swift + '\n' + kotlin))

# UniFFI calls these itself; no Rust or Swift source names them.
ALLOWED = {'new', 'uniffi_reexport_hack'}

dead = [(name, where) for name, where in definitions
        if name not in ALLOWED
        and name not in rust_words
        and camel(name) not in called_words]

for name, where in sorted(dead):
    print(f"  {name:<46} {where}")
print(f"\n  {len(dead)} uncalled public function(s)")
sys.exit(1 if dead else 0)
PY
