# App-only domain coverage audit

The original known-open item tracked rules reachable only from the app without
meaningful Rust/CLI verification. This audit checks domain families, their
callers and their failure rules. It is not a claim of line coverage or that every
FFI wrapper needs its own duplicate test. The newly planned visible
Build/Sign/Broadcast flow is separate work.

## Coverage decisions

| Domain | Evidence and decision |
|---|---|
| Import, secrets, wallet edits, state projections | `store/tests/wallet_import.rs`, `wallet_derived_state.rs`, `wallet_view_model.rs`, `snapshot.rs`; CLI wallet lifecycle and field intents. SecretStore remains a platform callback; core refusal/cleanup/reopen rules run without Keychain. |
| Quotes, maintenance, history cursors and pending status | `service/network_prices.rs`, `maintenance.rs`, `history_cursor.rs`, `history_refresh/tests.rs`; CLI price, diagnostics maintenance, tx polling. See the six-slice audit for the owning service operations. |
| EVM preview and destination checks | `service/send/tests.rs` exercises missing RPC fields, token metadata, exact amount conversion, selected wallet network, ENS changes and failed reads. CLI `send preview`, `send probe`, `send destination`. |
| Tron preview | Existing token scaling and failed-read fixtures now invoke `fetch_tron_send_preview_typed`, so both the protocol read and the app-facing projection are checked. |
| Bitcoin single-address preview/status | New `service/app_boundary_tests.rs`: Testnet4 endpoints, dust exclusion, exact fee and confirmed status. Found and fixed a mainnet-only dispatch guard. |
| Bitcoin HD derivation/receive/preview | New boundary fixture derives the account xpub, skips spent-but-empty addresses, respects the scan window, reads the fee target and refuses a non-Bitcoin network. Existing derivation vectors remain the cryptographic oracle. |
| Dogecoin and shared simple-chain previews | New boundary fixtures check spent-output exclusion/requested amount, Solana fee subtraction, unread balance errors and unsupported chain refusal. Other chain encoders/decoders retain protocol-specific tests. |
| EVM replacement nonce | New boundary fixture verifies the transaction hash RPC request, exact nonce, missing transaction refusal and malformed nonce refusal. No invented zero nonce. |
| Pure display/editor/notification rules | New root/service `app_boundary_tests.rs`: dust visibility, decimal overrides, fiat rules, token lookup, hardened derivation round-trip and invalid index, reset scope isolation, RPC error classification, key hex input, movement thresholds, collision-free portfolio signature, testnet pricing and foreign-network refusal. |
| Diagnostic export | New boundary fixture checks redaction through the exported JSON writer and strict required fields through the reader; registry, sanitizer and diagnostic-state tests cover owned state. |
| Seed envelope | New exported-boundary test checks wrong-key authentication, tampered nonce and invalid key length. Existing AES tests remain. |
| Catalog, address/amount validation, receive selection, staking and transport | Existing `registry`, `tokens`, `validation`, `receive`, `staking` and `tor` module tests exercise their rules; CLI chain/catalog, address, send amount, receive and staking operations drive the domain. OS transport startup and foreign callback installation remain platform integration concerns, covered separately rather than counted as offline network success. |

## Gate

`cargo test -p spectra_core --lib app_boundary_tests` runs 15 focused tests.
`./scripts/cli-acceptance.sh` invokes this filter alongside the earlier service
fixtures. The Rust workspace runs the broader module tests. The iOS suite
checks generated binding integration. Export reference counting is only a
reachability check; it is not used as proof of execution coverage.

## Behaviours corrected by this audit

- Bitcoin preview/status dispatch accepts Bitcoin testnets and uses their
  selected endpoint list.
- Derivation path segment indices must be below `2^31`; the hardened bit is
  represented separately.
- Negative/non-finite movement observations or thresholds cannot trigger an
  alert.
- Portfolio signatures serialize sorted holding keys with unambiguous string
  boundaries, so delimiter-containing keys cannot hide a composition change.
