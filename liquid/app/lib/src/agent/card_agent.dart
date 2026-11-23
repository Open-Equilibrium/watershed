import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import '../rust/api/session.dart';
import '../session/agents_state.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'agent_dialogs.dart';
import 'session_card_rows.dart';

const permissionNames = {
  BridgePermissions.supervised: 'Approval required',
  BridgePermissions.autoAcceptEdits: 'Auto-accept edits',
  BridgePermissions.auto: 'Auto',
  BridgePermissions.fullAccess: 'Full access',
};

class AgentSection extends StatefulWidget {
  const AgentSection({
    super.key,
    required this.state,
    required this.hosted,
    required this.divided,
  });

  final SessionState state;
  final HostAgent hosted;
  final bool divided;

  @override
  State<AgentSection> createState() => _AgentSectionState();
}

class _AgentSectionState extends State<AgentSection> {
  String? _instructions;
  String? _readFor;

  AgentsState get agents => widget.state.agents;

  @override
  void initState() {
    super.initState();
    agents.changes.addListener(_read);
    _read();
  }

  @override
  void didUpdateWidget(AgentSection old) {
    super.didUpdateWidget(old);
    if (old.hosted.agent.id != widget.hosted.agent.id) _read();
  }

  @override
  void dispose() {
    agents.changes.removeListener(_read);
    super.dispose();
  }

  void _read() {
    final id = widget.hosted.agent.id;
    _readFor = id;
    unawaited(
      agents.instructions().then((text) {
        if (mounted && _readFor == id) setState(() => _instructions = text);
      }),
    );
  }

  String get _summary {
    final lines = (_instructions ?? '')
        .split('\n')
        .where((line) => line.trim().isNotEmpty && !line.startsWith('# '))
        .toList();
    return lines.isEmpty ? 'No instructions yet.' : lines.take(3).join('\n');
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final agent = widget.hosted.agent;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final link = TextStyle(color: ws.accent, fontSize: WsMetrics.fsSmall);
    final earlier = [
      for (final thread in agent.threads)
        if (thread != agent.thread) thread,
    ];
    final local = widget.hosted.address == null && Platform.isMacOS;
    return CardSection(
      divided: widget.divided,
      children: [
        const CardHeading(title: 'Agent'),
        CardRow(
          onTap: () => unawaited(showRenameAgent(context, agents)),
          children: [
            const Flexible(
              child: Text('Name', maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            Expanded(
              child: Text(
                agent.named ? agent.name : 'Not named yet',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.end,
                style: faint,
              ),
            ),
            Text('Rename', style: link),
          ],
        ),
        _Choice<BridgeModelChoice>(
          label: 'Model',
          value: agent.model.model,
          options: [
            for (final provider in widget.state.providers.value)
              if (provider.available)
                for (final model in provider.models)
                  (
                    BridgeModelChoice(
                      instance: provider.instance,
                      model: model.id,
                      optionIds: const [],
                      values: const [],
                    ),
                    '${provider.name} · ${model.name}',
                  ),
          ],
          onSelect: (model) => unawaited(agents.setModel(model)),
        ),
        _Choice<BridgePermissions>(
          label: 'Permissions',
          value: permissionNames[agent.permissions] ?? '',
          options: [
            for (final MapEntry(:key, :value) in permissionNames.entries)
              (key, value),
          ],
          onSelect: (permissions) =>
              unawaited(agents.setPermissions(permissions)),
        ),
        CardRow(
          onTap: () => unawaited(showEditInstructions(context, agents)),
          children: [
            const Expanded(
              child: Text(
                'Instructions',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            Text('Edit', style: link),
          ],
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(10, 0, 10, WsMetrics.space2),
          child: Text(
            _summary,
            key: const Key('agent-instructions'),
            maxLines: 3,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsSmall),
          ),
        ),
        CardRow(
          onTap: local
              ? () => unawaited(Process.run('open', [agent.memory]))
              : null,
          children: [
            const Expanded(
              child: Text(
                'Memory',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (local) Text('Show in Finder', style: link),
          ],
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(10, 0, 10, WsMetrics.space2),
          child: Text(
            agent.memory,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: monoStyle(context, color: ws.textFaint),
          ),
        ),
        if (earlier.isNotEmpty)
          _Earlier(
            state: widget.state,
            hosted: widget.hosted,
            earlier: earlier,
          ),
      ],
    );
  }
}

class _Choice<T> extends StatelessWidget {
  const _Choice({
    required this.label,
    required this.value,
    required this.options,
    required this.onSelect,
  });

  final String label;
  final String value;
  final List<(T, String)> options;
  final ValueChanged<T> onSelect;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return MenuAnchor(
      menuChildren: [
        for (final (option, name) in options)
          MenuItemButton(onPressed: () => onSelect(option), child: Text(name)),
      ],
      builder: (context, controller, _) => CardRow(
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        children: [
          Flexible(
            child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          Expanded(
            child: Text(
              '$value ⌄',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.end,
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Earlier extends StatelessWidget {
  const _Earlier({
    required this.state,
    required this.hosted,
    required this.earlier,
  });

  final SessionState state;
  final HostAgent hosted;
  final List<String> earlier;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    String title(String id) =>
        state.recent.value
            .where((listed) => listed.thread.id == id)
            .firstOrNull
            ?.thread
            .title ??
        id;
    return MenuAnchor(
      menuChildren: [
        for (final id in earlier)
          MenuItemButton(
            onPressed: () => unawaited(state.open(id, address: hosted.address)),
            child: Text(title(id)),
          ),
      ],
      builder: (context, controller, _) => CardRow(
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        children: [
          const Expanded(
            child: Text(
              'Earlier threads',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          Text(
            '${earlier.length} ›',
            style: TextStyle(color: ws.accent, fontSize: WsMetrics.fsSmall),
          ),
        ],
      ),
    );
  }
}
