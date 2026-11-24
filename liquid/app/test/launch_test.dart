import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/session.dart' show BridgeThreadSummary;
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const _usage = 'Start Liquid with LIQUID_PROJECT set to a project directory.';
const studio = 'ws://studio.local:3774';

({Launch launch, RustAgentHostLinks links}) windowFor(
  Map<String, String> environment, {
  Map<String?, Object> answers = const {},
  Future<ProjectBackend> Function(String? address, String root)? open,
  Random? random,
  DateTime Function()? clock,
  Future<String> Function(ProjectBackend project)? machineOf,
}) {
  final launch = launchFromEnvironment(environment);
  final links = RustAgentHostLinks(
    launch,
    environment,
    open:
        open ??
        (address, root) async {
          final answer = answers[address];
          if (answer is ProjectBackend) return answer;
          throw answer ?? 'no answer';
        },
    random: random,
    clock: clock,
    machineOf: machineOf,
  );
  return (launch: launch, links: links);
}

Iterable<String> sessionIds(Launch launch) => [
  for (final listed in launch.state!.recent.value) listed.thread.id,
];

class FixedRandom implements Random {
  FixedRandom(this.value);

  final double value;

  @override
  double nextDouble() => value;

  @override
  int nextInt(int max) => 0;

  @override
  bool nextBool() => value >= 0.5;
}

class _AnswersOnce extends FakeProject {
  var _reads = 0;

  @override
  Future<List<BridgeThreadSummary>> threads() async {
    if (_reads++ > 0) throw 'the host is down';
    return threadList;
  }
}

