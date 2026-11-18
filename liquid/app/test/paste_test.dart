import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/code_block.dart';
import 'package:liquid/src/workspace/paste.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  var clipboard = const Pasted();

  setUp(() => readClipboard = () async => clipboard);

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];

  Future<void> paste(WidgetTester tester, int index, int offset) async {
    await tester.tap(block(index));
    await tester.pump();
    field(tester, index).controller!.selection = TextSelection.collapsed(
      offset: offset,
    );
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  group('the clipboard', () {
    test('files come first, then an image, then code an editor marked', () {
      final png = Uint8List.fromList([137, 80, 78, 71]);
      expect(
        pastedFrom({
          'files': ['/a.pdf'],
          'image': png,
          'text': 'a.pdf',
        }).files,
        ['/a.pdf'],
      );
      expect(pastedFrom({'image': png}).image, png);
      expect(
        pastedFrom({'image': png, 'text': 'https://example.com/tram.png'})
            .image,
        png,
        reason: 'a browser copies an image with its address',
      );
      final cells = pastedFrom({
        'image': png,
        'html': '<table><tr><td>12</td></tr></table>',
        'text': '12\t30',
      });
      expect(
        (cells.image, cells.text),
        (null, '12\t30'),
        reason: 'Excel, Word and Keynote put a picture beside copied text',
      );

      final vscode = pastedFrom({
        'vscode': '{"version":1,"mode":"rust"}',
        'text': 'fn main() {}',
      });
      expect((vscode.code, vscode.language), ('fn main() {}', 'rust'));
      expect(
        pastedFrom({'vscode': '{"mode":"shellscript"}', 'text': 'ls'}).language,
        'bash',
      );
      expect(
        pastedFrom({'vscode': '{"mode":"cobol"}', 'text': 'x'}).language,
        '',
        reason: 'a language the picker does not know is plain text',
      );
    });

    test('HTML that is one pre element is code; other HTML is text', () {
      const meta = '<meta charset="utf-8">';
      expect(
        pastedFrom({
          'html': '$meta<pre><code>a = 1</code></pre>',
          'text': 'a = 1',
        }).code,
        'a = 1',
      );
      expect(
        pastedFrom({'html': '<p>See</p><pre>a = 1</pre>', 'text': 'See a = 1'})
            .code,
        isNull,
      );
      expect(pastedFrom({'text': 'plain'}).text, 'plain');
    });
  });

  testWidgets('⌘V with an image adds it below a line with text', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('The castle:'),
      const TextPart('After'),
    ]);
    clipboard = Pasted(image: Uint8List.fromList([1, 2, 3]));
    await paste(tester, 0, 11);
    expect(kinds(), [
      BridgeBlockKind.text,
      BridgeBlockKind.media,
      BridgeBlockKind.text,
    ]);
    expect(kit.fake.textsOf(kit.page), ['The castle:', 'After']);
    final media = kit.fake.views[kit.page]![1].block;
    expect(kit.fake.media[media]!.name, 'Pasted image.png');
  });

  testWidgets('⌘V with code from an editor turns an empty line into code', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Intro'), const TextPart('')]);
    clipboard = const Pasted(code: 'fn main() {}', language: 'rust');
    await paste(tester, 1, 0);
    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.code]);
    final code = kit.fake.views[kit.page]![1].block;
    expect(kit.fake.texts[code], 'fn main() {}');
    expect(kit.fake.languages[code], 'rust');
    final field = tester.widget<TextField>(
      find.descendant(
        of: find.byType(CodeBlock),
        matching: find.byKey(const Key('code')),
      ),
    );
    expect(field.focusNode!.hasFocus, isTrue);
  });

  testWidgets('⌘V with code below a line with text adds a Code Block', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Shares round:')]);
    clipboard = const Pasted(code: 'share()');
    await paste(tester, 0, 13);
    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.code]);
    expect(kit.fake.textsOf(kit.page), ['Shares round:']);
    expect(kit.fake.texts[kit.fake.views[kit.page]![1].block], 'share()');
  });

  testWidgets('⌘V with plain text pastes it at the caret', (tester) async {
    await kit.show(tester, [const TextPart('Five Lisbon')]);
    clipboard = const Pasted(text: 'days in ');
    await paste(tester, 0, 5);
    expect(kit.fake.textsOf(kit.page), ['Five days in Lisbon']);
    expect(field(tester, 0).controller!.selection.baseOffset, 13);
  });

  testWidgets('⌘V pastes where the caret is once the clipboard answers', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Five days in Lisbon')]);
    final answer = Completer<Pasted>();
    readClipboard = () => answer.future;
    await tester.tap(block(0));
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 19,
    );
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'Five days',
        selection: TextSelection.collapsed(offset: 9),
      ),
    );
    await tester.pump();
    answer.complete(const Pasted(text: ' in Porto'));
    await tester.pumpAndSettle();
    expect(kit.fake.textsOf(kit.page), ['Five days in Porto']);
  });
}
