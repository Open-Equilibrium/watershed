import 'package:flutter/foundation.dart';

import '../rust/api/history.dart';

/// One branch of the session: the path from the first entry to a tip.
@immutable
class Branch {
  const Branch({required this.tip, required this.ref});

  /// The tip's entry id.
  final String tip;

  /// A, B, C… in tree order.
  final String ref;
}

/// One row of the session tree: a step, or a fork, tip or leaf outside the steps.
@immutable
class GraphRow {
  const GraphRow({required this.node, required this.step});

  final BridgeTreeNode node;

  /// The row's step number on its branch.
  final int step;

  /// Whether the row is a step rather than a fork, tip or leaf outside the steps.
  bool get isStep => _steps.contains(node.kind);
}

/// The session tree's rows in depth-first order, and its branches.
@immutable
class SessionGraph {
  const SessionGraph(this.rows, this.branches, this._firstChild);

  final List<GraphRow> rows;
  final List<Branch> branches;
  final Map<String, String> _firstChild;

  /// The branch reached from `entry` by following first children, so a tip that grew stays the
  /// same branch.
  Branch? branchOf(String? entry) {
    var current = entry;
    final seen = <String>{};
    while (current != null && seen.add(current)) {
      final next = _firstChild[current];
      if (next == null) break;
      current = next;
    }
    return branches.where((branch) => branch.tip == current).firstOrNull;
  }
}

SessionGraph sessionGraph(List<BridgeTreeNode> entries, String? leaf) {
  // A repeated id keeps its last entry, as the backend resolves parent ids. Then each id is one
  // row, and the walk below ends even when a stored parent leads back to its own entry.
  final nodes = {for (final entry in entries) entry.id: entry}.values.toList();
  final children = <String, List<BridgeTreeNode>>{};
  final roots = <BridgeTreeNode>[];
  final ids = {for (final node in nodes) node.id};
  for (final node in nodes) {
    final parent = node.parent;
    if (parent == null || !ids.contains(parent)) {
      roots.add(node);
    } else {
      (children[parent] ??= []).add(node);
    }
  }
  final rows = <GraphRow>[];
  final branches = <Branch>[];
  // An explicit stack instead of recursion: long sessions have deep trees.
  final stack = <_Visit>[for (final root in roots.reversed) _Visit(root, 0)];
  while (stack.isNotEmpty) {
    final visit = stack.removeLast();
    final node = visit.node;
    final kids = children[node.id] ?? const [];
    final isStep = _steps.contains(node.kind);
    final step = visit.step + (isStep ? 1 : 0);
    // Forks and tips outside steps, and the leaf, keep a row too.
    if (isStep || kids.length != 1 || node.id == leaf) {
      if (kids.isEmpty) {
        branches.add(Branch(tip: node.id, ref: _ref(branches.length)));
      }
      rows.add(GraphRow(node: node, step: step));
    }
    for (var index = kids.length - 1; index >= 0; index--) {
      stack.add(_Visit(kids[index], step));
    }
  }
  return SessionGraph(rows, branches, {
    for (final entry in children.entries) entry.key: entry.value.first.id,
  });
}

/// Tree entries that are steps.
const _steps = {
  BridgeEntryKind.user,
  BridgeEntryKind.assistant,
  BridgeEntryKind.toolResult,
};

class _Visit {
  const _Visit(this.node, this.step);

  final BridgeTreeNode node;

  /// The steps on the path above the node.
  final int step;
}

String _ref(int index) =>
    String.fromCharCode(65 + index % 26) + (index < 26 ? '' : '${index ~/ 26}');

/// Where the current branch leaves the others, as the chat marks it.
@immutable
class BranchMarker {
  const BranchMarker({
    required this.fork,
    required this.ref,
    required this.step,
    required this.other,
    required this.otherMore,
  });

  /// The entry after which the branches part.
  final String fork;

  /// The current branch's letter, or `null` while its first prompt is still to come.
  final String? ref;

  /// The step number of the fork.
  final int step;

  /// The other branch at the fork and how many steps it has beyond it.
  final String? other;
  final int otherMore;
}

/// The nearest fork on the path to `leaf`; `null` when the path never forks.
BranchMarker? branchMarker(List<BridgeTreeNode> tree, String? leaf) {
  if (leaf == null) return null;
  final byId = {for (final node in tree) node.id: node};
  final children = <String, List<String>>{};
  for (final node in tree) {
    if (node.parent case final parent?) {
      (children[parent] ??= []).add(node.id);
    }
  }
  String? fork;
  String? onPath;
  if (children[leaf]?.isNotEmpty ?? false) {
    fork = leaf;
  } else {
    var current = leaf;
    for (
      var parent = byId[current]?.parent;
      parent != null;
      parent = byId[current]?.parent
    ) {
      if ((children[parent]?.length ?? 0) > 1) {
        fork = parent;
        onPath = current;
        break;
      }
      current = parent;
    }
  }
  if (fork == null) return null;
  final graph = sessionGraph(tree, leaf);
  final rows = {for (final row in graph.rows) row.node.id: row};
  int stepAt(String? id) {
    // A stored parent cycle ends the walk at the first entry it meets again.
    final seen = <String>{};
    while (id != null && seen.add(id)) {
      final row = rows[id];
      if (row != null && row.isStep) return row.step;
      id = byId[id]?.parent;
    }
    return 0;
  }

  final step = stepAt(fork);
  final current = onPath == null ? null : graph.branchOf(leaf);
  final otherChild = children[fork]!.firstWhere((child) => child != onPath);
  final other = graph.branchOf(otherChild);
  return BranchMarker(
    fork: fork,
    ref: current?.ref,
    step: step,
    other: other?.ref,
    otherMore: other == null ? 0 : stepAt(other.tip) - step,
  );
}
