import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../settings_file.dart';
import 'keymap.dart';

String? _xdg(Map<String, String> environment, String name) {
  final value = environment[name];
  return value == null || value.isEmpty ? null : value;
}

String settingsDirectory(
  Map<String, String> environment,
  TargetPlatform platform,
) => switch (platform) {
  TargetPlatform.macOS =>
    '${environment['HOME']}/Library/Application Support/Liquid',
  TargetPlatform.windows => '${environment['APPDATA']}\\Liquid',
  _ =>
    '${_xdg(environment, 'XDG_CONFIG_HOME') ?? '${environment['HOME']}/.config'}/liquid',
};

class ShortcutStore {
  ShortcutStore(File file) : _file = SettingsFile(file);

  final SettingsFile _file;

  Future<ShortcutSettings?> load() async {
    final json = await _file.load();
    if (json == null) return null;
    try {
      return ShortcutSettings.fromJson(json);
    } on FormatException {
      return null;
    }
  }

  Future<void> save(ShortcutSettings settings) => _file.save(settings.toJson());
}

abstract interface class LiquidKeyHost {
  bool get available;

  Future<void> apply(bool on);
}

bool liquidKeySupported(TargetPlatform platform, String version) {
  if (platform != TargetPlatform.macOS) return false;
  final major = RegExp(r'(\d+)\.').firstMatch(version)?.group(1);
  return (int.tryParse(major ?? '') ?? 0) >= 13;
}

class ChannelLiquidKeyHost implements LiquidKeyHost {
  const ChannelLiquidKeyHost();

  static const _channel = MethodChannel('liquid/liquid_key');

  @override
  bool get available => liquidKeySupported(
    defaultTargetPlatform,
    Platform.operatingSystemVersion,
  );

  @override
  Future<void> apply(bool on) =>
      _channel.invokeMethod<void>(on ? 'enable' : 'disable');
}

class ShortcutController extends SavedSettings<ShortcutSettings> {
  ShortcutController._(
    ShortcutSettings value,
    ShortcutStore? store,
    this._host,
    this.firstStart,
  ) : _settled = value,
      super(value, store?.save, 'the shortcuts');

  ShortcutController.memory([ShortcutSettings? settings])
    : this._(settings ?? ShortcutSettings.standard(), null, null, false);

  static Future<ShortcutController> load(
    ShortcutStore store,
    LiquidKeyHost host,
  ) async {
    final stored = await store.load();
    final shortcuts = ShortcutController._(
      switch (stored) {
        null => ShortcutSettings.standard(),
        _ when !host.available => stored.withoutLiquidKey(),
        _ => stored,
      },
      store,
      host,
      stored == null && host.available,
    );
    if (shortcuts.value.liquidKey) {
      unawaited(shortcuts._sync(store: false));
    }
    return shortcuts;
  }

  final LiquidKeyHost? _host;

  bool firstStart;

  bool get liquidKeyAvailable => _host?.available ?? false;

  bool _applied = false;

  ShortcutSettings _settled;
  Future<void> _pending = Future.value();

  @override
  Future<void> change(ShortcutSettings next) {
    value = liquidKeyAvailable || _host == null
        ? next
        : next.withoutLiquidKey();
    firstStart = false;
    liquidKeyProblem.value = null;
    return _sync(store: true);
  }

  Future<void> _sync({required bool store}) =>
      _pending = _pending.then((_) async {
        final next = value;
        final want = next.liquidKey;
        var changed = false;
        if (want != _applied) {
          if (await _apply(want)) {
            _applied = want;
          } else {
            if (identical(value, next)) {
              value = want ? _withoutLiquidKey(value) : _settled;
            }
            changed = true;
          }
        }
        if (value.liquidKey == _applied) {
          _settled = value;
        } else if (next.liquidKey == _applied) {
          _settled = next;
        }
        if (store || changed) await save();
      });

  ShortcutSettings _withoutLiquidKey(ShortcutSettings settings) {
    final before = _settled.profile;
    final back =
        settings.profile == Profile.liquidKey && before != Profile.liquidKey
        ? settings.choose(before)
        : settings;
    return back.withoutLiquidKey();
  }

  Future<bool> _apply(bool on) async {
    final host = _host;
    if (host == null || !host.available) return true;
    try {
      await host.apply(on);
      return true;
    } on PlatformException catch (error) {
      _failed(on, error.message ?? error.code);
    } on MissingPluginException {
      _failed(on, 'this build has no Liquid key');
    }
    return false;
  }

  void _failed(bool on, String reason) {
    final problem =
        'Could not turn the Liquid key ${on ? 'on' : 'off'}: $reason';
    liquidKeyProblem.value = problem;
    this.problem(problem);
  }

  final liquidKeyProblem = ValueNotifier<String?>(null);

  @override
  void dispose() {
    liquidKeyProblem.dispose();
    super.dispose();
  }
}
