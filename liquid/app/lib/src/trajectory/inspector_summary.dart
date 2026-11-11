import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'context_track.dart';
import 'inspector_parts.dart';

/// A message step's summary: its timing and usage, and what the model saw for the request
/// that read it.
class MessageSummary extends StatelessWidget {
  const MessageSummary({
    super.key,
    required this.records,
    required this.index,
    required this.detail,
  });

  final List<BridgeRecord> records;
  final int index;
  final BridgeContextDetail? detail;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(flex: 100, child: _timing()),
        Container(width: 1, color: ws.border),
        Expanded(flex: 115, child: _seen(ws)),
      ],
    );
  }

  Widget _timing() {
    final record = records[index];
    final usage = record.usage;
    final duration = record.durationMs;
    final firstToken = record.firstTokenMs;
    final generation = duration == null ? null : duration - (firstToken ?? 0);
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const InspectorHeading('Timing and usage'),
          InspectorPairs([
            if (firstToken != null) ('First token', formatDuration(firstToken)),
            if (generation != null)
              (
                'Generation',
                [
                  formatDuration(generation),
                  if (usage != null && generation > 0)
                    '${(usage.output / (generation / 1000)).round()} tok/s',
                ].join(' · '),
              ),
            if (usage != null) ...[
              (
                'Tokens',
                [
                  '${formatCount(promptTokens(usage))} in',
                  '${formatCount(usage.output)} out',
                  '${formatCount(usage.reasoning)} think',
                  if (usage.cacheRead > 0)
                    '${formatCount(usage.cacheRead)} cached',
                ].join(' · '),
              ),
              ('Cost', '≈ ${formatCost(usage.costMicroUsd)}'),
            ],
            if (record.model.isNotEmpty) ('Model', record.model),
          ]),
        ],
      ),
    );
  }

  Widget _seen(WsPalette ws) {
    final record = records[index];
    final usage = record.usage;
    final saw = record.context;
    final split = tokenSplit(record);
    final previous = records
        .take(index)
        .toList()
        .lastIndexWhere((step) => step.kind == BridgeRecordKind.message);
    final sections = detail?.sections.map((part) => part.name).toList();
    final freshChars = detail?.messages
        .where((message) => message.fresh && message.inContext)
        .fold(0, (sum, message) => sum + message.chars);
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InspectorHeading(
            usage == null
                ? 'What the model saw'
                : 'What the model saw · '
                      '${formatCount(promptTokens(usage))} tokens',
          ),
          if (saw != null) ...[
            _SplitBar(split),
            const SizedBox(height: 10),
            _ShareRow(
              ws.data2,
              sections == null || sections.isEmpty
                  ? 'System prompt'
                  : 'System prompt · ${_names(sections)}',
              split[0],
            ),
            _ShareRow(ws.data3, 'Tools · ${saw.tools.length}', split[1]),
            _ShareRow(ws.data1, 'Messages · ${saw.messages}', split[2]),
            if (saw.newSincePrevious.isNotEmpty) ...[
              const SizedBox(height: 10),
              Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: WsMetrics.space2,
                ),
                decoration: BoxDecoration(
                  color: ws.accentSubtle,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Text(
                  [
                    '${previous < 0 ? 'New since the session started' : 'New since step ${previous + 1}'}: '
                        '${saw.newSincePrevious.join(', ')}',
                    if (freshChars != null && freshChars > 0)
                      '≈ ${formatCount(tokensOf(record, freshChars))} tokens',
                  ].join(' · '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: ws.accent,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
            ],
          ],
        ],
      ),
    );
  }

  /// Up to three names and how many more.
  static String _names(List<String> names) => names.length <= 3
      ? names.join(', ')
      : '${names.take(3).join(', ')}, ${names.length - 3} more';
}

/// The summary of any other step: a prompt, or a tool call with its result.
class StepSummary extends StatelessWidget {
  const StepSummary({super.key, required this.record});

  final BridgeRecord record;

  @override
  Widget build(BuildContext context) {
    final duration = record.durationMs;
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: switch (record.kind) {
          BridgeRecordKind.user => [
            const InspectorHeading('Prompt'),
            Text(record.label, maxLines: 6, overflow: TextOverflow.ellipsis),
          ],
          _ => [
            const InspectorHeading('Tool call'),
            InspectorPairs([
              ('Tool', record.label),
              ('Result', record.failed ? 'failed' : 'succeeded'),
              if (duration != null) ('Duration', formatDuration(duration)),
            ]),
            const SizedBox(height: WsMetrics.space3),
            Text(
              record.detail,
              maxLines: 4,
              overflow: TextOverflow.ellipsis,
              style: monoStyle(context),
            ),
          ],
        },
      ),
    );
  }
}

/// System prompt, tools and messages as one bar.
class _SplitBar extends StatelessWidget {
  const _SplitBar(this.split);

  final List<int> split;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return SizedBox(
      height: 6,
      child: Row(
        children: [
          for (final (index, part) in split.indexed)
            if (part > 0)
              Expanded(
                flex: part,
                child: Container(
                  margin: EdgeInsets.only(left: index == 0 ? 0 : 2),
                  decoration: BoxDecoration(
                    color: [ws.data2, ws.data3, ws.data1][index],
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
              ),
        ],
      ),
    );
  }
}

/// A colored square, the label and an estimated token count.
class _ShareRow extends StatelessWidget {
  const _ShareRow(this.color, this.label, this.tokens);

  final Color color;
  final String label;
  final int tokens;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space1),
      child: Row(
        children: [
          Container(
            width: 8,
            height: 8,
            margin: const EdgeInsets.only(right: WsMetrics.space2),
            decoration: BoxDecoration(
              color: color,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
          Expanded(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsCompact,
              ),
            ),
          ),
          const SizedBox(width: WsMetrics.space4),
          Text(
            '≈ ${formatCount(tokens)}',
            style: const TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontFeatures: [FontFeature.tabularFigures()],
            ),
          ),
        ],
      ),
    );
  }
}
