import Foundation

/// One seed-phrase entry grid and core's verdict on it.
///
/// The import flow and the funds finder both read a phrase through this, so
/// the two cannot disagree about what a phrase is. The grid grows to the length
/// core judges the entry at and never drops a typed word: a fixed length that
/// is too short is refused by core rather than enforced here by cutting.
@MainActor
@Observable
final class SeedPhraseEntry {
    /// The network the phrase is for, whose own formats core judges it in —
    /// Monero's 25-word seed or Polyseed, TON's mnemonic, BIP-39 elsewhere.
    /// `nil` judges BIP-39, which is what the funds finder scans.
    var chain: Chain? {
        didSet {
            guard chain != oldValue else { return }
            lengths = seedPhraseLengths(chain: chain)
            reset()
        }
    }
    /// The lengths a phrase on `chain` may have, shortest first, from core.
    private(set) var lengths: [SeedPhraseLength] = seedPhraseLengths(chain: nil)
    /// The grid's length before anything is typed: the network's shortest.
    var initialSlotCount: Int { Int(lengths.first?.wordCount ?? 12) }

    /// The wordlist, from `seedPhraseLanguages(chain:)`, or `nil` for core to detect it.
    var language: String?
    /// One of core's lengths, or `nil` for core to infer it from the words.
    var wordCountOverride: Int? {
        didSet { fitSlots(shrinking: true) }
    }
    /// One entry per slot, normalized as typed.
    private(set) var slots: [String] = Array(repeating: "", count: 12)

    /// Everything core has to say about the grid, decided in one pass. One
    /// render reads this several times, so core is asked once per grid.
    var verdict: SeedPhraseVerdict {
        let check = SeedPhraseCheck(
            words: slots, language: language, wordCount: wordCountOverride.map(UInt32.init), chain: chain)
        if let cached = verdictCache, cached.check == check { return cached.verdict }
        let verdict = checkSeedPhrase(check: check)
        verdictCache = (check, verdict)
        return verdict
    }
    /// The last grid core judged. Holds the words, so `reset` drops it.
    @ObservationIgnored private var verdictCache: (check: SeedPhraseCheck, verdict: SeedPhraseVerdict)?

    /// The phrase as core reads it: the filled slots, normalized and joined.
    var phrase: String { verdict.words.joined(separator: " ") }

    func reset() {
        verdictCache = nil
        language = nil
        wordCountOverride = nil
        slots = Array(repeating: "", count: initialSlotCount)
    }

    /// Replace the grid with `words`, judged at `wordCount`: a generated
    /// phrase, shown rather than typed.
    func load(_ words: [String], wordCount: Int) {
        language = nil
        wordCountOverride = wordCount
        slots = words + Array(repeating: "", count: max(0, wordCount - words.count))
    }

    func slot(at index: Int) -> String {
        slots.indices.contains(index) ? slots[index] : ""
    }

    /// Put what was typed or pasted at `index`. Several words fill the slots
    /// from there on, adding slots rather than dropping words that do not fit.
    func update(at index: Int, with newValue: String) {
        guard slots.indices.contains(index) else { return }
        let words = newValue.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        var updated = slots
        let end = index + max(words.count, 1)
        if updated.count < end { updated.append(contentsOf: Array(repeating: "", count: end - updated.count)) }
        updated.replaceSubrange(index..<end, with: words.isEmpty ? [""] : words)
        guard updated != slots else { return }
        slots = updated
        fitSlots(shrinking: false)
    }

    /// Replace the whole entry with a pasted phrase.
    func paste(_ text: String) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        slots = Array(repeating: "", count: wordCountOverride ?? initialSlotCount)
        update(at: 0, with: text)
    }

    func clear() {
        slots = Array(repeating: "", count: wordCountOverride ?? initialSlotCount)
    }

    /// The network's next length past the grid's, or `nil` at the longest.
    var nextSlotCount: Int? {
        lengths.map { Int($0.wordCount) }.first { $0 > slots.count }
    }

    /// Grow the grid to the next length, for a phrase typed past it.
    func addSlots() {
        guard let next = nextSlotCount else { return }
        slots.append(contentsOf: Array(repeating: "", count: next - slots.count))
    }

    /// Size the grid to the length core judges it at, never below its last
    /// filled slot. Growing alone keeps slots the user added; a new override
    /// also drops blank slots past it.
    private func fitSlots(shrinking: Bool) {
        let filledThrough = (slots.lastIndex { !$0.isEmpty } ?? -1) + 1
        let target = max(Int(verdict.wordCount), filledThrough)
        if slots.count < target {
            slots.append(contentsOf: Array(repeating: "", count: target - slots.count))
        } else if shrinking, slots.count > target {
            slots = Array(slots.prefix(target))
        }
    }
}

extension SeedPhraseProblem {
    /// Core says what is wrong; the sentence is this app's to translate.
    var localizedMessage: String {
        switch self {
        case .nonStandardLength(let wordCount, let allowed):
            let lengths = allowed.map(String.init).formatted(.list(type: .or).locale(AppLocalization.locale))
            return AppLocalization.format("That is %lld words. A seed phrase has %@ words.", Int(wordCount), lengths)
        case .wrongWordCount(let expected):
            return AppLocalization.format("Seed phrase must be %lld words.", Int(expected))
        case .invalidChecksum:
            return AppLocalization.string("Invalid seed phrase checksum. Please verify your words.")
        case .ambiguousLanguage:
            return AppLocalization.string("These words read as different phrases in two languages. Type them in full.")
        case .encryptedPolyseed:
            return AppLocalization.string("This Polyseed is encrypted with a password, which Spectra cannot take for Monero.")
        case .unsupportedPolyseed:
            return AppLocalization.string("This Polyseed uses features Spectra does not support.")
        }
    }
}
