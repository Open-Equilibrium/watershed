import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../agent_hosts/agent_hosts.dart';
import '../format.dart';
import '../rust/api/moves.dart';
import 'backend.dart';
import 'move_settle.dart';

enum MoveStepState { now, done, failed }

@immutable
class MoveStep {
  const MoveStep(this.text, this.state);

  final String text;
  final MoveStepState state;
}

const otherRemoteReason =
    "This project's Git remote is not the session's remote, so the session cannot move here.";
const noRemoteReason =
    'This project has no Git remote that T3 can resolve, so the session cannot move here.';
const olderHostReason = 'Needs a newer Meta-Harness';
const noRemoteFilesReason =
    'The project has no Git remote, so its files cannot travel';

@immutable
class MovePlan {
  const MovePlan({
    required this.title,
    required this.sourceName,
    required this.targetName,
    required this.branch,
    required this.uncommitted,
    required this.running,
    required this.commit,
    required this.root,
    this.cloneRemote,
    this.bringing,
  });

  final String title;

  final String sourceName;
  final String targetName;
  final String? branch;

  final int uncommitted;
  final bool running;

  final bool commit;

  final String root;

  final String? cloneRemote;

  final ConfigurationChanges? bringing;
}

class MoveUnconfirmed implements Exception {
  const MoveUnconfirmed(this.message, {this.session});

  final String message;

  final SessionBackend? session;

  @override
  String toString() => message;
}

class MoveFailure implements Exception {
  const MoveFailure(this.step, this.message);

  final String step;

  final String message;

  @override
  String toString() => message;
}

String newMoveTransfer() {
  final random = Random.secure();
  return [
    for (var i = 0; i < 16; i++)
      random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ].join();
}

class MoveFlow {
  MoveFlow({
    required ProjectOpener open,
    required HostPlace from,
    required HostPlace to,
    required this.threadId,
    required this.plan,
    required this.running,
    required this.started,
    this.newTransfer = newMoveTransfer,
  }) : _source = OwnConnection(open, from),
       _target = OwnConnection(open, to);

  final String threadId;
  final MovePlan plan;
  final Set<String> running;
  final Set<String> started;
  final String Function() newTransfer;
  final OwnConnection _source;
  final OwnConnection _target;
  late String _targetEnvironment;
  late HostLink _verified;

  final steps = ValueNotifier<List<MoveStep>>(const []);
  final _listed = <MoveStep>[];
  var _cloned = false;
  late String _root = plan.root;
  var _brought = false;
  var _pushed = false;
  String? _transfer;

  Future<SessionBackend> run() async {
    try {
      return await _move();
    } finally {
      if (_transfer case final transfer?) running.remove(transfer);
      _source.drop();
      _target.drop();
    }
  }

  Future<void> _connectBoth() async {
    try {
      await _source.backend;
      final target = await _target.backend;
      _targetEnvironment = await target.environmentId();
      _verified = VerifiedLink(_target, _targetEnvironment);
    } on Object catch (error) {
      throw _failure('connecting', error);
    }
  }

