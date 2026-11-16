import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/card_automations.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeSchedule daily(String time, {List<int> weekdays = const []}) =>
    BridgeSchedule(
      kind: BridgeScheduleKind.fixedTime,
      timeOfDay: time,
      weekdays: Int64List.fromList(weekdays),
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
  test('a schedule reads the way T3 Code words it', () {
    final cases = {
      BridgeSchedule(
        kind: BridgeScheduleKind.interval,
        everyMs: 900000,
        weekdays: Int64List(0),
      ): 'Every 15 min',
      BridgeSchedule(
        kind: BridgeScheduleKind.interval,
        everyMs: 45000,
        weekdays: Int64List(0),
      ): 'Every 45 sec',
      daily('09:30'): 'Daily at 09:30',
      daily('09:30', weekdays: [1, 2, 3, 4, 5]): 'Weekdays at 09:30',
      daily('08:00', weekdays: [1, 3]): 'Mon, Wed at 08:00',
      BridgeSchedule(kind: BridgeScheduleKind.webhook, weekdays: Int64List(0)):
          'On webhook',
      BridgeSchedule(
        kind: BridgeScheduleKind.once,
        runAt: DateTime(2026, 11, 17, 9).toIso8601String(),
        weekdays: Int64List(0),
      ): 'Once, Nov 17, 2026, 9:00 AM',
    };
    cases.forEach((schedule, label) {
      expect(scheduleLabel(schedule), label);
    });
  });

  testWidgets('the tasks show their schedule and next run, under Automations', (
    tester,
  ) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..automationList = [
          taskOf(
            nextRunMs: DateTime.now()
                .add(const Duration(days: 3))
                .millisecondsSinceEpoch,
          ),
        ],
    );
    expect(find.text('Automations'), findsOneWidget);
    expect(find.text('Nightly check'), findsOneWidget);
    expect(find.text('Daily at 09:30 · next in 3d'), findsOneWidget);
  });

  testWidgets('Run now runs the task through the session', (tester) async {
    final session = FakeSession()..automationList = [taskOf()];
    await pumpCard(tester, session: session);
    expect(find.bySemanticsLabel('Run Nightly check now'), findsOneWidget);
    await tester.tap(find.byTooltip('Run now'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('run automation a1'));
  });

  testWidgets('a webhook task has no Run now, as T3 Code leaves it out', (
    tester,
  ) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..automationList = [
          taskOf(
            schedule: BridgeSchedule(
              kind: BridgeScheduleKind.webhook,
              weekdays: Int64List(0),
            ),
          ),
        ],
    );
    expect(find.text('Nightly check'), findsOneWidget);
    expect(find.byTooltip('Run now'), findsNothing);
  });

  testWidgets('Run now is off while the task runs', (tester) async {
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

  testWidgets('the switch pauses and resumes the task through the session', (
    tester,
  ) async {
    final session = FakeSession()..automationList = [taskOf()];
    await pumpCard(tester, session: session);
    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();
    expect(session.calls, contains('automation a1 enabled=false'));
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
}
