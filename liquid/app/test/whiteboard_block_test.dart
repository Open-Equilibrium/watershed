import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/whiteboard.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/whiteboard/board.dart';
import 'package:liquid/src/whiteboard/board_controller.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

/// Whiteboard Blocks (approved Whiteboard Block mockups, ADR-0306).
void main() {
  final kit = PageKit();

  setUp(() {
    boardTools
      ..tool = BoardTool.pen
      ..pen = BridgePen.fountain
      ..shape = ShapeKind.snap
      ..wholeEraser = false;
  });

  Finder board() => find.byKey(const Key('whiteboard'));

  /// The board's elements as the fake Workspace keeps them.
  List<BridgeElement> stored() => kit.fake.whiteboards.values.single.elements;

  BridgeWhiteboardEdit lastEdit() =>
      kit.applied(BridgeMutationKind.editWhiteboard).last.whiteboard!;

  /// A screen point at board point `at`, the canvas starting at board (-20, -20).
  Offset screen(WidgetTester tester, Offset at) =>
      tester.getTopLeft(find.byKey(const Key('whiteboard canvas'))) +
      at +
      const Offset(20, 20);

  Future<void> focus(WidgetTester tester) async {
    await tester.tapAt(screen(tester, const Offset(500, 20)));
    await tester.pumpAndSettle();
  }

  /// Drags with the left button along board points.
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

  BridgeElement stroke(List<Offset> points) {
    final ink = [for (final at in points) InkPoint(at.dx, at.dy)];
    return newElement(
      BridgeElementKind.stroke,
      frame: pointsBounds(ink),
      color: BridgeInkColor.data3,
      width: 1,
      pen: BridgePen.ball,
      points: packPoints(ink),
    );
  }

  testWidgets(
    'the slash command adds a focused Whiteboard in the line\'s place',
    (tester) async {
      await kit.show(tester, [
        const TextPart('How we get around:'),
        const TextPart(''),
        const TextPart('Book the train ahead.'),
      ]);
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
      expect(find.byKey(const ValueKey('choice Whiteboard')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(
        [for (final view in kit.fake.views[kit.page]!) view.kind],
        [
          BridgeBlockKind.text,
          BridgeBlockKind.whiteboard,
          BridgeBlockKind.text,
        ],
      );
      expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);
      expect(boardTools.tool, BoardTool.pen);
    },
  );

  testWidgets('a first click only focuses; the pen then draws a stroke', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    expect(find.byKey(const Key('whiteboard tools')), findsNothing);
    await focus(tester);
    expect(kit.applied(BridgeMutationKind.editWhiteboard), isEmpty);
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

  testWidgets('drawing and holding still snaps the stroke into a shape', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    await focus(tester);
    await draw(tester, [
      for (var x = 100.0; x <= 300; x += 10) Offset(x, 150),
    ], hold: const Duration(milliseconds: 600));
    final [added] = lastEdit().add;
    expect(added.kind, BridgeElementKind.shape);
    expect(added.shape, BridgeShape.line);
    expect((added.w, added.h), (200.0, 0.0));
  });

  testWidgets('with a mouse the shape tool drags out shapes and arrows', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    await focus(tester);
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
    await kit.show(tester, [
      WhiteboardPart([
        stroke([for (var x = 100.0; x <= 300; x += 5) Offset(x, 100)]),
        stroke([for (var x = 100.0; x <= 300; x += 5) Offset(x, 200)]),
      ]),
    ]);
    await focus(tester);
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

  testWidgets(
    'the lasso selects what it encloses; a drag moves it, ⌫ deletes',
    (tester) async {
      await kit.show(tester, [
        WhiteboardPart([
          newElement(
            BridgeElementKind.shape,
            frame: const Rect.fromLTWH(100, 100, 80, 50),
            color: BridgeInkColor.data1,
            width: 1,
          ),
          newElement(
            BridgeElementKind.shape,
            frame: const Rect.fromLTWH(400, 100, 80, 50),
            color: BridgeInkColor.data2,
            width: 1,
          ),
        ]),
      ]);
      await focus(tester);
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
    await kit.show(tester, [const WhiteboardPart()]);
    await focus(tester);
    await key(tester, LogicalKeyboardKey.keyT);
    await tester.tapAt(screen(tester, const Offset(120, 120)));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('whiteboard text')), 'Lisbon');
    await tester.tapAt(screen(tester, const Offset(500, 300)));
    await tester.pumpAndSettle();
    final [text] = lastEdit().add;
    expect(text.kind, BridgeElementKind.text);
    expect(text.text, 'Lisbon');
    expect(text.x, 120);
  });

  testWidgets('the style button switches the whole board to clean and back', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    await focus(tester);
    await tester.tap(find.byKey(const Key('whiteboard style')));
    await tester.pumpAndSettle();
    expect(
      kit.fake.whiteboards.values.single.style,
      BridgeWhiteboardStyle.clean,
    );
    await tester.tap(find.byKey(const Key('whiteboard style')));
    await tester.pumpAndSettle();
    expect(
      kit.fake.whiteboards.values.single.style,
      BridgeWhiteboardStyle.handDrawn,
    );
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
    expect(kit.applied(BridgeMutationKind.editWhiteboard), isEmpty);
  });

  testWidgets('full screen shows the same board until Done', (tester) async {
    await kit.show(tester, [
      WhiteboardPart([
        stroke([const Offset(10, 10), const Offset(60, 40)]),
      ]),
    ]);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(board()));
    addTearDown(mouse.removePointer);
    await tester.pump();
    await tester.tap(find.byKey(const Key('whiteboard full screen')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('whiteboard full')), findsOneWidget);
    expect(find.byKey(const Key('whiteboard tools')), findsOneWidget);
    await tester.tap(find.byKey(const Key('whiteboard done')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('whiteboard full')), findsNothing);
    expect(board(), findsOneWidget);
  });

  testWidgets('strokes still on their way draw apart and select nothing', (
    tester,
  ) async {
    await kit.show(tester, [const WhiteboardPart()]);
    await focus(tester);
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

  test('a line\'s arrowhead redraws once it changes', () {
    final cache = BoardCache();
    BridgeElement line({required bool arrowhead}) => withId(
      newElement(
        BridgeElementKind.shape,
        frame: const Rect.fromLTWH(0, 0, 100, 0),
        color: BridgeInkColor.text,
        width: 1,
        shape: BridgeShape.line,
        arrowhead: arrowhead,
      ),
      'line',
    );
    final plain = cache.shape(line(arrowhead: false), false).getBounds();
    final headed = cache.shape(line(arrowhead: true), false).getBounds();
    expect(headed.height, greaterThan(plain.height));
  });

  testWidgets('⋯ › Delete removes the Whiteboard', (tester) async {
    await kit.show(tester, [
      const TextPart('Intro'),
      const WhiteboardPart(),
      const TextPart('After'),
    ]);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(board()));
    addTearDown(mouse.removePointer);
    await tester.pump();
    await tester.tap(find.byKey(const Key('whiteboard more')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('whiteboard Delete')));
    await tester.pumpAndSettle();
    expect(
      [for (final view in kit.fake.views[kit.page]!) view.kind],
      [BridgeBlockKind.text, BridgeBlockKind.text],
    );
  });
}
