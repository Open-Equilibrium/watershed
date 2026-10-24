import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../agent/action_area.dart';
import '../rust/api/history.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// The color of a graph lane.
Color laneColor(WsPalette ws, int lane) =>
    [ws.data1, ws.data2, ws.data3][lane % 3];

/// The session's branches as a graph like GitHub's network view: one lane per branch and one
/// dot per step. Each row paints its own slice, so long trees stay a lazy list.
class SessionTreeView extends StatelessWidget {
  const SessionTreeView({
    super.key,
    required this.graph,
    required this.selected,
    required this.onRow,
    this.picked,
  });

  final SessionGraph graph;

  /// The entry of the highlighted row: the selected step, or the cursor while comparing.
  final String? selected;
  final ValueChanged<GraphRow> onRow;

  /// The tips selected for comparison, when comparing.
  final Set<String>? picked;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final lines = _linesOf[graph] ??= _lines(graph);
    final spacing = graph.lanes < 2
        ? 18.0
        : math.min(18.0, 122 / (graph.lanes - 1));
    final graphWidth = 28 + (graph.lanes - 1) * spacing;
    return Container(
      width: 300,
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        border: Border(left: BorderSide(color: ws.border)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: WsMetrics.rowNav,
            child: Row(
              children: [
                Text(
                  'Session tree',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                    fontWeight: WsMetrics.fwMedium,
                  ),
                ),
                const Spacer(),
                Hint('⌥↑↓'),
              ],
            ),
          ),
          const SizedBox(height: 2),
          Expanded(
            child: ListView.builder(
              itemExtent: WsMetrics.rowNav,
              itemCount: graph.rows.length,
              itemBuilder: (context, index) {
                final row = graph.rows[index];
                final highlighted = row.node.id == selected;
                return InkWell(
                  key: ValueKey('tree-${row.node.id}'),
                  canRequestFocus: false,
                  onTap: () => onRow(row),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  child: Container(
                    decoration: BoxDecoration(
                      color: highlighted ? ws.selected : null,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Stack(
                      children: [
                        Positioned(
                          left: 0,
                          top: 0,
                          bottom: 0,
                          width: graphWidth,
                          child: CustomPaint(
                            painter: _RowPainter(
                              lines[index],
                              lane: row.lane,
                              tip: row.branch != null,
                              spacing: spacing,
                              palette: ws,
                            ),
                          ),
                        ),
                        Padding(
                          padding: EdgeInsets.only(
                            left: graphWidth + WsMetrics.space2,
                            right: WsMetrics.space2,
                          ),
                          child: _RowContent(
                            row: row,
                            picked: picked,
                            highlighted: highlighted,
                          ),
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _RowContent extends StatelessWidget {
  const _RowContent({
    required this.row,
    required this.picked,
    required this.highlighted,
  });

  final GraphRow row;
  final Set<String>? picked;
  final bool highlighted;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final muted = TextStyle(
      color: highlighted ? ws.text : ws.textMuted,
      fontSize: 13,
    );
    final faint = TextStyle(
      color: ws.textFaint,
      fontSize: WsMetrics.fsSmall,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    final branch = row.branch;
    final node = row.node;
    final picked = this.picked;
    final isPrompt = node.kind == BridgeEntryKind.user;
    return Row(
      children: [
        if (row.isStep && !isPrompt) ...[
          ConstrainedBox(
            constraints: const BoxConstraints(minWidth: 16),
            child: Text('${row.step}', style: faint.copyWith(fontSize: 13)),
          ),
          const SizedBox(width: WsMetrics.space2),
        ],
        if (branch != null) ...[
          RefBadge(branch: branch),
          const SizedBox(width: WsMetrics.space2),
          Expanded(
            child: Text(
              branch.name,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: ws.text, fontSize: 13),
            ),
          ),
        ] else if (isPrompt) ...[
          Text(
            'Turn ${row.turn}',
            style: TextStyle(
              color: ws.text,
              fontSize: 13,
              fontWeight: WsMetrics.fwSemibold,
            ),
          ),
          const SizedBox(width: WsMetrics.space2),
          Expanded(
            child: Text(
              node.preview,
              overflow: TextOverflow.ellipsis,
              style: muted,
            ),
          ),
        ] else
          Expanded(
            child: Text(
              switch (node.kind) {
                BridgeEntryKind.assistant => 'Message',
                BridgeEntryKind.approval => 'Approval',
                BridgeEntryKind.toolResult => node.preview,
                _ => 'Branch point',
              },
              overflow: TextOverflow.ellipsis,
              style: muted,
            ),
          ),
        if (row.pending)
          Text('continues here', style: faint.copyWith(color: ws.accent))
        else if (branch != null && picked != null)
          _Check(on: picked.contains(branch.tip))
        else if (branch?.durationMs case final duration?)
          Text(formatDuration(duration), style: faint),
      ],
    );
  }
}

/// A branch's letter in its lane color.
class RefBadge extends StatelessWidget {
  const RefBadge({super.key, required this.branch});

  final Branch branch;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
    decoration: BoxDecoration(
      color: laneColor(context.ws, branch.lane),
      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
    ),
    child: Text(
      branch.ref,
      style: const TextStyle(
        color: Colors.white,
        fontSize: WsMetrics.fsSmall,
        height: 18 / WsMetrics.fsSmall,
        fontWeight: WsMetrics.fwMedium,
      ),
    ),
  );
}

class _Check extends StatelessWidget {
  const _Check({required this.on});

  final bool on;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      width: 13,
      height: 13,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: on ? ws.accent : null,
        borderRadius: BorderRadius.circular(3),
        border: on ? null : Border.all(color: ws.borderStrong, width: 1.5),
      ),
      child: on ? Icon(LucideIcons.check, size: 10, color: ws.onAccent) : null,
    );
  }
}

/// The graph lines crossing one row.
class _RowLines {
  /// Lanes that pass through without a node here.
  final Set<int> through = {};

  /// Lanes a fork from this row's node curves into.
  final List<int> curves = [];

  /// Whether the row's own lane continues below its node or arrives from above.
  bool down = false;
  bool up = false;
}

/// The lines of each graph, computed once per graph rather than on every build.
final _linesOf = Expando<List<_RowLines>>();

List<_RowLines> _lines(SessionGraph graph) {
  final lines = [for (final _ in graph.rows) _RowLines()];
  for (var index = 0; index < graph.rows.length; index++) {
    final row = graph.rows[index];
    final parent = row.parentRow;
    if (parent == null) continue;
    lines[index].up = true;
    if (graph.rows[parent].lane == row.lane) {
      lines[parent].down = true;
    } else {
      lines[parent].curves.add(row.lane);
    }
    for (var between = parent + 1; between < index; between++) {
      lines[between].through.add(row.lane);
    }
  }
  return lines;
}

class _RowPainter extends CustomPainter {
  _RowPainter(
    this.lines, {
    required this.lane,
    required this.tip,
    required this.spacing,
    required this.palette,
  });

  final _RowLines lines;
  final int lane;
  final bool tip;
  final double spacing;
  final WsPalette palette;

  @override
  void paint(Canvas canvas, Size size) {
    double x(int lane) => 14 + lane * spacing;
    final middle = size.height / 2;
    Paint stroke(int lane) => Paint()
      ..color = laneColor(palette, lane)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2;
    for (final through in lines.through) {
      canvas.drawLine(
        Offset(x(through), 0),
        Offset(x(through), size.height),
        stroke(through),
      );
    }
    if (lines.up) {
      canvas.drawLine(
        Offset(x(lane), 0),
        Offset(x(lane), middle),
        stroke(lane),
      );
    }
    if (lines.down) {
      canvas.drawLine(
        Offset(x(lane), middle),
        Offset(x(lane), size.height),
        stroke(lane),
      );
    }
    for (final target in lines.curves) {
      canvas.drawPath(
        Path()
          ..moveTo(x(lane), middle)
          ..cubicTo(
            x(lane),
            middle + 10,
            x(target),
            middle + 4,
            x(target),
            size.height,
          ),
        stroke(target),
      );
    }
    final center = Offset(x(lane), middle);
    if (tip) {
      canvas.drawCircle(center, 5, Paint()..color = laneColor(palette, lane));
    } else {
      canvas.drawCircle(center, 4, Paint()..color = palette.canvas);
      canvas.drawCircle(center, 4, stroke(lane));
    }
  }

  @override
  bool shouldRepaint(_RowPainter old) =>
      old.lines != lines ||
      old.lane != lane ||
      old.tip != tip ||
      old.spacing != spacing ||
      old.palette != palette;
}
