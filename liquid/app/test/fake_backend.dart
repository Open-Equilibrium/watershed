import 'dart:async';

import 'package:liquid/src/rust/api/details.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/backend.dart';

const fakeEnvironment = 'env-fake';

class FakeProject implements ProjectBackend {
  FakeProject({FakeSession? session, FakeGit? git})
    : session = session ?? FakeSession(),
      gitBackend = git ?? FakeGit();

  FakeSession session;
  String environment = fakeEnvironment;
  String? moveOutEnvironment;
  FakeGit gitBackend;
  final calls = <String>[];

  List<BridgeThreadSummary> threadList = [];
  List<BridgeProvider> providerList = [providerOf()];
  BridgeModelChoice? writer;
  bool onlyInWorktree = false;

  Object? openError;

  Completer<void>? hold;

  Object? onlyInWorktreeError;

  @override
  String root = '/work/acme-api';

  BridgeMoveCheck moveCheckAnswer = moveCheckOf();
  Object? moveCheckError;
  Map<String, BridgeLanding> landings = {};

  @override
  Future<BridgeLanding> preflight(String root, String branch) async {
    calls.add('preflight $root $branch');
    return landings[root] ??
        const BridgeLanding(kind: BridgeLandingKind.newWorktree);
  }

  Object? moveOutError;
  Object? cloneError;
  Object? moveInError;
  String? clonedRoot;
  List<BridgeHostProject> projectList = [];
  Object? hostProjectsError;
  bool movesSupported = true;
  BridgeMoveRecord? moveRecordAnswer;
  List<BridgeConfigChange> configChanges = [];

  BridgeHostProject? addedProject;
  Object? addProjectError;

  @override
  Future<List<BridgeThreadSummary>> threads() async => threadList;

  @override
  Future<SessionBackend> start(
    String text,
    BridgeModelChoice model, {
    String? base,
    String? worktree,
    required BridgePermissions permissions,
  }) async {
    calls.add(
      'start $text ${model.model} base=$base worktree=$worktree ${permissions.name}',
    );
    final gate = hold;
    final next = session;
    if (openError case final error?) throw error;
    await gate?.future;
    return next;
  }

  @override
  Future<SessionBackend> open(String id) async {
    calls.add('open $id');
    final gate = hold;
    final next = session;
    if (openError case final error?) throw error;
    await gate?.future;
    return next;
  }

  @override
  Future<bool> onlySessionInWorktree(String id) async {
    if (onlyInWorktreeError case final error?) throw error;
    return onlyInWorktree;
  }

  @override
  Future<void> delete(String id, {required bool worktree}) async =>
      calls.add('delete $id worktree=$worktree');

  @override
  Future<BridgeMoveCheck> moveCheck(String id) async {
    calls.add('move check $id');
    if (moveCheckError case final error?) throw error;
    return moveCheckWith(
      moveCheckAnswer,
      moved: moveCheckAnswer.moved || _sealed,
      moving: moving,
    );
  }

  bool moveOutLostAfterHandover = false;
  BridgeMoving? moving;
  bool _sealed = false;

  @override
  Future<String> environmentId() async => environment;

