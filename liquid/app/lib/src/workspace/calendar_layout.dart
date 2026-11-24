import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'row_popover.dart';
import 'table_tabs.dart';
import 'table_view_parts.dart' show touchFirst;
import 'workspace_state.dart';

const _dayHeight = 116.0;
const _shownPerDay = 3;
const _rowsRead = 1000;
const _listWidth = 260.0;

final _iso = DateFormat('yyyy-MM-dd');

String isoDay(DateTime day) => _iso.format(day);

class CalendarLayout extends StatefulWidget {
  const CalendarLayout({
    super.key,
    required this.data,
    required this.tab,
    required this.date,
    required this.month,
    required this.revision,
    required this.onAddDate,
  });

  final TableData data;
  final BridgeTab tab;
  final BridgeField? date;
  final DateTime month;
  final int revision;
  final VoidCallback onAddDate;

  @override
  State<CalendarLayout> createState() => _CalendarLayoutState();
}

class _CalendarLayoutState extends State<CalendarLayout> {
  Map<String, List<BridgeRow>> _days = const {};
  List<BridgeRow> _undated = const [];
  int _undatedCount = 0;
  ({int shown, int count}) _dated = (shown: 0, count: 0);
  int _reads = 0;
  String? _open;
  final _popover = Object();
  String? _listed;
  String? _hovered;
  DateTime? _first;
  int _weeks = 5;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _span();
    unawaited(_load());
  }

  @override
  void didUpdateWidget(CalendarLayout old) {
    super.didUpdateWidget(old);
    if (old.month != widget.month) _span();
    if (old.revision != widget.revision ||
        old.month != widget.month ||
        old.date?.id != widget.date?.id ||
        !sameSetup(old.tab, widget.tab)) {
      unawaited(_load());
    }
  }

  void _span() {
    final starts = MaterialLocalizations.of(context).firstDayOfWeekIndex;
    final month = DateTime(widget.month.year, widget.month.month);
    final back = (month.weekday % 7 - starts) % 7;
    _first = DateTime(month.year, month.month, 1 - back);
    final days = DateTime(month.year, month.month + 1, 0).day;
    _weeks = ((back + days) / 7).ceil();
  }

  DateTime _dayAt(int index) =>
      DateTime(_first!.year, _first!.month, _first!.day + index);

  Future<void> _load() async {
    final date = widget.date;
    final read = ++_reads;
    if (date == null || _first == null) return;
    final data = widget.data;
    final tab = widget.tab;
    BridgeQuery within(BridgeFilter range, int limit) => query(
      BridgeQueryKind.rows,
      block: data.block,
      limit: limit,
      sort: tab.sort,
      filter: tab.filter,
      within: range,
    );
    try {
      final dated = await data.read(
        within(
          BridgeFilter(
            field: date.id,
            matches: BridgeMatch.between,
            value: textValue(BridgeCellKind.date, isoDay(_dayAt(0))),
            until: isoDay(_dayAt(_weeks * 7 - 1)),
          ),
          _rowsRead,
        ),
      );
      final undated = await data.read(
        within(
          BridgeFilter(field: date.id, matches: BridgeMatch.empty),
          _listed == '' ? _rowsRead : 0,
        ),
      );
      if (!mounted || read != _reads) return;
      final days = <String, List<BridgeRow>>{};
      for (final row in dated.rows) {
        final day = cellOf(row, date.id)?.text;
        if (day != null) (days[day] ??= []).add(row);
      }
      setState(() {
        _days = days;
        _undated = undated.rows;
        _undatedCount = undated.count;
        _dated = (shown: dated.rows.length, count: dated.count);
      });
    } on Object {
      return;
    }
  }

  void _write(Future<Object?> write) =>
      unawaited(write.then((_) {}, onError: (Object _) {}));

  void _moveTo(BridgeRow row, String day) {
    final date = widget.date!;
    if (cellOf(row, date.id)?.text == day) return;
    _write(
      widget.data.setCell(row.id, date.id, textValue(BridgeCellKind.date, day)),
    );
  }

  Future<void> _add(String day) async {
    final filter = widget.tab.filter;
    try {
      final row = await widget.data.addRow({
        widget.date!.id: textValue(BridgeCellKind.date, day),
        if (filter
            case BridgeFilter(
              :final field,
              value: final shown?,
              matches: BridgeMatch.is_,
            )
            when field != widget.date!.id)
          field: shown,
      });
      if (mounted) setState(() => _open = row);
    } on Object {
      return;
    }
  }

  void _openList(String? day) {
    setState(() => _listed = day);
    if (day == '') unawaited(_load());
  }

  Widget _chip(BridgeRow row, {double? width}) {
    final ws = context.ws;
    Widget face({bool lifted = false}) => Container(
      width: width,
      height: 20,
      margin: const EdgeInsets.only(top: 2),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: ws.surface,
        border: Border.all(color: ws.border),
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        boxShadow: lifted ? ws.shadow : null,
      ),
      child: Text(
        rowTitle(row, widget.data.database),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.text),
      ),
    );
    return Draggable<BridgeRow>(
      data: row,
      feedback: Material(
        color: Colors.transparent,
        child: SizedBox(width: width ?? 140, child: face(lifted: true)),
      ),
      childWhenDragging: Opacity(opacity: 0.4, child: face()),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => setState(() {
          _listed = null;
          _open = row.id;
        }),
        child: face(),
      ),
    );
  }

  Widget _withPopover(List<BridgeRow> rows, Widget child, {Key? key}) {
    final open = rows.where((row) => row.id == _open).firstOrNull;
    if (open == null) return child;
    return AnchoredMenu(
      key: key ?? ValueKey('open $_open'),
      beside: RowPopover.width,
      group: _popover,
      height: 60 + 30.0 * widget.data.database.fields.length,
      onTapOutside: (_) => setState(() => _open = null),
      menu: (_) => RowPopover(
        data: widget.data,
        row: open,
        onClose: () => setState(() => _open = null),
        group: _popover,
      ),
      child: child,
    );
  }

  Widget _list(String title, List<BridgeRow> rows, Widget child) {
    final ws = context.ws;
    return AnchoredMenu(
      height: 320,
      onTapOutside: (_) => setState(() => _listed = null),
      menu: (_) => FloatingPanel(
        key: const Key('day list'),
        width: _listWidth,
        constraints: const BoxConstraints(maxHeight: 320),
        padding: const EdgeInsets.all(WsMetrics.space2),
        child: Material(
          type: MaterialType.transparency,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                title,
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
              Flexible(
                child: ListView(
                  shrinkWrap: true,
                  children: [for (final row in rows) _chip(row)],
                ),
              ),
            ],
          ),
        ),
      ),
      child: child,
    );
  }

  Widget _day(DateTime day, double width) {
    final ws = context.ws;
    final key = isoDay(day);
    final rows = _days[key] ?? const <BridgeRow>[];
    final inMonth = day.month == widget.month.month;
    final today = key == isoDay(DateTime.now());
    final more = rows.length - _shownPerDay;
    final hovered = _hovered == key || touchFirst;
    Widget moreLabel = GestureDetector(
      onTap: () => _openList(key),
      child: Padding(
        padding: const EdgeInsets.only(top: 2, left: WsMetrics.space1),
        child: Text(
          '+${widget.data.numbers.format(more)} more',
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted),
        ),
      ),
    );
    if (_listed == key) {
      moreLabel = _list(DateFormat.MMMd().format(day), rows, moreLabel);
    }
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = key),
      onExit: (_) {
        if (_hovered == key) setState(() => _hovered = null);
      },
      child: DragTarget<BridgeRow>(
        onAcceptWithDetails: (dropped) => _moveTo(dropped.data, key),
        builder: (context, offered, _) => _withPopover(
          rows,
          Container(
            key: Key('day $key'),
            width: width,
            height: _dayHeight,
            padding: const EdgeInsets.all(WsMetrics.space1),
            decoration: BoxDecoration(
              color: offered.isNotEmpty
                  ? ws.accentSubtle
                  : inMonth
                  ? ws.canvas
                  : ws.subtle,
              border: Border(
                right: BorderSide(color: ws.border),
                bottom: BorderSide(color: ws.border),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  height: 20,
                  child: Row(
                    children: [
                      Container(
                        padding: const EdgeInsets.symmetric(horizontal: 5),
                        decoration: BoxDecoration(
                          color: today ? ws.accent : null,
                          borderRadius: BorderRadius.circular(10),
                        ),
                        child: Text(
                          '${day.day}',
                          style: TextStyle(
                            fontSize: WsMetrics.fsSmall,
                            color: today
                                ? ws.surface
                                : inMonth
                                ? ws.textMuted
                                : ws.textFaint,
                          ),
                        ),
                      ),
                      const Spacer(),
                      if (hovered)
                        Semantics(
                          button: true,
                          label:
                              'Add a row on ${DateFormat.MMMd().format(day)}',
                          child: GestureDetector(
                            key: Key('add on $key'),
                            onTap: () => unawaited(_add(key)),
                            child: Icon(
                              LucideIcons.plus,
                              size: 14,
                              color: ws.textFaint,
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
                for (final row in rows.take(_shownPerDay)) _chip(row),
                if (more > 0) moreLabel,
              ],
            ),
          ),
          key: ValueKey('open day $key'),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (widget.date == null) {
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
                'A Calendar places its rows on the days of a Date field.',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textMuted,
                ),
              ),
            ),
            SecondaryButton(
              label: 'Add a Date field',
              onPressed: widget.onAddDate,
            ),
          ],
        ),
      );
    }
    final locale = Localizations.localeOf(context).toString();
    final weekdays = DateFormat.E(
      DateFormat.localeExists(locale) ? locale : 'en',
    );
    Widget noDate = GestureDetector(
      onTap: () => _openList(''),
      child: Text(
        'No date · ${widget.data.numbers.format(_undatedCount)}',
        style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textMuted),
      ),
    );
    if (_listed == '') noDate = _list('No date', _undated, noDate);
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = (constraints.maxWidth - 1) / 7;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              child: Container(
                decoration: BoxDecoration(
                  border: Border(
                    left: BorderSide(color: ws.border),
                    top: BorderSide(color: ws.border),
                  ),
                ),
                child: Column(
                  children: [
                    Row(
                      children: [
                        for (var index = 0; index < 7; index++)
                          Container(
                            width: width,
                            height: 26,
                            alignment: Alignment.center,
                            decoration: BoxDecoration(
                              color: ws.subtle,
                              border: Border(
                                right: BorderSide(color: ws.border),
                                bottom: BorderSide(color: ws.border),
                              ),
                            ),
                            child: Text(
                              weekdays.format(_dayAt(index)),
                              style: TextStyle(
                                fontSize: WsMetrics.fsSmall,
                                color: ws.textFaint,
                              ),
                            ),
                          ),
                      ],
                    ),
                    for (var week = 0; week < _weeks; week++)
                      Row(
                        children: [
                          for (var index = 0; index < 7; index++)
                            _day(_dayAt(week * 7 + index), width),
                        ],
                      ),
                  ],
                ),
              ),
            ),
            if (_undatedCount > 0 || _dated.count > _dated.shown)
              Padding(
                padding: const EdgeInsets.only(top: WsMetrics.space2),
                child: Row(
                  children: [
                    if (_undatedCount > 0) _withPopover(_undated, noDate),
                    const Spacer(),
                    if (_dated.count > _dated.shown)
                      Text(
                        '${widget.data.numbers.format(_dated.shown)} of '
                        '${widget.data.numbers.format(_dated.count)} rows shown',
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.textFaint,
                        ),
                      ),
                  ],
                ),
              ),
          ],
        );
      },
    );
  }
}
