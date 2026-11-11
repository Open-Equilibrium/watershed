import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// Label–value pairs in two aligned columns.
class InspectorPairs extends StatelessWidget {
  const InspectorPairs(this.pairs, {super.key});

  final List<(String, String)> pairs;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      children: [
        for (final (label, value) in pairs)
          Padding(
            padding: const EdgeInsets.only(bottom: WsMetrics.space1),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  label,
                  maxLines: 1,
                  style: TextStyle(
                    color: ws.textMuted,
                    fontSize: WsMetrics.fsCompact,
                  ),
                ),
                const SizedBox(width: WsMetrics.space4),
                Expanded(
                  child: Text(
                    value,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.right,
                    style: const TextStyle(
                      fontSize: WsMetrics.fsCompact,
                      fontFeatures: [FontFeature.tabularFigures()],
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

/// The heading that opens a group of rows in an inspector tab.
class InspectorHeading extends StatelessWidget {
  const InspectorHeading(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: WsMetrics.space3),
    child: Text(
      text,
      style: TextStyle(
        color: context.ws.textFaint,
        fontSize: WsMetrics.fsSmall,
        fontWeight: WsMetrics.fwMedium,
      ),
    ),
  );
}

/// The token split of one request: system prompt, tools and messages, estimated from their
/// characters' share of the measured input.
List<int> tokenSplit(BridgeRecord record) {
  final usage = record.usage;
  final saw = record.context;
  if (usage == null || saw == null) return const [0, 0, 0];
  return shares(promptTokens(usage), [
    saw.promptChars,
    saw.toolChars,
    saw.messageChars,
  ]);
}
