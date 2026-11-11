import 'dart:async';

import 'package:flutter/foundation.dart';

import '../diff/turn_scope.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/worktrees.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';
import 'backend.dart';
import 'branch_bases.dart';
import 'chat_history.dart';
import 'chat_items.dart';
import 'git_state.dart';
import 'pull_requests.dart';

export 'chat_items.dart';
export 'git_state.dart';

/// What the session is doing, as the status bar and sidebar show it.
enum RunStatus { idle, running, waitingForYou, exited, failed }

/// A notification from the agent or Liquid, as the status bar shows it.
@immutable
class Notice {
  const Notice(this.level, this.text, this.at);

  final BridgeLevel level;
  final String text;
  final DateTime at;
}

/// Notifications the status bar keeps for its list.
const _keptNotices = 20;

/// How long information and warnings stay in the status bar; errors stay until dismissed.
const _noticeFades = Duration(seconds: 6);

/// One agent session's state for the UI. It owns the event subscription and the backend, and
/// releases both in [dispose] (client architecture rule 14).
class SessionState {
  SessionState(this._backend, {BranchBases? bases})
    : _bases = bases ?? BranchBases() {
    _subscription = _backend.events().listen(_apply);
    hideWhitespace.addListener(() => unawaited(readChanges()));
    git.refreshed.addListener(() {
      if (diffScope.value is UncommittedChanges) unawaited(readChanges());
    });
    // Stashed changes restored or discarded leave the Diff tab.
    git.stash.addListener(() {
      if (diffScope.value is StashedChanges) unawaited(readChanges());
    });
  }

  final SessionBackend _backend;
  late final StreamSubscription<BridgeEvent> _subscription;
  final Map<int, ItemView> _byId = {};
  final List<BridgeRequest> _questions = [];
  bool _turnActive = false;

  /// Whether the user stopped the running turn; requests it ends are no failures.
  bool _stopping = false;

  /// Requests that ended before they were reported, which Stop can cause; they never show.
  final Set<String> _endedUnseen = {};

