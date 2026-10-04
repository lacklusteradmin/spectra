#!/usr/bin/env python3
"""Regression checks for production/test source selection used by the scans."""

from pathlib import Path
import tempfile
import unittest

from source_scan import frontend_test, production_text, test_modules


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
        repository = Path(__file__).resolve().parent.parent
        parent = repository / 'core/src/api/substrate_json_rpc.rs'
        child = repository / 'core/src/api/substrate_json_rpc/tests.rs'
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
        for name in ('swift/tests/Fixture.swift', 'kotlin/app/src/test/Fixture.kt',
                     'kotlin/app/src/androidTest/Fixture.kt', 'kotlin/app/src/testFixtures/Fixture.kt',
                     'kotlin/app/src/androidTest/res/values/strings.xml'):
            with self.subTest(source=name):
                path = self.write(name, 'fixture copy')
                self.assertTrue(frontend_test(path))
                self.assertEqual(production_text(path, set()), '')
        path = self.write('kotlin/app/src/main/Screen.kt', 'production copy')
        self.assertFalse(frontend_test(path))
        self.assertEqual(production_text(path, set()), 'production copy')


if __name__ == '__main__':
    unittest.main()
