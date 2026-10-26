# Liquid

Planned for M3; see the [Liquid V-Spec](../docs/concept/V-Spec_Liquid.html). The agent-surface slice in [PLAN.md](../PLAN.md) comes first.

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `app/` — the Flutter app (ADR-0181). Its exact Flutter release is pinned in `app/pubspec.yaml` (ADR-0193).
- `liquid-bridge/` — the Rust side of the flutter_rust_bridge bindings (ADR-0186). It embeds the Meta-Harness core (ADR-0184); its package is `liquid_bridge` because cargokit derives the library name from it.

After changing `liquid-bridge/src/api`, run `flutter_rust_bridge_codegen generate` in `app/` and then `cargo fmt --all`. The generator's version is the `flutter_rust_bridge` version in `app/pubspec.yaml` (`cargo install --locked flutter_rust_bridge_codegen@<version>`). The generated files are committed without web bindings, and CI regenerates them to check they match.

## Run

The app starts pi for the project named in the environment and opens the start page, where recent sessions continue and the first message starts a new one (ADR-0203). `flutter run -d macos` in `app/` needs Xcode and CocoaPods (`brew install cocoapods`), because the Rust plugin builds through a podspec. Release builds are signed with Developer ID, hardened and notarized, and run without the App Sandbox (ADR-0200).

- `LIQUID_PROJECT` — the agent's project directory (required).
- `LIQUID_PI_EXTENSION` — the path of `meta-harness/pi-extension/watershed.ts` (required: pi would run tools without approvals, so Liquid does not start a session unless this extension is loaded).
- `LIQUID_PI` — the pi executable; defaults to `pi`.
- `LIQUID_PI_SESSION` — a stored pi session file to reopen, with its Trajectory and branches.

## Liquid key

On a Mac the first start asks whether Caps Lock becomes the Liquid key; Settings → Shortcuts switches profiles later (ADR-0205, [mockups](../docs/design/mockups/liquid-shortcuts.html)). Settings live in `~/Library/Application Support/Liquid/shortcuts.json`.

The key needs macOS 13 and is the login item `LiquidKey.app` inside the app bundle, built by `app/macos/LiquidKey/build.sh` during the Xcode build. While it runs, it maps Caps Lock to F18 on every keyboard, opens Liquid on a tap in another app and toggles Caps Lock on Shift + Caps Lock, without extra permissions. The mapping holds for the whole computer, so the helper gives Caps Lock back when it quits or the user switches away; switching to Standard does so at once. Turning the item off under System Settings → General → Login Items stops the key; at its next start Liquid reports it and continues without the Liquid key. A helper that quit, for example because another app took its key, starts again when Liquid starts or the Liquid key is turned on. `app/macos/LiquidKey/test.sh` runs its checks and compiles the helper.
