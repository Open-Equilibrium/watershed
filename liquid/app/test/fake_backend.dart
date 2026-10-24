import 'dart:async';
import 'dart:math' as math;

import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/backend.dart';

/// Records commands and lets tests emit events.
class FakeBackend implements SessionBackend {
  final controller = StreamController<BridgeEvent>();
  final calls = <String>[];
  List<BridgeFileChange> changed = const [];
  Map<String, String> patches = const {};
  BridgeHistory stored = historyOf([]);

  /// Histories by the requested leaves, joined by spaces; others get [stored].
  final histories = <String, BridgeHistory>{};
  bool cancelled = false;

  @override
  Stream<BridgeEvent> events() {
    controller.onCancel = () => cancelled = true;
    return controller.stream;
  }

  @override
  Future<void> prompt(String message) async => calls.add('prompt $message');
  @override
  Future<void> steer(String message) async => calls.add('steer $message');
  @override
  Future<void> abort() async => calls.add('abort');
  @override
  Future<void> setApprovalMode(BridgeApprovalMode mode) async =>
      calls.add('mode ${mode.name}');
  @override
  Future<void> decide(String request, BridgeDecision decision) async =>
      calls.add('decide $request ${decision.name}');
  @override
  Future<void> answerText(String request, String text) async =>
      calls.add('answer $request $text');
  @override
  Future<void> answerConfirm(String request, bool confirmed) async =>
      calls.add('confirm $request $confirmed');
  @override
  Future<void> dismiss(String request) async => calls.add('dismiss $request');
  @override
  Future<List<BridgeFileChange>> changes() async => changed;
  @override
  Future<String> patch(String path) async => patches[path] ?? '';
  @override
  Future<BridgeHistory> history(List<String> leaves) async {
    final key = leaves.join(' ');
    calls.add('history $key'.trim());
    await gate?.future;
    return histories[key] ?? stored;
  }

  /// Holds history reads until it completes.
  Completer<void>? gate;

  /// Whether [branch] moves the session's leaf, as a successful branch does.
  bool branchMoves = true;

  /// Moves the leaf like pi: to the entry, or before it for a prompt.
  @override
  Future<void> branch(String entry) async {
    calls.add('branch $entry');
    if (branchMoves) {
      final node = stored.tree.where((node) => node.id == entry).firstOrNull;
      stored = BridgeHistory(
        tree: stored.tree,
        leaf: node?.kind == BridgeEntryKind.user ? node?.parent : entry,
        trajectories: stored.trajectories,
      );
    }
  }

  @override
  Future<bool> shutdown() async {
    calls.add('shutdown');
    return true;
  }
}

BridgeEvent event(
  BridgeEventKind kind, {
  int? turn,
  BridgeItem? item,
  BridgeStep? step,
  BridgeRequest? request,
  String? text,
  BridgeDisplay? display,
  bool? success,
}) => BridgeEvent(
  kind: kind,
  turn: turn,
  item: item,
  step: step,
  request: request,
  text: text,
  display: display,
  success: success,
);

BridgeItem item(
  int id, {
  BridgeItemKind? kind,
  String? tool,
  bool failed = false,
  int? durationMs,
}) => BridgeItem(
  id: id,
  kind: kind,
  tool: tool,
  failed: failed,
  durationMs: durationMs,
);

BridgeUsage usage(int input, int output, int cost) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
  costMicroUsd: cost,
);

BridgeRecord record(
  String entry,
  int turn,
  BridgeRecordKind kind,
  String label,
  int start, {
  int? duration,
  int? firstToken,
  String detail = '',
  BridgeUsage? usage,
  BridgeStepContext? context,
  bool failed = false,
}) => BridgeRecord(
  entry: entry,
  turn: turn,
  kind: kind,
  label: label,
  detail: detail,
  startMs: start,
  durationMs: duration,
  firstTokenMs: firstToken,
  model: kind == BridgeRecordKind.message ? 'faux-1' : '',
  usage: usage,
  stopReason: '',
  failed: failed,
  context: context,
);

/// A session whose current branch is `records`, as one tree path.
BridgeHistory historyOf(
  List<BridgeRecord> records, {
  List<BridgeTreeNode>? tree,
  String? leaf,
  List<List<BridgeRecord>> others = const [],
}) => BridgeHistory(
  tree:
      tree ??
      [
        const BridgeTreeNode(
          id: 'root',
          kind: BridgeEntryKind.other,
          preview: '',
        ),
        for (final (index, record) in records.indexed)
          BridgeTreeNode(
            id: record.entry,
            parent: index == 0 ? 'root' : records[index - 1].entry,
            kind: switch (record.kind) {
              BridgeRecordKind.user => BridgeEntryKind.user,
              BridgeRecordKind.message => BridgeEntryKind.assistant,
              BridgeRecordKind.tool => BridgeEntryKind.toolResult,
              BridgeRecordKind.approval => BridgeEntryKind.approval,
            },
            preview: record.label,
          ),
      ],
  leaf: leaf ?? records.lastOrNull?.entry ?? 'e1',
  trajectories: [
    for (final branch in [records, ...others])
      BridgeTrajectory(
        records: branch,
        durationMs: branch.fold(
          0,
          (end, record) =>
              math.max(end, record.startMs + (record.durationMs ?? 0)),
        ),
      ),
  ],
);
