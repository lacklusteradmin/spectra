import SwiftUI

/// The seed-phrase entry grid: one field per slot, as many slots as core
/// judges the phrase at, and what core made of it underneath. The import flow
/// and the funds finder both show this, over the same `SeedPhraseEntry`.
struct SeedPhraseEntryView: View {
    let entry: SeedPhraseEntry
    @FocusState private var focusedSlot: Int?

    private static let columns = Array(repeating: GridItem(.flexible(), spacing: SpectraLayout.Space.xs), count: 3)

    var body: some View {
        let verdict = entry.verdict
        let invalidWords = Set(verdict.invalidWords)
        let status = Self.status(of: verdict)
        VStack(alignment: .leading, spacing: SpectraLayout.Space.m) {
            header(verdict)
            LazyVGrid(columns: Self.columns, spacing: SpectraLayout.Space.xs) {
                ForEach(entry.slots.indices, id: \.self) { index in
                    let isInvalid = invalidWords.contains(entry.slot(at: index))
                    SeedPhraseWordCell(index: index, isHighlighted: focusedSlot == index, isInvalid: isInvalid) {
                        // The default keyboard: a Japanese, Korean or Chinese
                        // wordlist cannot be typed on an ASCII one.
                        TextField("", text: binding(for: index)).textInputAutocapitalization(.never).autocorrectionDisabled()
                            .font(.system(.callout, design: .monospaced).weight(.medium))
                            .foregroundStyle(isInvalid ? AnyShapeStyle(.red.opacity(0.95)) : AnyShapeStyle(.primary))
                            .focused($focusedSlot, equals: index)
                    }
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: SpectraLayout.Space.s) {
                Text(status.text).font(.footnote).foregroundStyle(status.color)
                Spacer(minLength: SpectraLayout.Space.s)
                if entry.wordCountOverride == nil, entry.nextSlotCount != nil {
                    Button(AppLocalization.string("More words"), systemImage: "plus") { entry.addSlots() }
                        .font(.footnote.weight(.semibold)).buttonStyle(.plain).foregroundStyle(.tint)
                }
            }
        }
        .privacySensitive()
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
                    entry.clear()
                    focusedSlot = 0
                }
                .labelStyle(.iconOnly).font(.title3).buttonStyle(.plain).foregroundStyle(.secondary)
            }
        }
    }

    private func binding(for index: Int) -> Binding<String> {
        Binding(
            get: { entry.slot(at: index) },
            set: { newValue in
                let shouldAdvance = newValue.last?.isWhitespace == true
                let trimmedValue = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
                entry.update(at: index, with: trimmedValue)
                guard shouldAdvance, !trimmedValue.isEmpty else { return }
                // A space in the last slot of an inferred length asks for more.
                if index + 1 == entry.slots.count, entry.wordCountOverride == nil { entry.addSlots() }
                focusedSlot = (index + 1) < entry.slots.count ? (index + 1) : nil
            }
        )
    }

    /// The line under the grid: what core read the entry as, or what is
    /// wrong with it.
    private static func status(of verdict: SeedPhraseVerdict) -> (text: String, color: Color) {
        let languageName = verdict.language.map { AppLocalization.string($0.name) }
        if verdict.words.isEmpty {
            return (AppLocalization.string("import_flow.seed_phrase_detect_hint"), .secondary)
        }
        if !verdict.invalidWords.isEmpty {
            let words = verdict.invalidWords.joined(separator: ", ")
            guard let languageName else {
                return (AppLocalization.format("import_flow.seed_phrase_unknown_words_format", words), .red)
            }
            return (AppLocalization.format("import_flow.seed_phrase_off_list_format", languageName, words), .red)
        }
        if let problem = verdict.problem { return (problem.localizedMessage, .red) }
        if verdict.isValid, let languageName {
            return (AppLocalization.format("import_flow.seed_phrase_valid_format", languageName, Int(verdict.wordCount)), .green)
        }
        return (
            AppLocalization.format(
                "import_flow.seed_phrase_typing_format", languageName ?? "—", verdict.words.count, Int(verdict.wordCount)),
            .secondary
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
