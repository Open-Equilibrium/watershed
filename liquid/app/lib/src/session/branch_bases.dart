import 'dart:io';

import '../settings_file.dart';

/// The branch each branch Liquid created started from, per device, repository and branch, in
/// Liquid's own data: a pull request's base unless the user chooses another (approved pull
/// request mockups §2). Several windows and processes share the file; each change re-reads it
/// under the file's lock.
class BranchBases {
  BranchBases({this.file, String? device})
    : _device = device ?? Platform.localHostname;

  /// None keeps the records in memory only.
  final SettingsFile? file;

  /// The host whose repositories the records name, since one data folder can serve several.
  final String _device;

  /// Records kept, oldest dropped first, so the file stays far below its size limit.
  static const _kept = 100;

  /// The records without a file.
  final _memory = <_Base>[];

  /// The branch `branch` of `repository` was created from, if Liquid created it.
  Future<String?> baseOf(String repository, String branch) async {
    final records = switch (file) {
      final stored? => _read(await stored.load()),
      null => _memory,
    };
    for (final record in records.reversed) {
      if (record.device == _device &&
          record.repository == repository &&
          record.branch == branch) {
        return record.base;
      }
    }
    return null;
  }

  /// Remembers that `branch` of `repository` was created from `base`.
  Future<void> record(String repository, String branch, String base) async {
    final added = (
      device: _device,
      repository: repository,
      branch: branch,
      base: base,
    );
    List<_Base> changed(List<_Base> records) {
      records
        ..removeWhere(
          (record) =>
              record.device == _device &&
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
      (json) => {
        'bases': [
          for (final record in changed(_read(json)))
            {
              'device': record.device,
              'repository': record.repository,
              'branch': record.branch,
              'base': record.base,
            },
        ],
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

/// Not empty, no leading `-`, no `.` or `..` segment and no control character, as the core
/// checks before a base reaches GitHub.
bool _branchName(String name) =>
    name.isNotEmpty &&
    !name.startsWith('-') &&
    !name.runes.any((rune) => rune < 0x20 || (rune >= 0x7f && rune < 0xa0)) &&
    name.split('/').every((segment) => segment != '.' && segment != '..');
