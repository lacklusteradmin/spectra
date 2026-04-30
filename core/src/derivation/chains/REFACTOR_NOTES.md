# Vertical-slicing refactor — done

Each chain file in this directory is fully self-contained: its own BIP-39,
its own curve walk (BIP-32 / SLIP-10 / BIP-32-Ed25519 / substrate-bip39 /
TON mnemonic / direct seed), and its own address encoder. No shared
`primitives/` or `curves/` subdirectory.

User explicitly approved code duplication for clarity. Don't try to dedupe
across chains.

## Layout

```
derivation/
├── engine.rs                # FFI surface, request types, dispatch table,
│                            # validation. Inlined helpers: bip39 language
│                            # resolution, base32_no_pad, base58check_encode,
│                            # display_error, normalize_seed_phrase,
│                            # script_type_from_purpose, format_secp_public_key.
├── bitcoin_primitives.rs    # Bitcoin-specific helpers (BIP-32 secp256k1,
│                            # xpub/xprv, hash160, P2PKH/P2SH/P2WPKH/P2TR
│                            # encoders) — used by validation.rs + xpub_walker.rs.
├── tests.rs / presets.rs / validation.rs / import.rs / xpub_walker.rs
└── chains/
    ├── REFACTOR_NOTES.md    # this file
    ├── bitcoin.rs           # full BIP-32 + bech32 + Taproot
    ├── bitcoin_cash.rs      # 0x00/0x6f legacy P2PKH
    ├── bitcoin_sv.rs        # 0x00/0x6f
    ├── bitcoin_gold.rs      # 0x26 (no testnet)
    ├── litecoin.rs          # 0x30/0x6f
    ├── dogecoin.rs          # 0x1e/0x71
    ├── dash.rs              # 0x4C/0x8C
    ├── zcash.rs             # 2-byte prefix 0x1CB8 / 0x1D25
    ├── decred.rs            # BLAKE-256 + custom base58check
    ├── kaspa.rs             # CashAddr-bech32, x-only pubkey
    ├── evm.rs               # Ethereum, EthClassic, Arbitrum, Optimism,
    │                        # Avalanche, Hyperliquid (+ all EVM testnets)
    ├── tron.rs              # secp + keccak[12..] + 0x41 prefix + base58check
    ├── xrp.rs               # secp + hash160 + 0x00 + base58check (RIPPLE alphabet)
    ├── solana.rs            # SLIP-10 ed25519 + bs58 pubkey
    ├── stellar.rs           # SLIP-10 ed25519 + strkey base32
    ├── sui.rs               # SLIP-10 ed25519 + Keccak (matches engine.rs);
    │                        # `pubkey_to_sui_address` keeps Blake2b for watch-only
    ├── aptos.rs             # SLIP-10 ed25519 + Keccak(pubkey || 0x00)
    ├── near.rs              # direct seed (no SLIP-10 walk) + hex
    ├── icp.rs               # SLIP-10 ed25519 + sha256(sha256(pubkey || "icp"))
    ├── ton.rs               # TonMnemonic / DirectSeed / SLIP-10 dispatch +
    │                        # v4R2 BOC + base64url
    ├── cardano.rs           # BIP-32-Ed25519 Icarus + Shelley enterprise bech32
    ├── monero.rs            # BIP-39 + spend-seed reduction + chunked base58
    ├── polkadot.rs          # substrate-bip39 + sr25519 + SS58 (prefix 0 / 42)
    └── bittensor.rs         # substrate-bip39 + sr25519 + SS58 (prefix 42)
```

## Testnets

Testnets share their mainnet's chain file. The dispatch table routes via
`Chain::mainnet_counterpart()`; the chain file branches on `request.chain`
to pick version bytes / HRP / SS58 prefix. Examples:

- `Chain::PolkadotWestend` → `polkadot::derive` (prefix becomes 42).
- `Chain::CardanoPreprod` → `cardano::derive` (network byte 0, HRP `addr_test`).
- `Chain::DogecoinTestnet` → `dogecoin::derive` (version byte 0x71).

## Tests

`cargo test --lib -p spectra_core` → 283 passed, 0 failed.

## What was deleted

- `derivation/primitives/{bip39,encoding,hmac,path,slip10}.rs` — every
  chain inlined its own copies; the small remainders moved into engine.rs.
- `derivation/primitives/bip32.rs` — moved to `derivation/bitcoin_primitives.rs`
  (it's Bitcoin-specific, not a generic primitive).
- `derivation/curves/{secp256k1,sr25519}.rs` — chains absorbed their own
  curve walks; `format_secp_public_key` and `encode_ss58` moved into
  engine.rs / chains/polkadot.rs respectively.
- ~21 dead `derive_<chain>(request)` functions from engine.rs (~510 lines).
- ~10 dead helper functions (`derive_secp_material`,
  `derive_ed25519_material`, `derive_substrate_sr25519_material`,
  `derive_cardano_icarus_material`, `derive_monero_keys_from_request`,
  `derive_bip39_seed_from_request`, `secp_derivation_path`,
  `ed25519_derivation_path`, `requests_output`, `derive_bitcoin_address`).

## Open follow-ups

- **Sui address algorithm inconsistency.** The seed-phrase derive path in
  `chains/sui.rs::derive` uses Keccak-256 (preserving the previous
  engine.rs behavior, which is what the tests pin). The watch-only
  `pubkey_to_sui_address` encoder in the same file uses Blake2b-256, which
  is Sui's published address algorithm. Reconciling the two — likely by
  switching the seed-phrase path to Blake2b and re-pinning test vectors —
  is tracked separately.

- **`derive_from_private_key`.** Lives in engine.rs and dispatches on
  chain to a shared address-encoder (`derive_address_from_keys` /
  `derive_ed25519_chain_address`). Could push into chain files for full
  symmetry with the seed-phrase path, but it's a smaller surface and the
  shared dispatch is currently ergonomic.
