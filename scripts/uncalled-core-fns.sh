#!/usr/bin/env bash
# Public core functions no caller reaches.
#
# `unreachable-exports.sh` checks the FFI surface; this checks below it. A
# `pub fn` in a lib crate is API to rustc, so `dead_code` never fires on one,
# and `#[uniffi::export]` never named it, so the bindings never mentioned it
# either. Between the two gates a function can lose its last caller and keep
# compiling for as long as the file does. Three had, when this was written.
#
# A function is reachable when something other than its own definition, a
# `use` line, or a comment names it — in Rust anywhere in the workspace, in
# hand-written Swift or Kotlin under its camelCase name. Tests do not count:
# a function kept alive only by the test that covers it is a test fixture,
# and belongs behind `#[cfg(test)]`, where `dead_code` can see it again.
#
# Exits non-zero when any are found, so it can gate.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import re, pathlib, sys

ATTRIBUTE = re.compile(r'\s*#\[(cfg\(test\)|test|tokio::test)')
SKIPPABLE = re.compile(r'\s*(#\[|///|//!|$)')
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

def item_end(lines, start):
    """Index just past the item beginning at `start`.

    A one-line declaration ends at its semicolon — `#[cfg(test)] mod tests;`
    is the shape that matters, and counting braces through it swallows the
    rest of the file. Anything else is brace-delimited, and its signature may
    wrap before the opening brace ever appears.
    """
    if lines[start].strip().endswith(';'):
        return start + 1
    depth, opened, i = 0, False, start
    while i < len(lines):
        depth += lines[i].count('{') - lines[i].count('}')
        opened = opened or '{' in lines[i]
        i += 1
        if opened and depth <= 0:
            break
    return i

def test_modules(paths):
    """Files a `#[cfg(test)] mod X;` declaration pulls in.

    The name is not the signal: `diagnostics/self_tests.rs` is production
    code — the chain self-tests the diagnostics screen runs — and reading it
    as a test file hides every caller in it. The declaration is the signal.
    """
    found = set()
    for path in paths:
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            if not ATTRIBUTE.match(line):
                continue
            m = re.match(r'\s*(?:pub(?:\([^)]*\))? )?mod (\w+);', lines[i + 1] if i + 1 < len(lines) else '')
            if m:
                found.add(path.parent / f"{m.group(1)}.rs")
                found.add(path.parent / m.group(1) / "mod.rs")
    return found

def split_test_code(path, text, declared_tests):
    """(production lines as (lineno, text), test text) for one Rust file."""
    lines = text.splitlines(keepends=True)
    if 'tests' in path.parts or path in declared_tests:
        return [], text
    production, test, i = [], [], 0
    while i < len(lines):
        if ATTRIBUTE.match(lines[i]):
            j = i
            while j < len(lines) and SKIPPABLE.match(lines[j]):
                j += 1
            end = len(lines) if j >= len(lines) else item_end(lines, j)
            test.extend(lines[i:end])
            i = end
            continue
        production.append((i + 1, lines[i]))
        i += 1
    return production, ''.join(test)

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
rust_calls = '\n'.join(production)

swift = strip_noise('\n'.join(p.read_text() for p in pathlib.Path('swift').rglob('*.swift')
                              if 'generated' not in p.parts), 'func')
kotlin = strip_noise('\n'.join(p.read_text() for p in pathlib.Path('kotlin').rglob('*.kt')), 'fun')

# UniFFI calls these itself; no Rust or Swift source names them.
ALLOWED = {'new', 'uniffi_reexport_hack'}

dead = [(name, where) for name, where in definitions
        if name not in ALLOWED
        and not re.search(rf'\b{re.escape(name)}\b', rust_calls)
        and not re.search(rf'\b{re.escape(camel(name))}\s*\(', swift + kotlin)]

for name, where in sorted(dead):
    print(f"  {name:<46} {where}")
print(f"\n  {len(dead)} uncalled public function(s)")
sys.exit(1 if dead else 0)
PY
