import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'cell_pickers.dart';
import 'row_popover.dart';
import 'table_cells.dart';
import 'table_groups.dart';
import 'table_tabs.dart';
import 'workspace_state.dart';

const _columnWidth = 224.0;
const _cardsShown = 50;

class BoardLayout extends StatefulWidget {
  const BoardLayout({
    super.key,
    required this.data,
    required this.tab,
    required this.group,
    required this.search,
    required this.revision,
    required this.onAddGroup,
  });

  final TableData data;
  final BridgeTab tab;
  final BridgeField? group;
  final String? search;
  final int revision;
  final VoidCallback onAddGroup;

  @override
  State<BoardLayout> createState() => _BoardLayoutState();
}

typedef _Column = ({BridgeGroup group, List<BridgeRow> rows});

class _BoardLayoutState extends State<BoardLayout> {
  List<_Column>? _columns;
  final _limits = <String, int>{};
  int _reads = 0;
  String? _open;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void didUpdateWidget(BoardLayout old) {
    super.didUpdateWidget(old);
    if (old.revision != widget.revision ||
        old.search != widget.search ||
        old.group?.id != widget.group?.id ||
        !sameSetup(old.tab, widget.tab)) {
      unawaited(_load());
    }
  }

  String _key(BridgeCell value) =>
      '${value.kind.name} ${value.text} ${value.checked}';

  Future<void> _load() async {
    final group = widget.group;
    final read = ++_reads;
    if (group == null) {
      setState(() => _columns = const []);
      return;
    }
    final data = widget.data;
    final tab = widget.tab;
    try {
      final groups = (await data.read(
        query(
          BridgeQueryKind.groups,
          block: data.block,
          field: group.id,
          filter: tab.filter,
          search: widget.search,
        ),
      )).groups;
      final columns = <_Column>[];
      for (final shown in groups) {
        final rows = (await data.read(
          query(
            BridgeQueryKind.rows,
            block: data.block,
            limit: _limits[_key(shown.value)] ?? _cardsShown,
            sort: tab.sort,
            filter: tab.filter,
            within: groupFilter(group, shown.value),
            search: widget.search,
          ),
        )).rows;
        columns.add((group: shown, rows: rows));
      }
      if (mounted && read == _reads) setState(() => _columns = columns);
    } on Object {
      return;
    }
  }

  void _write(Future<Object?> write) =>
      unawaited(write.then((_) {}, onError: (Object _) {}));

  void _dropped(BridgeRow row, _Column onto, BridgeRow? before) {
    final group = widget.group!;
    final held = cellOf(row, group.id) ?? emptyValue;
    final from = _columns!.where(
      (column) => column.rows.any((shown) => shown.id == row.id),
    );
    final same = from.isNotEmpty && identical(from.first, onto);
    if (!same) {
      final value = onto.group.value;
      if (sameGroup(held, value)) return;
      _write(widget.data.setCell(row.id, group.id, value));
      return;
    }
    if (widget.tab.sort != null || before == null || before.id == row.id) {
      return;
    }
    final rows = [
      for (final shown in onto.rows)
        if (shown.id != row.id) shown,
    ];
    final at = rows.indexWhere((shown) => shown.id == before.id);
    _write(
      widget.data.write(
        widget.data.change(
          BridgeMutationKind.moveRow,
          row: row.id,
          after: at <= 0 ? null : rows[at - 1].id,
        ),
      ),
    );
  }

  Future<void> _add(_Column column) async {
    final value = column.group.value;
    final filter = widget.tab.filter;
    final cells = {
      if (value.kind != BridgeCellKind.empty &&
          !(value.kind == BridgeCellKind.checkbox && !value.checked))
        widget.group!.id: value,
      if (filter
          case BridgeFilter(
            :final field,
            value: final shown?,
            matches: BridgeMatch.is_,
          )
          when field != widget.group!.id)
        field: shown,
    };
    try {
      final row = await widget.data.addRow(cells);
      if (mounted) setState(() => _open = row);
    } on Object {
      return;
    }
  }

