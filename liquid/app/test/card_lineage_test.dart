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
  testWidgets('Lineage lists threads, chats and old agents', (tester) async {
    final session = FakeSession()
      ..family = lineageWith(
        [
          rowOf(
            'thread-2',
            title: 'Planner',
            relation: BridgeLineageRelation.parentThread,
            group: BridgeLineageGroup.related,
            statusLabel: 'Idle',
          ),
          rowOf('thread-3', title: 'Scout'),
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
          rowOf(
            'side-1',
            title: 'Aside',
            relation: BridgeLineageRelation.sideChat,
            group: BridgeLineageGroup.sideChats,
          ),
        ],
        running: 1,
        sideChatsRunning: 1,
      );
    final state = await pumpCard(tester, session: session);
    expect(find.text('Lineage · 1 running'), findsOneWidget);
    expect(find.text('Side chats · 1 running'), findsOneWidget);
    expect(find.text('Planner'), findsOneWidget);
    expect(find.text('Scout'), findsOneWidget);
    expect(find.text('Aside'), findsOneWidget);
    expect(find.text('Previous agents'), findsOneWidget);
    expect(find.text('1 failed'), findsOneWidget);
    expect(find.text('Old run'), findsNothing);
    await tester.tap(find.text('Previous agents'));
    await tester.pumpAndSettle();
    expect(find.text('Old run'), findsOneWidget);

    await tester.tap(find.text('Planner'));
    await tester.pumpAndSettle();
    expect((state.project as FakeProject).calls, contains('open thread-2'));
  });

  testWidgets('Lineage actions reach the session', (tester) async {
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
          rowOf('sub-1', title: 'Scout', stopSubagent: 'sub-1'),
        ],
        running: 1,
        mergeTargetTitle: 'Main',
        mergeReady: true,
        canMerge: true,
        canDisconnect: true,
      );
    await pumpCard(tester, session: session);
    expect(find.bySemanticsLabel('Merge back to Main'), findsOneWidget);
    await tester.tap(find.byTooltip('Merge this conversation back into Main'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('merge back'));

    expect(find.bySemanticsLabel('Stop subagent Scout'), findsOneWidget);
    await tester.tap(find.byTooltip('Stop subagent'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('stop subagent sub-1'));

    await tester.tap(find.byTooltip('More thread actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Disconnect agent session'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('disconnect'));
  });
}
