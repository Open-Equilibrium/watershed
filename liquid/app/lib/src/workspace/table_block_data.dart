part of 'table_block.dart';

extension _TableBlockData on _TableBlockState {
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
    if (mounted) _update(() {});
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

  String _rowCount(NumberFormat format, int count, BridgeFilter? filter) {
    final all = _allRows ?? count;
    return filter == null && _search.isEmpty
        ? '${format.format(count)} ${count == 1 ? 'row' : 'rows'}'
        : '${format.format(count)} of ${format.format(all)} '
              '${all == 1 ? 'row' : 'rows'}';
  }

  void _setQuery(BridgeMutation change) => unawaited(
    widget.views.write(change, const []).then((_) {}, onError: (Object _) {}),
  );

  Future<void> _writeRows(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {}, onError: (Object _) {});

  Future<void> _writeField(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {
        if (mounted) unawaited(_read());
      }, onError: (Object _) {});

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

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
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
      _update(() {
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
      _update(() => _toLastRow = true);
    }
  }

  Future<void> _write(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {}, onError: (Object _) {});
}
