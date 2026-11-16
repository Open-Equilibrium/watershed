import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/details.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';

/// Whether the Lineage section shows: an agent runs, the session can be disconnected, or a thread
/// is related to it, as T3 Code's section does.
bool showsLineage(BridgeLineage lineage) =>
    lineage.running > 0 ||
    lineage.canDisconnect ||
    lineage.rows.any((row) => row.group != BridgeLineageGroup.sideChats);

/// Whether the Side chats section shows: a side chat belongs to the session.
bool showsSideChats(BridgeLineage lineage) =>
    lineage.rows.any((row) => row.group == BridgeLineageGroup.sideChats);

/// The Lineage section: the threads related to the session, in the groups T3 Code gives them. Its
/// heading counts the running agents, and its More menu disconnects the agent session.
class LineageSection extends StatelessWidget {
  const LineageSection({
    super.key,
    required this.state,
    required this.lineage,
    required this.divided,
  });

  final SessionState state;
  final BridgeLineage lineage;
  final bool divided;

  @override
  Widget build(BuildContext context) {
    List<BridgeLineageRow> rowsIn(BridgeLineageGroup group) => [
      for (final row in lineage.rows)
        if (row.group == group) row,
    ];
    final related = rowsIn(BridgeLineageGroup.related);
    final active = rowsIn(BridgeLineageGroup.active);
    final previous = rowsIn(BridgeLineageGroup.previous);
    // A lone "Previous agents" group names itself; the heading returns for a status or an action.
    final headed =
        lineage.running > 0 ||
        lineage.canDisconnect ||
        related.isNotEmpty ||
        active.isNotEmpty;
    return CardSection(
      divided: divided,
      children: [
        if (headed)
          CardHeading(
            title: lineage.running > 0
                ? 'Lineage · ${lineage.running} running'
                : 'Lineage',
            trailing: lineage.canDisconnect ? _MoreMenu(state: state) : null,
          ),
        if (related.isNotEmpty) _rows(state, lineage, related),
        if (active.isNotEmpty) _rows(state, lineage, active),
        if (previous.isNotEmpty)
          _PreviousAgents(state: state, lineage: lineage, rows: previous),
      ],
    );
  }
}

/// The Side chats section: the forks of the session that a side chat opened.
class SideChatsSection extends StatelessWidget {
  const SideChatsSection({
    super.key,
    required this.state,
    required this.lineage,
    required this.divided,
  });

  final SessionState state;
  final BridgeLineage lineage;
  final bool divided;

  @override
  Widget build(BuildContext context) {
    final chats = [
      for (final row in lineage.rows)
        if (row.group == BridgeLineageGroup.sideChats) row,
    ];
    final running = lineage.sideChatsRunning;
    return CardSection(
      divided: divided,
      children: [
        CardHeading(
          title: running > 0 ? 'Side chats · $running running' : 'Side chats',
        ),
        _rows(state, lineage, chats),
      ],
    );
  }
}

/// A list of related rows, six at first, then twelve more at a time.
PagedRows _rows(
  SessionState state,
  BridgeLineage lineage,
  List<BridgeLineageRow> rows,
) => PagedRows(
  initial: 6,
  page: 12,
  rows: [
    for (final row in rows)
      _LineageRow(state: state, lineage: lineage, row: row),
  ],
);

/// The previous agents, folded under their header until opened, which names the failed ones and
/// how many there are.
class _PreviousAgents extends StatefulWidget {
  const _PreviousAgents({
    required this.state,
    required this.lineage,
    required this.rows,
  });

  final SessionState state;
  final BridgeLineage lineage;
  final List<BridgeLineageRow> rows;

  @override
  State<_PreviousAgents> createState() => _PreviousAgentsState();
}

class _PreviousAgentsState extends State<_PreviousAgents> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      fontWeight: WsMetrics.fwRegular,
    );
    final failed = widget.rows.where((row) => row.failed).length;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CardRow(
          onTap: () => setState(() => _open = !_open),
          children: [
            Icon(LucideIcons.bot, size: 16, color: ws.textFaint),
            const Flexible(
              child: Text(
                'Previous agents',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (failed > 0)
              Text('$failed failed', style: small.copyWith(color: ws.danger)),
            if (!_open)
              Text(
                '${widget.rows.length}',
                style: small.copyWith(color: ws.textFaint),
              ),
            Transform.rotate(
              angle: _open ? 0 : -math.pi / 2,
              child: Icon(
                LucideIcons.chevronDown,
                size: 16,
                color: ws.textFaint,
              ),
            ),
          ],
        ),
        if (_open) _rows(widget.state, widget.lineage, widget.rows),
      ],
    );
  }
}

