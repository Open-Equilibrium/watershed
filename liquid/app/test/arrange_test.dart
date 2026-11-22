import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  List<BridgeView> views() => kit.fake.views[kit.page]!;

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
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
    Future<void> apply(BridgeMutation change) => kit.workspace.apply(change);
    await apply(
      mutation(
        BridgeMutationKind.setTile,
        page: kit.page,
        view: plan.id,
        tile: const BridgeTile(column: 0, span: 5),
      ),
    );
    await apply(
      mutation(
        BridgeMutationKind.setTile,
        page: kit.page,
        view: board.id,
        tile: const BridgeTile(column: 5, span: 7),
      ),
    );
    await apply(
      mutation(
        BridgeMutationKind.setTile,
        page: kit.page,
        view: ideas.id,
        tile: const BridgeTile(column: 0, span: 5),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder text(String shown) => find.descendant(
    of: find.byKey(const Key('arranged page')),
    matching: find.text(shown),
  );

  Future<void> arrange(WidgetTester tester) async {
    await tap(tester, find.byKey(const Key('arrange')));
    expect(find.byKey(const Key('arrange bar')), findsOneWidget);
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

  testWidgets('tiles keep their column and span and rise as high as they '
      'can', (tester) async {
    await trip(tester);
    final plan = tester.getRect(text('Plan'));
    final ideas = tester.getRect(text('Ideas'));
    final board = tester.getRect(find.byKey(ValueKey('tile ${views()[2].id}')));
    expect(board.left, greaterThan(plan.right));
    expect(board.top, lessThan(plan.bottom));
    expect(ideas.left, closeTo(plan.left, 1));
    expect(
      ideas.top,
      lessThan(board.bottom),
      reason: 'Ideas rises beside the taller board',
    );
  });

  testWidgets('Arrange shows its bar; Done and esc end it', (tester) async {
    await trip(tester);
    await arrange(tester);
    await tap(tester, find.byKey(const Key('arrange done')));
    expect(find.byKey(const Key('arrange bar')), findsNothing);
    await arrange(tester);
    await press(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('arrange bar')), findsNothing);
  });

  testWidgets('arrow keys move the selected tile, ⌥ and arrows resize it', (
    tester,
  ) async {
    await trip(tester);
    await arrange(tester);
    final board = views()[2];
    await tap(tester, find.byKey(Key('tile ${board.id}')));
    expect(find.text('7 of 12'), findsOneWidget);
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

  testWidgets('dragging the right edge resizes in whole columns', (
    tester,
  ) async {
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

  testWidgets('⏎ enters a Section, whose Text Blocks take its columns', (
    tester,
  ) async {
    await trip(tester);
    await arrange(tester);
    final [plan, days, ..._] = views();
    await tap(tester, find.byKey(Key('tile ${plan.id}')));
    await press(tester, LogicalKeyboardKey.enter);
    await tap(tester, find.byKey(Key('select inner ${days.id}')));
    expect(find.text('5 of 5'), findsOneWidget);
    await press(tester, LogicalKeyboardKey.arrowLeft, alt: true);
    final inner = kit.applied(BridgeMutationKind.setInnerTile).single;
    expect((inner.view, placed(inner)), (days.id, (0, 4)));
    await press(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('arrange bar')), findsOneWidget);
  });

  testWidgets('Narrow makes tiles under half the grid half, the rest full', (
    tester,
  ) async {
    await trip(tester);
    await arrange(tester);
    await tap(tester, find.byKey(const Key('preview Narrow')));
    final page = tester.getRect(find.byKey(const Key('arranged page')));
    final board = tester.getRect(find.byKey(ValueKey('tile ${views()[2].id}')));
    final plan = tester.getRect(find.byKey(ValueKey('tile ${views()[0].id}')));
    expect(board.width, closeTo(page.width, 1));
    expect(plan.width, closeTo((page.width - 16) / 2, 1));
  });

  testWidgets('a sideways drag moves the column and keeps the order', (
    tester,
  ) async {
    await trip(tester);
    await arrange(tester);
    final plan = views()[0];
    final page = tester.getRect(find.byKey(const Key('arranged page')));
    final column = (page.width + 16) / 12;
    await tester.drag(find.byKey(Key('move ${plan.id}')), Offset(column, 0));
    await tester.pumpAndSettle();
    expect(placed(tiles().last), (1, 5));
    expect(kit.applied(BridgeMutationKind.moveViews), isEmpty);
  });

  testWidgets('Narrow preview leaves the Desktop columns as they are', (
    tester,
  ) async {
    await trip(tester);
    await arrange(tester);
    await tap(tester, find.byKey(const Key('preview Narrow')));
    final board = views()[2];
    final before = tiles().length;
    await tester.drag(
      find.byKey(Key('resize ${board.id}')),
      const Offset(-120, 0),
    );
    await tester.drag(
      find.byKey(Key('move ${board.id}')),
      const Offset(-40, 0),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(Key('select ${board.id}')));
    await press(tester, LogicalKeyboardKey.arrowLeft);
    await press(tester, LogicalKeyboardKey.arrowLeft, alt: true);
    expect(tiles(), hasLength(before));
  });

  testWidgets('esc ends Arrange mode opened on a Page not yet arranged', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Plan')]);
    await arrange(tester);
    await press(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('arrange bar')), findsNothing);
  });

  testWidgets('a tile’s kept height is asked for once', (tester) async {
    await kit.show(tester, [
      for (var at = 0; at < 8; at++) const WhiteboardPart(),
    ]);
    final first = views().first;
    Future<void> place(int column) async {
      await kit.workspace.apply(
        mutation(
          BridgeMutationKind.setTile,
          page: kit.page,
          view: first.id,
          tile: BridgeTile(column: column, span: 6),
        ),
      );
      await tester.pumpAndSettle();
    }

    await place(0);
    final asked = kit.fake.recalled.length;
    expect(asked, greaterThan(0));
    await place(1);
    expect(kit.fake.recalled.length, asked);
  });

  testWidgets('measured heights stay on the device', (tester) async {
    await trip(tester);
    await tester.pump(const Duration(seconds: 2));
    expect(kit.fake.device.keys, contains('height/${views()[2].id}'));
  });
}
