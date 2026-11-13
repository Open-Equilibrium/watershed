import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../diff/revert_dialog.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../session/session_graph.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'input_lane.dart';
import 'sidebar.dart';
import 'start_page.dart';

/// The chat timeline of the current branch. It builds only visible items, each item rebuilds
/// alone while its text streams (client architecture rules 5 and 6), and scrolling to the top
/// reads earlier stored entries.
class ChatView extends StatelessWidget {
  const ChatView({
    super.key,
    required this.state,
    required this.info,
    this.input,
    this.onOpenDiff,
  });

  final SessionState state;

  /// Shows the Diff tab with a turn's changes, by the turn's id.
  final ValueChanged<String>? onOpenDiff;

  /// The session's project, harness and device, as the start page greets with them.
  final SessionInfo info;

  /// The input area, floating at the bottom.
  final Widget? input;

  /// How close to the oldest item reading earlier entries starts, in pixels.
  static const _readAhead = 600.0;

  @override
  Widget build(BuildContext context) {
    return InputLane(
      input: input,
      child: NotificationListener<Notification>(
        onNotification: (notification) {
          // Layout changes count as well: a newest page that fits the viewport never scrolls.
          if (notification
              case ScrollNotification(:final metrics) ||
                  ScrollMetricsNotification(:final metrics)) {
            if (metrics.pixels >= metrics.maxScrollExtent - _readAhead) {
              state.loadEarlier();
            }
          }
          return false;
        },
        child: ValueListenableBuilder(
          valueListenable: state.items,
          // A session without a conversation shows the start page.
          builder: (context, items, _) => items.isEmpty
              ? StartPage(state: state, info: info)
              // Messages start at the top and flow down; a long chat
              // still keeps its newest message above the input.
              : Align(
                  alignment: Alignment.topCenter,
                  child: ListView.builder(
                    shrinkWrap: true,
                    reverse: true,
                    padding: EdgeInsets.only(
                      top: WsMetrics.space5,
                      bottom:
                          WsMetrics.space5 +
                          (input == null ? 0 : inputClearance),
                    ),
                    itemCount: items.length,
                    itemBuilder: (context, index) {
                      final item = items[items.length - 1 - index];
                      return Center(
                        child: SizedBox(
                          width: readingWidth,
                          child: Padding(
                            padding: const EdgeInsets.only(
                              bottom: WsMetrics.space4,
                            ),
                            child: RepaintBoundary(
                              child: switch ((item.marker, item.turnEnd)) {
                                (final marker?, _) => BranchMark(
                                  marker: marker,
                                ),
                                (_, final turn?) => TurnChanges(
                                  turn: turn,
                                  onOpenDiff: onOpenDiff == null
                                      ? null
                                      : () => onOpenDiff!(turn.id),
                                  onRevert: () => unawaited(
                                    confirmRevert(context, state, turn),
                                  ),
                                ),
                                _ => ItemTile(item: item, state: state),
                              },
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                ),
        ),
      ),
    );
  }
}

/// What a turn changed, below its last item.
class TurnChanges extends StatelessWidget {
  const TurnChanges({
    super.key,
    required this.turn,
    this.onOpenDiff,
    this.onRevert,
  });

  final BridgeTurn turn;
  final VoidCallback? onOpenDiff;

  /// Asks to revert the files to before the turn.
  final VoidCallback? onRevert;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final files = turn.files ?? 0;
    final style = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    return Row(
      children: [
        Text('$files changed file${files == 1 ? '' : 's'}', style: style),
        const SizedBox(width: WsMetrics.space4),
        Text('+${turn.added ?? 0}', style: style.copyWith(color: ws.success)),
        const SizedBox(width: WsMetrics.space2),
        Text('−${turn.removed ?? 0}', style: style.copyWith(color: ws.danger)),
        if (onOpenDiff case final open?) ...[
          const SizedBox(width: WsMetrics.space3),
          GhostButton(label: 'Open diff', onPressed: open),
        ],
        if (onRevert case final revert?) ...[
          const SizedBox(width: WsMetrics.space1),
          // A narrow window shortens its label rather than the counts.
          Flexible(
            child: GhostButton(
              label: 'Revert files to before this turn…',
              icon: LucideIcons.undo2,
              onPressed: revert,
            ),
          ),
        ],
      ],
    );
  }
}

/// Where the current branch leaves the others.
class BranchMark extends StatelessWidget {
  const BranchMark({super.key, required this.marker});

  final BranchMarker marker;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = TextStyle(color: ws.accent, fontSize: WsMetrics.fsSmall);
    final ref = marker.ref;
    return Container(
      height: 24,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        color: ws.accentSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.gitBranch, size: 13, color: ws.accent),
          const SizedBox(width: WsMetrics.space2),
          if (ref != null) ...[
            RefBadge(ref: ref),
            const SizedBox(width: WsMetrics.space2),
          ] else ...[
            Text('New session branch', style: style),
            const SizedBox(width: WsMetrics.space1),
          ],
          Text('continues after step ${marker.step}', style: style),
          const Spacer(),
          if (marker.other case final other?)
            Text('$other: ${marker.otherMore} more', style: style),
        ],
      ),
    );
  }
}

/// One timeline entry: a user bubble, model prose, a line for reasoning or a tool row.
class ItemTile extends StatelessWidget {
  const ItemTile({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final prose = Theme.of(context).textTheme.bodyLarge;
    return ValueListenableBuilder(
      valueListenable: item.text,
      builder: (context, text, _) => switch (item.kind) {
        BridgeItemKind.userMessage => Align(
          alignment: Alignment.centerRight,
          child: Container(
            constraints: const BoxConstraints(maxWidth: readingWidth * 0.8),
            padding: const EdgeInsets.symmetric(
              horizontal: WsMetrics.space5,
              vertical: WsMetrics.space4,
            ),
            decoration: BoxDecoration(
              color: ws.hover,
              borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
            ),
            child: Text(text, style: prose),
          ),
        ),
        // The text's own scrolling must not read earlier entries: it stops here.
        BridgeItemKind.assistantMessage => NotificationListener<Notification>(
          onNotification: (notification) =>
              notification is ScrollNotification ||
              notification is ScrollMetricsNotification,
          child: SelectableText(text, style: prose),
        ),
        BridgeItemKind.reasoning => Text(
          'Thinking',
          style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
        ),
        _ => ToolRow(item: item),
      },
    );
  }
}

/// A tool call: its state, tool and target, and its outcome.
class ToolRow extends StatelessWidget {
  const ToolRow({super.key, required this.item});

