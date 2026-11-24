part of 'session_state.dart';

extension SessionLifecycle on SessionState {
  Future<void> showMoved(SessionBackend session, String? address) async {
    _attach(session, ++_switchRequests, address);
    await loadStart();
  }

  Future<void> newSession() async {
    agents.composing.value = false;
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

  Future<void> draftFor(BridgeCheckedOut checked) async {
    await newSession();
    worktree.value = checked.worktree;
    newWorktree.value = false;
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
        lastVisit.value = recent.value
            .where((listed) => listed.thread.id == next.id)
            .firstOrNull
            ?.thread
            .visitedMs;
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
            await _detach();
            notify(NoticeLevel.error, 'Could not open the session: $error');
          }
          return;
        }
        unawaited(readStatistics());
        unawaited(readChanges());
        unawaited(_follow(next, host, generation));
        unawaited(agents.attached(next.id, address));
        unawaited(next.visit().catchError((Object _) {}));
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
    agents.detached();
    lastVisit.value = null;
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
}
