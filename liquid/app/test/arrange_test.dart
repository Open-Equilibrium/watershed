import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  List<BridgeView> views() => kit.fake.views[kit.page]!;

  Future<void> setTile(
    WidgetTester tester,
    BridgeView view,
    BridgeTile tile,
  ) async {
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTile,
        page: kit.page,
        view: view.id,
        tile: tile,
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> trip(WidgetTester tester) async {
    await kit.show(tester, [
      const TextPart('Plan'),
      const TextPart('Five days in Lisbon'),
      const WhiteboardPart([]),
      const TextPart('Ideas'),
    ]);
    final [plan, _, board, ideas] = views();
    await setTile(tester, plan, const BridgeTile(column: 0, span: 5));
    await setTile(tester, board, const BridgeTile(column: 5, span: 7));
    await setTile(tester, ideas, const BridgeTile(column: 0, span: 5));
  }

  Future<void> arrange(WidgetTester tester) async {
    await tap(tester, find.byKey(const Key('arrange')));
    expectOne(find.byKey(const Key('arrange bar')));
  }

  Future<void> press(
    WidgetTester tester,
    LogicalKeyboardKey key, {
    bool alt = false,
  }) async {
    if (alt) await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(key);
    if (alt) await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pumpAndSettle();
  }

  Iterable<BridgeMutation> tiles() => kit.applied(BridgeMutationKind.setTile);

  (int, int)? placed(BridgeMutation change) => switch (change.tile) {
    final tile? => (tile.column, tile.span),
    null => null,
  };

  testWidgets('Arrange shows its bar; Done and esc end '
      'it', (tester) async {
    await trip(tester);
    await arrange(tester);
    await tap(tester, find.byKey(const Key('arrange done')));
    expectNone(find.byKey(const Key('arrange bar')));
    await arrange(tester);
    await press(tester, LogicalKeyboardKey.escape);
    expectNone(find.byKey(const Key('arrange bar')));
  });

  testWidgets('arrow keys move the selected tile, ⌥ and arrows resize '
      'it', (tester) async {
    await trip(tester);
    await arrange(tester);
    final board = views()[2];
    await tap(tester, find.byKey(Key('tile ${board.id}')));
    expectOne(find.text('7 of 12'));
    await press(tester, LogicalKeyboardKey.arrowLeft);
    expect(placed(tiles().last), (4, 7));
    await press(tester, LogicalKeyboardKey.arrowRight, alt: true);
    expect(placed(tiles().last), (4, 8));
    await press(tester, LogicalKeyboardKey.arrowUp);
    final moved = kit.applied(BridgeMutationKind.moveViews).single;
    expect(
      (moved.view, moved.lastView, moved.after),
      (board.id, board.id, null),
    );
    expect(views().first.id, board.id);
  });

  testWidgets('dragging the right edge resizes in whole '
      'columns', (tester) async {
    await trip(tester);
    await arrange(tester);
    final board = views()[2];
    final page = tester.getRect(find.byKey(const Key('arranged page')));
    final column = (page.width + 16) / 12;
    await tester.drag(
      find.byKey(Key('resize ${board.id}')),
      Offset(-2 * column, 0),
    );
    await tester.pumpAndSettle();
    expect(placed(tiles().last), (5, 5));
  });

  testWidgets('⏎ enters a Section, whose Text Blocks take its '
      'columns', (tester) async {
    await trip(tester);
    await arrange(tester);
    final [plan, days, ..._] = views();
    await tap(tester, find.byKey(Key('tile ${plan.id}')));
    await press(tester, LogicalKeyboardKey.enter);
    await tap(tester, find.byKey(Key('select inner ${days.id}')));
    expectOne(find.text('5 of 5'));
    await press(tester, LogicalKeyboardKey.arrowLeft, alt: true);
    final inner = kit.applied(BridgeMutationKind.setInnerTile).single;
    expect((inner.view, placed(inner)), (days.id, (0, 4)));
    await press(tester, LogicalKeyboardKey.escape);
    expectOne(find.byKey(const Key('arrange bar')));
  });

  testWidgets('a tile’s measured height stays on the device and is asked for '
      'once', (tester) async {
    await kit.show(tester, [
      for (var at = 0; at < 8; at++) const WhiteboardPart(),
    ]);
    final first = views().first;
    await setTile(tester, first, const BridgeTile(column: 0, span: 6));
    final asked = kit.fake.recalled.length;
    expect(asked, greaterThan(0));
    await setTile(tester, first, const BridgeTile(column: 1, span: 6));
    expect(kit.fake.recalled.length, asked);
    await tester.pump(const Duration(seconds: 2));
    expect(kit.fake.device.keys, contains('height/${first.id}'));
  });
}
