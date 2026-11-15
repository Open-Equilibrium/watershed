import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_statistics.dart';

BridgeUsage tokens({
  int input = 0,
  int output = 0,
  int cacheRead = 0,
  int cacheWrite = 0,
}) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: cacheRead,
  cacheWrite: cacheWrite,
  reasoning: 0,
);

BridgeModelUsage model(String name, BridgeUsage usage, {int responses = 1}) =>
    BridgeModelUsage(model: name, usage: usage, responses: responses);

BridgeTurnStatistics turn({
  required String prompt,
  int? startedMs,
  int? endedMs,
  int? toolMs,
}) => BridgeTurnStatistics(
  number: 0,
  prompt: prompt,
  startedMs: startedMs,
  endedMs: endedMs,
  models: const [],
  toolMs: toolMs,
  failed: 0,
);

BridgeStatistics stats({
  List<BridgeModelUsage> models = const [],
  List<BridgeToolCalls> tools = const [],
  List<BridgeTurnStatistics> turns = const [],
  int? startedMs,
  int? endedMs,
}) => BridgeStatistics(
  models: models,
  tools: tools,
  turns: turns,
  startedMs: startedMs,
  endedMs: endedMs,
);

void main() {
  test('tokens and responses add up over the models, and a model name drops its provider', () {
    final statistics = SessionStatistics(
      stats(
        models: [
          model(
            'anthropic/claude-sonnet',
            tokens(input: 1000, output: 200, cacheRead: 3000, cacheWrite: 50),
            responses: 4,
          ),
          model('openai/gpt', tokens(input: 500, output: 100), responses: 2),
        ],
      ),
    );

    expect(
      statistics.tokens,
      4850,
      reason: 'input, cache reads and writes, and output',
    );
    expect(statistics.responses, 6);
    expect(statistics.models.map((it) => it.name), ['claude-sonnet', 'gpt']);
    expect(modelName(''), 'Without a model');
  });

  test('working time adds the turns whose clock is known, and turns are numbered from 1', () {
    final statistics = SessionStatistics(
      stats(
        turns: [
          turn(prompt: 'done', startedMs: 0, endedMs: 10000),
          turn(prompt: 'running', startedMs: 20000),
        ],
      ),
    );

    expect(statistics.workingMs, 10000);
    expect(statistics.turns.map((turn) => (turn.index, turn.workingMs)), [
      (1, 10000),
      (2, null),
    ]);
  });

  test('tool time adds up only when every turn has it, and model time is never reported', () {
    final known = SessionStatistics(
      stats(
        turns: [
          turn(prompt: 'one', startedMs: 0, endedMs: 10000, toolMs: 3000),
          turn(prompt: 'two', startedMs: 20000, endedMs: 25000, toolMs: 1000),
        ],
      ),
    );
    expect(known.toolMs, 4000);

    final partial = SessionStatistics(
      stats(
        turns: [
          turn(prompt: 'new', startedMs: 0, endedMs: 4000, toolMs: 1000),
          turn(prompt: 'older', startedMs: 5000, endedMs: 9000),
        ],
      ),
    );
    expect(partial.toolMs, isNull);
  });

  test(
    'the session span runs from its first entry to its last, or is unknown',
    () {
      expect(
        SessionStatistics(stats(startedMs: 1000, endedMs: 7500000)).spanMs,
        7499000,
      );
      expect(SessionStatistics(stats()).spanMs, isNull);
    },
  );

  test('an open tool call is running while a turn runs, and cancelled once none does', () {
    const tool = BridgeToolCalls(
      name: 'bash',
      succeeded: 16,
      failed: 5,
      cancelled: 1,
      open: 2,
    );

    final idle = ToolStatistics(tool, turnRunning: false);
    expect([idle.cancelled, idle.running, idle.total], [3, 0, 24]);
    final busy = ToolStatistics(tool, turnRunning: true);
    expect([busy.cancelled, busy.running, busy.total], [1, 2, 24]);

    final session = SessionStatistics(
      stats(tools: const [tool]),
      running: true,
    );
    expect(
      [session.toolCalls, session.toolsCancelled, session.toolsRunning],
      [24, 1, 2],
    );
  });

  test(
    'the three largest models have a series of their own, the rest share one',
    () {
      final statistics = SessionStatistics(
        stats(
          models: [
            for (final name in ['a', 'b', 'c', 'd']) model('p/$name', tokens()),
          ],
        ),
      );

      expect(
        [
          for (final name in ['p/a', 'p/b', 'p/c', 'p/d'])
            statistics.seriesOf(name),
        ],
        [0, 1, 2, otherSeries],
      );
      expect(statistics.seriesOf('p/unknown'), otherSeries);
    },
  );
}
