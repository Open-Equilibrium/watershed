import 'dart:io';

import 'package:flutter/material.dart';
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

class _Disk extends SettingsFile {
  _Disk() : super(File('unused'));

  @override
  Future<void> save(Map<String, Object?> json) async {
    throw const FileSystemException('disk full');
  }
}

AppearanceController failingAppearance() =>
    AppearanceController(const Appearance(), _Disk());

void expectOne(Finder finder) => expect(finder, findsOneWidget);
void expectNone(Finder finder) => expect(finder, findsNothing);

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

  testWidgets('the sidebar\'s gear opens Settings, where theme and text size '
      'apply at once, and the thread details rows are set', (tester) async {
    final appearance = AppearanceController();
    final cards = SessionCardSettingsController();
    await pumpApp(tester, appearance: appearance, cards: cards);
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

    expectOne(find.text('Thread details'));
    expectOne(
      find.text(
        'Choose what the thread details card shows. You can also right-click the card.',
      ),
    );
    expectOne(find.bySemanticsLabel('Thread details: 9 of 11 on'));
    await tap(tester, find.text('9 of 11 on'));
    expectOne(find.text('Show in thread details'));
    await tap(tester, find.widgetWithText(CheckboxMenuButton, 'Usage limits'));

    expect(cards.value.shows(CardItem.usage), isTrue);
    expectOne(find.text('10 of 11 on'));
  });

  testWidgets('says in the status bar when settings could not be '
      'saved', (tester) async {
    final appearance = failingAppearance();
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
}