  @override
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  }) async {
    moveOutEnvironment = targetEnvironment;
    calls.add('move out $id commit=$commit');
    if (moveOutLostAfterHandover) {
      moving = _movingOf(transfer, targetName, targetEnvironment);
      throw moveOutError ??
          'Meta-Harness closed the connection before it answered.';
    }
    if (moveOutError case final error?) throw error;
    moving = _movingOf(transfer, targetName, targetEnvironment);
    return FakeCarriedMove(transfer);
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async {
    calls.add('move finish $id $transfer ${toHost ?? 'null'}');
    if (toHost != null) _sealed = true;
    moving = null;
    threadList = [
      for (final thread in threadList)
        thread.moving == null
            ? thread
            : threadOf(thread.id, title: thread.title),
    ];
  }

  @override
  Future<BridgeArrival> moveAbort(String transfer) async {
    calls.add('move abort $transfer');
    final landed = landedThread;
    if (landed != null && undelivered) {
      return const BridgeArrival(kind: BridgeArrivalKind.starting);
    }
    if (landed != null) {
      return BridgeArrival(kind: BridgeArrivalKind.landed, thread: landed);
    }
    return const BridgeArrival(kind: BridgeArrivalKind.refused);
  }

  @override
  Future<BridgeMoveRecord?> moveRecord(String id) async => moveRecordAnswer;

  @override
  Future<bool> supportsMoves() async => movesSupported;

  @override
  Future<List<BridgeHostProject>> hostProjects() async {
    if (hostProjectsError case final error?) throw error;
    return projectList;
  }

  @override
  Future<BridgeHostProject> addProject(String root) async {
    calls.add('add project $root');
    if (addProjectError case final error?) throw error;
    return addedProject ??
        BridgeHostProject(
          root: root,
          title: root,
          remoteUrl: 'git@github.com:acme/api.git',
        );
  }

  @override
  Future<BridgeHostProject> cloneProject(String remoteUrl, String path) async {
    calls.add('clone $remoteUrl $path');
    if (cloneError case final error?) throw error;
    return BridgeHostProject(
      root: clonedRoot ?? path,
      title: path,
      remoteUrl: remoteUrl,
    );
  }

  @override
  Future<SessionBackend> moveIn(
    CarriedMove move, {
    required String root,
  }) async {
    calls.add('move in $root');
    if (moveInError case final error?) throw error;
    if (moveInLostReplies > 0) {
      moveInLostReplies--;
      if (lostRepliesLanded) landedThread = session.id;
      throw MoveConnectionLost(
        'Meta-Harness closed the connection before it answered.',
      );
    }
    landedThread = session.id;
    return session;
  }

  int moveInLostReplies = 0;
  bool lostRepliesLanded = false;
  String? landedThread;

  @override
  Future<BridgeArrival> moveStatus(String transfer) async {
    calls.add('move status $transfer');
    final landed = landedThread;
    if (landed != null && undelivered) {
      return const BridgeArrival(kind: BridgeArrivalKind.starting);
    }
    if (landed != null) {
      return BridgeArrival(kind: BridgeArrivalKind.landed, thread: landed);
    }
    if (startingThread) {
      return const BridgeArrival(kind: BridgeArrivalKind.starting);
    }
    return const BridgeArrival(kind: BridgeArrivalKind.unknown);
  }

  bool startingThread = false;
  bool undelivered = false;

  @override
  Future<ConfigurationChanges> configurationChanges(
    ProjectBackend from,
  ) async => FakeConfigurationChanges(configChanges);

  ConfigurationChanges? broughtChanges;

  @override
  Future<void> bringConfiguration(
    ProjectBackend from,
    ConfigurationChanges shown,
  ) async {
    broughtChanges = shown;
    calls.add('bring configuration');
  }

  @override
  Future<void> archive(String id, {required bool archived}) async =>
      calls.add('archive $id $archived');

  @override
  GitBackend git() => gitBackend;

  @override
  Future<List<BridgeProvider>> providers() async => providerList;

  @override
  Future<BridgeModelChoice?> writerModel() async => writer;

  @override
  Future<void> setWriterModel(BridgeModelChoice model) async {
    calls.add('writer ${model.model}');
    writer = model;
  }

  String hostName = 'Mac mini';

  @override
  Future<String> environmentName() async => hostName;

  bool closed = false;

  @override
  void close() => closed = true;

  @override
  Future<String> environmentMachine() async => 'server';
}

class UnreachableProject extends FakeProject {
  @override
  Future<List<BridgeThreadSummary>> threads() =>
      Future.error('the host is down');
}

class FakeSession implements SessionBackend {
  FakeSession({this.id = 'thread-1'});

  @override
  final String id;
  final calls = <String>[];
  final _events = StreamController<BridgeEvent>.broadcast();

  BridgeSnapshot current = snapshotOf();
  List<BridgeTurn> turnList = [];
  List<BridgeFileChange> changes = [];
  Map<String, String> patches = {};
  BridgeStatistics stats = statisticsOf();
  BridgeContext? contextWindow;
  BridgeModelChoice? chosen;
  String sessionTitle = 'Session';
  FakeGit gitBackend = FakeGit();

  String revertedPrompt = '';

  Object? sendError;

  Object? permissionsError;
  Object? modelError;

  final confirmed = <int>[];

  void emit(BridgeEvent event) => _events.add(event);

  void fail(Object error) => _events.addError(error);

  bool closed = false;

  @override
  void close() => closed = true;

  @override
  Future<BridgeSubscription> subscribe() async =>
      BridgeSubscription(id: 1, snapshot: current);

