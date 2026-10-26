import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';

import 'fake_host.dart';

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
    await pumpEventQueue();
    expect(shortcuts.firstStart, isFalse);
    expect(shortcuts.value, ShortcutSettings.liquidKey());
    expect(host.calls, [true]);
  });

  test('a Liquid key that cannot be turned on is reported and leaves the profile as it was', () async {
    final host = FakeHost()
      ..failure = PlatformException(code: 'approval', message: 'denied');
    final store = ShortcutStore(file);
    final shortcuts = await ShortcutController.load(store, host);
    final reported = <String>[];
    shortcuts.report = reported.add;

    await shortcuts.update(ShortcutSettings.liquidKey());
    await shortcuts.update(ShortcutSettings.liquidKey());
    expect(shortcuts.value, ShortcutSettings.standard());
    expect(await store.load(), ShortcutSettings.standard());
    expect(reported, [
      'Could not turn the Liquid key on: denied',
      'Could not turn the Liquid key on: denied',
    ], reason: 'every failure is reported');
  });

  test(
    'choosing the Liquid key profile keeps the previous profile when it fails',
    () async {
      final custom = ShortcutSettings.standard().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyD, meta: true),
      );
      final store = ShortcutStore(file);
      await store.save(custom);
      final host = FakeHost()
        ..failure = PlatformException(code: 'approval', message: 'denied');
      final shortcuts = await ShortcutController.load(store, host);

      await shortcuts.update(custom.choose(Profile.liquidKey));
      expect(shortcuts.value, custom, reason: 'Custom stays, not Standard');
      expect(await store.load(), custom);
    },
  );

  test(
    'a change queued behind a failing Liquid key leaves the key off',
    () async {
      final host = FakeHost()
        ..failure = PlatformException(code: 'approval', message: 'denied')
        ..gate = Completer();
      final store = ShortcutStore(file);
      final shortcuts = await ShortcutController.load(store, host);

      final first = shortcuts.update(ShortcutSettings.liquidKey());
      final second = shortcuts.update(
        ShortcutSettings.liquidKey().change(
          Command.diff,
          const Keys(LogicalKeyboardKey.keyD, liquid: true),
        ),
      );
      host.gate!.complete();
      await Future.wait([first, second]);
      expect(shortcuts.value.liquidKey, isFalse);
      expect((await store.load())!.liquidKey, isFalse);
      expect(
        shortcuts.value.changed(Command.diff),
        isTrue,
        reason: 'the change itself stays',
      );
    },
  );

  test('the first frame does not wait for the Liquid key', () async {
    final store = ShortcutStore(file);
    await store.save(ShortcutSettings.liquidKey());
    final host = FakeHost()..gate = Completer();

    final shortcuts = await ShortcutController.load(
      store,
      host,
    ).timeout(const Duration(seconds: 5));
    expect(shortcuts.value, ShortcutSettings.liquidKey());
    await pumpEventQueue();
    expect(host.calls, [true]);
    host.gate!.complete();
  });

  test('a problem at start is reported once someone listens', () async {
    final store = ShortcutStore(file);
    await store.save(ShortcutSettings.liquidKey());
    final host = FakeHost()
      ..failure = PlatformException(code: 'approval', message: 'denied');

    final shortcuts = await ShortcutController.load(store, host);
    await pumpEventQueue();
    final reported = <String>[];
    shortcuts.report = reported.add;
    expect(reported, ['Could not turn the Liquid key on: denied']);
    expect(
      shortcuts.value,
      ShortcutSettings.standard(),
      reason: 'hints stay visible without the Liquid key',
    );
    expect(await store.load(), ShortcutSettings.standard());
  });

  test('the Liquid key needs macOS 13 for its login item', () {
    expect(
      liquidKeySupported(TargetPlatform.macOS, 'Version 27.0 (Build 26A428)'),
      isTrue,
    );
    expect(
      liquidKeySupported(
        TargetPlatform.macOS,
        'Version 12.7.6 (Build 21H1320)',
      ),
      isFalse,
    );
    expect(liquidKeySupported(TargetPlatform.linux, '6.8.0'), isFalse);
  });
}
