import 'dart:convert';
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
    required this.lane,
    this.durationMs,
  });

  /// The tip's entry id.
  final String tip;

  /// A, B, C… in tree order.
  final String ref;

  /// The tip's label, or the branch's latest prompt.
  final String name;
  final int lane;
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

  /// The row's step number on its branch, as the ledger counts it.
  final int step;

  /// The branch this row is the tip of.
  final Branch? branch;

  /// Whether the next prompt starts a new branch here.
  final bool pending;

  /// Whether the row is a Trajectory step rather than a fork or tip outside the steps.
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

SessionGraph sessionGraph(List<BridgeTreeNode> nodes, String? leaf) {
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
      _Visit(root, null, null, 0, 0, null, null),
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
    // Durations count from the first prompt, like the Trajectory.
    final startAt = visit.startAt ?? (isUser ? node.atMs : null);
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
          lane: lane,
          durationMs: at == null || startAt == null ? null : at - startAt,
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
        ),
      );
    }
  }
  return SessionGraph(rows, branches, math.max(lanes, 1), {
    for (final entry in children.entries) entry.key: entry.value.first.id,
  });
}

/// Tree entries that are Trajectory steps.
const _steps = {
  BridgeEntryKind.user,
  BridgeEntryKind.assistant,
  BridgeEntryKind.toolResult,
  BridgeEntryKind.approval,
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
  );

  final BridgeTreeNode node;

  /// `null` starts a new lane when the node is visited.
  final int? lane;
  final int? parentRow;
  final int turn;
  final int step;
  final String? prompt;
  final int? startAt;
}

String _ref(int index) =>
    String.fromCharCode(65 + index % 26) + (index < 26 ? '' : '${index ~/ 26}');

/// One turn of a branch: a prompt and the steps it caused.
@immutable
class TurnGroup {
  const TurnGroup({
    required this.turn,
    required this.prompt,
    required this.first,
    required this.last,
    required this.durationMs,
    required this.tools,
  });

  final int turn;
  final String prompt;

  /// Indexes of the turn's first and last record.
  final int first;
  final int last;
  final int durationMs;

  /// Tool calls per tool, in order of first use.
  final Map<String, int> tools;
}

List<TurnGroup> turnGroups(List<BridgeRecord> records) {
  final groups = <TurnGroup>[];
  var first = 0;
  for (var index = 1; index <= records.length; index++) {
    if (index < records.length && records[index].turn == records[first].turn) {
      continue;
    }
    final steps = records.sublist(first, index);
    final tools = <String, int>{};
    for (final step in steps) {
      if (step.kind == BridgeRecordKind.tool) {
        tools[step.label] = (tools[step.label] ?? 0) + 1;
      }
    }
    final start = steps.first.startMs;
    groups.add(
      TurnGroup(
        turn: steps.first.turn,
        prompt:
            steps
                .where((step) => step.kind == BridgeRecordKind.user)
                .firstOrNull
                ?.label ??
            '',
        first: first,
        last: index - 1,
        durationMs: steps.map(_end).reduce(math.max) - start,
        tools: tools,
      ),
    );
    first = index;
  }
  return groups;
}

int _end(BridgeRecord record) => record.startMs + (record.durationMs ?? 0);

/// The overview's tracks.
enum OverviewLane { model, tools, you }

/// A step's bar on an overview track.
@immutable
class Segment {
  const Segment(this.index, this.lane, this.start, this.end, this.firstToken);

  /// The record's index.
  final int index;
  final OverviewLane lane;
  final double start;
  final double end;

  /// Where a response's first token arrived; before it the model was thinking.
  final double? firstToken;
}

/// The bars of every model request, tool call and approval wait, in milliseconds or, with
/// `equalWidth`, in one slot per record.
List<Segment> segments(List<BridgeRecord> records, {required bool equalWidth}) {
  final result = <Segment>[];
  for (var index = 0; index < records.length; index++) {
    final record = records[index];
    final lane = switch (record.kind) {
      BridgeRecordKind.message => OverviewLane.model,
      BridgeRecordKind.tool => OverviewLane.tools,
      BridgeRecordKind.approval => OverviewLane.you,
      BridgeRecordKind.user => null,
    };
    if (lane == null) continue;
    final duration = record.durationMs ?? 0;
    final firstToken = record.firstTokenMs;
    final (start, end) = _extent(records, index, equalWidth: equalWidth);
    result.add(
      Segment(
        index,
        lane,
        start,
        end,
        firstToken == null
            ? null
            : equalWidth
            ? start + (duration == 0 ? 0 : firstToken / duration)
            : start + firstToken,
      ),
    );
  }
  return result;
}

(double, double) _extent(
  List<BridgeRecord> records,
  int index, {
  required bool equalWidth,
}) => equalWidth
    ? (index.toDouble(), index + 1.0)
    : (records[index].startMs.toDouble(), _end(records[index]).toDouble());

/// The length of the overview's axis.
double span(List<BridgeRecord> records, {required bool equalWidth}) =>
    equalWidth
    ? records.length.toDouble()
    : records.map(_end).fold(0, math.max).toDouble();

