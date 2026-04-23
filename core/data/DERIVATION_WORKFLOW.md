# Derivation Variables — Complete Inventory

Deterministic pipeline — same inputs always produce the same `(priv, pub, address)`. Grouped by stage:

## Stage 1 — Mnemonic → 64-byte seed (BIP-39 PBKDF2)
1. `seed_phrase` (mnemonic string)
2. `mnemonic_wordlist` (language — affects checksum validation; same entropy in two wordlists = different seeds)
3. `passphrase`
4. `salt_prefix` (default `"mnemonic"`)
5. `iteration_count` (default 2048)
6. *Fixed today:* PBKDF2 hash = SHA-512, output length = 64 bytes, Unicode normalization = NFKD

## Stage 2 — Seed → master key
7. `curve` (secp256k1, secp256r1, ed25519, sr25519, …)
8. `derivation_algorithm` (BIP-32, SLIP-0010, Substrate, EIP-2333, …)
9. `hmac_key` (master HMAC constant — `"Bitcoin seed"` / `"ed25519 seed"` / `"Nist256p1 seed"` / custom)
10. *Fixed today:* master HMAC = HMAC-SHA-512

## Stage 3 — Master → child (path walk)
11. `derivation_path` — the segments
12. Per-segment hardened flag (encoded in the path via `'`/`h`)
13. *Implicit:* CKDpriv function per (curve, algorithm) — retry rules, non-hardened support

## Stage 4 — Private key → public key
14. Curve scalar-multiply (determined by `curve`)
15. `public_key_format` (compressed / uncompressed / x-only / raw)

## Stage 5 — Public key → address
16. `chain` (often picks defaults for 17–22 below)
17. `network` (mainnet/testnet/signet/testnet4 → version bytes, HRP)
18. `address_algorithm` (Bitcoin script, EVM Keccak, Solana Base58, StrKey, …)
19. `script_type` (P2PKH / P2SH-P2WPKH / P2WPKH / P2TR / Account)
20. *Fixed per algorithm today:* address hash (Hash160 / Keccak-256 / Blake2b), encoding (Base58Check / Bech32 / Bech32m / Base32), checksum, version prefix

## Currently exposed as explicit request fields
`seed_phrase`, `passphrase`, `mnemonic_wordlist`, `salt_prefix`, `iteration_count`, `hmac_key`, `curve`, `derivation_algorithm`, `derivation_path`, `chain`, `network`, `public_key_format`, `address_algorithm`, `script_type`.

## Hardcoded but *could* be promoted to parameters
PBKDF2 hash function, PBKDF2 output length, mnemonic normalization form, master HMAC function, address hash function, address encoding, version/prefix bytes. Moving any of these to request fields would still keep the function deterministic — they're just currently pinned to the BIP-39 / chain specs.

**Total knobs = 14 exposed + ~7 latent spec constants.** The function is fully deterministic over that set.

---

# Seed Phrase → Public/Private Key Workflow

End-to-end description of how `spectra_core` turns a BIP-39 mnemonic into a usable private key, public key, and chain-specific address. Every customization point the caller can override is called out explicitly.

All implementation references point into [core/src/derivation/runtime.rs](src/derivation/runtime.rs).

---

## 0. Request entrypoint

Swift (or any UniFFI consumer) crosses the FFI boundary through one of four JSON functions:

