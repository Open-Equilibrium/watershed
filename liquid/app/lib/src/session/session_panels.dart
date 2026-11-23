part of 'session_state.dart';

extension SessionPanels on SessionState {
  Future<void> showTurns(DiffScope scope) async {
    diffScope.value = scope;
    await readChanges();
  }

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
}
