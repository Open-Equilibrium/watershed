import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_figures.dart';

BridgeUsage tokens({
  int input = 0,
  int output = 0,
  int cacheRead = 0,
  int cacheWrite = 0,
  int cost = 0,
}) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: cacheRead,
  cacheWrite: cacheWrite,
  reasoning: 0,
  costMicroUsd: cost,
);

BridgeModelUsage model(
  String name,
  BridgeUsage usage, {
  int responses = 1,
  int unpriced = 0,
}) => BridgeModelUsage(
  model: name,
  usage: usage,
  costByKind: const BridgeCostByKind(
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
  ),
  responses: responses,
  unpriced: unpriced,
);

BridgeTurnStatistics turn({
  required String prompt,
  int? startedMs,
  int? endedMs,
  List<BridgeModelUsage> models = const [],
  int? modelMs,
  int? toolMs,
  int toolCalls = 0,
  int failed = 0,
}) => BridgeTurnStatistics(
  entry: 'e-$prompt',
  prompt: prompt,
  startedMs: startedMs,
  endedMs: endedMs,
  models: models,
  modelMs: modelMs,
  toolMs: toolMs,
  toolCalls: toolCalls,
  failed: failed,
);

void main() {
  test('sums tokens, cost and responses over the models', () {
    final figures = SessionFigures(
      BridgeStatistics(
        models: [
          model(
            'anthropic/claude-sonnet',
            tokens(
              input: 1000,
              output: 200,
              cacheRead: 3000,
              cacheWrite: 50,
              cost: 1000000,
            ),
            responses: 4,
            unpriced: 1,
          ),
          model(
            'openai/gpt',
            tokens(input: 500, output: 100),
            responses: 2,
            unpriced: 2,
          ),
        ],
        tools: const [],
        turns: const [],
      ),
    );

    expect(
      figures.tokens,
      4850,
      reason: 'input, cache reads and writes, output',
    );
    expect(figures.cost, 1000000);
    expect(figures.responses, 6);
    expect(figures.unpriced, 3);
    expect(figures.models.map((it) => it.name), ['claude-sonnet', 'gpt']);
  });

  test('splits working time into model, tools and other, clamped at zero', () {
    final figures = SessionFigures(
      BridgeStatistics(
        models: const [],
        tools: const [],
        turns: [
          turn(
            prompt: 'one',
            startedMs: 0,
            endedMs: 10000,
            modelMs: 6000,
            toolMs: 3000,
          ),
          turn(
            prompt: 'two',
            startedMs: 20000,
            endedMs: 25000,
            modelMs: 6000,
            toolMs: 0,
          ),
        ],
      ),
    );

    expect(figures.workingMs, 15000);
    expect(figures.modelMs, 12000);
    expect(figures.toolMs, 3000);
    expect(
      figures.otherMs,
      1000,
      reason: 'a turn whose requests overlap its clock keeps no negative other time',
    );
  });

  test('a turn without timing entries leaves model and tool time unknown', () {
    final figures = SessionFigures(
      BridgeStatistics(
        models: const [],
        tools: const [],
        turns: [
          turn(
            prompt: 'new',
            startedMs: 0,
            endedMs: 4000,
            modelMs: 3000,
            toolMs: 1000,
          ),
          turn(prompt: 'older', startedMs: 5000, endedMs: 9000),
        ],
      ),
    );

    expect(
      figures.workingMs,
      8000,
      reason: 'the turn clocks need no timing entries',
    );
    expect(figures.modelMs, isNull);
    expect(figures.toolMs, isNull);
    expect(figures.otherMs, isNull);
    expect(figures.turns.last.workingMs, 4000);
    expect(figures.turns.last.modelMs, isNull);
  });

  test('the session span runs from its first entry to its last', () {
    final figures = SessionFigures(
      const BridgeStatistics(
        models: [],
        tools: [],
        turns: [],
        startedMs: 1000,
        endedMs: 7500000,
      ),
    );

    expect(figures.spanMs, 7499000);
    expect(
      SessionFigures(const BridgeStatistics(models: [], tools: [], turns: []))
          .spanMs,
      isNull,
    );
  });

  test(
    'open tool calls are running while a turn runs, cancelled otherwise',
    () {
      final stats = BridgeStatistics(
        models: const [],
        tools: [
          const BridgeToolCalls(
            name: 'bash',
            succeeded: 16,
            failed: 5,
            cancelled: 1,
            open: 2,
          ),
        ],
        turns: const [],
      );

      final idle = SessionFigures(stats);
      expect(
        [
          idle.toolCalls,
          idle.toolsSucceeded,
          idle.toolsFailed,
          idle.toolsCancelled,
          idle.toolsRunning,
        ],
        [24, 16, 5, 3, 0],
      );
      final running = SessionFigures(stats, running: true);
      expect(
        [running.toolCalls, running.toolsCancelled, running.toolsRunning],
        [24, 1, 2],
      );
    },
  );

  test(
    'the three largest models have a series of their own, the rest share one',
    () {
      final figures = SessionFigures(
        BridgeStatistics(
          models: [
            for (final name in ['a', 'b', 'c', 'd'])
              model('p/$name', tokens(cost: 1)),
          ],
          tools: const [],
          turns: [
            turn(prompt: 'x', models: [model('p/d', tokens(cost: 1))]),
          ],
        ),
      );

      expect(
        [
          for (final name in ['p/a', 'p/b', 'p/c', 'p/d'])
            figures.seriesOf(name),
        ],
        [0, 1, 2, otherSeries],
      );
      expect(figures.seriesOf('p/unknown'), otherSeries);
    },
  );
}
