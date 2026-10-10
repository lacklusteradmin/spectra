import Foundation
import Testing

@testable import Spectra

/// The entry grid follows core's verdict: it grows to hold what is pasted,
/// and a fixed length is refused by core rather than enforced by cutting.
@MainActor
struct SeedPhraseEntryTests {
    private let zero24 = Array(repeating: "abandon", count: 23).joined(separator: " ") + " art"

    @Test func pastingTwentyFourWordsGrowsTheGridToTwentyFour() {
        let entry = SeedPhraseEntry()
        #expect(entry.slots.count == 12)
        entry.paste(zero24)
        #expect(entry.slots.count == 24)
        #expect(entry.verdict.wordCount == 24)
        #expect(entry.verdict.language?.code == "en")
        #expect(entry.verdict.isValid)
        #expect(entry.phrase == zero24)
    }

    @Test func aFixedLengthKeepsEveryPastedWordAndRefusesThePhrase() {
        let entry = SeedPhraseEntry()
        entry.wordCountOverride = 12
        entry.paste(zero24)
        #expect(entry.slots.count == 24)
        #expect(entry.verdict.problem == .wrongWordCount(expected: 12))
        #expect(!entry.verdict.isValid)
    }

    @Test func moreWordsStepsToTheNextStandardLength() {
        let entry = SeedPhraseEntry()
        entry.addSlots()
        #expect(entry.slots.count == 15)
        entry.paste(zero24)
        #expect(entry.nextSlotCount == nil)
    }

    /// Words typed faster than focus moves arrive in one slot together; each
    /// takes its own slot from there on, and none is lost past the grid's end.
    @Test func severalWordsInOneSlotFillTheSlotsAfterIt() {
        let entry = SeedPhraseEntry()
        entry.update(at: 10, with: "Abandon about zoo ")
        #expect(Array(entry.slots[10...12]) == ["abandon", "about", "zoo"])
        #expect(entry.slots.count >= 13)
        entry.update(at: 10, with: "")
        #expect(entry.slot(at: 10).isEmpty)
        #expect(entry.slot(at: 11) == "about")
    }

    @Test func clearingReturnsTheGridToItsStartingLength() {
        let entry = SeedPhraseEntry()
        entry.paste(zero24)
        entry.clear()
        #expect(entry.slots == Array(repeating: "", count: 12))
    }

    /// Core states the problem; the words are this app's, in its language.
    @Test func everyProblemIsWorded() {
        let problems: [SeedPhraseProblem] = [
            .nonStandardLength(wordCount: 25, allowed: [12, 15]), .wrongWordCount(expected: 12), .invalidChecksum,
            .ambiguousLanguage, .encryptedPolyseed, .unsupportedPolyseed,
        ]
        for problem in problems {
            #expect(!problem.localizedMessage.isEmpty)
        }
        // The lengths named are core's, not a list kept beside them.
        let message = SeedPhraseProblem.nonStandardLength(wordCount: 25, allowed: [16, 25]).localizedMessage
        #expect(message.contains("16"))
    }

    /// The grid takes the network's own lengths: Monero's Polyseed and seed,
    /// TON's mnemonic.
    @Test func theGridFollowsTheNetworksFormats() {
        let entry = SeedPhraseEntry()
        entry.chain = .monero
        #expect(entry.slots.count == 16)
        #expect(entry.nextSlotCount == 25)
        entry.chain = .ton
        #expect(entry.slots.count == 24)
        #expect(entry.nextSlotCount == nil)
    }

    /// A created phrase is in the format the network's own wallets restore.
    @Test func createdPhrasesAreInEachNetworksFormat() {
        let draft = WalletImportDraft()
        for (chain, words, format) in [
            (Chain.monero, 25, WalletSecretFormat.moneroPhrase), (.ton, 24, .tonMnemonic), (.bitcoin, 12, .bip39Phrase),
        ] {
            draft.configure(chain: chain, method: .createPhrase)
            #expect(draft.seedPhraseWords.count == words, "\(chain.id)")
            #expect(draft.seedEntry.verdict.isValid, "\(chain.id)")
            #expect(draft.seedEntry.verdict.format == format, "\(chain.id)")
        }
        draft.configure(chain: .monero, method: .createPhrase)
        #expect(draft.createdLengths.map(\.wordCount) == [25])
    }

    /// A created phrase goes through the same entry, judged at the length
    /// it was generated at.
    @Test func aCreatedPhraseIsJudgedAtItsLength() {
        let draft = WalletImportDraft()
        draft.configure(chain: .bitcoin, method: .createPhrase)
        draft.selectedSeedPhraseWordCount = 24
        #expect(draft.seedPhraseWords.count == 24)
        #expect(draft.seedEntry.verdict.isValid)
    }
}
