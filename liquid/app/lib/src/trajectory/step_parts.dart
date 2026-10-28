import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// A step's kind as a colored tag: User, Message, Tool or Approval.
class KindTag extends StatelessWidget {
  const KindTag(this.kind, {super.key});

  final BridgeRecordKind kind;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (label, color, background) = switch (kind) {
      BridgeRecordKind.user => ('User', ws.textMuted, ws.hover),
      BridgeRecordKind.message => ('Message', ws.accent, ws.accentSubtle),
      BridgeRecordKind.tool => (
        'Tool',
        ws.data2,
        ws.data2.withValues(alpha: 0.14),
      ),
      BridgeRecordKind.approval => ('Approval', ws.warning, ws.warningSubtle),
    };
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        label,
        style: TextStyle(
          color: color,
          fontSize: WsMetrics.fsSmall,
          height: 20 / WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwMedium,
        ),
      ),
    );
  }
}

/// An approval decision in words.
String decisionLabel(BridgeDecision? decision) => switch (decision) {
  BridgeDecision.allowOnce => 'allowed once',
  BridgeDecision.allowSession => 'allowed for session',
  BridgeDecision.reject => 'rejected',
  null => 'decided',
};

/// What a step did, as the ledger's Step column shows it.
String stepText(BridgeRecord record) => switch (record.kind) {
  BridgeRecordKind.user || BridgeRecordKind.message => record.label,
  BridgeRecordKind.tool => toolSummary(record),
  BridgeRecordKind.approval =>
    '${record.label} · ${decisionLabel(record.decision)}',
};

/// The Step column's text; tool calls and approvals are code.
class StepText extends StatelessWidget {
  const StepText(this.record, {super.key});

  final BridgeRecord record;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final code =
        record.kind == BridgeRecordKind.tool ||
        record.kind == BridgeRecordKind.approval;
    return Text.rich(
      TextSpan(
        children: [
          TextSpan(
            text: stepText(record),
            style: code ? monoStyle(context) : null,
          ),
          if (record.failed)
            TextSpan(
              text: ' · failed',
              style: TextStyle(color: ws.danger),
            ),
        ],
      ),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
    );
  }
}

/// `parts` as spans in their tones' colors.
List<TextSpan> toneSpans(WsPalette ws, List<(String, Tone)> parts) => [
  for (final (text, tone) in parts)
    TextSpan(
      text: text,
      style: switch (tone) {
        Tone.good => TextStyle(color: ws.success),
        Tone.bad => TextStyle(color: ws.danger),
        Tone.muted => TextStyle(color: ws.textMuted),
        Tone.faint => TextStyle(color: ws.textFaint),
        Tone.plain => null,
      },
    ),
];

/// A tool call's outcome in color, such as `+9 −2` or `exit 0 · 14 ✓`; empty without one.
class OutcomeText extends StatelessWidget {
  const OutcomeText(this.outcome, {super.key});

  final BridgeToolOutcome outcome;

  @override
  Widget build(BuildContext context) {
    final parts = outcomeParts(outcome);
    if (parts.isEmpty) return const SizedBox.shrink();
    return Text.rich(
      TextSpan(children: toneSpans(context.ws, parts)),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: const TextStyle(
        fontSize: WsMetrics.fsCompact,
        fontFeatures: [FontFeature.tabularFigures()],
      ),
    );
  }
}
