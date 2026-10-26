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

  test('reopens a stored pi session when one is named', () {
    expect(piArguments('/w/watershed.ts', null), [
      '--mode',
      'rpc',
      '-e',
      '/w/watershed.ts',
    ]);
    expect(piArguments('/w/watershed.ts', '/s/session.jsonl'), [
      '--mode',
      'rpc',
      '-e',
      '/w/watershed.ts',
      '--session',
      '/s/session.jsonl',
    ]);
  });
}
