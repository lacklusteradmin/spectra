# Scripts

This directory contains build, source-check and runtime-data tools. CLI
acceptance suites, their helpers and independent test-fixture generators live
in [`tests/`](../tests/README.md). Use the Makefile for routine work:

```sh
make verify       # Formatting, Rust lint/tests, CLI tests and iOS simulator tests
make check-ui     # UI design tokens and SVG formatting
```

## Builds and binding generation

| File | Purpose |
|---|---|
| `build-ios.sh` | Run by the Xcode “Build Rust Core” phase: compiles core as a static library for the device or the arm64 simulator and generates the Swift bindings from it, replacing a binding only when its text changed so an edit that leaves the FFI as it was recompiles no Swift. |
| `ios-rust-build-env.sh` | Sourced by build scripts to set a consistent minimum iOS version and clear incompatible iOS build caches when that version changes. |

## Code and resource checks

These source scans help identify problems. Review findings for dynamic calls and
other cases a scan cannot resolve. Each scan is a Python script run as
`python3 -B scripts/<scan>.py` from any directory; it prints its findings and
a count, and exits non-zero when it found anything. `make lint` and
CI run every scan below except `check-design-tokens.py`, which `make check-ui`
runs.

| File | Purpose |
|---|---|
| `unreachable-exports.py` | Finds exported Rust interfaces with no detected Swift or CLI callers. |
| `uncalled-core-fns.py` | Finds public Rust core functions with no detected callers. |
| `unwritten-record-fields.py` | Finds `uniffi::Record` fields no production code writes. Builds the CLI in the `mir-scan` profile, with `emit-mir.sh` as the workspace wrapper writing core's and the CLI's MIR into `target/mir/`, and runs `record_writers.py` on the result. Its own profile keeps the dev build fresh. |
| `emit-mir.sh` | `RUSTC_WORKSPACE_WRAPPER` for that scan: adds `--emit=mir` to core's library and the CLI's binary, and passes every other crate through. |
| `record_writers.py` | Reads rustc's MIR, where every place carries its type, to find each record field's writes; its docstring says what counts as one. |
| `test-record-writers.py` | Compiles a fixture with the pinned rustc and checks what the MIR reader reports, so a toolchain that prints MIR differently fails there first. |
| `unused-strings.py` | Finds unused text and inconsistent translation keys across locales. |
| `swift-shell-literals.py` | Finds hard-coded chain names and amount precision in Swift, where domain rules should come from core. |
| `check-design-tokens.py` | Finds corner radii, spacing, literal colours, glass tints and glass shapes that bypass shared UI design tokens. |
| `source_scan.py` | What the scans share: production-source selection for Rust and Swift (test fixtures do not count as runtime callers or copy producers), call text with comments and declarations removed, and the report and exit status. |
| `test-source-scan.py` | Tests the shared helpers: production/test source selection, Rust module paths, call text and reporting. |

The function scan matches names rather than resolving receiver types or building
a call graph. An unrelated method with the same name can hide an unused method;
review these candidates manually even when the scan passes. The record-field
scan does resolve types — it reads them from the compiler — but it does not
follow a value through a function call, so a field set from a call's result
counts as written whatever the call returns.

## Icons

| File | Purpose |
|---|---|
| `normalize-icons.sh` | Normalizes crypto SVGs to the project's format. `--check` reports drift without modifying files. |
| `svgo.config.mjs` | SVG normalization rules used by the script above; this is a configuration file. |
| `export-swift-icons.sh` | Converts and synchronizes sources from `icons/` into the Xcode asset catalog, rendering app icons as PNGs. |

After editing icon sources, run:

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

## Runtime reference data generation

| File | Purpose |
|---|---|
| `generate-monero-checkpoints.py` | Monero height/timestamp checkpoints for `core/data/monero-checkpoints.json`. |
