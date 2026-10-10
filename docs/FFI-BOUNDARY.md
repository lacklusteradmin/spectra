# FFI boundary

Integration rules for **UniFFI 0.31.2 + Swift 6**. See
[architecture](ARCHITECTURE.md) for ownership.

## Export shapes

| Rust declaration | Use |
|---|---|
| `#[derive(uniffi::Record)]` | Data with FFI-compatible fields; add serde naming explicitly if serialized |
| `#[derive(uniffi::Enum)]` | Plain or payload-carrying enums; Swift gets exhaustive switches |
| `#[derive(uniffi::Object)]` | Identity and interior state behind `Arc`, such as `WalletService` |
| `#[derive(uniffi::Error)]` | Structured errors; branch on variants rather than message strings |
| `#[uniffi::export] pub fn` | Stateless validation, decoding and lookups |
| `#[uniffi::export] impl` | Methods on an object; keep internal helpers in an unexported block |
| `#[uniffi::export(with_foreign)]` trait | Shell callbacks implemented by Swift and called by Rust |

Name an exported type for what it is, once, in Rust: the binding carries that
name to Swift and Kotlin, and front ends use it with no `typealias` or prefix.
`TransactionRecord`, not `CoreTransactionRecord` — where a type is defined is
not what it is.

`SpectraBridgeError::InvalidInput` and `Failure` carry a `LocalizableMessage`: an
English `template` with a `%@` for each of its `args`. The template is the key a
front end looks up in its string tables, so a sentence a person reads names its
values as `args` (`SpectraBridgeError::refused`, `failed`,
`DerivationError::refused`) rather than interpolating them with `format!`.
`scripts/unused-strings.sh` fails on a templated sentence the tables do not
translate. Transport, parser and library text arrives with no args and reads
as sent.

Borrowed types, lifetimes, generics, closures and unexported trait objects do
not cross. Do not rely on a custom `Drop` implementation on a secret-bearing
record: scrub its owned strings on the receiving call path.

## Runtime and storage traps

- **Async exports:** use `#[uniffi::export(async_runtime = "tokio")]` for
  exported async operations using Tokio. Rust and CLI tests run inside their
  own runtime and cannot catch a missing reactor when Swift calls the binding.
  Exercise the affected path in the app as well.
- **Short-lived callers:** spawned work can outlive a CLI process. Provide an
  awaited operation, as `refresh_now` does alongside `trigger_immediate`.
- **Timestamps:** transaction payload/FFI `created_at_unix` and the indexed
  `HistoryRecord.created_at` all use Unix seconds, including fractional seconds.
  Swift renders with `Date(timeIntervalSince1970:)`; there is no epoch conversion.
- **Versions:** `ResidentState.revision` orders successful state publications within
  one service session. Failed and no-op commands do not advance it; it is not
  persisted. Snapshot sequence numbers order projections; their contained state
  revision must also pass the front end's committed-state check.
- **Secrets:** core owns layout and encryption; `SecretStore` supplies Keychain,
  file or in-memory storage. A new secret operation must work through that
  abstraction so the CLI can exercise it.
- **Serialization:** serde controls stored JSON independently of Swift naming.
  Field, variant and numeric-meaning changes require checking all producers and
  consumers. Spectra is prelaunch: change stored shapes directly; do not add
  migration shims merely to read an older checkout's data.

## Callers and coverage

Search both Rust names and generated camelCase names before deleting an export.
`SecretStore` and `RefreshObserver` are foreign callback protocols: their Swift
implementations need no Swift caller. A dead Swift wrapper does not prove that
the export behind it is unused.

Use `scripts/unreachable-exports.sh` to find candidates. Cross-check generated
bindings when macros are involved.

`scripts/uncalled-core-fns.sh` checks the layer below. A `pub fn` that no
longer carries `#[uniffi::export]` is invisible to both gates rustc offers —
`dead_code` treats it as API because the crate is a library, and the bindings
never mentioned it — so it can lose its last caller and keep compiling.
`derive_bitcoin_account_xpub_typed` did, for long enough that its doc comment
still said it was exported.

`scripts/unwritten-record-fields.sh` checks the fields of every
`uniffi::Record`: one that production code never sets to anything but `None`,
`0` or empty still crosses, renders and reads as data. rustc cannot see it on
a `pub` struct and a name match cannot tell which struct `x.field` writes, so
the scan reads the MIR rustc emits, where every place carries its type.
`scripts/record_writers.py` says what counts as a write. All four scripts run
in `make lint` and in CI.

An export used only by the app still needs its rule covered: test that rule in
core with `cargo test`. Offline tests do not prove broadcasting works; the
remaining coverage gaps are listed in [OPEN-ITEMS.md](OPEN-ITEMS.md).

## Regenerating bindings

```sh
make ios
```

Change the Rust API, then build: in Xcode, or with `make ios`. Never edit
`swift/generated/` by hand. The Xcode “Build Rust Core” phase runs
`scripts/build-ios.sh`, which builds the iOS library and generates the bindings
from that same library. It is the only writer of the generated sources and
post-processes nothing, so a Swift 6 problem in them is a UniFFI version or
API-shape problem, not something to patch downstream. 0.31.2 is the floor
because it is the release that emits `nonisolated(unsafe)` on the
callback-interface `vtablePtr` statics itself.

## Common errors

| Symptom | Check |
|---|---|
| `Cannot find type 'Foo' in scope` | Regenerate bindings and confirm the declaration is exported |
| `does not conform to protocol 'Codable'` | UniFFI does not generate `Codable`; add the appropriate Swift conformance |
| `Unknown network: …` | A stored row or TOML entry names an id the catalog does not have; chains cross the FFI as `Chain`, so the text came from storage or a file |
| A key path into a subscript will not compile | Avoid `dict[key, default:]` in key paths; provide a suitable subscript |
