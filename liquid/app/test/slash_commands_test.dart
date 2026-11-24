import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  const textTableText = [
    BridgeBlockKind.text,
    BridgeBlockKind.database,
    BridgeBlockKind.text,
  ];

  Future<void> show(WidgetTester tester, List<String> texts) =>
      kit.show(tester, [for (final text in texts) TextPart(text)]);

  Future<void> setText(
    WidgetTester tester,
    String text, {
    bool composing = false,
  }) async {
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
        composing: composing
            ? TextRange(start: 0, end: text.length)
            : TextRange.empty,
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, int index, String typed) async {
    if (!field(tester, index).focusNode!.hasFocus) {
      await tester.tap(block(index));
      await tester.pump();
    }
    await setText(tester, field(tester, index).controller!.text + typed);
  }

  Future<void> addTable(WidgetTester tester, int index) async {
    await type(tester, index, '/');
    await type(tester, index, 'table');
    await key(tester, LogicalKeyboardKey.enter);
  }

  Future<Completer<void>> startTableAdd(
    WidgetTester tester,
    List<String> texts, {
    Set<BridgeQueryKind>? deferReads,
    Set<BridgeMutationKind>? gated = const {
      BridgeMutationKind.addDatabaseBlock,
    },
  }) async {
    await show(tester, texts);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = gated;
    if (deferReads != null) {
      kit.fake.deferred = [];
      kit.fake.deferredKinds = deferReads;
    }
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    return gate;
  }

  void release(Completer<void> gate) {
    kit.fake.gate = null;
    gate.complete();
  }

  void releaseReads() {
    final reads = kit.fake.deferred!;
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
  }

  final menu = find.byKey(const Key('slash commands'));
  Finder item(String label) => find.descendant(
    of: menu,
    matching: find.byKey(ValueKey('choice $label')),
  );

  Finder inTable(Finder matching) =>
      find.descendant(of: find.byType(TableBlock), matching: matching);

  Finder tableName() => inTable(find.byType(TextField));

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];

  BridgeView viewAt(int index) => kit.fake.views[kit.page]![index];

  BridgeTextStyle? styleOf(int index) => kit.fake.styles[viewAt(index).block];

  String databaseName(int index) =>
      kit.fake.databases[viewAt(index).block]!.name;

  int renames() => kit.fake.applied
      .where((applied) => applied.kind == BridgeMutationKind.renameView)
      .length;

  Future<void> rename(WidgetTester tester, String title) async {
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.renameView,
        page: kit.page,
        view: viewAt(0).id,
        text: title,
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    '/ in an empty Text Block opens the slash command menu, typing filters it',
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
        'Code',
      ]) {
        expect(item(label), findsOneWidget, reason: label);
      }

      await type(tester, 1, 'quo');
      expect(item('Quote'), findsOneWidget);
      expect(item('Heading 1'), findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      expect(menu, findsNothing);
      expect(styleOf(1), BridgeTextStyle.quote);
      expect(kit.fake.textsOf(kit.page), ['Intro', '']);
      expect(kit.fake.views[kit.page], hasLength(2), reason: '⏎ split nothing');
    },
  );

  testWidgets('↑↓ choose a slash command and a click takes it', (tester) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.arrowUp);
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(kit.fake.textsOf(kit.page), ['']);

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
      expect(kinds(), textTableText);
      expect(kit.fake.textsOf(kit.page), ['Intro', 'After']);
      expect(tester.widget<TextField>(tableName()).focusNode!.hasFocus, isTrue);
      expect(inTable(find.text('Untitled')), findsOneWidget);
      expect(inTable(find.text('Name')), findsOneWidget);
      expect(
        inTable(find.textContaining('rows')),
        findsNothing,
        reason: 'a Table without rows counts none',
      );

      await tester.enterText(tableName(), 'Expenses');
      await tester.pumpAndSettle();
      expect(databaseName(1), 'Expenses');
    },
  );

  testWidgets('esc closes the slash command menu and keeps what was typed', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '/');
    await type(tester, 0, 'x');
    await key(tester, LogicalKeyboardKey.escape);
    expect(menu, findsNothing);
    expect(kit.fake.textsOf(kit.page), ['/x']);
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
  });

  testWidgets(
    'the slash command menu opens only in an empty Text Block and closes '
    'when nothing matches',
    (tester) async {
      await show(tester, ['Lisbon', '']);
      await type(tester, 0, '/');
      expect(menu, findsNothing);
      await type(tester, 1, '/');
      await type(tester, 1, 'zzz');
      expect(menu, findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      expect(kit.fake.textsOf(kit.page), [
        'Lisbon/',
        '/zzz',
        '',
      ], reason: '⏎ splits again');
    },
  );

  testWidgets('a View\'s title follows a rename elsewhere, focused or not', (
    tester,
  ) async {
    await show(tester, ['']);
    await addTable(tester, 0);
    await rename(tester, 'Trips');
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    expect(inTable(find.text('Trips')), findsOneWidget);
    await rename(tester, 'Hotels');
    expect(inTable(find.text('Hotels')), findsOneWidget);
  });

  for (final last in [false, true]) {
    testWidgets(
      'a Table chosen in a line that starts a Section starts it instead'
      '${last ? ', also in the last line' : ''}',
      (tester) async {
        await show(tester, ['Intro', '', if (!last) 'After']);
        await type(tester, 1, '---');
        expect(viewAt(1).boundary, isTrue);
        await type(tester, 1, '/');
        await type(tester, 1, 'table');
        await key(tester, LogicalKeyboardKey.enter);
        expect(viewAt(1).kind, BridgeBlockKind.database);
        expect(viewAt(1).boundary, isTrue);
        expect(
          kit.fake.views[kit.page]!.where((view) => view.boundary),
          hasLength(1),
        );
      },
    );
  }

  testWidgets('a Table\'s name is written once a composition ends', (
    tester,
  ) async {
    await show(tester, ['']);
    await addTable(tester, 0);
    for (final typed in ['n', 'ni', 'nih']) {
      await setText(tester, typed, composing: true);
    }
    expect(renames(), 0);
    await setText(tester, '你好');
    expect(renames(), 1);
    expect(databaseName(0), '你好');
  });

  testWidgets('the slash command menu opens in a list item too', (
    tester,
  ) async {
    await show(tester, ['']);
    await type(tester, 0, '- ');
    expect(styleOf(0), BridgeTextStyle.bullet);
    await type(tester, 0, '/');
    expect(menu, findsOneWidget);
    await type(tester, 0, 'quo');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.quote);
  });

  for (final quits in [true, false]) {
    testWidgets('a Table\'s name being composed is written when '
        '${quits ? 'Liquid quits' : 'left'}', (tester) async {
      await show(tester, ['']);
      await addTable(tester, 0);
      await setText(tester, 'ni', composing: true);
      if (quits) {
        await kit.workspace.drain();
      } else {
        await tester.tap(block(0));
      }
      await tester.pumpAndSettle();
      expect(databaseName(0), 'ni');
      expect(inTable(find.text('ni')), findsOneWidget);
    });
  }

  testWidgets('what is typed while a Table is added becomes its name', (
    tester,
  ) async {
    final gate = await startTableAdd(tester, [
      'Intro',
      '',
      'After',
    ], gated: null);
    await type(tester, 1, 'Ex');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    release(gate);
    await tester.pumpAndSettle();
    expect(kinds(), textTableText);
    expect(kit.fake.textsOf(kit.page), ['Intro', 'After']);
    expect(databaseName(1), 'Ex');
    final name = tester.widget<TextField>(tableName());
    expect(name.focusNode!.hasFocus, isTrue);
    expect(name.controller!.text, 'Ex');
  });

  testWidgets('a Table\'s name shows no read that crossed its renames', (
    tester,
  ) async {
    await show(tester, ['']);
    await addTable(tester, 0);
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.database};
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
    final name = tester.widget<TextField>(tableName());
    expect(name.controller!.text, 'AB');
    while (reads.isNotEmpty) {
      reads.removeAt(0).complete();
      await tester.pump();
      expect(name.controller!.text, 'AB');
    }
    kit.fake.deferred = null;
    await tester.pumpAndSettle();
    expect(name.controller!.text, 'AB');
  });

  testWidgets('a Table added in a line that stays takes the caret anyway', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    kit.fake.refuseKinds = {BridgeMutationKind.removeView};
    await key(tester, LogicalKeyboardKey.enter);
    expect(viewAt(2).kind, BridgeBlockKind.database);
    expect(kit.fake.textsOf(kit.page), ['Intro', '', 'After']);
    expect(tester.widget<TextField>(tableName()).focusNode!.hasFocus, isTrue);
    expect(kit.workspace.problem.value, isNotNull);
  });

  testWidgets('what is typed while a Table is added in the last line becomes '
      'its name', (tester) async {
    final gate = await startTableAdd(
      tester,
      ['Intro', ''],
      deferReads: {BridgeQueryKind.text},
    );
    await type(tester, 1, 'Ex');
    releaseReads();
    await tester.pumpAndSettle();
    release(gate);
    await tester.pumpAndSettle();
    expect(databaseName(1), 'Ex');
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
  });

  testWidgets('a person who moved on while a Table is added keeps the caret', (
    tester,
  ) async {
    final gate = await startTableAdd(tester, ['Intro', '', 'After']);
    await type(tester, 1, 'Ex');
    await tester.tap(block(0));
    await tester.pump();
    release(gate);
    await tester.pumpAndSettle();
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
    expect(databaseName(1), 'Ex');
  });

  testWidgets('the last line takes no keys until its Table took the caret', (
    tester,
  ) async {
    final gate = await startTableAdd(tester, ['Intro', '']);
    await type(tester, 1, 'E');
    kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.views};
    release(gate);
    await tester.pump();
    await type(tester, 1, 'x');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    releaseReads();
    await tester.pumpAndSettle();
    expect(databaseName(1), 'Ex');
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
  });

  for (final held in [true, false]) {
    testWidgets(
      'a name typed while a Table is added is written when Liquid quits'
      '${held ? ' during the add' : ' before the Table takes the caret'}',
      (tester) async {
        final gate = await startTableAdd(
          tester,
          ['Intro', '', 'After'],
          deferReads: {BridgeQueryKind.views},
        );
        await type(tester, 1, 'Ex');
        if (!held) {
          release(gate);
          await tester.pump();
        }
        final draining = kit.workspace.drain();
        if (held) release(gate);
        await tester.pump();
        await draining;
        expect(databaseName(1), 'Ex');
        releaseReads();
        await tester.pumpAndSettle();
      },
    );
  }

  testWidgets('a name handed from a line that went is written once', (
    tester,
  ) async {
    final gate = await startTableAdd(
      tester,
      ['Intro', '', 'After'],
      deferReads: {BridgeQueryKind.views},
    );
    await type(tester, 1, 'Ex');
    release(gate);
    await tester.pump();
    releaseReads();
    await tester.pumpAndSettle();
    expect(renames(), 1);
    expect(databaseName(1), 'Ex');
  });

  testWidgets('/ on a blank Page opens the slash command menu', (tester) async {
    await show(tester, []);
    await tester.enterText(find.byKey(const Key('first-line')), '/');
    await tester.pumpAndSettle();
    expect(menu, findsOneWidget);
    await type(tester, 0, 'h');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(kit.fake.textsOf(kit.page), ['']);
  });

  testWidgets('a quickly typed /ta on a blank Page opens the menu filtered', (
    tester,
  ) async {
    await show(tester, []);
    kit.fake
      ..gate = Completer()
      ..gatedKinds = {BridgeMutationKind.addTextBlock};
    final first = find.byKey(const Key('first-line'));
    await tester.enterText(first, '/');
    await tester.pump();
    await tester.enterText(first, '/ta');
    await tester.pump();
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(item('Table'), findsOneWidget);
    expect(item('Text'), findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    expect(kit.fake.views[kit.page]!.first.kind, BridgeBlockKind.database);
  });

  testWidgets('text pasted on a blank Page opens no menu', (tester) async {
    await show(tester, []);
    await tester.enterText(find.byKey(const Key('first-line')), '/quote');
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(kit.fake.textsOf(kit.page), ['/quote']);
  });

  testWidgets('a line left alone by a join opens no menu', (tester) async {
    await show(tester, ['/text', '']);
    await tester.tap(block(1));
    await tester.pump();
    await key(tester, LogicalKeyboardKey.backspace);
    expect(kit.fake.textsOf(kit.page), ['/text']);
    expect(menu, findsNothing);
  });

  testWidgets('what is typed while a Whiteboard is added stays in its line', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'whiteboard');
    expect(item('Whiteboard'), findsOneWidget);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addWhiteboardBlock};
    await key(tester, LogicalKeyboardKey.enter);
    await type(tester, 1, 'Keep this');
    release(gate);
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['Intro', 'Keep this', 'After']);
  });

  for (final (name, typed, kind) in [
    ('a Code shortcut', '```', BridgeBlockKind.code),
    ('a Math shortcut', r'$$', BridgeBlockKind.math),
  ]) {
    testWidgets('$name typed while the first block is added converts it', (
      tester,
    ) async {
      await show(tester, []);
      kit.fake
        ..gate = Completer()
        ..gatedKinds = {BridgeMutationKind.addTextBlock};
      await tester.enterText(find.byKey(const Key('first-line')), typed);
      await tester.pump();
      kit.fake.gate!.complete();
      await tester.pumpAndSettle();
      expect(kinds(), [kind]);
    });
  }
}
