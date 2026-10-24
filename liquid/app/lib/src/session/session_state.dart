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
  bool completed = false;
  bool failed = false;
  int? durationMs;
}

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
  final notification = ValueNotifier<(BridgeLevel, String)?>(null);
  final totals = ValueNotifier(const SessionTotals());
  final failure = ValueNotifier<String?>(null);
  final mode = ValueNotifier(BridgeApprovalMode.ask);

  /// Whether earlier stored entries of the chat remain to be read.
  final chatMore = ValueNotifier(false);
  String? _firstEntry;
  bool _readingEarlier = false;

  /// Rebuilt items count down from -1, so they never meet live item ids.
  int _nextStoredId = -1;

  /// Counts the changes of the session's stored history; the Trajectory reloads on each.
  final stored = ValueNotifier(0);

  /// Sends a prompt, or steers the running turn.
  Future<void> send(String message) =>
      _turnActive ? _backend.steer(message) : _backend.prompt(message);

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
    _firstEntry = page.items.firstOrNull?.entry;
    chatMore.value = page.more;
  }

  /// Reads the stored entries before the oldest one the chat shows.
  Future<void> loadEarlier() async {
    final first = _firstEntry;
    if (!chatMore.value || _readingEarlier || first == null) return;
    _readingEarlier = true;
    try {
      final page = await _backend.transcript(first);
      items.value = [..._stored(page.items), ...items.value];
      _firstEntry = page.items.firstOrNull?.entry ?? first;
      chatMore.value = page.more;
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
        ..completed = true
        ..failed = item.failed
        ..durationMs = item.durationMs,
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
  Future<void> dispose() async {
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
            ..completed = true
            ..failed = event.item!.failed
            ..durationMs = event.item!.durationMs
            ..text.value = event.text ?? view.text.value;
        }
      case BridgeEventKind.modelRequestCompleted:
        totals.value = totals.value.add(event.step!.usage);
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
      case BridgeEventKind.notification:
        notification.value = (
          event.display?.level ?? BridgeLevel.info,
          event.text ?? '',
        );
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
