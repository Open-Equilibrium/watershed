import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/settings_file.dart';

void main() {
  late Directory temp;
  late File file;

  Future<SessionCardSettingsController> load() =>
      SessionCardSettingsController.load(SettingsFile(file));

  setUp(() {
    temp = Directory.systemTemp.createTempSync('session-card-settings');
    file = File('${temp.path}/settings/session_card.json');
  });
  tearDown(() => temp.deleteSync(recursive: true));

  test('keeps the rows hidden across a restart', () async {
    final first = await load();
    expect(
      first.value.shows(CardItem.changes),
      isTrue,
      reason: 'all shown by default',
    );
    await first.change(first.value.withItem(CardItem.changes, false));

    final again = await load();
    expect(again.value.shows(CardItem.changes), isFalse);
    expect(again.value.shows(CardItem.branch), isTrue);
  });

  test('stores only the rows that differ from their default', () async {
    final settings = await load();
    await settings.change(
      settings.value
          .withItem(CardItem.changes, false)
          .withItem(CardItem.branch, false)
          .withItem(CardItem.branch, true)
          .withItem(CardItem.usage, true)
          .withItem(CardItem.context, false),
    );
    expect(settings.value.toJson(), {
      'hidden': ['changes'],
      'shown': ['usage'],
    });
  });

  test('a reset to default clears every choice', () async {
    final settings = await load();
    await settings.change(settings.value.withItem(CardItem.gitActions, false));
    await settings.change(const SessionCardSettings());
    expect(settings.value.isDefault, isTrue);
    expect(settings.value.toJson(), {
      'hidden': <String>[],
      'shown': <String>[],
    });
  });

  test('Usage limits and Context window are off until they are shown', () {
    const defaults = SessionCardSettings();
    expect(defaults.shows(CardItem.usage), isFalse);
    expect(defaults.shows(CardItem.context), isFalse);
    expect(defaults.shows(CardItem.servers), isTrue);
    expect(defaults.shows(CardItem.statistics), isTrue);
  });

  test('keeps the stored ids, their order and the rows a file names', () {
    final settings = SessionCardSettings.fromJson({
      'hidden': ['changes', 'sideways', 7, 'usage'],
      'shown': ['context', 'changes'],
    });
    expect(settings.shows(CardItem.changes), isFalse);
    expect(settings.shows(CardItem.usage), isFalse);
    expect(settings.shows(CardItem.context), isTrue);
    expect(
      CardItem.values.map((item) => item.id).join(' '),
      'workspace branch gitActions changes servers automations lineage sideChats figures usage context',
    );
  });
}
