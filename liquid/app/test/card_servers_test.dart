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
  testWidgets('a click opens a server, and Stop ends it', (tester) async {
    final opened = recordOpens();
    final session = FakeSession()..serverList = [serverOf(3000)];
    await pumpCard(tester, session: session);
    await tester.tap(find.text('localhost:3000'));
    await tester.pumpAndSettle();
    expect(opened, ['http://localhost:3000']);

    expect(find.bySemanticsLabel('Stop localhost:3000'), findsOneWidget);
    await tester.tap(find.byTooltip('Stop server'));
    await tester.pumpAndSettle();
    expect(session.calls, contains('stop server 4242 3000'));
  });
}