  @override
  Stream<BridgeEvent> events(int subscription) {
    late final StreamSubscription<BridgeEvent> source;
    late final StreamController<BridgeEvent> controller;
    controller = StreamController(
      onListen: () => source = _events.stream.listen(
        controller.add,
        onError: controller.addError,
      ),
      onCancel: () async => unawaited(source.cancel()),
    );
    return controller.stream;
  }

  @override
  Future<void> confirmApplied(int applied) async => confirmed.add(applied);
  @override
  Future<BridgeSnapshot> snapshot() async => current;

  @override
  Future<void> send(String text, {required bool steer}) async {
    calls.add('${steer ? 'steer' : 'send'} $text');
    if (sendError case final error?) throw error;
  }

  @override
  Future<void> stop() async => calls.add('stop');
  @override
  Future<List<BridgeTurn>> turns() async => turnList;

  @override
  Future<List<BridgeFileChange>> turnChanges(
    int from,
    int to, {
    required bool hideWhitespace,
  }) async {
    calls.add('changes $from..$to');
    return changes;
  }

  @override
  Future<String> turnPatch(
    int from,
    int to,
    String path, {
    required bool hideWhitespace,
  }) async => patches[path] ?? '';

  @override
  Future<String> revert(int turn, {required bool restoreFiles}) async {
    calls.add('revert $turn files=$restoreFiles');
    return revertedPrompt;
  }

  @override
  Future<String> fork(int turn) async {
    calls.add('fork $turn');
    return 'fork-of-$id';
  }

  @override
  Future<void> respond(String request, String decision) async =>
      calls.add('respond $request $decision');
  @override
  Future<void> answer(
    String request,
    List<BridgeQuestionAnswer> answers,
  ) async => calls.add(
    'answer $request ${[for (final a in answers) '${a.question}=${a.values.join('+')}'].join(' ')}',
  );
  @override
  Future<void> dismiss(String request) async => calls.add('dismiss $request');
  @override
  Future<BridgeStatistics> statistics() async => stats;
  @override
  Future<BridgeContext?> context() async => contextWindow;
  @override
  Future<BridgeModelChoice?> model() async => chosen;
  @override
  Future<void> setModel(BridgeModelChoice model) async {
    calls.add('model ${model.model}');
    if (modelError case final error?) throw error;
    chosen = model;
  }

  @override
  Future<void> setPermissions(BridgePermissions permissions) async {
    calls.add('permissions ${permissions.name}');
    if (permissionsError case final error?) throw error;
  }

  @override
  Future<void> sendQueuedNow(int turn) async => calls.add('send now $turn');
  @override
  Future<void> editQueued(int turn, String text) async =>
      calls.add('edit $turn $text');
  @override
  Future<void> cancelQueued(int turn) async => calls.add('cancel $turn');
  @override
  Future<String> title() async => sessionTitle;
  @override
  Future<String?> worktree() async => null;
  @override
  GitBackend git() => gitBackend;
  @override
  Future<void> setBranch(String branch, {String? worktree}) async =>
      calls.add('branch $branch worktree=$worktree');

  @override
  Future<String> exportRecord({
    required String host,
    required String day,
  }) async {
    calls.add('export $host $day');
    return '/Downloads/$day';
  }

  final _firstRead = <String>{};
  List<BridgeLocalServer> serverList = [];
  final serverUpdates = StreamController<List<BridgeLocalServer>>.broadcast();
  List<BridgeAutomation> automationList = [];
  final automationUpdates =
      StreamController<List<BridgeAutomation>>.broadcast();
  BridgeLineage family = lineageOf();
  final lineageUpdates = StreamController<BridgeLineage>.broadcast();
  BridgeUsageLimits? limits;

  Stream<T> _changes<T>(String section, Stream<T> Function() changes) =>
      _firstRead.contains(section)
      ? changes()
      : Stream.error(
          StateError('Subscribe to the $section before streaming them.'),
        );

  @override
  Future<List<BridgeLocalServer>> servers() async {
    _firstRead.add('servers');
    return serverList;
  }

  @override
  Stream<List<BridgeLocalServer>> serverChanges() =>
      _changes('servers', () => serverUpdates.stream);
  @override
  Future<void> stopServer(int pid, int port) async =>
      calls.add('stop server $pid $port');
  @override
  Future<List<BridgeAutomation>> automations() async {
    _firstRead.add('automations');
    return automationList;
  }

