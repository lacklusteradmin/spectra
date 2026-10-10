# iOS UI reference

Spectra uses the current iOS design system: system typography, rich color and
Liquid Glass. Liquid Glass is how iOS looks, not a version-gated opt-in, so this
document names no iOS version — the deployment target lives in the Xcode project.

This document is the source of truth for Spectra's iOS UI rules.

## Product decision

Apple describes Liquid Glass as a distinct functional layer for controls and navigation and advises against using it in the content layer. Spectra intentionally departs from that guidance by placing important body content on glass cards over `SpectraBackdrop`.

That departure is deliberate, not an interpretation of Apple's recommendation. Keep the hierarchy legible, avoid stacking glass on glass, and limit custom glass effects when a standard system control already provides the correct behavior.

Official references:

- [Adopting Liquid Glass](https://developer.apple.com/documentation/TechnologyOverviews/adopting-liquid-glass)
- [Applying Liquid Glass to custom views](https://developer.apple.com/documentation/SwiftUI/Applying-Liquid-Glass-to-custom-views)
- [Human Interface Guidelines: Materials](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Build a SwiftUI app with the new design](https://developer.apple.com/videos/play/wwdc2025/323/)

## Design baseline

- **Backdrop:** Keep `SpectraBackdrop` behind every top-level tab. Re-add it at main business detail roots such as asset, wallet, staking, and receive destinations. Settings-style utility details may continue to use system `Form` layouts. Its ground is neutral and its clouds are faint — the accent and one cool counterweight — because a saturated or many-hued wash tints every card above it, pulls secondary text toward its hue and competes with the accent. The clouds drift slowly and must stay slow: motion that draws the eye competes with the content on the glass above it. The backdrop holds still under Reduce Motion and in Low Power Mode.
- **Top-level tabs:** Use `ScrollView` plus glass cards with internal dividers. Do not use `List(.insetGrouped)` or `Form` for a main tab.
- **Chrome:** Hide the navigation bar background with `.toolbarBackground(.hidden, for: .navigationBar)` when content should scroll beneath it.
- **Toolbar actions:** Use standard `ToolbarItem` buttons and menus. The system places toolbar items on Liquid Glass automatically, so do not add `.buttonStyle(.glass)` inside a toolbar.
- **Cards:** Use a subtle white glass tint, and only these two steps.
  `SpectraLayout.GlassTint.elevated` (`0.04`) covers hero and header cards.
  `SpectraLayout.GlassTint.content` (`0.03`) covers ordinary content cards.
  Accent-tinted glass — an orange or red notice — carries its own colour and
  is not one of these steps.
- **Glass stops at the card:** nothing inside a card is glass. An input, an
  address block, a chip, a selectable tile or an icon backplate inside a card
  takes the flat `SpectraLayout.insetFill` (`spectraInsetFill`), which recesses
  into the card instead of stacking a second layer of glass on it. Buttons are
  controls, not surfaces, and keep their glass styles.
- **Lists:** A list is rows in one card — `SpectraRowGroup`, with each row's
  label padded by `spectraRowPadding()` — never a card per row. A card of cards
  is glass on glass, and a column of cards spends a card's padding and gap on
  every row. A selectable list marks the choice with a trailing checkmark.
  The group's header is a title or a view of the caller's (Home's page
  switch); its footer sits inside the card, under the rows — a loading row,
  an empty state or an action on the whole list. Fixed rows rather than data,
  such as the settings tab's, are a `SpectraRowSection`: each subview a row,
  and an explanatory footer under the card.
- **Empty states:** `SpectraEmptyStateCard` between cards on a glass page;
  `SpectraEmptyStateContent`, the same words without a surface, inside a card
  or a `Form` row; `ContentUnavailableView` for an empty search or a whole
  empty screen. A wallet tool page's list says its states with
  `WalletToolLoadingSection`, `WalletToolErrorSection` and
  `WalletToolEmptySection`.
- **Flows:** a flow's primary action rides the bottom edge in
  `.safeAreaBar(edge: .bottom)` holding a `SpectraBottomActionBar`, which draws
  no background or divider of its own; the content scrolls under it and the
  bar rises with the keyboard. A flow hides the tab bar with
  `.toolbar(.hidden, for: .tabBar)`, and Back in a multi-step flow is a step
  back, never the whole flow.
- **Secrets:** a recovery phrase or a private key on screen sits behind
  `.secretShield()` and marks its text `.privacySensitive()`: blurred while
  the screen is captured, with a warning after a screenshot. A seed word is
  typed in a `SecretWordField`, which turns off every keyboard aid that reads
  or learns what is typed.
- **Balances:** every amount and value a person can hide goes through
  `BalanceText`, which masks it and tells VoiceOver it is hidden. Prices are
  not balances and are never masked.
- **Addresses:** a recipient field is an `AddressEntryRow` — typed, pasted,
  scanned (read by core against the network) or picked from the contacts —
  on every page that asks for one. A tool sheet that sends closes with
  `sendSheetDismissal`: Cancel until a node has accepted, then Done.
- **Disclosures:** every `DisclosureGroup` takes `.disclosureGroupStyle(.spectra)`:
  the label with a turning chevron, and what it reveals at full width. The
  system style insets the content and lays a square plate behind a card.
- **Keyboard:** a page whose action bar rides on the keyboard marks the card
  holding its field `.keyboardAnchor()` and its scroll content
  `.scrollsToKeyboardAnchor(proxy)`, so the bar never covers what sits under
  the field being typed in.
- **Hashes and payloads:** shown with `breakableAnywhere`, so a wrap never adds
  a hyphen that is not in the value, and copied whole from a `CopyButton`.
- **Dates and numbers:** format them in `AppLocalization.locale` —
  `Date.appFormatted(time:)`, or `.locale(AppLocalization.locale)` on a
  format style — never the system locale, which need not be the language of
  the words around them.
- **Glass variant:** Always `.regular`. Apple's other variant, `.clear`, is for
  components floating over photos or video, and over bright content it needs a
  35% dark dimming layer beneath it to stay legible. Spectra has no media
  background — `SpectraBackdrop` is the app's own gradient — so `.clear` appears
  nowhere, and a surface that looks like it wants one wants a tint step instead.
- **Buttons:** Use `.buttonStyle(.glass)` and `.buttonStyle(.glassProminent)` for stand-alone actions outside toolbars. Prominent buttons take the accent on their own; a plain `.glass` button that should read as primary adds `.tint(.accentColor)`. Tint destructive actions red.
- **Typography:** Use system text styles such as `.largeTitle.weight(.bold)`, `.title`, `.headline`, and `.body`.
- **Text colors:** Use semantic styles such as `.primary`, `.secondary`, `.tertiary`, and `.quaternary`. Do not use `Color.primary.opacity(...)` for text.
- **Theme colour:** one accent, the asset catalog's `AccentColor` (system
  orange), set as the target's global accent. Tab bar, links, prominent
  buttons, list icons and selection read it on their own; switches get it from
  the root toggle style in `SpectraApp`, not `UISwitch.appearance()`, which
  SwiftUI overrides when a toggle re-renders. Views write `.tint` or
  `Color.accentColor`, never a literal orange, so the theme colour can change
  in one place. Toolbar glyphs stay monochrome. Decorative icons are the
  accent, not a per-row colour. The other colours are semantic and do not
  follow the theme: green for success, confirmed and received;
  `.spectraWarning` (orange) for pending, incomplete and warnings; red for
  failures, sends and destructive actions. `make check-ui` rejects a literal
  orange in a view; artwork with a fixed palette of its own ends the line with
  `design-tokens: artwork`.
- **Artwork exception:** Decorative icon artwork, including `SpectraLogo`, may use custom fonts, color opacity, and glass effects.

## Spacing

Every padding, stack spacing and spacer minimum is a step on one scale,
`SpectraLayout.Space`:

| Token | Value | Usage |
| --- | --- | --- |
| `xxs` | 2pt | Between the two lines of one text pair |
| `xs` | 4pt | Tight label/value gaps, pill vertical padding |
| `s` | 8pt | Icon-to-label gaps, row vertical padding, pill horizontal padding |
| `m` | 12pt | Between cards, between groups inside a card, row badge gap |
| `l` | 16pt | Card padding, screen horizontal inset |
| `xl` | 24pt | Large artwork insets |

The named values are steps too: `screenHorizontal` 16, `screenTop` 8,
`screenBottom` 16, `sectionSpacing` 12, `cardPadding` 16, `rowVertical` 8.
Every scrolling page — top-level tab, detail or flow — insets its content with
`spectraScreenPadding()`. `spacing: 0` is the absence of spacing and stays
literal; a frame, offset or icon size is a component's geometry, not spacing.

## Corner radii

The radius communicates hierarchy, in three steps:

| Token | Radius | Usage |
| --- | --- | --- |
| `Radius.card` | 20pt | Every card — tab, hero, detail, list and notice banner — and the only glass radius: `spectraCardFill`, `spectraElevatedFill`, `spectraDetailCard` |
| `Radius.inner` | 14pt | Surfaces inside a card: inputs, address blocks, tiles, chips and pills; `spectraInsetFill` |
| `Radius.control` | 10pt | Dense controls and single-character slots inside a card |
| size-relative | — | Icon artwork such as the `SpectraLogo` backing |

Write the token, never the number. A component's own internal geometry is not a
step on this scale — `SpectraShimmer` rounds a 12–14pt placeholder bar by 6pt,
which it owns — but a surface in the hierarchy always is.

The scale is fixed, not derived. Apple offers `ConcentricRectangle` and
`rect(corners:isUniform:)` so a nested shape can follow its container's
curvature, which is the right tool when a surface must hug a corner it did not
choose — the device's, or a sheet's. Spectra's cards sit on a flat backdrop and
choose their own corner, so they take a step from the table above instead, and a
card reads the same wherever it is nested. A concentric shape is an exception a
view argues for in a comment, never a substitute for a step.

`scripts/check-design-tokens.sh` (also `make check-ui`) fails when a view
restates a spacing, a radius or a white glass tint that `SpectraLayout` owns,
or draws glass in a shape only a surface inside a card has — `Radius.inner`,
`Radius.control`, a circle or a capsule. `spectraCardFill` and
`spectraElevatedFill` take no radius for the same reason.

Shared values and helpers live in:

- [`SpectraLayout`, `spectraCardFill`, `spectraElevatedFill`, `spectraInsetFill` and `spectraRowPadding`](../swift/views/SpectraLayout.swift)
- [`SpectraRowGroup` and `SpectraRowSection`](../swift/views/SpectraRowGroup.swift)
- [`SpectraEmptyStateCard` and `SpectraEmptyStateContent`](../swift/views/ViewExtensions.swift)
- [`AddressEntryRow`](../swift/views/AddressEntryRow.swift)
- [`SpectraDisclosureStyle`](../swift/views/SpectraDisclosureStyle.swift) and
  [`keyboardAnchor`](../swift/views/KeyboardAnchor.swift)
- [`SendCostRows` and `SendSummaryRow`](../swift/views/SendCostRows.swift)
- [`BalanceText`](../swift/views/BalanceText.swift), [`CopyButton`](../swift/views/CopyButton.swift),
  [`secretShield`](../swift/views/SecretShield.swift) and
  [`SecretWordField`](../swift/views/SecretWordField.swift)
- [`spectraInputFieldStyle` and `spectraDetailCard`](../swift/views/ViewExtensions.swift)

## Examples in the app

- [DashboardViews.swift](../swift/views/DashboardViews.swift): top-level tab,
  hero cards and asset detail rows.
- [ReceiveFlowViews.swift](../swift/views/ReceiveFlowViews.swift): business detail layout.
- [ChainWikiViews.swift](../swift/views/ChainWikiViews.swift): compact interactive cards.
- [HistoryView.swift](../swift/views/HistoryView.swift) and
  [AddWalletEntryView.swift](../swift/views/AddWalletEntryView.swift): lists as
  `SpectraRowGroup` rows.
- [SettingsViews.swift](../swift/views/SettingsViews.swift): a top-level tab
  of `SpectraRowSection` cards.
- [SendFlowViews.swift](../swift/views/SendFlowViews.swift): a flow with its
  action bar in `safeAreaBar`.

Pages use `spectraScreenPadding()` with `sectionSpacing` between cards, and
card content has `cardPadding`. Detail navigation titles are inline.

Detail key/value rows use an accent SF Symbol, a secondary label and a primary
value, `Space.m` row spacing and dividers at 0.4 opacity. Group adjacent glass
actions with `GlassEffectContainer(spacing: SpectraLayout.Space.s)`.

## Additional constraints

- Rounded black display typography is reserved for icon artwork.
- Avoid custom glass on every small element or stacked glass surfaces.
- Do not add `.ultraThinMaterial` or `.thinMaterial` where `.glassEffect` is appropriate.

## Numeric presentation boundary

Compact asset amounts share core's six-significant-digit policy, capped at
both the asset's supported precision and eight fractional places. This is a
cross-platform display policy, not transaction rounding or a signing constraint.
Detailed amounts may show the supported precision. Native formatters own locale,
separators and trailing zeros; core continues to validate exact typed amounts.

Core supplies portfolio/wallet valuation and completeness. A missing quote or
currency rate is unavailable, displayed as “—”. A total that leaves an unpriced
holding out shows its figure alone, since that holding's own row already shows
“—”, and a total no balance read has reached yet shows as loading. Never
replace an unavailable value with zero.
