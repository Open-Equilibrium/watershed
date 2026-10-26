import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';
import '../trajectory/trajectory_model.dart';
import 'backend.dart';

/// What the session is doing, as the status bar and sidebar show it.
enum RunStatus { idle, running, waitingForYou, exited, failed }

/// One entry of the chat timeline. Its text grows in place while it streams, so only the widget
/// that shows this item rebuilds (client architecture rule 6).
class ItemView {
  ItemView({
    required this.id,
    required this.kind,
    this.tool,
    this.target,
    this.entry,
    this.marker,
  });

  final int id;
  final BridgeItemKind kind;
  final String? tool;

  /// What a tool call works on, such as a command or a path, on one line.
  final String? target;

  /// The stored entry a rebuilt item comes from.
  final String? entry;

  /// Set on the row that marks where the current branch leaves the others.
  final BranchMarker? marker;
  final ValueNotifier<String> text = ValueNotifier('');

  /// How the item ended, once it has; observable apart from its text.
  final ValueNotifier<ItemOutcome?> outcome = ValueNotifier(null);
}

/// How a finished item ended, and the lines a file change added and removed.
typedef ItemOutcome = ({
  bool failed,
  int? durationMs,
  ({int added, int removed})? lines,
});

({int added, int removed})? _lines(int? added, int? removed) =>
    added == null || removed == null ? null : (added: added, removed: removed);

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

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;

/// Tokens and cost the session spent so far.
@immutable
class SessionTotals {
  const SessionTotals({this.input = 0, this.output = 0, this.costMicroUsd = 0});

  final int input;
  final int output;
  final int costMicroUsd;

  SessionTotals add(BridgeUsage usage) => SessionTotals(
    input: input + promptTokens(usage),
    output: output + usage.output,
    costMicroUsd: costMicroUsd + usage.costMicroUsd,
  );
}

/// One agent session's state for the UI. It owns the event subscription and the backend, and
/// releases both in [dispose] (client architecture rule 14).
class SessionState {
  SessionState(this._backend) {
    _subscription = _backend.events().listen(_apply);
  }

  final SessionBackend _backend;
  late final StreamSubscription<BridgeEvent> _subscription;
  final Map<int, ItemView> _byId = {};
  final List<BridgeRequest> _approvals = [];
  final List<BridgeRequest> _questions = [];
  bool _turnActive = false;

