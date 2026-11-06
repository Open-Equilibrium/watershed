import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'field_panel.dart';
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

/// The column after the last field, with + for a new one.
const _addWidth = 40.0;

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
  final _down = ScrollController();

  /// The field panel + opens, under that +.
  final _panel = OverlayPortalController();
  final _plus = LayerLink();

  /// What was just added, to scroll to once it shows: rows beyond this count, a field.
  int? _toNewRow;

  /// The field the last add made, the last until a read holds it, then shown.
  String? _fieldAdded;

  /// Field adds, one after another, so each goes after the one before it.
  Future<void> _fieldAdds = Future.value();

  /// Times the field panel opened, so an add closes only the panel it came from.
  int _panelOpened = 0;

  /// + New's adds, one after another. The row one added is the last until a read counts
  /// it, so the next goes after it.
  Future<void> _rowAdds = Future.value();
  String? _rowAdded;
  int _rowAddedAt = 0;

  /// The reads started when the last row add returned; any later one holds that row.
  int _readsAtRowAdd = 0;

  /// Row adds asked for and not yet returned.
  int _rowsAdding = 0;

  /// Whether to scroll to the last row or field once the next build laid it out.
  var _toLastRow = false;
  var _toLastField = false;
  StreamSubscription<BridgeChange>? _followed;

  /// The name as typed; a name changed elsewhere shows unless the person is typing one.
  final _name = TextEditingController();
  final _nameFocus = FocusNode(debugLabel: 'Table name');

  /// Renames asked for, so only the latest refusal puts the stored name back.
  int _renames = 0;

  /// The name as last read or written, which typing changes once a composition ends.
  String _named = '';

  /// The renames sent when the name shown last was read.
  int _readSent = 0;
  StreamSubscription<String>? _settled;

  /// Reads asked for, so only the latest shows.
  int _reads = 0;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _rows.addListener(_changed);
    // A sideways scroll moves + away from the field panel, which closes.
    _sideways.addListener(_closePanel);
    _name.addListener(_typed);
    _state.finishing.add(_typed);
    _nameFocus.addListener(_left);
    // Once every rename arrived, the name is read again.
    _settled = widget.views.settled
        .where((settled) => settled == _block)
        .listen((_) => unawaited(_read()));
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
    unawaited(_settled?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    _rows.dispose();
    _sideways.dispose();
    _down.dispose();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _closePanel() {
    if (_panel.isShowing && !_showingField) _panel.hide();
  }

  /// While the Table scrolls to a new field itself, which moves + along with the panel.
  var _showingField = false;

  void _changed() {
    if (!mounted) return;
    setState(() {});
    if (_rowAdded != null && (_rows.count ?? 0) >= _rowAddedAt) {
      _rowAdded = null;
    }
    if (_toNewRow case final before? when (_rows.count ?? 0) > before) {
      _toNewRow = null;
      _toLastRow = true;
    }
  }

  /// + New adds a row after the last one and shows it; the last row is read first if its
  /// window was not.
  void _addRow() {
    _rowsAdding++;
    _rowAdds = _rowAdds.then((_) => _addOneRow());
  }

  Future<void> _addOneRow() async {
    try {
      // Rows not yet counted are read first, so the row goes after the last.
      if (_rows.count == null) await _rows.load(0);
      final count = _rows.count;
      if (count == null) throw 'The Table\'s rows are not read yet; try again.';
      var after = _rowAdded;
      if (after == null && count > 0) {
        after = (await _rows.load(count - 1))?.id;
        if (after == null) throw 'The Table changed; try again.';
      }
      // The rows a read must count to hold this one too, and to show it.
      final at = (_rowAdded == null ? count : _rowAddedAt) + 1;
      _toNewRow = at - 1;
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          after: after,
        ),
        const [],
      );
      _rowAdded = added.row;
      _rowAddedAt = at;
      _readsAtRowAdd = _reads;
    } on Object catch (error) {
      _toNewRow = null;
      _rowAdded = null;
      _report(error);
    } finally {
      _rowsAdding--;
    }
  }

  void _togglePanel() {
    if (_panel.isShowing) return _panel.hide();
    _panelOpened++;
    _panel.show();
  }

  /// Adds a field after the last one, once the adds before it returned, and shows it
  /// once a read holds it; a refused one keeps the panel open.
  Future<void> _addField(String name, BridgeFieldKind kind) {
    final opened = _panelOpened;
    final add = _fieldAdds.then((_) => _addOneField(name, kind, opened));
    _fieldAdds = add.then<void>((_) {}, onError: (Object _) {});
    return add;
  }

  Future<void> _addOneField(
    String name,
    BridgeFieldKind kind,
    int opened,
  ) async {
    final BridgeApplied added;
    try {
      added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: name,
          fieldKind: kind,
          after: _fieldAdded ?? _database?.fields.lastOrNull?.id,
        ),
        const [],
      );
    } on Object {
      // The field it went after may be gone; the next goes after the last one read.
      _fieldAdded = null;
      rethrow;
    }
    if (!mounted) return;
    if (opened == _panelOpened) _panel.hide();
    _fieldAdded = added.field;
    if (_database?.fields.any((field) => field.id == added.field) ?? false) {
      _fieldAdded = null;
      setState(() => _toLastField = true);
    }
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// A caret handed to this Table goes to its name, as for a new Table.
  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null || caret.view != widget.view.id) return;
    widget.views.caret.value = null;
    // What was typed in the line it took the place of is its name.
    final (text, written) =
        widget.views.naming.remove(widget.view.id)?.call() ?? ('', false);
    if (text.isNotEmpty) {
      if (written) _named = text;
      _name.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      );
    }
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

  /// Leaving the name writes a composition still open as it stands; a name changed
  /// elsewhere meanwhile then shows, unless the person's own rename is on its way.
  void _left() {
    if (_nameFocus.hasFocus) return;
    _write();
    final views = widget.views;
    if (views.pending(_block) > 0 || views.sent(_block) != _readSent) return;
    _show(_database?.name ?? '');
  }

  void _show(String name) {
    _named = name;
    if (_name.text != name) _name.text = name;
  }

  /// Reads the name and fields, then the rows read so far.
  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    // A read that crosses a rename on its way may miss it, whenever it answers.
    final clean = views.pending(_block) == 0;
    final sent = views.sent(_block);
    try {
      final answer = await _state.workspace.query(
        query: query(BridgeQueryKind.database, page: _page, block: _block),
      );
      if (!mounted || read != _reads) return;
      if (_fieldAdded case final field?
          when answer.database?.fields.any((shown) => shown.id == field) ??
              false) {
        _fieldAdded = null;
        _toLastField = true;
      }
      if (clean && views.pending(_block) == 0 && sent == views.sent(_block)) {
        _readSent = sent;
        if (!_nameFocus.hasFocus) _show(answer.database?.name ?? '');
      }
      setState(() => _database = answer.database);
    } on Object catch (error) {
      _report(error);
      return;
    }
    final answered = await _rows.reread();
    // Counts may not tell when rows also go elsewhere; a read begun after the last add
    // holds its row, if it answered.
    if (answered && mounted && _rowAdded != null && read > _readsAtRowAdd) {
      _rowAdded = null;
      // The scroll waits for an add still on its way.
      if (_toNewRow != null && _rowsAdding == 0) {
        _toNewRow = null;
        setState(() => _toLastRow = true);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final database = _database;
    final locale = Localizations.localeOf(context);
    final numbers = _formats(locale, NumberFormat.localeExists);
    final days = _formats(locale, DateFormat.localeExists);
    final count = _rows.count ?? 0;
    if (_toLastRow || _toLastField) {
      final (down, sideways) = (_toLastRow, _toLastField);
      _toLastRow = _toLastField = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        // The rows' end follows from their fixed height.
        if (down && _down.hasClients) {
          _down.jumpTo(
            (count - math.min(count, _shownRows)) * WsMetrics.rowTable,
          );
        }
        if (sideways && _sideways.hasClients) {
          _showingField = true;
          _sideways.jumpTo(_sideways.position.maxScrollExtent);
          _showingField = false;
        }
      });
    }
    // The field panel opens under +, and clicking it or + keeps it open.
    return OverlayPortal(
      controller: _panel,
      overlayChildBuilder: (context) => Align(
        alignment: Alignment.topLeft,
        child: CompositedTransformFollower(
          link: _plus,
          showWhenUnlinked: false,
          targetAnchor: Alignment.bottomRight,
          followerAnchor: Alignment.topRight,
          offset: const Offset(0, WsMetrics.space1),
          child: TapRegion(
            groupId: _panel,
            onTapOutside: (_) => _panel.hide(),
            child: FieldPanel(onAdd: _addField, onClose: _panel.hide),
          ),
        ),
      ),
      child: Padding(
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
                      if (database.fields.isEmpty)
                        Container(
                          height: _headerHeight,
                          color: ws.subtle,
                          alignment: Alignment.centerRight,
                          child: SizedBox(
                            width: _addWidth,
                            child: _AddField(
                              link: _plus,
                              panel: _panel,
                              onTap: _togglePanel,
                            ),
                          ),
                        )
                      else
                        LayoutBuilder(
                          builder: (context, constraints) => _grid(
                            database.fields,
                            constraints.maxWidth,
                            NumberFormat.decimalPattern(numbers),
                            DateFormat.yMMMd(days),
                          ),
                        ),
                      _NewRow(onTap: _addRow),
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
    final rest = [
      for (final field in fields.skip(1)) _width(field.kind),
      _addWidth,
    ];
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
                  cell: (index) => index == fields.length
                      ? _AddField(
                          link: _plus,
                          panel: _panel,
                          onTap: _togglePanel,
                        )
                      : Row(
                          children: [
                            Icon(
                              fieldKindIcon(fields[index].kind),
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
                controller: _down,
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
                        if (index == fields.length) {
                          return const SizedBox.shrink();
                        }
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

/// + after the last field, which opens the field panel.
class _AddField extends StatelessWidget {
  const _AddField({
    required this.link,
    required this.panel,
    required this.onTap,
  });

  final LayerLink link;
  final OverlayPortalController panel;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => CompositedTransformTarget(
    link: link,
    child: TapRegion(
      groupId: panel,
      child: GestureDetector(
        key: const Key('add field'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Center(
          child: Icon(LucideIcons.plus, size: 14, color: context.ws.textMuted),
        ),
      ),
    ),
  );
}

/// + New under the last row, which adds a row there.
class _NewRow extends StatelessWidget {
  const _NewRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: 10),
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
    );
  }
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
