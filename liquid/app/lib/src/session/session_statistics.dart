import 'dart:math' as math;

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/thread.dart';

/// The chart series of the three largest models; the models after them share the last one.
const otherSeries = 3;

/// A session's statistics summed for the card and the statistics panel. Each value adds up
/// what T3 reports; T3 reports no cost and no model time, so those stay unknown, never a guess
/// (ADR-0279, ADR-0293).
class SessionStatistics {
  SessionStatistics(this.statistics, {this.running = false});

  final BridgeStatistics statistics;

  /// Whether a turn runs, when the calls still open count as running rather than cancelled.
  final bool running;

  late final BridgeUsage usage = sumUsage(
    statistics.models.map((model) => model.usage),
  );

  /// Input, cache reads and writes, and output, as the Tokens row counts them.
  int get tokens => tokensOf(usage);
  int get responses =>
      statistics.models.fold(0, (sum, model) => sum + model.responses);

  late final List<ModelStatistics> models = [
    for (final stats in statistics.models)
      ModelStatistics(stats, seriesOf(stats.model)),
  ];

  late final List<TurnStatistics> turns = [
    for (final (index, stats) in statistics.turns.indexed)
      TurnStatistics(index + 1, stats),
  ];

  /// The chart series of a model: its place among the largest models, or the shared one.
  int seriesOf(String model) {
    final index = statistics.models.indexWhere((it) => it.model == model);
    return index < 0 || index >= otherSeries ? otherSeries : index;
  }

  /// Working time of every turn whose clock is known, from its first entry to its last.
  int get workingMs =>
      turns.fold(0, (sum, turn) => sum + (turn.workingMs ?? 0));

  /// The tool time; null while a turn lacks it.
  int? get toolMs => _sum([for (final turn in turns) turn.toolMs]);

  /// From the session's first entry to its last.
  int? get spanMs {
    final (start, end) = (statistics.startedMs, statistics.endedMs);
    return start == null || end == null ? null : end - start;
  }

  /// Each tool's calls by outcome, in the order the statistics list the tools.
  late final List<ToolStatistics> tools = [
    for (final tool in statistics.tools)
      ToolStatistics(tool, turnRunning: running),
  ];

  /// Tool calls by outcome, over every tool and the whole session.
  int get toolCalls =>
      toolsSucceeded + toolsFailed + toolsCancelled + toolsRunning;
  int get toolsSucceeded => _count((tool) => tool.succeeded);
  int get toolsFailed => _count((tool) => tool.failed);
  int get toolsCancelled => _count((tool) => tool.cancelled);
  int get toolsRunning => _count((tool) => tool.running);

  int _count(int Function(ToolStatistics tool) of) =>
      tools.fold(0, (sum, tool) => sum + of(tool));

  /// The tokens an input and output of `usage` count to.
  static int tokensOf(BridgeUsage usage) => promptTokens(usage) + usage.output;

  /// The sum of `usages`, field by field.
  static BridgeUsage sumUsage(Iterable<BridgeUsage> usages) {
    var input = 0, output = 0, cacheRead = 0, cacheWrite = 0;
    var reasoning = 0;
    for (final usage in usages) {
      input += usage.input;
      output += usage.output;
      cacheRead += usage.cacheRead;
      cacheWrite += usage.cacheWrite;
      reasoning += usage.reasoning;
    }
    return BridgeUsage(
      input: input,
      output: output,
      cacheRead: cacheRead,
      cacheWrite: cacheWrite,
      reasoning: reasoning,
    );
  }

  /// The sum of `values`, or null when one of them is unknown.
  static int? _sum(Iterable<int?> values) {
    var total = 0;
    for (final value in values) {
      if (value == null) return null;
      total += value;
    }
    return total;
  }
}

/// What one model used over the whole session.
class ModelStatistics {
  ModelStatistics(this.stats, this.series);

  final BridgeModelUsage stats;

  /// The chart series the model fills, as [SessionStatistics.seriesOf] names it.
  final int series;

  String get name => modelName(stats.model);
  int get tokens => SessionStatistics.tokensOf(stats.usage);
  int get responses => stats.responses;
}

/// The name a model is shown by: its id without the provider, or a label for usage that no model
/// produced, such as a compaction summary.
String modelName(String model) {
  if (model.isEmpty) return 'Without a model';
  final slash = model.indexOf('/');
  return slash < 0 ? model : model.substring(slash + 1);
}

/// One tool's calls by outcome. A call still open counts as running while a turn runs and as
/// cancelled once none does, so the card and the statistics panel agree (ADR-0279).
class ToolStatistics {
  ToolStatistics(this.stats, {required this.turnRunning});

  final BridgeToolCalls stats;

  /// Whether a turn runs, when the open calls count as running rather than cancelled.
  final bool turnRunning;

  String get name => stats.name;
  int? get durationMs => stats.durationMs;
  int get succeeded => stats.succeeded;
  int get failed => stats.failed;
  int get cancelled => stats.cancelled + (turnRunning ? 0 : stats.open);
  int get running => turnRunning ? stats.open : 0;

  /// Every call of the tool, by any outcome.
  int get total => succeeded + failed + cancelled + running;
}

/// One turn of the session.
class TurnStatistics {
  TurnStatistics(this.index, this.stats);

  /// The turn's position, from 1.
  final int index;
  final BridgeTurnStatistics stats;

  String get prompt => stats.prompt;

  late final BridgeUsage usage = SessionStatistics.sumUsage(
    stats.models.map((model) => model.usage),
  );
  int get tokens => SessionStatistics.tokensOf(usage);
  int get responses =>
      stats.models.fold(0, (sum, model) => sum + model.responses);
  int get failed => stats.failed;

  /// From the turn's start to its end; null when either is unknown.
  int? get workingMs {
    final (start, end) = (stats.startedMs, stats.endedMs);
    if (start == null || end == null) return null;
    return math.max(0, end - start);
  }
  int? get toolMs => stats.toolMs;
}