  /// Whether the user stopped the running turn; requests it ends are no failures.
  bool _stopping = false;

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);
  final approval = ValueNotifier<BridgeRequest?>(null);
  final question = ValueNotifier<BridgeRequest?>(null);
  final widgets = ValueNotifier<Map<String, List<String>>>(const {});
  final statuses = ValueNotifier<Map<String, String>>(const {});
  final title = ValueNotifier<String?>(null);

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
  final mode = ValueNotifier(BridgeApprovalMode.ask);

  /// Messages waiting for the running agent.
  final queue = ValueNotifier(const BridgeQueue(steering: [], followUp: []));

  /// Whether earlier stored entries of the chat remain to be read.
  final chatMore = ValueNotifier(false);

  /// The earliest stored entry the chat covers; the next page ends before it.
  String? _chatStart;
  bool _readingEarlier = false;

  /// Why earlier messages could not be read, so a lasting failure is reported once.
  String? _unreadEarlier;

  /// Rebuilt items count down from -1, so they never meet live item ids.
  int _nextStoredId = -1;

  /// Counts the changes of the session's stored history; the Trajectory reloads on each.
  final stored = ValueNotifier(0);

  /// What the start page offers: the project's recent sessions, the models and the selected
  /// one as `provider/id`.
  final recent = ValueNotifier<List<BridgeStoredSession>>(const []);
  final models = ValueNotifier<List<BridgeModel>>(const []);
  final model = ValueNotifier<String?>(null);

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

  /// Starts a new session in the same agent; the current one stays available (ADR-0203).
  Future<void> newSession() => _switch(
    _backend.newSession,
    busy: 'Stop the agent before starting a new session.',
  );

  /// Continues a stored session in the same agent (ADR-0203).
  Future<void> continueSession(String path) => _switch(
    () => _backend.switchSession(path),
    busy: 'Stop the agent before continuing another session.',
  );

  /// A session switch in progress; messages wait for it and other switches are ignored.
  Completer<void>? _switching;

  /// Counts session switches; a chat read started before the latest one is dropped.
  int _generation = 0;

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
    final (shownWidgets, shownStatuses, shownTitle) = (
      widgets.value,
      statuses.value,
      title.value,
    );
    widgets.value = const {};
    statuses.value = const {};
    title.value = null;
    try {
      if (!await _replace(switchTo)) {
        widgets.value = shownWidgets;
        statuses.value = shownStatuses;
        title.value = shownTitle;
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
    sessionTitle.value = null;
    // The previous session's run no longer applies.
    changed.value = const [];
    changesProblem.value = null;
    _byId.clear();
    items.value = const [];
    chatMore.value = false;
    _chatStart = null;
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
    // chat is restored follows it, so the restored chat does not hold its items twice.
    await _switching?.future;
    try {
      await _chatLoad;
    } on Object {
      // The chat reports its own failure; the message still goes out.
    }
    try {
      await _backend.send(message, followUp: followUp);
      // Named like a stored session until pi stores this one.
      sessionTitle.value ??= _firstLine(message);
    } on Object catch (error) {
      draft.value = message;
      notify(BridgeLevel.error, 'The message was not sent: $error');
    }
  }

  /// Queue changes run one after another: each clears pi's whole queue, and a removal sends the
  /// others again, which a second removal or Stop must not overtake.
  Future<void> _queueChange = Future.value();

  Future<void> _changeQueue(Future<void> Function() change) {
    final next = _queueChange.then((_) => change());
    _queueChange = next.then((_) {}, onError: (Object _) {});
    return next;
  }

  /// Takes one waiting message out of the queue; pi clears the whole queue, so the others are
  /// sent again in their order.
  Future<void> unqueue(String message, {required bool followUp}) =>
      _changeQueue(() => _unqueue(message, followUp: followUp));

  Future<void> _unqueue(String message, {required bool followUp}) async {
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
      if (!removed && inFollowUp == followUp) removed = rest.remove(message);
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
      notify(BridgeLevel.error, 'Could not stop the agent: $error');
    }
  }

  /// Changes which tool calls wait for the user.
  Future<void> setMode(BridgeApprovalMode next) async {
    await _backend.setApprovalMode(next);
    mode.value = next;
  }

  /// Answers the approval `request`.
  Future<void> decide(String request, BridgeDecision decision) =>
      _reply(request, () => _backend.decide(request, decision));

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
    final open = [..._approvals, ..._questions].any((it) => it.id == request);
    if (!open || !_answered.add(request)) return;
    try {
      await send();
    } on Object catch (error) {
      _answered.remove(request);
      notify(BridgeLevel.error, 'The answer did not reach the agent: $error');
    }
  }

  /// The files the latest run changed, as last read; null before the first read. The Diff tab
  /// and its count show them.
  final changed = ValueNotifier<List<BridgeFileChange>?>(null);

  /// Why the latest read of the changes failed; null after one that worked.
  final changesProblem = ValueNotifier<String?>(null);

  /// Reads what the latest run changed. A failed read is reported once, and the Diff tab says
  /// so instead of showing an earlier run's changes.
  Future<void> readChanges() async {
    final generation = _generation;
    try {
      final read = await _backend.changes();
      if (generation != _generation) return;
      changesProblem.value = read == null
          ? 'This project is not in Git, so Liquid cannot show what a run changed.'
          : null;
      changed.value = read;
    } on Object catch (error) {
      if (generation != _generation) return;
      final problem = 'Could not show the changes: $error';
      if (problem != changesProblem.value) notify(BridgeLevel.error, problem);
      changesProblem.value = problem;
      changed.value = null;
    }
  }

  /// One file's unified diff since the latest run started.
  Future<String> patch(String path) => _backend.patch(path);

  /// The branch tree and the Trajectory of each branch ending at an entry in `leaves`, or of
  /// the current branch.
  Future<BridgeHistory> history([List<String> leaves = const []]) =>
      _backend.history(leaves);

  /// Everything the model saw for the request behind the response `entry`.
  Future<BridgeContextDetail?> context(String entry) => _backend.context(entry);

  /// Continues the session from `entry`; the next prompt starts a new branch there. The chat
  /// then shows the branch the session continues. A branch asked for while one runs waits for
  /// that one instead of branching again.
  Future<void> branch(String entry) =>
      _branching ??= _branch(entry).whenComplete(() => _branching = null);

  Future<void>? _branching;

  Future<void> _branch(String entry) async {
    await _backend.branch(entry);
    stored.value++;
    await loadChat();
  }

  /// Rebuilds the chat from the session's current branch, as the model sees it, and marks
  /// where it leaves the other branches. Items that arrive meanwhile stay after it.
  Future<void> loadChat() {
    final load = _loadChat();
    _chatLoad = load;
    return load.whenComplete(() {
      if (identical(_chatLoad, load)) _chatLoad = null;
    });
  }

  /// A chat restore in progress; messages wait for it.
  Future<void>? _chatLoad;

  Future<void> _loadChat() async {
    final generation = _generation;
    final version = ++_chatVersion;
    final before = items.value;
    final BridgeTranscript page;
    final BridgeHistory history;
    try {
      page = await _backend.transcript(null);
      history = await _backend.history(const []);
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not restore the chat: $error');
      return;
    }
    if (generation != _generation || version != _chatVersion) return;
    // What the stored responses used; live requests add to it.
    totals.value = const SessionTotals().add(history.usage);
    final rebuilt = _stored(page.items);
    _pendingMark = null;
    if (branchMarker(history.tree, history.leaf) case final marker?) {
      final path = _pathIndex(history.tree, history.leaf);
      final mark = (marker: marker, path: path, fork: path[marker.fork] ?? -1);
      if (!_placeMark(rebuilt, mark, complete: !page.more)) _pendingMark = mark;
    }
    // Only live items arrived meanwhile; items another restore rebuilt are replaced.
    final arrived = items.value.where(
      (view) =>
          !before.contains(view) && view.entry == null && view.marker == null,
    );
    items.value = [...rebuilt, ...arrived];
    _chatStart = page.start;
    chatMore.value = page.more;
    if (page.items.isEmpty) await loadEarlier();
  }

  /// The branch mark until the chat shows where the branch leaves the others.
  _Mark? _pendingMark;

  /// Counts chat restores; an earlier page read for a replaced chat is dropped.
  int _chatVersion = 0;

  /// Puts the mark before the first of `views` past the fork, once they show the fork itself
  /// or nothing earlier remains; whether it did.
  bool _placeMark(List<ItemView> views, _Mark mark, {required bool complete}) {
    int position(ItemView view) => mark.path[view.entry] ?? -1;
    final reachesFork = views.any(
      (view) => view.entry != null && position(view) <= mark.fork,
    );
    if (!reachesFork && !complete) return false;
    final after = views.indexWhere((view) => position(view) > mark.fork);
    views.insert(
      after < 0 ? views.length : after,
      ItemView(
        id: _nextStoredId--,
        kind: BridgeItemKind.dynamicToolCall,
        marker: mark.marker,
      ),
    );
    return true;
  }

  /// Reads the stored entries before the earliest one the chat covers, passing over pages the
  /// chat shows nothing of, such as tool results only.
  Future<void> loadEarlier() async {
    if (_readingEarlier) return;
    _readingEarlier = true;
    final (generation, version) = (_generation, _chatVersion);
    try {
      while (chatMore.value && _chatStart != null) {
        final before = _chatStart;
        final page = await _backend.transcript(before);
        _unreadEarlier = null;
        if (generation != _generation || version != _chatVersion) return;
        _chatStart = page.start;
        // A page that does not move back ends the reading, so it cannot repeat forever.
        chatMore.value = page.more && page.start != before;
        if (page.items.isNotEmpty) {
          final shown = [..._stored(page.items), ...items.value];
          if (_pendingMark case final mark?
              when _placeMark(shown, mark, complete: !chatMore.value)) {
            _pendingMark = null;
          }
          items.value = shown;
          return;
        }
      }
      // The oldest pages showed nothing, so the branch leaves before everything shown.
      if (_pendingMark case final mark? when !chatMore.value) {
        final shown = [...items.value];
        _placeMark(shown, mark, complete: true);
        _pendingMark = null;
        items.value = shown;
      }
    } on Object catch (error) {
      if ('$error' != _unreadEarlier) {
        notify(BridgeLevel.error, 'Could not read earlier messages: $error');
      }
      _unreadEarlier = '$error';
    } finally {
      _readingEarlier = false;
    }
  }

  List<ItemView> _stored(List<BridgeTranscriptItem> items) => [
    for (final item in items)
      ItemView(
          id: _nextStoredId--,
          kind: item.kind,
          tool: item.tool,
          target: item.target,
          entry: item.entry,
        )
        ..text.value = item.text
        ..outcome.value = (
          failed: item.failed,
          durationMs: item.durationMs,
          lines: _lines(item.added, item.removed),
        ),
  ];

  /// Each entry's position on the path from the root to `leaf`.
  static Map<String, int> _pathIndex(List<BridgeTreeNode> tree, String? leaf) {
    final parents = {for (final node in tree) node.id: node.parent};
    final path = <String>[];
    for (String? id = leaf; id != null; id = parents[id]) {
      path.add(id);
    }
    return {for (final (index, id) in path.reversed.indexed) id: index};
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
        unawaited(readChanges());
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
              lines: _lines(event.item!.added, event.item!.removed),
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
      case BridgeEventKind.requestOpened:
        _approvals.add(event.request!);
      case BridgeEventKind.requestResolved:
        _answered.remove(event.request!.id);
        _approvals.removeWhere((request) => request.id == event.request!.id);
        stored.value++;
      case BridgeEventKind.userInputRequested:
        _questions.add(event.request!);
      case BridgeEventKind.userInputResolved:
        _answered.remove(event.request!.id);
        _questions.removeWhere((request) => request.id == event.request!.id);
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
      case BridgeEventKind.titleChanged:
        title.value = event.text;
      case BridgeEventKind.inputTextSet:
        draft.value = event.text;
      case BridgeEventKind.queueChanged:
        queue.value =
            event.queue ?? const BridgeQueue(steering: [], followUp: []);
      case BridgeEventKind.notification:
        notify(event.display?.level ?? BridgeLevel.info, event.text ?? '');
      case BridgeEventKind.failed:
        failure.value = event.text;
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
          failure.value = event.text;
        }
        // A gone agent asks nothing more; its open cards cannot be answered.
        _approvals.clear();
        _questions.clear();
        approval.value = null;
        question.value = null;
        status.value = event.success == true && failure.value == null
            ? RunStatus.exited
            : RunStatus.failed;
        return;
    }
    approval.value = _approvals.firstOrNull;
    question.value = _questions.firstOrNull;
    status.value = _approvals.isNotEmpty || _questions.isNotEmpty
        ? RunStatus.waitingForYou
        : _turnActive
        ? RunStatus.running
        : RunStatus.idle;
  }
}

/// A message's first line, as long as a stored session's title can be.
String _firstLine(String message) {
  final line = message.trim().split('\n').first.trim();
  return String.fromCharCodes(line.runes.take(120));
}

/// Where a branch leaves the others: its mark, each entry's position on the path to the leaf,
/// and the fork's position.
typedef _Mark = ({BranchMarker marker, Map<String, int> path, int fork});