  @override
  Stream<List<BridgeAutomation>> automationChanges() =>
      _changes('automations', () => automationUpdates.stream);
  @override
  Future<void> setAutomationEnabled(String id, {required bool enabled}) async =>
      calls.add('automation $id enabled=$enabled');
  @override
  Future<void> runAutomationNow(String id) async =>
      calls.add('run automation $id');
  @override
  Future<BridgeLineage> lineage() async {
    _firstRead.add('lineage');
    return family;
  }

  @override
  Stream<BridgeLineage> lineageChanges() =>
      _changes('lineage', () => lineageUpdates.stream);
  @override
  Future<void> mergeBack() async => calls.add('merge back');
  @override
  Future<void> disconnect() async => calls.add('disconnect');
  @override
  Future<void> stopSubagent(String id) async => calls.add('stop subagent $id');
  @override
  Future<BridgeUsageLimits?> usageLimits() async => limits;
}

class FakeGit implements GitBackend {
  FakeGit({BridgeGitStatus? status}) : current = status ?? gitStatusOf();

  BridgeGitStatus current;
  final statuses = StreamController<BridgeGitStatus>.broadcast();
  final calls = <String>[];
  List<BridgeRef> refList = [refOf('main', current: true, isDefault: true)];
  List<BridgeFileChange> changes = [];
  Map<String, String> patches = {};

  List<BridgeGitProgress> steps = [
    progressOf(BridgeGitProgressKind.finished, 'Done'),
  ];

  Object? refError;

  Object? statusError;

  bool closed = false;

  @override
  void close() => closed = true;

  void change(BridgeGitStatus next) {
    current = next;
    statuses.add(next);
  }

  @override
  Future<BridgeGitStatus> status() async {
    if (statusError case final error?) throw error;
    return current;
  }

  @override
  Stream<BridgeGitStatus> watch() async* {
    yield current;
    yield* statuses.stream;
  }

  @override
  Future<BridgeRefs> refs(
    String query, {
    int cursor = 0,
    int limit = 50,
  }) async {
    if (refError case final error?) throw error;
    final found = [
      for (final ref in refList)
        if (ref.name.contains(query)) ref,
    ];
    return BridgeRefs(refs: found.take(limit).toList(), total: found.length);
  }

  @override
  Future<void> switchTo(String name) async {
    calls.add('switch $name');
    if (refError case final error?) throw error;
  }

  @override
  Future<void> createBranch(String name) async {
    calls.add('create $name');
    if (refError case final error?) throw error;
  }

  @override
  Future<void> init() async => calls.add('init');

  @override
  Future<(String, String)> pull() async {
    calls.add('pull');
    return ('Pulled', 'Up to date');
  }

  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) {
    calls.add(
      'run ${action.name} message=$message files=${files?.join(',')} feature=$featureBranch',
    );
    return Stream.fromIterable(steps);
  }

  @override
  Future<List<BridgeFileChange>> uncommittedChanges({
    required bool hideWhitespace,
  }) async => changes;

  @override
  Future<String> uncommittedPatch(
    String path, {
    required bool hideWhitespace,
  }) async => patches[path] ?? '';

  @override
  Future<BridgeResolvedPullRequest> resolvePullRequest(String reference) async {
    calls.add('resolve $reference');
    return const BridgeResolvedPullRequest(
      number: 42,
      title: 'Add a feature',
      url: 'https://github.com/o/r/pull/42',
      base: 'main',
      head: 'feature',
      state: 'open',
    );
  }

  @override
  Future<BridgeCheckedOut> checkOutPullRequest(
    String reference, {
    required bool worktree,
  }) async {
    calls.add('check out $reference worktree=$worktree');
    return BridgeCheckedOut(
      branch: 'feature',
      worktree: worktree ? '/worktrees/feature' : null,
    );
  }

  @override
  Future<void> publish(
    String provider,
    String repository, {
    required bool public,
  }) async => calls.add('publish $provider $repository public=$public');

  List<BridgeFileChange> branchFiles = [];
  BridgePullRequestDetail? detail;

  @override
  Future<List<BridgeFileChange>> branchChanges({
    required bool hideWhitespace,
  }) async => branchFiles;
  @override
  Future<String> branchPatch(
    String path, {
    required bool hideWhitespace,
  }) async => patches[path] ?? '';
  @override
  Future<BridgePullRequestDetail> pullRequest(int number) async =>
      detail ?? pullRequestDetailOf(number: number);
  @override
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  }) async => calls.add('pr $number $action merge=$mergeMethod');
  @override
  Future<BridgeCheckedOut> pullRequestWorktree(String url) async {
    calls.add('pr worktree $url');
    return const BridgeCheckedOut(branch: 'feature', worktree: '/worktrees/pr');
  }
}

