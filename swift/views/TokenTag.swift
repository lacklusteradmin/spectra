import Foundation

extension TokenTag {
    /// Every tag, in the order a filter offers them.
    static let filterOrder: [TokenTag] = listTokenTags()

    var title: String {
        switch self {
        case .stablecoin: AppLocalization.string("Stablecoin")
        case .meme: AppLocalization.string("Meme")
        }
    }
}
