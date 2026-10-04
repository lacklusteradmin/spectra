"""Shared source selection for the dead-code and localization scans."""

import pathlib
import re
import subprocess


def hand_written(root, suffix):
    """Tracked or new files, excluding ignored generated output."""
    listed = subprocess.run(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', root],
        check=True, capture_output=True, text=True).stdout.split('\0')
    return [pathlib.Path(p) for p in sorted(listed)
            if p.endswith(suffix) and pathlib.Path(p).exists()]


def frontend_test(path):
    """Swift tests and Kotlin test, instrumented-test and fixture source roots."""
    return any(part in {'tests', 'test', 'androidTest', 'testFixtures'}
               for part in path.parts)


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
            # explicit paths are relative to this file's containing directory.
            start = i
            while start > 0 and SKIPPABLE.match(lines[start - 1]):
                start -= 1
            explicit = re.search(r'#\[path\s*=\s*"([^"]+)"\]', '\n'.join(lines[start:j]))
            if explicit:
                found.add(path.parent / explicit.group(1))
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


def production_text(path, declared_tests):
    text = path.read_text(errors='replace')
    if path.suffix == '.rs':
        return ''.join(line for _, line in split_test_code(path, text, declared_tests)[0])
    return '' if frontend_test(path) else text
