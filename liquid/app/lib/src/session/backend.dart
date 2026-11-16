import 'dart:async';

import '../rust/api/details.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/session.dart';
import '../rust/api/thread.dart';

/// What Liquid needs from a project on Meta-Harness's agent host; tests substitute a fake.
abstract interface class ProjectBackend {
  /// The project's sessions, newest first.
  Future<List<BridgeThreadSummary>> threads();

  /// Starts a session: in a new worktree from `base`, in the existing `worktree`, or else in
  /// the project folder, as T3's workspace selector chose.
  Future<SessionBackend> start(
    String text,
    BridgeModelChoice model, {
    String? base,
    String? worktree,
    required BridgePermissions permissions,
  });
  Future<SessionBackend> open(String id);

  /// Whether `id` is the only session in its worktree, as T3 asks before deleting it.
  Future<bool> onlySessionInWorktree(String id);
  Future<void> delete(String id, {required bool worktree});
  Future<void> archive(String id, {required bool archived});

  /// Git for the project folder.
  GitBackend git();
  Future<List<BridgeProvider>> providers();
  Future<BridgeModelChoice?> writerModel();
  Future<void> setWriterModel(BridgeModelChoice model);

  /// The name T3 shows for this host, such as the computer's name.
  Future<String> environmentName();
}

/// What Liquid needs from one session.
abstract interface class SessionBackend {
  String get id;

  /// The session now, with the id that [events] streams the events after it by (ADR-0290).
  Future<BridgeSubscription> subscribe();
  Stream<BridgeEvent> events(int subscription);

  /// Confirms that the first [applied] events were applied, so the bridge may send more
  /// (ADR-0272).
  Future<void> confirmApplied(int applied);
  Future<BridgeSnapshot> snapshot();
  Future<void> send(String text, {required bool steer});
  Future<void> stop();
  Future<List<BridgeTurn>> turns();

  /// The files that changed from the start of turn `from` to the end of turn `to`.
  Future<List<BridgeFileChange>> turnChanges(
    int from,
    int to, {
    required bool hideWhitespace,
  });
  Future<String> turnPatch(
    int from,
    int to,
    String path, {
    required bool hideWhitespace,
  });

  /// T3's "Edit from here"; returns the turn's prompt.
  Future<String> revert(int turn, {required bool restoreFiles});

  /// T3's "Fork from this response"; returns the new session's id.
  Future<String> fork(int turn);
  Future<void> respond(String request, String decision);
  Future<void> answer(String request, List<BridgeQuestionAnswer> answers);
  Future<void> dismiss(String request);
  Future<BridgeStatistics> statistics();
  Future<BridgeContext?> context();
  Future<BridgeModelChoice?> model();
  Future<void> setModel(BridgeModelChoice model);
  Future<void> setPermissions(BridgePermissions permissions);
  Future<void> sendQueuedNow(int turn);
  Future<void> editQueued(int turn, String text);
  Future<void> cancelQueued(int turn);
  Future<String> title();
  Future<String?> worktree();

  /// Git for the session's checkout.
  GitBackend git();
  Future<void> setBranch(String branch, {String? worktree});

  /// Writes the session's record into Downloads; returns the folder.
  Future<String> exportRecord({required String host, required String day});

  /// The servers the session's agent or terminals run, now and as they change.
  Future<List<BridgeLocalServer>> servers();
  Stream<List<BridgeLocalServer>> serverChanges();
  Future<void> stopServer(int pid, int port);

  /// The session's scheduled tasks, now and as they change.
  Future<List<BridgeAutomation>> automations();
  Stream<List<BridgeAutomation>> automationChanges();
  Future<void> setAutomationEnabled(String id, {required bool enabled});
  Future<void> runAutomationNow(String id);

  /// The session's related threads (T3's Lineage and Side chats), now and as they change.
  Future<BridgeLineage> lineage();
  Stream<BridgeLineage> lineageChanges();
  Future<void> mergeBack();
  Future<void> disconnect();
  Future<void> stopSubagent(String id);

  /// The provider's usage limits, if it reports any.
  Future<BridgeUsageLimits?> usageLimits();

  /// Releases the session once Liquid left it, which ends its follows on T3.
  void close();
}

/// Git for one checkout, as T3 runs it.
abstract interface class GitBackend {
  Future<BridgeGitStatus> status();
  Stream<BridgeGitStatus> watch();
  Future<BridgeRefs> refs(String query, {int cursor = 0, int limit = 50});
  Future<void> switchTo(String name);
  Future<void> createBranch(String name);
  Future<void> init();

  /// Pulls; returns T3's title and description.
  Future<(String, String)> pull();
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  });
  Future<List<BridgeFileChange>> uncommittedChanges({
    required bool hideWhitespace,
  });
  Future<String> uncommittedPatch(String path, {required bool hideWhitespace});
  Future<BridgeResolvedPullRequest> resolvePullRequest(String reference);
  Future<BridgeCheckedOut> checkOutPullRequest(
    String reference, {
    required bool worktree,
  });
  Future<void> publish(
    String provider,
    String repository, {
    required bool public,
  });

  /// The branch compared with its base, as T3's "Changes vs" row.
  Future<List<BridgeFileChange>> branchChanges({required bool hideWhitespace});
  Future<String> branchPatch(String path, {required bool hideWhitespace});
  Future<BridgePullRequestDetail> pullRequest(int number);
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  });

  /// Checks out the pull request at `url` in a worktree, as T3's Fix and Resolve do.
  Future<BridgeCheckedOut> pullRequestWorktree(String url);

  /// Releases the checkout once Liquid no longer shows it, which ends its status watch.
  void close();
}