  Future<SessionBackend> _move() async {
    await _connectBoth();
    final root = plan.root;
    if (plan.cloneRemote case final remote?) {
      final cloned = await _step(
        'Cloning the project to $root on ${plan.targetName}…',
        'Cloned the project to $root on ${plan.targetName}',
        'cloning the project',
        () async => (await _target.backend).cloneProject(remote, root),
      );
      _root = cloned.root;
      _cloned = true;
    }
    if (plan.bringing case final shown?) {
      await _step(
        'Bringing the configuration to ${plan.targetName}…',
        'Brought the configuration to ${plan.targetName}',
        'bringing the configuration over',
        () async => (await _target.backend).bringConfiguration(
          await _source.backend,
          shown,
        ),
      );
      _brought = true;
    }
    final turn = plan.running ? _begin('Ending the current turn…') : null;
    final pushing = _begin(_pushingNow);
    final transfer = newTransfer();
    _transfer = transfer;
    running.add(transfer);
    started.add(transfer);
    final CarriedMove carried;
    try {
      carried = await (await _source.backend).moveOut(
        threadId,
        transfer: transfer,
        targetAddress: _target.place.address ?? '',
        targetName: plan.targetName,
        targetEnvironment: _targetEnvironment,
        commit: plan.commit,
      );
    } on Object catch (error) {
      _set(pushing, MoveStepState.failed);
      final outcome = await _settle(transfer);
      if (outcome.settled == Settled.unfinished) throw _unfinishedBy(outcome);
      throw _failure(_pushStep, error);
    }
    _pushed = true;
    if (turn != null) _finish(turn, 'The current turn ended');
    _finish(pushing, _pushedDone);
    final index = _begin('Starting the session on ${plan.targetName}…');
    final SessionBackend session;
    try {
      session = await _arrive(carried);
    } on Object {
      _set(index, MoveStepState.failed);
      rethrow;
    }
    _finish(index, 'Started the session on ${plan.targetName}');
    final sealing = await _settle(transfer);
    if (sealing.settled == Settled.sealed) return session;
    if (sealing.settled == Settled.unfinished) {
      throw MoveUnconfirmed(
        _unfinished(sealing.waitingFor ?? plan.sourceName),
        session: session,
      );
    }
    throw _failure(
      'sealing it on ${plan.sourceName}',
      'the move was released after the session started',
    );
  }

  Future<Outcome> _settle(String transfer) => settleMove(
    source: _source,
    target: _verified,
    sourceName: plan.sourceName,
    targetName: plan.targetName,
    threadId: threadId,
    transfer: transfer,
    startedHere: true,
    age: Duration.zero,
  );

  Future<SessionBackend> _arrive(CarriedMove carried) async {
    final ProjectBackend live;
    try {
      live = await _verified.backend;
    } on Object catch (error) {
      _verified.drop();
      return _afterArrival(carried.transfer, error);
    }
    try {
      return await live.moveIn(carried, root: _root);
    } on Object catch (error) {
      _verified.drop();
      return _afterArrival(carried.transfer, error);
    }
  }

  Future<SessionBackend> _afterArrival(String transfer, Object error) async {
    final outcome = await _settle(transfer);
    switch (outcome.settled) {
      case Settled.sealed:
        return await _openArrived(transfer) ??
            (throw MoveUnconfirmed(_unfinished(plan.targetName)));
      case Settled.unfinished:
        throw _unfinishedBy(outcome);
      case Settled.released:
        throw _failure('starting it on ${plan.targetName}', error);
    }
  }

  Future<SessionBackend?> _openArrived(String transfer) async {
    try {
      final arrival = await (await _verified.backend).moveStatus(transfer);
      final thread = arrival.thread;
      if (arrival.kind != BridgeArrivalKind.landed || thread == null) {
        return null;
      }
      return await (await _verified.backend).open(thread);
    } on Object {
      _verified.drop();
      return null;
    }
  }

  String get _pushingNow {
    final branch = plan.branch ?? 'the branch';
    if (plan.commit && plan.uncommitted > 0) {
      return 'Committing ${plural(plan.uncommitted, 'file')} and pushing $branch…';
    }
    return 'Pushing $branch to its remote…';
  }

  String get _pushStep => plan.commit && plan.uncommitted > 0
      ? 'committing and pushing'
      : 'pushing';

  String get _pushedDone {
    final branch = plan.branch ?? 'the branch';
    if (plan.commit && plan.uncommitted > 0) {
      return 'Committed ${plural(plan.uncommitted, 'file')} and pushed $branch to its remote';
    }
    return 'Pushed $branch to its remote';
  }

  Future<T> _step<T>(
    String now,
    String done,
    String name,
    Future<T> Function() act,
  ) async {
    final index = _begin(now);
    try {
      final value = await act();
      _finish(index, done);
      return value;
    } on Object catch (error) {
      _set(index, MoveStepState.failed);
      throw _failure(name, error);
    }
  }

  MoveUnconfirmed _unfinishedBy(Outcome outcome) =>
      MoveUnconfirmed(_unfinished(outcome.waitingFor ?? plan.targetName));

  String _unfinished(String host) =>
      'The move of “${plan.title}” to ${plan.targetName} is not finished yet. '
      'Liquid finishes it when $host answers.';

