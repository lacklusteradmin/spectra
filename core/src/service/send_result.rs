//! Protocol results stay typed until the persistence/FFI output boundary.
use crate::fetch::{self, SignedSubmission};

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub(crate) enum ProtocolSendResult {
    Bitcoin(fetch::bitcoin::BitcoinSendResult),
    Evm(fetch::evm::EvmSendResult),
    Solana(fetch::solana::SolanaSendResult),
    Xrp(fetch::xrp::XrpSendResult),
    Tron(fetch::tron::TronSendResult),
    Sui(fetch::sui::SuiSendResult),
    Aptos(fetch::aptos::AptosSendResult),
    Near(fetch::near::NearSendResult),
    Stellar(fetch::stellar::StellarSendResult),
    Cardano(fetch::cardano::CardanoSendResult),
    Polkadot(fetch::polkadot::DotSendResult),
    Bittensor(fetch::bittensor::TaoSendResult),
    Ton(fetch::ton::TonSendResult),
    Icp(fetch::icp::IcpSendResult),
    Monero(fetch::monero::MoneroSendResult),
    Blockbook(fetch::blockbook::BlockbookSendResult),
    Decred(fetch::decred::DcrSendResult),
    Kaspa(fetch::kaspa::KasSendResult),
    Dogecoin(fetch::dogecoin::DogeSendResult),
    BitcoinSV(fetch::bitcoin_sv::BsvSendResult),
}
impl ProtocolSendResult {
    fn submission(&self) -> &dyn SignedSubmission {
        match self {
            Self::Bitcoin(result) => result,
            Self::Evm(result) => result,
            Self::Solana(result) => result,
            Self::Xrp(result) => result,
            Self::Tron(result) => result,
            Self::Sui(result) => result,
            Self::Aptos(result) => result,
            Self::Near(result) => result,
            Self::Stellar(result) => result,
            Self::Cardano(result) => result,
            Self::Polkadot(result) => result,
            Self::Bittensor(result) => result,
            Self::Ton(result) => result,
            Self::Icp(result) => result,
            Self::Monero(result) => result,
            Self::Blockbook(result) => result,
            Self::Decred(result) => result,
            Self::Kaspa(result) => result,
            Self::Dogecoin(result) => result,
            Self::BitcoinSV(result) => result,
        }
    }
    pub(super) fn transaction_hash(&self) -> &str {
        self.submission().submission_id()
    }
    pub(super) fn signed_payload(&self) -> &str {
        self.submission().signed_payload()
    }
    pub(super) fn evm(
        &self,
    ) -> Result<Option<crate::send::ethereum::EvmSendDetails>, crate::SpectraBridgeError> {
        let Self::Evm(r) = self else { return Ok(None) };
        Ok(Some(crate::send::ethereum::EvmSendDetails {
            txid: r.txid.clone(),
            raw_tx_hex: r.raw_tx_hex.clone(),
            nonce: i64::try_from(r.nonce).map_err(|_| "EVM nonce exceeds supported range")?,
            gas_limit: i64::try_from(r.gas_limit)
                .map_err(|_| "EVM gas limit exceeds supported range")?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_evm_result_preserves_reviewed_values_and_rejects_overflow() {
        let mut result = fetch::evm::EvmSendResult {
            txid: "0xabc".into(),
            raw_tx_hex: "0x1234".into(),
            nonce: 12,
            gas_limit: 21000,
            max_fee_per_gas_wei: "50".into(),
            max_priority_fee_per_gas_wei: "2".into(),
        };
        let typed = ProtocolSendResult::Evm(result.clone());
        assert_eq!(typed.transaction_hash(), "0xabc");
        assert_eq!(typed.signed_payload(), "0x1234");
        let evm = typed.evm().unwrap().unwrap();
        assert_eq!((evm.nonce, evm.gas_limit), (12, 21000));
        assert_eq!(
            serde_json::to_value(&typed).unwrap(),
            serde_json::to_value(&result).unwrap()
        );
        result.nonce = u64::MAX;
        assert!(ProtocolSendResult::Evm(result).evm().is_err());
    }
}
