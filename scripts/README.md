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
| `build-ios.sh` | Run by the Xcode “Build Rust Core” phase: compiles the Rust library for the device or the arm64 simulator and generates the Swift bindings from it. |
| `build-android.sh` | Compiles Rust libraries for Android architectures and copies them to `jniLibs`. Requires the Android NDK and cargo-ndk. Accepts `--release`. |
| `bindgen-android.sh` | Generates Kotlin bindings from the compiled Rust library. |
| `ios-rust-build-env.sh` | Sourced by build scripts to set a consistent minimum iOS version and clear incompatible iOS build caches when that version changes. |

## Code and resource checks

These source scans help identify problems. Review findings for dynamic calls and
other cases a scan cannot resolve. `make lint` and CI run every scan below except
`check-design-tokens.sh`, which `make check-ui` runs.

| File | Purpose |
|---|---|
| `unreachable-exports.sh` | Finds exported Rust interfaces with no detected Swift or CLI callers. |
| `uncalled-core-fns.sh` | Finds public Rust core functions with no detected callers. |
| `unwritten-record-fields.sh` | Finds `uniffi::Record` fields no production code writes. Builds core and the CLI with `--emit=mir` into `target/mir/` and runs `record_writers.py` on the result. |
| `record_writers.py` | Reads rustc's MIR, where every place carries its type, to find each record field's writes; its docstring says what counts as one. |
| `test-record-writers.py` | Compiles a fixture with the pinned rustc and checks what the MIR reader reports, so a toolchain that prints MIR differently fails there first. |
| `unused-strings.sh` | Finds unused text and inconsistent translation keys across locales. |
| `source_scan.py` | Shared production-source extraction for Rust and foreign source scans; test fixtures do not count as runtime callers or copy producers. |
| `test-source-scan.py` | Tests production/test source selection and Rust module paths. |
| `swift-shell-literals.sh` | Finds hard-coded chain names and amount precision in Swift, where domain rules should come from core. |
| `check-design-tokens.sh` | Finds corner radii, opacity values and other style literals that bypass shared UI design tokens. |

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
