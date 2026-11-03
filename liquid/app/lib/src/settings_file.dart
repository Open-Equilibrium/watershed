import 'dart:async';
import 'dart:convert';
import 'dart:io';

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
    await pending.writeAsString(jsonEncode(json), flush: true);
    await pending.rename(file.path);
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