/// The project on Meta-Harness's T3 server, reached through the Rust bridge.
class RustProjectBackend implements ProjectBackend {
  RustProjectBackend(this._host, this._project);

  final AgentHost _host;
  final AgentProject _project;

  @override
  Future<List<BridgeThreadSummary>> threads() => _project.threads();
  @override
  Future<SessionBackend> start(
    String text,
    BridgeModelChoice model, {
    String? base,
    String? worktree,
    required BridgePermissions permissions,
  }) async => RustSessionBackend.of(
    await _project.start(
      text: text,
      model: model,
      base: base,
      worktree: worktree,
      permissions: permissions,
    ),
  );
  @override
  Future<SessionBackend> open(String id) async =>
      RustSessionBackend.of(await _project.open(id: id));
  @override
  Future<bool> onlySessionInWorktree(String id) =>
      _project.onlySessionInWorktree(id: id);
  @override
  Future<void> delete(String id, {required bool worktree}) =>
      _project.delete(id: id, worktree: worktree);
  @override
  Future<void> archive(String id, {required bool archived}) =>
      _project.archive(id: id, archived: archived);
  @override
  GitBackend git() => RustGitBackend(_project.git());
  @override
  Future<List<BridgeProvider>> providers() => _host.providers();
  @override
  Future<BridgeModelChoice?> writerModel() => _host.writerModel();
  @override
  Future<void> setWriterModel(BridgeModelChoice model) =>
      _host.setWriterModel(choice: model);
  @override
  Future<String> environmentName() => _host.environmentName();
}

/// One session on the agent host, reached through the Rust bridge.
class RustSessionBackend implements SessionBackend {
  RustSessionBackend._(this._session, this.id);

  static Future<RustSessionBackend> of(AgentSession session) async =>
      RustSessionBackend._(session, await session.id());

  final AgentSession _session;
  @override
  final String id;

  @override
  Future<BridgeSubscription> subscribe() => _session.subscribe();
  @override
  Stream<BridgeEvent> events(int subscription) =>
      _session.events(id: subscription);
  @override
  Future<void> confirmApplied(int applied) =>
      _session.confirmApplied(applied: applied);
  @override
  Future<BridgeSnapshot> snapshot() => _session.snapshot();
  @override
  Future<void> send(String text, {required bool steer}) =>
      _session.send(text: text, steer: steer);
  @override
  Future<void> stop() => _session.stop();
  @override
  Future<List<BridgeTurn>> turns() => _session.turns();
  @override
  Future<List<BridgeFileChange>> turnChanges(
    int from,
    int to, {
    required bool hideWhitespace,
  }) =>
      _session.turnChanges(from: from, to: to, hideWhitespace: hideWhitespace);
  @override
  Future<String> turnPatch(
    int from,
    int to,
    String path, {
    required bool hideWhitespace,
  }) => _session.turnPatch(
    from: from,
    to: to,
    path: path,
    hideWhitespace: hideWhitespace,
  );
  @override
  Future<String> revert(int turn, {required bool restoreFiles}) =>
      _session.revert(turn: turn, restoreFiles: restoreFiles);
  @override
  Future<String> fork(int turn) => _session.fork(turn: turn);
  @override
  Future<void> respond(String request, String decision) =>
      _session.respond(request: request, decision: decision);
  @override
  Future<void> answer(String request, List<BridgeQuestionAnswer> answers) =>
      _session.answer(request: request, answers: answers);
  @override
  Future<void> dismiss(String request) => _session.dismiss(request: request);
  @override
  Future<BridgeStatistics> statistics() => _session.statistics();
  @override
  Future<BridgeContext?> context() => _session.context();
  @override
  Future<BridgeModelChoice?> model() => _session.model();
  @override
  Future<void> setModel(BridgeModelChoice model) =>
      _session.setModel(model: model);
  @override
  Future<void> setPermissions(BridgePermissions permissions) =>
      _session.setPermissions(permissions: permissions);
  @override
  Future<void> sendQueuedNow(int turn) => _session.sendQueuedNow(turn: turn);
  @override
  Future<void> editQueued(int turn, String text) =>
      _session.editQueued(turn: turn, text: text);
  @override
  Future<void> cancelQueued(int turn) => _session.cancelQueued(turn: turn);
  @override
  Future<String> title() => _session.title();
  @override
  Future<String?> worktree() => _session.worktree();
  @override
  GitBackend git() => RustGitBackend(_session.git());
  @override
  Future<void> setBranch(String branch, {String? worktree}) =>
      _session.setBranch(branch: branch, worktree: worktree);
  @override
  Future<String> exportRecord({required String host, required String day}) =>
      _session.exportRecord(host: host, day: day);
  @override
  Future<List<BridgeLocalServer>> servers() => _session.serversSubscribe();
  @override
  Stream<List<BridgeLocalServer>> serverChanges() =>
      stopOnCancel(_session.serversEvents(), _session.serversStop);
  @override
  Future<void> stopServer(int pid, int port) =>
      _session.stopServer(pid: pid, port: port);
  @override
  Future<List<BridgeAutomation>> automations() =>
      _session.automationsSubscribe();
  @override
  Stream<List<BridgeAutomation>> automationChanges() =>
      stopOnCancel(_session.automationsEvents(), _session.automationsStop);
  @override
  Future<void> setAutomationEnabled(String id, {required bool enabled}) =>
      _session.setAutomationEnabled(id: id, enabled: enabled);
  @override
  Future<void> runAutomationNow(String id) => _session.runAutomationNow(id: id);
  @override
  Future<BridgeLineage> lineage() => _session.lineageSubscribe();
  @override
  Stream<BridgeLineage> lineageChanges() => _session.lineageEvents();
  @override
  Future<void> mergeBack() => _session.mergeBack();
  @override
  Future<void> disconnect() => _session.disconnect();
  @override
  Future<void> stopSubagent(String id) => _session.stopSubagent(id: id);
  @override
  Future<BridgeUsageLimits?> usageLimits() => _session.usageLimits();
  @override
  void close() => _session.dispose();
}