BridgeProvider providerOf({List<BridgeModel>? models}) => BridgeProvider(
  instance: 'claudeAgent',
  name: 'Claude',
  available: true,
  models: models ?? [modelOf('claude-opus-5-5', 'Claude Opus 5.5')],
);

BridgeModel modelOf(
  String id,
  String name, {
  List<BridgeModelOption>? options,
}) => BridgeModel(id: id, name: name, options: options ?? const []);

BridgeModelChoice choiceOf(String model) => BridgeModelChoice(
  instance: 'claudeAgent',
  model: model,
  optionIds: const [],
  values: const [],
);

BridgeMoveCheck moveCheckOf({
  int uncommittedFiles = 0,
  String? blocked,
  String? remoteUrl = 'git@github.com:acme/api.git',
  bool moved = false,
  BridgeMoving? moving,
}) => BridgeMoveCheck(
  moving: moving,
  title: 'Fix the flaky port test',
  movable: true,
  harness: 'Codex',
  model: 'gpt-6.1',
  running: false,
  uncommittedFiles: uncommittedFiles,
  branch: 'fix/ports',
  remoteUrl: remoteUrl,
  blocked: blocked,
  moved: moved,
);

class FakeConfigurationChanges implements ConfigurationChanges {
  FakeConfigurationChanges(this.changes);

  @override
  final List<BridgeConfigChange> changes;
}

BridgeMoving _movingOf(
  String transfer,
  String targetName,
  String targetEnvironment,
) => BridgeMoving(
  transfer: transfer,
  phase: BridgeMovePhase.handed,
  targetAddress: '',
  targetName: targetName,
  targetEnvironment: targetEnvironment,
  sinceMs: 0,
);

BridgeMoveCheck moveCheckWith(
  BridgeMoveCheck check, {
  required bool moved,
  BridgeMoving? moving,
}) => BridgeMoveCheck(
  title: check.title,
  movable: check.movable,
  harness: check.harness,
  model: check.model,
  running: check.running,
  uncommittedFiles: check.uncommittedFiles,
  branch: check.branch,
  remoteUrl: check.remoteUrl,
  blocked: check.blocked,
  moved: moved,
  moving: moving,
);

class FakeCarriedMove implements CarriedMove {
  FakeCarriedMove(this.transfer);

  @override
  final String transfer;
}

BridgeThreadSummary threadOf(
  String id, {
  String? title,
  String? worktree,
  bool running = false,
  int? updatedMs,
  bool movable = true,
  bool harnessKnown = true,
  BridgeMoving? moving,
}) => BridgeThreadSummary(
  id: id,
  title: title ?? id,
  worktree: worktree,
  running: running,
  archived: false,
  updatedMs: updatedMs,
  movable: movable,
  harnessKnown: harnessKnown,
  moving: moving,
);

BridgeSnapshot snapshotOf({
  int sequence = 0,
  List<BridgeTurn> turns = const [],
  List<BridgeTranscriptItem> transcript = const [],
  List<BridgeRequest> requests = const [],
}) => BridgeSnapshot(
  sequence: sequence,
  turns: turns,
  transcript: transcript,
  requests: requests,
);

BridgeEvent event(
  BridgeEventKind kind,
  int sequence, {
  int? turn,
  BridgeItem? item,
  String? text,
  BridgeUsage? usage,
  BridgeRequest? request,
  String? requestId,
}) => BridgeEvent(
  kind: kind,
  sequence: sequence,
  turn: turn,
  item: item,
  text: text,
  usage: usage,
  request: request,
  requestId: requestId,
);

BridgeItem item(
  int id,
  BridgeItemKind kind, {
  String? tool,
  String? target,
  bool ended = false,
  bool failed = false,
  int? durationMs,
}) => BridgeItem(
  id: id,
  kind: kind,
  tool: tool,
  target: target,
  ended: ended,
  failed: failed,
  durationMs: durationMs,
);

BridgeTranscriptItem chatItem(BridgeItem item, int turn, String text) =>
    BridgeTranscriptItem(item: item, turn: turn, text: text);

BridgeUsage usage(int input, int output) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
);

