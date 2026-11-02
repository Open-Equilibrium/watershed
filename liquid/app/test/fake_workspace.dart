import 'dart:async';

import 'package:liquid/src/rust/api/workspace.dart';

/// An in-memory Workspace behind the bridge's calls: areas and Page trees, with a change for
/// every write to a followed resource, as the bridge sends them.
class FakeWorkspace implements LiquidWorkspace {
  final areas = <BridgeArea>[];
  final pages = <String, List<BridgePage>>{};
  final applied = <BridgeMutation>[];

  /// Fails every apply with this text.
  String? refuse;

  /// Holds applies until it completes.
  Completer<void>? gate;

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
    if (refuse case final refused?) throw refused;
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
      default:
        throw UnimplementedError('${mutation.kind}');
    }
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
    List<T> window<T>(List<T> all) =>
        all.skip(query.start).take(query.limit).toList();
    return switch (query.kind) {
      BridgeQueryKind.areas => BridgeAnswer(
        areas: window(areas),
        pages: const [],
        views: const [],
        count: areas.length,
      ),
      BridgeQueryKind.pages => BridgeAnswer(
        areas: const [],
        pages: window(pages[query.area]!),
        views: const [],
        count: pages[query.area]!.length,
      ),
      _ => throw UnimplementedError('${query.kind}'),
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
