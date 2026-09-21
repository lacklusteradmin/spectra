# Cross-chain duplication audit — 2026-09-21

This pass compared function bodies with names normalized, then checked callers
and protocol differences manually. A text match is a candidate, not proof that
whole chain implementations can be merged.

## Consolidated

| Former copies | Shared owner | Boundary retained |
|---|---|---|
| Decred and Kaspa BIP-32 path parsers | `derivation/primitives.rs::parse_bip32_path` | Chain-specific address hashing and encoding |
| BCH, BSV, DOGE, LTC and Dash P2PKH address helpers, plus BTG's inline encoding | `derivation/bitcoin.rs::encode_p2pkh` | Existing network version bytes; Decred's different checksum is excluded |
| BCH, BSV, DOGE, LTC, Dash and BTG mnemonic P2PKH bodies | `derivation/bitcoin.rs::derive_legacy_p2pkh` | Chain entry points and network parameters |
| DOGE, LTC, Dash, BTG, Zcash and Decred P2PKH locking scripts | `send/bitcoin_wire.rs::p2pkh_script` | Address decoding and chain-specific sighashes |
| Polkadot and Bittensor compact SCALE, 32-byte hash decoding and Blake2b-256 helpers | `send/substrate.rs` | Call indexes, destination validation, era and tip handling |

Removed `SubstrateSignError` and its Display/Error/From implementations: no
production or test caller constructs the type. Its comments claimed a typed
error flow that the actual signers never used.

Removed three chain-local script-shape tests with the deleted functions.
`send::bitcoin_wire::tests::a_p2pkh_script_is_twenty_five_bytes` covers the shared
script, while existing independent derivation vectors and transaction tests
exercise the chain callers. No FFI exports or Swift implementations changed.

## Remaining candidates

- Polkadot/Bittensor RPC nonce, runtime-version and hash queries duplicate one
  another. Their signing envelopes also overlap, but era/tip and destination
  validation differ; merging complete signers requires reviewing those rules.
- DOGE/LTC/BCH private-key import bodies repeat length checking, key decoding
  and result construction. They now share the address encoder.
- SOL/SUI/TON balance formatters implement the same nine-decimal input and
  six-decimal display truncation. Consolidating them should account for the
  existing generic amount-formatting policy rather than add another formatter.
- DOGE/LTC legacy address decoders have the same permissive body (minimum
  payload length and no version check). Do not promote that body into a shared
  validator: network/version and exact length requirements need explicit review.

## Dead-code checks and limits

Both `scripts/unreachable-exports.sh` and `scripts/uncalled-core-fns.sh` passed
before the edit. They are name-based scans, not a typed reachability proof:
matching names can hide unused overloads, and the latter does not inspect dead
types such as `SubstrateSignError`. It also includes Swift test sources in its
caller corpus despite its comment saying tests do not count.

A manual scan of Swift functions with no same-name app reference found callback
and protocol implementations (balance observer, secure store, UIKit bridge),
not functions safe to delete. Generated bindings and callbacks must remain
roots of any future typed reachability analysis.

No project implementation named `parallel slicing` was found. The requested
simplification is shared protocol primitives, not removal of network concurrency.
