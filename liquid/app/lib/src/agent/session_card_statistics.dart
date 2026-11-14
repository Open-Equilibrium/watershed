import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../session/session_figures.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'segment_bar.dart';

/// The width of a figure row's value column.
const _valueWidth = 110.0;

/// A figure row of the session card: label, hairline and value, as T3 Code's usage rows read.
/// The row turns its background on hover, and its tooltip gives the exact parts (ADR-0279).
class FigureRow extends StatefulWidget {
  const FigureRow({
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
  State<FigureRow> createState() => _FigureRowState();
}

class _FigureRowState extends State<FigureRow> {
  bool _hovered = false;

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
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Container(
        height: 24,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        decoration: BoxDecoration(
          color: _hovered && tip != null ? ws.hover : null,
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
    return Tooltip(
      richMessage: _tipSpan(ws, tip),
      decoration: BoxDecoration(
        color: ws.text,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      child: row,
    );
  }

  static InlineSpan _tipSpan(WsPalette ws, List<String> lines) {
    final style = TextStyle(
      fontSize: WsMetrics.fsSmall,
      color: ws.canvas,
      height: 1.4,
    );
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

/// The Context row: the share of the window the next request fills, with a tick where pi
/// compacts. Its value is a dash while the tokens are not known.
class ContextFigure extends StatelessWidget {
  const ContextFigure({super.key, required this.window});

  final BridgeContext window;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final used = window.tokens;
    final limit = window.window;
    final compacts = window.compactsAt;
    return FigureRow(
      label: 'Context',
      bar: FillBar(
        fraction: used == null || limit == 0 ? null : used / limit,
        tick: compacts == null || limit == 0 ? null : compacts / limit,
        track: ws.borderStrong,
        fill: ws.textFaint,
        mark: ws.textMuted,
      ),
      value: Text(
        used == null ? '—' : '${formatTokens(used)} / ${formatTokens(limit)}',
      ),
      tip: [
        'Context',
        if (used == null)
          'Not known right after a compaction'
        else ...[
          '${formatShare(used, limit)} used',
          '${formatTokens(used)} of ${formatTokens(limit)} tokens',
        ],
        if (compacts != null) 'Compacts at ${formatTokens(compacts)} tokens',
      ],
    );
  }
}

/// The session's figures rows, in the card's order: tokens, cost, working time and tool calls.
/// Each row says what its parts are when hovered.
List<Widget> sessionFigureRows(BuildContext context, SessionFigures figures) {
  final ws = context.ws;
  final usage = figures.usage;
  final cost = formatCost(figures.cost);
  final unpriced = figures.unpriced;
  final failed = figures.toolsFailed;
  return [
    FigureRow(
      label: 'Tokens',
      bar: SegmentBar(
        segments: [
          BarSegment(usage.input, ws.data1),
          BarSegment(usage.cacheRead, ws.data2),
          BarSegment(usage.cacheWrite, ws.textFaint),
          BarSegment(usage.output, ws.data3),
        ],
      ),
      value: Text(formatTokens(figures.tokens)),
      tip: [
        'Tokens: ${formatTokens(figures.tokens)}',
        'Input ${formatTokens(usage.input)}',
        'Cache read ${formatTokens(usage.cacheRead)}',
        'Cache write ${formatTokens(usage.cacheWrite)}',
        'Output ${formatTokens(usage.output)}',
      ],
    ),
    FigureRow(
      label: 'Cost',
      value: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: 6,
        children: [
          Text('≈ $cost'),
          if (unpriced > 0)
            Icon(LucideIcons.info, size: 13, color: ws.textFaint),
        ],
      ),
      tip: unpriced == 0
          ? null
          : [
              '≈ $cost as pi recorded it',
              unpriced == 1
                  ? '1 response without a recorded cost is counted in tokens, not in cost'
                  : '$unpriced responses without a recorded cost are counted in tokens, not in cost',
            ],
    ),
    ..._timeRows(ws, figures),
    FigureRow(
      label: 'Tool calls',
      bar: SegmentBar(
        segments: [
          BarSegment(figures.toolsSucceeded, ws.textFaint),
          BarSegment(failed, ws.danger),
          BarSegment(figures.toolsCancelled, ws.borderStrong),
          BarSegment(figures.toolsRunning, ws.textFaint, hatched: true),
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
            TextSpan(text: '${figures.toolCalls}'),
          ],
        ),
      ),
      tip: figures.toolCalls == 0
          ? null
          : [
              'Tool calls: ${figures.toolCalls}',
              _outcomes(figures),
              ?_failingTools(figures),
            ],
    ),
  ];
}

/// The working time row, with its model and tool parts when every turn recorded them.
List<Widget> _timeRows(WsPalette ws, SessionFigures figures) {
  final working = formatSpan(figures.workingMs);
  final (model, tool, other) = (
    figures.modelMs,
    figures.toolMs,
    figures.otherMs,
  );
  if (model == null || tool == null || other == null) {
    return [
      FigureRow(
        label: 'Working time',
        value: Text(working),
        tip: ['Working time $working'],
      ),
      FigureRow(
        label: 'Model and tools',
        faint: true,
        value: const Text('—'),
        tip: const [
          'Model and tool time',
          'Not recorded: a turn without timing entries counts neither',
        ],
      ),
    ];
  }
  return [
    FigureRow(
      label: 'Working time',
      bar: SegmentBar(
        segments: [
          BarSegment(model, ws.data1),
          BarSegment(tool, ws.data2),
          BarSegment(other, ws.textFaint),
        ],
      ),
      value: Text(working),
      tip: [
        'Working time $working',
        'Model ${formatSpan(model)}',
        'Tools ${formatSpan(tool)}',
        'Other ${formatSpan(other)}',
      ],
    ),
  ];
}

String _outcomes(SessionFigures figures) {
  final running = figures.toolsRunning;
  return [
    '${figures.toolsSucceeded} succeeded',
    '${figures.toolsFailed} failed',
    '${figures.toolsCancelled} cancelled',
    if (running > 0) '$running running',
  ].join(' · ');
}

/// The tools with failed calls, the most failed first, as a tooltip line.
String? _failingTools(SessionFigures figures) {
  final failing = [
    for (final tool in figures.statistics.tools)
      if (tool.failed > 0) tool,
  ]..sort((a, b) => b.failed.compareTo(a.failed));
  if (failing.isEmpty) return null;
  return failing.map((tool) => '${tool.name} ${tool.failed} failed').join(', ');
}

/// The Statistics row, which opens the session's statistics panel.
class StatisticsRow extends StatelessWidget {
  const StatisticsRow({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final shortcut = Keymap.of(context).label(Command.statistics);
    return InkWell(
      borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      hoverColor: ws.hover,
      highlightColor: Colors.transparent,
      splashColor: Colors.transparent,
      onTap: onTap,
      child: SizedBox(
        height: 32,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: Row(
            spacing: 10,
            children: [
              Icon(LucideIcons.barChart3, size: 16, color: ws.textFaint),
              Text(
                'Statistics',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  fontWeight: WsMetrics.fwMedium,
                  color: ws.text,
                ),
              ),
              const Spacer(),
              Text(
                shortcut,
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
