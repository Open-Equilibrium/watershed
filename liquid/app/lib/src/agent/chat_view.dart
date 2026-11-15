import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../diff/revert_dialog.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/thread.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'input_lane.dart';
import 'sidebar.dart';
import 'start_page.dart';

/// The chat timeline of the session. It builds only visible items, and each item rebuilds alone
/// while its text streams (client architecture rules 5 and 6).
class ChatView extends StatefulWidget {
  const ChatView({
    super.key,
    required this.state,
    required this.info,
    this.input,
    this.onOpenDiff,
  });

  final SessionState state;

  /// Shows the Diff panel with a turn's changes, by the turn's number.
  final ValueChanged<int>? onOpenDiff;

  /// The session's project, harness and device, as the start page greets with them.
  final SessionInfo info;

  /// The input area, floating at the bottom.
  final Widget? input;

  @override
  State<ChatView> createState() => _ChatViewState();
}

class _ChatViewState extends State<ChatView> {
  /// Jumps a scroll makes at most to build the item it brings into view.
  static const _revealJumps = 6;

  final _scroll = ScrollController();

  /// The widget of each item by its id, so a scroll finds the item it brings into view.
  final _itemKeys = <int, GlobalKey>{};

  @override
  void initState() {
    super.initState();
    widget.state.scrollChatTo.addListener(_scrollRequested);
    widget.state.items.addListener(_pruneItemKeys);
  }

  @override
  void didUpdateWidget(ChatView old) {
    super.didUpdateWidget(old);
    if (old.state != widget.state) {
      old.state.scrollChatTo.removeListener(_scrollRequested);
      old.state.items.removeListener(_pruneItemKeys);
      widget.state.scrollChatTo.addListener(_scrollRequested);
      widget.state.items.addListener(_pruneItemKeys);
    }
  }

  @override
  void dispose() {
    widget.state.scrollChatTo.removeListener(_scrollRequested);
    widget.state.items.removeListener(_pruneItemKeys);
    _scroll.dispose();
    super.dispose();
  }

  /// Forgets the keys of the items that a reload of the chat dropped.
  void _pruneItemKeys() {
    final ids = {for (final item in widget.state.items.value) item.id};
    _itemKeys.removeWhere((id, _) => !ids.contains(id));
  }

  void _scrollRequested() {
    final turn = widget.state.scrollChatTo.value;
    if (turn == null) return;
    widget.state.scrollChatTo.value = null;
    unawaited(_scrollTo(turn));
  }

  /// Brings the first item of `turn` into view. A list builds only the items near its view, so
  /// the scroll first jumps near the item, where it builds, and repeats the jump if the estimate
  /// missed.
  Future<void> _scrollTo(int turn) async {
    for (var jump = 0; jump < _revealJumps; jump++) {
      final index = widget.state.items.value.indexWhere(
        (item) => item.turn == turn,
      );
      if (index < 0 || !mounted) return;
      final built =
          _itemKeys[widget.state.items.value[index].id]?.currentContext;
      if (built != null && built.mounted) {
        await Scrollable.ensureVisible(built, alignment: 0.5);
        return;
      }
      _jumpNear(index, widget.state.items.value.length);
      await WidgetsBinding.instance.endOfFrame;
    }
  }

  /// Jumps so the item at `index` of `count` lies near the middle of the view. The list is
  /// reversed, so offset 0 is the newest item and the estimate counts the items newer than it,
  /// at the average height of the items the list has measured.
  void _jumpNear(int index, int count) {
    if (!_scroll.hasClients) return;
    final position = _scroll.position;
    final average =
        (position.maxScrollExtent + position.viewportDimension) / count;
    final newer = (count - 1 - index) * average;
    _scroll.jumpTo(
      (newer - position.viewportDimension / 2)
          .clamp(0.0, position.maxScrollExtent)
          .toDouble(),
    );
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    return InputLane(
      input: widget.input,
      child: ListenableBuilder(
        listenable: Listenable.merge([state.items, state.request]),
        // A session without a conversation shows the start page.
        builder: (context, _) {
          final items = state.items.value;
          if (items.isEmpty) return StartPage(state: state, info: widget.info);
          // A pending approval sits under the last item, where the agent waits for it.
          final request = state.request.value;
          final approval = request?.approval;
          final offset = approval == null ? 0 : 1;
          // Messages start at the top and flow down; a long chat still keeps its newest message
          // above the input.
          return Align(
            alignment: Alignment.topCenter,
            child: ListView.builder(
              controller: _scroll,
              shrinkWrap: true,
              reverse: true,
              padding: EdgeInsets.only(
                top: WsMetrics.space5,
                bottom:
                    WsMetrics.space5 +
                    (widget.input == null ? 0 : inputClearance),
              ),
              itemCount: items.length + offset,
              itemBuilder: (context, index) {
                if (index < offset) {
                  return Center(
                    child: SizedBox(
                      width: readingWidth,
                      child: Padding(
                        padding: const EdgeInsets.only(
                          bottom: WsMetrics.space4,
                        ),
                        child: ApprovalRow(
                          state: state,
                          request: request!,
                          approval: approval!,
                        ),
                      ),
                    ),
                  );
                }
                final item = items[items.length - 1 - (index - offset)];
                return Center(
                  key: _itemKeys.putIfAbsent(item.id, GlobalKey.new),
                  child: SizedBox(
                    width: readingWidth,
                    child: Padding(
                      padding: const EdgeInsets.only(bottom: WsMetrics.space4),
                      child: RepaintBoundary(
                        child: switch (item.turnEnd) {
                          final turn? => TurnChanges(
                            turn: turn,
                            onOpenDiff: widget.onOpenDiff == null
                                ? null
                                : () => widget.onOpenDiff!(turn.number),
                            onRevert: () => unawaited(
                              confirmRevert(context, state, turn.number),
                            ),
                          ),
                          null => ItemTile(item: item, state: state),
                        },
                      ),
                    ),
                  ),
                );
              },
            ),
          );
        },
      ),
    );
  }
}

