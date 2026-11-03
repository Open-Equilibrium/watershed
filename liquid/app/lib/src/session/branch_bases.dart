import '../settings_file.dart';

/// The branch each branch Liquid created started from, per repository, in Liquid's own data:
/// a pull request's base unless the user chooses another (approved pull request mockups §2).
class BranchBases {
  BranchBases([this._file]);

  /// None keeps the records in memory only.
  final SettingsFile? _file;

  /// Records kept, oldest dropped first, so the file stays far below its size limit.
  static const _kept = 100;

  List<({String repository, String branch, String base})>? _records;

  Future<List<({String repository, String branch, String base})>>
  _load() async {
    if (_records case final records?) return records;
    final json = await _file?.load();
    return _records = [
      for (final entry in json?['bases'] as List? ?? const [])
        if (entry case {
          'repository': final String repository,
          'branch': final String branch,
          'base': final String base,
        })
          (repository: repository, branch: branch, base: base),
    ];
  }

  /// The branch `branch` of `repository` was created from, if Liquid created it.
  Future<String?> baseOf(String repository, String branch) async {
    for (final record in (await _load()).reversed) {
      if (record.repository == repository && record.branch == branch) {
        return record.base;
      }
    }
    return null;
  }

  /// Remembers that `branch` of `repository` was created from `base`.
  Future<void> record(String repository, String branch, String base) async {
    final records = await _load()
      ..removeWhere(
        (record) => record.repository == repository && record.branch == branch,
      )
      ..add((repository: repository, branch: branch, base: base));
    if (records.length > _kept) records.removeRange(0, records.length - _kept);
    await _file?.save({
      'bases': [
        for (final record in records)
          {
            'repository': record.repository,
            'branch': record.branch,
            'base': record.base,
          },
      ],
    });
  }
}
