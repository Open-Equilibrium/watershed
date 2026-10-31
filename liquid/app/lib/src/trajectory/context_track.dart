import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';
import 'trajectory_model.dart';

/// Tokens pi keeps free by default; it compacts once the context grows past the window minus
/// these.
const piCompactionReserve = 16384;

/// The model's context after one step, in tokens.
@immutable
class ContextLevel {
  const ContextLevel(
    this.tokens, {
    required this.exact,
    this.added = 0,
    this.addedExact = true,
  });

  /// Tokens in the context after the step.
  final int tokens;

  /// Whether the provider measured [tokens]; otherwise they are estimated from characters.
  final bool exact;

  /// Tokens the step added.
  final int added;

  /// Whether the provider measured [added].
  final bool addedExact;
}

/// The context along a branch: its size before the first step and after each one.
@immutable
class ContextTrack {
  const ContextTrack(this.start, this.levels);

  /// The system prompt and tools of the first request.
  final ContextLevel start;

  /// One level per record.
  final List<ContextLevel> levels;

  /// The level before `turn`'s first step.
  ContextLevel before(TurnGroup turn) =>
      turn.first == 0 ? start : levels[turn.first - 1];
}

/// The context after each record. A request measures it exactly: the prompt or tool result
/// right before it takes the request's size, and a response adds its output. The steps a
/// request read add their characters at its tokens per character, as the inspector splits it;
/// steps no request read yet use the latest request's.
ContextTrack contextTrack(List<BridgeRecord> records) {
  final first = records.where(_measured).firstOrNull;
  var ratio = _ratio(first);
  final saw = first?.context;
  final start = ContextLevel(
    saw == null ? 0 : ((saw.promptChars + saw.toolChars) * ratio).round(),
    exact: false,
  );
  final levels = <ContextLevel>[];
  // The context after the previous response, and the first step since it.
  var base = start.tokens;
  var since = 0;
  void estimate() {
    var level = base;
    for (var i = since; i < levels.length; i++) {
      final approval = records[i].kind == BridgeRecordKind.approval;
      final added = approval ? 0 : (records[i].addedChars * ratio).round();
      level += added;
      levels[i] = ContextLevel(
        level,
        exact: false,
        added: added,
        addedExact: approval,
      );
    }
  }

  for (final record in records) {
    final usage = record.usage;
    if (usage == null || !_measured(record)) {
      levels.add(const ContextLevel(0, exact: false));
      continue;
    }
    ratio = _ratio(record);
    estimate();
    final measured = promptTokens(usage);
    for (var i = levels.length - 1; i >= since; i--) {
      final kind = records[i].kind;
      if (kind == BridgeRecordKind.approval) {
        levels[i] = ContextLevel(measured, exact: false);
      } else if (kind != BridgeRecordKind.message) {
        levels[i] = ContextLevel(
          measured,
          exact: true,
          added: levels[i].added,
          addedExact: false,
        );
        break;
      }
    }
    base = measured + usage.output;
    levels.add(ContextLevel(base, exact: false, added: usage.output));
    since = levels.length;
  }
  estimate();
  return ContextTrack(start, levels);
}

bool _measured(BridgeRecord record) =>
    record.kind == BridgeRecordKind.message &&
    (record.usage == null ? 0 : promptTokens(record.usage!)) > 0;

/// Tokens per character of `request`, or pi's estimate of one token per four characters.
double _ratio(BridgeRecord? request) {
  final usage = request?.usage;
  final saw = request?.context;
  final chars = saw == null
      ? 0
      : saw.promptChars + saw.toolChars + saw.messageChars;
  if (usage == null || chars == 0 || promptTokens(usage) == 0) return 0.25;
  return promptTokens(usage) / chars;
}

/// The index of the request that read the step at `index`: a response's own, otherwise the
/// next one; none while no request read it yet.
int? requestOf(List<BridgeRecord> records, int index) {
  for (var i = index; i < records.length; i++) {
    if (records[i].kind == BridgeRecordKind.message) return i;
  }
  return null;
}

/// The request's number on its branch, counted from 1.
int requestNumber(List<BridgeRecord> records, int index) => records
    .take(index + 1)
    .where((record) => record.kind == BridgeRecordKind.message)
    .length;

/// One request's measured input in parts, in tokens; every part but the total is estimated
/// from its characters.
@immutable
class RequestParts {
  const RequestParts({
    required this.total,
    required this.system,
    required this.projectContext,
    required this.tools,
    required this.step,
    required this.carried,
    this.largest = const [],
  });

  /// The measured input.
  final int total;

  /// The system prompt without its project context.
  final int system;

  /// The project's `AGENTS.md` files in the system prompt.
  final int projectContext;
  final int tools;

  /// What the inspected step put into the request.
  final int step;

  /// Everything else from earlier steps.
  final int carried;

  /// The three largest parts, by name.
  final List<(String, int)> largest;
}

/// `request` split into the system prompt, its project context, the tools, the inspected
/// step's `stepChars` and what was carried over; the project context and the largest parts
/// need the request's `detail`.
RequestParts requestParts(
  BridgeRecord request, {
  BridgeContextDetail? detail,
  int stepChars = 0,
}) {
  final usage = request.usage;
  final saw = request.context;
  final total = usage == null ? 0 : promptTokens(usage);
  if (saw == null) {
    return RequestParts(
      total: total,
      system: 0,
      projectContext: 0,
      tools: 0,
      step: 0,
      carried: total,
    );
  }
  final ratio = _ratio(request);
  int tokens(int chars) => (chars * ratio).round();
  final projectChars = [
    for (final section in detail?.sections ?? const <BridgeContextPart>[])
      if (section.name == 'project_context') section.chars,
  ].fold(0, (sum, chars) => sum + chars);
  final system = tokens(saw.promptChars - projectChars);
  final project = tokens(projectChars);
  final tools = tokens(saw.toolChars);
  final step = tokens(stepChars);
  final largest = detail == null
      ? const <(String, int)>[]
      : ([
          ('System prompt', system),
          if (project > 0) ('AGENTS.md', project),
          ('Tools', tools),
          for (final message in detail.messages)
            if (message.inContext) (message.label, tokens(message.chars)),
        ]..sort((a, b) => b.$2.compareTo(a.$2))).take(3).toList();
  return RequestParts(
    total: total,
    system: system,
    projectContext: project,
    tools: tools,
    step: step,
    carried: math.max(0, total - system - project - tools - step),
    largest: largest,
  );
}

/// The context window of `model` among the agent's `models`; none when unknown.
int? contextWindowOf(String model, List<BridgeModel> models) =>
    models.where((known) => known.id == model).firstOrNull?.contextWindow;

/// Where pi compacts in a context of `window` tokens; none for a window that small.
int? compactionPoint(int window) =>
    window > piCompactionReserve ? window - piCompactionReserve : null;
