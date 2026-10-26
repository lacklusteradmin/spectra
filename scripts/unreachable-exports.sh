#!/usr/bin/env bash
# Exports no front end can reach.
#
# An FFI export nothing calls still costs: it is generated into the bindings,
# it has to keep compiling, and it reads as API. This lists the ones whose
# camelCase name appears nowhere in hand-written Swift or in the CLI.
#
# A call inside a `WalletServiceBridge` method counts only while that method is
# itself called: the bridge is one-line wrappers, so a wrapper nothing calls
# still names its export.
#
# Exits non-zero when any are found, so it can gate.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import re, pathlib, sys

def camel(s):
    head, *rest = s.split('_')
    return head + ''.join(p[:1].upper() + p[1:] for p in rest)

names = []
for f in sorted(pathlib.Path('core/src').rglob('*.rs')):
    t = f.read_text()
    for m in re.finditer(r'#\[uniffi::export[^\]]*\]\s*\npub(?:\([^)]*\))? (?:async )?fn (\w+)', t):
        names.append((m.group(1), str(f.relative_to('core/src')), False))
    for m in re.finditer(r'#\[uniffi::export[^\]]*\]\s*\nimpl ([^\{]*)\{', t):
        i, depth = m.end(), 1
        while depth and i < len(t):
            depth += (t[i] == '{') - (t[i] == '}')
            i += 1
        for n in re.findall(r'\n    pub(?:\([^)]*\))? (?:async )?fn (\w+)', t[m.end():i]):
            names.append((n, str(f.relative_to('core/src')), True))

def calls_only(source, language):
    # Declarations and comments are not evidence of calls. In particular a Swift
    # helper may have the same camelCase name as an unused Rust export.
    source = re.sub(r'/\*.*?\*/', '', source, flags=re.S)
    source = re.sub(r'(?m)^\s*//.*$', '', source)
    keyword = 'func' if language == 'swift' else 'fn'
    return re.sub(r'\b' + keyword + r'\s+\w+', keyword + ' __declaration__', source)

# The app, not its tests: an export only a test calls is a fixture, and it
# reads as API to everyone else.
BRIDGE = pathlib.Path('swift/app/App/WalletServiceBridge.swift')

def strip_comments(source):
    source = re.sub(r'/\*.*?\*/', '', source, flags=re.S)
    return re.sub(r'(?m)^\s*//.*$', '', source)

def methods(source):
    """(name, start, end) for every `func` in `source`, end past its body."""
    found = []
    for m in re.finditer(r'\bfunc\s+(\w+)', source):
        open_brace = source.find('{', m.end())
        if open_brace < 0:
            continue
        i, depth = open_brace + 1, 1
        while depth and i < len(source):
            depth += (source[i] == '{') - (source[i] == '}')
            i += 1
        found.append((m.group(1), m.start(), i))
    return found

app_files = [f for f in pathlib.Path('swift').rglob('*.swift')
             if 'generated' not in f.parts and 'tests' not in f.parts]
elsewhere = calls_only('\n'.join(f.read_text() for f in app_files if f != BRIDGE), 'swift')
bridge = strip_comments(BRIDGE.read_text())
wrappers = methods(bridge)
body = {name: bridge[start:end] for name, start, end in wrappers}

def called(name, text):
    return re.search(rf'\b{re.escape(name)}\s*\(', text) is not None

def called_on_bridge(name, text):
    # Outside the bridge a wrapper is reached through an instance —
    # `WalletServiceBridge.shared.name(`. A bare `name(` there is some other
    # type's method that happens to share the spelling, as `AppState`'s
    # `appendChainOperationalEvent` shares the bridge wrapper's.
    return re.search(rf'\.{re.escape(name)}\s*\(', text) is not None

# A wrapper is live while something live calls it: the rest of the app, or the
# body of another live wrapper. Remove the dead until nothing changes.
live = set(body)
while True:
    dead = {name for name in live
            if not called_on_bridge(name, elsewhere)
            and not any(called(name, calls_only(body[other], 'swift'))
                        for other in live if other != name)}
    if not dead:
        break
    live -= dead

outside = bridge
for name, start, end in sorted(wrappers, key=lambda w: w[1], reverse=True):
    outside = outside[:start] + outside[end:]
swift = '\n'.join([elsewhere, calls_only(outside, 'swift')]
                  + [calls_only(body[name], 'swift') for name in sorted(live)])
cli = calls_only('\n'.join(f.read_text() for f in pathlib.Path('cli/src').rglob('*.rs')), 'rust')

# Known-reachable by another route: `new` is a constructor UniFFI needs.
#
# And the test fixtures the iOS suite needs across the binding, each with the
# reason a Rust test cannot stand in — a Rust test runs inside its own Tokio
# runtime and cannot catch a missing reactor on the Swift side.
ALLOWED = {
    'new',
    # Seeds wallets into the service the `AppState` tests drive.
    'core_wallet_state',
    # Seeds and clears the transaction store those same tests read back.
    'apply_transaction_command',
}

# A method is called on its object, so its Swift call has a receiver: a bare
# name could match a different method with the same spelling.
def swift_calls(name, is_method):
    receiver = r'\.' if is_method else r'\b'
    return re.search(rf'{receiver}{re.escape(camel(name))}\s*\(', swift) is not None

dead = [(n, f) for n, f, is_method in names
        if n not in ALLOWED
        and not swift_calls(n, is_method)
        and not re.search(rf'\b{re.escape(n)}\s*\(', cli)]

for n, f in sorted(dead):
    print(f"  {n:<46} {f}")
print(f"\n  {len(dead)} unreachable export(s)")
sys.exit(1 if dead else 0)
PY
