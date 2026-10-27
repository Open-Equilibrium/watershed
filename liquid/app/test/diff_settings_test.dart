import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/settings_file.dart';

void main() {
  late Directory temp;
  late File file;

  setUp(() {
    temp = Directory.systemTemp.createTempSync('diff-settings');
    file = File('${temp.path}/settings/diff.json');
  });
  tearDown(() => temp.deleteSync(recursive: true));

  test('keeps the Diff tab settings across a restart', () async {
    final first = await DiffSettingsController.load(SettingsFile(file));
    expect(first.value.layout, DiffLayout.unified);
    expect(first.value.hideWhitespace, isFalse);
    await first.change(
      const DiffSettings(layout: DiffLayout.split, hideWhitespace: true),
    );

    final again = await DiffSettingsController.load(SettingsFile(file));
    expect(again.value.layout, DiffLayout.split);
    expect(again.value.hideWhitespace, isTrue);
  });

  test(
    'starts from the defaults when the file says nothing it knows',
    () async {
      file.parent.createSync(recursive: true);
      file.writeAsStringSync('{"layout": "sideways", "hideWhitespace": "yes"}');
      final settings = await DiffSettingsController.load(SettingsFile(file));
      expect(settings.value.layout, DiffLayout.unified);
      expect(settings.value.hideWhitespace, isFalse);
    },
  );

  test('saves quick changes one after another, keeping the last', () async {
    final settings = await DiffSettingsController.load(SettingsFile(file));
    for (var round = 0; round < 20; round++) {
      await Future.wait([
        settings.change(const DiffSettings(layout: DiffLayout.split)),
        settings.change(const DiffSettings(hideWhitespace: true)),
      ]);
      final kept = await DiffSettingsController.load(SettingsFile(file));
      expect(kept.value.layout, DiffLayout.unified);
      expect(kept.value.hideWhitespace, isTrue);
    }
  });
}