| Entrypoint | Purpose | Line |
|---|---|---|
| `derivation_derive_json` | Derive a single address/public/private key for one chain. | [runtime.rs:278](src/derivation/runtime.rs#L278) |
| `derivation_build_material_json` | Derive the private key bytes + address for a specific path (send pipeline, imports). | [runtime.rs:298](src/derivation/runtime.rs#L298) |
| `derivation_derive_all_addresses_json` | Fan out across every registered chain for a given seed. | [runtime.rs:328](src/derivation/runtime.rs#L328) |
| `derivation_derive_from_private_key_json` / `…_material_from_private_key_json` | Bypass stages 1–2; start from raw private-key hex. | [runtime.rs:287](src/derivation/runtime.rs#L287), [runtime.rs:309](src/derivation/runtime.rs#L309) |

Each JSON request is deserialized into `UniFFIDerivationRequest` ([runtime.rs:85](src/derivation/runtime.rs#L85)) or `UniFFIMaterialRequest` ([runtime.rs:125](src/derivation/runtime.rs#L125)), then converted into the internal `ParsedRequest` ([runtime.rs:166](src/derivation/runtime.rs#L166)). `ParsedRequest` implements `Drop` to zeroize the seed phrase, passphrase, HMAC key, wordlist, path, and salt prefix on scope exit ([runtime.rs:184](src/derivation/runtime.rs#L184)).

### Caller-tunable fields on the request

| Field | Default when `None`/`0` | Effect |
|---|---|---|
| `seed_phrase` | required | BIP-39 mnemonic (any supported wordlist). |
| `passphrase` | `""` | BIP-39 optional passphrase ("25th word"). |
| `derivation_path` | chain default (BIP-44/49/84/86 etc.) | Full path string, e.g. `m/84'/0'/0'/0/0`. |
| `hmac_key` | `"Bitcoin seed"` (secp) / `"ed25519 seed"` (ed25519) | Master-key HMAC constant. |
| `mnemonic_wordlist` | `"english"` | Any of the 10 BIP-39 languages bundled via `bip39/all-languages`. |
| `iteration_count` | `2048` | PBKDF2 rounds. |
| `salt_prefix` | `"mnemonic"` | String prepended to the passphrase inside the PBKDF2 salt. |
| `chain`, `network`, `curve`, `address_algorithm`, `public_key_format`, `script_type` | — | Address-format selectors (enums). |

---

## Stage 1 — Mnemonic → 64-byte seed (BIP-39)

Implemented in `derive_bip39_seed` ([runtime.rs:1702](src/derivation/runtime.rs#L1702)).

1. **Resolve language.** `resolve_bip39_language` ([runtime.rs:1820](src/derivation/runtime.rs#L1820)) maps the `mnemonic_wordlist` string to a `bip39::Language`. Unknown wordlists are rejected.
2. **Validate + parse mnemonic.** `Mnemonic::parse_in_normalized` enforces the checksum and the chosen wordlist.
3. **Normalize text.** Mnemonic, passphrase, and salt prefix are each run through Unicode **NFKD** (required by BIP-39).
4. **Build salt.** `salt = nfkd(salt_prefix) || nfkd(passphrase)`. BIP-39's spec value is `"mnemonic"`, but any string is accepted.
5. **PBKDF2-HMAC-SHA512.** `pbkdf2_hmac::<Sha512>(mnemonic_bytes, salt_bytes, iterations, &mut seed)` produces a 64-byte seed. Default iteration count is 2048; callers may raise or lower it.
6. **Zeroize.** The normalized mnemonic/passphrase/salt and the output seed are all held in `Zeroizing<_>` containers so they scrub on drop.

**Output:** `Zeroizing<[u8; 64]>` — the BIP-32/SLIP-0010 input seed.

---

## Stage 2a — Seed → secp256k1 private key (BIP-32)

Used by: Bitcoin family, EVM chains, Tron, XRP, Stellar (secp variant), Cardano (where configured).

Implemented in `derive_bip32_xpriv` ([runtime.rs:1607](src/derivation/runtime.rs#L1607)) + `derive_bip32_master` ([runtime.rs:1626](src/derivation/runtime.rs#L1626)).

1. **Resolve HMAC key.** `hmac_key` from the request, or `b"Bitcoin seed"` by default.
2. **Master node.**
   - `I = HMAC-SHA512(hmac_key, seed)`
   - `IL = I[0..32]` → master private key (validated via `SecretKey::from_slice`)
   - `IR = I[32..64]` → master chain code
   - We build the `Xpriv` struct **directly** with `depth=0`, `parent_fingerprint=0x00000000`, `child_number=Normal{0}`. This is equivalent to `Xpriv::new_master` but avoids the `bitcoin` crate's hardcoded `"Bitcoin seed"` constant so the HMAC key remains a parameter.
3. **Walk the path.** `master.derive_priv(&secp, &path)` runs BIP-32 CKDpriv for each segment in `derivation_path`. Hardened segments (`'` or `h`) use the private key; normal segments use the public key.
4. **Public key.** Generated from the child `SecretKey` via `Secp256k1::<All>::new()`.

**Output:** `Xpriv` (private key + chain code + path metadata) and, downstream, a compressed/uncompressed/x-only `PublicKey`.

---

## Stage 2b — Seed → ed25519 private key (SLIP-0010)

Used by: Solana, Sui, Aptos, Stellar (ed25519), Cardano (where configured), Polkadot, NEAR, Internet Computer, TON.

Implemented in `derive_solana_ed25519_key` ([runtime.rs:1752](src/derivation/runtime.rs#L1752)) + `parse_slip10_ed25519_path` ([runtime.rs:1787](src/derivation/runtime.rs#L1787)).

1. **Resolve HMAC key.** `hmac_key` from the request, or `b"ed25519 seed"` by default.
2. **Master node.**
   - `I = HMAC-SHA512(hmac_key, seed)`
   - `IL` → private key, `IR` → chain code.
3. **Parse path.** SLIP-0010 for ed25519 permits **only hardened** children. `parse_slip10_ed25519_path` accepts BIP-32-style strings (`m/44'/501'/0'/0'`) and **force-hardens every segment** — matches what every ed25519 wallet in the ecosystem actually does with paths like `m/44'/501'/0'/0'`.
4. **CKDpriv loop.** For each hardened index `i`:
   - `data = 0x00 || private_key || i_be32`
   - `I = HMAC-SHA512(chain_code, data)`
   - `private_key = I[0..32]`, `chain_code = I[32..64]`.
5. **Public key.** Derived from the private scalar via `ed25519-dalek` (SHA-512 clamp → scalar → `G * scalar`) when the consumer asks for a public key.

Hand-rolled specifically so the HMAC master-key constant is caller-tunable; the previously used `slip10` crate hardcoded `"ed25519 seed"`.

**Output:** `Zeroizing<[u8; 32]>` — the 32-byte ed25519 private scalar.

---

## Stage 3 — Private key → chain-specific address

`derive_address_from_keys` ([runtime.rs:834](src/derivation/runtime.rs#L834)) and the per-chain helpers dispatch on `chain` + `address_algorithm`:

| Chain family | Algorithm | Helper |
|---|---|---|
| Bitcoin / BCH / BSV / LTC / DOGE | P2PKH, P2SH-P2WPKH, P2WPKH, P2TR | `derive_bitcoin_address_for_network` ([runtime.rs:1681](src/derivation/runtime.rs#L1681)) |
| Ethereum / Arbitrum / Optimism / Avalanche / BNB / Hyperliquid / EthClassic | Keccak-256 of uncompressed pubkey | `derive_evm_address` ([runtime.rs:1863](src/derivation/runtime.rs#L1863)) |
| Tron | EVM derivation + `0x41` prefix + Base58Check | `derive_tron` ([runtime.rs:1330](src/derivation/runtime.rs#L1330)) |
| XRP | secp pubkey → RIPEMD160(SHA256) → XRP Base58Check | `derive_xrp` ([runtime.rs:1357](src/derivation/runtime.rs#L1357)) |
| Solana | ed25519 pubkey → Base58 | `derive_solana` ([runtime.rs:1384](src/derivation/runtime.rs#L1384)) |
| Stellar | ed25519 pubkey → StrKey (Base32) | `derive_stellar` ([runtime.rs:1405](src/derivation/runtime.rs#L1405)) |
| Cardano / Sui / Aptos / TON / ICP / NEAR / Polkadot | per-chain formatting | `derive_*` at [runtime.rs:1440](src/derivation/runtime.rs#L1440)+ |

`script_type` / `public_key_format` / `network` drive the exact output for each.

---

## Customization summary

| Knob | Default | Where consumed |
|---|---|---|
| `iteration_count` | 2048 | [runtime.rs:1716](src/derivation/runtime.rs#L1716) |
| `salt_prefix` | `"mnemonic"` | [runtime.rs:1721](src/derivation/runtime.rs#L1721) |
| `mnemonic_wordlist` | English | [runtime.rs:1713](src/derivation/runtime.rs#L1713) / [runtime.rs:1820](src/derivation/runtime.rs#L1820) |
| `passphrase` | `""` | [runtime.rs:1723](src/derivation/runtime.rs#L1723) |
| `hmac_key` (secp) | `"Bitcoin seed"` | [runtime.rs:1616](src/derivation/runtime.rs#L1616) |
| `hmac_key` (ed25519) | `"ed25519 seed"` | [runtime.rs:1759](src/derivation/runtime.rs#L1759) |
| `derivation_path` | per-chain default ([runtime.rs:395](src/derivation/runtime.rs#L395)) | stages 2a/2b |

Regression coverage for each knob lives in the `#[cfg(test)]` module at the bottom of `runtime.rs` — notably `custom_hmac_key_changes_secp_derivation`, `custom_hmac_key_changes_slip10_derivation`, `custom_salt_prefix_changes_seed`, `custom_iteration_count_changes_seed`, and `default_hmac_key_matches_standard_seed` (which pins the defaults to the spec constants).

---

## Memory safety

Every intermediate that can contain key material is wrapped in `Zeroizing<_>`:

- Normalized mnemonic / passphrase / salt prefix ([runtime.rs:1722-1724](src/derivation/runtime.rs#L1722-L1724))
- 64-byte seed ([runtime.rs:1730](src/derivation/runtime.rs#L1730))
- HMAC-SHA512 output ([runtime.rs:1653](src/derivation/runtime.rs#L1653))
- SLIP-0010 chain code + private key ([runtime.rs:1766-1767](src/derivation/runtime.rs#L1766-L1767))
- `ParsedRequest` fields, via the manual `Drop` impl ([runtime.rs:184](src/derivation/runtime.rs#L184))

The secp `Xpriv` retains its own internal zeroization (`bitcoin` crate handles it); only the final `private_key_hex` string crossing the FFI boundary is the caller's responsibility to clear.
