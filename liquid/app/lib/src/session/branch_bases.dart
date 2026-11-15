import 'dart:io';
import 'dart:math';

import '../settings_file.dart';

/// The branch each branch Liquid created started from, per computer, repository and branch, in
/// Liquid's own data: a pull request's base unless the user chooses another (approved pull
/// request mockups §2). Several windows and processes share the file; each change re-reads it
/// under the file's lock. A computer is named by a random id rather than by its host name, which
/// can change or repeat (ADR-0270). The id lives in the computer's own folder, not beside the
/// records: several computers may share those.
class BranchBases {
  BranchBases({this.file, this.deviceFile, String? device}) {
    _device = device;
  }

  /// None keeps the records in memory only.
  final SettingsFile? file;

  /// Where this computer keeps its id; none keeps the id in memory only.
  final SettingsFile? deviceFile;

  /// This computer's id in the records, once known.
  String? _device;

  /// Records kept, oldest dropped first, so the file stays far below its size limit.
  static const _kept = 100;

  /// The records without a file.
  final _memory = <_Base>[];

  /// The branch `branch` of `repository` was created from, if Liquid created it.
  Future<String?> baseOf(String repository, String branch) async {
    final device = await _deviceId();
    final records = switch (file) {
      final stored? => _read(await stored.load()),
      null => _memory,
    };
    for (final record in records.reversed) {
      if (record.device == device &&
          record.repository == repository &&
          record.branch == branch) {
        return record.base;
      }
    }
    return null;
  }

  /// Remembers that `branch` of `repository` was created from `base`.
  Future<void> record(String repository, String branch, String base) async {
    final device = await _deviceId();
    final added = (
      device: device,
      repository: repository,
      branch: branch,
      base: base,
    );
    List<_Base> changed(List<_Base> records) {
      records
        ..removeWhere(
          (record) =>
              record.device == device &&
              record.repository == repository &&
              record.branch == branch,
        )
        ..add(added);
      if (records.length > _kept) {
        records.removeRange(0, records.length - _kept);
      }
      return records;
    }

    final file = this.file;
    if (file == null) {
      changed(_memory);
      return;
    }
    await file.update(
      (json) => {...?json, 'bases': _encode(changed(_read(json)))},
    );
  }

  /// This computer's id. Its folder keeps the id once created, under that file's lock, and that
  /// first creation moves the records that named the host name to it. An id that an older Liquid
  /// stored in the shared records is not read: which computer it named cannot be told.
  Future<String> _deviceId() async {
    final known = _device;
    if (known != null) return known;
    final stored = deviceFile;
    if (stored == null) return _device = _newId();
    var id = '';
    if ((await stored.load())?['device'] case final String kept) {
      id = kept;
    } else {
      await stored.update((json) {
        // Another window may have created it since the read above.
        if (json?['device'] case final String kept) {
          id = kept;
          return {...?json};
        }
        id = _newId();
        return {...?json, 'device': id};
      });
    }
    // Also after a start that stopped between storing the id and moving the records.
    await _moveHostRecords(id);
    return _device = id;
  }

  /// Moves the records that named this computer by host name, as before ADR-0270, to `id`.
  Future<void> _moveHostRecords(String id) async {
    final stored = file;
    if (stored == null) return;
    final host = Platform.localHostname;
    if (!_read(await stored.load()).any((record) => record.device == host)) {
      return;
    }
    await stored.update(
      (json) => {
        ...?json,
        'bases': _encode([
          for (final record in _read(json))
            if (record.device == host)
              (
                device: id,
                repository: record.repository,
                branch: record.branch,
                base: record.base,
              )
            else
              record,
        ]),
      },
    );
  }
}

typedef _Base = ({
  String device,
  String repository,
  String branch,
  String base,
});

/// The stored records whose base can name a branch.
List<_Base> _read(Map<String, Object?>? json) => [
  for (final entry in json?['bases'] as List? ?? const [])
    if (entry
        case {
          'device': final String device,
          'repository': final String repository,
          'branch': final String branch,
          'base': final String base,
        }
        when _branchName(base))
      (device: device, repository: repository, branch: branch, base: base),
];

/// The records as the bases file keeps them.
List<Map<String, String>> _encode(List<_Base> records) => [
  for (final record in records)
    {
      'device': record.device,
      'repository': record.repository,
      'branch': record.branch,
      'base': record.base,
    },
];

/// Not empty, no leading `-`, no `.` or `..` segment and no control character, as the core
/// checks before a base reaches GitHub.
bool _branchName(String name) =>
    name.isNotEmpty &&
    !name.startsWith('-') &&
    !name.runes.any((rune) => rune < 0x20 || (rune >= 0x7f && rune < 0xa0)) &&
    name.split('/').every((segment) => segment != '.' && segment != '..');

/// A random id for a computer's records: 16 random bytes in hex.
String _newId() {
  final random = Random.secure();
  return [
    for (var i = 0; i < 16; i++)
      random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ].join();
}
