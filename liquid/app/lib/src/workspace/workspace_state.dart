import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import 'follow.dart';

/// The most a query returns at once (client architecture rule 3).
const window = 500;

/// A mutation with the parts its kind does not use left at their defaults.
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
);

/// A query with the parts its kind does not use left out.
BridgeQuery query(
  BridgeQueryKind kind, {
  String? area,
  String? page,
  String? block,
  int start = 0,
  int limit = window,
  BridgeSort? sort,
  BridgeFilter? filter,
}) => BridgeQuery(
  kind: kind,
  area: area,
  page: page,
  block: block,
  start: start,
  limit: limit,
  sort: sort,
  filter: filter,
);

/// The Page the window shows.
@immutable
class OpenPage {
  const OpenPage(this.area, this.page, {this.fresh = false});

  final String area;
  final String page;

  /// Just created, so its title takes the keys.
  final bool fresh;
}

/// Reads again after each change, one read at a time, so the last change is always read.
class _Reread {
  _Reread(this._read);

  final Future<void> Function() _read;
  bool _reading = false;
  bool _again = false;

  Future<void> call() async {
    if (_reading) {
      _again = true;
      return;
    }
    _reading = true;
    try {
      do {
        _again = false;
        await _read();
      } while (_again);
    } finally {
      _reading = false;
    }
  }
}

/// The Workspace the window shows: its areas with their Page trees, kept current, and the
/// open Page. It owns every subscription it follows (client architecture rule 14).
class WorkspaceState {
  WorkspaceState(this.workspace);

  final LiquidWorkspace workspace;

  /// The areas in the Workspace's order.
  final areas = ValueNotifier<List<BridgeArea>>(const []);

  /// The open Page, or none while the agent session shows.
  final open = ValueNotifier<OpenPage?>(null);

  /// The last change or read that failed, until the next one succeeds.
  final problem = ValueNotifier<String?>(null);

  final _trees = <String, ValueNotifier<List<BridgePage>>>{};
  final _followedAreas = <String>{};
  final _followed = <StreamSubscription<BridgeChange>>[];
  Future<void> _applied = Future.value();
  bool _disposed = false;

  /// `area`'s Pages in their tree's order, each after its parent's earlier siblings.
  ValueListenable<List<BridgePage>> pages(String area) =>
      _trees.putIfAbsent(area, () => ValueNotifier(const []));

  /// Completes once every mutation asked for so far is applied or refused.
  Future<void> get settled => _applied;

  /// Applies mutations in the order asked: the bridge may run two calls at once.
  Future<BridgeApplied> apply(BridgeMutation change) {
    final applied = _applied.then((_) => workspace.apply(mutation: change));
    _applied = applied.then<void>((_) {}, onError: (_) {});
    return applied.then(
      (applied) {
        if (!_disposed) problem.value = null;
        return applied;
      },
      onError: (Object error) {
        if (!_disposed) problem.value = '$error';
        throw error;
      },
    );
  }

  /// Follows the areas and their Page trees. A new Workspace gets the area "Private" with
  /// one blank Page, as Notion starts. With `show`, that Page or else the first one opens.
  Future<void> start({bool show = true}) async {
    final areas = _Reread(_readAreas);
    _listen(const BridgeResource(kind: BridgeResourceKind.areas), areas);
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

  /// Creates a blank Page in `area`, below `parent` if given, and opens it unless `show` is
  /// false.
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

  Future<void> rename(String page, String title) =>
      apply(mutation(BridgeMutationKind.renamePage, page: page, text: title));

  void _listen(BridgeResource resource, _Reread reread) {
    _followed.add(
      follow(workspace, resource).listen(
        (_) => unawaited(_guarded(reread)),
        onError: (Object error) {
          if (!_disposed) problem.value = '$error';
        },
      ),
    );
  }

  Future<void> _guarded(_Reread reread) async {
    try {
      await reread();
    } on Object catch (error) {
      if (!_disposed) problem.value = '$error';
    }
  }

  Future<void> _readAreas() async {
    final read = <BridgeArea>[];
    var count = 0;
    do {
      final answer = await workspace.query(
        query: query(BridgeQueryKind.areas, start: read.length),
      );
      count = answer.count;
      if (answer.areas.isEmpty) break;
      read.addAll(answer.areas);
    } while (read.length < count);
    // A read still running when the window closed changes nothing.
    if (_disposed) return;
    for (final area in read) {
      if (!_followedAreas.add(area.id)) continue;
      pages(area.id);
      _listen(
        BridgeResource(kind: BridgeResourceKind.pages, area: area.id),
        _Reread(() => _readPages(area.id)),
      );
    }
    areas.value = read;
  }

  Future<void> _readPages(String area) async {
    final read = <BridgePage>[];
    var count = 0;
    do {
      final answer = await workspace.query(
        query: query(BridgeQueryKind.pages, area: area, start: read.length),
      );
      count = answer.count;
      if (answer.pages.isEmpty) break;
      read.addAll(answer.pages);
    } while (read.length < count);
    if (!_disposed) _trees[area]!.value = read;
  }

  /// What to tell open fields and input queues when Liquid quits, so a composition still
  /// open is written as it stands and none waits for it.
  final finishing = <VoidCallback>{};

  /// Set once Liquid quits.
  bool get quitting => _quitting;
  bool _quitting = false;

  /// Work that will ask for writes, such as a Page's input queue (ADR-0240).
  final _tracked = <Future<void>>{};

  final _pages = <String, Future<void>>{};

  /// A Page's input queue still writing, also after the Page closed, or none.
  Future<void>? pageWork(String page) => _pages[page];

  /// Counts `work` on `page` until it is done, so that the Page reopened meanwhile waits
  /// for it before taking keys.
  void trackPage(String page, Future<void> work) {
    _pages[page] = work;
    track(work);
    unawaited(
      work.whenComplete(() {
        if (identical(_pages[page], work)) _pages.remove(page);
      }),
    );
  }

  /// Counts `work` until it is done, so that draining waits for its writes too.
  void track(Future<void> work) {
    _tracked.add(work);
    unawaited(work.whenComplete(() => _tracked.remove(work)));
  }

  /// Waits for every write asked for so far, and for those asked meanwhile, so that closing
  /// the window loses none.
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
    open.dispose();
    problem.dispose();
    for (final tree in _trees.values) {
      tree.dispose();
    }
  }
}
