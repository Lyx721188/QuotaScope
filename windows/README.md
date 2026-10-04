# QuotaScope for Windows

当前主线的能力与验收边界见 [上游移植状态](../Docs/upstream-implementation-status.md)，
发布前任务与后续顺序见 [验收计划](../Docs/release-acceptance-2026-10-04.md)。

Current Windows implementation and limitations:
[usage guide](../Docs/windows-1.2.md), [all 77 provider routes](../Docs/providers/windows-ports.md),
and [extension contract](../Docs/extensions.md). The main branch can contain
unreleased changes; consult [CHANGELOG](../CHANGELOG.md) for release availability.

A native Windows application for [QuotaScope](../README.md) — the screen-edge monitor
for your AI coding allowances — written in Rust against the Win32 / Direct2D
APIs, styled after WinUI. A Mica dock floats beside a screen edge; each
accent-coloured ring is a limit (the providers' own Lobe icons in the
middle), hover for the detail card, and everything refreshes on an adaptive
ladder so your status line and the panel agree.

The reading, caching, alerting and reporting logic lives in `quotascope-core` and
follows the central promise that **QuotaScope does not invent percentages**. Where a provider says how much of an
allowance is left but never how large it is, the denominator is either
inferred (and labelled `estimated`) or the ring is not drawn at all.

## Build

Build the full workspace on Windows with Rust stable and the MSVC toolchain.
The platform-independent core can be checked separately. The release build runs on
GitHub Actions ([`.github/workflows/windows.yml`](../.github/workflows/windows.yml))
and uploads `quotascope.exe` as an artifact on every push.

```bash
cd windows
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo build --release --locked # target/release/quotascope.exe
```

## Run

```bash
quotascope.exe            # the panel, the tray icon, the settings window
quotascope.exe --json     # print the cached rail for status lines and scripts
```

### `--json`

The status-line contract from
[`Docs/json-output.md`](../Docs/json-output.md) is kept verbatim: it prints
what the running app last banked and how old it is, **never fetches**, and
never writes. Window names are not localized in it; `kind` is a flat token
(`fiveHour`, `weekly`, `spend`, `monthly`, `balance`, `other:<seconds>`) and
`usedPercent` carries the display rule, so a script agrees with the ring.

```bash
quotascope.exe --json | jq -r '.accounts[] | "\(.name) \(.headline.usedPercent // "–")%"'
```

## What is ported

All 77 catalog providers have registered Windows routes. Depending on the
provider, a route uses an API key, an explicitly imported browser session,
a local client or a logged-in CLI. Implementation and parser fixtures do
not imply that every provider has been verified with a real account.
The [Windows route table](../Docs/providers/windows-ports.md) is the support reference.

The Token page catalogs 54 sources: 34 read a known native or export-cache
format, and 20 currently accept only the documented JSONL import format.
DSH supports real, bounded Zstandard decoding; its incomplete-record flag
remains visible in the source list and filtered totals.

Main also includes cancellable background Token aggregation,
[DeepSeek console history](../Docs/providers/windows-deepseek-console.md),
and [conservative Codex local clues](../Docs/providers/windows-codex-signals.md).
The remaining native formats, interactive login for added accounts,
full BotMark animation and some Windows-specific translations are tracked
in the [implementation status](../Docs/upstream-implementation-status.md).

## Architecture

Two crates:

- **`quotascope-core`** — everything without a window: the provider model, the
  provider routes, DPAPI-encrypted key storage (the Keychain's
  counterpart here), the reading cache with its reconcile rules, adaptive
  refresh pacing, alerts, local Token readers, localization and the `--json`
  report. English, 简体中文, 繁體中文, Japanese and Korean dictionaries are
  included; some Windows-specific strings still fall back to English.
- **`quotascope-win`** — the Win32 surface. The dock and the detail flyout are
  two `WS_EX_NOREDIRECTIONBITMAP` windows whose frames are composed by DWM:
  **real Mica** (`DWMWA_SYSTEMBACKDROP_TYPE`), Windows' own corner radius and
  border, dark/light following the system. Rendering is a flip-model
  `CreateSwapChainForComposition` swap chain handed to DWM through a
  DirectComposition visual — the only route where the premultiplied alpha
  **and** the system backdrop both survive — drawn with Direct2D. The
  settings window is WinUI 3 via Microsoft's [Windows
  Reactor](https://github.com/microsoft/windows-rs) (a
  `windows-reactor` component served on the main thread), while the panel,
  tray and notifications are classic Win32 on a worker thread. Provider
  marks are Lobe-derived assets, rasterised once to transparent PNGs and
  tinted at load into the theme's ink. Motion uses damped springs:
  the arrival, the hover halo, the arcs and the card's slide between rings
  are all damped springs — SwiftUI's `.spring(response:…)`,
  `dampingFraction:…)` integrated in fixed steps. Windows only.

Design tokens follow WinUI (`theme.rs`): the ink sits directly on Mica, in
one dark and one light voice matched to the system appearance; ring arcs,
progress bars and the hover glow use the **system accent colour**. The theme
and accent are re-read whenever Windows announces a change, and the icon
cache re-tints with them. Credentials are stored with `CryptProtectData`
under the current user, so they do not survive `roaming` to another machine
— by design.

## Keeping the port honest

The tests in `quotascope-core/tests/ported.rs` cover the percent display rule
(nothing used reads 0%, not quite full
never reads 100%, and a countdown gets the same rule at both ends), which
limit the second ring picks (fullest of the rest, inside the reading's own
scope group), the GLM Coding Plan refusals (every one of them an HTTP 200,
answered in Chinese and English), and DeepSeek's balance-only windows (no
length, no reset, ever — balance is not a limit).
