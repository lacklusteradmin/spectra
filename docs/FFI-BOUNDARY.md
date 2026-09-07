# FFI boundary

Integration rules for **UniFFI 0.31 + Swift 6**. See
[architecture](ARCHITECTURE.md) for ownership and [PLAN.md](../PLAN.md) for the
work to reduce the boundary.

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
- **Timestamps:** `CoreTransactionRecord.created_at_unix` and indexed
  `HistoryRecord.created_at` use Unix seconds. The persisted transaction
  payload uses Swift reference seconds (2001-01-01 UTC). The conversion lives
  in `core/src/fetch/transactions.rs`; both representations are `f64`, so the
  compiler cannot catch an epoch mix-up.
- **Secrets:** core owns layout and encryption; `SecretStore` supplies Keychain,
  file or in-memory storage. A new secret operation must work through that
  abstraction so the CLI can exercise it.
- **Serialization:** serde controls stored JSON independently of Swift naming.
  Field, variant and numeric-meaning changes require checking all producers and
  consumers. Spectra is prelaunch: change stored shapes directly; do not add
  migration shims merely to read an older checkout's data.

## Callers and coverage

Search both Rust names and generated camelCase names before deleting an export.
`SecretStore` and `BalanceObserver` are foreign callback protocols: their Swift
implementations need no Swift caller. A dead Swift wrapper does not prove that
the export behind it is unused.

Use `scripts/unreachable-exports.sh` to find candidates and
`scripts/count-exports.sh` to measure the callable surface. Cross-check generated
bindings when macros are involved; exclude `FfiConverter*` helpers from API
counts. Counting attribute occurrences or all generated `public func` lines
measures neither the same set nor the same thing.

An export used only by the app may have no Rust coverage. Add a CLI entry point
for domain rules: `spectra send assemble` exercises EVM assembly without keys
or network. Offline assembly does not prove broadcasting works; the remaining
coverage gaps are listed in [PLAN.md](../PLAN.md#known-open-items).

## Regenerating bindings

```sh
./scripts/bindgen-ios.sh
```

Change the Rust API, regenerate, then build Swift. Never edit `swift/generated/`
by hand. The Xcode “Build Rust Derivation Core” phase calls this script, which
also patches `vtablePtr` statics with `nonisolated(unsafe)` for Swift 6.
Change the generator patch there rather than duplicating it in Xcode.

## Common errors

| Symptom | Check |
|---|---|
| `Cannot find type 'CoreFoo' in scope` | Regenerate bindings and confirm the declaration is exported |
| `does not conform to protocol 'Codable'` | UniFFI does not generate `Codable`; add the appropriate Swift conformance |
| `unknown chain_id` | Resolve through the registry and check the supplied id |
| A key path into a subscript will not compile | Avoid `dict[key, default:]` in key paths; provide a suitable subscript |
