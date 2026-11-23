part of 'session_state.dart';

extension SessionHosts on SessionState {
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

  Future<ProjectBackend> connectTo(String? address, String root) {
    final open = openProject;
    if (open == null) {
      throw StateError(
        'Liquid cannot open its own connection to ${hostNameOf(address)}.',
      );
    }
    return open(address, root);
  }

  bool answers(String? address) => _agents.answers(address);

  ProjectBackend? get _viewProject =>
      _agents[_session == null ? _startAddress : _sessionAddress];

  ProjectBackend _connected(String? address) =>
      _agents[address] ?? (throw StateError(_unanswered(address)));

  String _unanswered(String? address) =>
      hostProblems.value[address] ??
      'Liquid is not connected to the agent host.';

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
      unawaited(agents.load());
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
}

BridgeModelChoice? _firstModel(List<BridgeProvider> providers) {
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
