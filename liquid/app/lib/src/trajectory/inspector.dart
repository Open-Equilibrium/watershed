import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/history.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../session/session_state.dart';
import 'step_parts.dart';
import 'trajectory_model.dart';

/// Height of the inspector below a selected step; fixed so opening it never shifts more than
/// one known amount.
const inspectorHeight = 232.0;

/// The parts of a step the inspector shows.
enum InspectorTab { summary, context, raw }

/// The selected step's details below its row: a summary, what the model saw, or the raw entry.
class Inspector extends StatelessWidget {
  const Inspector({
    super.key,
    required this.records,
    required this.index,
    required this.detail,
    this.unread,
    required this.tab,
    required this.canBranch,
    required this.onTab,
    required this.onBranch,
  });

  final List<BridgeRecord> records;
  final int index;
  final BridgeContextDetail? detail;

  /// Why [detail] could not be read.
  final String? unread;
  final InspectorTab tab;
  final bool canBranch;
  final ValueChanged<InspectorTab> onTab;
  final VoidCallback onBranch;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final record = records[index];
    final message = record.kind == BridgeRecordKind.message;
    final shown = !message && tab == InspectorTab.context
        ? InspectorTab.summary
        : tab;
    return SizedBox(
      height: inspectorHeight,
      child: Container(
        margin: const EdgeInsets.fromLTRB(52, 8, 16, 10),
        clipBehavior: Clip.antiAlias,
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          border: Border.all(color: ws.border),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 38,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: Row(
                children: [
                  for (final (label, value) in [
                    ('Summary', InspectorTab.summary),
                    if (message) ('Context', InspectorTab.context),
                    ('Raw', InspectorTab.raw),
                  ]) ...[
                    GestureDetector(
                      onTap: () => onTab(value),
                      child: Text(
                        label,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: value == shown ? ws.text : ws.textMuted,
                          fontWeight: value == shown
                              ? WsMetrics.fwMedium
                              : WsMetrics.fwRegular,
                        ),
                      ),
                    ),
                    const SizedBox(width: WsMetrics.space5),
                  ],
                  const Spacer(),
                  if (canBranch)
                    PrimaryButton(
                      label: 'Branch from here',
                      shortcut: Keymap.of(context).label(Command.branch),
                      onPressed: onBranch,
                    ),
                ],
              ),
            ),
            Expanded(
              child: switch (shown) {
                InspectorTab.raw => _Raw(record: record),
                InspectorTab.context => _ContextTab(
                  record: record,
                  detail: detail,
                  unread: unread,
                ),
                InspectorTab.summary when message => _MessageSummary(
                  records: records,
                  index: index,
                  detail: detail,
                ),
                InspectorTab.summary => _StepSummary(record: record),
              },
            ),
          ],
        ),
      ),
    );
  }
}

/// Label–value pairs in two aligned columns.
class _Pairs extends StatelessWidget {
  const _Pairs(this.pairs);

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

class _Heading extends StatelessWidget {
  const _Heading(this.text);

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
List<int> _tokenSplit(BridgeRecord record) {
  final usage = record.usage;
  final saw = record.context;
  if (usage == null || saw == null) return const [0, 0, 0];
  return shares(promptTokens(usage), [
    saw.promptChars,
    saw.toolChars,
    saw.messageChars,
  ]);
}

/// Tokens of `chars` characters at this request's ratio of tokens to characters.
int _tokensFor(BridgeRecord record, int chars) {
  final usage = record.usage;
  final saw = record.context;
  if (usage == null || saw == null) return 0;
  final total = saw.promptChars + saw.toolChars + saw.messageChars;
  return total == 0 ? 0 : (promptTokens(usage) * chars / total).round();
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
                    color: [ws.data1, ws.data2, ws.data3][index],
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

class _MessageSummary extends StatelessWidget {
  const _MessageSummary({
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
    final record = records[index];
    final usage = record.usage;
    final duration = record.durationMs;
    final firstToken = record.firstTokenMs;
    final generation = duration == null ? null : duration - (firstToken ?? 0);
    final saw = record.context;
    final detail = this.detail;
    final split = _tokenSplit(record);
    final previous = records
        .take(index)
        .toList()
        .lastIndexWhere((step) => step.kind == BridgeRecordKind.message);
    final sections = detail?.sections.map((part) => part.name).toList();
    final freshChars = detail?.messages
        .where((message) => message.fresh && message.inContext)
        .fold(0, (sum, message) => sum + message.chars);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          flex: 100,
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const _Heading('Timing and usage'),
                _Pairs([
                  if (firstToken != null)
                    ('First token', formatDuration(firstToken)),
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
          ),
        ),
        Container(width: 1, color: ws.border),
        Expanded(
          flex: 115,
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _Heading(
                  usage == null
                      ? 'What the model saw'
                      : 'What the model saw · '
                            '${formatCount(promptTokens(usage))} tokens',
                ),
                if (saw != null) ...[
                  _SplitBar(split),
                  const SizedBox(height: 10),
                  _ShareRow(
                    ws.data1,
                    sections == null || sections.isEmpty
                        ? 'System prompt'
                        : 'System prompt · ${_names(sections)}',
                    split[0],
                  ),
                  _ShareRow(ws.data2, 'Tools · ${saw.tools.length}', split[1]),
                  _ShareRow(ws.data3, 'Messages · ${saw.messages}', split[2]),
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
                            '≈ ${formatCount(_tokensFor(record, freshChars))} tokens',
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
          ),
        ),
      ],
    );
  }

  /// Up to three names and how many more.
  static String _names(List<String> names) => names.length <= 3
      ? names.join(', ')
      : '${names.take(3).join(', ')}, ${names.length - 3} more';
}

/// Every part of what the model saw for one request (approved session-flow mockup).
class _ContextTab extends StatelessWidget {
  const _ContextTab({required this.record, required this.detail, this.unread});

