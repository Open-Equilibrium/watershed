part of 'session_state.dart';

extension SessionActions on SessionState {
  Future<bool> send(String text, {bool steer = false}) async {
    final session = _session;
    try {
      if (session != null) {
        if (agents.isMention(text)) return await agents.mention(text);
        await session.send(text, steer: steer);
        return true;
      }
      if (agents.composing.value) return await agents.start(text);
      final chosen = model.value;
      if (chosen == null) {
        notify(NoticeLevel.error, 'No model is available.');
        return false;
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
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not send the message: $error');
      return false;
    }
  }

  Future<void> setBranch(String branch, {String? worktree}) async {
    final session = _session;
    if (session == null) return;
    final generation = _generation;
    await session.setBranch(branch, worktree: worktree);
    if (generation == _generation) _setGit(session.git());
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
    final generation = _generation;
    try {
      final text = await _session!.revert(turn, restoreFiles: restoreFiles);
      if (generation == _generation) draft.value = text;
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not revert: $error');
      return false;
    }
  }

  Future<void> fork(int turn) async {
    final session = _session;
    final address = _sessionAddress;
    final generation = _generation;
    try {
      final id = await session!.fork(turn);
      if (generation != _generation) return;
      await open(id, address: address);
      unawaited(loadStart());
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Failed to fork this response. $error');
    }
  }

  Future<void> setModel(BridgeModelChoice next) async {
    if (_session case final session?) {
      final generation = _generation;
      await _act(() async {
        await session.setModel(next);
        if (generation == _generation) model.value = next;
      }, 'apply the model selection');
      await readStatistics();
    } else {
      model.value = next;
    }
  }

  Future<void> setPermissions(BridgePermissions next) async {
    if (_session case final session?) {
      final generation = _generation;
      await _act(() async {
        await session.setPermissions(next);
        if (generation == _generation) permissions.value = next;
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
}