BridgeTurn turnOf(
  int number, {
  BridgeTurnStatus status = BridgeTurnStatus.completed,
  String prompt = '',
  int files = 0,
  int added = 0,
  int removed = 0,
  bool checkpoint = true,
}) => BridgeTurn(
  number: number,
  status: status,
  prompt: prompt,
  files: files,
  added: added,
  removed: removed,
  checkpoint: checkpoint,
);

BridgeRequest approvalOf(
  String id, {
  String kind = 'command',
  String prompt = 'rm -rf build',
  List<BridgeApprovalOption> options = const [],
}) => BridgeRequest(
  id: id,
  turn: 1,
  approval: BridgeApproval(kind: kind, prompt: prompt, options: options),
  questions: const [],
);

BridgeRequest questionsOf(String id, List<BridgeQuestion> questions) =>
    BridgeRequest(id: id, turn: 1, questions: questions);

BridgeQuestion questionOf(
  String id,
  String question, {
  List<String> options = const [],
  bool multiSelect = false,
}) => BridgeQuestion(
  id: id,
  header: id,
  question: question,
  options: options,
  descriptions: [for (final _ in options) ''],
  multiSelect: multiSelect,
  custom: true,
);

BridgeStatistics statisticsOf({List<BridgeTurnStatistics> turns = const []}) =>
    BridgeStatistics(models: const [], tools: const [], turns: turns);

BridgeFileChange fileOf(
  String path, {
  BridgeChangeStatus status = BridgeChangeStatus.modified,
  int added = 1,
  int removed = 0,
}) => BridgeFileChange(
  path: path,
  shown: path,
  status: status,
  added: added,
  removed: removed,
);

BridgeGitStatus gitStatusOf({
  bool repository = true,
  String? branch = 'main',
  bool defaultBranch = true,
  bool remote = true,
  String? provider = 'github',
  List<BridgeFileChange> changes = const [],
  int workingAdded = 0,
  int workingRemoved = 0,
  String? branchBase,
  int branchAdded = 0,
  int branchRemoved = 0,
  bool upstream = true,
  int ahead = 0,
  int behind = 0,
  int? aheadOfDefault,
  BridgeBranchPullRequest? pullRequest,
}) => BridgeGitStatus(
  repository: repository,
  branch: branch,
  defaultBranch: defaultBranch,
  remote: remote,
  provider: provider,
  changes: changes,
  workingAdded: workingAdded,
  workingRemoved: workingRemoved,
  branchBase: branchBase,
  branchAdded: branchAdded,
  branchRemoved: branchRemoved,
  upstream: upstream,
  ahead: ahead,
  behind: behind,
  aheadOfDefault: aheadOfDefault,
  pullRequest: pullRequest,
);

BridgeBranchPullRequest pullRequestOf() => BridgeBranchPullRequest(
  number: 7,
  title: 'Add a feature',
  url: 'https://github.com/o/r/pull/7',
  state: 'open',
  draft: false,
  base: 'main',
);

BridgeRef refOf(
  String name, {
  String? remote,
  bool current = false,
  bool isDefault = false,
  String? worktree,
}) => BridgeRef(
  name: name,
  shown: name,
  remote: remote,
  current: current,
  default_: isDefault,
  worktree: worktree,
);

BridgeGitProgress progressOf(
  BridgeGitProgressKind kind,
  String text, {
  String? description,
  String? pullRequest,
}) => BridgeGitProgress(
  kind: kind,
  text: text,
  description: description,
  pullRequest: pullRequest,
);

BridgeLineage lineageOf({List<BridgeLineageRow> rows = const []}) =>
    BridgeLineage(
      rows: rows,
      running: 0,
      sideChatsRunning: 0,
      canDisconnect: false,
      mergeReady: false,
      canMerge: false,
    );

BridgePullRequestDetail pullRequestDetailOf({int number = 7}) =>
    BridgePullRequestDetail(
      number: number,
      title: 'Add a feature',
      url: 'https://github.com/o/r/pull/$number',
      state: 'open',
      draft: false,
      base: 'main',
      head: 'feature',
      conflicting: false,
      files: 1,
      added: 1,
      removed: 0,
      checks: const [],
      checksState: 'none',
      checksSummary: '',
      action: null,
      mergeMethod: 'squash',
      fixPrompt: 'Fix the checks',
      resolvePrompt: 'Resolve the conflicts',
    );
