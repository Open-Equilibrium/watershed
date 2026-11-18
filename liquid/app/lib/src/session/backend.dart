import 'dart:async';

import '../rust/api/details.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/moves.dart' as moves;
import '../rust/api/session.dart';
import '../rust/api/thread.dart';

class MoveConnectionLost implements Exception {
  const MoveConnectionLost(this.message);

  final String message;

  @override
  String toString() => message;
}

typedef ProjectOpener = Future<ProjectBackend> Function(
  String? address,
  String root,
);

abstract interface class CarriedMove {
  String get transfer;
}

abstract interface class ConfigurationChanges {
  List<moves.BridgeConfigChange> get changes;
}

abstract interface class ProjectBackend {
  String get root;

  Future<List<BridgeThreadSummary>> threads();

  Future<SessionBackend> start(
    String text,
    BridgeModelChoice model, {
    String? base,
    String? worktree,
    required BridgePermissions permissions,
  });
  Future<SessionBackend> open(String id);

  Future<bool> onlySessionInWorktree(String id);
  Future<void> delete(String id, {required bool worktree});
  Future<void> archive(String id, {required bool archived});

  GitBackend git();
  Future<List<BridgeProvider>> providers();
  Future<BridgeModelChoice?> writerModel();
  Future<void> setWriterModel(BridgeModelChoice model);

  Future<String> environmentName();

  Future<moves.BridgeMoveCheck> moveCheck(String id);
  Future<String> environmentId();
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  });
  Future<void> moveFinish(String id, String transfer, String? toHost);
  Future<moves.BridgeArrival> moveStatus(String transfer);
  Future<moves.BridgeArrival> moveAbort(String transfer);
  Future<moves.BridgeLanding> preflight(String root, String branch);

  Future<moves.BridgeMoveRecord?> moveRecord(String id);

  Future<bool> supportsMoves();
  Future<List<moves.BridgeHostProject>> hostProjects();
  Future<moves.BridgeHostProject> addProject(String root);
  Future<moves.BridgeHostProject> cloneProject(String remoteUrl, String path);
  Future<ConfigurationChanges> configurationChanges(ProjectBackend from);
  Future<void> bringConfiguration(
    ProjectBackend from,
    ConfigurationChanges shown,
  );
  Future<SessionBackend> moveIn(CarriedMove move, {required String root});
  Future<String> environmentMachine();

  void close();
}

abstract interface class SessionBackend {
  String get id;

  Future<BridgeSubscription> subscribe();
  Stream<BridgeEvent> events(int subscription);

  Future<void> confirmApplied(int applied);
  Future<BridgeSnapshot> snapshot();
  Future<void> send(String text, {required bool steer});
  Future<void> stop();
  Future<List<BridgeTurn>> turns();

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

  Future<String> revert(int turn, {required bool restoreFiles});

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

  GitBackend git();
  Future<void> setBranch(String branch, {String? worktree});

  Future<String> exportRecord({required String host, required String day});

  Future<List<BridgeLocalServer>> servers();
  Stream<List<BridgeLocalServer>> serverChanges();
  Future<void> stopServer(int pid, int port);

  Future<List<BridgeAutomation>> automations();
  Stream<List<BridgeAutomation>> automationChanges();
  Future<void> setAutomationEnabled(String id, {required bool enabled});
  Future<void> runAutomationNow(String id);

  Future<BridgeLineage> lineage();
  Stream<BridgeLineage> lineageChanges();
  Future<void> mergeBack();
  Future<void> disconnect();
  Future<void> stopSubagent(String id);

  Future<BridgeUsageLimits?> usageLimits();

  void close();
}

abstract interface class GitBackend {
  Future<BridgeGitStatus> status();
  Stream<BridgeGitStatus> watch();
  Future<BridgeRefs> refs(String query, {int cursor = 0, int limit = 50});
  Future<void> switchTo(String name);
  Future<void> createBranch(String name);
  Future<void> init();

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

