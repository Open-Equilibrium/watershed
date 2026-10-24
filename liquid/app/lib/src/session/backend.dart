import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';

/// What [SessionState] needs from a running agent session; tests substitute a fake.
abstract interface class SessionBackend {
  Stream<BridgeEvent> events();
  Future<void> prompt(String message);
  Future<void> steer(String message);
  Future<void> abort();
  Future<void> setApprovalMode(BridgeApprovalMode mode);
  Future<void> decide(String request, BridgeDecision decision);
  Future<void> answerText(String request, String text);
  Future<void> answerConfirm(String request, bool confirmed);
  Future<void> dismiss(String request);
  Future<bool> shutdown();
  Future<List<BridgeFileChange>> changes();
  Future<String> patch(String path);
  Future<BridgeHistory> history(List<String> leaves);
  Future<BridgeContextDetail?> context(String entry);
  Future<void> branch(String entry);
}

/// A session in the embedded Meta-Harness core, reached through the Rust bridge.
class RustSessionBackend implements SessionBackend {
  RustSessionBackend(this._session);

  final AgentSession _session;

  @override
  Stream<BridgeEvent> events() => _session.events();
  @override
  Future<void> prompt(String message) => _session.prompt(message: message);
  @override
  Future<void> steer(String message) => _session.steer(message: message);
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
  Future<List<BridgeFileChange>> changes() => _session.changes();
  @override
  Future<String> patch(String path) => _session.patch(path: path);
  @override
  Future<BridgeHistory> history(List<String> leaves) =>
      _session.history(leaves: leaves);
  @override
  Future<BridgeContextDetail?> context(String entry) =>
      _session.context(entry: entry);
  @override
  Future<void> branch(String entry) => _session.branch(entry: entry);
}
