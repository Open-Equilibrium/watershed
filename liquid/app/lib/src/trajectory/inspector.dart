import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/history.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'context_track.dart';
import 'inspector_context.dart';
import 'inspector_parts.dart';
import 'inspector_summary.dart';
import 'ledger.dart';

/// Height of the inspector below a selected step; fixed so opening it never shifts more than
/// one known amount.
const inspectorHeight = 232.0;

/// The parts of a step the inspector shows.
enum InspectorTab { summary, context, usage, raw }

/// The selected step's details below its row: a summary, the context and usage of the request
/// that read it, or the raw entry.
class Inspector extends StatelessWidget {
  const Inspector({
    super.key,
    required this.records,
    required this.index,
    required this.detail,
    this.unread,
    required this.windowOf,
    required this.tab,
    required this.canBranch,
    required this.onTab,
    required this.onBranch,
  });

  final List<BridgeRecord> records;
  final int index;

  /// What the model saw for the request that read the step.
  final BridgeContextDetail? detail;

  /// The context window of a model, when known.
  final int? Function(String model) windowOf;

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
    final request = requestOf(records, index);
    return SizedBox(
      key: const Key('inspector'),
      height: inspectorHeight,
      child: Container(
        margin: const EdgeInsets.fromLTRB(ledgerInset, 8, 16, 10),
        clipBehavior: Clip.antiAlias,
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          border: Border.all(color: ws.border),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _tabBar(context),
            Expanded(
              child: switch (tab) {
                InspectorTab.raw => _Raw(record: record),
                InspectorTab.context => ContextTab(
                  records: records,
                  index: index,
                  request: request,
                  detail: detail,
                  unread: unread,
                  windowOf: windowOf,
                ),
                InspectorTab.usage => _UsageTab(
                  records: records,
                  request: request,
                ),
                InspectorTab.summary when message => MessageSummary(
                  records: records,
                  index: index,
                  detail: detail,
                ),
                InspectorTab.summary => StepSummary(record: record),
              },
            ),
          ],
        ),
      ),
    );
  }

  /// The tab strip, with the branch action at its end.
  Widget _tabBar(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 38,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (label, value) in [
            ('Summary', InspectorTab.summary),
            ('Context', InspectorTab.context),
            ('Usage', InspectorTab.usage),
            ('Raw', InspectorTab.raw),
          ]) ...[
            // Focusable, so the tabs work from the keyboard too; which one is
            // open is told to assistive technology, not only drawn.
            Semantics(
              selected: value == tab,
              inMutuallyExclusiveGroup: true,
              child: Shortcuts(
                shortcuts: activateKeys,
                child: InkWell(
                  onTap: () => onTab(value),
                  // The open tab takes an accent underline on the header's edge.
                  child: Container(
                    alignment: Alignment.center,
                    decoration: value == tab
                        ? BoxDecoration(
                            border: Border(
                              bottom: BorderSide(color: ws.accent, width: 2),
                            ),
                          )
                        : null,
                    child: Text(
                      label,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: value == tab ? ws.text : ws.textMuted,
                        fontWeight: value == tab
                            ? WsMetrics.fwSemibold
                            : WsMetrics.fwRegular,
                      ),
                    ),
                  ),
                ),
              ),
            ),
            const SizedBox(width: WsMetrics.space5),
          ],
          Expanded(
            child: canBranch
                ? Align(
                    alignment: Alignment.centerRight,
                    child: PrimaryButton(
                      label: 'Branch from here',
                      shortcut: Keymap.of(context).label(Command.branch),
                      onPressed: onBranch,
                    ),
                  )
                : const SizedBox.shrink(),
          ),
        ],
      ),
    );
  }
}

/// What the request that read a step used and cost.
class _UsageTab extends StatelessWidget {
  const _UsageTab({required this.records, required this.request});

  final List<BridgeRecord> records;
  final int? request;

  @override
  Widget build(BuildContext context) {
    final request = this.request;
    final read = request == null ? null : records[request];
    final usage = read?.usage;
    if (read == null || usage == null) {
      return Center(
        child: Text(
          read == null
              ? 'No request read this step yet.'
              : 'This request reported no tokens.',
          style: TextStyle(
            color: context.ws.textFaint,
            fontSize: WsMetrics.fsSmall,
          ),
        ),
      );
    }
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InspectorHeading(
            [
              'Request #${requestNumber(records, request!)}',
              if (read.model.isNotEmpty) read.model,
            ].join(' · '),
          ),
          InspectorPairs([
            ('In', formatCount(promptTokens(usage))),
            ('Read from cache', formatCount(usage.cacheRead)),
            ('Written to cache', formatCount(usage.cacheWrite)),
            ('Out', formatCount(usage.output)),
            ('Thinking', formatCount(usage.reasoning)),
            ('Cost', '≈ ${formatCost(usage.costMicroUsd)}'),
          ]),
        ],
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
