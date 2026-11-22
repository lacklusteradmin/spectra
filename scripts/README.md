# Scripts

This directory contains source-check, icon, runtime-data and Apple package
tools. CLI acceptance suites, their helpers and independent test-fixture
generators live in [`tests/`](../tests/README.md). Use the Makefile for routine
work:

```sh
make lint         # Formatting, clippy and the source scan
make check-icons  # SVG formatting of the icon library
```

The app-source checks live in spectra-apple, beside the app they read.

## Apple package

| File | Purpose |
|---|---|
| `build-apple-package.sh` | Builds core as the SpectraCore Swift package an Apple app links: a static library for the iPhone, the arm64 simulator and the Mac, bundled as an XCFramework, the C header of its FFI as a module of its own, and the UniFFI Swift bindings generated from it. It takes the app's minimum OS versions, a `dev` or `release` profile and the platforms to build; a run that finds nothing changed leaves the package as it was, and the header and bindings are rewritten only when their text changed, so a core edit that leaves the FFI as it was recompiles no app Swift. spectra-apple's `make core` runs it. |

## Code checks

A source scan helps identify problems. It reads source and compiles nothing.
Review findings for dynamic calls and other cases a scan cannot resolve. A scan
is a Python script run as `python3 -B scripts/<scan>.py` from any directory; it
prints its findings and a count, and exits non-zero when it found anything.
`make lint` runs it.

| File | Purpose |
|---|---|
| `uncalled-core-fns.py` | Finds public Rust core functions below the FFI surface with no detected callers. |
| `source_scan.py` | What a scan shares: production-source selection for Rust (test code does not count as a runtime caller), the UniFFI exports, call text with comments and declarations removed, and the report and exit status. |
| `test-source-scan.py` | Tests the shared helpers: production/test source selection, Rust module paths, export detection, call text and reporting. |

The function scan matches names rather than resolving receiver types or building
a call graph. An unrelated method with the same name can hide an unused method;
review these candidates manually even when the scan passes.

## Icons

Normalizing needs `svgo` 4.1.0 (`brew install svgo`). The check compares bytes,
and another svgo version can format the same icon differently.

| File | Purpose |
|---|---|
| `normalize-icons.sh` | Normalizes the SVGs in `icons/crypto/` to the project's format. `--check` reports drift without modifying files. |
| `svgo.config.mjs` | SVG normalization rules used by the script above; this is a configuration file. |

## Runtime reference data generation

| File | Purpose |
|---|---|
| `generate-monero-checkpoints.py` | Monero height/timestamp checkpoints for `core/data/monero-checkpoints.json`. |
