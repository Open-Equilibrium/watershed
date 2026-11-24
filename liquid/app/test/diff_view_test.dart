import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeProject project;

  Future<void> show(
    WidgetTester tester, {
    List<BridgeTurn> turns = const [],
    List<BridgeFileChange> changes = const [],
    Map<String, String> patches = const {},
    FakeSession? session,
    DiffSettingsController? diffSettings,
    PatchParser parse = parsePatch,
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
    await pumpApp(
      tester,
      AgentShell(
        state: state,
        info: info,
        parseDiff: parse,
        diffSettings: diffSettings,
      ),
    );
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Future<void> chooseScope(
    WidgetTester tester,
    String current,
    String name,
  ) async {
    await tester.tap(find.text(current));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(PopupMenuItem<DiffScope>, name));
    await tester.pumpAndSettle();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  Finder code(String text) => find.text(text, findRichText: true);

  Finder containing(String text) =>
      find.textContaining(text, findRichText: true);

  final twoTurns = [
    turnOf(1, prompt: 'Add login', files: 1, added: 1),
    turnOf(2, prompt: 'Fix login', files: 1, added: 1),
  ];

  testWidgets('⌘D shows the latest turn, and its file patches', (tester) async {
    await show(
      tester,
      turns: twoTurns,
      changes: [fileOf('a.txt'), fileOf('b.txt')],
      patches: {
        'a.txt': '@@ -1 +1 @@\n-a\n+first file\n',
        'b.txt': '@@ -1 +1 @@\n-b\n+second file\n',
      },
    );
    expect(find.byType(DiffPanel), findsOneWidget);
    expect(find.text('Latest turn'), findsWidgets);
    expect(find.text('2 files · +2 −0'), findsOneWidget);
    expect(containing('first file'), findsOneWidget);

    await tester.tap(find.text('b.txt'));
    await tester.pumpAndSettle();
    expect(containing('second file'), findsOneWidget);
    expect(containing('first file'), findsNothing);
    await tester.tap(find.byKey(const Key('diff-panel-close')));
    await tester.pumpAndSettle();
    expect(find.byType(DiffPanel), findsNothing);
  });

  testWidgets('a refresh while a patch loads shows the newer patch', (
    tester,
  ) async {
    final change = fileOf('a.txt');
    final gate = Completer<void>();
    var parses = 0;
    final session = FakeSession()
      ..turnList = twoTurns
      ..changes = [change]
      ..patches = {'a.txt': '@@ -1 +1 @@\n-a\n+old file\n'};
    await show(
      tester,
      session: session,
      turns: twoTurns,
      changes: [change],
      patches: session.patches,
      parse: (patch) async {
        if (parses++ == 0) await gate.future;
        return parsePatch(patch);
      },
    );
    session.patches = {'a.txt': '@@ -1 +1 @@\n-a\n+new file\n'};
    session.changes = [change];
    session.emit(event(BridgeEventKind.turnsChanged, 1));
    await tester.pumpAndSettle();

    gate.complete();
    await tester.pumpAndSettle();

    expect(containing('new file'), findsOneWidget);
    expect(containing('old file'), findsNothing);
  });

  testWidgets('the turn menu lists scopes, and a turn opens', (tester) async {
    await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
    await tester.tap(find.text('Latest turn'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.widgetWithText(PopupMenuItem<DiffScope>, 'Turn 1'));
    await tester.pumpAndSettle();
    expect(find.text('Turn 1'), findsOneWidget);
    expect(project.session.calls, contains('changes 1..1'));

    await chooseScope(tester, 'Turn 1', 'All turns');
    expect(find.text('All turns'), findsOneWidget);
    expect(project.session.calls, contains('changes 1..2'));
  });

  testWidgets('Uncommitted shows the checkout\'s changes', (tester) async {
    await show(tester, turns: twoTurns);
    project.session.gitBackend
      ..changes = [fileOf('notes.md')]
      ..patches = {'notes.md': '@@ -1 +1 @@\n-old\n+written today\n'};
    await chooseScope(tester, 'Latest turn', 'Uncommitted');
    expect(find.text('notes.md'), findsWidgets);
    expect(containing('written today'), findsOneWidget);
  });

  testWidgets('Changes says so, then shows the branch', (tester) async {
    await show(tester, turns: twoTurns);
    await chooseScope(tester, 'Latest turn', 'Changes');
    expect(find.text('No changes on this branch'), findsOneWidget);
    expect(find.text('The branch is the same as its base.'), findsOneWidget);

    project.session.gitBackend
      ..branchFiles = [fileOf('feature.txt')]
      ..patches = {'feature.txt': '@@ -1 +1 @@\n-old\n+on the branch\n'}
      ..change(gitStatusOf(branch: 'feature', defaultBranch: false));
    await tester.pumpAndSettle();
    expect(find.text('feature.txt'), findsWidgets);
    expect(containing('on the branch'), findsOneWidget);
    project.session.gitBackend
      ..branchFiles = [fileOf('feature.txt'), fileOf('more.txt')]
      ..change(
        gitStatusOf(branch: 'feature', defaultBranch: false, branchAdded: 1),
      );
    await tester.pumpAndSettle();
    expect(find.text('more.txt'), findsWidgets);
  });

  testWidgets('folding, and ↑↓ passing folded files', (tester) async {
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
      reason: '↓ passes the folded files: the list and the header show top.ts',
    );
    await press(tester, LogicalKeyboardKey.arrowUp);
    expect(find.text('lib/x.ts'), findsOneWidget, reason: '↑ passes them back');
  });

  testWidgets('U shows a file side by side, and back', (tester) async {
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

  testWidgets('reverts the shown turn, but not all turns', (tester) async {
    await show(tester, turns: twoTurns, changes: [fileOf('a.txt')]);
    await tester.tap(find.text('Revert files to before this turn…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Revert and keep changes'));
    await tester.pumpAndSettle();
    expect(project.session.calls, contains('revert 2 files=false'));

    await chooseScope(tester, 'Latest turn', 'All turns');
    expect(find.text('Revert files to before this turn…'), findsNothing);
  });
}
