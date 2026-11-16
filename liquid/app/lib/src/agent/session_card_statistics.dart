import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/thread.dart';
import '../session/session_state.dart';
import '../session/session_statistics.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/row_menu.dart';
import 'fill_bar.dart';
import 'segment_bar.dart';
import 'statistics_chart.dart';

/// The width of a statistics row's value column.
const _valueWidth = 110.0;

/// A statistics row of the session card: label, hairline and value, as T3 Code's usage rows read.
/// The row turns its background on hover, and its tooltip gives the exact parts (ADR-0279).
class StatisticsRow extends StatefulWidget {
  const StatisticsRow({
    super.key,
    required this.label,
    required this.value,
    this.bar,
    this.tip,
    this.faint = false,
  });

  final String label;
  final Widget value;

  /// The hairline between the label and the value; the value takes the row's width without one.
  final Widget? bar;

  /// The tooltip's lines, the first in bold; the row has no tooltip without them.
  final List<String>? tip;

  /// Whether the row reads as not recorded, with its label and value in faint text.
  final bool faint;

  @override
  State<StatisticsRow> createState() => _StatisticsRowState();
}

class _StatisticsRowState extends State<StatisticsRow> {
  bool _hovered = false;
  bool _focused = false;
  final _tip = GlobalKey<TooltipState>();

  /// Explains the row while the keyboard has it, as the pointer does, and hides the explanation
  /// when the keyboard leaves.
  void _focusChanged(bool focused) {
    if (!mounted) return;
    setState(() => _focused = focused);
    if (focused) {
      _tip.currentState?.ensureTooltipVisible();
    } else {
      Tooltip.dismissAllToolTips();
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      fontWeight: WsMetrics.fwRegular,
      color: widget.faint ? ws.textFaint : ws.text,
    );
    final value = DefaultTextStyle.merge(
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      softWrap: false,
      style: small.copyWith(
        fontWeight: WsMetrics.fwMedium,
        color: widget.faint ? ws.textFaint : ws.text,
        fontFeatures: const [FontFeature.tabularFigures()],
      ),
      child: widget.value,
    );
    final tip = widget.tip;
    final row = HoverRegion(
      onHover: (hovered, _) {
        if (mounted) setState(() => _hovered = hovered);
      },
      child: Container(
        height: 24,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        decoration: BoxDecoration(
          color: (_hovered || _focused) && tip != null ? ws.hover : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          spacing: 10,
          children: [
            SizedBox(
              width: 96,
              child: Text(
                widget.label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: small.copyWith(color: ws.textFaint),
              ),
            ),
            if (widget.bar case final bar?)
              Expanded(child: bar)
            else
              const Spacer(),
            // The value keeps its own column, so the hairline takes the rest of the row.
            SizedBox(
              width: _valueWidth,
              child: Align(alignment: Alignment.centerRight, child: value),
            ),
          ],
        ),
      ),
    );
    if (tip == null) return row;
    return Focus(
      onFocusChange: _focusChanged,
      child: Tooltip(
        key: _tip,
        richMessage: _tipSpan(ws, tip),
        decoration: statisticsTipDecoration(ws),
        padding: statisticsTipPadding,
        child: row,
      ),
    );
  }

  static InlineSpan _tipSpan(WsPalette ws, List<String> lines) {
    final style = statisticsTipStyle(ws).copyWith(height: 1.4);
    return TextSpan(
      style: style,
      children: [
        for (final (index, line) in lines.indexed)
          TextSpan(
            text: index == lines.length - 1 ? line : '$line\n',
            style: index == 0
                ? style.copyWith(fontWeight: WsMetrics.fwSemibold)
                : style.copyWith(color: ws.canvas.withValues(alpha: 0.72)),
          ),
      ],
    );
  }
}

/// The Context row: the share of the window the next request fills, with a tick where the agent
/// compacts. It warns from 80 % used and is an error from 90 %, as T3 Code's row does. Its value is
/// a dash while the tokens are not known.
class ContextRow extends StatelessWidget {
  const ContextRow({super.key, required this.window});

  final BridgeContext window;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final used = window.tokens;
    final limit = window.window;
    final compacts = window.compactsAt;
    final share = used == null || limit == 0 ? null : used / limit;
    final tone = share == null
        ? null
        : share >= 0.9
        ? ws.danger
        : share >= 0.8
        ? ws.warning
        : null;
    return StatisticsRow(
      label: 'Context',
      bar: FillBar(
        fraction: share,
        tick: compacts == null || limit == 0 ? null : compacts / limit,
        track: ws.borderStrong,
        fill: tone ?? ws.textFaint,
        mark: ws.textMuted,
      ),
      value: Text(
        used == null ? '—' : '${formatTokens(used)} / ${formatTokens(limit)}',
        style: tone == null ? null : TextStyle(color: tone),
      ),
      tip: [
        if (used == null) ...[
          'Context',
          'Not known right after a compaction',
        ] else ...[
          'Context: ${formatShare(used, limit)} used',
          '${formatTokens(used)} of ${formatTokens(limit)} tokens',
        ],
        if (compacts != null)
          'Compacts automatically at about ${formatTokens(compacts)} tokens.',
      ],
    );
  }
}