  MoveFailure _failure(String step, Object error) => MoveFailure(
    step,
    [
      'Could not move “${plan.title}” to ${plan.targetName}; it stays on ${plan.sourceName}',
      if (_pushed) ', its commit is pushed',
      if (_cloned) ', the clone at $_root remains',
      if (_brought)
        ', the configuration is already brought to ${plan.targetName}',
      ': ${messageOf(error)}',
    ].join(),
  );

  int _begin(String text) {
    _listed.add(MoveStep(text, MoveStepState.now));
    steps.value = List.unmodifiable(_listed);
    return _listed.length - 1;
  }

  void _finish(int index, String text) {
    _listed[index] = MoveStep(text, MoveStepState.done);
    steps.value = List.unmodifiable(_listed);
  }

  void _set(int index, MoveStepState state) {
    _listed[index] = MoveStep(_listed[index].text, state);
    steps.value = List.unmodifiable(_listed);
  }
}

@immutable
class MoveCandidate {
  const MoveCandidate({
    required this.address,
    required this.name,
    this.project,
  });

  final String? address;
  final String name;

  final ProjectBackend? project;
}

@immutable
class MoveOption {
  const MoveOption(this.candidate, {required this.available, this.reason});

  final MoveCandidate candidate;
  final bool available;
  final String? reason;
}

class MoveModel extends ChangeNotifier {
  MoveModel({
    required this.source,
    required this.sourceAddress,
    required this.threadId,
    required this.sourceName,
    required this.candidates,
    required this.open,
    required this.running,
    required this.started,
  });

  final ProjectBackend source;
  final String? sourceAddress;
  final String threadId;
  final String sourceName;
  final List<MoveCandidate> candidates;
  final ProjectOpener open;
  final Set<String> running;
  final Set<String> started;

  BridgeMoveCheck? _check;
  final _landings = <String, BridgeLanding>{};
  String? _problem;
  List<MoveOption> _options = const [];
  MoveOption? _selected;
  List<BridgeHostProject> _projects = const [];
  ConfigurationChanges? _configuration;
  int _selection = 0;
  bool _commit = true;
  bool _useProject = false;
  bool _bringConfiguration = false;
  BridgeHostProject? _picked;
  String _clonePath = '';
  MoveFlow? _flow;

  bool _loaded = false;

  BridgeMoveCheck? get check => _check;
  String? get problem => _problem;

  List<MoveOption> get options => _options;
  MoveOption? get selected => _selected;

  List<BridgeHostProject> get matching => [
    for (final project in _projects)
      if (_sameRemote(project)) project,
  ];

  List<BridgeHostProject> get projects => [
    ...matching,
    for (final project in _projects)
      if (!_sameRemote(project)) project,
  ];

  BridgeHostProject? get picked => _picked;

  BridgeHostProject? get _preferredMatch {
    final matches = matching;
    return matches.where((project) => !_blocked(project.root)).firstOrNull ??
        matches.firstOrNull;
  }

  String? get chosenRoot => (_picked ?? _preferredMatch)?.root;

  BridgeLanding? landingOf(String root) => _landings[root];

  bool _blocked(String root) =>
      _landings[root]?.kind == BridgeLandingKind.blocked;

  bool _sameRemote(BridgeHostProject project) =>
      project.remoteUrl != null && project.remoteUrl == _check?.remoteUrl;

  String? reasonAgainst(BridgeHostProject project) {
    if (_sameRemote(project)) return null;
    return project.remoteUrl == null ? noRemoteReason : otherRemoteReason;
  }

  Future<void> addFolder(String root) async {
    final target = _selected?.candidate.project;
    if (target == null) return;
    final project = await target.addProject(root);
    _projects = [
      ..._projects.where((existing) => existing.root != project.root),
      project,
    ];
    notifyListeners();
  }

  List<BridgeConfigChange> get differences =>
      _configuration?.changes ?? const [];

  bool get commit => _commit;
  set commit(bool value) => _update(() => _commit = value);

  bool get useProject => _useProject;
  set useProject(bool value) => _update(() {
    _useProject = value;
    if (!value) _picked = null;
  });
  bool get bringConfiguration => _bringConfiguration;
  set bringConfiguration(bool value) =>
      _update(() => _bringConfiguration = value);

