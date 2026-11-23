import 'package:flutter/material.dart';

import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'input_lane.dart';

const agentStarters = [
  'Every weekday morning, summarize what changed in this project.',
  'Watch CI and fix failing checks.',
  'Triage new issues.',
  'Keep the docs current.',
];

class NewAgentPage extends StatelessWidget {
  const NewAgentPage({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      children: [
        const SizedBox(height: WsMetrics.space8),
        Expanded(
          child: Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              spacing: WsMetrics.space4,
              children: [
                Text(
                  'What should this agent do?',
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.titleLarge
                      ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                ),
                Text(
                  'Write its job. It names itself, saves its instructions and '
                  'sets its own routines; you change all of it later by '
                  'writing to it or in its card.',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: ws.textMuted),
                ),
                Wrap(
                  alignment: WrapAlignment.center,
                  spacing: WsMetrics.space2,
                  runSpacing: WsMetrics.space2,
                  children: [
                    for (final starter in agentStarters)
                      ActionChip(
                        label: Text(starter),
                        onPressed: () => state.draft.value = starter,
                      ),
                  ],
                ),
              ],
            ),
          ),
        ),
        const SizedBox(height: inputClearance),
      ],
    );
  }
}
