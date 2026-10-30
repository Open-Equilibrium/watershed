import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/history.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'context_track.dart';
import 'step_parts.dart';
import 'trajectory_model.dart';

/// Widths of the ledger's fixed columns: number, kind, time, adds, tokens in and out.
/// Width of the number column, which also holds the turns' chevrons.
const _number = 28.0;

/// Where a row's content starts after the number column, as the inspector below a step aligns.
const ledgerInset = WsMetrics.space5 + _number + WsMetrics.space3;

const _fixed = (
  number: _number,
  kind: 72.0,
  time: 56.0,
  adds: 56.0,
  tokens: 48.0,
  out: 44.0,
);

/// The context track's width in a ledger `width` wide: up to 180, and none when the step
/// would keep less than 160.
double ledgerTrackWidth(double width) {
  final rest =
      width -
      2 * WsMetrics.space5 -
      _fixed.number -
      _fixed.kind -
      _fixed.time -
      _fixed.adds -
      _fixed.tokens -
      _fixed.out -
      7 * WsMetrics.space3 -
      160;
  return rest < 120 ? 0 : math.min(rest, 180);
}

/// The ledger's columns: number, kind, step, time, adds, context track, tokens in and out;
/// a `span` widget takes kind through adds, as turn lines do.
Widget _columns(
  List<Widget> cells, {
  required double trackWidth,
  Widget? span,
}) {
  Widget fixed(double width, Widget child, {bool right = true}) => SizedBox(
    width: width,
    child: Align(
      alignment: right ? Alignment.centerRight : Alignment.centerLeft,
      child: child,
    ),
  );
  const gap = SizedBox(width: WsMetrics.space3);
  return Row(
    children: [
      SizedBox(width: _fixed.number, child: cells[0]),
      gap,
      if (span != null)
        Expanded(child: span)
      else ...[
        fixed(_fixed.kind, cells[1], right: false),
        gap,
        Expanded(child: cells[2]),
        gap,
        fixed(_fixed.time, cells[3]),
        gap,
        fixed(_fixed.adds, cells[4]),
      ],
      if (trackWidth > 0) ...[
        gap,
        SizedBox(
          width: trackWidth,
          height: WsMetrics.rowTable,
          child: cells[5],
        ),
      ],
      gap,
      fixed(_fixed.tokens, cells[6]),
      gap,
      fixed(_fixed.out, cells[7]),
    ],
  );
}

class LedgerHeader extends StatelessWidget {
  const LedgerHeader({
    super.key,
    required this.trackWidth,
    required this.allFolded,
    required this.onFoldAll,
    required this.fit,
    required this.onFit,
  });

  final double trackWidth;
  final bool allFolded;
  final VoidCallback onFoldAll;

  /// Whether the track fits the branch; `null` without a known window, which it then fits.
  final bool? fit;
  final ValueChanged<bool> onFit;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final fit = this.fit;
    return Container(
      height: 34,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: _columns([
        // In the column of the turns' chevrons (approved look mockup); return and space
        // press it rather than the view's own keys.
        Center(
          child: Tooltip(
            message: allFolded ? 'Expand all turns' : 'Collapse all turns',
            child: Shortcuts(
              shortcuts: activateKeys,
              child: InkWell(
                onTap: onFoldAll,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                child: SizedBox(
                  width: 24,
                  height: 24,
                  child: Icon(
                    allFolded
                        ? LucideIcons.chevronsUpDown
                        : LucideIcons.chevronsDownUp,
                    size: 14,
                    color: ws.textMuted,
                  ),
                ),
              ),
            ),
          ),
        ),
        for (final label in ['Kind', 'Step', 'Time', 'Adds'])
          Text(label, style: style),
        Row(
          children: [
            Flexible(
              child: Text(
                'Context',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: style,
              ),
            ),
            if (fit != null) ...[
              const SizedBox(width: WsMetrics.space2),
              Segments(
                options: const [(false, 'Window'), (true, 'Fit')],
                selected: fit,
                onSelect: onFit,
                dense: true,
              ),
            ],
          ],
        ),
        Text('In', style: style),
        Text('Out', style: style),
      ], trackWidth: trackWidth),
    );
  }
}

/// A line with a summary on it, such as a turn's: a title, the summary and a rule up to the
/// context track.
class LineRow extends StatelessWidget {
  const LineRow({
    super.key,
    required this.title,
    required this.summary,
    required this.trackWidth,
    required this.track,
    this.lead = const SizedBox.shrink(),
    this.onLead,
    this.onTap,
  });

  final String title;
  final String summary;
  final double trackWidth;
  final Widget track;

