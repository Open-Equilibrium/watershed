import 'package:flutter/material.dart';

import '../session/task_tree.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

final _mentionQuery = RegExp(r'^@(\S*)$');

List<RunningInstance> mentionChoices(
  TextEditingValue value,
  List<RunningInstance> running,
) {
  final query = _mentionQuery.firstMatch(value.text)?.group(1);
  if (query == null || value.selection.baseOffset != value.text.length) {
    return const [];
  }
  return [
    for (final instance in running)
      if (instance.name.toLowerCase().contains(query.toLowerCase())) instance,
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

  final List<RunningInstance> choices;
  final int chosen;
  final ValueChanged<RunningInstance> onChoose;
  final ValueChanged<int> onHover;

  static const _labelHeight = 26.0;
  static const _rowHeight = 32.0;

  static double heightFor(int count) =>
      2 * WsMetrics.space2 + _labelHeight + count * _rowHeight;

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
            Container(
              height: _labelHeight,
              alignment: Alignment.bottomLeft,
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space3,
                WsMetrics.space2,
                WsMetrics.space3,
                2,
              ),
              child: Text(
                'Running instances',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
            ),
            for (final (index, choice) in choices.indexed)
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
                        Icon(LucideIcons.loader, size: 14, color: ws.accent),
                        const SizedBox(width: WsMetrics.space3),
                        Expanded(
                          child: Text(choice.name, style: monoStyle(context)),
                        ),
                        Text(
                          choice.phase,
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
        ),
      ),
    );
  }
}
