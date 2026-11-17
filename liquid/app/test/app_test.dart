import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';

void main() {
  testWidgets('shows a launch problem in the sidebar, with the shell built', (
    tester,
  ) async {
    await tester.pumpWidget(
      LiquidApp(
        launch: ValueNotifier(
          const Launch.problem(
            'Start Liquid with LIQUID_PROJECT set to a project directory.',
          ),
        ),
      ),
    );
    expect(
      find.text('Start Liquid with LIQUID_PROJECT set to a project directory.'),
      findsOneWidget,
    );
    expect(find.byType(AgentShell), findsOneWidget);
  });
}
