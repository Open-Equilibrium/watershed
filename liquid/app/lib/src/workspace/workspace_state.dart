import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/database.dart';
import '../rust/api/replica.dart';
import '../settings_file.dart';
import '../rust/api/drawing.dart';
import '../rust/api/workspace.dart';
import 'follow.dart';
import 'page_views.dart';

const window = 500;

BridgeMutation mutation(
  BridgeMutationKind kind, {
  String? area,
  String? page,
  String? parent,
  String? block,
  String? view,
  String? after,
  bool last = false,
  String? text,
  int at = 0,
  int delete = 0,
  BridgeTextStyle style = BridgeTextStyle.paragraph,
  bool boundary = false,
  String? field,
  String? row,
  BridgeFieldKind? fieldKind,
  int color = 0,
  BridgeCell? cell,
  BridgeSort? sort,
  BridgeFilter? filter,
  String? tab,
  BridgeTile? tile,
  String? lastView,
  BridgeLayout layout = BridgeLayout.table,
  List<BridgeTab> tabs = const [],
  String? mediaType,
  String? attachment,
  BridgeDrawingStyle? drawingStyle,
  BridgeDrawingEdit? drawing,
}) => BridgeMutation(
  kind: kind,
  area: area,
  page: page,
  parent: parent,
  block: block,
  view: view,
  after: after,
  last: last,
  text: text,
  at: at,
  delete: delete,
  style: style,
  boundary: boundary,
  field: field,
  row: row,
  fieldKind: fieldKind,
  color: color,
  cell: cell,
  sort: sort,
  filter: filter,
  tab: tab,
  tile: tile,
  lastView: lastView,
  layout: layout,
  tabs: tabs,
  mediaType: mediaType,
  attachment: attachment,
  drawingStyle: drawingStyle,
  drawing: drawing,
);

BridgeQuery query(
  BridgeQueryKind kind, {
  String? area,
  String? page,
  String? block,
  int start = 0,
  int limit = window,
  String? field,
  BridgeSort? sort,
  BridgeFilter? filter,
  BridgeFilter? within,
  String? search,
}) => BridgeQuery(
  kind: kind,
  area: area,
  page: page,
  block: block,
  start: start,
  limit: limit,
  field: field,
  sort: sort,
  filter: filter,
  within: within,
  search: search,
);

typedef CopiedView = ({
  String text,
  int? change,
  String table,
  String title,
  List<BridgeTab> tabs,
});

class EditChain {
  Object? refused;
}

@immutable
class OpenPage {
  const OpenPage(this.area, this.page, {this.fresh = false}) : table = null;

  const OpenPage.table(String this.table) : area = '', page = '', fresh = false;

  final String area;
  final String page;

  final bool fresh;

  final String? table;
}

class _Reread {
  _Reread(this._read);

  final Future<void> Function() _read;
  Future<void>? _reading;
  bool _again = false;

  Future<void> call() {
    if (_reading case final reading?) {
      _again = true;
      return reading;
    }
    return _reading = _loop();
  }

  Future<void> _loop() async {
    try {
      do {
        _again = false;
        await _read();
      } while (_again);
    } finally {
      _reading = null;
    }
  }
}

class WorkspaceState {
  CopiedView? copiedView;

  WorkspaceState(this.workspace, {this.layout});

  final LiquidWorkspace workspace;

  final SettingsFile? layout;

  final folded = ValueNotifier<Set<String>>(const {});

  Future<void> _loadLayout() async {
    final json = await layout?.load();
    if (json?['folded'] case final List<Object?> folded) {
      this.folded.value = {...folded.whereType<String>()};
    }
  }

  void toggleFold(String section) {
    final folded = {...this.folded.value};
    if (!folded.remove(section)) folded.add(section);
    this.folded.value = folded;
    final layout = this.layout;
    if (layout == null) return;
    final json = {'folded': folded.toList()};
    _layoutSaving = _layoutSaving
        .then((_) => layout.save(json))
        .then<void>((_) {}, onError: (Object _) {});
  }

  Future<void> _layoutSaving = Future.value();

  @visibleForTesting
  Future<void> get layoutSaved => _layoutSaving;

  final areas = ValueNotifier<List<BridgeArea>>(const []);

  final named = ValueNotifier<({String name, String icon})>((
    name: '',
    icon: '',
  ));

  final tables = ValueNotifier<List<BridgeTable>>(const []);

