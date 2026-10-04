import SwiftUI

/// `address` in groups of four, a `0x` prefix riding on the first, with the
/// first and last groups primary and the rest secondary.
func groupedAddress(_ address: String) -> AttributedString {
    let hasHexPrefix = address.hasPrefix("0x")
    let body = Array(hasHexPrefix ? address.dropFirst(2) : Substring(address))
    var groups = stride(from: 0, to: body.count, by: 4).map { String(body[$0..<min($0 + 4, body.count)]) }
    guard !groups.isEmpty else { return AttributedString(address) }
    if hasHexPrefix { groups[0] = "0x" + groups[0] }
    var text = AttributedString()
    for (index, group) in groups.enumerated() {
        if index > 0 { text += AttributedString(" ") }
        var part = AttributedString(group)
        part.swiftUI.foregroundColor = index == 0 || index == groups.count - 1 ? .primary : .secondary
        text += part
    }
    return text
}
