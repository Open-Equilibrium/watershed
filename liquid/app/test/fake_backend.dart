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

  /// The files the latest run changed; none outside Git.
  List<BridgeFileChange>? changed = const [];
  Map<String, String> patches = const {};
  BridgeHistory stored = historyOf([]);

  /// Histories by the requested leaves, joined by spaces; others get [stored].
  final histories = <String, BridgeHistory>{};
  bool cancelled = false;
  final _stopped = Completer<void>();

  /// Like the bridge's stream, cancelling it finishes only with the next event, which an idle
  /// agent sends only once it is shut down.
  @override
  Stream<BridgeEvent> events() {
    controller.onCancel = () {
      cancelled = true;
      return _stopped.future;
    };
    return controller.stream;
  }

  /// What the agent rejects sends with, like the bridge does.
  String? sendError;

  /// Holds sends until it completes.
  Completer<void>? sendGate;

  @override
  Future<void> send(String message, {required bool followUp}) async {
    calls.add('${followUp ? 'follow-up' : 'send'} $message');
    await sendGate?.future;
    if (sendError case final error?) throw error;
  }

  /// What clear_queue returns.
  BridgeQueue queued = const BridgeQueue(steering: [], followUp: []);

  @override
  Future<BridgeQueue> clearQueue() async {
    calls.add('clear queue');
    return queued;
  }

  /// What aborting fails with.
  Object? abortError;

  @override
  Future<void> abort() async {
    calls.add('abort');
    if (abortError case final error?) throw error;
  }

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

  /// What reading the changes fails with.
  Object? changesError;

  /// Holds change reads until it completes.
  Completer<void>? changesGate;
  int changeReads = 0;

  /// The changes once whitespace is hidden; [changed] when null.
  List<BridgeFileChange>? changedWithoutWhitespace;

  @override
  Future<List<BridgeFileChange>?> changes({
    required bool hideWhitespace,
  }) async {
    changeReads++;
    await changesGate?.future;
    if (changesError case final error?) throw error;
    return hideWhitespace ? changedWithoutWhitespace ?? changed : changed;
  }

  /// What reading a patch fails with, like the bridge does.
  String? patchError;

  /// Holds patch reads until it completes.
  Completer<void>? patchGate;

  /// The files whose patch was read, in order.
  final patchReads = <String>[];

  @override
  Future<String> patch(String path, {required bool hideWhitespace}) async {
    patchReads.add(hideWhitespace ? '$path without whitespace' : path);
    await patchGate?.future;
    if (patchError case final error?) throw error;
    return patches[path] ?? '';
  }

  @override
  Future<BridgeHistory> history(List<String> leaves) async {
    final key = leaves.join(' ');
    calls.add('history $key'.trim());
    await gate?.future;
    if (historyError case final error?) throw error;
    return histories[key] ?? stored;
  }

  Object? historyError;

  /// Holds history reads until it completes.
  Completer<void>? gate;

  /// Whether [branch] moves the session's leaf, as a successful branch does.
  bool branchMoves = true;
  Object? branchError;

  /// The project's stored sessions, the models and the selected one, as pi reports them.
  List<BridgeStoredSession> storedSessions = const [];
  List<BridgeModel> modelList = const [];
  String? model;

  /// Whether an extension cancels session switches.
  bool switchCancelled = false;

  @override
  Future<List<BridgeStoredSession>> recentSessions() async => storedSessions;

  /// The open session's title, as pi stored it.
  String? sessionTitle;

  @override
  Future<String?> title() async => sessionTitle;

  /// Holds session switches until it completes.
  Completer<void>? switchGate;

  @override
  Future<bool> newSession() async {
    calls.add('new session');
    await switchGate?.future;
    return !switchCancelled;
  }

  @override
  Future<bool> switchSession(String path) async {
    calls.add('switch $path');
    return !switchCancelled;
  }

  @override
  Future<List<BridgeModel>> models() async => modelList;

  @override
  Future<String?> currentModel() async => model;

  @override
  Future<void> setModel(BridgeModel next) async {
    calls.add('model ${next.provider}/${next.id}');
    model = '${next.provider}/${next.id}';
  }

  /// Transcript pages by their `before` entry; the newest page has the key ''.
  final transcripts = <String, BridgeTranscript>{};

  /// What reading the chat fails with.
  Object? transcriptError;

  /// Holds reads of earlier chat pages until it completes.
  Completer<void>? earlierGate;

  @override
  Future<BridgeTranscript> transcript(String? before) async {
    calls.add('transcript ${before ?? ''}'.trim());
    if (before != null) await earlierGate?.future;
    if (transcriptError case final error?) throw error;
    return transcripts[before ?? ''] ??
        const BridgeTranscript(items: [], more: false);
  }

  /// Context details by response entry.
  final contexts = <String, BridgeContextDetail>{};
  Object? contextError;

  @override
  Future<BridgeContextDetail?> context(String entry) async {
    calls.add('context $entry');
    if (contextError case final error?) throw error;
    return contexts[entry];
  }

  /// Moves the leaf like pi: to the entry, or before it for a prompt.
  @override
  Future<void> branch(String entry) async {
    calls.add('branch $entry');
    if (branchError case final error?) throw error;
    if (branchMoves) {
      final node = stored.tree.where((node) => node.id == entry).firstOrNull;
      stored = BridgeHistory(
        tree: stored.tree,
        leaf: node?.kind == BridgeEntryKind.user ? node?.parent : entry,
        trajectories: stored.trajectories,
        usage: stored.usage,
      );
    }
  }

  @override
  Future<bool> shutdown() async {
    calls.add('shutdown');
    if (!_stopped.isCompleted) _stopped.complete();
    return true;
  }
}

