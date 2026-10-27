import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';

void main() {
  testWidgets('explains a missing launch configuration', (tester) async {
    final launch = await launchFromEnvironment({'LIQUID_PROJECT': '/tmp'});
    await tester.pumpWidget(LiquidApp(launch: launch));
    expect(find.textContaining('LIQUID_PI_EXTENSION'), findsOneWidget);
    expect(find.byType(TextField), findsNothing);
  });

  test('refuses an extension path the project could supply', () async {
    // pi reads a relative path inside the project it works in.
    final launch = await launchFromEnvironment({
      'LIQUID_PROJECT': '/tmp',
      'LIQUID_PI_EXTENSION': 'meta-harness/pi-extension/watershed.ts',
      'LIQUID_PI': '/nonexistent/pi',
    });
    expect(launch.problem, contains('absolute path'));
  });

  test('refuses a program or session path the project could supply', () async {
    for (final (name, value) in [
      ('LIQUID_PI', 'node_modules/.bin/pi'),
      ('LIQUID_PI_SESSION', 'sessions/old.jsonl'),
    ]) {
      final launch = await launchFromEnvironment({
        'LIQUID_PROJECT': '/tmp',
        'LIQUID_PI_EXTENSION': '/w/watershed.ts',
        name: value,
      });
      expect(
        launch.problem,
        '$name must be an absolute path: $value',
        reason: name,
      );
    }
  });

  test('says when the project directory does not exist', () async {
    final launch = await launchFromEnvironment({
      'LIQUID_PROJECT': '/nonexistent/acme-api',
      'LIQUID_PI_EXTENSION': '/w/watershed.ts',
    });
    expect(
      launch.problem,
      'LIQUID_PROJECT names no directory: /nonexistent/acme-api',
    );
  });

  test('names the project by its directory', () {
    expect(projectName('/work/acme-api/'), 'acme-api');
    expect(projectName('/'), '/');
    expect(
      projectName('.'),
      Directory.current.uri.pathSegments.lastWhere((part) => part.isNotEmpty),
    );
  });
}
