#ifndef SPECTRA_DERIVATION_H
#define SPECTRA_DERIVATION_H

#include <stdint.h>
#include <stddef.h>

#define SPECTRA_CHAIN_BITCOIN 0
#define SPECTRA_CHAIN_ETHEREUM 1
#define SPECTRA_CHAIN_SOLANA 2

#define SPECTRA_NETWORK_MAINNET 0
#define SPECTRA_NETWORK_TESTNET 1
#define SPECTRA_NETWORK_TESTNET4 2
#define SPECTRA_NETWORK_SIGNET 3

#define SPECTRA_CURVE_SECP256K1 0
#define SPECTRA_CURVE_ED25519 1

#define SPECTRA_OUTPUT_ADDRESS (1u << 0)
#define SPECTRA_OUTPUT_PUBLIC_KEY (1u << 1)
#define SPECTRA_OUTPUT_PRIVATE_KEY (1u << 2)

#define SPECTRA_DERIVATION_AUTO 0
#define SPECTRA_DERIVATION_BIP32_SECP256K1 1
#define SPECTRA_DERIVATION_SLIP10_ED25519 2

#define SPECTRA_ADDRESS_AUTO 0
#define SPECTRA_ADDRESS_BITCOIN 1
#define SPECTRA_ADDRESS_EVM 2
#define SPECTRA_ADDRESS_SOLANA 3

#define SPECTRA_PUBLIC_KEY_AUTO 0
#define SPECTRA_PUBLIC_KEY_COMPRESSED 1
#define SPECTRA_PUBLIC_KEY_UNCOMPRESSED 2
#define SPECTRA_PUBLIC_KEY_X_ONLY 3
#define SPECTRA_PUBLIC_KEY_RAW 4

#define SPECTRA_SCRIPT_AUTO 0
#define SPECTRA_SCRIPT_P2PKH 1
#define SPECTRA_SCRIPT_P2SH_P2WPKH 2
#define SPECTRA_SCRIPT_P2WPKH 3
#define SPECTRA_SCRIPT_P2TR 4
#define SPECTRA_SCRIPT_ACCOUNT 5

typedef struct SpectraBuffer {
    uint8_t *ptr;
    size_t len;
} SpectraBuffer;

typedef struct SpectraDerivationRequest {
    uint32_t chain;
    uint32_t network;
    uint32_t curve;
    uint32_t requested_outputs;
    uint32_t derivation_algorithm;
    uint32_t address_algorithm;
    uint32_t public_key_format;
    uint32_t script_type;
    SpectraBuffer seed_phrase_utf8;
    SpectraBuffer derivation_path_utf8;
    SpectraBuffer passphrase_utf8;
    SpectraBuffer hmac_key_utf8;
    SpectraBuffer mnemonic_wordlist_utf8;
    uint32_t iteration_count;
} SpectraDerivationRequest;

typedef struct SpectraDerivationResponse {
    int32_t status_code;
    SpectraBuffer address_utf8;
    SpectraBuffer public_key_hex_utf8;
    SpectraBuffer private_key_hex_utf8;
    SpectraBuffer error_message_utf8;
} SpectraDerivationResponse;

SpectraDerivationResponse *spectra_derivation_derive(const SpectraDerivationRequest *request);
void spectra_derivation_response_free(SpectraDerivationResponse *response);
void spectra_derivation_buffer_free(SpectraBuffer buffer);

#endif
