# Spectra — Architecture decisions

**Goal:** shared Rust core, a CLI that proves the core needs no platform, and
native iOS (Liquid Glass) and Android (Material You) shells.

This file holds **decisions and their reasons** — the things reading the code
cannot tell you. It deliberately contains no file-by-file map: `ls` is more
accurate than any list checked into git, and the list this file used to carry
had rotted (it named `ffi.rs` and `utxo.rs`, neither of which exists).

For where the architecture is going and how far along it is, see
[`PLAN.md`](../PLAN.md) — the plan of record. For how to write code that
crosses the FFI, see [`FFI-BOUNDARY.md`](FFI-BOUNDARY.md).

---

## Why the workspace is small

Spectra is an *application*, not a library.

```
core/                  Rust domain logic and the UniFFI-exported surface
ffi/                   UniFFI binding crate — a one-line re-export shim
cli/                   a full front end, and the check that core is platform-neutral
tools/uniffi-bindgen/  a tiny binary, used only at codegen time
```

**`core` is one crate, not a crate per domain.** Splitting it (a
derivation-crate, a fetch-crate, …) was considered and rejected: it multiplies
Cargo overhead and buys isolation nothing downstream can use. There are no
published crates, no `[features]` flags for optional functionality, and no
semver commitments between crates — nothing outside this repo consumes them.

**`core` builds without a mobile toolchain.** `cargo test --workspace` needs no
Xcode and runs the full suite, which is what makes it the gate on every change.

---

## Design principles

- **Vertical slicing in `chains/`.** Each chain owns its full pipeline in one
  file. Some chain-local duplication is intentional where it makes protocol
  behaviour clearer. Pure wrapper repetition is not: fifty `derive<Chain>`
  exports were deleted in favour of one dispatcher, and the rule that separates
  the two is whether the repetition carries protocol meaning or only a name.

- **Decentralized FFI.** Each `#[uniffi::export]` lives next to the logic it
  exposes. Centralizing them was tried and abandoned — the boundary tracking
  grew faster than the surface itself.

  The corollary, learned the expensive way: **count the exported surface from
  the generated bindings, not from the source.** One macro invocation can
  expand to a hundred exports, and `grep -c uniffi::export` will report it as
  one. That gap hid ~120 functions.

- **`registry::Chain` is the single source of truth** for what chains exist.
  A new chain is one variant plus rows in the metadata tables;
  `Chain::from_str_id`, `Chain::from_display_name` and `Chain::all` keep
  iteration exhaustive. Per-chain facts — address format, EVM membership,
  derivation paths, address slots, diagnostics shape — belong on `Chain`, not
  in a `match` inside whichever module needs them. Every time that rule was
  broken the copy went stale and the staleness became a bug.

- **A per-chain table should be able to say "these are all of them".** The
  reason the registry wins is not tidiness: fifty separate derive functions
  cannot assert that the set they cover equals the set that exists, and five
  separate JSON builders cannot either. One function over `Chain::all()` can,
  and the test that does it catches the chain nobody wired up.

- **Rust owns the domain; the shells render and forward.** No cross-platform UI
  framework — both UIs are written natively. How much of this is true today is
  tracked in `PLAN.md` rather than asserted here.

- **A library does not take the process.** `core` must not install a global
  logging subscriber on stdout, exit, or otherwise assume it is the program.
  It did both of the first two at one point; the CLI's `--json` output was
  corrupted by core's connection logs until it stopped.

---

## What this is NOT

- Not a cross-platform UI framework.
- Not a public library workspace.
- Not a multi-crate domain split.
