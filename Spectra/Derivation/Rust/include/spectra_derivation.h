#ifndef SPECTRA_DERIVATION_H
#define SPECTRA_DERIVATION_H

#include <stdint.h>
#include <stddef.h>

#define SPECTRA_CHAIN_BITCOIN 0
#define SPECTRA_CHAIN_ETHEREUM 1
#define SPECTRA_CHAIN_SOLANA 2
#define SPECTRA_CHAIN_BITCOIN_CASH 3
#define SPECTRA_CHAIN_BITCOIN_SV 4
#define SPECTRA_CHAIN_LITECOIN 5
#define SPECTRA_CHAIN_DOGECOIN 6
#define SPECTRA_CHAIN_ETHEREUM_CLASSIC 7
#define SPECTRA_CHAIN_ARBITRUM 8
#define SPECTRA_CHAIN_OPTIMISM 9
#define SPECTRA_CHAIN_AVALANCHE 10
#define SPECTRA_CHAIN_HYPERLIQUID 11
#define SPECTRA_CHAIN_TRON 12
#define SPECTRA_CHAIN_STELLAR 13
#define SPECTRA_CHAIN_XRP 14
#define SPECTRA_CHAIN_CARDANO 15
#define SPECTRA_CHAIN_SUI 16
#define SPECTRA_CHAIN_APTOS 17
#define SPECTRA_CHAIN_TON 18
#define SPECTRA_CHAIN_INTERNET_COMPUTER 19
#define SPECTRA_CHAIN_NEAR 20
#define SPECTRA_CHAIN_POLKADOT 21

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

typedef struct SpectraPrivateKeyDerivationRequest {
    uint32_t chain;
    uint32_t network;
    uint32_t curve;
    uint32_t address_algorithm;
    uint32_t public_key_format;
    uint32_t script_type;
    SpectraBuffer private_key_hex_utf8;
} SpectraPrivateKeyDerivationRequest;

typedef struct SpectraDerivationResponse {
    int32_t status_code;
    SpectraBuffer address_utf8;
    SpectraBuffer public_key_hex_utf8;
    SpectraBuffer private_key_hex_utf8;
    SpectraBuffer error_message_utf8;
} SpectraDerivationResponse;

typedef struct SpectraJsonResponse {
    int32_t status_code;
    SpectraBuffer payload_utf8;
    SpectraBuffer error_message_utf8;
} SpectraJsonResponse;

typedef struct SpectraDerivationPathResolutionRequest {
    uint32_t chain;
    SpectraBuffer derivation_path_utf8;
} SpectraDerivationPathResolutionRequest;

typedef struct SpectraStringRequest {
    SpectraBuffer utf8;
} SpectraStringRequest;

typedef struct SpectraStringArrayRequest {
    SpectraBuffer json_utf8;
} SpectraStringArrayRequest;

typedef struct SpectraEndpointQueryRequest {
    SpectraBuffer chain_name_utf8;
    uint32_t role_mask;
    uint8_t settings_visible_only;
} SpectraEndpointQueryRequest;

typedef struct SpectraCoreLocalizationDocumentRequest {
    SpectraBuffer resource_name_utf8;
    SpectraBuffer preferred_locales_json_utf8;
} SpectraCoreLocalizationDocumentRequest;

typedef struct SpectraCoreStateReduceRequest {
    SpectraBuffer state_json_utf8;
    SpectraBuffer command_json_utf8;
} SpectraCoreStateReduceRequest;

typedef struct SpectraCoreJSONRequest {
    SpectraBuffer json_utf8;
} SpectraCoreJSONRequest;

SpectraDerivationResponse *spectra_derivation_derive(const SpectraDerivationRequest *request);
SpectraDerivationResponse *spectra_derivation_derive_from_private_key(const SpectraPrivateKeyDerivationRequest *request);
void spectra_derivation_response_free(SpectraDerivationResponse *response);
void spectra_derivation_buffer_free(SpectraBuffer buffer);
SpectraJsonResponse *spectra_app_core_chain_presets_json(void);
SpectraJsonResponse *spectra_app_core_request_compilation_presets_json(void);
SpectraJsonResponse *spectra_app_core_resolve_derivation_path_json(const SpectraDerivationPathResolutionRequest *request);
SpectraJsonResponse *spectra_app_core_derivation_paths_for_preset_json(uint32_t account_index);
SpectraJsonResponse *spectra_app_core_endpoint_records_json(void);
SpectraJsonResponse *spectra_app_core_endpoint_for_id_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_endpoints_for_ids_json(const SpectraStringArrayRequest *request);
SpectraJsonResponse *spectra_app_core_endpoint_records_for_chain_json(const SpectraEndpointQueryRequest *request);
SpectraJsonResponse *spectra_app_core_grouped_settings_entries_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_diagnostics_checks_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_transaction_explorer_entry_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_bitcoin_esplora_base_urls_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_bitcoin_wallet_store_default_base_urls_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_evm_rpc_endpoints_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_explorer_supplemental_endpoints_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_broadcast_provider_options_json(const SpectraStringRequest *request);
SpectraJsonResponse *spectra_app_core_chain_backends_json(void);
SpectraJsonResponse *spectra_app_core_live_chain_names_json(void);
SpectraJsonResponse *spectra_app_core_app_chain_descriptors_json(void);
void spectra_app_core_json_response_free(SpectraJsonResponse *response);
SpectraJsonResponse *spectra_core_bootstrap_json(void);
SpectraJsonResponse *spectra_core_localization_document_json(const SpectraCoreLocalizationDocumentRequest *request);
SpectraJsonResponse *spectra_core_reduce_state_json(const SpectraCoreStateReduceRequest *request);
SpectraJsonResponse *spectra_core_migrate_legacy_wallet_store_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_export_legacy_wallet_store_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_plan_wallet_import_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_active_maintenance_plan_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_should_run_background_maintenance_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_chain_refresh_plans_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_history_refresh_plans_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_normalize_history_json(const SpectraCoreJSONRequest *request);
SpectraJsonResponse *spectra_core_merge_transactions_json(const SpectraCoreJSONRequest *request);
void spectra_core_json_response_free(SpectraJsonResponse *response);

#endif
