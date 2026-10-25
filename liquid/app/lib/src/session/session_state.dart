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
    this.arguments,
    this.entry,
    this.marker,
  });

  final int id;
  final BridgeItemKind kind;
  final String? tool;
  final String? arguments;

  /// The stored entry a rebuilt item comes from.
  final String? entry;

  /// Set on the row that marks where the current branch leaves the others.
  final BranchMarker? marker;
  final ValueNotifier<String> text = ValueNotifier('');

  /// How the item ended, once it has; observable apart from its text.
  final ValueNotifier<ItemOutcome?> outcome = ValueNotifier(null);
}

/// How a finished item ended.
typedef ItemOutcome = ({bool failed, int? durationMs});

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

/// Tokens and cost the session spent so far.
@immutable
class SessionTotals {
  const SessionTotals({this.input = 0, this.output = 0, this.costMicroUsd = 0});

  final int input;
  final int output;
  final int costMicroUsd;

  SessionTotals add(BridgeUsage usage) => SessionTotals(
    input: input + usage.input,
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

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);
  final approval = ValueNotifier<BridgeRequest?>(null);
  final question = ValueNotifier<BridgeRequest?>(null);
  final widgets = ValueNotifier<Map<String, List<String>>>(const {});
  final statuses = ValueNotifier<Map<String, String>>(const {});
  final title = ValueNotifier<String?>(null);
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

  /// Rebuilt items count down from -1, so they never meet live item ids.
  int _nextStoredId = -1;

  /// Counts the changes of the session's stored history; the Trajectory reloads on each.
  final stored = ValueNotifier(0);

  /// Sends a message: an idle agent starts with it, a running one takes it before its next
  /// model request or, with `followUp`, once it is done. pi decides, so no turn state races
  /// the send. A message the agent rejects goes back to the input area.
  Future<void> send(String message, {bool followUp = false}) async {
    try {
      await _backend.send(message, followUp: followUp);
    } on Object catch (error) {
      draft.value = message;
      notify(BridgeLevel.error, 'The message was not sent: $error');
    }
  }

  /// Takes one waiting message out of the queue; pi clears the whole queue, so the others are
  /// sent again in their order.
  Future<void> unqueue(String message) async {
    final waiting = await _backend.clearQueue();
    var removed = false;
    bool keep(String text) {
      if (!removed && text == message) {
        removed = true;
        return false;
      }
      return true;
    }

    for (final text in waiting.steering.where(keep).toList()) {
      await _backend.send(text, followUp: false);
    }
    for (final text in waiting.followUp.where(keep).toList()) {
      await _backend.send(text, followUp: true);
    }
  }

  /// Stops the run and hands the waiting messages back to the input area for editing.
  Future<void> stop() async {
    final waiting = await _backend.clearQueue();
    await _backend.abort();
    final texts = [...waiting.steering, ...waiting.followUp];
    if (texts.isNotEmpty) draft.value = texts.join('\n\n');
  }

  Future<void> abort() => _backend.abort();

  /// Changes which tool calls wait for the user.
  Future<void> setMode(BridgeApprovalMode next) async {
    await _backend.setApprovalMode(next);
    mode.value = next;
  }

  /// Answers the approval request in front.
  Future<void> decide(BridgeDecision decision) async {
    final request = approval.value;
    if (request != null) {
      await _backend.decide(request.id, decision);
    }
  }

  /// Answers the question in front with text or a selected option.
  Future<void> answerText(String text) =>
      _answer((id) => _backend.answerText(id, text));

  Future<void> answerConfirm(bool confirmed) =>
      _answer((id) => _backend.answerConfirm(id, confirmed));

  Future<void> dismiss() => _answer(_backend.dismiss);

  Future<void> _answer(Future<void> Function(String id) reply) async {
    final request = question.value;
    if (request != null) {
      await reply(request.id);
    }
  }

  /// Files the latest run changed.
  Future<List<BridgeFileChange>> changes() => _backend.changes();

  /// One file's unified diff since the latest run started.
  Future<String> patch(String path) => _backend.patch(path);

  /// The branch tree and the Trajectory of each branch ending at an entry in `leaves`, or of
  /// the current branch.
  Future<BridgeHistory> history([List<String> leaves = const []]) =>
      _backend.history(leaves);