  /// What the number column shows, such as a chevron, and what a click on it does.
  final Widget lead;
  final VoidCallback? onLead;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      canRequestFocus: false,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: _columns(
          [
            if (onLead case final onLead?)
              InkWell(
                canRequestFocus: false,
                onTap: onLead,
                borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                child: SizedBox.expand(child: lead),
              )
            else
              lead,
            for (var i = 1; i < 5; i++) const SizedBox.shrink(),
            track,
            const SizedBox.shrink(),
            const SizedBox.shrink(),
          ],
          trackWidth: trackWidth,
          span: Row(
            children: [
              Text(
                title,
                style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
              ),
              const SizedBox(width: WsMetrics.space3),
              // The summary keeps its width and the rule takes the rest, at least 16.
              Expanded(
                child: LayoutBuilder(
                  builder: (context, constraints) => Row(
                    children: [
                      ConstrainedBox(
                        constraints: BoxConstraints(
                          maxWidth: math.max(0, constraints.maxWidth - 24),
                        ),
                        child: Text(
                          summary,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: ws.textFaint,
                            fontSize: WsMetrics.fsSmall,
                            fontFeatures: const [FontFeature.tabularFigures()],
                          ),
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      Expanded(
                        child: Container(height: 1, color: ws.borderStrong),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A turn's line (approved look mockup): its chevron, prompt, duration and what it added to
/// the context. A click on the chevron collapses or expands it, ⌥-click every turn; a click on
/// the line opens its last request's context.
class TurnRow extends StatelessWidget {
  const TurnRow({
    super.key,
    required this.turn,
    required this.before,
    required this.after,
    required this.folded,
    required this.trackWidth,
    required this.track,
    required this.onToggle,
    required this.onOpen,
  });

  final TurnGroup turn;

  /// The context before the turn's first step and after its last.
  final ContextLevel before;
  final ContextLevel after;
  final bool folded;
  final double trackWidth;
  final Widget track;
  final VoidCallback onToggle;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final added = after.tokens - before.tokens;
    return LineRow(
      title: turn.turn == 0 ? 'Setup' : 'Turn ${turn.turn}',
      summary: [
        if (turn.prompt.isNotEmpty) turn.prompt,
        formatDuration(turn.durationMs),
        '${added < 0 ? '−' : '+'}${formatCompact(added.abs())} → '
            '${after.exact ? '' : '≈'}${formatCompact(after.tokens)}',
      ].join(' · '),
      trackWidth: trackWidth,
      track: track,
      onTap: onOpen,
      onLead: onToggle,
      lead: Center(
        child: Icon(
          folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
          key: ValueKey('chevron-${turn.turn}'),
          size: 14,
          color: context.ws.textFaint,
        ),
      ),
    );
  }
}

class StepRow extends StatelessWidget {
  const StepRow({
    super.key,
    required this.number,
    required this.record,
    required this.request,
    required this.level,
    required this.trackWidth,
    required this.track,
    required this.selected,
    required this.onTap,
  });

  final int number;
  final BridgeRecord record;

  /// A response's request number on the branch.
  final int? request;

  /// The context after the step.
  final ContextLevel level;
  final double trackWidth;
  final Widget track;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final figures = TextStyle(
      color: ws.textMuted,
      fontSize: WsMetrics.fsCompact,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    final usage = record.usage;
    final duration = record.durationMs;
    final request = this.request;
    return InkWell(
      canRequestFocus: false,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
        decoration: BoxDecoration(
          color: selected ? ws.selected : null,
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: _columns([
          Text('$number', style: figures.copyWith(color: ws.textFaint)),
          KindTag(record.kind),
          // The result gives way before the step's name loses more than half the column.
          LayoutBuilder(
            builder: (context, constraints) => Row(
              children: [
                Flexible(child: StepText(record)),
                if (request != null) ...[
                  const SizedBox(width: WsMetrics.space2),
                  Text(
                    '#$request',
                    style: figures.copyWith(color: ws.textFaint),
                  ),
                ],
                const SizedBox(width: WsMetrics.space3),
                ConstrainedBox(
                  constraints: BoxConstraints(
                    maxWidth: constraints.maxWidth / 2,
                  ),
                  child: OutcomeText(record.outcome),
                ),
              ],
            ),
          ),
          Text(
            record.kind == BridgeRecordKind.user
                ? '+0 s'
                : duration == null
                ? ''
                : '+${formatDuration(duration)}',
            style: figures,
          ),
          Text(
            record.kind == BridgeRecordKind.approval
                ? ''
                : '${level.addedExact ? '' : '≈'}+${formatCompact(level.added)}',
            style: figures,
          ),
          track,
          Text(
            usage == null ? '' : formatCompact(promptTokens(usage)),
            style: figures,
          ),
          Text(
            usage == null ? '' : formatCompact(usage.output),
            style: figures,
          ),
        ], trackWidth: trackWidth),
      ),
    );
  }
}