/// One related thread: its relation's icon, its title and what it did, with the merge and stop
/// buttons where they apply. A click opens the thread in Liquid.
class _LineageRow extends StatelessWidget {
  const _LineageRow({
    required this.state,
    required this.lineage,
    required this.row,
  });

  final SessionState state;
  final BridgeLineage lineage;
  final BridgeLineageRow row;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final relation = _relationLabel(row.relation);
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      fontWeight: WsMetrics.fwRegular,
    );
    final stop = row.stopSubagent;
    return Tooltip(
      message: '${row.title}\nOpen ${relation.toLowerCase()} in this chat',
      child: CardRow(
        onTap: () => unawaited(state.open(row.threadId)),
        children: [
          Icon(_relationIcon(row.relation), size: 16, color: ws.textFaint),
          Flexible(
            child: Text(
              row.title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          Text(
            _outcome(row),
            style: row.failed
                ? small.copyWith(color: ws.danger)
                : small.copyWith(color: ws.textMuted),
          ),
          if (row.mergeTarget)
            CardIconButton(
              icon: LucideIcons.gitMerge,
              tooltip: _mergeTip(lineage),
              onPressed: lineage.canMerge
                  ? () => unawaited(state.mergeBack())
                  : null,
            ),
          if (stop != null)
            CardIconButton(
              icon: LucideIcons.square,
              tooltip: 'Stop subagent',
              onPressed: () => unawaited(state.stopSubagent(stop)),
            ),
        ],
      ),
    );
  }

  /// What a row ends with: its status, or for a subagent that ran, its elapsed time.
  static String _outcome(BridgeLineageRow row) {
    final started = row.startedMs;
    if (row.failed ||
        started == null ||
        row.relation != BridgeLineageRelation.subagent) {
      return row.statusLabel;
    }
    final ended = row.endedMs ?? DateTime.now().millisecondsSinceEpoch;
    return formatDuration(ended - started);
  }

  /// The merge button's tooltip, which says what merging needs or does.
  static String _mergeTip(BridgeLineage lineage) {
    if (!lineage.mergeReady) {
      return 'Complete a run in this fork before merging it back';
    }
    final target = lineage.mergeTargetTitle;
    return target == null
        ? 'Merge this conversation back into its source'
        : 'Merge this conversation back into $target';
  }

  static String _relationLabel(BridgeLineageRelation relation) =>
      switch (relation) {
        BridgeLineageRelation.contextTransfer => 'Context transfer',
        BridgeLineageRelation.subagent => 'Subagent',
        BridgeLineageRelation.parentAgent => 'Parent agent',
        BridgeLineageRelation.fork => 'Fork',
        BridgeLineageRelation.parentThread => 'Parent thread',
        BridgeLineageRelation.sideChat => 'Side chat',
      };

  static IconData _relationIcon(BridgeLineageRelation relation) =>
      switch (relation) {
        BridgeLineageRelation.parentAgent ||
        BridgeLineageRelation.parentThread => LucideIcons.cornerLeftUp,
        BridgeLineageRelation.subagent => LucideIcons.bot,
        BridgeLineageRelation.sideChat => LucideIcons.messagesSquare,
        BridgeLineageRelation.contextTransfer ||
        BridgeLineageRelation.fork => LucideIcons.gitFork,
      };
}

/// The Lineage heading's More menu, which disconnects the agent session.
class _MoreMenu extends StatelessWidget {
  const _MoreMenu({required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => PopupMenuButton<void>(
    tooltip: 'More thread actions',
    icon: Icon(LucideIcons.ellipsis, size: 16, color: context.ws.textMuted),
    itemBuilder: (context) => [
      PopupMenuItem<void>(
        onTap: () => unawaited(state.disconnect()),
        child: Row(
          spacing: 8,
          children: [
            Icon(LucideIcons.unplug, size: 14, color: context.ws.textMuted),
            Flexible(
              child: Text(
                'Disconnect agent session',
                style: TextStyle(fontSize: WsMetrics.fsCompact),
              ),
            ),
          ],
        ),
      ),
    ],
  );
}
