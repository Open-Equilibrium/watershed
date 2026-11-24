import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeSchedule scheduleOf(
  BridgeScheduleKind kind, {
  String? timeOfDay,
  int? everyMs,
  String? runAt,
  List<int> weekdays = const [],
}) => BridgeSchedule(
  kind: kind,
  timeOfDay: timeOfDay,
  everyMs: everyMs,
  runAt: runAt,
  weekdays: Int64List.fromList(weekdays),
);

BridgeSchedule daily(String time, {List<int> weekdays = const []}) =>
    scheduleOf(
      BridgeScheduleKind.fixedTime,
      timeOfDay: time,
      weekdays: weekdays,
    );

BridgeAutomation taskOf({
  String id = 'a1',
  String title = 'Nightly check',
  BridgeRunStatus lastRun = BridgeRunStatus.never,
  int? nextRunMs,
  BridgeSchedule? schedule,
}) => BridgeAutomation(
  id: id,
  title: title,
  schedule: schedule ?? daily('09:30'),
  enabled: true,
  nextRunMs: nextRunMs,
  lastRun: lastRun,
);

class FailingRuns extends FakeSession {
  @override
  Future<void> runAutomationNow(String id) => Future.error('boom');
}

void main() {
  testWidgets('a task shows its next run and runs or pauses', (tester) async {
    final session = FakeSession()
      ..automationList = [
        taskOf(
          nextRunMs: DateTime.now()
              .add(const Duration(days: 3))
              .millisecondsSinceEpoch,
        ),
      ];
    await pumpCard(tester, session: session);
    expect(find.text('Automations'), findsOneWidget);
    expect(find.text('Nightly check'), findsOneWidget);
    expect(find.text('Daily at 09:30 · next in 3d'), findsOneWidget);

    expect(find.bySemanticsLabel('Run Nightly check now'), findsOneWidget);
    await tester.tap(find.byTooltip('Run now'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('run automation a1'));

    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();
    expect(session.calls, contains('automation a1 enabled=false'));
  });

  testWidgets('webhooks lack Run now, and a running one is off', (
    tester,
  ) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..automationList = [
          taskOf(schedule: scheduleOf(BridgeScheduleKind.webhook)),
        ],
    );
    expect(find.text('Nightly check'), findsOneWidget);
    expect(find.byTooltip('Run now'), findsNothing);

    await pumpCard(
      tester,
      session: FakeSession()
        ..automationList = [taskOf(lastRun: BridgeRunStatus.running)],
    );
    final run = tester.widget<IconButton>(
      find.ancestor(
        of: find.byTooltip('Run now'),
        matching: find.byType(IconButton),
      ),
    );
    expect(run.onPressed, isNull);
  });

  testWidgets('a task that fails to run is reported', (tester) async {
    final state = await pumpCard(
      tester,
      session: FailingRuns()..automationList = [taskOf()],
    );
    await tester.tap(find.byTooltip('Run now'));
    await tester.pumpAndSettle();
    expect(state.notice.value?.text, 'Could not run automation: boom');
  });

  testWidgets('a long automation list that shrinks regains full density', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 1400));
    final state = await pumpCard(
      tester,
      session: FakeSession()
        ..automationList = [
          for (var index = 0; index < 30; index++)
            taskOf(id: 'a$index', title: 'Task $index'),
        ],
      height: 700,
    );
    expect(find.text('Automations'), findsNothing, reason: 'too long: compact');

    state.automations.value = [taskOf()];
    await tester.pumpAndSettle();
    expect(find.text('Automations'), findsOneWidget);
  });
}
