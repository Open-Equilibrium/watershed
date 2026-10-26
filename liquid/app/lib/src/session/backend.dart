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
  Future<List<BridgeFileChange>?> changes();
  Future<String> patch(String path);
  Future<BridgeHistory> history(List<String> leaves);
  Future<BridgeContextDetail?> context(String entry);
  Future<BridgeTranscript> transcript(String? before);
  Future<void> branch(String entry);

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
  Future<List<BridgeFileChange>?> changes() => _session.changes();
  @override
  Future<String> patch(String path) => _session.patch(path: path);
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
