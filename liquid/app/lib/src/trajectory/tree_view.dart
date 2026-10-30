import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/history.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// Widths of the open session tree and of its collapsed rail (approved navigation mockup).
const treeWidth = 200.0;
const treeRailWidth = 24.0;
const _rowHeight = 22.0;

/// The session's branches as a graph like GitHub's network view: one lane per branch and one
/// node per step, its shape and color telling the step's kind as the ledger's Kind tags do
/// (approved navigation mockup). Lines are gray but for the shown branches. Each row paints its
/// own slice, so long trees stay a lazy list. Collapsed, it is a rail with one dot per turn.
class SessionTreeView extends StatelessWidget {
  const SessionTreeView({
    super.key,
    required this.graph,
    required this.selected,
    required this.onRow,
    this.shown = const {},
    this.picked,
    this.open = true,
    this.onOpen,
  });

  final SessionGraph graph;

  /// The entry of the highlighted row: the selected step, or the cursor while comparing.
  final String? selected;
  final ValueChanged<GraphRow> onRow;

  /// The tips of the branches the ledger or the comparison shows.
  final Set<String> shown;

  /// The tips selected for comparison, when comparing.
  final Set<String>? picked;

  /// Whether the tree is open rather than a rail.
  final bool open;
  final ValueChanged<bool>? onOpen;

