import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_test_robots/flutter_test_robots.dart';
import 'package:section_spike/binding.dart';
import 'package:section_spike/core.dart';
import 'package:section_spike/own/editor.dart';
import 'package:section_spike/own/model.dart';

final mac = TargetPlatformVariant.only(TargetPlatform.macOS);

SectionModel modelOf(List<String> paragraphs, [List<Map<String, Object?>> extra = const []]) {
  final core = SectionCore();
  final ops = <Map<String, Object?>>[];
  var at = 1;
  for (var i = 0; i < paragraphs.length; i++) {
    if (i > 0) ops.add({'op': 'split', 'at': at++, 'type': 'paragraph'});
    ops.add({'op': 'splice', 'at': at, 'ins': paragraphs[i]});
    at += paragraphs[i].length;
  }
  core.apply([...ops, ...extra]);
  return SectionModel(core);
}

Future<SectionEditorState> show(WidgetTester tester, SectionModel model) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SectionEditor(key: ObjectKey(model), model: model),
      ),
    ),
  );
  final editor = tester.state<SectionEditorState>(find.byType(SectionEditor));
  editor.focus.requestFocus();
  await tester.pump();
  return editor;
}

Future<void> place(WidgetTester tester, SectionEditorState editor, int para, int offset) async {
  editor.select((para: para, offset: offset));
  await tester.pump();
}

Offset globalAt(SectionEditorState editor, int para, int offset) {
  final view = editor.views[para]!;
  final rp = view.renderParagraph!;
  final position = TextPosition(offset: view.toRendered(offset, true));
  final local = rp.getOffsetForCaret(position, Rect.zero) + Offset(1, rp.getFullHeightForCaret(position) / 2);
  return rp.localToGlobal(local);
}

Future<void> type(WidgetTester tester, SectionEditorState editor, String text) async {
  await tester.ime.typeText(text, getter: () => editor);
  await tester.pump();
}

Future<void> key(WidgetTester tester, LogicalKeyboardKey key, {bool shift = false, bool meta = false}) async {
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft, platform: 'macos');
  if (meta) await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
  await tester.sendKeyEvent(key, platform: 'macos');
  if (meta) await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft, platform: 'macos');
  await tester.pump();
}

List<String> paragraphsOf(SectionCore core) {
  final out = <String>[];
  for (final span in core.spans()) {
    if (span['block'] != null) {
      out.add('');
    } else {
      out[out.length - 1] += span['text'] as String;
    }
  }
  return out;
}

List<String> markersOf(SectionCore core) => [
  for (final span in core.spans())
    if (span['block'] case final String id) id,
];

void expectInStep(SectionModel m, [String reason = '']) =>
    expect(m.shape(), SectionBinding.coreShape(m.core.spans()), reason: reason);

String shown(SectionEditorState editor, int para) => editor.views[para]!.renderParagraph!.text.toPlainText();

