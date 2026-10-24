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
}
