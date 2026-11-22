import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'table_cells.dart';
import 'table_tabs.dart';

BridgeFilter groupFilter(BridgeField field, BridgeCell value) => BridgeFilter(
  field: field.id,
  matches: value.kind == BridgeCellKind.empty
      ? BridgeMatch.empty
      : BridgeMatch.is_,
  value: value.kind == BridgeCellKind.empty ? null : value,
);

bool folds(BridgeTab tab, BridgeCell value) =>
    tab.folded.any((folded) => sameGroup(folded, value));

List<BridgeCell> foldedToggled(BridgeTab tab, BridgeCell value) =>
    folds(tab, value)
    ? [
        for (final folded in tab.folded)
          if (!sameGroup(folded, value)) folded,
      ]
    : [...tab.folded, value];

class GroupLines {
  GroupLines(this.groups, bool Function(BridgeCell value) folded) {
    var row = 0;
    var line = 0;
    for (final (index, group) in groups.indexed) {
      final shut = folded(group.value);
      final count = shut ? 0 : group.count;
      _segments.add((group: index, line: line, row: row, count: count));
      line += 1 + count;
      row += count;
      _total += group.count;
    }
    lineCount = line;
    rowCount = row;
  }

  final List<BridgeGroup> groups;
  final _segments = <({int group, int line, int row, int count})>[];
  late final int lineCount;
  late final int rowCount;
  int _total = 0;

  int get total => _total;

  ({int? group, int? row}) at(int line) {
    for (final segment in _segments.reversed) {
      if (line < segment.line) continue;
      return line == segment.line
          ? (group: segment.group, row: null)
          : (group: null, row: segment.row + line - segment.line - 1);
    }
    return (group: null, row: null);
  }

  int lineOf(int row) {
    for (final segment in _segments) {
      if (row < segment.row + segment.count) {
        return segment.line + 1 + row - segment.row;
      }
    }
    return lineCount;
  }

  Iterable<({BridgeGroup group, int start, int count})> within(
    int start,
    int limit,
  ) sync* {
    for (final segment in _segments) {
      final from = start > segment.row ? start - segment.row : 0;
      final end = start + limit - segment.row;
      final to = end < segment.count ? end : segment.count;
      if (to > from) {
        yield (group: groups[segment.group], start: from, count: to - from);
      }
    }
  }
}

class GroupLabel extends StatelessWidget {
  const GroupLabel({
    super.key,
    required this.field,
    required this.value,
    required this.days,
    required this.numbers,
  });

  final BridgeField field;
  final BridgeCell value;
  final DateFormat days;
  final NumberFormat numbers;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return switch (value.kind) {
      BridgeCellKind.empty => Text(
        'No ${field.name}',
        style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textFaint),
      ),
      BridgeCellKind.checkbox => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          CellView(
            cell: value,
            kind: field.kind,
            options: const {},
            numbers: numbers,
            days: days,
          ),
          const SizedBox(width: WsMetrics.space2),
          Text(
            field.name,
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: ws.textMuted,
            ),
          ),
        ],
      ),
      _ => CellView(
        cell: value,
        kind: field.kind,
        options: {for (final option in field.options) option.id: option},
        numbers: numbers,
        days: days,
      ),
    };
  }
}

class GroupHeader extends StatelessWidget {
  const GroupHeader({
    super.key,
    required this.label,
    required this.count,
    required this.folded,
    required this.onToggle,
    required this.numbers,
  });

  final Widget label;
  final int count;
  final bool folded;
  final VoidCallback onToggle;
  final NumberFormat numbers;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('group header'),
      height: WsMetrics.rowTable,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.canvas,
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        children: [
          Semantics(
            button: true,
            label: folded ? 'Unfold the group' : 'Fold the group',
            child: GestureDetector(
              key: const Key('fold group'),
              behavior: HitTestBehavior.opaque,
              onTap: onToggle,
              child: Padding(
                padding: const EdgeInsets.all(WsMetrics.space1),
                child: Icon(
                  folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
                  size: 14,
                  color: ws.textFaint,
                ),
              ),
            ),
          ),
          const SizedBox(width: WsMetrics.space1),
          Flexible(child: label),
          const SizedBox(width: WsMetrics.space2),
          Text(
            numbers.format(count),
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: ws.textFaint,
            ),
          ),
        ],
      ),
    );
  }
}
