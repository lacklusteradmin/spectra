#!/usr/bin/env python3
"""Regression checks for what the source scans share."""

import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest

from source_scan import calls_only, exports, report, split_test_code, strip_comments, test_modules

REPOSITORY = Path(__file__).resolve().parent.parent


def production_text(path, declared_tests):
    """A Rust file's text outside its tests, as the scans read it."""
    text = path.read_text(errors='replace')
    return ''.join(line for _, line in split_test_code(path, text, declared_tests)[0])


class SourceSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='spectra-source-scan-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def test_ordinary_module_children_use_the_module_directory(self):
        parent = self.write('api/foo.rs', '#[cfg(test)]\nmod checks;\npub fn production() {}\n')
        child = self.write('api/foo/checks.rs', 'const COPY: &str = "fixture";\n')
        declared = test_modules([parent])
        self.assertIn(child, declared)
        self.assertIn(self.root / 'api/foo/checks/mod.rs', declared)
        self.assertNotIn(self.root / 'api/checks.rs', declared)
        self.assertEqual(production_text(child, declared), '')
        self.assertIn('pub fn production()', production_text(parent, declared))

    def test_root_and_mod_files_use_the_containing_directory(self):
        for name in ('lib.rs', 'main.rs', 'nested/mod.rs'):
            with self.subTest(source=name):
                parent = self.write(name, '#[cfg(test)]\nmod checks;\n')
                declared = test_modules([parent])
                self.assertIn(parent.parent / 'checks.rs', declared)
                self.assertIn(parent.parent / 'checks/mod.rs', declared)

    def test_explicit_path_is_relative_to_the_containing_directory(self):
        for name in ('lib.rs', 'main.rs', 'api/foo.rs', 'nested/mod.rs'):
            for attributes in ('#[cfg(test)]\n#[path = "fixture_file.rs"]',
                               '#[path = "fixture_file.rs"]\n#[cfg(test)]'):
                with self.subTest(source=name, attributes=attributes):
                    parent = self.write(name, attributes + '\nmod checks;\n')
                    declared = test_modules([parent])
                    self.assertEqual(declared, {parent.parent / 'fixture_file.rs'})

    def test_actual_substrate_tests_are_declared(self):
        parent = REPOSITORY / 'core/src/api/substrate_json_rpc/mod.rs'
        child = REPOSITORY / 'core/src/api/tests/substrate_json_rpc.rs'
        declared = test_modules([parent])
        self.assertIn(child, declared)
        self.assertEqual(production_text(child, declared), '')

    def test_inline_tests_do_not_hide_later_production(self):
        path = self.write('foo.rs', 'pub fn before() {}\n#[cfg(test)]\nmod checks {\n'
                          '    fn fixture() { let copy = "fixture"; }\n}\npub fn after() {}\n')
        text = production_text(path, set())
        self.assertIn('pub fn before()', text)
        self.assertIn('pub fn after()', text)
        self.assertNotIn('fixture', text)

    def test_rust_test_functions_do_not_hide_later_production(self):
        path = self.write('foo.rs', '#[test]\nfn fixture() {}\n'
                          '#[tokio::test]\nasync fn other_fixture() {}\npub fn after() {}\n')
        text = production_text(path, set())
        self.assertIn('pub fn after()', text)
        self.assertNotIn('fixture', text)

    def test_runtime_diagnostic_self_tests_remain_production(self):
        path = self.write('diagnostics/self_tests.rs', 'pub fn run() { let copy = "runtime"; }\n')
        self.assertIn('runtime', production_text(path, set()))



class ExportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='spectra-source-scan-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_functions_and_every_method_of_an_exported_impl(self):
        (self.root / 'lib.rs').write_text(
            '#[uniffi::export]\npub fn free() {}\n'
            '#[uniffi::export(async_runtime = "tokio")]\npub async fn later() {}\n'
            'pub fn internal() {}\n'
            '#[uniffi::export]\nimpl Service {\n    pub fn method(&self) {}\n'
            '    pub async fn other(&self) {}\n}\n'
            'impl Service {\n    pub fn helper(&self) {}\n}\n')
        with contextlib.chdir(self.root):
            subprocess.run(['git', 'init', '-q'], check=True)
            found = exports('.')
        self.assertEqual(sorted(found), [('free', 'lib.rs', False), ('later', 'lib.rs', False),
                                         ('method', 'lib.rs', True), ('other', 'lib.rs', True)])

    def test_core_exports_are_found(self):
        with contextlib.chdir(REPOSITORY):
            names = {name for name, _, _ in exports()}
        self.assertIn('list_all_builtin_token_deployments', names)


class CallTextTests(unittest.TestCase):
    def test_comments_are_not_code(self):
        text = 'a()\n  // b()\n/// c()\n/* d()\n e() */ f()\nlet x = 1 // g()\n'
        stripped = strip_comments(text)
        for gone in ('b()', 'c()', 'd()', 'e()'):
            self.assertNotIn(gone, stripped)
        # Only whole-line comments go: a trailing one stays.
        for kept in ('a()', 'f()', 'g()'):
            self.assertIn(kept, stripped)

    def test_declarations_and_imports_are_not_calls(self):
        rust = calls_only('use crate::wallet::load;\npub use x::y;\npub fn load() {}\n'
                          'fn main() { load(); }\n', 'fn')
        self.assertNotIn('wallet', rust)
        self.assertNotIn('x::y', rust)
        self.assertNotIn('fn load', rust)
        self.assertNotIn('fn main', rust)
        self.assertIn('load();', rust)


class ReportTests(unittest.TestCase):
    def report(self, findings):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = report(findings, f'{len(findings)} finding(s)')
        return status, output.getvalue()

    def test_findings_fail_the_scan(self):
        self.assertEqual(self.report(['  a', '  b']), (1, '  a\n  b\n\n  2 finding(s)\n'))

    def test_nothing_found_passes(self):
        self.assertEqual(self.report([]), (0, '\n  0 finding(s)\n'))


if __name__ == '__main__':
    unittest.main()
