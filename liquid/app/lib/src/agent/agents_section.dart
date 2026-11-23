import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../session/agents_state.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/explorer.dart';
import 'session_menus.dart';
import 'sidebar.dart';

class AgentsSection extends StatefulWidget {
  const AgentsSection({
    super.key,
    required this.state,
    this.onShow,
    this.onMove,
  });

  final SessionState state;
  final VoidCallback? onShow;
  final void Function(HostThread listed)? onMove;

  @override
  State<AgentsSection> createState() => _AgentsSectionState();
}

class _AgentsSectionState extends State<AgentsSection> {
  var _hovered = false;
  var _focused = false;

  AgentsState get agents => widget.state.agents;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ListenableBuilder(
      listenable: Listenable.merge([
        agents.list,
        agents.open,
        agents.composing,
        widget.state.recent,
        widget.state.status,
      ]),
      builder: (context, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          MouseRegion(
            onEnter: (_) => setState(() => _hovered = true),
            onExit: (_) => setState(() => _hovered = false),
            child: SidebarHeading(
              'Agents',
              trailing: Opacity(
                opacity: _hovered || _focused ? 1 : 0,
                alwaysIncludeSemantics: true,
                child: Tooltip(
                  message: 'New agent',
                  child: InkWell(
                    key: const Key('new-agent'),
                    borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                    onFocusChange: (focused) =>
                        setState(() => _focused = focused),
                    onTap: () {
                      unawaited(agents.compose());
                      widget.onShow?.call();
                    },
                    child: Padding(
                      padding: const EdgeInsets.all(WsMetrics.space1),
                      child: Icon(
                        LucideIcons.plus,
                        size: 14,
                        color: ws.textMuted,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          if (agents.composing.value) const _AgentRow.composing(),
          for (final hosted in agents.list.value)
            _AgentRow(
              state: widget.state,
              hosted: hosted,
              onShow: widget.onShow,
              onMove: widget.onMove,
            ),
        ],
      ),
    );
  }
}

class _AgentRow extends StatelessWidget {
  const _AgentRow({
    required this.state,
    required this.hosted,
    this.onShow,
    this.onMove,
  }) : composing = false;

  const _AgentRow.composing()
    : state = null,
      hosted = null,
      onShow = null,
      onMove = null,
      composing = true;

  final SessionState? state;
  final HostAgent? hosted;
  final VoidCallback? onShow;
  final void Function(HostThread listed)? onMove;
  final bool composing;

  (String, Color)? _label(BuildContext context, bool selected) {
    final state = this.state!;
    final agent = hosted!.agent;
    final ws = context.ws;
    final listed = state.recent.value
        .where(
          (listed) =>
              listed.thread.id == agent.thread &&
              listed.address == hosted!.address,
        )
        .firstOrNull;
    final status = selected
        ? state.status.value
        : listed?.thread.running == true
        ? RunStatus.running
        : RunStatus.idle;
    if (status != RunStatus.idle) return statusLabel(context, status);
    if (agent.nextRunMs case final next?) {
      return ('next ${nextRun(next, DateTime.now())}', ws.textFaint);
    }
    return null;
  }

  bool _unread(SessionState state, bool selected) {
    if (selected) return false;
    final listed = state.recent.value
        .where((listed) => listed.thread.id == hosted!.agent.thread)
        .firstOrNull;
    final updated = listed?.thread.updatedMs;
    final visited = listed?.thread.visitedMs;
    return updated != null && (visited == null || updated > visited);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (composing) {
      return _Row(
        selected: true,
        initial: '?',
        empty: true,
        name: 'New agent',
        trailing: const SizedBox.shrink(),
      );
    }
    final state = this.state!;
    final hosted = this.hosted!;
    final agent = hosted.agent;
    final selected =
        state.agents.open.value?.agent.id == agent.id &&
        !state.agents.composing.value;
    final label = _label(context, selected);
    final unread = _unread(state, selected);
    final listed = state.recent.value
        .where(
          (listed) =>
              listed.thread.id == agent.thread &&
              listed.address == hosted.address,
        )
        .firstOrNull;
    return RowMenu(
      actions: [
        RowAction(
          'Move to host…',
          () => onMove?.call(listed!),
          enabled: listed != null && listed.thread.movable && onMove != null,
          disabledReason: 'Only Claude Code and Codex agents can move.',
        ),
        null,
        RowAction(
          'Remove agent…',
          () => unawaited(confirmRemoveAgent(context, state, hosted)),
          danger: true,
        ),
      ],
      builder: (context, menu, shown) => Semantics(
        selected: selected,
        child: Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            key: Key('agent-${agent.id}'),
            onTap: () {
              if (state.session?.id != agent.thread) {
                unawaited(state.agents.show(hosted));
              }
              onShow?.call();
            },
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            child: _Row(
              selected: selected,
              initial: agent.named ? agent.name[0].toUpperCase() : '?',
              empty: !agent.named,
              name: agent.named ? agent.name : 'New agent',
              trailing: Stack(
                alignment: Alignment.centerRight,
                children: [
                  Opacity(
                    opacity: shown ? 0 : 1,
                    child: unread
                        ? Container(
                            key: const Key('agent-unread'),
                            width: 7,
                            height: 7,
                            decoration: BoxDecoration(
                              color: ws.accent,
                              shape: BoxShape.circle,
                            ),
                          )
                        : Text(
                            label?.$1 ?? '',
                            style: TextStyle(
                              color: label?.$2 ?? ws.textFaint,
                              fontSize: WsMetrics.fsSmall,
                            ),
                          ),
                  ),
                  menu,
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({
    required this.selected,
    required this.initial,
    required this.empty,
    required this.name,
    required this.trailing,
  });

  final bool selected;
  final String initial;
  final bool empty;
  final String name;
  final Widget trailing;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Ink(
      height: WsMetrics.rowSide,
      decoration: BoxDecoration(
        color: selected ? ws.hover : null,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      child: Row(
        children: [
          AgentInitial(initial, empty: empty),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: Text(name, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          trailing,
        ],
      ),
    );
  }
}

class AgentInitial extends StatelessWidget {
  const AgentInitial(this.initial, {super.key, this.empty = false});

  final String initial;
  final bool empty;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      width: 18,
      height: 18,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: empty ? ws.subtle : ws.accent,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        border: empty ? Border.all(color: ws.borderStrong) : null,
      ),
      child: Text(
        initial,
        style: TextStyle(
          color: empty ? ws.textFaint : ws.onAccent,
          fontSize: WsMetrics.fsSmall,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

String nextRun(int at, DateTime now) {
  final time = DateTime.fromMillisecondsSinceEpoch(at);
  final clock =
      '${time.hour.toString().padLeft(2, '0')}:'
      '${time.minute.toString().padLeft(2, '0')}';
  final today = DateTime(now.year, now.month, now.day);
  final day = DateTime(time.year, time.month, time.day);
  final days = day.difference(today).inDays;
  if (days <= 0) return clock;
  if (days == 1) return 'tomorrow $clock';
  const names = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  return '${names[time.weekday - 1]} $clock';
}

Future<void> confirmRemoveAgent(
  BuildContext context,
  SessionState state,
  HostAgent hosted,
) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => DangerDialog(
      title: 'Remove agent "${hosted.name}"?',
      action: 'Remove',
      children: [
        Text(
          'Its routines end. Its memory folder stays on the agent host, '
          'and its threads stay in the list of threads.',
          style: DangerDialog.text(context),
        ),
      ],
    ),
  );
  if (confirmed != true || !context.mounted) return;
  final memory = await state.agents.remove(hosted);
  if (memory != null) {
    state.notify(
      NoticeLevel.info,
      'Removed ${hosted.name}; its memory stays in $memory.',
    );
  }
}
