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
import 'windowed_list.dart';
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

/// `number` with every digit Dart's shortest form keeps, written out in `format`'s digits,
/// separators and grouping; rounding comes with field formats (ADR-0242). intl itself
/// keeps at most 18 decimals and 18 grouped digits exactly.
String _number(NumberFormat format, double number) {
  // The core refuses these, but a document it did not write may hold them.
  if (!number.isFinite) return format.format(number);
  final [mantissa, ...exponent] = number.abs().toString().split('e');
  final [whole, ...fraction] = mantissa.split('.');
  var digits = whole + fraction.join();
  var point =
      whole.length + (exponent.isEmpty ? 0 : int.parse(exponent.single));
  if (point <= 0) {
    digits = '0' * (1 - point) + digits;
    point = 1;
  }
  digits = digits.padRight(point, '0');
  final integer = digits.substring(0, point);
  final decimals = digits.substring(point).replaceFirst(RegExp(r'0+$'), '');
  final zero = format.symbols.ZERO_DIGIT.codeUnitAt(0);
  String local(String digits) =>
      String.fromCharCodes(digits.codeUnits.map((unit) => unit - 0x30 + zero));
  final text = [
    if (integer.length <= 18)
      format.format(int.parse(integer))
    else
      local(integer),
    if (decimals.isNotEmpty) ...[format.symbols.DECIMAL_SEP, local(decimals)],
  ].join();
  return number < 0
      ? '${format.negativePrefix}$text${format.negativeSuffix}'
      : text;
}

