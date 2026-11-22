#!/usr/bin/env python3
"""Public core functions no caller reaches.

An exported function is the FFI surface, and its callers are the front ends:
spectra-apple's `scripts/unreachable-exports.py` checks each export against the
app's and the CLI's calls. This checks the `pub fn`s below that surface. A
`pub fn` in a lib crate is API to rustc, so `dead_code` never fires on one,
and `#[uniffi::export]` never named it, so the bindings never mentioned it
either. Between the two gates a function can lose its last caller and keep
compiling for as long as the file does.

An unexported function only Rust can call, so this needs nothing outside the
workspace: it is reachable when something other than its own definition, a
`use` line, or a comment names it in production Rust. Tests do not count: a
function kept alive only by the test that covers it is a test fixture, and
belongs behind `#[cfg(test)]`, where `dead_code` can see it again.

This is a name-based source scan, not a resolved call graph: another function
with the same name, or an unreachable caller, can hide dead code. Review
candidates and module ownership rather than treating a clean result as proof.
"""

import re

from source_scan import calls_only, exports, hand_written, report, run, split_test_code, test_modules

DEFINITION = re.compile(r'\s*pub(?:\([^)]*\))? (?:async )?fn (\w+)')

# UniFFI calls these itself; no Rust source names them.
ALLOWED = {'new'}


def scan():
    sources = hand_written('core/src', '.rs') + hand_written('cli/src', '.rs')
    declared_tests = test_modules(sources)

    exported = {(name, where) for name, where, _ in exports()}
    definitions, production = [], []
    for path in sources:
        lines, _ = split_test_code(path, path.read_text(), declared_tests)
        production.append(calls_only(''.join(text for _, text in lines), 'fn'))
        # Only core's surface is checked: the CLI is a binary, where
        # `dead_code` already fires on an uncalled function.
        if path.parts[0] != 'core':
            continue
        where = str(path.relative_to('core/src'))
        for number, line in lines:
            definition = DEFINITION.match(line)
            if definition and (definition.group(1), where) not in exported:
                definitions.append((definition.group(1), f"{where}:{number}"))

    # Every word in production Rust: a name is reached when it is one of them.
    # Collected once, so the check is a lookup per definition rather than a
    # scan of the sources each.
    rust_words = set(re.findall(r'\w+', '\n'.join(production)))

    dead = sorted((name, where) for name, where in definitions
                  if name not in ALLOWED and name not in rust_words)
    return report([f'  {name:<46} {where}' for name, where in dead],
                  f'{len(dead)} uncalled public function(s)')


if __name__ == '__main__':
    run(scan)
