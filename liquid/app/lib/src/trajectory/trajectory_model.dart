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

/// Where record `index` lies on the axis: milliseconds, or one slot per record with
/// `equalWidth`.
(double, double) stepExtent(
  List<BridgeRecord> records,
  int index, {
  required bool equalWidth,
}) => _extent(records, index, equalWidth: equalWidth);

/// The record at `at` on the axis, or the nearest one; none without records.
int? stepAt(List<BridgeRecord> records, double at, {required bool equalWidth}) {
  int? nearest;
  var distance = double.infinity;
  for (var index = 0; index < records.length; index++) {
    final (start, end) = _extent(records, index, equalWidth: equalWidth);
    final away = at < start
        ? start - at
        : at > end
        ? at - end
        : 0.0;
    if (away < distance) {
      nearest = index;
      distance = away;
    }
  }
  return nearest;
}

/// What the Trajectory's filters let through (approved navigation mockup): kinds of step, tool
/// names, failed steps only and text; empty lets everything through.
@immutable
class StepFilter {
  const StepFilter({
    this.kinds = const {},
    this.tools = const {},
    this.failed = false,
    this.text = '',
  });

  final Set<BridgeRecordKind> kinds;
  final Set<String> tools;
  final bool failed;
  final String text;

  /// How many filters are on.
  int get count =>
      kinds.length +
      tools.length +
      (failed ? 1 : 0) +
      (text.trim().isEmpty ? 0 : 1);

  bool matches(BridgeRecord record) {
    final text = this.text.trim().toLowerCase();
    return (kinds.isEmpty || kinds.contains(record.kind)) &&
        (tools.isEmpty ||
            record.kind == BridgeRecordKind.tool &&
                tools.contains(record.label)) &&
        (!failed || record.failed) &&
        (text.isEmpty ||
            '${record.label} ${record.target} ${record.detail}'
                .toLowerCase()
                .contains(text));
  }

  StepFilter copyWith({
    Set<BridgeRecordKind>? kinds,
    Set<String>? tools,
    bool? failed,
    String? text,
  }) => StepFilter(
    kinds: kinds ?? this.kinds,
    tools: tools ?? this.tools,
    failed: failed ?? this.failed,
    text: text ?? this.text,
  );
}

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

/// A tool call as `tool · target`, such as `bash · npm test`.
String toolSummary(BridgeRecord record) =>
    record.target.isEmpty ? record.label : '${record.label} · ${record.target}';

/// How a part of a tool call's outcome reads; views give each tone its color.
enum Tone { good, bad, muted, faint, plain }

/// A tool call's outcome as parts with their tones, such as `+9 −2` or `exit 0 · 14 ✓`; empty
/// when the stored result shows none.
List<(String, Tone)> outcomeParts(BridgeToolOutcome outcome) {
  final groups = [
    if (outcome.added != null || outcome.removed != null)
      lineParts(outcome.added, outcome.removed),
    if (outcome.exitCode case final code?)
      [('exit $code', code == 0 ? Tone.muted : Tone.bad)],
    if (outcome.testsPassed case final passed?)
      [
        ('$passed ✓', Tone.good),
        if (outcome.testsFailed case final failed? when failed > 0)
          (' $failed ✗', Tone.bad),
      ],
  ];
  return [
    for (final (index, group) in groups.indexed) ...[
      if (index > 0) (' · ', Tone.faint),
      ...group,
    ],
  ];
}

/// Changed lines as `+added −removed`, leaving out a missing count.
List<(String, Tone)> lineParts(int? added, int? removed) => [
  if (added != null) ('+$added', Tone.good),
  if (added != null && removed != null) (' ', Tone.plain),
  if (removed != null) ('−$removed', Tone.bad),
];

/// A tool call's outcome as text; `null` when the stored result shows none.
String? outcomeText(BridgeToolOutcome outcome) {
  final parts = outcomeParts(outcome);
  return parts.isEmpty ? null : parts.map((part) => part.$1).join();
}

/// `total` split in proportion to `parts`, such as a request's tokens by the characters of
/// its system prompt, tools and messages.
List<int> shares(int total, List<int> parts) {
  final sum = parts.fold(0, (sum, part) => sum + part);
  return [
    for (final part in parts) sum == 0 ? 0 : (total * part / sum).round(),
  ];
}

/// Paths that successful file changes changed on a branch.
Set<String> changedFiles(List<BridgeRecord> records) => {
  for (final record in records) ?record.changedFile,
};

/// Whether `path` looks like a test file.
bool isTestPath(String path) {
  final lower = path.toLowerCase();
  return RegExp(r'(^|[/_.\-])(tests?|spec|__tests__)([/_.\-]|$)')
      .hasMatch(lower);
}

/// The outcome of the branch's last test run.
BridgeToolOutcome? lastTests(List<BridgeRecord> records) => records.reversed
    .map((record) => record.outcome)
    .where((outcome) => outcome.testsPassed != null)
    .firstOrNull;

/// Where the current branch leaves the others, as the chat marks it.
@immutable
class BranchMarker {
  const BranchMarker({
    required this.fork,
    required this.ref,
    required this.lane,
    required this.step,
    required this.other,
    required this.otherMore,
  });

  /// The entry after which the branches part.
  final String fork;

  /// The current branch's letter, or `null` while its first prompt is still to come.
  final String? ref;
  final int lane;

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
    while (id != null) {
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
    lane: current?.lane ?? 0,
    step: step,
    other: other?.ref,
    otherMore: other == null ? 0 : stepAt(other.tip) - step,
  );
}
