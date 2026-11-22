import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../session/task_tree.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

class TaskListRow extends StatefulWidget {
  const TaskListRow({super.key, required this.item, required this.state});

  final ItemView item;
  final SessionState state;

  @override
  State<TaskListRow> createState() => _TaskListRowState();
}

class _TaskListRowState extends State<TaskListRow> {
  var _open = false;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: widget.item.steps,
      builder: (context, steps, _) => Container(
        decoration: BoxDecoration(
          color: ws.subtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        padding: EdgeInsets.only(bottom: _open ? WsMetrics.space2 : 0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(() => _open = !_open),
              child: SizedBox(
                height: 32,
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                  ),
                  child: Row(
                    children: [
                      Icon(LucideIcons.listTodo, size: 14, color: ws.textFaint),
                      const SizedBox(width: WsMetrics.space3),
                      Expanded(
                        child: Text(
                          taskSummary(steps),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: ws.textMuted,
                            fontSize: WsMetrics.fsCompact,
                          ),
                        ),
                      ),
                      Icon(
                        _open ? LucideIcons.chevronUp : LucideIcons.chevronDown,
                        size: 14,
                        color: ws.textFaint,
                      ),
                    ],
                  ),
                ),
              ),
            ),
            if (_open)
              for (final node in taskTree(steps)) ...[
                _NodeRow(node: node, depth: 0, state: widget.state),
                for (final child in node.children)
                  _NodeRow(node: child, depth: 1, state: widget.state),
              ],
          ],
        ),
      ),
    );
  }
}

class _NodeRow extends StatefulWidget {
  const _NodeRow({
    required this.node,
    required this.depth,
    required this.state,
  });

  final TaskNode node;
  final int depth;
  final SessionState state;

  @override
  State<_NodeRow> createState() => _NodeRowState();
}

class _NodeRowState extends State<_NodeRow> {
  var _hovered = false;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final node = widget.node;
    final (icon, color, word) = switch (node.status) {
      BridgeStepStatus.completed => (
        LucideIcons.check,
        ws.success,
        'Completed',
      ),
      BridgeStepStatus.running => (LucideIcons.loader, ws.accent, 'Running'),
      BridgeStepStatus.pending => (LucideIcons.circle, ws.textFaint, 'Pending'),
    };
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final messageable =
        node.instance && node.status == BridgeStepStatus.running;
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Container(
        height: 28,
        margin: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
        padding: EdgeInsets.only(
          left: WsMetrics.space3 + widget.depth * 22,
          right: WsMetrics.space3,
        ),
        decoration: BoxDecoration(
          color: _hovered && messageable ? ws.hover : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        ),
        child: Row(
          children: [
            Icon(icon, size: 14, color: color),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Text(
                node.label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: node.instance
                    ? monoStyle(context)
                    : TextStyle(color: ws.text, fontSize: WsMetrics.fsCompact),
              ),
            ),
            if (_hovered && messageable)
              GestureDetector(
                onTap: () => widget.state.mention.value = node.label,
                child: Text(
                  'Message',
                  style: TextStyle(
                    color: ws.accent,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
            const SizedBox(width: WsMetrics.space4),
            Text(word, style: small),
          ],
        ),
      ),
    );
  }
}
