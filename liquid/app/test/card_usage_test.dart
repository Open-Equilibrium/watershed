import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeUsageWindow windowOf(String label, int remaining) =>
    BridgeUsageWindow(label: label, remainingPercent: remaining);

const usageShown = SessionCardSettings(shown: {CardItem.usage});

void main() {
  testWidgets('usage is off until shown', (tester) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..limits = BridgeUsageLimits(windows: [windowOf('5-hour', 42)]),
    );
    expect(find.text('5-hour'), findsNothing);

    await pumpCard(
      tester,
      session: FakeSession()
        ..limits = BridgeUsageLimits(
          windows: [windowOf('weekly', 15), windowOf('daily', 5)],
        ),
      settings: usageShown,
    );
    expect(find.text('weekly'), findsOneWidget);
    expect(find.text('daily'), findsOneWidget);
    await pumpCard(
      tester,
      session: FakeSession()
        ..limits = const BridgeUsageLimits(
          notice: 'This account has no subscription limits.',
          windows: [],
        ),
      settings: usageShown,
    );
    expect(find.text('Limits'), findsOneWidget);
    expect(find.text('Unavailable'), findsOneWidget);
  });
}
