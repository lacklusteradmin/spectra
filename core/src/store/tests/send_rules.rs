use crate::registry::{Chain, SendRule};

fn rule(display_name: &str) -> SendRule {
    Chain::from_display_name(display_name)
        .unwrap_or_else(|| panic!("unknown chain {display_name}"))
        .send_rule()
}

#[test]
fn the_two_exceptions_and_the_default() {
    assert_eq!(rule("Ethereum Classic"), SendRule::NativeOnly);
    assert_eq!(rule("Hyperliquid"), SendRule::NativeOnly);
    assert_eq!(rule("Solana"), SendRule::SupportedSolanaCoin);
    // Non-EVM chains carry no extra restriction.
    assert_eq!(rule("Bitcoin"), SendRule::Any);
    assert_eq!(rule("Polkadot"), SendRule::Any);
}

/// One rule for the EVM family.
///
/// This replaces `send_rule_asymmetry_across_evm_chains`, which asserted
/// that exactly three of twenty-three EVM chains gated non-native sends —
/// a test whose only job was to stop anyone fixing the split. Arbitrum and
/// Ethereum answer the same way now, and the two deliberate exceptions
/// (`EthereumClassic`, `Hyperliquid`) are native-only, which is stricter
/// still.
#[test]
fn every_evm_chain_gates_non_native_sends_the_same_way() {
    for chain in Chain::all().filter(|c| c.is_evm()) {
        let expected = match chain.mainnet_counterpart() {
            Chain::EthereumClassic | Chain::Hyperliquid => SendRule::NativeOnly,
            _ => SendRule::NativeOrSupportedToken,
        };
        assert_eq!(
            chain.send_rule(),
            expected,
            "{} gates differently from the rest of the EVM family",
            chain.str_id()
        );
    }
}

#[test]
fn a_testnet_sends_under_the_same_rule_as_its_mainnet() {
    for chain in Chain::all() {
        let mainnet = chain.mainnet_counterpart();
        if mainnet == chain {
            continue;
        }
        assert_eq!(
            chain.send_rule(),
            mainnet.send_rule(),
            "{} diverges from {}",
            chain.str_id(),
            mainnet.str_id()
        );
    }
}
