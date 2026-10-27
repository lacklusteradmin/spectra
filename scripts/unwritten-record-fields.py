#!/usr/bin/env python3
"""FFI record fields no production code writes.

A record field that is always `None`, `0` or empty still crosses the FFI,
still renders, and still reads as data. rustc's `dead_code` cannot see one on
a `pub` struct in a library, and a name-based scan cannot tell which struct
`x.field` writes, so this asks the compiler: it emits the MIR of core and the
CLI — production code only, with types resolved — and `record_writers.py`
looks for a write of each `uniffi::Record` field there or a construction of
the record in hand-written Swift. That file says what counts as a write.

Delete a field this reports; a record that only carries it for a uniform
shape does not need it.
"""

import os
import subprocess
import sys

from record_writers import analyse, records_in
from source_scan import ROOT, app_swift, hand_written, report, run

MIR = ROOT / 'target/mir'


def cargo(*arguments, env=None):
    """Run cargo, exiting with its status when it fails."""
    status = subprocess.run(['cargo', *arguments, '--quiet'], env=env).returncode
    if status:
        sys.exit(status)


def emit_mir():
    """The MIR of core's library and the CLI's binary, emitted into target/mir.

    The build has its own profile, so the dev build stays fresh for the tests
    and the CLI; `emit-mir.sh`, the workspace wrapper, adds the MIR to those
    two crates alone.
    """
    MIR.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, SPECTRA_MIR_DIR=str(MIR),
               RUSTC_WORKSPACE_WRAPPER=str(ROOT / 'scripts/emit-mir.sh'))
    build = ('build', '--profile', 'mir-scan', '-p', 'spectra_cli', '--bin', 'spectra')
    cargo(*build, env=env)
    files = [MIR / 'core.mir', MIR / 'cli.mir']
    if not all(file.is_file() for file in files):
        # Cargo found the crates fresh, so rustc did not run and could not
        # replace MIR that was deleted. Only a rebuild writes it again.
        cargo('clean', '--profile', 'mir-scan', '-p', 'spectra_core', '-p', 'spectra_cli')
        cargo(*build, env=env)
    return files


def scan():
    mir_files = emit_mir()
    records = records_in(hand_written('core/src', '.rs'))
    front_ends = '\n'.join(path.read_text() for path in app_swift())
    fields, unbuilt = analyse(mir_files, records, '.', front_ends)
    findings = [f'  {record + "." + field:<56} {records[record].path}:{records[record].line}'
                for record, field in fields]
    findings += [f'  {record:<56} no struct literal in the MIR' for record in unbuilt]
    return report(findings, f'{len(fields)} unwritten record field(s), '
                            f'{len(unbuilt)} unbuilt record(s)')


if __name__ == '__main__':
    run(scan)
