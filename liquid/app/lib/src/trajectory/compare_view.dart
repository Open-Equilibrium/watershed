import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'step_parts.dart';
import 'trajectory_model.dart';
import 'tree_view.dart';

/// Two branches side by side (approved mockup 3): headline metrics, the shared steps and the
/// aligned rest, with steps only one branch has marked.
class CompareView extends StatelessWidget {
  const CompareView({
    super.key,
    required this.a,
    required this.b,
    required this.comparison,
    required this.branchA,
    required this.branchB,
    this.bottom = 0,
  });

  final BridgeTrajectory a;
  final BridgeTrajectory b;
  final Comparison comparison;
  final Branch branchA;
  final Branch branchB;

  /// Room below the last row for the floating input area.
  final double bottom;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final shared = a.records.take(comparison.shared).toList();
    final prompt = b.records
        .skip(comparison.shared)
        .where((record) => record.kind == BridgeRecordKind.user)
        .firstOrNull
        ?.label;
    var numberA = comparison.shared;
    var numberB = comparison.shared;
    final rows = [
      for (final (left, right) in comparison.rows)
        (
          left == null ? null : (++numberA, left),
          right == null ? null : (++numberB, right),
        ),
    ];
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        14,
        WsMetrics.space5,
        0,
      ),
      // Everything scrolls together under the table's pinned header, so a short window cuts
      // rows, never the header.
      child: CustomScrollView(
        slivers: [
          SliverToBoxAdapter(
            child: _Metrics(a: a, b: b, comparison: comparison),
          ),
          const SliverToBoxAdapter(child: SizedBox(height: WsMetrics.space4)),
          DecoratedSliver(
            decoration: BoxDecoration(
              border: Border.all(color: ws.border),
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            ),
            sliver: SliverMainAxisGroup(
              slivers: [
                PinnedHeaderSliver(
                  child: Container(
                    height: WsMetrics.rowTable,
                    color: ws.subtle,
                    child: Row(
                      children: [
                        _Header(branch: branchA, text: branchA.name),
                        Container(width: 1, color: ws.border),
                        _Header(
                          branch: branchB,
                          text: prompt == null ? branchB.name : '“$prompt”',
                          quote: prompt != null,
                        ),
                      ],
                    ),
                  ),
                ),
                SliverToBoxAdapter(
                  child: Container(
                    height: 30,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      border: Border(top: BorderSide(color: ws.border)),
                    ),
                    child: Text(
                      shared.isEmpty
                          ? 'No shared steps'
                          : 'Steps 1–${shared.length} shared · '
                                '${shared.take(3).map(_sharedLabel).join(', ')}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ),
                ),
                SliverFixedExtentList(
                  itemExtent: WsMetrics.rowTable,
                  delegate: SliverChildBuilderDelegate((context, index) {
                    final (left, right) = rows[index];
                    return Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _Cell(step: left, other: right, side: 'A'),
                        Container(width: 1, color: ws.border),
                        _Cell(step: right, other: left, side: 'B'),
                      ],
                    );
                  }, childCount: rows.length),
                ),
              ],
            ),
          ),
          SliverToBoxAdapter(
            child: SizedBox(height: WsMetrics.space4 + bottom),
          ),
        ],
      ),
    );
  }

  static String _sharedLabel(BridgeRecord record) => switch (record.kind) {
    BridgeRecordKind.user => 'prompt',
    BridgeRecordKind.message => 'message',
    BridgeRecordKind.tool || BridgeRecordKind.approval => record.label,
  };
}

class _Header extends StatelessWidget {
  const _Header({required this.branch, required this.text, this.quote = false});

  final Branch branch;
  final String text;
  final bool quote;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Expanded(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
        child: Row(
          children: [
            RefBadge(ref: branch.ref, shown: true),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Text(
                text,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: quote ? ws.textMuted : ws.text,
                  fontWeight: quote
                      ? WsMetrics.fwRegular
                      : WsMetrics.fwSemibold,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Cell extends StatelessWidget {
  const _Cell({required this.step, required this.other, required this.side});

  final (int, BridgeRecord)? step;
  final (int, BridgeRecord)? other;
  final String side;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final step = this.step;
    final only = step != null && other == null;
    return Expanded(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
        decoration: BoxDecoration(
          color: only ? ws.warningSubtle : null,
          border: Border(top: BorderSide(color: ws.border)),
        ),
        child: step == null
            ? null
            : Row(
                children: [
                  SizedBox(
                    width: 22,
                    child: Text(
                      '${step.$1}',
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsCompact,
                      ),
                    ),
                  ),
                  KindTag(step.$2.kind),
                  const SizedBox(width: WsMetrics.space3),
                  Expanded(child: StepText(step.$2)),
                  if (only) ...[
                    const SizedBox(width: WsMetrics.space3),
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space2,
                      ),
                      decoration: BoxDecoration(
                        color: ws.canvas,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                      ),
                      child: Text(
                        'only in $side',
                        style: TextStyle(
                          color: ws.warning,
                          fontSize: WsMetrics.fsSmall,
                        ),
                      ),
                    ),
                  ],
                  const SizedBox(width: WsMetrics.space3),
                  Flexible(
                    child: Text(
                      _outcome(step.$2),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: ws.textMuted,
                        fontSize: WsMetrics.fsCompact,
                        fontFeatures: const [FontFeature.tabularFigures()],
                      ),
                    ),
                  ),
                ],
              ),
      ),
    );
  }

  static String _outcome(BridgeRecord record) {
    final duration = record.durationMs;
    final time = duration == null ? null : formatDuration(duration);
    final usage = record.usage;
    return switch (record.kind) {
      BridgeRecordKind.message when usage != null => [
        ?time,
        '${formatCompact(promptTokens(usage))} → ${formatCompact(usage.output)}',
      ].join(' · '),
      BridgeRecordKind.approval => [
        decisionLabel(record.decision),
        ?time,
      ].join(' · '),
      BridgeRecordKind.tool => outcomeText(record.outcome) ?? time ?? '',
      _ => time ?? '',
    };
  }
}

