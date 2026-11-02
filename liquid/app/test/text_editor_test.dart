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

  /// Opens a Page holding `texts` as paragraphs.
  Future<void> show(
    WidgetTester tester, [
    List<String> texts = const [],
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
}
