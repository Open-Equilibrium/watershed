import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeLocalServer serverOf(
  int port, {
  bool terminal = false,
  String process = 'node',
  int pid = 4242,
}) => BridgeLocalServer(
  host: 'localhost',
  port: port,
  url: 'http://localhost:$port',
  process: process,
  pid: pid,
  terminal: terminal,
);

void main() {
  testWidgets(
    'the servers the session started are listed under Servers, with who started them',
    (tester) async {
      await pumpCard(
        tester,
        session: FakeSession()
          ..serverList = [
            serverOf(3000),
            serverOf(5173, terminal: true, process: 'vite', pid: 4343),
          ],
      );
      expect(find.text('Servers'), findsOneWidget);
      expect(find.text('localhost:3000'), findsOneWidget);
      expect(find.text('Agent · node'), findsOneWidget);
      expect(find.text('localhost:5173'), findsOneWidget);
      expect(find.text('Terminal · vite'), findsOneWidget);
    },
  );

  testWidgets('the Servers section is left out while no server runs', (
    tester,
  ) async {
    await pumpCard(tester, session: FakeSession());
    expect(find.text('Servers'), findsNothing);
  });

  testWidgets('a click opens the server in the default browser', (
    tester,
  ) async {
    final opened = recordOpens();
    await pumpCard(
      tester,
      session: FakeSession()..serverList = [serverOf(3000)],
    );
    await tester.tap(find.text('localhost:3000'));
    await tester.pumpAndSettle();
    expect(opened, ['http://localhost:3000']);
  });

  testWidgets('Stop ends the server it names', (tester) async {
    final session = FakeSession()..serverList = [serverOf(3000)];
    await pumpCard(tester, session: session);
    expect(find.bySemanticsLabel('Stop localhost:3000'), findsOneWidget);
    await tester.tap(find.byTooltip('Stop server'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('stop server 4242 3000'));
  });

  testWidgets('four servers show at first, and Show N more reveals the rest', (
    tester,
  ) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..serverList = [
          for (final port in [3000, 3001, 3002, 3003, 3004]) serverOf(port),
        ],
    );
    expect(find.text('localhost:3004'), findsNothing);
    await tester.tap(find.text('Show 1 more'));
    await tester.pumpAndSettle();
    expect(find.text('localhost:3004'), findsOneWidget);
  });
}
