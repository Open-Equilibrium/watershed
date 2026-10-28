// Observation workload for a long line in an approval (PERFORMANCE.md, Liquid): a shell command
// of 1,000,000 characters without spaces. Prints one JSON line per sample, the time until the
// opened card has settled, and never compares with a threshold. Debug-mode JIT timings from `flutter test` are comparable only with each other.
// Run in liquid/app:
//
//   flutter test benchmark/approval_line_observe_test.dart
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import '../test/fake_backend.dart';

const chars = 1000000;
const samples = 5;

void main() {
  testWidgets('observe a long approval line', (tester) async {
    final command = [
      for (var char = 0; char < chars; char++) 'abcdefghij'[char % 10],
    ].join();
    Future<double> settled(Future<void> Function() change) async {
      final watch = Stopwatch()..start();
      await change();
      await tester.pumpAndSettle();
      return watch.elapsedMicroseconds / 1000;
    }

    for (var sample = 0; sample < samples; sample++) {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      final backend = FakeBackend();
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: AgentShell(
            state: SessionState(backend),
            info: const SessionInfo(
              project: 'acme-api',
              harness: 'pi',
              device: 'MacBook',
            ),
          ),
        ),
      );
      final opened = await settled(() async {
        backend.controller.add(
          event(
            BridgeEventKind.requestOpened,
            request: BridgeRequest(
              id: 'a$sample',
              approval: BridgeApproval(
                kind: BridgeRequestKind.commandExecution,
                tool: 'bash',
                text: command,
              ),
            ),
          ),
        );
      });
      // ignore: avoid_print
      print(jsonEncode({'sample': sample, 'opened_ms': opened}));
    }
    await tester.binding.setSurfaceSize(null);
  }, timeout: const Timeout(Duration(minutes: 10)));
}
