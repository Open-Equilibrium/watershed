import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/settings_file.dart';

void main() {
  late Directory temp;
  late File file;

  setUp(() {
    temp = Directory.systemTemp.createTempSync('session-card-settings');
    file = File('${temp.path}/settings/session_card.json');
  });
  tearDown(() => temp.deleteSync(recursive: true));

  test('keeps the rows hidden across a restart', () async {
    final first = await SessionCardSettingsController.load(SettingsFile(file));
    expect(
      first.value.shows(CardItem.changes),
      isTrue,
      reason: 'all shown by default',
    );
    await first.change(first.value.withItem(CardItem.changes, false));

    final again = await SessionCardSettingsController.load(SettingsFile(file));
    expect(again.value.shows(CardItem.changes), isFalse);
    expect(again.value.shows(CardItem.branch), isTrue);
  });

  test('stores only the rows that differ from the default', () async {
    final settings = await SessionCardSettingsController.load(
      SettingsFile(file),
    );
    await settings.change(
      settings.value
          .withItem(CardItem.branch, false)
          .withItem(CardItem.branch, true)
          .withItem(CardItem.context, false),
    );
    expect(
      SessionCardSettings.fromJson({
        'hidden': ['context'],
      }).toJson(),
      {
        'hidden': ['context'],
      },
    );
    expect(settings.value.toJson(), {
      'hidden': ['context'],
    });
  });

  test('a reset to default clears every choice', () async {
    final settings = await SessionCardSettingsController.load(
      SettingsFile(file),
    );
    await settings.change(settings.value.withItem(CardItem.gitActions, false));
    await settings.change(const SessionCardSettings());
    expect(settings.value.isDefault, isTrue);
    expect(settings.value.toJson(), {'hidden': <String>[]});
  });

  test('ignores what it does not know, and keeps the ids unchanged', () {
    final settings = SessionCardSettings.fromJson({
      'hidden': ['changes', 'sideways', 7],
    });
    expect(settings.shows(CardItem.changes), isFalse);
    expect(settings.hidden, {CardItem.changes});
    expect(CardItem.values.map((item) => item.id), [
      'branch',
      'gitActions',
      'changes',
      'figures',
      'context',
    ]);
  });
}