BridgeEvent event(
  BridgeEventKind kind, {
  BridgeItem? item,
  BridgeStep? step,
  BridgeRequest? request,
  String? text,
  BridgeDisplay? display,
  bool? success,
  BridgeQueue? queue,
}) => BridgeEvent(
  kind: kind,
  item: item,
  step: step,
  request: request,
  text: text,
  display: display,
  success: success,
  queue: queue,
);

BridgeItem item(
  int id, {
  BridgeItemKind? kind,
  String? tool,
  bool failed = false,
  int? durationMs,
  int? added,
  int? removed,
}) => BridgeItem(
  id: id,
  kind: kind,
  tool: tool,
  failed: failed,
  durationMs: durationMs,
  added: added,
  removed: removed,
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
  String target = '',
  String? changedFile,
  BridgeDecision? decision,
  BridgeUsage? usage,
  BridgeStepContext? context,
  bool failed = false,
  BridgeToolOutcome outcome = const BridgeToolOutcome(),
  int added = 0,
}) => BridgeRecord(
  entry: entry,
  turn: turn,
  kind: kind,
  label: label,
  detail: detail,
  target: target,
  changedFile: changedFile,
  decision: decision,
  startMs: start,
  durationMs: duration,
  firstTokenMs: firstToken,
  model: kind == BridgeRecordKind.message ? 'faux-1' : '',
  usage: usage,
  stopReason: '',
  failed: failed,
  context: context,
  outcome: outcome,
  addedChars: added,
);

/// A session whose current branch is `records`, as one tree path.
BridgeHistory historyOf(
  List<BridgeRecord> records, {
  List<BridgeTreeNode>? tree,
  String? leaf,
  List<List<BridgeRecord>> others = const [],
  BridgeUsage usage = const BridgeUsage(
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    costMicroUsd: 0,
  ),
}) => BridgeHistory(
  usage: usage,
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

BridgeTranscriptItem chatItem(
  String entry,
  BridgeItemKind kind, {
  String text = '',
  String? tool,
}) => BridgeTranscriptItem(
  entry: entry,
  kind: kind,
  text: text,
  tool: tool,
  failed: false,
);