/// What a turn changed, below its last item.
class TurnChanges extends StatelessWidget {
  const TurnChanges({
    super.key,
    required this.turn,
    this.onOpenDiff,
    this.onRevert,
  });

  final BridgeTurn turn;
  final VoidCallback? onOpenDiff;

  /// Asks to revert the files to before the turn.
  final VoidCallback? onRevert;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final files = turn.files;
    final style = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    return Row(
      children: [
        Text('$files changed file${files == 1 ? '' : 's'}', style: style),
        const SizedBox(width: WsMetrics.space4),
        Text('+${turn.added}', style: style.copyWith(color: ws.success)),
        const SizedBox(width: WsMetrics.space2),
        Text('−${turn.removed}', style: style.copyWith(color: ws.danger)),
        if (onOpenDiff case final open?) ...[
          const SizedBox(width: WsMetrics.space3),
          GhostButton(label: 'Open diff', onPressed: open),
        ],
        if (onRevert case final revert?) ...[
          const SizedBox(width: WsMetrics.space1),
          // A narrow window shortens its label rather than the counts.
          Flexible(
            child: GhostButton(
              label: 'Revert files to before this turn…',
              icon: LucideIcons.undo2,
              onPressed: revert,
            ),
          ),
        ],
      ],
    );
  }
}

/// T3's pending approval, in the Chat under the last item: what the agent asks to run, why, and
/// its answers. Decline and Approve show; the other options sit under More.
class ApprovalRow extends StatelessWidget {
  const ApprovalRow({
    super.key,
    required this.state,
    required this.request,
    required this.approval,
  });

