import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeLineageRow rowOf(
  String id, {
  String title = 'Thread',
  BridgeLineageRelation relation = BridgeLineageRelation.subagent,
  BridgeLineageGroup group = BridgeLineageGroup.active,
  String statusLabel = 'Running',
  bool failed = false,
  bool mergeTarget = false,
  String? stopSubagent,
}) => BridgeLineageRow(
  threadId: id,
  relation: relation,
  group: group,
  title: title,
  statusLabel: statusLabel,
  failed: failed,
  mergeTarget: mergeTarget,
  stopSubagent: stopSubagent,
);

BridgeLineage lineageWith(
  List<BridgeLineageRow> rows, {
  int running = 0,
  int sideChatsRunning = 0,
  bool canDisconnect = false,
  String? mergeTargetTitle,
  bool mergeReady = false,
  bool canMerge = false,
}) => BridgeLineage(
  rows: rows,
  running: running,
  sideChatsRunning: sideChatsRunning,
  canDisconnect: canDisconnect,
  mergeTargetTitle: mergeTargetTitle,
  mergeReady: mergeReady,
  canMerge: canMerge,
);

void main() {
  testWidgets(
    'the related threads list under Lineage, which counts the running ones',
    (tester) async {
      final session = FakeSession()
        ..family = lineageWith([
          rowOf(
            'thread-2',
            title: 'Planner',
            relation: BridgeLineageRelation.parentThread,
            group: BridgeLineageGroup.related,
            statusLabel: 'Idle',
          ),
          rowOf('thread-3', title: 'Scout'),
        ], running: 1);
      await pumpCard(tester, session: session);
      expect(find.text('Lineage · 1 running'), findsOneWidget);
      expect(find.text('Planner'), findsOneWidget);
      expect(find.text('Scout'), findsOneWidget);
    },
  );

  testWidgets('a click on a related thread opens it in Liquid', (tester) async {
    final session = FakeSession()
      ..family = lineageWith([
        rowOf(
          'thread-2',
          title: 'Planner',
          relation: BridgeLineageRelation.parentThread,
          group: BridgeLineageGroup.related,
          statusLabel: 'Idle',
        ),
      ]);
    final state = await pumpCard(tester, session: session);
    await tester.tap(find.text('Planner'));
    await tester.pumpAndSettle();
    expect((state.project as FakeProject).calls, contains('open thread-2'));
  });

  testWidgets(
    'previous agents stay folded until opened, and their failures are counted',
    (tester) async {
      final session = FakeSession()
        ..family = lineageWith([
          rowOf(
            'old-1',
            title: 'Old run',
            group: BridgeLineageGroup.previous,
            statusLabel: 'Failed',
            failed: true,
          ),
          rowOf(
            'old-2',
            title: 'Done run',
            group: BridgeLineageGroup.previous,
            statusLabel: 'Done',
          ),
        ]);
      await pumpCard(tester, session: session);
      expect(find.text('Previous agents'), findsOneWidget);
      expect(find.text('1 failed'), findsOneWidget);
      expect(find.text('Old run'), findsNothing);
      await tester.tap(find.text('Previous agents'));
      await tester.pumpAndSettle();
      expect(find.text('Old run'), findsOneWidget);
    },
  );

  testWidgets(
    'Merge back runs from the merge target row once the fork has a finished run',
    (tester) async {
      final session = FakeSession()
        ..family = lineageWith(
          [
            rowOf(
              'main',
              title: 'Main',
              relation: BridgeLineageRelation.parentThread,
              group: BridgeLineageGroup.related,
              statusLabel: 'Done',
              mergeTarget: true,
            ),
          ],
          mergeTargetTitle: 'Main',
          mergeReady: true,
          canMerge: true,
        );
      await pumpCard(tester, session: session);
      expect(find.bySemanticsLabel('Merge back to Main'), findsOneWidget);
      await tester.tap(
        find.byTooltip('Merge this conversation back into Main'),
      );
      await tester.pumpAndSettle();
      expect(session.calls, contains('merge back'));
    },
  );

  testWidgets('Disconnect agent session is in the More menu', (tester) async {
    final session = FakeSession()
      ..family = lineageWith([], canDisconnect: true);
    await pumpCard(tester, session: session);
    await tester.tap(find.byTooltip('More thread actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Disconnect agent session'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('disconnect'));
  });

  testWidgets('a running subagent stops from its row', (tester) async {
    final session = FakeSession()
      ..family = lineageWith([
        rowOf('sub-1', title: 'Scout', stopSubagent: 'sub-1'),
      ], running: 1);
    await pumpCard(tester, session: session);
    expect(find.bySemanticsLabel('Stop subagent Scout'), findsOneWidget);
    await tester.tap(find.byTooltip('Stop subagent'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('stop subagent sub-1'));
  });

  testWidgets(
    'side chats list under their own heading, which counts the running ones',
    (tester) async {
      final session = FakeSession()
        ..family = lineageWith([
          rowOf(
            'side-1',
            title: 'Aside',
            relation: BridgeLineageRelation.sideChat,
            group: BridgeLineageGroup.sideChats,
          ),
        ], sideChatsRunning: 1);
      await pumpCard(tester, session: session);
      expect(find.text('Side chats · 1 running'), findsOneWidget);
      expect(find.text('Aside'), findsOneWidget);
    },
  );
}
