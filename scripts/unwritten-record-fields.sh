#!/usr/bin/env bash
# FFI record fields no production code writes.
#
# A record field that is always `None`, `0` or empty still crosses the FFI,
# still renders, and still reads as data. rustc's `dead_code` cannot see one
# on a `pub` struct in a library, and a name-based scan cannot tell which
# struct `x.field` writes, so this asks the compiler: it emits the MIR of core
# and the CLI — production code only, with types resolved — and
# `record_writers.py` looks for a write of each `uniffi::Record` field there or
# a construction of the record in hand-written Swift or Kotlin. That file says
# what counts as a write.
#
# Exits non-zero when any field has no writer, so it can gate. Delete the
# field; a record that only carries it for a uniform shape does not need it.
set -euo pipefail
cd "$(dirname "$0")/.."

mir="$PWD/target/mir"
mkdir -p "$mir"

# Extra rustc arguments apply to the named target only; dependencies reuse
# the ordinary debug build.
emit() {
    local package=$1 target=$2 output=$3
    cargo rustc --quiet -p "$package" $target -- --emit=mir="$output"
    if [[ ! -f $output ]]; then
        # Cargo found the target fresh, so rustc did not run and could not
        # replace MIR that was deleted. Only a rebuild writes it again.
        cargo clean --quiet -p "$package"
        cargo rustc --quiet -p "$package" $target -- --emit=mir="$output"
    fi
}
emit spectra_core --lib "$mir/core.mir"
emit spectra_cli '--bin spectra' "$mir/cli.mir"
python3 -B scripts/record_writers.py "$mir/core.mir" "$mir/cli.mir"
