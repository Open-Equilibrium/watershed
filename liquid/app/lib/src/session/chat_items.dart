import 'package:flutter/foundation.dart';

import '../format.dart';
import '../rust/api/events.dart';
import '../trajectory/trajectory_model.dart';

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

/// The lines a file change added and removed, when the agent counted both.
({int added, int removed})? changedLines(int? added, int? removed) =>
    added == null || removed == null ? null : (added: added, removed: removed);

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