  void pick(BridgeHostProject? project) {
    if (project != null && reasonAgainst(project) != null) return;
    _update(() => _picked = project);
  }

  String get clonePath => _clonePath;
  set clonePath(String value) => _update(() => _clonePath = value);

  MoveFlow? get flow => _flow;

  void _update(void Function() change) {
    change();
    notifyListeners();
  }

  Future<void> load() async {
    try {
      _check = await source.moveCheck(threadId);
    } on Object catch (error) {
      _problem = messageOf(error);
      notifyListeners();
      return;
    }
    _clonePath = source.root;
    final options = [
      for (final candidate in candidates) await _optionOf(candidate),
    ];
    _options = options;
    notifyListeners();
    final first = options.where((option) => option.available).firstOrNull;
    if (first != null) await select(first);
  }

  Future<Map<String, BridgeLanding>> _landingsOf(
    ProjectBackend project,
    List<BridgeHostProject> projects,
  ) async {
    final branch = _check?.branch;
    if (branch == null) return const {};
    final landings = <String, BridgeLanding>{};
    for (final candidate in projects.where(_sameRemote)) {
      try {
        landings[candidate.root] = await project.preflight(
          candidate.root,
          branch,
        );
      } on Object catch (_) {}
    }
    return landings;
  }

  Future<MoveOption> _optionOf(MoveCandidate candidate) async {
    final project = candidate.project;
    if (project == null) return MoveOption(candidate, available: false);
    try {
      final supported = await project.supportsMoves();
      return MoveOption(
        candidate,
        available: supported,
        reason: supported ? null : olderHostReason,
      );
    } on Object {
      return MoveOption(candidate, available: false);
    }
  }

  Future<void> select(MoveOption option) async {
    if (!option.available) return;
    final turn = ++_selection;
    _selected = option;
    _loaded = false;
    _landings.clear();
    _projects = const [];
    _configuration = null;
    _picked = null;
    _useProject = false;
    _bringConfiguration = false;
    _problem = null;
    notifyListeners();
    final project = option.candidate.project!;
    try {
      final projects = await project.hostProjects();
      final configuration = await project.configurationChanges(source);
      final landings = await _landingsOf(project, projects);
      if (turn != _selection) return;
      _projects = projects;
      _configuration = configuration;
      _landings
        ..clear()
        ..addAll(landings);
      _loaded = true;
    } on Object catch (error) {
      if (turn != _selection) return;
      _problem =
          "Could not read ${option.candidate.name}'s projects or configuration: ${messageOf(error)}";
    }
    notifyListeners();
  }

  MovePlan? get plan {
    final check = _check;
    final option = _selected;
    if (check == null || option == null || !_loaded) return null;
    final matches = matching;
    final picked = _picked;
    return MovePlan(
      title: check.title,
      sourceName: sourceName,
      targetName: option.candidate.name,
      branch: check.branch,
      uncommitted: check.uncommittedFiles,
      running: check.running,
      commit: _commit,
      root: chosenRoot ?? _clonePath,
      cloneRemote: picked == null && matches.isEmpty ? check.remoteUrl : null,
      bringing: _bringConfiguration && differences.isNotEmpty
          ? _configuration
          : null,
    );
  }

  bool get canMove => _flow == null && plan != null && blockedReason == null;

  String? get blockedReason {
    final check = _check;
    if (check == null || _flow != null) return null;
    if (check.blocked case final blocked?) return blocked;
    if (_picked == null && matching.isEmpty && check.remoteUrl == null) {
      return noRemoteFilesReason;
    }
    final root = plan?.root;
    if (root != null && _blocked(root)) {
      return _landings[root]?.reason ?? 'This project cannot take the session.';
    }
    if (plan?.root.isEmpty ?? false) {
      return 'Enter the folder of the project on ${_selected!.candidate.name}.';
    }
    return null;
  }

  Future<SessionBackend> start() async {
    final plan = this.plan!;
    final target = _selected!.candidate;
    final flow = MoveFlow(
      open: open,
      from: (address: sourceAddress, root: source.root),
      to: (address: target.address, root: target.project!.root),
      threadId: threadId,
      plan: plan,
      running: running,
      started: started,
    );
    _flow = flow;
    notifyListeners();
    return flow.run();
  }
}