  void _end(String id) {
    _answered.remove(id);
    final before = _questions.length;
    _questions.removeWhere((request) => request.id == id);
    if (_questions.length == before) _endedUnseen.add(id);
  }

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);
  final question = ValueNotifier<BridgeRequest?>(null);

  /// Whether a question waits for the user.
  bool get asking => question.value != null;

  final widgets = ValueNotifier<Map<String, List<String>>>(const {});
  final statuses = ValueNotifier<Map<String, String>>(const {});

  /// The open session's title, as the start page names stored sessions; until pi stored one,
  /// the first line of the first message sent.
  final sessionTitle = ValueNotifier<String?>(null);
  final draft = ValueNotifier<String?>(null);

  /// The notification the status bar shows, and the latest ones, newest first.
  final notice = ValueNotifier<Notice?>(null);
  final notices = ValueNotifier<List<Notice>>(const []);
  Timer? _fade;
  final totals = ValueNotifier(const SessionTotals());
  final failure = ValueNotifier<String?>(null);

  /// Messages waiting for the running agent.
  final queue = ValueNotifier(const BridgeQueue(steering: [], followUp: []));

  /// The chat as the session stores it, read back page by page.
  late final _chat = ChatHistory(
    _backend,
    items: items,
    totals: totals,
    notify: notify,
    reportRead: reportRead,
  );

  /// Whether earlier stored entries of the chat remain to be read.
  ValueNotifier<bool> get chatMore => _chat.more;

  /// The checkout's branch, its counts and its stash.
  late final git = GitState(_backend, notify: notify, bases: _bases);

  /// Where branches Liquid creates record the branch they were created from.
  final BranchBases _bases;

  /// The branch's pull request on GitHub.
  PullRequests get pulls => _pulls ??= PullRequests(_backend, git, _bases);
  PullRequests? _pulls;

  /// Counts the changes of the session's stored history; the Trajectory reloads on each.
  final stored = ValueNotifier(0);

  /// What the start page offers: the project's recent sessions, the models and the selected
  /// one as `provider/id`.
  final recent = ValueNotifier<List<BridgeStoredSession>>(const []);
  final models = ValueNotifier<List<BridgeModel>>(const []);
  final model = ValueNotifier<String?>(null);

  /// Git's registered worktrees of the session's repository, the main checkout first.
  Future<List<BridgeWorktree>> worktrees() => _backend.worktrees();

  /// What removing the worktree at `path` would lose, or what keeps it.
  Future<BridgeWorktreeRemoval> worktreeRemoval(String path) =>
      _backend.worktreeRemoval(path);

  /// Removes the `reviewed` worktree once a recheck finds it unchanged and removable.
  Future<BridgeWorktreeOutcome> removeWorktree(
    BridgeWorktreeRemoval reviewed,
  ) => _backend.removeWorktree(reviewed);

  /// Reads what the start page offers, and the open session's title.
  Future<void> loadStart() async {
    unawaited(loadTitle());
    try {
      recent.value = await _backend.recentSessions();
      models.value = await _backend.models();
      model.value = await _backend.currentModel();
    } on Object catch (error) {
      notify(BridgeLevel.warning, 'Could not read the recent sessions: $error');
    }
  }

  /// Reads the title pi stored for the open session; a session switch meanwhile drops it.
  Future<void> loadTitle() async {
    final generation = _generation;
    try {
      final stored = await _backend.title();
      if (stored != null && generation == _generation) {
        sessionTitle.value = stored;
      }
    } on Object {
      // The title is read again after the next turn.
    }
  }

  /// Starts a new session in the same agent; the current one stays available.
  Future<void> newSession() => _switch(
    _backend.newSession,
    busy: 'Stop the agent before starting a new session.',
  );

  /// Continues a stored session in the same agent.
  Future<void> continueSession(String path) => _switch(
    () => _backend.switchSession(path),
    busy: 'Stop the agent before continuing another session.',
  );

  /// A session switch in progress; messages wait for it and other switches are ignored.
  Completer<void>? _switching;

  /// Counts session switches; a chat read started before the latest one is dropped.
  int _generation = 0;

  /// Identifies the session whose asynchronous UI work is still current.
  int get generation => _generation;

  /// Switches only while the agent is idle, then shows the session it continues.
  Future<void> _switch(
    Future<bool> Function() switchTo, {
    required String busy,
  }) async {
    if (_switching != null) return;
    if (status.value case RunStatus.running || RunStatus.waitingForYou) {
      notify(BridgeLevel.info, busy);
      return;
    }
    final switching = _switching = Completer<void>();
    // Extension displays belong to their session. The new session's extensions may set theirs
    // before pi confirms the switch, so the old ones go now and come back if it does not happen.
    final (shownWidgets, shownStatuses) = (widgets.value, statuses.value);
    widgets.value = const {};
    statuses.value = const {};
    try {
      if (!await _replace(switchTo)) {
        widgets.value = shownWidgets;
        statuses.value = shownStatuses;
      }
    } finally {
      _switching = null;
      switching.complete();
    }
  }

  /// Switches with `switchTo` and shows the session it continues; `false` when it did not.
  Future<bool> _replace(Future<bool> Function() switchTo) async {
    try {
      if (!await switchTo()) {
        notify(BridgeLevel.info, 'An extension kept the current session.');
        return false;
      }
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not change the session: $error');
      return false;
    }
    _generation++;
    _failing.clear();
    sessionTitle.value = null;
    // The previous session's turns no longer apply.
    turns.value = const [];
    diffScope.value = const LatestTurn();
    shownScope.value = const LatestTurn();
    shown.value = null;
    shownStash.value = null;
    changed.value = const [];
    changesProblem.value = null;
    _byId.clear();
    items.value = const [];
    _chat.reset();
    totals.value = const SessionTotals();
    queue.value = const BridgeQueue(steering: [], followUp: []);
    stored.value++;
    await loadChat();
    await loadStart();
    return true;
  }

  /// Selects the model the next requests use.
  Future<void> setModel(BridgeModel next) async {
    try {
      await _backend.setModel(next);
      model.value = '${next.provider}/${next.id}';
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not select ${next.name}: $error');
    }
  }

  /// Sends a message: an idle agent starts with it, a running one takes it before its next
  /// model request or, with `followUp`, once it is done. pi decides, so no turn state races
  /// the send. A message the agent rejects goes back to the input area.
  Future<void> send(String message, {bool followUp = false}) async {
    // A message typed while the session changes goes to the new one, and one typed while the
    // chat is restored or branched follows it, so the restored chat does not hold its items twice
    // and keeps the items of the turn the message starts. One typed during a queue change follows
    // the messages the change keeps.
    await _untilSettled();
    final generation = _generation;
    try {
      await _backend.send(message, followUp: followUp);
      // Named like a stored session until pi stores this one, unless another session replaced it.
      if (generation == _generation) sessionTitle.value ??= _firstLine(message);
    } on Object catch (error) {
      draft.value = message;
      notify(BridgeLevel.error, 'The message was not sent: $error');
    }
  }

  /// Waits until no session switch, branch, chat restore or queue change is running. Each may
  /// start another while it is awaited, so the check repeats.
  Future<void> _untilSettled() async {
    for (var pending = _pending; pending != null; pending = _pending) {
      try {
        await pending;
      } on Object {
        // Each reports its own failure; the message still goes out.
      }
    }
  }

  Future<Object?>? get _pending =>
      _switching?.future ?? _branching ?? _chat.loading ?? _queueChange;

  /// Queue changes run one after another: each clears pi's whole queue, and a removal sends the
  /// others again, which a second removal, Stop or a new message must not overtake. Null while
  /// none runs.
  Future<void>? _queueChange;

  Future<void> _changeQueue(Future<void> Function() change) {
    final next = (_queueChange ?? Future.value()).then((_) => change());
    final settled = next.then((_) {}, onError: (Object _) {});
    _queueChange = settled;
    unawaited(
      settled.whenComplete(() {
        if (identical(_queueChange, settled)) _queueChange = null;
      }),
    );
    return next;
  }

  /// Takes one waiting message out of the queue; pi clears the whole queue, so the others are
  /// sent again in their order. `occurrence` counts the equal messages before it in its queue.
  Future<void> unqueue(
    String message, {
    required bool followUp,
    int occurrence = 0,
  }) => _changeQueue(
    () => _unqueue(message, followUp: followUp, occurrence: occurrence),
  );

  Future<void> _unqueue(
    String message, {
    required bool followUp,
    required int occurrence,
  }) async {
    final BridgeQueue waiting;
    try {
      waiting = await _backend.clearQueue();
    } on Object catch (error) {
      notify(
        BridgeLevel.error,
        'Could not change the waiting messages: $error',
      );
      return;
    }
    var removed = false;
    // Only the queue the chip belongs to loses the message; the same text may wait in both.
    List<String> kept(List<String> texts, {required bool inFollowUp}) {
      final rest = [...texts];
      if (!removed && inFollowUp == followUp) {
        final matches = [
          for (final (index, text) in texts.indexed)
            if (text == message) index,
        ];
        // The cleared queue may hold fewer equal messages than the chip counted; the last goes.
        if (matches.isNotEmpty) {
          rest.removeAt(matches[occurrence.clamp(0, matches.length - 1)]);
          removed = true;
        }
      }
      return rest;
    }

    final again = [
      for (final text in kept(waiting.steering, inFollowUp: false))
        (text, false),
      for (final text in kept(waiting.followUp, inFollowUp: true)) (text, true),
    ];
    for (final (index, (text, followUp)) in again.indexed) {
      try {
        await _backend.send(text, followUp: followUp);
      } on Object catch (error) {
        // What did not reach the agent comes back to the input area.
        draft.value = [for (final (text, _) in again.skip(index)) text]
            .join('\n\n');
        notify(
          BridgeLevel.error,
          'Waiting messages did not reach the agent: $error',
        );
        return;
      }
    }
  }

  /// Stops the run and hands the waiting messages back to the input area for editing.
  Future<void> stop() => _changeQueue(_stop);

  Future<void> _stop() async {
    _stopping = true;
    try {
      final waiting = await _backend.clearQueue();
      final texts = [...waiting.steering, ...waiting.followUp];
      if (texts.isNotEmpty) draft.value = texts.join('\n\n');
      await _backend.abort();
    } on Object catch (error) {
      _stopping = false;
      notify(BridgeLevel.error, 'Could not stop the agent: $error');
    }
  }

  /// Answers the question `request` with text or a selected option.
  Future<void> answerText(String request, String text) =>
      _reply(request, () => _backend.answerText(request, text));

  Future<void> answerConfirm(String request, bool confirmed) =>
      _reply(request, () => _backend.answerConfirm(request, confirmed));

  Future<void> dismiss(String request) =>
      _reply(request, () => _backend.dismiss(request));

  /// Requests answered but not resolved yet.
  final _answered = <String>{};

  /// Sends one answer to `request` while it is open, so a key on a card that is closing never
  /// answers the request behind it, nor the same one twice.
  Future<void> _reply(String request, Future<void> Function() send) async {
    final open = _questions.any((it) => it.id == request);
    if (!open || !_answered.add(request)) return;
    try {
      await send();
    } on Object catch (error) {
      _answered.remove(request);
      notify(BridgeLevel.error, 'The answer did not reach the agent: $error');
    }
  }

  /// The turns of the current branch, oldest first, with what each changed.
  final turns = ValueNotifier<List<BridgeTurn>>(const []);

  /// Which turns the Diff tab compares.
  final diffScope = ValueNotifier<DiffScope>(const LatestTurn());

  /// The checkpoints the Diff tab compares, as last read; null while no turn ended.
  final shown = ValueNotifier<Compared?>(null);

  /// The turns the Diff tab shows, set with [shown] and their files; [diffScope] holds the
  /// choice still being read.
  final shownScope = ValueNotifier<DiffScope>(const LatestTurn());

  /// The stash and its branch read with the shown files, retained during a later refresh.
  final shownStash = ValueNotifier<({BridgeStash stash, String? branch})?>(
    null,
  );

  /// The files the shown turns changed, as last read; null before the first read. The Diff tab
  /// and its count show them.
  final changed = ValueNotifier<List<BridgeFileChange>?>(null);

  /// Equal polls keep the shown patch, but content, baseline or display changes reload it.
  (String, BridgeGitStatus?, bool)? _uncommittedRead;

  /// Tree identities of the published uncommitted list, also validating pending patches.
  String? get uncommittedRevision => _uncommittedRead?.$1;

  /// Why the Diff tab shows no files, with its heading; null while it shows a list.
  final changesProblem = ValueNotifier<({String title, String text})?>(null);

  /// Whether the changes leave out what is only whitespace; a change reads them
  /// again.
  final hideWhitespace = ValueNotifier(false);

  /// Shows `scope` in the Diff tab.
  Future<void> showTurns(DiffScope scope) async {
    diffScope.value = scope;
    if (scope is StashedChanges || scope is UncommittedChanges) {
      await git.refresh();
    }
    await readChanges();
  }

  /// Reads the branch's turns and what the shown ones changed between their checkpoints. A
  /// failed read is reported once, and the Diff tab says so instead of showing earlier changes.
  Future<void> readChanges() {
    // A read asked for meanwhile runs once after the current one instead of queueing.
    if (_readingChanges case final reading?) {
      _changesAgain = true;
      return reading;
    }
    return _readingChanges = _readChanges();
  }

  Future<void>? _readingChanges;
  bool _changesAgain = false;

  Future<void> _readChanges() async {
    try {
      do {
        _changesAgain = false;
        await _readChangesOnce();
      } while (_changesAgain);
    } finally {
      _readingChanges = null;
    }
  }

  Future<void> _readChangesOnce() async {
    final generation = _generation;
    try {
      final started = ++_turnReadsStarted;
      final read = await _backend.turns();
      if (generation != _generation) return;
      turns.value = read;
      _turnsRead = started;
      if (diffScope.value is UncommittedChanges) {
        final whitespace = hideWhitespace.value;
        final (revision, files) = await _backend.uncommittedChanges(
          hideWhitespace: whitespace,
        );
        if (generation != _generation) return;
        final identity = (revision, git.status.value, whitespace);
        final publish =
            _uncommittedRead != identity ||
            shownScope.value is! UncommittedChanges ||
            changesProblem.value != null ||
            changed.value == null;
        _uncommittedRead = identity;
        reportRead('changes', null);
        shownScope.value = const UncommittedChanges();
        shown.value = null;
        changesProblem.value = null;
        if (publish) changed.value = files;
        return;
      }
      if (diffScope.value is StashedChanges) {
        final stash = git.stash.value;
        if (stash != null) {
          final branch = git.status.value?.branch;
          final files = hideWhitespace.value
              ? await _backend.stashChanges(stash.id, hideWhitespace: true)
              : stash.files;
          if (generation != _generation) return;
          reportRead('changes', null);
          shownScope.value = const StashedChanges();
          shownStash.value = (stash: stash, branch: branch);
          changesProblem.value = null;
          changed.value = files;
          return;
        }
        diffScope.value = const LatestTurn();
      }
      final scope = diffScope.value;
      final pair = compared(read, scope);
      if (pair == null) {
        // Outside Git no turn can have checkpoints, so say so before one ends.
        final inRepository = await _backend.isInRepository();
        if (generation != _generation) return;
        reportRead('changes', null);
        shownScope.value = scope;
        shown.value = null;
        changesProblem.value = !inRepository
            ? _outsideGit
            : switch (read.lastOrNull?.failure) {
                final failure? => _noCheckpoint(failure),
                null => (
                  title: 'No finished turn yet',
                  text: 'A turn’s changes appear here when it ends.',
                ),
              };
        changed.value = const [];
        return;
      }
      final changes = await _backend.turnChanges(
        pair.from.id,
        pair.to.id,
        hideWhitespace: hideWhitespace.value,
      );
      if (generation != _generation) return;
      reportRead('changes', null);
      // The heading changes only with its files, so no patch stands under another turn.
      shownScope.value = scope;
      shown.value = pair;
      // A missing checkpoint of a turn that failed says why; outside Git every checkpoint
      // fails, and the project's own state says so first.
      final failure = pair.to.failure ?? pair.from.failure;
      changesProblem.value = switch (changes.state) {
        BridgeChangesState.removed when failure != null => _noCheckpoint(
          failure,
        ),
        BridgeChangesState.shown => null,
        BridgeChangesState.outsideGit => _outsideGit,
        BridgeChangesState.removed => (
          title: 'Checkpoints removed',
          text:
              'This session’s checkpoints were removed, so these turns show no '
              'changes.',
        ),
      };
      changed.value = changes.files;
    } on Object catch (error) {
      if (generation != _generation) return;
      final problem = 'Could not show the changes: $error';
      reportRead('changes', problem);
      changesProblem.value = (title: 'No changes shown', text: problem);
      changed.value = null;
    }
  }

  /// Reads the turns that ended and adds what each changed where it ended in the chat;
  /// `ended` turns had ended live when it was asked.
  Future<void> _endTurn(int ended) async {
    final generation = _generation;
    final asked = _turnReadsStarted;
    unawaited(git.refresh());
    await readChanges();
    if (generation != _generation) return;
    _chat.endTurns(turns.value, _turnsRead > asked ? ended : 0);
  }

  /// How many turn reads began, and the last of them that succeeded: a turn's end trusts
  /// only the turns of a read that began after it.
  int _turnReadsStarted = 0;
  int _turnsRead = 0;

  static const _outsideGit = (
    title: 'Not a Git repository',
    text:
        'This project has no Git repository, so Liquid keeps no checkpoints '
        'and shows no turn changes.',
  );

  static ({String title, String text}) _noCheckpoint(String failure) => (
    title: 'No checkpoint',
    text: 'Liquid could not capture the project: $failure',
  );

  /// One file's unified diff for the shown Diff scope.
  Future<String> patch(String path) {
    if (shownScope.value is UncommittedChanges) {
      return _backend.uncommittedPatch(
        path,
        hideWhitespace: hideWhitespace.value,
      );
    }
    if (shownScope.value is StashedChanges) {
      final id = shownStash.value?.stash.id;
      if (id == null) return Future.value('');
      return _backend.stashPatch(
        id,
        path,
        hideWhitespace: hideWhitespace.value,
      );
    }
    final pair = shown.value;
    if (pair == null) return Future.value('');
    return _backend.turnPatch(
      pair.from.id,
      pair.to.id,
      path,
      hideWhitespace: hideWhitespace.value,
    );
  }

  /// The branch tree and the Trajectory of each branch ending at an entry in `leaves`, or of
  /// the current branch.
  Future<BridgeHistory> history([List<String> leaves = const []]) =>
      _backend.history(leaves);

  /// Everything the model saw for the request behind the response `entry`.
  Future<BridgeContextDetail?> context(String entry) => _backend.context(entry);

  /// Continues the session from `entry`; the next prompt starts a new branch there. The chat
  /// then shows the branch the session continues. Returns whether this call branched: one asked
  /// for while another runs waits for that one instead, and one asked for while the session
  /// changes is dropped, as its entry may belong to the session being replaced.
  Future<bool> branch(String entry) {
    if (_branching case final running?) {
      return running.then((_) => false, onError: (Object _) => false);
    }
    if (_switching != null) {
      notify(
        BridgeLevel.info,
        'The session is changing, so the branch was not made.',
      );
      return Future.value(false);
    }
    return _branching = _branch(entry).whenComplete(() => _branching = null);
  }

  Future<bool>? _branching;

  /// The sessions with checkpoints in the project's repository.
  Future<List<BridgeCheckpointSession>> checkpointSessions() =>
      _backend.checkpointSessions();

  /// Removes every checkpoint of the project; returns whether it did, and says why not.
  Future<bool> removeAllCheckpoints() async {
    try {
      await _backend.removeAllCheckpoints();
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not remove the checkpoints: $error');
      return false;
    }
    await readChanges();
    return true;
  }

  /// The file pi stores the open session in, unless it keeps it in memory.
  Future<String?> sessionFile() => _backend.sessionFile();

  /// The turns with checkpoints of the session stored at `path`.
  Future<int> storedTurns(String path) => _backend.storedTurns(path);

  /// Deletes the session stored at `path` with its checkpoints.
  /// The open session's agent stops and a new session takes its place first, as T3 Code moves
  /// on after deleting the active thread. Returns whether it deleted, and says why not.
  Future<bool> deleteSession(String path) async {
    try {
      if (path == await _backend.sessionFile()) {
        if (status.value case RunStatus.running || RunStatus.waitingForYou) {
          await stop();
          await _settled();
        }
        await newSession();
        if (path == await _backend.sessionFile()) {
          notify(
            BridgeLevel.info,
            'The session stayed open, so it was not deleted.',
          );
          return false;
        }
      }
      await _backend.deleteSession(path);
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not delete the session: $error');
      return false;
    }
    await loadStart();
    return true;
  }

  /// Writes the record of the session stored at `path` into Downloads and returns its folder;
  /// returns none and says why when nothing was written.
  Future<String?> exportRecord({
    required String path,
    required String title,
    required String host,
    required String day,
  }) async {
    try {
      final folder = await _backend.exportRecord(
        path: path,
        title: title,
        host: host,
        day: day,
      );
      notify(BridgeLevel.info, 'Exported the record of “$title” to Downloads.');
      return folder;
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Nothing was exported: $error');
      return null;
    }
  }

  /// Waits, up to a few seconds, until the stopped agent no longer runs.
  Future<void> _settled() async {
    bool busy() => switch (status.value) {
      RunStatus.running || RunStatus.waitingForYou => true,
      _ => false,
    };
    if (!busy()) return;
    final settled = Completer<void>();
    void check() {
      if (!busy() && !settled.isCompleted) settled.complete();
    }

    status.addListener(check);
    try {
      await settled.future.timeout(
        const Duration(seconds: 5),
        onTimeout: () {},
      );
    } finally {
      status.removeListener(check);
    }
  }

  /// What reverting the files to before `turn` would change.
  Future<BridgeRevertPlan> revertPlan(BridgeTurn turn) =>
      _backend.revertPlan(turn.id);

  /// Reverts the files to before `turn` and continues the session on a new branch from before
  /// its prompt, which returns to the input area (ADR-0234). Only the plan `confirmation` came
  /// with runs (ADR-0256). Returns whether it reverted; a failure is reported.
  Future<bool> revert(BridgeTurn turn, BridgeRevertConfirmation confirmation) {
    if (_branching != null || _switching != null) {
      notify(
        BridgeLevel.info,
        'The session is changing, so the files were not reverted.',
      );
      return Future.value(false);
    }
    return _branching = _revert(
      turn,
      confirmation,
    ).whenComplete(() => _branching = null);
  }

  /// Reverts the files and shows the branch; messages wait until the chat has been restored.
  Future<bool> _revert(
    BridgeTurn turn,
    BridgeRevertConfirmation confirmation,
  ) async {
    try {
      await _backend.revertToTurn(turn.id, confirmation);
    } on Object catch (error) {
      // The error says whether the files changed; the Diff tab shows them either way.
      final said = '$error';
      notify(
        BridgeLevel.error,
        'Could not revert and branch. '
        '${said.isEmpty ? '' : '${said[0].toUpperCase()}${said.substring(1)}'}',
      );
      await readChanges();
      return false;
    }
    await _branched();
    await readChanges();
    return true;
  }

  Future<bool> _branch(String entry) async {
    await _backend.branch(entry);
    await _branched();
    return true;
  }

  /// Shows the branch the session now continues.
  Future<void> _branched() async {
    stored.value++;
    await loadChat();
  }

  /// Rebuilds the chat from the session's current branch, as the model sees it, and marks
  /// where it leaves the other branches. Items that arrive meanwhile stay after it.
  Future<void> loadChat() => _chat.load();

  /// Reads the stored entries before the earliest one the chat covers.
  Future<void> loadEarlier() => _chat.loadEarlier();

  /// Says why the agent stopped, and keeps all of it among the notifications, where it can be
  /// read in full.
  void _failed(String? text) {
    failure.value = text;
    if (text != null && text.isNotEmpty) notify(BridgeLevel.error, text);
  }

  /// Shows a notification in the status bar and keeps it in the list.
  void notify(BridgeLevel level, String text) {
    final shown = Notice(level, text, DateTime.now());
    notices.value = [shown, ...notices.value].take(_keptNotices).toList();
    // An error stays in the bar until it is dismissed; anything less only joins the list.
    if (notice.value?.level == BridgeLevel.error &&
        level != BridgeLevel.error) {
      return;
    }
    notice.value = shown;
    _fade?.cancel();
    if (level != BridgeLevel.error) {
      _fade = Timer(_noticeFades, () {
        if (notice.value == shown) notice.value = null;
      });
    }
  }

  /// The lasting read failures already reported, by what could not be read.
  final Map<String, String> _failing = {};

  /// Reports that reading [what] failed with [problem], or works again when it is null. A
  /// failure is reported once until it changes or the read works, and an exited agent's
  /// failures add nothing to what its exit said.
  void reportRead(String what, String? problem) {
    if (problem == null) {
      _failing.remove(what);
      return;
    }
    if (_failing[what] == problem) return;
    _failing[what] = problem;
    if (status.value case RunStatus.exited || RunStatus.failed) return;
    notify(BridgeLevel.error, problem);
  }

  /// Takes the notification out of the status bar; the list keeps it.
  void dismissNotice() {
    _fade?.cancel();
    notice.value = null;
  }

  /// Empties the list and the status bar.
  void clearNotices() {
    dismissNotice();
    notices.value = const [];
  }

  /// Releases the event stream and stops the agent.
  Future<void> dispose() async {
    _fade?.cancel();
    git.stop();
    _pulls?.dispose();
    // The bridge's stream finishes cancelling only with its next event, which an idle agent
    // sends once it is shut down, so the agent stops first.
    final cancelled = _subscription.cancel();
    await _backend.shutdown();
    await cancelled;
  }

  void _apply(BridgeEvent event) {
    switch (event.kind) {
      case BridgeEventKind.turnStarted:
        _turnActive = true;
        _stopping = false;
      case BridgeEventKind.turnCompleted || BridgeEventKind.turnAborted:
        _turnActive = false;
        _stopping = false;
        stored.value++;
        unawaited(loadTitle());
        unawaited(_endTurn(_chat.turnCompleted()));
      case BridgeEventKind.itemStarted:
        final started = event.item!;
        final view = ItemView(
          id: started.id,
          kind: started.kind ?? BridgeItemKind.dynamicToolCall,
          tool: started.tool,
          target: started.target,
        );
        _byId[view.id] = view;
        items.value = [...items.value, view];
      case BridgeEventKind.contentDelta:
        final view = _byId[event.item!.id];
        if (view != null) {
          view.text.value += event.text ?? '';
        }
      case BridgeEventKind.itemUpdated:
        _byId[event.item!.id]?.text.value = event.text ?? '';
      case BridgeEventKind.itemCompleted:
        stored.value++;
        final view = _byId[event.item!.id];
        if (view != null) {
          view
            ..outcome.value = (
              failed: event.item!.failed,
              durationMs: event.item!.durationMs,
              lines: changedLines(event.item!.added, event.item!.removed),
            )
            ..text.value = event.text ?? view.text.value;
        }
      case BridgeEventKind.modelRequestCompleted:
        totals.value = totals.value.add(event.step!.usage);
        // A request Stop ended reports an abort, some providers as an error.
        if (event.step!.error case final error?
            when event.step!.stopReason != 'aborted' && !_stopping) {
          notify(BridgeLevel.error, 'The model request failed: $error');
        }
      case BridgeEventKind.userInputRequested:
        if (!_endedUnseen.remove(event.request!.id)) {
          _questions.add(event.request!);
        }
      case BridgeEventKind.userInputResolved:
        _end(event.request!.id);
      case BridgeEventKind.widgetChanged:
        final lines = event.display!.lines;
        final next = {...widgets.value};
        if (lines == null) {
          next.remove(event.display!.key);
        } else {
          next[event.display!.key] = lines;
        }
        widgets.value = next;
      case BridgeEventKind.statusChanged:
        final text = event.text;
        final next = {...statuses.value};
        text == null
            ? next.remove(event.display!.key)
            : next[event.display!.key] = text;
        statuses.value = next;
      case BridgeEventKind.inputTextSet:
        draft.value = event.text;
      case BridgeEventKind.queueChanged:
        queue.value =
            event.queue ?? const BridgeQueue(steering: [], followUp: []);
      case BridgeEventKind.notification:
        notify(event.display?.level ?? BridgeLevel.info, event.text ?? '');
      case BridgeEventKind.failed:
        _failed(event.text);
      case BridgeEventKind.exited:
        // Nothing an exited agent started is still running.
        for (final view in _byId.values) {
          if (view.outcome.value == null &&
              view.kind != BridgeItemKind.userMessage) {
            view.outcome.value = (failed: true, durationMs: null, lines: null);
          }
        }
        // A failed agent's error output says why, such as an extension that did not load.
        if (event.success != true && failure.value == null) {
          _failed(event.text);
        }
        // A gone agent asks nothing more; its open cards cannot be answered.
        _questions.clear();
        question.value = null;
        status.value = event.success == true && failure.value == null
            ? RunStatus.exited
            : RunStatus.failed;
        return;
    }
    question.value = _questions.firstOrNull;
    status.value = _questions.isNotEmpty
        ? RunStatus.waitingForYou
        : _turnActive
        ? RunStatus.running
        : RunStatus.idle;
  }
}

/// A message's first line, as long as a stored session's title can be; the core names stored
/// sessions by the same rule (`TITLE_CHARS` in meta-harness-core's `stored.rs`).
String _firstLine(String message) {
  final line = message.trim().split('\n').first.trim();
  return String.fromCharCodes(line.runes.take(120));
}
