import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/settings_file.dart';

void main() {
  test('a failed save leaves no temporary file', () async {
    final dir = Directory.systemTemp.createTempSync('settings-file');
    addTearDown(() => dir.deleteSync(recursive: true));
    // A directory where the settings file belongs makes the rename fail after the write.
    final path = '${dir.path}/appearance.json';
    Directory(path).createSync();
    final settings = SettingsFile(File(path));

    for (var attempt = 0; attempt < 2; attempt++) {
      await expectLater(
        settings.save({'theme': 'dark'}),
        throwsA(isA<FileSystemException>()),
      );
    }
    expect(
      dir.listSync().where((entry) => entry.path.endsWith('.pending')),
      isEmpty,
    );
  });

  test(
    'a save that fails with any error still lets later saves write',
    () async {
      final written = <int>[];
      final settings = SavedSettings<int>(0, (value) async {
        if (value == 1) throw StateError('refused');
        written.add(value);
      }, 'the test settings');
      final outcomes = <String?>[];
      settings.saveOutcome = outcomes.add;

      await settings.change(1);
      await settings.change(2);

      expect(written, [2]);
      expect(outcomes.first, startsWith('Could not save the test settings: '));
      expect(outcomes.last, isNull);
    },
  );
}
