# Core test audit — 2026-09-21

This pass focuses on `core/src/store/tests/` and the related registry, wallet
model, state and send tests. It is not a line-by-line audit of every core test.
The criterion is an independently useful failure, not test size or test count.
Production behavior is unchanged.

## Removed or consolidated

| Removed coverage | Reason | Retained coverage |
|---|---|---|
| `evm_native_symbols` (2 tests) | Six names copied from an old Swift switch are an arbitrary historical snapshot; the all-EVM nonempty check repeats metadata coverage. | `send::tests::every_evm_chain_names_its_native_asset` exercises the history decoder for every EVM network; it now also checks a nonempty symbol. Catalog facts remain owned by the registry/token catalog. |
| `seed_derivation_chain_coverage::every_chain_the_switch_named_still_resolves` | Copies a historical caller list and only checks `Some`; the current helper returns `Some` for every `Chain`, so it cannot distinguish a correct derivation mapping. | Catalog-name resolution and actual derived-address/import tests remain. The separate concrete-testnet metadata assertion is retained in `registry::tests`. |
| `settings_forward_compatibility::state_written_before_token_preferences_existed_still_loads` | Requires an obsolete whole-state JSON fixture to load, outside the prelaunch contract. Its `fiatCurrencyCode` field is not even the current `fiatCurrency` field. | Current database reopen/corruption tests remain. The missing-setting/default test was first relocated beside `AppSettings`; the concurrent strict-storage change subsequently replaced it with `stored_settings_require_every_current_field`. That production policy change is separate from this audit. |
| `wallet_view_model::the_wallets_own_network_survives_the_round_trip` | Only checks one copied field; it does not actually perform a round trip. | The full two-way equality test remains; the forward-view test now explicitly checks `chain_id`. |
| `wallet_view_model::holding_ids_are_stable_across_rebuilds` | Repeats the same fixture through a converter and the one-line `holding_identity` delegate. | `network_token_identity::symbols_and_market_ids_never_identify_a_holding` exercises native/token and cross-network deployment distinctions directly. |
| `tracked_tokens_persist::a_tracked_token_survives_a_reopen` | Duplicates the accepted-token reopen path in the precision-boundary test. | `an_impossible_precision_is_refused_rather_than_clamped` now checks a nonempty accepted result, its exact maximum precision, and equality of the complete token-preference payload after reopening. |
| `resident_state_round_trip::a_price_alert_survives_a_reopen` | Duplicates alert persistence in the multi-collection test. | `alerts_contacts_and_currency_survive_reopening` now compares full nonempty alert/contact payloads, not just counts. Its name no longer claims to cover every collection. |
| `registry::tests::testnet_counts_match_total` | Adds the sizes of complementary filters over the same iterator; it has no independent oracle for correct network classification. | Explicit EVM identities, network/counterpart checks and concrete-network import/signing coverage remain. |

Also removed the duplicate token-hosting enum round-trip loop from
`built_in_tokens::the_catalog_chain_ids_all_resolve`; the identical loop remains
in `store::wallet_domain::token_hosting_chain_tests` with an additional registry
resolution assertion. The catalog-deployment validation in the first test stays.

Net result of this audit: 9 fewer tests and 3 fewer test files. Two useful tests
were initially relocated; the settings check was subsequently superseded by
the concurrent strict-storage change described above. This audit disables no
production function or test feature.

## Deliberately retained

- Protocol fee units, signing capability and transaction merge strategy tables:
  wrong constants can change affordability or transaction identity. Their test
  names now describe those contracts rather than old Swift implementations.
- Address/network rejection, watch-only/signing eligibility and exact token
  precision checks: short assertions can still protect a financial boundary.
- Database reopen, partial/corrupt data, failed-write rollback, no-op event and
  late/concurrent update tests: different layers exercise different failure
  modes, even when fixtures look similar.
- Keypool reservation/concurrency and secret import rollback tests: removing
  these to reduce file count would remove substantive coverage.
- Serialization field-name tests: they check a real Rust/Swift data boundary.

## Verification

`make verify` passed: formatting, Clippy, 838 core unit tests plus the transport
integration test, 441 offline CLI checks, and 106 iPhone 17 Pro simulator tests.
After concurrent staking edits, `make lint test` was rerun successfully on the
sources at that rerun; the iOS build also regenerated and tested their Swift
bindings.
The workspace also contains other ongoing changes, so the suite total is not
the baseline for this audit's nine deletions.

After those successful runs, concurrent edits changed the CLI acceptance script,
stored-settings decoding, password verifier and SQLite persistence files. The
838/441/106 results describe the verified snapshot, not those later storage
changes; those changes need their own verification. They were not reverted or
folded into this test-pruning task.
