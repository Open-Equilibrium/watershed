import 'package:flutter/material.dart';

import '../session/task_tree.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'agents_section.dart';

final _mentionQuery = RegExp(r'^@(\S*)$');

typedef MentionChoice = ({String name, String detail, bool agent});

List<MentionChoice> mentionChoices(
  TextEditingValue value,
  List<RunningInstance> running, {
  List<MentionChoice> agents = const [],
}) {
  final query = _mentionQuery.firstMatch(value.text)?.group(1);
  if (query == null || value.selection.baseOffset != value.text.length) {
    return const [];
  }
  final wanted = query.toLowerCase();
  return [
    for (final agent in agents)
      if (agent.name.toLowerCase().contains(wanted)) agent,
    for (final instance in running)
      if (instance.name.toLowerCase().contains(wanted))
        (name: instance.name, detail: instance.phase, agent: false),
  ];
}

class MentionMenu extends StatelessWidget {
  const MentionMenu({
    super.key,
    required this.choices,
    required this.chosen,
    required this.onChoose,
    required this.onHover,
  });

  final List<MentionChoice> choices;
  final int chosen;
  final ValueChanged<MentionChoice> onChoose;
  final ValueChanged<int> onHover;

  static const _labelHeight = 26.0;
  static const _rowHeight = 32.0;

  static double heightFor(List<MentionChoice> choices) =>
      2 * WsMetrics.space2 +
      {for (final choice in choices) choice.agent}.length * _labelHeight +
      choices.length * _rowHeight;

  Widget _label(BuildContext context, String text) => Container(
    height: _labelHeight,
    alignment: Alignment.bottomLeft,
    padding: const EdgeInsets.fromLTRB(
      WsMetrics.space3,
      WsMetrics.space2,
      WsMetrics.space3,
      2,
    ),
    child: Text(
      text,
      style: TextStyle(
        fontSize: WsMetrics.fsSmall,
        color: context.ws.textFaint,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      width: 340,
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (index, choice) in choices.indexed) ...[
              if (index == 0 || choices[index - 1].agent != choice.agent)
                _label(context, choice.agent ? 'Agents' : 'Running instances'),
              MouseRegion(
                onEnter: (_) => onHover(index),
                child: GestureDetector(
                  onTap: () => onChoose(choice),
                  child: Container(
                    height: _rowHeight,
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space3,
                    ),
                    decoration: BoxDecoration(
                      color: index == chosen ? ws.hover : null,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      children: [
                        if (choice.agent)
                          AgentInitial(choice.name[0].toUpperCase())
                        else
                          Icon(LucideIcons.loader, size: 14, color: ws.accent),
                        const SizedBox(width: WsMetrics.space3),
                        Expanded(
                          child: Text(
                            choice.name,
                            style: choice.agent ? null : monoStyle(context),
                          ),
                        ),
                        Text(
                          choice.detail,
                          style: TextStyle(
                            fontSize: WsMetrics.fsSmall,
                            color: ws.textFaint,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
