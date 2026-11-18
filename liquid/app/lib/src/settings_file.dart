import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

class SettingsFile {
  SettingsFile(this.file);

  final File file;

  static const _limit = 64 * 1024;

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

  Future<void> save(Map<String, Object?> json) async {
    await file.parent.create(recursive: true);
    final pending = File('${file.path}.$pid.${_saves++}.pending');
    try {
      await pending.writeAsString(jsonEncode(json), flush: true);
      await pending.rename(file.path);
    } on Object {
      try {
        await pending.delete();
      } on FileSystemException catch (_) {}
      rethrow;
    }
  }

  static var _saves = 0;
}

typedef SaveFailure = ({Object settings, String text});

class SavedSettings<T> extends ValueNotifier<T> {
  SavedSettings(super.value, this._save, this._name);

  final Future<void> Function(T settings)? _save;

  final String _name;

  Future<void> _saving = Future.value();

  void Function(String? failure)? saveOutcome;

  Future<void> change(T next) {
    value = next;
    return save();
  }

  @protected
  Future<void> save() {
    final write = _save;
    if (write == null) return Future.value();
    final settings = value;
    return _saving = _saving.then((_) async {
      try {
        await write(settings);
        saveOutcome?.call(null);
      } on Object catch (error) {
        final detail = error is FileSystemException ? error.message : '$error';
        final failure = 'Could not save $_name: $detail';
        saveOutcome?.call(failure);
        problem(failure);
      }
    });
  }

  void Function(String problem)? get report => _report;

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
