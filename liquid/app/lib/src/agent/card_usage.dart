import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/details.dart';
import '../theme/theme.dart';
import 'fill_bar.dart';
import 'session_card_rows.dart';
import 'session_card_statistics.dart';

/// Whether the usage limits show: the provider reports windows, or says why it has none.
bool showsLimits(BridgeUsageLimits limits) =>
    limits.windows.isNotEmpty || limits.notice != null;

/// The Usage limits section, as T3 Code's thread details show it, under no heading: each window's
/// share left, with a warning at 20 % left and an error at 10 %, or the notice when there are no
/// windows.
class UsageSection extends StatelessWidget {
  const UsageSection({super.key, required this.limits, required this.divided});

  final BridgeUsageLimits limits;
  final bool divided;

  @override
  Widget build(BuildContext context) => CardSection(
    divided: divided,
    children: [
      if (limits.windows.isEmpty)
        StatisticsRow(
          label: 'Limits',
          value: const Text('Unavailable'),
          tip: [?limits.notice],
        ),
      for (final window in limits.windows) _WindowRow(window: window),
    ],
  );
}

class _WindowRow extends StatelessWidget {
  const _WindowRow({required this.window});

  final BridgeUsageWindow window;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final left = window.remainingPercent;
    final tone = left <= 10
        ? ws.danger
        : left <= 20
        ? ws.warning
        : null;
    final resetsAt = window.resetsAtMs;
    final pace = window.pace;
    return StatisticsRow(
      label: window.label,
      bar: FillBar(
        fraction: left / 100,
        track: ws.borderStrong,
        fill: tone ?? ws.textFaint,
        mark: ws.textMuted,
      ),
      value: Text(
        '$left%',
        style: tone == null ? null : TextStyle(color: tone),
      ),
      tip: [
        '${window.label}: $left% left',
        if (resetsAt != null)
          'Resets in ${formatDuration(resetsAt - DateTime.now().millisecondsSinceEpoch)} · ${formatClockMs(resetsAt)}',
        if (pace != null) _paceLabel(pace),
      ],
    );
  }

  static String _paceLabel(BridgePace pace) => switch (pace) {
    BridgePace.ahead => 'Ahead of pace',
    BridgePace.on_ => 'On pace',
    BridgePace.under => 'Under pace',
  };
}
