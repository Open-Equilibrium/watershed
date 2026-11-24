import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Future<void> show(
    WidgetTester tester, [
    List<String> texts = const [],
    BridgeTextStyle style = BridgeTextStyle.paragraph,
    ShortcutSettings? settings,
  ]) => kit.show(tester, [
    for (final text in texts) TextPart(text, style),
  ], shortcuts: ShortcutController.memory(settings));

  Future<void> caret(WidgetTester tester, int index, int offset) async {
    await tester.tap(block(index));
    await tester.pump();
    field(tester, index).controller!.selection = TextSelection.collapsed(
      offset: offset,
    );
    await tester.pump();
  }

  Future<void> chord(
    WidgetTester tester,
    LogicalKeyboardKey modifier,
    LogicalKeyboardKey key,
  ) async {
    await tester.sendKeyDownEvent(modifier, platform: 'macos');
    await tester.sendKeyEvent(key, platform: 'macos');
    await tester.sendKeyUpEvent(modifier, platform: 'macos');
    await tester.pumpAndSettle();
  }

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

  Future<void> frames(WidgetTester tester, int count) async {
    for (var frame = 0; frame < count; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
  }

  Finder pageScroll() => find
      .descendant(
        of: find.byType(CustomScrollView),
        matching: find.byType(Scrollable),
      )
      .first;

  void ime(WidgetTester tester, String text, [TextRange? composing]) =>
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
          composing: composing ?? TextRange.empty,
        ),
      );

  Future<void> release(
    WidgetTester tester, [
    Set<BridgeQueryKind> kinds = const {},
  ]) async {
    kit.fake.deferredKinds = kinds;
    final held = [...kit.fake.deferred!];
    kit.fake.deferred!.clear();
    for (final query in held) {
      query.complete();
    }
    await tester.pumpAndSettle();
  }

  void hold() => kit.fake.gate = Completer();

  Future<void> enter(WidgetTester tester) async {
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
  }

  Future<void> finishWrites(WidgetTester tester) async {
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
  }

  bool focused(WidgetTester tester, int index) =>
      field(tester, index).focusNode!.hasFocus;

  BridgeTextStyle styleOf(int index) =>
      kit.fake.styles[kit.fake.views[kit.page]![index].block]!;

  List<BridgeTextStyle> written(int index) {
    final block = kit.fake.views[kit.page]![index].block;
    return [
      for (final change in kit.fake.applied)
        if (change.kind == BridgeMutationKind.setTextStyle &&
            change.block == block)
          change.style,
    ];
  }

  testWidgets('an empty Page makes its first Text Block', (tester) async {
    await show(tester);
    expect(find.text('Type to write.'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('first-line')), 'F');
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['F']);
    expect(focused(tester, 0), isTrue);

    await type(tester, 0, 'ive days');
    expect(kit.fake.textsOf(kit.page), ['Five days']);
    expect(field(tester, 0).controller!.text, 'Five days');
  });

  testWidgets('⏎ splits a Text Block and moves the caret down', (tester) async {
    await show(tester, ['Lisbon Porto']);
    await caret(tester, 0, 6);
    await key(tester, LogicalKeyboardKey.enter);
    expect(kit.fake.textsOf(kit.page), ['Lisbon', ' Porto']);
    expect(focused(tester, 1), isTrue);
    expect(field(tester, 1).controller!.selection.baseOffset, 0);

    await key(tester, LogicalKeyboardKey.enter);
    expect(kit.fake.textsOf(kit.page), ['Lisbon', '', ' Porto']);
    expect(focused(tester, 2), isTrue);
  });

  testWidgets('⌫ at the start joins the Text Block above', (tester) async {
    await show(tester, ['Five days', ' in Lisbon']);
    await caret(tester, 1, 0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(kit.fake.textsOf(kit.page), ['Five days in Lisbon']);
    expect(focused(tester, 0), isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 9);
  });

  testWidgets('⌘Z after ⌫ joins does not restore the Block', (tester) async {
    await show(tester, ['Hello', ' world']);
    await caret(tester, 1, 0);
    await tester.pump(const Duration(milliseconds: 600));
    await key(tester, LogicalKeyboardKey.backspace);
    expect(kit.fake.textsOf(kit.page), ['Hello world']);
    await tester.pump(const Duration(milliseconds: 600));
    await undo(tester);
    expect(kit.fake.textsOf(kit.page), ['Hello world']);
    expect(field(tester, 0).controller!.text, 'Hello world');
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('⌘Z after ⏎ and ↑ keeps the first Block whole', (tester) async {
    await show(tester, ['Hello world']);
    await caret(tester, 0, 5);
    await tester.pump(const Duration(milliseconds: 600));
    await key(tester, LogicalKeyboardKey.enter);
    expect(kit.fake.textsOf(kit.page), ['Hello', ' world']);
    await tester.pump(const Duration(milliseconds: 600));
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(focused(tester, 0), isTrue);
    await undo(tester);
    expect(kit.fake.textsOf(kit.page), ['Hello', ' world']);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('⌘Z after a Markdown shortcut keeps the heading', (tester) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    await tester.pump(const Duration(milliseconds: 600));
    await type(tester, 0, 'Title');
    await tester.pump(const Duration(milliseconds: 600));
    await caret(tester, 0, 0);
    await type(tester, 0, '# ');
    expect(styleOf(0), BridgeTextStyle.heading1);
    await undo(tester);
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(kit.fake.textsOf(kit.page), ['Title']);
    expect(field(tester, 0).controller!.text, 'Title');
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('⌘Z in a Text Block undoes through History, not the field', (
    tester,
  ) async {
    await show(tester, ['Hello', ' world']);
    await caret(tester, 1, 0);
    await tester.pump(const Duration(milliseconds: 600));
    await key(tester, LogicalKeyboardKey.backspace);
    await tester.pump(const Duration(milliseconds: 600));
    await type(tester, 0, 'x');
    await tester.pump(const Duration(milliseconds: 600));
    await undo(tester);
    expect(kit.fake.undos, 1);
    expect(kit.fake.textsOf(kit.page), ['Hellox world']);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets(
    'typing before the next frame after a shortcut keeps what is typed',
    (tester) async {
      await show(tester, ['']);
      await caret(tester, 0, 0);
      ime(tester, '- ');
      ime(tester, 'P');
      await tester.pumpAndSettle();
      expect(styleOf(0), BridgeTextStyle.bullet);
      expect(kit.fake.textsOf(kit.page), ['P']);
      expect(field(tester, 0).controller!.text, 'P');
    },
  );

  testWidgets('Markdown shortcuts; lists end when left empty', (tester) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    await type(tester, 0, '#');
    await type(tester, 0, ' ');
    expect(styleOf(0), BridgeTextStyle.heading1);
    expect(kit.fake.textsOf(kit.page), ['']);
    await type(tester, 0, 'Plan');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(1), BridgeTextStyle.paragraph);

    await type(tester, 1, '- ');
    expect(styleOf(1), BridgeTextStyle.bullet);
    await type(tester, 1, 'Passport');
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(2), BridgeTextStyle.bullet);
    await key(tester, LogicalKeyboardKey.enter);
    expect(styleOf(2), BridgeTextStyle.paragraph);
    expect(kit.fake.textsOf(kit.page), ['Plan', 'Passport', '']);

    await caret(tester, 1, 0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(styleOf(1), BridgeTextStyle.paragraph);
    expect(kit.fake.textsOf(kit.page), ['Plan', 'Passport', '']);
  });

  testWidgets('numbers count, and a to-do ticks by its box', (tester) async {
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

  testWidgets('⌘⏎ ticks and unticks the to-do at the caret', (tester) async {
    await show(tester, ['Pack', 'Fly']);
    await caret(tester, 0, 0);
    await type(tester, 0, '[] ');
    expect(styleOf(0), BridgeTextStyle.todo);

    await chord(tester, LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.done);
    expect(kit.fake.textsOf(kit.page), [
      'Pack',
      'Fly',
    ], reason: 'nothing is split');

    await chord(tester, LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.todo);
    expect(written(0), [
      BridgeTextStyle.todo,
      BridgeTextStyle.done,
      BridgeTextStyle.todo,
    ], reason: 'each tick is written');
  });

  testWidgets('⇪⏎ ticks a to-do without splitting it', (tester) async {
    await show(
      tester,
      ['Pack'],
      BridgeTextStyle.todo,
      ShortcutSettings.liquidKey(),
    );
    await caret(tester, 0, 4);

    await chord(tester, liquidKey, LogicalKeyboardKey.enter);
    expect(styleOf(0), BridgeTextStyle.done);
    expect(kit.fake.textsOf(kit.page), ['Pack']);
  });

  testWidgets('--- starts a Section, and ⌫ there removes it', (tester) async {
    await show(tester, ['Plan', '']);
    await caret(tester, 1, 0);
    await type(tester, 1, '---');
    expect(kit.fake.views[kit.page]![1].boundary, isTrue);
    expect(kit.fake.textsOf(kit.page), ['Plan', '']);
    expect(find.byKey(const Key('section-divider')), findsOneWidget);

    await key(tester, LogicalKeyboardKey.backspace);
    expect(kit.fake.views[kit.page]![1].boundary, isFalse);
    expect(find.byKey(const Key('section-divider')), findsNothing);
    expect(kit.fake.textsOf(kit.page), ['Plan', '']);
  });

  testWidgets('⏎ in the title and arrows move between Blocks', (tester) async {
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
    kit.fake.refuse = 'the replica is read-only';
    await type(tester, 0, '!');
    expect(field(tester, 0).controller!.text, 'Lisbon');
    expect(
      find.text('Liquid did not save this change: the replica is read-only'),
      findsOneWidget,
    );
    expect(
      tester.widget<WarningNotice>(find.byType(WarningNotice)).danger,
      isTrue,
      reason: 'a failure shows as one',
    );
  });

  testWidgets('typing right after ⏎ lands in the new Text Block', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    hold();
    await enter(tester);
    await type(tester, 0, 'Two');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['One', 'Two']);
    expect(focused(tester, 1), isTrue);
    expect(field(tester, 1).controller!.text, 'Two');
    expect(field(tester, 0).controller!.text, 'One');
  });

  testWidgets('the first line hands over all typed while its Block is made', (
    tester,
  ) async {
    await show(tester);
    hold();
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    await tester.enterText(find.byKey(const Key('first-line')), 'b');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['b']);
    expect(field(tester, 0).controller!.text, 'b');
  });

  testWidgets('the first line waits until a composition ends', (tester) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('first-line')));
    await tester.pump();
    ime(tester, 'ni', const TextRange(start: 0, end: 2));
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), isEmpty);
    ime(tester, '你');
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['你']);
  });

  testWidgets('a read that crossed an edit shows no old text', (tester) async {
    await show(tester, ['']);
    await caret(tester, 0, 0);
    hold();
    kit.fake.deferredKinds = {BridgeQueryKind.text};
    kit.fake.deferred = [];
    await type(tester, 0, 'a');
    await type(tester, 0, 'b');
    kit.fake.gate!.complete();
    await tester.pump();
    await tester.pump();
    final held = kit.fake.deferred!;
    kit.fake.deferred = null;
    for (final release in held.reversed) {
      release.complete();
      await tester.pump();
    }
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['ab']);
    expect(field(tester, 0).controller!.text, 'ab');
  });

  testWidgets('a Text Block takes no typing before its text is read', (
    tester,
  ) async {
    await show(tester, ['One']);
    kit.fake.deferredKinds = {BridgeQueryKind.text};
    kit.fake.deferred = [];
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addTextBlock,
        page: kit.page,
        after: kit.fake.views[kit.page]!.first.id,
        text: 'Two',
      ),
    );
    await tester.pumpAndSettle();
    expect(field(tester, 1).readOnly, isTrue);
    final held = kit.fake.deferred!;
    kit.fake.deferred = null;
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
    final scroll = tester.state<ScrollableState>(pageScroll());
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
    hold();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    await type(tester, 1, '!');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['Five days! in Lisbon']);
    expect(focused(tester, 0), isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 10);
  });

  testWidgets('typing after ⏎ survives leaving the Page at once', (
    tester,
  ) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    hold();
    await enter(tester);
    await type(tester, 0, 'Two');
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['One', 'Two']);
  });

  testWidgets('the first line keeps typing while handed over', (tester) async {
    await show(tester);
    hold();
    final made = kit.fake.gate!;
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    await tester.enterText(find.byKey(const Key('first-line')), 'ab');
    final handed = Completer<void>();
    made.complete();
    kit.fake.gate = handed;
    await tester.pump();
    await tester.enterText(
      find.byKey(const Key('first-line'), skipOffstage: false),
      'abc',
    );
    handed.complete();
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['abc']);
    expect(field(tester, 0).controller!.text, 'abc');
  });

  testWidgets('a shortcut typed in the first line formats it', (tester) async {
    await show(tester);
    hold();
    await tester.enterText(find.byKey(const Key('first-line')), '-');
    await tester.enterText(find.byKey(const Key('first-line')), '- ');
    await finishWrites(tester);
    expect(styleOf(0), BridgeTextStyle.bullet);
    expect(kit.fake.textsOf(kit.page), ['']);
  });

  testWidgets('⏎ in the title confirms a composition there', (tester) async {
    await show(tester, ['One']);
    await tester.tap(find.byKey(const Key('page-title')));
    await tester.pump();
    ime(tester, 'ni', const TextRange(start: 0, end: 2));
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

  testWidgets('a selection deleted before the Block is made', (tester) async {
    await show(tester);
    hold();
    await tester.enterText(find.byKey(const Key('first-line')), 'abc');
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'abc',
        selection: TextSelection(baseOffset: 0, extentOffset: 3),
      ),
    );
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['']);
  });

  testWidgets('a shortcut typed while ⏎ waits formats it', (tester) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    hold();
    await enter(tester);
    await type(tester, 0, '-');
    await type(tester, 0, ' ');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['One', '']);
    expect(styleOf(1), BridgeTextStyle.bullet);
  });

  testWidgets('⏎ ending a composition splits nothing', (tester) async {
    await show(tester);
    hold();
    await tester.enterText(find.byKey(const Key('first-line')), 'a');
    ime(tester, 'ani', const TextRange(start: 1, end: 3));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    ime(tester, 'a你');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['a你']);
  });

  testWidgets('quitting takes an open composition as is', (tester) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    hold();
    await enter(tester);
    final at = field(tester, 0).controller!.selection.baseOffset;
    ime(tester, 'Oneni', TextRange(start: at, end: at + 2));
    await tester.pump();
    var drained = false;
    unawaited(kit.workspace.drain().then((_) => drained = true));
    kit.fake.gate!.complete();
    await tester.pump();
    await tester.pump();
    expect(drained, isTrue);
    expect(kit.fake.textsOf(kit.page), ['One', 'ni']);
  });

  testWidgets('⏎ near the start of a long paragraph moves the caret down', (
    tester,
  ) async {
    await show(tester, ['Lisbon ${'and more ' * 600}']);
    field(tester, 0).focusNode!.requestFocus();
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 6,
    );
    await tester.pump();
    hold();
    await enter(tester);
    kit.fake.gate!.complete();
    await frames(tester, 30);
    expect(kit.fake.textsOf(kit.page).first, 'Lisbon');
    expect(focused(tester, 1), isTrue);
  });

  testWidgets('a heading made a paragraph by ⌫ joins at the next ⌫', (
    tester,
  ) async {
    await show(tester, ['Plan', 'Before we go']);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTextStyle,
        page: kit.page,
        block: kit.fake.views[kit.page]![1].block,
        style: BridgeTextStyle.heading2,
      ),
    );
    await tester.pumpAndSettle();
    await caret(tester, 1, 0);
    kit.fake.delay = () => const Duration(milliseconds: 30);
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await frames(tester, 30);
    expect(styleOf(1), BridgeTextStyle.paragraph);
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await frames(tester, 30);
    expect(kit.fake.textsOf(kit.page), ['PlanBefore we go']);
  });

  Future<void> openBlockComposition(WidgetTester tester) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    ime(tester, 'Oneni', const TextRange(start: 3, end: 5));
    await tester.pump();
  }

  Future<void> openFirstLineComposition(WidgetTester tester) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('first-line')));
    await tester.pump();
    ime(tester, 'ni', const TextRange(start: 0, end: 2));
    await tester.pump();
  }

  Future<void> quit(WidgetTester tester) async {
    var drained = false;
    unawaited(kit.workspace.drain().then((_) => drained = true));
    await tester.pump();
    await tester.pump();
    expect(drained, isTrue);
  }

  Future<void> leave(WidgetTester tester) async {
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
  }

  for (final (start, open, text) in [
    ('a Block', openBlockComposition, 'Oneni'),
    ('the first line', openFirstLineComposition, 'ni'),
  ]) {
    for (final (exit, go) in [
      ('quitting', quit),
      ('leaving the Page', leave),
    ]) {
      testWidgets('$exit writes an open composition in $start', (tester) async {
        await open(tester);
        await go(tester);
        expect(kit.fake.textsOf(kit.page), [text]);
      });
    }
  }

  testWidgets('⌫ replayed after ⏎ deletes a whole character', (tester) async {
    await show(tester, ['One']);
    await caret(tester, 0, 3);
    hold();
    await enter(tester);
    await type(tester, 0, 'e\u0301');
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['One', '']);
  });

  testWidgets(
    '⌘⌫ in the first line deletes as the field does while its Block is made',
    (tester) async {
      await show(tester);
      hold();
      await tester.enterText(find.byKey(const Key('first-line')), 'abc');
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await finishWrites(tester);
      expect(kit.fake.textsOf(kit.page), ['']);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets('another Text Block edits as usual while ⏎ waits', (
    tester,
  ) async {
    await show(tester, ['A', 'Bc']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await caret(tester, 1, 2);
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    await enter(tester);
    kit.fake.gate!.complete();
    await frames(tester, 60);
    expect(kit.fake.textsOf(kit.page), ['A', '', 'B', '']);
  });

  testWidgets('the new Text Block keeps its list style before it reads it', (
    tester,
  ) async {
    await show(tester, ['Pack'], BridgeTextStyle.bullet);
    await caret(tester, 0, 4);
    kit.fake.deferredKinds = {BridgeQueryKind.text};
    kit.fake.deferred = [];
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await frames(tester, 10);
    expect(focused(tester, 1), isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await frames(tester, 10);
    final held = kit.fake.deferred!;
    kit.fake.deferred = null;
    for (final release in held) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['Pack', '']);
    expect(styleOf(1), BridgeTextStyle.paragraph);
  });

  testWidgets('typing after ⏎ in another Text Block lands after its split', (
    tester,
  ) async {
    await show(tester, ['A', 'BC']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await caret(tester, 1, 1);
    await enter(tester);
    await type(tester, 1, 'x');
    kit.fake.gate!.complete();
    await frames(tester, 60);
    expect(kit.fake.textsOf(kit.page), ['A', '', 'B', 'xC']);
  });

  testWidgets('--- typed in a list item while ⏎ waits starts a Section', (
    tester,
  ) async {
    await show(tester, ['Item'], BridgeTextStyle.bullet);
    await caret(tester, 0, 4);
    hold();
    await enter(tester);
    await type(tester, 0, '---');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['Item', '']);
    expect(kit.fake.views[kit.page]![1].boundary, isTrue);
  });

  testWidgets('a Page reopened while its queue writes waits', (tester) async {
    await show(tester, ['ABC']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    kit.workspace.open.value = OpenPage(
      kit.workspace.open.value?.area ?? '',
      kit.page,
    );
    await tester.pumpAndSettle();
    expect(field(tester, 0).readOnly, isTrue);
    kit.fake.deferredKinds = {BridgeQueryKind.views};
    kit.fake.deferred = [];
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['A', 'BC']);
    expect(
      field(tester, 0).readOnly,
      isTrue,
      reason: 'until the Views it wrote are read',
    );
    await release(tester, {BridgeQueryKind.text});
    expect(find.byType(TextBlock), findsNWidgets(2));
    expect(field(tester, 0).readOnly, isTrue, reason: 'and then the text');
    await release(tester);
    expect(field(tester, 0).readOnly, isFalse);
    expect(field(tester, 0).controller!.text, 'A');
  });

  testWidgets('a refused edit drops the edits typed after it', (tester) async {
    await show(tester, ['abcd']);
    await caret(tester, 0, 1);
    hold();
    kit.fake.refuseNext = 1;
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'bcd',
        selection: TextSelection.collapsed(offset: 0),
      ),
    );
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 3,
    );
    await type(tester, 0, 'X');
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), [
      'abcd',
    ], reason: 'the X assumed the refused deletion');
    expect(field(tester, 0).controller!.text, 'abcd');

    await caret(tester, 0, 4);
    await type(tester, 0, 'e');
    expect(kit.fake.textsOf(kit.page), ['abcde'], reason: 'later edits go on');
  });

  testWidgets('a refused edit is read again after a failure', (tester) async {
    await show(tester, ['abcd']);
    await caret(tester, 0, 1);
    hold();
    kit.fake.refuseNext = 1;
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'bcd',
        selection: TextSelection.collapsed(offset: 0),
      ),
    );
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 3,
    );
    await type(tester, 0, 'X');
    kit.fake.refuseNextQuery = BridgeQueryKind.text;
    await finishWrites(tester);

    await type(tester, 0, 'Y');
    await type(tester, 0, 'Z');
    expect(kit.fake.textsOf(kit.page), [
      'abcdZ',
    ], reason: 'a key after a refused edit reads the text again');
    expect(field(tester, 0).controller!.text, 'abcdZ');
  });

  testWidgets('a person moved to another Block keeps caret', (tester) async {
    await show(tester, ['A', 'B']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await caret(tester, 1, 1);
    await type(tester, 1, 'x');
    kit.fake.gate!.complete();
    await frames(tester, 60);
    expect(kit.fake.textsOf(kit.page), ['A', '', 'Bx']);
    expect(focused(tester, 2), isTrue);
  });

  testWidgets('a shortcut typed before moving to another Text Block applies', (
    tester,
  ) async {
    await show(tester, ['A', 'C']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await type(tester, 0, '- ');
    await caret(tester, 1, 1);
    await enter(tester);
    kit.fake.gate!.complete();
    await frames(tester, 60);
    expect(kit.fake.textsOf(kit.page), ['A', '', 'C', '']);
    expect(styleOf(1), BridgeTextStyle.bullet);
  });

  testWidgets('a field left while ⏎ waits is written once', (tester) async {
    await show(tester, ['A', 'B']);
    await caret(tester, 0, 1);
    hold();
    await enter(tester);
    await type(tester, 0, 'x');
    await caret(tester, 1, 1);
    await enter(tester);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await finishWrites(tester);
    expect(kit.fake.textsOf(kit.page), ['A', 'x', 'B', '']);
  });

  testWidgets('⌫ far down a long Page reads only nearby Views', (tester) async {
    await show(tester, [
      for (var number = 1; number <= 600; number++) 'Item $number',
    ]);
    await tester.scrollUntilVisible(
      find.text('Item 560'),
      2000,
      scrollable: pageScroll(),
    );
    await tester.pumpAndSettle();
    final at = find
        .byType(TextBlock)
        .evaluate()
        .toList()
        .indexWhere(
          (element) =>
              ((element.widget as TextBlock).view.id ==
              kit.fake.views[kit.page]![559].id),
        );
    await caret(tester, at, 0);
    kit.fake.asked.clear();
    await key(tester, LogicalKeyboardKey.backspace);
    expect(kit.fake.textsOf(kit.page)[558], 'Item 559Item 560');
    final big = kit.fake.asked.where(
      (asked) => asked.kind == BridgeQueryKind.views && asked.limit > 100,
    );
    expect(big, isEmpty, reason: 'the View above is found beside the caret');
  });

  testWidgets('a numbered list longer than the Views stays', (tester) async {
    await show(tester, [
      for (var number = 1; number <= 1600; number++) 'Item $number',
    ], BridgeTextStyle.numbered);
    await tester.scrollUntilVisible(
      find.text('Item 1600'),
      3000,
      scrollable: pageScroll(),
    );
    await tester.pumpAndSettle();
    final reads = kit.fake.asked.length;
    await tester.pumpAndSettle();
    expect(find.text('Item 1600'), findsOneWidget);
    expect(kit.fake.asked.length, reads, reason: 'nothing reads on and on');
  });
}