/// Git through the Rust bridge; the checkout arrives from the bridge's worker threads.
class RustGitBackend implements GitBackend {
  RustGitBackend(this._git);

  final Future<GitCheckout> _git;

  @override
  Future<BridgeGitStatus> status() async => (await _git).status();
  @override
  Stream<BridgeGitStatus> watch() => stopOnCancel(
    Stream.fromFuture(_git).asyncExpand((git) => git.watch()),
    () async => (await _git).watchStop(),
  );
  @override
  Future<BridgeRefs> refs(
    String query, {
    int cursor = 0,
    int limit = 50,
  }) async => (await _git).refs(query: query, cursor: cursor, limit: limit);
  @override
  Future<void> switchTo(String name) async => (await _git).switch_(name: name);
  @override
  Future<void> createBranch(String name) async =>
      (await _git).createBranch(name: name);
  @override
  Future<void> init() async => (await _git).init();
  @override
  Future<(String, String)> pull() async {
    final result = await (await _git).pull();
    return (result.first, result.last);
  }

  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) => Stream.fromFuture(_git).asyncExpand(
    (git) => git.run(
      action: action,
      message: message,
      files: files,
      featureBranch: featureBranch,
    ),
  );
  @override
  Future<List<BridgeFileChange>> uncommittedChanges({
    required bool hideWhitespace,
  }) async => (await _git).uncommittedChanges(hideWhitespace: hideWhitespace);
  @override
  Future<String> uncommittedPatch(
    String path, {
    required bool hideWhitespace,
  }) async =>
      (await _git).uncommittedPatch(path: path, hideWhitespace: hideWhitespace);
  @override
  Future<BridgeResolvedPullRequest> resolvePullRequest(
    String reference,
  ) async => (await _git).resolvePullRequest(reference: reference);
  @override
  Future<BridgeCheckedOut> checkOutPullRequest(
    String reference, {
    required bool worktree,
  }) async => (await _git).checkOutPullRequest(
    reference: reference,
    worktree: worktree,
  );
  @override
  Future<void> publish(
    String provider,
    String repository, {
    required bool public,
  }) async => (await _git).publish(
    provider: provider,
    repository: repository,
    public: public,
  );
  @override
  Future<List<BridgeFileChange>> branchChanges({
    required bool hideWhitespace,
  }) async => (await _git).branchFiles(hideWhitespace: hideWhitespace);
  @override
  Future<String> branchPatch(
    String path, {
    required bool hideWhitespace,
  }) async =>
      (await _git).branchPatch(path: path, hideWhitespace: hideWhitespace);
  @override
  Future<BridgePullRequestDetail> pullRequest(int number) async =>
      (await _git).pullRequest(number: number);
  @override
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  }) async => (await _git).runPullRequestAction(
    number: number,
    action: action,
    mergeMethod: mergeMethod,
  );
  @override
  Future<BridgeCheckedOut> pullRequestWorktree(String url) async =>
      (await _git).pullRequestWorktree(url: url);
  @override
  void close() => unawaited(_git.then((git) => git.dispose()));
}

/// [changes], which calls [stop] as its listener cancels. The bridge notices a cancel only at its
/// next change, so [stop] ends the bridge's follow at once. It is called within the cancel, ahead
/// of anything the cancel then releases.
Stream<T> stopOnCancel<T>(Stream<T> changes, Future<void> Function() stop) {
  late final StreamSubscription<T> subscription;
  final controller = StreamController<T>(
    onCancel: () => Future.wait([subscription.cancel(), stop()]),
  );
  subscription = changes.listen(
    controller.add,
    onError: controller.addError,
    onDone: controller.close,
  );
  return controller.stream;
}
