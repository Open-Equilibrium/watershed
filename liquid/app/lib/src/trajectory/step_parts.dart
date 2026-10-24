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
String decisionLabel(String decision) => switch (decision) {
  'allow-once' => 'allowed once',
  'allow-session' => 'allowed for session',
  'reject' => 'rejected',
  _ => decision,
};

/// What a step did, as the ledger's Step column shows it.
String stepText(BridgeRecord record) => switch (record.kind) {
  BridgeRecordKind.user || BridgeRecordKind.message => record.label,
  BridgeRecordKind.tool => toolSummary(record.label, record.detail),
  BridgeRecordKind.approval =>
    '${record.label} · ${decisionLabel(record.detail)}',
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

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;
