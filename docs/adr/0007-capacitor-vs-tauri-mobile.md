# ADR 0007: Capacitor for the mobile apps (not Tauri 2 mobile)

**Status:** accepted

| | Capacitor 8 | Tauri 2 mobile |
|---|---|---|
| What it is | Thin native shell around a WebView loading bundled web files | The same idea, with a Rust core process |
| Maturity on mobile | Years in production; large plugin set | Stable since Tauri 2.0, but the mobile plugin ecosystem is younger |
| Camera in the WebView (`getUserMedia`) | Supported on both platforms; the bridge answers WebView permission requests | Possible, but needs more manual native configuration |
| Plugins we need (haptics, share, filesystem, push, deep links, splash) | All first-party | Partly first-party, partly community |
| Build in CI | Gradle + Xcode, well-trodden; no Rust cross-compilation | Needs the Android NDK and iOS Rust targets; longer, more fragile builds |
| Fit with this codebase | The web app is already WASM; a second Rust runtime adds nothing | A Rust core is attractive when logic lives natively; ours lives in the WebView and on the server |
| App size | Small (WebView + assets) | Similar, plus the Rust core |

## Decision
Capacitor. The app is the web app; what it needs from native is
permissions, haptics, sharing, deep links and store packaging, which
Capacitor provides with the least machinery. Tauri would be reconsidered
if substantial native-side Rust logic appeared (for example running the
pose model natively).

## Consequences
- A small Node toolchain exists only under `mobile/` for the Capacitor CLI.
- The native projects (`mobile/android`, `mobile/ios`) are committed, as
  Capacitor recommends, so permissions and signing config are reviewable.
- iOS uses Swift Package Manager (no CocoaPods).
- To lower the risk of an App Store "minimum functionality" (4.2)
  rejection the apps add native permission prompts, haptics, the native
  share sheet, deep links, an offline shell and push groundwork. Approval
  is still Apple's call and is not guaranteed.
