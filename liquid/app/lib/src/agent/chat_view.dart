import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../trajectory/trajectory_model.dart';
import '../trajectory/tree_view.dart';
import 'start_page.dart';

/// Width of the reading column in the chat and the input area.
const double readingWidth = 740;

/// The chat timeline of the current branch. It builds only visible items, each item rebuilds
/// alone while its text streams (client architecture rules 5 and 6), and scrolling to the top
/// reads earlier stored entries.
class ChatView extends StatelessWidget {
  const ChatView({super.key, required this.state, required this.project});

  final SessionState state;

  /// The project's name, as the start page shows it.
  final String project;

  /// How close to the oldest item reading earlier entries starts, in pixels.
  static const _readAhead = 600.0;

  @override
  Widget build(BuildContext context) {
    return NotificationListener<ScrollNotification>(
      onNotification: (notification) {
        final metrics = notification.metrics;
        if (metrics.pixels >= metrics.maxScrollExtent - _readAhead) {
          state.loadEarlier();
        }
        return false;
      },
      child: ValueListenableBuilder(
        valueListenable: state.items,
        // A session without a conversation shows the start page (ADR-0203).
        builder: (context, items, _) => items.isEmpty
            ? StartPage(state: state, project: project)
            : ListView.builder(
                reverse: true,
                padding: const EdgeInsets.symmetric(vertical: WsMetrics.space5),
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
                          child: switch (item.marker) {
                            final marker? => BranchMark(marker: marker),
                            null => ItemTile(item: item, state: state),
                          },
                        ),
                      ),
                    ),
                  );
                },
              ),
      ),
    );
  }
}

/// Where the current branch leaves the others (approved session-flow mockup).
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
            RefBadge(
              branch: Branch(tip: '', ref: ref, name: '', lane: marker.lane),
            ),
            const SizedBox(width: WsMetrics.space2),
          ] else ...[
            Text('New branch', style: style),
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

/// One timeline entry: a user bubble, model prose, collapsed reasoning or a tool row.
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
        BridgeItemKind.assistantMessage => SelectableText(text, style: prose),
        BridgeItemKind.reasoning => Row(
          children: [
            Icon(LucideIcons.chevronRight, size: 14, color: ws.textFaint),
            const SizedBox(width: WsMetrics.space1),
            Text(
              'Thinking',
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
          ],
        ),
        _ => ToolRow(item: item, state: state),
      },
    );
  }
}

/// A tool call: its state, tool and target, and its outcome.
class ToolRow extends StatelessWidget {
  const ToolRow({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  String get _target {
    final arguments = item.arguments ?? '';
    final match = RegExp(r'"(?:command|path|pattern)"\s*:\s*"([^"]*)"')
        .firstMatch(arguments);
    return match?.group(1) ?? arguments;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ListenableBuilder(
      listenable: Listenable.merge([state.approval, item.outcome]),
      builder: (context, _) {
        final approval = state.approval.value;
        final outcome = item.outcome.value;
        final waiting =
            outcome == null &&
            approval != null &&
            approval.approval?.tool == item.tool;
        final (mark, markColor) = switch (outcome) {
          (failed: true, durationMs: _) => (LucideIcons.x, ws.danger),
          (failed: false, durationMs: _) => (LucideIcons.check, ws.success),
          null when waiting => (LucideIcons.clock, ws.warning),
          null => (LucideIcons.loader, ws.textFaint),
        };
        final trailing = waiting
            ? 'waiting for approval'
            : outcome?.durationMs != null
            ? '${outcome!.durationMs} ms'
            : '';
        return Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          decoration: BoxDecoration(
            color: waiting ? ws.warningSubtle : ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              Icon(mark, size: 14, color: markColor),
              const SizedBox(width: WsMetrics.space3),
              Text(
                item.tool ?? '',
                style: TextStyle(
                  color: waiting ? ws.warning : ws.textMuted,
                  fontSize: WsMetrics.fsCompact,
                ),
              ),
              const SizedBox(width: WsMetrics.space3),
              Expanded(
                child: Text(
                  _target,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: monoStyle(context),
                ),
              ),
              Text(
                trailing,
                style: TextStyle(
                  color: waiting ? ws.warning : ws.textMuted,
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
