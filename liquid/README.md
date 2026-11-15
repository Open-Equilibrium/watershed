# Liquid

The Liquid app: the agent surface and the Workspace's Explorer and Pages, growing slice by slice through [PLAN.md](../PLAN.md); see the [Liquid V-Spec](../docs/concept/V-Spec_Liquid.html).

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `app/` — the Flutter app. Its exact Flutter release is pinned in `app/pubspec.yaml`, which CI reads.
- `liquid-bridge/` — the Rust side of the flutter_rust_bridge bindings (ADR-0186). It embeds the Meta-Harness core (ADR-0184) and opens the Workspace with its key from the keychain (ADR-0216, ADR-0237); neither core depends on the bridge; `app/hook/build.dart` compiles it into the app (ADR-0213).
- `liquid-core/` — the Workspace core: object model, mutation pipeline and History over Automerge (ADR-0235, ADR-0238), committing every write to a store.
- `liquid-storage/` — the local replica: that store in SQLite with SQLCipher (ADR-0237). Build hosts: [TESTING.md](../TESTING.md#development-toolchain).

After changing `liquid-bridge/src/api`, run `flutter_rust_bridge_codegen generate` in `app/` and then `cargo fmt --all`. The generator's version is the `flutter_rust_bridge` version in `app/pubspec.yaml` (`cargo install --locked flutter_rust_bridge_codegen@<version>`). The generated files are committed without web bindings, and CI regenerates them to check they match.

## Run

The app opens the Workspace, whose replica lives in `~/Library/Application Support/Liquid/Workspace/` with its key in the keychain. With the project named in the environment it also starts pi and opens the start page, where recent sessions continue and the first message starts a new one; without a session, it opens the Workspace's first Page and says why on stderr. An agent session needs pi 1.3.0 with a configured model, which the user installs; Liquid bundles neither pi nor Node, also not for office users (ADR-0219). `flutter run -d macos` in `app/` needs Xcode and rustup ([build hosts](../TESTING.md#development-toolchain)). The project signs development builds ad hoc; release builds are to be signed with Developer ID, hardened and notarized, and to run without the App Sandbox (ADR-0200).

- `LIQUID_PROJECT` — the agent's project directory (required for an agent session).
- `LIQUID_PI_EXTENSION` — the absolute path of `meta-harness/pi-extension/watershed.ts` (required: without it a turn has no checkpoints, so Liquid does not start a session unless this extension is loaded).
- `LIQUID_PI` — the pi executable, a name looked up on `PATH` or an absolute path; defaults to `pi`.
- `LIQUID_PI_SESSION` — the absolute path of a stored pi session file to reopen, with its chat and branches.
- `LIQUID_KEY_LOG=1` — writes key events to stderr twice, as macOS hands them to the window (`native key`) and as Flutter receives them (`flutter key`), to diagnose shortcuts; only with the Liquid key, or with ⌘ or ⌃ but not ⌥ (which types characters such as @ on many layouts), so plain typing is not recorded.

## Liquid key

On a Mac the first start asks whether Caps Lock becomes the Liquid key; Settings → Shortcuts switches profiles later ([design guidelines](../docs/design/guidelines.md#principles)). Windows and then Linux get the key after macOS. Settings live in `~/Library/Application Support/Liquid/`: `shortcuts.json` for the shortcuts, `diff.json` for how the Diff panel shows changes, `session_card.json` for the rows the session card hides and `appearance.json` for the theme and text size.

The key needs macOS 13 and is the login item `LiquidKey.app` inside the app bundle, built by `app/macos/LiquidKey/build.sh` during the Xcode build. While it runs, it maps Caps Lock to F18 on every keyboard, opens Liquid on a tap in another app and toggles Caps Lock on Shift + Caps Lock, without extra permissions. Without those permissions it cannot tell another key pressed within 50 ms of Caps Lock from Caps Lock itself, so such a fast chord still counts as a tap. The mapping holds for the whole computer, so the helper gives Caps Lock back when it quits or the user switches to another account; switching to Standard does so at once; if the helper cannot give it back, the Liquid key profile stays on and Liquid shows the helper's reason. With fast user switching on a shared Mac, the departing user's helper may give Caps Lock back after the incoming user's helper mapped it, so Caps Lock can stay unmapped for the incoming user until that helper maps it again 1 s and 5 s after the switch. Turning the item off under System Settings → General → Login Items stops the key; at its next start Liquid reports it and continues without the Liquid key. A helper that quit, for example because another app took its key, starts again when Liquid starts or the Liquid key is turned on. `app/macos/LiquidKey/test.sh` runs its checks and compiles the helper.
