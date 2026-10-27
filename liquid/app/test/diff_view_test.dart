import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/diff/diff_view.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  /// Shows the Diff tab of a session on [backend].
  Future<SessionState> show(
    WidgetTester tester,
    FakeBackend backend, {
    Size size = const Size(1600, 900),
    DiffSettingsController? settings,
  }) async {
    await tester.binding.setSurfaceSize(size);
    final kept = settings ??= DiffSettingsController();
    final state = SessionState(backend);
    // As the shell does: the settings own the choice the change reads use.
    void hide() => state.hideWhitespace.value = kept.value.hideWhitespace;
    kept.addListener(hide);
    hide();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Material(
          child: DiffView(state: state, settings: settings),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return state;
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  testWidgets(
    'lists changed files as folders and shows the selected file\'s diff',
    (tester) async {
      final backend = FakeBackend()
        ..changed = [
          BridgeFileChange(
            path: 'src/routes/login.ts',
            shown: 'src/routes/login.ts',
            status: BridgeChangeStatus.modified,
            added: 1,
            removed: 1,
          ),
          BridgeFileChange(
            path: 'src/middleware/rateLimit.ts',
            shown: 'src/middleware/rateLimit.ts',
            status: BridgeChangeStatus.added,
            added: 2,
            removed: 0,
          ),
        ]
        ..patches = {
          'src/middleware/rateLimit.ts':
              '@@ -0,0 +1,2 @@\n+export const limit = 5;\n+// per minute\n',
          'src/routes/login.ts':
              '@@ -1,2 +1,2 @@\n-const old = 1;\n+const fresh = 2;\n same();\n',
        };
      await show(tester, backend);
      expect(find.text('2 files · +3 −1'), findsOneWidget);
      for (final name in [
        'src',
        'middleware',
        'rateLimit.ts',
        'routes',
        'login.ts',
      ]) {
        expect(find.text(name), findsOneWidget);
      }
      expect(
        find.text('src/middleware/rateLimit.ts'),
        findsOneWidget,
        reason: 'the first file opens',
      );
      expect(
        find.textContaining('export const limit = 5;', findRichText: true),
        findsOneWidget,
      );

      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.pumpAndSettle();
      expect(find.text('src/routes/login.ts'), findsOneWidget);
      expect(
        find.textContaining('const fresh = 2;', findRichText: true),
        findsOneWidget,
      );
      expect(
        find.textContaining('const old = 1;', findRichText: true),
        findsOneWidget,
      );
    },
  );

  testWidgets('says when no run started yet and when a run changed nothing', (
    tester,
  ) async {
    final backend = FakeBackend();
    await show(tester, backend);
    expect(
      find.text('No run since Liquid opened this session'),
      findsOneWidget,
      reason: 'a continued session\'s earlier runs are not compared',
    );

    backend.controller.add(event(BridgeEventKind.turnStarted));
    backend.controller.add(event(BridgeEventKind.turnCompleted));
    await tester.pumpAndSettle();
    expect(find.text('No changes in this run'), findsOneWidget);
  });

  testWidgets('shows a file\'s name with hiding characters written out', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'src/a\u202eb.ts',
          shown: 'src/a<U+202E>b.ts',
          status: BridgeChangeStatus.added,
          added: 1,
          removed: 0,
        ),
      ]
      ..patches = {'src/a\u202eb.ts': '@@ -0,0 +1 @@\n+x\n'};
    await show(tester, backend);
    expect(find.text('a<U+202E>b.ts'), findsOneWidget, reason: 'the list');
    expect(
      find.text('src/a<U+202E>b.ts'),
      findsOneWidget,
      reason: 'the header',
    );
    expect(find.textContaining('\u202e'), findsNothing);
    expect(backend.patchReads, [
      'src/a\u202eb.ts',
    ], reason: 'the patch is read by the file\'s own name');
  });

  testWidgets('says why a diff cannot be shown', (tester) async {
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'big.txt',
          shown: 'big.txt',
          status: BridgeChangeStatus.modified,
          added: 200000,
          removed: 0,
        ),
      ]
      ..patchError = 'the diff is larger than 4 MiB';
    final state = await show(tester, backend);
    const reason = 'Could not show the changes: the diff is larger than 4 MiB';
    expect(find.text(reason), findsOneWidget, reason: 'the file says why');
    // Each run reads the changes again; the lasting failure is reported once.
    for (var turn = 0; turn < 2; turn++) {
      backend.controller
        ..add(event(BridgeEventKind.turnStarted))
        ..add(event(BridgeEventKind.turnCompleted));
      await tester.pumpAndSettle();
    }
    expect(state.notices.value.map((notice) => notice.text), [
      'Could not show the changes of big.txt: the diff is larger than 4 MiB',
    ], reason: 'the notification names the file');
  });

  testWidgets('scrolls a long line sideways to its end', (tester) async {
    final long = '${'x' * 400} changed at the end';
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'wide.ts',
          shown: 'wide.ts',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 0,
        ),
      ]
      ..patches = {'wide.ts': '@@ -1 +1 @@\n+$long\n'};
    await show(tester, backend, size: const Size(1200, 900));
    final line = find.textContaining('changed at the end', findRichText: true);
    final before = tester.getTopLeft(line).dx;
    await tester.dragFrom(
      tester.getTopLeft(line) + const Offset(20, 5),
      const Offset(-3000, 0),
    );
    await tester.pumpAndSettle();
    expect(tester.getTopLeft(line).dx, lessThan(before - 1000));
  });

  testWidgets('keeps every arrow press while a patch loads and reads only '
      'the last', (tester) async {
    BridgeFileChange file(String path) => BridgeFileChange(
      path: path,
      shown: path,
      status: BridgeChangeStatus.modified,
      added: 1,
      removed: 0,
    );
    final backend = FakeBackend()
      ..changed = [file('a.ts'), file('b.ts'), file('c.ts'), file('d.ts')]
      ..patches = {
        for (final name in ['a', 'b', 'c', 'd'])
          '$name.ts': '@@ -1 +1 @@\n+$name changed\n',
      };
    await show(tester, backend);
    expect(
      find.textContaining('a changed', findRichText: true),
      findsOneWidget,
    );
    backend.patchGate = Completer();
    for (var press = 0; press < 3; press++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    }
    backend.patchGate!.complete();
    await tester.pumpAndSettle();
    expect(
      find.textContaining('d changed', findRichText: true),
      findsOneWidget,
    );
    expect(backend.patchReads, [
      'a.ts',
      'b.ts',
      'd.ts',
    ], reason: 'c.ts was passed while b.ts loaded');
  });

  testWidgets('folds folders by click and with ← and →', (tester) async {
    BridgeFileChange file(String path) => BridgeFileChange(
      path: path,
      shown: path,
      status: BridgeChangeStatus.modified,
      added: 1,
      removed: 0,
    );
    final backend = FakeBackend()
      ..changed = [file('src/a.ts'), file('src/b.ts'), file('top.ts')];
    await show(tester, backend);

    await tester.tap(find.text('src'));
    await tester.pumpAndSettle();
    expect(find.text('b.ts'), findsNothing);
    expect(find.text('top.ts'), findsWidgets);
    await tester.tap(find.text('src'));
    await tester.pumpAndSettle();
    expect(find.text('b.ts'), findsOneWidget);

    expect(find.text('src/a.ts'), findsOneWidget, reason: 'a.ts is open');
    await press(tester, LogicalKeyboardKey.arrowLeft);
    expect(find.text('a.ts'), findsNothing, reason: '← folds its folder');
    await press(tester, LogicalKeyboardKey.arrowRight);
    expect(find.text('a.ts'), findsOneWidget, reason: '→ unfolds it');
    await press(tester, LogicalKeyboardKey.arrowLeft);
    await press(tester, LogicalKeyboardKey.arrowDown);
    expect(
      find.text('top.ts'),
      findsNWidgets(2),
      reason: '↓ passes the folded files: the list and the header show top.ts',
    );
  });

  testWidgets('says when the changes cannot be read instead of old ones', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'old.ts',
          shown: 'old.ts',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 0,
        ),
      ];
    final state = await show(tester, backend);
    expect(find.text('old.ts'), findsWidgets);

    backend.changesError = 'the project could not be captured';
    await state.readChanges();
    await tester.pumpAndSettle();
    expect(find.text('old.ts'), findsNothing);
    expect(
      find.text(
        'Could not show the changes: the project could not be captured',
      ),
      findsOneWidget,
    );
  });

  testWidgets('says when the project is not in Git', (tester) async {
    final backend = FakeBackend()..changed = null;
    final state = await show(tester, backend);
    expect(
      find.text(
        'This project is not in Git, so Liquid cannot show what a run changed.',
      ),
      findsOneWidget,
    );
    expect(state.notices.value, isEmpty, reason: 'nothing failed');
  });

  testWidgets('reports a lasting failure to read the changes once', (
    tester,
  ) async {
    final backend = FakeBackend()..changesError = 'git could not start';
    final state = SessionState(backend);
    await state.readChanges();
    await state.readChanges();
    expect(state.notices.value, hasLength(1));
  });

  testWidgets('moves past a folded folder in both directions', (tester) async {
    BridgeFileChange file(String path) => BridgeFileChange(
      path: path,
      shown: path,
      status: BridgeChangeStatus.modified,
      added: 1,
      removed: 0,
    );
    final backend = FakeBackend()
      ..changed = [
        file('lib/x.ts'),
        file('src/a.ts'),
        file('src/b.ts'),
        file('top.ts'),
      ];
    await show(tester, backend);

    await press(tester, LogicalKeyboardKey.arrowDown);
    expect(find.text('src/a.ts'), findsOneWidget);
    await press(tester, LogicalKeyboardKey.arrowLeft);
    await press(tester, LogicalKeyboardKey.arrowDown);
    expect(find.text('top.ts'), findsNWidgets(2), reason: 'the next file');
    await press(tester, LogicalKeyboardKey.arrowLeft);
    await press(tester, LogicalKeyboardKey.arrowUp);
    expect(find.text('lib/x.ts'), findsOneWidget, reason: 'past the folder');
  });

  testWidgets('counts one file and explains a file without a text diff', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'logo.png',
          shown: 'logo.png',
          status: BridgeChangeStatus.modified,
          added: 0,
          removed: 0,
        ),
      ]
      ..patches = {
        'logo.png': 'Binary files a/logo.png and b/logo.png differ\n',
      };
    await show(tester, backend);
    expect(find.text('1 file · +0 −0'), findsOneWidget);
    expect(
      find.text(
        'No text changes to show, as for a binary file or a new file mode.',
      ),
      findsOneWidget,
    );
  });

  BridgeFileChange modified(String path, int added, int removed) =>
      BridgeFileChange(
        path: path,
        shown: path,
        status: BridgeChangeStatus.modified,
        added: added,
        removed: removed,
      );

  Finder code(String text) => find.text(text, findRichText: true);

  testWidgets('shows a file side by side with U and back from the control', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..changed = [modified('a.ts', 1, 1)]
      ..patches = {'a.ts': '@@ -1,2 +1,2 @@\n keep\n-old\n+new\n'};
    final settings = DiffSettingsController();
    await show(tester, backend, settings: settings);
    expect(
      tester.getTopLeft(code('old')).dy,
      lessThan(tester.getTopLeft(code('new')).dy),
    );

    await press(tester, LogicalKeyboardKey.keyU);
    expect(settings.value.layout, DiffLayout.split);
    expect(
      tester.getTopLeft(code('old')).dy,
      tester.getTopLeft(code('new')).dy,
      reason: 'the old and the new line share a row',
    );
    expect(code('keep'), findsNWidgets(2));

    await tester.tap(find.text('Unified'));
    await tester.pumpAndSettle();
    expect(settings.value.layout, DiffLayout.unified);
    expect(code('keep'), findsOneWidget);
  });

  testWidgets('jumps between the changes of a file with ⌥↑↓', (tester) async {
    final unchanged = [for (var line = 1; line <= 200; line++) ' line $line'];
    final backend = FakeBackend()
      ..changed = [modified('a.ts', 2, 2)]
      ..patches = {
        'a.ts': [
          '@@ -1,201 +1,201 @@',
          '-first',
          '+first changed',
          ...unchanged,
          '@@ -400,1 +400,1 @@',
          '-second',
          '+second changed',
          '',
        ].join('\n'),
      };
    await show(tester, backend);
    Future<void> alt(LogicalKeyboardKey key) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
    }

    expect(find.text('@@ -1,201 +1,201 @@'), findsOneWidget);
    expect(find.text('@@ -400,1 +400,1 @@'), findsNothing);
    await alt(LogicalKeyboardKey.arrowDown);
    expect(find.text('@@ -400,1 +400,1 @@'), findsOneWidget);
    expect(find.text('@@ -1,201 +1,201 @@'), findsNothing);
    await alt(LogicalKeyboardKey.arrowUp);
    expect(find.text('@@ -1,201 +1,201 @@'), findsOneWidget);
  });

  testWidgets('hides whitespace changes with W and from the control', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..changed = [modified('code.ts', 2, 1), modified('spaced.ts', 1, 1)]
      ..changedWithoutWhitespace = [modified('code.ts', 1, 0)]
      ..patches = {'code.ts': '@@ -1 +1,2 @@\n keep\n+added\n'};
    final settings = DiffSettingsController();
    await show(tester, backend, settings: settings);
    expect(find.text('spaced.ts'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyW);
    expect(settings.value.hideWhitespace, isTrue);
    expect(find.text('spaced.ts'), findsNothing);
    expect(backend.patchReads.last, 'code.ts without whitespace');

    await tester.tap(find.text('Hide whitespace'));
    await tester.pumpAndSettle();
    expect(settings.value.hideWhitespace, isFalse);
    expect(find.text('spaced.ts'), findsOneWidget);
  });

  testWidgets('opens with the Diff settings kept from before', (tester) async {
    final backend = FakeBackend()
      ..changed = [modified('code.ts', 2, 1), modified('spaced.ts', 1, 1)]
      ..changedWithoutWhitespace = [modified('code.ts', 1, 0)]
      ..patches = {'code.ts': '@@ -1,2 +1,2 @@\n keep\n-old\n+new\n'};
    await show(
      tester,
      backend,
      settings: DiffSettingsController(
        const DiffSettings(layout: DiffLayout.split, hideWhitespace: true),
      ),
    );
    expect(find.text('spaced.ts'), findsNothing);
    expect(
      tester.getTopLeft(code('old')).dy,
      tester.getTopLeft(code('new')).dy,
    );
  });

  testWidgets('marks a Windows line end with a faint ␍', (tester) async {
    final backend = FakeBackend()
      ..changed = [modified('max.ts', 1, 1)]
      ..patches = {
        'max.ts': '@@ -1 +1 @@\n-const max = 5;\r\n+const max = 10;\r\n',
      };
    await show(tester, backend);
    expect(code('const max = 10;␍'), findsOneWidget);
    final line = tester.widget<RichText>(code('const max = 10;␍'));
    TextSpan? mark;
    line.text.visitChildren((span) {
      if (span is TextSpan && span.text == '␍') mark = span;
      return true;
    });
    final palette = tester.element(code('const max = 10;␍')).ws;
    expect(mark?.style?.color, palette.textFaint);
  });
}
