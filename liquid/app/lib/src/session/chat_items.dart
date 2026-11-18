import 'package:flutter/foundation.dart';

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/thread.dart';

class ItemView {
  ItemView({
    required this.id,
    required this.kind,
    required this.turn,
    this.tool,
    this.target,
    this.turnEnd,
    this.marker,
  });

  final int id;
  final BridgeItemKind kind;

  final int turn;
  final String? tool;

  final String? target;

  final BridgeTurn? turnEnd;

  final String? marker;
  final ValueNotifier<String> text = ValueNotifier('');

  final ValueNotifier<ItemOutcome?> outcome = ValueNotifier(null);
}

typedef ItemOutcome = ({bool failed, int? durationMs});

@immutable
class SessionTotals {
  const SessionTotals({this.input = 0, this.output = 0});

  final int input;
  final int output;

  SessionTotals add(BridgeUsage usage) => SessionTotals(
    input: input + promptTokens(usage),
    output: output + usage.output,
  );
}