/// Whether each record overlaps the focused `range` of the axis.
List<bool> inRange(
  List<BridgeRecord> records,
  (double, double) range, {
  required bool equalWidth,
}) => [
  for (var index = 0; index < records.length; index++)
    switch (_extent(records, index, equalWidth: equalWidth)) {
      (final start, final end) => start <= range.$2 && end >= range.$1,
    },
];

/// Two branches side by side: their shared steps and the aligned rest.
@immutable
class Comparison {
  const Comparison(this.shared, this.rows, this.idleA, this.idleB);

  /// Leading records both branches share.
  final int shared;

  /// Aligned pairs; `null` marks a step only the other branch has.
  final List<(BridgeRecord?, BridgeRecord?)> rows;

  /// Milliseconds each branch waited at the branch point for its first own prompt; comparisons
  /// leave this human time out.
  final int idleA;
  final int idleB;
}

/// Pairs larger than this align by position instead of by content.
const _alignLimit = 250000;

Comparison compareBranches(List<BridgeRecord> a, List<BridgeRecord> b) {
  var shared = 0;
  while (shared < a.length &&
      shared < b.length &&
      a[shared].entry == b[shared].entry) {
    shared++;
  }
  final branchPoint = shared == 0 ? null : _end(a[shared - 1]);
  int idle(List<BridgeRecord> records) =>
      branchPoint == null ||
          records.length <= shared ||
          records[shared].kind != BridgeRecordKind.user
      ? 0
      : math.max(0, records[shared].startMs - branchPoint);
  final left = a.sublist(shared);
  final right = b.sublist(shared);
  final rows = <(BridgeRecord?, BridgeRecord?)>[];
  Comparison result() => Comparison(shared, rows, idle(a), idle(b));
  if (left.length * right.length > _alignLimit) {
    for (var index = 0; index < math.max(left.length, right.length); index++) {
      rows.add((
        index < left.length ? left[index] : null,
        index < right.length ? right[index] : null,
      ));
    }
    return result();
  }
  // Steps of the same kind may pair up, and the same tool or text pairs first: a weighted
  // longest common subsequence.
  int weight(BridgeRecord x, BridgeRecord y) =>
      x.kind != y.kind ? 0 : (x.label == y.label ? 3 : 2);
  final best = List.generate(
    left.length + 1,
    (_) => List.filled(right.length + 1, 0),
  );
  for (var i = left.length - 1; i >= 0; i--) {
    for (var j = right.length - 1; j >= 0; j--) {
      final pair = weight(left[i], right[j]);
      best[i][j] = math.max(
        math.max(best[i + 1][j], best[i][j + 1]),
        pair == 0 ? 0 : pair + best[i + 1][j + 1],
      );
    }
  }
  var i = 0;
  var j = 0;
  final onlyA = <BridgeRecord>[];
  final onlyB = <BridgeRecord>[];
  void flush() {
    rows
      ..addAll([for (final step in onlyA) (step, null)])
      ..addAll([for (final step in onlyB) (null, step)]);
    onlyA.clear();
    onlyB.clear();
  }

  while (i < left.length || j < right.length) {
    final pair = i < left.length && j < right.length
        ? weight(left[i], right[j])
        : 0;
    if (pair > 0 && best[i][j] == pair + best[i + 1][j + 1]) {
      flush();
      rows.add((left[i++], right[j++]));
    } else if (j >= right.length ||
        (i < left.length && best[i + 1][j] >= best[i][j + 1])) {
      onlyA.add(left[i++]);
    } else {
      onlyB.add(right[j++]);
    }
  }
  flush();
  return result();
}

String formatDuration(int ms) {
  if (ms < 1000) return '$ms ms';
  if (ms < 60000) return '${(ms / 1000).toStringAsFixed(1)} s';
  final seconds = ms ~/ 1000;
  return '${seconds ~/ 60}m ${(seconds % 60).toString().padLeft(2, '0')}s';
}

String formatCount(int count) => count.toString().replaceAllMapped(
  RegExp(r'\B(?=(\d{3})+(?!\d))'),
  (_) => ',',
);

/// A cost in millionths of a dollar; amounts under a cent keep three decimals.
String formatCost(int microUsd) =>
    '\$${(microUsd / 1e6).toStringAsFixed(microUsd < 10000 ? 3 : 2)}';

String formatCompact(int count) =>
    count >= 1000 ? '${(count / 1000).toStringAsFixed(1)}k' : '$count';

/// Tool arguments every reader understands at a glance.
const _primaryArguments = ['command', 'path', 'file_path'];

/// A tool call as `tool · main argument`, such as `bash · npm test`.
String toolSummary(String tool, String arguments) {
  Object? parsed;
  try {
    parsed = jsonDecode(arguments);
  } on FormatException {
    return tool;
  }
  if (parsed is! Map<String, Object?> || parsed.isEmpty) return tool;
  final primary = _primaryArguments
      .map((name) => parsed is Map ? parsed[name] : null)
      .whereType<String>()
      .firstOrNull;
  final text = primary ?? arguments;
  return '$tool · ${text.length > 120 ? '${text.substring(0, 119)}…' : text}';
}
