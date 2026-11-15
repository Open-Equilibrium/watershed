// Spike (throwaway): a Chat whose thread runs on a pinned T3 Code server through Meta-Harness
// (ADR-0280, ADR-0281). Only Chat, Diff, revert and the statistics are wired; everything else
// reports that the spike does not support it.

import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';
import '../rust/api/t3.dart';
import '../rust/api/worktrees.dart';
import 'backend.dart';

class T3SessionBackend implements SessionBackend {
  T3SessionBackend(this._chat, this._model);

  final T3Chat _chat;
  final String _model;

  @override
  Stream<BridgeEvent> events() => _chat.events();
  @override
  Future<void> confirmApplied(int applied) async {}
  @override
  Future<void> send(String message, {required bool followUp}) =>
      _chat.send(message: message);
  @override
  Future<void> abort() => _chat.abort();
  @override
  Future<bool> shutdown() => _chat.shutdown();
  @override
  Future<bool> isInRepository() async => true;
  @override
  Future<List<BridgeTurn>> turns() => _chat.turns();
  @override
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  }) => _chat.turnChanges(from: from, to: to, hideWhitespace: hideWhitespace);
  @override
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  }) => _chat.turnPatch(
    from: from,
    to: to,
    path: path,
    hideWhitespace: hideWhitespace,
  );
  @override
  Future<BridgeHistory> history() => _chat.history();
  @override
  Future<BridgeStatistics> statistics() => _chat.statistics();
  @override
  Future<BridgeContext?> context() => _chat.context();
  @override
  Future<BridgeTranscript> transcript(String? before) async => before == null
      ? _chat.transcript()
      : const BridgeTranscript(items: [], more: false);
  @override
  Future<BridgeRevertPlan> revertPlan(String turn) =>
      _chat.revertPlan(turn: turn);
  @override
  Future<void> revertToTurn(
    String turn,
    BridgeRevertConfirmation confirmation,
  ) => _chat.revertToTurn(turn: turn);
  @override
  Future<List<BridgeCheckpointSession>> checkpointSessions() async => const [];
  @override
  Future<String?> sessionFile() async => null;
  @override
  Future<List<BridgeStoredSession>> recentSessions() async => const [];
  @override
  Future<String?> title() async => null;
  @override
  // The thread works in its own T3 worktree; the spike reports it as a clean Git checkout so the
  // session card offers Changes and the Diff.
  Future<BridgeGitStatus?> gitStatus() async => const BridgeGitStatus(
    branch: 't3',
    hasRemotes: false,
    gone: false,
    ahead: 0,
    behind: 0,
    changed: 0,
    merging: false,
    conflicts: [],
    repository: '',
  );
  @override
  Future<List<BridgeWorktree>> worktrees() async => const [];
  @override
  Future<List<BridgeModel>> models() async => const [];
  @override
  Future<String?> currentModel() async => _model;
  @override
  Future<bool> newSession() async => true;

  @override
  dynamic noSuchMethod(Invocation invocation) =>
      throw UnsupportedError('The T3 spike does not support this yet.');
}
