import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/calculation.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'board_layout.dart';
import 'calendar_layout.dart';
import 'cell_pickers.dart';
import 'explorer.dart' show untitled;
import 'field_menu.dart';
import 'field_panel.dart';
import 'media_block.dart';
import 'row_menu.dart';
import 'row_popover.dart';
import 'stored_name.dart';
import 'table_cells.dart';
import 'table_query.dart';
import 'table_groups.dart';
import 'table_selection.dart';
import 'table_tabs.dart';
import 'roles.dart' show ProtectedMark;
import 'table_view_parts.dart';
import 'total_menu.dart';
import 'follow.dart';
import 'page_views.dart';
import 'paste.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

part 'table_block_data.dart';
part 'table_block_tabs.dart';
part 'table_block_query.dart';
part 'table_block_layouts.dart';
part 'table_block_grid.dart';
part 'table_block_edits.dart';
part 'table_block_title.dart';

const _shownRows = 10;
const _headerHeight = 32.0;
const _footerHeight = 30.0;

const _firstWidth = 200.0;

const _addWidth = 40.0;

class TableBlock extends StatefulWidget {
  const TableBlock({
    super.key,
    required this.views,
    required this.view,
    this.fullPage = false,
  });

  final PageViews views;
  final BridgeView view;

  final bool fullPage;

  @override
  State<TableBlock> createState() => _TableBlockState();
}

class _TableBlockState extends State<TableBlock> implements CellDrop {
  BridgeDatabase? _database;
  late final _rows = WindowedList<BridgeRow>(
    (start, limit) async {
      final sort = widget.fullPage ? _pageSort : _tab.sort;
      final filter = widget.fullPage ? null : _tab.filter;
      final search = _search.isEmpty ? null : _search;
      final grouped = _grouping;
      if (grouped == null) {
        _lines = null;
      } else {
        return _groupedRows(grouped, start, limit, sort, filter, search);
      }
      final answer = await _state.read(
        query(
          BridgeQueryKind.rows,
          block: _block,
          start: start,
          limit: limit,
          sort: sort,
          filter: filter,
          search: search,
        ),
      );
      if (start == 0) {
        _sourcesFollowed.sync(answer.follows);
        await _readTotals(filter, search);
        _allRows = filter == null && search == null
            ? answer.count
            : (await _state.read(
                query(BridgeQueryKind.rows, block: _block, limit: 0),
              )).count;
      }
      return (answer.count, answer.rows);
    },
    _report,
    kept: 5,
  );
  GroupLines? _lines;

  Map<String, BridgeComputed> _totals = const {};
  late final FollowedSources _sourcesFollowed = FollowedSources(
    _state.workspace,
    changed: () => unawaited(_rows.reread()),
    failed: _report,
  );
  final _totalMenus = <String, OverlayPortalController>{};
  var _footerHovered = false;

  int _revision = 0;

  var _month = _thisMonth();

  final _sideways = ScrollController();
  final _down = ScrollController();

  int? _allRows;

  final _panel = OverlayPortalController();
  final _plus = LayerLink();

  var _rowAdded = false;
  int _rowsWriting = 0;
  int _readsAtRowAdd = 0;

  String? _fieldAdded;

  int _readsAtFieldAdd = 0;

  late final _cells = CellSelection(
    rowAt: _rows.at,
    rowCount: () => _rows.count ?? 0,
    fields: () => _database?.fields ?? const [],
    write: _setCell,
    reveal: _reveal,
    leave: _leave,
    report: (problem) => scheduleMicrotask(() => _state.report(problem)),
    addOption: _addOption,
    attach: (sources) => attachAll(_state, sources),
    save: (file) => saveCopy(_state, file),
  )..addListener(_changed);

  final _rowsKey = GlobalKey(debugLabel: 'Table rows');

  (int, int)? _dropAt;

  @override
  bool hover(Offset global) {
    final at = _fileCellAt(global);
    if (at != _dropAt) setState(() => _dropAt = at);
    return at != null;
  }

  @override
  void leave() {
    if (_dropAt != null) setState(() => _dropAt = null);
  }

  @override
  Future<void> drop(List<MediaSource> sources, Offset global) async {
    final at = _fileCellAt(global);
    leave();
    if (at case (final row, final field)) {
      await _cells.addFiles(row, field, sources);
    }
  }

  List<double> _starts = const [0];
  double _first = 0;

  int _panelOpened = 0;

  int _fieldsWriting = 0;

  var _toLastRow = false;
  var _toLastField = false;
  StreamSubscription<BridgeChange>? _followed;