  Future<List<BridgeFileChange>> branchChanges({required bool hideWhitespace});
  Future<String> branchPatch(String path, {required bool hideWhitespace});
  Future<BridgePullRequestDetail> pullRequest(int number);
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  });

  Future<BridgeCheckedOut> pullRequestWorktree(String url);

  void close();
}

class RustProjectBackend implements ProjectBackend {
  RustProjectBackend(this._host, this._project, {required this.root});

  final AgentHost _host;
  final AgentProject _project;
  @override
  final String root;

  @override
  Future<moves.BridgeMoveCheck> moveCheck(String id) async =>
      (await _project.open(id: id)).moveCheck();
  @override
  Future<String> environmentId() => _host.environmentId();

  @override
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  }) async {
    final package = await (await _project.open(id: id)).moveOut(
      transfer: transfer,
      targetAddress: targetAddress,
      targetName: targetName,
      targetEnvironment: targetEnvironment,
      commit: commit,
    );
    return _RustCarriedMove(package, await package.transfer());
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async =>
      (await _project.open(id: id))
          .moveFinish(transfer: transfer, toHost: toHost);
  @override
  Future<moves.BridgeArrival> moveStatus(String transfer) =>
      _host.moveStatus(transfer: transfer);
  @override
  Future<moves.BridgeArrival> moveAbort(String transfer) =>
      _host.moveAbort(transfer: transfer);
  @override
  Future<moves.BridgeLanding> preflight(String root, String branch) =>
      _host.preflight(root: root, branch: branch);
  @override
  Future<moves.BridgeMoveRecord?> moveRecord(String id) async =>
      (await _project.open(id: id)).moveRecord();
  @override
  Future<bool> supportsMoves() => _host.supportsMoves();
  @override
  Future<List<moves.BridgeHostProject>> hostProjects() => _host.hostProjects();
  @override
  Future<moves.BridgeHostProject> addProject(String root) =>
      _host.addHostProject(root: root);
  @override
  Future<moves.BridgeHostProject> cloneProject(String remoteUrl, String path) =>
      _host.cloneHostProject(remoteUrl: remoteUrl, path: path);
  @override
  Future<ConfigurationChanges> configurationChanges(ProjectBackend from) async {
    final plan = await _host.configurationChanges(
      from: (from as RustProjectBackend)._host,
    );
    return _RustConfigurationChanges(plan, await plan.changes());
  }

  @override
  Future<void> bringConfiguration(
    ProjectBackend from,
    ConfigurationChanges shown,
  ) => _host.bringConfiguration(
    from: (from as RustProjectBackend)._host,
    shown: (shown as _RustConfigurationChanges).plan,
  );
  @override
  Future<SessionBackend> moveIn(
    CarriedMove move, {
    required String root,
  }) async {
    final package = (move as _RustCarriedMove).package;
    final arrival = await _host.moveIn(root: root, package: package);
    final thread = arrival.thread;
    if (thread == null) {
      if (arrival.lost) throw MoveConnectionLost(arrival.message);
      throw arrival.message;
    }
    final project = await _host.project(root: root);
    return RustSessionBackend.of(await project.open(id: thread));
  }

  @override
  Future<String> environmentMachine() => _host.environmentMachine();

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
  @override
  void close() {
    _project.dispose();
    _host.dispose();
  }
}

class _RustConfigurationChanges implements ConfigurationChanges {
  _RustConfigurationChanges(this.plan, this.changes);

  final moves.BridgeConfigChanges plan;
  @override
  final List<moves.BridgeConfigChange> changes;
}

class _RustCarriedMove implements CarriedMove {
  _RustCarriedMove(this.package, this.transfer);

  final moves.BridgeMovePackage package;
  @override
  final String transfer;
}

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
      stopOnCancel(_session.events(id: subscription), _session.eventsStop);
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
  Stream<BridgeLineage> lineageChanges() =>
      stopOnCancel(_session.lineageEvents(), _session.lineageStop);
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