/// Whether a turn runs, which counts the open tool calls as running rather than cancelled.
bool turnRuns(SessionState state) =>
    state.turns.value.any((turn) => turn.status == BridgeTurnStatus.running);

/// The session's statistics rows, in the card's order: tokens, cost, times and tool calls. Each row
/// says what its parts are when hovered; T3 reports no cost and no model time (ADR-0293).
List<Widget> sessionStatisticsRows(
  BuildContext context,
  SessionStatistics statistics,
) {
  final ws = context.ws;
  final usage = statistics.usage;
  final failed = statistics.toolsFailed;
  return [
    StatisticsRow(
      label: 'Tokens',
      bar: SegmentBar(
        segments: [
          BarSegment(usage.input, ws.data1),
          BarSegment(usage.cacheRead, ws.data2),
          BarSegment(usage.cacheWrite, ws.textFaint),
          BarSegment(usage.output, ws.data3),
        ],
      ),
      value: Text(formatTokens(statistics.tokens)),
      tip: [
        'Tokens: ${formatTokens(statistics.tokens)}',
        'Input ${formatTokens(usage.input)}',
        'Cache read ${formatTokens(usage.cacheRead)}',
        'Cache write ${formatTokens(usage.cacheWrite)}',
        'Output ${formatTokens(usage.output)}',
      ],
    ),
    StatisticsRow(
      label: 'Cost',
      faint: true,
      bar: Text(
        'T3 Code reports no cost',
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
      ),
      value: const Text('—'),
      tip: const ['Cost', 'T3 Code reports no cost'],
    ),
    ..._timeRows(statistics),
    StatisticsRow(
      label: 'Tool calls',
      bar: SegmentBar(
        segments: [
          BarSegment(statistics.toolsSucceeded, ws.textFaint),
          BarSegment(failed, ws.danger),
          BarSegment(statistics.toolsCancelled, ws.borderStrong),
          BarSegment(statistics.toolsRunning, ws.textFaint, hatched: true),
        ],
      ),
      value: Text.rich(
        TextSpan(
          children: [
            if (failed > 0)
              TextSpan(
                text: '$failed failed · ',
                style: TextStyle(color: ws.danger),
              ),
            TextSpan(text: '${statistics.toolCalls}'),
          ],
        ),
      ),
      tip: statistics.toolCalls == 0
          ? null
          : [
              'Tool calls: ${statistics.toolCalls}',
              _outcomes(statistics),
              ?_failingTools(statistics),
            ],
    ),
  ];
}

/// The working time row, the tool time row, and the model time row, which shows a dash: T3 times
/// no single model request (ADR-0293).
List<Widget> _timeRows(SessionStatistics statistics) {
  final working = formatDuration(statistics.workingMs);
  final tool = statistics.toolMs;
  return [
    StatisticsRow(
      label: 'Working time',
      value: Text(working),
      tip: ['Working time $working'],
    ),
    StatisticsRow(
      label: 'Tool time',
      faint: tool == null,
      value: Text(tool == null ? '—' : formatDuration(tool)),
      tip: tool == null ? null : ['Tool time ${formatDuration(tool)}'],
    ),
    const StatisticsRow(
      label: 'Model time',
      faint: true,
      value: Text('—'),
      tip: ['Model time', 'T3 Code reports no timing of single model requests'],
    ),
  ];
}

String _outcomes(SessionStatistics statistics) {
  final running = statistics.toolsRunning;
  return [
    '${statistics.toolsSucceeded} succeeded',
    '${statistics.toolsFailed} failed',
    '${statistics.toolsCancelled} cancelled',
    if (running > 0) '$running running',
  ].join(' · ');
}

/// The tools with failed calls, the most failed first, as a tooltip line.
String? _failingTools(SessionStatistics statistics) {
  final failing = [
    for (final tool in statistics.tools)
      if (tool.failed > 0) tool,
  ]..sort((a, b) => b.failed.compareTo(a.failed));
  if (failing.isEmpty) return null;
  return failing.map((tool) => '${tool.name} ${tool.failed} failed').join(', ');
}