  final open = ValueNotifier<OpenPage?>(null);

  final currentPage = ValueNotifier<PageViews?>(null);

  final recent = ValueNotifier<List<String>>(const []);

  static const _recentKept = 8;

  Future<void> _loadRecent() async {
    try {
      final kept = await workspace.remembered(key: 'recent');
      if (kept != null && kept.isNotEmpty) recent.value = kept.split(',');
    } on Object {
      recent.value = const [];
    }
    open.addListener(_opened);
    _opened();
  }

  void _opened() {
    final page = open.value?.page;
    if (open.value?.table != null ||
        page == null ||
        page.isEmpty ||
        recent.value.firstOrNull == page) {
      return;
    }
    recent.value = [
      page,
      for (final kept in recent.value)
        if (kept != page) kept,
    ].take(_recentKept).toList();
    unawaited(
      workspace
          .remember(key: 'recent', value: recent.value.join(','))
          .then((_) {}, onError: (Object _) {}),
    );
  }

  final problem = ValueNotifier<String?>(null);

  String? _changeFailed;
  String? _readFailed;

  final _trees = <String, ValueNotifier<List<BridgePage>>>{};
  final _treeReads = <String, _Reread>{};
  final _followed = <StreamSubscription<BridgeChange>>[];
  Future<void> _applied = Future.value();
  bool _disposed = false;

  ValueListenable<List<BridgePage>> pages(String area) =>
      _trees.putIfAbsent(area, () => ValueNotifier(const []));

  void report(String problem) => _show(change: problem, read: _readFailed);

  void readFailed(Object error) => _show(
    change: _changeFailed,
    read:
        'Liquid could not read the Workspace, so what shows may be out of date: $error',
  );

  void readSucceeded() => _show(change: _changeFailed, read: null);

  Future<BridgeAnswer> read(BridgeQuery asked) async {
    final answer = await workspace.query(query: asked);
    readSucceeded();
    return answer;
  }

  void _show({required String? change, required String? read}) {
    if (_disposed) return;
    _changeFailed = change;
    _readFailed = read;
    final shown = [?change, ?read];
    problem.value = shown.isEmpty ? null : shown.join('\n');
  }

  Future<void> get settled => _applied;

  Future<BridgeApplied> apply(BridgeMutation change, {EditChain? chain}) {
    final shown = _applied
        .then((_) => _applyIn(chain, change))
        .then(
          (applied) {
            _show(change: null, read: null);
            return applied;
          },
          onError: (Object error) {
            _show(
              change: 'Liquid did not save this change: $error',
              read: _readFailed,
            );
            throw error;
          },
        );
    _applied = shown.then<void>((_) {}, onError: (_) {});
    return shown;
  }

  Future<BridgeApplied> _applyIn(
    EditChain? chain,
    BridgeMutation change,
  ) async {
    if (chain?.refused case final refused?) throw refused;
    try {
      return await workspace.apply(mutation: change);
    } on Object catch (error) {
      chain?.refused = error;
      rethrow;
    }
  }

  Future<void> start({bool show = true}) async {
    await _loadLayout();
    await _loadRecent();
    final areas = _Reread(_readAreas);
    _listen(const BridgeResource(kind: BridgeResourceKind.areas), areas);
    final tables = _Reread(_readTables);
    _listen(const BridgeResource(kind: BridgeResourceKind.tables), tables);
    _listen(
      const BridgeResource(kind: BridgeResourceKind.workspace),
      _Reread(_readNamed),
    );
    final first = await workspace.query(query: query(BridgeQueryKind.areas));
    if (first.count == 0) {
      final private = await apply(
        mutation(BridgeMutationKind.createArea, text: 'Private'),
      );
      await newPage(private.area!, show: show);
      return;
    }
    if (!show) return;
    final area = first.areas.first.id;
    final pages = await workspace.query(
      query: query(BridgeQueryKind.pages, area: area),
    );
    if (pages.pages.firstOrNull case final page?) {
      open.value ??= OpenPage(area, page.id);
    }
  }

  Future<void> newPage(String area, {String? parent, bool show = true}) async {
    final created = await apply(
      mutation(
        BridgeMutationKind.createPage,
        area: area,
        parent: parent,
        text: '',
      ),
    );
    if (show) open.value = OpenPage(area, created.page!, fresh: true);
  }

