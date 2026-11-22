# Spike: an editor for a Section's text (D-116)

Throwaway spike for [D-116](../../../docs/decisions/open-decisions.html#d-116) under ADR-0317; it is never merged. It binds [super_editor](https://pub.dev/packages/super_editor) 0.3.0-dev.52 (option A) to one Automerge rich text with block markers, as `liquid-core` would hold a Section.

- `core/` — a `cdylib` over Automerge 0.12 (UTF-16 offsets, as `liquid-core`): splice, split (a new marker), block attributes, marks with Peritext expand rules (bold, italic and strike grow at their end; code and links never), spans, fork and merge.
- `app/lib/binding.dart` — keeps the core in step with the editor. Each edit diffs the touched window of Paragraphs as tokens, where a Paragraph's marker is a token of its own: a new Paragraph becomes `split_block`, a removed one deletes its marker, so ⏎ and ⌫ never copy text. Ambiguous runs of equal characters take the offset the editor reported. Typing during IME composition waits until it is committed.
- `app/lib/main.dart` — two devices side by side; **Sync** merges both ways, as the Sync Server would.

## Run

```sh
cd core && cargo test && cargo build --release && cd ../app
export SECTION_SPIKE_LIB=$PWD/../core/target/release/libsection_spike.dylib
flutter test                                    # behavior, randomized edits
flutter test --dart-define=FUZZ_STEPS=1500 --dart-define=FUZZ_SEED=2 --plain-name "random edits"
flutter run -d macos --dart-define=SECTION_SPIKE_LIB=$SECTION_SPIKE_LIB
flutter drive --driver=test_driver/integration_test.dart --target=integration_test/perf_test.dart \
  -d macos --profile --dart-define=SECTION_SPIKE_LIB=$SECTION_SPIKE_LIB
```

## Results against D-116's criterion (macOS, 2026-11-22)

| Criterion | Result |
|---|---|
| Mouse and ⇧-arrow selection across Paragraphs | Works (`section_test.dart`) |
| ⏎ adds a marker, ⇧⏎ breaks a line, ⌫ removes a marker | Works; the upper Paragraph keeps its marker ID |
| Concurrent split or join with an edit on another device | Each edit stays in its Paragraph (core and editor tests) |
| German dead keys, Japanese IME | Simulated composition works and reaches the core only once committed; **a person still has to type both on a real Mac** |
| Marks as the formatting mockup shows | Expand rules, ⌘B, ⌘I and typed `**x**` work; the Markdown shown at the caret, two caret stops at an edge and ⌘\ are not in super_editor and would be built |
| Toggles and indentation on the markers' parents | Not tried: list and task indentation exist, a toggle does not |
| Time per keystroke recorded | See below |

Randomized edits (`random edits`, keys only, editor compared with the core after every step): 1,500 steps pass for six of eight seeds; the other two hit an upstream bug in `attributed_text` 0.4.7, reproduced in `attributed_text_bug_test.dart`: two adjacent bold spans stay unmerged, and bold added over both drops the last character. The core is right in both cases.

### Time per keystroke (profile build, macOS, Apple silicon)

| Section | Sync per keystroke (median, p99) | Typing frame build (avg, worst) | ⏎ frame build (avg, worst) | Load to first frame |
|---|---|---|---|---|
| 100 Paragraphs | 0.067 ms, 0.086 ms | 0.83 ms, 1.14 ms | 2.1 ms, 2.8 ms | 21 ms |
| 1,000 Paragraphs | 0.085 ms, 0.123 ms | 3.3 ms, 4.4 ms | 21.3 ms, 23.6 ms | 134 ms |

Sync covers the diff, the FFI call and Automerge; it stays flat. The frames grow with the Section, because super_editor lays every Paragraph out in one `Column` without virtualization (`layout_single_column/_layout.dart`); ⏎ in a 1,000-Paragraph Section takes longer than a 60 Hz frame.

## Other findings

- **Release state:** the last stable release is 0.2.7 (2024-06); 0.3 has been in `-dev` releases since, the latest 2026-06.
- **Runtime dependencies:** super_editor ships its test tools as runtime dependencies (`flutter_test`, `mockito`, `golden_toolkit`, discontinued, `flutter_test_robots`), plus `http`, `url_launcher` and a Quill Delta library; adding it to a new app added 76 packages to its lock file. Licenses are MIT, BSD-3-Clause and Apache-2.0.
- **Teardown:** disposing a `SuperEditor` schedules a frame (`ime_keyboard_control.dart`), which fails a widget test that ends right after it.
- **Spike shortcuts:** the core is called synchronously over FFI on the main isolate, which client architecture rule 1 forbids; the product would keep the focused Section's document in Dart and send its ops through the bridge. Each inserted run also writes its marks explicitly, a few Automerge ops per keystroke.
