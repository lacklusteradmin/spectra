# Architecture decisions

Spectra is one application with a shared Rust core and native front ends.
[PLAN.md](../PLAN.md) tracks implementation status; [FFI-BOUNDARY.md](FFI-BOUNDARY.md)
covers binding mechanics.

## Workspace

| Directory | Responsibility |
|---|---|
| `core/` | Domain logic, persistence, network, crypto and UniFFI exports |
| `ffi/` | Binding crate; re-exports core |
| `cli/` | Argument parsing, terminal output and password prompting |
| `swift/` | Native iOS UI and platform services |
| `kotlin/` | Android shell, currently a skeleton |
| `tools/uniffi-bindgen/` | Binding generation binary |

Core stays one crate. A crate per domain would add Cargo overhead without an
external consumer that needs the isolation. There are no published library APIs
or semver commitments between these crates. Core builds and tests without Xcode
or an Android toolchain.

## Ownership

Rust owns wallets, settings, address rules, transaction history, persistence,
network policy and signing. Front ends send intents and render the results.
The CLI is the check that none of those operations depends on a phone.

Swift owns navigation, sheets, text being edited, focus, animation and rendering
caches. Keychain, biometrics, notifications, Live Activities and device signals
stay on the platform and supply inputs to core-owned policy. Losing view state
on restart may cost a redraw; losing domain state must not lose user data.

`CoreAppState` holds small resident collections. Unbounded history is a separate
queryable store so changing a setting does not clone every transaction.
`WalletService::open_state` binds the database; state commands persist before
returning and no-op commands emit no events or writes. UI projections have one
writer. Derived domain data is computed from core's store and adopted by the UI,
not computed by sending the projection back to core.

Core drives secret reads, writes and password encryption through `SecretStore`.
The platform implements storage; native file and in-memory backends let the CLI
and tests use the same domain operations.

## Chain rules

`registry::Chain` is the authority for chain identity and capabilities, backed
by `core/data/chains.toml`. Adding a chain requires a catalog row and an enum
variant in matching order. Registry tests check that relationship independently.
Address formats, derivation paths, address slots, EVM membership and routing
facts belong there rather than in caller-owned lists.

Chain-specific implementations live under each domain's `chains/` directory.
Keep differences that carry protocol meaning; share cryptographic primitives
and wrappers that differ only in a chain name. Tests over the registry should
assert complete capability coverage, rather than only test named examples.

## Boundary design

Exports live next to their implementations. Stateful operations are service
methods; genuinely stateless calculations may be free functions. Do not merge
distinct preview inputs into a wide command/response union just to lower the
export count: Bitcoin's xpub and gap bounds are different inputs from an EVM
nonce and fee configuration.

The native UIs share domain code, not a cross-platform UI framework. Core must
not exit the process or install a global logger; the executable owns logging
configuration and keeps stdout available for CLI JSON.

## Signing and service modules

`service/state.rs` owns the serialized state writer and app projections.
`keypool`, `address_discovery`, `transactions`, `wallet_import` and
`operational_events` hold cohesive operations on that state; all persistent
mutations still use the same writer rather than creating per-file owners.

The send service is split into `send_execution` (stored identity and exact
amount conversion), `send_destination` (fresh resolution and review binding),
`send_preview` (quotes), `send_signing` (protocol dispatch) and `send_broadcast`
(rebroadcast). Internal `send_params` are Rust records, not a second JSON API.
Secrets use redacted, zeroizing storage; an Ed25519 seed is a distinct type
rather than an unlabelled 32/64-byte array.

The Tron, Aptos and Sui protocols build their transaction bytes locally from
explicit transfer inputs and fetched chain metadata. Their prepared values
keep bytes private and expose offline signing; network submission accepts the
signed output. Solana's local builders likewise feed a broadcast-only stage.
Independent SDK fixtures test real mnemonic derivation through these signing
boundaries; mock-node tests exercise the stored-wallet execution route.


## Core module boundaries

- `service/network.rs` owns endpoint health and status probes. Its siblings
  `network_balance`, `network_tokens`, `network_history`, `network_hd` and
  `network_prices` own the corresponding reads and dispatch.
- `service/history_bitcoin.rs` selects wallet/network/HD scope and persists
  results. `fetch/bitcoin_history.rs` owns provider pagination and buffered
  block-cohort aggregation; a display limit never means provider exhaustion.
- `send/chains/bitcoin_wire.rs` contains only Bitcoin-format serialization.
  Other UTXO protocols must establish byte compatibility before reusing it.
  Kaspa owns its own hash preimage encoding. Solana compiles a unique account
  list across all instructions before signing.
- `store/wallet_db/` separates connection/schema, keypool, addresses, history,
  wallets, state and teardown. `state` and `teardown` keep their cross-table
  transactions; splitting files does not split commits. `store/tests/` groups
  regressions by domain.
