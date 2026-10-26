#!/usr/bin/env bash
# FFI record fields no production code writes.
#
# A record field that is always `None`, `0` or empty still crosses the FFI,
# still renders, and still reads as data. rustc's `dead_code` cannot see one
# on a `pub` struct in a library, and a name-based scan cannot tell which
# struct `x.field` writes, so this asks the compiler: it emits the MIR of core
# and the CLI — production code only, with types resolved — and
# `record_writers.py` looks for a write of each `uniffi::Record` field there or
# a construction of the record in hand-written Swift. That file says
# what counts as a write.
#
# Exits non-zero when any field has no writer, so it can gate. Delete the
# field; a record that only carries it for a uniform shape does not need it.
set -euo pipefail
cd "$(dirname "$0")/.."

mir="$PWD/target/mir"
mkdir -p "$mir"

# Its own profile, so the dev build stays fresh for the tests and the CLI;
# the wrapper adds the MIR to core's library and the CLI's binary alone.
emit() {
    SPECTRA_MIR_DIR="$mir" RUSTC_WORKSPACE_WRAPPER="$PWD/scripts/emit-mir.sh" \
        cargo build --quiet --profile mir-scan -p spectra_cli --bin spectra
}
emit
if [[ ! -f $mir/core.mir || ! -f $mir/cli.mir ]]; then
    # Cargo found the crates fresh, so rustc did not run and could not
    # replace MIR that was deleted. Only a rebuild writes it again.
    cargo clean --quiet --profile mir-scan -p spectra_core -p spectra_cli
    emit
fi
python3 -B scripts/record_writers.py "$mir/core.mir" "$mir/cli.mir"