/// Headline numbers of both branches.
class _Metrics extends StatelessWidget {
  const _Metrics({required this.a, required this.b, required this.comparison});

  final BridgeTrajectory a;
  final BridgeTrajectory b;
  final Comparison comparison;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final totalsA = _Totals(a);
    final totalsB = _Totals(b);
    final durationA = math.max(0, a.durationMs - comparison.idleA);
    final durationB = math.max(0, b.durationMs - comparison.idleB);
    String change(int from, int to) {
      if (from == 0) return '';
      final percent = ((to - from) / from * 100).round();
      return percent == 0
          ? 'same'
          : '${percent > 0 ? '+' : '−'}${percent.abs()} %';
    }

    Color tone(int from, int to) => to < from ? ws.success : ws.textMuted;
    return Row(
      children: [
        for (final (index, (label, value, note, color)) in [
          (
            'Duration',
            '${formatDuration(durationA)} → ${formatDuration(durationB)}',
            change(durationA, durationB),
            tone(durationA, durationB),
          ),
          (
            'Tokens in / out',
            '${formatCompact(totalsA.input)} → ${formatCompact(totalsB.input)}',
            '${formatCompact(totalsA.output)} → ${formatCompact(totalsB.output)} out',
            ws.textMuted,
          ),
          _files(ws, changedFiles(a.records), changedFiles(b.records)),
          _tests(ws, lastTests(a.records), lastTests(b.records)),
        ].indexed) ...[
          if (index > 0) const SizedBox(width: 10),
          Expanded(
            child: Container(
              padding: const EdgeInsets.symmetric(
                horizontal: WsMetrics.space4,
                vertical: WsMetrics.space3,
              ),
              decoration: BoxDecoration(
                color: ws.subtle,
                borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    label,
                    style: TextStyle(
                      color: ws.textMuted,
                      fontSize: WsMetrics.fsSmall,
                    ),
                  ),
                  Text(
                    value,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: WsMetrics.fsH3,
                      height: 24 / WsMetrics.fsH3,
                      fontWeight: WsMetrics.fwSemibold,
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                  ),
                  Text(
                    note,
                    style: TextStyle(color: color, fontSize: WsMetrics.fsSmall),
                  ),
                ],
              ),
            ),
          ),
        ],
      ],
    );
  }
}

/// Files the branches changed, and whether only one changed tests.
(String, String, String, Color) _files(
  WsPalette ws,
  Set<String> a,
  Set<String> b,
) {
  final testsA = a.any(isTestPath);
  final testsB = b.any(isTestPath);
  return (
    'Files changed',
    '${a.length} → ${b.length}',
    testsA == testsB
        ? (testsA ? 'tests changed in both' : 'no test changes')
        : 'no test changes in ${testsA ? 'B' : 'A'}',
    testsA == testsB ? ws.textMuted : ws.warning,
  );
}

/// The branches' last test runs.
(String, String, String, Color) _tests(
  WsPalette ws,
  BridgeToolOutcome? a,
  BridgeToolOutcome? b,
) {
  String count(BridgeToolOutcome? run) => run?.testsPassed?.toString() ?? '–';
  final failedA = a?.testsFailed ?? 0;
  final failedB = b?.testsFailed ?? 0;
  final note = a == null && b == null
      ? 'no test runs'
      : a == null || b == null
      ? 'no test run in ${a == null ? 'A' : 'B'}'
      : failedA + failedB == 0
      ? 'all passed in both'
      : '$failedA → $failedB failed';
  return (
    'Tests',
    '${count(a)} → ${count(b)}',
    note,
    failedA + failedB > 0
        ? ws.danger
        : a == null || b == null
        ? ws.warning
        : ws.textMuted,
  );
}

class _Totals {
  _Totals(BridgeTrajectory trajectory) {
    for (final record in trajectory.records) {
      final usage = record.usage;
      if (usage != null) {
        input += promptTokens(usage);
        output += usage.output;
      }
    }
  }

  int input = 0;
  int output = 0;
}
