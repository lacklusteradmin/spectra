//! Litecoin fee and capacity use the same owned inputs as the durable builder.
use crate::registry::Chain;
use crate::send::error::SendError;
use crate::send::litecoin_mweb::prepared::CanonicalRecipient;
use crate::send::preview_types::BitcoinSendPreview;
use crate::send::stages::UtxoPreparedInput;

pub(crate) fn quote_inputs(
    chain: Chain,
    inputs: &[UtxoPreparedInput],
    change_script: &[u8],
    destination: &str,
    amount: u64,
    rate: u64,
) -> Result<Option<BitcoinSendPreview>, SendError> {
    if amount > chain.litecoin_max_money()? {
        return Err(SendError::invalid("Litecoin amount exceeds MAX_MONEY"));
    }
    // An empty form quotes space for the largest standard transparent
    // output. An MWEB recipient is paid by a peg-in, whose kernel's fee the
    // canonical output carries beside the amount.
    let (recipient_len, recipient_dust, mweb_fee) = if destination.trim().is_empty() {
        (34, None, 0)
    } else {
        let recipient = CanonicalRecipient::parse(chain, destination)?;
        let dust = recipient.minimum_amount(chain)?;
        if amount > 0 && amount < dust {
            return Err(SendError::invalid(
                "Litecoin recipient amount is below the dust threshold",
            ));
        }
        (recipient.script_len(), Some(dust), recipient.mweb_fee()?)
    };
    if inputs.is_empty() {
        return Ok(None);
    }
    let total =
        super::litecoin::validate_ltc_values(chain, inputs.iter().map(|input| input.utxo.2))?;
    let size = super::litecoin::estimate_ltc_vsize(
        inputs.iter().map(|input| input.utxo.3.as_slice()),
        recipient_len,
        Some(change_script),
    )?;
    let fee = size
        .checked_mul(rate)
        .ok_or_else(|| SendError::invalid("Litecoin fee overflow"))?;
    let remainder = total
        .checked_sub(amount)
        .and_then(|value| value.checked_sub(fee))
        .and_then(|value| value.checked_sub(mweb_fee));
    let dust = super::litecoin::litecoin_dust_threshold(chain, change_script)?;
    let uses_change = remainder.is_some_and(|change| change >= dust && change > 0);
    let actual_fee = match remainder {
        Some(change) if !uses_change => fee
            .checked_add(change)
            .ok_or_else(|| SendError::invalid("Litecoin fee overflow"))?,
        _ => fee,
    };
    let actual_fee = actual_fee
        .checked_add(mweb_fee)
        .ok_or_else(|| SendError::invalid("Litecoin fee overflow"))?;
    let maximum = total.saturating_sub(fee).saturating_sub(mweb_fee);
    let maximum = if recipient_dust.is_some_and(|dust| maximum < dust) {
        0
    } else {
        maximum
    };
    let coins = |value| crate::decimal::from_units(u128::from(value), 8);
    Ok(Some(BitcoinSendPreview {
        estimatedFeeRateSatVb: rate,
        estimatedNetworkFee: coins(actual_fee),
        feeRateDescription: Some(format!("{rate} sat/vB")),
        spendableBalance: Some(coins(total)),
        estimatedTransactionBytes: Some(i64::try_from(size).map_err(SendError::invalid)?),
        selectedInputCount: Some(i64::try_from(inputs.len()).map_err(SendError::invalid)?),
        usesChangeOutput: Some(uses_change),
        maxSendable: Some(coins(maximum)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derivation::litecoin::encode_litecoin_address;
    use crate::derivation::types::BitcoinScriptType;
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::send::stages::UtxoSendSource;

    fn prepared_input(
        chain: Chain,
        script_type: BitcoinScriptType,
        value: u64,
    ) -> UtxoPreparedInput {
        let secp = secp256k1::Secp256k1::new();
        let public_key = secp256k1::PublicKey::from_secret_key(
            &secp,
            &secp256k1::SecretKey::from_slice(&[1; 32]).unwrap(),
        );
        let address = encode_litecoin_address(chain, script_type, &public_key).unwrap();
        let script = parse_utxo_address(chain, &address).unwrap().script_pubkey();
        UtxoPreparedInput {
            source: UtxoSendSource {
                address,
                derivation_path: None,
                script_pubkey: script.clone(),
            },
            utxo: ("11".repeat(32), 0, value, script),
        }
    }

    fn units(display: &str) -> u64 {
        u64::try_from(crate::send::amount_input::parse_raw_amount(display, 8).unwrap()).unwrap()
    }

    #[test]
    fn quoted_dust_fee_and_change_match_the_signed_transaction_on_both_networks() {
        for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
            for script_type in [
                BitcoinScriptType::P2pkh,
                BitcoinScriptType::P2wpkh,
                BitcoinScriptType::P2shP2wpkh,
            ] {
                let input = prepared_input(chain, script_type, 100_000);
                let destination = prepared_input(chain, BitcoinScriptType::P2pkh, 1)
                    .source
                    .address;
                let recipient = parse_utxo_address(chain, &destination)
                    .unwrap()
                    .script_pubkey();
                let estimated_size = crate::send::litecoin::estimate_ltc_vsize(
                    [input.utxo.3.as_slice()],
                    recipient.len(),
                    Some(&input.utxo.3),
                )
                .unwrap();
                let base_fee = estimated_size * 2;
                let dust =
                    crate::send::litecoin::litecoin_dust_threshold(chain, &input.utxo.3).unwrap();
                for change in [0, dust - 1, dust, dust + 1] {
                    let amount = input.utxo.2 - base_fee - change;
                    let preview = quote_inputs(
                        chain,
                        std::slice::from_ref(&input),
                        &input.utxo.3,
                        &destination,
                        amount,
                        2,
                    )
                    .unwrap()
                    .unwrap();
                    let fee = units(&preview.estimatedNetworkFee);
                    let uses_change = change >= dust && change > 0;
                    assert_eq!(preview.usesChangeOutput, Some(uses_change));
                    assert_eq!(fee, base_fee + if uses_change { 0 } else { change });
                    assert_eq!(
                        units(preview.maxSendable.as_deref().unwrap()),
                        input.utxo.2 - base_fee
                    );
                    let raw = crate::send::litecoin::sign_ltc_with_output_script(
                        chain,
                        std::slice::from_ref(&input.utxo),
                        &recipient,
                        amount,
                        fee,
                        &input.source.address,
                        &[1; 32],
                    )
                    .unwrap();
                    let transaction: bitcoin::Transaction =
                        bitcoin::consensus::deserialize(&raw).unwrap();
                    assert_eq!(transaction.output.len(), if uses_change { 2 } else { 1 });
                    let output_total: u64 =
                        transaction.output.iter().map(|o| o.value.to_sat()).sum();
                    assert_eq!(input.utxo.2 - output_total, fee);
                    assert!(transaction.vsize() as u64 <= estimated_size);
                }
            }
        }
    }

    #[test]
    fn empty_and_insufficient_wallets_quote_capacity_without_inventing_funds() {
        let chain = Chain::Litecoin;
        let input = prepared_input(chain, BitcoinScriptType::P2wpkh, 10);
        assert!(
            quote_inputs(chain, &[], &input.utxo.3, "", 0, 1)
                .unwrap()
                .is_none()
        );
        assert!(quote_inputs(chain, &[], &input.utxo.3, "invalid", 0, 1).is_err());
        let change_script = input.utxo.3.clone();
        let preview = quote_inputs(chain, &[input], &change_script, "", 10, 1)
            .unwrap()
            .unwrap();
        assert_eq!(preview.spendableBalance.as_deref(), Some("0.0000001"));
        assert_eq!(preview.maxSendable.as_deref(), Some("0"));
        assert_eq!(preview.usesChangeOutput, Some(false));
        assert!(units(&preview.estimatedNetworkFee) > 10);
    }

    #[test]
    fn recipient_dust_is_refused_while_zero_or_blank_form_can_still_quote() {
        let chain = Chain::Litecoin;
        let input = prepared_input(chain, BitcoinScriptType::P2wpkh, 100_000);
        for script_type in [
            BitcoinScriptType::P2pkh,
            BitcoinScriptType::P2shP2wpkh,
            BitcoinScriptType::P2wpkh,
        ] {
            let recipient = prepared_input(chain, script_type, 1);
            let dust =
                crate::send::litecoin::litecoin_dust_threshold(chain, &recipient.utxo.3).unwrap();
            let quote = |amount| {
                quote_inputs(
                    chain,
                    std::slice::from_ref(&input),
                    &input.utxo.3,
                    &recipient.source.address,
                    amount,
                    1,
                )
            };
            assert!(quote(dust - 1).is_err());
            assert!(quote(dust).unwrap().is_some());
            assert!(quote(dust + 1).unwrap().is_some());
            assert!(quote(0).unwrap().is_some());
        }
        assert!(
            quote_inputs(chain, std::slice::from_ref(&input), &input.utxo.3, "", 1, 1)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn known_recipient_has_no_sendable_maximum_until_its_dust_threshold_can_be_paid() {
        let chain = Chain::Litecoin;
        let mut input = prepared_input(chain, BitcoinScriptType::P2wpkh, 100_000);
        let recipient = prepared_input(chain, BitcoinScriptType::P2pkh, 1);
        let dust =
            crate::send::litecoin::litecoin_dust_threshold(chain, &recipient.utxo.3).unwrap();
        let size = crate::send::litecoin::estimate_ltc_vsize(
            [input.utxo.3.as_slice()],
            recipient.utxo.3.len(),
            Some(&input.utxo.3),
        )
        .unwrap();
        for (available, maximum) in [(dust + size - 1, 0), (dust + size, dust)] {
            input.utxo.2 = available;
            let preview = quote_inputs(
                chain,
                std::slice::from_ref(&input),
                &input.utxo.3,
                &recipient.source.address,
                0,
                1,
            )
            .unwrap()
            .unwrap();
            assert_eq!(units(preview.maxSendable.as_deref().unwrap()), maximum);
        }
        assert!(
            quote_inputs(
                chain,
                std::slice::from_ref(&input),
                &input.utxo.3,
                "",
                chain.litecoin_max_money().unwrap() + 1,
                1,
            )
            .is_err()
        );
    }

    #[test]
    fn highest_consensus_balance_is_exact_and_provider_or_fee_overflow_is_refused() {
        let chain = Chain::LitecoinTestnet;
        let maximum = chain.litecoin_max_money().unwrap();
        let mut input = prepared_input(chain, BitcoinScriptType::P2wpkh, maximum);
        let preview = quote_inputs(
            chain,
            std::slice::from_ref(&input),
            &input.utxo.3,
            "",
            maximum,
            1,
        )
        .unwrap()
        .unwrap();
        assert_eq!(preview.spendableBalance.as_deref(), Some("84000000"));
        let fee = units(&preview.estimatedNetworkFee);
        assert_eq!(
            units(preview.maxSendable.as_deref().unwrap()),
            maximum - fee
        );
        assert!(
            quote_inputs(
                chain,
                std::slice::from_ref(&input),
                &input.utxo.3,
                "",
                1,
                u64::MAX,
            )
            .is_err()
        );
        input.utxo.2 = maximum + 1;
        assert!(
            quote_inputs(chain, std::slice::from_ref(&input), &input.utxo.3, "", 1, 1,).is_err()
        );
        input.utxo.2 = 0;
        let change_script = input.utxo.3.clone();
        assert!(quote_inputs(chain, &[input], &change_script, "", 1, 1).is_err());
    }
}