  Future<void> newTable(String area) async {
    final created = await apply(
      mutation(BridgeMutationKind.createTable, area: area, text: ''),
    );
    openTable(created.block!);
  }

  OpenPage? _back;

  void openTable(String table) {
    if (open.value case final page? when page.table == null) _back = page;
    open.value = OpenPage.table(table);
  }

  Future<void> deleteTable(String table) async {
    await apply(mutation(BridgeMutationKind.deleteTable, block: table));
    if (open.value?.table == table) open.value = _back;
  }

  Future<void> _readNamed() async {
    final answer = await read(query(BridgeQueryKind.workspace));
    if (!_disposed) {
      named.value = (name: answer.text ?? '', icon: answer.icon ?? '');
    }
  }

  Future<void> renameWorkspace(String name) =>
      apply(mutation(BridgeMutationKind.renameWorkspace, text: name));

  Future<void> setWorkspaceIcon(String icon) =>
      apply(mutation(BridgeMutationKind.setWorkspaceIcon, text: icon));

  Future<void> _readTables() async {
    final read = <BridgeTable>[];
    var count = 0;
    do {
      final answer = await this.read(
        query(BridgeQueryKind.tables, start: read.length),
      );
      count = answer.count;
      if (answer.tables.isEmpty) break;
      read.addAll(answer.tables);
    } while (read.length < count);
    if (!_disposed) tables.value = read;
  }

  Future<void> rename(String page, String title) =>
      apply(mutation(BridgeMutationKind.renamePage, page: page, text: title));

  Future<void> readPages(String area) =>
      _treeReads[area]?.call() ?? Future.value();

  void _listen(BridgeResource resource, _Reread reread) {
    _followed.add(
      follow(
        workspace,
        resource,
      ).listen((_) => unawaited(_guarded(reread)), onError: readFailed),
    );
  }

  Future<void> _guarded(_Reread reread) async {
    try {
      await reread();
    } on Object catch (error) {
      readFailed(error);
    }
  }

  Future<void> _readAreas() async {
    final read = <BridgeArea>[];
    var count = 0;
    do {
      final answer = await this.read(
        query(BridgeQueryKind.areas, start: read.length),
      );
      count = answer.count;
      if (answer.areas.isEmpty) break;
      read.addAll(answer.areas);
    } while (read.length < count);
    if (_disposed) return;
    for (final area in read) {
      if (_treeReads.containsKey(area.id)) continue;
      pages(area.id);
      _listen(
        BridgeResource(kind: BridgeResourceKind.pages, area: area.id),
        _treeReads[area.id] = _Reread(() => _readPages(area.id)),
      );
    }
    areas.value = read;
  }

  Future<void> _readPages(String area) async {
    final read = <BridgePage>[];
    var count = 0;
    do {
      final answer = await this.read(
        query(BridgeQueryKind.pages, area: area, start: read.length),
      );
      count = answer.count;
      if (answer.pages.isEmpty) break;
      read.addAll(answer.pages);
    } while (read.length < count);
    if (!_disposed) _trees[area]!.value = read;
  }

  final finishing = <VoidCallback>{};

  bool get quitting => _quitting;
  bool _quitting = false;

  final _tracked = <Future<void>>{};

  final _pages = <String, Future<void>>{};

  Future<void>? pageWork(String page) => _pages[page];

  void trackPage(String page, Future<void> work) {
    _pages[page] = work;
    track(work);
    unawaited(
      work.whenComplete(() {
        if (identical(_pages[page], work)) _pages.remove(page);
      }),
    );
  }

  void track(Future<void> work) {
    _tracked.add(work);
    unawaited(work.whenComplete(() => _tracked.remove(work)));
  }

  Future<void> drain() async {
    _quitting = true;
    for (final finish in [...finishing]) {
      finish();
    }
    Future<void> last;
    do {
      await Future.wait([..._tracked]);
      last = _applied;
      await last;
    } while (!identical(last, _applied) || _tracked.isNotEmpty);
  }

  Future<void> dispose() async {
    _disposed = true;
    final followed = [..._followed];
    _followed.clear();
    await Future.wait(followed.map((subscription) => subscription.cancel()));
    areas.dispose();
    tables.dispose();
    named.dispose();
    folded.dispose();
    open.dispose();
    problem.dispose();
    for (final tree in _trees.values) {
      tree.dispose();
    }
  }
}