  final ItemView item;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: item.outcome,
      builder: (context, outcome, _) {
        final (mark, markColor) = switch (outcome) {
          (failed: true, durationMs: _, lines: _) => (LucideIcons.x, ws.danger),
          (failed: false, durationMs: _, lines: _) => (
            LucideIcons.check,
            ws.success,
          ),
          null => (LucideIcons.loader, ws.textFaint),
        };
        // A file change shows its lines, other tools how long they ran.
        final lines = outcome?.lines;
        final trailing = [
          if (lines != null)
            ...toneSpans(
              ws,
              lineParts(
                lines.added > 0 ? lines.added : null,
                lines.removed > 0 ? lines.removed : null,
              ),
            )
          else if (outcome?.durationMs != null)
            TextSpan(text: formatDuration(outcome!.durationMs!)),
        ];
        return Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          decoration: BoxDecoration(
            color: ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              Icon(mark, size: 14, color: markColor),
              const SizedBox(width: WsMetrics.space3),
              // One line, so a long tool name shortens with its target.
              Expanded(
                child: Text.rich(
                  TextSpan(
                    children: [
                      TextSpan(
                        text: item.tool ?? '',
                        style: TextStyle(
                          color: ws.textMuted,
                          fontSize: WsMetrics.fsCompact,
                        ),
                      ),
                      const WidgetSpan(
                        child: SizedBox(width: WsMetrics.space3),
                      ),
                      TextSpan(
                        text: item.target ?? '',
                        style: monoStyle(context),
                      ),
                    ],
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              Text.rich(
                TextSpan(children: trailing),
                style: TextStyle(
                  color: ws.textMuted,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

/// A branch's letter in the accent color.
class RefBadge extends StatelessWidget {
  const RefBadge({super.key, required this.ref});

  final String ref;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.accent,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        ref,
        style: TextStyle(
          color: ws.onAccent,
          fontSize: WsMetrics.fsSmall,
          height: WsMetrics.lhSmall / WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwMedium,
        ),
      ),
    );
  }
}

/// How a part of a changed-lines count reads; the chat gives each tone its color.
enum Tone { good, bad, plain }

/// Changed lines as `+added −removed`, leaving out a missing count.
List<(String, Tone)> lineParts(int? added, int? removed) => [
  if (added != null) ('+$added', Tone.good),
  if (added != null && removed != null) (' ', Tone.plain),
  if (removed != null) ('−$removed', Tone.bad),
];

/// `parts` as spans in their tones' colors.
List<TextSpan> toneSpans(WsPalette ws, List<(String, Tone)> parts) => [
  for (final (text, tone) in parts)
    TextSpan(
      text: text,
      style: switch (tone) {
        Tone.good => TextStyle(color: ws.success),
        Tone.bad => TextStyle(color: ws.danger),
        Tone.plain => null,
      },
    ),
];
