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

/// A tool call's outcome in color, such as `+9 −2` or `exit 0 · 14 ✓`; empty without one.
class OutcomeText extends StatelessWidget {
  const OutcomeText(this.outcome, {super.key});

  final BridgeToolOutcome outcome;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final groups = <List<TextSpan>>[
      if (outcome.added != null || outcome.removed != null)
        [
          if (outcome.added case final added?)
            TextSpan(
              text: '+$added',
              style: TextStyle(color: ws.success),
            ),
          if (outcome.added != null && outcome.removed != null)
            const TextSpan(text: ' '),
          if (outcome.removed case final removed?)
            TextSpan(
              text: '−$removed',
              style: TextStyle(color: ws.danger),
            ),
        ],
      if (outcome.exitCode case final code?)
        [
          TextSpan(
            text: 'exit $code',
            style: TextStyle(color: code == 0 ? ws.textMuted : ws.danger),
          ),
        ],
      if (outcome.testsPassed case final passed?)
        [
          TextSpan(
            text: '$passed ✓',
            style: TextStyle(color: ws.success),
          ),
          if ((outcome.testsFailed ?? 0) > 0)
            TextSpan(
              text: ' ${outcome.testsFailed} ✗',
              style: TextStyle(color: ws.danger),
            ),
        ],
    ];
    if (groups.isEmpty) return const SizedBox.shrink();
    return Text.rich(
      TextSpan(
        children: [
          for (final (index, group) in groups.indexed) ...[
            if (index > 0)
              TextSpan(
                text: ' · ',
                style: TextStyle(color: ws.textFaint),
              ),
            ...group,
          ],
        ],
      ),
      maxLines: 1,
      style: const TextStyle(
        fontSize: WsMetrics.fsCompact,
        fontFeatures: [FontFeature.tabularFigures()],
      ),
    );
  }
}

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;
