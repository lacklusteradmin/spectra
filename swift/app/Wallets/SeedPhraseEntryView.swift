import SwiftUI

/// The seed-phrase entry grid: one field per slot, as many slots as core
/// judges the phrase at, and what core made of it underneath. The import flow
/// and the funds finder both show this, over the same `SeedPhraseEntry`.
///
/// The slot being typed in is not judged yet: "aban" is on its way to
/// "abandon", not a wrong word. Core's wordlist suggests where it may go.
struct SeedPhraseEntryView: View {
    let entry: SeedPhraseEntry
    @State private var focusedSlot: Int?
    @State private var isConfirmingClear = false
    @State private var fields = SecretWordFieldGroup()
    @ScaledMetric(relativeTo: .callout) private var suggestionBarHeight: CGFloat = 34

    private static let columns = Array(repeating: GridItem(.flexible(), spacing: SpectraLayout.Space.xs), count: 3)
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    /// Two columns at accessibility sizes, so no word is cut short.
    private var columns: [GridItem] {
        dynamicTypeSize.isAccessibilitySize ? Array(Self.columns.prefix(2)) : Self.columns
    }

    var body: some View {
        let verdict = entry.verdict
        let typing = focusedSlot.map { entry.slot(at: $0) } ?? ""
        // The word under the cursor is unfinished, not invalid.
        let invalidWords = Set(verdict.invalidWords).subtracting(typing.isEmpty ? [] : [typing])
        let status = Self.status(of: verdict, settledInvalid: invalidWords)
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            header(verdict)
            suggestionBar(typing: typing)
            LazyVGrid(columns: columns, spacing: SpectraLayout.Space.xs) {
                ForEach(entry.slots.indices, id: \.self) { index in
                    let isInvalid = invalidWords.contains(entry.slot(at: index))
                    SeedPhraseWordCell(index: index, isHighlighted: focusedSlot == index, isInvalid: isInvalid) {
                        // The default keyboard: a Japanese, Korean or Chinese
                        // wordlist cannot be typed on an ASCII one.
                        SecretWordField(
                            text: binding(for: index), isFocused: focusedSlot == index,
                            accessibilityLabel: AppLocalization.format("Word %lld", index + 1),
                            isInvalid: isInvalid,
                            group: fields, index: index,
                            onFocus: { focusedSlot = index },
                            onSubmit: { advance(from: index) })
                    }
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                Label(status.text, systemImage: status.symbol).font(.footnote).foregroundStyle(status.color)
                Spacer(minLength: SpectraLayout.Space.s)
                if entry.wordCountOverride == nil, entry.nextSlotCount != nil {
                    Button(AppLocalization.string("More words"), systemImage: "plus") { entry.addSlots() }
                        .font(.footnote.weight(.semibold)).buttonStyle(.plain).foregroundStyle(.tint)
                }
            }
        }
        .privacySensitive()
        .confirmationDialog(
            AppLocalization.string("Clear every word?"), isPresented: $isConfirmingClear, titleVisibility: .visible
        ) {
            Button(AppLocalization.string("Clear"), role: .destructive) {
                entry.clear()
                focusedSlot = 0
            }
        }
    }

    private func header(_ verdict: SeedPhraseVerdict) -> some View {
        let filled = verdict.words.count
        let isComplete = verdict.isValid
        return HStack(spacing: SpectraLayout.Space.s) {
            Text("\(filled) / \(verdict.wordCount)")
                .font(.caption.weight(.semibold).monospacedDigit())
                .foregroundStyle(isComplete ? Color.green : Color.secondary)
                .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
                .background(Capsule(style: .continuous).fill(isComplete ? Color.green.opacity(0.14) : SpectraLayout.insetFill))
                .accessibilityLabel(AppLocalization.format("%lld of %lld words", filled, Int(verdict.wordCount)))
            Spacer()
            // The system paste button reads the clipboard on the user's tap,
            // so iOS does not ask permission to paste.
            PasteButton(payloadType: String.self) { pasted in
                guard let text = pasted.first else { return }
                entry.paste(text)
                focusedSlot = nil
            }
            .buttonBorderShape(.capsule)
            .labelStyle(.titleAndIcon)
            .controlSize(.small)
            .tint(.accentColor)
            if filled > 0 {
                Button(AppLocalization.string("Clear"), systemImage: "xmark.circle.fill", role: .destructive) {
                    isConfirmingClear = true
                }
                .labelStyle(.iconOnly).font(.title3).buttonStyle(.plain).foregroundStyle(.secondary)
            }
        }
    }

