import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const _usage = 'Start Liquid with LIQUID_PROJECT set to a project directory.';

void main() {
  final project = Directory.current.path;

  Future<String?> problemOf(
    Map<String, String> environment, {
    String? activeAddress,
  }) async => (await launchFromEnvironment(
    environment,
    activeAddress: activeAddress,
  )).problem;

  test('refuses a missing project', () async {
    expect(await problemOf({}), _usage);
  });

  test('says when the project directory does not exist', () async {
    expect(
      await problemOf({'LIQUID_PROJECT': '/nonexistent/acme-api'}),
      'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
    );
  });

  test('a remote agent host has the project folder, so this Mac does not look for it', () async {
    expect(
      await problemOf({
        'LIQUID_PROJECT': '/only/on/studio/acme-api',
      }, activeAddress: 'ws://studio.local:3774'),
      startsWith(
        'Liquid could not connect to the agent host ws://studio.local:3774: ',
      ),
    );
  });
  test(
    'refuses a relative Meta-Harness home, which Liquid cannot resolve',
    () async {
      expect(
        await problemOf({
          'LIQUID_PROJECT': project,
          'META_HARNESS_HOME': 'meta-data',
        }),
        'META_HARNESS_HOME must be an absolute path: meta-data',
      );
    },
  );

  test(
    'a remote agent host needs the project folder as an absolute path on it',
    () async {
      expect(
        await problemOf({
          'LIQUID_PROJECT': 'acme-api',
        }, activeAddress: 'ws://studio.local:3774'),
        'LIQUID_PROJECT must be an absolute path on the agent host ws://studio.local:3774: acme-api',
      );
    },
  );

  test(
    'refuses a relative Meta-Harness home, which Liquid cannot resolve',
    () async {
      expect(
        await problemOf({
          'LIQUID_PROJECT': project,
          'META_HARNESS_HOME': 'meta-data',
        }),
        'META_HARNESS_HOME must be an absolute path: meta-data',
      );
    },
  );

  test('shows another agent host at once, and keeps the one shown when it does not connect', () async {
    const info = SessionInfo(project: 'acme-api');
    final shown = Launch.session(SessionState(FakeProject()), info);
    final surface = ValueNotifier(shown);
    final asked = <String?>[];

    final failure = await showAgentHost(
      surface,
      (address) async {
        asked.add(address);
        return const Launch.problem('no answer');
      },
      'ws://studio.local:3774',
      isCurrent: () => true,
    );
    expect(failure, 'no answer');
    expect(surface.value, same(shown));

    final next = Launch.session(SessionState(FakeProject()), info);
    expect(
      await showAgentHost(
        surface,
        (address) async {
          asked.add(address);
          return next;
        },
        null,
        isCurrent: () => true,
      ),
      isNull,
    );
    expect(surface.value, same(next));
    expect(asked, [
      'ws://studio.local:3774',
      null,
    ], reason: 'This Mac is asked by null');
  });

  test(
    'a superseded choice whose session fails to release keeps the one shown',
    () async {
      const info = SessionInfo(project: 'acme-api');
      final shown = Launch.session(SessionState(FakeProject()), info);
      final surface = ValueNotifier(shown);
      final superseded = FakeProject()
        ..closeError = StateError('the host is gone');

      final failure = await showAgentHost(
        surface,
        (address) async => Launch.session(SessionState(superseded), info),
        'ws://studio.local:3774',
        isCurrent: () => false,
      );

      expect(failure, isNotNull);
      expect(surface.value, same(shown));
    },
  );

  test('names the project by its directory', () {
    expect(projectName('/work/acme-api/'), 'acme-api');
    expect(projectName('/'), '/');
  });
}
