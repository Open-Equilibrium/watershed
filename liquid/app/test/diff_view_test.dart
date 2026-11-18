import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/diff/turn_header.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/theme/tokens.g.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

class _OversizedSession extends FakeSession {
  @override
  Future<String> turnPatch(
    int from,
    int to,
    String path, {
    required bool hideWhitespace,
  }) => Future.error('the diff is larger than 4 MiB');
}

void main() {
  late FakeProject project;

  Future<void> show(
    WidgetTester tester, {
    List<BridgeTurn> turns = const [],
    List<BridgeFileChange> changes = const [],
    Map<String, String> patches = const {},
    FakeSession? session,
    DiffSettingsController? diffSettings,
    Size size = const Size(1600, 900),
  }) async {
    await tester.binding.setSurfaceSize(size);
    project = FakeProject(session: session);
    project.session
      ..turnList = turns
      ..changes = changes
      ..patches = patches;
    final state = SessionState(project);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: info,
          parseDiff: parsePatch,
          diffSettings: diffSettings,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Future<void> openMenu(WidgetTester tester, String shown) async {
    await tester.tap(find.text(shown));
    await tester.pumpAndSettle();
  }

  Future<void> pickScope(WidgetTester tester, String name) async {
    await tester.tap(find.widgetWithText(PopupMenuItem<DiffScope>, name));
    await tester.pumpAndSettle();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  Future<void> alt(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(key);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pumpAndSettle();
  }

  Finder code(String text) => find.text(text, findRichText: true);

  Finder containing(String text) =>
      find.textContaining(text, findRichText: true);

  Color? colorOf(WidgetTester tester, Finder line, String text) {
    Color? color;
    tester.widget<RichText>(line).text.visitChildren((span) {
      if (span is TextSpan && span.text == text) color = span.style?.color;
      return true;
    });
    return color;
  }

  final twoTurns = [
    turnOf(1, prompt: 'Add login', files: 1, added: 1),
    turnOf(2, prompt: 'Fix login', files: 1, added: 1),
  ];

  testWidgets('⌘D opens the latest turn\'s changes with their first patch', (
    tester,
  ) async {
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('a.txt'), fileOf('b.txt')],
      patches: {'a.txt': '@@ -1 +1 @@\n-a\n+first file\n'},
    );
    expect(find.byType(DiffPanel), findsOneWidget);
    expect(find.text('Latest turn'), findsWidgets);
    expect(find.text('2 files · +2 −0'), findsOneWidget);
    expect(containing('first file'), findsOneWidget);
  });

  testWidgets(
    'the turn menu lists the latest turn, each turn, all turns and the uncommitted changes',
    (tester) async {
      await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
      await openMenu(tester, 'Latest turn');
      for (final name in [
        'Latest turn',
        'Turn 2',
        'Turn 1',
        'All turns',
        'Changes',
        'Uncommitted',
      ]) {
        expect(
          find.widgetWithText(PopupMenuItem<DiffScope>, name),
          findsOneWidget,
          reason: name,
        );
      }
    },
  );

  testWidgets(
    'a turn shows its own changes, and all turns show from the first to the last',
    (tester) async {
      await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
      await openMenu(tester, 'Latest turn');
      await pickScope(tester, 'Turn 1');
      expect(find.text('Turn 1'), findsOneWidget);
      expect(project.session.calls, contains('changes 1..1'));

      await openMenu(tester, 'Turn 1');
      await pickScope(tester, 'All turns');
      expect(find.text('All turns'), findsOneWidget);
      expect(project.session.calls, contains('changes 1..2'));
    },
  );

  testWidgets('Uncommitted shows the checkout\'s changes', (tester) async {
    await show(tester, turns: twoTurns);
    project.session.gitBackend
      ..changes = [fileOf('notes.md')]
      ..patches = {'notes.md': '@@ -1 +1 @@\n-old\n+written today\n'};
    await openMenu(tester, 'Latest turn');
    await pickScope(tester, 'Uncommitted');
    expect(find.text('notes.md'), findsWidgets);
    expect(containing('written today'), findsOneWidget);
  });

  testWidgets(
    'Changes shows the branch\'s changes against its base, and re-reads them when the branch moves',
    (tester) async {
      await show(tester, turns: twoTurns);
      project.session.gitBackend
        ..branchFiles = [fileOf('feature.txt')]
        ..patches = {'feature.txt': '@@ -1 +1 @@\n-old\n+on the branch\n'};
      await openMenu(tester, 'Latest turn');
      await pickScope(tester, 'Changes');
      expect(find.text('feature.txt'), findsWidgets);
      expect(containing('on the branch'), findsOneWidget);
      project.session.gitBackend
        ..branchFiles = [fileOf('feature.txt'), fileOf('more.txt')]
        ..change(gitStatusOf(branch: 'feature', defaultBranch: false));
      await tester.pumpAndSettle();
      expect(find.text('more.txt'), findsWidgets);
    },
  );

  testWidgets('a branch with no changes against its base says so', (
    tester,
  ) async {
    await show(tester, turns: twoTurns);
    await openMenu(tester, 'Latest turn');
    await pickScope(tester, 'Changes');
    expect(find.text('No changes on this branch'), findsOneWidget);
    expect(find.text('The branch is the same as its base.'), findsOneWidget);
  });

  testWidgets('choosing another file shows its patch', (tester) async {
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('a.txt'), fileOf('b.txt')],
      patches: {
        'a.txt': '@@ -1 +1 @@\n-a\n+first file\n',
        'b.txt': '@@ -1 +1 @@\n-b\n+second file\n',
      },
    );
    await tester.tap(find.text('b.txt'));
    await tester.pumpAndSettle();
    expect(containing('second file'), findsOneWidget);
    expect(containing('first file'), findsNothing);
  });

  testWidgets('W hides whitespace changes, and the control shows them again', (
    tester,
  ) async {
    await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
    expect(find.text('Hide whitespace changes'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    await tester.pumpAndSettle();
    expect(find.text('Show whitespace changes'), findsOneWidget);
    expect(project.session.calls, contains('changes 2..2'));

    await tester.tap(find.text('Show whitespace changes'));
    await tester.pumpAndSettle();
    expect(find.text('Hide whitespace changes'), findsOneWidget);
  });

  testWidgets('before a turn ends, the Diff says so', (tester) async {
    await show(tester);
    expect(find.text('No finished turn yet'), findsOneWidget);
    expect(
      find.text('A turn’s changes appear here when it ends.'),
      findsOneWidget,
    );
  });

  testWidgets('the close button closes the Diff panel', (tester) async {
    await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
    await tester.tap(find.byKey(const Key('diff-panel-close')));
    await tester.pumpAndSettle();
    expect(find.byType(DiffPanel), findsNothing);
  });

  testWidgets('says why a diff cannot be shown', (tester) async {
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('big.txt', added: 200000)],
      session: _OversizedSession(),
    );
    expect(
      find.text('Could not show the changes: the diff is larger than 4 MiB'),
      findsOneWidget,
    );
  });

  testWidgets('each file shows its status letter and its line counts', (
    tester,
  ) async {
    await show(
      tester,
      turns: twoTurns,
      changes: [
        fileOf('added.txt', status: BridgeChangeStatus.added),
        fileOf(
          'gone.txt',
          status: BridgeChangeStatus.deleted,
          added: 0,
          removed: 3,
        ),
        fileOf('moved.txt', status: BridgeChangeStatus.renamed, added: 0),
        fileOf('edit.txt', added: 2, removed: 1),
      ],
    );
    for (final letter in ['A', 'D', 'R', 'M']) {
      expect(find.text(letter), findsOneWidget, reason: letter);
    }
    expect(code('−3'), findsOneWidget);
    expect(code('+2 −1'), findsOneWidget);
  });

  testWidgets(
    'folds folders by a click and with ← and →, and ↑↓ pass a folded folder\'s files',
    (tester) async {
      await show(
        tester,
        turns: twoTurns,
        changes: [
          fileOf('lib/x.ts'),
          fileOf('src/a.ts'),
          fileOf('src/b.ts'),
          fileOf('top.ts'),
        ],
      );
      await tester.tap(find.text('src'));
      await tester.pumpAndSettle();
      expect(find.text('b.ts'), findsNothing);
      await tester.tap(find.text('src'));
      await tester.pumpAndSettle();
      expect(find.text('b.ts'), findsOneWidget);

      await press(tester, LogicalKeyboardKey.arrowDown);
      expect(find.text('src/a.ts'), findsOneWidget, reason: 'the next file');
      await press(tester, LogicalKeyboardKey.arrowLeft);
      expect(find.text('a.ts'), findsNothing, reason: '← folds its folder');
      await press(tester, LogicalKeyboardKey.arrowRight);
      expect(find.text('a.ts'), findsOneWidget, reason: '→ unfolds it');

      await press(tester, LogicalKeyboardKey.arrowLeft);
      await press(tester, LogicalKeyboardKey.arrowDown);
      expect(
        find.text('top.ts'),
        findsNWidgets(2),
        reason:
            '↓ passes the folded files: the list and the header show top.ts',
      );
      await press(tester, LogicalKeyboardKey.arrowUp);
      expect(
        find.text('lib/x.ts'),
        findsOneWidget,
        reason: '↑ passes them back',
      );
    },
  );

  testWidgets('shows a file side by side with U and back from the control', (
    tester,
  ) async {
    final settings = DiffSettingsController();
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('a.ts', added: 2, removed: 1)],
      patches: {'a.ts': '@@ -1,2 +1,3 @@\n keep\n-old\n+new\n+extra\n'},
      diffSettings: settings,
    );
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
    expect(code('extra'), findsOneWidget, reason: 'beside an empty side');

    await tester.tap(find.text('Unified'));
    await tester.pumpAndSettle();
    expect(settings.value.layout, DiffLayout.unified);
    expect(code('keep'), findsOneWidget);
  });

  testWidgets('jumps between the changes of a file with ⌥↑↓ in both layouts', (
    tester,
  ) async {
    final unchanged = [for (var line = 1; line <= 200; line++) ' line $line'];
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('a.ts', added: 2, removed: 2)],
      patches: {
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
      },
    );
    expect(find.text('@@ -1,201 +1,201 @@'), findsOneWidget);
    expect(find.text('@@ -400,1 +400,1 @@'), findsNothing);
    await alt(tester, LogicalKeyboardKey.arrowDown);
    expect(find.text('@@ -400,1 +400,1 @@'), findsOneWidget);
    expect(find.text('@@ -1,201 +1,201 @@'), findsNothing);
    await alt(tester, LogicalKeyboardKey.arrowUp);
    expect(find.text('@@ -1,201 +1,201 @@'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyU);
    await alt(tester, LogicalKeyboardKey.arrowDown);
    expect(find.text('@@ -400,1 +400,1 @@'), findsOneWidget);
  });

  testWidgets('a wide panel keeps the switches beside the turn header', (
    tester,
  ) async {
    tester.view
      ..physicalSize = const Size(2000, 900)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await show(
      tester,
      size: const Size(2000, 900),
      turns: twoTurns,
      changes: [fileOf('code.ts', added: 2, removed: 1)],
      patches: {'code.ts': '@@ -1 +1,2 @@\n keep\n+added\n'},
      diffSettings: DiffSettingsController(const DiffSettings(width: 700)),
    );
    final header = tester.getRect(find.byType(TurnHeader));
    final toggle = tester.getRect(find.text('Hide whitespace changes'));
    expect(
      toggle.top,
      lessThan(header.bottom),
      reason: 'the switches share the header line, not a line of their own',
    );
    expect(
      header.right,
      greaterThan(toggle.left - WsMetrics.space6),
      reason: 'the header fills the line up to the switches',
    );
  });

  testWidgets(
    'colors the code, marks a Windows line end with a faint ␍ and cuts a very long line',
    (tester) async {
      final bundle = 'let a = 1;' * 100000;
      await show(
        tester,
        turns: twoTurns,
        changes: [fileOf('max.ts', added: 2, removed: 1)],
        patches: {
          'max.ts':
              '@@ -1,2 +1,3 @@\n-let name = "old";\r\n+const max = 10; // ok\r\n+$bundle\n',
        },
      );
      final added = code('const max = 10; // ok␍');
      final ws = tester.element(added).ws;
      expect(colorOf(tester, added, 'const'), ws.synKw);
      expect(colorOf(tester, added, '10'), ws.synNum);
      expect(colorOf(tester, added, '// ok'), ws.synCom);
      expect(colorOf(tester, added, '␍'), ws.textFaint);
      expect(colorOf(tester, code('let name = "old";␍'), '"old"'), ws.synStr);

      final cut = tester.widget<RichText>(
        find.byWidgetPredicate(
          (widget) =>
              widget is RichText &&
              widget.text.toPlainText().startsWith('let a = 1;'),
        ),
      );
      final shown = cut.text.toPlainText();
      expect(shown, endsWith('…'));
      expect(shown.length, lessThan(5000));
    },
  );

  testWidgets('a turn with no changes left says so once whitespace is hidden', (
    tester,
  ) async {
    await show(tester, turns: twoTurns);
    expect(find.text('No changes in this turn'), findsOneWidget);
    await press(tester, LogicalKeyboardKey.keyW);
    expect(
      find.text('No changes in this turn with whitespace hidden'),
      findsOneWidget,
    );
    expect(find.text('W shows them.'), findsOneWidget);
  });

  testWidgets(
    'reverts the shown turn from the Diff header, but not for all turns',
    (tester) async {
      await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
      await tester.tap(find.text('Revert files to before this turn…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Revert and keep changes'));
      await tester.pumpAndSettle();
      expect(project.session.calls, contains('revert 2 files=false'));

      await openMenu(tester, 'Latest turn');
      await pickScope(tester, 'All turns');
      expect(find.text('Revert files to before this turn…'), findsNothing);
    },
  );

  testWidgets(
    'the Diff panel’s left edge resizes it within its limits, and the width is kept',
    (tester) async {
      final diffSettings = DiffSettingsController();
      await show(
        tester,
        turns: twoTurns,
        changes: [fileOf('a.txt')],
        diffSettings: diffSettings,
      );
      double width() => tester.getSize(find.byType(DiffPanel)).width;
      final window = MediaQuery.sizeOf(tester.element(find.byType(DiffPanel)))
          .width;
      expect(width(), 540, reason: 'T3 Code’s right panel default');

      await tester.tap(find.byKey(const Key('diff-resize')));
      await tester.pumpAndSettle();
      expect(
        diffSettings.value.width,
        isNull,
        reason: 'a press without a drag leaves the width unset',
      );

      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(-10, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), 550);
      expect(diffSettings.value.width, 550);

      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(-3000, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), closeTo(window * 0.7, 0.5));
      expect(
        diffSettings.value.width,
        width(),
        reason: 'the saved width is the one shown',
      );

      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(3000, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), 360);
    },
  );
}