  final SessionState state;
  final BridgeRequest request;
  final BridgeApproval approval;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final options = approval.options.isEmpty
        ? _defaultOptions
        : approval.options;
    final primary = [
      for (final option in options)
        if (option.decision == 'decline' || option.decision == 'accept') option,
    ];
    final more = [
      for (final option in options)
        if (option.decision != 'decline' && option.decision != 'accept') option,
    ];
    void answer(BridgeApprovalOption option) =>
        unawaited(state.respond(request, option.decision));
    final small = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    return Container(
      padding: const EdgeInsets.all(WsMetrics.space4),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: WsMetrics.space2,
        children: [
          Text(
            _approvalTitle(approval.kind),
            style: Theme.of(context).textTheme.labelMedium,
          ),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 160),
            child: SingleChildScrollView(
              child: SelectableText(approval.prompt, style: monoStyle(context)),
            ),
          ),
          if (approval.reason case final reason?) Text(reason, style: small),
          Row(
            spacing: WsMetrics.space3,
            children: [
              Icon(LucideIcons.shield, size: 12, color: ws.warning),
              Text(
                'Waiting for approval',
                style: TextStyle(
                  color: ws.warning,
                  fontSize: WsMetrics.fsCompact,
                ),
              ),
              const Spacer(),
              for (final option in primary) _answerButton(option, answer),
              if (more.isNotEmpty)
                PopupMenuButton<BridgeApprovalOption>(
                  tooltip: 'More approval options',
                  onSelected: answer,
                  itemBuilder: (context) => [
                    for (final option in more)
                      PopupMenuItem(value: option, child: Text(option.label)),
                  ],
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space3,
                    ),
                    child: Text('More', style: small),
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _answerButton(
    BridgeApprovalOption option,
    void Function(BridgeApprovalOption) answer,
  ) {
    final button = option.decision == 'accept'
        ? PrimaryButton(label: option.label, onPressed: () => answer(option))
        : SecondaryButton(label: option.label, onPressed: () => answer(option));
    final warning = option.warning;
    return warning == null ? button : Tooltip(message: warning, child: button);
  }
}

/// The options of an approval the agent sent without any, in T3's order.
const _defaultOptions = [
  BridgeApprovalOption(decision: 'cancel', label: 'Cancel'),
  BridgeApprovalOption(decision: 'decline', label: 'Decline'),
  BridgeApprovalOption(
    decision: 'acceptForSession',
    label: 'Always allow this session',
  ),
  BridgeApprovalOption(decision: 'accept', label: 'Approve'),
];

/// The title of an approval of T3's `kind`.
String _approvalTitle(String kind) => switch (kind) {
  'command' => 'Command approval',
  'file_change' || 'file-change' => 'File change approval',
  'file_read' || 'file-read' => 'File read approval',
  'permission' => 'App permission approval',
  'mcp-elicitation' => 'App access approval',
  _ => 'Approval',
};

/// One timeline entry: a user message, model prose, a line for reasoning, an error or a tool
/// row. Messages show their actions while the pointer is over them, as T3 does.
class ItemTile extends StatelessWidget {
  const ItemTile({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final prose = Theme.of(context).textTheme.bodyLarge;
    return ListenableBuilder(
      listenable: Listenable.merge([item.text, item.outcome]),
      builder: (context, _) {
        final text = item.text.value;
        return switch (item.kind) {
          BridgeItemKind.userMessage => _HoverActions(
            end: true,
            message: Align(
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
            actions: [
              GhostButton(
                label: 'Edit from here',
                onPressed: () =>
                    unawaited(confirmRevert(context, state, item.turn)),
              ),
              GhostButton(
                label: 'Copy message',
                onPressed: () =>
                    unawaited(Clipboard.setData(ClipboardData(text: text))),
              ),
            ],
          ),
          BridgeItemKind.assistantMessage => _HoverActions(
            end: false,
            message: SelectableText(text, style: prose),
            actions: [
              GhostButton(
                label: 'Copy message',
                onPressed: () =>
                    unawaited(Clipboard.setData(ClipboardData(text: text))),
              ),
              if (item.outcome.value != null)
                GhostButton(
                  label: 'Fork from this response',
                  onPressed: () => unawaited(state.fork(item.turn)),
                ),
            ],
          ),
          BridgeItemKind.reasoning => Text(
            'Thinking',
            style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
          ),
          BridgeItemKind.error => _ErrorRow(text: text),
          _ => ToolRow(item: item),
        };
      },
    );
  }
}

/// A message with its actions, which show while the pointer is over the message.
class _HoverActions extends StatefulWidget {
  const _HoverActions({
    required this.end,
    required this.message,
    required this.actions,
  });

  /// Whether the message sits at the end of the row, as a user message does.
  final bool end;
  final Widget message;
  final List<Widget> actions;

  @override
  State<_HoverActions> createState() => _HoverActionsState();
}

class _HoverActionsState extends State<_HoverActions> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) => MouseRegion(
    onEnter: (_) => setState(() => _hovered = true),
    onExit: (_) => setState(() => _hovered = false),
    child: Column(
      crossAxisAlignment: widget.end
          ? CrossAxisAlignment.end
          : CrossAxisAlignment.start,
      children: [
        widget.message,
        // Kept in place when hidden, so the message does not move when they show.
        Opacity(
          opacity: _hovered ? 1 : 0,
          child: IgnorePointer(
            ignoring: !_hovered,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: widget.actions,
            ),
          ),
        ),
      ],
    ),
  );
}

/// An error the agent or its provider reported; a JSON error payload shows its message.
class _ErrorRow extends StatelessWidget {
  const _ErrorRow({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: ws.dangerSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(LucideIcons.circleX, size: 14, color: ws.danger),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: SelectableText(
              _errorMessage(text),
              style: TextStyle(color: ws.danger, fontSize: WsMetrics.fsCompact),
            ),
          ),
        ],
      ),
    );
  }

  static String _errorMessage(String text) {
    try {
      if (jsonDecode(text) case {'error': {'message': final String message}}
          when message.trim().isNotEmpty) {
        return message;
      }
    } on FormatException {
      // Plain text, shown as it is.
    }
    return text;
  }
}

/// A tool call: its state, tool and target, and how long it ran.
class ToolRow extends StatelessWidget {
  const ToolRow({super.key, required this.item});

  final ItemView item;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: item.outcome,
      builder: (context, outcome, _) {
        final (mark, markColor) = switch (outcome) {
          (failed: true, durationMs: _) => (LucideIcons.x, ws.danger),
          (failed: false, durationMs: _) => (LucideIcons.check, ws.success),
          null => (LucideIcons.loader, ws.textFaint),
        };
        final duration = outcome?.durationMs;
        return Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          decoration: BoxDecoration(
            color: ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              Icon(mark, size: 14, color: markColor),
              const SizedBox(width: WsMetrics.space3),
              // One line, so a long tool name shortens with its target.
              Expanded(
                child: Text.rich(
                  TextSpan(
                    children: [
                      TextSpan(
                        text: item.tool ?? '',
                        style: TextStyle(
                          color: ws.textMuted,
                          fontSize: WsMetrics.fsCompact,
                        ),
                      ),
                      const WidgetSpan(
                        child: SizedBox(width: WsMetrics.space3),
                      ),
                      TextSpan(
                        text: item.target ?? '',
                        style: monoStyle(context),
                      ),
                    ],
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (duration != null)
                Text(
                  formatDuration(duration),
                  style: TextStyle(
                    color: ws.textMuted,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}
