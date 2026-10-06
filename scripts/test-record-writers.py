#!/usr/bin/env python3
"""Regression checks for the MIR reading in record_writers.py.

The fixture is compiled by the pinned rustc, so a toolchain that prints MIR
differently fails here rather than quietly passing the gate.
"""

from pathlib import Path
import subprocess
import tempfile
import unittest

from record_writers import Derives, analyse, records_in

REPOSITORY = Path(__file__).resolve().parent.parent

FIXTURE = '''
#[derive(Default, Clone, Debug)]
pub struct Record {
    pub built: Option<String>,
    pub spread: Option<String>,
    pub always_none: Option<String>,
    pub assigned: u64,
    pub pushed: Vec<u8>,
    pub test_only: bool,
    pub filled: Vec<String>,
    pub copied: Option<String>,
    pub copied_unwritten: Option<String>,
    pub in_closure: bool,
    pub in_async: String,
    pub through_ref: u32,
    pub same_name: u32,
}

#[derive(
    Default,
    Clone,
)]
pub struct Other {
    pub source: Option<String>,
    pub never: Option<String>,
    pub same_name: u32,
}

pub enum Kind {
    Record { built: Option<String>, spread: Option<String> },
}

pub fn build(name: String, other: &Other) -> Record {
    Record {
        built: Some(name),
        always_none: None,
        copied: other.source.clone(),
        copied_unwritten: other.never.clone(),
        filled: {
            let mut filled = Vec::new();
            filled.push(String::from("x"));
            filled
        },
        ..Default::default()
    }
}

pub fn other(source: String, same_name: u32) -> Other {
    Other { source: Some(source), never: None, same_name }
}

pub fn kind() -> Kind {
    Kind::Record { built: None, spread: Some(String::new()) }
}

pub fn again(record: &mut Record) {
    record.always_none = None;
    record.assigned = 7;
    record.pushed.push(1);
}

pub fn closure(mut record: Record) -> Record {
    let mut set = || record.in_closure = true;
    set();
    record
}

pub async fn later(mut record: Record) -> Record {
    record.in_async = String::from("done");
    record
}

fn helper(record: &mut Record, value: u32) {
    record.through_ref = value;
}

pub fn calls_helper(mut record: Record) -> Record {
    helper(&mut record, 3);
    record
}

#[cfg(test)]
mod tests {
    #[test]
    fn sets_a_field() {
        let mut record = super::Record::default();
        record.test_only = true;
    }
}
'''

DERIVES = '#[derive(Default, Clone, Debug, uniffi::Record)]\n'


class RecordWriterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix='spectra-record-writers-')
        root = Path(cls.temporary.name)
        cls.source = root / 'fixture.rs'
        cls.source.write_text(FIXTURE)
        cls.mir = root / 'fixture.mir'
        # From the repository, so rustup picks the pinned toolchain.
        subprocess.run(['rustc', '--edition', '2024', '--crate-type', 'lib',
                        '--crate-name', 'fixture', f'--emit=mir={cls.mir}', str(cls.source)],
                       cwd=REPOSITORY, check=True, capture_output=True)
        # The gate finds records by `uniffi::Record`, which the fixture cannot
        # depend on; describe both structs as records to the reader.
        cls.described = root / 'records.rs'
        cls.described.write_text(FIXTURE.replace('#[derive(Default, Clone, Debug)]\n', DERIVES)
                                 .replace('#[derive(\n    Default,\n    Clone,\n)]\n', DERIVES))

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def unwritten(self, front_ends=''):
        records = records_in([self.described])
        self.assertEqual(set(records), {'Record', 'Other'})
        fields, unbuilt = analyse([self.mir], records, REPOSITORY, front_ends)
        self.assertEqual(unbuilt, [])
        return set(fields)

    def test_only_fields_without_a_production_write_are_reported(self):
        self.assertEqual(self.unwritten(), {
            # Left to `..Default::default()`.
            ('Record', 'spread'),
            # `None` in the literal and in an assignment.
            ('Record', 'always_none'),
            # Written under `#[cfg(test)]` only.
            ('Record', 'test_only'),
            # A copy of a field that is never written.
            ('Record', 'copied_unwritten'),
            # Another record's `same_name` is written; this one is not.
            ('Record', 'same_name'),
            ('Other', 'never'),
        })

    def test_a_front_end_construction_writes_every_field(self):
        swift = 'let other = Other(source: "a", never: nil, sameName: 1)'
        self.assertEqual({field for field in self.unwritten(swift) if field[0] == 'Other'}, set())
        # The copy of `never` now copies a written field.
        self.assertNotIn(('Record', 'copied_unwritten'), self.unwritten(swift))

    def test_a_record_core_only_deserializes_is_read_from_outside(self):
        source = Path(self.temporary.name) / 'catalog.rs'
        source.write_text('#[derive(Debug, Deserialize, uniffi::Record)]\n'
                          '#[serde(deny_unknown_fields)]\npub struct Catalog {\n    pub name: String,\n}\n'
                          '/// Doc.\n#[derive(\n    Debug,\n    serde::Serialize,\n    serde::Deserialize,\n'
                          '    uniffi::Record,\n)]\npub struct Stored {\n    pub name: String,\n}\n')
        records = records_in([source])
        self.assertTrue(records['Catalog'].read_from_outside())
        self.assertFalse(records['Stored'].read_from_outside())

    def test_derived_impls_are_told_from_written_ones_by_their_span(self):
        source = Path(self.temporary.name) / 'spans.rs'
        source.write_text('#[derive(Debug, uniffi::Record)]\npub struct A {}\n#[derive(\n'
                          '    Clone,\n    serde::Deserialize,\n)]\npub struct B {}\nimpl B {\n}\n')
        derives = Derives(self.temporary.name)
        self.assertTrue(derives.derived('fn <impl at spans.rs:1:17: 1:31>::try_read(_1: &mut &[u8])'))
        self.assertTrue(derives.derived('fn x::_::<impl at spans.rs:5:5: 5:23>::deserialize(_1: __D)'))
        self.assertFalse(derives.derived('fn <impl at spans.rs:8:1: 8:7>::new() -> B {'))
        # A struct defined inside a written impl's method, deriving its own.
        self.assertTrue(derives.derived('fn <impl at spans.rs:8:1: 8:7>::f::{closure#0}::_::'
                                        '<impl at spans.rs:4:5: 4:10>::clone(_1: &B) -> B {'))


if __name__ == '__main__':
    unittest.main()
