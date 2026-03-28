# Spectra

Spectra is intended to be the Swiss Army knife of crypto wallets: powerful enough for serious users, but direct and approachable for everyday use.

It is built around a simple idea: a wallet should first be a wallet. It should help people hold assets safely, send payments quickly, recover access to legacy funds when possible, and understand what the software is doing under the hood. It should not bury core money movement behind hype features, noisy finance gimmicks, or "get rich quick" product clutter.

## Ethos

- Professional without being hostile. Spectra should expose meaningful diagnostics, logs, and network details for advanced users, while still feeling clean and usable for people who just want to hold and send crypto.
- Fast and practical. The app is designed for holding assets and making swift transactions, which is what crypto wallets were originally meant to do well.
- Minimal and lightweight. Spectra avoids bloated surfaces and unnecessary moving parts so the product stays understandable, responsive, and easier to trust.
- Privacy-first and decentralized. The wallet is built as open source local-first software, with a strong bias toward user control, low data exposure, and minimal trust assumptions.
- Safer by staying focused. Spectra deliberately avoids risky product sprawl such as trading, swap funnels, credit-card purchase flows, and similar integrations that expand attack surface and create more paths for scams or theft.
- Customizable and inspectable. Users should be able to adapt the wallet to their workflow and inspect diagnostics when something goes wrong instead of being locked into a black box.
- Broad compatibility. Spectra aims to support legacy wallet conventions, recovery patterns, and backward-compatible import paths wherever practical, including cases where that helps users recover old or stranded funds.

## What Spectra Is

Spectra is meant to be:

- a self-custodial wallet for holding crypto securely
- a practical payment tool for quick transfers
- a staking-capable wallet where chain support makes sense
- a recovery-oriented wallet with strong compatibility across older formats and wallet conventions
- a transparent tool that exposes diagnostics and logs instead of hiding operational detail

## What Spectra Is Not

Spectra is not trying to be:

- a trading terminal
- a DeFi casino
- a swap-first funnel
- a speculative "financial entertainment" app
- a platform built around credit-card on-ramp upsells and other high-risk surface area

The goal is restraint. Fewer gimmicks means fewer distractions, fewer attack paths, and a clearer safety model.

## Product Direction

Features that fit this ethos well:

- Multi-chain wallet support with consistent send, receive, history, and backup flows
- Broad wallet import and recovery support, including legacy derivation and compatibility modes
- Strong watch-only support for users who want visibility without exposing signing keys
- Rich diagnostics pages with endpoint health, history-source confidence, provider status, and exportable logs
- Local-first security features such as biometric protection, optional wallet passwording, and careful secret storage
- Clear transaction verification states after broadcast so users can tell whether a send is pending, indexed, or failed
- Customizable dashboards, pinned assets, visibility controls, and advanced network settings
- Flexible endpoint and provider configuration for users who want to bring their own infrastructure
- Simple staking surfaces for supported chains without turning the wallet into a speculative product maze
- Lightweight live operational tooling for support, troubleshooting, and self-serve debugging
- Plain-text content and registries where appropriate to improve portability across platforms
- Open-source development with readable architecture and low-obfuscation behavior

## Security Philosophy

Spectra reduces risk by reducing unnecessary complexity.

Every added integration can create new trust assumptions, new exploit surface, and new ways for users to be tricked. By staying focused on custody, payments, staking, diagnostics, and recovery, the app can remain safer and easier to reason about than feature-heavy wallets chasing every trend.

## Open Source

Spectra is intended to be truly open source, privacy-conscious, and decentralized in spirit. Users should be able to inspect how it works, understand the network paths it uses, and keep control over their own keys and wallet data.

## Support

If you find the project useful, please consider donating.

More detail on the philosophy, design decisions, and long-form explanations belongs on the project website.
