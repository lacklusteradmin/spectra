#!/usr/bin/env python3
"""Per-chain facts and protocol precision written into the app.

Every other gate here looks outward from Rust — exports nothing calls,
functions nothing calls — so none of them can see the app deciding something
by naming a chain. This reads the app, not core.

  * A string literal that is exactly a chain's catalog name or id
    (`"Ethereum"`, `"bitcoin"`) is a chain chosen by spelling. Ask the
    registry for the fact that made it that chain.
  * A fixed `%.Nf` count renders an amount at a precision core did not pick.
    Amounts and fees go through `formattedAmountValue` / `formattedNetworkFee`.
    A percentage (`%.1f%%`) is not an amount and is allowed.

Tests and generated bindings are out of scope; comments are not code.
"""

import pathlib
import re
import tomllib

from source_scan import app_swift, report, run

LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
FIXED = re.compile(r'%\.\d+f(?!%%)')

# A literal that spells a chain word and means something else, with why.
ALLOWED = {
    ('swift/app/Models/StaticContentCatalog.swift', 'Base'): "Xcode's base localization, not the Base chain",
}


def code_lines(text):
    """(line number, code) for each line, comments blanked."""
    text = re.sub(r'/\*.*?\*/', lambda m: '\n' * m.group(0).count('\n'), text, flags=re.S)
    for number, line in enumerate(text.split('\n'), 1):
        # Drop a trailing line comment, but not `//` inside a string literal.
        out, in_string, i = [], False, 0
        while i < len(line):
            c = line[i]
            if c == '\\' and in_string:
                out.append(line[i:i + 2])
                i += 2
                continue
            if c == '"':
                in_string = not in_string
            if not in_string and line.startswith('//', i):
                break
            out.append(c)
            i += 1
        yield number, ''.join(out)


def scan():
    chains = tomllib.loads(pathlib.Path('core/data/chains.toml').read_text())['chains']
    chain_words = {chain['name'] for chain in chains} | {chain['id'] for chain in chains}

    findings = []
    for path in app_swift():
        for number, line in code_lines(path.read_text()):
            for literal in LITERAL.findall(line):
                if literal in chain_words and (str(path), literal) not in ALLOWED:
                    findings.append(f'  {path}:{number}  chain named by spelling: "{literal}"')
                if FIXED.search(literal):
                    findings.append(f'  {path}:{number}  fixed precision: "{literal}"')
    return report(findings, f'{len(findings)} chain literal(s) or fixed precision format(s) in the app')


if __name__ == '__main__':
    run(scan)
