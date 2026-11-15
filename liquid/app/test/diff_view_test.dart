import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeProject project;

  /// Shows an open session whose finished `turns` changed `changes`, with each file's `patches`,
  /// and opens its Diff panel with ⌘D.
  Future<void> show(
    WidgetTester tester, {
    List<BridgeTurn> turns = const [],
    List<BridgeFileChange> changes = const [],
    Map<String, String> patches = const {},
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    project = FakeProject();
    project.session
      ..turnList = turns
      ..changes = changes
      ..patches = patches;
    final state = SessionState(project);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, parseDiff: parsePatch),
      ),
    );
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  /// Opens the turn menu, the header's button that names the shown scope.
  Future<void> openMenu(WidgetTester tester, String shown) async {
    await tester.tap(find.text(shown));
    await tester.pumpAndSettle();
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
    expect(
      find.textContaining('first file', findRichText: true),
      findsOneWidget,
    );
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
      await tester.tap(find.widgetWithText(PopupMenuItem<DiffScope>, 'Turn 1'));
      await tester.pumpAndSettle();
      expect(find.text('Turn 1'), findsOneWidget);
      expect(project.session.calls, contains('changes 1..1'));

      await openMenu(tester, 'Turn 1');
      await tester.tap(
        find.widgetWithText(PopupMenuItem<DiffScope>, 'All turns'),
      );
      await tester.pumpAndSettle();
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
    await tester.tap(
      find.widgetWithText(PopupMenuItem<DiffScope>, 'Uncommitted'),
    );
    await tester.pumpAndSettle();
    expect(find.text('notes.md'), findsWidgets);
    expect(
      find.textContaining('written today', findRichText: true),
      findsOneWidget,
    );
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
    expect(
      find.textContaining('second file', findRichText: true),
      findsOneWidget,
    );
    expect(find.textContaining('first file', findRichText: true), findsNothing);
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
}
