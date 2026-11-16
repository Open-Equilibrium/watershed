import 'dart:async';

import 'package:flutter/foundation.dart';

import '../diff/turn_scope.dart';
import '../rust/api/details.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/session.dart';
import '../rust/api/thread.dart';
import 'backend.dart';
import 'chat_items.dart';
import 'git_state.dart';

export 'chat_items.dart';
export 'git_state.dart';

/// What the session is doing, as the status bar and sidebar show it.
enum RunStatus { idle, running, waitingForYou }

/// How urgent a notice is.
enum NoticeLevel { info, warning, error }

/// A notice from Liquid, as the status bar shows it.
@immutable
class Notice {
  const Notice(this.level, this.text, this.at);

  final NoticeLevel level;
  final String text;
  final DateTime at;
}

/// Notices the status bar keeps for its list.
const noticeLimit = 50;

/// A window's agent sessions: the project's sessions, the one open, and its Chat, Diff, Git and
/// statistics, all from Meta-Harness's T3 server (ADR-0280).
class SessionState {
  SessionState(this._project) {
    hideWhitespace.addListener(() => unawaited(readChanges()));
  }

  final ProjectBackend _project;
  SessionBackend? _session;
  StreamSubscription<BridgeEvent>? _subscription;

  /// The follows of the open session's card lists, cancelled when the session changes.
  final _follows = <StreamSubscription<dynamic>>[];

  /// Incremented on each switch, so reads of an earlier session never land.
  int _generation = 0;

  /// Counts the switches the user asked for, so an open or start that returns after a later switch
  /// is released, not shown.
  int _switchRequests = 0;

  /// The open session, or null on the start page.
  SessionBackend? get session => _session;

