import 'dart:async';

import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/backend.dart';

/// Records commands and lets tests emit events.
class FakeBackend implements SessionBackend {
  final controller = StreamController<BridgeEvent>();
  final calls = <String>[];
  List<BridgeFileChange> changed = const [];
  Map<String, String> patches = const {};
  bool cancelled = false;

  @override
  Stream<BridgeEvent> events() {
    controller.onCancel = () => cancelled = true;
    return controller.stream;
  }

  @override
  Future<void> prompt(String message) async => calls.add('prompt $message');
  @override
  Future<void> steer(String message) async => calls.add('steer $message');
  @override
  Future<void> abort() async => calls.add('abort');
  @override
  Future<void> setApprovalMode(BridgeApprovalMode mode) async =>
      calls.add('mode ${mode.name}');
  @override
  Future<void> decide(String request, BridgeDecision decision) async =>
      calls.add('decide $request ${decision.name}');
  @override
  Future<void> answerText(String request, String text) async =>
      calls.add('answer $request $text');
  @override
  Future<void> answerConfirm(String request, bool confirmed) async =>
      calls.add('confirm $request $confirmed');
  @override
  Future<void> dismiss(String request) async => calls.add('dismiss $request');
  @override
  Future<List<BridgeFileChange>> changes() async => changed;
  @override
  Future<String> patch(String path) async => patches[path] ?? '';
  @override
  Future<bool> shutdown() async {
    calls.add('shutdown');
    return true;
  }
}

BridgeEvent event(
  BridgeEventKind kind, {
  int? turn,
  BridgeItem? item,
  BridgeStep? step,
  BridgeRequest? request,
  String? text,
  BridgeDisplay? display,
  bool? success,
}) => BridgeEvent(
  kind: kind,
  turn: turn,
  item: item,
  step: step,
  request: request,
  text: text,
  display: display,
  success: success,
);

BridgeItem item(
  int id, {
  BridgeItemKind? kind,
  String? tool,
  bool failed = false,
  int? durationMs,
}) => BridgeItem(
  id: id,
  kind: kind,
  tool: tool,
  failed: failed,
  durationMs: durationMs,
);

BridgeUsage usage(int input, int output, int cost) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
  costMicroUsd: cost,
);
