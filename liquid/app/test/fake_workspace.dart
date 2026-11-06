import 'dart:async';

import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

/// An in-memory Workspace behind the bridge's calls: areas and Page trees, with a change for
/// every write to a followed resource, as the bridge sends them.
class FakeWorkspace implements LiquidWorkspace {
  final areas = <BridgeArea>[];
  final pages = <String, List<BridgePage>>{};
  final applied = <BridgeMutation>[];

  /// Each Page's Views in order, and each Text Block's text and style.
  final views = <String, List<BridgeView>>{};
  final texts = <String, String>{};
  final styles = <String, BridgeTextStyle>{};

  /// Each Database Block's name and fields, and its rows in order.
  final databases = <String, BridgeDatabase>{};
  final rows = <String, List<BridgeRow>>{};

  /// Fails every apply with this text.
  String? refuse;

  /// Fails this many of the next applies.
  int refuseNext = 0;

  /// Holds applies until it completes.
  Completer<void>? gate;

  /// How long each apply takes, as a slow disk would.
  Duration Function()? delay;

  /// Every query asked, in order.
  final asked = <BridgeQuery>[];

  /// Fails every query with this text.
  String? refuseQueries;

  /// While set, each query of [deferredKinds] answers from the state at its call but waits
  /// for its own release.
  List<Completer<void>>? deferred;
  Set<BridgeQueryKind> deferredKinds = BridgeQueryKind.values.toSet();

  final _followers = <int, (BridgeResource, StreamController<BridgeChange>)>{};
  final _seqs = <BridgeResource, int>{};
  int _ids = 0;

  /// The followed resources, once each.
  Set<BridgeResource> get followed => {
    for (final (resource, _) in _followers.values) resource,
  };

  String _id() => (_ids++).toRadixString(16).padLeft(32, '0');

