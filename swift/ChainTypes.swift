import Foundation

// MARK: - Transactions & price alerts (Rust-owned enums)

extension PriceAlertCondition {
    static let allCases: [PriceAlertCondition] = [.above, .below]
}
