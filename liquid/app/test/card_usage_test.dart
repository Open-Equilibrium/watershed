import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/rust/api/details.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

BridgeUsageWindow windowOf(String label, int remaining) =>
    BridgeUsageWindow(label: label, remainingPercent: remaining);

const usageShown = SessionCardSettings(shown: {CardItem.usage});

void main() {
  testWidgets('Usage limits are off by default', (tester) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..limits = BridgeUsageLimits(windows: [windowOf('5-hour', 42)]),
    );
    expect(find.text('5-hour'), findsNothing);
  });

  testWidgets('each window shows how much of it is left', (tester) async {
    await pumpCard(
      tester,
      session: FakeSession()
        ..limits = BridgeUsageLimits(windows: [windowOf('5-hour', 42)]),
      settings: usageShown,
    );
    expect(find.text('5-hour'), findsOneWidget);
    expect(find.text('42%'), findsOneWidget);
  });

  testWidgets('without windows, the limits say they are unavailable and why', (
    tester,
  ) async {
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

  testWidgets(
    'a window with 20 % left or less warns, and with 10 % or less is an error',
    (tester) async {
      await pumpCard(
        tester,
        session: FakeSession()
          ..limits = BridgeUsageLimits(
            windows: [windowOf('weekly', 15), windowOf('daily', 5)],
          ),
        settings: usageShown,
      );
      expect(
        tester.widget<Text>(find.text('15%')).style?.color,
        toneOf((palette) => palette.warning),
      );
      expect(
        tester.widget<Text>(find.text('5%')).style?.color,
        toneOf((palette) => palette.danger),
      );
    },
  );
}
