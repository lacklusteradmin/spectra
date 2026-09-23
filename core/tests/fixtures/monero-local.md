# Monero local-signing fixture

Generated with official Monero v0.18.5.1 (`--regtest --offline --fixed-difficulty 1`).
The downloaded mac-armv8 archive matched the official hashes.txt SHA256:
`dba08921841e675384ce019fd7c93b59fe7b1e6edaa0a3cf0e3253e263f61864`.

This is **public test material with no value**, derived from the BIP39 phrase
`abandon` repeated eleven times followed by `about`. The cache contains 66
regtest blocks' outputs. Spectra built and signed the transaction locally;
monerod accepted `accepted_raw` via `send_raw_transaction`. No wallet RPC was
used. A loopback proxy changes only get_info.nettype from fakechain to mainnet,
allowing production validation to run without adding a fakechain escape hatch.

The core regression test decrypts the cache, signs the frozen plan again,
compares its public transaction prefix with the daemon-accepted transaction,
and rejects changed recipient/amount, bad encryption keys, locked and spent
inputs. The fixture is not a claim of mainnet inclusion or an independent audit
of monero-wallet. The supported spend format is RingCT with HF16 CLSAG and
Bulletproof+; legacy pre-RingCT outputs and subaddress discovery are not supported.
