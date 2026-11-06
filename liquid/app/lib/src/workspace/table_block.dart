import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'follow.dart';
import 'page_views.dart';
import 'windows.dart';
import 'workspace_state.dart';

/// Rows a Table shows before it scrolls inside, as the Chat Block keeps a fixed height.
const _shownRows = 10;
const _headerHeight = 32.0;
const _footerHeight = 30.0;

/// The narrowest the first field gets; it takes the width the others leave.
const _firstWidth = 200.0;

double _width(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 180,
  BridgeFieldKind.number => 104,
  BridgeFieldKind.select => 128,
  BridgeFieldKind.date => 120,
  BridgeFieldKind.checkbox => 72,
};

IconData _icon(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => LucideIcons.text,
  BridgeFieldKind.number => LucideIcons.hash,
  BridgeFieldKind.select => LucideIcons.circleChevronDown,
  BridgeFieldKind.date => LucideIcons.calendar,
  BridgeFieldKind.checkbox => LucideIcons.squareCheck,
};

/// A Table on its Page, as the approved Table View mockups show it: its name, a header of
/// its fields, its rows, read in windows and scrolling inside a fixed height, and their
/// count. Fields wider than the reading column scroll sideways with the first field in
/// view. It follows only its own Block (client architecture rule 6).
class TableBlock extends StatefulWidget {
  const TableBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<TableBlock> createState() => _TableBlockState();
}

class _TableBlockState extends State<TableBlock> {
  BridgeDatabase? _database;
  late final _rows = Windows<BridgeRow>((start, limit) async {
    final answer = await _state.workspace.query(
      query: query(
        BridgeQueryKind.rows,
        page: _page,
        block: _block,
        start: start,
        limit: limit,
      ),
    );
    return (answer.count, answer.rows);
  }, _report);
  final _sideways = ScrollController();
  StreamSubscription<BridgeChange>? _followed;

