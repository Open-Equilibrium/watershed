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
import 'table_view_parts.dart';
import 'total_menu.dart';
import 'follow.dart';
import 'page_views.dart';
import 'paste.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

const _shownRows = 10;
const _headerHeight = 32.0;
const _footerHeight = 30.0;

const _firstWidth = 200.0;

const _addWidth = 40.0;

double _width(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 180,
  BridgeFieldKind.number => 104,
  BridgeFieldKind.select => 128,
  BridgeFieldKind.date => 120,
  BridgeFieldKind.checkbox => 72,
  BridgeFieldKind.file => 180,
  BridgeFieldKind.formula => 120,
};

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

  Future<void> _readTotals(BridgeFilter? filter, String? search) async {
    final totals = _tab.totals;
    final answer = totals.isEmpty
        ? null
        : await _state.read(
            query(
              BridgeQueryKind.rows,
              block: _block,
              limit: 0,
              filter: filter,
              search: search,
              totals: totals,
            ),
          );
    _totals = {
      for (final total in answer?.totals ?? const []) total.field: total,
    };
  }

  Future<void> _totalsChanged() async {
    final filter = widget.fullPage ? null : _tab.filter;
    try {
      await _readTotals(filter, _search.isEmpty ? null : _search);
    } on Object catch (error) {
      _report(error);
    }
    if (mounted) setState(() {});
  }

  void _setTotal(BridgeField field, BridgeTotal? total) => _setQuery(
    mutation(
      BridgeMutationKind.setTabTotal,
      page: _page,
      view: widget.view.id,
      tab: _tab.id,
      field: field.id,
      total: total,
    ),
  );

  Widget _totalCell(BridgeField field, NumberFormat numbers) {
    final ws = context.ws;
    final set = _tab.totals
        .where((total) => total.field == field.id)
        .firstOrNull;
    final value = _totals[field.id];
    final menu = _totalMenus.putIfAbsent(field.id, OverlayPortalController.new);
    final numeric =
        field.kind == BridgeFieldKind.number ||
        field.kind == BridgeFieldKind.formula;
    final faint = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    return AnchoredMenu(
      controller: menu,
      onTapOutside: (_) => menu.hide(),
      menu: (_) => TotalMenu(
        numeric: numeric,
        chosen: set?.total,
        onPick: (total) {
          menu.hide();
          _setTotal(field, total);
        },
      ),
      child: GestureDetector(
        key: ValueKey('total ${field.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => setState(menu.toggle),
        child: Align(
          alignment: Alignment.centerRight,
          child: set == null
              ? Opacity(
                  opacity: _footerHovered || touchFirst ? 1 : 0,
                  child: Text(
                    'Calculate',
                    style: faint,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                )
              : Text.rich(
                  TextSpan(
                    children: [
                      TextSpan(text: '${totalName(set.total)} ', style: faint),
                      TextSpan(
                        text: switch (value) {
                          null => '',
                          BridgeComputed(kind: BridgeValueKind.number) =>
                            numberText(numbers, value.number),
                          _ => value.value,
                        },
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.text,
                          fontFeatures: const [FontFeature.tabularFigures()],
                        ),
                      ),
                    ],
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
        ),
      ),
    );
  }

  int _revision = 0;

  var _month = DateTime(DateTime.now().year, DateTime.now().month);

  void _shiftMonth(int by) =>
      setState(() => _month = DateTime(_month.year, _month.month + by));

  Future<void> _addDateField() async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: 'Date',
          fieldKind: BridgeFieldKind.date,
          last: true,
        ),
        const [],
      );
      _setDate(added.field);
    } on Object {
      return;
    }
  }

  Widget _monthControls(String locale) {
    final ws = context.ws;
    Widget arrow(Key key, IconData icon, String label, int by) => Semantics(
      button: true,
      label: label,
      child: GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        onTap: () => _shiftMonth(by),
        child: Padding(
          padding: const EdgeInsets.all(WsMetrics.space1),
          child: Icon(icon, size: 14, color: ws.textMuted),
        ),
      ),
    );
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          DateFormat.yMMMM(locale).format(_month),
          style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
        ),
        const SizedBox(width: WsMetrics.space2),
        arrow(
          const Key('previous month'),
          LucideIcons.chevronLeft,
          'Previous month',
          -1,
        ),
        GestureDetector(
          key: const Key('this month'),
          behavior: HitTestBehavior.opaque,
          onTap: () => setState(
            () => _month = DateTime(DateTime.now().year, DateTime.now().month),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
            child: Text(
              'Today',
              style: TextStyle(
                fontSize: WsMetrics.fsCompact,
                color: ws.textMuted,
              ),
            ),
          ),
        ),
        arrow(
          const Key('next month'),
          LucideIcons.chevronRight,
          'Next month',
          1,
        ),
      ],
    );
  }

  TableData _data(BridgeDatabase database, NumberFormat numbers, String days) =>
      TableData(
        page: _page,
        block: _block,
        database: database,
        read: _state.read,
        write: (change) => widget.views.write(change, const []),
        numbers: numbers,
        days: days,
      );

  Future<void> _addGroupField() async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: 'Status',
          fieldKind: BridgeFieldKind.select,
          last: true,
        ),
        const [],
      );
      for (final (color, name) in [
        'Not started',
        'In progress',
        'Done',
      ].indexed) {
        await widget.views.write(
          mutation(
            BridgeMutationKind.addOption,
            page: _page,
            block: _block,
            field: added.field,
            text: name,
            color: color,
          ),
          const [],
        );
      }
      _setGroup(added.field);
    } on Object {
      return;
    }
  }

  BridgeField? get _grouping => switch (_database) {
    final database?
        when !widget.fullPage && _tab.layout == BridgeLayout.table =>
      _groupOf(database),
    _ => null,
  };

  Future<(int, List<BridgeRow>)> _groupedRows(
    BridgeField grouped,
    int start,
    int limit,
    BridgeSort? sort,
    BridgeFilter? filter,
    String? search,
  ) async {
    var lines = _lines;
    if (start == 0 || lines == null) {
      final groups = (await _state.read(
        query(
          BridgeQueryKind.groups,
          block: _block,
          limit: 0,
          field: grouped.id,
          filter: filter,
          search: search,
        ),
      )).groups;
      final tab = _tab;
      await _readTotals(filter, search);
      lines = _lines = GroupLines(groups, (value) => folds(tab, value));
      _allRows = filter == null && search == null
          ? lines.total
          : (await _state.read(
              query(BridgeQueryKind.rows, block: _block, limit: 0),
            )).count;
    }
    final rows = <BridgeRow>[];
    final follows = <BridgeResource>[];
    for (final part in lines.within(start, limit)) {
      final answer = await _state.read(
        query(
          BridgeQueryKind.rows,
          block: _block,
          start: part.start,
          limit: part.count,
          sort: sort,
          filter: filter,
          within: groupFilter(grouped, part.group.value),
          search: search,
        ),
      );
      rows.addAll(answer.rows);
      follows.addAll(answer.follows);
    }
    if (start == 0) _sourcesFollowed.sync(follows);
    return (lines.rowCount, rows);
  }

  void _toggleFold(BridgeCell value) => _setQuery(
    _tabWrite(
      BridgeMutationKind.setTabFolded,
      tabs: [copiedTab(_tab, folded: foldedToggled(_tab, value))],
    ),
  );

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

  (int, int)? _fileCellAt(Offset global) {
    final box = _rowsKey.currentContext?.findRenderObject();
    final fields = _database?.fields ?? const <BridgeField>[];
    if (box is! RenderBox || !box.hasSize) return null;
    final local = box.globalToLocal(global);
    if (!(Offset.zero & box.size).contains(local)) return null;
    final line = ((local.dy + _down.offset) / WsMetrics.rowTable).floor();
    final row = _lines == null ? line : _lines!.at(line).row;
    if (row == null || row < 0 || row >= (_rows.count ?? 0)) return null;
    final offset = _sideways.hasClients ? _sideways.offset : 0.0;
    var field = 0;
    if (local.dx - offset >= _first) {
      field = _starts.lastIndexWhere((start) => start <= local.dx);
    }
    if (field < 0 || field >= fields.length) return null;
    return fields[field].kind == BridgeFieldKind.file ? (row, field) : null;
  }

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

  String _rowCount(NumberFormat format, int count, BridgeFilter? filter) {
    final all = _allRows ?? count;
    return filter == null && _search.isEmpty
        ? '${format.format(count)} ${count == 1 ? 'row' : 'rows'}'
        : '${format.format(count)} of ${format.format(all)} '
              '${all == 1 ? 'row' : 'rows'}';
  }

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

  BridgeTab get _tab => _tabIn(widget.view);

  BridgeTab _tabIn(BridgeView view) =>
      view.tabs.where((tab) => tab.id == _tabId).firstOrNull ??
      view.tabs.firstOrNull ??
      tabOfLayout(BridgeLayout.table);

  String get _remembered => 'tab/${widget.view.id}';

  Future<void> _recall() async {
    try {
      final id = await _state.workspace.remembered(key: _remembered);
      if (!mounted || id == null || _tabId != null) return;
      if (widget.view.tabs.any((tab) => tab.id == id)) _choose(id);
    } on Object {
      return;
    }
  }

  void _choose(String id) {
    if (id == _tab.id && _tabId != null) return;
    final before = _tab;
    setState(() => _tabId = id);
    if (!sameSetup(before, _tab)) _tabChanged();
  }

  void _select(String id) {
    _choose(id);
    unawaited(
      _state.workspace
          .remember(key: _remembered, value: id)
          .then((_) {}, onError: (Object _) {}),
    );
  }

  void _tabChanged() {
    if (_down.hasClients) {
      _scrollingItself = true;
      _down.jumpTo(0);
      _scrollingItself = false;
    }
    unawaited(_rows.reread());
  }

  BridgeMutation _tabWrite(
    BridgeMutationKind kind, {
    String? tab,
    String? text,
    String? field,
    BridgeLayout layout = BridgeLayout.table,
    BridgeSort? sort,
    BridgeFilter? filter,
    String? after,
    List<BridgeTab> tabs = const [],
  }) => mutation(
    kind,
    page: _page,
    view: widget.view.id,
    tab: tab ?? _tab.id,
    text: text,
    field: field,
    layout: layout,
    sort: sort,
    filter: filter,
    after: after,
    tabs: tabs,
  );

  Future<void> _addTab(BridgeTab tab, {String? after}) async {
    try {
      final added = await widget.views.write(
        _tabWrite(
          BridgeMutationKind.addTab,
          after: after ?? widget.view.tabs.lastOrNull?.id,
          tabs: [tab],
        ),
        const [],
      );
      if (mounted && added.tab != null) _select(added.tab!);
    } on Object {
      return;
    }
  }

  void _deleteTab(String id) {
    final tabs = widget.view.tabs;
    if (id == _tab.id) {
      final at = tabs.indexWhere((tab) => tab.id == id);
      final next = at > 0 ? tabs[at - 1] : tabs.elementAtOrNull(at + 1);
      if (next != null) _select(next.id);
    }
    _setQuery(_tabWrite(BridgeMutationKind.removeTab, tab: id));
  }

  Widget _tabStrip() => TabStrip(
    tabs: widget.view.tabs,
    active: _tab.id,
    onSelect: _select,
    onAdd: (layout) => unawaited(_addTab(tabOfLayout(layout))),
    onRename: (tab, name) => _setQuery(
      _tabWrite(BridgeMutationKind.renameTab, tab: tab, text: name),
    ),
    onLayout: (tab, layout) => _setQuery(
      _tabWrite(BridgeMutationKind.setTabLayout, tab: tab, layout: layout),
    ),
    onDuplicate: (tab) => unawaited(
      _addTab(
        widget.view.tabs.firstWhere((shown) => shown.id == tab),
        after: tab,
      ),
    ),
    onDelete: _deleteTab,
    onMove: (tab, after) => _setQuery(
      _tabWrite(BridgeMutationKind.moveTab, tab: tab, after: after),
    ),
  );

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

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

  void _setQuery(BridgeMutation change) => unawaited(
    widget.views.write(change, const []).then((_) {}, onError: (Object _) {}),
  );

  void _searched(String text) {
    _searchSettle?.cancel();
    _searchSettle = Timer(const Duration(milliseconds: 150), () {
      if (!mounted || text == _search) return;
      setState(() => _search = text);
      _toTop();
      unawaited(_rows.reread());
    });
  }

  void _toTop() {
    if (!_down.hasClients) return;
    _scrollingItself = true;
    _down.jumpTo(0);
    _scrollingItself = false;
  }

  void _sortBy(BridgeField field) {
    final sort = _liveSort;
    _setSort(switch (sort) {
      final sort? when sort.field == field.id && !sort.descending => BridgeSort(
        field: field.id,
        descending: true,
      ),
      final sort? when sort.field == field.id => null,
      _ => BridgeSort(field: field.id, descending: false),
    });
  }

  void _setSort(BridgeSort? sort) {
    if (widget.fullPage) {
      if (sort == _pageSort) return;
      setState(() => _pageSort = sort);
      _toTop();
      unawaited(_rows.reread());
      return;
    }
    if (sort == _tab.sort) return;
    _setQuery(_tabWrite(BridgeMutationKind.setTabSort, sort: sort));
  }

  void _setFilter(BridgeFilter? filter) {
    if (sameFilter(filter, _tab.filter)) return;
    _setQuery(_tabWrite(BridgeMutationKind.setTabFilter, filter: filter));
  }

  void _setGroup(String? field) {
    if (field == _tab.group) return;
    _setQuery(_tabWrite(BridgeMutationKind.setTabGroup, field: field));
  }

  void _setDate(String? field) {
    if (field == _tab.date) return;
    _setQuery(_tabWrite(BridgeMutationKind.setTabDate, field: field));
  }

  Widget _sortAnchor(
    List<BridgeField> fields,
    Widget Function(VoidCallback toggle, bool open) anchor,
  ) => QueryAnchor(
    anchor: anchor,
    menu: (close) => SortMenu(
      fields: fields,
      sort: _liveSort,
      onSet: _setSort,
      onClose: close,
    ),
  );

  Widget _filterAnchor(
    List<BridgeField> fields,
    NumberFormat numbers,
    String days,
    Widget Function(VoidCallback toggle, bool open) anchor,
  ) => QueryAnchor(
    anchor: anchor,
    menu: (close) => FilterMenu(
      fields: [
        for (final field in fields)
          if (field.kind != BridgeFieldKind.formula) field,
      ],
      filter: _liveFilter,
      numbers: numbers,
      locale: days,
      onSet: _setFilter,
      onClose: close,
    ),
  );

  bool _isField(String id) =>
      _database?.fields.any((field) => field.id == id) ?? false;

  BridgeSort? get _liveSort =>
      switch (widget.fullPage ? _pageSort : _tab.sort) {
        final sort? when _isField(sort.field) => sort,
        _ => null,
      };
  BridgeFilter? get _liveFilter => switch (_tab.filter) {
    final filter? when _isField(filter.field) => filter,
    _ => null,
  };

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

  void _moved() {
    if (_scrollingItself) return;
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
    _rowHandles.closeMenu();
    _cells.closePick();
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

  void _panelOutOfView() {
    if (!_panel.isShowing || _fieldsWriting > 0) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_sideways.hasClients) return;
      final position = _sideways.position;
      if (position.pixels <= position.maxScrollExtent - _addWidth) {
        _closePanel();
      }
    });
  }

  void _closePanel() {
    if (_scrollingItself) return;
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
  }

  var _scrollingItself = false;

  void _scrolled() {
    if (_scrollingItself) return;
    _cells.closePick();
    _rowHandles.closeMenu();
  }

  void _changed() {
    if (!mounted) return;
    int? rowIndex(String id) => _rows.indexWhere((row) => row.id == id);
    _cells
      ..relocate(rowIndex)
      ..read(rowIndex);
    _rowHandles.read();
    setState(() {});
  }

  late final _rowHandles = RowHandles(
    indexOf: (row) => _rows.indexWhere((shown) => shown.id == row),
    idAt: (index) async => (await _rows.load(index))?.id,
    insert: _insertRow,
    remove: (row) => _writeRows(
      mutation(
        BridgeMutationKind.removeRow,
        page: _page,
        block: _block,
        row: row,
      ),
    ),
  )..addListener(_menusChanged);

  Future<void> _insertRow(String? after) async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          after: after,
        ),
        const [],
      );
      await _takeFilterValue(added.row);
    } on Object catch (_) {}
  }

  Future<void> _takeFilterValue(String? row) async {
    if (_liveFilter case BridgeFilter(:final field, value: final value?)
        when value.kind != BridgeCellKind.checkbox || value.checked) {
      await widget.views.write(
        mutation(
          BridgeMutationKind.setCell,
          page: _page,
          block: _block,
          row: row,
          field: field,
          cell: value,
        ),
        const [],
      );
    }
  }

  Future<void> _writeRows(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {}, onError: (Object _) {});

  Future<String> _addOption(BridgeField field, String name, int color) async {
    final added = await widget.views.write(
      mutation(
        BridgeMutationKind.addOption,
        page: _page,
        block: _block,
        field: field.id,
        text: name,
        color: color,
      ),
      const [],
    );
    return added.option!;
  }

  void _placed(RenderObject cell) {
    _scrollingItself = true;
    cell.showOnScreen();
    _scrollingItself = false;
  }

  Future<void> _addRow() async {
    _rowsWriting++;
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          last: true,
        ),
        const [],
      );
      _rowAdded = true;
      await _takeFilterValue(added.row);
    } on Object catch (_) {
    } finally {
      _rowsWriting--;
      _readsAtRowAdd = _reads;
      if (_rowAdded && _rowsWriting == 0 && mounted) unawaited(_read());
    }
  }

  void _togglePanel() {
    if (_panel.isShowing) return _panel.hide();
    _fieldMenus.close();
    _panelOpened++;
    _panel.show();
  }

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

  Future<void> _writeField(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {
        if (mounted) unawaited(_read());
      }, onError: (Object _) {});

  void _menusChanged() {
    if (mounted) setState(() {});
  }

  Future<void> _addField(String name, BridgeFieldKind kind) async {
    final opened = _panelOpened;
    final BridgeApplied added;
    _fieldsWriting++;
    try {
      added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: name,
          fieldKind: kind,
          last: true,
        ),
        const [],
      );
    } finally {
      _fieldsWriting--;
      if (mounted && _panel.isShowing) setState(() {});
    }
    if (!mounted) return;
    if (opened == _panelOpened) _panel.hide();
    if (_database?.fields.any((field) => field.id == added.field) ?? false) {
      setState(() => _toLastField = true);
    } else {
      _fieldAdded = added.field;
      _readsAtFieldAdd = _reads;
      unawaited(_read());
    }
  }

  Future<void> _setCell(BridgeRow row, BridgeField field, BridgeCell cell) =>
      widget.views.write(
        mutation(
          BridgeMutationKind.setCell,
          page: _page,
          block: _block,
          row: row.id,
          field: field.id,
          cell: cell,
        ),
        const [],
      );

  void _reveal(int row, int field) =>
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _scrollingItself = true;
        if (_down.hasClients) {
          final top = (_lines?.lineOf(row) ?? row) * WsMetrics.rowTable;
          final view = _down.position.viewportDimension;
          if (top < _down.offset) {
            _down.jumpTo(top);
          } else if (top + WsMetrics.rowTable > _down.offset + view) {
            _down.jumpTo(top + WsMetrics.rowTable - view);
          }
        }
        _scrollingItself = false;
        if (_sideways.hasClients && field > 0 && field + 1 < _starts.length) {
          final view = _sideways.position.viewportDimension;
          final (left, right) = (_starts[field], _starts[field + 1]);
          _scrollingItself = true;
          if (left < _sideways.offset + _first) {
            _sideways.jumpTo(left - _first);
          } else if (right > _sideways.offset + view) {
            _sideways.jumpTo(right - view);
          }
          _scrollingItself = false;
        }
      });

  void _leave() {
    final views = widget.views;
    final at = views.indexOf(widget.view.id);
    final next = at == null ? null : views.textFrom(at + 1, 1);
    if (next != null) {
      views.moveCaret(Caret(next.id, 0));
    } else {
      _cells.cellsFocus.unfocus();
    }
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
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

  void _left() {
    if (_nameFocus.hasFocus) return;
    _name.write();
    if (widget.views.pending(widget.view.id) > 0) return;
    _name.show(widget.view.title);
  }

  Future<void> _read() async {
    final read = ++_reads;
    try {
      final answer = await _state.read(
        query(BridgeQueryKind.database, block: _block),
      );
      if (!mounted || read != _reads) return;
      if (_fieldAdded case final field?) {
        if (answer.database?.fields.any((shown) => shown.id == field) ??
            false) {
          _fieldAdded = null;
          _toLastField = true;
        } else if (read > _readsAtFieldAdd) {
          _fieldAdded = null;
        }
      }
      setState(() {
        _database = answer.database;
        _read1 = true;
        _revision++;
      });
      _fieldMenus.read();
      _cells.relocate((id) => _rows.indexWhere((row) => row.id == id));
    } on Object catch (error) {
      if (read > _readsAtRowAdd) _rowAdded = false;
      if (read > _readsAtFieldAdd) _fieldAdded = null;
      _report(error);
      return;
    }
    await _rows.reread();
    if (mounted && _rowAdded && _rowsWriting == 0 && read > _readsAtRowAdd) {
      _rowAdded = false;
      setState(() => _toLastRow = true);
    }
  }

  void _openTable() => _state.openTable(_block);

  Future<void> _deleteView() async {
    final BridgeAnswer shown;
    try {
      shown = await _state.read(
        query(BridgeQueryKind.tableViews, block: _block, limit: 2),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    if (!mounted) return;
    final remove = mutation(
      BridgeMutationKind.removeView,
      page: _page,
      view: widget.view.id,
    );
    if (shown.count > 1) {
      unawaited(_write(remove));
      return;
    }
    final chosen = await showDialog<LastView>(
      context: context,
      builder: (_) => LastViewDialog(
        name: _database?.name ?? '',
        rows: _allRows ?? _rows.count ?? 0,
      ),
    );
    if (!mounted || chosen == null) return;
    unawaited(
      _write(
        chosen == LastView.table
            ? mutation(BridgeMutationKind.deleteTable, block: _block)
            : remove,
      ),
    );
  }

  void _duplicate() {
    final name = _database?.name ?? widget.view.title;
    unawaited(
      _write(
        mutation(
          BridgeMutationKind.duplicateTable,
          page: _page,
          after: widget.view.id,
          block: _block,
          text: '${name.isEmpty ? untitled : name} (copy)',
          tabs: widget.view.tabs,
        ),
      ),
    );
  }

  Future<void> _copy() async {
    final name = _database?.name ?? widget.view.title;
    final text = name.isEmpty ? untitled : name;
    _state.copiedView = null;
    try {
      await writeClipboard(text);
      final written = await readClipboard();
      _state.copiedView = (
        text: text,
        change: written.change,
        table: _block,
        title: widget.view.title,
        tabs: widget.view.tabs,
      );
    } on Object catch (error) {
      _state.report('Liquid could not copy the View: $error');
    }
  }

  Future<void> _pasteAsView(CopiedView copied) async {
    widget.views.pastedCopy.value = null;
    try {
      await widget.views.write(
        mutation(
          BridgeMutationKind.addTableView,
          page: _page,
          after: widget.view.id,
          block: copied.table,
          text: copied.title,
          tabs: copied.tabs,
        ),
        const [],
      );
      await widget.views.write(
        mutation(BridgeMutationKind.deleteTable, block: _block),
        const [],
      );
    } on Object {
      return;
    }
  }

  Widget _pastedChip() => ValueListenableBuilder(
    valueListenable: widget.views.pastedCopy,
    builder: (context, pasted, _) {
      if (pasted == null || pasted.view != widget.view.id) {
        return const SizedBox.shrink();
      }
      final ws = context.ws;
      return Padding(
        padding: const EdgeInsets.only(top: WsMetrics.space2),
        child: Align(
          alignment: Alignment.centerLeft,
          child: Container(
            height: 30,
            padding: const EdgeInsets.only(left: WsMetrics.space3),
            decoration: BoxDecoration(
              color: ws.surface,
              border: Border.all(color: ws.border),
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              boxShadow: ws.shadow,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  'Pasted as a copy',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textMuted,
                  ),
                ),
                const SizedBox(width: WsMetrics.space2),
                TextButton(
                  onPressed: () => unawaited(_pasteAsView(pasted.copied)),
                  child: const Text('Paste as View'),
                ),
                IconButton(
                  tooltip: 'Keep the copy',
                  iconSize: 14,
                  onPressed: () => widget.views.pastedCopy.value = null,
                  icon: Icon(LucideIcons.x, color: ws.textFaint),
                ),
              ],
            ),
          ),
        ),
      );
    },
  );

  BridgeField? _groupOf(BridgeDatabase database) {
    bool groups(BridgeField field) =>
        field.kind == BridgeFieldKind.select ||
        field.kind == BridgeFieldKind.checkbox;
    return database.fields
            .where((field) => field.id == _tab.group && groups(field))
            .firstOrNull ??
        (_tab.layout == BridgeLayout.board
            ? database.fields
                  .where((field) => field.kind == BridgeFieldKind.select)
                  .firstOrNull
            : null);
  }

  BridgeField? _dateOf(BridgeDatabase database) {
    final dates = database.fields.where(
      (field) => field.kind == BridgeFieldKind.date,
    );
    return dates.where((field) => field.id == _tab.date).firstOrNull ??
        dates.firstOrNull;
  }

  Widget _groupControl(BridgeDatabase database) {
    final calendar = _tab.layout == BridgeLayout.calendar;
    final chosen = calendar ? _dateOf(database) : _groupOf(database);
    final eligible = [
      for (final field in database.fields)
        if (calendar
            ? field.kind == BridgeFieldKind.date
            : field.kind == BridgeFieldKind.select ||
                  field.kind == BridgeFieldKind.checkbox)
          field,
    ];
    return QueryAnchor(
      anchor: (toggle, _) => SetupControl(
        key: Key(calendar ? 'table date' : 'table group'),
        icon: calendar ? LucideIcons.calendar : LucideIcons.layoutList,
        label: calendar ? 'Date' : 'Group',
        value: chosen?.name,
        onTap: toggle,
      ),
      menu: (close) => ChoiceMenu(
        title: calendar ? 'Date' : 'Group by',
        onClose: close,
        hint: calendar ? 'Date fields' : 'Select and Checkbox fields',
        sections: [
          (
            label: null,
            choices: [
              if (!calendar && _tab.layout == BridgeLayout.table)
                (
                  icon: LucideIcons.x,
                  label: 'None',
                  checked: chosen == null,
                  onTap: () => _setGroup(null),
                ),
              for (final field in eligible)
                (
                  icon: fieldKindIcon(field.kind),
                  label: field.name,
                  checked: field.id == chosen?.id,
                  onTap: () =>
                      calendar ? _setDate(field.id) : _setGroup(field.id),
                ),
            ],
          ),
        ],
      ),
    );
  }

  Future<void> _write(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {}, onError: (Object _) {});

  Widget _titleLine(BridgeDatabase? database) {
    final ws = context.ws;
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: SizedBox(
        height: 32,
        child: Row(
          children: [
            Icon(LucideIcons.table, size: 16, color: ws.textFaint),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: TextField(
                controller: _name.take(),
                focusNode: _nameFocus,
                selectAllOnFocus: false,
                style: TextStyle(
                  fontSize: WsMetrics.fsProse,
                  fontWeight: WsMetrics.fwSemibold,
                  color: ws.text,
                ),
                decoration: InputDecoration.collapsed(
                  hintText: untitled,
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            if (database != null) ...[
              const SizedBox(width: WsMetrics.space3),
              Flexible(
                child: Align(
                  alignment: Alignment.centerRight,
                  child: TableNote(name: database.name, onOpen: _openTable),
                ),
              ),
            ],
            MoreMenu(
              label: 'View actions',
              visible: _hovered || touchFirst,
              items: [
                (LucideIcons.copy, 'Duplicate', _duplicate),
                (LucideIcons.clipboardCopy, 'Copy', () => unawaited(_copy())),
                (LucideIcons.trash, 'Delete View', _deleteView),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _setupLine(
    BridgeDatabase database,
    BridgeSort? sort,
    BridgeFilter? filter,
    NumberFormat numbers,
    String days,
  ) {
    String? sortLabel(BridgeSort sort) {
      final field = database.fields
          .where((field) => field.id == sort.field)
          .firstOrNull;
      if (field == null) return null;
      return '${field.name}, ${sort.descending ? 'descending' : 'ascending'}';
    }

    return Padding(
      padding: const EdgeInsets.only(
        top: WsMetrics.space1,
        bottom: WsMetrics.space2,
      ),
      child: SizedBox(
        height: 28,
        child: Row(
          children: [
            Expanded(
              child: Row(
                children: [
                  Flexible(child: _groupControl(database)),
                  const SizedBox(width: WsMetrics.space2),
                  if (_tab.layout != BridgeLayout.calendar)
                    Flexible(
                      child: _sortAnchor(
                        database.fields,
                        (toggle, _) => SetupControl(
                          key: const Key('table sort'),
                          icon: LucideIcons.arrowDownUp,
                          label: 'Sort',
                          value: sort == null ? null : sortLabel(sort),
                          onTap: toggle,
                        ),
                      ),
                    ),
                  const SizedBox(width: WsMetrics.space2),
                  Flexible(
                    child: _filterAnchor(
                      database.fields,
                      numbers,
                      days,
                      (toggle, _) => SetupControl(
                        key: const Key('table filter'),
                        icon: LucideIcons.filter,
                        label: 'Filter',
                        value: filter == null
                            ? null
                            : filterLabel(
                                filter,
                                database.fields,
                                numbers,
                                shownDays(days),
                              ),
                        onTap: toggle,
                      ),
                    ),
                  ),
                ],
              ),
            ),
            if (_tab.layout == BridgeLayout.calendar)
              _monthControls(days)
            else if (_searching)
              SizedBox(width: 200, child: _searchField(boxed: true))
            else
              QueryButton(
                key: const Key('table search'),
                icon: LucideIcons.search,
                label: 'Search',
                on: false,
                onTap: () {
                  setState(() => _searching = true);
                  _searchFocus.requestFocus();
                },
              ),
          ],
        ),
      ),
    );
  }

  Widget _searchField({required bool boxed}) {
    final ws = context.ws;
    return Container(
      height: 28,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        border: Border.all(color: ws.borderStrong),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.search, size: 14, color: ws.textFaint),
          const SizedBox(width: WsMetrics.space2),
          Expanded(
            child: TapRegion(
              onTapOutside: (_) {
                if (_searchText.text.isEmpty && !widget.fullPage) {
                  setState(() => _searching = false);
                }
              },
              child: TextField(
                key: const Key('table search field'),
                controller: _searchText,
                focusNode: _searchFocus,
                onChanged: _searched,
                style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
                decoration: InputDecoration.collapsed(
                  hintText: 'Search',
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final database = _database;
    final locale = Localizations.localeOf(context);
    final numbers = intlLocale(locale, NumberFormat.localeExists);
    _cells.numbers = NumberFormat.decimalPattern(numbers);
    final days = intlLocale(locale, DateFormat.localeExists);
    _cells.days = days;
    final count = _rows.count ?? 0;
    final lineCount = _lines?.lineCount ?? count;
    final numberFormat = NumberFormat.decimalPattern(numbers);
    final sort = _liveSort;
    final filter = _liveFilter;
    if (_toLastRow || _toLastField) {
      final (down, sideways) = (_toLastRow, _toLastField);
      _toLastRow = _toLastField = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        if (down && _down.hasClients) {
          _scrollingItself = true;
          _down.jumpTo(
            (lineCount - math.min(lineCount, _shownRows)) * WsMetrics.rowTable,
          );
          _scrollingItself = false;
        }
        if (sideways && _sideways.hasClients) {
          _scrollingItself = true;
          _sideways.jumpTo(_sideways.position.maxScrollExtent);
          _scrollingItself = false;
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
                                  NumberFormat.decimalPattern(numbers),
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

  Widget _grid(
    List<BridgeField> fields,
    double width,
    NumberFormat numbers,
    DateFormat days, {
    required bool footer,
    required String counted,
  }) {
    final ws = context.ws;
    final rest = [
      for (final field in fields.skip(1)) _width(field.kind),
      _addWidth,
    ];
    final others = rest.fold(0.0, (sum, width) => sum + width);
    final first = math.max(_firstWidth, width - others);
    final widths = [first, ...rest];
    final starts = [0.0];
    for (final width in widths) {
      starts.add(starts.last + width);
    }
    _starts = starts;
    _first = first;
    final count = _rows.count ?? 0;
    final lines = _lines;
    final grouped = _grouping;
    final options = {
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select)
          for (final option in field.options) option.id: option,
    };
    Widget content(BridgeField field, BridgeCell? cell) => CellView(
      cell: cell,
      kind: field.kind,
      options: options,
      numbers: numbers,
      days: days,
    );

    Widget line({
      required double height,
      required Color? color,
      required Widget Function(int index) cell,
      VoidCallback? addOutOfView,
      Decoration? Function(int index)? mark,
      void Function(int index)? onTap,
      void Function(int index)? onMenu,
      Key? Function(int index)? keyOf,
      bool Function(int index)? editing,
      bool Function(int index)? picking,
      bool Function(int index)? menu,
      LayerLink? handle,
    }) => ListenableBuilder(
      listenable: _sideways,
      builder: (context, _) {
        final offset = _sideways.hasClients ? _sideways.offset : 0.0;
        Widget box(int index) {
          final box = Container(
            key: keyOf?.call(index),
            width: widths[index],
            height: height,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            foregroundDecoration: mark?.call(index),
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
          if (onTap == null) return box;
          final tapped = GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => onTap(index),
            onSecondaryTap: onMenu == null ? null : () => onMenu(index),
            onLongPress: onMenu == null ? null : () => onMenu(index),
            child: box,
          );
          if (menu?.call(index) ?? false) {
            return AnchoredMenu(
              onTapOutside: (_) => _fieldMenus.close(),
              menu: _fieldMenus.view,
              child: tapped,
            );
          }
          if (picking?.call(index) ?? false) {
            return PickedCell(cells: _cells, onPlaced: _placed, child: tapped);
          }
          return editing?.call(index) ?? false
              ? TextFieldTapRegion(child: tapped)
              : tapped;
        }

        var from = 1;
        while (from < widths.length && starts[from + 1] <= offset + first) {
          from++;
        }
        var to = from;
        while (to < widths.length && starts[to] < offset + width) {
          to++;
        }
        if (to < widths.length) addOutOfView?.call();
        return Stack(
          children: [
            Row(
              children: [
                SizedBox(width: starts[from], height: height),
                for (var index = from; index < to; index++) box(index),
              ],
            ),
            Positioned(
              left: offset,
              top: 0,
              bottom: 0,
              child: handle == null
                  ? box(0)
                  : CompositedTransformTarget(link: handle, child: box(0)),
            ),
          ],
        );
      },
    );

    return SingleChildScrollView(
      key: const Key('table columns'),
      controller: _sideways,
      scrollDirection: Axis.horizontal,
      child: SizedBox(
        width: first + others,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MouseRegion(
              onEnter: (_) => setState(() => _headerHovered = true),
              onExit: (_) => setState(() => _headerHovered = false),
              child: DecoratedBox(
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
                    addOutOfView: _panelOutOfView,
                    onTap: (index) {
                      if (index >= fields.length) return;
                      _fieldMenus.close();
                      _sortBy(fields[index]);
                    },
                    onMenu: (index) {
                      if (index >= fields.length) return;
                      if (_panel.isShowing) _panel.hide();
                      _fieldMenus.toggle(fields[index]);
                    },
                    menu: (index) =>
                        index < fields.length &&
                        fields[index].id == _fieldMenus.open,
                    cell: (index) => index == fields.length
                        ? AddField(
                            plus: _plus,
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
                              Flexible(child: cellText(fields[index].name)),
                              if (_liveSort case final sort?
                                  when sort.field == fields[index].id) ...[
                                const SizedBox(width: WsMetrics.space1),
                                Icon(
                                  sort.descending
                                      ? LucideIcons.arrowDown
                                      : LucideIcons.arrowUp,
                                  key: const Key('column sorted'),
                                  size: 12,
                                  color: ws.accent,
                                ),
                              ],
                              const Spacer(),
                              _ColumnMenuButton(
                                shown:
                                    _headerHovered ||
                                    touchFirst ||
                                    fields[index].id == _fieldMenus.open,
                                onTap: () {
                                  if (_panel.isShowing) _panel.hide();
                                  _fieldMenus.toggle(fields[index]);
                                },
                              ),
                            ],
                          ),
                  ),
                ),
              ),
            ),
            SizedBox(
              height:
                  math.min(lines?.lineCount ?? count, _shownRows) *
                  WsMetrics.rowTable,
              key: _rowsKey,
              child: ListView.builder(
                key: const Key('table rows'),
                controller: _down,
                itemCount: lines?.lineCount ?? count,
                itemExtent: WsMetrics.rowTable,
                itemBuilder: (context, shownLine) {
                  final at = lines?.at(shownLine);
                  if (at?.group case final index? when grouped != null) {
                    final group = lines!.groups[index];
                    return ListenableBuilder(
                      listenable: _sideways,
                      builder: (context, _) => Padding(
                        padding: EdgeInsets.only(
                          left: _sideways.hasClients ? _sideways.offset : 0,
                        ),
                        child: GroupHeader(
                          label: GroupLabel(
                            field: grouped,
                            value: group.value,
                            days: days,
                            numbers: numbers,
                          ),
                          count: group.count,
                          folded: folds(_tab, group.value),
                          onToggle: () => _toggleFold(group.value),
                          numbers: numbers,
                        ),
                      ),
                    );
                  }
                  final rowIndex = at?.row ?? shownLine;
                  final row = _rows.at(rowIndex);
                  final cells = <String, BridgeCell>{
                    if (row != null)
                      for (final field in fields)
                        field.id: ?cellOf(row, field.id),
                  };
                  final hovered =
                      row != null && row.id == _rowHandles.handleRow;
                  final shown = DecoratedBox(
                    position: DecorationPosition.foreground,
                    decoration: BoxDecoration(
                      border: Border(bottom: BorderSide(color: ws.border)),
                    ),
                    child: line(
                      height: WsMetrics.rowTable,
                      color: hovered && _rowHandles.handle.isShowing
                          ? ws.hover
                          : ws.canvas,
                      handle: hovered ? _rowHandles.handleAnchor : null,
                      keyOf: (index) => row == null || index == fields.length
                          ? null
                          : ValueKey('cell ${row.id} ${fields[index].id}'),
                      onTap: row == null
                          ? null
                          : (index) {
                              if (index < fields.length) {
                                _cells.tap(rowIndex, index);
                              }
                            },
                      mark: (index) => _dropAt == (rowIndex, index)
                          ? BoxDecoration(
                              color: ws.accent.withValues(alpha: 0.06),
                              border: Border.all(color: ws.accent, width: 2),
                            )
                          : _cells.mark(context, rowIndex, index),
                      picking: (index) =>
                          _cells.picking &&
                          _cells.selected == (rowIndex, index),
                      editing: (index) =>
                          _cells.editing &&
                          _cells.selected == (rowIndex, index),
                      cell: (index) {
                        if (index == fields.length) {
                          return const SizedBox.shrink();
                        }
                        final field = fields[index];
                        if (_cells.editing &&
                            _cells.selected == (rowIndex, index) &&
                            row != null) {
                          return _cells.editor(context, field);
                        }
                        return Align(
                          alignment: switch (field.kind) {
                            BridgeFieldKind.number ||
                            BridgeFieldKind.formula => Alignment.centerRight,
                            BridgeFieldKind.checkbox => Alignment.center,
                            _ => Alignment.centerLeft,
                          },
                          child: content(field, cells[field.id]),
                        );
                      },
                    ),
                  );
                  if (row == null) return shown;
                  return HoverRegion(
                    key: ValueKey('row ${row.id}'),
                    onHover: (hovered, region) =>
                        _rowHandles.hoverRow(row.id, hovered, region),
                    child: shown,
                  );
                },
              ),
            ),
            ListenableBuilder(
              listenable: _sideways,
              builder: (context, child) => Padding(
                padding: EdgeInsets.only(
                  left: _sideways.hasClients ? _sideways.offset : 0,
                ),
                child: child,
              ),
              child: Align(
                alignment: Alignment.centerLeft,
                child: SizedBox(
                  width: width,
                  child: _NewRow(onTap: () => unawaited(_addRow())),
                ),
              ),
            ),
            if (footer)
              MouseRegion(
                onEnter: (_) => setState(() => _footerHovered = true),
                onExit: (_) => setState(() => _footerHovered = false),
                child: DecoratedBox(
                  position: DecorationPosition.foreground,
                  decoration: BoxDecoration(
                    border: Border(top: BorderSide(color: ws.border)),
                  ),
                  child: line(
                    height: _footerHeight,
                    color: ws.subtle,
                    cell: (index) => index == 0
                        ? Align(
                            alignment: Alignment.centerLeft,
                            child: Text(
                              counted,
                              style: TextStyle(
                                fontSize: WsMetrics.fsCompact,
                                color: ws.textFaint,
                              ),
                            ),
                          )
                        : index == fields.length
                        ? const SizedBox.shrink()
                        : _totalCell(fields[index], numbers),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _NewRow extends StatelessWidget {
  const _NewRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      button: true,
      child: GestureDetector(
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
      ),
    );
  }
}

class _ColumnMenuButton extends StatelessWidget {
  const _ColumnMenuButton({required this.shown, required this.onTap});

  final bool shown;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'Column menu',
    child: GestureDetector(
      key: const Key('column menu'),
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox(
        width: 18,
        height: 24,
        child: Icon(
          LucideIcons.chevronDown,
          size: 12,
          color: shown ? context.ws.textMuted : const Color(0x00000000),
        ),
      ),
    ),
  );
}
