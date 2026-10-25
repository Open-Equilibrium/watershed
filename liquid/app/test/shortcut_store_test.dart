import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';

/// Records what the app asks of the Liquid key.
class FakeHost implements LiquidKeyHost {
  FakeHost({this.available = true});

  @override
  final bool available;
  final calls = <bool>[];
  Object? failure;

  @override
  Future<void> apply(bool on) async {
    calls.add(on);
    if (failure case final failure?) throw failure;
  }
}

void main() {
  late Directory temp;
  late File file;

  setUp(() {
    temp = Directory.systemTemp.createTempSync('shortcuts');
    file = File('${temp.path}/settings/shortcuts.json');
  });
  tearDown(() => temp.deleteSync(recursive: true));

  test('settings live in the platform app data folder', () {
    expect(
      settingsDirectory({'HOME': '/Users/b'}, TargetPlatform.macOS),
      '/Users/b/Library/Application Support/Liquid',
    );
    expect(
      settingsDirectory({'HOME': '/home/b'}, TargetPlatform.linux),
      '/home/b/.config/liquid',
    );
    expect(
      settingsDirectory({
        'HOME': '/home/b',
        'XDG_CONFIG_HOME': '/xdg',
      }, TargetPlatform.linux),
      '/xdg/liquid',
    );
    expect(
      settingsDirectory({
        'APPDATA': r'C:\Users\b\AppData\Roaming',
      }, TargetPlatform.windows),
      r'C:\Users\b\AppData\Roaming\Liquid',
    );
  });

  test(
    'the store keeps settings as JSON and reads nothing it cannot trust',
    () async {
      final store = ShortcutStore(file);
      expect(await store.load(), isNull);

      final settings = ShortcutSettings.liquidKey().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyD, liquid: true),
      );
      await store.save(settings);
      expect(await store.load(), settings);
      expect(jsonDecode(file.readAsStringSync()), settings.toJson());

      file.writeAsStringSync('{not json');
      expect(await store.load(), isNull);
      file.writeAsStringSync(' ' * (64 * 1024) + jsonEncode(settings.toJson()));
      expect(await store.load(), isNull);
    },
  );

  test(
    'the first start asks for a profile only where the Liquid key exists',
    () async {
      final store = ShortcutStore(file);
      final mac = await ShortcutController.load(store, FakeHost());
      expect(mac.firstStart, isTrue);
      expect(mac.value, ShortcutSettings.standard());

      final other = await ShortcutController.load(
        store,
        FakeHost(available: false),
      );
      expect(other.firstStart, isFalse);
    },
  );

  test('choosing the Liquid key saves it and turns it on once', () async {
    final host = FakeHost();
    final store = ShortcutStore(file);
    final shortcuts = await ShortcutController.load(store, host);

    await shortcuts.update(ShortcutSettings.liquidKey());
    await shortcuts.update(
      ShortcutSettings.liquidKey().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyD, liquid: true),
      ),
    );
    expect(shortcuts.firstStart, isFalse);
    expect(host.calls, [true]);
    expect(await store.load(), shortcuts.value);

    await shortcuts.update(ShortcutSettings.standard());
    expect(host.calls, [true, false]);
  });

  test('a stored Liquid key is turned on again at start', () async {
    final store = ShortcutStore(file);
    await store.save(ShortcutSettings.liquidKey());
    final host = FakeHost();

    final shortcuts = await ShortcutController.load(store, host);
    expect(shortcuts.firstStart, isFalse);
    expect(shortcuts.value, ShortcutSettings.liquidKey());
    expect(host.calls, [true]);
  });

  test('a host that cannot turn the Liquid key on is reported', () async {
    final host = FakeHost()
      ..failure = PlatformException(code: 'register', message: 'denied');
    final shortcuts = await ShortcutController.load(ShortcutStore(file), host);

    await shortcuts.update(ShortcutSettings.liquidKey());
    expect(shortcuts.problem.value, contains('denied'));
  });
}
