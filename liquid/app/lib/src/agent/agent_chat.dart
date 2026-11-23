import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/agents.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'agent_dialogs.dart';
import 'agents_section.dart';
import 'card_automations.dart';

@immutable
class AgentExtras {
  const AgentExtras({
    this.divider = const {},
    this.lastOfTurn = const {},
    this.mentionsAfter = const {},
  });

  final Set<int> divider;
  final Set<int> lastOfTurn;
  final Map<int, List<BridgeMention>> mentionsAfter;
}

AgentExtras agentExtras(SessionState state, List<ItemView> items) {
  final started = {
    for (final turn in state.turns.value) turn.number: turn.startedMs,
  };
  final lastOfTurn = <int>{};
  for (var index = 0; index < items.length; index++) {
    final item = items[index];
    if (item.marker != null) continue;
    if (index == items.length - 1 || items[index + 1].turn != item.turn) {
      lastOfTurn.add(item.id);
    }
  }
  final divider = <int>{};
  final visited = state.lastVisit.value;
  if (state.agents.open.value != null && visited != null) {
    final first = items.indexed
        .where(
          (entry) =>
              entry.$2.kind == BridgeItemKind.userMessage &&
              (started[entry.$2.turn] ?? 0) > visited,
        )
        .firstOrNull;
    if (first != null && first.$1 > 0) divider.add(first.$2.id);
  }
  final mentionsAfter = <int, List<BridgeMention>>{};
  for (final mention in state.agents.mentions.value) {
    ItemView? anchor;
    for (final item in items) {
      final at = started[item.turn];
      if (at == null || at <= mention.createdMs) anchor = item;
    }
    final key = anchor?.id ?? (items.isEmpty ? 0 : items.first.id);
    (mentionsAfter[key] ??= []).add(mention);
  }
  return AgentExtras(
    divider: divider,
    lastOfTurn: lastOfTurn,
    mentionsAfter: mentionsAfter,
  );
}

class SinceLastVisit extends StatelessWidget {
  const SinceLastVisit({super.key});

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final line = Expanded(child: Divider(color: ws.border, height: 1));
    return Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space4),
      child: Row(
        spacing: WsMetrics.space3,
        children: [
          line,
          Text(
            'Since your last visit',
            style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
          ),
          line,
        ],
      ),
    );
  }
}

class RoutineHeading extends StatelessWidget {
  const RoutineHeading({super.key, required this.state, required this.item});

  final SessionState state;
  final ItemView item;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final title =
        state.automations.value
            .where((task) => task.id == item.routine)
            .firstOrNull
            ?.title ??
        'Routine';
    final started = state.turns.value
        .where((turn) => turn.number == item.turn)
        .firstOrNull
        ?.startedMs;
    final when = started == null ? '' : ' · ${_when(started)}';
    return Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space2),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.end,
        spacing: WsMetrics.space2,
        children: [
          Icon(LucideIcons.calendarClock, size: 12, color: ws.textFaint),
          Text(
            '$title$when',
            style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
          ),
        ],
      ),
    );
  }
}

String _when(int ms) {
  final at = DateTime.fromMillisecondsSinceEpoch(ms);
  const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  final clock =
      '${at.hour.toString().padLeft(2, '0')}:'
      '${at.minute.toString().padLeft(2, '0')}';
  return '${days[at.weekday - 1]} $clock';
}

class OpeningLine extends StatelessWidget {
  const OpeningLine({super.key, required this.state, required this.item});

  final SessionState state;
  final ItemView item;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final name = state.agents.open.value?.name ?? 'an agent';
    return Text(
      item.opening == BridgeOpening.handoff
          ? 'Continued from the earlier thread, with $name\'s memory'
          : 'Started as $name',
      style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
    );
  }
}

class AgentTurnLines extends StatelessWidget {
  const AgentTurnLines({super.key, required this.state, required this.turn});

  final SessionState state;
  final int turn;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.agents.changes,
    builder: (context, changes, _) {
      final thread = state.session?.id;
      final shown = [
        for (final change in changes)
          if (change.turn == turn && change.thread == thread) change,
      ];
      if (shown.isEmpty) return const SizedBox.shrink();
      return Padding(
        padding: const EdgeInsets.only(top: WsMetrics.space3),
        child: Column(
          spacing: WsMetrics.space1,
          children: [
            for (final change in shown)
              _ChangeLine(state: state, change: change),
          ],
        ),
      );
    },
  );
}

