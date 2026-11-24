part of 'fake_workspace.dart';

extension _FakePages on FakeWorkspace {
  BridgeApplied? _pagesMutation(BridgeMutation mutation) {
    switch (mutation.kind) {
      case BridgeMutationKind.renameWorkspace ||
          BridgeMutationKind.setWorkspaceIcon:
        if (mutation.kind == BridgeMutationKind.renameWorkspace) {
          name = mutation.text!;
        } else {
          icon = mutation.text!;
        }
        _changed(const BridgeResource(kind: BridgeResourceKind.workspace));
        return _done();
      case BridgeMutationKind.createArea:
        final area = BridgeArea(id: _id(), name: mutation.text!);
        areas.add(area);
        pages[area.id] = [];
        _changed(const BridgeResource(kind: BridgeResourceKind.areas));
        return _done(area: area.id);
      case BridgeMutationKind.createPage:
        final tree = pages[mutation.area]!;
        final page = BridgePage(
          id: _id(),
          parent: mutation.parent,
          title: mutation.text!,
          icon: '',
        );
        var at = tree.length;
        if (mutation.parent case final parent?) {
          at = tree.indexWhere((page) => page.id == parent) + 1;
          while (at < tree.length && _below(tree, tree[at], parent)) {
            at++;
          }
        }
        tree.insert(at, page);
        _changed(
          BridgeResource(kind: BridgeResourceKind.pages, area: mutation.area),
        );
        return _done(page: page.id);
      case BridgeMutationKind.renamePage || BridgeMutationKind.setPageIcon:
        final icon = mutation.kind == BridgeMutationKind.setPageIcon;
        for (final MapEntry(key: area, value: tree) in pages.entries) {
          final at = tree.indexWhere((page) => page.id == mutation.page);
          if (at < 0) continue;
          final page = tree[at];
          tree[at] = BridgePage(
            id: page.id,
            parent: page.parent,
            title: icon ? page.title : mutation.text!,
            icon: icon ? mutation.text! : page.icon,
          );
          _changed(BridgeResource(kind: BridgeResourceKind.pages, area: area));
        }
        return _done();
      case BridgeMutationKind.setSectionBoundary:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        page[at] = _withView(page[at], boundary: mutation.boundary);
        _viewsChanged(mutation.page);
        return _done();
      case BridgeMutationKind.setTile || BridgeMutationKind.setInnerTile:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        if (at < 0) throw 'not found';
        final inner = mutation.kind == BridgeMutationKind.setInnerTile;
        page[at] = _withView(
          page[at],
          tile: inner ? null : (mutation.tile,),
          inner: inner ? (mutation.tile,) : null,
        );
        _viewsChanged(mutation.page);
        return _done();
      case BridgeMutationKind.moveViews:
        final page = views[mutation.page]!;
        final from = page.indexWhere((view) => view.id == mutation.view);
        final to = page.indexWhere((view) => view.id == mutation.lastView);
        if (from < 0 || to < from) throw 'text position out of range';
        final run = page.sublist(from, to + 1);
        if (run.any((view) => view.id == mutation.after)) {
          throw 'text position out of range';
        }
        page.removeRange(from, to + 1);
        page.insertAll(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          run,
        );
        _viewsChanged(mutation.page);
        return _done();
      case BridgeMutationKind.removeView:
        views[mutation.page]!.removeWhere((view) => view.id == mutation.view);
        _viewsChanged(mutation.page);
        return _done();
      case BridgeMutationKind.renameView:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final shown = page[at];
        page[at] = _withView(shown, title: mutation.text!);
        _viewsChanged(mutation.page);
        if (shown.kind == BridgeBlockKind.database &&
            tables.any(
              (table) => table.id == shown.block && table.name.isEmpty,
            )) {
          _renameTable(shown.block, mutation.text!);
        }
        return _done();
      default:
        return null;
    }
  }

  BridgeView _withView(
    BridgeView view, {
    String? title,
    bool? boundary,
    BridgeBlockKind? kind,
    List<BridgeTab>? tabs,
    (BridgeTile?,)? tile,
    (BridgeTile?,)? inner,
  }) => BridgeView(
    id: view.id,
    block: view.block,
    title: title ?? view.title,
    boundary: boundary ?? view.boundary,
    kind: kind ?? view.kind,
    tabs: tabs ?? view.tabs,
    tile: tile == null ? view.tile : tile.$1,
    inner: inner == null ? view.inner : inner.$1,
  );

  String _areaOf(String page) => pages.entries
      .firstWhere((entry) => entry.value.any((shown) => shown.id == page))
      .key;

  BridgeView _newView(
    BridgeBlockKind kind, {
    bool boundary = false,
    String title = '',
    List<BridgeTab> tabs = const [],
  }) => BridgeView(
    id: _id(),
    block: _id(),
    title: title,
    boundary: boundary,
    kind: kind,
    tabs: tabs,
  );
  BridgeApplied _placeView(BridgeMutation mutation, BridgeView view) {
    final page = views.putIfAbsent(mutation.page!, () => []);
    page.insert(
      mutation.after == null
          ? 0
          : page.indexWhere((shown) => shown.id == mutation.after) + 1,
      view,
    );
    _viewsChanged(mutation.page);
    return _done(block: view.block, view: view.id);
  }

  void _viewsChanged(String? page) =>
      _changed(BridgeResource(kind: BridgeResourceKind.views, page: page));

  BridgeBlockKind? _kindOf(String page, String block) =>
      views[page]?.where((view) => view.block == block).firstOrNull?.kind;

  bool _isCode(String? block) => views.values.any(
    (page) => page.any(
      (view) => view.block == block && view.kind == BridgeBlockKind.code,
    ),
  );

  BridgeResource _blockResource(BridgeMutation mutation) => BridgeResource(
    kind: BridgeResourceKind.block,
    page: mutation.page,
    block: mutation.block,
  );

  bool _below(List<BridgePage> tree, BridgePage page, String parent) {
    for (var at = page.parent; at != null;) {
      if (at == parent) return true;
      at = tree.firstWhere((page) => page.id == at).parent;
    }
    return false;
  }
}