void main() {
  testWidgets('⏎ splits by a marker and the upper Paragraph keeps its own', (tester) async {
    final m = modelOf(['Hello world']);
    final first = markersOf(m.core).single;
    final editor = await show(tester, m);
    await place(tester, editor, 0, 5);
    await key(tester, LogicalKeyboardKey.enter);
    expect(paragraphsOf(m.core), ['Hello', ' world']);
    expect(markersOf(m.core).first, first);
    expectInStep(m);
  }, variant: mac);

  testWidgets('⏎ at a Paragraph start adds an empty one above and keeps the marker', (tester) async {
    final m = modelOf(['Hello']);
    final first = markersOf(m.core).single;
    final editor = await show(tester, m);
    await place(tester, editor, 0, 0);
    await key(tester, LogicalKeyboardKey.enter);
    expect(paragraphsOf(m.core), ['', 'Hello']);
    expect(markersOf(m.core).last, first);
    expectInStep(m);
  }, variant: mac);

  testWidgets('⌫ at a Paragraph start removes its marker', (tester) async {
    final m = modelOf(['ab', 'cd']);
    final first = markersOf(m.core).first;
    final editor = await show(tester, m);
    await place(tester, editor, 1, 0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(paragraphsOf(m.core), ['abcd']);
    expect(markersOf(m.core), [first]);
    expectInStep(m);
  }, variant: mac);

  testWidgets('⇧⏎ breaks a line inside the Paragraph', (tester) async {
    final m = modelOf(['abcd']);
    final editor = await show(tester, m);
    await place(tester, editor, 0, 2);
    await key(tester, LogicalKeyboardKey.enter, shift: true);
    expect(paragraphsOf(m.core), ['ab\ncd']);
    expectInStep(m);
  }, variant: mac);

  testWidgets('a keyboard selection crosses Paragraphs and typing replaces it', (tester) async {
    final m = modelOf(['ab', 'cd']);
    final editor = await show(tester, m);
    await place(tester, editor, 0, 1);
    for (var i = 0; i < 3; i++) {
      await key(tester, LogicalKeyboardKey.arrowRight, shift: true);
    }
    expect(editor.selection.value.base.para, isNot(editor.selection.value.extent.para));
    await type(tester, editor, 'X');
    expect(paragraphsOf(m.core), ['aXd']);
    expectInStep(m);
  }, variant: mac);

  testWidgets('a mouse drag selects from one Paragraph into the next', (tester) async {
    final m = modelOf(['first line here', 'second line here']);
    final editor = await show(tester, m);
    final gesture = await tester.startGesture(globalAt(editor, 0, 6), kind: PointerDeviceKind.mouse);
    await tester.pump();
    await gesture.moveTo(globalAt(editor, 1, 6));
    await tester.pump();
    await gesture.up();
    final s = editor.selection.value;
    expect((s.base, s.extent), ((para: 0, offset: 6), (para: 1, offset: 6)));
  }, variant: mac);

  testWidgets('⌘B marks the core, typing inside stays bold, and → at the edge types plain', (tester) async {
    final m = modelOf(['make bold']);
    final editor = await show(tester, m);
    editor.select((para: 0, offset: 5), (para: 0, offset: 9));
    await key(tester, LogicalKeyboardKey.keyB, meta: true);
    expectInStep(m);
    await place(tester, editor, 0, 9);
    await type(tester, editor, 'er');
    expect(m.paras[0].marksAt(10), {'bold': true});
    await key(tester, LogicalKeyboardKey.arrowRight);
    await type(tester, editor, ' plain');
    expectInStep(m);
    final runs = m.core.spans().where((s) => s['text'] != null).toList();
    expect(runs.last['text'], ' plain');
    expect(runs.last['marks'], isEmpty);
  }, variant: mac);

  testWidgets('the styled word at the caret shows its Markdown, and only it', (tester) async {
    final m = modelOf(
      ['This is important for all'],
      [
        {'op': 'mark', 'start': 9, 'end': 18, 'name': 'bold'},
      ],
    );
    final editor = await show(tester, m);
    await place(tester, editor, 0, 2);
    expect(shown(editor, 0), 'This is important for all');
    await place(tester, editor, 0, 12);
    expect(shown(editor, 0), 'This is **important** for all');
    expect(m.paras[0].text, 'This is important for all');
  }, variant: mac);

  testWidgets('text typed after a link is not part of it', (tester) async {
    final m = modelOf(
      ['see docs'],
      [
        {'op': 'mark', 'start': 5, 'end': 9, 'name': 'link', 'value': 'https://example.com'},
      ],
    );
    final editor = await show(tester, m);
    await place(tester, editor, 0, 8);
    await type(tester, editor, ' now');
    expectInStep(m);
    final runs = m.core.spans().where((s) => s['text'] != null).toList();
    expect(runs.last['text'], ' now');
    expect(runs.last['marks'], isEmpty);
  }, variant: mac);

  testWidgets('a split on one device and an edit on another merge into the right Paragraphs', (tester) async {
    final a = modelOf(['Hello world']);
    final b = SectionModel(a.core.fork());
    var editor = await show(tester, a);
    await place(tester, editor, 0, 5);
    await key(tester, LogicalKeyboardKey.enter);
    editor = await show(tester, b);
    await place(tester, editor, 0, 11);
    await type(tester, editor, '!');
    a.core.merge(b.core);
    b.core.merge(a.core);
    a.reload();
    b.reload();
    expect(paragraphsOf(a.core), ['Hello', ' world!']);
    expectInStep(a);
    expectInStep(b);
  }, variant: mac);

  testWidgets('Japanese composition and a dead key reach the core only once committed', (tester) async {
    final m = modelOf(['ab']);
    final editor = await show(tester, m);
    await place(tester, editor, 0, 2);
    await tester.ime.sendDeltas([
      const TextEditingDeltaInsertion(
        oldText: 'ab',
        textInserted: 'か',
        insertionOffset: 2,
        selection: TextSelection.collapsed(offset: 3),
        composing: TextRange(start: 2, end: 3),
      ),
    ], getter: () => editor);
    await tester.pump();
    expect(paragraphsOf(m.core), ['ab']);
    expect(shown(editor, 0), 'abか');
    await tester.ime.sendDeltas([
      const TextEditingDeltaReplacement(
        oldText: 'abか',
        replacementText: '漢',
        replacedRange: TextRange(start: 2, end: 3),
        selection: TextSelection.collapsed(offset: 3),
        composing: TextRange.empty,
      ),
    ], getter: () => editor);
    await tester.pump();
    expect(paragraphsOf(m.core), ['ab漢']);
    await tester.ime.sendDeltas([
      const TextEditingDeltaInsertion(
        oldText: 'ab漢',
        textInserted: '´',
        insertionOffset: 3,
        selection: TextSelection.collapsed(offset: 4),
        composing: TextRange(start: 3, end: 4),
      ),
      const TextEditingDeltaReplacement(
        oldText: 'ab漢´',
        replacementText: 'é',
        replacedRange: TextRange(start: 3, end: 4),
        selection: TextSelection.collapsed(offset: 4),
        composing: TextRange.empty,
      ),
    ], getter: () => editor);
    await tester.pump();
    expect(paragraphsOf(m.core), ['ab漢é']);
    expectInStep(m);
  }, variant: mac);

  testWidgets('a Section of 1,000 Paragraphs builds only what is visible', (tester) async {
    final m = modelOf([for (var i = 0; i < 1000; i++) 'Paragraph $i']);
    final editor = await show(tester, m);
    expect(editor.views.length, lessThan(60));
  }, variant: mac);

  const steps = int.fromEnvironment('FUZZ_STEPS', defaultValue: 200);
  const seed = int.fromEnvironment('FUZZ_SEED', defaultValue: 7);
  testWidgets('$steps random edits keep the editor and the core in step', (tester) async {
    final random = Random(seed);
    final m = modelOf(['alpha beta', 'gamma delta', 'epsilon']);
    final editor = await show(tester, m);
    final log = <Object>[];
    const dump = String.fromEnvironment('FUZZ_DUMP');
    for (var step = 0; step < steps; step++) {
      final index = random.nextInt(m.paras.length);
      final offset = random.nextInt(m.paras[index].text.length + 1);
      final action = random.nextInt(9);
      final before = m.shape();
      m.opLog.clear();
      await place(tester, editor, index, offset);
      switch (action) {
        case 0:
          await key(tester, LogicalKeyboardKey.enter);
        case 1:
          await key(tester, LogicalKeyboardKey.backspace);
        case 2:
          await key(tester, LogicalKeyboardKey.enter, shift: true);
        case 3:
          for (var i = random.nextInt(12); i >= 0; i--) {
            await key(tester, LogicalKeyboardKey.arrowRight, shift: true);
          }
          await key(tester, LogicalKeyboardKey.backspace);
        case 4:
          await key(tester, LogicalKeyboardKey.arrowRight, shift: true);
          await key(tester, LogicalKeyboardKey.arrowRight, shift: true);
          await key(tester, LogicalKeyboardKey.keyB, meta: true);
        case 5:
          await key(tester, LogicalKeyboardKey.delete);
        case 6:
          await key(tester, LogicalKeyboardKey.arrowRight);
          await type(tester, editor, ['x', 'yz', ' ', 'wörd', '😀'][random.nextInt(5)]);
        case 7:
          for (var i = random.nextInt(8); i >= 0; i--) {
            await key(tester, LogicalKeyboardKey.arrowRight, shift: true);
          }
          await key(tester, LogicalKeyboardKey.backslash, meta: true);
        default:
          await type(tester, editor, ['x', 'yz', ' ', 'wörd', '😀'][random.nextInt(5)]);
      }
      log.add(List.of(m.opLog));
      if (dump.isNotEmpty) File(dump).writeAsStringSync(jsonEncode(log));
      expectInStep(m, 'step $step: action $action at $index:$offset, before $before, ops ${m.opLog}');
    }
  }, variant: mac);
}
