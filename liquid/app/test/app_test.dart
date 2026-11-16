import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';

void main() {
  testWidgets('shows a launch problem in place of the shell', (tester) async {
    await tester.pumpWidget(
      const LiquidApp(
        launch: Launch.problem('Start Liquid with LIQUID_PROJECT set to a project directory.'),
      ),
    );
    expect(
      find.text('Start Liquid with LIQUID_PROJECT set to a project directory.'),
      findsOneWidget,
    );
    expect(find.byType(AgentShell), findsNothing);
  });
}
