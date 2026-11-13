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

  test('each computer has its own folder apart from the settings', () {
    expect(
      computerDirectory({'HOME': '/Users/b'}, TargetPlatform.macOS),
      '/Users/b/Library/Caches/Liquid',
    );
    expect(
      computerDirectory({'HOME': '/home/b'}, TargetPlatform.linux),
      '/home/b/.local/state/liquid',
    );
    expect(
      computerDirectory({
        'HOME': '/home/b',
        'XDG_STATE_HOME': '/xdg',
      }, TargetPlatform.linux),
      '/xdg/liquid',
    );
    expect(
      computerDirectory({
        'LOCALAPPDATA': r'C:\Users\b\AppData\Local',
      }, TargetPlatform.windows),
      r'C:\Users\b\AppData\Local\Liquid',
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

    await shortcuts.change(ShortcutSettings.liquidKey());
    await shortcuts.change(
      ShortcutSettings.liquidKey().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyD, liquid: true),
      ),
    );
    expect(shortcuts.firstStart, isFalse);
    expect(host.calls, [true]);
    expect(await store.load(), shortcuts.value);

    await shortcuts.change(ShortcutSettings.standard());
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

    await shortcuts.change(ShortcutSettings.liquidKey());
    await shortcuts.change(ShortcutSettings.liquidKey());
    expect(shortcuts.value, ShortcutSettings.standard());
    expect(await store.load(), ShortcutSettings.standard());
    expect(reported, [
      'Could not turn the Liquid key on: denied',
      'Could not turn the Liquid key on: denied',
    ], reason: 'every failure is reported');
  });

  test(
    'a stored Liquid key stays off where the Liquid key does not exist',
    () async {
      final store = ShortcutStore(file);
      await store.save(ShortcutSettings.liquidKey());
      final host = FakeHost(available: false);

      final shortcuts = await ShortcutController.load(store, host);
      expect(shortcuts.value.liquidKey, isFalse);
      expect(shortcuts.value.label(Command.newSession), '⌘N');
      expect(host.calls, isEmpty);
    },
  );

  test(
    'a kept Custom Liquid key stays off where the Liquid key does not exist',
    () async {
      final store = ShortcutStore(file);
      await store.save(
        ShortcutSettings.standard()
            .withLiquidKey(true)
            .choose(Profile.standard),
      );
      final host = FakeHost(available: false);
      final shortcuts = await ShortcutController.load(store, host);

      await shortcuts.change(shortcuts.value.choose(Profile.custom));
      expect(shortcuts.value.liquidKey, isFalse);
      expect(shortcuts.value.label(Command.newSession), '⌘N');
      expect((await store.load())!.liquidKey, isFalse);
      expect(host.calls, isEmpty);
    },
  );

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

      await shortcuts.change(custom.choose(Profile.liquidKey));
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

      final first = shortcuts.change(ShortcutSettings.liquidKey());
      final second = shortcuts.change(
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

  test(
    'a Liquid key that cannot be turned off stays on in the profile',
    () async {
      final host = FakeHost();
      final store = ShortcutStore(file);
      final shortcuts = await ShortcutController.load(store, host);
      final reported = <String>[];
      shortcuts.report = reported.add;
      await shortcuts.change(ShortcutSettings.liquidKey());

      host.failure = PlatformException(code: 'disable', message: 'refused');
      await shortcuts.change(ShortcutSettings.standard());
      expect(shortcuts.value, ShortcutSettings.liquidKey());
      expect(await store.load(), ShortcutSettings.liquidKey());
      expect(reported, ['Could not turn the Liquid key off: refused']);
      expect(
        shortcuts.liquidKeyProblem.value,
        reported.single,
        reason: 'Settings say why the choice did not hold',
      );

      host.failure = null;
      await shortcuts.change(ShortcutSettings.standard());
      expect(host.calls, [true, false, false], reason: 'it tries again');
      expect(shortcuts.value, ShortcutSettings.standard());
    },
  );

  test(
    'a stored Liquid key stays off where the Liquid key does not exist',
    () async {
      final store = ShortcutStore(file);
      await store.save(ShortcutSettings.liquidKey());

      final shortcuts = await ShortcutController.load(
        store,
        FakeHost(available: false),
      );
      expect(shortcuts.value.liquidKey, isFalse);
      expect(shortcuts.value.profile, Profile.standard);
    },
  );

  test('a Standard chosen while the Liquid key turns on keeps it on when turning it off fails', () async {
    final enable = Completer<void>();
    final host = FakeHost()..gate = enable;
    final store = ShortcutStore(file);
    final shortcuts = await ShortcutController.load(store, host);

    final on = shortcuts.change(ShortcutSettings.liquidKey());
    await pumpEventQueue();
    // The disable waits on a gate of its own, so the test can fail it after the enable.
    final disable = Completer<void>();
    host.gate = disable;
    final off = shortcuts.change(ShortcutSettings.standard());
    enable.complete();
    await pumpEventQueue();
    host.failure = PlatformException(code: 'approval', message: 'denied');
    disable.complete();
    await Future.wait([on, off]);

    expect(host.calls, [true, false]);
    expect(shortcuts.value.liquidKey, isTrue, reason: 'the host still has it');
    expect((await store.load())!.liquidKey, isTrue);
    expect(shortcuts.liquidKeyProblem.value, isNotNull);
  });

  test('a failed Liquid key turn-off keeps a change made meanwhile', () async {
    final host = FakeHost();
    final store = ShortcutStore(file);
    final shortcuts = await ShortcutController.load(store, host);
    await shortcuts.change(ShortcutSettings.liquidKey());

    final disable = Completer<void>();
    host.gate = disable;
    final off = shortcuts.change(ShortcutSettings.standard());
    await pumpEventQueue();
    // While the host turns the key off, the user keeps it on and gives Diff a key.
    final diff = ShortcutSettings.liquidKey().change(
      Command.diff,
      const Keys(LogicalKeyboardKey.keyD, liquid: true),
    );
    final kept = shortcuts.change(diff);
    host.failure = PlatformException(code: 'approval', message: 'denied');
    disable.complete();
    await Future.wait([off, kept]);

    expect(shortcuts.value, diff);
    expect(await store.load(), diff);
  });

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
    // The file is written in the background, outside the event queue.
    var saved = await store.load();
    for (
      var tries = 0;
      saved != ShortcutSettings.standard() && tries < 200;
      tries++
    ) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
      saved = await store.load();
    }
    expect(saved, ShortcutSettings.standard());
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