  /// Everything the model saw for the request behind the response `entry`.
  Future<BridgeContextDetail?> context(String entry) => _backend.context(entry);

  /// Continues the session from `entry`; the next prompt starts a new branch there. The chat
  /// then shows the branch the session continues.
  Future<void> branch(String entry) async {
    await _backend.branch(entry);
    stored.value++;
    await loadChat();
  }

  /// Rebuilds the chat from the session's current branch, as the model sees it, and marks
  /// where it leaves the other branches. Items that arrive meanwhile stay after it.
  Future<void> loadChat() async {
    final before = items.value;
    final page = await _backend.transcript(null);
    final history = await _backend.history(const []);
    final marker = branchMarker(history.tree, history.leaf);
    final rebuilt = _stored(page.items);
    if (marker != null) {
      final path = _pathIndex(history.tree, history.leaf);
      final forkIndex = path[marker.fork] ?? -1;
      final after = rebuilt.indexWhere(
        (view) => (path[view.entry] ?? -1) > forkIndex,
      );
      rebuilt.insert(
        after < 0 ? rebuilt.length : after,
        ItemView(
          id: _nextStoredId--,
          kind: BridgeItemKind.dynamicToolCall,
          marker: marker,
        ),
      );
    }
    final arrived = items.value.where((view) => !before.contains(view));
    items.value = [...rebuilt, ...arrived];
    _chatStart = page.start;
    chatMore.value = page.more;
    if (page.items.isEmpty) await loadEarlier();
  }

  /// Reads the stored entries before the earliest one the chat covers, passing over pages the
  /// chat shows nothing of, such as tool results only.
  Future<void> loadEarlier() async {
    if (_readingEarlier) return;
    _readingEarlier = true;
    try {
      while (chatMore.value && _chatStart != null) {
        final before = _chatStart;
        final page = await _backend.transcript(before);
        _chatStart = page.start;
        // A page that does not move back ends the reading, so it cannot repeat forever.
        chatMore.value = page.more && page.start != before;
        if (page.items.isNotEmpty) {
          items.value = [..._stored(page.items), ...items.value];
          return;
        }
      }
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
          arguments: item.arguments,
          entry: item.entry,
        )
        ..text.value = item.text
        ..outcome.value = (failed: item.failed, durationMs: item.durationMs),
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

  /// Releases the event stream and stops the agent.
  /// Shows a notification in the status bar and keeps it in the list.
  void notify(BridgeLevel level, String text) {
    final shown = Notice(level, text, DateTime.now());
    notices.value = [shown, ...notices.value].take(_keptNotices).toList();
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

  Future<void> dispose() async {
    _fade?.cancel();
    await _subscription.cancel();
    await _backend.shutdown();
  }

  void _apply(BridgeEvent event) {
    switch (event.kind) {
      case BridgeEventKind.turnStarted:
        _turnActive = true;
      case BridgeEventKind.turnCompleted || BridgeEventKind.turnAborted:
        _turnActive = false;
        stored.value++;
      case BridgeEventKind.itemStarted:
        final started = event.item!;
        final view = ItemView(
          id: started.id,
          kind: started.kind ?? BridgeItemKind.dynamicToolCall,
          tool: started.tool,
          arguments: started.arguments,
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
            )
            ..text.value = event.text ?? view.text.value;
        }
      case BridgeEventKind.modelRequestCompleted:
        totals.value = totals.value.add(event.step!.usage);
        if (event.step!.error case final error?) {
          notify(BridgeLevel.error, 'The model request failed: $error');
        }
      case BridgeEventKind.requestOpened:
        _approvals.add(event.request!);
      case BridgeEventKind.requestResolved:
        _approvals.removeWhere((request) => request.id == event.request!.id);
        stored.value++;
      case BridgeEventKind.userInputRequested:
        _questions.add(event.request!);
      case BridgeEventKind.userInputResolved:
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
        status.value = event.success == true && failure.value == null
            ? RunStatus.exited
            : RunStatus.failed;
        return;
      case BridgeEventKind.contextChanged ||
          BridgeEventKind.modelRequestStarted:
        break;
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
