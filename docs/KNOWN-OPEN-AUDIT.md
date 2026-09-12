# Known-open audit — 2026-09-12 UTC

This audit closes the original repository coverage gaps, independent-vector item,
endpoint diagnosis bugs and EVM public-testnet broadcast proof. The user explicitly
retained unavailable external providers as dependencies. The separately added
visible Build/Sign/Broadcast workflow remains planned work.

## Closed

- App-only rule audit: [coverage map](APP-BOUNDARY-COVERAGE.md), 15 focused
  boundary tests plus existing service/protocol/CLI suites. Fixed Bitcoin
  testnet dispatch, invalid derivation indices, invalid movement alerts and
  ambiguous portfolio signatures.
- Funded Sepolia broadcast: the user funded and authorized a 0.000001 Sepolia
  ETH self-transfer. `spectra send broadcast --sign-only` built and signed it;
  the unchanged payload was submitted with `eth_sendRawTransaction`. The node
  accepted it and returned a successful mined receipt. Actual gas: 21,000;
  actual fee: 0.000022431561432 Sepolia ETH (below the approved maximum).
  [Public evidence](SEPOLIA-BROADCAST-PROOF.json),
  [transaction](https://sepolia.etherscan.io/tx/0x7c96dddb0ae86e2ee91dab7b0aeb0de7a4b6acaaf9848c07c7666a9bf7475555).
  This proves the original EVM node acceptance/mining gap, not every chain or
  the new UI workflow. No real ETH or existing user wallet was used.
- Base keyless history: three valid public Blockscout account-history reads and
  a successful Spectra CLI `history` query. The earlier Python-client 403 did
  not establish unavailability for Spectra's HTTP client; classification is
  corrected to `Open`.

- Decred mnemonic + BIP39 passphrase + `m/44'/42'/0'/0/0` reproduces the
  published Trust Wallet address. The reference's ExtendedKeys test supplies
  the mnemonic → dpub step; DerivePubkeyFromDpub supplies the same dpub → address.
  [Pinned reference](https://github.com/trustwallet/wallet-core/blob/cd5a27481d2181e63362cb57e2b2160506cce163/tests/chains/Decred/TWDecredTests.cpp).
- Kaspa's published gen1 mnemonic, account 0, receive index 1 reproduces its
  testnet address at `m/44'/111111'/0'/0/1`.
  [Pinned reference](https://github.com/kaspanet/rusty-kaspa/blob/15cee1a4cbcdd7abbaf005cbf77d29c5ad0354eb/wallet/keys/src/derivation/gen1/hd.rs).
- ICP health uses POST `{"metadata":{}}` and requires a nonempty Rosetta network
  list. The chain registry owns the protocol choice. Transport continues through
  the shared HTTP client and its privacy policy.
- Explicit HTTP probes are no longer discarded when equal to the endpoint URL.
  This restores the Bitcoin SV `/chain/info` row's actual probe.
- Diagnostics retain HTTP method/status, including access denials. Tests cover
  POST/body, empty network lists and HTTP 403. They run in CLI acceptance too.

## Live observations

These are observations from this machine, not promises of provider uptime.
The fixed CLI used an isolated temporary data directory and made read-only
requests. The endpoint/history audit used no user wallet; the separately
approved broadcast used a newly generated, isolated Sepolia test wallet.

| Provider/check | Result | Consequence |
|---|---|---|
| ICP Rosetta `/network/list` | POST 200, network list present | Probe defect closed |
| Bitcoin SV WhatsOnChain `/chain/info` | GET 200, including equal-URL row | Probe selection defect closed |
| Bitcoin SV Blockchair | GET 404 on catalog health probe | Redundancy remains unresolved |
| Zcash Trezor `zec1` | GET 403 | Availability remains unresolved |
| Zcash Trezor `zec2` candidate | GET 403 | Not added as a replacement |
| Base Blockscout public account history | Python client 403; three standard HTTP queries and Spectra CLI succeed | Restored keyless source |

Shared provider health does not establish broadcast, balance or historical
query correctness. In particular WhatsOnChain broadcast rows probe chain info;
the endpoint sweep submitted no transaction. The separate approved Sepolia
transaction above is the public broadcast proof.

## External dependencies retained by user (2026-09-12)

- Zcash's configured Trezor Blockbook API returns 403. NOWNodes documents a
  supported Zcash Blockbook service requiring an `api-key` header; no credential
  was supplied and no account or subscription was created.
  [Provider documentation](https://docs.nownodes.io/zec/mainnet/blockbook-0-6-0/blockbook-0-6-0/).
- Bitcoin SV's Blockchair health probe returns 404; WhatsOnChain works. Additional
  supported independent providers and single-node redundancy remain external
  infrastructure work. More URLs on the same provider do not prove redundancy.
- BNB Chain, Sonic, opBNB, Sei, Linea and Hyperliquid still require Etherscan V2
  credentials in the registry. Cronos and X Layer still have no configured
  supported history source. The current Routescan supported-chain list did not
  supply replacements for these chains.
  [Routescan network API](https://routescan.io/docs/api/network),
  [Base's documented Blockscout API](https://github.com/blockscout/docs/blob/main/base-api.mdx).
- The visible Build/Sign/Broadcast item added to PLAN during this audit is a
  separate implementation project; it has not been marked complete here.

## Test-wallet material

The newly created test wallet is encrypted and retained in the isolated local
folder recorded during this task; no seed/password is included in repository
artifacts. It is not the user's default wallet store. Its local directory is
`/var/folders/_8/796z9mwn40xcw5dy7tt2sydm0000gn/T/spectra-sepolia-proof-uh9a6gy8`.
This is a temporary-system folder; the public proof JSON is the durable evidence.

## Verification

- `cargo test --workspace`: 820 core tests passed.
- `./scripts/cli-acceptance.sh`: 332 checks plus Stage 3/follow-up fixtures passed, including 15 app-boundary regressions.
- iPhone 17 Pro `xcodebuild test`: 81 tests passed, zero failures.
- Live CLI: `spectra endpoints --chain icp`, `--chain 'Bitcoin SV'`, `--chain zcash`.
- FFI: 179 callables, zero unreachable candidates; no exported shape changes.
