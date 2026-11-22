import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:collection/collection.dart';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_test_robots/flutter_test_robots.dart';
import 'package:section_spike/binding.dart';
import 'package:section_spike/core.dart';
import 'package:super_editor/super_editor.dart';
import 'package:super_editor/super_editor_test.dart';

final mac = TargetPlatformVariant.only(TargetPlatform.macOS);

SectionBinding section(List<String> paragraphs, [List<Map<String, Object?>> extra = const []]) {
  final core = SectionCore();
  final ops = <Map<String, Object?>>[];
  var at = 1;
  for (var i = 0; i < paragraphs.length; i++) {
    if (i > 0) ops.add({'op': 'split', 'at': at++, 'type': 'paragraph'});
    ops.add({'op': 'splice', 'at': at, 'ins': paragraphs[i]});
    at += paragraphs[i].length;
  }
  core.apply([...ops, ...extra]);
  return SectionBinding(core);
}

Future<void> show(WidgetTester tester, SectionBinding s) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SuperEditor(key: ObjectKey(s), editor: s.editor),
      ),
    ),
  );
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

void expectInStep(SectionBinding s) =>
    expect(s.editorShape(), SectionBinding.coreShape(s.core.spans()), reason: 'editor and core differ');

String node(SectionBinding s, int index) => s.document.getNodeAt(index)!.id;