/// The most specific form of `locale` intl has formats for, keeping its script as in
/// Serbian written in Latin, or US English where intl has none for a language Material
/// knows.
String _formats(Locale locale, bool Function(String) exists) {
  final Locale(:languageCode, :scriptCode, :countryCode) = locale;
  return [
        locale.toString(),
        if (scriptCode != null) '${languageCode}_$scriptCode',
        if (countryCode != null) '${languageCode}_$countryCode',
        languageCode,
      ].where(exists).firstOrNull ??
      'en_US';
}

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
  late final _rows = WindowedList<BridgeRow>(
    (start, limit) async {
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
    },
    _report,
    kept: 5,
  );
  final _sideways = ScrollController();
  StreamSubscription<BridgeChange>? _followed;

  /// The name as typed; a name changed elsewhere shows unless the person is typing one.
  final _name = TextEditingController();
  final _nameFocus = FocusNode(debugLabel: 'Table name');

  /// Renames asked for, so only the latest refusal puts the stored name back.
  int _renames = 0;

  /// The name as last read or written, which typing changes once a composition ends.
  String _named = '';

  /// Reads asked for, so only the latest shows.
  int _reads = 0;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _rows.addListener(_changed);
    _name.addListener(_typed);
    _state.finishing.add(_typed);
    _nameFocus.addListener(_left);
    widget.views.caret.addListener(_takeCaret);
    // Built after the caret was handed to it, as a new Table is.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _takeCaret();
    });
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
    // A composition still open is written as it stands.
    _write();
    _state.finishing.remove(_typed);
    unawaited(_followed?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    _rows.dispose();
    _sideways.dispose();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// A caret handed to this Table goes to its name, as for a new Table.
  void _takeCaret() {
    if (widget.views.caret.value?.view != widget.view.id) return;
    widget.views.caret.value = null;
    _nameFocus.requestFocus();
  }

  /// Each change of the name, once any composition ends, is one rename; a refused one puts
  /// the stored name back.
  void _typed() {
    // On quitting, a composition still open is written as it stands.
    if (!_name.value.composing.isCollapsed && !_state.quitting) return;
    _write();
  }

  void _write() {
    final name = _name.text;
    if (name == _named) return;
    _named = name;
    final rename = ++_renames;
    unawaited(
      widget.views
          .write(
            mutation(
              BridgeMutationKind.renameDatabase,
              page: _page,
              block: _block,
              text: name,
            ),
            [_block],
          )
          .then<void>(
            (_) {},
            onError: (Object _) {
              if (!mounted || rename != _renames) return;
              _show(_database?.name ?? '');
            },
          ),
    );
  }

  /// A name changed elsewhere while the person typed shows once they leave, unless their
  /// own rename is still on its way.
  void _left() {
    if (_nameFocus.hasFocus || widget.views.pending(_block) > 0) return;
    _show(_database?.name ?? '');
  }

  void _show(String name) {
    _named = name;
    if (_name.text != name) _name.text = name;
  }

  /// Reads the name and fields, then the rows read so far.
  Future<void> _read() async {
    final read = ++_reads;
    try {
      final answer = await _state.workspace.query(
        query: query(BridgeQueryKind.database, page: _page, block: _block),
      );
      if (!mounted || read != _reads) return;
      if (!_nameFocus.hasFocus) _show(answer.database?.name ?? '');
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
    final locale = Localizations.localeOf(context);
    final numbers = _formats(locale, NumberFormat.localeExists);
    final days = _formats(locale, DateFormat.localeExists);
    final count = _rows.count ?? 0;
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
                Expanded(
                  child: TextField(
                    controller: _name,
                    focusNode: _nameFocus,
                    style: TextStyle(
                      fontSize: WsMetrics.fsProse,
                      fontWeight: WsMetrics.fwSemibold,
                      color: ws.text,
                    ),
                    decoration: InputDecoration.collapsed(
                      hintText: 'Untitled',
                      hintStyle: TextStyle(color: ws.textFaint),
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
                    // A Table may lose all its fields, as when another device removes
                    // them; its rows still count.
                    if (database.fields.isNotEmpty)
                      LayoutBuilder(
                        builder: (context, constraints) => _grid(
                          database.fields,
                          constraints.maxWidth,
                          NumberFormat.decimalPattern(numbers),
                          DateFormat.yMMMd(days),
                        ),
                      ),
                    if (count > 0)
                      Container(
                        height: _footerHeight,
                        color: ws.subtle,
                        padding: const EdgeInsets.symmetric(horizontal: 10),
                        alignment: Alignment.centerLeft,
                        child: Text(
                          '${NumberFormat.decimalPattern(numbers).format(count)} '
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
  Widget _grid(
    List<BridgeField> fields,
    double width,
    NumberFormat numbers,
    DateFormat days,
  ) {
    final ws = context.ws;
    final rest = [for (final field in fields.skip(1)) _width(field.kind)];
    final others = rest.fold(0.0, (sum, width) => sum + width);
    final first = math.max(_firstWidth, width - others);
    final widths = [first, ...rest];
    // Where each field starts, to build only the fields in view (client architecture
    // rule 8).
    final starts = [0.0];
    for (final width in widths) {
      starts.add(starts.last + width);
    }
    final count = _rows.count ?? 0;
    final options = {
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select)
          for (final option in field.options) option.id: option,
    };

    Widget content(BridgeCell? cell) => switch (cell?.kind) {
      null || BridgeCellKind.empty => const SizedBox.shrink(),
      BridgeCellKind.text => _text(cell!.text ?? ''),
      BridgeCellKind.number => _text(
        _number(numbers, cell!.number),
        style: const TextStyle(fontFeatures: [FontFeature.tabularFigures()]),
      ),
      BridgeCellKind.select => switch (options[cell!.text]) {
        final option? => _Tag(option),
        null => const SizedBox.shrink(),
      },
      BridgeCellKind.date => _text(switch (DateTime.tryParse(
        cell!.text ?? '',
      )) {
        final day? => days.format(day),
        null => cell.text ?? '',
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
        // The first field stays in view while the others scroll sideways; fields out of
        // view are not built.
        var from = 1;
        while (from < widths.length && starts[from + 1] <= offset + first) {
          from++;
        }
        var to = from;
        while (to < widths.length && starts[to] < offset + width) {
          to++;
        }
        return Stack(
          children: [
            Row(
              children: [
                // Also gives the line its height when only the first field is built.
                SizedBox(width: starts[from], height: height),
                for (var index = from; index < to; index++) box(index),
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