  late final _name = StoredName(
    widget.view.title,
    rename: (title) => widget.views.write(
      mutation(
        BridgeMutationKind.renameView,
        page: _page,
        view: widget.view.id,
        text: title,
      ),
      [widget.view.id],
    ),
    restore: () async => mounted ? widget.view.title : null,
    quitting: () => _state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  final _nameFocus = FocusNode(debugLabel: 'View title');

  String _search = '';
  bool _searching = false;
  final _searchText = TextEditingController();
  final _searchFocus = FocusNode(debugLabel: 'Table search');
  Timer? _searchSettle;

  BridgeSort? _pageSort;

  bool _read1 = false;

  bool _hovered = false;
  bool _headerHovered = false;

  StreamSubscription<String>? _settled;

  int _reads = 0;

  String? _tabId;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  void _update(VoidCallback change) => setState(change);

  void _changed() {
    if (!mounted) return;
    int? rowIndex(String id) => _rows.indexWhere((row) => row.id == id);
    _cells
      ..relocate(rowIndex)
      ..read(rowIndex);
    _rowHandles.read();
    _update(() {});
  }

  void _moved() {
    if (_scrollingItself) return;
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
    _rowHandles.closeMenu();
    _cells.closePick();
  }

  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null || caret.view != widget.view.id) return;
    widget.views.caret.value = null;
    final (text, written) =
        widget.views.naming.remove(widget.view.id)?.call() ?? ('', false);
    if (text.isNotEmpty) _name.handOver(text, written: written);
    _nameFocus.requestFocus();
  }

  @override
  void initState() {
    super.initState();
    _rows.addListener(_changed);
    widget.views.cellDrops[widget.view.id] = this;
    _sideways
      ..addListener(_closePanel)
      ..addListener(_scrolled);
    _down.addListener(_scrolled);
    _state.finishing.add(_name.typed);
    _state.finishing.add(_cells.finish);
    _state.finishing.add(_fieldMenus.renameTyped);
    _nameFocus.addListener(_left);
    _settled = widget.views.settled
        .where((settled) => settled == _block)
        .listen((_) => unawaited(_read()));
    widget.views.caret.addListener(_takeCaret);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _takeCaret();
    });
    unawaited(_recall());
    _followed = follow(
      _state.workspace,
      BridgeResource(kind: BridgeResourceKind.table, block: _block),
    ).listen((_) => unawaited(_read()), onError: _report);
  }

  @override
  void didUpdateWidget(TableBlock old) {
    super.didUpdateWidget(old);
    if (widget.view.title != old.view.title &&
        !_nameFocus.hasFocus &&
        widget.views.pending(widget.view.id) == 0) {
      _name.show(widget.view.title);
    }
    final before = _tabIn(old.view);
    if (!sameTotals(before.totals, _tab.totals)) unawaited(_totalsChanged());
    if (sameSetup(before, _tab)) return;
    _tabChanged();
  }

  ScrollPosition? _pageScroll;
  Size? _window;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final window = MediaQuery.sizeOf(context);
    if (_window != null && window != _window) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _moved();
      });
    }
    _window = window;
    final scroll = Scrollable.maybeOf(context)?.position;
    if (scroll == _pageScroll) return;
    _pageScroll?.removeListener(_moved);
    _pageScroll = scroll?..addListener(_moved);
  }

  @override
  void dispose() {
    _sourcesFollowed.dispose();
    if (widget.views.cellDrops[widget.view.id] == this) {
      widget.views.cellDrops.remove(widget.view.id);
    }
    _pageScroll?.removeListener(_moved);
    _state.finishing.remove(_name.typed);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    _rows.dispose();
    _sideways.dispose();
    _down.dispose();
    _name.dispose();
    _nameFocus.dispose();
    _searchSettle?.cancel();
    _searchText.dispose();
    _searchFocus.dispose();
    _fieldMenus.renameTyped();
    _state.finishing.remove(_fieldMenus.renameTyped);
    _fieldMenus.dispose();
    _rowHandles.dispose();
    _cells.removeListener(_changed);
    _cells.finish();
    _state.finishing.remove(_cells.finish);
    _cells.dispose();
    super.dispose();
  }

  var _scrollingItself = false;

  late final _rowHandles = RowHandles(
    indexOf: (row) => _rows.indexWhere((shown) => shown.id == row),
    idAt: (index) async => (await _rows.load(index))?.id,
    insert: _insertRow,
    remove: (row) => _write(
      mutation(
        BridgeMutationKind.removeRow,
        page: _page,
        block: _block,
        row: row,
      ),
    ),
  )..addListener(_menusChanged);

  late final _fieldMenus = FieldMenus(
    fieldOf: (id) =>
        _database?.fields.where((field) => field.id == id).firstOrNull,
    fields: () => _database?.fields ?? const [],
    table: () => (_block, _database?.name ?? ''),
    formula: (field, formula) => _writeField(
      mutation(
        BridgeMutationKind.setFieldFormula,
        page: _page,
        block: _block,
        field: field,
        text: formula,
      ),
    ),
    rename: (field, name) => unawaited(
      _writeField(
        mutation(
          BridgeMutationKind.renameField,
          page: _page,
          block: _block,
          field: field,
          text: name,
        ),
      ),
    ),
    remove: (field) => _writeField(
      mutation(
        BridgeMutationKind.removeField,
        page: _page,
        block: _block,
        field: field,
      ),
    ),
    confirm: (field) => confirmDeleteField(context, field, _allRows),
  )..addListener(_menusChanged);

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final database = _database;
    final locale = Localizations.localeOf(context);
    final numberFormat = NumberFormat.decimalPattern(
      intlLocale(locale, NumberFormat.localeExists),
    );
    _cells.numbers = numberFormat;
    final days = intlLocale(locale, DateFormat.localeExists);
    _cells.days = days;
    final count = _rows.count ?? 0;
    final lineCount = _lines?.lineCount ?? count;
    final sort = _liveSort;
    final filter = _liveFilter;
    if (_toLastRow || _toLastField) {
      final (down, sideways) = (_toLastRow, _toLastField);
      _toLastRow = _toLastField = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        if (down && _down.hasClients) {
          _silently(
            () => _down.jumpTo(
              (lineCount - math.min(lineCount, _shownRows)) *
                  WsMetrics.rowTable,
            ),
          );
        }
        if (sideways && _sideways.hasClients) {
          _silently(() => _sideways.jumpTo(_sideways.position.maxScrollExtent));
        }
      });
    }
    return OverlayPortal(
      controller: _rowHandles.handle,
      overlayChildBuilder: _rowHandles.handleView,
      child: OverlayPortal(
        controller: _rowHandles.menu,
        overlayChildBuilder: _rowHandles.menuView,
        child: FieldPanelPortal(
          panel: _panel,
          plus: _plus,
          onAdd: _addField,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: WsMetrics.space3),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (widget.fullPage)
                  Padding(
                    padding: const EdgeInsets.only(bottom: WsMetrics.space3),
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: SizedBox(
                        width: 260,
                        child: _searchField(boxed: true),
                      ),
                    ),
                  )
                else ...[
                  _titleLine(database),
                  _tabStrip(),
                  if (database != null && database.fields.isNotEmpty)
                    _setupLine(database, sort, filter, numberFormat, days),
                  if (database == null && _read1)
                    TablePlaceholder(
                      _state.tables.value.any((table) => table.id == _block)
                          ? 'Liquid could not read this Table.'
                          : 'This Table was deleted.',
                    ),
                ],
                if (database != null &&
                    !widget.fullPage &&
                    _tab.layout == BridgeLayout.board)
                  BoardLayout(
                    data: _data(database, numberFormat, days),
                    tab: _tab,
                    group: _groupOf(database),
                    search: _search.isEmpty ? null : _search,
                    revision: _revision,
                    onAddGroup: () => unawaited(_addGroupField()),
                  )
                else if (database != null &&
                    !widget.fullPage &&
                    _tab.layout == BridgeLayout.calendar)
                  CalendarLayout(
                    data: _data(database, numberFormat, days),
                    tab: _tab,
                    date: _dateOf(database),
                    month: _month,
                    revision: _revision,
                    onAddDate: () => unawaited(_addDateField()),
                  )
                else if (database != null)
                  Focus(
                    focusNode: _cells.cellsFocus,
                    includeSemantics: false,
                    child: ClipRRect(
                      borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                      child: Container(
                        foregroundDecoration: BoxDecoration(
                          border: Border.all(color: ws.border),
                          borderRadius: BorderRadius.circular(
                            WsMetrics.radiusLg,
                          ),
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            if (database.fields.isEmpty)
                              Container(
                                height: _headerHeight,
                                color: ws.subtle,
                                alignment: Alignment.centerRight,
                                child: SizedBox(
                                  width: _addWidth,
                                  child: AddField(
                                    plus: _plus,
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
                                  numberFormat,
                                  shownDays(days),
                                  footer: count > 0 || filter != null,
                                  counted: _rowCount(
                                    numberFormat,
                                    _lines?.total ?? count,
                                    filter,
                                  ),
                                ),
                              ),
                            if (database.fields.isEmpty)
                              _NewRow(onTap: () => unawaited(_addRow())),
                            if (database.fields.isEmpty &&
                                (count > 0 || filter != null))
                              Container(
                                height: _footerHeight,
                                color: ws.subtle,
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 10,
                                ),
                                alignment: Alignment.centerLeft,
                                child: Text(
                                  _rowCount(
                                    numberFormat,
                                    _lines?.total ?? count,
                                    filter,
                                  ),
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
                  ),
                _pastedChip(),
                if (_cells.error case final error?)
                  Padding(
                    padding: const EdgeInsets.only(top: WsMetrics.space2),
                    child: Text(
                      error,
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.danger,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