  @override
  Future<BridgeApplied> apply({required BridgeMutation mutation}) async {
    await gate?.future;
    if (delay case final delay?) await Future<void>.delayed(delay());
    if (refuse case final refused?) throw refused;
    if (refuseNext > 0) {
      refuseNext--;
      throw 'refused';
    }
    applied.add(mutation);
    switch (mutation.kind) {
      case BridgeMutationKind.createArea:
        final area = BridgeArea(id: _id(), name: mutation.text!);
        areas.add(area);
        pages[area.id] = [];
        _changed(const BridgeResource(kind: BridgeResourceKind.areas));
        return BridgeApplied(action: _id(), area: area.id);
      case BridgeMutationKind.createPage:
        final tree = pages[mutation.area]!;
        final page = BridgePage(
          id: _id(),
          parent: mutation.parent,
          title: mutation.text!,
        );
        // After the parent's last descendant, as the core orders a tree.
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
        return BridgeApplied(action: _id(), page: page.id);
      case BridgeMutationKind.renamePage:
        for (final MapEntry(key: area, value: tree) in pages.entries) {
          final at = tree.indexWhere((page) => page.id == mutation.page);
          if (at < 0) continue;
          final page = tree[at];
          tree[at] = BridgePage(
            id: page.id,
            parent: page.parent,
            title: mutation.text!,
          );
          _changed(BridgeResource(kind: BridgeResourceKind.pages, area: area));
        }
        return BridgeApplied(action: _id());
      case BridgeMutationKind.addTextBlock:
        final page = views.putIfAbsent(mutation.page!, () => []);
        final at = mutation.after == null
            ? 0
            : page.indexWhere((view) => view.id == mutation.after) + 1;
        final view = _block(
          mutation.page!,
          at,
          mutation.text!,
          BridgeTextStyle.paragraph,
        );
        return BridgeApplied(action: _id(), block: view.block, view: view.id);
      case BridgeMutationKind.editText:
        final text = texts[mutation.block]!;
        texts[mutation.block!] = text.replaceRange(
          mutation.at,
          mutation.at + mutation.delete,
          mutation.text!,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(action: _id());
      case BridgeMutationKind.splitTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final text = texts[mutation.block]!;
        final BridgeView created;
        if (mutation.at == 0) {
          // At the start the new empty Text Block goes above and takes the boundary.
          final shown = page[at];
          page[at] = BridgeView(
            id: shown.id,
            block: shown.block,
            boundary: false,
            kind: shown.kind,
          );
          created = _block(
            mutation.page!,
            at,
            '',
            mutation.style,
            boundary: shown.boundary,
          );
        } else {
          texts[mutation.block!] = text.substring(0, mutation.at);
          _changed(_blockResource(mutation));
          created = _block(
            mutation.page!,
            at + 1,
            text.substring(mutation.at),
            mutation.style,
          );
        }
        return BridgeApplied(
          action: _id(),
          block: created.block,
          view: created.id,
        );
      case BridgeMutationKind.joinTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        // The core joins only Text Blocks.
        if (at <= 0 || page[at - 1].kind != BridgeBlockKind.text) {
          throw 'not found';
        }
        if (page[at].boundary) throw 'a Section starts here';
        final above = page[at - 1].block;
        texts[above] = texts[above]! + texts.remove(page[at].block)!;
        page.removeAt(at);
        _changed(
          BridgeResource(
            kind: BridgeResourceKind.block,
            page: mutation.page,
            block: above,
          ),
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(action: _id());
      case BridgeMutationKind.setTextStyle:
        styles[mutation.block!] = mutation.style;
        _changed(_blockResource(mutation));
        return BridgeApplied(action: _id());
      case BridgeMutationKind.setSectionBoundary:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final shown = page[at];
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          boundary: mutation.boundary,
          kind: shown.kind,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(action: _id());
      case BridgeMutationKind.removeView:
        views[mutation.page]!.removeWhere((view) => view.id == mutation.view);
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(action: _id());
      case BridgeMutationKind.renameDatabase:
        final database = databases[mutation.block]!;
        databases[mutation.block!] = BridgeDatabase(
          name: mutation.text!,
          fields: database.fields,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(action: _id());
      case BridgeMutationKind.addDatabaseBlock:
        final page = views.putIfAbsent(mutation.page!, () => []);
        final view = BridgeView(
          id: _id(),
          block: _id(),
          boundary: false,
          kind: BridgeBlockKind.database,
        );
        databases[view.block] = BridgeDatabase(
          name: mutation.text!,
          fields: [
            BridgeField(
              id: _id(),
              name: 'Name',
              kind: BridgeFieldKind.text,
              options: const [],
            ),
          ],
        );
        rows[view.block] = [];
        page.insert(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          view,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(action: _id(), block: view.block, view: view.id);
      default:
        throw UnimplementedError('${mutation.kind}');
    }
  }

  BridgeResource _blockResource(BridgeMutation mutation) => BridgeResource(
    kind: BridgeResourceKind.block,
    page: mutation.page,
    block: mutation.block,
  );

  /// A new Text Block with its View at `at`.
  BridgeView _block(
    String page,
    int at,
    String text,
    BridgeTextStyle style, {
    bool boundary = false,
  }) {
    final view = BridgeView(
      id: _id(),
      block: _id(),
      boundary: boundary,
      kind: BridgeBlockKind.text,
    );
    texts[view.block] = text;
    styles[view.block] = style;
    views[page]!.insert(at, view);
    _changed(BridgeResource(kind: BridgeResourceKind.views, page: page));
    return view;
  }

  /// The Page's Text Blocks' texts in order.
  List<String> textsOf(String page) => [
    for (final view in views[page] ?? const <BridgeView>[])
      if (view.kind == BridgeBlockKind.text) texts[view.block]!,
  ];

  /// Puts `rows` into the Database `block`, as writes elsewhere would.
  void putRows(String page, String block, List<BridgeRow> rows) {
    this.rows[block] = rows;
    _changed(
      BridgeResource(kind: BridgeResourceKind.block, page: page, block: block),
    );
  }

  bool _below(List<BridgePage> tree, BridgePage page, String parent) {
    for (var at = page.parent; at != null;) {
      if (at == parent) return true;
      at = tree.firstWhere((page) => page.id == at).parent;
    }
    return false;
  }

  @override
  Future<BridgeAnswer> query({required BridgeQuery query}) async {
    asked.add(query);
    if (refuseQueries case final refused?) throw refused;
    final answer = _answer(query);
    if (deferred case final held? when deferredKinds.contains(query.kind)) {
      final release = Completer<void>();
      held.add(release);
      await release.future;
    }
    return answer;
  }

  BridgeAnswer _answer(BridgeQuery query) {
    List<T> window<T>(List<T> all) =>
        all.skip(query.start).take(query.limit).toList();
    return switch (query.kind) {
      BridgeQueryKind.areas => BridgeAnswer(
        areas: window(areas),
        pages: const [],
        views: const [],
        count: areas.length,
        rows: const [],
      ),
      BridgeQueryKind.pages => BridgeAnswer(
        areas: const [],
        pages: window(pages[query.area]!),
        views: const [],
        count: pages[query.area]!.length,
        rows: const [],
      ),
      BridgeQueryKind.views => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: window(views[query.page] ?? const []),
        count: views[query.page]?.length ?? 0,
        rows: const [],
      ),
      BridgeQueryKind.text => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        text: texts[query.block],
        style: styles[query.block],
        rows: const [],
      ),
      BridgeQueryKind.database => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        database: databases[query.block],
        rows: const [],
      ),
      BridgeQueryKind.rows => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: rows[query.block]?.length ?? 0,
        rows: window(rows[query.block] ?? const []),
      ),
    };
  }

  @override
  Stream<BridgeChange> subscribe({
    required int subscription,
    required BridgeResource resource,
  }) {
    final controller = StreamController<BridgeChange>();
    _followers[subscription] = (resource, controller);
    // The first change acknowledges the subscription, as the bridge's does.
    controller.add(BridgeChange(resource: resource, seq: _seqs[resource] ?? 0));
    return controller.stream;
  }

  @override
  Future<void> close({required int subscription}) async {
    await _followers.remove(subscription)?.$2.close();
  }

  void _changed(BridgeResource resource) {
    final seq = _seqs[resource] = (_seqs[resource] ?? 0) + 1;
    for (final (followed, controller) in _followers.values) {
      if (followed == resource) {
        controller.add(BridgeChange(resource: resource, seq: seq));
      }
    }
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
