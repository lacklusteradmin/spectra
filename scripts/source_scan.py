"""What the source scans share: which files they read, what in a file is a
call, and how a scan reports.

Each scan is a script beside this one, run from any directory by
`python3 -B scripts/<scan>.py`. It prints its findings and a count, and exits
non-zero when it found anything, so it can gate.
"""

import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def run(scan):
    """Run `scan` from the repository root and exit with its status."""
    os.chdir(ROOT)
    sys.exit(scan())


def report(findings, summary):
    """Print each finding, then the summary; the scan's exit status."""
    for line in findings:
        print(line)
    print(f'\n  {summary}')
    return 1 if findings else 0


def hand_written(root, suffix):
    """Tracked or new files, excluding ignored generated output."""
    listed = subprocess.run(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', root],
        check=True, capture_output=True, text=True).stdout.split('\0')
    return [pathlib.Path(p) for p in sorted(listed)
            if p.endswith(suffix) and pathlib.Path(p).exists()]


EXPORTED_FN = re.compile(r'#\[uniffi::export[^\]]*\]\s*\npub(?:\([^)]*\))? (?:async )?fn (\w+)')
EXPORTED_IMPL = re.compile(r'#\[uniffi::export[^\]]*\]\s*\nimpl ([^\{]*)\{')
METHOD = re.compile(r'\n    pub(?:\([^)]*\))? (?:async )?fn (\w+)')


def block_end(text, i):
    """Index just past the `}` closing the block whose `{` is just before `i`."""
    depth = 1
    while depth and i < len(text):
        depth += (text[i] == '{') - (text[i] == '}')
        i += 1
    return i


def exports(root='core/src'):
    """(name, file under `root`, is a method) for every UniFFI export.

    Exporting an `impl` block exports every method in it.
    """
    found = []
    for path in hand_written(root, '.rs'):
        text, where = path.read_text(), str(path.relative_to(root))
        found.extend((name, where, False) for name in EXPORTED_FN.findall(text))
        for impl in EXPORTED_IMPL.finditer(text):
            body = text[impl.end():block_end(text, impl.end())]
            found.extend((name, where, True) for name in METHOD.findall(body))
    return found


def strip_comments(text):
    """Block comments and whole-line `//` comments."""
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    return re.sub(r'(?m)^\s*//.*$', '', text)


def calls_only(text, keyword):
    """`text` with what only looks like a call removed.

    Comments, `use` lines and the name a declaration introduces (after
    `keyword`) are not evidence of a call.
    """
    text = re.sub(r'(?m)^\s*(?:pub )?use .*$', '', strip_comments(text))
    return re.sub(r'\b' + keyword + r'\s+\w+', keyword + ' __declaration__', text)


ATTRIBUTE = re.compile(r'\s*#\[(cfg\(test\)|test|tokio::test)')
SKIPPABLE = re.compile(r'\s*(#\[|///|//!|$)')


def item_end(lines, start):
    """Index just past the Rust item beginning at `start`."""
    # A declaration has no body: counting braces after `mod tests;` would
    # consume the production items that follow it.
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
    """Files pulled in by an external `#[cfg(test)] mod X;` declaration.

    Names alone are not evidence: diagnostics/self_tests.rs is production
    code the diagnostics screen runs.
    """
    found = set()
    for path in paths:
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            if not ATTRIBUTE.match(line):
                continue
            j = i
            while j < len(lines) and SKIPPABLE.match(lines[j]):
                j += 1
            if j == len(lines):
                continue
            module = re.match(r'\s*(?:pub(?:\([^)]*\))? )?mod (\w+);', lines[j])
            if not module:
                continue
            # A path attribute can precede or follow cfg(test). Top-level
            # explicit paths are relative to this file's containing directory
            # and may leave it with `..`, so they are normalized to match the
            # listed path.
            start = i
            while start > 0 and SKIPPABLE.match(lines[start - 1]):
                start -= 1
            explicit = re.search(r'#\[path\s*=\s*"([^"]+)"\]', '\n'.join(lines[start:j]))
            if explicit:
                found.add(pathlib.Path(os.path.normpath(path.parent / explicit.group(1))))
            else:
                # foo.rs owns foo/; lib.rs, main.rs and mod.rs already sit in
                # their module directory. Both child-file layouts are legal.
                directory = path.parent if path.name in {'lib.rs', 'main.rs', 'mod.rs'} else path.with_suffix('')
                found.add(directory / f"{module.group(1)}.rs")
                found.add(directory / module.group(1) / "mod.rs")
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
