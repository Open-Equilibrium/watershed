import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

void main() {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  /// Opens a Page of paragraphs holding `texts`.
  Future<void> show(WidgetTester tester, List<String> texts) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    fake = FakeWorkspace();
    workspace = WorkspaceState(fake);
    await workspace.start();
    page = workspace.open.value!.page;
    String? after;
    for (final text in texts) {
      final added = await workspace.apply(
        mutation(
          BridgeMutationKind.addTextBlock,
          page: page,
          after: after,
          text: text,
        ),
      );
      after = added.view;
    }
    workspace.open.value = OpenPage(workspace.open.value!.area, page);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(workspace: workspace),
      ),
    );
    await tester.pumpAndSettle();
    addTearDown(workspace.dispose);
  }

  Finder block(int index) => find.descendant(
    of: find.byType(TextBlock).at(index),
    matching: find.byType(TextField),
  );
  TextField field(WidgetTester tester, int index) =>
      tester.widget<TextField>(block(index));

  /// Types `typed` at the end of the Text Block at `index`, as the text input does.
  Future<void> type(WidgetTester tester, int index, String typed) async {
    if (!field(tester, index).focusNode!.hasFocus) {
      await tester.tap(block(index));
      await tester.pump();
    }
    final text = field(tester, index).controller!.text + typed;
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  final menu = find.byKey(const Key('block menu'));
  Finder item(String label) => find.descendant(
    of: menu,
    matching: find.byKey(ValueKey('choice $label')),
  );

  BridgeTextStyle? styleOf(int index) =>
      fake.styles[fake.views[page]![index].block];

  testWidgets(
    '/ in an empty Text Block opens the Block menu, typing filters it',
    (tester) async {
      await show(tester, ['Intro', '']);
      await type(tester, 1, '/');
      expect(menu, findsOneWidget);
      for (final label in [
        'Text',
        'Heading 1',
        'Heading 2',
        'Heading 3',
        'Bulleted list',
        'Numbered list',
        'To-do list',
        'Quote',
        'Table',
      ]) {
        expect(item(label), findsOneWidget, reason: label);
      }

      await type(tester, 1, 'quo');
      expect(item('Quote'), findsOneWidget);
      expect(item('Heading 1'), findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      expect(menu, findsNothing);
      expect(styleOf(1), BridgeTextStyle.quote);
      expect(fake.textsOf(page), ['Intro', '']);
      expect(fake.views[page], hasLength(2), reason: '⏎ split nothing');
    },
  );

  testWidgets('↑↓ choose in the Block menu and a click inserts', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.arrowUp);
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(fake.textsOf(page), ['']);

    await type(tester, 0, '/');
    await tester.tap(item('Numbered list'));
    await tester.pumpAndSettle();
    expect(styleOf(0), BridgeTextStyle.numbered);
    expect(menu, findsNothing);
  });

  testWidgets(
    'Table replaces the line with a Table whose name takes the keys',
    (tester) async {
      await show(tester, ['Intro', '', 'After']);
      await type(tester, 1, '/');
      await type(tester, 1, 'tab');
      await key(tester, LogicalKeyboardKey.enter);
      expect(fake.views[page]!.map((view) => view.kind), [
        BridgeBlockKind.text,
        BridgeBlockKind.database,
        BridgeBlockKind.text,
      ]);
      expect(fake.textsOf(page), ['Intro', 'After']);
      final name = find.descendant(
        of: find.byType(TableBlock),
        matching: find.byType(TextField),
      );
      expect(tester.widget<TextField>(name).focusNode!.hasFocus, isTrue);
      expect(
        find.descendant(
          of: find.byType(TableBlock),
          matching: find.text('Untitled'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byType(TableBlock),
          matching: find.text('Name'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byType(TableBlock),
          matching: find.textContaining('rows'),
        ),
        findsNothing,
        reason: 'a Table without rows counts none',
      );

      await tester.enterText(name, 'Expenses');
      await tester.pumpAndSettle();
      final table = fake.views[page]![1].block;
      expect(fake.databases[table]!.name, 'Expenses');
    },
  );

  testWidgets('a Table chosen in the last line keeps a line below it', (
    tester,
  ) async {
    await show(tester, ['Intro', '']);
    await type(tester, 1, '/');
    await tester.tap(item('Table'));
    await tester.pumpAndSettle();
    expect(fake.views[page]!.map((view) => view.kind), [
      BridgeBlockKind.text,
      BridgeBlockKind.database,
      BridgeBlockKind.text,
    ]);
    expect(fake.textsOf(page), ['Intro', '']);
  });

  testWidgets('esc closes the Block menu and keeps what was typed', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'x');
    await key(tester, LogicalKeyboardKey.escape);
    expect(menu, findsNothing);
    expect(fake.textsOf(page), ['/x']);
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
  });

  testWidgets('the Block menu opens only in an empty Text Block and closes '
      'when nothing matches', (tester) async {
    await show(tester, ['Lisbon', '']);
    await type(tester, 0, '/');
    expect(menu, findsNothing);
    await type(tester, 1, '/');
    await type(tester, 1, 'zzz');
    expect(menu, findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    expect(fake.textsOf(page), [
      'Lisbon/',
      '/zzz',
      '',
    ], reason: '⏎ splits again');
  });

  testWidgets('a Table\'s name follows a rename elsewhere', (tester) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'table');
    await key(tester, LogicalKeyboardKey.enter);
    // The caret leaves the name for the line below.
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    final table = fake.views[page]![0].block;
    await workspace.apply(
      mutation(
        BridgeMutationKind.renameDatabase,
        page: page,
        block: table,
        text: 'Trips',
      ),
    );
    await tester.pumpAndSettle();
    expect(
      find.descendant(
        of: find.byType(TableBlock),
        matching: find.text('Trips'),
      ),
      findsOneWidget,
    );
  });

  for (final last in [false, true]) {
    testWidgets(
      'a Table chosen in a line that starts a Section starts it instead'
      '${last ? ', also in the last line' : ''}',
      (tester) async {
        await show(tester, ['Intro', '', if (!last) 'After']);
        await type(tester, 1, '---');
        expect(fake.views[page]![1].boundary, isTrue);
        await type(tester, 1, '/');
        await type(tester, 1, 'table');
        await key(tester, LogicalKeyboardKey.enter);
        final views = fake.views[page]!;
        expect(views[1].kind, BridgeBlockKind.database);
        expect(views[1].boundary, isTrue);
        expect(views.where((view) => view.boundary), hasLength(1));
      },
    );
  }

  testWidgets('a Table\'s name is written once a composition ends', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'table');
    await key(tester, LogicalKeyboardKey.enter);
    int renames() => fake.applied
        .where((applied) => applied.kind == BridgeMutationKind.renameDatabase)
        .length;
    for (final typed in ['n', 'ni', 'nih']) {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: typed,
          selection: TextSelection.collapsed(offset: typed.length),
          composing: TextRange(start: 0, end: typed.length),
        ),
      );
      await tester.pumpAndSettle();
    }
    expect(renames(), 0);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '你好',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.pumpAndSettle();
    expect(renames(), 1);
    expect(fake.databases[fake.views[page]![0].block]!.name, '你好');
  });

  testWidgets('the Block menu opens in a list item too', (tester) async {
    await show(tester, ['']);
    await type(tester, 0, '- ');
    expect(styleOf(0), BridgeTextStyle.bullet);
    await type(tester, 0, '/');
    expect(menu, findsOneWidget);
    await type(tester, 0, 'quo');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.quote);
  });

  testWidgets('a Table\'s name being composed is written when Liquid quits', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'table');
    await key(tester, LogicalKeyboardKey.enter);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pumpAndSettle();
    await workspace.drain();
    await tester.pumpAndSettle();
    expect(fake.databases[fake.views[page]![0].block]!.name, 'ni');
  });

  testWidgets(
    'a Table\'s name renamed elsewhere while typed in shows once left',
    (tester) async {
      await show(tester, ['']);
      await type(tester, 0, '/');
      await type(tester, 0, 'table');
      await key(tester, LogicalKeyboardKey.enter);
      final table = fake.views[page]![0].block;
      await workspace.apply(
        mutation(
          BridgeMutationKind.renameDatabase,
          page: page,
          block: table,
          text: 'Trips',
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(block(0));
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byType(TableBlock),
          matching: find.text('Trips'),
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets('a Table\'s name being composed is written when left', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'table');
    await key(tester, LogicalKeyboardKey.enter);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    expect(fake.databases[fake.views[page]![0].block]!.name, 'ni');
    expect(
      find.descendant(of: find.byType(TableBlock), matching: find.text('ni')),
      findsOneWidget,
    );
  });

  testWidgets('what is typed while a Table is added becomes its name', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = fake.gate = Completer<void>();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(fake.views[page]!.map((view) => view.kind), [
      BridgeBlockKind.text,
      BridgeBlockKind.database,
      BridgeBlockKind.text,
    ]);
    expect(fake.textsOf(page), ['Intro', 'After']);
    expect(fake.databases[fake.views[page]![1].block]!.name, 'Ex');
    final name = tester.widget<TextField>(
      find.descendant(
        of: find.byType(TableBlock),
        matching: find.byType(TextField),
      ),
    );
    expect(name.focusNode!.hasFocus, isTrue);
    expect(name.controller!.text, 'Ex');
  });

  testWidgets('a Table\'s name shows no read that crossed its renames', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'table');
    await key(tester, LogicalKeyboardKey.enter);
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database};
    for (final typed in ['A', 'AB']) {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: typed,
          selection: TextSelection.collapsed(offset: typed.length),
        ),
      );
      await tester.pump();
    }
    await tester.tap(block(0));
    await tester.pump();
    final name = tester.widget<TextField>(
      find.descendant(
        of: find.byType(TableBlock),
        matching: find.byType(TextField),
      ),
    );
    expect(name.controller!.text, 'AB');
    while (reads.isNotEmpty) {
      reads.removeAt(0).complete();
      await tester.pump();
      expect(name.controller!.text, 'AB');
    }
    fake.deferred = null;
    await tester.pumpAndSettle();
    expect(name.controller!.text, 'AB');
  });

  testWidgets('a Table added in a line that stays takes the caret anyway', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    fake.refuseKinds = {BridgeMutationKind.removeView};
    await key(tester, LogicalKeyboardKey.enter);
    expect(fake.views[page]![2].kind, BridgeBlockKind.database);
    expect(fake.textsOf(page), ['Intro', '', 'After']);
    final name = tester.widget<TextField>(
      find.descendant(
        of: find.byType(TableBlock),
        matching: find.byType(TextField),
      ),
    );
    expect(name.focusNode!.hasFocus, isTrue);
    expect(workspace.problem.value, isNotNull);
  });

  testWidgets('what is typed while a Table is added in the last line becomes '
      'its name', (tester) async {
    await show(tester, ['Intro', '']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = fake.gate = Completer<void>();
    fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    // The line's read after its cleared text arrived answers once typing went on.
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.text};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(fake.databases[fake.views[page]![1].block]!.name, 'Ex');
    expect(fake.textsOf(page), ['Intro', '']);
  });
}
