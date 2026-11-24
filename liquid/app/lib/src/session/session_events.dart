part of 'session_state.dart';

extension SessionEvents on SessionState {
  void _restore(BridgeSnapshot snapshot) {
    _sequence = snapshot.sequence;
    _held = null;
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
    routine: item.routine,
    opening: item.opening,
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

  void _receive(BridgeEvent event) {
    final held = _held;
    if (held != null &&
        event.kind != BridgeEventKind.fellBehind &&
        event.kind != BridgeEventKind.rolledBack) {
      held.add(event);
      return;
    }
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
        if (event.kind == BridgeEventKind.turnCompleted) {
          unawaited(agents.turnEnded());
        }
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
    final turnRead = ++_turnReads;
    try {
      final read = await session.turns();
      if (generation != _generation) return;
      if (turnRead == _turnReads) {
        _setTurns(read);
        _status();
      }
      if (ended) {
        final title = await session.title();
        if (generation != _generation) return;
        sessionTitle.value = title;
        unawaited(readChanges());
        unawaited(readStatistics());
        unawaited(loadStart());
      }
    } on Object catch (error) {
      notify(NoticeLevel.warning, 'Could not read the turns: $error');
    }
  }

  void _replay(List<BridgeEvent> held, int after) {
    _held = null;
    for (final event in held) {
      if (event.sequence > after) _receive(event);
    }
  }

  Future<void> _reload() async {
    final session = _session;
    final generation = _generation;
    if (session == null) return;
    final reload = ++_reloads;
    _held ??= [];
    try {
      final snapshot = await session.snapshot();
      if (generation != _generation || reload != _reloads) return;
      final held = _held ?? const <BridgeEvent>[];
      _restore(snapshot);
      _replay(held, snapshot.sequence);
      unawaited(readChanges());
      unawaited(readStatistics());
    } on Object catch (error) {
      if (generation == _generation && reload == _reloads) {
        _replay(_held ?? const [], _sequence);
        notify(NoticeLevel.warning, 'Could not reload the Chat: $error');
      }
    }
  }
}
