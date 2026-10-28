# Liquid

Planned for M3; see the [Liquid V-Spec](../docs/concept/V-Spec_Liquid.html). The agent-surface slice in [PLAN.md](../PLAN.md) comes first.

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `app/` — the Flutter app. Its exact Flutter release is pinned in `app/pubspec.yaml`, which CI reads.
- `liquid-bridge/` — the Rust side of the flutter_rust_bridge bindings (ADR-0186). It embeds the Meta-Harness core (ADR-0184), which never depends on the bridge; `app/hook/build.dart` compiles it into the app (ADR-0213).

After changing `liquid-bridge/src/api`, run `flutter_rust_bridge_codegen generate` in `app/` and then `cargo fmt --all`. The generator's version is the `flutter_rust_bridge` version in `app/pubspec.yaml` (`cargo install --locked flutter_rust_bridge_codegen@<version>`). The generated files are committed without web bindings, and CI regenerates them to check they match.

## Run

The app starts pi for the project named in the environment and opens the start page, where recent sessions continue and the first message starts a new one. It needs pi 1.3.0 with a configured model. `flutter run -d macos` in `app/` needs Xcode and rustup ([build hosts](../TESTING.md#development-toolchain)). The project signs development builds ad hoc; release builds are to be signed with Developer ID, hardened and notarized, and to run without the App Sandbox (ADR-0200).

- `LIQUID_PROJECT` — the agent's project directory (required).
- `LIQUID_PI_EXTENSION` — the absolute path of `meta-harness/pi-extension/watershed.ts` (required: pi would run tools without approvals, so Liquid does not start a session unless this extension is loaded).
- `LIQUID_PI` — the pi executable, a name looked up on `PATH` or an absolute path; defaults to `pi`.
- `LIQUID_PI_SESSION` — the absolute path of a stored pi session file to reopen, with its Trajectory and branches.

## Liquid key

On a Mac the first start asks whether Caps Lock becomes the Liquid key; Settings → Shortcuts switches profiles later ([mockups](../docs/design/mockups/liquid-shortcuts.html)). Windows and then Linux get the key after macOS. Settings live in `~/Library/Application Support/Liquid/`: `shortcuts.json` for the shortcuts and `diff.json` for how the Diff tab shows changes.

The key needs macOS 13 and is the login item `LiquidKey.app` inside the app bundle, built by `app/macos/LiquidKey/build.sh` during the Xcode build. While it runs, it maps Caps Lock to F18 on every keyboard, opens Liquid on a tap in another app and toggles Caps Lock on Shift + Caps Lock, without extra permissions. The mapping holds for the whole computer, so the helper gives Caps Lock back when it quits or the user switches to another account; switching to Standard does so at once. Turning the item off under System Settings → General → Login Items stops the key; at its next start Liquid reports it and continues without the Liquid key. A helper that quit, for example because another app took its key, starts again when Liquid starts or the Liquid key is turned on. `app/macos/LiquidKey/test.sh` runs its checks and compiles the helper.
