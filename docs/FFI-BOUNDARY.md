# FFI boundary

How to write code that crosses between `core` (Rust) and the shells
(Swift today, Kotlin later), for **UniFFI 0.31 + Swift 6**. Examples written
for older UniFFI releases usually do not apply.

This file covers mechanics and traps. What *should* cross — and why the surface
should keep shrinking — is `PLAN.md`'s subject, not this one.

---

## What crosses, and how

**`#[derive(uniffi::Record)]`** — a plain struct of FFI-legal fields. Add
`#[serde(rename_all = "camelCase")]` when the same type is also serialized;
Swift sees camelCase either way, but the *stored* JSON follows serde.

**`#[derive(uniffi::Enum)]`** — a C-like or payload-carrying enum. Swift gets a
real Swift enum, so exhaustive `switch` works and stays exhaustive.

**`#[derive(uniffi::Object)]`** — a reference type behind `Arc`. Use it for
things with identity and interior state (`WalletService`,
`BalanceRefreshEngine`), not for data.

**`#[derive(uniffi::Error)]`** — an error enum. `SpectraBridgeError` is the one
that crosses; its variants exist so the UI can branch (`Network` → offline
banner, `InvalidInput` → inline field error) rather than string-match.

**Free `#[uniffi::export] pub fn`** — for anything stateless: validation,
decoding, registry lookups. If it observes no prior call and holds no cache, it
is a free function.

**`#[uniffi::export] impl` on an Object** — for anything touching shared state:
HTTP clients, endpoint lists, the secret store, the resident app state.

**A trait with `#[uniffi::export(with_foreign)]`** — a callback the shell
implements and Rust calls. `SecretStore` and `BalanceObserver` are the two.

### What cannot cross

Borrowed types, lifetimes, generics, trait objects other than exported
callbacks, closures, and anything with a custom `Drop` contract you rely on —
UniFFI 0.31 records cannot implement `Drop` cleanly, so a call path that
receives secret-bearing records must scrub its own strings before returning.

---

## Traps

**Two epochs that look identical.** Wire shapes that flow through
merge/refresh use **Unix epoch seconds**; shapes that flow through SQLite
persistence use **Swift reference time** (seconds since 2001-01-01 UTC). Both
are `f64`. Getting them backwards misorders history by 31 years and nothing
type-checks differently. The conversion now lives in
`core/src/fetch/transactions.rs` rather than on the boundary, which is where it
belongs — see `CoreTransactionRecord` vs `CorePersistedTransactionRecord`.

**Counting the exported surface from the source undercounts it.** A macro can
expand to a hundred `#[uniffi::export]`s while `grep -c` reports one. Count
`public func` in `swift/generated/spectra_core.swift`.

**A library must not take the process.** `core` installed a global tracing
subscriber writing to *stdout* in a `OnceLock` — no caller could opt out, and
it corrupted every `spectra --json` document. Logging goes to stderr, quiet
unless `RUST_LOG` says otherwise.

**An `async fn` block needs `async_runtime = "tokio"`, and nothing says so.**
`#[uniffi::export]` on an `impl` block containing `async fn`s compiles, links
and generates correct-looking Swift. At runtime UniFFI polls the future with no
reactor installed and every call fails with *"there is no reactor running, must
be called from the context of a Tokio 1.x runtime"*. `StakingService` shipped
that way and its whole tab was inert — and **neither gate could see it**: the
CLI drives core from inside its own runtime (`ctx.rt.block_on`), so the Rust
tests and `cli-acceptance.sh` both pass. If you export an `async fn`, the
attribute is not optional.

**An exported function's callers are not in the Rust tree.** Grep `core/` and
`cli/` for a `#[uniffi::export]` function and you can get zero hits while the
iOS app calls it on the funds path. Two consequences, and both have been paid.

*It is not dead.* Before concluding an export is unused, grep `swift/` for its
**camelCase** name — the generated bindings rename it, so the Rust spelling
finds nothing. Also check for `#[uniffi::export(with_foreign)]` traits: Swift
*implements* those and Rust calls them, so even the camelCase name appears only
as a protocol conformance.

*It may have no coverage at all.* Seventy-two exports currently have no caller
in `core/` or `cli/` outside tests, and about half of those are not reached by
a Rust test either — `core_ethereum_custom_fee_validation` parses and compares
two EIP-1559 fee fields on the funds path with nothing but the iOS app calling
it. Whatever is inside one of those can be wrong indefinitely with three green
suites.

The remedy is a CLI command, not a mock. `prepare_evm_send_assembly` was the
worst case here — sixteen EVM mainnets could not assemble at all and two
assembled the wrong asset, invisibly — until `spectra tx assemble` gave it a
caller the gates can run. It is a pure function over its arguments, so the
command needs no key, no network and no store.

**Spawned work is invisible to a short-lived caller.** `trigger_immediate` on
the refresh engine spawns and returns; a CLI process exits before the callbacks
arrive. Any fire-and-forget API needs an awaited sibling
(`refresh_now`) or it is app-only by accident.

**Secrets are borrowed, never owned.** The shell's keystore owns long-lived
secret material; Rust receives it for the duration of one call. `SecretStore`
keeps that uniform — iOS backs it with Keychain, `secret_backends.rs` provides
`InMemorySecretStore` for tests and `FileSecretStore` for the CLI. **Never add
a secret path only one platform can satisfy**: if the CLI cannot drive it, it
is a platform detail wearing a core API's clothes.

---

## Regenerating bindings

```bash
./scripts/bindgen-ios.sh
```

Rust change → regenerate → build Swift. Skipping the middle step is what
"Cannot find type `CoreFoo` in scope" means.

`swift/generated/` is **generated**: never edit it. Change the Rust API or the
generator patch in `scripts/bindgen-ios.sh` instead. That script applies the one
patch a plain UniFFI run does not: `nonisolated(unsafe)` on the `vtablePtr`
statics, which Swift 6 otherwise rejects.

The Xcode "Build Rust Derivation Core" phase **calls this script** rather than
repeating it. The two used to generate separately and patch *differently* — the
script wrote `nonisolated` onto 678 declarations and the next Xcode build
removed every one of them — so which version of `swift/generated/` was on disk
depended on which had run last. The blanket `nonisolated` was for a UniFFI
version this project no longer uses and is gone.

### Changes that break the boundary silently

Adding a field to a `Record` is source-compatible for readers and breaks every
Rust constructor — the compiler catches that. What it does **not** catch:

- **A stored struct gaining a field** makes previously-written JSON unreadable
  unless the field is `#[serde(default)]`. `settings_forward_compatibility`
  pins this.
- **Renaming a variant** changes its serialized string. Front ends match on
  those strings (`addressBookRejected` reasons, for instance).
- **Changing a numeric field's meaning** — the two epochs above.

---

## Common errors

| Symptom | Cause |
|---|---|
| `Cannot find type 'CoreFoo' in scope` | Bindings not regenerated. |
| Swift sees an empty struct | The Rust struct is missing `#[derive(uniffi::Record)]` and generated as an opaque object. |
| `does not conform to protocol 'Codable'` | UniFFI does not generate `Codable`. Add `extension CoreFoo: Codable {}`. |
| `unknown chain_id` at runtime | A stale chain-id constant on the Swift side, or a new registry chain with no Swift constant. |
| A key path into a subscript will not compile | Swift key paths cannot use `dict[key, default:]` — the index must be `Hashable`. Give the type a real subscript. |