  /// The project on the agent host, whose writer model Settings reads and writes.
  ProjectBackend get project => _project;

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);

  /// The request the agent waits on that the Chat shows, oldest first.
  final request = ValueNotifier<BridgeRequest?>(null);
  final sessionTitle = ValueNotifier<String?>(null);

  /// Text for the input area, such as the prompt "Edit from here" returns.
  final draft = ValueNotifier<String?>(null);
  final notice = ValueNotifier<Notice?>(null);
  final notices = ValueNotifier<List<Notice>>(const []);
  final totals = ValueNotifier(const SessionTotals());

  /// Turns queued behind the running one, oldest first.
  final queued = ValueNotifier<List<BridgeTurn>>(const []);
  final turns = ValueNotifier<List<BridgeTurn>>(const []);
  final statistics = ValueNotifier<BridgeStatistics?>(null);

  /// Why the statistics could not be read, until a read succeeds.
  final statisticsProblem = ValueNotifier<({String title, String text})?>(null);
  final contextWindow = ValueNotifier<BridgeContext?>(null);
  final statisticsOpen = ValueNotifier(false);

  /// What the session card lists for the open session: its local servers, its scheduled tasks,
  /// and its related threads; the provider's usage limits; and the host's name.
  final servers = ValueNotifier<List<BridgeLocalServer>>(const []);
  final automations = ValueNotifier<List<BridgeAutomation>>(const []);
  final lineage = ValueNotifier<BridgeLineage?>(null);
  final limits = ValueNotifier<BridgeUsageLimits?>(null);
  final hostName = ValueNotifier<String?>(null);

  /// Asks the Chat to scroll to a turn, by its number.
  final scrollChatTo = ValueNotifier<int?>(null);

  /// The project's sessions, newest first, as the sidebar lists them.
  final recent = ValueNotifier<List<BridgeThreadSummary>>(const []);
  final providers = ValueNotifier<List<BridgeProvider>>(const []);

  /// The model the next session or turn uses.
  final model = ValueNotifier<BridgeModelChoice?>(null);
  final permissions = ValueNotifier(BridgePermissions.fullAccess);

  /// Whether a new session starts in a new worktree, T3's default, or in the project folder.
  final newWorktree = ValueNotifier(true);

  /// The ref a new worktree starts from; the repository's default branch until chosen.
  final base = ValueNotifier<String?>(null);

  /// An existing worktree the next session starts in, as T3's pull request checkout leaves one.
  final worktree = ValueNotifier<String?>(null);

  /// Git for the open session's checkout, else for the project folder.
  late final git = ValueNotifier<GitState>(_gitFor(_project.git()));

  GitState _gitFor(GitBackend backend) {
    final state = GitState(
      backend,
      onError: (title, message) =>
          notify(NoticeLevel.error, '$title: $message'),
    );
    // Uncommitted changes have no event of their own; the checkout's status says they moved.
    state.status.addListener(() {
      final scope = diffScope.value;
      if (scope is UncommittedChanges || scope is BranchChanges) {
        unawaited(readChanges());
      }
    });
    return state;
  }

  final Map<int, ItemView> _byId = {};
  final Map<String, BridgeRequest> _requests = {};
  int _sequence = 0;

  /// Reads the project's sessions and models for the start page.
  Future<void> loadStart() async {
    try {
      recent.value = await _project.threads();
      providers.value = await _project.providers();
      model.value ??= _firstModel(providers.value);
      if (base.value == null) {
        final refs = await _project.git().refs('', limit: 200);
        base.value = refs.refs.where((r) => r.default_).firstOrNull?.name;
      }
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not read the sessions: $error');
    }
  }

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

  /// Leaves the open session for the start page.
  Future<void> newSession() async {
    _switchRequests++;
    await _detach();
    worktree.value = null;
    _setGit(_project.git());
    unawaited(loadStart());
  }

  /// Opens the stored session `id`.
  Future<void> open(String id) async {
    final switchRequest = ++_switchRequests;
    try {
      _attach(await _project.open(id), switchRequest);
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not open the session: $error');
    }
  }

  /// Sends `text`: on the start page it starts a session where the workspace selector says;
  /// otherwise an idle session starts a turn, and a running one takes it as a steer with
  /// `steer` or queues it.
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
      final switchRequest = ++_switchRequests;
      _attach(
        await _project.start(
          text,
          chosen,
          base: newWorktree.value ? base.value : null,
          worktree: newWorktree.value ? null : worktree.value,
          permissions: permissions.value,
        ),
        switchRequest,
      );
      unawaited(loadStart());
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not send the message: $error');
    }
  }

  /// Points the open session at `branch`, in `worktree` when the branch picker chose another
  /// one; Git follows it.
  Future<void> setBranch(String branch, {String? worktree}) async {
    final session = _session;
    if (session == null) return;
    final generation = _generation;
    await session.setBranch(branch, worktree: worktree);
    // A switch meanwhile released this session; the session it went to brings its own Git.
    if (generation == _generation) _setGit(session.git());
  }

  /// Opens the start page on a checked-out pull request, as T3 drafts a thread for it.
  Future<void> draftFor(BridgeCheckedOut checked) async {
    await newSession();
    worktree.value = checked.worktree;
    newWorktree.value = false;
  }

  /// Interrupts the running turn.
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

  /// Answers an approval with one of its options.
  Future<void> respond(BridgeRequest request, String decision) =>
      _act(() => _session!.respond(request.id, decision), 'answer');

  /// Answers questions.
  Future<void> answer(
    BridgeRequest request,
    List<BridgeQuestionAnswer> answers,
  ) => _act(() => _session!.answer(request.id, answers), 'answer');

  /// Dismisses questions without answering.
  Future<void> dismiss(BridgeRequest request) =>
      _act(() => _session!.dismiss(request.id), 'dismiss the question');

  /// T3's "Edit from here": rewinds to before `turn`, with `restoreFiles` its files too, and
  /// puts its prompt back into the input area.
  Future<bool> revert(int turn, {required bool restoreFiles}) async {
    try {
      draft.value = await _session!.revert(turn, restoreFiles: restoreFiles);
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not revert: $error');
      return false;
    }
  }

  /// T3's "Fork from this response": opens a new session from the end of `turn`.
  Future<void> fork(int turn) async {
    try {
      final id = await _session!.fork(turn);
      await open(id);
      unawaited(loadStart());
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Failed to fork this response. $error');
    }
  }

  /// Sets the model: the open session's next turn, else the next new session's. An open session
  /// takes it first, so a refused change is not shown, and reads its context window again, which
  /// depends on the model.
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

  /// Sets what the agent may do without asking. An open session takes it first, so a refused
  /// change is not shown.
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

  /// Whether `id` is the only session in its worktree, so deleting it may delete that too.
  Future<bool> onlySessionInWorktree(String id) =>
      _project.onlySessionInWorktree(id);

  /// Deletes the session `id`, with `worktree` its worktree too; the open one goes to the
  /// start page first.
  Future<bool> delete(String id, {required bool worktree}) async {
    try {
      if (_session?.id == id) await newSession();
      await _project.delete(id, worktree: worktree);
      await loadStart();
      return true;
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not delete the session: $error');
      return false;
    }
  }

  /// Archives or restores the session `id`.
  Future<void> archive(String id, {required bool archived}) => _act(() async {
    await _project.archive(id, archived: archived);
    await loadStart();
  }, archived ? 'archive the session' : 'restore the session');

  /// Writes the open session's record into Downloads; returns the folder.
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

  /// Stops the server `pid` listens on at `port`, from the card's Servers.
  Future<void> stopServer(int pid, int port) =>
      _act(() => _session!.stopServer(pid, port), 'stop server');

  /// Pauses or resumes the scheduled task `id`, from the card's Automations.
  Future<void> setAutomationEnabled(String id, bool enabled) => _act(
    () => _session!.setAutomationEnabled(id, enabled: enabled),
    'update automation',
  );

  /// Runs the scheduled task `id` now.
  Future<void> runAutomation(String id) =>
      _act(() => _session!.runAutomationNow(id), 'run automation');

  /// Merges the session back into its parent, from the card's Lineage; then opens the parent, as
  /// T3 does.
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
    // Only a session still open is switched to the target; the user may have left it meanwhile.
    if (target != null && generation == _generation) await open(target);
  }

  /// Disconnects the agent session, from the card's Lineage.
  Future<void> disconnect() =>
      _act(() => _session!.disconnect(), 'disconnect agent session');

  /// Stops the subagent `id`, from the card's Lineage.
  Future<void> stopSubagent(String id) =>
      _act(() => _session!.stopSubagent(id), 'stop subagent');

  Future<void> _act(Future<void> Function() action, String what) async {
    try {
      await action();
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not $what: $error');
    }
  }

  // The Diff panel.

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

  /// Counts the reads of the Diff, so a read that a newer one overtakes never lands.
  int _diffRequests = 0;

  /// Reads the turns and what the shown ones changed.
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

  /// One file's unified diff for the shown Diff scope.
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

  /// Reads the statistics and the context window (ADR-0279).
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

  // Following the open session.

  /// Shows `next` unless a later switch was asked for meanwhile, then releases it.
  void _attach(SessionBackend next, int switchRequest) {
    if (switchRequest != _switchRequests) {
      next.close();
      return;
    }
    unawaited(
      _detach().then((_) async {
        if (switchRequest != _switchRequests) {
          next.close();
          return;
        }
        _session = next;
        final generation = ++_generation;
        try {
          _setGit(next.git());
          final subscribed = await next.subscribe();
          if (generation != _generation) return;
          _restore(subscribed.snapshot);
          final events = next.events(subscribed.id);
          _subscription = events.listen(
            _receive,
            onError: (Object error) {
              if (generation == _generation) {
                notify(
                  NoticeLevel.error,
                  'Could not follow the session: $error',
                );
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
        unawaited(_follow(next, generation));
      }),
    );
  }

  Future<void> _detach() async {
    _generation++;
    await _subscription?.cancel();
    _subscription = null;
    for (final follow in _follows) {
      unawaited(follow.cancel());
    }
    _follows.clear();
    // Releasing the session ends its follows on T3 at once, not when Dart collects it.
    _session?.close();
    servers.value = const [];
    automations.value = const [];
    lineage.value = null;
    limits.value = null;
    hostName.value = null;
    _session = null;
    _sequence = 0;
    _applied = 0;
    _confirmed = 0;
    _byId.clear();
    _requests.clear();
    items.value = const [];
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

  /// Follows the open session's servers, scheduled tasks and related threads, and reads the host's
  /// name; a switch cancels the follows and drops what a read brings late.
  Future<void> _follow(SessionBackend next, int generation) async {
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
        'the host name',
        _project.environmentName,
        (value) => hostName.value = value,
      ),
    ]);
  }

  /// Reads a live section, then follows its changes: the bridge streams them only after the
  /// read subscribed to them. A failed change stream is a warning.
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

  /// Passes a read's value to `set` unless the session changed meanwhile; a failed read is a
  /// warning.
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

  /// Shows `snapshot` and remembers its sequence, so the events it contains are skipped.
  void _restore(BridgeSnapshot snapshot) {
    _sequence = snapshot.sequence;
    _byId.clear();
    for (final entry in snapshot.transcript) {
      final view = _view(entry.item, entry.turn)..text.value = entry.text;
      if (entry.item.ended) {
        view.outcome.value = (
          failed: entry.item.failed,
          durationMs: entry.item.durationMs,
        );
      }
      _byId[view.id] = view;
    }
    _requests
      ..clear()
      ..addEntries(snapshot.requests.map((r) => MapEntry(r.id, r)));
    _setTurns(snapshot.turns);
    _status();
  }

  ItemView _view(BridgeItem item, int turn) => ItemView(
    id: item.id,
    kind: item.kind,
    turn: turn,
    tool: item.tool,
    target: item.target,
  );

  /// Sets the turns and rebuilds the Chat's rows: each turn's items, then a row with what it
  /// changed once its checkpoint is ready.
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
    items.value = rows;
  }

  void _status() {
    request.value = _requests.values.firstOrNull;
    final running = turns.value.any(
      (t) => t.status == BridgeTurnStatus.running,
    );
    status.value = _requests.isNotEmpty
        ? RunStatus.waitingForYou
        : running
        ? RunStatus.running
        : RunStatus.idle;
  }

  /// Events applied so far, and the count Liquid last confirmed to the bridge.
  int _applied = 0;
  int _confirmed = 0;

  /// The generation whose confirmations are in flight, so a switch starts its own.
  int? _confirmingFor;

  void _receive(BridgeEvent event) {
    try {
      _apply(event);
    } finally {
      _applied++;
      if (_confirmingFor != _generation) unawaited(_confirm());
    }
  }

  /// Confirms what has been applied, one call at a time (ADR-0272).
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
    } on Object {
      // The session is gone, so nothing waits for the count.
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
    // The snapshot already holds this event (ADR-0290).
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

  /// Restores the Chat from a new snapshot after falling behind or a revert (ADR-0290).
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
    await _detach();
    git.value.dispose();
  }
}
