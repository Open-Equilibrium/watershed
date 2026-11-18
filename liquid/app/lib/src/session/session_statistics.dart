import 'dart:math' as math;

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/thread.dart';

const otherSeries = 3;

class SessionStatistics {
  SessionStatistics(this.statistics, {this.running = false});

  final BridgeStatistics statistics;

  final bool running;

  late final BridgeUsage usage = sumUsage(
    statistics.models.map((model) => model.usage),
  );

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

  int seriesOf(String model) {
    final index = statistics.models.indexWhere((it) => it.model == model);
    return index < 0 || index >= otherSeries ? otherSeries : index;
  }

  int get workingMs =>
      turns.fold(0, (sum, turn) => sum + (turn.workingMs ?? 0));

  int? get toolMs => _sum([for (final turn in turns) turn.toolMs]);

  int? get spanMs {
    final (start, end) = (statistics.startedMs, statistics.endedMs);
    return start == null || end == null ? null : end - start;
  }

  late final List<ToolStatistics> tools = [
    for (final tool in statistics.tools)
      ToolStatistics(tool, turnRunning: running),
  ];

  int get toolCalls =>
      toolsSucceeded + toolsFailed + toolsCancelled + toolsRunning;
  int get toolsSucceeded => _count((tool) => tool.succeeded);
  int get toolsFailed => _count((tool) => tool.failed);
  int get toolsCancelled => _count((tool) => tool.cancelled);
  int get toolsRunning => _count((tool) => tool.running);

  int _count(int Function(ToolStatistics tool) of) =>
      tools.fold(0, (sum, tool) => sum + of(tool));

  static int tokensOf(BridgeUsage usage) => promptTokens(usage) + usage.output;

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

  static int? _sum(Iterable<int?> values) {
    var total = 0;
    for (final value in values) {
      if (value == null) return null;
      total += value;
    }
    return total;
  }
}

class ModelStatistics {
  ModelStatistics(this.stats, this.series);

  final BridgeModelUsage stats;

  final int series;

  String get name => modelName(stats.model);
  int get tokens => SessionStatistics.tokensOf(stats.usage);
  int get responses => stats.responses;
}

String modelName(String model) {
  if (model.isEmpty) return 'Without a model';
  final slash = model.indexOf('/');
  return slash < 0 ? model : model.substring(slash + 1);
}

class ToolStatistics {
  ToolStatistics(this.stats, {required this.turnRunning});

  final BridgeToolCalls stats;

  final bool turnRunning;

  String get name => stats.name;
  int? get durationMs => stats.durationMs;
  int get succeeded => stats.succeeded;
  int get failed => stats.failed;
  int get cancelled => stats.cancelled + (turnRunning ? 0 : stats.open);
  int get running => turnRunning ? stats.open : 0;

  int get total => succeeded + failed + cancelled + running;
}

class TurnStatistics {
  TurnStatistics(this.index, this.stats);

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

  int? get workingMs {
    final (start, end) = (stats.startedMs, stats.endedMs);
    if (start == null || end == null) return null;
    return math.max(0, end - start);
  }

  int? get toolMs => stats.toolMs;
}
