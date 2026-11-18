import 'dart:async';

import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/details.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';

const _weekdayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const _monthNames = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

String scheduleLabel(BridgeSchedule schedule) => switch (schedule.kind) {
  BridgeScheduleKind.webhook => 'On webhook',
  BridgeScheduleKind.once => 'Once, ${_onceLabel(schedule.runAt)}',
  BridgeScheduleKind.interval => _everyLabel(schedule.everyMs ?? 0),
  BridgeScheduleKind.fixedTime =>
    '${_daysLabel(schedule.weekdays)} at ${schedule.timeOfDay}',
};

String _onceLabel(String? runAt) {
  final at = DateTime.tryParse(runAt ?? '')?.toLocal();
  if (at == null) return 'at an unknown time';
  final hour = at.hour % 12 == 0 ? 12 : at.hour % 12;
  final minute = at.minute.toString().padLeft(2, '0');
  final half = at.hour < 12 ? 'AM' : 'PM';
  return '${_monthNames[at.month - 1]} ${at.day}, ${at.year}, $hour:$minute $half';
}

String _everyLabel(int ms) => ms % 60000 == 0
    ? 'Every ${ms ~/ 60000} min'
    : 'Every ${(ms / 1000).round()} sec';

String _daysLabel(Iterable<BigInt> weekdays) {
  final days = [for (final day in weekdays) day.toInt()];
  if (days.isEmpty) return 'Daily';
  if (days.length == 5 && days.every((day) => day >= 1 && day <= 5)) {
    return 'Weekdays';
  }
  return days.map((day) => _weekdayNames[day]).join(', ');
}

String automationLine(BridgeAutomation task, DateTime now) {
  final schedule = scheduleLabel(task.schedule);
  final policy = task.blockedByPolicy;
  if (policy != null) return '$schedule · $policy';
  if (!task.enabled) return '$schedule · paused';
  final next = task.nextRunMs;
  return next == null ? schedule : '$schedule · next ${_nextRun(next, now)}';
}

String _nextRun(int ms, DateTime now) {
  final diff = ms - now.millisecondsSinceEpoch;
  if (diff <= 0) return formatAgo(ms, now);
  final minutes = (diff / 60000).ceil();
  if (minutes < 2) return 'in under a minute';
  if (minutes < 60) return 'in ${minutes}m';
  final hours = (minutes / 60).round();
  if (hours < 24) return 'in ${hours}h';
  return 'in ${(hours / 24).round()}d';
}

class AutomationsSection extends StatelessWidget {
  const AutomationsSection({
    super.key,
    required this.state,
    required this.tasks,
    required this.divided,
  });

  final SessionState state;
  final List<BridgeAutomation> tasks;
  final bool divided;

  @override
  Widget build(BuildContext context) => CardSection(
    divided: divided,
    children: [
      const CardHeading(title: 'Automations'),
      for (final task in tasks) _AutomationRow(state: state, task: task),
    ],
  );
}

class _AutomationRow extends StatelessWidget {
  const _AutomationRow({required this.state, required this.task});

  final SessionState state;
  final BridgeAutomation task;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final running = task.lastRun == BridgeRunStatus.running;
    return CardRow(
      children: [
        Stack(
          clipBehavior: Clip.none,
          children: [
            Icon(LucideIcons.calendarClock, size: 16, color: ws.textFaint),
            Positioned(
              right: -2,
              bottom: -2,
              child: Container(
                width: 6,
                height: 6,
                decoration: BoxDecoration(
                  color: _statusColor(ws, task.lastRun),
                  shape: BoxShape.circle,
                ),
              ),
            ),
          ],
        ),
        Flexible(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                task.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(height: 1.2),
              ),
              Text(
                automationLine(task, DateTime.now()),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: WsMetrics.fsCaption,
                  fontWeight: WsMetrics.fwRegular,
                  height: 1.2,
                  color: ws.textMuted,
                ),
              ),
            ],
          ),
        ),
        if (task.schedule.kind != BridgeScheduleKind.webhook)
          CardIconButton(
            icon: LucideIcons.play,
            tooltip: 'Run now',
            label: 'Run ${task.title} now',
            onPressed: running
                ? null
                : () => unawaited(state.runAutomation(task.id)),
          ),
        SizedBox(
          width: 36,
          height: 24,
          child: FittedBox(
            child: Semantics(
              label: task.enabled
                  ? 'Pause ${task.title}'
                  : 'Resume ${task.title}',
              child: Switch(
                materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                value: task.enabled,
                onChanged: (on) =>
                    unawaited(state.setAutomationEnabled(task.id, on)),
              ),
            ),
          ),
        ),
      ],
    );
  }

  static Color _statusColor(WsPalette ws, BridgeRunStatus status) =>
      switch (status) {
        BridgeRunStatus.never => ws.textFaint,
        BridgeRunStatus.running => ws.accent,
        BridgeRunStatus.succeeded => ws.success,
        BridgeRunStatus.failed => ws.danger,
      };
}
