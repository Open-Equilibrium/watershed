import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/appearance.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  test('appearance follows the system at the default size until chosen', () {
    const fresh = Appearance();
    expect(
      (fresh.theme, fresh.textSize),
      (ThemeChoice.system, TextSize.standard),
    );
    final kept = Appearance.fromJson(
      const Appearance(
        theme: ThemeChoice.dark,
        textSize: TextSize.large,
      ).toJson(),
    );
    expect((kept.theme, kept.textSize), (ThemeChoice.dark, TextSize.large));
    expect(
      Appearance.fromJson({'theme': 'neon', 'textSize': 7}).theme,
      ThemeChoice.system,
      reason: 'what it does not know keeps the default',
    );
  });

  test('keeps the chosen appearance across restarts', () async {
    final dir = Directory.systemTemp.createTempSync('liquid-appearance');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = SettingsFile(File('${dir.path}/appearance.json'));
    final first = await AppearanceController.load(file);
    first.change(first.value.copyWith(theme: ThemeChoice.light));
    await first.change(first.value.copyWith(textSize: TextSize.compact));
    final again = await AppearanceController.load(file);
    expect(
      (again.value.theme, again.value.textSize),
      (ThemeChoice.light, TextSize.compact),
      reason: 'quick changes are saved one after another',
    );
  });

  testWidgets(
    'the sidebar\'s gear opens Settings, where theme and text size apply at once',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      final appearance = AppearanceController();
      await tester.pumpWidget(
        LiquidApp(
          launch: Launch.session(SessionState(FakeBackend()), info),
          appearance: appearance,
        ),
      );
      await tester.pumpAndSettle();
      double sidebar() => tester.getRect(find.byType(Sidebar)).width;
      expect(sidebar(), 280, reason: 'the default size');
      expect(
        tester.getSize(find.byTooltip('Settings')).height,
        32,
        reason: 'navigation rows at the default size',
      );

      await tester.tap(find.byTooltip('Settings'));
      await tester.pumpAndSettle();
      expect(find.text('Appearance'), findsWidgets);
      await tester.tap(find.text('Dark'));
      await tester.pumpAndSettle();
      expect(
        Theme.of(tester.element(find.byType(Sidebar))).brightness,
        Brightness.dark,
      );
      await tester.tap(find.text('Large'));
      await tester.pumpAndSettle();
      expect(
        sidebar(),
        closeTo(280 * 17 / 15, 1),
        reason: 'text and rows grow together',
      );
      expect(appearance.value.textSize, TextSize.large);
    },
  );

  testWidgets('says when the appearance could not be saved', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(body: AppearancePane(appearance: _Unsaved())),
      ),
    );
    await tester.tap(find.text('Dark'));
    await tester.pumpAndSettle();
    expect(
      find.text('Could not save the appearance: disk full'),
      findsOneWidget,
    );
  });
}

/// An appearance that applies but cannot be saved.
class _Unsaved extends AppearanceController {
  @override
  Future<void> change(Appearance appearance) {
    value = appearance;
    return Future.error(const FileSystemException('disk full'));
  }
}
