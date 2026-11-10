import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

/// One of Liquid's settings files: a JSON object, replaced whole on each save.
class SettingsFile {
  SettingsFile(this.file);

  final File file;

  /// Far above any real settings; a larger file is not Liquid's.
  static const _limit = 64 * 1024;

  /// The stored object, or null before the first save or when the file is unreadable.
  Future<Map<String, Object?>?> load() async {
    try {
      if (await file.length() > _limit) return null;
      final json = jsonDecode(await file.readAsString());
      return json is Map<String, Object?> ? json : null;
    } on FileSystemException {
      return null;
    } on FormatException {
      return null;
    }
  }

  /// Replaces the file whole, so a crash leaves the old or the new settings. Each save writes
  /// its own temporary file, so saves at the same time cannot mix.
  Future<void> save(Map<String, Object?> json) async {
    await file.parent.create(recursive: true);
    final pending = File('${file.path}.$pid.${_saves++}.pending');
    try {
      await pending.writeAsString(jsonEncode(json), flush: true);
      await pending.rename(file.path);
    } on Object {
      // A failed save leaves no temporary file; the save's own error still propagates.
      try {
        await pending.delete();
      } on FileSystemException {
        // Never created, or already gone.
      }
      rethrow;
    }
  }

  static var _saves = 0;

  /// Updates waiting in this process, per file; the lock file orders other processes.
  static final _updates = <String, Future<void>>{};

  /// Saves `change` of the stored object, read under an exclusive lock on a sibling lock file,
  /// so writers in other windows and processes do not lose each other's changes.
  Future<void> update(
    Map<String, Object?> Function(Map<String, Object?>? json) change,
  ) async {
    final key = file.absolute.path;
    final previous = _updates[key];
    final mine = Completer<void>();
    _updates[key] = mine.future;
    try {
      await previous;
      await file.parent.create(recursive: true);
      final lock = await File('${file.path}.lock').open(mode: FileMode.append);
      try {
        await lock.lock(FileLock.blockingExclusive);
        await save(change(await load()));
      } finally {
        await lock.close();
      }
    } finally {
      mine.complete();
      if (identical(_updates[key], mine.future)) _updates.remove(key);
    }
  }
}

/// Settings in use that a [SettingsFile] keeps: each change applies at once and is saved after
/// the saves before it, so the last change is the one kept. A failed save goes to [report].
class SavedSettings<T> extends ValueNotifier<T> {
  SavedSettings(super.value, this._save, this._name);

  /// Writes the settings; none for settings kept only in memory.
  final Future<void> Function(T settings)? _save;

  /// What a failed save calls the settings, such as "the appearance".
  final String _name;

  Future<void> _saving = Future.value();

  /// Uses `next` at once and saves it.
  Future<void> change(T next) {
    value = next;
    return save();
  }

  /// Saves the settings in use after the saves before; completes once written or reported.
  @protected
  Future<void> save() {
    final write = _save;
    if (write == null) return Future.value();
    final settings = value;
    return _saving = _saving.then((_) async {
      try {
        await write(settings);
      } on FileSystemException catch (error) {
        problem('Could not save $_name: ${error.message}');
      }
    });
  }

  /// Receives what goes wrong with these settings; problems wait until it is set.
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

  @protected
  void problem(String problem) {
    final report = _report;
    if (report == null) {
      _unreported.add(problem);
    } else {
      report(problem);
    }
  }
}
