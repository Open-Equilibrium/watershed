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

  test('keeps the Diff panel settings across a restart', () async {
    final first = await DiffSettingsController.load(SettingsFile(file));
    expect(first.value.layout, DiffLayout.unified);
    expect(first.value.hideWhitespace, isFalse);
    expect(first.value.width, isNull, reason: 'unset until dragged');
    await first.change(
      const DiffSettings(
        layout: DiffLayout.split,
        hideWhitespace: true,
        width: 640,
      ),
    );

    final again = await DiffSettingsController.load(SettingsFile(file));
    expect(again.value.layout, DiffLayout.split);
    expect(again.value.hideWhitespace, isTrue);
    expect(again.value.width, 640);
  });
}
