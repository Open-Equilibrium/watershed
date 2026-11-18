import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'agent/sidebar.dart';
import 'agent_hosts/agent_hosts.dart';
import 'rust/api/replica.dart';
import 'rust/api/session.dart';
import 'session/backend.dart';
import 'session/session_state.dart';
import 'workspace/workspace_state.dart';

class Launch {
  const Launch.session(this.state, this.info) : problem = null;
  const Launch.problem(this.problem) : state = null, info = null;

  final SessionState? state;
  final SessionInfo? info;
  final String? problem;
}

String projectName(String project) =>
    Directory(project).absolute.uri
        .normalizePath()
        .pathSegments
        .lastWhere((part) => part.isNotEmpty, orElse: () => '/');

Launch launchFromEnvironment(
  Map<String, String> environment, {
  String? startAddress,
}) {
  final project = environment['LIQUID_PROJECT'];
  if (project == null) {
    return const Launch.problem(
      'Start Liquid with LIQUID_PROJECT set to a project directory.',
    );
  }
  return Launch.session(
    SessionState(null, startAddress),
    SessionInfo(
      project: projectName(project),
      path: Directory(project).absolute.path,
    ),
  );
}

Duration retryDelay(int failureCount, double random) {
  final ceiling = min(300000, 1000 * pow(2, min(failureCount + 1, 30)).toInt());
  return Duration(milliseconds: (ceiling / 2 + ceiling / 2 * random).round());
}

const _stableAfter = Duration(seconds: 30);

class RustAgentHostLinks implements AgentHostLinks {
  RustAgentHostLinks(
    this._launch,
    Map<String, String> environment, {
    Future<ProjectBackend> Function(String? address, String root)? open,
    Future<String> Function(ProjectBackend project)? machineOf,
    Random? random,
    DateTime Function()? clock,
  }) : _environment = environment,
       _open =
           open ??
           ((address, root) =>
               _connect(address, root, environment['META_HARNESS_HOME'])),
       _machineOf = machineOf ?? ((project) => project.environmentMachine()),
       _random = random ?? Random(),
       _clock = clock ?? DateTime.now {
    _launch.state?.onHostLost = _retryLater;
    _launch.state?.openProject = _open;
  }

  final Launch _launch;
  final Map<String, String> _environment;
  final Future<ProjectBackend> Function(String? address, String root) _open;
  final Future<String> Function(ProjectBackend project) _machineOf;
  final Random _random;
  final DateTime Function() _clock;

  final _attempts = <String?, Object>{};
  final _retries = <String?, Timer>{};
  final _failures = <String?, int>{};
  final _connectedAt = <String?, DateTime>{};

  String get _project => _environment['LIQUID_PROJECT'] ?? '';

  @override
  Future<void> connect(String? address) async {
    final state = _launch.state;
    if (state == null ||
        state.answers(address) ||
        _attempts.containsKey(address)) {
      return;
    }
    final attempt = _attempts[address] = Object();
    bool current() => identical(_attempts[address], attempt);
    _retries.remove(address)?.cancel();
    try {
      final refusal = _refusal(address);
      if (refusal != null) {
        state.failHost(address, refusal);
      } else {
        final root = address == null
            ? Directory(_project).absolute.path
            : _project;
        final project = await _open(address, root);
        final machine = address == null ? null : await _kindOf(project);
        if (!current()) {
          project.close();
          return;
        }
        await state.addHost(address, project, machine: machine);
        _connectedAt[address] = _clock();
      }
    } on Object catch (error) {
      if (current()) state.failHost(address, hostProblem(address, error));
    } finally {
      if (current()) _attempts.remove(address);
    }
  }

  Future<String> _kindOf(ProjectBackend project) async {
    try {
      return await _machineOf(project);
    } on Object {
      return 'server';
    }
  }

  void _retryLater(String? address) {
    final since = _connectedAt.remove(address);
    if (since != null && _clock().difference(since) >= _stableAfter) {
      _failures.remove(address);
    }
    final firstLoss = since != null && (_failures[address] ?? 0) == 0;
    if (_refusal(address) != null) return;
    final failures = (_failures[address] ?? 0) + 1;
    _failures[address] = failures;
    final random = _random.nextDouble();
    final delay = firstLoss
        ? Duration(milliseconds: (250 * (0.5 + random / 2)).round())
        : retryDelay(failures - 1, random);
    _retries[address]?.cancel();
    _retries[address] = Timer(delay, () {
      _retries.remove(address);
      unawaited(connect(address));
    });
  }

  void retryAll() {
    final state = _launch.state;
    if (state == null) return;
    for (final address in state.hostProblems.value.keys.toList()) {
      _failures.remove(address);
      unawaited(connect(address));
    }
  }

  String? _refusal(String? address) {
    if (address != null) {
      return Directory(_project).isAbsolute
          ? null
          : 'LIQUID_PROJECT must be an absolute path on the agent host $address: $_project';
    }
    if (!Directory(_project).existsSync()) {
      return 'LIQUID_PROJECT names no directory: $_project';
    }
    final home = _environment['META_HARNESS_HOME'];
    if (home != null && !File(home).isAbsolute) {
      return 'META_HARNESS_HOME must be an absolute path: $home';
    }
    return null;
  }

  @override
  void disconnect(String address) {
    _attempts.remove(address);
    _retries.remove(address)?.cancel();
    _connectedAt.remove(address);
    _failures.remove(address);
    final state = _launch.state;
    if (state != null) unawaited(state.removeHost(address));
  }

  @override
  Future<String?> startOn(String? address) async {
    final state = _launch.state;
    if (state == null) return _launch.problem;
    if (!state.answers(address)) await connect(address);
    if (!state.answers(address)) {
      return state.hostProblems.value[address] ??
          'Liquid could not connect to the agent host.';
    }
    await state.startOn(address);
    return null;
  }

  static Future<ProjectBackend> _connect(
    String? address,
    String root,
    String? home,
  ) async {
    final host = address == null
        ? await AgentHost.connect(home: home)
        : await AgentHost.connectRemote(address: address);
    return RustProjectBackend(host, await host.project(root: root), root: root);
  }
}

Future<(WorkspaceState?, String?)> openWorkspace(
  String dir, {
  required bool show,
}) async {
  final LiquidWorkspace workspace;
  try {
    workspace = await LiquidWorkspace.open(dir: dir);
  } on Object catch (error) {
    return (null, 'The Workspace did not open: $error');
  }
  return startWorkspace(workspace, show: show);
}

Future<(WorkspaceState, String?)> startWorkspace(
  LiquidWorkspace workspace, {
  required bool show,
}) async {
  final state = WorkspaceState(workspace);
  try {
    await state.start(show: show);
  } on Object catch (error) {
    return (state, 'The Workspace did not load: $error');
  }
  return (state, null);
}
