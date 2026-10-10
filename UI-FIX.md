# UI fix checklist

From the 2026-10-09 UI review: the app driven in the iPhone 17 Pro and
iPhone 17e simulators, plus a read of every view against
[docs/IOS-UI.md](docs/IOS-UI.md). Each item is ticked when its change is in
the tree; behaviour changes are recorded in
[docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md).

## 1. Funds safety and blocking failures

- [x] **1. Unreadable store.** A database core cannot decode leaves the app
  half-working: an empty Home, a generic "couldn't read" error on every
  action, and a Reset that cannot run because it needs the same database.
  Core reports an incompatible store as its own error, the schema version is
  bumped when a stored shape changes, a reset works without a bound database,
  and the app opens on a blocking "reset" screen instead of an empty Home.
- [x] **2. The fee where the user signs.** `SendArtifactReview` carries the
  network fee; the check-and-sign page and the sign alert show it in crypto
  and fiat.
- [x] **3. Failed fee quote.** Build stays disabled with a reason, the fee row
  says "unavailable" with Retry instead of "Estimating…" forever, and the
  error sits near the field it concerns rather than at the bottom.
- [x] **4. Amount over the balance** is refused inline under the field and
  Review is disabled with that reason.
- [x] **5. Send flow back navigation.** System back and the edge swipe step
  back one page instead of leaving the flow and wiping the draft.
- [x] **6. Speed Up / Cancel in the composer** are removed; the transaction
  page keeps them.
- [x] **7. Hide balances** masks every amount and fiat figure (Home rows,
  asset detail, wallet holdings, History, transaction detail), the mask reads
  "Hidden" to VoiceOver, and Home has a quick toggle.
- [x] **8. No zero before the first balance pass.** Home shows a loading state
  rather than "$0.00 / 0 ETH" until balances arrive.
- [x] **9. Receive to a watch-only wallet** warns that Spectra holds no keys
  for that address.
- [x] **10. New-wallet passphrase** has a reveal toggle and a confirmation
  field.
- [x] **11. Reset** starts with no category selected and always confirms.
- [x] **12. Seed phrase protection.** The new phrase is hidden until revealed,
  Copy is removed, secrets blur while the screen is captured, the inert
  `.privacySensitive()` calls are made real or removed, and word fields keep
  keyboard suggestions off.

## 2. Visual and layout

- [x] **13. Addresses never take a layout hyphen** (wallet address card, watch
  input, anywhere an address wraps), and EVM addresses display in their
  checksummed form.
- [x] **14. Wallet action bar** fits a 390pt screen without breaking words.
- [x] **15. Flow bottom bar** is not an opaque slab over the content, and flows
  hide the tab bar.
- [x] **16. Duplicate copy.** Setup pages stop repeating their subtitle in the
  card, and pages stop naming themselves three times.
- [x] **17. Name Your Wallet** shows the name field first, and its placeholder
  is the name the wallet will actually get.
- [x] **18. Watch Multisig** row icon sits on the same backplate as its
  siblings.
- [x] **19. Home asset list.** Default pins do not show assets the user has no
  wallet for, the pin marker is not red, and the asset and wallet counts agree.
- [x] **20. History rows** lead with the transaction kind, a zero-value
  contract call does not read "-0 ETH", the status pill shows only when not
  confirmed, the time drops seconds and the date the section header already
  gives, and section counts are not loaded-row counts.
- [x] **21. Asset detail** reads live data, has Send/Receive and the price, and
  folds the separate Details page in.
- [x] **22. Settings** is glass rows over the backdrop like the other tabs,
  regrouped, with the destructive row red and consistent casing.
- [x] **23. Receive page** has one exit, a legible grouped address, a warning
  icon on the network notice, one Share action that carries the address text.
- [x] **24. Small screens and dust.** Asset names do not truncate on a 390pt
  screen, and Home can hide dust balances.

## 3. Interaction and flow

- [x] **25. Setup flow order and progress.** Record → verify → name and
  password on one page, with a step indicator.
- [x] **26. Create page** shows the words, the warning and one Advanced entry;
  length, account and path move behind it.
- [x] **27. Add Wallet page** stops leading with Find Lost Funds and the
  test-network switch; the chain-first model is decided and recorded.
- [x] **28. Seed entry.** The word being typed is not marked invalid, the
  wordlist suggests, pasted numbered or comma lists are cleaned, Clear
  confirms, and each field has an accessibility label.
- [x] **29. Recipient validation** does not flash on every keystroke, shows
  core's reason, offers Retry only for network failures, and shows
  destination notes only for a valid address.
- [x] **30. Scanned payment URIs** keep their amount and memo and say so.
- [x] **31. After a broadcast** the primary action is Done; the explorer link
  is secondary.
- [x] **32. History loading and filters.** No false "No matches" while loading
  or typing, the filter icon shows an active filter, and Recheck/Rebroadcast
  are on the transaction page with their result shown.
- [x] **33. Funds Finder** keeps its results when an account is imported from
  it.
- [x] **34. Endpoints.** "Use only my endpoints" is in Settings per network and
  custom endpoints can be removed.
- [x] **35. Staking** asks for the password once, keeps it on failure, shows
  errors where they happen, and names the validator.
- [x] **36. Tor** kill switch defaults on with Tor, and the page claims only
  what it does.
- [x] **37. Biometry** is named for the device (Face ID, Touch ID, Optic ID,
  passcode), and the controls that depend on it are disabled when it is off.

## 4. Consistency, accessibility, localization

- [x] **38. One list per card.** The address book draws rows in one card, and
  glass-on-glass (Add to Another Network, Price Alerts) is removed.
- [x] **39. Wallet tool pages** share one style.
- [x] **40. Empty states** use one component per context.
- [x] **41. Home and History** use `SpectraRowGroup`: no interactive whole-card
  glass, and the whole row is the tap target.