  /// The rows on the paths from the root to the shown tips.
  Set<int> _path() {
    final path = <int>{};
    for (final (index, row) in graph.rows.indexed) {
      if (!shown.contains(row.node.id)) continue;
      int? at = index;
      while (at != null && path.add(at)) {
        at = graph.rows[at].parentRow;
      }
    }
    return path;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (!open) return _Rail(tree: this);
    final lines = _linesOf[graph] ??= _lines(graph);
    final path = _path();
    final spacing = graph.lanes < 2
        ? 14.0
        : math.min(14.0, 60 / (graph.lanes - 1));
    final graphWidth = 20 + (graph.lanes - 1) * spacing;
    return Container(
      width: treeWidth,
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space2,
        WsMetrics.space2,
        WsMetrics.space2,
        WsMetrics.space2,
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
                const SizedBox(width: WsMetrics.space1),
                Text(
                  'Session tree',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                    fontWeight: WsMetrics.fwMedium,
                  ),
                ),
                const Spacer(),
                Hint(Fixed.moveInTree.display),
                _Toggle(open: true, onOpen: onOpen),
              ],
            ),
          ),
          Expanded(
            child: ListView.builder(
              itemExtent: _rowHeight,
              itemCount: graph.rows.length,
              itemBuilder: (context, index) {
                final row = graph.rows[index];
                final highlighted = row.node.id == selected;
                return InkWell(
                  key: ValueKey('tree-${row.node.id}'),
                  canRequestFocus: false,
                  onTap: () => onRow(row),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                  child: Container(
                    decoration: BoxDecoration(
                      color: highlighted ? ws.selected : null,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                    ),
                    child: Stack(
                      children: [
                        Positioned(
                          left: 0,
                          top: 0,
                          bottom: 0,
                          width: graphWidth,
                          // The node's kind is told, not only drawn.
                          child: Semantics(
                            container: true,
                            label: _kindLabel(row.node),
                            child: CustomPaint(
                              painter: _RowPainter(
                                lines[index],
                                row: index,
                                kind: row.node.kind,
                                failed: row.node.failed,
                                selected: highlighted,
                                lane: row.lane,
                                path: path,
                                spacing: spacing,
                                palette: ws,
                              ),
                            ),
                          ),
                        ),
                        Padding(
                          padding: EdgeInsets.only(
                            left: graphWidth + WsMetrics.space1,
                            right: WsMetrics.space1,
                          ),
                          child: _RowContent(
                            row: row,
                            picked: picked,
                            shown: shown,
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

/// What a node's shape and color show, for assistive technology.
String _kindLabel(BridgeTreeNode node) {
  final kind = switch (node.kind) {
    BridgeEntryKind.user => 'prompt',
    BridgeEntryKind.assistant => 'response step',
    BridgeEntryKind.toolResult => 'tool step',
    BridgeEntryKind.approval => 'approval step',
    _ => 'branch point',
  };
  return node.failed ? '$kind, failed' : kind;
}

/// Opens or collapses the tree; return and space press it rather than the view's own keys.
class _Toggle extends StatelessWidget {
  const _Toggle({required this.open, required this.onOpen});

  final bool open;
  final ValueChanged<bool>? onOpen;

  @override
  Widget build(BuildContext context) => Tooltip(
    message: open ? 'Collapse the session tree' : 'Expand the session tree',
    child: Shortcuts(
      shortcuts: activateKeys,
      child: InkWell(
        onTap: onOpen == null ? null : () => onOpen!(!open),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        child: SizedBox(
          width: 22,
          height: 22,
          child: Icon(
            open ? LucideIcons.panelRightClose : LucideIcons.panelRightOpen,
            size: 14,
            color: context.ws.textMuted,
          ),
        ),
      ),
    ),
  );
}

/// The collapsed tree: a rail with one dot per turn, filled for the selected step's turn.
class _Rail extends StatelessWidget {
  const _Rail({required this.tree});

  final SessionTreeView tree;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final turns = [
      for (final row in tree.graph.rows)
        if (row.node.kind == BridgeEntryKind.user) row,
    ];
    final selected = tree.graph.rows
        .where((row) => row.node.id == tree.selected)
        .firstOrNull;
    return Container(
      width: treeRailWidth,
      padding: const EdgeInsets.only(top: WsMetrics.space2),
      decoration: BoxDecoration(
        border: Border(left: BorderSide(color: ws.border)),
      ),
      child: Column(
        children: [
          _Toggle(open: false, onOpen: tree.onOpen),
          const SizedBox(height: WsMetrics.space2),
          Expanded(
            child: ListView.builder(
              itemExtent: 16,
              itemCount: turns.length,
              itemBuilder: (context, index) {
                final turn = turns[index];
                final on = selected?.turn == turn.turn;
                return Tooltip(
                  message: 'Turn ${turn.turn} · ${turn.node.preview}',
                  child: InkWell(
                    canRequestFocus: false,
                    onTap: () => tree.onRow(turn),
                    child: Center(
                      child: Container(
                        width: 8,
                        height: 8,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: on ? ws.accent : null,
                          border: Border.all(
                            color: on ? ws.accent : ws.textFaint,
                            width: 1.5,
                          ),
                        ),
                      ),
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
    required this.shown,
    required this.highlighted,
  });

  final GraphRow row;
  final Set<String>? picked;
  final Set<String> shown;
  final bool highlighted;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final muted = TextStyle(
      color: highlighted ? ws.text : ws.textMuted,
      fontSize: WsMetrics.fsSmall,
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
    final text = switch (node.kind) {
      BridgeEntryKind.assistant when node.preview.isEmpty => '(tool calls)',
      BridgeEntryKind.assistant ||
      BridgeEntryKind.approval ||
      BridgeEntryKind.toolResult => node.preview,
      _ => 'Branch point',
    };
    return Row(
      children: [
        if (row.isStep && !isPrompt) ...[
          ConstrainedBox(
            constraints: const BoxConstraints(minWidth: 14),
            child: Text('${row.step}', style: faint),
          ),
          const SizedBox(width: WsMetrics.space1),
        ],
        if (branch != null) ...[
          RefBadge(ref: branch.ref, shown: shown.contains(branch.tip)),
          const SizedBox(width: WsMetrics.space1),
          Expanded(
            child: Text(
              branch.name,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: ws.text, fontSize: WsMetrics.fsSmall),
            ),
          ),
        ] else if (isPrompt) ...[
          Text(
            'T${row.turn}',
            style: TextStyle(
              color: ws.text,
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwSemibold,
            ),
          ),
          const SizedBox(width: WsMetrics.space1),
          Expanded(
            child: Tooltip(
              message: node.preview,
              child: Text(
                node.preview,
                overflow: TextOverflow.ellipsis,
                style: muted,
              ),
            ),
          ),
        ] else
          Expanded(
            child: Tooltip(
              message: text,
              child: Text(
                text,
                overflow: TextOverflow.ellipsis,
                style: node.kind == BridgeEntryKind.toolResult
                    ? monoStyle(
                        context,
                        color: muted.color,
                      ).copyWith(fontSize: WsMetrics.fsCaption)
                    : muted,
              ),
            ),
          ),
        if (row.pending)
          Flexible(
            child: Text(
              'continues here',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: faint.copyWith(color: ws.accent),
            ),
          )
        else if (branch != null && picked != null)
          _Check(on: picked.contains(branch.tip))
        else if (branch?.durationMs case final duration?)
          Flexible(
            child: Text(
              formatDuration(duration),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: faint,
            ),
          ),
      ],
    );
  }
}

/// A branch's letter: blue for a shown branch, gray otherwise (approved navigation mockup).
class RefBadge extends StatelessWidget {
  const RefBadge({super.key, required this.ref, required this.shown});

  final String ref;
  final bool shown;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: shown ? ws.accent : ws.hover,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        ref,
        style: TextStyle(
          color: shown ? ws.onAccent : ws.textMuted,
          fontSize: WsMetrics.fsSmall,
          height: 18 / WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwMedium,
        ),
      ),
    );
  }
}

class _Check extends StatelessWidget {
  const _Check({required this.on});

  final bool on;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    // Whether the branch is picked is told to assistive technology, not only drawn.
    return Semantics(
      checked: on,
      child: Container(
        width: 13,
        height: 13,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: on ? ws.accent : null,
          borderRadius: BorderRadius.circular(3),
          border: on ? null : Border.all(color: ws.borderStrong, width: 1.5),
        ),
        child: on
            ? Icon(LucideIcons.check, size: 10, color: ws.onAccent)
            : null,
      ),
    );
  }
}

/// The graph lines crossing one row, each with the row it leads to.
class _RowLines {
  /// Lanes that pass through without a node here.
  final Map<int, int> through = {};

  /// Lanes a fork from this row's node curves into.
  final List<(int, int)> curves = [];

  /// The row below in the same lane that this row's node leads to.
  int? down;

  /// Whether the row's own lane arrives from above.
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
      lines[parent].down = index;
    } else {
      lines[parent].curves.add((row.lane, index));
    }
    for (var between = parent + 1; between < index; between++) {
      lines[between].through[row.lane] = index;
    }
  }
  return lines;
}

class _RowPainter extends CustomPainter {
  _RowPainter(
    this.lines, {
    required this.row,
    required this.kind,
    required this.failed,
    required this.selected,
    required this.lane,
    required this.path,
    required this.spacing,
    required this.palette,
  });

  final _RowLines lines;
  final int row;
  final BridgeEntryKind kind;
  final bool failed;
  final bool selected;
  final int lane;

  /// The rows on the shown branches' paths, whose lines are blue.
  final Set<int> path;
  final double spacing;
  final WsPalette palette;

  @override
  void paint(Canvas canvas, Size size) {
    double x(int lane) => 10 + lane * spacing;
    final middle = size.height / 2;
    Paint stroke(int target) => Paint()
      ..color = path.contains(target)
          ? palette.accent
          : palette.textFaint.withValues(alpha: 0.45)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    for (final MapEntry(key: through, value: target) in lines.through.entries) {
      canvas.drawLine(
        Offset(x(through), 0),
        Offset(x(through), size.height),
        stroke(target),
      );
    }
    if (lines.up) {
      canvas.drawLine(Offset(x(lane), 0), Offset(x(lane), middle), stroke(row));
    }
    if (lines.down case final down?) {
      canvas.drawLine(
        Offset(x(lane), middle),
        Offset(x(lane), size.height),
        stroke(down),
      );
    }
    for (final (target, child) in lines.curves) {
      canvas.drawPath(
        Path()
          ..moveTo(x(lane), middle)
          ..cubicTo(
            x(lane),
            middle + 8,
            x(target),
            middle + 3,
            x(target),
            size.height,
          ),
        stroke(child),
      );
    }
    _node(canvas, Offset(x(lane), middle));
  }

  /// The node: a ring for a prompt, a dot for a response, a square for a tool, a diamond for an
  /// approval; failed ones outlined in red, the selected one outlined in blue.
  void _node(Canvas canvas, Offset center) {
    final fill = Paint();
    final outline = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    final Path shape;
    var outlined = failed;
    switch (kind) {
      case BridgeEntryKind.user:
        shape = Path()..addOval(Rect.fromCircle(center: center, radius: 5));
        fill.color = palette.canvas;
        outline.color = palette.textFaint;
        outlined = true;
      case BridgeEntryKind.assistant:
        shape = Path()..addOval(Rect.fromCircle(center: center, radius: 3.5));
        fill.color = palette.data1;
      case BridgeEntryKind.toolResult:
        shape = Path()
          ..addRect(Rect.fromCenter(center: center, width: 6.5, height: 6.5));
        fill.color = palette.data2;
      case BridgeEntryKind.approval:
        shape = Path()
          ..moveTo(center.dx, center.dy - 4.5)
          ..lineTo(center.dx + 4.5, center.dy)
          ..lineTo(center.dx, center.dy + 4.5)
          ..lineTo(center.dx - 4.5, center.dy)
          ..close();
        fill.color = palette.warning;
      default:
        shape = Path()..addOval(Rect.fromCircle(center: center, radius: 3));
        fill.color = palette.canvas;
        outline.color = palette.textFaint;
        outlined = true;
    }
    if (failed) {
      fill.color = palette.canvas;
      outline.color = palette.danger;
    }
    canvas.drawPath(shape, fill);
    if (outlined) canvas.drawPath(shape, outline);
    if (selected) {
      canvas.drawPath(
        shape,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5
          ..color = palette.accent,
      );
    }
  }

  @override
  bool shouldRepaint(_RowPainter old) =>
      old.lines != lines ||
      old.row != row ||
      old.kind != kind ||
      old.failed != failed ||
      old.selected != selected ||
      old.lane != lane ||
      old.path != path ||
      old.spacing != spacing ||
      old.palette != palette;
}
