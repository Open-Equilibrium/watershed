import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// Width of the reading column in the chat and the input area.
const double readingWidth = 740;

/// The chat timeline. It builds only visible items, and each item rebuilds alone while its text
/// streams (client architecture rules 5 and 6).
class ChatView extends StatelessWidget {
  const ChatView({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder(
      valueListenable: state.items,
      builder: (context, items, _) => ListView.builder(
        reverse: true,
        padding: const EdgeInsets.symmetric(vertical: WsMetrics.space5),
        itemCount: items.length,
        itemBuilder: (context, index) {
          final item = items[items.length - 1 - index];
          return Center(
            child: SizedBox(
              width: readingWidth,
              child: Padding(
                padding: const EdgeInsets.only(bottom: WsMetrics.space4),
                child: RepaintBoundary(
                  child: ItemTile(item: item, state: state),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// One timeline entry: a user bubble, model prose, collapsed reasoning or a tool row.
class ItemTile extends StatelessWidget {
  const ItemTile({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final prose = Theme.of(context).textTheme.bodyLarge;
    return ValueListenableBuilder(
      valueListenable: item.text,
      builder: (context, text, _) => switch (item.kind) {
        BridgeItemKind.userMessage => Align(
          alignment: Alignment.centerRight,
          child: Container(
            constraints: const BoxConstraints(maxWidth: readingWidth * 0.8),
            padding: const EdgeInsets.symmetric(
              horizontal: WsMetrics.space5,
              vertical: WsMetrics.space4,
            ),
            decoration: BoxDecoration(
              color: ws.hover,
              borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
            ),
            child: Text(text, style: prose),
          ),
        ),
        BridgeItemKind.assistantMessage => SelectableText(text, style: prose),
        BridgeItemKind.reasoning => Text(
          '▸ Thinking',
          style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
        ),
        _ => ToolRow(item: item, state: state),
      },
    );
  }
}

/// A tool call: its state, tool and target, and its outcome.
class ToolRow extends StatelessWidget {
  const ToolRow({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  String get _target {
    final arguments = item.arguments ?? '';
    final match = RegExp(r'"(?:command|path|pattern)"\s*:\s*"([^"]*)"')
        .firstMatch(arguments);
    return match?.group(1) ?? arguments;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: state.approval,
      builder: (context, approval, _) {
        final waiting =
            !item.completed &&
            approval != null &&
            approval.approval?.tool == item.tool;
        final (mark, markColor) = item.completed
            ? (item.failed ? ('✕', ws.danger) : ('✓', ws.success))
            : (waiting ? ('◷', ws.warning) : ('…', ws.textFaint));
        final trailing = waiting
            ? 'waiting for approval'
            : item.durationMs != null
            ? '${item.durationMs} ms'
            : '';
        return Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          decoration: BoxDecoration(
            color: waiting ? ws.warningSubtle : ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              Text(mark, style: TextStyle(color: markColor)),
              const SizedBox(width: WsMetrics.space3),
              Text(
                item.tool ?? '',
                style: TextStyle(
                  color: waiting ? ws.warning : ws.textMuted,
                  fontSize: 13,
                ),
              ),
              const SizedBox(width: WsMetrics.space3),
              Expanded(
                child: Text(
                  _target,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: monoStyle(context),
                ),
              ),
              Text(
                trailing,
                style: TextStyle(
                  color: waiting ? ws.warning : ws.textMuted,
                  fontSize: 12,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
