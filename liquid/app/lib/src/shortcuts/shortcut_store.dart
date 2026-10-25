import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'keymap.dart';

/// The folder for Liquid's settings in the platform's app data location.
String settingsDirectory(
  Map<String, String> environment,
  TargetPlatform platform,
) => switch (platform) {
  TargetPlatform.macOS =>
    '${environment['HOME']}/Library/Application Support/Liquid',
  TargetPlatform.windows => '${environment['APPDATA']}\\Liquid',
  _ =>
    '${environment['XDG_CONFIG_HOME'] ?? '${environment['HOME']}/.config'}/liquid',
};

/// Shortcut settings as a JSON file (ADR-0205).
class ShortcutStore {
  ShortcutStore(this.file);

  final File file;

  /// Far above any real settings; a larger file is not Liquid's.
  static const _limit = 64 * 1024;

  /// The stored settings, or null before the first choice or when the file is unreadable.
  Future<ShortcutSettings?> load() async {
    try {
      if (await file.length() > _limit) return null;
      final json = jsonDecode(await file.readAsString());
      return json is Map<String, Object?>
          ? ShortcutSettings.fromJson(json)
          : null;
    } on FileSystemException {
      return null;
    } on FormatException {
      return null;
    }
  }

  /// Replaces the file whole, so a crash leaves the old or the new settings.
  Future<void> save(ShortcutSettings settings) async {
    await file.parent.create(recursive: true);
    final pending = File('${file.path}.pending');
    await pending.writeAsString(jsonEncode(settings.toJson()), flush: true);
    await pending.rename(file.path);
  }
}

/// Turns the Liquid key on or off on this computer (ADR-0205).
abstract interface class LiquidKeyHost {
  /// Whether this computer can have the Liquid key.
  bool get available;

  Future<void> apply(bool on);
}

/// Whether `platform` at `version`, as [Platform.operatingSystemVersion] reports it, has the
/// Liquid key: macOS 13 or later, which can register its login item. macOS comes first.
bool liquidKeySupported(TargetPlatform platform, String version) {
  if (platform != TargetPlatform.macOS) return false;
  final major = RegExp(r'(\d+)\.').firstMatch(version)?.group(1);
  return (int.tryParse(major ?? '') ?? 0) >= 13;
}

/// The macOS runner's Liquid key: it remaps Caps Lock and registers the login item that
/// listens for it.
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

/// The shortcut settings in use; changes are applied and saved in order.
class ShortcutController extends ValueNotifier<ShortcutSettings> {
  ShortcutController._(super.value, this._store, this._host, this.firstStart);

  /// Settings that are neither stored nor applied, for a Liquid without its app data.
  ShortcutController.memory([ShortcutSettings? settings])
    : this._(settings ?? ShortcutSettings.standard(), null, null, false);

  /// The stored settings. A stored Liquid key is turned on again, since a restart or a newly
  /// attached keyboard can lose its remapping; the first frame does not wait for it.
  static Future<ShortcutController> load(
    ShortcutStore store,
    LiquidKeyHost host,
  ) async {
    final stored = await store.load();
    final shortcuts = ShortcutController._(
      stored ?? ShortcutSettings.standard(),
      store,
      host,
      stored == null && host.available,
    );
    if (stored != null && stored.liquidKey && host.available) {
      unawaited(shortcuts._apply(true));
    }
    return shortcuts;
  }

  final ShortcutStore? _store;
  final LiquidKeyHost? _host;

  /// Whether the user has not chosen a profile yet.
  bool firstStart;

  bool get liquidKeyAvailable => _host?.available ?? false;

  /// Receives what goes wrong saving or applying shortcuts; problems wait until it is set.
  set report(void Function(String problem)? report) {
    _report = report;
    if (report == null) return;
    for (final problem in _unreported) {
      report(problem);
    }
    _unreported.clear();
  }

  void Function(String problem)? _report;
  final _unreported = <String>[];

  void _problem(String problem) {
    final report = _report;
    if (report == null) {
      _unreported.add(problem);
    } else {
      report(problem);
    }
  }

  Future<void> _pending = Future.value();

  Future<void> update(ShortcutSettings next) {
    final before = value;
    value = next;
    firstStart = false;
    return _pending = _pending.then((_) async {
      var saved = next;
      if (before.liquidKey != next.liquidKey &&
          !await _apply(next.liquidKey) &&
          next.liquidKey) {
        // Without the Liquid key, its profile would hide every shortcut hint.
        saved = before;
        if (value == next) value = before;
      }
      try {
        await _store?.save(saved);
      } on FileSystemException catch (error) {
        _problem('Could not save the shortcuts: ${error.message}');
      }
    });
  }

  /// Whether the Liquid key is now on or off as asked.
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

  void _failed(bool on, String reason) =>
      _problem('Could not turn the Liquid key ${on ? 'on' : 'off'}: $reason');
}
