import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/session.dart' show BridgeThreadSummary;
import 'package:liquid/src/session/backend.dart';

import 'fake_backend.dart';

const _usage = 'Start Liquid with LIQUID_PROJECT set to a project directory.';
const studio = 'ws://studio.local:3774';

({Launch launch, RustAgentHostLinks links}) windowFor(
  Map<String, String> environment,
  Map<String?, Object> answers,
) {
  final launch = launchFromEnvironment(environment);
  final links = RustAgentHostLinks(
    launch,
    environment,
    open: (address, root) async {
      final answer = answers[address];
      if (answer is ProjectBackend) return answer;
      throw answer ?? 'no answer';
    },
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

  test('the retry delay doubles from one second up to five minutes, and draws within half to all of it', () {
    expect(retryDelay(0, 0), const Duration(seconds: 1));
    expect(retryDelay(0, 1), const Duration(seconds: 2));
    expect(retryDelay(1, 0), const Duration(seconds: 2));
    expect(retryDelay(2, 1), const Duration(seconds: 8));
    expect(retryDelay(20, 0), const Duration(seconds: 150));
    expect(retryDelay(20, 1), const Duration(minutes: 5));
  });
  final environment = {'LIQUID_PROJECT': project};

  test('refuses a missing project', () {
    expect(launchFromEnvironment({}).problem, _usage);
  });

  test('names the project by its directory', () {
    expect(projectName('/work/acme-api/'), 'acme-api');
    expect(projectName('/'), '/');
  });

  group('agent host links (ADR-0295)', () {
    test('connects This Mac and each agent host, and names one that does not answer', () async {
      final (:launch, :links) = windowFor(environment, {
        null: FakeProject()..threadList = [threadOf('local')],
        studio: 'no answer',
      });

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

    test('This Mac names a missing project folder, and an agent host still lists its sessions', () async {
      final (:launch, :links) = windowFor(
        {'LIQUID_PROJECT': '/nonexistent/acme-api'},
        {
          studio: FakeProject()..threadList = [threadOf('studio')],
        },
      );

      await links.connect(null);
      await links.connect(studio);

      expect(
        launch.state!.hostProblems.value[null],
        'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
      );
      expect(sessionIds(launch), ['studio']);
    });

    test("a refused Meta-Harness home is This Mac's problem", () async {
      final (:launch, :links) = windowFor({
        'LIQUID_PROJECT': project,
        'META_HARNESS_HOME': 'meta-data',
      }, {});

      await links.connect(null);

      expect(
        launch.state!.hostProblems.value[null],
        'META_HARNESS_HOME must be an absolute path: meta-data',
      );
    });

    test('an agent host needs the project folder as an absolute path on it', () async {
      final (:launch, :links) = windowFor({'LIQUID_PROJECT': 'acme-api'}, {});

      await links.connect(studio);

      expect(
        launch.state!.hostProblems.value[studio],
        'LIQUID_PROJECT must be an absolute path on the agent host $studio: acme-api',
      );
    });

    test('a host chosen for sessions connects first, and names why it cannot start', () async {
      final (:launch, :links) = windowFor(environment, {
        null: FakeProject(),
        studio: 'no answer',
      });

      expect(
        await links.startOn(studio),
        'Liquid could not connect to the agent host $studio: no answer.',
      );
      expect(launch.state!.startAddress, isNull);
    });

    test(
      'a host chosen for sessions that answers starts new sessions there',
      () async {
        final (:launch, :links) = windowFor(environment, {
          null: FakeProject(),
          studio: FakeProject()..threadList = [threadOf('studio')],
        });

        expect(await links.startOn(studio), isNull);
        expect(launch.state!.startAddress, studio);
        expect(sessionIds(launch), ['studio']);
      },
    );

    test('without a project, no agent host starts sessions and the launch problem says why', () async {
      final (:launch, :links) = windowFor({}, {null: FakeProject()});

      await links.connect(null);

      expect(launch.state, isNull);
      expect(await links.startOn(null), _usage);
    });

    testWidgets(
      'an agent host that does not answer is retried in the background, with T3\'s backoff',
      (tester) async {
        var opens = 0;
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
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

        await tester.pump(const Duration(milliseconds: 999));
        expect(opens, 1);
        await tester.pump(const Duration(milliseconds: 1));
        expect(opens, 2);
        await tester.pump(const Duration(milliseconds: 1999));
        expect(opens, 2);
        await tester.pump(const Duration(milliseconds: 1));
        expect(opens, 3);
        await tester.pump(const Duration(milliseconds: 3999));
        expect(opens, 3);
        await tester.pump(const Duration(milliseconds: 1));
        expect(opens, 4);
        expect(launch.state!.answers(studio), isTrue);
        expect(launch.state!.hostProblems.value, isEmpty);
      },
    );

    testWidgets(
      'a remote session carries the machine kind its host reports for it',
      (tester) async {
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
          environment,
          open: (address, root) async =>
              FakeProject()..threadList = [threadOf('studio')],
          machineOf: (_) async => 'mac-studio',
        );

        await links.connect(studio);

        expect(launch.state!.recent.value.single.machine, 'mac-studio');
      },
    );

    testWidgets(
      'an agent host that cannot say its machine kind shows T3\'s default, server',
      (tester) async {
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
          environment,
          open: (address, root) async =>
              FakeProject()..threadList = [threadOf('studio')],
          machineOf: (_) async => throw 'an older host',
        );

        await links.connect(studio);

        expect(launch.state!.recent.value.single.machine, 'server');
      },
    );

    testWidgets(
      'coming back to the app retries a host that waits on its backoff at once',
      (tester) async {
        var opens = 0;
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
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
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
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

    testWidgets('a connection that lasted 30 s starts the retry delays again', (
      tester,
    ) async {
      var now = DateTime(2026, 11, 17);
      var opens = 0;
      final launch = launchFromEnvironment(environment);
      final links = RustAgentHostLinks(
        launch,
        environment,
        random: FixedRandom(0),
        clock: () => now,
        open: (address, root) async {
          opens++;
          return opens == 1 ? UnreachableProject() : _AnswersOnce();
        },
      );
      unawaited(links.connect(studio));
      await tester.pump(const Duration(seconds: 1));
      expect(opens, 2);
      expect(launch.state!.answers(studio), isTrue);

      now = now.add(const Duration(seconds: 31));
      await launch.state!.loadStart();
      await tester.pump(const Duration(milliseconds: 124));
      expect(opens, 2);
      await tester.pump(const Duration(milliseconds: 1));
      expect(
        opens,
        3,
        reason: 'the first loss after a stable connection is quick again',
      );
    });

    testWidgets(
      'a missing project folder is named and not retried until the app comes back',
      (tester) async {
        const environment = {'LIQUID_PROJECT': '/nonexistent/acme-api'};
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
          environment,
          open: (address, root) async => FakeProject(),
        );

        await links.connect(null);
        await tester.pump(const Duration(hours: 1));

        expect(launch.state!.answers(null), isFalse);
        expect(
          launch.state!.hostProblems.value[null],
          'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
        );
      },
    );

    testWidgets(
      'removing an agent host while it connects ends that attempt, its retries and its project, and pairing it again connects it',
      (tester) async {
        var opens = 0;
        final attempt = Completer<ProjectBackend>();
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
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
        final launch = launchFromEnvironment(environment);
        final links = RustAgentHostLinks(
          launch,
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

    test('removing an agent host removes its sessions', () async {
      final (:launch, :links) = windowFor(environment, {
        null: FakeProject(),
        studio: FakeProject()..threadList = [threadOf('studio')],
      });
      await links.connect(null);
      await links.connect(studio);

      links.disconnect(studio);
      await pumpEventQueue();

      expect(launch.state!.answers(studio), isFalse);
      expect(launch.state!.recent.value, isEmpty);
    });
  });
}
