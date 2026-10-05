import Foundation

extension ChainTag {
    /// Every tag, in the order the picker offers them.
    static let pickerOrder: [ChainTag] = listChainTags()

    var title: String {
        switch self {
        case .layer1: AppLocalization.string("Layer 1")
        case .layer2: AppLocalization.string("Layer 2")
        case .evm: AppLocalization.string("EVM")
        case .utxo: AppLocalization.string("UTXO")
        case .eutxo: AppLocalization.string("eUTXO")
        case .move: AppLocalization.string("Move")
        case .substrate: AppLocalization.string("Substrate")
        case .pow: AppLocalization.string("PoW")
        case .privacy: AppLocalization.string("Privacy")
        case .payments: AppLocalization.string("Payments")
        case .testnet: AppLocalization.string("Testnet")
        }
    }
}

