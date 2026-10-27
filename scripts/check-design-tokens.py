#!/usr/bin/env python3
"""Swift views that restate a design value SpectraLayout owns.

docs/IOS-UI.md defines the spacing scale, the corner-radius scale and the two
Liquid Glass tints; swift/app/Components/SpectraLayout.swift spells them in
Swift. A screen that writes the number instead of the token drifts silently,
and no review catches it by eye.

The scale itself is not policed here. Changing a step is a design decision:
edit docs/IOS-UI.md and SpectraLayout.swift together.
"""

import pathlib
import re

from source_scan import app_swift, report, run

TOKENS = pathlib.Path('swift/app/Components/SpectraLayout.swift')
# Decorative artwork with a fixed palette of its own ends the line, or a glass
# call's first line, with this.
ARTWORK = 'design-tokens: artwork'


def lines_matching(pattern, unless=None):
    """A rule that reports each line `pattern` finds and `unless` does not."""
    pattern, unless = re.compile(pattern), unless and re.compile(unless)

    def hits(path, text):
        for number, line in enumerate(text.split('\n'), 1):
            if pattern.search(line) and not (unless and unless.search(line)):
                yield f'{path}:{number}:{line}'
    return hits


def calls(text, name):
    """(start, end) of each `name(…)` call whose parentheses balance."""
    start = text.find(name + '(')
    while start >= 0:
        i, depth = start + len(name), 0
        while i < len(text):
            depth += (text[i] == '(') - (text[i] == ')')
            i += 1
            if not depth:
                yield start, i
                break
        else:
            i = start + 1
        start = text.find(name + '(', i)


NESTED_SHAPE = re.compile(r'Radius\.(inner|control)|in: \.(circle|capsule)|\b(Circle|Capsule)\(')


def glass_in_card(path, text):
    # The call is read whole, since its shape often sits on a later line than
    # `glassEffect(`.
    for start, end in calls(text, 'glassEffect'):
        call = text[start:end]
        if ARTWORK in text[start:].split('\n', 1)[0] or not NESTED_SHAPE.search(call):
            continue
        line = 1 + text.count('\n', 0, start)
        yield f"{path}:{line}: {re.sub(r'\s+', ' ', call)}"


# (title, remedy, rule) in the order they report.
RULES = [
    # A numeric radius at a call site. Component property declarations
    # (`var cornerRadius: CGFloat = 6`) are a component's own geometry and do
    # not match.
    ('Numeric corner radius outside SpectraLayout:',
     'use SpectraLayout.Radius (card/inner/control)',
     lines_matching(r'cornerRadius: [0-9]')),
    # A numeric padding, stack spacing or spacer minimum. `spacing: 0` is the
    # absence of spacing, not a step, and stays literal. A component's own
    # geometry — a frame, an offset, an icon size — is not spacing and is not
    # matched.
    ('Numeric spacing outside SpectraLayout:',
     'use SpectraLayout.Space (xxs/xs/s/m/l/xl) or a named SpectraLayout value',
     lines_matching(r'padding\((\.[a-zA-Z]+, )?[0-9]|(^|[^A-Za-z])spacing: [1-9]|minLength: [1-9]')),
    # A literal orange. The theme colour is the asset catalog's AccentColor
    # (`.tint`, `Color.accentColor`) so that it can change in one place; a
    # warning is `.spectraWarning`, which does not follow the theme. A comment
    # line is not code.
    ('Literal orange outside SpectraLayout:',
     'use .tint / Color.accentColor for the theme colour, .spectraWarning for warnings and pending states',
     lines_matching(r'\borange\b', unless=re.escape(ARTWORK) + r'|^ *//')),
    # A raw neutral glass tint. Accent-tinted glass (.orange/.red notices)
    # carries its own colour and is deliberately not a token.
    ('Raw white glass tint outside SpectraLayout:',
     'use SpectraLayout.GlassTint.elevated / .content, or spectraElevatedFill / spectraCardFill',
     lines_matching(re.escape('tint(.white.opacity('))),
    # Glass inside a card. Glass stops at the card, and every card has the
    # card radius; a glass shape on a smaller step, a circle or a capsule is a
    # surface nested inside one — an input, a chip, a tile, an icon backplate —
    # which is glass on glass.
    ('Glass surface inside a card:',
     'use spectraInsetFill / SpectraLayout.insetFill for a surface inside a card',
     glass_in_card),
]


def scan():
    views = [(path, path.read_text()) for path in app_swift('swift/app') if path != TOKENS]
    lines, count = [], 0
    for title, remedy, rule in RULES:
        hits = [hit for path, text in views for hit in rule(path, text)]
        if hits:
            lines += ['', title] + [f'  {hit}' for hit in hits] + [f'  -> {remedy}']
            count += len(hits)
    return report(lines, f'{count} design token violation(s)')


if __name__ == '__main__':
    run(scan)
