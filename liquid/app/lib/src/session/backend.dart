import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';

/// Stored entries the chat reads at a time.
const chatPage = 200;

/// How many recent sessions the start page offers.
const recentLimit = 8;

/// What [SessionState] needs from a running agent session; tests substitute a fake.
abstract interface class SessionBackend {
  Stream<BridgeEvent> events();
  Future<void> send(String message, {required bool followUp});
  Future<BridgeQueue> clearQueue();
  Future<void> abort();
  Future<void> setApprovalMode(BridgeApprovalMode mode);
  Future<void> decide(String request, BridgeDecision decision);
  Future<void> answerText(String request, String text);
  Future<void> answerConfirm(String request, bool confirmed);
  Future<void> dismiss(String request);
  Future<bool> shutdown();

  /// Whether the project is in a Git repository, so its turns can have checkpoints.
  Future<bool> isInRepository();

  /// The turns of the current branch, oldest first, with what each changed.
  Future<List<BridgeTurn>> turns();

  /// The files that changed from the start of turn `from` to the end of turn `to`.
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  });

  /// One file's unified diff from the start of turn `from` to the end of turn `to`.
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  });
  Future<BridgeHistory> history(List<String> leaves);
  Future<BridgeContextDetail?> context(String entry);
  Future<BridgeTranscript> transcript(String? before);
  Future<void> branch(String entry);

  /// What reverting the files to before turn `turn` would change.
  Future<BridgeRevertPlan> revertPlan(String turn);

  /// Reverts the files to before turn `turn` and continues the session from before its
  /// prompt, which returns to the input area.
  Future<void> revertToTurn(String turn);

  /// The sessions with checkpoints in the project's repository.
  Future<List<BridgeCheckpointSession>> checkpointSessions();

  /// Removes every checkpoint in the project's repository.
  Future<void> removeAllCheckpoints();

  /// The file pi stores the open session in, unless it keeps it in memory.
  Future<String?> sessionFile();

  /// The turns with checkpoints of the session stored at `path`.
  Future<int> storedTurns(String path);

  /// Deletes the stored session at `path`, not the open one, with its checkpoints.
  Future<void> deleteSession(String path);

  /// The project's recent stored sessions, newest first, without the current one.
  Future<List<BridgeStoredSession>> recentSessions();

  /// The open session's title, as the recent sessions are named; null until pi stored one.
  Future<String?> title();

  /// Starts a new session or continues a stored one; `false` when an extension cancelled it.
  Future<bool> newSession();
  Future<bool> switchSession(String path);

  Future<List<BridgeModel>> models();

  /// The selected model as `provider/id`.
  Future<String?> currentModel();
  Future<void> setModel(BridgeModel model);
}

/// A session in the embedded Meta-Harness core, reached through the Rust bridge.
class RustSessionBackend implements SessionBackend {
  RustSessionBackend(this._session);

  final AgentSession _session;

  @override
  Stream<BridgeEvent> events() => _session.events();
  @override
  Future<void> send(String message, {required bool followUp}) =>
      _session.send(message: message, followUp: followUp);
  @override
  Future<BridgeQueue> clearQueue() => _session.clearQueue();
  @override
  Future<void> abort() => _session.abort();
  @override
  Future<void> setApprovalMode(BridgeApprovalMode mode) =>
      _session.setApprovalMode(mode: mode);
  @override
  Future<void> decide(String request, BridgeDecision decision) =>
      _session.decide(request: request, decision: decision);
  @override
  Future<void> answerText(String request, String text) =>
      _session.answerText(request: request, text: text);
  @override
  Future<void> answerConfirm(String request, bool confirmed) =>
      _session.answerConfirm(request: request, confirmed: confirmed);
  @override
  Future<void> dismiss(String request) => _session.dismiss(request: request);
  @override
  Future<bool> shutdown() => _session.shutdown();
  @override
  Future<bool> isInRepository() => _session.inRepository();
  @override
  Future<List<BridgeTurn>> turns() => _session.turns();
  @override
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  }) =>
      _session.turnChanges(from: from, to: to, hideWhitespace: hideWhitespace);
  @override
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  }) => _session.turnPatch(
    from: from,
    to: to,
    path: path,
    hideWhitespace: hideWhitespace,
  );
  @override
  Future<BridgeHistory> history(List<String> leaves) =>
      _session.history(leaves: leaves);
  @override
  Future<BridgeTranscript> transcript(String? before) =>
      _session.transcript(before: before, limit: chatPage);
  @override
  Future<BridgeContextDetail?> context(String entry) =>
      _session.context(entry: entry);
  @override
  Future<void> branch(String entry) => _session.branch(entry: entry);
  @override
  Future<BridgeRevertPlan> revertPlan(String turn) =>
      _session.revertPlan(turn: turn);
  @override
  Future<void> revertToTurn(String turn) => _session.revertToTurn(turn: turn);
  @override
  Future<List<BridgeCheckpointSession>> checkpointSessions() =>
      _session.checkpointSessions();
  @override
  Future<void> removeAllCheckpoints() => _session.removeAllCheckpoints();
  @override
  Future<String?> sessionFile() => _session.sessionFile();
  @override
  Future<int> storedTurns(String path) async =>
      (await _session.storedTurns(path: path)).toInt();
  @override
  Future<void> deleteSession(String path) => _session.deleteSession(path: path);
  @override
  Future<List<BridgeStoredSession>> recentSessions() =>
      _session.recentSessions(limit: recentLimit);
  @override
  Future<String?> title() => _session.title();
  @override
  Future<bool> newSession() => _session.newSession();
  @override
  Future<bool> switchSession(String path) => _session.switchSession(path: path);
  @override
  Future<List<BridgeModel>> models() => _session.models();
  @override
  Future<String?> currentModel() => _session.currentModel();
  @override
  Future<void> setModel(BridgeModel model) =>
      _session.setModel(provider: model.provider, id: model.id);
}
