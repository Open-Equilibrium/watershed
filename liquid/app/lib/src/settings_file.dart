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

  /// Replaces the file whole, so a crash leaves the old or the new settings.
  Future<void> save(Map<String, Object?> json) async {
    await file.parent.create(recursive: true);
    final pending = File('${file.path}.pending');
    await pending.writeAsString(jsonEncode(json), flush: true);
    await pending.rename(file.path);
  }
}
