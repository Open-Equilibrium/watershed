import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

void main() {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  /// Opens a Page holding `texts`, as paragraphs unless `style` says otherwise.
  Future<void> show(
    WidgetTester tester, [
    List<String> texts = const [],
    BridgeTextStyle style = BridgeTextStyle.paragraph,
  ]) async {
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
      fake.styles[added.block!] = style;
    }
    // Opened afresh, not as the new Page that takes the keys in its title.
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

  /// Puts the caret at `offset` in the Text Block at `index`.
  Future<void> caret(WidgetTester tester, int index, int offset) async {
    await tester.tap(block(index));
    await tester.pump();
    field(tester, index).controller!.selection = TextSelection.collapsed(
      offset: offset,
    );
    await tester.pump();
  }

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  /// Types `typed` at the caret, as the text input does.
  Future<void> type(WidgetTester tester, int index, String typed) async {
    final text = field(tester, index).controller!;
    final at = text.selection.baseOffset;
    final value = text.text.replaceRange(at, at, typed);
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: value,
        selection: TextSelection.collapsed(offset: at + typed.length),
      ),
    );
    await tester.pumpAndSettle();
  }

  bool focused(WidgetTester tester, int index) =>
      field(tester, index).focusNode!.hasFocus;

  BridgeTextStyle styleOf(int index) =>
      fake.styles[fake.views[page]![index].block]!;

  testWidgets('typing on an empty Page creates its first Text Block', (
    tester,
  ) async {
    await show(tester);
    expect(find.text('Type to write.'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('first-line')), 'F');
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['F']);
    expect(focused(tester, 0), isTrue);

    await type(tester, 0, 'ive days');
    expect(fake.textsOf(page), ['Five days']);
    expect(field(tester, 0).controller!.text, 'Five days');
  });

  testWidgets('⏎ splits a Text Block at the caret and moves the caret down', (
    tester,
  ) async {
    await show(tester, ['Lisbon Porto']);
    await caret(tester, 0, 6);
    await key(tester, LogicalKeyboardKey.enter);
    expect(fake.textsOf(page), ['Lisbon', ' Porto']);
    expect(focused(tester, 1), isTrue);
    expect(field(tester, 1).controller!.selection.baseOffset, 0);

    // At the start an empty Text Block goes above and the caret stays.
    await key(tester, LogicalKeyboardKey.enter);
    expect(fake.textsOf(page), ['Lisbon', '', ' Porto']);
    expect(focused(tester, 2), isTrue);
  });

  testWidgets('⌫ at the start joins the Text Block above at the caret', (
    tester,
  ) async {
    await show(tester, ['Five days', ' in Lisbon']);
    await caret(tester, 1, 0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(fake.textsOf(page), ['Five days in Lisbon']);
    expect(focused(tester, 0), isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 9);
  });

  testWidgets('Markdown shortcuts format, and lists go on until left empty', (
    tester,
  ) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    await type(tester, 0, '#');
    await type(tester, 0, ' ');
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(fake.textsOf(page), ['']);
    await type(tester, 0, 'Plan');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(1), BridgeTextStyle.paragraph);

    await type(tester, 1, '- ');
    expect(styleOf(1), BridgeTextStyle.bullet);
    await type(tester, 1, 'Passport');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(2), BridgeTextStyle.bullet);
    // ⏎ in an empty item ends the list, as in Notion.
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(2), BridgeTextStyle.paragraph);
    expect(fake.textsOf(page), ['Plan', 'Passport', '']);

    // ⌫ at the start of a formatted Text Block first makes it a paragraph.
    await caret(tester, 1, 0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(styleOf(1), BridgeTextStyle.paragraph);
    expect(fake.textsOf(page), ['Plan', 'Passport', '']);
  });

  testWidgets('numbered items count, and a to-do is ticked by its box', (
    tester,
  ) async {
    await show(tester, ['Book', 'Pack', 'Fly']);
    for (var index = 0; index < 2; index++) {
      await caret(tester, index, 0);
      await type(tester, index, '1. ');
    }
    expect(find.text('1.'), findsOneWidget);
    expect(find.text('2.'), findsOneWidget);

    await caret(tester, 2, 0);
    await type(tester, 2, '[] ');
    expect(styleOf(2), BridgeTextStyle.todo);
    await tester.tap(find.byKey(const Key('to-do')));
    await tester.pumpAndSettle();
    expect(styleOf(2), BridgeTextStyle.done);
  });

  testWidgets('--- starts a Section below a divider, and ⌫ there removes it', (
    tester,
  ) async {
    await show(tester, ['Plan', '']);
    await caret(tester, 1, 0);
    await type(tester, 1, '---');
    expect(fake.views[page]![1].boundary, isTrue);
    expect(fake.textsOf(page), ['Plan', '']);
    expect(find.byKey(const Key('section-divider')), findsOneWidget);

    await key(tester, LogicalKeyboardKey.backspace);
    expect(fake.views[page]![1].boundary, isFalse);
    expect(find.byKey(const Key('section-divider')), findsNothing);
    expect(fake.textsOf(page), ['Plan', '']);
  });

  testWidgets('--- at the Page’s start stays text', (tester) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    await type(tester, 0, '---');
    expect(fake.views[page]![0].boundary, isFalse);
    expect(fake.textsOf(page), ['---']);
  });

  testWidgets('⏎ in the title and arrows at the edges move between Blocks', (
    tester,
  ) async {
    await show(tester, ['One', 'Two']);
    await tester.tap(find.byKey(const Key('page-title')));
    await key(tester, LogicalKeyboardKey.enter);
    expect(focused(tester, 0), isTrue);

    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 3,
    );
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(focused(tester, 1), isTrue);
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(focused(tester, 0), isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 3);
  });

  testWidgets('a refused edit shows the stored text again', (tester) async {
    await show(tester, ['Lisbon']);
    await caret(tester, 0, 6);
    fake.refuse = 'the replica is read-only';
    await type(tester, 0, '!');
    expect(field(tester, 0).controller!.text, 'Lisbon');
    expect(find.text('the replica is read-only'), findsOneWidget);
  });

  testWidgets('typing right after ⏎ lands in the new Text Block', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    // The split still waits, so the caret is still in the first Text Block.
    await type(tester, 0, 'Two');
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['One', 'Two']);
    expect(focused(tester, 1), isTrue);
    expect(field(tester, 1).controller!.text, 'Two');
    expect(field(tester, 0).controller!.text, 'One');
  });

  testWidgets('the first line hands over all typed while its Block is made', (
    tester,
  ) async {
    await show(tester);
    fake.gate = Completer();
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    await tester.enterText(find.byKey(const Key('first-line')), 'b');
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['b']);
    expect(field(tester, 0).controller!.text, 'b');
  });

  testWidgets('the first line waits until a composition ends', (tester) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('first-line')));
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), isEmpty);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '你',
        selection: TextSelection.collapsed(offset: 1),
      ),
    );
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['你']);
  });

  testWidgets('a read that crossed an edit never shows older text', (
    tester,
  ) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    fake.gate = Completer();
    fake.deferredKinds = {BridgeQueryKind.text};
    fake.deferred = [];
    await type(tester, 0, 'a');
    await type(tester, 0, 'b');
    fake.gate!.complete();
    await tester.pump();
    await tester.pump();
    final held = fake.deferred!;
    fake.deferred = null;
    // The newest reads answer first, the oldest last.
    for (final release in held.reversed) {
      release.complete();
      await tester.pump();
    }
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['ab']);
    expect(field(tester, 0).controller!.text, 'ab');
  });

  testWidgets('a Text Block takes no typing before its text is read', (
    tester,
  ) async {
    await show(tester, ['One']);
    fake.deferredKinds = {BridgeQueryKind.text};
    fake.deferred = [];
    await workspace.apply(
      mutation(
        BridgeMutationKind.addTextBlock,
        page: page,
        after: fake.views[page]!.first.id,
        text: 'Two',
      ),
    );
    await tester.pumpAndSettle();
    expect(field(tester, 1).readOnly, isTrue);
    final held = fake.deferred!;
    fake.deferred = null;
    for (final release in held) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(field(tester, 1).readOnly, isFalse);
    expect(field(tester, 1).controller!.text, 'Two');
  });

  testWidgets('numbers count items scrolled out of view', (tester) async {
    await show(tester, [
      for (var item = 1; item <= 150; item++) 'Item $item',
    ], BridgeTextStyle.numbered);
    final scroll = tester.state<ScrollableState>(
      find
          .descendant(
            of: find.byType(CustomScrollView),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    for (var round = 0; round < 5; round++) {
      scroll.position.jumpTo(scroll.position.maxScrollExtent);
      await tester.pumpAndSettle();
    }
    expect(find.text('Item 150'), findsOneWidget);
    expect(find.text('1.'), findsNothing);
    expect(find.text('150.'), findsOneWidget);
  });

  testWidgets('typing while ⌫ joins lands in the joined Text Block', (
    tester,
  ) async {
    await show(tester, ['Five days', ' in Lisbon']);
    await caret(tester, 1, 0);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    await type(tester, 1, '!');
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['Five days! in Lisbon']);
    expect(focused(tester, 0), isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 10);
  });

  testWidgets('typing after ⏎ survives leaving the Page at once', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 0, 'Two');
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['One', 'Two']);
  });

  testWidgets('the first line keeps typing while its text is handed over', (
    tester,
  ) async {
    await show(tester);
    final made = fake.gate = Completer();
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    await tester.enterText(find.byKey(const Key('first-line')), 'ab');
    final handed = Completer<void>();
    made.complete();
    fake.gate = handed;
    await tester.pump();
    // Its Text Block exists, so the line is out of sight but still takes the keys.
    await tester.enterText(
      find.byKey(const Key('first-line'), skipOffstage: false),
      'abc',
    );
    handed.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['abc']);
    expect(field(tester, 0).controller!.text, 'abc');
  });

  testWidgets('a shortcut typed in the first line formats it', (tester) async {
    await show(tester);
    fake.gate = Completer();
    await tester.enterText(find.byKey(const Key('first-line')), '-');
    await tester.enterText(find.byKey(const Key('first-line')), '- ');
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(styleOf(0), BridgeTextStyle.bullet);
    expect(fake.textsOf(page), ['']);
  });

  testWidgets('⏎ in the title confirms a composition there', (tester) async {
    await show(tester, ['One']);
    await tester.tap(find.byKey(const Key('page-title')));
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pump();
    await key(tester, LogicalKeyboardKey.enter);
    expect(focused(tester, 0), isFalse);
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('page-title')))
          .focusNode!
          .hasFocus,
      isTrue,
    );
  });

  testWidgets('a selection deleted before the first Block is made goes', (
    tester,
  ) async {
    await show(tester);
    fake.gate = Completer();
    await tester.enterText(find.byKey(const Key('first-line')), 'abc');
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'abc',
        selection: TextSelection(baseOffset: 0, extentOffset: 3),
      ),
    );
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['']);
  });

  testWidgets('a shortcut typed while ⏎ waits formats the new Text Block', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 0, '-');
    await type(tester, 0, ' ');
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['One', '']);
    expect(styleOf(1), BridgeTextStyle.bullet);
  });

  testWidgets('⏎ that ends a composition in the first line splits nothing', (
    tester,
  ) async {
    await show(tester);
    fake.gate = Completer();
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ani',
        selection: TextSelection.collapsed(offset: 3),
        composing: TextRange(start: 1, end: 3),
      ),
    );
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'a你',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['a你']);
  });

  testWidgets('quitting takes a composition still open as it stands', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    final at = field(tester, 0).controller!.selection.baseOffset;
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: 'Oneni',
        selection: TextSelection.collapsed(offset: at + 2),
        composing: TextRange(start: at, end: at + 2),
      ),
    );
    await tester.pump();
    var drained = false;
    unawaited(workspace.drain().then((_) => drained = true));
    fake.gate!.complete();
    await tester.pump();
    await tester.pump();
    expect(drained, isTrue);
    expect(fake.textsOf(page), ['One', 'ni']);
  });

  testWidgets('⏎ near the start of a long paragraph moves the caret down', (
    tester,
  ) async {
    await show(tester, ['Lisbon ${'and more ' * 600}']);
    // Taller than the window, so a tap could miss it.
    field(tester, 0).focusNode!.requestFocus();
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 6,
    );
    await tester.pump();
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    fake.gate!.complete();
    for (var frame = 0; frame < 30; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(fake.textsOf(page).first, 'Lisbon');
    expect(focused(tester, 1), isTrue);
  });

  testWidgets('a heading made a paragraph by ⌫ joins at the next ⌫', (
    tester,
  ) async {
    await show(tester, ['Plan', 'Before we go']);
    await workspace.apply(
      mutation(
        BridgeMutationKind.setTextStyle,
        page: page,
        block: fake.views[page]![1].block,
        style: BridgeTextStyle.heading2,
      ),
    );
    await tester.pumpAndSettle();
    await caret(tester, 1, 0);
    fake.delay = () => const Duration(milliseconds: 30);
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    for (var frame = 0; frame < 30; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(styleOf(1), BridgeTextStyle.paragraph);
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    for (var frame = 0; frame < 30; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(fake.textsOf(page), ['PlanBefore we go']);
  });

  testWidgets('quitting writes a composition still open in a Text Block', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'Oneni',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 3, end: 5),
      ),
    );
    await tester.pump();
    var drained = false;
    unawaited(workspace.drain().then((_) => drained = true));
    await tester.pump();
    expect(drained, isTrue);
    expect(fake.textsOf(page), ['Oneni']);
  });

  testWidgets('quitting writes a composition still open in the first line', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('first-line')));
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.pump();
    var drained = false;
    unawaited(workspace.drain().then((_) => drained = true));
    await tester.pump();
    await tester.pump();
    expect(drained, isTrue);
    expect(fake.textsOf(page), ['ni']);
  });

  testWidgets('⌫ replayed after ⏎ deletes a whole character', (tester) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    fake.gate = Completer();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await type(tester, 0, 'e\u0301');
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.textsOf(page), ['One', '']);
  });

  testWidgets(
    '⌘⌫ in the first line deletes as the field does while its Block is made',
    (tester) async {
      await show(tester);
      fake.gate = Completer();
      await tester.enterText(find.byKey(const Key('first-line')), 'abc');
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      fake.gate!.complete();
      await tester.pumpAndSettle();
      expect(fake.textsOf(page), ['']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );
}
