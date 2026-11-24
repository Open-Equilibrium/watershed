part of 'table_block.dart';

extension _TableBlockQuery on _TableBlockState {
  void _searched(String text) {
    _searchSettle?.cancel();
    _searchSettle = Timer(const Duration(milliseconds: 150), () {
      if (!mounted || text == _search) return;
      _update(() => _search = text);
      _toTop();
      unawaited(_rows.reread());
    });
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
      _update(() => _pageSort = sort);
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

    final filterable = [
      for (final field in database.fields)
        if (field.kind != BridgeFieldKind.formula &&
            field.kind != BridgeFieldKind.button)
          field,
    ];
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
                      child: QueryAnchor(
                        anchor: (toggle, _) => SetupControl(
                          key: const Key('table sort'),
                          icon: LucideIcons.arrowDownUp,
                          label: 'Sort',
                          value: sort == null ? null : sortLabel(sort),
                          onTap: toggle,
                        ),
                        menu: (close) => SortMenu(
                          fields: database.fields,
                          sort: _liveSort,
                          onSet: _setSort,
                          onClose: close,
                        ),
                      ),
                    ),
                  const SizedBox(width: WsMetrics.space2),
                  Flexible(
                    child: QueryAnchor(
                      anchor: (toggle, _) => SetupControl(
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
                        onTap: filterable.isEmpty ? null : toggle,
                        disabledReason: 'No field can be filtered',
                      ),
                      menu: (close) => FilterMenu(
                        fields: filterable,
                        filter: _liveFilter,
                        numbers: numbers,
                        locale: days,
                        onSet: _setFilter,
                        onClose: close,
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
                  _update(() => _searching = true);
                  _searchFocus.requestFocus();
                },
              ),
          ],
        ),
      ),
    );
  }

  void _clearSearch() {
    writeText(_searchText, const TextEditingValue());
    _searched('');
  }

  Widget _searchField({required bool boxed}) {
    final ws = context.ws;
    return Focus(
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent ||
            event.logicalKey != LogicalKeyboardKey.escape) {
          return KeyEventResult.ignored;
        }
        _clearSearch();
        if (!widget.fullPage) _update(() => _searching = false);
        return KeyEventResult.handled;
      },
      child: Container(
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
                    _update(() => _searching = false);
                  }
                },
                child: TextField(
                  key: const Key('table search field'),
                  controller: _searchText,
                  focusNode: _searchFocus,
                  onChanged: _searched,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.text,
                  ),
                  decoration: InputDecoration.collapsed(
                    hintText: 'Search',
                    hintStyle: TextStyle(color: ws.textFaint),
                  ),
                ),
              ),
            ),
            ListenableBuilder(
              listenable: _searchText,
              builder: (context, _) => _searchText.text.isEmpty
                  ? const SizedBox.shrink()
                  : Semantics(
                      button: true,
                      label: 'Clear search',
                      child: GestureDetector(
                        key: const Key('table search clear'),
                        behavior: HitTestBehavior.opaque,
                        onTap: () {
                          _clearSearch();
                          _searchFocus.requestFocus();
                        },
                        child: Padding(
                          padding: const EdgeInsets.only(
                            left: WsMetrics.space2,
                          ),
                          child: Icon(
                            LucideIcons.x,
                            size: 14,
                            color: ws.textFaint,
                          ),
                        ),
                      ),
                    ),
            ),
          ],
        ),
      ),
    );
  }
}
