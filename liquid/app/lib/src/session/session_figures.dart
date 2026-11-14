import 'dart:math' as math;

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/history.dart';

/// The chart series of the three largest models; the models after them share the last one.
const otherSeries = 3;

/// A session's statistics summed for the card and the statistics panel. Each figure adds up
/// what the stored entries hold; a figure that needs timing a turn lacks is null, never a guess
/// (ADR-0279).
class SessionFigures {
  SessionFigures(this.statistics, {this.running = false});

  final BridgeStatistics statistics;

  /// Whether a turn runs, when the calls still open count as running rather than cancelled.
  final bool running;

  late final BridgeUsage usage = sumUsage(
    statistics.models.map((model) => model.usage),
  );

  /// Input, cache reads and writes, and output, as the Tokens figure counts them.
  int get tokens => tokensOf(usage);
  int get cost => usage.costMicroUsd;

  /// What each kind of the session's tokens cost, summed over the models.
  late final BridgeCostByKind costByKind = BridgeCostByKind(
    input: _costOf((kind) => kind.input),
    output: _costOf((kind) => kind.output),
    cacheRead: _costOf((kind) => kind.cacheRead),
    cacheWrite: _costOf((kind) => kind.cacheWrite),
  );
  int get responses =>
      statistics.models.fold(0, (sum, model) => sum + model.responses);
  int get unpriced =>
      statistics.models.fold(0, (sum, model) => sum + model.unpriced);

  int _costOf(int Function(BridgeCostByKind kind) of) =>
      statistics.models.fold(0, (sum, model) => sum + of(model.costByKind));

  late final List<ModelFigure> models = [
    for (final stats in statistics.models)
      ModelFigure(stats, seriesOf(stats.model)),
  ];

  late final List<TurnFigure> turns = [
    for (final (index, stats) in statistics.turns.indexed)
      TurnFigure(index + 1, stats, seriesOf),
  ];

  /// The chart series of a model: its place among the largest models, or the shared one.
  int seriesOf(String model) {
    final index = statistics.models.indexWhere((it) => it.model == model);
    return index < 0 || index >= otherSeries ? otherSeries : index;
  }

  /// Working time of every turn whose clock is known, from its first entry to its last.
  int get workingMs => _sum([for (final turn in turns) turn.workingMs]) ?? 0;

  /// The model and tool time, and the rest of the turns' time; null while a turn lacks timing.
  int? get modelMs => _sum([for (final turn in turns) turn.modelMs]);
  int? get toolMs => _sum([for (final turn in turns) turn.toolMs]);
  int? get otherMs => _sum([for (final turn in turns) turn.otherMs]);

  /// From the session's first entry to its last.
  int? get spanMs {
    final (start, end) = (statistics.startedMs, statistics.endedMs);
    return start == null || end == null ? null : end - start;
  }

  /// Tool calls by outcome, over every tool and the whole session.
  int get toolCalls =>
      toolsSucceeded + toolsFailed + toolsCancelled + toolsRunning;
  int get toolsSucceeded => _count((tool) => tool.succeeded);
  int get toolsFailed => _count((tool) => tool.failed);
  int get toolsCancelled =>
      _count((tool) => tool.cancelled) + (running ? 0 : _count((t) => t.open));
  int get toolsRunning => running ? _count((tool) => tool.open) : 0;

  int _count(int Function(BridgeToolCalls tool) of) =>
      statistics.tools.fold(0, (sum, tool) => sum + of(tool));

  /// The tokens an input and output of `usage` count to.
  static int tokensOf(BridgeUsage usage) => promptTokens(usage) + usage.output;

  /// The sum of `usages`, field by field.
  static BridgeUsage sumUsage(Iterable<BridgeUsage> usages) {
    var input = 0, output = 0, cacheRead = 0, cacheWrite = 0, cost = 0;
    var reasoning = 0;
    for (final usage in usages) {
      input += usage.input;
      output += usage.output;
      cacheRead += usage.cacheRead;
      cacheWrite += usage.cacheWrite;
      reasoning += usage.reasoning;
      cost += usage.costMicroUsd;
    }
    return BridgeUsage(
      input: input,
      output: output,
      cacheRead: cacheRead,
      cacheWrite: cacheWrite,
      reasoning: reasoning,
      costMicroUsd: cost,
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
class ModelFigure {
  ModelFigure(this.stats, this.series);

  final BridgeModelUsage stats;

  /// The chart series the model fills, as [SessionFigures.seriesOf] names it.
  final int series;

  String get name => modelName(stats.model);
  int get tokens => SessionFigures.tokensOf(stats.usage);
  int get cost => stats.usage.costMicroUsd;
  int get responses => stats.responses;
}

/// The name a model is shown by: its id without the provider, or a label for usage that no model
/// produced, such as a compaction summary.
String modelName(String model) {
  if (model.isEmpty) return 'Without a model';
  final slash = model.indexOf('/');
  return slash < 0 ? model : model.substring(slash + 1);
}

/// One turn of the session, with its time split as the timing entries record it.
class TurnFigure {
  TurnFigure(this.index, this.stats, this._seriesOf);

  /// The turn's number, from 1.
  final int index;
  final BridgeTurnStatistics stats;
  final int Function(String model) _seriesOf;

  String get prompt => stats.prompt;

  late final BridgeUsage usage = SessionFigures.sumUsage(
    stats.models.map((model) => model.usage),
  );
  int get tokens => SessionFigures.tokensOf(usage);
  int get cost => usage.costMicroUsd;
  int get responses =>
      stats.models.fold(0, (sum, model) => sum + model.responses);
  int get failed => stats.failed;
  int get toolCalls => stats.toolCalls;

  /// From the turn's first entry to its last; null when either is unknown.
  int? get workingMs {
    final (start, end) = (stats.startedMs, stats.endedMs);
    if (start == null || end == null) return null;
    return math.max(0, end - start);
  }

  int? get modelMs => stats.modelMs;
  int? get toolMs => stats.toolMs;

  /// What is left of the working time once the model and the tools took theirs.
  int? get otherMs {
    final (working, model, tool) = (workingMs, modelMs, toolMs);
    if (working == null || model == null || tool == null) return null;
    return math.max(0, working - model - tool);
  }

  /// Each series' sum of `value` over the turn's models, by [SessionFigures.seriesOf].
  List<int> bySeries(int Function(BridgeModelUsage model) value) {
    final sums = List.filled(otherSeries + 1, 0);
    for (final model in stats.models) {
      sums[_seriesOf(model.model)] += value(model);
    }
    return sums;
  }
}
