import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';
import '../rust/api/session.dart';
import 'backend.dart';

/// What the session is doing, as the status bar and sidebar show it.
enum RunStatus { idle, running, waitingForYou, exited, failed }

/// One entry of the chat timeline. Its text grows in place while it streams, so only the widget
/// that shows this item rebuilds (client architecture rule 6).
class ItemView {
  ItemView({required this.id, required this.kind, this.tool, this.arguments});

  final int id;
  final BridgeItemKind kind;
  final String? tool;
  final String? arguments;
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
