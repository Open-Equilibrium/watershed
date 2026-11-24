import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/whiteboard/board.dart';
import 'package:liquid/src/whiteboard/board_controller.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

List<Offset> across(double y, {double step = 5}) => [
  for (var x = 100.0; x <= 300; x += step) Offset(x, y),
];

BridgeElement stroke(
  List<Offset> points, {
  Rect? frame,
  BridgeInkColor color = BridgeInkColor.data3,
  BridgePen pen = BridgePen.ball,
}) {
  final ink = [for (final at in points) InkPoint(at.dx, at.dy)];
  return newElement(
    BridgeElementKind.stroke,
    frame: frame ?? pointsBounds(ink),
    color: color,
    width: 1,
    pen: pen,
    points: packPoints(ink),
  );
}

BridgeElement box(Rect frame, BridgeInkColor color) =>
    newElement(BridgeElementKind.shape, frame: frame, color: color, width: 1);

void main() {
  final kit = PageKit();

  setUp(() {
    boardTools
      ..tool = BoardTool.pen
      ..pen = BridgePen.fountain
      ..shape = ShapeKind.snap
      ..wholeEraser = false
      ..highlighterOnly = false;
  });

  Finder board() => find.byKey(const Key('whiteboard'));

  List<BridgeElement> stored() => kit.fake.whiteboards.values.single.elements;

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];

  BridgeDrawingEdit lastEdit() =>
      kit.applied(BridgeMutationKind.editDrawing).last.drawing!;

  Offset screen(WidgetTester tester, Offset at) =>
      tester.getTopLeft(find.byKey(const Key('whiteboard canvas'))) +
      at +
      const Offset(20, 20);

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> clickAt(WidgetTester tester, Offset at) async {
    await tester.tapAt(screen(tester, at));
    await tester.pumpAndSettle();
  }

  Future<void> focus(WidgetTester tester) =>
      clickAt(tester, const Offset(500, 20));

  Future<void> showFocused(
    WidgetTester tester, [
    List<BridgeElement> elements = const [],
  ]) async {
    await kit.show(tester, [WhiteboardPart(elements)]);
    await focus(tester);
  }

  Future<void> hover(WidgetTester tester) async {
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(board()));
    addTearDown(mouse.removePointer);
    await tester.pump();
  }

  Future<void> typeSlashWhiteboard(WidgetTester tester) async {
    await tester.tap(block(1));
    await tester.pump();
    for (final text in ['/', '/wh']) {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
        ),
      );
      await tester.pumpAndSettle();
    }
  }

  Future<void> draw(
    WidgetTester tester,
    List<Offset> path, {
    Duration hold = Duration.zero,
  }) async {
    final gesture = await tester.startGesture(
      screen(tester, path.first),
      kind: PointerDeviceKind.mouse,
    );
    for (final at in path.skip(1)) {
      await gesture.moveTo(screen(tester, at));
      await tester.pump(const Duration(milliseconds: 16));
    }
    if (hold > Duration.zero) await tester.pump(hold);
    await gesture.up();
    await tester.pumpAndSettle();
  }

  testWidgets(
    'the slash command adds a focused Whiteboard in the line\'s place',
    (tester) async {
      await kit.show(tester, [
        const TextPart('How we get around:'),
        const TextPart(''),
        const TextPart('Book the train ahead.'),
      ]);
      await typeSlashWhiteboard(tester);
      expect(find.byKey(const ValueKey('choice Whiteboard')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.enter);
      expect(kinds(), [
        BridgeBlockKind.text,
        BridgeBlockKind.whiteboard,
        BridgeBlockKind.text,
      ]);
      expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);
      expect(boardTools.tool, BoardTool.pen);
    },
  );

  testWidgets('a Whiteboard in a Section\'s first line starts that Section', (
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
    await typeSlashWhiteboard(tester);
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      [
        for (final view in kit.fake.views[kit.page]!)
          (view.kind, view.boundary),
      ],
      [
        (BridgeBlockKind.text, false),
        (BridgeBlockKind.whiteboard, true),
        (BridgeBlockKind.text, false),
      ],
    );
  });

  testWidgets('esc in a full-screen text box ends the box, then full screen', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    await hover(tester);
    await tap(tester, find.byKey(const Key('whiteboard full screen')));
    await key(tester, LogicalKeyboardKey.keyT);
    await clickAt(tester, const Offset(120, 120));
    await tester.enterText(find.byKey(const Key('whiteboard text')), 'Porto');
    await key(tester, LogicalKeyboardKey.escape);
    expect(lastEdit().add.single.text, 'Porto');
    expect(find.byKey(const Key('whiteboard full')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('whiteboard full')), findsNothing);
  });

  testWidgets('a first click only focuses; the pen then draws a stroke', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    expect(find.byKey(const Key('whiteboard tools')), findsNothing);
    await focus(tester);
    expect(kit.applied(BridgeMutationKind.editDrawing), isEmpty);
    expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);

    await draw(tester, [
      for (var x = 100.0; x <= 200; x += 10) Offset(x, 100 + (x % 20)),
    ]);
    final [added] = lastEdit().add;
    expect(added.kind, BridgeElementKind.stroke);
    expect(added.pen, BridgePen.fountain);
    expect(added.color, BridgeInkColor.data3);
    final points = unpackPoints(added.points);
    expect(points.first.offset, const Offset(100, 100));
    expect(points.length, 11);
    expect(added.x, 100);
    expect(added.w, 100);
  });

  testWidgets('the highlighter marks beneath, straight along a line', (
    tester,
  ) async {
    await showFocused(tester);
    await key(tester, LogicalKeyboardKey.keyH);
    await draw(tester, [
      for (var x = 100.0; x <= 300; x += 10) Offset(x, 150 + (x % 20) / 10),
    ]);
    final [mark] = lastEdit().add;
    expect(mark.pen, BridgePen.highlighter);
    expect(mark.color, BridgeInkColor.yellow);
    expect(unpackPoints(mark.points), hasLength(2), reason: 'straightened');
  });

  testWidgets('the eraser can take only the highlighter\'s marks', (
    tester,
  ) async {
    await showFocused(tester, [
      stroke(across(100)),
      stroke(
        across(100),
        color: BridgeInkColor.yellow,
        pen: BridgePen.highlighter,
      ),
    ]);
    await key(tester, LogicalKeyboardKey.keyE);
    boardTools
      ..wholeEraser = true
      ..highlighterOnly = true;
    await draw(tester, [const Offset(200, 80), const Offset(200, 120)]);
    expect(lastEdit().remove, ['element 1']);
  });

  testWidgets('drawing and holding still snaps the stroke into a shape', (
    tester,
  ) async {
    await showFocused(tester);
    await draw(
      tester,
      across(150, step: 10),
      hold: const Duration(milliseconds: 600),
    );
    final [added] = lastEdit().add;
    expect(added.kind, BridgeElementKind.shape);
    expect(added.shape, BridgeShape.line);
    expect((added.w, added.h), (200.0, 0.0));
  });

  testWidgets('with a mouse the shape tool drags out shapes and arrows', (
    tester,
  ) async {
    await showFocused(tester);
    await key(tester, LogicalKeyboardKey.keyR);
    await draw(tester, [const Offset(40, 60), const Offset(190, 130)]);
    await key(tester, LogicalKeyboardKey.keyA);
    await draw(tester, [const Offset(200, 100), const Offset(400, 190)]);
    final [rectangle, arrow] = stored();
    expect(rectangle.shape, BridgeShape.rectangle);
    expect(frameOf(rectangle), const Rect.fromLTWH(40, 60, 150, 70));
    expect(arrow.shape, BridgeShape.line);
    expect(arrow.arrowhead, isTrue);
  });

  testWidgets('the eraser cuts a stroke where it passes, or takes it whole', (
    tester,
  ) async {
    await showFocused(tester, [stroke(across(100)), stroke(across(200))]);
    await key(tester, LogicalKeyboardKey.keyE);
    await draw(tester, [const Offset(200, 80), const Offset(200, 120)]);
    final cut = lastEdit();
    expect(cut.remove, ['element 0']);
    expect(cut.add, hasLength(2), reason: 'the two pieces either side');
    for (final piece in cut.add) {
      final points = unpackPoints(piece.points);
      expect(
        points.every((point) => (point.x - 200).abs() > 10),
        isTrue,
        reason: 'nothing left under the eraser',
      );
    }

    boardTools.wholeEraser = true;
    await draw(tester, [const Offset(150, 190), const Offset(150, 210)]);
    expect(lastEdit().remove, ['element 1']);
    expect(lastEdit().add, isEmpty);
  });

  testWidgets('a board read during an eraser drag brings nothing back', (
    tester,
  ) async {
    await showFocused(tester, [stroke(across(100))]);
    await key(tester, LogicalKeyboardKey.keyE);
    final gesture = await tester.startGesture(
      screen(tester, const Offset(150, 80)),
      kind: PointerDeviceKind.mouse,
    );
    await gesture.moveTo(screen(tester, const Offset(150, 120)));
    await tester.pump();
    await kit.boardOf(tester).read();
    await tester.pump();
    await gesture.moveTo(screen(tester, const Offset(150, 60)));
    await gesture.moveTo(screen(tester, const Offset(250, 60)));
    await gesture.moveTo(screen(tester, const Offset(250, 120)));
    await gesture.up();
    await tester.pumpAndSettle();
    final edit = lastEdit();
    expect(edit.remove, ['element 0']);
    expect(edit.add, hasLength(3), reason: 'three pieces, none twice');
  });

  testWidgets('erasing part of a resized stroke keeps its pieces\' size', (
    tester,
  ) async {
    await showFocused(tester, [
      stroke(across(100), frame: const Rect.fromLTWH(100, 100, 400, 0)),
    ]);
    await key(tester, LogicalKeyboardKey.keyE);
    await draw(tester, [const Offset(300, 80), const Offset(300, 120)]);
    expect(lastEdit().add, hasLength(2));
    for (final piece in lastEdit().add) {
      final bounds = pointsBounds(unpackPoints(piece.points));
      expect(piece.w / bounds.width, closeTo(2, 0.01), reason: 'still twice');
    }
  });

  testWidgets(
    'the lasso selects what it encloses; a drag moves it, ⌫ deletes',
    (tester) async {
      await showFocused(tester, [
        box(const Rect.fromLTWH(100, 100, 80, 50), BridgeInkColor.data1),
        box(const Rect.fromLTWH(400, 100, 80, 50), BridgeInkColor.data2),
      ]);
      await key(tester, LogicalKeyboardKey.keyL);
      await draw(tester, [
        const Offset(80, 80),
        const Offset(220, 80),
        const Offset(220, 180),
        const Offset(80, 180),
        const Offset(80, 85),
      ]);
      expect(find.byKey(const Key('whiteboard delete')), findsOneWidget);
      await draw(tester, [const Offset(140, 125), const Offset(160, 145)]);
      final [moved] = lastEdit().change;
      expect(moved.id, 'element 0');
      expect((moved.x, moved.y), (120.0, 120.0));

      await key(tester, LogicalKeyboardKey.backspace);
      expect(lastEdit().remove, ['element 0']);
      expect([for (final element in stored()) element.id], ['element 1']);
    },
  );

  testWidgets('the text tool places a text box', (tester) async {
    await showFocused(tester);
    await key(tester, LogicalKeyboardKey.keyT);
    await clickAt(tester, const Offset(120, 120));
    await tester.enterText(find.byKey(const Key('whiteboard text')), 'Lisbon');
    await tap(tester, find.byKey(const Key('whiteboard text')));
    expect(find.byKey(const Key('whiteboard text')), findsOneWidget);
    expect(kit.applied(BridgeMutationKind.editDrawing), isEmpty);
    await clickAt(tester, const Offset(500, 300));
    final [text] = lastEdit().add;
    expect(text.kind, BridgeElementKind.text);
    expect(text.text, 'Lisbon');
    expect(text.x, 120);
  });

  testWidgets('the style switch sets the whole board to clean and back', (
    tester,
  ) async {
    await showFocused(tester);
    BridgeDrawingStyle style() => kit.fake.whiteboards.values.single.style;
    await tap(tester, find.byKey(const ValueKey('style hand-drawn')));
    expect(
      kit.applied(BridgeMutationKind.setDrawingStyle),
      isEmpty,
      reason: 'the chosen style stays as it is',
    );
    await tap(tester, find.byKey(const ValueKey('style clean')));
    expect(style(), BridgeDrawingStyle.clean);
    await tap(tester, find.byKey(const ValueKey('style hand-drawn')));
    expect(style(), BridgeDrawingStyle.handDrawn);
  });

  testWidgets('pinching zooms a board and the right button pans it', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    final center = tester.getCenter(board());
    final pinch = TestPointer(1, PointerDeviceKind.trackpad);
    await tester.sendEventToBinding(pinch.panZoomStart(center));
    await tester.sendEventToBinding(pinch.panZoomUpdate(center, scale: 2));
    await tester.sendEventToBinding(pinch.panZoomEnd());
    await tester.pumpAndSettle();
    await focus(tester);
    expect(find.text('200 %'), findsOneWidget, reason: 'unfocused, too');

    final right = await tester.startGesture(
      center,
      kind: PointerDeviceKind.mouse,
      buttons: kSecondaryMouseButton,
    );
    await right.moveBy(const Offset(40, 0));
    await right.up();
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.editDrawing), isEmpty);
  });

  testWidgets(
    'full screen shows the same board until Done, and closes once its Whiteboard is gone',
    (tester) async {
      await kit.show(tester, [
        WhiteboardPart([
          stroke(const [Offset(10, 10), Offset(60, 40)]),
        ]),
      ]);
      await hover(tester);
      await tap(tester, find.byKey(const Key('whiteboard full screen')));
      expect(find.byKey(const Key('whiteboard full')), findsOneWidget);
      expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);
      await tap(tester, find.byKey(const Key('whiteboard done')));
      expect(find.byKey(const Key('whiteboard full')), findsNothing);
      expect(board(), findsOneWidget);

      final shown = kit.boardOf(tester);
      await tap(tester, find.byKey(const Key('whiteboard full screen')));
      kit.fake.whiteboards.clear();
      await shown.read();
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('whiteboard full')), findsNothing);
    },
  );

  testWidgets('strokes still on their way draw apart and select nothing', (
    tester,
  ) async {
    await showFocused(tester);
    kit.fake.gate = Completer<void>();
    await draw(tester, [const Offset(100, 100), const Offset(200, 100)]);
    await draw(tester, [const Offset(100, 200), const Offset(100, 300)]);
    final board = kit.boardOf(tester);
    final pending = board.elements;
    expect(pending, hasLength(2));
    expect(
      board.cache.points(pending[0]).first.offset,
      isNot(board.cache.points(pending[1]).first.offset),
    );
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
    expect(tester.takeException(), isNull);
    expect(board.selected, isEmpty, reason: 'only stored elements select');
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(stored(), hasLength(2));
  });

  testWidgets(
    'a Whiteboard clicked under the pointer keeps its ⋯ menu after esc',
    (tester) async {
      await kit.show(tester, [
        const TextPart('Intro'),
        const WhiteboardPart(),
        const TextPart('After'),
      ]);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: tester.getCenter(board()));
      addTearDown(mouse.removePointer);
      await focus(tester);
      expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.escape);
      expect(find.byKey(const Key('whiteboard tools')), findsNothing);
      expect(tester.takeException(), isNull);
      await tap(tester, find.byKey(const Key('whiteboard more')));
      expect(find.byKey(const ValueKey('whiteboard Delete')), findsOneWidget);
    },
  );

  testWidgets('a tool change mid-drag keeps the move it had shown', (
    tester,
  ) async {
    await showFocused(tester, [
      box(const Rect.fromLTWH(100, 100, 80, 50), BridgeInkColor.data1),
    ]);
    await key(tester, LogicalKeyboardKey.keyL);
    await draw(tester, [
      const Offset(80, 80),
      const Offset(220, 80),
      const Offset(220, 180),
      const Offset(80, 180),
      const Offset(80, 85),
    ]);
    final gesture = await tester.startGesture(
      screen(tester, const Offset(140, 125)),
      kind: PointerDeviceKind.mouse,
    );
    addTearDown(gesture.removePointer);
    await gesture.moveTo(screen(tester, const Offset(140, 135)));
    await key(tester, LogicalKeyboardKey.keyP);
    await gesture.moveTo(screen(tester, const Offset(140, 150)));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(frameOf(stored().single).top, 110);
  });

  testWidgets('a horizontal line grabs by its middle, so a drag moves it', (
    tester,
  ) async {
    await showFocused(tester, [
      newElement(
        BridgeElementKind.shape,
        frame: const Rect.fromLTWH(100, 100, 200, 0),
        color: BridgeInkColor.data1,
        width: 1,
        shape: BridgeShape.line,
      ),
    ]);
    await key(tester, LogicalKeyboardKey.keyL);
    await draw(tester, [
      const Offset(80, 80),
      const Offset(320, 80),
      const Offset(320, 120),
      const Offset(80, 120),
      const Offset(80, 85),
    ]);
    await draw(tester, [const Offset(200, 100), const Offset(200, 140)]);
    expect(frameOf(stored().single).top, 140);
  });

  testWidgets(
    'a highlighter\'s whole band erases, not only its pen-size core',
    (tester) async {
      await showFocused(tester, [
        copyElement(
          stroke(
            across(100),
            color: BridgeInkColor.yellow,
            pen: BridgePen.highlighter,
          ),
          width: 2,
        ),
      ]);
      await key(tester, LogicalKeyboardKey.keyE);
      boardTools.wholeEraser = true;
      await draw(tester, [const Offset(200, 120), const Offset(200, 121)]);
      expect(lastEdit().remove, ['element 0']);
    },
  );

  testWidgets('a partial erase cuts a stretched stroke where the circle is', (
    tester,
  ) async {
    await showFocused(tester, [
      stroke(across(100), frame: const Rect.fromLTWH(100, 100, 100, 0)),
    ]);
    await key(tester, LogicalKeyboardKey.keyE);
    await draw(tester, [const Offset(208, 100), const Offset(208, 101)]);
    final piece = lastEdit().add.single;
    expect(piece.x + piece.w, lessThan(197), reason: 'cut 12 board px short');
  });

  testWidgets('a text box is laid out at its own width at any zoom', (
    tester,
  ) async {
    await showFocused(tester);
    kit.boardOf(tester).zoomBy(2, Offset.zero);
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.keyT);
    await clickAt(tester, const Offset(120, 120));
    expect(tester.getSize(find.byKey(const Key('whiteboard text'))).width, 240);
  });

  testWidgets('a text box\'s frame grows with its size', (tester) async {
    await showFocused(tester, [
      newElement(
        BridgeElementKind.text,
        frame: const Rect.fromLTWH(100, 100, 240, 30),
        color: BridgeInkColor.text,
        width: 0,
        text: 'Lisbon',
      ),
    ]);
    await key(tester, LogicalKeyboardKey.keyL);
    await draw(tester, [
      const Offset(80, 80),
      const Offset(400, 80),
      const Offset(400, 160),
      const Offset(80, 160),
      const Offset(80, 85),
    ]);
    await tap(tester, find.byKey(const ValueKey('width 2')));
    expect(stored().single.h, greaterThanOrEqualTo(28 * 1.25));
  });

  testWidgets('⋯ › Delete removes the Whiteboard', (tester) async {
    await kit.show(tester, [
      const TextPart('Intro'),
      const WhiteboardPart(),
      const TextPart('After'),
    ]);
    await hover(tester);
    await tap(tester, find.byKey(const Key('whiteboard more')));
    await tap(tester, find.byKey(const ValueKey('whiteboard Delete')));
    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);
    expect(
      kit.fake.followed.where(
        (resource) =>
            resource.kind == BridgeResourceKind.block &&
            kit.fake.whiteboards.containsKey(resource.block),
      ),
      isEmpty,
      reason: 'the board no longer follows its Block',
    );
  });
}
