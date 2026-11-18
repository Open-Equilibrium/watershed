import 'dart:async';
import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/media_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final picked = <MediaSource>[];
  String? saveTo;

  setUp(() {
    picked.clear();
    saveTo = null;
    MediaPlatform.pickFiles = () async => [...picked];
    MediaPlatform.saveLocation = (_) async => saveTo;
    MediaPlatform.fileSize = (path) async => kit.fake.files[path]!.length;
  });

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];
  List<String> names() => [
    for (final view in kit.fake.views[kit.page]!)
      if (view.kind == BridgeBlockKind.media) kit.fake.media[view.block]!.name,
  ];

  MediaSource file(String path, int size) {
    kit.fake.files[path] = Uint8List(size);
    return MediaSource.file(path.split('/').last, path, size);
  }

  Future<void> slash(WidgetTester tester, int index, String typed) async {
    await tester.tap(block(index));
    await tester.pump();
    for (final text in ['/', typed]) {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
        ),
      );
      await tester.pumpAndSettle();
    }
  }

  Future<void> hover(WidgetTester tester, Finder finder) async {
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(finder));
    addTearDown(mouse.removePointer);
    await tester.pump();
  }

  testWidgets('the Blocks in slash commands are in alphabetical order', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('')]);
    await slash(tester, 0, '/');
    final tops = [
      for (final label in ['Code', 'Media', 'Table'])
        tester.getTopLeft(find.byKey(ValueKey('choice $label'))).dy,
    ];
    expect(tops, orderedEquals([...tops]..sort()));
  });

  testWidgets(
    'Media opens the file dialog and adds the files in the line\'s place',
    (tester) async {
      await kit.show(tester, [
        const TextPart('What we planned to spend:'),
        const TextPart(''),
        const TextPart('After'),
      ]);
      picked
        ..add(file('/Users/me/Lisbon trip budget.pdf', 2400000))
        ..add(file('/Users/me/Hotel booking.eml', 48000));
      await slash(tester, 1, '/med');
      expect(find.byKey(const ValueKey('choice Media')), findsOneWidget);
      expect(find.byKey(const ValueKey('choice Table')), findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();

      expect(kinds(), [
        BridgeBlockKind.text,
        BridgeBlockKind.media,
        BridgeBlockKind.media,
        BridgeBlockKind.text,
      ]);
      expect(names(), ['Lisbon trip budget.pdf', 'Hotel booking.eml']);
      expect(kit.fake.textsOf(kit.page), [
        'What we planned to spend:',
        'After',
      ]);
      expect(find.text('2.4 MB · PDF'), findsOneWidget);
      expect(find.text('48 KB · EML'), findsOneWidget);
    },
  );

  testWidgets('media in a Section\'s first line starts that Section', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('Intro'),
      const TextPart(''),
      const TextPart('After'),
    ]);
    final line = kit.fake.views[kit.page]![1];
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setSectionBoundary,
        page: kit.page,
        view: line.id,
        boundary: true,
      ),
    );
    await tester.pumpAndSettle();
    picked.add(file('/Users/me/notes.txt', 10));
    await slash(tester, 1, '/media');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      [
        for (final view in kit.fake.views[kit.page]!)
          (view.kind, view.boundary),
      ],
      [
        (BridgeBlockKind.text, false),
        (BridgeBlockKind.media, true),
        (BridgeBlockKind.text, false),
      ],
    );
  });

  testWidgets(
    'cancelling the dialog keeps the line; the last line stays below',
    (tester) async {
      await kit.show(tester, [const TextPart('Intro'), const TextPart('')]);
      await slash(tester, 1, '/media');
      await key(tester, LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);

      picked.add(file('/Users/me/notes.txt', 10));
      await slash(tester, 1, '/media');
      await key(tester, LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(kinds(), [
        BridgeBlockKind.text,
        BridgeBlockKind.media,
        BridgeBlockKind.text,
      ]);
    },
  );

  testWidgets('a file going in shows how much is in, then its Block', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Intro'), const TextPart('')]);
    kit.fake.uploadGate = Completer<void>();
    picked.add(file('/Users/me/Tram ride.mov', 3 << 20));
    await slash(tester, 1, '/media');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.pump();
    expect(find.text('Tram ride.mov'), findsOneWidget);
    expect(find.text('2.1 MB of 3.1 MB'), findsOneWidget);
    expect(find.byType(LinearProgressIndicator), findsOneWidget);

    kit.fake.uploadGate!.complete();
    await tester.pumpAndSettle();
    expect(find.byType(LinearProgressIndicator), findsNothing);
    expect(find.text('3.1 MB · MOV'), findsOneWidget);
  });

  testWidgets('a file over 5 GiB is refused before it is read', (tester) async {
    await kit.show(tester, [const TextPart('Intro'), const TextPart('')]);
    picked.add(
      const MediaSource.file(
        'Castle drone flight.mov',
        '/Users/me/Castle drone flight.mov',
        7800000000,
      ),
    );
    await slash(tester, 1, '/media');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);
    expect(
      kit.workspace.problem.value,
      '“Castle drone flight.mov” is 7.8 GB; a file can be at most 5 GiB.',
    );
  });

  testWidgets('an image shows at its own size, never wider than the column', (
    tester,
  ) async {
    final png = base64Decode(
      'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAK0lEQVR4nO3NMQ0AAAgDsMlBNnKQ'
      'hQw4mvRvputExGKxWCwWi8VisVj8N158a1NqSEcsCQAAAABJRU5ErkJggg==',
    );
    await kit.show(tester, [MediaPart('tram.png', png)]);
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    final image = find.byKey(const Key('media image'));
    expect(image, findsOneWidget);
    expect(tester.getSize(image), const Size(40, 30));
  });

  testWidgets(
    'Save as… writes a copy where the person chose; Delete removes it',
    (tester) async {
      await kit.show(tester, [
        const TextPart('Intro'),
        const MediaPart('Lisbon trip budget.pdf', [1, 2, 3]),
        const TextPart('After'),
      ]);
      await hover(tester, find.text('Lisbon trip budget.pdf'));
      saveTo = '/Users/me/Desktop/budget.pdf';
      await tester.tap(find.byKey(const Key('media save')));
      await tester.pumpAndSettle();
      expect(kit.fake.saved['/Users/me/Desktop/budget.pdf'], [1, 2, 3]);

      await tester.tap(find.byKey(const Key('media more')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('media Delete')));
      await tester.pumpAndSettle();
      expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);
    },
  );

  testWidgets('the caret passes over a Media Block', (tester) async {
    await kit.show(tester, [
      const TextPart('Intro'),
      const MediaPart('budget.pdf', [1]),
      const TextPart('After'),
    ]);
    await tester.tap(block(1));
    await tester.pump();
    field(tester, 1).controller!.selection = const TextSelection.collapsed(
      offset: 0,
    );
    await tester.pump();
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
  });

  testWidgets('files dropped on a Page land where the line shows', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('The tram up to the castle:'),
      const TextPart('What we planned to spend:'),
    ]);
    final paths = ['/Users/me/tram.mov', '/Users/me/budget.pdf'];
    for (final path in paths) {
      kit.fake.files[path] = Uint8List.fromList([1, 2, 3]);
    }
    Future<void> native(String method, Object arguments) async {
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        'desktop_drop',
        const StandardMethodCodec().encodeMethodCall(
          MethodCall(method, arguments),
        ),
        (_) {},
      );
      await tester.pump();
    }

    final second = tester.getRect(block(1));
    final at = [second.center.dx, second.top + 4];
    await native('entered', at);
    await native('updated', at);
    expect(find.byKey(const Key('drop line')), findsOneWidget);
    await native('performOperation', paths);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('drop line')), findsNothing);
    expect(kinds(), [
      BridgeBlockKind.text,
      BridgeBlockKind.media,
      BridgeBlockKind.media,
      BridgeBlockKind.text,
    ]);
    expect(names(), ['tram.mov', 'budget.pdf']);
  });
}
