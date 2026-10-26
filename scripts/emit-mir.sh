#!/usr/bin/env bash
# RUSTC_WORKSPACE_WRAPPER for unwritten-record-fields.sh: compiles a workspace
# crate as cargo asks, and core's library and the CLI's binary into MIR as
# well, at $SPECTRA_MIR_DIR/core.mir and cli.mir. One build then emits both,
# compiling core once.
set -euo pipefail
rustc="$1"
shift
name="" kind=""
args=("$@")
for ((i = 0; i + 1 < ${#args[@]}; i++)); do
    case "${args[i]}" in
        --crate-name) name="${args[i + 1]}" ;;
        --crate-type) kind="${args[i + 1]}" ;;
    esac
done
case "$name:$kind" in
    spectra_core:lib) exec "$rustc" "$@" --emit=mir="$SPECTRA_MIR_DIR/core.mir" ;;
    spectra:bin) exec "$rustc" "$@" --emit=mir="$SPECTRA_MIR_DIR/cli.mir" ;;
    *) exec "$rustc" "$@" ;;
esac
