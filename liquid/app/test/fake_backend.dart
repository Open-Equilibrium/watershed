import 'dart:async';

import 'package:liquid/src/rust/api/details.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/backend.dart';

/// A project on a fake agent host. It records what Liquid asks in [calls]; tests set what it
/// answers and subclass it for anything else.
class FakeProject implements ProjectBackend {
  FakeProject({FakeSession? session, FakeGit? git})
    : session = session ?? FakeSession(),
      gitBackend = git ?? FakeGit();

  /// The session that [start] and [open] return.
  FakeSession session;
  FakeGit gitBackend;
  final calls = <String>[];

  List<BridgeThreadSummary> threadList = [];
  List<BridgeProvider> providerList = [providerOf()];
  BridgeModelChoice? writer;
  bool onlyInWorktree = false;

  /// What [start] and [open] fail with.
  Object? openError;

  /// Holds what [start] and [open] return until it completes, so a later call can return first.
  Completer<void>? hold;

  /// What [onlySessionInWorktree] fails with.
  Object? onlyInWorktreeError;

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
}

/// One session on the fake host: [emit] streams events; everything else is recorded in [calls].
class FakeSession implements SessionBackend {
  FakeSession({this.id = 'thread-1'});

  @override
  final String id;
  final calls = <String>[];
  final _events = StreamController<BridgeEvent>.broadcast();

  /// What [subscribe] and [snapshot] return.
  BridgeSnapshot current = snapshotOf();
  List<BridgeTurn> turnList = [];
  List<BridgeFileChange> changes = [];
  Map<String, String> patches = {};
  BridgeStatistics stats = statisticsOf();
  BridgeContext? contextWindow;
  BridgeModelChoice? chosen;
  String sessionTitle = 'Session';
  String? worktreePath;
  FakeGit gitBackend = FakeGit();

  /// The prompt [revert] returns.
  String revertedPrompt = '';

  /// What [send] fails with.
  Object? sendError;

  /// What [setPermissions] fails with.
  Object? permissionsError;

  /// The applied counts Liquid confirmed, in order.
  final confirmed = <int>[];

  void emit(BridgeEvent event) => _events.add(event);

  /// Whether Liquid released the session.
  bool closed = false;

  @override
  void close() => closed = true;

  @override
  Future<BridgeSubscription> subscribe() async =>
      BridgeSubscription(id: 1, snapshot: current);

  /// Like the bridge's stream, its cancel completes in the caller's zone, so widget tests that
  /// leave a session finish.
  @override
  Stream<BridgeEvent> events(int subscription) {
    late final StreamSubscription<BridgeEvent> source;
    late final StreamController<BridgeEvent> controller;
    controller = StreamController(
      onListen: () => source = _events.stream.listen(controller.add),
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
  Future<String?> worktree() async => worktreePath;
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

  /// What the card's live sections show; [serverUpdates], [automationUpdates] and
  /// [lineageUpdates] stream their changes. As the bridge does, a section's changes fail
  /// unless its first read ran first.
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

/// Git for a fake checkout: [statuses] streams status changes; actions are recorded in [calls].
class FakeGit implements GitBackend {
  FakeGit({BridgeGitStatus? status}) : current = status ?? gitStatusOf();

  BridgeGitStatus current;
  final statuses = StreamController<BridgeGitStatus>.broadcast();
  final calls = <String>[];
  List<BridgeRef> refList = [refOf('main', current: true, isDefault: true)];
  List<BridgeFileChange> changes = [];
  Map<String, String> patches = {};

  /// The steps [run] streams.
  List<BridgeGitProgress> steps = [
    progressOf(BridgeGitProgressKind.finished, 'Done'),
  ];

  /// What [refs], [switchTo] and [createBranch] fail with.
  Object? refError;

  /// Whether Liquid released the checkout.
  bool closed = false;

  @override
  void close() => closed = true;

  /// Changes the status, as T3's status stream reports it.
  void change(BridgeGitStatus next) {
    current = next;
    statuses.add(next);
  }

  @override
  Future<BridgeGitStatus> status() async => current;

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

BridgeProvider providerOf({
  String instance = 'claudeAgent',
  String name = 'Claude',
  List<BridgeModel>? models,
}) => BridgeProvider(
  instance: instance,
  name: name,
  available: true,
  models: models ?? [modelOf('claude-opus-5-5', 'Claude Opus 5.5')],
);

BridgeModel modelOf(
  String id,
  String name, {
  List<BridgeModelOption>? options,
}) => BridgeModel(id: id, name: name, options: options ?? const []);

BridgeModelChoice choiceOf(String model, {String instance = 'claudeAgent'}) =>
    BridgeModelChoice(
      instance: instance,
      model: model,
      optionIds: const [],
      values: const [],
    );

BridgeThreadSummary threadOf(
  String id, {
  String? title,
  String? worktree,
  bool running = false,
  bool archived = false,
}) => BridgeThreadSummary(
  id: id,
  title: title ?? id,
  worktree: worktree,
  running: running,
  archived: archived,
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
  int turn = 1,
  String kind = 'command',
  String prompt = 'rm -rf build',
  List<BridgeApprovalOption> options = const [],
}) => BridgeRequest(
  id: id,
  turn: turn,
  approval: BridgeApproval(kind: kind, prompt: prompt, options: options),
  questions: const [],
);

BridgeRequest questionsOf(
  String id,
  List<BridgeQuestion> questions, {
  int turn = 1,
}) => BridgeRequest(id: id, turn: turn, questions: questions);

BridgeQuestion questionOf(
  String id,
  String question, {
  List<String> options = const [],
  bool multiSelect = false,
  bool custom = true,
}) => BridgeQuestion(
  id: id,
  header: id,
  question: question,
  options: options,
  descriptions: [for (final _ in options) ''],
  multiSelect: multiSelect,
  custom: custom,
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

BridgeBranchPullRequest pullRequestOf({
  int number = 7,
  String title = 'Add a feature',
  String state = 'open',
}) => BridgeBranchPullRequest(
  number: number,
  title: title,
  url: 'https://github.com/o/r/pull/$number',
  state: state,
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

BridgePullRequestDetail pullRequestDetailOf({
  int number = 7,
  String state = 'open',
  bool draft = false,
  String? action,
}) => BridgePullRequestDetail(
  number: number,
  title: 'Add a feature',
  url: 'https://github.com/o/r/pull/$number',
  state: state,
  draft: draft,
  base: 'main',
  head: 'feature',
  conflicting: false,
  files: 1,
  added: 1,
  removed: 0,
  checks: const [],
  checksState: 'none',
  checksSummary: '',
  action: action,
  mergeMethod: 'squash',
  fixPrompt: 'Fix the checks',
  resolvePrompt: 'Resolve the conflicts',
);
