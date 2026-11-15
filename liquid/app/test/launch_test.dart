import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/launch.dart';

const _usage =
    'Start Liquid with LIQUID_PROJECT set to a project directory and LIQUID_T3 set to the '
    'absolute path of the pinned t3 executable.';

void main() {
  final project = Directory.current.path;

  Future<String?> problemOf(Map<String, String> environment) async =>
      (await launchFromEnvironment(
        environment,
        defaultHome: '/Users/me/.t3',
      )).problem;

  test('refuses a missing or relative T3 program or project', () async {
    for (final environment in [
      {'LIQUID_PROJECT': project},
      {'LIQUID_T3': '/opt/t3/bin/t3'},
      {'LIQUID_PROJECT': project, 'LIQUID_T3': 'bin/t3'},
    ]) {
      expect(await problemOf(environment), _usage, reason: '$environment');
    }
  });

  test('says when the project directory does not exist', () async {
    expect(
      await problemOf({
        'LIQUID_PROJECT': '/nonexistent/acme-api',
        'LIQUID_T3': '/opt/t3/bin/t3',
      }),
      'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
    );
  });

  test('refuses a relative T3 home, which Liquid cannot resolve', () async {
    expect(
      await problemOf({
        'LIQUID_PROJECT': project,
        'LIQUID_T3': '/opt/t3/bin/t3',
        'LIQUID_T3_HOME': 't3-data',
      }),
      'LIQUID_T3_HOME must be an absolute path: t3-data',
    );
  });

  test('names the project by its directory', () {
    expect(projectName('/work/acme-api/'), 'acme-api');
    expect(projectName('/'), '/');
  });
}