  final BridgeRecord record;
  final BridgeContextDetail? detail;
  final String? unread;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final detail = this.detail;
    final saw = record.context;
    if (detail == null || saw == null) {
      return Center(
        child: Text(switch (unread) {
          final reason? => 'Could not read what the model saw: $reason',
          null => 'Reading what the model saw…',
        }, style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall)),
      );
    }
    final split = _tokenSplit(record);
    int total(Iterable<int> chars) =>
        chars.fold(0, (sum, value) => sum + value);
    Widget group(String title, int tokens) => Padding(
      padding: const EdgeInsets.only(top: WsMetrics.space2),
      child: Row(
        children: [
          Expanded(
            child: Text(
              title,
              style: const TextStyle(
                fontSize: WsMetrics.fsSmall,
                fontWeight: WsMetrics.fwSemibold,
              ),
            ),
          ),
          Text(
            '≈ ${formatCount(tokens)}',
            style: const TextStyle(
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwSemibold,
              fontFeatures: [FontFeature.tabularFigures()],
            ),
          ),
        ],
      ),
    );
    Widget part(
      String label,
      int chars,
      int groupChars,
      Color color, {
      bool fresh = false,
      bool gone = false,
    }) => SizedBox(
      height: 18,
      child: Row(
        children: [
          Expanded(
            child: Row(
              children: [
                Flexible(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: gone ? ws.textFaint : ws.textMuted,
                      fontSize: WsMetrics.fsSmall,
                      decoration: gone ? TextDecoration.lineThrough : null,
                    ),
                  ),
                ),
                if (fresh) ...[
                  const SizedBox(width: WsMetrics.space1),
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 4),
                    decoration: BoxDecoration(
                      color: ws.accentSubtle,
                      borderRadius: BorderRadius.circular(3),
                    ),
                    child: Text(
                      'new',
                      style: TextStyle(
                        color: ws.accent,
                        fontSize: WsMetrics.fsCaption,
                      ),
                    ),
                  ),
                ],
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space4),
          Container(
            width: 120,
            height: 5,
            decoration: BoxDecoration(
              color: ws.hover,
              borderRadius: BorderRadius.circular(3),
            ),
            alignment: Alignment.centerLeft,
            child: FractionallySizedBox(
              widthFactor: groupChars == 0 || gone ? 0 : chars / groupChars,
              child: Container(
                decoration: BoxDecoration(
                  color: color,
                  borderRadius: BorderRadius.circular(3),
                ),
              ),
            ),
          ),
          SizedBox(
            width: 64,
            child: Text(
              gone ? '–' : '≈ ${formatCount(_tokensFor(record, chars))}',
              textAlign: TextAlign.right,
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsSmall,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ),
        ],
      ),
    );
    final sectionChars = total(detail.sections.map((part) => part.chars));
    final toolChars = total(detail.tools.map((part) => part.chars));
    final messageChars = total([
      for (final message in detail.messages)
        if (message.inContext) message.chars,
    ]);
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _SplitBar(split),
          group('System prompt', split[0]),
          for (final section in detail.sections)
            part(section.name, section.chars, sectionChars, ws.data1),
          group('Tools · ${detail.tools.length}', split[1]),
          for (final tool in detail.tools)
            part(tool.name, tool.chars, toolChars, ws.data2),
          group('Messages · ${saw.messages}', split[2]),
          for (final (index, message) in detail.messages.indexed)
            part(
              '${index + 1} · ${message.label}',
              message.chars,
              messageChars,
              ws.data3,
              fresh: message.fresh,
              gone: !message.inContext,
            ),
          const SizedBox(height: WsMetrics.space3),
          Text(
            'Shares estimated from characters; the total is the provider\'s measured input. '
            'Compacted or removed entries appear crossed out.',
            style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
          ),
        ],
      ),
    );
  }
}

class _StepSummary extends StatelessWidget {
  const _StepSummary({required this.record});

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
            const _Heading('Prompt'),
            Text(record.label, maxLines: 6, overflow: TextOverflow.ellipsis),
          ],
          BridgeRecordKind.approval => [
            const _Heading('Approval'),
            _Pairs([
              ('Tool', record.label),
              ('Decision', decisionLabel(record.decision)),
              if (duration != null)
                ('Waited for you', formatDuration(duration)),
            ]),
          ],
          _ => [
            const _Heading('Tool call'),
            _Pairs([
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

class _Raw extends StatelessWidget {
  const _Raw({required this.record});

  final BridgeRecord record;

  @override
  Widget build(BuildContext context) {
    final usage = record.usage;
    final lines = [
      'entry ${record.entry}',
      'turn ${record.turn} · ${record.kind.name}',
      'start ${record.startMs} ms · duration ${record.durationMs ?? '–'} ms'
          '${record.firstTokenMs == null ? '' : ' · first token ${record.firstTokenMs} ms'}',
      if (record.model.isNotEmpty)
        'model ${record.model} · stop ${record.stopReason}',
      if (usage != null)
        'input ${usage.input} · output ${usage.output} · reasoning ${usage.reasoning} · '
            'cache read ${usage.cacheRead} · cache write ${usage.cacheWrite} · '
            'cost ${usage.costMicroUsd} µ\$',
      if (record.detail.isNotEmpty) 'detail ${record.detail}',
    ];
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: SelectableText(lines.join('\n'), style: monoStyle(context)),
    );
  }
}
