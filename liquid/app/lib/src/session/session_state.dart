import 'dart:async';

import 'package:flutter/foundation.dart';

import '../agent_hosts/agent_hosts.dart';
import '../diff/turn_scope.dart';
import '../format.dart';
import '../rust/api/details.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/moves.dart';
import '../rust/api/session.dart';
import '../rust/api/thread.dart';
import 'agent_host_set.dart';
import 'move_rule.dart';
import 'move_settle.dart';
import 'task_tree.dart';
import 'backend.dart';
import 'chat_items.dart';
import 'git_state.dart';

export 'chat_items.dart';
export 'git_state.dart';

enum RunStatus { idle, running, waitingForYou }

enum NoticeLevel { info, warning, error }

@immutable
class Notice {
  const Notice(this.level, this.text, this.at);

  final NoticeLevel level;
  final String text;
  final DateTime at;
}

const noticeLimit = 50;

@immutable
class HostThread {
  const HostThread({required this.address, required this.thread, this.machine});

  final String? address;
  final BridgeThreadSummary thread;

  final String? machine;
}

class SessionState {
  SessionState([ProjectBackend? thisMac, String? startsOn])
    : _startAddress = startsOn {
    if (thisMac != null) _agents.put(null, thisMac, null);
    hideWhitespace.addListener(() => unawaited(readChanges()));
    recent.addListener(_syncMoving);
  }

  final _agents = AgentHostSet();

  String hostNameOf(String? address) => _agents.nameOf(address);

  ProjectOpener? openProject;

  final runningMoves = <String>{};

  final startedMoves = <String>{};

  DateTime Function() clock = DateTime.now;

  final moveWaits = ValueNotifier<({String transfer, String? waitingFor})?>(
    null,
  );

  Timer? _recoveryTimer;

  final moving = ValueNotifier<BridgeMoving?>(null);

  void retryMove() => unawaited(loadStart());

  Future<void> cancelMove() async {
    final id = _session?.id;
    final move = moving.value;
    final source = projectOf(_sessionAddress);
    if (id == null || move == null || source == null) return;
    final reach = await _linkTo(
      move.targetEnvironment,
      except: [_sessionAddress],
    );
    BridgeArrival? answer;
    try {
      answer = await (await reach.link.backend).moveAbort(move.transfer);
    } on Object {
      answer = null;
    } finally {
      reach.link.drop();
    }
    final kind = answer?.kind;
    if (kind == BridgeArrivalKind.starting ||
        kind == BridgeArrivalKind.unknown) {
      notify(
        NoticeLevel.warning,
        '${move.targetName} is still taking the session, so the move cannot be cancelled yet.',
      );
      return;
    }
    final landed = kind == BridgeArrivalKind.landed ? answer?.thread : null;
    try {
      if (landed == null) {
        await source.moveFinish(id, move.transfer, null);
        await loadStart();
      } else {
        await source.moveFinish(id, move.transfer, move.targetName);
        notify(
          NoticeLevel.info,
          '${move.targetName} already had this session, so the move finished.',
        );
        await open(landed, address: reach.address);
        await loadStart();
      }
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not cancel the move: $error');
    }
  }

  Future<({HostLink link, String? address})> _linkTo(
    String environment, {
    required List<String?> except,
  }) async {
    for (final address in _agents.connected) {
      if (except.contains(address)) continue;
      final link = VerifiedLink(
        OwnConnection(connectTo, (
          address: address,
          root: projectOf(address)?.root ?? '',
        )),
        environment,
      );
      try {
        await link.backend;
        return (link: link, address: address);
      } on Object {
        link.drop();
      }
    }
    return (link: UnreachableLink(environment), address: null);
  }

  void _syncMoving() {
    final id = _session?.id;
    moving.value = id == null
        ? null
        : recent.value
              .where(
                (listed) =>
                    listed.thread.id == id && listed.address == _sessionAddress,
              )
              .firstOrNull
              ?.thread
              .moving;
  }

  Future<ProjectBackend> connectTo(String? address, String root) {
    final open = openProject;
    if (open == null) {
      throw StateError(
        'Liquid cannot open its own connection to ${hostNameOf(address)}.',
      );
    }
    return open(address, root);
  }

  String Function(String? address)? hostLabel;

  BridgeMoveRecord? _moveRecord;
  List<ItemView> _prefix = const [];
  String? _startAddress;
  SessionBackend? _session;
  String? _sessionAddress;
  StreamSubscription<BridgeEvent>? _subscription;

