part of 'table_block.dart';

extension _TableBlockTabs on _TableBlockState {
  void _toggleFold(BridgeCell value) => _setQuery(
    _tabWrite(
      BridgeMutationKind.setTabFolded,
      tabs: [copiedTab(_tab, folded: foldedToggled(_tab, value))],
    ),
  );

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
    _update(() => _tabId = id);
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
    _toTop();
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
}
