import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/launch.dart';

const _usage = 'Start Liquid with LIQUID_PROJECT set to a project directory.';

void main() {
  final project = Directory.current.path;

  Future<String?> problemOf(Map<String, String> environment) async =>
      (await launchFromEnvironment(environment)).problem;

  test('refuses a missing project', () async {
    expect(await problemOf({}), _usage);
  });

  test('says when the project directory does not exist', () async {
    expect(
      await problemOf({'LIQUID_PROJECT': '/nonexistent/acme-api'}),
      'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
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

  test('names the project by its directory', () {
    expect(projectName('/work/acme-api/'), 'acme-api');
    expect(projectName('/'), '/');
  });
}