- [x] **42. Accessibility.** Labels on the QR code and icon-only buttons,
  status never by colour alone, no truncation of values at large text sizes,
  expanded state reported as such.
- [x] **43. Localization.** Dates follow the app locale, sentences are format
  keys rather than concatenations, and no literal English reaches the screen.
- [x] **44. Tab bar** uses the `Tab` API, minimizes on scroll, and badges
  History with pending transactions.

## 5. Findings the first pass dropped

Checked again against the three review reports the list above was drawn from.

- [x] **45. Tor** says only what it does: explorer links open in Safari, outside
  Tor.
- [x] **46. Seed phrase password** prompt describes the order it is asked in.
- [x] **47. Monero sync** shows a failure to read its status, its progress in
  blocks, and padded fields.
- [x] **48. Verify Message** is titled for what it does.
- [x] **49. Donations** copies the address it says a tap copies.
- [x] **50. Logs** asks before clearing.
- [x] **51. Wallet page** pull to refresh lasts as long as the refresh.
- [x] **52. Speed Up / Cancel** from History stays in History.
- [x] **53. Review copy** tells quoting from building and does not ask for an
  amount already entered.
- [x] **54. Saving a contact** asks for its name, shows core's refusal, and
  keeps the form until core answers.
- [x] **55. Staking numbers**: commission to its precision, the validator list
  loading, amounts in the app locale, and shortcuts written in the field's
  decimal separator.
- [x] **56. Monero payment proof** can be read in full and its Copied state
  resets.
- [x] **57. Asset places**: a chevron only on links, contracts copyable and
  read by VoiceOver.
- [x] **58. Notices**: a 9+ cap, a colour by severity, and an action per notice.
- [x] **59. Portfolio hero** says when wallets are left out, and its link says
  where it goes.
- [x] **60. Send stages** never by colour alone; the pill radius and the
  primary glyph follow the tokens.
- [x] **61. EVM fee and nonce fields** keep their labels and units once filled.
- [x] **62. Send From page** tells networks apart and marks the choice with a
  trailing checkmark in one card.
- [x] **63. Send result**: the hash copyable without a long press, errors in red
  with a symbol.
- [x] **64. Staking** shows the balance, the fee in fiat, the address
  grouped, and each submission's outcome. No Max: what a stake must leave for
  its fee and reserve differs per network and core does not compute it, so a
  guessed Max could sign a transaction that fails on chain.
- [x] **65. Wallet tool sheets** close with Cancel until something is done, in
  one place, and their address fields paste and scan.
- [x] **66. Settings pages**: currency picked inline, Report a Problem a direct
  link, catalog pages with inline titles.
- [x] **67. Address book**: screen padding, validity with a symbol, one title,
  Send to a contact, and paste or scan into the address.
- [x] **68. Wallet page** "Advanced" is named for what it holds.
- [x] **69. Funds Finder** says what the providers it asks will see.
- [x] **70. Screen padding** on About and Funds Finder through the shared inset.
- [x] **71. Receive** can request an amount (new).
- [x] **72. Funds Finder** can import every found account at once (new).
- [x] **73. Custom endpoints** can be edited (new).

## 6. Found importing a test wallet

Driving the import of a funded Ethereum Sepolia wallet in the simulator.

- [x] **74. Chain search** finds a test network by name with the test-network
  switch off, rather than "No Results" for a network the list has.
- [x] **75. Seed entry keeps every word typed quickly.** Words that land in one
  slot before focus moves each take their own slot, and the cursor follows the
  last of them, so the next key no longer overwrites the second.
- [x] **76. Seed suggestion bar** holds its height while a slot is being typed
  in, so the grid no longer jumps under the cursor with each keystroke.
- [x] **77. Reset Wallet** clears history records this build cannot read,
  rather than failing on them with "Spectra received data it couldn't read"
  after the wallets were already removed.

## 7. The send flow with a funded wallet

Driving a 0.001 tETH send on Ethereum Sepolia to the review and build pages,
never signing.

- [x] **78. Fee overrides being typed** do not request a quote: an empty nonce
  or fee showed "Something went wrong" twice and an unavailable fee.
- [x] **79. Hex in transaction details** breaks without hyphenation, so a digest
  never shows a character it does not have.
- [x] **80. The action bar over the keyboard** no longer covers the page: the
  recipient warning, the available amount and the seed grid stay readable.
- [x] **81. Paste** matches Scan and Contacts on the recipient page.
- [x] **82. The amount** sits with its unit.
- [x] **83. No "≈ —"** for an asset without a price.
- [x] **84. Fee and advanced settings** is as wide as the cards around it, with
  no plate behind it.
- [x] **85. Network fee** reads the same on review and on check-and-sign.
- [x] **86. Stage checkmarks** share the send steps' accent.
- [x] **87. Transfer parties**: the wallet's own icon and even spacing.
- [x] **88. Review notes** carry an icon that says check, not scan.
- [x] **89. Asset page headers** in one style.
- [x] **90. Total** (amount plus fee, when both are the same coin) on review and
  check-and-sign.
- [x] **91. A new destination** is pointed out on review, before building.
- [x] **92. Transaction details** as labelled rows rather than raw JSON.
- [x] **93. Nonce and fee fields** start from the estimate, with placeholders
  that are not their labels.
- [x] **94. Back from check-and-sign** returns to review.
- [x] **95. The amount page's balance card** keeps its distance from the
  action bar.
- [x] **96. Unpriced assets on Home** show one dash, and the portfolio says what
  it leaves out.
- [x] **97. Check and Sign** in title case.
- [x] **98. Send page** no longer fails with "Spectra received data it couldn't
  read" when a stored send from another build is in the resume list.
