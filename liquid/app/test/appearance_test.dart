import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/appearance.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/theme/tokens.g.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

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

  Future<void> pumpApp(
    WidgetTester tester, {
    SessionState? state,
    AppearanceController? appearance,
    DiffSettingsController? diff,
    SessionCardSettingsController? cards,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    await tester.pumpWidget(
      LiquidApp(
        launch: ValueNotifier(
          Launch.session(state ?? SessionState(FakeProject()), info),
        ),
        appearance: appearance ?? AppearanceController(),
        diffSettings: diff,
        cardSettings: cards,
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> openSettings(
    WidgetTester tester,
    AppearanceController appearance, [
    DiffSettingsController? diff,
  ]) async {
    await pumpApp(tester, appearance: appearance, diff: diff);
    await tap(tester, find.byTooltip('Settings'));
  }

  testWidgets(
    'the sidebar\'s gear opens Settings, where theme and text size apply at once',
    (tester) async {
      final appearance = AppearanceController();
      await pumpApp(tester, appearance: appearance);
      double sidebar() => tester.getRect(find.byType(Sidebar)).width;
      expect(sidebar(), WsMetrics.widthSide, reason: 'the default size');
      expect(
        tester.getSize(find.byTooltip('Settings')).height,
        WsMetrics.rowSide,
        reason: 'sidebar rows at the default size',
      );

      await tap(tester, find.byTooltip('Settings'));
      expect(find.text('Appearance'), findsWidgets);
      await tap(tester, find.text('Dark'));
      expect(
        Theme.of(tester.element(find.byType(Sidebar))).brightness,
        Brightness.dark,
      );
      await tap(tester, find.text('Large'));
      expect(
        sidebar(),
        closeTo(WsMetrics.widthSide * 16 / 14, 1),
        reason: 'text and rows grow together',
      );
      expect(appearance.value.textSize, TextSize.large);
    },
  );

  testWidgets(
    'Settings → Appearance sets the thread details rows the card menu sets',
    (tester) async {
      final cards = SessionCardSettingsController();
      await pumpApp(tester, cards: cards);
      await tap(tester, find.byTooltip('Settings'));

      expect(find.text('Thread details'), findsOneWidget);
      expect(
        find.text(
          'Choose what the thread details card shows. You can also right-click the card.',
        ),
        findsOneWidget,
      );
      expect(
        find.bySemanticsLabel('Thread details: 9 of 11 on'),
        findsOneWidget,
      );
      await tap(tester, find.text('9 of 11 on'));
      expect(find.text('Show in thread details'), findsOneWidget);
      await tap(
        tester,
        find.widgetWithText(CheckboxMenuButton, 'Usage limits'),
      );

      expect(cards.value.shows(CardItem.usage), isTrue);
      expect(find.text('10 of 11 on'), findsOneWidget);
    },
  );

  testWidgets('a scaled screen keeps the keyboard inset at window size', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final body = GlobalKey();
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context)
                .copyWith(viewInsets: const EdgeInsets.only(bottom: 300)),
            child: UiScale(
              scale: 13 / 14,
              child: Scaffold(body: SizedBox.expand(key: body)),
            ),
          ),
        ),
      ),
    );
    expect(
      tester.getRect(find.byKey(body)).bottom,
      closeTo(600 - 300, 1),
      reason: 'the keyboard covers 300 window pixels whatever the text size',
    );
  });

  testWidgets('says in the status bar when settings could not be saved', (
    tester,
  ) async {
    final appearance = AppearanceController(const Appearance(), _Disk());
    final diff = DiffSettingsController(const DiffSettings(), _Disk());
    final state = SessionState(FakeProject());
    await pumpApp(tester, state: state, appearance: appearance, diff: diff);
    await appearance.change(const Appearance(theme: ThemeChoice.dark));
    await diff.change(const DiffSettings(layout: DiffLayout.split));
    await tester.pumpAndSettle();
    expect(
      [for (final notice in state.notices.value) notice.text],
      containsAll([
        'Could not save the appearance: disk full',
        'Could not save the Diff settings: disk full',
      ]),
    );
    expect(appearance.value.theme, ThemeChoice.dark, reason: 'still applies');
  });

  testWidgets(
    'a failed save is still shown after another agent host is shown',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      final appearance = AppearanceController(const Appearance(), _Disk());
      final surface = ValueNotifier(
        Launch.session(SessionState(FakeProject()), info),
      );
      await tester.pumpWidget(
        LiquidApp(launch: surface, appearance: appearance),
      );
      await tester.pumpAndSettle();
      surface.value = Launch.session(SessionState(FakeProject()), info);
      await tester.pumpAndSettle();

      await tap(tester, find.byTooltip('Settings'));
      await tap(tester, find.text('Dark'));

      expect(
        find.text('Could not save the appearance: disk full'),
        findsOneWidget,
      );
    },
  );

  testWidgets('a failed save shows one notice at the top of Settings until ✕', (
    tester,
  ) async {
    final appearance = AppearanceController(const Appearance(), _Disk());
    await openSettings(tester, appearance);
    await tap(tester, find.text('Dark'));

    final notice = find.text('Could not save the appearance: disk full');
    expect(notice, findsOneWidget);
    expect(appearance.value.theme, ThemeChoice.dark, reason: 'still applies');
    await tap(tester, find.text('Shortcuts'));
    expect(notice, findsOneWidget, reason: 'whichever section is open');

    await tap(tester, find.byTooltip('Dismiss'));
    expect(notice, findsNothing);
  });

  testWidgets('esc dismisses the notice and keeps Settings open', (
    tester,
  ) async {
    final appearance = AppearanceController(const Appearance(), _Disk());
    await openSettings(tester, appearance);
    await tap(tester, find.text('Dark'));

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.text('Could not save the appearance: disk full'), findsNothing);
    expect(find.text('Appearance'), findsWidgets, reason: 'Settings stay open');
  });

  testWidgets('the next save of the same settings clears the notice', (
    tester,
  ) async {
    final disk = _Disk();
    final appearance = AppearanceController(const Appearance(), disk);
    final diff = DiffSettingsController(
      const DiffSettings(),
      _Disk(failing: false),
    );
    await openSettings(tester, appearance, diff);
    await tap(tester, find.text('Dark'));

    final notice = find.text('Could not save the appearance: disk full');
    await diff.change(const DiffSettings(layout: DiffLayout.split));
    await tester.pumpAndSettle();
    expect(notice, findsOneWidget, reason: 'another settings saved');

    disk.failing = false;
    await tap(tester, find.text('Light'));
    expect(notice, findsNothing);
  });
}

class _Disk extends SettingsFile {
  _Disk({this.failing = true}) : super(File('unused'));

  bool failing;

  @override
  Future<void> save(Map<String, Object?> json) async {
    if (failing) throw const FileSystemException('disk full');
  }
}
