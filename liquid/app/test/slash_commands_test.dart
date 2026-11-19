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

  Future<void> addTable(WidgetTester tester, int index) async {
    await type(tester, index, '/');
    await type(tester, index, 'table');
    await key(tester, LogicalKeyboardKey.enter);
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

  BridgeTextStyle? styleOf(int index) =>
      kit.fake.styles[kit.fake.views[kit.page]![index].block];

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
      final table = kit.fake.views[kit.page]![1].block;
      expect(kit.fake.databases[table]!.name, 'Expenses');
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

  testWidgets('a View\'s title follows a rename elsewhere', (tester) async {
    await show(tester, ['']);
    await addTable(tester, 0);
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    final view = kit.fake.views[kit.page]![0].id;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.renameView,
        page: kit.page,
        view: view,
        text: 'Trips',
      ),
    );
    await tester.pumpAndSettle();
    expect(inTable(find.text('Trips')), findsOneWidget);
  });

  for (final last in [false, true]) {
    testWidgets(
      'a Table chosen in a line that starts a Section starts it instead'
      '${last ? ', also in the last line' : ''}',
      (tester) async {
        await show(tester, ['Intro', '', if (!last) 'After']);
        await type(tester, 1, '---');
        expect(kit.fake.views[kit.page]![1].boundary, isTrue);
        await type(tester, 1, '/');
        await type(tester, 1, 'table');
        await key(tester, LogicalKeyboardKey.enter);
        final views = kit.fake.views[kit.page]!;
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
    await addTable(tester, 0);
    int renames() => kit.fake.applied
        .where((applied) => applied.kind == BridgeMutationKind.renameView)
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
    expect(kit.fake.databases[kit.fake.views[kit.page]![0].block]!.name, '你好');
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

  testWidgets('a Table\'s name being composed is written when Liquid quits', (
    tester,
  ) async {
    await show(tester, ['']);
    await addTable(tester, 0);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pumpAndSettle();
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(kit.fake.databases[kit.fake.views[kit.page]![0].block]!.name, 'ni');
  });

  testWidgets(
    'a View\'s title renamed elsewhere while typed in shows once left',
    (tester) async {
      await show(tester, ['']);
      await addTable(tester, 0);
      final view = kit.fake.views[kit.page]![0].id;
      await kit.workspace.apply(
        mutation(
          BridgeMutationKind.renameView,
          page: kit.page,
          view: view,
          text: 'Trips',
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(block(0));
      await tester.pumpAndSettle();
      expect(inTable(find.text('Trips')), findsOneWidget);
    },
  );

  testWidgets('a Table\'s name being composed is written when left', (
    tester,
  ) async {
    await show(tester, ['']);
    await addTable(tester, 0);
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
    expect(kit.fake.databases[kit.fake.views[kit.page]![0].block]!.name, 'ni');
    expect(inTable(find.text('ni')), findsOneWidget);
  });

  testWidgets('what is typed while a Table is added becomes its name', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(kinds(), textTableText);
    expect(kit.fake.textsOf(kit.page), ['Intro', 'After']);
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
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
    expect(kit.fake.views[kit.page]![2].kind, BridgeBlockKind.database);
    expect(kit.fake.textsOf(kit.page), ['Intro', '', 'After']);
    expect(tester.widget<TextField>(tableName()).focusNode!.hasFocus, isTrue);
    expect(kit.workspace.problem.value, isNotNull);
  });

  testWidgets('what is typed while a Table is added in the last line becomes '
      'its name', (tester) async {
    await show(tester, ['Intro', '']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.text};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
  });

  testWidgets('a person who moved on while a Table is added keeps the caret', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    await tester.tap(block(0));
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
  });

  testWidgets('the last line takes no keys until its Table took the caret', (
    tester,
  ) async {
    await show(tester, ['Intro', '']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'E');
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.views};
    kit.fake.gate = null;
    gate.complete();
    await tester.pump();
    await type(tester, 1, 'x');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
  });

  testWidgets('a name typed while a Table is added survives the Page closing', (
    tester,
  ) async {
    await show(tester, ['Intro', '']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    final area = kit.workspace.open.value!.area;
    unawaited(kit.workspace.newPage(area));
    await tester.pump();
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
  });

  for (final held in [true, false]) {
    testWidgets(
      'a name typed while a Table is added is written when Liquid quits'
      '${held ? ' during the add' : ' before the Table takes the caret'}',
      (tester) async {
        await show(tester, ['Intro', '', 'After']);
        await type(tester, 1, '/');
        await type(tester, 1, 'table');
        final gate = kit.fake.gate = Completer<void>();
        kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
        final reads = kit.fake.deferred = [];
        kit.fake.deferredKinds = {BridgeQueryKind.views};
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await tester.pump();
        await type(tester, 1, 'Ex');
        if (!held) {
          kit.fake.gate = null;
          gate.complete();
          await tester.pump();
        }
        final draining = kit.workspace.drain();
        if (held) {
          kit.fake.gate = null;
          gate.complete();
        }
        await tester.pump();
        await draining;
        final table = kit.fake.views[kit.page]!.firstWhere(
          (view) => view.kind == BridgeBlockKind.database,
        );
        expect(kit.fake.databases[table.block]!.name, 'Ex');
        kit.fake.deferred = null;
        for (final read in reads) {
          read.complete();
        }
        await tester.pumpAndSettle();
      },
    );
  }

  testWidgets('a name handed from a line that went is written once', (
    tester,
  ) async {
    await show(tester, ['Intro', '', 'After']);
    await type(tester, 1, '/');
    await type(tester, 1, 'table');
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addDatabaseBlock};
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.views};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 1, 'Ex');
    kit.fake.gate = null;
    gate.complete();
    await tester.pump();
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(
      kit.fake.applied.where(
        (applied) => applied.kind == BridgeMutationKind.renameView,
      ),
      hasLength(1),
    );
    expect(kit.fake.databases[kit.fake.views[kit.page]![1].block]!.name, 'Ex');
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

  testWidgets('near the window\'s bottom, the menu opens above the line', (
    tester,
  ) async {
    await show(tester, [
      for (var line = 0; line < 20; line++) 'Line $line',
      '',
    ]);
    await type(tester, 20, '/');
    final line = tester.getRect(block(20));
    final shown = tester.getRect(menu);
    expect(line.top, greaterThan(450), reason: 'the line is low');
    expect(shown.bottom, lessThanOrEqualTo(line.top));
    expect(shown.top, greaterThanOrEqualTo(0));
  });

  testWidgets('scrolling the Page chooses the menu\'s side again', (
    tester,
  ) async {
    await show(tester, [
      for (var line = 0; line < 20; line++) 'Line $line',
      '',
      for (var line = 21; line < 50; line++) 'Line $line',
    ]);
    await type(tester, 20, '/');
    final slash = find.byWidgetPredicate(
      (widget) => widget is TextField && widget.controller?.text == '/',
    );
    expect(
      tester.getRect(menu).bottom,
      lessThanOrEqualTo(tester.getRect(slash).top),
    );
    final scroll = tester.state<ScrollableState>(
      find
          .descendant(
            of: find.byType(CustomScrollView),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    scroll.position.jumpTo(
      scroll.position.pixels + tester.getRect(slash).top - 40,
    );
    await tester.pumpAndSettle();
    final line = tester.getRect(slash);
    final shown = tester.getRect(menu);
    expect(line.top, lessThan(100), reason: 'the line is high');
    expect(shown.top, greaterThanOrEqualTo(line.bottom));
    expect(shown.bottom, lessThanOrEqualTo(900));
  });

  testWidgets('in a low window, the menu scrolls to the chosen command', (
    tester,
  ) async {
    await show(tester, ['']);
    await tester.binding.setSurfaceSize(const Size(1400, 420));
    await tester.pumpAndSettle();
    await type(tester, 0, '/');
    final shown = tester.getRect(menu);
    expect(shown.bottom, lessThanOrEqualTo(420), reason: 'it fits the window');
    expect(
      tester.getRect(find.text('Type to filter · ↑↓ · ⏎')).bottom,
      lessThanOrEqualTo(shown.bottom),
    );
    await key(tester, LogicalKeyboardKey.arrowUp);
    final table = tester.getRect(item('Table'));
    expect(table.top, greaterThanOrEqualTo(shown.top));
    expect(table.bottom, lessThanOrEqualTo(shown.bottom));
    await key(tester, LogicalKeyboardKey.arrowDown);
    final text = tester.getRect(item('Text'));
    expect(text.top, greaterThanOrEqualTo(shown.top));
  });
}