  Widget _card(BridgeRow row, _Column column) {
    final ws = context.ws;
    final data = widget.data;
    final days = shownDays(data.days);
    final fields = data.database.fields;
    final shownFields = [
      for (final field in fields.skip(1))
        if (field.id != widget.group?.id && cellOf(row, field.id) != null)
          field,
    ];
    Widget face({bool lifted = false}) => Container(
      width: _columnWidth - 2 * WsMetrics.space2,
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.surface,
        border: Border.all(color: ws.border),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        boxShadow: lifted ? ws.shadow : null,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            rowTitle(row, data.database),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontWeight: WsMetrics.fwMedium,
              color: ws.text,
            ),
          ),
          for (final field in shownFields)
            Padding(
              padding: const EdgeInsets.only(top: WsMetrics.space1),
              child: DefaultTextStyle.merge(
                style: TextStyle(fontSize: WsMetrics.fsSmall),
                child: CellView(
                  cell: cellOf(row, field.id),
                  kind: field.kind,
                  options: data.options,
                  numbers: data.numbers,
                  days: days,
                ),
              ),
            ),
        ],
      ),
    );
    Widget shown = GestureDetector(
      key: const Key('board card'),
      behavior: HitTestBehavior.opaque,
      onTap: () => setState(() => _open = row.id),
      child: face(),
    );
    if (_open == row.id) {
      shown = AnchoredMenu(
        key: ValueKey('open ${row.id}'),
        beside: RowPopover.width,
        height: 60 + 30.0 * fields.length,
        onTapOutside: (_) => setState(() => _open = null),
        menu: (_) => RowPopover(
          data: data,
          row: row,
          onClose: () => setState(() => _open = null),
        ),
        child: shown,
      );
    }
    return Padding(
      padding: const EdgeInsets.only(bottom: WsMetrics.space2),
      child: DragTarget<BridgeRow>(
        onAcceptWithDetails: (dropped) => _dropped(dropped.data, column, row),
        builder: (context, offered, _) => Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (offered.isNotEmpty)
              Container(
                height: 2,
                margin: const EdgeInsets.only(bottom: WsMetrics.space1),
                color: ws.accent,
              ),
            Draggable<BridgeRow>(
              data: row,
              feedback: Material(
                color: Colors.transparent,
                child: face(lifted: true),
              ),
              childWhenDragging: Opacity(opacity: 0.4, child: face()),
              child: shown,
            ),
          ],
        ),
      ),
    );
  }

  Widget _column(_Column column) {
    final ws = context.ws;
    final group = widget.group!;
    final data = widget.data;
    final more = column.group.count - column.rows.length;
    return DragTarget<BridgeRow>(
      onAcceptWithDetails: (dropped) => _dropped(dropped.data, column, null),
      builder: (context, offered, _) => Container(
        key: const Key('board column'),
        width: _columnWidth,
        margin: const EdgeInsets.only(right: WsMetrics.space2),
        padding: const EdgeInsets.all(WsMetrics.space2),
        decoration: BoxDecoration(
          color: offered.isNotEmpty ? ws.accentSubtle : ws.subtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              key: const Key('column header'),
              height: 28,
              child: Row(
                children: [
                  Flexible(
                    child: GroupLabel(
                      field: group,
                      value: column.group.value,
                      days: shownDays(data.days),
                      numbers: data.numbers,
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  Text(
                    data.numbers.format(column.group.count),
                    style: TextStyle(
                      fontSize: WsMetrics.fsCompact,
                      color: ws.textFaint,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: WsMetrics.space1),
            for (final row in column.rows) _card(row, column),
            if (more > 0)
              TextButton(
                onPressed: () {
                  final key = _key(column.group.value);
                  _limits[key] = (_limits[key] ?? _cardsShown) + _cardsShown;
                  unawaited(_load());
                },
                child: Text('${data.numbers.format(more)} more'),
              ),
            InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: () => unawaited(_add(column)),
              child: SizedBox(
                height: 28,
                child: Row(
                  children: [
                    Icon(LucideIcons.plus, size: 14, color: ws.textFaint),
                    const SizedBox(width: WsMetrics.space2),
                    Text(
                      'New',
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.textFaint,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (widget.group == null) {
      return Container(
        padding: const EdgeInsets.all(WsMetrics.space4),
        decoration: BoxDecoration(
          color: ws.subtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                'A Board groups its rows by a Select field.',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textMuted,
                ),
              ),
            ),
            SecondaryButton(
              label: 'Add a Select field',
              onPressed: widget.onAddGroup,
            ),
          ],
        ),
      );
    }
    final columns = _columns;
    if (columns == null) return const SizedBox(height: 120);
    return SingleChildScrollView(
      key: const Key('board columns'),
      scrollDirection: Axis.horizontal,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [for (final column in columns) _column(column)],
      ),
    );
  }
}
