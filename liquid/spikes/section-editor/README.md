# Spike: an editor for a Section's text (D-116)

Throwaway spike for [D-116](../../../docs/decisions/open-decisions.html#d-116) under ADR-0317; it is never merged. Both options edit one Automerge rich text with block markers, as `liquid-core` would hold a Section, and run the same scenarios, randomized edits and measurements.

- `core/` — a `cdylib` over Automerge 0.12 (UTF-16 offsets, as `liquid-core`): splice, split (a new marker), block attributes, marks with Peritext expand rules (bold, italic and strike grow at their end; code and links never), `insert` with exact marks, spans, fork and merge.
- **A, super_editor 0.3.0-dev.52:** `app/lib/binding.dart` diffs the touched window of Paragraphs after each edit, a marker being a token of its own, so ⏎ and ⌫ become `split_block` and a deleted marker, never copied text. Entry `lib/main.dart`, tests `test/section_test.dart`, `integration_test/perf_test.dart`.
- **C, Liquid's own editor:** `app/lib/own/` (model 262 lines, editor 721) sends each edit as the core's own op, with no diff; one selection over all Paragraphs, one IME connection, a lazily built list, and the Markdown of the styled word at the caret with two caret stops at its end, as the formatting mockup shows. Entry `lib/main_own.dart`, tests `test/own_test.dart`, `integration_test/own_perf_test.dart`.

## Run

```sh
cd core && cargo test && cargo build --release && cd ../app
export SECTION_SPIKE_LIB=$PWD/../core/target/release/libsection_spike.dylib
flutter test
flutter test test/own_test.dart --dart-define=FUZZ_STEPS=1500 --dart-define=FUZZ_SEED=2 --plain-name "random edits"
flutter run -d macos -t lib/main_own.dart --dart-define=SECTION_SPIKE_LIB=$SECTION_SPIKE_LIB   # C; lib/main.dart for A
flutter drive --driver=test_driver/integration_test.dart --target=integration_test/own_perf_test.dart \
  -d macos --profile --dart-define=SECTION_SPIKE_LIB=$SECTION_SPIKE_LIB
```

## Results against D-116's criterion (macOS, Apple M6, 2026-11-23)

| Criterion | A: super_editor | C: own editor |
|---|---|---|
| Mouse and ⇧-arrow selection across Paragraphs | Works | Works |
| ⏎ adds a marker, ⇧⏎ breaks a line, ⌫ removes one; the upper Paragraph keeps its marker | Works | Works; ⏎ at a start adds the empty Paragraph above |
| Concurrent split or join with an edit on another device | Each edit stays in its Paragraph | Same |
| Composition (Japanese) and a dead key (´ e → é), simulated | Reaches the core once committed | Same; the candidate window is placed at the caret |
| Real German and Japanese typing on a Mac | **Not yet done by a person** | **Not yet done by a person** |
| Expand rules and mark shortcuts | Expand rules, ⌘B and typed `**x**` tested | Expand rules, ⌘B and ⌘\ tested; ⌘I and ⌘E bound; typed `**x**` not built |
| Markdown of the styled word at the caret, two caret stops | Not in super_editor | Works |
| Toggles and indentation on the markers' parents | Not tried | Not tried; indentation renders |
| Randomized edits, 8 seeds × 1,500 steps | 6 pass; 2 hit an `attributed_text` bug | All pass |

### Time per keystroke (profile build)

| Section | Sync per keystroke, median | Typing frame build, avg / worst | ⏎ frame build, avg / worst | Load to first frame |
|---|---|---|---|---|
| A, 100 Paragraphs | 0.067 ms | 0.83 / 1.14 ms | 2.1 / 2.8 ms | 21 ms |
| A, 1,000 Paragraphs | 0.085 ms | 3.3 / 4.4 ms | 21.3 / 23.6 ms | 134 ms |
| C, 100 Paragraphs | 0.054 ms | 0.26 / 0.62 ms | 1.0 / 1.4 ms | 23 ms |
| C, 1,000 Paragraphs | 0.055 ms | 0.27 / 0.62 ms | 0.8 / 1.3 ms | 49 ms |
| C, 10,000 Paragraphs | 0.065 ms | 0.32 / 0.80 ms | 1.2 / 1.5 ms | 542 ms |

A lays every Paragraph out in one `Column`, so its frames grow with the Section; C builds only visible Paragraphs, so they stay flat. Sync covers the FFI call and Automerge; the typing runs insert plain text.

## Findings for `liquid-core`, whichever editor

- **Mark ops are expensive in Automerge 0.12:** about 0.6 ms for `mark` and 0.2 ms for `unmark`, against 3 µs for a splice, at any Section size (`cargo test --release cost -- --ignored --nocapture`). Each also stays in history, so an editor must not mark every keystroke; the `insert` op corrects only what Automerge's expand rules got wrong.
- **`get_marks` can disagree with `marks` and `spans`:** after mark and unmark history it reported no mark where both show bold. `get-marks-replay.json` reproduces it (`SECTION_REPLAY=../get-marks-replay.json cargo test replay -- --ignored --nocapture`); the core reads `marks` instead. Worth an upstream report.
- **Predicting expand rules in Dart fails:** a first version of C mirrored Peritext's rules to skip ops and broke the randomized test; the core must decide.

## Findings on A

- **Maintenance:** see D-116; one maintainer, no stable release since 2024, Superlist keeps its own fork.
- **Upstream bug:** `attributed_text` 0.4.7 leaves adjacent bold spans unmerged, and bold over both drops the last character (`test/attributed_text_bug_test.dart`).
- **Runtime dependencies:** test tools ship as runtime dependencies (`flutter_test`, `mockito`, `golden_toolkit`, discontinued); adding it to a new app added 76 packages to its lock file.
- **Teardown:** disposing a `SuperEditor` schedules a frame, which fails a widget test that ends right after it.

## What C still lacks

Everything a product editor needs beyond the criterion: accessibility (no semantics yet), double- and triple-click selection, scrolling the caret into view and auto-scroll while dragging, copy and paste with Markdown, undo through History, spellcheck, a context menu, right-to-left text, touch handles for the phone client, and ⏎/arrows into Paragraphs not yet built. The spike also calls the core synchronously over FFI on the main isolate, which client architecture rule 1 forbids; the product would send ops through the bridge.
