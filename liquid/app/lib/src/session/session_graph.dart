import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../rust/api/history.dart';

/// One branch of the session: the path from the first entry to a tip.
@immutable
class Branch {
  const Branch({
    required this.tip,
    required this.ref,
    required this.name,
    this.durationMs,
  });

  /// The tip's entry id.
  final String tip;

  /// A, B, C… in tree order.
  final String ref;

  /// The tip's label, or the branch's latest prompt.
  final String name;
  final int? durationMs;
}

/// One row of the session tree: a step, a branch tip or the entry the next prompt continues
/// from.
@immutable
class GraphRow {
  const GraphRow({
    required this.node,
    required this.lane,
    required this.parentRow,
    required this.turn,
    required this.step,
    this.branch,
    this.pending = false,
  });

  final BridgeTreeNode node;
  final int lane;

  /// The row of the nearest shown ancestor.
  final int? parentRow;

  /// Prompts on the path up to this row.
  final int turn;

  /// The row's step number on its branch.
  final int step;

  /// The branch this row is the tip of.
  final Branch? branch;

  /// Whether the next prompt starts a new branch here.
  final bool pending;

  /// Whether the row is a step rather than a fork or tip outside the steps.
  bool get isStep => _steps.contains(node.kind);
}

/// The session tree as a graph with one lane per branch and one row per step, in depth-first
/// order like `git log --graph`.
@immutable
class SessionGraph {
  const SessionGraph(this.rows, this.branches, this.lanes, this._firstChild);

  final List<GraphRow> rows;
  final List<Branch> branches;
  final int lanes;
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
  var lanes = 0;
  // An explicit stack instead of recursion: long sessions have deep trees.
  final stack = <_Visit>[
    for (final root in roots.reversed)
      _Visit(root, null, null, 0, 0, null, null, null),
  ];
  while (stack.isNotEmpty) {
    final visit = stack.removeLast();
    final node = visit.node;
    final lane = visit.lane ?? lanes++;
    final kids = children[node.id] ?? const [];
    final isUser = node.kind == BridgeEntryKind.user;
    final isStep = _steps.contains(node.kind);
    final turn = visit.turn + (isUser ? 1 : 0);
    final step = visit.step + (isStep ? 1 : 0);
    final prompt = isUser ? node.preview : visit.prompt;
    // Durations count from the first prompt: an untimed one takes the latest time before it,
    // and without any, the first time on the path.
    final lastAt = node.atMs ?? visit.lastAt;
    final startAt = isUser && visit.turn == 0
        ? lastAt
        : visit.startAt ?? node.atMs;
    var parentRow = visit.parentRow;
    // Forks outside steps keep a row, so their branches stay connected.
    if (isStep || kids.length != 1 || node.id == leaf) {
      Branch? branch;
      if (kids.isEmpty) {
        final at = node.atMs;
        branch = Branch(
          tip: node.id,
          ref: _ref(branches.length),
          name: node.label ?? prompt ?? node.preview,
          durationMs: at == null || startAt == null
              ? null
              : math.max(0, at - startAt),
        );
        branches.add(branch);
      }
      rows.add(
        GraphRow(
          node: node,
          lane: lane,
          parentRow: parentRow,
          turn: turn,
          step: step,
          branch: branch,
          pending: kids.isNotEmpty && node.id == leaf,
        ),
      );
      parentRow = rows.length - 1;
    }
    for (var index = kids.length - 1; index >= 0; index--) {
      stack.add(
        _Visit(
          kids[index],
          index == 0 ? lane : null,
          parentRow,
          turn,
          step,
          prompt,
          startAt,
          lastAt,
        ),
      );
    }
  }
  return SessionGraph(rows, branches, math.max(lanes, 1), {
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
  const _Visit(
    this.node,
    this.lane,
    this.parentRow,
    this.turn,
    this.step,
    this.prompt,
    this.startAt,
    this.lastAt,
  );

  final BridgeTreeNode node;

  /// `null` starts a new lane when the node is visited.
  final int? lane;
  final int? parentRow;
  final int turn;
  final int step;
  final String? prompt;
  final int? startAt;

  /// The latest time on the path so far.
  final int? lastAt;
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
