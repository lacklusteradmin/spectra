#!/usr/bin/env bash
# Count what Swift can call across the FFI boundary.
#
# The definition, so the number is reproducible: a free function carrying
# `#[uniffi::export]`, plus every `pub fn` inside an `#[uniffi::export] impl`
# block — UniFFI exports all of them, whether or not anyone calls them.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import re, pathlib

free, methods, blocks = 0, 0, []
for f in sorted(pathlib.Path('core/src').rglob('*.rs')):
    t = f.read_text()
    free += len(re.findall(r'#\[uniffi::export[^\]]*\]\s*\npub (?:async )?fn ', t))
    for m in re.finditer(r'#\[uniffi::export[^\]]*\]\s*\nimpl ([^\{]*)\{', t):
        i, depth = m.end(), 1
        while depth and i < len(t):
            depth += (t[i] == '{') - (t[i] == '}')
            i += 1
        n = len(re.findall(r'\n    pub (?:async )?fn ', t[m.end():i]))
        methods += n
        blocks.append((n, f"{f.relative_to('core/src')} impl {m.group(1).strip()}"))

for n, where in sorted(blocks, reverse=True):
    print(f"  {n:>4}  {where}")
print()
print(f"  free functions   {free}")
print(f"  methods          {methods}")
print(f"  total            {free + methods}")
PY
