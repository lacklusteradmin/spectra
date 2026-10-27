#!/usr/bin/env python3
"""Regression checks for what the source scans share."""

import contextlib
import io
from pathlib import Path
import tempfile
import unittest

from source_scan import (app_swift, calls_only, camel, frontend_test, production_text, report,
                         strip_comments, test_modules)

REPOSITORY = Path(__file__).resolve().parent.parent


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
        parent = REPOSITORY / 'core/src/api/substrate_json_rpc.rs'
        child = REPOSITORY / 'core/src/api/substrate_json_rpc/tests.rs'
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

    def test_frontend_test_source_roots_do_not_produce_shipped_copy(self):
        path = self.write('swift/tests/Fixture.swift', 'fixture copy')
        self.assertTrue(frontend_test(path))
        self.assertEqual(production_text(path, set()), '')
        path = self.write('swift/app/Screen.swift', 'production copy')
        self.assertFalse(frontend_test(path))
        self.assertEqual(production_text(path, set()), 'production copy')


class AppSwiftTests(unittest.TestCase):
    def test_lists_the_app_without_its_tests_or_generated_bindings(self):
        with contextlib.chdir(REPOSITORY):
            paths = app_swift()
        self.assertIn(Path('swift/app/App/WalletServiceBridge.swift'), paths)
        self.assertFalse([path for path in paths if path.parts[1] in {'tests', 'generated'}])
        self.assertEqual(paths, sorted(paths))


class CallTextTests(unittest.TestCase):
    def test_camel_follows_uniffi(self):
        self.assertEqual(camel('core_wallet_state'), 'coreWalletState')
        self.assertEqual(camel('new'), 'new')

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
        swift = calls_only('func refresh() { reload() }\n', 'func')
        self.assertNotIn('refresh', swift)
        self.assertIn('reload()', swift)


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