    /// Where the word being typed may be going, from core's wordlist. A tap
    /// finishes the word and moves to the next slot.
    ///
    /// Its height is held for as long as a slot is being typed in, whatever
    /// the suggestions: a bar that came and went with each keystroke moved
    /// the whole grid under the cursor.
    @ViewBuilder
    private func suggestionBar(typing: String) -> some View {
        if let slot = focusedSlot {
            let suggestions = typing.isEmpty ? [] : entry.suggestions(for: typing)
            let shown = suggestions == [typing] ? [] : suggestions
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: SpectraLayout.Space.xs) {
                    ForEach(shown, id: \.self) { word in
                        Button {
                            entry.update(at: slot, with: word)
                            advance(from: slot)
                        } label: {
                            Text(verbatim: word).font(.callout.monospaced().weight(.medium))
                                .padding(.horizontal, SpectraLayout.Space.m).padding(.vertical, SpectraLayout.Space.xs)
                                .spectraInsetFill(cornerRadius: SpectraLayout.Radius.inner)
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint(AppLocalization.format("Word %lld", slot + 1))
                    }
                }
            }
            .scrollClipDisabled()
            .frame(height: suggestionBarHeight)
        }
    }

    private func advance(from index: Int) {
        // A space or Return in the last slot of an inferred length asks for more.
        if index + 1 == entry.slots.count, entry.wordCountOverride == nil, !entry.slot(at: index).isEmpty {
            entry.addSlots()
        }
        let next = (index + 1) < entry.slots.count ? (index + 1) : nil
        focusedSlot = next
        // The keyboard moves now, in the event that typed the space, so the
        // next key lands in the next slot.
        if let next { fields.focus(next) }
    }

    private func binding(for index: Int) -> Binding<String> {
        Binding(
            get: { entry.slot(at: index) },
            set: { newValue in
                entry.update(at: index, with: newValue)
                // Several words arrive at once when typed faster than focus
                // moves, or pasted from the field's edit menu; `update` puts
                // them in the slots from here on, so the cursor follows the
                // last rather than the next key overwriting the second.
                let wordCount = newValue.split(whereSeparator: \.isWhitespace).count
                guard wordCount > 0 else { return }
                let last = index + wordCount - 1
                if newValue.last?.isWhitespace == true {
                    advance(from: last)
                } else if last != index {
                    focusedSlot = last
                    fields.focus(last)
                }
            }
        )
    }

    /// The line under the grid: what core read the entry as, or what is
    /// wrong with it. Never colour alone: each state has its symbol.
    private static func status(
        of verdict: SeedPhraseVerdict, settledInvalid: Set<String>
    ) -> (text: String, color: Color, symbol: String) {
        let languageName = verdict.language.map { AppLocalization.string($0.name) }
        if verdict.words.isEmpty {
            return (AppLocalization.string("import_flow.seed_phrase_detect_hint"), .secondary, "text.cursor")
        }
        if !settledInvalid.isEmpty {
            let words = verdict.invalidWords.filter(settledInvalid.contains).joined(separator: ", ")
            guard let languageName else {
                return (AppLocalization.format("import_flow.seed_phrase_unknown_words_format", words), .red, "exclamationmark.triangle.fill")
            }
            return (AppLocalization.format("import_flow.seed_phrase_off_list_format", languageName, words), .red, "exclamationmark.triangle.fill")
        }
        if let problem = verdict.problem { return (problem.localizedMessage, .red, "exclamationmark.triangle.fill") }
        if verdict.isValid, let languageName {
            return (AppLocalization.format("import_flow.seed_phrase_valid_format", languageName, Int(verdict.wordCount)), .green, "checkmark.circle.fill")
        }
        return (
            AppLocalization.format(
                "import_flow.seed_phrase_typing_format", languageName ?? "—", verdict.words.count, Int(verdict.wordCount)),
            .secondary, "ellipsis.circle"
        )
    }
}

/// One numbered word slot, editable or shown.
struct SeedPhraseWordCell<Content: View>: View {
    let index: Int
    var isHighlighted = false
    var isInvalid = false
    @ViewBuilder let content: Content
    @ScaledMetric(relativeTo: .caption2) private var indexWidth: CGFloat = 16

    var body: some View {
        let accentColor: Color = isInvalid ? Color.red.opacity(0.85) : Color.accentColor.opacity(0.7)
        HStack(spacing: SpectraLayout.Space.xs) {
            Text("\(index + 1)").font(.caption2.weight(.bold)).foregroundStyle(.tertiary)
                .frame(width: indexWidth, alignment: .trailing).monospacedDigit()
            content.frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, minHeight: 36)
        .padding(.horizontal, SpectraLayout.Space.s).padding(.vertical, SpectraLayout.Space.xs)
        .spectraInsetFill(cornerRadius: SpectraLayout.Radius.control)
        .overlay(RoundedRectangle(cornerRadius: SpectraLayout.Radius.control, style: .continuous)
            .stroke((isHighlighted || isInvalid) ? accentColor : Color.clear, lineWidth: 1))
        .animation(.easeInOut(duration: 0.15), value: isHighlighted)
    }
}
