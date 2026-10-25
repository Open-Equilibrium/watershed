# Liquid

Planned for M3; see the [Liquid V-Spec](../docs/concept/V-Spec_Liquid.html). The agent-surface slice in [PLAN.md](../PLAN.md) comes first.

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `app/` — the Flutter app (ADR-0181). Its exact Flutter release is pinned in `app/pubspec.yaml` (ADR-0193).
- `liquid-bridge/` — the Rust side of the flutter_rust_bridge bindings (ADR-0186). It embeds the Meta-Harness core (ADR-0184); its package is `liquid_bridge` because cargokit derives the library name from it.

After changing `liquid-bridge/src/api`, run `flutter_rust_bridge_codegen generate` in `app/` and then `cargo fmt --all`. The generated files are committed.

## Run

The app starts pi for the project named in the environment and opens the start page, where recent sessions continue and the first message starts a new one (ADR-0203). `flutter run -d macos` in `app/` needs Xcode and CocoaPods (`brew install cocoapods`), because the Rust plugin builds through a podspec. Release builds are signed with Developer ID, hardened and notarized, and run without the App Sandbox (ADR-0200).

- `LIQUID_PROJECT` — the agent's project directory (required).
- `LIQUID_PI_EXTENSION` — the path of `meta-harness/pi-extension/watershed.ts` (required: without it pi would run tools without approvals).
- `LIQUID_PI` — the pi executable; defaults to `pi`.
- `LIQUID_PI_SESSION` — a stored pi session file to reopen, with its Trajectory and branches.

## Liquid key

On a Mac the first start asks whether Caps Lock becomes the Liquid key; Settings → Shortcuts switches profiles later (ADR-0205, [mockups](../docs/design/mockups/liquid-shortcuts.html)). Settings live in `~/Library/Application Support/Liquid/shortcuts.json`.

The key is the login item `LiquidKey.app` inside the app bundle, built by `app/macos/LiquidKey/build.sh` during the Xcode build. It maps Caps Lock to F18 on every keyboard until logout, opens Liquid on a tap in another app and toggles Caps Lock on Shift + Caps Lock, without extra permissions. Switching to Standard gives Caps Lock back at once; turning the item off under System Settings → General → Login Items does so from the next login. `app/macos/LiquidKey/test.sh` runs its checks.
