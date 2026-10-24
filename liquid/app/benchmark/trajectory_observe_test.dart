// Observation workload for long Trajectories in the Dart UI model (PERFORMANCE.md, Liquid): the
// same session shape as meta-harness-core's `trajectory_observe` example. Prints one JSON line
// per operation and never compares with a threshold. Debug-mode JIT timings from `flutter test`
// are comparable only with each other. Run in liquid/app:
//
//   flutter test benchmark/trajectory_observe_test.dart
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/trajectory/trajectory_model.dart';

import '../test/fake_backend.dart';

const turns = 1000;
const forks = 8;
const forkTurns = 10;
const warmups = 3;
const samples = 20;

void main() {
  test('observe long Trajectories', () {
    final tree = <BridgeTreeNode>[];
    final main = <BridgeRecord>[];
    final forkPoints = <(String, int)>[];
    var clock = 0;
    String? parent;
    void turn(String id, int number, List<BridgeRecord> records) {
      for (final (suffix, kind, entry) in [
        ('u', BridgeEntryKind.user, BridgeRecordKind.user),
        ('ta', BridgeEntryKind.custom, null),
        ('a', BridgeEntryKind.assistant, BridgeRecordKind.message),
        ('p', BridgeEntryKind.custom, BridgeRecordKind.approval),
        ('tt', BridgeEntryKind.custom, null),
        ('r', BridgeEntryKind.toolResult, BridgeRecordKind.tool),
        ('tb', BridgeEntryKind.custom, null),
        ('d', BridgeEntryKind.assistant, BridgeRecordKind.message),
      ]) {
        clock += 500;
        tree.add(
          BridgeTreeNode(
            id: '$id$suffix',
            parent: parent,
            kind: kind,
            preview: 'Add rate limiting $id',
            atMs: clock,
          ),
        );
        parent = '$id$suffix';
        if (entry != null) {
          records.add(
            record(
              '$id$suffix',
              number,
              entry,
              entry == BridgeRecordKind.tool ? 'bash' : 'Step $id',
              clock - 400,
              duration: 400,
              firstToken: entry == BridgeRecordKind.message ? 100 : null,
            ),
          );
        }
      }
    }

    for (var number = 0; number < turns; number++) {
      turn('m$number', number + 1, main);
      if ((number + 1) % (turns ~/ (forks + 1)) == 0 &&
          forkPoints.length < forks) {
        forkPoints.add((parent!, main.length));
      }
    }
    final leaf = parent;
    var fork = <BridgeRecord>[];
    for (final (index, (point, shared)) in forkPoints.indexed) {
      parent = point;
      fork = [...main.take(shared)];
      for (var number = 0; number < forkTurns; number++) {
        turn('f${index}t$number', 1000 + number, fork);
      }
    }
    final first = [...main.take(forkPoints.first.$2)];
    parent = forkPoints.first.$1;
    for (var number = 0; number < forkTurns; number++) {
      turn('g$number', 2000 + number, first);
    }

    void observe(String name, Object? Function() work) {
      for (var i = 0; i < warmups; i++) {
        work();
      }
      final times = <double>[];
      for (var i = 0; i < samples; i++) {
        final watch = Stopwatch()..start();
        work();
        times.add(watch.elapsedMicroseconds / 1000);
      }
      times.sort();
      double rank(double q) =>
          times[((q * times.length).ceil()).clamp(1, times.length) - 1];
      // ignore: avoid_print
      print(
        jsonEncode({
          'operation': name,
          'samples': samples,
          'p50_ms': rank(0.5),
          'p95_ms': rank(0.95),
          'max_ms': times.last,
        }),
      );
    }

    // ignore: avoid_print
    print(
      jsonEncode({
        'nodes': tree.length,
        'records': main.length,
        'fork_records': fork.length,
      }),
    );
    observe('sessionGraph', () => sessionGraph(tree, leaf));
    observe('turnGroups', () => turnGroups(main));
    observe('segments', () => segments(main, equalWidth: false));
    observe(
      'inRange',
      () => inRange(main, (100000, 900000), equalWidth: false),
    );
    observe('compareBranches.late', () => compareBranches(main, fork));
    observe('compareBranches.early', () => compareBranches(main, first));
  });
}