class _ChangeLine extends StatelessWidget {
  const _ChangeLine({required this.state, required this.change});

  final SessionState state;
  final BridgeAgentChange change;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final agents = state.agents;
    final routine = change.routine;
    String routineText(String what) => routine == null
        ? what
        : '$what: ${routine.title} · ${scheduleLabel(routine.schedule)}';
    final (icon, text) = switch (change.kind) {
      BridgeAgentChangeKind.name => (
        LucideIcons.pencil,
        change.before == null
            ? 'Named itself ${change.after}'
            : 'Renamed itself ${change.after}',
      ),
      BridgeAgentChangeKind.instructions => (
        LucideIcons.file,
        'Instructions saved · '
            '${'\n'.allMatches((change.after ?? '').trimRight()).length + 1} lines',
      ),
      BridgeAgentChangeKind.routineAdded => (
        LucideIcons.calendarClock,
        routineText('Routine added'),
      ),
      BridgeAgentChangeKind.routineRemoved => (
        LucideIcons.calendarClock,
        routineText('Routine removed'),
      ),
      BridgeAgentChangeKind.routineChanged => (
        LucideIcons.calendarClock,
        routineText('Routine changed'),
      ),
    };
    final link = TextStyle(color: ws.accent, fontSize: WsMetrics.fsSmall);
    Widget action(String label, VoidCallback onTap) => InkWell(
      key: Key('change-${change.id}-$label'),
      onTap: onTap,
      child: Text(label, style: link),
    );
    final live =
        change.kind == BridgeAgentChangeKind.routineAdded &&
        routine != null &&
        state.automations.value.any((task) => task.id == routine.id);
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space2,
      ),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        spacing: WsMetrics.space3,
        children: [
          Icon(icon, size: 14, color: ws.textFaint),
          Expanded(
            child: Text(
              text,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
            ),
          ),
          if (change.undone)
            Text(
              'Undone',
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            )
          else ...[
            if (change.kind == BridgeAgentChangeKind.name)
              action(
                'Rename',
                () => unawaited(showRenameAgent(context, agents)),
              ),
            if (change.kind == BridgeAgentChangeKind.instructions)
              action(
                'Open',
                () => unawaited(showEditInstructions(context, agents)),
              ),
            if (live)
              action(
                'Run now',
                () => unawaited(state.runAutomation(routine.id)),
              ),
            if (change.kind != BridgeAgentChangeKind.name ||
                change.before != null)
              action('Undo', () => unawaited(agents.undo(change))),
          ],
        ],
      ),
    );
  }
}

class MentionBlock extends StatelessWidget {
  const MentionBlock({super.key, required this.state, required this.mention});

  final SessionState state;
  final BridgeMention mention;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final prose = Theme.of(context).textTheme.bodyLarge;
    return Padding(
      padding: const EdgeInsets.only(top: WsMetrics.space4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space3,
        children: [
          Align(
            alignment: Alignment.centerRight,
            child: Container(
              padding: const EdgeInsets.symmetric(
                horizontal: WsMetrics.space5,
                vertical: WsMetrics.space4,
              ),
              decoration: BoxDecoration(
                color: ws.hover,
                borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
              ),
              child: Text('@${mention.name} ${mention.text}', style: prose),
            ),
          ),
          Row(
            spacing: WsMetrics.space2,
            children: [
              AgentInitial(mention.name[0].toUpperCase()),
              Text(
                mention.name,
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  fontWeight: WsMetrics.fwSemibold,
                  color: ws.text,
                ),
              ),
              Text(
                mention.answered
                    ? 'answered from its thread'
                    : 'is answering in its thread',
                style: TextStyle(
                  color: ws.textFaint,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
              const Spacer(),
              if (state.agents.list.value
                      .where((hosted) => hosted.agent.id == mention.agent)
                      .firstOrNull
                  case final hosted?)
                InkWell(
                  onTap: () => unawaited(state.agents.show(hosted)),
                  child: Text(
                    'Open',
                    style: TextStyle(
                      color: ws.accent,
                      fontSize: WsMetrics.fsSmall,
                    ),
                  ),
                ),
            ],
          ),
          if (mention.answer.isNotEmpty)
            SelectableText(mention.answer, style: prose),
        ],
      ),
    );
  }
}