void main() {
  testWidgets('⏎ splits by a marker and the upper Paragraph keeps its own', (tester) async {
    final s = section(['Hello world']);
    final first = markersOf(s.core).single;
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 0), 5);
    await tester.pressEnter();
    expect(paragraphsOf(s.core), ['Hello', ' world']);
    expect(markersOf(s.core).first, first);
    expectInStep(s);
  }, variant: mac);

  testWidgets('⌫ at a Paragraph start removes its marker', (tester) async {
    final s = section(['ab', 'cd']);
    final first = markersOf(s.core).first;
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 1), 0);
    await tester.pressBackspace();
    expect(paragraphsOf(s.core), ['abcd']);
    expect(markersOf(s.core), [first]);
    expectInStep(s);
  }, variant: mac);

  testWidgets('⇧⏎ breaks a line inside the Paragraph', (tester) async {
    final s = section(['abcd']);
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 0), 2);
    await tester.pressShiftEnter();
    expect(paragraphsOf(s.core), ['ab\ncd']);
    expectInStep(s);
  }, variant: mac);

  testWidgets('a keyboard selection crosses Paragraphs and typing replaces it', (tester) async {
    final s = section(['ab', 'cd']);
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 0), 1);
    for (var i = 0; i < 3; i++) {
      await tester.pressShiftRightArrow();
    }
    final selection = s.composer.selection!;
    expect(selection.base.nodeId, isNot(selection.extent.nodeId));
    await tester.typeImeText('X');
    expect(paragraphsOf(s.core), ['aXd']);
    expectInStep(s);
  }, variant: mac);

  testWidgets('a mouse drag selects from one Paragraph into the next', (tester) async {
    final s = section(['first line here', 'second line here']);
    await show(tester, s);
    await tester.dragSelectDocumentFromPositionByOffset(
      from: DocumentPosition(nodeId: node(s, 0), nodePosition: const TextNodePosition(offset: 6)),
      delta: const Offset(20, 80),
    );
    final selection = s.composer.selection!;
    expect({selection.base.nodeId, selection.extent.nodeId}, {node(s, 0), node(s, 1)});
  }, variant: mac);

  testWidgets('⌘B marks the core, and bold typed after ⌘B off is plain', (tester) async {
    final s = section(['make bold']);
    await show(tester, s);
    await tester.doubleTapInParagraph(node(s, 0), 6);
    await tester.pressCmdB();
    expectInStep(s);
    await tester.placeCaretInParagraph(node(s, 0), 9);
    await tester.typeImeText('er');
    expectInStep(s);
    await tester.pressCmdB();
    await tester.typeImeText(' plain');
    expectInStep(s);
    final runs = s.core.spans().where((span) => span['text'] != null).toList();
    expect(runs.last['text'], ' plain');
    expect(runs.last['marks'], isEmpty);
  }, variant: mac);

  testWidgets('text typed after a link is not part of it', (tester) async {
    final s = section(
      ['see docs'],
      [
        {'op': 'mark', 'start': 5, 'end': 9, 'name': 'link', 'value': 'https://example.com'},
      ],
    );
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 0), 8);
    await tester.typeImeText(' now');
    expectInStep(s);
    final runs = s.core.spans().where((span) => span['text'] != null).toList();
    expect(runs.last['text'], ' now');
    expect(runs.last['marks'], isEmpty);
  }, variant: mac);

  testWidgets('a split on one device and an edit on another merge into the right Paragraphs', (tester) async {
    final a = section(['Hello world']);
    final b = SectionBinding(a.core.fork());
    await show(tester, a);
    await tester.placeCaretInParagraph(node(a, 0), 5);
    await tester.pressEnter();
    await show(tester, b);
    await tester.placeCaretInParagraph(node(b, 0), 11);
    await tester.typeImeText('!');
    a.core.merge(b.core);
    b.core.merge(a.core);
    a.reloadFromCore();
    b.reloadFromCore();
    expect(paragraphsOf(a.core), ['Hello', ' world!']);
    expect(paragraphsOf(b.core), ['Hello', ' world!']);
    expectInStep(a);
    expectInStep(b);
  }, variant: mac);

  testWidgets('a composing IME text reaches the core only once committed', (tester) async {
    final s = section(['ab']);
    await show(tester, s);
    await tester.placeCaretInParagraph(node(s, 0), 2);
    final client = imeClientGetter();
    final value = client.currentTextEditingValue!;
    final caret = value.selection.extentOffset;
    await tester.ime.sendDeltas([
      TextEditingDeltaInsertion(
        oldText: value.text,
        textInserted: 'か',
        insertionOffset: caret,
        selection: TextSelection.collapsed(offset: caret + 1),
        composing: TextRange(start: caret, end: caret + 1),
      ),
    ], getter: imeClientGetter);
    expect(paragraphsOf(s.core), ['ab']);
    final composed = client.currentTextEditingValue!;
    await tester.ime.sendDeltas([
      TextEditingDeltaReplacement(
        oldText: composed.text,
        replacementText: '漢',
        replacedRange: TextRange(start: caret, end: caret + 1),
        selection: TextSelection.collapsed(offset: caret + 1),
        composing: TextRange.empty,
      ),
    ], getter: imeClientGetter);
    await tester.pump();
    expect(paragraphsOf(s.core), ['ab漢']);
    expectInStep(s);
  }, variant: mac);

  const steps = int.fromEnvironment('FUZZ_STEPS', defaultValue: 200);
  const seed = int.fromEnvironment('FUZZ_SEED', defaultValue: 7);
  testWidgets('$steps random edits keep editor and core in step', (tester) async {
    final random = Random(seed);
    final s = section(['alpha beta', 'gamma delta', 'epsilon']);
    final log = <Object>[];
    const dump = String.fromEnvironment('FUZZ_DUMP');
    await show(tester, s);
    for (var step = 0; step < steps; step++) {
      final index = random.nextInt(s.document.nodeCount);
      final length = textOf(s.document.getNodeAt(index)!).length;
      final offset = random.nextInt(length + 1);
      final action = random.nextInt(7);
      final before = s.editorShape();
      final spansBefore = textOf(s.document.getNodeAt(index)!).spans.toString();
      s.opLog.clear();
      await tester.placeCaretInParagraph(node(s, index), offset);
      switch (action) {
        case 0:
          await tester.pressEnter();
        case 1:
          await tester.pressBackspace();
        case 2:
          await tester.pressShiftEnter();
        case 3:
          for (var i = random.nextInt(12); i >= 0; i--) {
            await tester.pressShiftRightArrow();
          }
          await tester.pressBackspace();
        case 4:
          await tester.pressShiftRightArrow();
          await tester.pressShiftRightArrow();
          await tester.pressCmdB();
        case 5:
          await tester.pressDelete();
        default:
          await tester.typeImeText(['x', 'yz', ' ', 'wörd', '😀'][random.nextInt(5)]);
      }
      log.add(List.of(s.opLog));
      if (dump.isNotEmpty &&
          !const DeepCollectionEquality().equals(s.editorShape(), SectionBinding.coreShape(s.core.spans()))) {
        File(dump).writeAsStringSync(jsonEncode(log));
      }
      expect(
        s.editorShape(),
        SectionBinding.coreShape(s.core.spans()),
        reason:
            'step $step: action $action at paragraph $index offset $offset, before: $before, ops: ${s.opLog}, spans before: $spansBefore, after: ${s.document.nodeCount > index ? textOf(s.document.getNodeAt(index)!).spans : ''}',
      );
    }
  }, variant: mac);
}