void main() {
  final project = Directory.current.path;
  final environment = {'LIQUID_PROJECT': project};

  group('agent host links (ADR-0295)', () {
    test('connects This Mac and each agent host, and names one that does not answer', () async {
      final (:launch, :links) = windowFor(
        environment,
        answers: {
          null: FakeProject()..threadList = [threadOf('local')],
          studio: 'no answer',
        },
      );

      await links.connect(null);
      await links.connect(studio);

      final state = launch.state!;
      expect(state.answers(null), isTrue);
      expect(state.answers(studio), isFalse);
      expect(
        state.hostProblems.value[studio],
        'Liquid could not connect to the agent host $studio: no answer.',
      );
      expect(sessionIds(launch), ['local']);
    });

    testWidgets(
      'a missing project folder is named and not retried, while an agent host lists its sessions',
      (tester) async {
        final (:launch, :links) = windowFor(
          {'LIQUID_PROJECT': '/nonexistent/acme-api'},
          answers: {
            studio: FakeProject()..threadList = [threadOf('studio')],
          },
        );

        await links.connect(null);
        await links.connect(studio);
        await tester.pump(const Duration(hours: 1));

        expect(launch.state!.answers(null), isFalse);
        expect(
          launch.state!.hostProblems.value[null],
          'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
        );
        expect(sessionIds(launch), ['studio']);
      },
    );

    test('a path that cannot be used is named on the host it fails on', () async {
      final cases = [
        (
          {'LIQUID_PROJECT': project, 'META_HARNESS_HOME': 'meta-data'},
          null,
          'META_HARNESS_HOME must be an absolute path: meta-data',
        ),
        (
          {'LIQUID_PROJECT': 'acme-api'},
          studio,
          'LIQUID_PROJECT must be an absolute path on the agent host $studio: acme-api',
        ),
      ];
      for (final (env, address, problem) in cases) {
        final (:launch, :links) = windowFor(env);
        await links.connect(address);
        expect(launch.state!.hostProblems.value[address], problem);
      }
    });

    test('a host for new sessions connects first, and starts there once it answers', () async {
      final answers = <String?, Object>{
        null: FakeProject(),
        studio: 'no answer',
      };
      final (:launch, :links) = windowFor(environment, answers: answers);

      expect(
        await links.startOn(studio),
        'Liquid could not connect to the agent host $studio: no answer.',
      );
      expect(launch.state!.startAddress, isNull);

      answers[studio] = FakeProject()..threadList = [threadOf('studio')];
      expect(await links.startOn(studio), isNull);
      expect(launch.state!.startAddress, studio);
      expect(sessionIds(launch), ['studio']);
    });

    test(
      'a host for new sessions waits for a connection already in progress',
      () async {
        final attempt = Completer<ProjectBackend>();
        final (:launch, :links) = windowFor(
          environment,
          open: (address, root) => attempt.future,
        );

        unawaited(links.connect(studio));
        final starting = links.startOn(studio);
        await pumpEventQueue();
        attempt.complete(FakeProject()..threadList = [threadOf('studio')]);

        expect(await starting, isNull);
        expect(launch.state!.startAddress, studio);
      },
    );

    test('without a project, no agent host starts sessions and the launch problem says why', () async {
      final (:launch, :links) = windowFor({}, answers: {null: FakeProject()});

      await links.connect(null);

      expect(launch.state, isNull);
      expect(launchFromEnvironment({}).problem, _usage);
      expect(await links.startOn(null), _usage);
    });

    testWidgets(
      'an agent host that does not answer is retried in the background, with T3\'s backoff',
      (tester) async {
        var opens = 0;
        final (:launch, :links) = windowFor(
          environment,
          random: FixedRandom(0),
          open: (address, root) async {
            opens++;
            if (opens < 4) throw 'no answer';
            return FakeProject();
          },
        );

        unawaited(links.connect(studio));
        await tester.pump();
        expect(opens, 1);

        for (final (milliseconds, attempts) in [
          (999, 1),
          (1, 2),
          (1999, 2),
          (1, 3),
          (3999, 3),
          (1, 4),
        ]) {
          await tester.pump(Duration(milliseconds: milliseconds));
          expect(opens, attempts, reason: 'after $milliseconds ms more');
        }
        expect(launch.state!.answers(studio), isTrue);
        expect(launch.state!.hostProblems.value, isEmpty);
      },
    );

    testWidgets(
      'coming back to the app retries a host that waits on its backoff at once',
      (tester) async {
        var opens = 0;
        final (:launch, :links) = windowFor(
          environment,
          random: FixedRandom(0),
          open: (address, root) async {
            opens++;
            if (opens == 1) throw 'no answer';
            return FakeProject();
          },
        );
        unawaited(links.connect(studio));
        await tester.pump();
        expect(opens, 1);

        links.retryAll();
        await tester.pump();

        expect(opens, 2);
        expect(launch.state!.answers(studio), isTrue);
        await tester.pump(const Duration(hours: 1));
        expect(opens, 2, reason: 'the answered host is not retried');
      },
    );

    testWidgets(
      'a host that drops a live connection is retried after a quarter second, as T3 does',
      (tester) async {
        var opens = 0;
        final (:launch, :links) = windowFor(
          environment,
          random: FixedRandom(0),
          open: (address, root) async {
            opens++;
            return _AnswersOnce();
          },
        );
        await links.connect(studio);
        expect(launch.state!.answers(studio), isTrue);

        await launch.state!.loadStart();
        expect(launch.state!.answers(studio), isFalse);

        await tester.pump(const Duration(milliseconds: 124));
        expect(opens, 1);
        await tester.pump(const Duration(milliseconds: 1));
        expect(opens, 2);
        expect(launch.state!.answers(studio), isTrue);
      },
    );

    testWidgets(
      'removing an agent host while it connects ends that attempt, its retries and its project, and pairing it again connects it',
      (tester) async {
        var opens = 0;
        final attempt = Completer<ProjectBackend>();
        final (:launch, :links) = windowFor(
          environment,
          random: FixedRandom(0),
          open: (address, root) {
            opens++;
            return opens == 1 ? attempt.future : Future.value(FakeProject());
          },
          machineOf: (_) async => 'server',
        );

        unawaited(links.connect(studio));
        await tester.pump();
        links.disconnect(studio);
        final gone = FakeProject();
        attempt.complete(gone);
        await tester.pump(const Duration(hours: 1));

        expect(launch.state!.answers(studio), isFalse);
        expect(gone.closed, isTrue);
        expect(opens, 1, reason: 'no retry ran');

        await links.connect(studio);
        expect(launch.state!.answers(studio), isTrue);
      },
    );

    testWidgets(
      'a failed attempt for a host removed meanwhile schedules no retry',
      (tester) async {
        var opens = 0;
        final attempt = Completer<ProjectBackend>();
        final (:launch, :links) = windowFor(
          environment,
          random: FixedRandom(0),
          open: (address, root) {
            opens++;
            return attempt.future;
          },
        );

        unawaited(links.connect(studio));
        await tester.pump();
        links.disconnect(studio);
        attempt.completeError('no answer');
        await tester.pump(const Duration(hours: 1));

        expect(opens, 1);
        expect(launch.state!.hostProblems.value, isEmpty);
      },
    );
  });
}