  ValueNotifier<Map<String?, String>> get hostProblems => _agents.problems;

  final sessionProblem = ValueNotifier<String?>(null);

  void Function(String? address)? onHostLost;

  bool _sessionLost = false;

  String? get startAddress => _startAddress;
  String? get sessionAddress => _sessionAddress;

  bool answers(String? address) => _agents.answers(address);

  final _follows = <StreamSubscription<dynamic>>[];

  int _generation = 0;

  int _switchRequests = 0;

  SessionBackend? get session => _session;

  ProjectBackend? get project => _agents[_startAddress];

  ProjectBackend? projectOf(String? address) => _agents[address];

  List<String?> get hostAddresses => _agents.addresses;

  ProjectBackend? get _viewProject =>
      _agents[_session == null ? _startAddress : _sessionAddress];

  ProjectBackend _connected(String? address) =>
      _agents[address] ?? (throw StateError(_unanswered(address)));

  String _unanswered(String? address) =>
      hostProblems.value[address] ??
      'Liquid is not connected to the agent host.';

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);

  final request = ValueNotifier<BridgeRequest?>(null);
  final sessionTitle = ValueNotifier<String?>(null);

  final draft = ValueNotifier<String?>(null);
  final runningInstances = ValueNotifier<List<RunningInstance>>(const []);
  final mention = ValueNotifier<String?>(null);
  final notice = ValueNotifier<Notice?>(null);
  final notices = ValueNotifier<List<Notice>>(const []);
  final totals = ValueNotifier(const SessionTotals());

  final queued = ValueNotifier<List<BridgeTurn>>(const []);
  final turns = ValueNotifier<List<BridgeTurn>>(const []);
  final statistics = ValueNotifier<BridgeStatistics?>(null);

  final statisticsProblem = ValueNotifier<({String title, String text})?>(null);
  final contextWindow = ValueNotifier<BridgeContext?>(null);
  final statisticsOpen = ValueNotifier(false);

  final servers = ValueNotifier<List<BridgeLocalServer>>(const []);
  final automations = ValueNotifier<List<BridgeAutomation>>(const []);
  final lineage = ValueNotifier<BridgeLineage?>(null);
  final limits = ValueNotifier<BridgeUsageLimits?>(null);
  final hostName = ValueNotifier<String?>(null);

  final scrollChatTo = ValueNotifier<int?>(null);

  final recent = ValueNotifier<List<HostThread>>(const []);
  final providers = ValueNotifier<List<BridgeProvider>>(const []);

  final model = ValueNotifier<BridgeModelChoice?>(null);
  final permissions = ValueNotifier(BridgePermissions.fullAccess);

  final newWorktree = ValueNotifier(true);

  final base = ValueNotifier<String?>(null);

  final worktree = ValueNotifier<String?>(null);

  late final git = ValueNotifier<GitState>(
    _gitFor(project?.git() ?? const NoCheckout()),
  );

  GitState _gitFor(GitBackend backend) {
    final state = GitState(
      backend,
      onError: (title, message) =>
          notify(NoticeLevel.error, '$title: $message'),
    );
    state.status.addListener(() {
      final scope = diffScope.value;
      if (scope is UncommittedChanges || scope is BranchChanges) {
        unawaited(readChanges());
      }
    });
    return state;
  }

  int _listings = 0;

  final Map<int, ItemView> _byId = {};
  final Map<String, BridgeRequest> _requests = {};
  int _sequence = 0;

  Future<void> showMoved(SessionBackend session, String? address) async {
    _attach(session, ++_switchRequests, address);
    await loadStart();
  }

  Future<void> loadStart() async {
    final listing = ++_listings;
    final lists = await Future.wait([
      for (final address in _agents.connected) _listSessions(address),
    ]);
    if (listing == _listings) {
      final sorted = [for (final list in lists) ...list].indexed.toList()
        ..sort((left, right) {
          final newer = (right.$2.thread.updatedMs ?? 0).compareTo(
            left.$2.thread.updatedMs ?? 0,
          );
          return newer != 0 ? newer : left.$1.compareTo(right.$1);
        });
      recent.value = [for (final (_, listed) in sorted) listed];
      unawaited(recoverMoves());
    }
    try {
      final view = _viewProject;
      if (view != null) {
        providers.value = await view.providers();
        model.value ??= _firstModel(providers.value);
      }
      final start = project;
      if (base.value == null && start != null) {
        final refs = await start.git().refs('', limit: 200);
        base.value = refs.refs.where((r) => r.default_).firstOrNull?.name;
      }
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not read the sessions: $error');
    }
  }

  Future<List<HostThread>> _listSessions(String? address) async {
    final host = _agents[address];
    if (host == null) return const [];
    if (!_agents.hasName(address)) {
      try {
        _agents.setName(address, await host.environmentName());
      } on Object catch (_) {}
    }
    try {
      return [
        for (final thread in await host.threads())
          HostThread(
            address: address,
            thread: thread,
            machine: _agents.machineOf(address),
          ),
      ];
    } on Object catch (error) {
      failHost(address, hostProblem(address, error));
      return const [];
    }
  }

  void _settledListing(HostThread listed) {
    final thread = listed.thread;
    recent.value = [
      for (final item in recent.value)
        if (identical(item, listed))
          HostThread(
            address: item.address,
            machine: item.machine,
            thread: BridgeThreadSummary(
              id: thread.id,
              title: thread.title,
              updatedMs: thread.updatedMs,
              branch: thread.branch,
              worktree: thread.worktree,
              running: thread.running,
              archived: thread.archived,
              movable: thread.movable,
              harnessKnown: thread.harnessKnown,
            ),
          )
        else
          item,
    ];
  }

  Future<void> recoverMoves() async {
    DateTime? wake;
    for (final listed in [...recent.value]) {
      final moving = listed.thread.moving;
      if (moving == null || runningMoves.contains(moving.transfer)) continue;
      final since = DateTime.fromMillisecondsSinceEpoch(moving.sinceMs.toInt());
      final age = clock().difference(since);
      final started = startedMoves.contains(moving.transfer);
      if (!started && age < recoveryAge) {
        final due = since.add(recoveryAge);
        if (wake == null || due.isBefore(wake)) wake = due;
      }
      final source = listed.address;
      final sourceLink = OwnConnection(connectTo, (
        address: source,
        root: projectOf(source)?.root ?? '',
      ));
      runningMoves.add(moving.transfer);
      final targetLink = (await _linkTo(
        moving.targetEnvironment,
        except: [source],
      )).link;
      moveWaits.value = null;
      Outcome? outcome;
      try {
        outcome = await settleMove(
          source: sourceLink,
          target: targetLink,
          sourceName: hostNameOf(source),
          targetName: moving.targetName,
          threadId: listed.thread.id,
          transfer: moving.transfer,
          startedHere: started,
          age: age,
        );
        if (outcome.settled == Settled.sealed) {
          notify(
            NoticeLevel.info,
            'Moved “${listed.thread.title}” to ${moving.targetName}.',
          );
        } else if (outcome.settled == Settled.released) {
          notify(
            NoticeLevel.warning,
            'The move of “${listed.thread.title}” to ${moving.targetName} did not happen; the session stayed on ${hostNameOf(source)}.',
          );
        }
        if (outcome.settled != Settled.unfinished) _settledListing(listed);
      } finally {
        runningMoves.remove(moving.transfer);
        moveWaits.value = outcome?.settled == Settled.unfinished
            ? (transfer: moving.transfer, waitingFor: outcome!.waitingFor)
            : null;
        sourceLink.drop();
        targetLink.drop();
      }
    }
    _recoveryTimer?.cancel();
    _recoveryTimer = wake == null
        ? null
        : Timer(wake.difference(clock()), () {
            _recoveryTimer = null;
            unawaited(recoverMoves());
          });
  }

  Future<void> addHost(
    String? address,
    ProjectBackend project, {
    String? machine,
  }) async {
    _agents.put(address, project, machine);
    _setProblem(address, null);
    if (address == _startAddress && _session == null) _showStartCheckout();
    final session = _session;
    if (_sessionLost && session != null && address == _sessionAddress) {
      unawaited(open(session.id, address: address));
    }
    await loadStart();
  }

  void failHost(String? address, String problem) {
    _agents.close(address);
    _setProblem(address, problem);
    recent.value = [
      for (final listed in recent.value)
        if (listed.address != address) listed,
    ];
    if (address == _startAddress && _session == null) _showStartCheckout();
    onHostLost?.call(address);
  }

  Future<void> removeHost(String? address) async {
    if (_session != null && _sessionAddress == address) await newSession();
    _agents.close(address);
    _setProblem(address, null);
    if (address == _startAddress && _session == null) _showStartCheckout();
    await loadStart();
  }

  Future<void> startOn(String? address) async {
    _startAddress = address;
    worktree.value = null;
    base.value = null;
    if (_session == null) {
      model.value = null;
      _showStartCheckout();
    }
    await loadStart();
  }

  void _setProblem(String? address, String? problem) {
    _agents.setProblem(address, problem);
    _updateSessionProblem();
  }

  void _updateSessionProblem() => sessionProblem.value = _session == null
      ? null
      : hostProblems.value[_sessionAddress];

  void _showStartCheckout() => _setGit(project?.git() ?? const NoCheckout());

  static BridgeModelChoice? _firstModel(List<BridgeProvider> providers) {
    for (final provider in providers) {
      if (!provider.available || provider.models.isEmpty) continue;
      return BridgeModelChoice(
        instance: provider.instance,
        model: provider.models.first.id,
        optionIds: const [],
        values: const [],
      );
    }
    return null;
  }

  Future<void> newSession() async {
    _switchRequests++;
    await _detach();
    worktree.value = null;
    _showStartCheckout();
    unawaited(loadStart());
  }

  Future<void> open(String id, {String? address}) async {
    final switchRequest = ++_switchRequests;
    try {
      _attach(await _connected(address).open(id), switchRequest, address);
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not open the session: $error');
    }
  }

  Future<void> send(String text, {bool steer = false}) async {
    final session = _session;
    try {
      if (session != null) {
        await session.send(text, steer: steer);
        return;
      }
      final chosen = model.value;
      if (chosen == null) {
        notify(NoticeLevel.error, 'No model is available.');
        return;
      }
      final start = _startAddress;
      final switchRequest = ++_switchRequests;
      _attach(
        await _connected(start).start(
          text,
          chosen,
          base: newWorktree.value ? base.value : null,
          worktree: newWorktree.value ? null : worktree.value,
          permissions: permissions.value,
        ),
        switchRequest,
        start,
      );
      unawaited(loadStart());
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not send the message: $error');
    }
  }

  Future<void> setBranch(String branch, {String? worktree}) async {
    final session = _session;
    if (session == null) return;
    final generation = _generation;
    await session.setBranch(branch, worktree: worktree);
    if (generation == _generation) _setGit(session.git());
  }

  Future<void> draftFor(BridgeCheckedOut checked) async {
    await newSession();
    worktree.value = checked.worktree;
    newWorktree.value = false;
  }

  Future<void> stop() async {
    try {
      await _session?.stop();
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not stop the turn: $error');
    }
  }

  Future<void> sendQueuedNow(BridgeTurn turn) => _act(
    () => _session!.sendQueuedNow(turn.number),
    'send the queued message',
  );
  Future<void> editQueued(BridgeTurn turn, String text) => _act(
    () => _session!.editQueued(turn.number, text),
    'update the queued message',
  );
  Future<void> cancelQueued(BridgeTurn turn) => _act(
    () => _session!.cancelQueued(turn.number),
    'remove the queued message',
  );

  Future<void> respond(BridgeRequest request, String decision) =>
      _act(() => _session!.respond(request.id, decision), 'answer');

  Future<void> answer(
    BridgeRequest request,
    List<BridgeQuestionAnswer> answers,
  ) => _act(() => _session!.answer(request.id, answers), 'answer');

  Future<void> dismiss(BridgeRequest request) =>
      _act(() => _session!.dismiss(request.id), 'dismiss the question');

  Future<bool> revert(int turn, {required bool restoreFiles}) async {
    try {
      draft.value = await _session!.revert(turn, restoreFiles: restoreFiles);
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not revert: $error');
      return false;
    }
  }

  Future<void> fork(int turn) async {
    try {
      final id = await _session!.fork(turn);
      await open(id, address: _sessionAddress);
      unawaited(loadStart());
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Failed to fork this response. $error');
    }
  }

  Future<void> setModel(BridgeModelChoice next) async {
    if (_session case final session?) {
      await _act(() async {
        await session.setModel(next);
        model.value = next;
      }, 'apply the model selection');
      await readStatistics();
    } else {
      model.value = next;
    }
  }

  Future<void> setPermissions(BridgePermissions next) async {
    if (_session case final session?) {
      await _act(() async {
        await session.setPermissions(next);
        permissions.value = next;
      }, 'change the permissions');
    } else {
      permissions.value = next;
    }
  }

  Future<bool> onlySessionInWorktree(String id, {String? address}) =>
      _connected(address).onlySessionInWorktree(id);

  Future<bool> delete(
    String id, {
    required bool worktree,
    String? address,
  }) async {
    try {
      if (_session?.id == id && _sessionAddress == address) {
        await newSession();
      }
      await _connected(address).delete(id, worktree: worktree);
      await loadStart();
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not delete the session: $error');
      return false;
    }
  }

  Future<void> archive(String id, {String? address, required bool archived}) =>
      _act(() async {
        await _connected(address).archive(id, archived: archived);
        await loadStart();
      }, archived ? 'archive the session' : 'restore the session');

  Future<String?> exportRecord({
    required String host,
    required String day,
  }) async {
    try {
      return await _session?.exportRecord(host: host, day: day);
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not export the record: $error');
      return null;
    }
  }

  Future<void> stopServer(int pid, int port) =>
      _act(() => _session!.stopServer(pid, port), 'stop server');

  Future<void> setAutomationEnabled(String id, bool enabled) => _act(
    () => _session!.setAutomationEnabled(id, enabled: enabled),
    'update automation',
  );

  Future<void> runAutomation(String id) =>
      _act(() => _session!.runAutomationNow(id), 'run automation');

  Future<void> mergeBack() async {
    final target = lineage.value?.rows
        .where((row) => row.mergeTarget)
        .firstOrNull
        ?.threadId;
    final generation = _generation;
    try {
      await _session!.mergeBack();
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not merge back: $error');
      return;
    }
    if (target != null && generation == _generation) {
      await open(target, address: _sessionAddress);
    }
  }

  Future<void> disconnect() =>
      _act(() => _session!.disconnect(), 'disconnect agent session');

  Future<void> stopSubagent(String id) =>
      _act(() => _session!.stopSubagent(id), 'stop subagent');

  Future<void> _act(Future<void> Function() action, String what) async {
    try {
      await action();
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not $what: $error');
    }
  }

  final diffScope = ValueNotifier<DiffScope>(const LatestTurn());
  final shown = ValueNotifier<Compared?>(null);
  final shownScope = ValueNotifier<DiffScope>(const LatestTurn());
  final changed = ValueNotifier<List<BridgeFileChange>?>(null);
  final changesProblem = ValueNotifier<({String title, String text})?>(null);
  final hideWhitespace = ValueNotifier(false);

  Future<void> showTurns(DiffScope scope) async {
    diffScope.value = scope;
    await readChanges();
  }

  int _diffRequests = 0;

  Future<void> readChanges() async {
    final generation = _generation;
    final diffRequest = ++_diffRequests;
    bool isLatest() =>
        generation == _generation && diffRequest == _diffRequests;
    final session = _session;
    try {
      final scope = diffScope.value;
      if (scope is UncommittedChanges || scope is BranchChanges) {
        final checkout = git.value.backend;
        final files = scope is BranchChanges
            ? await checkout.branchChanges(hideWhitespace: hideWhitespace.value)
            : await checkout.uncommittedChanges(
                hideWhitespace: hideWhitespace.value,
              );
        if (!isLatest()) return;
        shownScope.value = scope;
        shown.value = null;
        changesProblem.value = null;
        changed.value = files;
        return;
      }
      final read = session == null
          ? const <BridgeTurn>[]
          : await session.turns();
      if (!isLatest()) return;
      _setTurns(read);
      final pair = compared(read, scope);
      if (pair == null || session == null) {
        shownScope.value = scope;
        shown.value = null;
        changesProblem.value = (
          title: 'No finished turn yet',
          text: 'A turn’s changes appear here when it ends.',
        );
        changed.value = const [];
        return;
      }
      final files = await session.turnChanges(
        pair.from.number,
        pair.to.number,
        hideWhitespace: hideWhitespace.value,
      );
      if (!isLatest()) return;
      shownScope.value = scope;
      shown.value = pair;
      changesProblem.value = null;
      changed.value = files;
    } on Object catch (error) {
      if (!isLatest()) return;
      changesProblem.value = (
        title: 'No changes shown',
        text: 'Could not show the changes: $error',
      );
      changed.value = null;
    }
  }

  Future<String> patch(String path) {
    final scope = shownScope.value;
    if (scope is BranchChanges) {
      return git.value.backend.branchPatch(
        path,
        hideWhitespace: hideWhitespace.value,
      );
    }
    if (scope is UncommittedChanges) {
      return git.value.backend.uncommittedPatch(
        path,
        hideWhitespace: hideWhitespace.value,
      );
    }
    final pair = shown.value;
    final session = _session;
    if (pair == null || session == null) return Future.value('');
    return session.turnPatch(
      pair.from.number,
      pair.to.number,
      path,
      hideWhitespace: hideWhitespace.value,
    );
  }

  Future<void> readStatistics() async {
    final session = _session;
    final generation = _generation;
    if (session == null) return;
    try {
      final read = await session.statistics();
      if (generation != _generation) return;
      statistics.value = read;
      statisticsProblem.value = null;
    } on Object catch (error) {
      if (generation != _generation) return;
      final text = 'Could not read the statistics: $error';
      statisticsProblem.value = (title: 'No statistics shown', text: text);
      notify(NoticeLevel.warning, text);
    }
    try {
      final window = await session.context();
      if (generation != _generation) return;
      contextWindow.value = window;
    } on Object catch (error) {
      if (generation != _generation) return;
      notify(NoticeLevel.warning, 'Could not read the context window: $error');
    }
    await _read(
      generation,
      'the usage limits',
      session.usageLimits,
      (value) => limits.value = value,
    );
  }

  void notify(NoticeLevel level, String text) {
    final next = Notice(level, text, DateTime.now());
    notice.value = next;
    notices.value = [...notices.value, next].length > noticeLimit
        ? [...notices.value.skip(1), next]
        : [...notices.value, next];
  }

  void dismissNotice() => notice.value = null;
  void clearNotices() {
    notice.value = null;
    notices.value = const [];
  }

  void _attach(SessionBackend next, int switchRequest, String? address) {
    if (switchRequest != _switchRequests) {
      next.close();
      return;
    }
    unawaited(
      _detach().then((_) async {
        final host = _agents[address];
        if (switchRequest != _switchRequests || host == null) {
          next.close();
          return;
        }
        _session = next;
        _sessionAddress = address;
        _syncMoving();
        _sessionLost = false;
        _updateSessionProblem();
        final generation = ++_generation;
        try {
          _setGit(next.git());
          final subscribed = await next.subscribe();
          if (generation != _generation) return;
          _moveRecord = await _recordOf(host, next.id);
          if (generation != _generation) return;
          _restore(subscribed.snapshot);
          final events = next.events(subscribed.id);
          _subscription = events.listen(
            _receive,
            onError: (Object error) {
              if (generation == _generation) {
                _sessionLost = true;
                failHost(address, hostProblem(address, error));
              }
            },
          );
          final title = await next.title();
          if (generation != _generation) return;
          sessionTitle.value = title;
          final chosen = await next.model();
          if (generation != _generation) return;
          if (chosen != null) model.value = chosen;
        } on Object catch (error) {
          if (generation == _generation) {
            notify(NoticeLevel.error, 'Could not open the session: $error');
          }
          return;
        }
        unawaited(readStatistics());
        unawaited(readChanges());
        unawaited(_follow(next, host, generation));
      }),
    );
  }

  Future<void> _detach() async {
    _generation++;
    _moveRecord = null;
    _prefix = const [];
    await _subscription?.cancel();
    _subscription = null;
    for (final follow in _follows) {
      unawaited(follow.cancel());
    }
    _follows.clear();
    _session?.close();
    _sessionAddress = null;
    _sessionLost = false;
    servers.value = const [];
    automations.value = const [];
    lineage.value = null;
    limits.value = null;
    hostName.value = null;
    _session = null;
    _syncMoving();
    _updateSessionProblem();
    _sequence = 0;
    _applied = 0;
    _confirmed = 0;
    _byId.clear();
    _requests.clear();
    items.value = const [];
    runningInstances.value = const [];
    request.value = null;
    queued.value = const [];
    turns.value = const [];
    totals.value = const SessionTotals();
    statistics.value = null;
    statisticsProblem.value = null;
    contextWindow.value = null;
    sessionTitle.value = null;
    status.value = RunStatus.idle;
    diffScope.value = const LatestTurn();
    changed.value = null;
    shown.value = null;
  }

  Future<void> _follow(
    SessionBackend next,
    ProjectBackend host,
    int generation,
  ) async {
    await Future.wait([
      _watch(
        generation,
        'the servers',
        next.servers,
        next.serverChanges,
        (value) => servers.value = value,
      ),
      _watch(
        generation,
        'the scheduled tasks',
        next.automations,
        next.automationChanges,
        (value) => automations.value = value,
      ),
      _watch(
        generation,
        'the related threads',
        next.lineage,
        next.lineageChanges,
        (value) => lineage.value = value,
      ),
      _read(
        generation,
        'the models',
        host.providers,
        (value) => providers.value = value,
      ),
      _read(
        generation,
        'the host name',
        host.environmentName,
        (value) => hostName.value = value,
      ),
    ]);
  }

  Future<void> _watch<T>(
    int generation,
    String what,
    Future<T> Function() read,
    Stream<T> Function() changes,
    void Function(T value) set,
  ) => _read(generation, what, read, (value) {
    set(value);
    _follows.add(
      changes().listen(
        set,
        onError: (Object error) {
          if (generation == _generation) {
            notify(NoticeLevel.warning, 'Could not follow $what: $error');
          }
        },
      ),
    );
  });

  Future<void> _read<T>(
    int generation,
    String what,
    Future<T> Function() read,
    void Function(T value) set,
  ) async {
    try {
      final value = await read();
      if (generation == _generation) set(value);
    } on Object catch (error) {
      if (generation == _generation) {
        notify(NoticeLevel.warning, 'Could not read $what: $error');
      }
    }
  }

  void _setGit(GitBackend backend) {
    final previous = git.value;
    git.value = _gitFor(backend);
    previous.dispose();
  }

  void _restore(BridgeSnapshot snapshot) {
    _sequence = snapshot.sequence;
    _byId.clear();
    final record = _moveRecord;
    _prefix = record == null
        ? const []
        : [
            for (var index = 0; index < record.transcript.length; index++)
              _carriedView(record.transcript[index], index),
            ItemView(
              id: _moveMarkerId,
              kind: BridgeItemKind.assistantMessage,
              turn: 0,
              marker:
                  'Moved from ${_labelOf(record.fromHost)} to ${_labelOf(record.toHost)} · ${formatClockMs(record.atMs)}',
            ),
          ];
    final transcript =
        record != null &&
            snapshot.transcript.firstOrNull?.item.kind ==
                BridgeItemKind.userMessage
        ? snapshot.transcript.skip(1)
        : snapshot.transcript;
    for (final entry in transcript) {
      _byId[_view(entry.item, entry.turn).id] = _settled(
        _view(entry.item, entry.turn),
        entry,
      );
    }
    _requests
      ..clear()
      ..addEntries(snapshot.requests.map((r) => MapEntry(r.id, r)));
    _setTurns(snapshot.turns);
    _status();
  }

  static const _moveRowsBase = -(1 << 40);
  static const _moveMarkerId = _moveRowsBase - (1 << 30);

  String _labelOf(String name) {
    final known = _agents.names
        .where((entry) => entry.value.toLowerCase() == name.toLowerCase())
        .firstOrNull;
    if (known == null) return name;
    final label = hostLabel;
    if (label != null) return label(known.key);
    return _agents.nameOf(known.key);
  }

  ItemView _carriedView(BridgeTranscriptItem entry, int index) => _settled(
    ItemView(
      id: _moveRowsBase - index,
      kind: entry.item.kind,
      turn: 0,
      tool: entry.item.tool,
      target: entry.item.target,
    ),
    entry,
  );

  ItemView _settled(ItemView view, BridgeTranscriptItem entry) {
    view.text.value = entry.text;
    view.steps.value = entry.item.steps;
    if (entry.item.ended) {
      view.outcome.value = (
        failed: entry.item.failed,
        durationMs: entry.item.durationMs,
      );
    }
    return view;
  }

  ItemView _view(BridgeItem item, int turn) => ItemView(
    id: item.id,
    kind: item.kind,
    turn: turn,
    tool: item.tool,
    target: item.target,
  )..steps.value = item.steps;

  void _setTurns(List<BridgeTurn> read) {
    turns.value = read;
    queued.value = [
      for (final turn in read)
        if (turn.status == BridgeTurnStatus.queued) turn,
    ];
    final ends = {
      for (final turn in read)
        if (turn.checkpoint) turn.number: turn,
    };
    final rows = <ItemView>[];
    final views = _byId.values.toList();
    for (var index = 0; index < views.length; index++) {
      final view = views[index];
      rows.add(view);
      final last =
          index == views.length - 1 || views[index + 1].turn != view.turn;
      final end = ends[view.turn];
      if (last && end != null) {
        rows.add(
          ItemView(
            id: -end.number,
            kind: BridgeItemKind.assistantMessage,
            turn: end.number,
            turnEnd: end,
          ),
        );
      }
    }
    items.value = [..._prefix, ...rows];
  }

  void _status() {
    request.value = _requests.values.firstOrNull;
    final tasks = items.value.lastWhere(
      (item) => item.kind == BridgeItemKind.taskList,
      orElse: () => ItemView(id: 0, kind: BridgeItemKind.taskList, turn: 0),
    );
    final running = turns.value.any(
      (t) => t.status == BridgeTurnStatus.running,
    );
    final names = running
        ? instancesOf(tasks.steps.value)
        : const <RunningInstance>[];
    if (!listEquals(names, runningInstances.value)) {
      runningInstances.value = names;
    }
    status.value = _requests.isNotEmpty
        ? RunStatus.waitingForYou
        : running
        ? RunStatus.running
        : RunStatus.idle;
  }

  int _applied = 0;
  int _confirmed = 0;

  int? _confirmingFor;

  void _receive(BridgeEvent event) {
    try {
      _apply(event);
    } finally {
      _applied++;
      if (_confirmingFor != _generation) unawaited(_confirm());
    }
  }

  Future<void> _confirm() async {
    final session = _session;
    final generation = _generation;
    if (session == null) return;
    _confirmingFor = generation;
    try {
      while (generation == _generation && _confirmed < _applied) {
        final applied = _applied;
        await session.confirmApplied(applied);
        if (generation != _generation) return;
        _confirmed = applied;
      }
    } on Object catch (_) {
    } finally {
      if (_confirmingFor == generation) _confirmingFor = null;
    }
  }

  void _apply(BridgeEvent event) {
    if (event.kind == BridgeEventKind.fellBehind ||
        event.kind == BridgeEventKind.rolledBack) {
      unawaited(_reload());
      return;
    }
    if (event.sequence <= _sequence) return;
    switch (event.kind) {
      case BridgeEventKind.turnStarted ||
          BridgeEventKind.turnsChanged ||
          BridgeEventKind.turnCompleted ||
          BridgeEventKind.turnAborted:
        unawaited(_readTurns(ended: event.kind != BridgeEventKind.turnStarted));
      case BridgeEventKind.itemStarted:
        final view = _view(event.item!, event.turn ?? 0);
        _byId[view.id] = view;
        _setTurns(turns.value);
      case BridgeEventKind.contentDelta:
        final view = _byId[event.item!.id];
        if (view != null) view.text.value += event.text ?? '';
      case BridgeEventKind.itemUpdated:
        _byId[event.item!.id]?.steps.value = event.item!.steps;
      case BridgeEventKind.itemCompleted:
        final view = _byId[event.item!.id];
        if (view != null) {
          view
            ..outcome.value = (
              failed: event.item!.failed,
              durationMs: event.item!.durationMs,
            )
            ..text.value = event.text ?? view.text.value;
        }
      case BridgeEventKind.modelRequestCompleted:
        totals.value = totals.value.add(event.usage!);
        unawaited(readStatistics());
      case BridgeEventKind.requestOpened:
        _requests[event.request!.id] = event.request!;
      case BridgeEventKind.requestResolved:
        _requests.remove(event.requestId);
      case BridgeEventKind.fellBehind || BridgeEventKind.rolledBack:
        break;
    }
    _status();
  }

  Future<void> _readTurns({required bool ended}) async {
    final session = _session;
    final generation = _generation;
    if (session == null) return;
    try {
      final read = await session.turns();
      if (generation != _generation) return;
      _setTurns(read);
      _status();
      if (ended) {
        sessionTitle.value = await session.title();
        unawaited(readChanges());
        unawaited(readStatistics());
        unawaited(loadStart());
      }
    } on Object catch (error) {
      notify(NoticeLevel.warning, 'Could not read the turns: $error');
    }
  }

  Future<BridgeMoveRecord?> _recordOf(ProjectBackend host, String id) async {
    try {
      return await host.moveRecord(id);
    } on Object {
      return null;
    }
  }

  Future<void> _reload() async {
    final session = _session;
    final generation = _generation;
    if (session == null) return;
    try {
      final snapshot = await session.snapshot();
      if (generation != _generation) return;
      _restore(snapshot);
      unawaited(readChanges());
      unawaited(readStatistics());
    } on Object catch (error) {
      notify(NoticeLevel.warning, 'Could not reload the Chat: $error');
    }
  }

  Future<void> dispose() async {
    _recoveryTimer?.cancel();
    await _detach();
    git.value.dispose();
    _agents.closeAll();
  }
}
