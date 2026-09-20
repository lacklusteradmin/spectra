# Naming

Use domain meaning rather than the layer that originally implemented a rule.

- `Chain` identifies one concrete mainnet or testnet. Its registry string is
  `chain_id` (`chainId` in Swift/JSON); display text is `chain_name`.
  A family's selected chain is `selected_chain_by_family`. A wallet's `chain()`
  resolves its stored identity without consulting application settings.
- `token_id` identifies an asset; `deployment_id` identifies that asset on a
  concrete chain. `TokenDeploymentEntry` contains both and stores a `chain_id`,
  including for user-added tokens. Resolve display names through the registry.
- Spell the provider key `coingecko_id` / `coingeckoId` everywhere.
- Free functions use descriptive verbs/nouns without `core_` or `app_core_`.
  Modules convey ownership. FFI exports use the same naming as other APIs.
  Internal borrowed parsing helpers may state their input (`*_str`, `*_segments`).
- Structured results use the ordinary function name. A separate raw JSON API
  ends in `_json`; `_typed` is not a substitute for naming the difference.
- Endpoint kinds use `ENDPOINT_KIND_*`; abilities use `ENDPOINT_CAPABILITY_*`.
  A combined selection uses a filter mask. Core defines the bits; callers import
  them. Do not accept historical synonyms for catalog kind names.
- Handwritten Swift follows UniFFI's `Id` / `Ids` spelling for identifiers.
  Keep platform names such as `UUID` and protocol terms unchanged.
- Name Swift files after their primary type, with the existing domain-extension
  conventions for `AppState` and `Store`. Prefer canonical module paths over
  aliases that hide the module's purpose.

Protocol-specific network identifiers and JSON-RPC field names retain their
protocol-defined meaning and spelling. A naming cleanup must not merge distinct
identities or change wire protocols.