  /// Reads asked for, so only the latest shows.
  int _reads = 0;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _rows.addListener(_changed);
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
  }

  @override
  void dispose() {
    unawaited(_followed?.cancel());
    _rows.dispose();
    _sideways.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// Reads the name and fields, then the rows read so far.
  Future<void> _read() async {
    final read = ++_reads;
    try {
      final answer = await _state.workspace.query(
        query: query(BridgeQueryKind.database, page: _page, block: _block),
      );
      if (!mounted || read != _reads) return;
      setState(() => _database = answer.database);
    } on Object catch (error) {
      _report(error);
      return;
    }
    await _rows.reread();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final database = _database;
    final locale = Localizations.localeOf(context).toString();
    final count = _rows.count ?? 0;
    final name = database?.name ?? '';
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: WsMetrics.space3),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: 32,
            child: Row(
              children: [
                Icon(LucideIcons.table, size: 16, color: ws.textFaint),
                const SizedBox(width: WsMetrics.space3),
                Flexible(
                  child: Text(
                    name.isEmpty ? 'Untitled' : name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 16,
                      fontWeight: WsMetrics.fwSemibold,
                      color: name.isEmpty ? ws.textFaint : ws.text,
                    ),
                  ),
                ),
              ],
            ),
          ),
          if (database != null)
            ClipRRect(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              child: Container(
                foregroundDecoration: BoxDecoration(
                  border: Border.all(color: ws.border),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    LayoutBuilder(
                      builder: (context, constraints) =>
                          _grid(database.fields, constraints.maxWidth, locale),
                    ),
                    Container(
                      height: _footerHeight,
                      color: ws.subtle,
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      alignment: Alignment.centerLeft,
                      child: Text(
                        '${NumberFormat.decimalPattern(locale).format(count)} '
                        '${count == 1 ? 'row' : 'rows'}',
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.textFaint,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// The header and the rows, scrolling sideways together when wider than `width`.
  Widget _grid(List<BridgeField> fields, double width, String locale) {
    final ws = context.ws;
    final rest = [for (final field in fields.skip(1)) _width(field.kind)];
    final others = rest.fold(0.0, (sum, width) => sum + width);
    final first = math.max(_firstWidth, width - others);
    final widths = [first, ...rest];
    final count = _rows.count ?? 0;
    final options = {
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select)
          for (final option in field.options) option.id: option,
    };
    final numbers = NumberFormat.decimalPattern(locale);
    final days = DateFormat.yMMMd(locale);

    Widget content(BridgeCell? cell) => switch (cell?.kind) {
      null || BridgeCellKind.empty => const SizedBox.shrink(),
      BridgeCellKind.text => _text(cell!.text ?? ''),
      BridgeCellKind.number => _text(
        numbers.format(cell!.number),
        style: const TextStyle(fontFeatures: [FontFeature.tabularFigures()]),
      ),
      BridgeCellKind.select => switch (options[cell!.text]) {
        final option? => _Tag(option),
        null => const SizedBox.shrink(),
      },
      BridgeCellKind.date => _text(switch (DateTime.tryParse(cell!.text!)) {
        final day? => days.format(day),
        null => cell.text!,
      }),
      BridgeCellKind.checkbox => _Checkbox(checked: cell!.checked),
    };

    Widget line({
      required double height,
      required Color? color,
      required Widget Function(int index) cell,
    }) => ListenableBuilder(
      listenable: _sideways,
      builder: (context, _) {
        final offset = _sideways.hasClients ? _sideways.offset : 0.0;
        Widget box(int index) => Container(
          width: widths[index],
          height: height,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          decoration: BoxDecoration(
            color: color,
            border: index > 0
                ? Border(left: BorderSide(color: ws.border))
                : null,
            boxShadow: index == 0 && offset > 0
                ? const [
                    BoxShadow(
                      color: Color(0x40000000),
                      blurRadius: 6,
                      offset: Offset(4, 0),
                      spreadRadius: -4,
                    ),
                  ]
                : null,
          ),
          child: cell(index),
        );
        // The first field stays in view while the others scroll sideways.
        return Stack(
          children: [
            Row(
              children: [
                SizedBox(width: first),
                for (var index = 1; index < widths.length; index++) box(index),
              ],
            ),
            Positioned(left: offset, top: 0, bottom: 0, child: box(0)),
          ],
        );
      },
    );

    return SingleChildScrollView(
      controller: _sideways,
      scrollDirection: Axis.horizontal,
      child: SizedBox(
        width: first + others,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Over the cells, so the first field's cell keeps the line too.
            DecoratedBox(
              position: DecorationPosition.foreground,
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: DefaultTextStyle.merge(
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textMuted,
                ),
                child: line(
                  height: _headerHeight,
                  color: ws.subtle,
                  cell: (index) => Row(
                    children: [
                      Icon(
                        _icon(fields[index].kind),
                        size: 14,
                        color: ws.textFaint,
                      ),
                      const SizedBox(width: WsMetrics.space2),
                      Flexible(child: _text(fields[index].name)),
                    ],
                  ),
                ),
              ),
            ),
            SizedBox(
              height: math.min(count, _shownRows) * WsMetrics.rowTable,
              child: ListView.builder(
                key: const Key('table rows'),
                itemCount: count,
                itemExtent: WsMetrics.rowTable,
                itemBuilder: (context, index) {
                  final row = _rows.at(index);
                  final cells = {
                    for (final cell in row?.cells ?? const <BridgeFieldCell>[])
                      cell.field: cell.value,
                  };
                  return DecoratedBox(
                    position: DecorationPosition.foreground,
                    decoration: BoxDecoration(
                      border: Border(bottom: BorderSide(color: ws.border)),
                    ),
                    child: line(
                      height: WsMetrics.rowTable,
                      color: ws.canvas,
                      cell: (index) {
                        final field = fields[index];
                        return Align(
                          alignment: switch (field.kind) {
                            BridgeFieldKind.number => Alignment.centerRight,
                            BridgeFieldKind.checkbox => Alignment.center,
                            _ => Alignment.centerLeft,
                          },
                          child: content(cells[field.id]),
                        );
                      },
                    ),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  static Widget _text(String text, {TextStyle? style}) =>
      Text(text, maxLines: 1, overflow: TextOverflow.ellipsis, style: style);
}

/// An option as a colored tag; its color number picks one of the ten tag colors, wrapping.
class _Tag extends StatelessWidget {
  const _Tag(this.option);

  final BridgeOption option;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final colors = [
      ws.tag0,
      ws.tag1,
      ws.tag2,
      ws.tag3,
      ws.tag4,
      ws.tag5,
      ws.tag6,
      ws.tag7,
      ws.tag8,
      ws.tag9,
    ];
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: colors[option.color % colors.length],
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        option.name,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontSize: WsMetrics.fsCompact,
          height: 20 / WsMetrics.fsCompact,
          color: ws.tagText,
        ),
      ),
    );
  }
}

class _Checkbox extends StatelessWidget {
  const _Checkbox({required this.checked});

  final bool checked;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      checked: checked,
      child: Container(
        width: 14,
        height: 14,
        decoration: BoxDecoration(
          color: checked ? ws.accent : null,
          borderRadius: BorderRadius.circular(3),
          border: checked
              ? null
              : Border.all(color: ws.borderStrong, width: 1.5),
        ),
        child: checked
            ? Icon(LucideIcons.check, size: 10, color: ws.onAccent)
            : null,
      ),
    );
  }
}
